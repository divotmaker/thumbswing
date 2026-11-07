//! The input abstraction: anything that yields [`Sample`]s on a clock.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::sample::Sample;

/// A source of [`Sample`]s.
///
/// Implemented by the Linux evdev backend ([`device::Device`](crate::device::Device))
/// and by [`Replay`](crate::recording::Replay) for recordings.
pub trait Input {
    /// The next report, if one is waiting. Never blocks.
    ///
    /// # Errors
    /// [`Error::Disconnected`] when the device has gone away; other errors
    /// for read failures.
    fn try_recv(&mut self) -> Result<Option<Sample>, Error>;

    /// The current time in microseconds, on the clock the samples are
    /// stamped with. Silence advances it, which drives the segmenter's timers.
    fn now_us(&mut self) -> u64;

    /// What the input is.
    fn info(&self) -> &DeviceInfo;

    /// Reopen the device after [`Error::Disconnected`].
    ///
    /// # Errors
    /// If the device is still absent; the default refuses.
    fn reconnect(&mut self) -> Result<(), Error> {
        Err(Error::Disconnected)
    }

    /// Take or release the exclusive grab, where the input has one.
    ///
    /// # Errors
    /// [`Error::Busy`] if another process holds the grab.
    fn set_grab(&mut self, grab: bool) -> Result<(), Error> {
        let _ = grab;
        Ok(())
    }

    /// Whether the input's clock keeps to the wall clock, so a caller
    /// polling it in a loop may sleep while it has nothing. An input whose
    /// clock moves on every poll (a fast [`Replay`](crate::recording::Replay)
    /// with reports left) returns `false`: it is polled again at once.
    fn paced(&self) -> bool {
        true
    }
}

impl<T: Input + ?Sized> Input for Box<T> {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        (**self).try_recv()
    }

    fn now_us(&mut self) -> u64 {
        (**self).now_us()
    }

    fn info(&self) -> &DeviceInfo {
        (**self).info()
    }

    fn reconnect(&mut self) -> Result<(), Error> {
        (**self).reconnect()
    }

    fn set_grab(&mut self, grab: bool) -> Result<(), Error> {
        (**self).set_grab(grab)
    }

    fn paced(&self) -> bool {
        (**self).paced()
    }
}

/// An input device, as listed or opened.
// The flags are independent facts about the device, not a state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    /// The device node, e.g. `/dev/input/event7`, or the recording's path.
    pub path: PathBuf,
    /// Stable `/dev/input/by-id` names for the node.
    pub by_id: Vec<PathBuf>,
    /// The name the device reports.
    pub name: String,
    /// USB (or bus) vendor id.
    pub vendor: u16,
    /// USB (or bus) product id.
    pub product: u16,
    /// Whether it has relative X and Y axes.
    pub relative_xy: bool,
    /// Whether it has any absolute axis (touchpads, tablets, joysticks).
    pub absolute: bool,
    /// Whether this user can open it.
    pub readable: bool,
    /// Why it could not be opened, when it could not.
    pub error: Option<String>,
    /// Whether samples carry the kernel's monotonic timestamps (otherwise
    /// they are stamped when read).
    pub kernel_timestamps: bool,
    /// Whether the device is held with an exclusive grab.
    pub grabbed: bool,
}

impl DeviceInfo {
    /// A trackball or mouse: relative X/Y and no absolute axes.
    #[must_use]
    pub fn is_pointer(&self) -> bool {
        self.relative_xy && !self.absolute
    }

    /// The name to store settings under: the first `by-id` link, else the node.
    #[must_use]
    pub fn stable_path(&self) -> &Path {
        self.by_id
            .first()
            .map_or(self.path.as_path(), PathBuf::as_path)
    }
}
