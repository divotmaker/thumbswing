//! thumbswing-frp — a Flight Relay Protocol device for a trackball swing.
//!
//! Reads a trackball (or any mouse), turns each swing into a shot, and serves
//! the shots to FRP controllers (a simulator) on port 5881. The club and the
//! player's hand are the `club` and `hand` settings (the 7-iron and
//! right-handed by default) until a controller's `set_detection_mode` names
//! others.
//!
//! ```sh
//! thumbswing-frp                                           # the only trackball found, 0.0.0.0:5881
//! thumbswing-frp --set club=DR --set hand=lh               # start with the driver, left-handed
//! thumbswing-frp --set device=usb-…-event-mouse --set tuning.full_power_speed=40
//! thumbswing-frp --config trackball.toml --target ws://flighthook:5880/frp
//! thumbswing-frp --replay session.jsonl --fast --target 127.0.0.1:5882
//! thumbswing-frp --help                                    # every setting with its default
//! ```
//!
//! The flags are flightrelay's `run` flags (`--config`, `--target`, `--set`,
//! `--log`, `--help`); every [`thumbswing::Settings`] field is a `--set` key.
//! Development only: `--replay FILE` serves a recording instead of a device,
//! starting when the first controller connects; `--fast` replays it as fast
//! as it can be processed instead of at the recorded timing,
//! `--exit-after-replay` quits when it is done. Ctrl-C quits.

use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use flightrelay::config::DriverConfig;
use flightrelay::driver::{DeviceEvent, Level, Stop};
use flightrelay::{FrpEvent, FrpServer, RunOptions, ServerConfig, ServerEvent, Transport};
use thumbswing::driver::{DEFAULT_TARGET, Thumbswing};
use thumbswing::recording::{Pace, Recording, Replay};
use thumbswing::{DeviceInfo, Input, Sample, Settings};

const PROGRAM: &str = "thumbswing-frp";

const DEV_HELP: &str = "\nDevelopment:\n\
    \x20 --replay <file>        serve a recording instead of a device; it starts when the first\n\
    \x20                        controller connects (cpi and report_limit default to its header)\n\
    \x20 --fast                 replay as fast as it can be processed, not at the recorded timing\n\
    \x20 --exit-after-replay    quit when the recording has been played";

fn main() -> ExitCode {
    let dev = match DevFlags::strip(std::env::args().skip(1)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{PROGRAM}: {e}");
            return ExitCode::from(2);
        }
    };
    if dev.rest.iter().any(|a| a == "--help" || a == "-h") {
        let code = flightrelay::run_args::<Thumbswing>(std::iter::once("--help".into()), options());
        println!("{DEV_HELP}");
        return code;
    }
    match dev.replay {
        None if dev.fast || dev.exit_after_replay => {
            eprintln!("{PROGRAM}: --fast and --exit-after-replay need --replay");
            ExitCode::from(2)
        }
        None => flightrelay::run_args::<Thumbswing>(dev.rest.into_iter(), options()),
        Some(path) => ExitCode::from(serve_replay(
            &path,
            dev.fast,
            dev.exit_after_replay,
            &dev.rest,
        )),
    }
}

fn options() -> RunOptions<Thumbswing> {
    RunOptions {
        default_target: DEFAULT_TARGET,
        program: Some(PROGRAM),
        ..RunOptions::default()
    }
}

/// The development flags, taken out of the command line; `rest` is what is
/// left for the `run` flags.
#[derive(Debug, Default, PartialEq)]
struct DevFlags {
    replay: Option<String>,
    fast: bool,
    exit_after_replay: bool,
    rest: Vec<String>,
}

impl DevFlags {
    fn strip(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut flags = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--replay" => {
                    flags.replay = Some(args.next().ok_or("--replay needs a file")?);
                }
                "--fast" => flags.fast = true,
                "--exit-after-replay" => flags.exit_after_replay = true,
                _ => {
                    if let Some(file) = arg.strip_prefix("--replay=") {
                        flags.replay = Some(file.to_owned());
                        continue;
                    }
                    flags.rest.push(arg.clone());
                    // A value of a `run` flag is passed along as it is, even
                    // when it looks like a development flag.
                    if matches!(arg.as_str(), "--config" | "--target" | "--set" | "--log")
                        && let Some(value) = args.next()
                    {
                        flags.rest.push(value);
                    }
                }
            }
        }
        Ok(flags)
    }
}

/// The `run` flags of the replay mode, which runs the server itself.
#[derive(Debug, Default)]
struct RunFlags {
    config: Option<String>,
    target: Option<String>,
    sets: Vec<String>,
    log: Option<Level>,
}

impl RunFlags {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut flags = Self::default();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f, Some(v.to_owned())),
                _ => (arg.as_str(), None),
            };
            let mut value = || {
                inline
                    .clone()
                    .or_else(|| it.next().cloned())
                    .ok_or_else(|| format!("{flag} needs a value"))
            };
            match flag {
                "--config" => flags.config = Some(value()?),
                "--target" => flags.target = Some(value()?),
                "--set" => flags.sets.push(value()?),
                "--log" => {
                    flags.log = Some(match value()?.as_str() {
                        "info" => Level::Info,
                        "debug" => Level::Debug,
                        "trace" => Level::Trace,
                        other => {
                            return Err(format!("--log: {other:?} is not info, debug or trace"));
                        }
                    });
                }
                other => return Err(format!("unknown argument {other:?} (--help lists them)")),
            }
        }
        Ok(flags)
    }
}

/// The settings: `base`, then the `--config` file, then each `--set`, then
/// reading and validation with the key named, the way `run` resolves them.
fn resolve(base: &Settings, flags: &RunFlags) -> Result<Settings, String> {
    let mut table = match toml::Value::try_from(base) {
        Ok(toml::Value::Table(t)) => t,
        Ok(_) => toml::Table::new(),
        Err(e) => return Err(e.to_string()),
    };
    if let Some(path) = &flags.config {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let file: toml::Table =
            toml::from_str(&text).map_err(|e| format!("{path}: {}", e.message()))?;
        merge(&mut table, file);
    }
    let known: toml::Table = toml::from_str(&Settings::help_toml()).unwrap_or_default();
    for set in &flags.sets {
        let (key, raw) = set
            .split_once('=')
            .ok_or_else(|| format!("--set {set:?}: expected key=value"))?;
        // `device` is the one key `--help` lists as unset (a comment).
        if key != "device" && !has_path(&known, key) {
            return Err(format!("{key}: unknown key"));
        }
        set_path(&mut table, key, value_of(raw))?;
    }
    let text = toml::to_string(&table).map_err(|e| e.to_string())?;
    Settings::from_toml_str(&text).map_err(|e| e.to_string())
}

/// A `--set` value: a TOML value when it parses as one, else the text.
fn value_of(raw: &str) -> toml::Value {
    toml::from_str::<toml::Table>(&format!("v = {raw}"))
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| toml::Value::String(raw.to_owned()))
}

fn merge(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn has_path(table: &toml::Table, path: &str) -> bool {
    let mut parts = path.split('.').peekable();
    let mut node = table;
    while let Some(part) = parts.next() {
        match (node.get(part), parts.peek()) {
            (Some(_), None) => return true,
            (Some(toml::Value::Table(inner)), Some(_)) => node = inner,
            _ => return false,
        }
    }
    false
}

fn set_path(table: &mut toml::Table, path: &str, value: toml::Value) -> Result<(), String> {
    let mut parts: Vec<&str> = path.split('.').collect();
    let last = parts
        .pop()
        .filter(|l| !l.is_empty())
        .ok_or("--set: empty key")?;
    let mut node = table;
    for part in parts {
        node = node
            .entry(part)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()))
            .as_table_mut()
            .ok_or_else(|| format!("{path}: {part} is not a table"))?;
    }
    node.insert(last.to_owned(), value);
    Ok(())
}

/// Serve a recording until Ctrl-C (or its end, with `exit_after_replay`).
/// Returns the exit code: 0 after a stop, 1 when serving failed, 2 for a bad
/// command line.
fn serve_replay(path: &str, fast: bool, exit_after_replay: bool, rest: &[String]) -> u8 {
    let fail = |e: &dyn std::fmt::Display, code: u8| {
        eprintln!("{PROGRAM}: {e}");
        code
    };
    let flags = match RunFlags::parse(rest) {
        Ok(f) => f,
        Err(e) => return fail(&e, 2),
    };
    let rec = match Recording::load(path) {
        Ok(r) => r,
        Err(e) => return fail(&e, 2),
    };
    let mut base = Settings::default();
    base.config.cpi = rec.header.cpi;
    base.config.report_limit = rec.header.report_limit;
    base.config.grab = false;
    let settings = match resolve(&base, &flags) {
        Ok(s) => s,
        Err(e) => return fail(&e, 2),
    };
    let target = flags.target.as_deref().unwrap_or(DEFAULT_TARGET).to_owned();
    if target
        .get(..6)
        .is_some_and(|s| s.eq_ignore_ascii_case("wss://"))
    {
        return fail(
            &format!("--target {target:?}: wss:// is not supported; use ws://"),
            2,
        );
    }

    let stop = Stop::new();
    let handler = stop.clone();
    if let Err(e) = ctrlc::set_handler(move || handler.stop()) {
        eprintln!("{PROGRAM}: cannot install the Ctrl-C handler: {e}");
    }
    let open = Arc::new(AtomicBool::new(false));
    let pace = if fast { Pace::Fast } else { Pace::RealTime };
    let input = GatedReplay {
        frozen_us: rec.samples().next().map_or(0, |s| s.t_us),
        replay: Replay::new(&rec, pace),
        open: Arc::clone(&open),
        stop_when_done: exit_after_replay.then(|| stop.clone()),
    };
    let driver = match Thumbswing::with_input(settings, input) {
        Ok(d) => d,
        Err(e) => return fail(&e, 2),
    };
    let transport = if target.starts_with("ws://") {
        Transport::connect(target)
    } else {
        Transport::listen(target)
    };
    let log = flags.log.unwrap_or(Level::Info);
    let mut config = ServerConfig::new(transport);
    config.log_level = log;
    eprintln!("{PROGRAM}: replaying {path} once a controller connects");
    let result = FrpServer::new(driver, config)
        .on_event(move |event| {
            if matches!(event, ServerEvent::ControllerConnected { .. }) {
                open.store(true, Ordering::SeqCst);
            }
            if let Some(line) = describe(event, log) {
                eprintln!("{PROGRAM}: {line}");
            }
        })
        .run(&stop);
    match result {
        Ok(()) => 0,
        Err(e) => fail(&e, 1),
    }
}

/// One output line for a server event, or none: the lines `run` prints.
fn describe(event: &ServerEvent<'_>, log: Level) -> Option<String> {
    let name = |peer: Option<&str>| peer.map_or(String::new(), |p| format!(" ({p})"));
    Some(match event {
        ServerEvent::Listening(addr) => format!("listening on {addr}"),
        ServerEvent::ControllerConnected { peer } => format!("controller connected{}", name(*peer)),
        ServerEvent::ControllerLeft { peer, reason } => {
            format!("controller left{}: {reason}", name(*peer))
        }
        ServerEvent::DeviceConnected(identity) => format!("device connected: {}", identity.label()),
        ServerEvent::DeviceLost { error, retry_in } => {
            let why = error.map_or_else(|| "stopped".to_owned(), ToString::to_string);
            match retry_in {
                Some(d) => format!("device lost: {why}; retrying in {}", delay_text(*d)),
                None => format!("device lost: {why}; giving up"),
            }
        }
        ServerEvent::Command(cmd) => format!(
            "command: mode={} hand={} club={}",
            cmd.mode.map_or("-".into(), |m| m.to_string()),
            cmd.handed.map_or("-".into(), |h| h.to_string()),
            cmd.club.map_or("-".into(), |c| c.to_string()),
        ),
        ServerEvent::Device(DeviceEvent::Log(level, line)) => format!("{level}: {line}"),
        ServerEvent::Device(DeviceEvent::Phase(phase)) if log <= Level::Debug => {
            format!("phase: {phase}")
        }
        ServerEvent::Device(DeviceEvent::Frp(FrpEvent::Launch { key, launch })) => format!(
            "shot {}: ball {:.1} mph, launch {:.1} deg, spin {} rpm",
            key.shot_number,
            launch
                .launch_speed
                .map_or(0.0, flightrelay::Velocity::as_mph),
            launch.launch_elevation.unwrap_or_default(),
            launch.total_spin_rpm.unwrap_or_default()
        ),
        ServerEvent::Device(DeviceEvent::Frp(FrpEvent::Alert { severity, message })) => {
            format!("alert ({severity}): {message}")
        }
        _ => return None,
    })
}

/// A delay as `run` prints it: whole seconds, else milliseconds below a
/// second, else tenths of a second.
fn delay_text(d: Duration) -> String {
    let ms = d.as_millis();
    if ms.is_multiple_of(1000) {
        format!("{} s", ms / 1000)
    } else if ms < 1000 {
        format!("{ms} ms")
    } else {
        format!("{:.1} s", d.as_secs_f64())
    }
}

/// A replay that holds still until `open` is set (the first controller), and
/// stops the server when it has been played, if asked to.
struct GatedReplay {
    replay: Replay,
    open: Arc<AtomicBool>,
    /// The clock while closed: the recording's first report.
    frozen_us: u64,
    stop_when_done: Option<Stop>,
}

impl Input for GatedReplay {
    fn try_recv(&mut self) -> Result<Option<Sample>, thumbswing::Error> {
        if !self.open.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if self.replay.finished()
            && let Some(stop) = &self.stop_when_done
        {
            stop.stop();
        }
        self.replay.try_recv()
    }

    fn now_us(&mut self) -> u64 {
        if self.open.load(Ordering::SeqCst) {
            self.replay.now_us()
        } else {
            self.frozen_us
        }
    }

    fn info(&self) -> &DeviceInfo {
        self.replay.info()
    }

    fn paced(&self) -> bool {
        !self.open.load(Ordering::SeqCst) || self.replay.paced()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|a| (*a).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn development_flags_are_taken_out() {
        let flags = DevFlags::strip(args(&[
            "--set",
            "cpi=800",
            "--replay",
            "s.jsonl",
            "--fast",
            "--target",
            "127.0.0.1:5990",
        ]))
        .unwrap();
        assert_eq!(flags.replay.as_deref(), Some("s.jsonl"));
        assert!(flags.fast && !flags.exit_after_replay);
        assert_eq!(
            flags.rest,
            ["--set", "cpi=800", "--target", "127.0.0.1:5990"]
        );
        let flags = DevFlags::strip(args(&["--replay=a.jsonl", "--exit-after-replay"])).unwrap();
        assert_eq!(flags.replay.as_deref(), Some("a.jsonl"));
        assert!(flags.exit_after_replay);
        assert!(DevFlags::strip(args(&["--replay"])).is_err());
    }

    #[test]
    fn replay_settings_resolve_like_run() {
        let mut base = Settings::default();
        base.config.cpi = 800.0;
        base.config.grab = false;
        let flags = RunFlags::parse(&[
            "--set".into(),
            "tuning.curve_sense=physical".into(),
            "--set=thresholds.gap_ms=60".into(),
            "--set".into(),
            "device=usb-ball".into(),
            "--set".into(),
            "club=PW".into(),
            "--set=hand=lh".into(),
        ])
        .unwrap();
        let s = resolve(&base, &flags).unwrap();
        assert!(
            (s.config.cpi - 800.0).abs() < f64::EPSILON,
            "the base stays"
        );
        assert!(!s.config.grab);
        assert_eq!(s.tuning.curve_sense, thumbswing::CurveSense::Physical);
        assert_eq!(s.config.thresholds.gap_ms, 60);
        assert_eq!(s.device(), Some("usb-ball"));
        assert_eq!(s.club, thumbswing::Club::PitchingWedge);
        assert_eq!(s.hand, thumbswing::Hand::Left);

        let bad = |set: &str| {
            resolve(
                &base,
                &RunFlags::parse(&["--set".into(), set.into()]).unwrap(),
            )
            .unwrap_err()
        };
        assert_eq!(bad("tuning.nope=1"), "tuning.nope: unknown key");
        assert_eq!(
            bad("tuning.power_gamma=0"),
            "tuning.power_gamma: must be greater than 0"
        );
        assert!(
            bad("cpi=abc").starts_with("cpi: invalid type"),
            "{}",
            bad("cpi=abc")
        );
        assert!(bad("club=ZZ").starts_with("club: "), "{}", bad("club=ZZ"));
        assert!(RunFlags::parse(&["--bogus".into()]).is_err());
    }
}
