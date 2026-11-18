//! Video decoding through the system ffmpeg binary (sidecar mode, plan §7:
//! zero linking). Implements core's [`FrameSource`] — the contract every
//! video element consumes (plan §6.2: render never imports this crate).
//!
//! Decode-forward (research/M §2): ffmpeg's input-side `-ss` seeks to the
//! keyframe at or before the target and decodes forward to it, discarding
//! the leading frames — exactly the keyframe-accurate semantics the plan
//! requires for non-keyframe trims. Frames are decoded in bounded chunks
//! and kept in a small LRU keyed by frame index, so sequential renders
//! (the pipeline's normal access pattern) spawn one ffmpeg per chunk, not
//! per frame, and memory stays bounded (research/R's "never buffer the
//! animation" rule).

use quillon_core::document::Document;
use quillon_core::frame_source::{Frame, FrameSource, MediaError};
use quillon_core::time::Tick;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Frames decoded per ffmpeg spawn; bounded so a long clip never lands in
/// RAM wholesale.
const CHUNK_FRAMES: u64 = 32;
/// Ceiling for cached frame bytes (before decoding a chunk would exceed it,
/// the cache is trimmed to the most recent frames only).
const CACHE_BYTE_BUDGET: usize = 64 * 1024 * 1024;

/// Mutable decoder state behind one lock: renders hit the cache in bulk
/// (chunk-ordered access), so a single mutex keeps accounting exact without
/// contention.
#[derive(Default)]
struct DecoderState {
    cache: BTreeMap<u64, Frame>,
    cache_bytes: usize,
    spawns: u32,
}

/// A video file decoded on demand through the ffmpeg sidecar. Safe to share
/// across render workers (core's `FrameSource` contract).
pub struct SidecarDecoder {
    path: PathBuf,
    width: u32,
    height: u32,
    /// Frame rate as an exact rational (numerator, denominator).
    fps: (u32, u32),
    /// Clip duration in document ticks; ticks past it hold the last frame.
    duration_ticks: i64,
    timebase: i64,
    state: Mutex<DecoderState>,
}

impl SidecarDecoder {
    /// Open `path` by probing it first (dimensions, frame rate, duration).
    /// Fails up front — cheapest-first — rather than mid-render.
    pub fn open(path: &Path, timebase: i64) -> Result<SidecarDecoder, MediaError> {
        let info = crate::probe::probe(path).map_err(|e| MediaError::Spawn(e.to_string()))?;
        let (width, height) = match (info.width, info.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => (w, h),
            _ => {
                return Err(MediaError::Decode(
                    "the file has no video stream with usable dimensions".into(),
                ));
            }
        };
        let Some(fps) = info.fps else {
            return Err(MediaError::Decode(
                "the video stream reports no frame rate".into(),
            ));
        };
        let duration_ticks = info.duration_ticks(timebase).unwrap_or(i64::MAX);
        Ok(SidecarDecoder {
            path: path.to_path_buf(),
            width,
            height,
            fps,
            duration_ticks,
            timebase,
            state: Mutex::new(DecoderState::default()),
        })
    }

    /// How many ffmpeg processes have been spawned so far.
    pub fn spawns(&self) -> u32 {
        self.state.lock().expect("decoder lock").spawns
    }

    /// Map a document tick to a source frame index (exact rational math).
    fn frame_index(&self, tick: Tick) -> u64 {
        let (num, den) = self.fps;
        // round(tick * num / (den * timebase)) in integer arithmetic.
        let numerator =
            tick as i128 * i128::from(num) * 2 + i128::from(den) * self.timebase as i128;
        let denominator = i128::from(den) * self.timebase as i128 * 2;
        (numerator / denominator).max(0) as u64
    }

    /// Last frame index reachable before the clip's duration runs out:
    /// the total frame count implied by duration and rate, minus one.
    /// (Querying `frame_index(duration - 1)` instead can overshoot: the
    /// last tick before the boundary still rounds up to the next frame
    /// slot, which the clip does not contain.)
    fn last_frame_index(&self) -> Option<u64> {
        if self.duration_ticks <= 0 {
            return None;
        }
        let (num, den) = self.fps;
        let numerator = self.duration_ticks as i128 * i128::from(num) * 2
            + i128::from(den) * self.timebase as i128;
        let denominator = i128::from(den) * self.timebase as i128 * 2;
        let total = numerator / denominator;
        Some((total - 1).max(0) as u64)
    }

    /// Decode the chunk starting at `start` into the cache.
    fn decode_chunk(&self, start: u64) -> Result<(), MediaError> {
        let (num, den) = self.fps;
        // Seek half a frame early: the first decoded frame is then exactly
        // `start` (the previous frame's timestamp falls below the seek point
        // and is discarded by ffmpeg's accurate input seek).
        let frame_secs = f64::from(den) / f64::from(num);
        let seek_secs = (start as f64 - 0.5) * frame_secs;
        let seek = if seek_secs > 0.0 {
            format!("{seek_secs:.6}")
        } else {
            "0".to_string()
        };
        let count = CHUNK_FRAMES.min(self.cache_capacity().max(2) as u64);

        let mut command = ffmpeg_sidecar::command::FfmpegCommand::new();
        command
            .args(["-hide_banner", "-loglevel", "error"])
            .seek(&seek)
            .input(self.path.to_string_lossy().as_ref())
            .frames(count as u32)
            .no_audio()
            // Without passthrough, ffmpeg's default CFR mode pads the seeked
            // stream by duplicating its first frame — every subsequent frame
            // would shift by one (verified against select=eq(n,N) references).
            .args([
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
            ])
            .overwrite()
            .output("-");
        let mut child = command
            .spawn()
            .map_err(|e| MediaError::Spawn(e.to_string()))?;
        let mut stdout = child
            .take_stdout()
            .ok_or_else(|| MediaError::Spawn("ffmpeg stdout unavailable".into()))?;

        let frame_bytes = self.width as usize * self.height as usize * 4;
        let mut decoded = BTreeMap::new();
        let mut idx = start;
        loop {
            let mut buf = vec![0u8; frame_bytes];
            match read_exact_or_eof(&mut stdout, &mut buf) {
                Ok(true) => {
                    decoded.insert(
                        idx,
                        Frame {
                            width: self.width,
                            height: self.height,
                            rgba: buf,
                        },
                    );
                    idx += 1;
                }
                Ok(false) => break, // clean EOF: the clip ended mid-chunk
                Err(e) => return Err(MediaError::Decode(e)),
            }
        }
        let status = child
            .wait()
            .map_err(|e| MediaError::Decode(e.to_string()))?;
        if !status.success() && decoded.is_empty() {
            return Err(MediaError::Decode(format!(
                "ffmpeg exited with {status} before producing any frame"
            )));
        }

        let mut state = self.state.lock().expect("decoder lock");
        let gained: usize = decoded.values().map(|f| f.rgba.len()).sum();
        for (i, frame) in decoded {
            state.cache.insert(i, frame);
        }
        state.cache_bytes += gained;
        state.spawns += 1;
        self.trim_to_budget(&mut state);
        Ok(())
    }

    /// How many whole frames fit the byte budget.
    fn cache_capacity(&self) -> usize {
        let frame_bytes = (self.width as usize * self.height as usize * 4).max(1);
        (CACHE_BYTE_BUDGET / frame_bytes).max(2)
    }

    /// Keep only the most recent frames when the budget is exceeded.
    fn trim_to_budget(&self, state: &mut DecoderState) {
        while state.cache_bytes > CACHE_BYTE_BUDGET && state.cache.len() > 1 {
            // Drop the oldest cached frame.
            let oldest = *state.cache.keys().next().expect("non-empty cache");
            if let Some(frame) = state.cache.remove(&oldest) {
                state.cache_bytes -= frame.rgba.len();
            }
        }
    }

    /// The decoded frame at `tick` (the `FrameSource` body, shared by the
    /// document-level source below).
    pub fn decode_at(&self, tick: Tick) -> Result<Frame, MediaError> {
        let mut index = self.frame_index(tick);
        // Ticks past the end hold the last frame (freeze/hold semantics,
        // matching how animation tracks clamp at their final key).
        if let Some(last) = self.last_frame_index() {
            index = index.min(last);
        }
        {
            let state = self.state.lock().expect("decoder lock");
            if let Some(frame) = state.cache.get(&index) {
                return Ok(frame.clone());
            }
        }
        self.decode_chunk(index)?;
        let state = self.state.lock().expect("decoder lock");
        state
            .cache
            .get(&index)
            .cloned()
            .ok_or_else(|| MediaError::Decode(format!("no frame decoded at index {index}")))
    }
}

/// A document's video assets, each with its own decoder, behind one
/// `FrameSource` (plan §6.3): the render pipeline consumes this.
pub struct DocumentFrameSource {
    decoders: BTreeMap<String, SidecarDecoder>,
}

impl DocumentFrameSource {
    /// Open a decoder for every video asset the document declares. Fails up
    /// front (cheapest-first) on unreadable or unprobeable files.
    /// True when the document declares no video assets.
    pub fn is_empty(&self) -> bool {
        self.decoders.is_empty()
    }

    pub fn open(doc: &Document, base_dir: &Path) -> Result<DocumentFrameSource, MediaError> {
        let mut decoders = BTreeMap::new();
        for (id, asset) in &doc.assets {
            if let quillon_core::document::Asset::Video { .. } = asset {
                let decoder = SidecarDecoder::open(&base_dir.join(asset.src()), doc.timebase)?;
                decoders.insert(id.clone(), decoder);
            }
        }
        Ok(DocumentFrameSource { decoders })
    }
}

impl FrameSource for DocumentFrameSource {
    fn frame_at(&self, asset: &str, tick: Tick) -> Result<Frame, MediaError> {
        let decoder = self.decoders.get(asset).ok_or_else(|| {
            MediaError::Spawn(format!("no decoder was opened for video asset \"{asset}\""))
        })?;
        decoder.decode_at(tick)
    }
}

/// Read exactly `buf.len()` bytes (`Ok(true)`) or hit a clean EOF at the
/// very start (`Ok(false)`). A partial frame at EOF is an error: raw frame
/// streams must end on frame boundaries.
fn read_exact_or_eof<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<bool, String> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => {
                return if filled == 0 {
                    Ok(false)
                } else {
                    Err(format!(
                        "raw stream ended mid-frame ({filled} of {} bytes)",
                        buf.len()
                    ))
                };
            }
            Ok(n) => filled += n,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_decoder(fps: (u32, u32), timebase: i64, duration_ticks: i64) -> SidecarDecoder {
        SidecarDecoder {
            path: PathBuf::from("unused"),
            width: 4,
            height: 4,
            fps,
            duration_ticks,
            timebase,
            state: Mutex::new(DecoderState::default()),
        }
    }

    #[test]
    fn frame_index_maps_ticks_exactly() {
        // 10 fps, one tick per tenth of a frame: pure integer rounding.
        let d = test_decoder((10, 1), 705_600_000, 3 * 705_600_000);
        assert_eq!(d.frame_index(0), 0);
        assert_eq!(d.frame_index(705_600_000), 10);
        assert_eq!(d.frame_index(352_800_000), 5);
        assert_eq!(d.frame_index(35_279_999), 0); // just below half a frame
        assert_eq!(d.frame_index(35_280_000), 1); // exactly half rounds up
    }

    #[test]
    fn fractional_rates_map_without_float_drift() {
        // NTSC-ish 30000/1001: the rational must survive tick math exactly.
        let d = test_decoder((30000, 1001), 705_600_000, 705_600_000);
        let ticks_per_frame = 705_600_000 * 1001 / 30000;
        assert_eq!(d.frame_index(ticks_per_frame), 1);
        assert_eq!(d.frame_index(10 * ticks_per_frame), 10);
    }

    #[test]
    fn last_frame_index_stops_at_duration() {
        let d = test_decoder((10, 1), 705_600_000, 2 * 705_600_000);
        assert_eq!(d.last_frame_index(), Some(19));
        let empty = test_decoder((10, 1), 705_600_000, 0);
        assert_eq!(empty.last_frame_index(), None);
    }
}
