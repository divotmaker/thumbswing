//! Swing monitor: prints every swing and shot, with a status line of the
//! settings in use.
//!
//! ```sh
//! cargo run --example monitor -- [--device DEVICE] [--club 7I] [--hand RH] [settings…]
//! cargo run --example monitor -- --replay tests/fixtures/straight_draw_fade_125hz.jsonl --curve-gain 0.3
//! cargo run --example monitor -- --help        # every settings flag with its unit and default
//! ```
//!
//! Without `--device` the only readable trackball or mouse is used; with more
//! than one it lists them and asks for `--device`. Each settings flag is a
//! `Settings` field (`--full-power-speed` is `tuning.full_power_speed`, which
//! `thumbswing-frp` sets with `--set`). Ctrl-C quits.

#[path = "common/cli.rs"]
mod cli;
#[path = "common/devices.rs"]
mod devices;
#[path = "common/session.rs"]
mod session;
#[path = "common/term.rs"]
mod term;

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use cli::{Opt, ParseError};
use thumbswing::recording::{Pace, Recording, Replay};
use thumbswing::{Client, Club, Event, Input};

const USAGE: &str = "usage: monitor [--device DEVICE | --replay FILE] [--club CODE] [--hand RH|LH] \
                     [settings…]";

const OPTS: &[Opt] = &[
    Opt {
        name: "--device",
        meta: Some("DEVICE"),
        help: "trackball to read: a /dev/input path, a /dev/input/by-id name or the name the \
               device reports (needed when more than one is found)",
    },
    Opt {
        name: "--replay",
        meta: Some("FILE"),
        help: "replay a recording in real time instead of reading a device",
    },
    Opt {
        name: "--club",
        meta: Some("CODE"),
        help: "club for every shot, an FRP club code [default: 7I]",
    },
    Opt {
        name: "--hand",
        meta: Some("RH|LH"),
        help: "the player's hand [default: RH]",
    },
];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("monitor: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = match cli::parse(std::env::args().skip(1), OPTS) {
        Ok(a) => a,
        Err(ParseError::Help) => {
            print!("{}", cli::help(USAGE, OPTS));
            return Ok(());
        }
        Err(ParseError::Invalid(e)) => return Err(e),
    };
    if let Some(p) = args.positional.first() {
        return Err(format!(
            "unexpected argument {p:?} (--help lists the options)"
        ));
    }
    let club = args
        .value("--club")
        .map(str::parse::<Club>)
        .transpose()
        .map_err(|e| e.to_string())?;
    let hand = args.value("--hand").map(cli::parse_hand).transpose()?;
    let replay = args.value("--replay").map(str::to_owned);
    if replay.is_some() && args.flag("--device") {
        return Err("--device and --replay cannot be used together".to_owned());
    }

    let mut config = args.settings.config.clone();
    let input: Box<dyn Input> = if let Some(path) = &replay {
        let rec = Recording::load(path).map_err(|e| e.to_string())?;
        if !args.given("--cpi") {
            config.cpi = rec.header.cpi;
        }
        if !args.given("--report-limit") {
            config.report_limit = rec.header.report_limit;
        }
        config.grab = false;
        Box::new(Replay::new(&rec, Pace::RealTime))
    } else {
        Box::new(devices::open(args.value("--device"), config.grab)?)
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

    let mut client = Client::with_tuning(input, config, args.settings.tuning);
    if let Some(club) = club {
        client.set_club(club);
    }
    if let Some(hand) = hand {
        client.set_hand(hand);
    }
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
