//! `quillon audio analyze` (plan §12 v0.5): mix the document's audio, then
//! measure each scene segment — integrated LUFS, true peak (dBTP), and
//! silence ranges (research/M §1: ebur128 is the measurement half).

use quillon_audio::{analyze_segment, mix_document_decoding, silence_ranges, tick_to_sample};
use quillon_core::document::Document;
use quillon_core::time::Tick;
use serde_json::json;
use std::path::Path;

/// Analysis of one scene's slice of the mix.
pub struct SceneReport {
    pub id: String,
    pub from: Tick,
    pub to: Tick,
    pub lufs: Option<f64>,
    pub true_peak_dbtp: Option<f64>,
    /// Half-open [from, to) tick ranges below the silence threshold.
    pub silence: Vec<(Tick, Tick)>,
}

/// Analyze the document mix per scene. `silence_db` is the threshold in
/// dBFS; `min_silence_ms` the shortest run that counts as silence.
pub fn analyze(
    doc: &Document,
    base_dir: &Path,
    silence_db: f64,
    min_silence_ms: u64,
) -> Result<Vec<SceneReport>, String> {
    if doc.audio.is_empty() {
        return Err("the document has no audio tracks; there is nothing to analyze".into());
    }
    let mix = mix_document_decoding(doc, base_dir).map_err(|e| e.to_string())?;
    let threshold = 10f64.powf(silence_db / 20.0) as f32;
    let min_silence = min_silence_ms as i64 * doc.timebase / 1000;
    let frame = tick_to_sample(1, mix.rate, doc.timebase).max(1) as usize;
    let channels = usize::from(mix.channels);

    let mut reports = Vec::new();
    let mut start = 0i64;
    for scene in &doc.scenes {
        let end = start + scene.duration;
        let from_sample = (tick_to_sample(start, mix.rate, doc.timebase) as usize) * channels;
        let to_sample = ((tick_to_sample(end, mix.rate, doc.timebase) as usize) * channels)
            .min(mix.samples.len());
        // Keep whole frames even if the scene ends between samples.
        let to_sample =
            from_sample + ((to_sample - from_sample) / (channels * frame)) * channels * frame;
        let segment = &mix.samples
            [from_sample.min(mix.samples.len())..to_sample.max(from_sample.min(mix.samples.len()))];
        let measured =
            analyze_segment(segment, mix.rate, mix.channels).map_err(|e| e.to_string())?;
        let silence = silence_ranges(
            segment,
            mix.channels,
            mix.rate,
            doc.timebase,
            threshold,
            min_silence,
            start,
        );
        reports.push(SceneReport {
            id: scene.id.clone(),
            from: start,
            to: end,
            lufs: measured.lufs,
            true_peak_dbtp: measured.true_peak_dbtp,
            silence,
        });
        start = end;
    }
    Ok(reports)
}

/// The `--json` body for `audio analyze`.
pub fn json_body(
    doc: &Document,
    reports: &[SceneReport],
    silence_db: f64,
    min_silence_ms: u64,
) -> serde_json::Value {
    let scenes: Vec<_> = reports
        .iter()
        .map(|r| {
            json!({
                "id": r.id,
                "from": r.from,
                "to": r.to,
                "lufs": r.lufs,
                "true_peak_dbtp": r.true_peak_dbtp,
                "silence": r.silence.iter().map(|(f, t)| json!({ "from": f, "to": t })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "audio_rate": doc.audio_rate,
        "duration_ticks": doc.total_duration(),
        "silence_threshold_dbfs": silence_db,
        "min_silence_ms": min_silence_ms,
        "scenes": scenes,
    })
}

/// The human-readable report lines.
pub fn text_lines(reports: &[SceneReport], timebase: i64) -> Vec<String> {
    let mut lines = Vec::new();
    for r in reports {
        let lufs = r
            .lufs
            .map(|l| format!("{l:.1} LUFS"))
            .unwrap_or_else(|| "silent".into());
        let peak = r
            .true_peak_dbtp
            .map(|p| format!("{p:.1} dBTP"))
            .unwrap_or_else(|| "silent".into());
        let silence_total: i64 = r.silence.iter().map(|(f, t)| t - f).sum();
        let seconds = |ticks: i64| ticks as f64 / timebase as f64;
        lines.push(format!(
            "scene {} [{:.2}s..{:.2}s]: {lufs}, peak {peak}, silence {:.2}s in {} range(s)",
            r.id,
            seconds(r.from),
            seconds(r.to),
            seconds(silence_total),
            r.silence.len(),
        ));
    }
    lines
}
