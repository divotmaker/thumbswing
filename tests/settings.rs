//! `Settings` as a flightrelay driver config: defaults that write nothing,
//! optional and nested keys, validation errors that name the field.

use flightrelay::config::{ConfigError, DriverConfig};
use serde_json::{Map, Value, json};
use thumbswing::{Club, CurveSense, Hand, Settings};

fn map(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn read(v: &Value) -> Result<Settings, ConfigError> {
    Settings::from_map(map(v))
}

#[test]
fn a_default_config_writes_nothing_and_reads_back() {
    assert!(Settings::default().to_map().is_empty());
    assert_eq!(read(&json!({})).unwrap(), Settings::default());
    // Defaults the crate depends on: grab on (the standalone binary), 400 CPI.
    let d = Settings::default();
    assert!(d.config.grab);
    assert!((d.config.cpi - 400.0).abs() < f64::EPSILON);
    assert_eq!(d.device(), None);
    // The 7-iron, right-handed, until a setting or a host says otherwise.
    assert_eq!((d.club, d.hand), (Club::Iron7, Hand::Right));
    assert_eq!(
        read(&json!({ "club": "7I", "hand": "rh" })).unwrap(),
        Settings::default()
    );
}

#[test]
fn club_and_hand_are_set_by_code() {
    // What `--set club=PW --set hand=lh` produces.
    let s = read(&json!({ "club": "PW", "hand": "lh" })).unwrap();
    assert_eq!((s.club, s.hand), (Club::PitchingWedge, Hand::Left));
    assert_eq!(
        Value::Object(s.to_map()),
        json!({ "club": "PW", "hand": "lh" })
    );
    // Any FRP code, any case; a code without a row reads as the nearest.
    assert_eq!(read(&json!({ "club": "dr" })).unwrap().club, Club::Driver);
    assert_eq!(read(&json!({ "club": "5H" })).unwrap().club, Club::Hybrid3);
    assert_eq!(read(&json!({ "hand": "LH" })).unwrap().hand, Hand::Left);
}

#[test]
fn changed_fields_are_written_alone_and_read_back() {
    let mut s = Settings {
        device: Some("usb-Example_Trackball-event-mouse".to_owned()),
        ..Settings::default()
    };
    s.config.cpi = 800.0;
    s.config.grab = false;
    s.config.thresholds.gap_ms = 60;
    s.tuning.curve_sense = CurveSense::Physical;
    s.tuning.full_power_speed = 40.0;
    let written = Value::Object(s.to_map());
    assert_eq!(
        written,
        json!({
            "device": "usb-Example_Trackball-event-mouse",
            "cpi": 800.0,
            "grab": false,
            "thresholds": { "gap_ms": 60 },
            "tuning": { "curve_sense": "physical", "full_power_speed": 40.0 },
        })
    );
    assert_eq!(read(&written).unwrap(), s);
}

#[test]
fn optional_and_nested_keys_are_set_on_their_own() {
    // What `--set device=…` and `--set tuning.power_gamma=2` produce; a TOML
    // integer reads into a decimal field.
    let s = read(&json!({ "device": "event-ball", "cpi": 1600, "tuning": { "power_gamma": 2 } }))
        .unwrap();
    assert_eq!(s.device(), Some("event-ball"));
    assert!((s.config.cpi - 1600.0).abs() < f64::EPSILON);
    assert!((s.tuning.power_gamma - 2.0).abs() < f64::EPSILON);
    assert_eq!(s.tuning, {
        let mut t = Settings::default().tuning;
        t.power_gamma = 2.0;
        t
    });
    // A blank device means "the only one found".
    assert_eq!(read(&json!({ "device": "  " })).unwrap().device(), None);
    // Keys the config does not know are ignored, as flighthook's sections do.
    assert_eq!(
        read(&json!({ "name": "Trackball", "bogus": 1 })).unwrap(),
        Settings::default()
    );
}

#[test]
fn invalid_values_name_the_field() {
    let cases = [
        (json!({ "cpi": 0 }), "cpi", "must be greater than 0"),
        (
            json!({ "report_limit": 0 }),
            "report_limit",
            "must be at least 1",
        ),
        (
            json!({ "tuning": { "backswing_power": 1.5 } }),
            "tuning.backswing_power",
            "must be 0 to 1",
        ),
        (
            json!({ "tuning": { "angle_gain": -0.1 } }),
            "tuning.angle_gain",
            "must be 0 or more",
        ),
        (
            json!({ "thresholds": { "pull_cone_deg": 270 } }),
            "thresholds.pull_cone_deg",
            "must be 1 to 180",
        ),
        (
            json!({ "thresholds": { "speed_window_ms": 0 } }),
            "thresholds.speed_window_ms",
            "must be at least 1",
        ),
        (
            json!({ "thresholds": { "v_stop": 0.1 } }),
            "thresholds.v_stop",
            "must be below thresholds.v_start (0.067)",
        ),
    ];
    for (value, field, message) in cases {
        let e = read(&value).unwrap_err();
        assert_eq!(e.field.as_deref(), Some(field), "{value}: {e}");
        assert_eq!(e.message, message, "{value}");
    }
    // A wrong type is refused by serde, naming the key.
    let e = read(&json!({ "tuning": { "curve_sense": "sideways" } })).unwrap_err();
    assert!(e.to_string().contains("sideways"), "{e}");
    // An unknown club or hand names its key.
    let e = read(&json!({ "club": "ZZ" })).unwrap_err();
    assert_eq!(e.field.as_deref(), Some("club"), "{e}");
    assert!(e.message.contains("ZZ"), "{e}");
    let e = read(&json!({ "hand": "both" })).unwrap_err();
    assert_eq!(e.field.as_deref(), Some("hand"), "{e}");
    assert!(e.message.contains("rh or lh"), "{e}");
    let e = read(&json!({ "club": 7 })).unwrap_err();
    assert_eq!(e.field.as_deref(), Some("club"), "{e}");
}

#[test]
fn device_fields_restart_and_the_rest_applies_live() {
    let base = Settings::default();
    let changed = |f: fn(&mut Settings)| {
        let mut s = base.clone();
        f(&mut s);
        Settings::restart_needed(&base, &s)
    };
    assert!(changed(|s| s.device = Some("event-ball".into())));
    assert!(changed(|s| s.config.cpi = 800.0));
    assert!(changed(|s| s.config.report_limit = 2047));
    assert!(changed(|s| s.config.grab = false));
    assert!(!changed(|s| s.config.thresholds.v_start = 0.1));
    assert!(!changed(|s| s.tuning.curve_gain = 0.3));
    assert!(!changed(|s| s.club = Club::Driver));
    assert!(!changed(|s| s.hand = Hand::Left));
    assert!(
        !changed(|s| s.device = Some(String::new())),
        "blank is unset"
    );
    // The trait method is the same rule.
    let mut s = base.clone();
    s.config.cpi = 800.0;
    assert!(<Settings as DriverConfig>::restart_needed(&base, &s));
}

#[test]
fn keys_stay_clear_of_a_host_sections_own() {
    // A host such as flighthook embeds the config flattened next to its own
    // `name`, `enabled` and `impact_lead_s`.
    let mut every = Settings::example();
    every.config.cpi = 1.0;
    every.config.report_limit = 1;
    every.config.grab = false;
    every.config.thresholds.gap_ms = 1;
    every.tuning.face_max = 1.0;
    every.club = Club::Driver;
    every.hand = Hand::Left;
    let keys = every.to_map();
    for key in [
        "device",
        "club",
        "hand",
        "cpi",
        "report_limit",
        "grab",
        "thresholds",
        "tuning",
    ] {
        assert!(keys.contains_key(key), "{key}");
    }
    for host in ["name", "enabled", "impact_lead_s"] {
        assert!(!keys.contains_key(host), "{host}");
    }
}

#[cfg(feature = "frp")]
#[test]
fn help_lists_every_key_with_its_default() {
    let help = Settings::help_toml();
    for line in [
        "# device = (unset)",
        "club = \"7I\"",
        "hand = \"rh\"",
        "cpi = 400.0",
        "grab = true",
        "report_limit = 127",
        "[thresholds]",
        "v_start = 0.067",
        "[tuning]",
        "curve_sense = \"arcade\"",
        "mirror_for_left_hand = false",
    ] {
        assert!(
            help.lines().any(|l| l == line),
            "{line} missing from\n{help}"
        );
    }
}

/// `thumbswing-frp --set`: the club and hand are known keys, and a bad value
/// is refused with its key before anything is opened.
#[cfg(feature = "frp")]
#[test]
fn the_binary_takes_club_and_hand_with_set() {
    let run = |args: &[&str]| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_thumbswing-frp"))
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (code, err) = run(&["--set", "club=ZZ"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("club:") && err.contains("ZZ"), "{err}");
    let (code, err) = run(&["--set", "hand=both"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("hand:"), "{err}");
    // Valid club and hand pass; the invalid cpi after them is what stops it.
    let (code, err) = run(&["--set", "club=PW", "--set", "hand=lh", "--set", "cpi=0"]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("cpi: must be greater than 0"), "{err}");
    assert!(!err.contains("club") && !err.contains("hand"), "{err}");
}
