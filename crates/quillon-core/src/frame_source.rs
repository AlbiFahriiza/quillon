//! The render contract (myquillonplan.md §6.3): a frame source maps a tick
//! to a decoded frame. The trait lives in core; quillon-media implements it.
//! Format v1 documents have no video elements, so the only implementation
//! here is the null source that every video element would reject against.

/// A decoded RGBA frame, 8 bits per channel, straight (non-premultiplied)
/// alpha, 4 bytes per pixel, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// The document references media this build cannot decode (format v1 has
    /// no video elements; this reaches the caller as a render error).
    #[error("no media source is available for this element")]
    Unavailable,
    /// The decoder process could not be started (ffmpeg missing or rejected
    /// the arguments).
    #[error("cannot start the media decoder — is ffmpeg installed and on PATH? ({0})")]
    Spawn(String),
    /// The decoder ran but produced a failure or unusable output.
    #[error("media decode failed: {0}")]
    Decode(String),
}

/// Frames of a source clip, addressed by tick. Implementations decode from
/// the nearest keyframe at or before `tick` (plan §6.3, research/M §2).
pub trait FrameSource {
    fn frame_at(&self, tick: crate::time::Tick) -> Result<Frame, MediaError>;
}

/// The v1 placeholder: no media, every tick errors.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullFrameSource;

impl FrameSource for NullFrameSource {
    fn frame_at(&self, _tick: crate::time::Tick) -> Result<Frame, MediaError> {
        Err(MediaError::Unavailable)
    }
}
