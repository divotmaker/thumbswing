//! A thumbswing [`Shot`] as FRP domain types.

use flightrelay::types::{ClubData, Launch as FrpLaunch};
use flightrelay::units::Velocity;

use crate::client::Shot;
use crate::launch::Launch;

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// The FRP launch (`shot.launch`) for a launch: speed, elevation, azimuth and both spin
/// pairs (the components come from the model, total and axis are completed
/// from them; a shot without spin has a total of 0 and no axis). There is
/// no `shot.flight`: a game or flight model flies the ball from the launch.
#[must_use]
// The spins are rounded rpm, far inside `i32`.
#[allow(clippy::cast_possible_truncation)]
pub fn launch(l: &Launch) -> FrpLaunch {
    let mut out = FrpLaunch {
        launch_speed: Some(Velocity::MilesPerHour(round1(l.ball_speed_mph))),
        launch_elevation: Some(round2(l.launch_angle_deg)),
        launch_azimuth: Some(round2(l.launch_azimuth_deg)),
        backspin_rpm: Some(l.backspin_rpm.round() as i32),
        sidespin_rpm: Some(l.sidespin_rpm.round() as i32),
        ..FrpLaunch::default()
    };
    out.complete_spin();
    out.spin_axis = out.spin_axis.map(round2);
    out
}

/// The FRP club data (`shot.club`) for a shot: club speed, path, face, attack angle,
/// dynamic loft and smash factor. The swing plane is the path (a trackball
/// has no separate plane); spin loft is not sent, since the spec reserves it
/// for a measured 3-D value.
#[must_use]
pub fn club_data(shot: &Shot) -> ClubData {
    let d = &shot.delivery;
    ClubData {
        club_speed: Some(Velocity::MilesPerHour(round1(d.club_speed_mph))),
        path: Some(round2(d.path_deg)),
        face_angle: Some(round2(d.face_deg)),
        attack_angle: Some(round2(d.attack_deg)),
        dynamic_loft: Some(round2(d.dynamic_loft_deg)),
        smash_factor: Some((shot.launch.smash_factor * 1000.0).round() / 1000.0),
        swing_plane_horizontal: Some(round2(d.path_deg)),
        ..ClubData::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spin_pairs_are_both_sent() {
        let l = Launch {
            ball_speed_mph: 148.23,
            launch_angle_deg: 10.4,
            launch_azimuth_deg: 0.86,
            backspin_rpm: 2206.4,
            sidespin_rpm: -981.2,
            spin_loft_deg: 14.6,
            smash_factor: 1.482,
        };
        let b = launch(&l);
        assert_eq!(b.launch_speed, Some(Velocity::MilesPerHour(148.2)));
        assert_eq!(b.launch_elevation, Some(10.4));
        assert_eq!(b.launch_azimuth, Some(0.86));
        assert_eq!(b.backspin_rpm, Some(2206));
        assert_eq!(b.sidespin_rpm, Some(-981));
        assert_eq!(b.total_spin_rpm, Some(2414));
        assert!(b.spin_axis.is_some_and(|a| (a + 23.97).abs() < 0.05));
    }

    #[test]
    fn a_putt_has_zero_spin_and_no_axis() {
        let l = Launch {
            ball_speed_mph: 16.5,
            launch_angle_deg: 0.3,
            smash_factor: 1.5,
            ..Launch::default()
        };
        let b = launch(&l);
        assert_eq!(b.total_spin_rpm, Some(0));
        assert_eq!(b.spin_axis, None);
    }
}
