//! Backend for platforms without trackball input.

use std::path::Path;

use crate::error::Error;
use crate::input::{DeviceInfo, Input};
use crate::sample::Sample;

/// List input devices. Always [`Error::Unsupported`] on this platform.
///
/// # Errors
/// Always.
pub fn list() -> Result<Vec<DeviceInfo>, Error> {
    Err(Error::Unsupported)
}

/// An opened input device. Cannot be constructed on this platform.
#[derive(Debug)]
pub struct Device {
    info: DeviceInfo,
}

impl Device {
    /// Open a device. Always [`Error::Unsupported`] on this platform.
    ///
    /// # Errors
    /// Always.
    pub fn open(path: impl AsRef<Path>, grab: bool) -> Result<Self, Error> {
        let _ = (path, grab);
        Err(Error::Unsupported)
    }

    /// Open a device by name. Always [`Error::Unsupported`] on this platform.
    ///
    /// # Errors
    /// Always.
    pub fn open_by_name(name: &str, grab: bool) -> Result<Self, Error> {
        let _ = (name, grab);
        Err(Error::Unsupported)
    }
}

impl Input for Device {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        Err(Error::Unsupported)
    }

    fn now_us(&mut self) -> u64 {
        0
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }
}
