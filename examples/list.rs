//! List trackballs and mice, or measure one.
//!
//! ```sh
//! cargo run --example list                                  # every /dev/input/event* node
//! cargo run --example list -- --measure /dev/input/by-id/usb-…-event-mouse [--seconds 20] [--no-grab] [--quiet]
//! ```
//!
//! `--measure` takes a `/dev/input` path, a `/dev/input/by-id` name or the
//! name the device reports, and prints every report (`Δt`, `dx`, `dy`) and
//! then a summary: the report rate, the largest `|dx|`/`|dy|` seen (spin the
//! ball hard to find the device's per-report limit) and the idle noise
//! (leave the ball alone for a while).

#[path = "common/devices.rs"]
mod devices;
#[path = "common/session.rs"]
mod session;
#[path = "common/term.rs"]
mod term;

use std::process::ExitCode;
use std::thread;
use std::time::{Duration, Instant};

use session::Args;
use thumbswing::{Input, device};

fn main() -> ExitCode {
    let args = match Args::parse(&["--measure", "--seconds"]) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("list: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(path) = args.value("--measure") {
        let seconds = args.number("--seconds").ok().flatten().unwrap_or(20.0);
        return measure(path, seconds, !args.flag("--no-grab"), args.flag("--quiet"));
    }
    list()
}

fn list() -> ExitCode {
    let devices = match device::list() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("list: {e}");
            return ExitCode::FAILURE;
        }
    };
    print!("{}", devices::table(&devices));
    let unreadable_pointers = devices
        .iter()
        .filter(|d| d.is_pointer() && !d.readable)
        .count();
    if devices.is_empty() {
        println!("no /dev/input/event* nodes");
    }
    if unreadable_pointers > 0 {
        println!(
            "\n{unreadable_pointers} pointer(s) cannot be read by this user. Either join the \
             `input` group:\n  sudo usermod -aG input $USER   # then log out and back in\n\
             or install a udev rule that grants the logged-in user access:\n  \
             echo 'KERNEL==\"event*\", SUBSYSTEM==\"input\", TAG+=\"uaccess\"' | \
             sudo tee /etc/udev/rules.d/70-thumbswing.rules\n  \
             sudo udevadm control --reload && sudo udevadm trigger"
        );
    }
    println!("\nMeasure one: cargo run --example list -- --measure <path from above>");
    ExitCode::SUCCESS
}

#[allow(
    clippy::cast_precision_loss,
    clippy::similar_names,
    clippy::too_many_lines
)]
fn measure(path: &str, seconds: f64, grab: bool, quiet: bool) -> ExitCode {
    term::install_signal_handlers();
    let mut dev = match device::open_setting(Some(path), grab) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("list: {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let info = dev.info().clone();
    println!(
        "{} ({:04x}:{:04x}) at {}{}, kernel timestamps: {}",
        info.name,
        info.vendor,
        info.product,
        info.path.display(),
        if info.grabbed { ", grabbed" } else { "" },
        if info.kernel_timestamps {
            "monotonic"
        } else {
            "no (stamped on read)"
        }
    );
    println!(
        "measuring for {seconds:.0} s: roll slowly, roll hard, spin it as hard as you can, then \
         leave it alone (Ctrl-C ends early)"
    );

    let end = Instant::now() + Duration::from_secs_f64(seconds);
    let mut last_t: Option<u64> = None;
    let mut dts: Vec<u64> = Vec::new();
    let (mut max_dx, mut max_dy) = (0i32, 0i32);
    let (mut sum_dx, mut sum_dy, mut path_counts) = (0i64, 0i64, 0f64);
    let mut idle_reports = 0usize;
    let mut reports = 0usize;
    let mut sizes: Vec<(i32, i32)> = Vec::new();
    while Instant::now() < end && !term::quit_requested() {
        match dev.try_recv() {
            Ok(Some(s)) => {
                reports += 1;
                let dt = last_t.map(|l| s.t_us.saturating_sub(l));
                last_t = Some(s.t_us);
                if let Some(dt) = dt.filter(|dt| *dt < 50_000) {
                    dts.push(dt);
                }
                max_dx = max_dx.max(s.dx.abs());
                max_dy = max_dy.max(s.dy.abs());
                sizes.push((s.dx.abs(), s.dy.abs()));
                sum_dx += i64::from(s.dx);
                sum_dy += i64::from(s.dy);
                path_counts += s.counts();
                if s.dx.abs() <= 1 && s.dy.abs() <= 1 && dt.is_none_or(|d| d > 50_000) {
                    idle_reports += 1;
                }
                if !quiet {
                    println!(
                        "{:>9} {:>4} {:>4}",
                        dt.map_or_else(
                            || "-".to_owned(),
                            |d| format!("{:.2}ms", d as f64 / 1000.0)
                        ),
                        s.dx,
                        s.dy
                    );
                }
            }
            Ok(None) => thread::sleep(Duration::from_millis(1)),
            Err(e) => {
                eprintln!("list: {e}");
                break;
            }
        }
    }
    drop(dev);

    let at_max = sizes
        .iter()
        .filter(|(x, y)| (*x == max_dx && max_dx > 0) || (*y == max_dy && max_dy > 0))
        .count();
    println!("\nsummary for {}", info.name);
    println!("  reports              {reports}");
    if dts.is_empty() {
        println!("  report rate          (not enough motion to measure)");
    } else {
        dts.sort_unstable();
        let median = dts[dts.len() / 2] as f64 / 1000.0;
        let min = dts[0] as f64 / 1000.0;
        println!(
            "  report interval      median {median:.2} ms, min {min:.2} ms → about {:.0} Hz",
            1000.0 / median
        );
    }
    println!(
        "  largest |dx| / |dy|  {max_dx} / {max_dy} ({at_max} reports at the largest value; \
         many at exactly 127, 2047 or 32767 means that is the per-report limit)"
    );
    println!("  idle-like reports    {idle_reports} isolated reports of at most 1 count");
    println!(
        "  travel               Σdx {sum_dx}, Σdy {sum_dy}, path {path_counts:.0} counts \
         (roll exactly one turn of the ball along Y and divide |Σdy| by π × the ball diameter \
         in inches for the CPI)"
    );
    ExitCode::SUCCESS
}
