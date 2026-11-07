//! End to end through the kernel: a virtual mouse made with uinput, opened by
//! name through the evdev backend, a swing written into it in real time, then
//! an unplug and replug.
//!
//! Needs a writable `/dev/uinput` (e.g. `sudo chmod 666 /dev/uinput`, or a
//! udev rule giving group `input` access) and read access to the new
//! `/dev/input/event*` node, so it is ignored by default:
//!
//! ```sh
//! cargo test --features testing --test uinput -- --ignored
//! ```

#![cfg(target_os = "linux")]
#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::ffi::CString;
use std::os::raw::{c_int, c_ulong};
use std::thread;
use std::time::{Duration, Instant};

use thumbswing::device::Device;
use thumbswing::testing::StreamBuilder;
use thumbswing::{Client, Config, Event, Sample};

const NAME: &str = "thumbswing-uinput-test";

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const BTN_LEFT: c_int = 0x110;

const fn ioc(dir: u32, nr: u32, size: u32) -> c_ulong {
    ((dir << 30) | (size << 16) | ((b'U' as u32) << 8) | nr) as c_ulong
}
const UI_DEV_CREATE: c_ulong = ioc(0, 1, 0);
const UI_DEV_DESTROY: c_ulong = ioc(0, 2, 0);
const UI_DEV_SETUP: c_ulong = ioc(1, 3, 92);
const UI_SET_EVBIT: c_ulong = ioc(1, 100, 4);
const UI_SET_KEYBIT: c_ulong = ioc(1, 101, 4);
const UI_SET_RELBIT: c_ulong = ioc(1, 102, 4);

#[repr(C)]
struct UinputSetup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; 80],
    ff_effects_max: u32,
}

/// A virtual relative mouse, destroyed on drop.
struct VirtualMouse(c_int);

impl VirtualMouse {
    fn create() -> Self {
        let path = CString::new("/dev/uinput").unwrap();
        // SAFETY: valid NUL-terminated path, plain flags.
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        assert!(
            fd >= 0,
            "cannot open /dev/uinput: {} (needs write access)",
            std::io::Error::last_os_error()
        );
        // SAFETY: each request takes its int argument by value.
        unsafe {
            assert!(libc::ioctl(fd, UI_SET_EVBIT as _, c_int::from(EV_KEY)) >= 0);
            assert!(libc::ioctl(fd, UI_SET_KEYBIT as _, BTN_LEFT) >= 0);
            assert!(libc::ioctl(fd, UI_SET_EVBIT as _, c_int::from(EV_REL)) >= 0);
            assert!(libc::ioctl(fd, UI_SET_RELBIT as _, 0 as c_int) >= 0);
            assert!(libc::ioctl(fd, UI_SET_RELBIT as _, 1 as c_int) >= 0);
        }
        let mut setup = UinputSetup {
            bustype: 0x03,
            vendor: 0x1234,
            product: 0x5678,
            version: 1,
            name: [0; 80],
            ff_effects_max: 0,
        };
        setup.name[..NAME.len()].copy_from_slice(NAME.as_bytes());
        // SAFETY: `setup` matches `struct uinput_setup` (92 bytes) and is live.
        unsafe {
            assert!(libc::ioctl(fd, UI_DEV_SETUP as _, &raw mut setup) >= 0);
            assert!(libc::ioctl(fd, UI_DEV_CREATE as _) >= 0);
        }
        Self(fd)
    }

    fn emit(&self, ty: u16, code: u16, value: i32) {
        let ev = libc::input_event {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_: ty,
            code,
            value,
        };
        // SAFETY: writes one fully initialized input_event from the stack.
        let n = unsafe {
            libc::write(
                self.0,
                (&raw const ev).cast::<libc::c_void>(),
                size_of::<libc::input_event>(),
            )
        };
        assert_eq!(n as usize, size_of::<libc::input_event>());
    }

    /// Write `samples` with their timing, starting now.
    fn play(&self, samples: &[Sample]) {
        let start = Instant::now();
        let t0 = samples.first().map_or(0, |s| s.t_us);
        for s in samples {
            let due = Duration::from_micros(s.t_us - t0);
            if let Some(wait) = due.checked_sub(start.elapsed()) {
                thread::sleep(wait);
            }
            if s.dx != 0 {
                self.emit(EV_REL, 0, s.dx);
            }
            if s.dy != 0 {
                self.emit(EV_REL, 1, s.dy);
            }
            self.emit(EV_SYN, 0, 0);
        }
    }
}

impl Drop for VirtualMouse {
    fn drop(&mut self) {
        // SAFETY: the fd is ours; destroy then close.
        unsafe {
            libc::ioctl(self.0, UI_DEV_DESTROY as _);
            libc::close(self.0);
        }
    }
}

fn swing() -> Vec<Sample> {
    StreamBuilder::new(125.0)
        .pull(0.0, 600.0, 150.0)
        .pause_ms(300.0)
        .push(0.0, 1200.0, 100.0)
        .build()
}

/// Poll until `want` matches an event or `secs` pass.
fn wait_for<I: thumbswing::Input>(
    client: &mut Client<I>,
    secs: f64,
    want: impl Fn(&Event) -> bool,
) -> Option<Event> {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    while Instant::now() < end {
        match client.poll().unwrap() {
            Some(e) if want(&e) => return Some(e),
            Some(_) => {}
            None => thread::sleep(Duration::from_millis(2)),
        }
    }
    None
}

fn open_when_ready() -> Device {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        match Device::open_by_name(NAME, true) {
            Ok(d) => return d,
            Err(e) if Instant::now() > end => panic!("could not open the virtual mouse: {e}"),
            Err(_) => thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[test]
#[ignore = "needs a writable /dev/uinput and access to the new event node"]
fn a_swing_through_the_kernel_then_unplug_and_replug() {
    let mouse = VirtualMouse::create();
    let dev = open_when_ready();
    assert!(thumbswing::Input::info(&dev).kernel_timestamps);
    assert!(thumbswing::Input::info(&dev).grabbed);
    let mut client = Client::new(dev, Config::default());
    assert_eq!(client.poll().unwrap(), Some(Event::Ready));

    let samples = swing();
    let writer = {
        let samples = samples.clone();
        thread::spawn(move || {
            mouse.play(&samples);
            mouse
        })
    };
    let shot = wait_for(&mut client, 5.0, |e| matches!(e, Event::Shot(_))).expect("a shot");
    let Event::Shot(shot) = shot else {
        unreachable!()
    };
    assert!(shot.swing.back_dir_deg.abs() < 1.5, "{:?}", shot.swing);
    assert!(shot.swing.fwd_dir_deg.abs() < 1.5);
    assert!(
        (shot.swing.fwd_peak_in_s / 30.0 - 1.0).abs() < 0.05,
        "{:?}",
        shot.swing
    );
    let mouse = writer.join().unwrap();

    // Unplug: the client reports it, then reopens the device by name.
    drop(mouse);
    assert!(wait_for(&mut client, 3.0, |e| *e == Event::Disconnected).is_some());
    let mouse = VirtualMouse::create();
    assert!(wait_for(&mut client, 10.0, |e| *e == Event::Ready).is_some());
    let writer = thread::spawn(move || {
        mouse.play(&samples);
        mouse
    });
    assert!(wait_for(&mut client, 5.0, |e| matches!(e, Event::Shot(_))).is_some());
    drop(writer.join().unwrap());
}
