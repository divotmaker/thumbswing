//! The clocked swing state machine: [`Sample`]s in, [`Swing`]s out.
//!
//! ```text
//! IDLE ──pull──▶ BACKSWING ──stop (dwell) or reversal──▶ COILED ──push──▶ FORWARD ──▶ COOLDOWN ──▶ IDLE
//! ```
//!
//! - **IDLE → BACKSWING** when the windowed speed exceeds `v_start` and the
//!   motion in the window is a pull toward the player (within `pull_cone_deg`
//!   of straight back). Forward motion from rest is ignored.
//! - **BACKSWING** sums the displacement. It ends when the speed stays below
//!   `v_stop` for `dwell_min` (silence counts as stopped: the clock drives it)
//!   or when the ball reverses. A backswing shorter than `back_min` is
//!   canceled ([`CancelReason::Short`]); one still moving after `back_max_s`
//!   is canceled ([`CancelReason::Timeout`]).
//! - **COILED** waits for the push, up to `coil_timeout_s` after the last
//!   motion ([`CancelReason::CoilTimeout`]). Forward motion starts a
//!   provisional stroke. It becomes a stroke (FORWARD) once its travel
//!   reaches `fwd_min` and its speed has reached `v_min_swing`. A forward
//!   motion that dies first leaves the backswing intact
//!   ([`CancelReason::Dribble`] is reported as information when it was more
//!   than a twitch). A new pull starts a new backswing.
//! - **FORWARD** tracks the windowed speed and its peak and sums the
//!   displacement. The stroke ends when the speed falls below half the peak
//!   after the peak, or `fwd_window_ms` after it started; a [`Swing`] is
//!   emitted.
//! - **COOLDOWN** swallows the coasting ball until it is quiet for
//!   `idle_settle_ms` or `cooldown_max_s` has passed. A pull pre-empts it and
//!   starts the next backswing.
//!
//! Speed is measured over a trailing window (`speed_window_ms`), never per
//! report, so a 125 Hz and a 1000 Hz device give the same numbers. A report
//! after `gap_ms` of silence, or one whose timestamp is not after the
//! previous one, starts a new speed series: the window is emptied and that
//! report has no speed of its own.
//!
//! The segmenter has no clock of its own. Call [`Segmenter::push`] for every
//! report and [`Segmenter::poll`] regularly with the current time (on the
//! samples' clock) so that silence advances the timers, then drain the
//! outputs with [`Segmenter::next_output`].

use std::collections::VecDeque;

use crate::config::{Config, Thresholds};
use crate::sample::Sample;
use crate::swing::Swing;

/// Why a swing in progress was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReason {
    /// The backswing kept moving for longer than `back_max_s`.
    Timeout,
    /// The coiled backswing waited longer than `coil_timeout_s`.
    CoilTimeout,
    /// The backswing was shorter than `back_min`.
    Short,
    /// A forward motion died before it became a stroke. Information only:
    /// the backswing is still coiled.
    Dribble,
    /// The input device went away.
    Disconnected,
}

/// Where the segmenter is in a swing, for a swing meter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for a backswing.
    Idle,
    /// Pulling back.
    Backswing,
    /// At the top, waiting for the push.
    Coiled,
    /// Pushing.
    Forward,
    /// Ignoring the coasting ball after a shot.
    Cooldown,
}

/// What the segmenter reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// The backswing so far (sent while pulling, and once more when coiled).
    Backswing {
        /// The backswing line continued forward, degrees, positive = right.
        dir_deg: f64,
        /// Length so far, inches, capped at the full-cock length.
        len_in: f64,
    },
    /// A completed swing and the reports it was measured from.
    Swing {
        /// The swing geometry.
        swing: Swing,
        /// Every report from the start of the backswing to the end of the
        /// forward stroke.
        raw: Vec<Sample>,
    },
    /// A swing in progress was dropped (or, for
    /// [`CancelReason::Dribble`], a forward motion was ignored).
    Canceled(CancelReason),
}

/// Minimum interval between live [`Output::Backswing`] updates, µs.
const BACKSWING_UPDATE_US: u64 = 20_000;

/// Window the direction of motion is taken over, µs.
const DIR_WINDOW_US: u64 = 8_000;

#[derive(Debug, Clone, Copy)]
struct Entry {
    t: u64,
    dx: i32,
    dy: i32,
    /// Whether the report counts toward speed (not the first of a series).
    timed: bool,
}

#[derive(Debug, Clone)]
struct Back {
    sx: i64,
    sy: i64,
    start: u64,
    /// Time of the last report that itself pointed backward.
    last_back_t: u64,
    /// Reports since `last_back_t` that pointed forward on their own but were
    /// summed into the backswing because the motion still was: at a reversal
    /// they move to the forward stroke.
    tail: (i64, i64),
    tail_start: Option<u64>,
    /// Time of the last report of any kind (for the coil timeout).
    last_motion: u64,
    slow_since: Option<u64>,
    last_emit: u64,
}

impl Back {
    #[allow(clippy::cast_precision_loss)]
    fn len_counts(&self) -> f64 {
        (self.sx as f64).hypot(self.sy as f64)
    }

    /// Dot product of a report with the pull direction.
    fn along(&self, dx: i32, dy: i32) -> i64 {
        self.sx * i64::from(dx) + self.sy * i64::from(dy)
    }

    /// The backswing line continued forward: `angle(−Σdx, Σdy)`.
    #[allow(clippy::cast_precision_loss)]
    fn dir_deg(&self) -> f64 {
        (-self.sx as f64).atan2(self.sy as f64).to_degrees()
    }
}

#[derive(Debug, Clone)]
struct Fwd {
    /// Reports after this time count toward the stroke's speed.
    origin: u64,
    start: u64,
    sx: i64,
    sy: i64,
    peak: f64,
    peak_t: u64,
    slow_since: Option<u64>,
    sat_run: u32,
    sat_max_run: u32,
    sat_total: u32,
    run_start_sum: (i64, i64),
    pre_sat: Option<(i64, i64)>,
}

impl Fwd {
    fn new(origin: u64, start: u64) -> Self {
        Self {
            origin,
            start,
            sx: 0,
            sy: 0,
            peak: 0.0,
            peak_t: start,
            slow_since: None,
            sat_run: 0,
            sat_max_run: 0,
            sat_total: 0,
            run_start_sum: (0, 0),
            pre_sat: None,
        }
    }

    fn add(&mut self, e: &Entry, limit: i32) {
        let saturated = e.dx.abs() >= limit || e.dy.abs() >= limit;
        if saturated {
            if self.sat_run == 0 {
                self.run_start_sum = (self.sx, self.sy);
            }
            self.sat_run += 1;
            self.sat_total += 1;
            self.sat_max_run = self.sat_max_run.max(self.sat_run);
            if self.sat_run == 2 && self.pre_sat.is_none() {
                self.pre_sat = Some(self.run_start_sum);
            }
        } else {
            self.sat_run = 0;
        }
        self.sx += i64::from(e.dx);
        self.sy += i64::from(e.dy);
    }

    fn update_peak(&mut self, speed: f64, now: u64) {
        if speed > self.peak {
            self.peak = speed;
            self.peak_t = now;
        }
    }

    /// Forward travel along the push line `−back`, counts.
    #[allow(clippy::cast_precision_loss)]
    fn travel(&self, back: &Back) -> f64 {
        let len = back.len_counts();
        if len == 0.0 {
            return 0.0;
        }
        -((self.sx * back.sx + self.sy * back.sy) as f64) / len
    }

    /// The push direction `angle(Σdx, −Σdy)`, from the reports before
    /// saturation when the stroke saturated.
    #[allow(clippy::cast_precision_loss)]
    fn dir_deg(&self) -> f64 {
        let (sx, sy) = match self.pre_sat {
            Some(s) if s != (0, 0) => s,
            _ => (self.sx, self.sy),
        };
        (sx as f64).atan2(-sy as f64).to_degrees()
    }
}

#[derive(Debug, Clone)]
enum State {
    Idle,
    Backswing(Back),
    Coiled {
        back: Back,
        /// False while the coil came straight from a reversal and the ball
        /// has not stopped yet: a pull then resumes the backswing.
        settled: bool,
        provisional: Option<Fwd>,
    },
    Forward {
        back: Back,
        fwd: Fwd,
    },
    Cooldown {
        since: u64,
        quiet_since: Option<u64>,
    },
}

/// Thresholds in counts and microseconds at one full-power speed and CPI.
#[derive(Debug, Clone, Copy)]
struct Limits {
    window_us: u64,
    gap_us: u64,
    v_start: f64,
    v_stop: f64,
    v_min_swing: f64,
    dwell_us: u64,
    back_min: f64,
    full_cock: f64,
    fwd_min: f64,
    back_max_us: u64,
    coil_timeout_us: u64,
    fwd_window_us: u64,
    idle_settle_us: u64,
    cooldown_max_us: u64,
    pull_cos: f64,
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn secs_to_us(s: f64) -> u64 {
    (s.max(0.0) * 1e6) as u64
}

impl Limits {
    fn new(t: &Thresholds, full_cps: f64) -> Self {
        let l_full = full_cps * 0.1;
        Self {
            window_us: u64::from(t.speed_window_ms.max(1)) * 1000,
            gap_us: u64::from(t.gap_ms) * 1000,
            v_start: t.v_start * full_cps,
            v_stop: t.v_stop * full_cps,
            v_min_swing: t.v_min_swing * full_cps,
            dwell_us: u64::from(t.dwell_min_ms) * 1000,
            back_min: t.back_min * l_full,
            full_cock: t.full_cock * l_full,
            fwd_min: t.fwd_min * l_full,
            back_max_us: secs_to_us(t.back_max_s),
            coil_timeout_us: secs_to_us(t.coil_timeout_s),
            fwd_window_us: u64::from(t.fwd_window_ms) * 1000,
            idle_settle_us: u64::from(t.idle_settle_ms) * 1000,
            cooldown_max_us: secs_to_us(t.cooldown_max_s),
            pull_cos: t.pull_cone_deg.to_radians().cos(),
        }
    }
}

/// The swing state machine.
#[derive(Debug, Clone)]
pub struct Segmenter {
    thresholds: Thresholds,
    cpi: f64,
    report_limit: i32,
    full_power_speed: f64,
    limits: Limits,
    window: VecDeque<Entry>,
    last_t: Option<u64>,
    last_tick: u64,
    state: State,
    raw: Vec<Sample>,
    out: VecDeque<Output>,
}

impl Segmenter {
    /// A segmenter for a device with `config`, scaled so that
    /// `full_power_speed` inches per second is 100 % power.
    #[must_use]
    pub fn new(config: &Config, full_power_speed: f64) -> Self {
        let cpi = config.cpi.max(1.0);
        Self {
            thresholds: config.thresholds,
            cpi,
            report_limit: config.report_limit.max(1),
            full_power_speed,
            limits: Limits::new(&config.thresholds, full_power_speed * cpi),
            window: VecDeque::new(),
            last_t: None,
            last_tick: 0,
            state: State::Idle,
            raw: Vec::new(),
            out: VecDeque::new(),
        }
    }

    /// Apply a new device configuration. A swing in progress keeps going.
    pub fn set_config(&mut self, config: &Config) {
        self.thresholds = config.thresholds;
        self.cpi = config.cpi.max(1.0);
        self.report_limit = config.report_limit.max(1);
        self.rescale();
    }

    /// Rescale the thresholds: `full_power_speed` inches per second is 100 %
    /// power.
    pub fn set_full_power_speed(&mut self, full_power_speed: f64) {
        if (full_power_speed - self.full_power_speed).abs() > f64::EPSILON {
            self.full_power_speed = full_power_speed;
            self.rescale();
        }
    }

    fn rescale(&mut self) {
        self.limits = Limits::new(&self.thresholds, self.full_power_speed * self.cpi);
    }

    /// The current phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        match self.state {
            State::Idle => Phase::Idle,
            State::Backswing(_) => Phase::Backswing,
            State::Coiled { .. } => Phase::Coiled,
            State::Forward { .. } => Phase::Forward,
            State::Cooldown { .. } => Phase::Cooldown,
        }
    }

    /// Drop any swing in progress and the speed history. Emits
    /// `Canceled(reason)` if a swing was in progress.
    pub fn reset(&mut self, reason: CancelReason) {
        if matches!(
            self.state,
            State::Backswing(_) | State::Coiled { .. } | State::Forward { .. }
        ) {
            self.out.push_back(Output::Canceled(reason));
        }
        self.state = State::Idle;
        self.window.clear();
        self.raw.clear();
        self.last_t = None;
    }

    /// The next pending output, if any.
    pub fn next_output(&mut self) -> Option<Output> {
        self.out.pop_front()
    }

    /// Feed one report, then advance the clock to `now_us` (which must not be
    /// before the report). Returns the first pending output; call
    /// [`next_output`](Self::next_output) for the rest.
    pub fn push(&mut self, sample: Sample, now_us: u64) -> Option<Output> {
        let (t, gap) = match self.last_t {
            Some(last) if sample.t_us <= last => (last, true),
            Some(last) => (sample.t_us, sample.t_us - last > self.limits.gap_us),
            None => (sample.t_us, true),
        };
        self.tick(t);
        if gap {
            self.window.clear();
        }
        let e = Entry {
            t,
            dx: sample.dx,
            dy: sample.dy,
            timed: !gap,
        };
        self.window.push_back(e);
        self.last_t = Some(t);
        if !matches!(self.state, State::Idle | State::Cooldown { .. }) {
            self.raw.push(Sample::new(t, sample.dx, sample.dy));
        }
        self.report(&e);
        self.tick(now_us.max(t));
        self.out.pop_front()
    }

    /// Advance the clock to `now_us` with no report: timers, timeouts and the
    /// decay of the windowed speed. Returns the first pending output.
    pub fn poll(&mut self, now_us: u64) -> Option<Output> {
        self.tick(now_us);
        self.out.pop_front()
    }

    /// Windowed speed at `now`: the displacement of the timed reports after
    /// `after` in the trailing window over the window length, counts/s.
    #[allow(clippy::cast_precision_loss)]
    fn speed(&self, now: u64, after: Option<u64>) -> f64 {
        let from = now.saturating_sub(self.limits.window_us);
        let (x, y) = self
            .window
            .iter()
            .filter(|e| e.timed && e.t > from && e.t <= now && after.is_none_or(|a| e.t > a))
            .fold((0i64, 0i64), |(x, y), e| {
                (x + i64::from(e.dx), y + i64::from(e.dy))
            });
        (x as f64).hypot(y as f64) / (self.limits.window_us as f64 / 1e6)
    }

    /// Displacement of the reports in the last `DIR_WINDOW_US`: the direction
    /// the ball is moving in, robust to single-count reports at high rates.
    fn motion(&self, now: u64) -> (i32, i32) {
        let from = now.saturating_sub(DIR_WINDOW_US);
        self.window
            .iter()
            .filter(|e| e.t > from && e.t <= now)
            .fold((0, 0), |(x, y), e| (x + e.dx, y + e.dy))
    }

    /// If the window at `now` holds a pull toward the player (within the pull
    /// cone of straight back) faster than `v_start`, the displacement of its
    /// pulling reports and the time of the first one.
    #[allow(clippy::cast_precision_loss)]
    fn pull_in_window(&self, now: u64) -> Option<(i64, i64, u64)> {
        let from = now.saturating_sub(self.limits.window_us);
        let recent = || self.window.iter().filter(|e| e.t > from && e.t <= now);
        let (wx, wy) = recent().fold((0i64, 0i64), |(x, y), e| {
            (x + i64::from(e.dx), y + i64::from(e.dy))
        });
        let len = (wx as f64).hypot(wy as f64);
        if wy <= 0
            || (wy as f64) < len * self.limits.pull_cos
            || self.speed(now, None) <= self.limits.v_start
        {
            return None;
        }
        let mut start = now;
        let (sx, sy) = recent()
            .filter(|e| i64::from(e.dx) * wx + i64::from(e.dy) * wy >= 0)
            .fold((0i64, 0i64), |(x, y), e| {
                start = start.min(e.t);
                (x + i64::from(e.dx), y + i64::from(e.dy))
            });
        Some((sx, sy, start))
    }

    fn start_backswing(&mut self, now: u64, sx: i64, sy: i64, start: u64) {
        self.raw = self
            .window
            .iter()
            .filter(|e| e.t >= start && e.t <= now)
            .map(|e| Sample::new(e.t, e.dx, e.dy))
            .collect();
        self.state = State::Backswing(Back {
            sx,
            sy,
            start,
            last_back_t: now,
            tail: (0, 0),
            tail_start: None,
            last_motion: now,
            slow_since: None,
            last_emit: 0,
        });
    }

    #[allow(clippy::cast_precision_loss)]
    fn backswing_output(&self, back: &Back) -> Output {
        Output::Backswing {
            dir_deg: back.dir_deg(),
            len_in: back.len_counts().min(self.limits.full_cock) / self.cpi,
        }
    }

    /// Start a backswing if the window holds a pull; otherwise `None`.
    fn try_pull(&mut self, now: u64) -> Option<State> {
        let (sx, sy, start) = self.pull_in_window(now)?;
        self.start_backswing(now, sx, sy, start);
        Some(std::mem::replace(&mut self.state, State::Idle))
    }

    fn report(&mut self, e: &Entry) {
        let (mx, my) = self.motion(e.t);
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match state {
            State::Idle | State::Cooldown { .. } => self.try_pull(e.t).unwrap_or(state),
            State::Backswing(mut back) => {
                if back.along(mx, my) < 0 {
                    if back.len_counts() < self.limits.back_min {
                        self.out.push_back(Output::Canceled(CancelReason::Short));
                        self.raw.clear();
                        State::Idle
                    } else {
                        back.last_motion = e.t;
                        back.sx -= back.tail.0;
                        back.sy -= back.tail.1;
                        let mut fwd = Fwd::new(back.last_back_t, back.tail_start.unwrap_or(e.t));
                        (fwd.sx, fwd.sy) = back.tail;
                        back.tail = (0, 0);
                        back.tail_start = None;
                        fwd.add(e, self.report_limit);
                        let speed = self.speed(e.t, Some(fwd.origin));
                        fwd.update_peak(speed, e.t);
                        self.coiled_or_forward(back, false, fwd)
                    }
                } else {
                    if back.along(e.dx, e.dy) < 0 {
                        back.tail.0 += i64::from(e.dx);
                        back.tail.1 += i64::from(e.dy);
                        back.tail_start.get_or_insert(e.t);
                    } else {
                        back.tail = (0, 0);
                        back.tail_start = None;
                        back.last_back_t = e.t;
                    }
                    back.sx += i64::from(e.dx);
                    back.sy += i64::from(e.dy);
                    back.last_motion = e.t;
                    if e.t >= back.last_emit + BACKSWING_UPDATE_US {
                        back.last_emit = e.t;
                        self.out.push_back(self.backswing_output(&back));
                    }
                    State::Backswing(back)
                }
            }
            State::Coiled {
                mut back,
                settled,
                provisional,
            } => {
                back.last_motion = e.t;
                if back.along(mx, my) < 0 {
                    let mut fwd = provisional.unwrap_or_else(|| Fwd::new(back.last_back_t, e.t));
                    fwd.add(e, self.report_limit);
                    let speed = self.speed(e.t, Some(fwd.origin));
                    fwd.update_peak(speed, e.t);
                    self.coiled_or_forward(back, settled, fwd)
                } else if let (false, Some(fwd)) = (settled, &provisional) {
                    // The ball went back to pulling before it ever stopped:
                    // one backswing with a wobble in it.
                    back.sx += fwd.sx + i64::from(e.dx);
                    back.sy += fwd.sy + i64::from(e.dy);
                    back.last_back_t = e.t;
                    back.slow_since = None;
                    State::Backswing(back)
                } else if let Some(next) = self.try_pull(e.t) {
                    next
                } else {
                    let provisional = provisional.map(|mut f| {
                        f.add(e, self.report_limit);
                        f
                    });
                    State::Coiled {
                        back,
                        settled,
                        provisional,
                    }
                }
            }
            State::Forward { back, mut fwd } => {
                fwd.add(e, self.report_limit);
                let speed = self.speed(e.t, Some(fwd.origin));
                fwd.update_peak(speed, e.t);
                if self.pull_in_window(e.t).is_some() {
                    // A pull: the stroke is over, and this is the next backswing.
                    self.finish(&back, &fwd, e.t);
                    self.try_pull(e.t)
                        .unwrap_or_else(|| std::mem::replace(&mut self.state, State::Idle))
                } else {
                    State::Forward { back, fwd }
                }
            }
        };
    }

    /// Commit a provisional stroke to FORWARD once it has traveled `fwd_min`
    /// and reached `v_min_swing`; otherwise stay coiled.
    fn coiled_or_forward(&mut self, back: Back, settled: bool, fwd: Fwd) -> State {
        if fwd.travel(&back) >= self.limits.fwd_min && fwd.peak >= self.limits.v_min_swing {
            State::Forward { back, fwd }
        } else {
            State::Coiled {
                back,
                settled,
                provisional: Some(fwd),
            }
        }
    }

    /// Emit the swing and enter COOLDOWN.
    #[allow(clippy::cast_precision_loss)]
    fn finish(&mut self, back: &Back, fwd: &Fwd, now: u64) {
        let back_len = back.len_counts();
        let swing = Swing {
            back_dir_deg: back.dir_deg(),
            back_len_in: back_len.min(self.limits.full_cock) / self.cpi,
            back_fraction: if self.limits.full_cock > 0.0 {
                (back_len / self.limits.full_cock).min(1.0)
            } else {
                1.0
            },
            fwd_dir_deg: fwd.dir_deg(),
            fwd_peak_in_s: fwd.peak / self.cpi,
            fwd_len_in: fwd.travel(back) / self.cpi,
            saturated_reports: if fwd.sat_max_run >= 2 {
                fwd.sat_total
            } else {
                0
            },
            duration_ms: now.saturating_sub(back.start) as f64 / 1000.0,
            pause_ms: fwd.start.saturating_sub(back.last_back_t) as f64 / 1000.0,
        };
        let raw = std::mem::take(&mut self.raw);
        self.out.push_back(Output::Swing { swing, raw });
        self.state = State::Cooldown {
            since: now,
            quiet_since: None,
        };
    }

    // One match over the states; splitting it would scatter the transitions.
    #[allow(clippy::too_many_lines)]
    fn tick(&mut self, now: u64) {
        if now < self.last_tick {
            return;
        }
        self.last_tick = now;
        let keep_from = now.saturating_sub(self.limits.window_us.max(100_000));
        while self.window.front().is_some_and(|e| e.t < keep_from) {
            self.window.pop_front();
        }
        let l = self.limits;
        let state = std::mem::replace(&mut self.state, State::Idle);
        self.state = match state {
            State::Idle => State::Idle,
            State::Backswing(mut back) => {
                if now.saturating_sub(back.start) > l.back_max_us {
                    self.out.push_back(Output::Canceled(CancelReason::Timeout));
                    self.raw.clear();
                    State::Idle
                } else if self.speed(now, None) < l.v_stop {
                    let since = *back.slow_since.get_or_insert(now);
                    if now - since >= l.dwell_us {
                        if back.len_counts() < l.back_min {
                            self.out.push_back(Output::Canceled(CancelReason::Short));
                            self.raw.clear();
                            State::Idle
                        } else {
                            self.out.push_back(self.backswing_output(&back));
                            State::Coiled {
                                back,
                                settled: true,
                                provisional: None,
                            }
                        }
                    } else {
                        State::Backswing(back)
                    }
                } else {
                    back.slow_since = None;
                    State::Backswing(back)
                }
            }
            State::Coiled {
                back,
                settled,
                provisional,
            } => {
                if now.saturating_sub(back.last_motion) >= l.coil_timeout_us {
                    self.out
                        .push_back(Output::Canceled(CancelReason::CoilTimeout));
                    self.raw.clear();
                    State::Idle
                } else if let Some(mut fwd) = provisional {
                    let speed = self.speed(now, Some(fwd.origin));
                    fwd.update_peak(speed, now);
                    if speed < l.v_stop {
                        let since = *fwd.slow_since.get_or_insert(now);
                        if now - since >= l.dwell_us {
                            if fwd.peak >= l.v_stop {
                                self.out.push_back(Output::Canceled(CancelReason::Dribble));
                            }
                            if !settled {
                                self.out.push_back(self.backswing_output(&back));
                            }
                            State::Coiled {
                                back,
                                settled: true,
                                provisional: None,
                            }
                        } else {
                            State::Coiled {
                                back,
                                settled,
                                provisional: Some(fwd),
                            }
                        }
                    } else {
                        fwd.slow_since = None;
                        State::Coiled {
                            back,
                            settled,
                            provisional: Some(fwd),
                        }
                    }
                } else {
                    State::Coiled {
                        back,
                        settled,
                        provisional: None,
                    }
                }
            }
            State::Forward { back, mut fwd } => {
                let speed = self.speed(now, Some(fwd.origin));
                fwd.update_peak(speed, now);
                let fell = now > fwd.peak_t && speed < 0.5 * fwd.peak;
                if fell || now.saturating_sub(fwd.start) >= l.fwd_window_us {
                    self.finish(&back, &fwd, now);
                    std::mem::replace(&mut self.state, State::Idle)
                } else {
                    State::Forward { back, fwd }
                }
            }
            State::Cooldown {
                since,
                mut quiet_since,
            } => {
                if now.saturating_sub(since) >= l.cooldown_max_us {
                    State::Idle
                } else if self.speed(now, None) < l.v_stop {
                    let q = *quiet_since.get_or_insert(now);
                    if now - q >= l.idle_settle_us {
                        State::Idle
                    } else {
                        State::Cooldown { since, quiet_since }
                    }
                } else {
                    State::Cooldown {
                        since,
                        quiet_since: None,
                    }
                }
            }
        };
    }
}
