//! Opening and listing trackballs and mice.
//!
//! Linux reads evdev nodes (`/dev/input/event*`) directly. On other platforms
//! [`list`] and [`Device::open`] return [`Error::Unsupported`],
//! so code that uses them builds everywhere.
//!
//! [`open_setting`] opens the input a [`Settings::device`](crate::Settings::device)
//! value names, and [`choose`] picks the device when none is named.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::input::DeviceInfo;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{Device, list};

#[cfg(not(target_os = "linux"))]
mod unsupported;
#[cfg(not(target_os = "linux"))]
pub use unsupported::{Device, list};

/// Whether this platform has a device backend.
pub const SUPPORTED: bool = cfg!(target_os = "linux");

/// Where a bare device name (one without a `/`) is looked up first.
pub const BY_ID_DIR: &str = "/dev/input/by-id";

/// The device to open when none is named: the only readable trackball or
/// mouse in `devices`, by its `by-id` link when it has one.
///
/// # Errors
/// [`Error::Ambiguous`] listing them when there is more than one;
/// [`Error::PermissionDenied`] when the only trackballs or mice cannot be
/// read; [`Error::NotFound`] when there is none.
pub fn choose(devices: &[DeviceInfo]) -> Result<PathBuf, Error> {
    let readable: Vec<&DeviceInfo> = devices
        .iter()
        .filter(|d| d.readable && d.is_pointer())
        .collect();
    match readable.as_slice() {
        [one] => Ok(one.stable_path().to_owned()),
        [] => match devices.iter().find(|d| !d.readable && d.is_pointer()) {
            Some(locked) => Err(Error::PermissionDenied {
                path: locked.stable_path().to_owned(),
            }),
            None => Err(Error::NotFound("no trackball or mouse found".to_owned())),
        },
        many => Err(Error::Ambiguous {
            devices: many.iter().map(|d| d.stable_path().to_owned()).collect(),
        }),
    }
}

/// Open the input a `device` setting names, with an exclusive grab if
/// `grab`:
///
/// - unset: the device [`choose`] picks from [`list`];
/// - a value with a `/`: that path (an event node or a `by-id` link);
/// - a bare name: the [`BY_ID_DIR`] link of that name when it exists, else
///   the first readable trackball or mouse that reports that name
///   ([`Device::open_by_name`], for devices without a `by-id` link).
///
/// # Errors
/// As [`choose`] and [`Device::open`]; [`Error::Unsupported`] off Linux.
pub fn open_setting(device: Option<&str>, grab: bool) -> Result<Device, Error> {
    match device {
        None => Device::open(choose(&list()?)?, grab),
        Some(path) if path.contains('/') => Device::open(path, grab),
        Some(name) => {
            let link = Path::new(BY_ID_DIR).join(name);
            if link.exists() {
                Device::open(link, grab)
            } else {
                Device::open_by_name(name, grab).map_err(|e| match e {
                    Error::NotFound(_) => Error::NotFound(format!(
                        "{name:?}: no such link in {BY_ID_DIR} and no readable trackball or \
                         mouse by that name"
                    )),
                    other => other,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pointer(node: &str, by_id: &str, readable: bool) -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from(node),
            by_id: vec![PathBuf::from(by_id)],
            relative_xy: true,
            readable,
            ..DeviceInfo::default()
        }
    }

    fn keyboard() -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from("/dev/input/event2"),
            readable: true,
            ..DeviceInfo::default()
        }
    }

    #[test]
    fn the_only_readable_pointer_is_chosen_by_its_link() {
        let ball = pointer("/dev/input/event5", "/dev/input/by-id/ball", true);
        let locked = pointer("/dev/input/event9", "/dev/input/by-id/locked", false);
        assert_eq!(
            choose(&[keyboard(), ball, locked]).unwrap(),
            PathBuf::from("/dev/input/by-id/ball")
        );
    }

    #[test]
    fn several_pointers_are_refused_and_listed() {
        let a = pointer("/dev/input/event5", "/dev/input/by-id/a", true);
        let b = pointer("/dev/input/event7", "/dev/input/by-id/b", true);
        let e = choose(&[keyboard(), a, b]).unwrap_err();
        assert!(matches!(&e, Error::Ambiguous { devices } if devices.len() == 2));
        let text = e.to_string();
        assert!(text.contains("found 2 trackballs/mice"), "{text}");
        assert!(
            text.contains("/dev/input/by-id/a, /dev/input/by-id/b"),
            "{text}"
        );
        assert!(text.contains("`device` setting"), "{text}");
    }

    #[test]
    fn unreadable_or_missing_pointers_say_why() {
        let locked = pointer("/dev/input/event9", "/dev/input/by-id/locked", false);
        assert!(matches!(
            choose(&[keyboard(), locked]),
            Err(Error::PermissionDenied { path }) if path == Path::new("/dev/input/by-id/locked")
        ));
        assert!(matches!(choose(&[keyboard()]), Err(Error::NotFound(_))));
    }
}
