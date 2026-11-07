//! Record a session to a JSON Lines fixture.
//!
//! ```sh
//! cargo run --example record -- <device> <out.jsonl> [--cpi 400] [--report-limit 127] [--rate-hz 125] [--no-grab]
//! ```
//!
//! `<device>` is a `/dev/input` path, a `/dev/input/by-id` name or the name
//! the device reports. Every report is written as it arrives. Type a note and press Enter to add
//! a `mark` line (say what the next swing is meant to be: "straight drive",
//! "draw, pull 20° right"); detected swings are printed as feedback. `q` +
//! Enter or Ctrl-C stops and appends a summary line (report rate, largest
//! `|dx|`/`|dy|`), which readers skip.

#[path = "common/session.rs"]
mod session;
#[path = "common/term.rs"]
mod term;

use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use session::Args;
use thumbswing::recording::{Header, Recorder};
use thumbswing::{Client, Config, DeviceInfo, Error, Event, Input, Sample, device};

/// Passes samples through to the client and writes each to the recording.
struct Tee<I: Input, W: Write> {
    inner: I,
    rec: Recorder<W>,
    reports: u64,
    max: (i32, i32),
    dts: Vec<u64>,
    last_t: Option<u64>,
}

impl<I: Input, W: Write> Input for Tee<I, W> {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        let s = self.inner.try_recv()?;
        if let Some(s) = &s {
            self.rec.sample(s)?;
            self.reports += 1;
            self.max = (self.max.0.max(s.dx.abs()), self.max.1.max(s.dy.abs()));
            if let Some(l) = self.last_t {
                let dt = s.t_us.saturating_sub(l);
                if dt < 50_000 {
                    self.dts.push(dt);
                }
            }
            self.last_t = Some(s.t_us);
        }
        Ok(s)
    }

    fn now_us(&mut self) -> u64 {
        self.inner.now_us()
    }

    fn info(&self) -> &DeviceInfo {
        self.inner.info()
    }
}

// The setup and the poll loop read top to bottom.
#[allow(clippy::too_many_lines)]
fn main() -> ExitCode {
    let args = match Args::parse(&["--cpi", "--report-limit", "--rate-hz"]) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("record: {e}");
            return ExitCode::FAILURE;
        }
    };
    let (Some(dev_path), Some(out)) = (args.positional.first(), args.positional.get(1)) else {
        eprintln!(
            "usage: record <device> <out.jsonl> [--cpi N] [--report-limit N] [--rate-hz N] [--no-grab]"
        );
        return ExitCode::FAILURE;
    };
    let cpi = args.number("--cpi").ok().flatten().unwrap_or(400.0);
    let limit = args
        .number("--report-limit")
        .ok()
        .flatten()
        .unwrap_or(127.0);
    let rate = args.number("--rate-hz").ok().flatten().unwrap_or(0.0);
    let grab = !args.flag("--no-grab");

    term::install_signal_handlers();
    let mut dev = match device::open_setting(Some(dev_path), grab) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("record: {dev_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    #[allow(clippy::cast_possible_truncation)]
    let header = Header {
        device: dev.info().name.clone(),
        cpi,
        rate_hz: rate,
        report_limit: limit as i32,
        grab,
        ..Header::default()
    };
    let file = match OpenOptions::new().write(true).create_new(true).open(out) {
        Ok(f) => std::io::LineWriter::new(f),
        Err(e) => {
            eprintln!("record: {out}: {e} (refusing to overwrite)");
            return ExitCode::FAILURE;
        }
    };
    let mut rec = match Recorder::new(file, &header) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("record: {out}: {e}");
            return ExitCode::FAILURE;
        }
    };
    rec.start_at(dev.now_us());
    eprintln!(
        "recording {} to {out}; type a note + Enter to mark the next swing, q + Enter or Ctrl-C to stop",
        header.device
    );

    let (tx, notes) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let config = Config {
        cpi,
        report_limit: header.report_limit,
        grab,
        ..Config::default()
    };
    let tee = Tee {
        inner: dev,
        rec,
        reports: 0,
        max: (0, 0),
        dts: Vec::new(),
        last_t: None,
    };
    let mut client = Client::new(tee, config);
    while !term::quit_requested() {
        match notes.try_recv() {
            Ok(note) if note.trim() == "q" => break,
            Ok(note) if !note.trim().is_empty() => {
                let now = client.input_mut().now_us();
                if let Err(e) = client.input_mut().rec.mark(now, note.trim()) {
                    eprintln!("record: {e}");
                }
                eprintln!("mark: {}", note.trim());
            }
            _ => {}
        }
        match client.poll() {
            Ok(Some(e)) => {
                if let Some(line) = session::event_line(&e) {
                    eprintln!("{line}");
                }
                if let Event::Shot(s) = &e {
                    eprintln!(
                        "  back {:+.1}° {:.2} in, fwd {:+.1}° peak {:.1} in/s",
                        s.swing.back_dir_deg,
                        s.swing.back_len_in,
                        s.swing.fwd_dir_deg,
                        s.swing.fwd_peak_in_s
                    );
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(2)),
            Err(e) => {
                eprintln!("record: {e}");
                break;
            }
        }
    }

    let tee = client.into_input();
    #[allow(clippy::cast_precision_loss)]
    let rate = {
        let mut dts = tee.dts.clone();
        dts.sort_unstable();
        dts.get(dts.len() / 2).map_or(0.0, |m| 1e6 / *m as f64)
    };
    let mut out_w = match tee.rec.finish() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("record: {e}");
            return ExitCode::FAILURE;
        }
    };
    let _ = writeln!(
        out_w,
        "{{\"kind\":\"summary\",\"reports\":{},\"rate_hz\":{rate:.1},\"max_abs_dx\":{},\"max_abs_dy\":{}}}",
        tee.reports, tee.max.0, tee.max.1
    );
    eprintln!(
        "wrote {} reports to {out} (about {rate:.0} Hz, largest |dx| {} |dy| {})",
        tee.reports, tee.max.0, tee.max.1
    );
    ExitCode::SUCCESS
}
