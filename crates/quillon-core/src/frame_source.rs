//! The render contract (myquillonplan.md §6.3): a frame source maps an
//! (asset, tick) pair to a decoded frame. The trait lives in core;
//! quillon-media implements it. `Send + Sync` is part of the contract:
//! the parallel pipeline shares one source across rayon workers.

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
    /// The document references media this build cannot decode.
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

/// Frames of a document's video assets, addressed by asset id and tick.
/// Implementations decode from the nearest keyframe at or before `tick`
/// (plan §6.3, research/M §2) and must be safe to share across threads.
pub trait FrameSource: Send + Sync {
    fn frame_at(&self, asset: &str, tick: crate::time::Tick) -> Result<Frame, MediaError>;
}

/// The no-media source: every (asset, tick) errors. Documents without video
/// assets render against it; a video element reaching it is a render error.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullFrameSource;

impl FrameSource for NullFrameSource {
    fn frame_at(&self, _asset: &str, _tick: crate::time::Tick) -> Result<Frame, MediaError> {
        Err(MediaError::Unavailable)
    }
}
