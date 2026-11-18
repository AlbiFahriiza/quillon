//! Audio pipeline (plan §4 F4, §12 v0.3): decode (symphonia) → resample to
//! the document rate (rubato) → mix in the tick domain (gain, placement,
//! source windows, linear fades — research/M §1) → loudness (ebur128, EBU
//! R128). All mixing math is integer-exact at the tick↔sample boundary
//! because the document validator guarantees the timebase divides the audio
//! rate exactly.

use quillon_core::document::{AudioTrack, Document};
use std::collections::BTreeMap;
use std::path::Path;

/// The fixed channel layout of the mix: stereo. Mono sources are copied to
/// both channels; wider sources contribute their first two channels.
pub const MIX_CHANNELS: u16 = 2;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("cannot decode {path}: {reason}")]
    Decode { path: String, reason: String },
    #[error("cannot resample {from} Hz to {to} Hz: {reason}")]
    Resample { from: u32, to: u32, reason: String },
    #[error("loudness measurement failed: {0}")]
    Loudness(String),
    #[error("audio i/o failed: {0}")]
    Io(String),
}

/// Decoded audio: interleaved f32 samples in [-1, 1].
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub channels: u16,
    pub rate: u32,
}

impl DecodedAudio {
    fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    /// Sample `frame` of channel `ch`.
    fn sample(&self, frame: usize, ch: usize) -> f32 {
        let channels = self.channels.max(1) as usize;
        if ch >= channels {
            return 0.0;
        }
        self.samples
            .get(frame * channels + ch)
            .copied()
            .unwrap_or(0.0)
    }
}

/// Decode an audio file with symphonia (WAV/MP3/FLAC/OGG per the feature
/// set; plan F4).
pub fn decode(path: &Path) -> Result<DecodedAudio, AudioError> {
    use symphonia::core::codecs::CodecParameters;
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).map_err(|e| AudioError::Decode {
        path: path.display().to_string(),
        reason: e.to_string(),
    })?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let hint = Hint::new();
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Decode {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    // The first audio track.
    let track = format
        .tracks()
        .iter()
        .find(|t| matches!(t.codec_params, Some(CodecParameters::Audio(_))))
        .ok_or_else(|| AudioError::Decode {
            path: path.display().to_string(),
            reason: "no audio track".into(),
        })?
        .clone();
    let track_id = track.id;
    let Some(CodecParameters::Audio(params)) = track.codec_params else {
        unreachable!("matched above");
    };

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| AudioError::Decode {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;

    let mut samples: Vec<f32> = Vec::new();
    let mut channels = params
        .channels
        .map(|c| c.count())
        .unwrap_or(MIX_CHANNELS as usize) as u16;
    let mut rate = params.sample_rate.unwrap_or(48_000);

    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break, // clean end of stream
            Err(e) => {
                return Err(AudioError::Decode {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                });
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = decoder.decode(&packet).map_err(|e| AudioError::Decode {
            path: path.display().to_string(),
            reason: e.to_string(),
        })?;
        let spec = decoded.spec();
        rate = spec.rate();
        channels = spec.channels().count() as u16;
        // copy_to_vec_interleaved RESIZES the destination to this packet's
        // sample count (it overwrites), so copy into a per-packet chunk and
        // append.
        let mut chunk: Vec<f32> = Vec::new();
        decoded.copy_to_vec_interleaved(&mut chunk);
        debug_assert_eq!(chunk.len(), decoded.frames() * channels as usize);
        samples.extend_from_slice(&chunk);
    }

    Ok(DecodedAudio {
        samples,
        channels,
        rate,
    })
}

/// Resample to `to_rate` with rubato's polynomial resampler (cubic). A
/// same-rate input is returned unchanged.
pub fn resample(input: &DecodedAudio, to_rate: u32) -> Result<DecodedAudio, AudioError> {
    if input.rate == to_rate || input.samples.is_empty() {
        return Ok(input.clone());
    }
    use rubato::audioadapter_buffers::direct::SequentialSliceOfVecs;
    use rubato::{Async, FixedAsync, PolynomialDegree, Resampler};

    let channels = input.channels.max(1) as usize;
    let frames = input.frames();

    // De-interleave into planar buffers (the adapter's view is planar).
    let mut planes: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); channels];
    for chunk in input.samples.chunks(channels) {
        for (plane, &value) in planes.iter_mut().zip(chunk) {
            plane.push(value);
        }
    }

    let mut resampler = Async::<f32>::new_poly(
        f64::from(to_rate) / f64::from(input.rate),
        1.0,
        PolynomialDegree::Cubic,
        1024,
        channels,
        FixedAsync::Input,
    )
    .map_err(|e| AudioError::Resample {
        from: input.rate,
        to: to_rate,
        reason: e.to_string(),
    })?;

    let out_capacity = resampler.process_all_needed_output_len(frames);
    let view = SequentialSliceOfVecs::new(&planes, channels, frames).map_err(|e| {
        AudioError::Resample {
            from: input.rate,
            to: to_rate,
            reason: e.to_string(),
        }
    })?;
    let mut out_planes: Vec<Vec<f32>> = vec![vec![0.0; out_capacity]; channels];
    let produced = {
        let mut out_view = SequentialSliceOfVecs::new_mut(&mut out_planes, channels, out_capacity)
            .map_err(|e| AudioError::Resample {
                from: input.rate,
                to: to_rate,
                reason: e.to_string(),
            })?;
        let (_consumed, produced) = resampler
            .process_all_into_buffer(&view, &mut out_view, frames, None)
            .map_err(|e| AudioError::Resample {
                from: input.rate,
                to: to_rate,
                reason: e.to_string(),
            })?;
        produced
    };

    // Interleave back: one sample per column, channels in parallel.
    let mut samples = Vec::with_capacity(produced * channels);
    let mut columns: Vec<std::slice::Iter<'_, f32>> =
        out_planes.iter().map(|p| p[..produced].iter()).collect();
    for _ in 0..produced {
        for column in &mut columns {
            samples.push(*column.next().expect("column has `produced` frames"));
        }
    }
    Ok(DecodedAudio {
        samples,
        channels: input.channels,
        rate: to_rate,
    })
}

/// Tick → sample index at the document rate (exact: the validator only
/// accepts rates that divide the timebase).
pub fn tick_to_sample(tick: i64, rate: u32, timebase: i64) -> i64 {
    tick * i64::from(rate) / timebase
}

/// Mix every document audio track into one stereo buffer at the document's
/// rate. `sources` must already be resampled to `doc.audio_rate` (see
/// [`mix_document_decoding`], which does decode+resample for you).
pub fn mix_document(
    doc: &Document,
    sources: &BTreeMap<String, DecodedAudio>,
) -> Result<DecodedAudio, AudioError> {
    let rate = doc.audio_rate;
    let total_samples = tick_to_sample(doc.total_duration(), rate, doc.timebase).max(0) as usize
        * MIX_CHANNELS as usize;
    let mut mix = vec![0.0f32; total_samples];

    for track in &doc.audio {
        let source = sources
            .get(&track.asset)
            .ok_or_else(|| AudioError::Decode {
                path: track.asset.clone(),
                reason: "track asset was not decoded".into(),
            })?;
        mix_track(&mut mix, track, source, rate, doc.timebase);
    }

    // Saturate the summed float mix to [-1, 1] once, at the end.
    for s in mix.iter_mut() {
        *s = s.clamp(-1.0, 1.0);
    }
    Ok(DecodedAudio {
        samples: mix,
        channels: MIX_CHANNELS,
        rate,
    })
}

/// Decode + resample every audio asset a document references, then mix.
pub fn mix_document_decoding(doc: &Document, base_dir: &Path) -> Result<DecodedAudio, AudioError> {
    let mut sources = BTreeMap::new();
    for track in &doc.audio {
        if sources.contains_key(&track.asset) {
            continue;
        }
        let src = doc
            .assets
            .get(&track.asset)
            .map(|a| a.src())
            .ok_or_else(|| AudioError::Decode {
                path: track.asset.clone(),
                reason: "unknown asset".into(),
            })?;
        let decoded = decode(&base_dir.join(src))?;
        let decoded = resample(&decoded, doc.audio_rate)?;
        sources.insert(track.asset.clone(), decoded);
    }
    mix_document(doc, &sources)
}

/// Add one track into the mix buffer (tick domain, plan §5.1).
fn mix_track(mix: &mut [f32], track: &AudioTrack, source: &DecodedAudio, rate: u32, timebase: i64) {
    let mix_ch = MIX_CHANNELS as usize;
    let src_ch = source.channels.max(1) as usize;
    let start = tick_to_sample(track.start_at, rate, timebase).max(0) as usize;
    let from_sample = track
        .source_range
        .map(|r| tick_to_sample(r.from, rate, timebase).max(0) as usize)
        .unwrap_or(0);
    let to_sample = track
        .source_range
        .map(|r| tick_to_sample(r.to, rate, timebase).max(0) as usize)
        .unwrap_or(usize::MAX);

    let mut frame = from_sample;
    while frame < to_sample && frame < source.frames() {
        let mix_frame = start + (frame - from_sample);
        let out_base = mix_frame * mix_ch;
        if out_base + mix_ch > mix.len() {
            break; // past the document end
        }
        let envelope = fade_envelope(track, frame - from_sample, rate, timebase);
        for ch in 0..mix_ch {
            // Mono upmix / first-two-channel downmix (documented MIX_CHANNELS).
            let value = source.sample(frame, ch.min(src_ch - 1));
            mix[out_base + ch] += value * track.gain * envelope;
        }
        frame += 1;
    }
}

/// Product of all fades at a track-local frame. Fade-in: silence before
/// `at`, ramp 0→1 over `len`, unity after. Fade-out: unity before, ramp 1→0,
/// silence after.
fn fade_envelope(track: &AudioTrack, local_frame: usize, rate: u32, timebase: i64) -> f32 {
    use quillon_core::document::FadeKind;
    let mut envelope = 1.0f32;
    for fade in &track.fades {
        let at = tick_to_sample(fade.at, rate, timebase) as usize;
        let len = tick_to_sample(fade.len, rate, timebase) as usize;
        if len == 0 {
            continue;
        }
        let factor = match fade.kind {
            FadeKind::In => {
                if local_frame < at {
                    0.0
                } else {
                    ((local_frame - at) as f32 / len as f32).min(1.0)
                }
            }
            FadeKind::Out => {
                if local_frame < at {
                    1.0
                } else {
                    (1.0 - (local_frame - at) as f32 / len as f32).max(0.0)
                }
            }
        };
        envelope *= factor;
    }
    envelope
}

/// Integrated loudness (LUFS) of an interleaved stereo buffer (EBU R128).
pub fn measure_lufs(samples: &[f32], rate: u32) -> Result<f64, AudioError> {
    let mut meter = ebur128::EbuR128::new(u32::from(MIX_CHANNELS), rate, ebur128::Mode::I)
        .map_err(|e| AudioError::Loudness(e.to_string()))?;
    meter
        .add_frames_f32(samples)
        .map_err(|e| AudioError::Loudness(e.to_string()))?;
    meter
        .loudness_global()
        .map_err(|e| AudioError::Loudness(e.to_string()))
}

/// Scale the buffer by the gain that moves its integrated loudness to
/// `target_lufs`. Returns the loudness measured before normalization.
/// Silence is returned untouched (nothing to normalize).
pub fn normalize_lufs(samples: &mut [f32], rate: u32, target_lufs: f64) -> Result<f64, AudioError> {
    let measured = measure_lufs(samples, rate)?;
    if !measured.is_finite() {
        return Ok(measured); // silence: -inf, nothing to do
    }
    let gain_db = target_lufs - measured;
    let gain = 10f64.powf(gain_db / 20.0);
    for s in samples.iter_mut() {
        *s = (*s * gain as f32).clamp(-1.0, 1.0);
    }
    Ok(measured)
}

/// Write a 16-bit PCM WAV (the deterministic interchange format for muxing;
/// the lossy encode happens once, in ffmpeg, at the very end).
pub fn write_wav(path: &Path, samples: &[f32], channels: u16, rate: u32) -> Result<(), AudioError> {
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = rate * u32::from(channels) * 2;
    let block_align = channels * 2;
    let mut wav = Vec::with_capacity(44 + data_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
        wav.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, wav).map_err(|e| AudioError::Io(e.to_string()))
}

/// The loudness target preset (EBU R128 broadcast standard, research/M §1).
pub const TARGET_LUFS: f64 = -23.0;

#[cfg(test)]
mod tests {
    use super::*;
    use quillon_core::builder::DocumentBuilder;
    use quillon_core::document::{Asset, FadeKind, Scene, Size};
    use quillon_core::time::FLICKS_PER_SECOND;

    /// A stereo sine of `secs` seconds at 48 kHz, amplitude 0.5.
    fn sine(freq: f32, secs: f32, rate: u32) -> DecodedAudio {
        let frames = (secs * rate as f32) as usize;
        let mut samples = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f32 / rate as f32;
            let v = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.5;
            samples.push(v);
            samples.push(v);
        }
        DecodedAudio {
            samples,
            channels: 2,
            rate,
        }
    }

    fn doc_with_audio(first: AudioTrack, second: AudioTrack) -> Document {
        DocumentBuilder::new(Size { w: 320, h: 240 })
            .asset(
                "a",
                Asset::Audio {
                    src: "a.wav".into(),
                    sha256: None,
                },
            )
            .asset(
                "b",
                Asset::Audio {
                    src: "b.wav".into(),
                    sha256: None,
                },
            )
            .audio_track(first)
            .audio_track(second)
            .scene(Scene {
                id: "s".into(),
                duration: FLICKS_PER_SECOND * 3,
                transition_in: None,
                elements: vec![],
            })
            .build()
    }

    #[test]
    fn tick_sample_conversion_is_exact() {
        assert_eq!(
            tick_to_sample(FLICKS_PER_SECOND / 2, 48_000, FLICKS_PER_SECOND),
            24_000
        );
    }

    #[test]
    fn fade_in_envelope_is_piecewise_linear() {
        let track = AudioTrack::new("a").fade(FadeKind::In, 0, FLICKS_PER_SECOND);
        let rate = 48_000;
        assert_eq!(fade_envelope(&track, 0, rate, FLICKS_PER_SECOND), 0.0);
        assert_eq!(fade_envelope(&track, 24_000, rate, FLICKS_PER_SECOND), 0.5);
        assert_eq!(fade_envelope(&track, 48_000, rate, FLICKS_PER_SECOND), 1.0);
        assert_eq!(fade_envelope(&track, 60_000, rate, FLICKS_PER_SECOND), 1.0);
    }

    #[test]
    fn fade_out_envelope_falls_to_silence() {
        let track = AudioTrack::new("a").fade(FadeKind::Out, FLICKS_PER_SECOND, FLICKS_PER_SECOND);
        let rate = 48_000;
        assert_eq!(fade_envelope(&track, 0, rate, FLICKS_PER_SECOND), 1.0);
        assert_eq!(fade_envelope(&track, 72_000, rate, FLICKS_PER_SECOND), 0.5);
        assert_eq!(fade_envelope(&track, 96_000, rate, FLICKS_PER_SECOND), 0.0);
    }

    #[test]
    fn source_window_trims_and_places() {
        // Track b: window [0.5s, 1.0s) placed at 1.0s.
        let track = AudioTrack::new("b")
            .start_at(FLICKS_PER_SECOND)
            .source_range(FLICKS_PER_SECOND / 2, FLICKS_PER_SECOND);
        let doc = doc_with_audio(AudioTrack::new("a"), track);
        let mut sources = BTreeMap::new();
        sources.insert("a".to_string(), sine(440.0, 3.0, 48_000));
        sources.insert("b".to_string(), sine(880.0, 3.0, 48_000));
        let mix = mix_document(&doc, &sources).expect("mix");
        assert_eq!(mix.samples.len(), 3 * 48_000 * 2);
        // After 1.0 s both tracks overlap, so the mix is louder than before.
        let after = 48_000 + 100; // 100 frames past track b's 1.0 s start
        let before = 24_000; // 0.5 s, before track b starts
        assert!(mix.samples[after * 2].abs() > mix.samples[before * 2].abs() + 0.1);
    }

    #[test]
    fn wav_round_trip_preserves_signal() {
        let dir = std::env::temp_dir().join(format!("quillon-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tempdir");
        let path = dir.join("rt.wav");
        let original = sine(440.0, 1.0, 48_000);
        write_wav(&path, &original.samples, 2, 48_000).expect("write");
        let decoded = decode(&path).expect("decode");
        assert_eq!(decoded.rate, 48_000);
        assert_eq!(decoded.channels, 2);
        assert_eq!(decoded.samples.len(), original.samples.len());
        // s16 quantization: ~one LSB (3e-5), assert with margin.
        for (a, b) in decoded.samples.iter().zip(&original.samples) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn silence_normalization_is_a_noop() {
        let mut silence = vec![0.0f32; 48_000 * 2];
        let measured = normalize_lufs(&mut silence, 48_000, TARGET_LUFS).expect("normalize");
        assert!(measured < -70.0, "silence is very quiet, got {measured}");
        assert!(silence.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn loud_sine_measures_and_normalizes() {
        // A 0.5-amplitude sine is loud (well above -23 LUFS).
        let mut samples = sine(440.0, 3.0, 48_000).samples;
        let before = measure_lufs(&samples, 48_000).expect("measure");
        assert!(before > TARGET_LUFS + 1.0, "sine should be loud: {before}");
        normalize_lufs(&mut samples, 48_000, TARGET_LUFS).expect("normalize");
        let after = measure_lufs(&samples, 48_000).expect("re-measure");
        assert!(
            (after - TARGET_LUFS).abs() <= 0.5,
            "target {TARGET_LUFS} ±0.5, got {after}"
        );
    }

    #[test]
    fn resample_changes_length_and_rate() {
        let input = DecodedAudio {
            samples: sine(440.0, 1.0, 44_100).samples,
            channels: 2,
            rate: 44_100,
        };
        let output = resample(&input, 48_000).expect("resample");
        assert_eq!(output.rate, 48_000);
        assert!(
            (output.frames() as i64 - 48_000).abs() < 64,
            "frame count {} ≈ 48000",
            output.frames()
        );
    }

    /// Acceptance #3 (plan §12 v0.3): two tracks with a fade render with
    /// offset < 1 sample against the analytic expectation.
    #[test]
    fn mix_two_tracks_with_fade_matches_expectation() {
        let rate = 48_000u32;
        let half = rate as usize / 2; // 0.5 s in samples
        let track_a = AudioTrack::new("a").gain(0.8);
        let track_b = AudioTrack::new("b")
            .gain(0.5)
            .start_at(FLICKS_PER_SECOND / 2)
            .fade(FadeKind::In, 0, FLICKS_PER_SECOND / 2);
        let doc = doc_with_audio(track_a.clone(), track_b.clone());
        let mut sources = BTreeMap::new();
        sources.insert("a".to_string(), sine(440.0, 3.0, rate));
        sources.insert("b".to_string(), sine(880.0, 3.0, rate));

        let mix = mix_document(&doc, &sources).expect("mix");
        assert_eq!(mix.channels, MIX_CHANNELS);

        let s = |freq: f32, i: usize| {
            (2.0 * std::f32::consts::PI * freq * (i as f32 / rate as f32)).sin() * 0.5
        };
        for i in 0..mix.frames() {
            let mut expected = 0.8 * s(440.0, i);
            if i >= half {
                let local = i - half;
                let env = (local as f32 / half as f32).min(1.0);
                expected += 0.5 * env * s(880.0, local);
            }
            let got = mix.samples[i * 2];
            assert!(
                (got - expected).abs() < 1e-3,
                "frame {i}: got {got}, expected {expected}"
            );
        }

        // The fade midpoint (envelope exactly 0.5) sits on an exact sample.
        let midpoint = half + half / 2;
        let env = fade_envelope(&track_b, midpoint - half, rate, FLICKS_PER_SECOND);
        assert_eq!(env, 0.5, "fade midpoint lands on the exact sample");
    }

    #[test]
    fn freeze_window_mixes_as_silence() {
        // from == to is a legal (empty) audio window.
        let doc = doc_with_audio(
            AudioTrack::new("a").source_range(0, 0),
            AudioTrack::new("b").gain(0.0),
        );
        let mut sources = BTreeMap::new();
        sources.insert("a".to_string(), sine(440.0, 3.0, 48_000));
        sources.insert("b".to_string(), sine(440.0, 3.0, 48_000));
        let mix = mix_document(&doc, &sources).expect("mix");
        assert!(mix.samples.iter().all(|&v| v.abs() < 1e-6));
    }
}
