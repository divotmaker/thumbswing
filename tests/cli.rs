//! The command line of `examples/monitor`: settings flags over the crate
//! defaults, the choice of device, and the ranges against the library's.

#[path = "../examples/common/cli.rs"]
mod cli;
#[path = "../examples/common/devices.rs"]
mod devices;

use std::path::PathBuf;

use cli::{Opt, ParseError, Settings};
use thumbswing::{Config, CurveSense, DeviceInfo, Hand, Thresholds, Tuning};

const OPTS: &[Opt] = &[
    Opt {
        name: "--port",
        meta: Some("PORT"),
        help: "port",
    },
    Opt {
        name: "--fast",
        meta: None,
        help: "fast",
    },
];

fn parse(args: &[&str]) -> Result<cli::Parsed, ParseError> {
    cli::parse(args.iter().map(|a| (*a).to_owned()), OPTS)
}

fn invalid(args: &[&str]) -> String {
    match parse(args) {
        Err(ParseError::Invalid(e)) => e,
        other => panic!("{args:?} should be rejected, got {other:?}"),
    }
}

#[test]
fn no_flags_give_the_crate_defaults() {
    let p = parse(&["serve"]).unwrap();
    assert_eq!(p.settings, Settings::default());
    assert_eq!(p.settings.config, Config::default());
    assert_eq!(p.settings.tuning, Tuning::default());
    assert!(p.settings.config.grab, "grab is on by default");
    assert_eq!(p.positional, ["serve"]);
    assert!(!p.given("--cpi"));
    assert!(!p.flag("--fast"));
    assert_eq!(p.value("--port"), None);
}

/// Every flag with a value that differs from its default, and the settings
/// they make. The struct literals name every field, so a new `Config`,
/// `Thresholds` or `Tuning` field fails to compile here until it has a flag.
fn every_flag() -> (Vec<(&'static str, Option<&'static str>)>, Settings) {
    let flags = vec![
        ("--cpi", Some("800")),
        ("--report-limit", Some("2047")),
        ("--no-grab", None),
        ("--speed-window-ms", Some("30")),
        ("--gap-ms", Some("60")),
        ("--v-start", Some("0.08")),
        ("--v-stop", Some("0.04")),
        ("--v-min-swing", Some("0.15")),
        ("--dwell-min-ms", Some("50")),
        ("--back-min", Some("0.2")),
        ("--full-cock", Some("0.8")),
        ("--fwd-min", Some("0.25")),
        ("--back-max-s", Some("3")),
        ("--coil-timeout-s", Some("12")),
        ("--fwd-window-ms", Some("400")),
        ("--idle-settle-ms", Some("300")),
        ("--cooldown-max-s", Some("2.5")),
        ("--pull-cone-deg", Some("60")),
        ("--full-power-speed", Some("40")),
        ("--max-club-speed", Some("110")),
        ("--max-putt-speed", Some("9")),
        ("--angle-gain", Some("0.3")),
        ("--putt-angle-gain", Some("1.2")),
        ("--curve-gain", Some("0.35")),
        ("--curve-sense", Some("physical")),
        ("--backswing-power", Some("0.5")),
        ("--power-gamma", Some("1.5")),
        ("--mirror-for-left-hand", None),
        ("--face-max", Some("15")),
        ("--face-to-path-max", Some("20")),
    ];
    let settings = Settings {
        config: Config {
            cpi: 800.0,
            report_limit: 2047,
            grab: false,
            thresholds: Thresholds {
                speed_window_ms: 30,
                gap_ms: 60,
                v_start: 0.08,
                v_stop: 0.04,
                v_min_swing: 0.15,
                dwell_min_ms: 50,
                back_min: 0.2,
                full_cock: 0.8,
                fwd_min: 0.25,
                back_max_s: 3.0,
                coil_timeout_s: 12.0,
                fwd_window_ms: 400,
                idle_settle_ms: 300,
                cooldown_max_s: 2.5,
                pull_cone_deg: 60.0,
            },
        },
        tuning: Tuning {
            full_power_speed: 40.0,
            max_club_speed: 110.0,
            max_putt_speed: 9.0,
            angle_gain: 0.3,
            putt_angle_gain: 1.2,
            curve_gain: 0.35,
            curve_sense: CurveSense::Physical,
            backswing_power: 0.5,
            power_gamma: 1.5,
            mirror_for_left_hand: true,
            face_max: 15.0,
            face_to_path_max: 20.0,
        },
    };
    (flags, settings)
}

#[test]
fn every_flag_sets_its_field() {
    let (flags, expected) = every_flag();
    let mut names: Vec<&str> = flags.iter().map(|(n, _)| *n).collect();
    let mut known = cli::flag_names();
    names.sort_unstable();
    known.sort_unstable();
    assert_eq!(names, known, "the test covers every settings flag");

    // All at once, `--flag value` form.
    let mut args = vec!["serve"];
    for (name, value) in &flags {
        args.push(name);
        args.extend(value);
    }
    let p = parse(&args).unwrap();
    assert_eq!(p.settings, expected);

    // All at once, `--flag=value` form.
    let joined: Vec<String> = flags
        .iter()
        .map(|(n, v)| v.map_or_else(|| (*n).to_owned(), |v| format!("{n}={v}")))
        .collect();
    let p = cli::parse(joined, OPTS).unwrap();
    assert_eq!(p.settings, expected);

    // One at a time: each flag moves the settings off the defaults and is
    // reported as given; nothing else is.
    for (name, value) in &flags {
        let mut args = vec![*name];
        args.extend(value);
        let p = parse(&args).unwrap();
        assert_ne!(p.settings, Settings::default(), "{name} changed nothing");
        assert!(p.given(name), "{name}");
        for other in cli::flag_names().into_iter().filter(|o| o != name) {
            assert!(!p.given(other), "{name} also marked {other}");
        }
    }
}

#[test]
fn single_flags_set_only_their_field() {
    let p = parse(&["--full-power-speed", "45"]).unwrap();
    assert_eq!(
        p.settings.tuning,
        Tuning {
            full_power_speed: 45.0,
            ..Tuning::default()
        }
    );
    assert_eq!(p.settings.config, Config::default());

    let p = parse(&["--v-start", "0.1"]).unwrap();
    assert_eq!(
        p.settings.config.thresholds,
        Thresholds {
            v_start: 0.1,
            ..Thresholds::default()
        }
    );

    let p = parse(&["--curve-sense", "ARCADE", "--curve-sense", "physical"]).unwrap();
    assert_eq!(
        p.settings.tuning.curve_sense,
        CurveSense::Physical,
        "the last one wins"
    );
}

#[test]
fn invalid_values_are_rejected_with_the_flag_named() {
    let cases: &[(&[&str], &str)] = &[
        (&["--cpi", "abc"], "--cpi: \"abc\" is not a number"),
        (&["--cpi", "0"], "--cpi: \"0\" must be greater than 0"),
        (&["--cpi", "NaN"], "--cpi: \"NaN\" is not a finite number"),
        (&["--full-power-speed", "inf"], "is not a finite number"),
        (&["--full-power-speed", "-5"], "must be greater than 0"),
        (
            &["--angle-gain", "-0.1"],
            "--angle-gain: \"-0.1\" must be 0 or more",
        ),
        (&["--backswing-power", "1.5"], "must be between 0 and 1"),
        (&["--pull-cone-deg", "270"], "must be between 1 and 180"),
        (
            &["--report-limit", "0"],
            "--report-limit: \"0\" must be at least 1",
        ),
        (&["--report-limit", "12.5"], "is not a whole number"),
        (&["--speed-window-ms", "0"], "must be at least 1"),
        (&["--dwell-min-ms", "-1"], "is not a whole number"),
        (&["--curve-sense", "sideways"], "is not arcade or physical"),
        (&["--cpi"], "--cpi needs a value (N)"),
        (&["--no-grab=yes"], "--no-grab takes no value"),
        (&["--fast=1"], "--fast takes no value"),
        (&["--port"], "--port needs a value (PORT)"),
        (&["--bogus"], "unknown option --bogus"),
        (
            &["--v-stop", "0.1"],
            "--v-stop (0.1) must be below --v-start (0.067)",
        ),
    ];
    for (args, want) in cases {
        let e = invalid(args);
        assert!(e.contains(want), "{args:?}: {e:?} lacks {want:?}");
    }
}

#[test]
fn help_lists_every_flag_with_its_default() {
    assert_eq!(parse(&["serve", "--help"]).unwrap_err(), ParseError::Help);
    assert_eq!(parse(&["-h"]).unwrap_err(), ParseError::Help);
    let text = cli::help("usage: test", OPTS);
    for name in cli::flag_names().into_iter().chain(["--port", "--fast"]) {
        let line = text
            .lines()
            .find(|l| l.trim_start().starts_with(name))
            .unwrap_or_else(|| panic!("{name} missing from --help"));
        if !matches!(name, "--port" | "--fast") {
            assert!(line.contains("[default: "), "{line}");
        }
    }
    assert!(text.contains("--full-power-speed X"), "{text}");
    assert!(text.contains("[default: 30 in/s]"), "{text}");
    assert!(
        text.contains("[default: 0.067 × full-power speed]"),
        "{text}"
    );
    assert!(text.contains("[default: arcade]"), "{text}");
}

#[test]
fn program_options_are_kept_apart_from_settings() {
    let p = parse(&["serve", "--port", "5990", "--fast", "--cpi", "1600"]).unwrap();
    assert_eq!(p.value("--port"), Some("5990"));
    assert!(p.flag("--fast"));
    assert!(p.given("--cpi"));
    assert!((p.settings.config.cpi - 1600.0).abs() < f64::EPSILON);
    assert_eq!(p.positional, ["serve"]);
}

#[test]
fn hand_values() {
    assert_eq!(cli::parse_hand("lh"), Ok(Hand::Left));
    assert_eq!(cli::parse_hand("RH"), Ok(Hand::Right));
    assert!(cli::parse_hand("left").is_err());
}

fn pointer(node: &str, by_id: &str, name: &str, readable: bool) -> DeviceInfo {
    DeviceInfo {
        path: PathBuf::from(node),
        by_id: vec![PathBuf::from(by_id)],
        name: name.to_owned(),
        relative_xy: true,
        readable,
        ..DeviceInfo::default()
    }
}

fn keyboard() -> DeviceInfo {
    DeviceInfo {
        path: PathBuf::from("/dev/input/event2"),
        name: "Keyboard".to_owned(),
        readable: true,
        ..DeviceInfo::default()
    }
}

#[test]
fn more_than_one_device_is_refused_without_device() {
    let ball = pointer(
        "/dev/input/event5",
        "/dev/input/by-id/usb-Arcade_Ball-event-mouse",
        "Arcade Ball",
        true,
    );
    let mouse = pointer(
        "/dev/input/event7",
        "/dev/input/by-id/usb-Thumb_Ball-event-mouse",
        "Thumb Ball",
        true,
    );
    let all = [keyboard(), ball.clone(), mouse];
    let e = devices::choose(&all).unwrap_err();
    assert!(e.contains("found 2 trackballs/mice"), "{e}");
    assert!(e.contains("--device"), "{e}");
    for listed in [
        "/dev/input/event5",
        "usb-Arcade_Ball-event-mouse",
        "/dev/input/event7",
        "usb-Thumb_Ball-event-mouse",
    ] {
        assert!(e.contains(listed), "{listed} not listed: {e}");
    }
    assert!(!e.contains("Keyboard"), "only pointers are listed: {e}");
    // The rows are the ones `examples/list` prints.
    assert!(e.contains(devices::table([&ball]).trim_end()), "{e}");
}

#[test]
fn exactly_one_readable_device_is_used() {
    let ball = pointer(
        "/dev/input/event5",
        "/dev/input/by-id/usb-Arcade_Ball-event-mouse",
        "Arcade Ball",
        true,
    );
    let locked = pointer(
        "/dev/input/event9",
        "/dev/input/by-id/usb-Other-event-mouse",
        "Other",
        false,
    );
    assert_eq!(
        devices::choose(&[keyboard(), ball, locked.clone()]).unwrap(),
        PathBuf::from("/dev/input/by-id/usb-Arcade_Ball-event-mouse")
    );

    let e = devices::choose(&[keyboard(), locked]).unwrap_err();
    assert!(e.contains("cannot read them"), "{e}");
    let e = devices::choose(&[keyboard()]).unwrap_err();
    assert!(e.contains("no trackball or mouse found"), "{e}");
}

/// The example's flag ranges and the library's `Settings::validate` refuse
/// and accept the same values.
#[test]
fn flag_ranges_agree_with_the_library() {
    let cases: &[(&str, &str, &str)] = &[
        ("--cpi", "0", "cpi"),
        ("--report-limit", "0", "report_limit"),
        ("--full-power-speed", "0", "tuning.full_power_speed"),
        ("--max-club-speed", "-1", "tuning.max_club_speed"),
        ("--max-putt-speed", "0", "tuning.max_putt_speed"),
        ("--angle-gain", "-0.1", "tuning.angle_gain"),
        ("--putt-angle-gain", "-1", "tuning.putt_angle_gain"),
        ("--curve-gain", "-1", "tuning.curve_gain"),
        ("--backswing-power", "1.5", "tuning.backswing_power"),
        ("--power-gamma", "0", "tuning.power_gamma"),
        ("--face-max", "-1", "tuning.face_max"),
        ("--face-to-path-max", "-1", "tuning.face_to_path_max"),
        ("--speed-window-ms", "0", "thresholds.speed_window_ms"),
        ("--gap-ms", "0", "thresholds.gap_ms"),
        ("--v-start", "0", "thresholds.v_start"),
        ("--v-stop", "0", "thresholds.v_stop"),
        ("--v-min-swing", "0", "thresholds.v_min_swing"),
        ("--back-min", "-1", "thresholds.back_min"),
        ("--full-cock", "0", "thresholds.full_cock"),
        ("--fwd-min", "0", "thresholds.fwd_min"),
        ("--back-max-s", "0", "thresholds.back_max_s"),
        ("--coil-timeout-s", "0", "thresholds.coil_timeout_s"),
        ("--fwd-window-ms", "0", "thresholds.fwd_window_ms"),
        ("--cooldown-max-s", "0", "thresholds.cooldown_max_s"),
        ("--pull-cone-deg", "0.5", "thresholds.pull_cone_deg"),
    ];
    // 1 is inside every range but v_stop's, which must stay below v_start.
    let inside = |flag: &str| if flag == "--v-stop" { "0.01" } else { "1" };
    for (flag, bad, field) in cases {
        invalid(&[flag, bad]);
        let p = parse(&[flag, inside(flag)]).unwrap_or_else(|e| panic!("{flag}: {e:?}"));
        let mut settings = thumbswing::Settings {
            config: p.settings.config.clone(),
            tuning: p.settings.tuning,
            ..thumbswing::Settings::default()
        };
        settings
            .validate()
            .unwrap_or_else(|e| panic!("{flag} {}: {e}", inside(flag)));
        // The same out-of-range value, set directly, is refused by the library.
        let value: f64 = bad.parse().unwrap();
        set_field(&mut settings, field, value);
        let e = settings.validate().unwrap_err();
        assert_eq!(e.field, *field, "{flag}");
    }
    let mut stopped = thumbswing::Settings::default();
    stopped.config.thresholds.v_stop = 0.1;
    assert_eq!(stopped.validate().unwrap_err().field, "thresholds.v_stop");
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn set_field(s: &mut thumbswing::Settings, field: &str, v: f64) {
    let (c, t) = (&mut s.config, &mut s.tuning);
    match field {
        "cpi" => c.cpi = v,
        "report_limit" => c.report_limit = v as i32,
        "tuning.full_power_speed" => t.full_power_speed = v,
        "tuning.max_club_speed" => t.max_club_speed = v,
        "tuning.max_putt_speed" => t.max_putt_speed = v,
        "tuning.angle_gain" => t.angle_gain = v,
        "tuning.putt_angle_gain" => t.putt_angle_gain = v,
        "tuning.curve_gain" => t.curve_gain = v,
        "tuning.backswing_power" => t.backswing_power = v,
        "tuning.power_gamma" => t.power_gamma = v,
        "tuning.face_max" => t.face_max = v,
        "tuning.face_to_path_max" => t.face_to_path_max = v,
        "thresholds.speed_window_ms" => c.thresholds.speed_window_ms = v as u32,
        "thresholds.gap_ms" => c.thresholds.gap_ms = v as u32,
        "thresholds.v_start" => c.thresholds.v_start = v,
        "thresholds.v_stop" => c.thresholds.v_stop = v,
        "thresholds.v_min_swing" => c.thresholds.v_min_swing = v,
        "thresholds.back_min" => c.thresholds.back_min = v,
        "thresholds.full_cock" => c.thresholds.full_cock = v,
        "thresholds.fwd_min" => c.thresholds.fwd_min = v,
        "thresholds.back_max_s" => c.thresholds.back_max_s = v,
        "thresholds.coil_timeout_s" => c.thresholds.coil_timeout_s = v,
        "thresholds.fwd_window_ms" => c.thresholds.fwd_window_ms = v as u32,
        "thresholds.cooldown_max_s" => c.thresholds.cooldown_max_s = v,
        "thresholds.pull_cone_deg" => c.thresholds.pull_cone_deg = v,
        other => panic!("no field {other}"),
    }
}
