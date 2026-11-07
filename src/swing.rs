//! Swing geometry: what the hands did.

/// One completed swing, measured from the raw strokes.
///
/// Directions are degrees from straight forward (away from the player),
/// positive = right.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Swing {
    /// The backswing line continued forward through the ball: a pull toward
    /// the player's left gives a positive (right) value.
    pub back_dir_deg: f64,
    /// Backswing length, inches, capped at the full-cock length.
    pub back_len_in: f64,
    /// Backswing length as a fraction of the full-cock length, 0..=1.
    pub back_fraction: f64,
    /// Direction of the forward stroke (the push).
    pub fwd_dir_deg: f64,
    /// Peak windowed speed of the forward stroke, inches per second.
    pub fwd_peak_in_s: f64,
    /// Forward travel over the stroke, inches.
    pub fwd_len_in: f64,
    /// Number of forward-stroke reports at the device's per-report limit.
    pub saturated_reports: u32,
    /// Time from the start of the backswing to the end of the forward
    /// stroke, ms.
    pub duration_ms: f64,
    /// Time the ball sat still at the top, ms (0 for one smooth motion).
    pub pause_ms: f64,
}

impl Swing {
    /// Whether the forward stroke hit the per-report limit (a run of two or
    /// more saturated reports). Its direction then comes from the reports
    /// before saturation and its peak speed is a lower bound.
    #[must_use]
    pub fn saturated(&self) -> bool {
        self.saturated_reports >= 2
    }
}
