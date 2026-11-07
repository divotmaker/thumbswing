//! The device table `examples/list` prints, and the choice of device for
//! `examples/monitor`.

#![allow(dead_code)]

use std::fmt::Write as _;
use std::path::PathBuf;

use thumbswing::device::Device;
use thumbswing::{DeviceInfo, Error, device};

/// One line per device node, followed by its `by-id` links.
pub fn table<'a>(devices: impl IntoIterator<Item = &'a DeviceInfo>) -> String {
    let mut out = String::new();
    for d in devices {
        let kind = if d.is_pointer() {
            "pointer"
        } else if d.relative_xy {
            "rel+abs"
        } else {
            "-"
        };
        let _ = writeln!(
            out,
            "{:<20} {:<9} {:<8} {:04x}:{:04x}  {}",
            d.path.display(),
            if d.readable { "readable" } else { "NO ACCESS" },
            kind,
            d.vendor,
            d.product,
            d.name
        );
        for link in &d.by_id {
            let _ = writeln!(out, "{:<20} {}", "", link.display());
        }
    }
    out
}

/// The device to open from `devices` when no `--device` is given: the one
/// [`device::choose`] picks. Its errors are reworded for the command line,
/// listing the devices when there is more than one.
///
/// # Errors
/// No readable trackball or mouse, or more than one.
pub fn choose(devices: &[DeviceInfo]) -> Result<PathBuf, String> {
    let pointers = |readable: bool| {
        devices
            .iter()
            .filter(move |d| d.readable == readable && d.is_pointer())
    };
    device::choose(devices).map_err(|e| match e {
        Error::Ambiguous { devices: paths } => format!(
            "found {} trackballs/mice; choose one with --device <path, by-id name or device name>:\n{}",
            paths.len(),
            table(pointers(true)).trim_end()
        ),
        Error::PermissionDenied { .. } => format!(
            "found {} trackball/mouse node(s) but cannot read them: add your user to the \
             `input` group (sudo usermod -aG input $USER, then log in again); `cargo run \
             --example list` shows them",
            pointers(false).count()
        ),
        _ => "no trackball or mouse found (cargo run --example list)".to_owned(),
    })
}

/// Open the `--device` value the way the library's `device` setting is
/// opened ([`device::open_setting`]: a path, a by-id name, or the name the
/// device reports); without one, the device [`choose`] picks.
///
/// # Errors
/// As [`choose`], or when the device cannot be listed or opened.
pub fn open(arg: Option<&str>, grab: bool) -> Result<Device, String> {
    if let Some(a) = arg {
        return device::open_setting(Some(a), grab).map_err(|e| format!("{a}: {e}"));
    }
    let all = device::list().map_err(|e| e.to_string())?;
    let path = choose(&all)?;
    Device::open(&path, grab).map_err(|e| format!("{}: {e}", path.display()))
}
