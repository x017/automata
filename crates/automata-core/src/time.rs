//! Frame timekeeping.
//!
//! [`Time`] is advanced by the main loop once per frame, before game logic
//! runs, so every observer within a frame agrees on the timing values.

use std::time::Duration;

/// Tracks per-frame and cumulative time for the main loop.
///
/// The engine updates this once per frame before invoking the update
/// callback. The first frame reports a near-zero delta, since no previous
/// frame exists to measure against.
#[derive(Debug, Clone, Default)]
pub struct Time {
    delta: Duration,
    elapsed: Duration,
    frame_count: u64,
}

impl Time {
    /// Duration of the most recent frame.
    #[must_use]
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// Duration of the most recent frame, in seconds.
    #[must_use]
    pub fn delta_seconds(&self) -> f32 {
        self.delta.as_secs_f32()
    }

    /// Total time elapsed since the main loop started.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Total time elapsed since the main loop started, in seconds.
    #[must_use]
    pub fn elapsed_seconds(&self) -> f64 {
        self.elapsed.as_secs_f64()
    }

    /// Number of frames processed so far, including the current one.
    #[must_use]
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Advances the clock by one frame.
    pub(crate) fn advance(&mut self, delta: Duration) {
        self.delta = delta;
        self.elapsed += delta;
        self.frame_count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance_accumulates_state() {
        let mut time = Time::default();
        time.advance(Duration::from_millis(16));
        time.advance(Duration::from_millis(16));

        assert_eq!(time.frame_count(), 2);
        assert_eq!(time.delta(), Duration::from_millis(16));
        assert_eq!(time.elapsed(), Duration::from_millis(32));
    }

    #[test]
    fn seconds_conversions_match_durations() {
        let mut time = Time::default();
        time.advance(Duration::from_secs(1));

        assert!((time.delta_seconds() - 1.0).abs() < f32::EPSILON);
        assert!((time.elapsed_seconds() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn new_time_starts_at_zero() {
        let time = Time::default();
        assert_eq!(time.delta(), Duration::ZERO);
        assert_eq!(time.elapsed(), Duration::ZERO);
        assert_eq!(time.frame_count(), 0);
    }
}
