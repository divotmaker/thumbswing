//! The poll-driven client: an [`Input`] in, [`Event`]s out.

use std::collections::VecDeque;

use crate::club::{Club, Hand};
use crate::config::{Config, Tuning};
use crate::error::Error;
use crate::input::Input;
use crate::launch::{Delivery, Launch};
use crate::mapping;
use crate::profile::Smash;
use crate::sample::Sample;
use crate::segment::{CancelReason, Output, Phase, Segmenter};
use crate::swing::Swing;

/// Most samples taken from the input per [`Client::poll`].
const MAX_SAMPLES_PER_POLL: usize = 512;
/// First and longest reconnect delay after a disconnect, µs.
const RECONNECT_FIRST_US: u64 = 500_000;
const RECONNECT_MAX_US: u64 = 5_000_000;

/// A complete synthesized shot.
#[derive(Debug, Clone, PartialEq)]
pub struct Shot {
    /// Shot counter, from 1.
    pub number: u32,
    /// What the hands did.
    pub swing: Swing,
    /// Power, 0..=1.
    pub power: f64,
    /// The club delivery the swing maps to.
    pub delivery: Delivery,
    /// The ball launch.
    pub launch: Launch,
    /// The club the shot was made with.
    pub club: Club,
    /// The player's hand.
    pub hand: Hand,
    /// The reports the swing was measured from.
    pub raw: Vec<Sample>,
}

/// What [`Client::poll`] reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The input is open and the client is waiting for a swing (sent first,
    /// and again after a reconnect).
    Ready,
    /// The backswing so far, for a swing meter.
    Backswing {
        /// The backswing line continued forward, degrees, positive = right.
        dir_deg: f64,
        /// Length, inches, capped at full cock.
        len_in: f64,
    },
    /// The forward stroke hit the device's per-report limit; sent just before
    /// the [`Event::Shot`]. Lower the full-power speed or use a faster or
    /// wider report.
    Saturated {
        /// Saturated reports in the stroke.
        reports: u32,
    },
    /// A shot.
    Shot(Box<Shot>),
    /// A swing in progress was dropped.
    Canceled(CancelReason),
    /// The input device went away. The client keeps trying to reopen it and
    /// sends [`Event::Ready`] when it is back.
    Disconnected,
}

/// Turns an [`Input`] into shots.
///
/// Call [`poll`](Self::poll) every few milliseconds; it never blocks.
#[derive(Debug)]
pub struct Client<I: Input> {
    input: I,
    config: Config,
    tuning: Tuning,
    smash: Smash,
    seg: Segmenter,
    club: Club,
    hand: Hand,
    pending: VecDeque<Event>,
    shot_number: u32,
    last_swing: Option<Swing>,
    started: bool,
    disconnected: bool,
    reconnect_at: u64,
    reconnect_delay: u64,
}

impl<I: Input> Client<I> {
    /// A client over `input` with `config` and the default [`Tuning`].
    pub fn new(input: I, config: Config) -> Self {
        Self::with_tuning(input, config, Tuning::default())
    }

    /// A client over `input` with `config` and `tuning`.
    pub fn with_tuning(input: I, config: Config, tuning: Tuning) -> Self {
        let seg = Segmenter::new(&config, tuning.full_power_speed);
        Self {
            input,
            config,
            tuning,
            smash: Smash::default(),
            seg,
            club: Club::default(),
            hand: Hand::default(),
            pending: VecDeque::new(),
            shot_number: 0,
            last_swing: None,
            started: false,
            disconnected: false,
            reconnect_at: 0,
            reconnect_delay: RECONNECT_FIRST_US,
        }
    }

    /// Read what the input has, advance the clock, and return the next event.
    ///
    /// # Errors
    /// Read errors other than a disconnect (which is an [`Event`]).
    pub fn poll(&mut self) -> Result<Option<Event>, Error> {
        if let Some(e) = self.pending.pop_front() {
            return Ok(Some(e));
        }
        if !self.started {
            self.started = true;
            return Ok(Some(Event::Ready));
        }
        if self.disconnected {
            self.try_reconnect();
            return Ok(self.pending.pop_front());
        }

        self.seg.set_full_power_speed(self.tuning.full_power_speed);
        for _ in 0..MAX_SAMPLES_PER_POLL {
            match self.input.try_recv() {
                Ok(Some(s)) => {
                    let first = self.seg.push(s, s.t_us);
                    self.drain(first);
                }
                Ok(None) => break,
                Err(Error::Disconnected) => {
                    self.seg.reset(CancelReason::Disconnected);
                    self.drain(None);
                    self.pending.push_back(Event::Disconnected);
                    self.disconnected = true;
                    self.reconnect_delay = RECONNECT_FIRST_US;
                    self.reconnect_at = self.input.now_us() + self.reconnect_delay;
                    return Ok(self.pending.pop_front());
                }
                Err(e) => return Err(e),
            }
        }
        let now = self.input.now_us();
        let first = self.seg.poll(now);
        self.drain(first);
        Ok(self.pending.pop_front())
    }

    fn try_reconnect(&mut self) {
        let now = self.input.now_us();
        if now < self.reconnect_at {
            return;
        }
        if self.input.reconnect().is_ok() {
            self.disconnected = false;
            self.pending.push_back(Event::Ready);
        } else {
            self.reconnect_delay = (self.reconnect_delay * 2).min(RECONNECT_MAX_US);
            self.reconnect_at = now + self.reconnect_delay;
        }
    }

    /// Turn segmenter outputs (`first`, then whatever is queued) into events.
    fn drain(&mut self, first: Option<Output>) {
        let mut next = first;
        while let Some(out) = next.take().or_else(|| self.seg.next_output()) {
            match out {
                Output::Backswing { dir_deg, len_in } => {
                    self.pending.push_back(Event::Backswing { dir_deg, len_in });
                }
                Output::Canceled(r) => self.pending.push_back(Event::Canceled(r)),
                Output::Swing { swing, raw } => {
                    if swing.saturated() {
                        self.pending.push_back(Event::Saturated {
                            reports: swing.saturated_reports,
                        });
                    }
                    let shot = self.shot(swing, raw);
                    self.pending.push_back(Event::Shot(Box::new(shot)));
                }
            }
        }
    }

    /// Map a swing to a shot with the current club, hand and tuning.
    fn shot(&mut self, swing: Swing, raw: Vec<Sample>) -> Shot {
        self.shot_number += 1;
        self.last_swing = Some(swing);
        let delivery = mapping::delivery(&swing, self.club, self.hand, &self.tuning);
        let launch = mapping::launch_for(&delivery, self.club, &self.smash);
        Shot {
            number: self.shot_number,
            swing,
            power: mapping::power(&swing, &self.tuning),
            delivery,
            launch,
            club: self.club,
            hand: self.hand,
            raw,
        }
    }

    /// The club for the next shot.
    #[must_use]
    pub fn club(&self) -> Club {
        self.club
    }

    /// Select the club for the next shot.
    pub fn set_club(&mut self, club: Club) {
        self.club = club;
    }

    /// The player's hand.
    #[must_use]
    pub fn hand(&self) -> Hand {
        self.hand
    }

    /// Set the player's hand.
    pub fn set_hand(&mut self, hand: Hand) {
        self.hand = hand;
    }

    /// The tuning.
    #[must_use]
    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    /// The tuning, for changes; they apply from the next report.
    pub fn tuning_mut(&mut self) -> &mut Tuning {
        &mut self.tuning
    }

    /// The device configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Change the configuration: thresholds, CPI and report limit apply at
    /// once; a change of `grab` is applied to the input.
    ///
    /// # Errors
    /// [`Error::Busy`] if a grab was asked for and another process holds it.
    pub fn set_config(&mut self, config: Config) -> Result<(), Error> {
        let grab_changed = config.grab != self.config.grab;
        self.seg.set_config(&config);
        self.config = config;
        if grab_changed {
            self.input.set_grab(self.config.grab)?;
        }
        Ok(())
    }

    /// The smash rule.
    #[must_use]
    pub fn smash(&self) -> &Smash {
        &self.smash
    }

    /// Replace the smash rule.
    pub fn set_smash(&mut self, smash: Smash) {
        self.smash = smash;
    }

    /// Where the segmenter is in a swing.
    #[must_use]
    pub fn state(&self) -> Phase {
        self.seg.phase()
    }

    /// The last swing, if any.
    #[must_use]
    pub fn last_swing(&self) -> Option<&Swing> {
        self.last_swing.as_ref()
    }

    /// Whether the input is currently disconnected.
    #[must_use]
    pub fn is_disconnected(&self) -> bool {
        self.disconnected
    }

    /// The input.
    pub fn input(&self) -> &I {
        &self.input
    }

    /// The input, mutably.
    pub fn input_mut(&mut self) -> &mut I {
        &mut self.input
    }

    /// Give the input back.
    pub fn into_input(self) -> I {
        self.input
    }
}
