//! Quillon document model: schema, validation, lint, canonical JSON, builder.
//!
//! Layer contract (myquillonplan.md §6.2): this crate imports no internal
//! Quillon crate. Everything downstream (render, media, cli) builds on it.

pub mod builder;
pub mod canonical;
pub mod caption;
pub mod color;
pub mod diff;
pub mod document;
pub mod draft;
pub mod easing;
pub mod frame_source;
pub mod lint;
pub mod schema;
pub mod subtitle;
pub mod template;
pub mod time;
pub mod validate;

pub use builder::DocumentBuilder;
pub use canonical::canonical_json;
pub use caption::{CaptionCue, CaptionPosition, CaptionStyle, CaptionTrack};
pub use color::Color;
pub use diff::{DiffEntry, DiffReport, diff_documents};
pub use document::{
    Anim, Asset, AudioTrack, ColorSpace, Document, Element, FadeDef, FadeKind, Fit, ImageElement,
    Key, RectElement, Scene, Size, SourceRange, TextAlign, TextElement, Track, VideoElement,
};
pub use easing::{Ease, ElementState, apply_ease, evaluate_anim, evaluate_track};
pub use frame_source::{Frame, FrameSource, MediaError, NullFrameSource};
pub use lint::{LintFinding, lint_document};
pub use subtitle::{
    RegularizeConfig, SubtitleError, ms_to_ticks, parse_srt, parse_vtt, regularize_cues,
    ticks_to_ms, to_srt, to_vtt,
};
pub use template::{TemplateError, VariableDef, VariableType, apply_template_json};
pub use validate::{ValidationError, validate_document, validate_str};

/// Document format version accepted by this build (plan §5, §14).
pub const DOCUMENT_VERSION: u32 = 1;

/// Explicit document limits (plan §11: unbounded input is not accepted).
pub const MAX_SCENES: usize = 64;
pub const MAX_ELEMENTS_PER_SCENE: usize = 256;
pub const MAX_ASSETS: usize = 128;
pub const MAX_TEXT_LEN: usize = 10_000;
pub const MAX_TEXT_SIZE: f32 = 512.0;
pub const MIN_DIMENSION: u32 = 16;
pub const MAX_DIMENSION: u32 = 7680;
pub const MAX_FPS: u32 = 240;
pub const MAX_ID_LEN: usize = 128;
pub const MAX_KEYS_PER_TRACK: usize = 64;

/// Largest accepted shadow blur radius in pixels (plan §11: bounded input).
pub const MAX_BLUR_PX: f32 = 512.0;
/// Largest accepted corner radius in pixels.
pub const MAX_RADIUS_PX: f32 = 10_000.0;
/// Most gradient stops accepted on one fill.
pub const MAX_GRADIENT_STOPS: usize = 32;
/// Largest accepted video retime factor (plan §12 v0.3).
pub const MAX_SPEED: f32 = 10.0;
/// Largest accepted linear audio gain.
pub const MAX_GAIN: f32 = 8.0;
/// Audio mix rates accepted (the flicks timebase divides them exactly).
pub const MIN_AUDIO_RATE: u32 = 8_000;
pub const MAX_AUDIO_RATE: u32 = 192_000;
/// Most audio tracks in one mix.
pub const MAX_AUDIO_TRACKS: usize = 32;
/// Most fades accepted on one audio track.
pub const MAX_FADES_PER_TRACK: usize = 64;
/// Most caption tracks in one document (track 2 is usually a translation).
pub const MAX_CAPTION_TRACKS: usize = 8;
/// Most cues accepted on one caption track.
pub const MAX_CUES_PER_TRACK: usize = 5_000;
