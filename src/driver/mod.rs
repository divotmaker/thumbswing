//! [`Thumbswing`]: the trackball as a
//! [`flightrelay`](https://crates.io/crates/flightrelay) device driver.
//!
//! Requires the `driver` feature. A host (the `thumbswing-frp` binary, a golf
//! automation bus) builds the driver from [`Settings`] with
//! [`DeviceDriver::new`], which validates them and opens nothing, then calls
//! [`connect`](DeviceDriver::connect) to open the input and
//! [`poll`](DeviceSession::poll) to turn swings into FRP messages: the
//! driver is a launch monitor actor on the bus.
//!
//! Every value this device sends is synthesized from the swing, so its
//! [`Source`] is [`Virtual`](Source::Virtual). Each shot is
//! `shot.trigger` (with the time of the swing) → `shot.launch` (speed,
//! launch angle and direction, both spin pairs) → `shot.club` (club speed,
//! path, face, attack angle, dynamic loft, smash) → `shot.finished`. There
//! is no `shot.flight`: a game or flight model flies the ball. A shot is
//! numbered and sent only while the host allows shots
//! ([`Events::shots_allowed`]); a swing made while shots are held is
//! dropped with a log line.
//!
//! The actor's `blockers` are `not_armed` until the input is open and
//! reading, and empty while it is. Telemetry: `synthetic = "true"`, `input`
//! (the name the input device reports), `cpi`, `profile` (the club in play)
//! and `handed` (`rh` or `lh`), all of them when a session starts and then
//! only what changed.
//!
//! The device id, for the host's logs and displays, is the input's
//! `/dev/input/by-id` link name (the file name only, e.g.
//! `usb-Example_Trackball-event-mouse`), which stays the same when the device
//! reconnects or is plugged in again. An input without such a link (some
//! built-in devices, a recording) is known by the name it reports. The label
//! shows the reported name. Neither goes on the bus.
//!
//! The club and the player's hand come from each `golfer.turn`
//! ([`TurnHints`]): the turn's club and handedness when it gives them, else
//! the settings' [`club`](Settings::club) and [`hand`](Settings::hand) (the
//! 7-iron and right-handed by default), so a change of a setting applies at
//! once while the turn gives no value for it. The putting mode selects the
//! putter whatever the club.
//!
//! When the input device goes away the session stays up: it raises
//! `not_armed`, reopens the device by itself, and clears it when the device
//! is back. With `device` unset the reopen chooses the device again, so a
//! device that comes back under another node is found. The club, the hand,
//! the thresholds and the tuning apply to a running session; a change of
//! `device`, `cpi`, `report_limit` or `grab` needs a new one
//! ([`Settings::restart_needed`]).
//!
//! When the device cannot be opened, the driver alerts the bus with
//! what to do (pick a device with the `device` setting, fix the permissions,
//! …) and retries with a growing delay. Alerts never name device paths, which
//! can carry a USB serial number; the full error is a debug log line. A
//! choice the user has to make (several trackballs, no permission) is
//! alerted once until the device opens. Off Linux, `connect` fails with
//! [`Error::Unsupported`] and the driver gives up instead of retrying.

mod convert;

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use flightrelay::config::{ConfigError, DriverConfig};
use flightrelay::driver::{
    Backoff, ConnectCtx, DeviceDriver, DeviceIdentity, DeviceSession, DriverInfo, Events, Level,
    Live, NoAction, SessionEnd, ShotIds, TurnHints,
};
use flightrelay::{Handedness, Mode, Severity, Source};

use crate::client::{Client, Event, Shot};
use crate::club::{Club, Hand, UnknownClub};
use crate::config::Settings;
use crate::device;
use crate::error::Error;
use crate::input::{DeviceInfo, Input};
use crate::sample::Sample;
use crate::segment::CancelReason;

pub use convert::{club_data, launch};

/// The driver kind, for logs and config tables.
pub const KIND: &str = "thumbswing";
/// The manufacturer the device reports.
pub const MANUFACTURER: &str = "divotmaker";
/// The model the device reports.
pub const MODEL: &str = "thumbswing trackball";
/// Where `thumbswing-frp` listens by default: port 5881, so it runs next to a
/// hub on 5880.
pub const DEFAULT_TARGET: &str = "0.0.0.0:5881";

/// How long a poll sleeps when the input has nothing.
const IDLE_SLEEP: Duration = Duration::from_millis(2);

/// An input a driver serves.
type BoxedInput = Box<dyn Input + Send>;

/// Opens the input a `device` setting names, with or without the grab.
type Opener = Arc<dyn Fn(Option<&str>, bool) -> Result<BoxedInput, Error> + Send + Sync>;

/// The trackball launch monitor. See the [module docs](self).
pub struct Thumbswing {
    settings: Settings,
    /// The settings a running session follows (club, hand, thresholds and
    /// tuning).
    live: Live<Settings>,
    ids: ShotIds,
    /// Opens the device: [`device::open_setting`], or a test's inputs.
    opener: Opener,
    /// An input given up front, served by the next connect instead of a
    /// device.
    given: Option<BoxedInput>,
    /// Whether the given input has been served.
    given_used: bool,
    /// The failure already alerted in this outage, for those alerted once.
    alerted: Option<Outage>,
}

/// A failure to open the device that only the user can fix, alerted once
/// per outage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outage {
    Ambiguous,
    PermissionDenied,
}

impl Outage {
    fn of(error: &Error) -> Option<Self> {
        match error {
            Error::Ambiguous { .. } => Some(Self::Ambiguous),
            Error::PermissionDenied { .. } => Some(Self::PermissionDenied),
            _ => None,
        }
    }
}

impl std::fmt::Debug for Thumbswing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Thumbswing")
            .field("settings", &self.settings)
            .field("given", &self.given.as_ref().map(|i| i.info().name.clone()))
            .finish_non_exhaustive()
    }
}

impl Thumbswing {
    /// A driver that serves `input` (a [`Replay`](crate::recording::Replay)
    /// of a recording) instead of opening a device. The first connect uses
    /// it; once it has been served the driver gives up rather than
    /// reconnecting.
    ///
    /// # Errors
    /// The [`ConfigError`] of settings that are not valid.
    pub fn with_input(
        settings: Settings,
        input: impl Input + Send + 'static,
    ) -> Result<Self, ConfigError> {
        let mut driver = Self::new(settings)?;
        driver.given = Some(Box::new(input));
        Ok(driver)
    }

    /// A session over `input`, opening nothing, with the club and hand from
    /// `hints`: what [`connect`](DeviceDriver::connect) returns after it has
    /// opened the device, for tests that feed the session a synthetic stream
    /// or a recording.
    #[cfg(feature = "testing")]
    #[must_use]
    pub fn connect_with(
        &mut self,
        input: impl Input + Send + 'static,
        hints: &TurnHints,
    ) -> Session {
        self.session(Box::new(input), *hints)
    }

    /// A driver whose [`connect`](DeviceDriver::connect) gets its inputs from
    /// `open` instead of the evdev devices: `open` is called with the
    /// [`device`](Settings::device) value and the grab, for every connect
    /// and, with `device` unset, for every reopen after an unplug. For tests
    /// of the connect and reconnect paths without hardware.
    ///
    /// # Errors
    /// The [`ConfigError`] of settings that are not valid.
    #[cfg(feature = "testing")]
    pub fn with_opener(
        settings: Settings,
        open: impl Fn(Option<&str>, bool) -> Result<BoxedInput, Error> + Send + Sync + 'static,
    ) -> Result<Self, ConfigError> {
        let mut driver = Self::new(settings)?;
        driver.opener = Arc::new(open);
        Ok(driver)
    }

    /// The alert text for a failure: what went wrong and what to do,
    /// without device paths or link names.
    fn alert_text(&self, error: &Error) -> String {
        match error {
            Error::Ambiguous { devices } => format!(
                "found {} trackballs/mice; pick one with the `device` setting",
                devices.len()
            ),
            Error::PermissionDenied { .. } => "cannot read the trackball: add the user to the \
                 `input` group (then log in again) or install a udev rule that tags input \
                 devices `uaccess`"
                .to_owned(),
            Error::Busy { .. } => "the trackball is grabbed by another program; close it, or \
                 set `grab` to false"
                .to_owned(),
            Error::NotFound(_) if self.settings.device().is_some() => {
                "no trackball or mouse matches the `device` setting".to_owned()
            }
            Error::NotFound(_) => "no trackball or mouse found".to_owned(),
            Error::NotAPointer { .. } => {
                "the `device` setting names an input that is not a trackball or mouse".to_owned()
            }
            Error::Parse { .. } => "the recording could not be read".to_owned(),
            other => other.to_string(),
        }
    }

    fn session(&self, input: BoxedInput, hints: TurnHints) -> Session {
        let info = input.info();
        let name = if info.name.is_empty() {
            KIND.to_owned()
        } else {
            info.name.clone()
        };
        let identity = DeviceIdentity::new(device_id(info, &name))
            .with_manufacturer(MANUFACTURER)
            .with_model(MODEL)
            .with_firmware(env!("CARGO_PKG_VERSION"))
            .with_label(format!("thumbswing ({name})"));
        let client = Client::with_tuning(input, self.settings.config.clone(), self.settings.tuning);
        let mut session = Session {
            client,
            identity,
            input_name: name,
            ids: self.ids.clone(),
            live: self.live.clone(),
            applied: self.settings.clone(),
            hints: TurnHints::default(),
            started: false,
            profile_due: false,
            handed_due: false,
            notes: Vec::new(),
        };
        session.take_hints(hints);
        session
    }
}

/// The FRP `device` id: the file name of the input's first
/// `/dev/input/by-id` link, which names the same device across reconnects
/// and replugs, else `name` (the name the device reports).
fn device_id(info: &DeviceInfo, name: &str) -> String {
    info.by_id
        .first()
        .and_then(|link| link.file_name())
        .map(|file| file.to_string_lossy())
        .filter(|file| !file.is_empty())
        .map_or_else(|| name.to_owned(), std::borrow::Cow::into_owned)
}

/// An input chosen because no `device` was named: after an unplug it is
/// reopened by its `by-id` link when it has one, else chosen again, since
/// the device can come back under another node.
struct Reselect {
    input: BoxedInput,
    opener: Opener,
    grab: bool,
}

impl Input for Reselect {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        self.input.try_recv()
    }

    fn now_us(&mut self) -> u64 {
        self.input.now_us()
    }

    fn info(&self) -> &DeviceInfo {
        self.input.info()
    }

    fn reconnect(&mut self) -> Result<(), Error> {
        if !self.input.info().by_id.is_empty() && self.input.reconnect().is_ok() {
            return Ok(());
        }
        self.input = (self.opener)(None, self.grab)?;
        Ok(())
    }

    fn set_grab(&mut self, grab: bool) -> Result<(), Error> {
        self.input.set_grab(grab)?;
        self.grab = grab;
        Ok(())
    }

    fn paced(&self) -> bool {
        self.input.paced()
    }
}

impl DeviceDriver for Thumbswing {
    type Config = Settings;
    type Error = Error;
    type Action = NoAction;
    type Session = Session;

    /// Opening a device is a few syscalls, and closing joins the reader
    /// thread, which wakes every 100 ms.
    const STOP_WAIT: Duration = Duration::from_secs(1);

    fn info(config: &Settings) -> DriverInfo {
        let info = DriverInfo::new(KIND, MODEL, Source::Virtual).with_manufacturer(MANUFACTURER);
        if config.config.grab {
            info.exclusive()
        } else {
            info
        }
    }

    fn new(config: Settings) -> Result<Self, ConfigError> {
        DriverConfig::validate(&config)?;
        Ok(Self {
            live: Live::new(config.clone()),
            settings: config,
            ids: ShotIds::new(),
            opener: Arc::new(|device, grab| {
                device::open_setting(device, grab).map(|d| Box::new(d) as BoxedInput)
            }),
            given: None,
            given_used: false,
            alerted: None,
        })
    }

    fn config(&self) -> &Settings {
        &self.settings
    }

    fn connect(&mut self, ctx: &mut ConnectCtx<'_>) -> Result<Session, Error> {
        ctx.tick()?;
        ctx.out.phase("connecting");
        let input: BoxedInput = if let Some(given) = self.given.take() {
            self.given_used = true;
            given
        } else if self.given_used {
            return Err(Error::NotFound(
                "the input given to the driver has been served".to_owned(),
            ));
        } else {
            let grab = self.settings.config.grab;
            let input = (self.opener)(self.settings.device(), grab)?;
            let info = input.info();
            ctx.out.log(
                Level::Info,
                format!(
                    "opened {}{}",
                    info.name,
                    if info.grabbed { ", grabbed" } else { "" }
                ),
            );
            ctx.out
                .log(Level::Debug, format!("input at {}", info.path.display()));
            if self.settings.device().is_none() {
                Box::new(Reselect {
                    input,
                    opener: Arc::clone(&self.opener),
                    grab,
                })
            } else {
                input
            }
        };
        // A stop while the device opened releases it here (dropping it closes it).
        ctx.tick()?;
        self.alerted = None;
        Ok(self.session(input, *ctx.hints()))
    }

    fn retry_delay(
        &mut self,
        ended: &SessionEnd<'_, Error>,
        backoff: &mut Backoff,
        out: &mut Events<'_>,
    ) -> Option<Duration> {
        let give_up = matches!(ended.error, Some(Error::Unsupported))
            || (self.given_used && self.given.is_none());
        if let Some(error) = ended.error {
            out.log(Level::Debug, format!("connect failed: {error}"));
            let outage = Outage::of(error);
            if outage.is_none() || outage != self.alerted {
                let severity = if give_up {
                    Severity::Error
                } else {
                    Severity::Warn
                };
                out.alert(severity, self.alert_text(error));
            }
            self.alerted = outage;
        }
        if give_up {
            None
        } else {
            Some(backoff.delay(ended.connected_for))
        }
    }

    fn apply_config(&mut self, new: &Settings) {
        self.settings = new.clone();
        self.live.set(new.clone());
    }
}

/// A connected trackball (or the input a driver was given).
pub struct Session {
    client: Client<BoxedInput>,
    identity: DeviceIdentity,
    input_name: String,
    ids: ShotIds,
    live: Live<Settings>,
    /// The settings in effect.
    applied: Settings,
    /// The hints of the current turn.
    hints: TurnHints,
    /// Whether the session's telemetry has been sent.
    started: bool,
    /// Whether the club changed since the last telemetry.
    profile_due: bool,
    /// Whether the hand changed since the last telemetry.
    handed_due: bool,
    /// Log lines from `hints`, which has no event sink, for the next poll.
    notes: Vec<String>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("identity", &self.identity)
            .field("club", &self.client.club())
            .field("hand", &self.client.hand())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// The client underneath: the club, hand, tuning and swing state in use.
    #[must_use]
    pub fn client(&self) -> &Client<BoxedInput> {
        &self.client
    }

    fn profile(&self) -> (&'static str, String) {
        ("profile", self.client.club().code().to_owned())
    }

    fn handed(&self) -> (&'static str, String) {
        ("handed", self.client.hand().code().to_ascii_lowercase())
    }

    /// All the keys when the session starts, then the club or hand that
    /// changed.
    fn telemetry(&mut self, out: &mut Events<'_>) {
        if !self.started {
            self.started = true;
            self.profile_due = false;
            self.handed_due = false;
            out.telemetry([
                ("synthetic", "true".to_owned()),
                ("input", self.input_name.clone()),
                ("cpi", format!("{}", self.applied.config.cpi)),
                self.profile(),
                self.handed(),
            ]);
            return;
        }
        let mut changed = Vec::new();
        if std::mem::take(&mut self.profile_due) {
            changed.push(self.profile());
        }
        if std::mem::take(&mut self.handed_due) {
            changed.push(self.handed());
        }
        if !changed.is_empty() {
            out.telemetry(changed);
        }
    }

    /// Take a turn's hints, which replace the ones before, and put the club
    /// and hand in play. A club code without a profile is noted and is no
    /// hint.
    fn take_hints(&mut self, hints: TurnHints) {
        if let Some(code) = hints.club
            && let Err(UnknownClub(code)) = Club::try_from(code)
        {
            self.notes.push(format!("no club profile for {code}"));
        }
        self.hints = hints;
        self.select();
    }

    /// Put the club and hand in play: the putter in the putting mode, else
    /// the turn's club, else the setting's; the turn's hand, else the
    /// setting's.
    fn select(&mut self) {
        let club = if self.hints.mode == Some(Mode::Putting) {
            Club::Putter
        } else {
            self.hints
                .club
                .and_then(|code| Club::try_from(code).ok())
                .unwrap_or(self.applied.club)
        };
        let hand = self.hints.handedness.map_or(self.applied.hand, Hand::from);
        if club != self.client.club() {
            self.client.set_club(club);
            self.profile_due = true;
        }
        if hand != self.client.hand() {
            self.client.set_hand(hand);
            self.handed_due = true;
        }
    }

    /// Take the club, hand, thresholds and tuning the driver was given since
    /// the last poll.
    fn follow_live_settings(&mut self) -> Result<(), Error> {
        let live = self.live.get();
        if live.club != self.applied.club || live.hand != self.applied.hand {
            self.applied.club = live.club;
            self.applied.hand = live.hand;
            self.select();
        }
        if live.tuning != self.applied.tuning {
            *self.client.tuning_mut() = live.tuning;
            self.applied.tuning = live.tuning;
        }
        if live.config.thresholds != self.applied.config.thresholds {
            let mut config = self.client.config().clone();
            config.thresholds = live.config.thresholds;
            self.client.set_config(config)?;
            self.applied.config.thresholds = live.config.thresholds;
        }
        Ok(())
    }

    fn handle(&mut self, event: Event, out: &mut Events<'_>) {
        match event {
            Event::Ready => {
                out.ready(true);
                out.log(Level::Info, "ready: pull back, then push");
            }
            Event::Disconnected => {
                out.ready(false);
                out.log(
                    Level::Warn,
                    "input device disconnected; waiting for it to come back",
                );
            }
            Event::Backswing { dir_deg, len_in } => {
                if out.enabled(Level::Debug) {
                    out.log(
                        Level::Debug,
                        format!("backswing {dir_deg:+.1}° {len_in:.2} in"),
                    );
                }
            }
            Event::Saturated { reports } => out.log(
                Level::Warn,
                format!(
                    "saturated: {reports} reports at the per-report limit, so the speed is a \
                     lower bound; lower tuning.full_power_speed, or set report_limit to the \
                     device's real limit"
                ),
            ),
            Event::Canceled(CancelReason::Dribble) => out.log(
                Level::Debug,
                "dribble ignored (the backswing is still coiled)",
            ),
            Event::Canceled(reason) => {
                out.log(Level::Debug, format!("swing canceled: {reason:?}"));
            }
            Event::Shot(shot) => self.send_shot(&shot, out),
        }
    }

    /// The UTC time of the swing's last report (the end of the push), in ms:
    /// now, less how long ago that report was on the input's clock.
    fn strike_ms(&mut self, shot: &Shot) -> Option<u64> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        let ago_us = shot.raw.last().map_or(0, |last| {
            self.client.input_mut().now_us().saturating_sub(last.t_us)
        });
        let strike = now.checked_sub(Duration::from_micros(ago_us))?;
        u64::try_from(strike.as_millis()).ok()
    }

    fn send_shot(&mut self, shot: &Shot, out: &mut Events<'_>) {
        if !out.shots_allowed() {
            out.log(
                Level::Info,
                format!(
                    "swing not sent: the host is holding shots ({:.1} mph ball)",
                    shot.launch.ball_speed_mph
                ),
            );
            return;
        }
        let key = self.ids.next_key();
        match self.strike_ms(shot) {
            Some(ms) => out.shot_trigger_at(key.clone(), ms),
            None => out.shot_trigger(key.clone()),
        }
        out.launch(key.clone(), launch(&shot.launch));
        out.club(key.clone(), club_data(shot));
        out.shot_finished(key);
    }
}

impl DeviceSession for Session {
    type Driver = Thumbswing;

    fn poll(&mut self, timeout: Duration, out: &mut Events<'_>) -> Result<(), Error> {
        for note in self.notes.drain(..) {
            out.log(Level::Info, note);
        }
        self.follow_live_settings()?;
        self.telemetry(out);
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(event) = self.client.poll()? {
                self.handle(event, out);
            } else if self.client.input().paced() {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Ok(());
                }
                std::thread::sleep(left.min(IDLE_SLEEP));
            }
            if Instant::now() >= deadline {
                return Ok(());
            }
        }
    }

    fn hints(&mut self, hints: &TurnHints) -> Result<(), Error> {
        self.take_hints(*hints);
        Ok(())
    }

    fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    fn close(self, out: &mut Events<'_>) -> Result<(), Error> {
        out.ready(false);
        // Dropping the client closes the input, which releases the grab.
        drop(self.client);
        Ok(())
    }
}

impl TryFrom<flightrelay::Club> for Club {
    type Error = UnknownClub;

    /// FRP codes without a row of their own map to the nearest row; a code
    /// this crate does not know is an error.
    fn try_from(c: flightrelay::Club) -> Result<Self, Self::Error> {
        let code = c.to_string();
        Club::from_code(&code).ok_or(UnknownClub(code))
    }
}

impl From<Handedness> for Hand {
    fn from(h: Handedness) -> Self {
        match h {
            Handedness::Right => Hand::Right,
            Handedness::Left => Hand::Left,
        }
    }
}

/// The FRP club code for a thumbswing club.
#[must_use]
pub fn frp_club(club: Club) -> Option<flightrelay::Club> {
    club.code().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frp_club_maps_to_a_row() {
        for c in flightrelay::Club::ALL {
            assert!(Club::try_from(*c).is_ok(), "{c}");
        }
        for c in Club::ALL {
            assert_eq!(
                frp_club(c).map(|f| f.to_string()),
                Some(c.code().to_owned())
            );
        }
        assert_eq!(
            Club::try_from(flightrelay::Club::Hybrid5),
            Ok(Club::Hybrid3)
        );
    }

    #[test]
    fn info_is_virtual_and_exclusive_only_with_the_grab() {
        let mut settings = Settings::default();
        let info = Thumbswing::info(&settings);
        assert_eq!(info.source, Source::Virtual);
        assert_eq!(info.kind, KIND);
        assert_eq!(info.manufacturer, Some(MANUFACTURER));
        assert!(info.raw.is_none(), "no wire protocol, no raw frames");
        assert!(info.exclusive);
        settings.config.grab = false;
        assert!(!Thumbswing::info(&settings).exclusive);
    }

    #[test]
    fn new_refuses_invalid_settings_without_opening_anything() {
        let mut settings = Settings::default();
        settings.tuning.power_gamma = 0.0;
        let e = Thumbswing::new(settings).unwrap_err();
        assert_eq!(e.field.as_deref(), Some("tuning.power_gamma"));
        assert!(Thumbswing::new(Settings::default()).is_ok());
    }
}
