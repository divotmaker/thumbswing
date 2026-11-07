//! Strokes → club delivery → launch.
//!
//! With `f` the push direction, `b` the backswing line continued forward
//! (both degrees from straight forward, positive = right), `δ = b − f` and
//! `p` the power:
//!
//! | Step | Formula |
//! |---|---|
//! | Club speed | `V = p · max_club_speed · speed_ratio` |
//! | Face | `F = clamp(angle_gain · f, ±face_max)` |
//! | Path, arcade sense | `P = F − curve_gain · δ` |
//! | Path, physical sense | `P = F + curve_gain · δ` |
//! | Face-to-path | `A = clamp(F − P, ±face_to_path_max)`, then `P = F − A` |
//! | Attack, dynamic loft | from the club table |
//! | Putter | `V = p · max_putt_speed`, `F = P = putt_angle_gain · f` |
//!
//! Power is `p = clamp(v_peak / full_power_speed, 0, 1)^γ`, scaled by
//! `1 − β + β · back_fraction` with `β` = [`Tuning::backswing_power`].

use crate::club::{Club, Hand};
use crate::config::{CurveSense, Tuning};
use crate::launch::{self, Delivery, Launch};
use crate::profile::{ClubProfile, Smash};
use crate::swing::Swing;

/// Power for a swing, 0..=1.
#[must_use]
pub fn power(swing: &Swing, tuning: &Tuning) -> f64 {
    let full = tuning.full_power_speed.max(f64::EPSILON);
    let p_fwd = (swing.fwd_peak_in_s / full)
        .clamp(0.0, 1.0)
        .powf(tuning.power_gamma.max(0.01));
    let beta = tuning.backswing_power.clamp(0.0, 1.0);
    p_fwd * (1.0 - beta + beta * swing.back_fraction.clamp(0.0, 1.0))
}

/// The club delivery for a swing.
#[must_use]
pub fn delivery(swing: &Swing, club: Club, hand: Hand, tuning: &Tuning) -> Delivery {
    let mirror = tuning.mirror_for_left_hand && hand == Hand::Left;
    let sign = if mirror { -1.0 } else { 1.0 };
    let f = sign * swing.fwd_dir_deg;
    let b = sign * swing.back_dir_deg;
    let p = power(swing, tuning);
    let profile = ClubProfile::of(club);

    if club.is_putter() {
        let line = tuning.putt_angle_gain * f;
        return Delivery {
            club_speed_mph: p * tuning.max_putt_speed,
            path_deg: line,
            face_deg: line,
            attack_deg: profile.attack_deg,
            dynamic_loft_deg: profile.dynamic_loft_deg,
        };
    }

    let delta = b - f;
    let face = (tuning.angle_gain * f).clamp(-tuning.face_max, tuning.face_max);
    let path = match tuning.curve_sense {
        CurveSense::Arcade => face - tuning.curve_gain * delta,
        CurveSense::Physical => face + tuning.curve_gain * delta,
    };
    let f2p = (face - path).clamp(-tuning.face_to_path_max, tuning.face_to_path_max);
    Delivery {
        club_speed_mph: p * tuning.max_club_speed * profile.speed_ratio,
        path_deg: face - f2p,
        face_deg: face,
        attack_deg: profile.attack_deg,
        dynamic_loft_deg: profile.dynamic_loft_deg,
    }
}

/// The launch for a delivery with `club`.
#[must_use]
pub fn launch_for(delivery: &Delivery, club: Club, smash: &Smash) -> Launch {
    if club.is_putter() {
        launch::putt(delivery)
    } else {
        launch::launch(delivery, &ClubProfile::of(club).model(smash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swing(b: f64, f: f64, peak: f64) -> Swing {
        Swing {
            back_dir_deg: b,
            back_len_in: 1.5,
            back_fraction: 0.75,
            fwd_dir_deg: f,
            fwd_peak_in_s: peak,
            ..Swing::default()
        }
    }

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    #[test]
    fn pull_left_push_straight() {
        let t = Tuning::default();
        let d = delivery(&swing(20.0, 0.0, 30.0), Club::Iron7, Hand::Right, &t);
        assert_close(d.face_deg, 0.0);
        assert_close(d.path_deg, -4.0);
        assert_close(d.face_to_path_deg(), 4.0);

        let phys = Tuning {
            curve_sense: CurveSense::Physical,
            ..t
        };
        let d = delivery(&swing(20.0, 0.0, 30.0), Club::Iron7, Hand::Right, &phys);
        assert_close(d.path_deg, 4.0);
        assert_close(d.face_to_path_deg(), -4.0);
    }

    #[test]
    fn push_right_of_a_straight_pull() {
        let t = Tuning::default();
        let d = delivery(&swing(0.0, 10.0, 30.0), Club::Iron7, Hand::Right, &t);
        assert_close(d.face_deg, 2.5);
        assert_close(d.path_deg, 4.5);
        assert_close(d.face_to_path_deg(), -2.0);
        let l = launch_for(&d, Club::Iron7, &Smash::LOFT_LINEAR);
        assert!(l.launch_azimuth_deg > d.face_deg && l.launch_azimuth_deg < d.path_deg);
        assert!(l.sidespin_rpm < 0.0);

        let phys = Tuning {
            curve_sense: CurveSense::Physical,
            ..t
        };
        let d = delivery(&swing(0.0, 10.0, 30.0), Club::Iron7, Hand::Right, &phys);
        assert_close(d.path_deg, 0.5);
        assert_close(d.face_to_path_deg(), 2.0);
        let l = launch_for(&d, Club::Iron7, &Smash::LOFT_LINEAR);
        assert!(l.launch_azimuth_deg < d.face_deg && l.launch_azimuth_deg > d.path_deg);
        assert!(l.sidespin_rpm > 0.0);
    }

    #[test]
    fn left_hand_mirror_flips_every_lateral_sign() {
        let t = Tuning {
            mirror_for_left_hand: true,
            ..Tuning::default()
        };
        let s = swing(20.0, 0.0, 30.0);
        let rh = delivery(&s, Club::Iron7, Hand::Right, &t);
        let lh = delivery(&s, Club::Iron7, Hand::Left, &t);
        assert_close(lh.path_deg, -rh.path_deg);
        assert_close(lh.face_deg, -rh.face_deg);
        let (lr, ll) = (
            launch_for(&rh, Club::Iron7, &Smash::LOFT_LINEAR),
            launch_for(&lh, Club::Iron7, &Smash::LOFT_LINEAR),
        );
        assert_close(ll.sidespin_rpm, -lr.sidespin_rpm);
        assert_close(ll.launch_azimuth_deg, -lr.launch_azimuth_deg);
        // Without the mirror a left-hander's strokes fly like a right-hander's.
        let plain = delivery(&s, Club::Iron7, Hand::Left, &Tuning::default());
        assert_close(plain.path_deg, rh.path_deg);
    }

    #[test]
    fn half_the_peak_is_half_the_club_speed() {
        let t = Tuning::default();
        let full = delivery(&swing(0.0, 0.0, 30.0), Club::Iron7, Hand::Right, &t);
        let half = delivery(&swing(0.0, 0.0, 15.0), Club::Iron7, Hand::Right, &t);
        assert_close(full.club_speed_mph, 80.0);
        assert_close(half.club_speed_mph, 40.0);
        let lf = launch_for(&full, Club::Iron7, &Smash::LOFT_LINEAR);
        let lh = launch_for(&half, Club::Iron7, &Smash::LOFT_LINEAR);
        assert!((lh.ball_speed_mph * 2.0 - lf.ball_speed_mph).abs() < 1e-9);
        assert!((lf.ball_speed_mph - 80.0 * 1.353).abs() < 0.05);
    }

    #[test]
    fn power_is_capped_and_shaped() {
        let t = Tuning::default();
        assert_close(power(&swing(0.0, 0.0, 60.0), &t), 1.0);
        let g = Tuning {
            power_gamma: 2.0,
            ..t
        };
        assert_close(power(&swing(0.0, 0.0, 15.0), &g), 0.25);
        let b = Tuning {
            backswing_power: 0.5,
            ..t
        };
        // back_fraction 0.75: 1 − 0.5 + 0.5·0.75 = 0.875.
        assert_close(power(&swing(0.0, 0.0, 30.0), &b), 0.875);
    }

    #[test]
    fn putter_follows_the_push_one_to_one() {
        let t = Tuning::default();
        let d = delivery(&swing(30.0, 4.0, 30.0), Club::Putter, Hand::Right, &t);
        assert_close(d.club_speed_mph, 11.0);
        assert_close(d.path_deg, 4.0);
        assert_close(d.face_deg, 4.0);
        let l = launch_for(&d, Club::Putter, &Smash::LOFT_LINEAR);
        assert_close(l.launch_angle_deg, 0.3);
        assert_close(l.total_spin_rpm(), 0.0);
        assert_close(l.ball_speed_mph, 16.5);
    }
}
