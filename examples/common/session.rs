//! Argument parsing for the smaller examples, and the event and status text
//! shared by the examples.

#![allow(dead_code)]

use std::fmt::Write as _;

use thumbswing::{CancelReason, Client, Event, Hand, Input, Shot};

/// Command-line arguments: positionals plus `--flag` and `--key value`.
pub struct Args {
    pub positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    /// Parse `std::env::args()`; the names in `with_value` take a value.
    pub fn parse(with_value: &[&str]) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = std::env::args().skip(1);
        while let Some(a) = it.next() {
            if a.starts_with("--") {
                if with_value.contains(&a.as_str()) {
                    let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
                    flags.push((a, Some(v)));
                } else {
                    flags.push((a, None));
                }
            } else {
                positional.push(a);
            }
        }
        Ok(Self { positional, flags })
    }

    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == name)
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.as_deref())
    }

    pub fn number(&self, name: &str) -> Result<Option<f64>, String> {
        self.value(name)
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|_| format!("{name}: not a number: {v}"))
            })
            .transpose()
    }
}

/// A log line for an event, `None` for events not worth a line.
pub fn event_line(e: &Event) -> Option<String> {
    Some(match e {
        Event::Ready => "ready: pull back, then push".to_owned(),
        Event::Backswing { .. } => return None,
        Event::Saturated { reports } => format!(
            "saturated: {reports} reports at the per-report limit; lower --full-power-speed, \
             or set --report-limit to the device's real limit"
        ),
        Event::Shot(shot) => launch_line(shot),
        Event::Canceled(CancelReason::Dribble) => {
            "dribble ignored (the backswing is still coiled)".to_owned()
        }
        Event::Canceled(r) => format!("canceled: {r:?}"),
        Event::Disconnected => "device disconnected; waiting for it to come back".to_owned(),
    })
}

fn signed(v: f64) -> String {
    format!("{v:+.1}")
}

/// One status line: the settings that shape a shot, the club and hand, and
/// the last shot.
///
/// `full 30.0 in/s | max 100 mph | angle 0.25 | curve 0.20 arcade | β 0.00 γ 1.0 | 7I RH | last 7I: back +12.0° 1.4 in, fwd −3.0° 27.1 in/s → 90 % 72 mph, path +1.1 face −0.8`
pub fn status_line<I: Input>(client: &Client<I>, last: Option<&Shot>) -> String {
    let t = client.tuning();
    let mut s = String::new();
    let _ = write!(s, "full {:.1} in/s | ", t.full_power_speed);
    if client.club().is_putter() {
        let _ = write!(
            s,
            "putt {:.0} mph | line {:.2} | ",
            t.max_putt_speed, t.putt_angle_gain
        );
    } else {
        let _ = write!(
            s,
            "max {:.0} mph | angle {:.2} | curve {:.2} {} | ",
            t.max_club_speed,
            t.angle_gain,
            t.curve_gain,
            t.curve_sense.label()
        );
    }
    let _ = write!(
        s,
        "β {:.2} γ {:.1}{} | {} {}",
        t.backswing_power,
        t.power_gamma,
        if t.mirror_for_left_hand && client.hand() == Hand::Left {
            " mirror"
        } else {
            ""
        },
        client.club(),
        client.hand()
    );
    if !client.config().grab {
        s.push_str(" | grab off");
    }
    if let Some(shot) = last {
        let w = &shot.swing;
        let d = &shot.delivery;
        let _ = write!(
            s,
            " | last {}: back {}° {:.1} in, fwd {}° {:.1} in/s → {:.0} % {:.0} mph, path {} face {}{}",
            shot.club,
            signed(w.back_dir_deg),
            w.back_len_in,
            signed(w.fwd_dir_deg),
            w.fwd_peak_in_s,
            shot.power * 100.0,
            d.club_speed_mph,
            signed(d.path_deg),
            signed(d.face_deg),
            if w.saturated() { " SATURATED" } else { "" }
        );
    }
    s
}

/// One line describing a shot's launch, for logs.
pub fn launch_line(shot: &Shot) -> String {
    let l = &shot.launch;
    format!(
        "shot {} {}: ball {:.1} mph, launch {:.1}° / {}°, spin {:.0} back {:.0} side (axis {}°), smash {:.3}",
        shot.number,
        shot.club,
        l.ball_speed_mph,
        l.launch_angle_deg,
        signed(l.launch_azimuth_deg),
        l.backspin_rpm,
        l.sidespin_rpm,
        signed(l.spin_axis_deg()),
        l.smash_factor
    )
}
