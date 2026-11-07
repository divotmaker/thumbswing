//! The README's worked examples, flown with dimple: the launch numbers are
//! asserted, the carries printed (`cargo test --test worked_examples -- --nocapture`).

use dimple::aero::BallModel;
use dimple::environment::Environment;
use dimple::ground::Surface;
use dimple::stats::ShotStats;
use dimple::trajectory::{ShotInput, simulate_shot};
use thumbswing::mapping::{delivery, launch_for};
use thumbswing::{Club, CurveSense, Hand, Launch, Smash, Swing, Tuning};

struct Row {
    name: &'static str,
    club: Club,
    back: f64,
    fwd: f64,
    peak: f64,
    sense: CurveSense,
    ball: f64,
    vla: f64,
    hla: f64,
    back_spin: f64,
    side_spin: f64,
    /// Expected carry (yd) or, for the putt, roll (ft).
    carry: f64,
}

const ROWS: [Row; 8] = [
    Row {
        name: "straight drive, DR, full",
        club: Club::Driver,
        back: 0.0,
        fwd: 0.0,
        peak: 30.0,
        sense: CurveSense::Arcade,
        ball: 149.0,
        vla: 10.4,
        hla: 0.0,
        back_spin: 2208.0,
        side_spin: 0.0,
        carry: 230.0,
    },
    Row {
        name: "draw, DR (pull 30° right, push straight)",
        club: Club::Driver,
        back: -30.0,
        fwd: 0.0,
        peak: 30.0,
        sense: CurveSense::Arcade,
        ball: 148.2,
        vla: 10.4,
        hla: 0.9,
        back_spin: 2206.0,
        side_spin: -981.0,
        carry: 221.0,
    },
    Row {
        name: "slice, DR 90 % (pull 30° left, push 30° left)",
        club: Club::Driver,
        back: 30.0,
        fwd: -30.0,
        peak: 27.0,
        sense: CurveSense::Arcade,
        ball: 131.2,
        vla: 10.4,
        hla: -9.5,
        back_spin: 1968.0,
        side_spin: 1770.0,
        carry: 168.0,
    },
    Row {
        name: "gentle fade, DR (pull 15° left, push straight)",
        club: Club::Driver,
        back: 15.0,
        fwd: 0.0,
        peak: 30.0,
        sense: CurveSense::Arcade,
        ball: 148.8,
        vla: 10.4,
        hla: -0.4,
        back_spin: 2208.0,
        side_spin: 490.0,
        carry: 228.0,
    },
    Row {
        name: "7-iron at 80 %",
        club: Club::Iron7,
        back: 0.0,
        fwd: 0.0,
        peak: 24.0,
        sense: CurveSense::Arcade,
        ball: 86.6,
        vla: 16.1,
        hla: 0.0,
        back_spin: 4975.0,
        side_spin: 0.0,
        carry: 105.0,
    },
    Row {
        name: "short chip, PW at 25 %",
        club: Club::PitchingWedge,
        back: 0.0,
        fwd: 0.0,
        peak: 7.5,
        sense: CurveSense::Arcade,
        ball: 22.6,
        vla: 23.7,
        hla: 0.0,
        back_spin: 2029.0,
        side_spin: 0.0,
        carry: 8.0,
    },
    Row {
        name: "putt, PT at 100 % (roll, ft)",
        club: Club::Putter,
        back: 0.0,
        fwd: 0.0,
        peak: 30.0,
        sense: CurveSense::Arcade,
        ball: 16.5,
        vla: 0.3,
        hla: 0.0,
        back_spin: 0.0,
        side_spin: 0.0,
        carry: 56.0,
    },
    Row {
        name: "A2 hands, physical sense",
        club: Club::Driver,
        back: 15.0,
        fwd: 0.0,
        peak: 30.0,
        sense: CurveSense::Physical,
        ball: 148.8,
        vla: 10.4,
        hla: 0.4,
        back_spin: 2208.0,
        side_spin: -490.0,
        carry: 228.0,
    },
];

fn launch_of(row: &Row) -> Launch {
    let swing = Swing {
        back_dir_deg: row.back,
        back_len_in: 1.5,
        back_fraction: 0.75,
        fwd_dir_deg: row.fwd,
        fwd_peak_in_s: row.peak,
        ..Swing::default()
    };
    let tuning = Tuning {
        curve_sense: row.sense,
        ..Tuning::default()
    };
    let d = delivery(&swing, row.club, Hand::Right, &tuning);
    launch_for(&d, row.club, &Smash::LOFT_LINEAR)
}

fn fly(l: &Launch, surface: &Surface) -> ShotStats {
    let input = ShotInput {
        ball_speed_mph: l.ball_speed_mph,
        launch_angle_deg: l.launch_angle_deg,
        launch_azimuth_deg: l.launch_azimuth_deg,
        backspin_rpm: l.backspin_rpm,
        sidespin_rpm: l.sidespin_rpm,
    };
    ShotStats::from_shot(&simulate_shot(
        &input,
        &BallModel::TOUR,
        &Environment::SEA_LEVEL,
        surface,
    ))
}

#[test]
fn worked_examples() {
    println!(
        "| Swing | ball mph (doc) | VLA / HLA | back / side rpm | carry yd (doc) | total yd | lateral yd |"
    );
    println!("|---|---|---|---|---|---|---|");
    for row in &ROWS {
        let l = launch_of(row);
        let err = l.ball_speed_mph / row.ball - 1.0;
        assert!(
            err.abs() <= 0.02,
            "{}: ball {:.1} vs {}",
            row.name,
            l.ball_speed_mph,
            row.ball
        );
        assert!(
            (l.launch_angle_deg - row.vla).abs() <= 0.1,
            "{}: VLA {:.2}",
            row.name,
            l.launch_angle_deg
        );
        assert!(
            (l.launch_azimuth_deg - row.hla).abs() <= 0.1,
            "{}: HLA {:.2}",
            row.name,
            l.launch_azimuth_deg
        );
        assert!(
            (l.backspin_rpm - row.back_spin).abs() <= 0.01 * row.back_spin + 2.0,
            "{}: back {:.0}",
            row.name,
            l.backspin_rpm
        );
        assert!(
            (l.sidespin_rpm - row.side_spin).abs() <= 0.01 * row.side_spin.abs() + 2.0,
            "{}: side {:.0}",
            row.name,
            l.sidespin_rpm
        );

        if row.club.is_putter() {
            let s = fly(&l, &Surface::GREEN);
            let feet = s.total_yards * 3.0;
            println!(
                "| {} | {:.1} ({}) | {:.1} / {:+.1} | 0 / 0 | roll {:.0} ft ({} ft) | | |",
                row.name,
                l.ball_speed_mph,
                row.ball,
                l.launch_angle_deg,
                l.launch_azimuth_deg,
                feet,
                row.carry
            );
            assert!((feet - row.carry).abs() <= 6.0, "putt rolls {feet:.0} ft");
        } else {
            let s = fly(&l, &Surface::FAIRWAY);
            println!(
                "| {} | {:.1} ({}) | {:.1} / {:+.1} | {:.0} / {:+.0} | {:.0} ({}) | {:.0} | {:+.0} |",
                row.name,
                l.ball_speed_mph,
                row.ball,
                l.launch_angle_deg,
                l.launch_azimuth_deg,
                l.backspin_rpm,
                l.sidespin_rpm,
                s.carry_yards,
                row.carry,
                s.total_yards,
                s.lateral_yards
            );
            assert!(
                (s.carry_yards - row.carry).abs() <= 0.05 * row.carry + 2.0,
                "{}: carry {:.0}",
                row.name,
                s.carry_yards
            );
        }
    }
}
