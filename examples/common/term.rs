//! Terminal helpers shared by the examples: a redrawn
//! status line, and a quit flag set by SIGINT/SIGTERM (Ctrl-C).

#![allow(dead_code)]

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};

static QUIT: AtomicBool = AtomicBool::new(false);

/// Whether SIGINT or SIGTERM has arrived (or [`request_quit`] was called).
pub fn quit_requested() -> bool {
    QUIT.load(Ordering::Relaxed)
}

/// Ask the main loop to stop.
pub fn request_quit() {
    QUIT.store(true, Ordering::Relaxed);
}

#[cfg(target_os = "linux")]
extern "C" fn on_signal(_: libc::c_int) {
    QUIT.store(true, Ordering::Relaxed);
}

/// Make SIGINT and SIGTERM set the quit flag instead of killing the
/// process, so the main loop can release the device and restore the terminal.
pub fn install_signal_handlers() {
    #[cfg(target_os = "linux")]
    // SAFETY: the handler only stores to an atomic, which is async-signal-safe.
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

/// Prints log lines above a status line that is redrawn in place (only when
/// stderr is a terminal; otherwise log lines are printed plainly).
pub struct Screen {
    tty: bool,
    status: String,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            tty: std::io::stderr().is_terminal(),
            status: String::new(),
        }
    }

    /// Print a log line.
    pub fn log(&mut self, line: &str) {
        let mut err = std::io::stderr().lock();
        if self.tty {
            let _ = write!(err, "\r\x1b[2K{line}\r\n{}", self.status);
        } else {
            let _ = writeln!(err, "{line}");
        }
        let _ = err.flush();
    }

    /// Replace the status line.
    pub fn status(&mut self, status: String) {
        if self.tty {
            let mut err = std::io::stderr().lock();
            let _ = write!(err, "\r\x1b[2K{status}");
            let _ = err.flush();
        }
        self.status = status;
    }

    /// Leave the status line on screen and move below it.
    pub fn finish(&mut self) {
        if self.tty {
            let _ = writeln!(std::io::stderr(), "\r");
        }
    }
}
