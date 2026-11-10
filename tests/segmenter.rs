//! Synthetic-stream tests of the segmenter and the client: every swing test
//! runs at 125 Hz and at 1000 Hz and expects the same result.

use thumbswing::launch::face_weight;
use thumbswing::recording::{Pace, Replay};
use thumbswing::testing::{Clock, StreamBuilder, jitter, synthetic_recording};
use thumbswing::{
    CancelReason, Client, Club, Config, CurveSense, Event, Output, Phase, Sample, Segmenter, Shot,
    Swing, Tuning,
};

const RATES: [f64; 2] = [125.0, 1000.0];
/// 400 CPI: 30 in/s (full power) is 12 000 counts/s; 1 in is 400 counts.
const CPI: f64 = 400.0;

fn counts(inches: f64) -> f64 {
    inches * CPI
}

/// A forward stroke of `peak` in/s lasting 100 ms.
fn push_counts(peak: f64) -> f64 {
    counts(peak * 0.1)
}

fn run(samples: &[Sample], tail_ms: u64) -> (Vec<Output>, Segmenter) {
    run_with(samples, tail_ms, &Config::default(), 30.0)
}

fn run_with(
    samples: &[Sample],
    tail_ms: u64,
    config: &Config,
    full: f64,
) -> (Vec<Output>, Segmenter) {
    let mut seg = Segmenter::new(config, full);
    let mut clock = Clock::new();
    let mut out = clock.feed(&mut seg, samples);
    out.extend(clock.pause_ms(&mut seg, tail_ms));
    (out, seg)
}

fn swings(out: &[Output]) -> Vec<Swing> {
    out.iter()
        .filter_map(|o| match o {
            Output::Swing { swing, .. } => Some(*swing),
            _ => None,
        })
        .collect()
}

fn cancels(out: &[Output]) -> Vec<CancelReason> {
    out.iter()
        .filter_map(|o| match o {
            Output::Canceled(r) => Some(*r),
            _ => None,
        })
        .collect()
}

/// Straight back 1.5 in over 150 ms, a pause, a push of `peak` in/s at `f`.
fn basic(rate: f64, b: f64, f: f64, peak: f64, pause_ms: f64) -> Vec<Sample> {
    StreamBuilder::new(rate)
        .pull(b, counts(1.5), 150.0)
        .pause_ms(pause_ms)
        .push(f, push_counts(peak), 100.0)
        .build()
}

/// Run samples through a client (fast replay) and return the events.
fn client_events(samples: &[Sample], club: Club, tuning: Tuning) -> Vec<Event> {
    let rec = synthetic_recording(samples, &[], CPI, 125.0);
    let mut client = Client::with_tuning(Replay::new(&rec, Pace::Fast), Config::default(), tuning);
    client.set_club(club);
    let mut events = Vec::new();
    while !client.input().finished() {
        while let Some(e) = client.poll().unwrap() {
            events.push(e);
        }
    }
    events
}

fn shots(events: &[Event]) -> Vec<Shot> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Shot(s) => Some((**s).clone()),
            _ => None,
        })
        .collect()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn straight_swing_is_one_square_shot_at_the_reference_speed() {
    for rate in RATES {
        let events = client_events(
            &basic(rate, 0.0, 0.0, 15.0, 300.0),
            Club::Iron7,
            Tuning::default(),
        );
        let s = shots(&events);
        assert_eq!(s.len(), 1, "{rate} Hz: {events:?}");
        let shot = &s[0];
        let d = shot.delivery;
        assert!(
            close(d.path_deg, 0.0, 0.3),
            "{rate} Hz: path {}",
            d.path_deg
        );
        assert!(
            close(d.face_deg, 0.0, 0.3),
            "{rate} Hz: face {}",
            d.face_deg
        );
        assert!(close(d.face_to_path_deg(), 0.0, 0.3));
        // V = p · max_club_speed · ratio_7I, p = 1 at the reference speed.
        assert!(
            close(shot.power, 1.0, 0.02),
            "{rate} Hz: power {}",
            shot.power
        );
        assert!(
            close(d.club_speed_mph, 80.0, 1.6),
            "{rate} Hz: V {}",
            d.club_speed_mph
        );
        assert!(close(shot.swing.fwd_peak_in_s, 15.0, 0.3), "{rate} Hz");
        assert!(!shot.raw.is_empty());
    }
}

#[test]
fn pull_left_push_straight_fades_in_arcade_sense_and_draws_in_physical() {
    for rate in RATES {
        let samples = basic(rate, 20.0, 0.0, 30.0, 300.0);
        let arcade = shots(&client_events(&samples, Club::Iron7, Tuning::default()));
        let d = arcade[0].delivery;
        assert!(close(arcade[0].swing.back_dir_deg, 20.0, 1.0), "{rate} Hz");
        assert!(close(d.face_deg, 0.0, 0.3), "{rate} Hz: F {}", d.face_deg);
        assert!(close(d.path_deg, -4.0, 0.3), "{rate} Hz: P {}", d.path_deg);
        assert!(close(d.face_to_path_deg(), 4.0, 0.3), "{rate} Hz");
        assert!(arcade[0].launch.sidespin_rpm > 0.0, "a fade curves right");

        let phys = Tuning {
            curve_sense: CurveSense::Physical,
            ..Tuning::default()
        };
        let p = shots(&client_events(&samples, Club::Iron7, phys));
        assert!(close(p[0].delivery.path_deg, 4.0, 0.3));
        assert!(close(p[0].delivery.face_to_path_deg(), -4.0, 0.3));
        assert!(p[0].launch.sidespin_rpm < 0.0, "a draw curves left");

        let lh_mirror = Tuning {
            mirror_for_left_hand: true,
            ..Tuning::default()
        };
        let rec = synthetic_recording(&samples, &[], CPI, rate);
        let mut client =
            Client::with_tuning(Replay::new(&rec, Pace::Fast), Config::default(), lh_mirror);
        client.set_hand(thumbswing::Hand::Left);
        let mut events = Vec::new();
        while !client.input().finished() {
            while let Some(e) = client.poll().unwrap() {
                events.push(e);
            }
        }
        let m = shots(&events);
        assert!(
            close(m[0].delivery.path_deg, 4.0, 0.3),
            "mirror flips the path"
        );
        assert!(close(m[0].delivery.face_to_path_deg(), -4.0, 0.3));
        assert!(m[0].launch.sidespin_rpm < 0.0);
    }
}

#[test]
fn push_right_of_a_straight_pull() {
    for rate in RATES {
        let samples = basic(rate, 0.0, 10.0, 30.0, 300.0);
        for (sense, path, f2p) in [
            (CurveSense::Arcade, 4.5, -2.0),
            (CurveSense::Physical, 0.5, 2.0),
        ] {
            let t = Tuning {
                curve_sense: sense,
                ..Tuning::default()
            };
            let s = shots(&client_events(&samples, Club::Iron7, t));
            let d = s[0].delivery;
            let l = s[0].launch;
            assert!(
                close(d.face_deg, 2.5, 0.3),
                "{rate} Hz {sense:?}: F {}",
                d.face_deg
            );
            assert!(
                close(d.path_deg, path, 0.3),
                "{rate} Hz {sense:?}: P {}",
                d.path_deg
            );
            assert!(close(d.face_to_path_deg(), f2p, 0.3));
            let (lo, hi) = if d.path_deg < d.face_deg {
                (d.path_deg, d.face_deg)
            } else {
                (d.face_deg, d.path_deg)
            };
            assert!(l.launch_azimuth_deg > lo && l.launch_azimuth_deg < hi);
            assert!(
                l.sidespin_rpm * d.face_to_path_deg() > 0.0,
                "sidespin has the sign of A"
            );
        }
    }
}

#[test]
fn half_the_peak_halves_power_club_speed_and_ball_speed() {
    for rate in RATES {
        let full = shots(&client_events(
            &basic(rate, 0.0, 0.0, 12.0, 300.0),
            Club::Iron7,
            Tuning::default(),
        ));
        let half = shots(&client_events(
            &basic(rate, 0.0, 0.0, 6.0, 300.0),
            Club::Iron7,
            Tuning::default(),
        ));
        let ratio = half[0].power / full[0].power;
        assert!(close(ratio, 0.5, 0.02), "{rate} Hz: {ratio}");
        assert!(close(
            half[0].delivery.club_speed_mph / full[0].delivery.club_speed_mph,
            0.5,
            0.02
        ));
        // v = V · SF · cos Φ / cos L, linear in V for a square face.
        let l = half[0].launch;
        let expect = half[0].delivery.club_speed_mph * 1.353;
        assert!(
            close(l.ball_speed_mph, expect, 0.2),
            "{} vs {expect}",
            l.ball_speed_mph
        );
    }
}

#[test]
fn a_backswing_still_moving_after_back_max_times_out() {
    for rate in RATES {
        // 2.04 s of pulling at 2.5 in/s (above v_stop, so it never coils).
        let samples = StreamBuilder::new(rate)
            .pull(0.0, counts(5.1), 2040.0)
            .build();
        let (out, _) = run(&samples, 50);
        assert_eq!(
            cancels(&out).first(),
            Some(&CancelReason::Timeout),
            "{rate} Hz"
        );
        assert!(swings(&out).is_empty());
    }
}

#[test]
fn a_backswing_followed_by_silence_coils_then_times_out() {
    for rate in RATES {
        let samples = StreamBuilder::new(rate)
            .pull(0.0, counts(1.5), 150.0)
            .build();
        let (out, seg) = run(&samples, 9_000);
        assert!(cancels(&out).is_empty());
        assert_eq!(seg.phase(), Phase::Coiled, "silence counts as stopped");
        let (out, seg) = run(&samples, 10_100);
        assert_eq!(cancels(&out), vec![CancelReason::CoilTimeout], "{rate} Hz");
        assert_eq!(seg.phase(), Phase::Idle);
    }
}

#[test]
fn a_short_backswing_is_canceled() {
    for rate in RATES {
        let samples = StreamBuilder::new(rate)
            .pull(0.0, counts(0.25), 50.0)
            .build();
        let (out, _) = run(&samples, 200);
        assert_eq!(cancels(&out), vec![CancelReason::Short], "{rate} Hz");
    }
}

#[test]
fn a_forward_roll_from_rest_is_ignored() {
    for rate in RATES {
        let samples = StreamBuilder::new(rate)
            .push(0.0, counts(3.0), 100.0)
            .build();
        let (out, seg) = run(&samples, 3000);
        assert!(out.is_empty(), "{rate} Hz: {out:?}");
        assert_eq!(seg.phase(), Phase::Idle);
    }
}

#[test]
fn idle_jitter_never_leaves_idle_and_a_twitch_keeps_the_coil() {
    for rate in RATES {
        // ±1-count reports every 50 ms for 5 s.
        let mut b = StreamBuilder::new(rate);
        for i in 0..100 {
            b = b.pause_ms(50.0).report(
                if i % 2 == 0 { 1 } else { -1 },
                if i % 3 == 0 { 1 } else { -1 },
            );
        }
        let samples = jitter(&b.build(), 1, 7);
        let (out, seg) = run(&samples, 100);
        assert!(out.is_empty(), "{rate} Hz: {out:?}");
        assert_eq!(seg.phase(), Phase::Idle);

        let samples = StreamBuilder::new(rate)
            .pull(0.0, counts(1.5), 150.0)
            .pause_ms(200.0)
            .report(1, 0)
            .pause_ms(200.0)
            .report(0, -1)
            .build();
        let (out, seg) = run(&samples, 100);
        assert!(cancels(&out).is_empty(), "{rate} Hz: {out:?}");
        assert_eq!(seg.phase(), Phase::Coiled);
    }
}

#[test]
fn a_one_count_wobble_at_the_top_keeps_the_backswing() {
    for rate in RATES {
        let samples = StreamBuilder::new(rate)
            .pull(30.0, counts(1.5), 150.0)
            .pause_ms(150.0)
            .report(0, -1)
            .report(0, -1)
            .pause_ms(250.0)
            .push(0.0, push_counts(30.0), 100.0)
            .build();
        let (out, _) = run(&samples, 3000);
        let s = swings(&out);
        assert_eq!(s.len(), 1, "{rate} Hz: {out:?}");
        assert!(
            close(s[0].back_dir_deg, 30.0, 1.0),
            "{rate} Hz: {}",
            s[0].back_dir_deg
        );
        assert!(close(s[0].back_len_in, 1.5, 0.05));
        assert!(
            cancels(&out).is_empty(),
            "a 1-count wobble is not even a dribble"
        );
    }
}

#[test]
fn a_dribble_keeps_the_coil_and_the_next_push_counts() {
    for rate in RATES {
        // 0.3 in at 2 in/s: faster than v_stop, shorter than fwd_min, slower
        // than v_min_swing.
        let dribble = StreamBuilder::new(rate)
            .pull(-15.0, counts(1.5), 150.0)
            .pause_ms(200.0)
            .push(0.0, counts(0.3), 150.0)
            .build();
        let (out, seg) = run(&dribble, 300);
        assert_eq!(
            cancels(&out),
            vec![CancelReason::Dribble],
            "{rate} Hz: {out:?}"
        );
        assert_eq!(seg.phase(), Phase::Coiled);

        let samples = StreamBuilder::new(rate)
            .pull(-15.0, counts(1.5), 150.0)
            .pause_ms(200.0)
            .push(0.0, counts(0.3), 150.0)
            .pause_ms(300.0)
            .push(5.0, push_counts(28.0), 100.0)
            .build();
        let (out, _) = run(&samples, 3000);
        let s = swings(&out);
        assert_eq!(s.len(), 1, "{rate} Hz: {out:?}");
        assert!(close(s[0].back_dir_deg, -15.0, 1.0));
        assert!(close(s[0].fwd_dir_deg, 5.0, 1.0));
        assert!(close(s[0].fwd_peak_in_s, 28.0, 0.6));
    }
}

#[test]
fn a_lone_report_after_a_gap_does_not_spike_or_start_a_swing() {
    for rate in RATES {
        // A big single report 200 ms after the last one.
        let samples = StreamBuilder::new(rate)
            .report(0, 1)
            .pause_ms(200.0)
            .report(-2, 40)
            .build();
        let (out, seg) = run(&samples, 500);
        assert!(out.is_empty(), "{rate} Hz: {out:?}");
        assert_eq!(seg.phase(), Phase::Idle);
    }
    // A report not after the previous one is a gap, not a negative interval.
    let mut samples = basic(125.0, 0.0, 0.0, 30.0, 300.0);
    let dup = samples[3];
    samples.insert(4, Sample::new(dup.t_us, 0, 3));
    let (out, _) = run(&samples, 3000);
    assert_eq!(swings(&out).len(), 1);
}

#[test]
fn rate_equivalence_125_vs_1000_hz() {
    for (b, f, peak) in [(0.0, 0.0, 30.0), (30.0, -30.0, 27.0), (-20.0, 10.0, 18.0)] {
        let get = |rate| {
            let (out, _) = run(&basic(rate, b, f, peak, 250.0), 3000);
            swings(&out)[0]
        };
        let (lo, hi) = (get(125.0), get(1000.0));
        assert!(
            (lo.fwd_peak_in_s / hi.fwd_peak_in_s - 1.0).abs() <= 0.02,
            "peak {} vs {}",
            lo.fwd_peak_in_s,
            hi.fwd_peak_in_s
        );
        assert!(
            close(lo.fwd_dir_deg, hi.fwd_dir_deg, 1.0),
            "{} vs {}",
            lo.fwd_dir_deg,
            hi.fwd_dir_deg
        );
        assert!(close(lo.back_dir_deg, hi.back_dir_deg, 1.0));
        assert!(close(lo.fwd_dir_deg, f, 1.0) && close(lo.back_dir_deg, b, 1.0));
    }
}

#[test]
fn a_smooth_reversal_is_one_swing() {
    for rate in RATES {
        let samples = StreamBuilder::new(rate)
            .pull(10.0, counts(1.5), 150.0)
            .push(0.0, push_counts(30.0), 100.0)
            .build();
        let (out, _) = run(&samples, 3000);
        let s = swings(&out);
        assert_eq!(s.len(), 1, "{rate} Hz: {out:?}");
        assert!(close(s[0].back_dir_deg, 10.0, 1.0));
        assert!(close(s[0].back_len_in, 1.5, 0.05), "{}", s[0].back_len_in);
        assert!(s[0].pause_ms < 20.0);
        assert!(cancels(&out).is_empty());
    }
}

#[test]
fn a_pull_during_cooldown_starts_the_next_swing() {
    for rate in RATES {
        let samples = StreamBuilder::new(rate)
            .pull(0.0, counts(1.5), 150.0)
            .pause_ms(200.0)
            .push(0.0, push_counts(30.0), 100.0)
            // The ball coasts on, slowing, then the player pulls again.
            .push(0.0, counts(1.0), 200.0)
            .pull(15.0, counts(1.5), 150.0)
            .pause_ms(200.0)
            .push(-5.0, push_counts(20.0), 100.0)
            .build();
        let (out, _) = run(&samples, 3000);
        let s = swings(&out);
        assert_eq!(s.len(), 2, "{rate} Hz: {out:?}");
        assert!(close(s[1].back_dir_deg, 15.0, 1.5), "{}", s[1].back_dir_deg);
        assert!(close(s[1].fwd_dir_deg, -5.0, 1.0));
    }
}

#[test]
fn sensitivity_follows_cpi_and_full_power_speed() {
    // Doubling the CPI halves the counts for the same speed.
    let at = |cpi: f64, scale: f64| {
        let config = Config {
            cpi,
            ..Config::default()
        };
        let samples = StreamBuilder::new(1000.0)
            .pull(0.0, counts(1.5) * scale, 150.0)
            .pause_ms(200.0)
            .push(0.0, push_counts(24.0) * scale, 100.0)
            .build();
        let (out, _) = run_with(&samples, 3000, &config, 30.0);
        swings(&out)[0]
    };
    let a = at(400.0, 1.0);
    let b = at(800.0, 2.0);
    assert!(close(a.fwd_peak_in_s, 24.0, 0.5) && close(b.fwd_peak_in_s, 24.0, 0.5));
    assert!(close(a.back_len_in, b.back_len_in, 0.02));

    // Every threshold is a fraction of the full-power speed: at 10 in/s a
    // 1 in/s pull (above 0.067 · 10) swings, at 30 in/s it is noise.
    let full = 10.0;
    let slow = StreamBuilder::new(125.0)
        .pull(0.0, counts(0.5), 500.0)
        .pause_ms(300.0)
        .push(0.0, counts(1.0), 100.0)
        .build();
    let (out, _) = run(&slow, 3000);
    assert!(
        swings(&out).is_empty(),
        "at 30 in/s full power a 1 in/s pull is noise"
    );
    let (out, _) = run_with(&slow, 3000, &Config::default(), full);
    let s = swings(&out);
    assert_eq!(s.len(), 1, "{out:?}");
    assert!(close(s[0].fwd_peak_in_s / full, 1.0, 0.05));
}

#[test]
fn saturation_is_flagged_and_direction_comes_from_before_it() {
    // 125 Hz, a push that starts at 10° right, then two reports pinned at the
    // 127 limit straight forward.
    let mut b = StreamBuilder::new(125.0)
        .pull(0.0, counts(1.5), 150.0)
        .pause_ms(300.0)
        .push(10.0, counts(0.6), 24.0);
    b = b
        .report(0, -127)
        .report(0, -127)
        .push(10.0, counts(0.4), 16.0);
    let samples = b.build();
    let (out, _) = run(&samples, 3000);
    let s = swings(&out);
    assert_eq!(s.len(), 1, "{out:?}");
    assert!(s[0].saturated());
    assert_eq!(s[0].saturated_reports, 2);
    assert!(close(s[0].fwd_dir_deg, 10.0, 1.0), "{}", s[0].fwd_dir_deg);

    let rec = synthetic_recording(&samples, &[], CPI, 125.0);
    let mut client = Client::new(Replay::new(&rec, Pace::Fast), Config::default());
    let mut events = Vec::new();
    while !client.input().finished() {
        while let Some(e) = client.poll().unwrap() {
            events.push(e);
        }
    }
    let i = events
        .iter()
        .position(|e| matches!(e, Event::Saturated { reports: 2 }))
        .expect("Saturated event");
    assert!(
        matches!(events[i + 1], Event::Shot(_)),
        "Saturated comes just before the shot"
    );
}

#[test]
fn putting() {
    for rate in RATES {
        let samples = basic(rate, 25.0, 4.0, 30.0, 300.0);
        let s = shots(&client_events(&samples, Club::Putter, Tuning::default()));
        let shot = &s[0];
        assert!(close(shot.launch.launch_angle_deg, 0.3, 1e-9));
        assert!(shot.launch.total_spin_rpm() == 0.0);
        assert!(close(shot.delivery.club_speed_mph, shot.power * 11.0, 1e-9));
        assert!(close(shot.delivery.path_deg, shot.swing.fwd_dir_deg, 1e-9));
        assert!(close(shot.delivery.face_deg, shot.swing.fwd_dir_deg, 1e-9));
        assert!(
            close(shot.delivery.path_deg, 4.0, 1.0),
            "{rate} Hz: pull has no effect"
        );
    }
}

#[test]
fn backswing_events_are_live_and_capped_at_full_cock() {
    let samples = StreamBuilder::new(125.0)
        .pull(-10.0, counts(3.0), 300.0)
        .build();
    let (out, _) = run(&samples, 100);
    let lens: Vec<f64> = out
        .iter()
        .filter_map(|o| match o {
            Output::Backswing { len_in, .. } => Some(*len_in),
            _ => None,
        })
        .collect();
    assert!(lens.len() >= 5, "{lens:?}");
    assert!(lens.windows(2).all(|w| w[1] >= w[0]));
    assert!(
        close(*lens.last().unwrap(), 2.0, 0.01),
        "full cock is 2 in at the defaults"
    );
}

#[test]
fn launch_start_line_sits_nearer_the_face() {
    // The D-plane rule: a 7-iron's face weight at ~27° spin loft.
    assert!(close(face_weight(26.7), 0.77, 0.01));
}
