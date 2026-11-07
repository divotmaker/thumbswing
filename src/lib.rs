//! # thumbswing
//!
//! A trackball launch monitor: pull a trackball (or any mouse) back, push it
//! forward, and get a full golf shot — club speed, path, face, ball speed,
//! launch, spin and spin axis — the way an arcade-style trackball golf game
//! reads the swing.
//!
//! - The **backswing** direction sets where the shot curves.
//! - The **push** direction sets the start line (the face).
//! - The **push speed** sets the club speed, capped by the selected club.
//!
//! The pipeline is platform-independent: [`Sample`]s (one report from the
//! device) go through a clocked [`Segmenter`] into a [`Swing`], which
//! [`mapping`] turns into a [`Delivery`] and the impact model ([`launch`])
//! into a [`Launch`]. [`Client`] runs the whole thing over an [`Input`] and
//! reports [`Event`]s from a non-blocking [`poll`](Client::poll).
//!
//! Input comes from Linux evdev ([`device`]); on other platforms
//! [`device::list`] and [`device::Device::open`] return
//! [`Error::Unsupported`]. [`recording`] reads and writes sessions as JSON
//! Lines and replays them as an [`Input`].
//!
//! ```no_run
//! use thumbswing::{Client, Config, Event, device::Device};
//!
//! let config = Config::default();
//! let input = Device::open("/dev/input/by-id/usb-Example_Trackball-event-mouse", config.grab)?;
//! let mut client = Client::new(input, config);
//! loop {
//!     match client.poll()? {
//!         Some(Event::Shot(shot)) => println!("{:.1} mph ball", shot.launch.ball_speed_mph),
//!         Some(_) => {}
//!         None => std::thread::sleep(std::time::Duration::from_millis(2)),
//!     }
//! }
//! # Ok::<(), thumbswing::Error>(())
//! ```
//!
//! [`Settings`] is everything a device runs with (which input, its
//! [`Config`] and the [`Tuning`]), with [`Settings::validate`] for the ranges.
//!
//! Features:
//! - **`config`** — serde on [`Settings`] and its parts, and `Settings` as a
//!   [`flightrelay`](https://crates.io/crates/flightrelay) `DriverConfig`; no
//!   device stack, so a settings form can use it.
//! - **`driver`** — `driver::Thumbswing`, the flightrelay `DeviceDriver` a
//!   host such as a golf automation bus runs (implies `config`).
//! - **`frp`** — the `thumbswing-frp` binary, which serves the driver to
//!   Flight Relay Protocol controllers (implies `driver`).
//! - **`testing`** — `thumbswing::testing`: synthetic swing streams and a
//!   manual clock.

mod client;
mod club;
mod config;
pub mod device;
mod error;
mod input;
mod json;
pub mod launch;
pub mod mapping;
mod profile;
pub mod recording;
mod sample;
mod segment;
mod swing;

#[cfg(feature = "driver")]
pub mod driver;
#[cfg(feature = "testing")]
pub mod testing;

pub use client::{Client, Event, Shot};
pub use club::{Club, Hand, UnknownClub, UnknownHand};
pub use config::{Config, CurveSense, InvalidSetting, Settings, Thresholds, Tuning};
pub use error::Error;
pub use input::{DeviceInfo, Input};
pub use launch::{Delivery, ImpactModel, Launch};
pub use profile::{CLUB_TABLE, ClubProfile, Smash};
pub use sample::Sample;
pub use segment::{CancelReason, Output, Phase, Segmenter};
pub use swing::Swing;
