//! Command-line settings of `examples/monitor`:
//! one flag per [`Config`] and [`Tuning`] field over the crate defaults, plus
//! each program's own options.

#![allow(dead_code)]

use std::fmt::Write as _;

use thumbswing::{Config, CurveSense, Hand, Tuning};

/// The settings a program starts the client with.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub config: Config,
    pub tuning: Tuning,
}

/// The values a number may take.
#[derive(Debug, Clone, Copy)]
enum Range {
    Positive,
    NonNegative,
    Between(f64, f64),
}

impl Range {
    fn check(self, v: f64) -> Result<(), String> {
        match self {
            Range::Positive if v <= 0.0 => Err("must be greater than 0".to_owned()),
            Range::NonNegative if v < 0.0 => Err("must be 0 or more".to_owned()),
            Range::Between(lo, hi) if v < lo || v > hi => {
                Err(format!("must be between {lo} and {hi}"))
            }
            _ => Ok(()),
        }
    }
}

/// Where a flag writes, and how its value is read.
enum Field {
    /// A decimal number.
    Real(fn(&mut Settings) -> &mut f64, Range),
    /// A whole number, at least the given minimum.
    Whole(fn(&mut Settings) -> &mut u32, u32),
    /// A positive whole number that fits an `i32`.
    Limit(fn(&mut Settings) -> &mut i32),
    /// `arcade` or `physical`.
    Sense(fn(&mut Settings) -> &mut CurveSense),
    /// Takes no value; its presence sets the field to `false`.
    Off(fn(&mut Settings) -> &mut bool),
    /// Takes no value; its presence sets the field to `true`.
    On(fn(&mut Settings) -> &mut bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Group {
    Device,
    Tuning,
    Detection,
}

/// One settings flag.
struct Knob {
    name: &'static str,
    meta: &'static str,
    unit: &'static str,
    group: Group,
    help: &'static str,
    field: Field,
}

const SPEED: &str = "× full-power speed";
const LENGTH: &str = "× ℓ_full";

#[allow(clippy::too_many_lines)]
fn knobs() -> Vec<Knob> {
    use Field::{Limit, Off, On, Real, Sense, Whole};
    use Group::{Detection, Device, Tuning};
    use Range::{Between, NonNegative, Positive};
    let knob = |name, meta, unit, group, help, field| Knob {
        name,
        meta,
        unit,
        group,
        help,
        field,
    };
    vec![
        knob(
            "--cpi",
            "N",
            "counts/in",
            Device,
            "counts per inch of ball surface travel",
            Real(|s| &mut s.config.cpi, Positive),
        ),
        knob(
            "--report-limit",
            "N",
            "counts",
            Device,
            "largest |dx|/|dy| one report carries: 127 (8-bit), 2047 (12-bit), 32767 (16-bit)",
            Limit(|s| &mut s.config.report_limit),
        ),
        knob(
            "--no-grab",
            "",
            "",
            Device,
            "no exclusive grab: the desktop cursor follows the ball (the grab is on without it)",
            Off(|s| &mut s.config.grab),
        ),
        knob(
            "--full-power-speed",
            "X",
            "in/s",
            Tuning,
            "forward peak speed that counts as 100 % power",
            Real(|s| &mut s.tuning.full_power_speed, Positive),
        ),
        knob(
            "--max-club-speed",
            "X",
            "mph",
            Tuning,
            "driver club speed at full power; other clubs scale it by their speed ratio",
            Real(|s| &mut s.tuning.max_club_speed, Positive),
        ),
        knob(
            "--max-putt-speed",
            "X",
            "mph",
            Tuning,
            "putter club speed at full power",
            Real(|s| &mut s.tuning.max_putt_speed, Positive),
        ),
        knob(
            "--angle-gain",
            "X",
            "°/°",
            Tuning,
            "face angle per degree of push direction",
            Real(|s| &mut s.tuning.angle_gain, NonNegative),
        ),
        knob(
            "--putt-angle-gain",
            "X",
            "°/°",
            Tuning,
            "putt line per degree of push direction",
            Real(|s| &mut s.tuning.putt_angle_gain, NonNegative),
        ),
        knob(
            "--curve-gain",
            "X",
            "°/°",
            Tuning,
            "face-to-path per degree between the backswing line and the push",
            Real(|s| &mut s.tuning.curve_gain, NonNegative),
        ),
        knob(
            "--curve-sense",
            "SENSE",
            "",
            Tuning,
            "how the backswing bends the shot: arcade or physical",
            Sense(|s| &mut s.tuning.curve_sense),
        ),
        knob(
            "--backswing-power",
            "X",
            "",
            Tuning,
            "share of the power set by the backswing length, 0 to 1 (0 = forward speed only)",
            Real(|s| &mut s.tuning.backswing_power, Between(0.0, 1.0)),
        ),
        knob(
            "--power-gamma",
            "X",
            "",
            Tuning,
            "exponent of the power curve (1 = linear)",
            Real(|s| &mut s.tuning.power_gamma, Positive),
        ),
        knob(
            "--mirror-for-left-hand",
            "",
            "",
            Tuning,
            "mirror the stroke directions for a left-handed player",
            On(|s| &mut s.tuning.mirror_for_left_hand),
        ),
        knob(
            "--face-max",
            "X",
            "°",
            Tuning,
            "largest face angle",
            Real(|s| &mut s.tuning.face_max, NonNegative),
        ),
        knob(
            "--face-to-path-max",
            "X",
            "°",
            Tuning,
            "largest face-to-path",
            Real(|s| &mut s.tuning.face_to_path_max, NonNegative),
        ),
        knob(
            "--speed-window-ms",
            "N",
            "ms",
            Detection,
            "trailing window the speed is measured over",
            Whole(|s| &mut s.config.thresholds.speed_window_ms, 1),
        ),
        knob(
            "--gap-ms",
            "N",
            "ms",
            Detection,
            "silence after which a report starts a new speed series",
            Whole(|s| &mut s.config.thresholds.gap_ms, 1),
        ),
        knob(
            "--v-start",
            "X",
            SPEED,
            Detection,
            "speed that starts a backswing",
            Real(|s| &mut s.config.thresholds.v_start, Positive),
        ),
        knob(
            "--v-stop",
            "X",
            SPEED,
            Detection,
            "speed below which the ball counts as stopped (below --v-start)",
            Real(|s| &mut s.config.thresholds.v_stop, Positive),
        ),
        knob(
            "--v-min-swing",
            "X",
            SPEED,
            Detection,
            "speed a forward stroke must reach to be a swing",
            Real(|s| &mut s.config.thresholds.v_min_swing, Positive),
        ),
        knob(
            "--dwell-min-ms",
            "N",
            "ms",
            Detection,
            "how long the ball must stay below --v-stop to count as stopped",
            Whole(|s| &mut s.config.thresholds.dwell_min_ms, 0),
        ),
        knob(
            "--back-min",
            "X",
            LENGTH,
            Detection,
            "shortest backswing",
            Real(|s| &mut s.config.thresholds.back_min, NonNegative),
        ),
        knob(
            "--full-cock",
            "X",
            LENGTH,
            Detection,
            "backswing length that counts as fully cocked",
            Real(|s| &mut s.config.thresholds.full_cock, Positive),
        ),
        knob(
            "--fwd-min",
            "X",
            LENGTH,
            Detection,
            "forward travel that makes a forward motion a stroke",
            Real(|s| &mut s.config.thresholds.fwd_min, Positive),
        ),
        knob(
            "--back-max-s",
            "X",
            "s",
            Detection,
            "longest backswing",
            Real(|s| &mut s.config.thresholds.back_max_s, Positive),
        ),
        knob(
            "--coil-timeout-s",
            "X",
            "s",
            Detection,
            "how long a coiled backswing waits for the forward stroke",
            Real(|s| &mut s.config.thresholds.coil_timeout_s, Positive),
        ),
        knob(
            "--fwd-window-ms",
            "N",
            "ms",
            Detection,
            "longest forward stroke",
            Whole(|s| &mut s.config.thresholds.fwd_window_ms, 1),
        ),
        knob(
            "--idle-settle-ms",
            "N",
            "ms",
            Detection,
            "quiet time that ends the cooldown after a shot",
            Whole(|s| &mut s.config.thresholds.idle_settle_ms, 0),
        ),
        knob(
            "--cooldown-max-s",
            "X",
            "s",
            Detection,
            "longest cooldown after a shot",
            Real(|s| &mut s.config.thresholds.cooldown_max_s, Positive),
        ),
        knob(
            "--pull-cone-deg",
            "X",
            "°",
            Detection,
            "widest angle from straight back that still counts as a pull",
            Real(
                |s| &mut s.config.thresholds.pull_cone_deg,
                Between(1.0, 180.0),
            ),
        ),
    ]
}

/// A program's own option.
pub struct Opt {
    pub name: &'static str,
    /// The value's placeholder; `None` for an option without a value.
    pub meta: Option<&'static str>,
    pub help: &'static str,
}

/// The parsed command line.
#[derive(Debug)]
pub struct Parsed {
    pub settings: Settings,
    pub positional: Vec<String>,
    given: Vec<&'static str>,
    options: Vec<(&'static str, Option<String>)>,
}

impl Parsed {
    /// Whether the settings flag `name` was on the command line.
    pub fn given(&self, name: &str) -> bool {
        self.given.contains(&name)
    }

    /// Whether the program option `name` was on the command line.
    pub fn flag(&self, name: &str) -> bool {
        self.options.iter().any(|(k, _)| *k == name)
    }

    /// The value of the program option `name` (the last one given).
    pub fn value(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .rev()
            .find(|(k, _)| *k == name)
            .and_then(|(_, v)| v.as_deref())
    }
}

/// Why parsing stopped.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    /// `--help` or `-h`.
    Help,
    /// A bad command line; the text says what is wrong.
    Invalid(String),
}

fn invalid(msg: String) -> ParseError {
    ParseError::Invalid(msg)
}

/// Parse `args` (without the program name). Settings flags start from the
/// crate defaults; `opts` are the program's own options. Values follow the
/// flag (`--cpi 800`) or an `=` (`--cpi=800`).
///
/// # Errors
/// [`ParseError::Help`] for `--help`; [`ParseError::Invalid`] for an unknown
/// flag, a missing or malformed value, or a value out of range.
pub fn parse<I>(args: I, opts: &[Opt]) -> Result<Parsed, ParseError>
where
    I: IntoIterator<Item = String>,
{
    let knobs = knobs();
    let mut parsed = Parsed {
        settings: Settings::default(),
        positional: Vec::new(),
        given: Vec::new(),
        options: Vec::new(),
    };
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        if arg == "--help" || arg == "-h" {
            return Err(ParseError::Help);
        }
        if !arg.starts_with("--") {
            parsed.positional.push(arg);
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) => (n.to_owned(), Some(v.to_owned())),
            None => (arg.clone(), None),
        };
        if let Some(k) = knobs.iter().find(|k| k.name == name) {
            let takes_value = !matches!(k.field, Field::Off(_) | Field::On(_));
            let value = take_value(&name, k.meta, takes_value, inline, &mut it)?;
            set(k, value.as_deref(), &mut parsed.settings)?;
            parsed.given.push(k.name);
        } else if let Some(o) = opts.iter().find(|o| o.name == name) {
            let meta = o.meta.unwrap_or("");
            let value = take_value(&name, meta, o.meta.is_some(), inline, &mut it)?;
            parsed.options.push((o.name, value));
        } else {
            return Err(invalid(format!(
                "unknown option {name} (--help lists them)"
            )));
        }
    }
    let th = &parsed.settings.config.thresholds;
    if th.v_stop >= th.v_start {
        return Err(invalid(format!(
            "--v-stop ({}) must be below --v-start ({})",
            th.v_stop, th.v_start
        )));
    }
    Ok(parsed)
}

fn take_value(
    name: &str,
    meta: &str,
    takes_value: bool,
    inline: Option<String>,
    rest: &mut impl Iterator<Item = String>,
) -> Result<Option<String>, ParseError> {
    match (takes_value, inline) {
        (true, Some(v)) => Ok(Some(v)),
        (true, None) => rest
            .next()
            .map(Some)
            .ok_or_else(|| invalid(format!("{name} needs a value ({meta})"))),
        (false, Some(_)) => Err(invalid(format!("{name} takes no value"))),
        (false, None) => Ok(None),
    }
}

fn set(k: &Knob, value: Option<&str>, s: &mut Settings) -> Result<(), ParseError> {
    let name = k.name;
    let v = value.unwrap_or_default();
    let bad = |why: &str| invalid(format!("{name}: {v:?} {why}"));
    match k.field {
        Field::Real(field, range) => {
            let x: f64 = v.parse().map_err(|_| bad("is not a number"))?;
            if !x.is_finite() {
                return Err(bad("is not a finite number"));
            }
            range.check(x).map_err(|why| bad(&why))?;
            *field(s) = x;
        }
        Field::Whole(field, min) => {
            let n: u32 = v.parse().map_err(|_| bad("is not a whole number"))?;
            if n < min {
                return Err(bad(&format!("must be at least {min}")));
            }
            *field(s) = n;
        }
        Field::Limit(field) => {
            let n: i32 = v.parse().map_err(|_| bad("is not a whole number"))?;
            if n < 1 {
                return Err(bad("must be at least 1"));
            }
            *field(s) = n;
        }
        Field::Sense(field) => {
            *field(s) = match v.to_ascii_lowercase().as_str() {
                "arcade" => CurveSense::Arcade,
                "physical" => CurveSense::Physical,
                _ => return Err(bad("is not arcade or physical")),
            };
        }
        Field::Off(field) => *field(s) = false,
        Field::On(field) => *field(s) = true,
    }
    Ok(())
}

fn default_text(k: &Knob, d: &mut Settings) -> String {
    match k.field {
        Field::Real(f, _) => format!("{}", *f(d)),
        Field::Whole(f, _) => f(d).to_string(),
        Field::Limit(f) => f(d).to_string(),
        Field::Sense(f) => f(d).label().to_owned(),
        // A switch is off unless it is given.
        Field::Off(f) => if *f(d) { "off" } else { "on" }.to_owned(),
        Field::On(f) => if *f(d) { "on" } else { "off" }.to_owned(),
    }
}

fn line(out: &mut String, flag: &str, help: &str) {
    if flag.chars().count() < 28 {
        let _ = writeln!(out, "  {flag:<28}{help}");
    } else {
        let _ = writeln!(out, "  {flag}\n  {:<28}{help}", "");
    }
}

/// The `--help` text: `usage`, the program's options, then every settings
/// flag with its unit and default.
pub fn help(usage: &str, opts: &[Opt]) -> String {
    let mut out = format!("{usage}\n\nOptions:\n");
    for o in opts {
        let flag = match o.meta {
            Some(m) => format!("{} {m}", o.name),
            None => o.name.to_owned(),
        };
        line(&mut out, &flag, o.help);
    }
    line(&mut out, "-h, --help", "print this help");
    let mut defaults = Settings::default();
    for (group, title) in [
        (Group::Device, "Device:"),
        (Group::Tuning, "Tuning:"),
        (
            Group::Detection,
            "Swing detection (speeds are fractions of the full-power speed, lengths of \
             ℓ_full, the\nball travel of 100 ms at full power):",
        ),
    ] {
        let _ = write!(out, "\n{title}\n");
        for k in knobs().iter().filter(|k| k.group == group) {
            let flag = if k.meta.is_empty() {
                k.name.to_owned()
            } else {
                format!("{} {}", k.name, k.meta)
            };
            let unit = if k.unit.is_empty() {
                String::new()
            } else {
                format!(" {}", k.unit)
            };
            let text = format!(
                "{} [default: {}{unit}]",
                k.help,
                default_text(k, &mut defaults)
            );
            line(&mut out, &flag, &text);
        }
    }
    out
}

/// Every settings flag name, in `--help` order.
pub fn flag_names() -> Vec<&'static str> {
    knobs().iter().map(|k| k.name).collect()
}

/// A `--hand` value: `RH` or `LH`, any case.
///
/// # Errors
/// Anything else.
pub fn parse_hand(s: &str) -> Result<Hand, String> {
    match s.to_ascii_uppercase().as_str() {
        "RH" => Ok(Hand::Right),
        "LH" => Ok(Hand::Left),
        _ => Err(format!("--hand: {s:?} is not RH or LH")),
    }
}
