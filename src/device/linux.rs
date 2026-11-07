//! Linux backend: evdev nodes through `read(2)`, `poll(2)` and the evdev
//! ioctls (no bindings).
//!
//! The device is opened non-blocking. A reader thread waits in `poll(2)` with
//! a 100 ms timeout, so it notices a stop request promptly, sums each report's
//! `REL_X`/`REL_Y` events up to `SYN_REPORT` into one [`Sample`] and sends it
//! over a channel; [`Input::try_recv`] never blocks. Timestamps are switched
//! to `CLOCK_MONOTONIC` with `EVIOCSCLOCKID`, the clock [`Input::now_us`]
//! reads. `ENODEV` (the device was unplugged) ends the thread with
//! [`Error::Disconnected`]; [`Input::reconnect`] reopens the same path.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::raw::{c_int, c_ulong};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread::{self, JoinHandle};

use crate::error::Error;
use crate::input::{DeviceInfo, Input};
use crate::sample::Sample;

const EV_SYN: u16 = 0x00;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;
const SYN_REPORT: u16 = 0;
const SYN_DROPPED: u16 = 3;
const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;

const IOC_WRITE: u32 = 1;
const IOC_READ: u32 = 2;

/// `_IOC(dir, 'E', nr, size)` in the asm-generic encoding.
const fn ioc(dir: u32, nr: u32, size: usize) -> c_ulong {
    #[allow(clippy::cast_possible_truncation)]
    let size = size as u32;
    ((dir << 30) | (size << 16) | ((b'E' as u32) << 8) | nr) as c_ulong
}

const EVIOCGID: c_ulong = ioc(IOC_READ, 0x02, 8);
const EVIOCGRAB: c_ulong = ioc(IOC_WRITE, 0x90, size_of::<c_int>());
const EVIOCSCLOCKID: c_ulong = ioc(IOC_WRITE, 0xa0, size_of::<c_int>());

const fn eviocgname(len: usize) -> c_ulong {
    ioc(IOC_READ, 0x06, len)
}

const fn eviocgbit(ev: u16, len: usize) -> c_ulong {
    ioc(IOC_READ, 0x20 + ev as u32, len)
}

/// Poll timeout of the reader thread, ms.
const POLL_TIMEOUT_MS: c_int = 100;

fn errno() -> io::Error {
    io::Error::last_os_error()
}

/// `CLOCK_MONOTONIC` in microseconds.
pub(crate) fn monotonic_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec for the duration of the call.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
    #[allow(clippy::cast_sign_loss)]
    let us = ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000;
    us
}

fn open_fd(path: &Path) -> Result<OwnedFd, Error> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::NotFound(path.display().to_string()))?;
    // SAFETY: `c` is a valid NUL-terminated path; the flags are plain ints.
    let fd = unsafe {
        libc::open(
            c.as_ptr(),
            libc::O_RDONLY | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let e = errno();
        return Err(match e.raw_os_error() {
            Some(libc::EACCES | libc::EPERM) => Error::PermissionDenied {
                path: path.to_owned(),
            },
            Some(libc::ENOENT | libc::ENODEV | libc::ENXIO) => {
                Error::NotFound(path.display().to_string())
            }
            _ => Error::Io(e),
        });
    }
    // SAFETY: `fd` was just returned by open(2) and is owned by nobody else.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// ioctl with a pointer argument.
fn ioctl_ptr<T>(fd: RawFd, req: c_ulong, arg: &mut T) -> io::Result<c_int> {
    // SAFETY: `arg` points to a live, writable `T` whose size matches the size
    // encoded in `req` at every call site.
    let r = unsafe { libc::ioctl(fd, req as _, std::ptr::from_mut(arg)) };
    if r < 0 { Err(errno()) } else { Ok(r) }
}

/// ioctl with an integer argument passed by value (`EVIOCGRAB`).
fn ioctl_val(fd: RawFd, req: c_ulong, arg: c_ulong) -> io::Result<c_int> {
    // SAFETY: the request takes its argument by value; no memory is shared.
    let r = unsafe { libc::ioctl(fd, req as _, arg) };
    if r < 0 { Err(errno()) } else { Ok(r) }
}

fn device_name(fd: RawFd) -> String {
    let mut buf = [0u8; 256];
    match ioctl_ptr(fd, eviocgname(buf.len()), &mut buf) {
        Ok(_) => {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..end]).into_owned()
        }
        Err(_) => String::new(),
    }
}

/// `(bustype, vendor, product, version)`.
fn device_id(fd: RawFd) -> (u16, u16) {
    let mut id = [0u16; 4];
    match ioctl_ptr(fd, EVIOCGID, &mut id) {
        Ok(_) => (id[1], id[2]),
        Err(_) => (0, 0),
    }
}

fn capabilities(fd: RawFd) -> (bool, bool) {
    let mut rel = [0u8; 2];
    let mut abs = [0u8; 8];
    let rel_xy =
        ioctl_ptr(fd, eviocgbit(EV_REL, rel.len()), &mut rel).is_ok() && rel[0] & 0b11 == 0b11;
    let has_abs = ioctl_ptr(fd, eviocgbit(EV_ABS, abs.len()), &mut abs).is_ok()
        && abs.iter().any(|&b| b != 0);
    (rel_xy, has_abs)
}

/// The sysfs directory of an event node, e.g. `/sys/class/input/event7/device`.
fn sysfs_dir(node: &Path) -> Option<PathBuf> {
    let name = node.file_name()?;
    Some(Path::new("/sys/class/input").join(name).join("device"))
}

fn sysfs_read(dir: &Path, file: &str) -> Option<String> {
    fs::read_to_string(dir.join(file))
        .ok()
        .map(|s| s.trim().to_owned())
}

/// A sysfs capability bitmap: space-separated hex words, lowest word last.
fn sysfs_bitmap_low(dir: &Path, file: &str) -> Option<u64> {
    let text = sysfs_read(dir, file)?;
    u64::from_str_radix(text.split_whitespace().last()?, 16).ok()
}

/// Name, ids and capabilities from sysfs, for nodes this user cannot open.
fn sysfs_info(node: &Path, info: &mut DeviceInfo) {
    let Some(dir) = sysfs_dir(node) else {
        return;
    };
    if let Some(name) = sysfs_read(&dir, "name") {
        info.name = name;
    }
    let hex = |f: &str| sysfs_read(&dir, f).and_then(|s| u16::from_str_radix(&s, 16).ok());
    info.vendor = hex("id/vendor").unwrap_or(0);
    info.product = hex("id/product").unwrap_or(0);
    info.relative_xy = sysfs_bitmap_low(&dir, "capabilities/rel").is_some_and(|b| b & 0b11 == 0b11);
    info.absolute = sysfs_read(&dir, "capabilities/abs").is_some_and(|s| {
        s.split_whitespace()
            .any(|w| !w.trim_start_matches('0').is_empty())
    });
}

/// `/dev/input/by-id` links that resolve to `node`.
fn by_id_links(node: &Path) -> Vec<PathBuf> {
    let Ok(dir) = fs::read_dir("/dev/input/by-id") else {
        return Vec::new();
    };
    let mut links: Vec<PathBuf> = dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| fs::canonicalize(p).is_ok_and(|c| c == node))
        .collect();
    // The `-event-mouse` link first: it is the one a trackball is known by.
    links.sort_by_key(|p| {
        let s = p.to_string_lossy();
        (!s.ends_with("-event-mouse"), s.into_owned())
    });
    links
}

fn event_number(p: &Path) -> Option<u32> {
    p.file_name()?.to_str()?.strip_prefix("event")?.parse().ok()
}

/// Every `/dev/input/event*` node, readable or not, with what it is.
///
/// Nodes this user cannot open are listed with `readable: false` and the
/// reason in `error`; their name and capabilities come from sysfs.
///
/// # Errors
/// If `/dev/input` cannot be read.
pub fn list() -> Result<Vec<DeviceInfo>, Error> {
    let dir = fs::read_dir("/dev/input").map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => {
            Error::NotFound("/dev/input does not exist (no input devices visible here)".into())
        }
        _ => Error::Io(e),
    })?;
    let mut nodes: Vec<PathBuf> = dir
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| event_number(p).is_some())
        .collect();
    nodes.sort_by_key(|p| event_number(p).unwrap_or(u32::MAX));
    Ok(nodes.iter().map(|p| probe(p)).collect())
}

fn probe(node: &Path) -> DeviceInfo {
    let mut info = DeviceInfo {
        path: node.to_owned(),
        by_id: by_id_links(node),
        ..DeviceInfo::default()
    };
    match open_fd(node) {
        Ok(fd) => {
            describe(fd.as_raw_fd(), &mut info);
            info.readable = true;
        }
        Err(e) => {
            sysfs_info(node, &mut info);
            info.error = Some(e.to_string());
        }
    }
    info
}

fn describe(fd: RawFd, info: &mut DeviceInfo) {
    info.name = device_name(fd);
    (info.vendor, info.product) = device_id(fd);
    (info.relative_xy, info.absolute) = capabilities(fd);
}

/// What a [`Device`] was opened by: a path (a `by-id` link stays valid across
/// replugs) or a device name.
#[derive(Debug, Clone)]
enum Source {
    Path(PathBuf),
    Name(String),
}

enum Msg {
    Sample(Sample),
    Disconnected,
    Failed(io::Error),
}

/// An opened trackball or mouse.
///
/// Dropping it stops the reader thread and closes the device, which also
/// releases the grab.
pub struct Device {
    info: DeviceInfo,
    /// How to find the device again on reconnect.
    source: Source,
    want_grab: bool,
    fd: Option<Arc<OwnedFd>>,
    rx: Option<Receiver<Msg>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Device {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Device")
            .field("info", &self.info)
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

impl Device {
    /// Open the device at `path` (an event node or a `/dev/input/by-id` link),
    /// with an exclusive grab if `grab`.
    ///
    /// # Errors
    /// [`Error::PermissionDenied`] (with the fix in the message),
    /// [`Error::NotFound`], [`Error::NotAPointer`] for a device without
    /// relative X/Y, [`Error::Busy`] if another process holds the grab.
    pub fn open(path: impl AsRef<Path>, grab: bool) -> Result<Self, Error> {
        Self::open_source(Source::Path(path.as_ref().to_owned()), grab)
    }

    /// Open the first readable pointer whose reported name is `name`. A
    /// reconnect looks the name up again.
    ///
    /// # Errors
    /// [`Error::NotFound`] if none matches; otherwise as [`Device::open`].
    pub fn open_by_name(name: &str, grab: bool) -> Result<Self, Error> {
        Self::open_source(Source::Name(name.to_owned()), grab)
    }

    fn open_source(source: Source, grab: bool) -> Result<Self, Error> {
        let mut dev = Self {
            info: DeviceInfo::default(),
            source,
            want_grab: grab,
            fd: None,
            rx: None,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        dev.start()?;
        Ok(dev)
    }

    fn resolve(&self) -> Result<PathBuf, Error> {
        match &self.source {
            Source::Path(p) => fs::canonicalize(p).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => Error::NotFound(p.display().to_string()),
                _ => Error::Io(e),
            }),
            Source::Name(name) => list()?
                .into_iter()
                .find(|d| d.name == *name && d.readable && d.is_pointer())
                .map(|d| d.path)
                .ok_or_else(|| Error::NotFound(format!("no readable pointer named {name:?}"))),
        }
    }

    fn start(&mut self) -> Result<(), Error> {
        self.shutdown();
        let node = self.resolve()?;
        let fd = open_fd(&node)?;
        let raw = fd.as_raw_fd();
        let mut info = DeviceInfo {
            path: node.clone(),
            by_id: by_id_links(&node),
            readable: true,
            ..DeviceInfo::default()
        };
        describe(raw, &mut info);
        if !info.relative_xy {
            return Err(Error::NotAPointer { path: node });
        }
        let mut clock: c_int = libc::CLOCK_MONOTONIC;
        info.kernel_timestamps = ioctl_ptr(raw, EVIOCSCLOCKID, &mut clock).is_ok();
        if self.want_grab {
            grab_fd(raw, true, &node)?;
            info.grabbed = true;
        }

        let fd = Arc::new(fd);
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let fd = Arc::clone(&fd);
            let stop = Arc::clone(&stop);
            let kernel_ts = info.kernel_timestamps;
            thread::Builder::new()
                .name("thumbswing-evdev".into())
                .spawn(move || reader(&fd, &tx, &stop, kernel_ts))?
        };
        self.info = info;
        self.fd = Some(fd);
        self.rx = Some(rx);
        self.stop = stop;
        self.thread = Some(thread);
        Ok(())
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.rx = None;
        self.fd = None;
    }
}

fn grab_fd(fd: RawFd, grab: bool, node: &Path) -> Result<(), Error> {
    match ioctl_val(fd, EVIOCGRAB, c_ulong::from(grab)) {
        Ok(_) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EBUSY) => Err(Error::Busy {
            path: node.to_owned(),
        }),
        // Releasing a grab that is not held is not an error worth reporting.
        Err(_) if !grab => Ok(()),
        Err(e) => Err(Error::Io(e)),
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Input for Device {
    fn try_recv(&mut self) -> Result<Option<Sample>, Error> {
        let Some(rx) = &self.rx else {
            return Err(Error::Disconnected);
        };
        match rx.try_recv() {
            Ok(Msg::Sample(s)) => Ok(Some(s)),
            Ok(Msg::Disconnected) | Err(TryRecvError::Disconnected) => {
                self.shutdown();
                Err(Error::Disconnected)
            }
            Ok(Msg::Failed(e)) => {
                self.shutdown();
                Err(Error::Io(e))
            }
            Err(TryRecvError::Empty) => Ok(None),
        }
    }

    fn now_us(&mut self) -> u64 {
        monotonic_us()
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn reconnect(&mut self) -> Result<(), Error> {
        self.start()
    }

    fn set_grab(&mut self, grab: bool) -> Result<(), Error> {
        self.want_grab = grab;
        if let Some(fd) = &self.fd {
            grab_fd(fd.as_raw_fd(), grab, &self.info.path)?;
            self.info.grabbed = grab;
        }
        Ok(())
    }
}

#[allow(clippy::needless_pass_by_value)]
fn reader(fd: &OwnedFd, tx: &Sender<Msg>, stop: &AtomicBool, kernel_ts: bool) {
    const EMPTY: libc::input_event = libc::input_event {
        time: libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        },
        type_: 0,
        code: 0,
        value: 0,
    };
    let raw = fd.as_raw_fd();
    let mut buf = [EMPTY; 64];
    let (mut dx, mut dy) = (0i32, 0i32);
    let mut dropping = false;

    while !stop.load(Ordering::Relaxed) {
        let mut pfd = libc::pollfd {
            fd: raw,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pfd` is one valid pollfd and the count is 1.
        let r = unsafe { libc::poll(&raw mut pfd, 1, POLL_TIMEOUT_MS) };
        if r < 0 {
            let e = errno();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            let _ = tx.send(Msg::Failed(e));
            return;
        }
        if r == 0 {
            continue;
        }
        if pfd.revents & libc::POLLIN == 0
            && pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0
        {
            let _ = tx.send(Msg::Disconnected);
            return;
        }
        loop {
            // SAFETY: `buf` is a writable array of `input_event`s and the
            // length passed is its size in bytes.
            let n = unsafe {
                libc::read(
                    raw,
                    buf.as_mut_ptr().cast::<libc::c_void>(),
                    size_of_val(&buf),
                )
            };
            if n < 0 {
                let e = errno();
                match e.raw_os_error() {
                    Some(libc::EAGAIN) => break,
                    Some(libc::EINTR) => continue,
                    Some(libc::ENODEV) => {
                        let _ = tx.send(Msg::Disconnected);
                        return;
                    }
                    _ => {
                        let _ = tx.send(Msg::Failed(e));
                        return;
                    }
                }
            }
            if n == 0 {
                let _ = tx.send(Msg::Disconnected);
                return;
            }
            #[allow(clippy::cast_sign_loss)]
            let count = n as usize / size_of::<libc::input_event>();
            for ev in &buf[..count] {
                match (ev.type_, ev.code) {
                    (EV_REL, REL_X) => dx = dx.saturating_add(ev.value),
                    (EV_REL, REL_Y) => dy = dy.saturating_add(ev.value),
                    (EV_SYN, SYN_DROPPED) => {
                        dropping = true;
                        (dx, dy) = (0, 0);
                    }
                    (EV_SYN, SYN_REPORT) => {
                        if !dropping && (dx != 0 || dy != 0) {
                            #[allow(clippy::cast_sign_loss)]
                            let t_us = if kernel_ts {
                                ev.time.tv_sec as u64 * 1_000_000 + ev.time.tv_usec as u64
                            } else {
                                monotonic_us()
                            };
                            if tx.send(Msg::Sample(Sample::new(t_us, dx, dy))).is_err() {
                                return;
                            }
                        }
                        dropping = false;
                        (dx, dy) = (0, 0);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctl_numbers_match_the_kernel_headers() {
        // Values from <linux/input.h> on x86_64.
        assert_eq!(EVIOCGID, 0x8008_4502);
        assert_eq!(EVIOCGRAB, 0x4004_4590);
        assert_eq!(EVIOCSCLOCKID, 0x4004_45a0);
        assert_eq!(eviocgname(256), 0x8100_4506);
        assert_eq!(eviocgbit(EV_REL, 2), 0x8002_4522);
    }

    #[test]
    fn input_event_is_the_kernel_size() {
        assert_eq!(size_of::<libc::input_event>(), 24);
    }

    #[test]
    fn monotonic_clock_advances() {
        let a = monotonic_us();
        let b = monotonic_us();
        assert!(b >= a && a > 0);
    }
}
