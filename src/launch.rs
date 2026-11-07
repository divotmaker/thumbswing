//! Club delivery → ball launch (the D-plane impact model).
//!
//! With `V` club speed (mph), `P` path, `F` face (degrees, positive = right),
//! `A = F − P` face-to-path, `K` attack angle, `D` dynamic loft and
//! `L = D − K` the loft relative to the path:
//!
//! | Quantity | Formula |
//! |---|---|
//! | 3-D spin loft | `cos Φ = cos A · cos L` |
//! | Horizontal launch | `HLA = P + f(Φ)·A`, `f(Φ) = 0.96 − 0.0071·Φ` |
//! | Vertical launch | `VLA = w·D + (1 − w)·K` |
//! | Ball speed | `v = V · SF · cos Φ / cos L` |
//! | Total spin | `S = k · V · sin Φ` (rpm) |
//! | Spin axis | `tan(axis) = tan A / tan L`; backspin `S·cos(axis)`, sidespin `S·sin(axis)` |
//!
//! `SF` (smash at zero face-to-path), `k` and `w` come from the club
//! ([`ImpactModel`], built by [`ClubProfile::model`](crate::ClubProfile::model)).
//! The face weight `f(Φ)` is 0.86 at 14°, 0.78 at 25° and 0.68 at 40° of spin
//! loft. Every lateral value is positive to the right, the Flight Relay
//! Protocol convention: positive sidespin curves the ball right.
//!
//! Sources: Tutelman's ball-flight pages (3-D launch, smash factor), Trackman's
//! spin-loft and spin-axis notes. The `w` rule for the vertical launch is a
//! Trackman-derived rule of thumb that reaches wedge launch angles.

/// Club head state at impact.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Delivery {
    /// Club head speed, mph.
    pub club_speed_mph: f64,
    /// Club path, degrees, positive = right (in-to-out for a right-hander).
    pub path_deg: f64,
    /// Face angle to the target line, degrees, positive = right (open).
    pub face_deg: f64,
    /// Attack angle, degrees, positive = up.
    pub attack_deg: f64,
    /// Dynamic loft, degrees.
    pub dynamic_loft_deg: f64,
}

impl Delivery {
    /// Face-to-path `F − P`, degrees.
    #[must_use]
    pub fn face_to_path_deg(&self) -> f64 {
        self.face_deg - self.path_deg
    }
}

/// The per-club constants of the impact model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImpactModel {
    /// Smash factor at zero face-to-path.
    pub smash: f64,
    /// Spin constant `k`, rpm per mph of club speed per unit `sin Φ`.
    pub k_spin: f64,
    /// Weight `w` of the dynamic loft in the vertical launch.
    pub launch_weight: f64,
}

/// Ball launch conditions.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Launch {
    /// Ball speed, mph.
    pub ball_speed_mph: f64,
    /// Vertical launch angle, degrees.
    pub launch_angle_deg: f64,
    /// Horizontal launch angle, degrees, positive = right.
    pub launch_azimuth_deg: f64,
    /// Backspin, rpm.
    pub backspin_rpm: f64,
    /// Sidespin, rpm, positive = curves right.
    pub sidespin_rpm: f64,
    /// 3-D spin loft `Φ`, degrees.
    pub spin_loft_deg: f64,
    /// Ball speed over club speed, including the obliqueness loss.
    pub smash_factor: f64,
}

impl Launch {
    /// Total spin, rpm.
    #[must_use]
    pub fn total_spin_rpm(&self) -> f64 {
        self.backspin_rpm.hypot(self.sidespin_rpm)
    }

    /// Spin axis, degrees, positive = tilted right (curves right); 0 without spin.
    #[must_use]
    pub fn spin_axis_deg(&self) -> f64 {
        if self.total_spin_rpm() == 0.0 {
            0.0
        } else {
            self.sidespin_rpm.atan2(self.backspin_rpm).to_degrees()
        }
    }
}

/// The face weight `f(Φ)` of the horizontal launch.
#[must_use]
pub fn face_weight(spin_loft_deg: f64) -> f64 {
    0.96 - 0.0071 * spin_loft_deg
}

/// Launch conditions for a full-swing delivery.
#[must_use]
pub fn launch(d: &Delivery, m: &ImpactModel) -> Launch {
    let a = d.face_to_path_deg().to_radians();
    let l = (d.dynamic_loft_deg - d.attack_deg).to_radians();
    let phi = (a.cos() * l.cos()).clamp(-1.0, 1.0).acos();
    let phi_deg = phi.to_degrees();

    let azimuth = d.path_deg + face_weight(phi_deg) * d.face_to_path_deg();
    let elevation = m.launch_weight * d.dynamic_loft_deg + (1.0 - m.launch_weight) * d.attack_deg;
    let ball_speed = if l.cos() > 0.0 {
        d.club_speed_mph * m.smash * phi.cos() / l.cos()
    } else {
        0.0
    };
    let spin = m.k_spin * d.club_speed_mph * phi.sin();
    let axis = a.tan().atan2(l.tan());

    Launch {
        ball_speed_mph: ball_speed,
        launch_angle_deg: elevation,
        launch_azimuth_deg: azimuth,
        backspin_rpm: spin * axis.cos(),
        sidespin_rpm: spin * axis.sin(),
        spin_loft_deg: phi_deg,
        smash_factor: if d.club_speed_mph > 0.0 {
            ball_speed / d.club_speed_mph
        } else {
            m.smash
        },
    }
}

/// Smash factor of the putter.
pub const PUTT_SMASH: f64 = 1.5;
/// Vertical launch of a putt, degrees: low enough that the ball lands at once
/// and rolls.
pub const PUTT_LAUNCH_DEG: f64 = 0.3;

/// Launch conditions for a putt: the ball starts on the path at
/// [`PUTT_SMASH`] times the club speed, [`PUTT_LAUNCH_DEG`] up, with no spin.
#[must_use]
pub fn putt(d: &Delivery) -> Launch {
    Launch {
        ball_speed_mph: d.club_speed_mph * PUTT_SMASH,
        launch_angle_deg: PUTT_LAUNCH_DEG,
        launch_azimuth_deg: d.path_deg,
        backspin_rpm: 0.0,
        sidespin_rpm: 0.0,
        spin_loft_deg: d.dynamic_loft_deg - d.attack_deg,
        smash_factor: PUTT_SMASH,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DRIVER: ImpactModel = ImpactModel {
        smash: 1.49,
        k_spin: 96.0,
        launch_weight: 0.85,
    };

    fn driver(face: f64, path: f64) -> Delivery {
        Delivery {
            club_speed_mph: 100.0,
            path_deg: path,
            face_deg: face,
            attack_deg: -0.9,
            dynamic_loft_deg: 12.4,
        }
    }

    #[test]
    fn square_driver() {
        let l = launch(&driver(0.0, 0.0), &DRIVER);
        assert!((l.ball_speed_mph - 149.0).abs() < 1e-9);
        assert!((l.launch_angle_deg - 10.405).abs() < 1e-9);
        assert!(l.launch_azimuth_deg.abs() < 1e-12);
        assert!((l.backspin_rpm - 2208.0).abs() < 1.0, "{}", l.backspin_rpm);
        assert!(l.sidespin_rpm.abs() < 1e-9);
        assert!(l.spin_axis_deg().abs() < 1e-9);
    }

    #[test]
    fn zero_face_to_path_gives_zero_axis_whatever_the_path() {
        let l = launch(&driver(5.0, 5.0), &DRIVER);
        assert!(l.spin_axis_deg().abs() < 1e-9);
        assert!((l.launch_azimuth_deg - 5.0).abs() < 1e-9);
    }

    #[test]
    fn face_weights_follow_the_d_plane_rule() {
        assert!((face_weight(14.0) - 0.8606).abs() < 1e-4);
        assert!((face_weight(25.0) - 0.7825).abs() < 1e-4);
        assert!((face_weight(40.0) - 0.676).abs() < 1e-4);
    }

    #[test]
    fn start_direction_lies_between_path_and_face_nearer_the_face() {
        // Face 0, path +6: a draw for a right-hander.
        let l = launch(&driver(0.0, 6.0), &DRIVER);
        assert!(l.launch_azimuth_deg > 0.0 && l.launch_azimuth_deg < 3.0);
        assert!(l.sidespin_rpm < 0.0, "face left of path curves left");
        assert!(
            (l.spin_axis_deg() + 24.0).abs() < 0.5,
            "{}",
            l.spin_axis_deg()
        );
        assert!(
            (l.ball_speed_mph - 148.2).abs() < 0.1,
            "{}",
            l.ball_speed_mph
        );
    }

    #[test]
    fn mirrored_delivery_mirrors_every_lateral_value() {
        let rh = launch(&driver(-1.0, 4.0), &DRIVER);
        let lh = launch(&driver(1.0, -4.0), &DRIVER);
        assert!((rh.launch_azimuth_deg + lh.launch_azimuth_deg).abs() < 1e-9);
        assert!((rh.sidespin_rpm + lh.sidespin_rpm).abs() < 1e-9);
        assert!((rh.backspin_rpm - lh.backspin_rpm).abs() < 1e-9);
        assert!((rh.ball_speed_mph - lh.ball_speed_mph).abs() < 1e-9);
    }

    #[test]
    fn putt_has_no_spin() {
        let d = Delivery {
            club_speed_mph: 11.0,
            path_deg: 2.0,
            face_deg: 2.0,
            attack_deg: 0.0,
            dynamic_loft_deg: 3.0,
        };
        let l = putt(&d);
        assert!((l.ball_speed_mph - 16.5).abs() < 1e-9);
        assert!((l.launch_angle_deg - 0.3).abs() < 1e-12);
        assert!((l.launch_azimuth_deg - 2.0).abs() < 1e-12);
        assert!(l.total_spin_rpm() == 0.0);
    }
}
