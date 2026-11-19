//! Transcription through whisper-rs (plan §4 F5). Only compiled with the
//! optional `transcribe` feature (plan §15 #7: transcription is never a
//! mandatory dependency — default builds pull no C toolchain). Model
//! weights are downloaded on first use and pinned by sha256; raw whisper
//! output always passes through core's timing regularization before it can
//! become a caption track (research/H §1).

use crate::{AudioError, DecodedAudio, decode, resample};
use quillon_core::caption::CaptionCue;
use quillon_core::ms_to_ticks;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// The default model: whisper.cpp's tiny.en (English), from the official
/// ggml repository. ~75 MB, downloaded once into the user's cache.
pub const DEFAULT_MODEL_URL: &str =
    "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin";

/// sha256 of the model this build pins, verified from the official
/// repository over HTTPS. A mismatched file is refused, not used.
pub const DEFAULT_MODEL_SHA256: &str =
    "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f";

/// Whisper wants mono 16 kHz f32; convert any decoded audio to exactly
/// that (average-downmix multi-channel sources).
pub fn to_mono_16k(audio: &DecodedAudio) -> Result<DecodedAudio, AudioError> {
    let resampled = resample(audio, 16_000)?;
    let ch = resampled.channels.max(1) as usize;
    let mono: Vec<f32> = resampled
        .samples
        .chunks(ch)
        .map(|frame| frame.iter().sum::<f32>() / ch as f32)
        .collect();
    Ok(DecodedAudio {
        samples: mono,
        channels: 1,
        rate: 16_000,
    })
}

/// Decode a file straight to whisper's expected format.
pub fn decode_mono_16k(path: &Path) -> Result<DecodedAudio, AudioError> {
    to_mono_16k(&decode(path)?)
}

/// Where the default model is cached (no new dependency for this):
/// `$XDG_CACHE_HOME`/`~/.cache` on Unix, `%LOCALAPPDATA%` on Windows.
pub fn default_model_path() -> Option<PathBuf> {
    #[cfg(unix)]
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")));
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    base.map(|b| b.join("quillon").join("whisper").join("ggml-tiny.en.bin"))
}

fn file_sha256(path: &Path) -> Result<String, AudioError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| AudioError::Io(format!("open {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)
            .map_err(|e| AudioError::Io(format!("read {}: {e}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    Ok(out)
}

/// Ensure `path` holds the pinned default model, downloading it first when
/// missing or mismatched. The download lands under a `.part` name and is
/// verified before it replaces the target.
pub fn ensure_model(path: &Path) -> Result<(), AudioError> {
    if path.exists() {
        let sum = file_sha256(path)?;
        if sum == DEFAULT_MODEL_SHA256 {
            return Ok(());
        }
        return Err(AudioError::Transcribe(format!(
            "model file at {} does not match the pinned sha256; delete it and retry",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .ok_or_else(|| AudioError::Io("model path has no directory".into()))?;
    std::fs::create_dir_all(parent)
        .map_err(|e| AudioError::Io(format!("create {}: {e}", parent.display())))?;
    let tmp = path.with_extension("part");
    let response = ureq::get(DEFAULT_MODEL_URL)
        .call()
        .map_err(|e| AudioError::Transcribe(format!("cannot download the whisper model: {e}")))?;
    let mut file = std::fs::File::create(&tmp)
        .map_err(|e| AudioError::Io(format!("create {}: {e}", tmp.display())))?;
    let mut reader = response.into_body().into_reader();
    std::io::copy(&mut reader, &mut file)
        .map_err(|e| AudioError::Transcribe(format!("download interrupted: {e}")))?;
    drop(file);
    let sum = file_sha256(&tmp)?;
    if sum != DEFAULT_MODEL_SHA256 {
        let _ = std::fs::remove_file(&tmp);
        return Err(AudioError::Transcribe(
            "the downloaded model does not match the pinned sha256; refusing to use it".into(),
        ));
    }
    std::fs::rename(&tmp, path).map_err(|e| AudioError::Io(format!("install the model: {e}")))?;
    Ok(())
}

/// Transcribe mono 16 kHz samples into cues on the document timeline.
/// Segments are half-open [from, to) tick ranges; empty texts are dropped
/// at the source so regularization sees only real cues.
pub fn transcribe(
    model: &Path,
    samples: &[f32],
    timebase: i64,
    threads: u32,
) -> Result<Vec<CaptionCue>, AudioError> {
    let ctx = whisper_rs::WhisperContext::new_with_params(
        model,
        whisper_rs::WhisperContextParameters::new(),
    )
    .map_err(|e| AudioError::Transcribe(format!("cannot load the whisper model: {e}")))?;
    let mut state = ctx
        .create_state()
        .map_err(|e| AudioError::Transcribe(format!("cannot init the transcriber: {e}")))?;
    let mut params =
        whisper_rs::FullParams::new(whisper_rs::SamplingStrategy::Greedy { best_of: 1 });
    params.set_n_threads(threads as std::ffi::c_int);
    params.set_language(Some("en"));
    state
        .full(params, samples)
        .map_err(|e| AudioError::Transcribe(format!("whisper failed: {e}")))?;
    let count = state.full_n_segments();
    let mut cues = Vec::new();
    for i in 0..count {
        let Some(segment) = state.get_segment(i) else {
            continue;
        };
        let text = segment
            .to_str_lossy()
            .map_err(|e| AudioError::Transcribe(format!("segment text: {e}")))?
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let (from_ms, to_ms) = (segment.start_timestamp(), segment.end_timestamp());
        if to_ms <= from_ms {
            continue;
        }
        cues.push(CaptionCue {
            from: ms_to_ticks(from_ms, timebase),
            to: ms_to_ticks(to_ms, timebase),
            text,
        });
    }
    Ok(cues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_16k_downmixes_and_resamples() {
        // Stereo 48k: 1s, left = +0.5 constant, right = -0.5.
        let audio = DecodedAudio {
            samples: vec![0.5f32, -0.5; 48_000],
            channels: 2,
            rate: 48_000,
        };
        let mono = to_mono_16k(&audio).unwrap();
        assert_eq!(mono.rate, 16_000);
        assert_eq!(mono.channels, 1);
        assert_eq!(mono.samples.len(), 16_000);
        // The downmix of +0.5 and -0.5 is silence.
        assert!(mono.samples.iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn default_model_path_points_into_the_cache() {
        let path = default_model_path().expect("a cache dir is derivable");
        assert!(path.ends_with("ggml-tiny.en.bin"), "{}", path.display());
    }
}
