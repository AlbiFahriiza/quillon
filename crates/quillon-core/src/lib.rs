//! Quillon document model: schema, validation, lint, canonical JSON, builder.
//!
//! Layer contract (myquillonplan.md §6.2): this crate imports no internal
//! Quillon crate. Everything downstream (render, media, cli) builds on it.

pub mod builder;
pub mod canonical;
pub mod color;
pub mod document;
pub mod draft;
pub mod easing;
pub mod frame_source;
pub mod lint;
pub mod schema;
pub mod time;
pub mod validate;

pub use builder::DocumentBuilder;
pub use canonical::canonical_json;
pub use color::Color;
pub use document::{
    Anim, Asset, ColorSpace, Document, Element, Fit, ImageElement, Key, RectElement, Scene, Size,
    TextAlign, TextElement, Track,
};
pub use easing::{Ease, ElementState, apply_ease, evaluate_anim, evaluate_track};
pub use frame_source::{Frame, FrameSource, MediaError, NullFrameSource};
pub use lint::{LintFinding, lint_document};
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
