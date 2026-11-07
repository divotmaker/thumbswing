//! The club table and the smash rule.
//!
//! Each full-swing club has a speed ratio (its full-power club speed over the
//! driver's), an attack angle, a dynamic loft, a spin constant and a launch
//! weight, fitted so that at the PGA Tour's 2023 average club speed for that
//! club the impact model returns Trackman's tour-average launch and spin. The
//! gap, sand and lob wedges have no tour row and are extrapolated.
//!
//! Smash is a function of spin loft, not a value per club:
//! `SF = 1.49 − 0.0102 · (L − 13.3)` ([`Smash::LOFT_LINEAR`]).

use crate::club::Club;
use crate::launch::{ImpactModel, PUTT_SMASH};

/// The rule that gives a club its smash factor.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Smash {
    /// One line in spin loft `L`: `SF = at_13_3 − per_deg · (L − 13.3)`.
    LoftLinear {
        /// Smash at 13.3° of spin loft (the driver).
        at_13_3: f64,
        /// Smash lost per degree of spin loft.
        per_deg: f64,
    },
}

impl Smash {
    /// `SF = 1.49 − 0.0102 · (L − 13.3)`, fitted to the PGA and LPGA 2023 tour
    /// averages (every tour club within 0.015).
    pub const LOFT_LINEAR: Smash = Smash::LoftLinear {
        at_13_3: 1.49,
        per_deg: 0.0102,
    };

    /// Smash factor at spin loft `spin_loft_deg`.
    #[must_use]
    pub fn at(&self, spin_loft_deg: f64) -> f64 {
        match *self {
            Smash::LoftLinear { at_13_3, per_deg } => at_13_3 - per_deg * (spin_loft_deg - 13.3),
        }
    }
}

impl Default for Smash {
    fn default() -> Self {
        Smash::LOFT_LINEAR
    }
}

/// One row of the club table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClubProfile {
    /// The club.
    pub club: Club,
    /// Full-power club speed as a fraction of the driver's.
    pub speed_ratio: f64,
    /// Attack angle, degrees.
    pub attack_deg: f64,
    /// Dynamic loft, degrees.
    pub dynamic_loft_deg: f64,
    /// Weight of the dynamic loft in the vertical launch.
    pub launch_weight: f64,
    /// Spin constant, rpm per mph per unit `sin Φ`.
    pub k_spin: f64,
}

const fn row(
    club: Club,
    speed_ratio: f64,
    attack_deg: f64,
    dynamic_loft_deg: f64,
    launch_weight: f64,
    k_spin: f64,
) -> ClubProfile {
    ClubProfile {
        club,
        speed_ratio,
        attack_deg,
        dynamic_loft_deg,
        launch_weight,
        k_spin,
    }
}

/// The club table, driver through putter. The putter row carries only its
/// attack angle and loft; its speed comes from
/// [`Tuning::max_putt_speed`](crate::Tuning::max_putt_speed).
pub const CLUB_TABLE: [ClubProfile; 16] = [
    row(Club::Driver, 1.00, -0.9, 12.4, 0.85, 96.0),
    row(Club::Wood3, 0.96, -2.3, 11.3, 0.85, 141.0),
    row(Club::Wood5, 0.92, -2.5, 11.9, 0.85, 164.0),
    row(Club::Hybrid3, 0.89, -2.4, 12.4, 0.85, 176.0),
    row(Club::Iron3, 0.87, -2.5, 14.6, 0.75, 150.0),
    row(Club::Iron4, 0.85, -2.9, 15.4, 0.75, 156.0),
    row(Club::Iron5, 0.83, -3.4, 17.0, 0.75, 158.0),
    row(Club::Iron6, 0.82, -3.7, 19.9, 0.75, 165.0),
    row(Club::Iron7, 0.80, -3.9, 22.8, 0.75, 173.0),
    row(Club::Iron8, 0.77, -4.2, 25.1, 0.75, 185.0),
    row(Club::Iron9, 0.76, -4.3, 28.1, 0.75, 189.0),
    row(Club::PitchingWedge, 0.73, -4.7, 33.2, 0.75, 181.0),
    row(Club::GapWedge, 0.71, -4.8, 36.3, 0.75, 182.0),
    row(Club::SandWedge, 0.70, -5.0, 39.7, 0.75, 181.0),
    row(Club::LobWedge, 0.67, -5.0, 43.0, 0.75, 183.0),
    row(Club::Putter, 0.0, 0.0, 3.0, 0.0, 0.0),
];

impl ClubProfile {
    /// The table row for `club`.
    #[must_use]
    pub fn of(club: Club) -> &'static ClubProfile {
        CLUB_TABLE
            .iter()
            .find(|p| p.club == club)
            .unwrap_or(&CLUB_TABLE[8])
    }

    /// Spin loft `L = D − K` at zero face-to-path, degrees.
    #[must_use]
    pub fn spin_loft_deg(&self) -> f64 {
        self.dynamic_loft_deg - self.attack_deg
    }

    /// The impact-model constants for this club under `smash`.
    #[must_use]
    pub fn model(&self, smash: &Smash) -> ImpactModel {
        ImpactModel {
            smash: if self.club.is_putter() {
                PUTT_SMASH
            } else {
                smash.at(self.spin_loft_deg())
            },
            k_spin: self.k_spin,
            launch_weight: self.launch_weight,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::{Delivery, launch};

    /// Trackman PGA Tour 2023 averages: club, club speed, ball speed, launch, spin.
    const TOUR: [(Club, f64, f64, f64, f64); 12] = [
        (Club::Driver, 115.0, 171.0, 10.4, 2545.0),
        (Club::Wood3, 110.0, 162.0, 9.3, 3663.0),
        (Club::Wood5, 106.0, 156.0, 9.7, 4322.0),
        (Club::Hybrid3, 102.0, 149.0, 10.2, 4587.0),
        (Club::Iron3, 100.0, 145.0, 10.3, 4404.0),
        (Club::Iron4, 98.0, 140.0, 10.8, 4782.0),
        (Club::Iron5, 96.0, 135.0, 11.9, 5280.0),
        (Club::Iron6, 94.0, 130.0, 14.0, 6204.0),
        (Club::Iron7, 92.0, 123.0, 16.1, 7124.0),
        (Club::Iron8, 89.0, 118.0, 17.8, 8078.0),
        (Club::Iron9, 87.0, 112.0, 20.0, 8793.0),
        (Club::PitchingWedge, 84.0, 104.0, 23.7, 9316.0),
    ];

    #[test]
    fn tour_deliveries_reproduce_trackman() {
        for (club, club_speed, ball, vla, spin) in TOUR {
            let p = ClubProfile::of(club);
            let d = Delivery {
                club_speed_mph: club_speed,
                path_deg: 0.0,
                face_deg: 0.0,
                attack_deg: p.attack_deg,
                dynamic_loft_deg: p.dynamic_loft_deg,
            };
            let l = launch(&d, &p.model(&Smash::LOFT_LINEAR));
            let ball_err = l.ball_speed_mph / ball - 1.0;
            assert!(
                ball_err.abs() <= 0.02,
                "{club}: ball {:.1} vs {ball}",
                l.ball_speed_mph
            );
            assert!(
                (l.launch_angle_deg - vla).abs() <= 0.1,
                "{club}: launch {:.2} vs {vla}",
                l.launch_angle_deg
            );
            let spin_err = l.total_spin_rpm() / spin - 1.0;
            assert!(
                spin_err.abs() <= 0.01,
                "{club}: spin {:.0} vs {spin}",
                l.total_spin_rpm()
            );
        }
    }

    #[test]
    fn loft_linear_smash_matches_the_table() {
        let expected = [
            (Club::Driver, 1.490),
            (Club::Iron3, 1.451),
            (Club::Iron7, 1.353),
            (Club::Iron9, 1.295),
            (Club::PitchingWedge, 1.239),
            (Club::GapWedge, 1.206),
            (Club::SandWedge, 1.170),
            (Club::LobWedge, 1.136),
        ];
        for (club, sf) in expected {
            let got = ClubProfile::of(club).model(&Smash::LOFT_LINEAR).smash;
            assert!((got - sf).abs() < 0.0006, "{club}: {got:.4} vs {sf}");
        }
    }

    #[test]
    fn every_club_has_a_row() {
        for club in Club::ALL {
            assert_eq!(ClubProfile::of(club).club, club);
        }
    }
}
