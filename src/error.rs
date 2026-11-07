//! Error type.

use std::io;
use std::path::PathBuf;

/// Errors from opening and reading an input device, and from recordings.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Trackball input is not implemented on this platform (Linux only for now).
    #[error("trackball input is not supported on this platform (Linux only)")]
    Unsupported,

    /// The device node exists but this user may not read it.
    #[error(
        "permission denied opening {path}: add your user to the `input` group \
         (`sudo usermod -aG input $USER`, then log out and back in) or install a \
         udev rule that tags input devices `uaccess`"
    )]
    PermissionDenied {
        /// The node that could not be opened.
        path: PathBuf,
    },

    /// Another process holds an exclusive grab on the device.
    #[error("{path} is grabbed by another process (exclusive grab, EBUSY)")]
    Busy {
        /// The grabbed node.
        path: PathBuf,
    },

    /// No device matched the path or name.
    #[error("no input device found: {0}")]
    NotFound(String),

    /// No device was named and more than one readable trackball or mouse is
    /// present, so none is chosen.
    #[error(
        "found {} trackballs/mice, choose one with the `device` setting: {}",
        devices.len(),
        devices.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
    )]
    Ambiguous {
        /// Each candidate's stable path (its `by-id` link, else the node).
        devices: Vec<PathBuf>,
    },

    /// The device has no relative X/Y axes, so it is not a trackball or mouse.
    #[error("{path} has no relative X/Y axes (not a trackball or mouse)")]
    NotAPointer {
        /// The rejected node.
        path: PathBuf,
    },

    /// The device went away (unplugged).
    #[error("input device disconnected")]
    Disconnected,

    /// A host stopped a connect before it finished.
    #[error("stopped")]
    Stopped,

    /// A recording could not be parsed.
    #[error("{file}:{line}: {message}")]
    Parse {
        /// The file, or a placeholder for in-memory text.
        file: String,
        /// 1-based line number.
        line: usize,
        /// What was wrong.
        message: String,
    },

    /// Any other I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[cfg(feature = "driver")]
impl From<flightrelay::driver::Stopped> for Error {
    fn from(_: flightrelay::driver::Stopped) -> Self {
        Self::Stopped
    }
}
