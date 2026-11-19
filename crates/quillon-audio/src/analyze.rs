//! Loudness and silence analysis of mixes (plan §12 v0.5 `audio analyze`;
//! research/M §1: ebur128 is the measurement half — integrated LUFS and
//! true peak — while silence detection is our own sample scan).

use crate::{AudioError, tick_to_sample};

/// Loudness summary of one mix segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentLoudness {
    /// Integrated loudness in LUFS; `None` when the segment is silence
    /// (the meter reports -inf, which JSON cannot carry).
    pub lufs: Option<f64>,
    /// True peak in dBTP, the maximum over channels; `None` when the
    /// segment is digital silence.
    pub true_peak_dbtp: Option<f64>,
}

/// Measure integrated LUFS and true peak of an interleaved buffer
/// (EBU R128 through ebur128).
pub fn analyze_segment(
    samples: &[f32],
    rate: u32,
    channels: u16,
) -> Result<SegmentLoudness, AudioError> {
    let mode = ebur128::Mode::I | ebur128::Mode::TRUE_PEAK;
    let mut meter = ebur128::EbuR128::new(u32::from(channels), rate, mode)
        .map_err(|e| AudioError::Loudness(e.to_string()))?;
    meter
        .add_frames_f32(samples)
        .map_err(|e| AudioError::Loudness(e.to_string()))?;
    let lufs = meter
        .loudness_global()
        .map_err(|e| AudioError::Loudness(e.to_string()))?;
    let mut peak = 0.0f64;
    for ch in 0..u32::from(channels) {
        let p = meter
            .true_peak(ch)
            .map_err(|e| AudioError::Loudness(e.to_string()))?;
        peak = peak.max(p);
    }
    Ok(SegmentLoudness {
        lufs: lufs.is_finite().then_some(lufs),
        true_peak_dbtp: (peak > 0.0).then(|| 20.0 * peak.log10()),
    })
}

/// Tick ranges where the buffer stays below `threshold` (linear amplitude)
/// for at least `min_ticks`. `offset_ticks` is the buffer's start on the
/// document timeline; returned ranges are absolute, half-open [from, to).
pub fn silence_ranges(
    samples: &[f32],
    channels: u16,
    rate: u32,
    timebase: i64,
    threshold: f32,
    min_ticks: i64,
    offset_ticks: i64,
) -> Vec<(i64, i64)> {
    let ch = channels.max(1) as usize;
    let frames = samples.len() / ch;
    let min_frames = (tick_to_sample(min_ticks, rate, timebase).max(0) as usize).max(1);
    let frame_tick =
        |frame: usize| -> i64 { offset_ticks + frame as i64 * timebase / i64::from(rate) };

    let mut ranges = Vec::new();
    let mut run_start: Option<usize> = None;
    // The `frame == frames` iteration closes a run that reaches the end.
    for frame in 0..=frames {
        let quiet = frame < frames && (0..ch).all(|c| samples[frame * ch + c].abs() <= threshold);
        match (run_start, quiet) {
            (None, true) => run_start = Some(frame),
            (Some(start), false) => {
                if frame - start >= min_frames {
                    ranges.push((frame_tick(start), frame_tick(frame)));
                }
                run_start = None;
            }
            _ => {}
        }
    }
    ranges
}

/// Percussive onsets as (from, to) tick pairs: a cue starts at each
/// quiet-to-loud transition above `threshold` and runs until the next
/// onset or `max_duration`, whichever comes first. A burst's own
/// oscillation crosses the threshold many times per cycle, so a new onset
/// requires at least 30 ms of quiet since the last loud frame. This is
/// the deterministic, dependency-free transcript source the acceptance
/// test for timing regularization uses (plan §12 v0.5 #2, synthetic
/// click-track), and a cheap speech-activity marker in its own right.
pub fn onset_cues(
    samples: &[f32],
    channels: u16,
    rate: u32,
    timebase: i64,
    threshold: f32,
    max_duration: i64,
) -> Vec<(i64, i64)> {
    let ch = channels.max(1) as usize;
    let frames = samples.len() / ch;
    let frame_tick = |frame: usize| -> i64 { frame as i64 * timebase / i64::from(rate) };
    let loud =
        |frame: usize| -> bool { (0..ch).any(|c| samples[frame * ch + c].abs() > threshold) };
    // The refractory window in frames: 30 ms of quiet between onsets.
    let quiet_frames = (i64::from(rate) * 30 / 1000).max(1) as usize;

    let mut onsets: Vec<i64> = Vec::new();
    let mut last_loud: Option<usize> = None;
    for frame in 0..frames {
        if !loud(frame) {
            continue;
        }
        let is_new_onset = last_loud.is_none_or(|last| frame - last > quiet_frames);
        if is_new_onset {
            onsets.push(frame_tick(frame));
        }
        last_loud = Some(frame);
    }
    let mut cues = Vec::with_capacity(onsets.len());
    for (i, &from) in onsets.iter().enumerate() {
        let end = onsets.get(i + 1).copied().unwrap_or(from + max_duration);
        cues.push((from, end.min(from + max_duration)));
    }
    cues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MIX_CHANNELS;

    const RATE: u32 = 48_000;
    const TB: i64 = 705_600_000;

    /// An interleaved stereo tone of `seconds` at `amplitude`. Cosine, not
    /// sine: a cosine starts and ends at its peak, so a tone block's first
    /// and last frames are never below the silence threshold (a sine's
    /// zero-crossings would blur range edges by one frame).
    fn tone(seconds: f64, amplitude: f32) -> Vec<f32> {
        let frames = (seconds * RATE as f64) as usize;
        (0..frames)
            .flat_map(|f| {
                let t = f as f64 / RATE as f64;
                let v = (2.0 * std::f64::consts::PI * 440.0 * t).cos() as f32 * amplitude;
                [v, v]
            })
            .collect()
    }

    #[test]
    fn sine_segment_measures_finite_lufs_and_peak() {
        let samples = tone(1.0, 0.5);
        let seg = analyze_segment(&samples, RATE, MIX_CHANNELS).unwrap();
        let lufs = seg.lufs.expect("a tone has finite loudness");
        let peak = seg.true_peak_dbtp.expect("a tone has a peak");
        assert!((-30.0..0.0).contains(&lufs), "LUFS {lufs}");
        assert!((-12.0..0.0).contains(&peak), "peak {peak} dBTP");
        assert!(peak > lufs, "true peak sits above integrated loudness");
    }

    #[test]
    fn digital_silence_reports_none() {
        let samples = vec![0.0f32; RATE as usize * 2];
        let seg = analyze_segment(&samples, RATE, MIX_CHANNELS).unwrap();
        assert_eq!(seg.lufs, None);
        assert_eq!(seg.true_peak_dbtp, None);
    }

    #[test]
    fn silence_ranges_find_both_ends_of_a_tone() {
        // 0.5s silence, 1.0s tone, 0.5s silence.
        let mut samples = vec![0.0f32; RATE as usize / 2 * 2];
        samples.extend(tone(1.0, 0.5));
        samples.extend(vec![0.0f32; RATE as usize / 2 * 2]);

        let half_second = TB / 2;
        let ranges = silence_ranges(&samples, MIX_CHANNELS, RATE, TB, 0.01, half_second, 0);
        assert_eq!(
            ranges,
            vec![(0, half_second), (3 * half_second, 2 * TB)],
            "leading and trailing silence"
        );

        // A 600ms minimum drops both 500ms ranges.
        let ranges = silence_ranges(&samples, MIX_CHANNELS, RATE, TB, 0.01, 600 * TB / 1000, 0);
        assert!(ranges.is_empty(), "{ranges:?}");
    }

    #[test]
    fn silence_ranges_are_offset_onto_the_document_timeline() {
        let offset = 4 * TB;
        let quiet = vec![0.0f32; RATE as usize / 4 * 2]; // 250ms
        let mut both = quiet.clone();
        both.extend(tone(0.5, 0.5));
        both.extend(&quiet);
        let ranges = silence_ranges(&both, MIX_CHANNELS, RATE, TB, 0.01, 200 * TB / 1000, offset);
        assert_eq!(
            ranges,
            vec![
                (offset, offset + TB / 4),
                (offset + 3 * TB / 4, offset + TB),
            ]
        );
    }

    /// Three 50ms bursts at 0.0s, 1.5s and 3.0s — a click-track.
    fn click_track() -> Vec<f32> {
        let mut samples = vec![0.0f32; RATE as usize * 4 * 2];
        for start in [0usize, 72000, 144000] {
            for f in 0..2400 {
                let t = f as f64 / RATE as f64;
                let v = (2.0 * std::f64::consts::PI * 880.0 * t).cos() as f32 * 0.6;
                samples[(start + f) * 2] = v;
                samples[(start + f) * 2 + 1] = v;
            }
        }
        samples
    }

    #[test]
    fn onset_cues_mark_each_burst() {
        let samples = click_track();
        // Three bursts at 0.0s, 1.5s and 3.0s; each cue runs to the next
        // onset (1.5s apart) or max_duration (2s), whichever comes first —
        // the last burst has no successor, so it runs the full 2s.
        let cues = onset_cues(&samples, MIX_CHANNELS, RATE, TB, 0.3, 2 * TB);
        assert_eq!(
            cues,
            vec![(0, 3 * TB / 2), (3 * TB / 2, 3 * TB), (3 * TB, 5 * TB)]
        );
    }

    #[test]
    fn onset_cues_cap_the_last_cue_at_max_duration() {
        let samples = click_track();
        let cues = onset_cues(&samples, MIX_CHANNELS, RATE, TB, 0.3, TB);
        let last = cues.last().copied().unwrap();
        assert_eq!(last.1 - last.0, TB, "capped at max_duration: {cues:?}");
    }
}
