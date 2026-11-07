//! The platform-independent input unit.

/// One report from a relative pointing device: the motion it accumulated since
/// the previous report.
///
/// On Linux a sample is one evdev report (`REL_X`/`REL_Y` events up to
/// `SYN_REPORT`), stamped with the kernel's `CLOCK_MONOTONIC` timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sample {
    /// Report time in microseconds, on the same clock as
    /// [`Input::now_us`](crate::Input::now_us).
    pub t_us: u64,
    /// Horizontal counts, positive = right.
    pub dx: i32,
    /// Vertical counts, positive = toward the player (the evdev mouse
    /// convention: positive = down).
    pub dy: i32,
}

impl Sample {
    /// A sample at `t_us` with motion `(dx, dy)`.
    #[must_use]
    pub const fn new(t_us: u64, dx: i32, dy: i32) -> Self {
        Self { t_us, dx, dy }
    }

    /// Length of the motion in counts.
    #[must_use]
    pub fn counts(&self) -> f64 {
        f64::from(self.dx).hypot(f64::from(self.dy))
    }
}
