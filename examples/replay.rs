//! Replay a recorded session through the client and print every swing.
//!
//! ```sh
//! cargo run --example replay -- tests/fixtures/straight_draw_fade_125hz.jsonl [--club DR] [--realtime] [--physical]
//! ```

#[path = "common/session.rs"]
mod session;

use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use session::Args;
use thumbswing::recording::{Pace, Recording, Replay};
use thumbswing::{Client, Club, Config, CurveSense, Event, Tuning};

fn main() -> ExitCode {
    let args = match Args::parse(&["--club"]) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("replay: {e}");
            return ExitCode::FAILURE;
        }
    };
    let Some(path) = args.positional.first() else {
        eprintln!("usage: replay <fixture.jsonl> [--club 7I] [--realtime] [--physical]");
        return ExitCode::FAILURE;
    };
    let rec = match Recording::load(path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("replay: {e}");
            return ExitCode::FAILURE;
        }
    };
    let club = match args.value("--club").map(str::parse::<Club>).transpose() {
        Ok(c) => c.unwrap_or_default(),
        Err(e) => {
            eprintln!("replay: {e}");
            return ExitCode::FAILURE;
        }
    };
    let pace = if args.flag("--realtime") {
        Pace::RealTime
    } else {
        Pace::Fast
    };
    let config = Config {
        cpi: rec.header.cpi,
        report_limit: rec.header.report_limit,
        ..Config::default()
    };
    let tuning = Tuning {
        curve_sense: if args.flag("--physical") {
            CurveSense::Physical
        } else {
            CurveSense::Arcade
        },
        ..Tuning::default()
    };
    println!(
        "{}: {} reports from \"{}\" at {} CPI",
        path,
        rec.samples().count(),
        rec.header.device,
        rec.header.cpi
    );
    for (t, note) in rec.marks() {
        #[allow(clippy::cast_precision_loss)]
        let secs = t as f64 / 1e6;
        println!("  mark at {secs:.2} s: {note}");
    }

    let mut client = Client::with_tuning(Replay::new(&rec, pace), config, tuning);
    client.set_club(club);
    while !client.input().finished() {
        match client.poll() {
            Ok(Some(e)) => {
                if let Some(line) = session::event_line(&e) {
                    println!("{line}");
                }
                if let Event::Shot(s) = &e {
                    let (w, d) = (&s.swing, &s.delivery);
                    println!(
                        "  back {:+.1}° {:.2} in, fwd {:+.1}° peak {:.1} in/s → {:.0} % {:.1} mph, path {:+.1} face {:+.1} f2p {:+.1}",
                        w.back_dir_deg,
                        w.back_len_in,
                        w.fwd_dir_deg,
                        w.fwd_peak_in_s,
                        s.power * 100.0,
                        d.club_speed_mph,
                        d.path_deg,
                        d.face_deg,
                        d.face_to_path_deg()
                    );
                }
            }
            Ok(None) => {
                if pace == Pace::RealTime {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            Err(e) => {
                eprintln!("replay: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}
