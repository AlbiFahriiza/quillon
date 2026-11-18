//! Media probing through the system ffprobe binary (plan §12 v0.3: the
//! `probe` subcommand reports duration/resolution/fps/codec). Structured
//! JSON output only — stderr text is for the operator channel, never parsed
//! as data.

use serde::Deserialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("cannot run ffprobe — is it installed and on PATH? ({0})")]
    Spawn(String),
    #[error("probe failed: {0}")]
    Failed(String),
    #[error("the file is not a recognizable media container: {0}")]
    NotMedia(String),
}

/// What `quillon probe` reports about one media file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MediaInfo {
    /// Container duration in seconds, when the container states one.
    pub duration: Option<f64>,
    /// First video stream's pixel dimensions.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Frame rate as a rational (numerator, denominator) — `30000/1001`
    /// stays exact, no float rounding before tick math.
    pub fps: Option<(u32, u32)>,
    /// Codec names of the first video and audio streams.
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
}

impl MediaInfo {
    /// Duration converted to document ticks (rounded to the nearest tick).
    pub fn duration_ticks(&self, timebase: i64) -> Option<i64> {
        self.duration
            .map(|secs| (secs * timebase as f64).round() as i64)
    }

    /// Total frame count implied by duration and frame rate, when both are
    /// known.
    pub fn frame_count(&self) -> Option<u64> {
        let (num, den) = self.fps?;
        let duration = self.duration?;
        if num == 0 {
            return None;
        }
        Some((duration * f64::from(num) / f64::from(den)).round() as u64)
    }
}

#[derive(Debug, Deserialize)]
struct FfprobeReport {
    #[serde(default)]
    streams: Vec<FfprobeStream>,
    format: Option<FfprobeFormat>,
}

#[derive(Debug, Deserialize)]
struct FfprobeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FfprobeFormat {
    duration: Option<String>,
}

/// Probe one media file with ffprobe and map the JSON report.
pub fn probe(path: &Path) -> Result<MediaInfo, ProbeError> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .output()
        .map_err(|e| ProbeError::Spawn(e.to_string()))?;
    if !output.status.success() {
        return Err(ProbeError::Failed(format!(
            "exit status {} — the file may be missing or unreadable",
            output.status.code().unwrap_or(-1)
        )));
    }
    let report: FfprobeReport =
        serde_json::from_slice(&output.stdout).map_err(|e| ProbeError::NotMedia(e.to_string()))?;
    if report.streams.is_empty() {
        return Err(ProbeError::NotMedia("no media streams found".into()));
    }

    let video = report
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"));
    let audio = report
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"));

    Ok(MediaInfo {
        duration: report
            .format
            .as_ref()
            .and_then(|f| f.duration.as_deref())
            .and_then(|d| d.parse::<f64>().ok()),
        width: video.and_then(|s| s.width),
        height: video.and_then(|s| s.height),
        fps: video
            .and_then(|s| s.r_frame_rate.as_deref())
            .and_then(parse_rational),
        video_codec: video.and_then(|s| s.codec_name.clone()),
        audio_codec: audio.and_then(|s| s.codec_name.clone()),
    })
}

/// Parse ffprobe's `r_frame_rate` (`"30/1"`, `"30000/1001"`) exactly.
fn parse_rational(value: &str) -> Option<(u32, u32)> {
    let (num, den) = value.split_once('/')?;
    let (num, den): (u32, u32) = (num.parse().ok()?, den.parse().ok()?);
    if num == 0 || den == 0 {
        return None;
    }
    Some((num, den))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_rates_parse_exactly() {
        assert_eq!(parse_rational("30/1"), Some((30, 1)));
        assert_eq!(parse_rational("30000/1001"), Some((30000, 1001)));
        assert_eq!(parse_rational("0/0"), None);
        assert_eq!(parse_rational("garbage"), None);
    }

    #[test]
    fn duration_converts_to_ticks_without_drift() {
        let info = MediaInfo {
            duration: Some(3.0),
            width: Some(320),
            height: Some(240),
            fps: Some((30, 1)),
            video_codec: Some("h264".into()),
            audio_codec: Some("aac".into()),
        };
        assert_eq!(info.duration_ticks(705_600_000), Some(3 * 705_600_000));
        assert_eq!(info.frame_count(), Some(90));
    }
}
