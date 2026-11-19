//! Media adapters (sidecar mode, plan §7: zero linking, pure MIT path; the
//! user supplies the ffmpeg/ffprobe binaries).
//!
//! Three surfaces: [`Mp4Sink`] encodes raw RGBA frames to MP4 through
//! ffmpeg's stdin (stderr drained on a thread so a chatty encoder can never
//! deadlock the render — research/R); [`probe`] reports what ffprobe sees in
//! a file; [`decoder::SidecarDecoder`] implements core's `FrameSource` with
//! keyframe-accurate, decode-forward seeks (plan §12 v0.3).

pub mod decoder;
pub mod probe;

pub use decoder::{DocumentFrameSource, SidecarDecoder};
pub use probe::{MediaInfo, ProbeError, probe};

use quillon_core::frame_source::Frame;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin};

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("cannot start the video encoder — is ffmpeg installed and on PATH? ({detail})")]
    Spawn { detail: String },
    #[error("the video encoder rejected the render: {detail}")]
    Encode { detail: String },
    #[error("cannot write frame data to the video encoder: {detail}")]
    Write { detail: String },
}

/// An MP4 file receiving raw RGBA frames.
pub struct Mp4Sink {
    stdin: ChildStdin,
    child: ffmpeg_sidecar::child::FfmpegChild,
    stderr_tail: std::sync::mpsc::Receiver<String>,
    path: PathBuf,
}

/// yuv420p needs even dimensions; odd canvases fail deep inside the encoder
/// with a confusing message, so refuse them up front with a clear one.
fn require_even(width: u32, height: u32) -> Result<(), SinkError> {
    if width % 2 != 0 || height % 2 != 0 {
        return Err(SinkError::Encode {
            detail: format!(
                "MP4 output needs even canvas dimensions (got {width}x{height}); \
                 export PNG frames or use an even size"
            ),
        });
    }
    Ok(())
}

impl Mp4Sink {
    /// Spawn ffmpeg writing H.264 (yuv420p) to `path`.
    pub fn new(
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        video_codec: &str,
    ) -> Result<Mp4Sink, SinkError> {
        require_even(width, height)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SinkError::Spawn {
                detail: e.to_string(),
            })?;
        }
        let dims = format!("{width}x{height}");
        let mut command = ffmpeg_sidecar::command::FfmpegCommand::new();
        command
            .args([
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-s",
                &dims,
                "-r",
                &fps.to_string(),
                "-i",
                "-",
                "-c:v",
                video_codec,
                "-preset",
                "medium",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
            ])
            .overwrite()
            .output(path.to_string_lossy().as_ref());
        let mut child = command.spawn().map_err(|e| SinkError::Spawn {
            detail: e.to_string(),
        })?;
        let stdin = child.take_stdin().ok_or_else(|| SinkError::Spawn {
            detail: "ffmpeg stdin unavailable".into(),
        })?;
        let stderr = child.take_stderr().ok_or_else(|| SinkError::Spawn {
            detail: "ffmpeg stderr unavailable".into(),
        })?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || drain_stderr(stderr, tx));
        Ok(Mp4Sink {
            stdin,
            child,
            stderr_tail: rx,
            path: path.to_path_buf(),
        })
    }

    /// Stream one frame. After [`Mp4Sink::finish`] ownership makes further
    /// writes a compile error, so no runtime "already finished" state exists.
    pub fn write_frame(&mut self, frame: &Frame) -> Result<(), SinkError> {
        let expected = frame.width as usize * frame.height as usize * 4;
        if frame.rgba.len() != expected {
            return Err(SinkError::Write {
                detail: format!(
                    "frame buffer is {} bytes, expected {expected} for {}x{}",
                    frame.rgba.len(),
                    frame.width,
                    frame.height
                ),
            });
        }
        self.stdin
            .write_all(&frame.rgba)
            .and_then(|_| self.stdin.flush())
            .map_err(|e| SinkError::Write {
                detail: e.to_string(),
            })
    }

    /// Close stdin, wait for the encoder, verify the output file exists.
    /// Consumes the sink: writing afterwards is a compile error.
    pub fn finish(self) -> Result<(), SinkError> {
        // Dropping stdin signals EOF; ffmpeg flushes the muxer and exits.
        let Mp4Sink {
            stdin,
            mut child,
            stderr_tail,
            path,
        } = self;
        drop(stdin);
        let status = child.wait().map_err(|e| SinkError::Encode {
            detail: e.to_string(),
        })?;
        let tail = Self::drain_tail(&stderr_tail);
        if !status.success() {
            return Err(SinkError::Encode { detail: tail });
        }
        if !path.exists() {
            return Err(SinkError::Encode {
                detail: "the encoder reported success but wrote no file".into(),
            });
        }
        Ok(())
    }

    /// The last lines of encoder output: the operator channel for a local
    /// CLI (never shown as-is in machine-readable output).
    fn drain_tail(stderr_tail: &std::sync::mpsc::Receiver<String>) -> String {
        let mut lines: Vec<String> = Vec::new();
        while let Ok(line) = stderr_tail.try_recv() {
            lines.push(line);
        }
        let tail: Vec<String> = lines.into_iter().rev().take(3).collect();
        tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
    }
}

/// Read stderr line-ish chunks forever; the channel history keeps only what
/// the receiver drains, and the thread exits when ffmpeg closes its stderr.
fn drain_stderr(mut stderr: ChildStderr, tx: std::sync::mpsc::Sender<String>) {
    let mut chunk = [0u8; 4096];
    let mut buffer = Vec::new();
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => emit_complete_lines(&mut buffer, &chunk[..n], &tx),
        }
    }
}

/// Split the buffer on newlines and forward each non-empty line.
fn emit_complete_lines(
    buffer: &mut Vec<u8>,
    incoming: &[u8],
    tx: &std::sync::mpsc::Sender<String>,
) {
    buffer.extend_from_slice(incoming);
    while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
        let line: Vec<u8> = buffer.drain(..=pos).collect();
        let text = String::from_utf8_lossy(&line).trim().to_string();
        if !text.is_empty() {
            // Send errors only mean the receiver is gone.
            let _ = tx.send(text);
        }
    }
    // Keep the tail bounded even without a newline.
    if buffer.len() > 8192 {
        buffer.drain(..buffer.len() - 4096);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_canvas_is_refused_before_spawning() {
        let err = match Mp4Sink::new(Path::new("/tmp/quillon-odd.mp4"), 321, 240, 30, "libx264") {
            Err(err) => err,
            Ok(_) => panic!("odd width must be refused"),
        };
        assert!(err.to_string().contains("even"), "{err}");
    }
}
