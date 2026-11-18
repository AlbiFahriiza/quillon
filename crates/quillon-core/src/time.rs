//! Integer time base (plan §3 P3): ticks are "flicks", 705,600,000 per
//! second, so 24/25/30/50/60 fps all divide the timebase exactly and no float
//! drift can enter timing.

/// Ticks per second; the only value document format v1 accepts.
pub const FLICKS_PER_SECOND: i64 = 705_600_000;

/// A point in time, or a duration, in ticks.
pub type Tick = i64;

/// Duration of one frame at `fps`, in ticks. The caller guarantees
/// `FLICKS_PER_SECOND % fps == 0` (validated with the document).
pub fn frame_tick(fps: u32) -> Tick {
    FLICKS_PER_SECOND / i64::from(fps)
}

/// Convert seconds (exact integer division of the timebase) to ticks.
pub fn seconds(sec: i64) -> Tick {
    sec * FLICKS_PER_SECOND
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_framerates_divide_exactly() {
        for fps in [24, 25, 30, 50, 60] {
            assert_eq!(FLICKS_PER_SECOND % i64::from(fps), 0, "fps {fps} not exact");
        }
    }

    #[test]
    fn quarter_second_is_exact() {
        // 176,400,000 ticks = 0.25 s, the value used in the plan's example.
        assert_eq!(seconds(1) / 4, 176_400_000);
    }
}
