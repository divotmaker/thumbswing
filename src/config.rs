//! Device configuration, segmenter thresholds, the tuning knobs, and
//! [`Settings`], all of them plus the choice of device.
//!
//! With the `config` feature every type here is a serde struct: a field at its
//! default is left out when written, so a default [`Settings`] serializes to
//! nothing, and every field is optional when read.

#[cfg(feature = "config")]
use serde::{Deserialize, Serialize};

use crate::club::{Club, Hand};

/// How the backswing direction bends the shot. Serialized as `arcade` or
/// `physical`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "config", serde(rename_all = "lowercase"))]
pub enum CurveSense {
    /// The arcade feel: push toward where the ball should start, pull back
    /// from where it should finish. The ball curves toward the backswing line,
    /// so the club path sent downstream is the backswing reflected about the
    /// push.
    #[default]
    Arcade,
    /// Modern ball-flight physics: the backswing line is the club path and the
    /// ball curves away from it.
    Physical,
}

impl CurveSense {
    /// Short label for status lines: `arcade` or `physical`.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            CurveSense::Arcade => "arcade",
            CurveSense::Physical => "physical",
        }
    }
}

/// Tuning knobs: plain fields the caller sets. A change takes effect on the
/// next swing.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "config", serde(default))]
pub struct Tuning {
    /// Forward peak speed that counts as 100 % power, inches per second.
    /// Every segmenter threshold is a fraction of it.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::full_power_speed")
    )]
    pub full_power_speed: f64,
    /// Driver club speed at full power, mph. Other clubs scale it by their
    /// speed ratio.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::max_club_speed")
    )]
    pub max_club_speed: f64,
    /// Putter club speed at full power, mph.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::max_putt_speed")
    )]
    pub max_putt_speed: f64,
    /// Face angle per degree of push direction.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::angle_gain")
    )]
    pub angle_gain: f64,
    /// Putt line per degree of push direction.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::putt_angle_gain")
    )]
    pub putt_angle_gain: f64,
    /// Face-to-path per degree between the backswing line and the push.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::curve_gain")
    )]
    pub curve_gain: f64,
    /// Which way the backswing bends the shot.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::curve_sense")
    )]
    pub curve_sense: CurveSense,
    /// Share of the power set by the backswing length (0 = forward speed only).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::backswing_power")
    )]
    pub backswing_power: f64,
    /// Exponent of the power curve (1 = linear).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::power_gamma")
    )]
    pub power_gamma: f64,
    /// Mirror the stroke directions for a left-handed player.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::mirror_for_left_hand")
    )]
    pub mirror_for_left_hand: bool,
    /// Largest face angle, degrees.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::face_max")
    )]
    pub face_max: f64,
    /// Largest face-to-path, degrees.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::tuning::face_to_path_max")
    )]
    pub face_to_path_max: f64,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            full_power_speed: 15.0,
            max_club_speed: 100.0,
            max_putt_speed: 11.0,
            angle_gain: 0.25,
            putt_angle_gain: 1.0,
            curve_gain: 0.20,
            curve_sense: CurveSense::Arcade,
            backswing_power: 0.0,
            power_gamma: 1.0,
            mirror_for_left_hand: false,
            face_max: 20.0,
            face_to_path_max: 25.0,
        }
    }
}

/// Stroke segmentation thresholds.
///
/// Speeds are fractions of [`Tuning::full_power_speed`] and lengths are
/// fractions of `ℓ_full`, the distance the ball travels in 100 ms at full
/// power, so setting the full-power speed scales all of them. The
/// values in parentheses are at the default 15 in/s (`ℓ_full` = 1.5 in).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "config", serde(default))]
pub struct Thresholds {
    /// Trailing window the speed is measured over, ms.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::speed_window_ms")
    )]
    pub speed_window_ms: u32,
    /// Silence after which a report starts a new speed series, ms.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::gap_ms")
    )]
    pub gap_ms: u32,
    /// Speed that starts a backswing (1 in/s).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::v_start")
    )]
    pub v_start: f64,
    /// Speed below which the ball counts as stopped (0.5 in/s).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::v_stop")
    )]
    pub v_stop: f64,
    /// Speed a forward stroke must reach to be a swing (2 in/s).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::v_min_swing")
    )]
    pub v_min_swing: f64,
    /// How long the ball must stay below `v_stop` to count as stopped, ms.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::dwell_min_ms")
    )]
    pub dwell_min_ms: u32,
    /// Shortest backswing (0.2 in).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::back_min")
    )]
    pub back_min: f64,
    /// Backswing length that counts as fully cocked (1 in).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::full_cock")
    )]
    pub full_cock: f64,
    /// Forward travel that makes a forward motion a stroke (0.25 in).
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::fwd_min")
    )]
    pub fwd_min: f64,
    /// Longest backswing, s.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::back_max_s")
    )]
    pub back_max_s: f64,
    /// How long a coiled backswing waits for the forward stroke, s.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::coil_timeout_s")
    )]
    pub coil_timeout_s: f64,
    /// Longest forward stroke, ms.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::fwd_window_ms")
    )]
    pub fwd_window_ms: u32,
    /// Quiet time that ends the cooldown after a shot, ms.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::idle_settle_ms")
    )]
    pub idle_settle_ms: u32,
    /// Longest cooldown after a shot, s.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::cooldown_max_s")
    )]
    pub cooldown_max_s: f64,
    /// Widest angle from straight back that still counts as a pull, degrees.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::thresholds::pull_cone_deg")
    )]
    pub pull_cone_deg: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            speed_window_ms: 24,
            gap_ms: 50,
            v_start: 0.067,
            v_stop: 0.033,
            v_min_swing: 0.133,
            dwell_min_ms: 40,
            back_min: 0.13,
            full_cock: 0.67,
            fwd_min: 0.17,
            back_max_s: 2.0,
            coil_timeout_s: 10.0,
            fwd_window_ms: 300,
            idle_settle_ms: 250,
            cooldown_max_s: 2.0,
            pull_cone_deg: 75.0,
        }
    }
}

/// Device configuration. Changes need the input reopened, except the
/// thresholds, which the segmenter reads on every report.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "config", serde(default))]
pub struct Config {
    /// Counts per inch of ball surface travel.
    #[cfg_attr(feature = "config", serde(skip_serializing_if = "skip::config::cpi"))]
    pub cpi: f64,
    /// Largest `|dx|`/`|dy|` one report can carry: 127 for 8-bit reports,
    /// 2047 for 12-bit, 32767 for 16-bit.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "skip::config::report_limit")
    )]
    pub report_limit: i32,
    /// Hold an exclusive grab so the desktop cursor does not follow the ball.
    #[cfg_attr(feature = "config", serde(skip_serializing_if = "skip::config::grab"))]
    pub grab: bool,
    /// Stroke segmentation thresholds.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "flightrelay::config::is_default")
    )]
    pub thresholds: Thresholds,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cpi: 400.0,
            report_limit: 127,
            grab: true,
            thresholds: Thresholds::default(),
        }
    }
}

/// Everything a thumbswing device runs with: which input to read, the club
/// and hand it starts with, its [`Config`] and the [`Tuning`].
///
/// This is the driver's config (the `driver` feature). With the `config`
/// feature it is a serde struct whose keys are `device`, `club`, `hand`, the
/// [`Config`] fields at the top level (`cpi`, `report_limit`, `grab`, a
/// `thresholds` table) and a `tuning` table, e.g. in TOML:
///
/// ```toml
/// device = "usb-Example_Trackball-event-mouse"
/// club = "DR"
/// cpi = 800
///
/// [tuning]
/// full_power_speed = 40
/// curve_sense = "physical"
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "config", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "config", serde(default))]
pub struct Settings {
    /// The input to read: a `/dev/input` path, a `/dev/input/by-id` name, or
    /// the name the device reports. Unset (or blank): the only readable
    /// trackball or mouse; with more than one, opening fails and lists them.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "flightrelay::config::is_default")
    )]
    pub device: Option<String>,
    /// The club in play while the turn names none, as an FRP club code
    /// (`DR`, `7I`, `PW`, `PT`, …; codes without a row of their own map to
    /// the nearest row). The club of a `golfer.turn` takes its place.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "flightrelay::config::is_default")
    )]
    pub club: Club,
    /// The player's hand while the turn names none: `rh` or `lh`. The
    /// handedness of a `golfer.turn` takes its place.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "flightrelay::config::is_default")
    )]
    pub hand: Hand,
    /// CPI, report limit, grab and the segmentation thresholds.
    #[cfg_attr(feature = "config", serde(flatten))]
    pub config: Config,
    /// The tuning knobs.
    #[cfg_attr(
        feature = "config",
        serde(skip_serializing_if = "flightrelay::config::is_default")
    )]
    pub tuning: Tuning,
}

impl Settings {
    /// The `device` value, or `None` when it is unset or blank.
    #[must_use]
    pub fn device(&self) -> Option<&str> {
        self.device
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
    }

    /// Range and consistency checks of every field.
    ///
    /// # Errors
    /// The first field out of range, named by its key (`cpi`,
    /// `thresholds.v_stop`, `tuning.power_gamma`, …).
    pub fn validate(&self) -> Result<(), InvalidSetting> {
        self.config.validate()?;
        self.tuning.validate()
    }

    /// Whether moving from `self` to `new` needs the input reopened: a change
    /// of `device`, `cpi`, `report_limit` or `grab`. The club, the hand, the
    /// thresholds and the tuning apply to a running session.
    #[must_use]
    pub fn restart_needed(&self, new: &Settings) -> bool {
        self.device() != new.device()
            || self.config.cpi.to_bits() != new.config.cpi.to_bits()
            || self.config.report_limit != new.config.report_limit
            || self.config.grab != new.config.grab
    }
}

/// A setting out of range, from [`Settings::validate`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {message}")]
pub struct InvalidSetting {
    /// The key of the field, dotted for nested fields
    /// (`thresholds.v_stop`).
    pub field: &'static str,
    /// What is wrong, e.g. `must be greater than 0`.
    pub message: String,
}

fn invalid(field: &'static str, message: impl Into<String>) -> InvalidSetting {
    InvalidSetting {
        field,
        message: message.into(),
    }
}

fn positive(field: &'static str, v: f64) -> Result<(), InvalidSetting> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(invalid(field, "must be greater than 0"))
    }
}

fn non_negative(field: &'static str, v: f64) -> Result<(), InvalidSetting> {
    if v.is_finite() && v >= 0.0 {
        Ok(())
    } else {
        Err(invalid(field, "must be 0 or more"))
    }
}

fn between(field: &'static str, v: f64, lo: f64, hi: f64) -> Result<(), InvalidSetting> {
    if (lo..=hi).contains(&v) {
        Ok(())
    } else {
        Err(invalid(field, format!("must be {lo} to {hi}")))
    }
}

fn at_least(field: &'static str, v: u32, min: u32) -> Result<(), InvalidSetting> {
    if v >= min {
        Ok(())
    } else {
        Err(invalid(field, format!("must be at least {min}")))
    }
}

impl Config {
    /// Range checks of the CPI, the report limit and the thresholds, and
    /// `v_stop` below `v_start`.
    ///
    /// # Errors
    /// The first field out of range.
    pub fn validate(&self) -> Result<(), InvalidSetting> {
        positive("cpi", self.cpi)?;
        if self.report_limit < 1 {
            return Err(invalid("report_limit", "must be at least 1"));
        }
        let t = &self.thresholds;
        at_least("thresholds.speed_window_ms", t.speed_window_ms, 1)?;
        at_least("thresholds.gap_ms", t.gap_ms, 1)?;
        positive("thresholds.v_start", t.v_start)?;
        positive("thresholds.v_stop", t.v_stop)?;
        positive("thresholds.v_min_swing", t.v_min_swing)?;
        non_negative("thresholds.back_min", t.back_min)?;
        positive("thresholds.full_cock", t.full_cock)?;
        positive("thresholds.fwd_min", t.fwd_min)?;
        positive("thresholds.back_max_s", t.back_max_s)?;
        positive("thresholds.coil_timeout_s", t.coil_timeout_s)?;
        at_least("thresholds.fwd_window_ms", t.fwd_window_ms, 1)?;
        positive("thresholds.cooldown_max_s", t.cooldown_max_s)?;
        between("thresholds.pull_cone_deg", t.pull_cone_deg, 1.0, 180.0)?;
        if t.v_stop >= t.v_start {
            return Err(invalid(
                "thresholds.v_stop",
                format!("must be below thresholds.v_start ({})", t.v_start),
            ));
        }
        Ok(())
    }
}

impl Tuning {
    /// Range checks of every knob.
    ///
    /// # Errors
    /// The first field out of range.
    pub fn validate(&self) -> Result<(), InvalidSetting> {
        positive("tuning.full_power_speed", self.full_power_speed)?;
        positive("tuning.max_club_speed", self.max_club_speed)?;
        positive("tuning.max_putt_speed", self.max_putt_speed)?;
        non_negative("tuning.angle_gain", self.angle_gain)?;
        non_negative("tuning.putt_angle_gain", self.putt_angle_gain)?;
        non_negative("tuning.curve_gain", self.curve_gain)?;
        between("tuning.backswing_power", self.backswing_power, 0.0, 1.0)?;
        positive("tuning.power_gamma", self.power_gamma)?;
        non_negative("tuning.face_max", self.face_max)?;
        non_negative("tuning.face_to_path_max", self.face_to_path_max)
    }
}

#[cfg(feature = "config")]
impl From<InvalidSetting> for flightrelay::config::ConfigError {
    fn from(e: InvalidSetting) -> Self {
        Self::field(e.field, e.message)
    }
}

#[cfg(feature = "config")]
impl flightrelay::config::DriverConfig for Settings {
    fn validate(&self) -> Result<(), flightrelay::config::ConfigError> {
        Ok(Settings::validate(self)?)
    }

    fn restart_needed(old: &Self, new: &Self) -> bool {
        old.restart_needed(new)
    }

    fn example() -> Self {
        Self {
            device: Some(String::new()),
            ..Self::default()
        }
    }
}

/// `skip_serializing_if` predicates: a field is left out while it holds its
/// struct's default.
#[cfg(feature = "config")]
mod skip {
    macro_rules! at_default {
        ($ty:ident { $($field:ident: $t:ty),* $(,)? }) => {
            $(
                #[allow(clippy::trivially_copy_pass_by_ref)] // `skip_serializing_if` takes a reference
                pub(crate) fn $field(v: &$t) -> bool {
                    flightrelay::config::is_default_value(v, &crate::config::$ty::default().$field)
                }
            )*
        };
    }

    pub(crate) mod config {
        at_default!(Config {
            cpi: f64,
            report_limit: i32,
            grab: bool
        });
    }

    pub(crate) mod thresholds {
        at_default!(Thresholds {
            speed_window_ms: u32,
            gap_ms: u32,
            v_start: f64,
            v_stop: f64,
            v_min_swing: f64,
            dwell_min_ms: u32,
            back_min: f64,
            full_cock: f64,
            fwd_min: f64,
            back_max_s: f64,
            coil_timeout_s: f64,
            fwd_window_ms: u32,
            idle_settle_ms: u32,
            cooldown_max_s: f64,
            pull_cone_deg: f64,
        });
    }

    pub(crate) mod tuning {
        use crate::config::CurveSense;

        at_default!(Tuning {
            full_power_speed: f64,
            max_club_speed: f64,
            max_putt_speed: f64,
            angle_gain: f64,
            putt_angle_gain: f64,
            curve_gain: f64,
            curve_sense: CurveSense,
            backswing_power: f64,
            power_gamma: f64,
            mirror_for_left_hand: bool,
            face_max: f64,
            face_to_path_max: f64,
        });
    }
}
