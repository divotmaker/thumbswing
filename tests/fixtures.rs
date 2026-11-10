//! The recorded-session fixtures in `tests/fixtures/`, replayed through the
//! client and checked against their `*.expect.json` sidecars.
//!
//! Both fixtures are generated synthetically by `thumbswing::testing` (their
//! headers say so); `cargo test --features testing --test fixtures --
//! --ignored regenerate` rewrites them. Recordings from real devices go next
//! to them with hand-written sidecars.

use std::path::{Path, PathBuf};

use thumbswing::recording::{Pace, Recording, Replay};
use thumbswing::testing::{Expectations, StreamBuilder, jitter, synthetic_recording};
use thumbswing::{CancelReason, Client, Config, Event, Sample, Swing};

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Replay a recording through a client configured from its header.
fn replay(rec: &Recording) -> (Vec<Swing>, Vec<CancelReason>) {
    let config = Config {
        cpi: rec.header.cpi,
        report_limit: rec.header.report_limit,
        ..Config::default()
    };
    let mut client = Client::new(Replay::new(rec, Pace::Fast), config);
    let (mut swings, mut cancels) = (Vec::new(), Vec::new());
    while !client.input().finished() {
        while let Some(e) = client.poll().unwrap() {
            match e {
                Event::Shot(s) => swings.push(s.swing),
                Event::Canceled(r) => cancels.push(r),
                _ => {}
            }
        }
    }
    (swings, cancels)
}

#[test]
fn fixtures_match_their_expectations() {
    let mut checked = 0;
    for entry in std::fs::read_dir(dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let rec = Recording::load(&path).unwrap();
        let expect = Expectations::load(path.with_extension("expect.json")).unwrap();
        let (swings, cancels) = replay(&rec);
        if let Err(e) = expect.check(&swings, &cancels) {
            panic!("{}: {e}\nswings: {swings:#?}", path.display());
        }
        checked += 1;
    }
    assert!(
        checked >= 2,
        "expected at least two fixtures, found {checked}"
    );
}

const CPI: f64 = 400.0;
fn c(inches: f64) -> f64 {
    inches * CPI
}

/// Isolated ±1-count reports, as an idle trackball sends.
fn idle(b: StreamBuilder, seconds: u32) -> StreamBuilder {
    let mut b = b;
    for i in 0..seconds * 2 {
        b = b
            .pause_ms(500.0)
            .report(i32::from(i % 2 == 0), if i % 3 == 0 { -1 } else { 1 });
    }
    b
}

struct Fixture {
    name: &'static str,
    rate: f64,
    samples: Vec<Sample>,
    marks: Vec<(usize, &'static str)>,
    sidecar: String,
}

/// 125 Hz with ±1-count jitter: a straight drive, a draw stroke and a fade
/// with a smooth reversal.
fn fixture_125() -> Fixture {
    let mut marks = Vec::new();
    let mut b = idle(StreamBuilder::new(125.0), 2);
    marks.push((
        b.clone().build().len(),
        "straight: pull straight back, push straight, full power",
    ));
    b = b
        .pull(0.0, c(0.75), 150.0)
        .pause_ms(400.0)
        .push(0.0, c(1.5), 100.0)
        .push(0.0, c(0.25), 200.0);
    b = idle(b, 3);
    marks.push((
        b.clone().build().len(),
        "draw stroke: pull 30° right, push straight, 90 %",
    ));
    b = b
        .pull(-30.0, c(0.6), 150.0)
        .pause_ms(300.0)
        .push(0.0, c(1.35), 100.0)
        .push(0.0, c(0.2), 200.0);
    b = idle(b, 3);
    marks.push((
        b.clone().build().len(),
        "fade stroke: pull 15° left, push 5° left in one motion, 67 %",
    ));
    b = b
        .pull(15.0, c(0.9), 200.0)
        .push(-5.0, c(1.0), 100.0)
        .push(-5.0, c(0.2), 200.0);
    b = idle(b, 3);
    Fixture {
        name: "straight_draw_fade_125hz",
        rate: 125.0,
        samples: jitter(&b.build(), 1, 125),
        marks,
        sidecar: r#"{
  "note": "Generated synthetically by thumbswing::testing (tests/fixtures.rs), not recorded from hardware.",
  "tol_deg": 1.5,
  "tol_speed_pct": 3,
  "tol_len_in": 0.1,
  "swings": [
    {"back_dir_deg": 0, "back_len_in": 0.75, "fwd_dir_deg": 0, "fwd_peak_in_s": 15},
    {"back_dir_deg": -30, "back_len_in": 0.6, "fwd_dir_deg": 0, "fwd_peak_in_s": 13.5},
    {"back_dir_deg": 15, "back_len_in": 0.9, "fwd_dir_deg": -5, "fwd_peak_in_s": 10}
  ],
  "cancels": []
}
"#
        .to_owned(),
    }
}

/// 1000 Hz: a wobble at the top, a dribble before the real push, a quick
/// second swing during the coast, and a pull too short to count.
fn fixture_1000() -> Fixture {
    let mut marks = Vec::new();
    let mut b = idle(StreamBuilder::new(1000.0), 2);
    marks.push((
        b.clone().build().len(),
        "wobble at the top, then a full push",
    ));
    b = b
        .pull(10.0, c(0.75), 150.0)
        .pause_ms(150.0)
        .report(0, -1)
        .report(0, -1)
        .pause_ms(250.0)
        .push(0.0, c(1.5), 100.0)
        .push(0.0, c(0.2), 200.0);
    b = idle(b, 3);
    marks.push((
        b.clone().build().len(),
        "dribble forward, wait, then the real push",
    ));
    b = b
        .pull(-20.0, c(0.75), 150.0)
        .pause_ms(200.0)
        .push(0.0, c(0.15), 150.0)
        .pause_ms(300.0)
        .push(8.0, c(1.1), 100.0)
        .push(8.0, c(0.3), 150.0);
    marks.push((
        b.clone().build().len(),
        "quick second swing while the ball coasts",
    ));
    b = b
        .pull(0.0, c(0.75), 150.0)
        .pause_ms(200.0)
        .push(-10.0, c(0.75), 100.0)
        .push(-10.0, c(0.15), 200.0);
    b = idle(b, 3);
    marks.push((b.clone().build().len(), "a pull too short to count"));
    b = b.pull(0.0, c(0.125), 50.0);
    b = idle(b, 2);
    Fixture {
        name: "wobble_dribble_quick_1000hz",
        rate: 1000.0,
        samples: b.build(),
        marks,
        sidecar: r#"{
  "note": "Generated synthetically by thumbswing::testing (tests/fixtures.rs), not recorded from hardware.",
  "tol_deg": 1.5,
  "tol_speed_pct": 3,
  "tol_len_in": 0.1,
  "swings": [
    {"back_dir_deg": 10, "back_len_in": 0.75, "fwd_dir_deg": 0, "fwd_peak_in_s": 15},
    {"back_dir_deg": -20, "back_len_in": 0.75, "fwd_dir_deg": 8, "fwd_peak_in_s": 11},
    {"back_dir_deg": 0, "back_len_in": 0.75, "fwd_dir_deg": -10, "fwd_peak_in_s": 7.5}
  ],
  "cancels": ["Dribble", "Short"]
}
"#
        .to_owned(),
    }
}

#[test]
#[ignore = "rewrites tests/fixtures; run when the generator changes"]
fn regenerate() {
    for f in [fixture_125(), fixture_1000()] {
        let rec = synthetic_recording(&f.samples, &f.marks, CPI, f.rate);
        let path = dir().join(format!("{}.jsonl", f.name));
        let mut out = Vec::new();
        rec.write(&mut out).unwrap();
        std::fs::write(&path, out).unwrap();
        std::fs::write(path.with_extension("expect.json"), &f.sidecar).unwrap();
    }
}

#[test]
fn the_generator_still_produces_the_committed_fixtures() {
    for f in [fixture_125(), fixture_1000()] {
        let rec = synthetic_recording(&f.samples, &f.marks, CPI, f.rate);
        let committed = Recording::load(dir().join(format!("{}.jsonl", f.name))).unwrap();
        assert_eq!(
            committed.entries, rec.entries,
            "{}: run the ignored `regenerate` test",
            f.name
        );
    }
}
