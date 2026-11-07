//! Test support: synthetic swing streams, a manual clock, and recordings
//! built from them. Not for production use.
//!
//! Directions are degrees from straight forward (away from the player),
//! positive = right. A **pull** is given by its backswing line `b` (the
//! line continued forward through the ball, so a pull toward the player's
//! left is `b > 0`); a **push** by its direction `f`.
//!
//! ```
//! use thumbswing::testing::{Clock, StreamBuilder};
//! use thumbswing::{Config, Output, Segmenter};
//!
//! // 1.5 in straight back over 150 ms, a 300 ms pause, 3 in straight forward
//! // over 100 ms (30 in/s), at 125 Hz and 400 CPI.
//! let samples = StreamBuilder::new(125.0)
//!     .pull(0.0, 600.0, 150.0)
//!     .pause_ms(300.0)
//!     .push(0.0, 1200.0, 100.0)
//!     .build();
//! let mut seg = Segmenter::new(&Config::default(), 30.0);
//! let mut clock = Clock::new();
//! let mut out = clock.feed(&mut seg, &samples);
//! out.extend(clock.pause_ms(&mut seg, 500));
//! assert!(out.iter().any(|o| matches!(o, Output::Swing { .. })));
//! ```

use crate::recording::{Entry, Header, Recording};
use crate::sample::Sample;
use crate::segment::{Output, Segmenter};

/// Time of the first report of a [`StreamBuilder`] stream, µs.
pub const STREAM_START_US: u64 = 1_000_000;

/// Builds a report stream from strokes and pauses at a fixed report rate.
#[derive(Debug, Clone)]
pub struct StreamBuilder {
    period_us: f64,
    t_us: f64,
    samples: Vec<Sample>,
}

#[allow(clippy::cast_precision_loss)]
impl StreamBuilder {
    /// A stream at `rate_hz` reports per second, starting at
    /// [`STREAM_START_US`].
    #[must_use]
    pub fn new(rate_hz: f64) -> Self {
        Self {
            period_us: 1e6 / rate_hz,
            t_us: STREAM_START_US as f64,
            samples: Vec::new(),
        }
    }

    /// A straight motion in direction `dir_deg` (from straight forward,
    /// positive = right) at the constant speed of `counts` per `duration_ms`,
    /// for the whole number of report periods nearest `duration_ms`. Counts
    /// are quantized per report with the remainder carried, as a sensor does;
    /// reports with no motion are not sent.
    #[must_use]
    pub fn stroke(self, dir_deg: f64, counts: f64, duration_ms: f64) -> Self {
        self.curved(dir_deg, 0.0, counts, duration_ms)
    }

    /// Like [`stroke`](Self::stroke), with the direction turning linearly from
    /// `dir_deg − curve_deg/2` to `dir_deg + curve_deg/2`.
    #[must_use]
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    pub fn curved(mut self, dir_deg: f64, curve_deg: f64, counts: f64, duration_ms: f64) -> Self {
        let n = ((duration_ms * 1000.0 / self.period_us).round() as usize).max(1);
        let per = counts * self.period_us / (duration_ms * 1000.0);
        let (mut fx, mut fy) = (0.0f64, 0.0f64);
        let (mut ix, mut iy) = (0i64, 0i64);
        for i in 0..n {
            let frac = (i as f64 + 0.5) / n as f64;
            let d = (dir_deg + curve_deg * (frac - 0.5)).to_radians();
            fx += per * d.sin();
            fy += -per * d.cos();
            let (nx, ny) = (fx.round() as i64, fy.round() as i64);
            let (dx, dy) = (nx - ix, ny - iy);
            (ix, iy) = (nx, ny);
            self.t_us += self.period_us;
            if dx != 0 || dy != 0 {
                self.samples.push(Sample::new(
                    self.t_us.round() as u64,
                    i32::try_from(dx).unwrap_or(i32::MAX),
                    i32::try_from(dy).unwrap_or(i32::MAX),
                ));
            }
        }
        self
    }

    /// A pull whose backswing line is `b_deg` (`b > 0`: pulled toward the
    /// player's left), of `counts` over `duration_ms`.
    #[must_use]
    pub fn pull(self, b_deg: f64, counts: f64, duration_ms: f64) -> Self {
        self.stroke(b_deg + 180.0, counts, duration_ms)
    }

    /// A push in direction `f_deg` of `counts` over `duration_ms`.
    #[must_use]
    pub fn push(self, f_deg: f64, counts: f64, duration_ms: f64) -> Self {
        self.stroke(f_deg, counts, duration_ms)
    }

    /// Silence for `ms`.
    #[must_use]
    pub fn pause_ms(mut self, ms: f64) -> Self {
        self.t_us += ms * 1000.0;
        self
    }

    /// One report of `(dx, dy)` one period from now.
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn report(mut self, dx: i32, dy: i32) -> Self {
        self.t_us += self.period_us;
        self.samples
            .push(Sample::new(self.t_us.round() as u64, dx, dy));
        self
    }

    /// The reports.
    #[must_use]
    pub fn build(self) -> Vec<Sample> {
        self.samples
    }
}

/// A straight stroke from [`STREAM_START_US`]: see [`StreamBuilder::stroke`].
#[must_use]
pub fn stroke(dir_deg: f64, counts: f64, duration_ms: f64, rate_hz: f64) -> Vec<Sample> {
    StreamBuilder::new(rate_hz)
        .stroke(dir_deg, counts, duration_ms)
        .build()
}

/// Add ±`amplitude`-count noise to every report with a seeded generator, so
/// the result is deterministic.
#[must_use]
pub fn jitter(samples: &[Sample], amplitude: i32, seed: u64) -> Vec<Sample> {
    let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let span = u64::from(amplitude.unsigned_abs()) * 2 + 1;
        i32::try_from((state >> 33) % span).unwrap_or(0) - amplitude
    };
    samples
        .iter()
        .map(|s| Sample::new(s.t_us, s.dx + next(), s.dy + next()))
        .collect()
}

/// A manual clock that feeds a [`Segmenter`] the way a poll loop does:
/// every report at its timestamp and a clock tick every millisecond in
/// between.
#[derive(Debug, Clone, Copy, Default)]
pub struct Clock {
    now: u64,
}

impl Clock {
    /// A clock at 0.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current time, µs.
    #[must_use]
    pub fn now_us(&self) -> u64 {
        self.now
    }

    fn drain(seg: &mut Segmenter, out: &mut Vec<Output>, first: Option<Output>) {
        out.extend(first);
        while let Some(o) = seg.next_output() {
            out.push(o);
        }
    }

    /// Feed `samples` in order, ticking every millisecond between them, and
    /// return every output.
    pub fn feed(&mut self, seg: &mut Segmenter, samples: &[Sample]) -> Vec<Output> {
        let mut out = Vec::new();
        for s in samples {
            while self.now + 1000 < s.t_us {
                self.now += 1000;
                let first = seg.poll(self.now);
                Self::drain(seg, &mut out, first);
            }
            self.now = self.now.max(s.t_us);
            let first = seg.push(*s, self.now);
            Self::drain(seg, &mut out, first);
        }
        out
    }

    /// Advance the clock `ms` with no reports, ticking every millisecond.
    pub fn pause_ms(&mut self, seg: &mut Segmenter, ms: u64) -> Vec<Output> {
        let mut out = Vec::new();
        for _ in 0..ms {
            self.now += 1000;
            let first = seg.poll(self.now);
            Self::drain(seg, &mut out, first);
        }
        out
    }
}

/// A recording of `samples` (re-based to start at 0) with `marks` at the given
/// sample indices, marked in its header as synthetic.
#[must_use]
pub fn synthetic_recording(
    samples: &[Sample],
    marks: &[(usize, &str)],
    cpi: f64,
    rate_hz: f64,
) -> Recording {
    let t0 = samples.first().map_or(0, |s| s.t_us);
    let mut entries = Vec::new();
    for (i, s) in samples.iter().enumerate() {
        for (_, note) in marks.iter().filter(|(at, _)| *at == i) {
            entries.push(Entry::Mark {
                t_us: s.t_us.saturating_sub(t0),
                note: (*note).to_owned(),
            });
        }
        entries.push(Entry::Sample(Sample::new(
            s.t_us.saturating_sub(t0),
            s.dx,
            s.dy,
        )));
    }
    Recording {
        header: Header {
            device: "synthetic trackball".to_owned(),
            cpi,
            rate_hz,
            report_limit: 127,
            grab: false,
            started: "2026-11-05T00:00:00Z".to_owned(),
            extra: vec![
                ("synthetic".to_owned(), "true".to_owned()),
                (
                    "note".to_owned(),
                    "\"generated by thumbswing::testing, not recorded from hardware\"".to_owned(),
                ),
            ],
            ..Header::default()
        },
        entries,
        path: None,
    }
}

/// Expected swing numbers for a fixture, from its `*.expect.json` sidecar.
///
/// ```json
/// {"note": "...", "tol_deg": 1.5, "tol_speed_pct": 3, "tol_len_in": 0.1,
///  "swings": [{"back_dir_deg": 0, "back_len_in": 1.5, "fwd_dir_deg": 0, "fwd_peak_in_s": 30}],
///  "cancels": ["Dribble"]}
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Expectations {
    /// Expected swings, in order.
    pub swings: Vec<crate::Swing>,
    /// Expected cancel reasons, in order (by name, e.g. `"Dribble"`).
    pub cancels: Vec<String>,
    /// Direction tolerance, degrees.
    pub tol_deg: f64,
    /// Peak-speed tolerance, percent.
    pub tol_speed_pct: f64,
    /// Backswing-length tolerance, inches.
    pub tol_len_in: f64,
}

impl Expectations {
    /// Load a sidecar.
    ///
    /// # Errors
    /// I/O errors and [`Error::Parse`](crate::Error::Parse).
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, crate::Error> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)?;
        Self::parse(&text).map_err(|message| crate::Error::Parse {
            file: path.display().to_string(),
            line: 1,
            message,
        })
    }

    /// Parse a sidecar's text.
    ///
    /// # Errors
    /// A description of what is malformed.
    pub fn parse(text: &str) -> Result<Self, String> {
        use crate::json::Value;
        let v = crate::json::parse(text)?;
        let num = |o: &Value, k: &str| o.get(k).and_then(Value::as_f64);
        let swings = v
            .get("swings")
            .and_then(Value::as_array)
            .ok_or("no swings array")?
            .iter()
            .map(|s| crate::Swing {
                back_dir_deg: num(s, "back_dir_deg").unwrap_or(0.0),
                back_len_in: num(s, "back_len_in").unwrap_or(0.0),
                fwd_dir_deg: num(s, "fwd_dir_deg").unwrap_or(0.0),
                fwd_peak_in_s: num(s, "fwd_peak_in_s").unwrap_or(0.0),
                ..crate::Swing::default()
            })
            .collect();
        let cancels = v
            .get("cancels")
            .and_then(Value::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|c| c.as_str().map(str::to_owned))
            .collect();
        Ok(Self {
            swings,
            cancels,
            tol_deg: num(&v, "tol_deg").unwrap_or(1.5),
            tol_speed_pct: num(&v, "tol_speed_pct").unwrap_or(3.0),
            tol_len_in: num(&v, "tol_len_in").unwrap_or(0.1),
        })
    }

    /// Compare what a replay produced with the expectations.
    ///
    /// # Errors
    /// A description of the first mismatch.
    pub fn check(
        &self,
        swings: &[crate::Swing],
        cancels: &[crate::CancelReason],
    ) -> Result<(), String> {
        if swings.len() != self.swings.len() {
            return Err(format!(
                "{} swings, expected {}",
                swings.len(),
                self.swings.len()
            ));
        }
        for (i, (got, want)) in swings.iter().zip(&self.swings).enumerate() {
            let n = i + 1;
            let dir = |g: f64, w: f64, what: &str| {
                if (g - w).abs() > self.tol_deg {
                    Err(format!(
                        "swing {n}: {what} {g:.2}°, expected {w}° ± {}",
                        self.tol_deg
                    ))
                } else {
                    Ok(())
                }
            };
            dir(got.back_dir_deg, want.back_dir_deg, "back_dir")?;
            dir(got.fwd_dir_deg, want.fwd_dir_deg, "fwd_dir")?;
            if (got.fwd_peak_in_s / want.fwd_peak_in_s - 1.0).abs() * 100.0 > self.tol_speed_pct {
                return Err(format!(
                    "swing {n}: peak {:.2} in/s, expected {} ± {} %",
                    got.fwd_peak_in_s, want.fwd_peak_in_s, self.tol_speed_pct
                ));
            }
            if (got.back_len_in - want.back_len_in).abs() > self.tol_len_in {
                return Err(format!(
                    "swing {n}: back_len {:.3} in, expected {} ± {}",
                    got.back_len_in, want.back_len_in, self.tol_len_in
                ));
            }
        }
        let got: Vec<String> = cancels.iter().map(|c| format!("{c:?}")).collect();
        if got != self.cancels {
            return Err(format!("cancels {got:?}, expected {:?}", self.cancels));
        }
        Ok(())
    }
}
