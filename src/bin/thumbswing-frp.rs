//! thumbswing-frp — a Flight Relay Protocol device for a trackball swing.
//!
//! Reads a trackball (or any mouse), turns each swing into a shot, and
//! publishes the shots as a launch monitor actor on an FRP bus, listening on
//! port 5881. The club and the player's hand are those of each `golfer.turn`,
//! else the `club` and `hand` settings (the 7-iron and right-handed by
//! default).
//!
//! ```sh
//! thumbswing-frp                                           # the only trackball found, 0.0.0.0:5881
//! thumbswing-frp --set club=DR --set hand=lh               # start with the driver, left-handed
//! thumbswing-frp --set device=usb-…-event-mouse --set tuning.full_power_speed=40
//! thumbswing-frp --config trackball.toml --target ws://flighthook:5880/frp
//! thumbswing-frp --help                                    # every setting with its default
//! ```
//!
//! The flags are flightrelay's `run` flags (`--config`, `--target`, `--set`,
//! `--log`, `--help`); every [`thumbswing::Settings`] field is a `--set` key.

use std::process::ExitCode;

use flightrelay::RunOptions;
use thumbswing::driver::{DEFAULT_TARGET, Thumbswing};

fn main() -> ExitCode {
    flightrelay::run_with::<Thumbswing>(RunOptions {
        default_target: DEFAULT_TARGET,
        program: Some("thumbswing-frp"),
        ..RunOptions::default()
    })
}
