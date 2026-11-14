//! Swing monitor: prints every swing and shot, with a status line of the
//! settings in use.
//!
//! ```sh
//! cargo run --example monitor --features config -- [--replay FILE] [key=value]…
//! cargo run --example monitor --features config -- device=usb-…-event-mouse club=DR tuning.full_power_speed=40
//! cargo run --example monitor --features config -- --replay tests/fixtures/straight_draw_fade_125hz.jsonl tuning.curve_gain=0.3
//! ```
//!
//! The settings are the keys of `thumbswing-frp --set` (`thumbswing-frp --help`
//! lists them with their defaults): `device`, `club`, `hand`, `cpi`,
//! `report_limit`, `grab`, `thresholds.*` and `tuning.*`, checked by
//! [`Config::validate`]. Without `device` the only readable trackball or
//! mouse is used; with more than one it lists them and asks for `device`.
//! `--replay FILE` plays a recording in real time instead, with `cpi` and
//! `report_limit` from its header unless given. Ctrl-C quits.

#[path = "common/devices.rs"]
mod devices;
#[path = "common/session.rs"]
mod session;
#[path = "common/term.rs"]
mod term;

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use flightrelay::config::DriverConfig;
use serde_json::{Map, Value};
use thumbswing::recording::{Pace, Recording, Replay};
use thumbswing::{Client, Config, Event, Input};

const USAGE: &str =
    "usage: monitor [--replay FILE] [key=value]…  (the keys of thumbswing-frp --set)";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("monitor: {e}");
            ExitCode::FAILURE
        }
    }
}

/// What the command line asks for.
struct Opts {
    settings: Config,
    /// The keys the command line gave, as a settings table.
    given: Map<String, Value>,
    replay: Option<String>,
}

/// The settings the `key=value` arguments name, and the replay file when
/// `--replay` is given.
fn parse(mut args: impl Iterator<Item = String>) -> Result<Opts, String> {
    let mut given = Map::new();
    let mut replay = None;
    while let Some(arg) = args.next() {
        if arg == "--replay" {
            replay = Some(args.next().ok_or("--replay needs a file")?);
        } else if let Some((key, value)) = arg.split_once('=') {
            // A value is JSON when it parses as JSON, else text.
            let value =
                serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_owned()));
            let mut table = &mut given;
            let mut parts = key.split('.').peekable();
            while let Some(part) = parts.next() {
                if parts.peek().is_none() {
                    table.insert(part.to_owned(), value);
                    break;
                }
                table = table
                    .entry(part)
                    .or_insert_with(|| Value::Object(Map::new()))
                    .as_object_mut()
                    .ok_or_else(|| format!("{key}: {part} is not a table"))?;
            }
        } else {
            return Err(format!("unexpected argument {arg:?}\n{USAGE}"));
        }
    }
    let settings = Config::from_map(given.clone()).map_err(|e| e.to_string())?;
    Ok(Opts {
        settings,
        given,
        replay,
    })
}

fn run() -> Result<(), String> {
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let Opts {
        settings,
        given,
        replay,
    } = parse(std::env::args().skip(1))?;
    if replay.is_some() && settings.device().is_some() {
        return Err("device and --replay cannot be used together".to_owned());
    }

    let mut config = settings.detector.clone();
    let input: Box<dyn Input> = if let Some(path) = &replay {
        let rec = Recording::load(path).map_err(|e| e.to_string())?;
        if !given.contains_key("cpi") {
            config.cpi = rec.header.cpi;
        }
        if !given.contains_key("report_limit") {
            config.report_limit = rec.header.report_limit;
        }
        config.grab = false;
        Box::new(Replay::new(&rec, Pace::RealTime))
    } else {
        Box::new(devices::open(settings.device(), config.grab)?)
    };

    term::install_signal_handlers();
    let mut screen = term::Screen::new();
    let info = input.info().clone();
    screen.log(&format!(
        "{} ({}){}",
        info.name,
        info.path.display(),
        if info.grabbed { ", grabbed" } else { "" }
    ));

    let mut client = Client::with_tuning(input, config, settings.tuning);
    client.set_club(settings.club);
    client.set_hand(settings.hand);
    let mut last_shot = None;
    screen.status(session::status_line(&client, None));

    while !term::quit_requested() {
        match client.poll() {
            Ok(Some(e)) => {
                if let Some(line) = session::event_line(&e) {
                    screen.log(&line);
                }
                if let Event::Shot(s) = e {
                    last_shot = Some(*s);
                }
                screen.status(session::status_line(&client, last_shot.as_ref()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(e) => {
                screen.log(&format!("input error: {e}"));
                break;
            }
        }
    }
    screen.finish();
    Ok(())
}
