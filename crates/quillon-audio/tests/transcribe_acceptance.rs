//! Acceptance v0.5 #2 (plan §12): a transcript-shaped cue list from a
//! synthetic click-track passes through timing regularization and every
//! cue comes out inside the min/max duration bounds while still covering
//! the click that produced it. Deterministic: no model, no network.

use quillon_audio::onset_cues;
use quillon_core::caption::CaptionCue;
use quillon_core::{RegularizeConfig, regularize_cues};

const RATE: u32 = 48_000;
const TB: i64 = 705_600_000;

/// A click-track: `count` 60ms bursts, `spacing` seconds apart, stereo.
fn click_track(count: usize, spacing: f64) -> Vec<f32> {
    let total = (count as f64 * spacing + 1.0) * RATE as f64;
    let mut samples = vec![0.0f32; total as usize * 2];
    let burst = (0.060 * RATE as f64) as usize;
    for k in 0..count {
        let start = (k as f64 * spacing * RATE as f64) as usize;
        for f in 0..burst {
            let t = f as f64 / RATE as f64;
            let v = (2.0 * std::f64::consts::PI * 880.0 * t).cos() as f32 * 0.6;
            samples[(start + f) * 2] = v;
            samples[(start + f) * 2 + 1] = v;
        }
    }
    samples
}

#[test]
fn click_track_transcript_regularizes_into_bounds() {
    // Eight clicks 2.0s apart: cue spans after regularization are 1s
    // (the minimum), leaving 1s gaps — above the 0.5s merge threshold,
    // so each click keeps its own cue.
    let samples = click_track(8, 2.0);
    let onsets = onset_cues(&samples, 2, RATE, TB, 0.3, 2 * TB);
    assert_eq!(onsets.len(), 8, "one onset per click: {onsets:?}");

    // Transcript-shaped cues, the way a transcriber emits them: each cue
    // starts at an onset but is far too short (0.2s).
    let cues: Vec<CaptionCue> = onsets
        .iter()
        .enumerate()
        .map(|(i, (from, _))| CaptionCue {
            from: *from,
            to: from + TB / 5,
            text: format!("click {i}"),
        })
        .collect();

    let cfg = RegularizeConfig::default();
    let out = regularize_cues(&cues, &cfg, TB);
    assert_eq!(out.len(), 8, "gaps of 1s do not merge: {out:?}");

    for (i, cue) in out.iter().enumerate() {
        let duration = cue.to - cue.from;
        assert!(
            duration >= cfg.min_duration,
            "cue {i} is {duration} ticks, below the minimum"
        );
        assert!(
            duration <= cfg.max_duration,
            "cue {i} is {duration} ticks, above the maximum"
        );
        // The click that produced this cue is still covered by it.
        let onset = onsets[i].0;
        assert!(
            cue.from <= onset && onset < cue.to,
            "cue {i} no longer covers its click at {onset}"
        );
    }
    for pair in out.windows(2) {
        assert!(pair[0].to <= pair[1].from, "cues must not overlap");
    }
}

#[test]
fn dense_clicks_merge_into_fewer_cues() {
    // Clicks every 0.4s force the minimum-duration cues to chain: the
    // output stays sorted and non-overlapping with in-bounds durations,
    // even though the raw transcript overlaps itself.
    let samples = click_track(6, 0.4);
    let onsets = onset_cues(&samples, 2, RATE, TB, 0.3, 2 * TB);
    assert_eq!(onsets.len(), 6);
    let cues: Vec<CaptionCue> = onsets
        .iter()
        .enumerate()
        .map(|(i, (from, _))| CaptionCue {
            from: *from,
            to: from + TB / 10,
            text: format!("c{i}"),
        })
        .collect();
    let cfg = RegularizeConfig::default();
    let out = regularize_cues(&cues, &cfg, TB);
    for pair in out.windows(2) {
        assert!(pair[0].to <= pair[1].from, "{out:?}");
    }
    for cue in &out {
        let duration = cue.to - cue.from;
        assert!(duration >= cfg.min_duration && duration <= cfg.max_duration);
    }
}
