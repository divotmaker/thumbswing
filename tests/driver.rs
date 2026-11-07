//! The driver's session without hardware: synthetic swing streams and the
//! JSONL fixtures through a replay input, via `Thumbswing::connect_with`, and
//! `connect` with a given input.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use flightrelay::config::DriverConfig;
use flightrelay::driver::{
    Backoff, ConnectCtx, ConnectHost, DeviceCommand, DeviceDriver, DeviceEvent, DeviceSession,
    Events, HostFlags, SessionEnd, Stop,
};
use flightrelay::{FrpEvent, Handedness, Severity, ShotKey};
use thumbswing::driver::{Session, Thumbswing};
use thumbswing::recording::{Pace, Recording, Replay};
use thumbswing::testing::{StreamBuilder, synthetic_recording};
use thumbswing::{DeviceInfo, Error, Input, Sample, Settings};

/// Two swings: straight at full power, then a pull 20° left and a push 5°
/// right at 75 %.
fn two_swings() -> Recording {
    let samples = StreamBuilder::new(125.0)
        .pull(0.0, 600.0, 150.0)
        .pause_ms(300.0)
        .push(0.0, 1200.0, 100.0)
        .pause_ms(2500.0)
        .pull(20.0, 600.0, 150.0)
        .pause_ms(300.0)
        .push(5.0, 900.0, 100.0)
        .build();
    synthetic_recording(&samples, &[], 400.0, 125.0)
}

/// Polls with no wait, enough times for a fast replay of `rec` to play out
/// (every poll moves its clock 1 ms).
fn polls_for(rec: &Recording) -> usize {
    let last_ms = rec.samples().last().map_or(0, |s| s.t_us / 1000);
    usize::try_from(last_ms).unwrap() + 4000
}

fn play(session: &mut Session, flags: &HostFlags, polls: usize) -> Vec<DeviceEvent> {
    let mut seen = Vec::new();
    let mut sink = |e| seen.push(e);
    let mut out = Events::new(&mut sink, flags);
    for _ in 0..polls {
        session.poll(Duration::ZERO, &mut out).unwrap();
    }
    seen
}

fn frp(events: &[DeviceEvent]) -> Vec<&FrpEvent> {
    events
        .iter()
        .filter_map(|e| match e {
            DeviceEvent::Frp(f) => Some(f),
            _ => None,
        })
        .collect()
}

/// Every telemetry key in order, the later value winning.
fn telemetry(events: &[&FrpEvent]) -> HashMap<String, String> {
    let mut all = HashMap::new();
    for e in events {
        if let FrpEvent::DeviceTelemetry {
            telemetry: Some(t), ..
        } = e
        {
            all.extend(t.clone());
        }
    }
    all
}

fn shot_keys(events: &[&FrpEvent]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match e {
            FrpEvent::ShotTrigger { key, .. } => Some(key.shot_number),
            _ => None,
        })
        .collect()
}

fn driver() -> Thumbswing {
    Thumbswing::new(Settings::default()).unwrap()
}

#[test]
fn a_session_sends_telemetry_then_each_shot_once_with_the_latched_club() {
    let rec = two_swings();
    let mut driver = driver();
    let latched = DeviceCommand::default()
        .with_club(flightrelay::Club::Driver)
        .with_handed(Handedness::Left);
    let mut session = driver.connect_with(Replay::new(&rec, Pace::Fast), &latched);
    assert_eq!(session.identity().device, "synthetic trackball");
    assert_eq!(
        session.identity().model.as_deref(),
        Some("thumbswing trackball")
    );
    assert_eq!(
        session.identity().manufacturer.as_deref(),
        Some("divotmaker")
    );

    let flags = HostFlags::new();
    flags.set_wants_raw(true); // no wire protocol: still no raw frames
    let events = play(&mut session, &flags, polls_for(&rec));
    let events = frp(&events);

    // Telemetry comes first, before any shot event.
    let FrpEvent::DeviceTelemetry {
        telemetry: Some(first),
        source,
        ..
    } = events[0]
    else {
        panic!("{:?}", events[0]);
    };
    assert_eq!(*source, None, "the host stamps source");
    assert_eq!(first["synthetic"], "true");
    assert_eq!(first["input"], "synthetic trackball");
    assert_eq!(first["cpi"], "400");
    assert_eq!(first["profile"], "DR");
    assert_eq!(first["handed"], "lh");
    assert_eq!(telemetry(&events)["ready"], "true");

    assert!(
        events.iter().all(|e| !matches!(
            e,
            FrpEvent::RawFrame(_) | FrpEvent::FlightResult { .. } | FrpEvent::FaceImpact { .. }
        )),
        "{events:?}"
    );

    // Each shot: trigger (with the time) → launch → club_path → finished.
    let shots: Vec<&[&FrpEvent]> = events
        .split_inclusive(|e| matches!(e, FrpEvent::ShotFinished { .. }))
        .filter_map(|chunk| {
            let start = chunk
                .iter()
                .position(|e| matches!(e, FrpEvent::ShotTrigger { .. }))?;
            Some(&chunk[start..])
        })
        .collect();
    assert_eq!(shots.len(), 2, "{events:?}");
    let mut first_key: Option<ShotKey> = None;
    for (i, shot) in shots.iter().enumerate() {
        let [
            FrpEvent::ShotTrigger { key, timestamp },
            FrpEvent::Launch { key: k1, launch },
            FrpEvent::ClubPath { key: k2, club },
            FrpEvent::ShotFinished { key: k3 },
        ] = shot
        else {
            panic!("{shot:?}");
        };
        assert!(k1 == key && k2 == key && k3 == key);
        assert_eq!(key.shot_number as usize, i + 1);
        assert!(timestamp.is_some_and(|t| t > 1_600_000_000_000));
        if let Some(first) = &first_key {
            assert_ne!(first.shot_id, key.shot_id);
        }
        first_key = Some(key.clone());

        let speed = launch.launch_speed.unwrap().as_mph();
        let club_speed = club.club_speed.unwrap().as_mph();
        assert!(launch.total_spin_rpm.is_some() && launch.spin_axis.is_some());
        assert!(
            club.smash_factor
                .is_some_and(|s| (s - speed / club_speed).abs() < 0.01)
        );
        if i == 0 {
            // A full-power drive: 100 mph club, 149 mph ball, 10.4° up, square.
            assert!((club_speed - 100.0).abs() < 2.0, "{club_speed}");
            assert!((speed - 149.0).abs() < 3.0, "{speed}");
            assert!((launch.launch_elevation.unwrap() - 10.4).abs() < 0.1);
            assert!(club.path.unwrap().abs() < 0.3 && club.face_angle.unwrap().abs() < 0.3);
        } else {
            // 75 % with a fade stroke (δ = 20 − 5): face +1.25, path −1.75, curving right.
            assert!((club_speed - 75.0).abs() < 2.0, "{club_speed}");
            assert!((club.face_angle.unwrap() - 1.25).abs() < 0.3);
            assert!((club.path.unwrap() + 1.75).abs() < 0.3, "{:?}", club.path);
            assert!(launch.sidespin_rpm.unwrap() > 0);
        }
    }

    let mut seen = Vec::new();
    let mut sink = |e| seen.push(e);
    session.close(&mut Events::new(&mut sink, &flags)).unwrap();
    assert_eq!(telemetry(&frp(&seen))["ready"], "false");
}

#[test]
fn shot_numbers_keep_counting_across_sessions() {
    let rec = two_swings();
    let mut driver = driver();
    let flags = HostFlags::new();
    let mut keys = Vec::new();
    for _ in 0..2 {
        let mut session =
            driver.connect_with(Replay::new(&rec, Pace::Fast), &DeviceCommand::default());
        keys.extend(shot_keys(&frp(&play(
            &mut session,
            &flags,
            polls_for(&rec),
        ))));
    }
    assert_eq!(keys, [1, 2, 3, 4]);
}

#[test]
fn a_swing_while_shots_are_held_takes_no_number() {
    let rec = two_swings();
    let mut driver = driver();
    let mut session = driver.connect_with(Replay::new(&rec, Pace::Fast), &DeviceCommand::default());
    let flags = HostFlags::new();
    flags.set_shots_allowed(false);
    // Up to the first swing's end and its cooldown.
    let held = play(&mut session, &flags, 1500);
    assert!(shot_keys(&frp(&held)).is_empty(), "{held:?}");
    assert!(
        held.iter()
            .any(|e| matches!(e, DeviceEvent::Log(_, line) if line.contains("holding shots")))
    );
    flags.set_shots_allowed(true);
    let rest = play(&mut session, &flags, polls_for(&rec));
    assert_eq!(shot_keys(&frp(&rest)), [1]);
}

#[test]
fn commands_change_the_club_and_hand_only_when_they_differ() {
    let rec = two_swings();
    let mut driver = driver();
    let mut session = driver.connect_with(Replay::new(&rec, Pace::Fast), &DeviceCommand::default());
    let flags = HostFlags::new();
    let start = telemetry(&frp(&play(&mut session, &flags, 1)));
    assert_eq!(
        (start["profile"].as_str(), start["handed"].as_str()),
        ("7I", "rh")
    );

    // The same club and hand again: nothing to report.
    session
        .command(
            &DeviceCommand::default()
                .with_club(flightrelay::Club::Iron7)
                .with_handed(Handedness::Right),
        )
        .unwrap();
    assert!(telemetry(&frp(&play(&mut session, &flags, 1))).is_empty());

    // A putt is the putter selected; the mode needs nothing.
    session
        .command(
            &DeviceCommand::default()
                .with_mode(flightrelay::DetectionMode::Putting)
                .with_club(flightrelay::Club::Putter),
        )
        .unwrap();
    let after = telemetry(&frp(&play(&mut session, &flags, 1)));
    assert_eq!(after["profile"], "PT");
    assert_eq!(session.client().club(), thumbswing::Club::Putter);
    let rest = play(&mut session, &flags, polls_for(&rec));
    let rest = frp(&rest);
    let launches: Vec<_> = rest
        .iter()
        .filter_map(|e| match e {
            FrpEvent::Launch { launch, .. } => Some(launch),
            _ => None,
        })
        .collect();
    assert_eq!(launches.len(), 2);
    for l in launches {
        assert_eq!(l.total_spin_rpm, Some(0), "a putt has no spin");
        assert_eq!(l.spin_axis, None);
    }
}

#[test]
fn tuning_and_thresholds_apply_to_a_running_session() {
    let rec = two_swings();
    let mut driver = driver();
    let mut session = driver.connect_with(Replay::new(&rec, Pace::Fast), &DeviceCommand::default());
    let mut slower = Settings::default();
    slower.tuning.max_club_speed = 50.0;
    slower.config.thresholds.gap_ms = 60;
    assert!(!Settings::restart_needed(&Settings::default(), &slower));
    driver.apply_config(&slower);
    assert_eq!(driver.config(), &slower);

    let events = play(&mut session, &HostFlags::new(), polls_for(&rec));
    let events = frp(&events);
    assert!((session.client().tuning().max_club_speed - 50.0).abs() < f64::EPSILON);
    assert_eq!(session.client().config().thresholds.gap_ms, 60);
    let club_speeds: Vec<f64> = events
        .iter()
        .filter_map(|e| match e {
            FrpEvent::ClubPath { club, .. } => club.club_speed.map(flightrelay::Velocity::as_mph),
            _ => None,
        })
        .collect();
    // 7-iron at full power: 50 mph × 0.80.
    assert!((club_speeds[0] - 40.0).abs() < 1.5, "{club_speeds:?}");
}

#[test]
fn every_fixture_plays_through_the_driver() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let rec = Recording::load(&path).unwrap();
        let expect =
            thumbswing::testing::Expectations::load(path.with_extension("expect.json")).unwrap();
        let mut settings = Settings::default();
        settings.config.cpi = rec.header.cpi;
        settings.config.report_limit = rec.header.report_limit;
        let mut driver = Thumbswing::new(settings).unwrap();
        let mut session =
            driver.connect_with(Replay::new(&rec, Pace::Fast), &DeviceCommand::default());
        let events = play(&mut session, &HostFlags::new(), polls_for(&rec));
        let keys = shot_keys(&frp(&events));
        let want: Vec<u32> = (1..=u32::try_from(expect.swings.len()).unwrap()).collect();
        assert_eq!(keys, want, "{}", path.display());
        checked += 1;
    }
    assert!(checked >= 2);
}

/// An input that is unplugged after its first poll and comes back on the
/// second reconnect attempt.
struct Flaky {
    info: DeviceInfo,
    now: u64,
    unplugged: bool,
    gone: bool,
    attempts: u32,
}

impl Input for Flaky {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        if !self.unplugged {
            self.unplugged = true;
            self.gone = true;
            return Err(Error::Disconnected);
        }
        Ok(None)
    }

    fn now_us(&mut self) -> u64 {
        self.now += 100_000;
        self.now
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn reconnect(&mut self) -> Result<(), Error> {
        self.attempts += 1;
        if self.attempts >= 2 {
            self.gone = false;
            Ok(())
        } else {
            Err(Error::Disconnected)
        }
    }
}

#[test]
fn an_unplugged_input_reports_not_ready_until_it_is_back() {
    let input = Flaky {
        info: DeviceInfo {
            name: "Flaky Ball".to_owned(),
            relative_xy: true,
            ..DeviceInfo::default()
        },
        now: 0,
        unplugged: false,
        gone: false,
        attempts: 0,
    };
    let mut driver = driver();
    let mut session = driver.connect_with(input, &DeviceCommand::default());
    let events = play(&mut session, &HostFlags::new(), 200);
    let ready: Vec<&str> = frp(&events)
        .iter()
        .filter_map(|e| match e {
            FrpEvent::DeviceTelemetry {
                telemetry: Some(t), ..
            } => t.get("ready").map(String::as_str),
            _ => None,
        })
        .collect();
    assert_eq!(ready, ["true", "false", "true"]);
}

struct Host(DeviceCommand);

impl ConnectHost for Host {
    fn tick(&mut self) {}
    fn latched(&self) -> &DeviceCommand {
        &self.0
    }
}

#[test]
fn connect_serves_a_given_input_once_and_honors_stop() {
    let rec = two_swings();
    let flags = HostFlags::new();
    let mut seen = Vec::new();
    let mut sink = |e| seen.push(e);
    let stop = Stop::new();
    let mut host = Host(DeviceCommand::default().with_club(flightrelay::Club::PitchingWedge));
    let mut driver =
        Thumbswing::with_input(Settings::default(), Replay::new(&rec, Pace::Fast)).unwrap();

    let mut ctx = ConnectCtx::new(&stop, Events::new(&mut sink, &flags), &mut host);
    let session = driver.connect(&mut ctx).unwrap();
    assert_eq!(session.client().club(), thumbswing::Club::PitchingWedge);
    assert!(matches!(driver.connect(&mut ctx), Err(Error::NotFound(_))));
    stop.stop();
    assert!(matches!(driver.connect(&mut ctx), Err(Error::Stopped)));

    // A served input is not reconnected.
    let mut backoff = Backoff::default();
    let e = Error::Disconnected;
    let ended = SessionEnd {
        error: Some(&e),
        connected_for: Some(Duration::from_secs(1)),
        attempts: 0,
    };
    let mut alerts = Vec::new();
    let mut sink = |e| {
        if let DeviceEvent::Frp(FrpEvent::Alert { severity, .. }) = e {
            alerts.push(severity);
        }
    };
    assert_eq!(
        driver.retry_delay(&ended, &mut backoff, &mut Events::new(&mut sink, &flags)),
        None
    );
    assert_eq!(alerts, [Severity::Error]);
}

#[test]
fn an_unsupported_platform_is_given_up_on_and_other_errors_retried() {
    let flags = HostFlags::new();
    let mut sink = |_| {};
    let mut out = Events::new(&mut sink, &flags);
    let mut driver = driver();
    let mut backoff = Backoff::default();
    let failed = |e| SessionEnd {
        error: Some(e),
        connected_for: None,
        attempts: 1,
    };
    assert_eq!(
        driver.retry_delay(&failed(&Error::Unsupported), &mut backoff, &mut out),
        None
    );
    let busy = Error::Busy {
        path: "/dev/input/event5".into(),
    };
    assert_eq!(
        driver.retry_delay(&failed(&busy), &mut backoff, &mut out),
        Some(Duration::from_secs(1))
    );
    assert_eq!(Thumbswing::STOP_WAIT, Duration::from_secs(1));
    assert!(Settings::default().validate().is_ok());
    assert!(DriverConfig::validate(&Settings::default()).is_ok());
}

fn telemetry_of(events: &[DeviceEvent]) -> HashMap<String, String> {
    telemetry(&frp(events))
}

#[test]
fn the_setting_starts_the_club_and_hand_and_a_command_overrides_it() {
    let rec = two_swings();
    let mut settings = Settings {
        club: thumbswing::Club::Driver,
        hand: thumbswing::Hand::Left,
        ..Settings::default()
    };
    let mut driver = Thumbswing::new(settings.clone()).unwrap();
    let flags = HostFlags::new();

    // No command: the setting.
    let mut session = driver.connect_with(Replay::new(&rec, Pace::Fast), &DeviceCommand::default());
    let start = telemetry_of(&play(&mut session, &flags, 1));
    assert_eq!(
        (start["profile"].as_str(), start["handed"].as_str()),
        ("DR", "lh")
    );

    // A changed setting applies at once while the host has named nothing,
    // and only the changed key is sent.
    settings.club = thumbswing::Club::Wood3;
    driver.apply_config(&settings);
    let changed = telemetry_of(&play(&mut session, &flags, 1));
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert_eq!(changed["profile"], "3W");

    // The host's club replaces it, and a later setting change leaves it.
    session
        .command(&DeviceCommand::default().with_club(flightrelay::Club::PitchingWedge))
        .unwrap();
    let after = telemetry_of(&play(&mut session, &flags, 1));
    assert_eq!(after.len(), 1, "only the club changed: {after:?}");
    assert_eq!(after["profile"], "PW");
    settings.club = thumbswing::Club::Iron5;
    driver.apply_config(&settings);
    assert!(telemetry_of(&play(&mut session, &flags, 1)).is_empty());
    assert_eq!(session.client().club(), thumbswing::Club::PitchingWedge);
    // The hand still follows the setting: the host named no hand.
    settings.hand = thumbswing::Hand::Right;
    driver.apply_config(&settings);
    let hand = telemetry_of(&play(&mut session, &flags, 1));
    assert_eq!(hand.len(), 1, "{hand:?}");
    assert_eq!(hand["handed"], "rh");

    // A latched command at connect wins over the setting too.
    let latched = DeviceCommand::default().with_handed(Handedness::Left);
    let session = driver.connect_with(Replay::new(&rec, Pace::Fast), &latched);
    assert_eq!(session.client().club(), thumbswing::Club::Iron5);
    assert_eq!(session.client().hand(), thumbswing::Hand::Left);
}

#[test]
fn the_putting_mode_selects_the_putter_whatever_the_club() {
    let rec = two_swings();
    let mut driver = driver();
    let latched = DeviceCommand::default()
        .with_mode(flightrelay::DetectionMode::Putting)
        .with_club(flightrelay::Club::Driver);
    let mut session = driver.connect_with(Replay::new(&rec, Pace::Fast), &latched);
    assert_eq!(session.client().club(), thumbswing::Club::Putter);
    session
        .command(&DeviceCommand::default().with_mode(flightrelay::DetectionMode::Full))
        .unwrap();
    assert_eq!(session.client().club(), thumbswing::Club::Driver);
}

#[test]
fn a_fast_replay_plays_without_sleeping() {
    let rec = two_swings();
    let mut driver = driver();
    let mut session = driver.connect_with(Replay::new(&rec, Pace::Fast), &DeviceCommand::default());
    let flags = HostFlags::new();
    let mut seen = Vec::new();
    let started = std::time::Instant::now();
    // The recording is 3.6 s long, plus 3 s for the last swing to settle;
    // played without sleeping it takes a fraction of a second.
    while started.elapsed() < Duration::from_secs(3) && shot_keys(&frp(&seen)).len() < 2 {
        let mut sink = |e| seen.push(e);
        session
            .poll(
                Duration::from_millis(100),
                &mut Events::new(&mut sink, &flags),
            )
            .unwrap();
    }
    assert_eq!(shot_keys(&frp(&seen)), [1, 2]);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

/// Every error that carries a device path or link name, with the texts
/// holding them.
fn errors_with_paths() -> Vec<Error> {
    let link = "/dev/input/by-id/usb-Example_Ball_SERIAL123-event-mouse";
    vec![
        Error::Ambiguous {
            devices: vec![link.into(), "/dev/input/event7".into()],
        },
        Error::PermissionDenied { path: link.into() },
        Error::Busy { path: link.into() },
        Error::NotFound(format!("{link}: no such link")),
        Error::NotAPointer {
            path: "/dev/input/event3".into(),
        },
        Error::Parse {
            file: "/home/user/session.jsonl".into(),
            line: 3,
            message: "bad".into(),
        },
    ]
}

fn alerts_for(driver: &mut Thumbswing, error: &Error, backoff: &mut Backoff) -> Vec<String> {
    let flags = HostFlags::new();
    flags.set_min_level(flightrelay::driver::Level::Debug);
    let mut alerts = Vec::new();
    let mut debug = Vec::new();
    let mut sink = |e| match e {
        DeviceEvent::Frp(FrpEvent::Alert { message, .. }) => alerts.push(message),
        DeviceEvent::Log(flightrelay::driver::Level::Debug, line) => debug.push(line),
        _ => {}
    };
    let ended = SessionEnd {
        error: Some(error),
        connected_for: None,
        attempts: 1,
    };
    let delay = driver.retry_delay(&ended, backoff, &mut Events::new(&mut sink, &flags));
    assert!(delay.is_some(), "{error}: retried");
    assert!(
        debug.iter().any(|l| l.contains(&error.to_string())),
        "the full error is a debug line: {debug:?}"
    );
    alerts
}

#[test]
fn alerts_name_no_device_paths() {
    for error in errors_with_paths() {
        let mut d = driver();
        let alerts = alerts_for(&mut d, &error, &mut Backoff::default());
        assert_eq!(alerts.len(), 1, "{error}");
        let text = &alerts[0];
        for leak in ["/dev", "by-id", "SERIAL123", "event7", "event3", "/home"] {
            assert!(!text.contains(leak), "{text:?} holds {leak:?}");
        }
    }
    let mut d = driver();
    let ambiguous = &errors_with_paths()[0];
    let text = &alerts_for(&mut d, ambiguous, &mut Backoff::default())[0];
    assert!(
        text.contains("found 2") && text.contains("`device` setting"),
        "{text}"
    );
    let mut d = driver();
    let denied = &errors_with_paths()[1];
    let text = &alerts_for(&mut d, denied, &mut Backoff::default())[0];
    assert!(text.contains("`input` group"), "{text}");
}

#[test]
fn a_choice_left_to_the_user_is_alerted_once_per_outage() {
    let [ambiguous, denied, busy, ..] = <[Error; 6]>::try_from(errors_with_paths()).unwrap();
    // Opening goes: ambiguous ×3, no permission ×2, busy ×2, then the device
    // opens, and the next outage starts over.
    let results: Vec<Result<(), Error>> = vec![
        Err(ambiguous),
        Err(Error::Ambiguous {
            devices: vec!["/dev/input/event1".into(), "/dev/input/event2".into()],
        }),
        Err(Error::Ambiguous {
            devices: vec!["/dev/input/event1".into(), "/dev/input/event2".into()],
        }),
        Err(denied),
        Err(Error::PermissionDenied {
            path: "/dev/input/event1".into(),
        }),
        Err(busy),
        Err(Error::Busy {
            path: "/dev/input/event1".into(),
        }),
        Ok(()),
        Err(Error::Ambiguous {
            devices: vec!["/dev/input/event1".into(), "/dev/input/event2".into()],
        }),
    ];
    let queue = std::sync::Mutex::new(results.into_iter());
    let mut driver = Thumbswing::with_opener(Settings::default(), move |_, _| {
        match queue.lock().unwrap().next() {
            Some(Ok(())) => {
                Ok(Box::new(Replay::new(&two_swings(), Pace::Fast)) as Box<dyn Input + Send>)
            }
            Some(Err(e)) => Err(e),
            None => Err(Error::NotFound("done".into())),
        }
    })
    .unwrap();
    let flags = HostFlags::new();
    let stop = Stop::new();
    let mut host = Host(DeviceCommand::default());
    let mut backoff = Backoff::default();
    let mut alerts = Vec::new();
    let mut delays = Vec::new();
    for _ in 0..9 {
        let mut sink = |e| {
            if let DeviceEvent::Frp(FrpEvent::Alert { message, .. }) = e {
                alerts.push(message);
            }
        };
        let mut ctx = ConnectCtx::new(&stop, Events::new(&mut sink, &flags), &mut host);
        let ended = driver.connect(&mut ctx).err();
        if let Some(error) = ended {
            let mut sink = |e| {
                if let DeviceEvent::Frp(FrpEvent::Alert { message, .. }) = e {
                    alerts.push(message);
                }
            };
            let end = SessionEnd {
                error: Some(&error),
                connected_for: None,
                attempts: 1,
            };
            delays.push(driver.retry_delay(
                &end,
                &mut backoff,
                &mut Events::new(&mut sink, &flags),
            ));
        }
    }
    assert!(
        delays.iter().all(Option::is_some),
        "always retried: {delays:?}"
    );
    let kinds: Vec<&str> = alerts
        .iter()
        .map(|a| {
            if a.contains("found 2") {
                "ambiguous"
            } else if a.contains("cannot read") {
                "denied"
            } else if a.contains("grabbed") {
                "busy"
            } else {
                "other"
            }
        })
        .collect();
    assert_eq!(
        kinds,
        ["ambiguous", "denied", "busy", "busy", "ambiguous"],
        "{alerts:?}"
    );
}

/// An input that is unplugged after its first poll and never comes back
/// under the same node (its reconnect always fails, and it has no `by-id`
/// link).
struct Gone {
    info: DeviceInfo,
    now: u64,
    unplugged: bool,
}

impl Input for Gone {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        if self.unplugged {
            Ok(None)
        } else {
            self.unplugged = true;
            Err(Error::Disconnected)
        }
    }

    fn now_us(&mut self) -> u64 {
        self.now += 100_000;
        self.now
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn reconnect(&mut self) -> Result<(), Error> {
        Err(Error::NotFound("/dev/input/event7".into()))
    }
}

fn gone(name: &str) -> Box<dyn Input + Send> {
    gone_at("/dev/input/event7", &[], name)
}

/// A [`Gone`] input at `node` with `by_id` links.
fn gone_at(node: &str, by_id: &[&str], name: &str) -> Box<dyn Input + Send> {
    Box::new(Gone {
        info: DeviceInfo {
            path: node.into(),
            by_id: by_id.iter().map(Into::into).collect(),
            name: name.to_owned(),
            relative_xy: true,
            readable: true,
            ..DeviceInfo::default()
        },
        now: 0,
        unplugged: false,
    })
}

/// Connects `driver` once, through its opener.
fn connect(driver: &mut Thumbswing) -> Result<Session, Error> {
    let flags = HostFlags::new();
    let stop = Stop::new();
    let mut host = Host(DeviceCommand::default());
    let mut sink = |_| {};
    let mut ctx = ConnectCtx::new(&stop, Events::new(&mut sink, &flags), &mut host);
    driver.connect(&mut ctx)
}

fn ready_values(events: &[DeviceEvent]) -> Vec<String> {
    frp(events)
        .iter()
        .filter_map(|e| match e {
            FrpEvent::DeviceTelemetry {
                telemetry: Some(t), ..
            } => t.get("ready").cloned(),
            _ => None,
        })
        .collect()
}

#[test]
fn with_no_device_named_an_unplug_chooses_the_device_again() {
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = std::sync::Arc::clone(&opened);
    let mut driver = Thumbswing::with_opener(Settings::default(), move |device, grab| {
        let mut log = log.lock().unwrap();
        log.push((device.map(str::to_owned), grab));
        // The connect opens it; the first reopen finds nothing, as if it
        // were still away; the next one finds it under its new node.
        if log.len() == 2 {
            Err(Error::NotFound("nothing yet".into()))
        } else {
            Ok(gone("Ball"))
        }
    })
    .unwrap();
    let flags = HostFlags::new();
    let stop = Stop::new();
    let mut host = Host(DeviceCommand::default());
    let mut sink = |_| {};
    let mut ctx = ConnectCtx::new(&stop, Events::new(&mut sink, &flags), &mut host);
    let mut session = driver.connect(&mut ctx).unwrap();
    let events = play(&mut session, &flags, 200);
    assert_eq!(ready_values(&events)[..3], ["true", "false", "true"]);
    let opened = opened.lock().unwrap();
    assert!(opened.len() >= 3, "{opened:?}");
    assert!(
        opened
            .iter()
            .all(|(device, grab)| device.is_none() && *grab),
        "chosen again, with the grab: {opened:?}"
    );
}

#[test]
fn a_named_device_is_reopened_by_its_name_not_chosen_again() {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let count = std::sync::Arc::clone(&calls);
    let settings = Settings {
        device: Some("usb-ball".into()),
        ..Settings::default()
    };
    let mut driver = Thumbswing::with_opener(settings, move |device, _| {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(device, Some("usb-ball"));
        Ok(gone("Ball"))
    })
    .unwrap();
    let flags = HostFlags::new();
    let stop = Stop::new();
    let mut host = Host(DeviceCommand::default());
    let mut sink = |_| {};
    let mut ctx = ConnectCtx::new(&stop, Events::new(&mut sink, &flags), &mut host);
    let mut session = driver.connect(&mut ctx).unwrap();
    let events = play(&mut session, &flags, 200);
    assert_eq!(ready_values(&events), ["true", "false"]);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

const LINK: &str = "/dev/input/by-id/usb-Example_Trackball_SERIAL123-event-mouse";
const LINK_NAME: &str = "usb-Example_Trackball_SERIAL123-event-mouse";

#[test]
fn the_device_id_is_the_by_id_link_name() {
    let mut driver = Thumbswing::with_opener(Settings::default(), |_, _| {
        Ok(gone_at(
            "/dev/input/event7",
            &[
                LINK,
                "/dev/input/by-id/usb-Example_Trackball_SERIAL123-if01-event-kbd",
            ],
            "Example Trackball",
        ))
    })
    .unwrap();
    let mut session = connect(&mut driver).unwrap();
    let identity = session.identity();
    assert_eq!(identity.device, LINK_NAME, "the first link, file name only");
    assert_eq!(identity.label(), "thumbswing (Example Trackball)");
    let events = play(&mut session, &HostFlags::new(), 1);
    assert_eq!(telemetry(&frp(&events))["input"], "Example Trackball");
}

#[test]
fn without_a_by_id_link_the_device_id_is_the_reported_name() {
    let mut driver = Thumbswing::with_opener(Settings::default(), |_, _| {
        Ok(gone_at("/dev/input/event7", &[], "Built-in Pointer"))
    })
    .unwrap();
    let session = connect(&mut driver).unwrap();
    assert_eq!(session.identity().device, "Built-in Pointer");

    // A link with no file name is no id either.
    let mut driver = Thumbswing::with_opener(Settings::default(), |_, _| {
        Ok(gone_at("/dev/input/event7", &["/"], "Built-in Pointer"))
    })
    .unwrap();
    let session = connect(&mut driver).unwrap();
    assert_eq!(session.identity().device, "Built-in Pointer");
}

#[test]
fn the_device_id_is_the_same_after_a_replug() {
    // The device opens at event7, is unplugged, comes back at event9 (the
    // session reopens it; it keeps being unplugged and found again), and a
    // later session finds it at event12; its `by-id` link is the same each
    // time.
    let opened = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let count = std::sync::Arc::clone(&opened);
    let mut driver = Thumbswing::with_opener(Settings::default(), move |_, _| {
        let node = match count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
            0 => "/dev/input/event7",
            1 => "/dev/input/event9",
            _ => "/dev/input/event12",
        };
        Ok(gone_at(node, &[LINK], "Example Trackball"))
    })
    .unwrap();
    let mut session = connect(&mut driver).unwrap();
    assert_eq!(session.identity().device, LINK_NAME);
    let events = play(&mut session, &HostFlags::new(), 200);
    assert_eq!(ready_values(&events)[..3], ["true", "false", "true"]);
    let reopened = opened.load(std::sync::atomic::Ordering::SeqCst);
    assert!(reopened >= 2, "{reopened}");
    assert_eq!(session.identity().device, LINK_NAME);
    drop(session);

    let session = connect(&mut driver).unwrap();
    assert_eq!(
        opened.load(std::sync::atomic::Ordering::SeqCst),
        reopened + 1
    );
    assert_eq!(session.identity().device, LINK_NAME);

    // Named by its link, it gets the same id.
    let settings = Settings {
        device: Some(LINK_NAME.into()),
        ..Settings::default()
    };
    let mut driver = Thumbswing::with_opener(settings, |_, _| {
        Ok(gone_at("/dev/input/event4", &[LINK], "Example Trackball"))
    })
    .unwrap();
    assert_eq!(connect(&mut driver).unwrap().identity().device, LINK_NAME);
}
