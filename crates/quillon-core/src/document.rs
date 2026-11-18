//! The v1 document model (myquillonplan.md §5). Elements are a
//! `type`-discriminated union; time is ticks; colors are sRGB hex.

use crate::color::Color;
use crate::easing::Ease;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One second in ticks; the only timebase accepted by format v1.
fn default_timebase() -> i64 {
    crate::time::FLICKS_PER_SECOND
}

fn default_fps() -> u32 {
    30
}

fn default_bit_depth() -> u8 {
    8
}

fn default_bg() -> Color {
    Color::from_rgb8(0, 0, 0)
}

fn default_text_color() -> Color {
    Color::from_rgb8(255, 255, 255)
}

fn default_text_align() -> TextAlign {
    TextAlign::Center
}

fn default_fit() -> Fit {
    Fit::Contain
}

/// Canvas size in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub w: u32,
    pub h: u32,
}

/// Authoring color space of document colors. Format v1 accepts sRGB only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColorSpace {
    #[default]
    Srgb,
}

/// Horizontal text alignment within its line box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    #[default]
    Center,
    Right,
}

/// How an image fills its destination box (Editly's resizeMode set, research/G §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    Contain,
    Cover,
    Stretch,
}

/// How a scene enters: a hard cut (the default) or a crossfade blended in
/// linear light over the previous scene's continuing frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    Cut,
    Crossfade,
}

/// The transition into a scene. It consumes the head of the incoming scene;
/// the timeline stays the sum of scene durations (no overlap arithmetic).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transition {
    pub kind: TransitionKind,
    /// In ticks. `0` for a cut; for a crossfade, `> 0` and `<=` the scene
    /// duration.
    pub duration: i64,
}

/// One color stop of a gradient. `at` is a position along the gradient axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GradientStop {
    /// In [0, 1]; stops must be strictly increasing in `at`.
    pub at: f32,
    pub color: Color,
}

/// A linear gradient fill. Colors are interpolated in linear light
/// (research/T); `angle` follows screen space: 0 is left-to-right, 90 is
/// top-to-bottom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GradientDef {
    /// Degrees, clockwise from the +x axis.
    pub angle: f32,
    /// At least two.
    pub stops: Vec<GradientStop>,
}

/// A drop shadow cast by an element's silhouette, blurred in linear light
/// and composited under the element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShadowDef {
    /// Horizontal offset in pixels, right-positive.
    pub offset_x: f32,
    /// Vertical offset in pixels, down-positive.
    pub offset_y: f32,
    /// Blur radius in pixels, [0, 512].
    pub blur: f32,
    pub color: Color,
    /// Shadow opacity in [0, 1].
    #[serde(default = "default_full")]
    pub alpha: f32,
}

fn default_full() -> f32 {
    1.0
}

/// A rectangular clip in element-author coordinates: rect and image origins
/// are their top-left corner, text origin is its center. The clip travels
/// with the element's transform and masks the element and its shadow.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClipRect {
    pub x: i64,
    pub y: i64,
    /// Must be > 0.
    pub w: i64,
    /// Must be > 0.
    pub h: i64,
}

/// A referenced asset. `src` is relative to the document's directory and must
/// stay inside it (no absolute paths, no `..`); `sha256`, when present, pins
/// the exact bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Asset {
    Image {
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Font {
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Video {
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    Audio {
        src: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
}

impl Asset {
    pub fn src(&self) -> &str {
        match self {
            Asset::Image { src, .. }
            | Asset::Font { src, .. }
            | Asset::Video { src, .. }
            | Asset::Audio { src, .. } => src,
        }
    }

    pub fn sha256(&self) -> Option<&str> {
        match self {
            Asset::Image { sha256, .. }
            | Asset::Font { sha256, .. }
            | Asset::Video { sha256, .. }
            | Asset::Audio { sha256, .. } => sha256.as_deref(),
        }
    }
}

/// A half-open range of source-media time in ticks. `to > from` unless the
/// range freezes a single frame (`from == to`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceRange {
    /// Inclusive start tick within the source.
    pub from: i64,
    /// Exclusive end tick within the source; `> from`, or `== from` for a
    /// single frozen frame.
    pub to: i64,
}

impl SourceRange {
    /// Duration of the range in ticks (0 for a freeze).
    pub fn duration(&self) -> i64 {
        self.to - self.from
    }

    /// A freeze holds one frame: `from == to`.
    pub fn is_freeze(&self) -> bool {
        self.from == self.to
    }
}

/// One keyframe. `ease` applies to the segment starting at this key.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Key {
    /// Scene-local time in ticks, `>= 0` and `<=` the scene duration.
    pub t: i64,
    /// Animated value: opacity in [0,1]; translate in px; scale around 1.0;
    /// rotation in degrees.
    pub v: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ease: Option<Ease>,
}

/// A keyframe track for one animatable property.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// At least two, strictly increasing in `t`.
    pub keys: Vec<Key>,
}

/// Per-element animation. Every property is optional and independent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Anim {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate_x: Option<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translate_y: Option<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Track>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<Track>,
}

/// A text block. `(x, y)` is the *center* of the laid-out text, matching the
/// plan's example (title at 960, 540 on a 1920x1080 canvas).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextElement {
    /// Non-empty.
    pub text: String,
    /// Key of a `font` asset.
    pub font: String,
    pub x: i64,
    pub y: i64,
    /// Font size in pixels, (0, 512].
    pub size: f32,
    #[serde(default = "default_text_color")]
    pub color: Color,
    #[serde(default = "default_text_align")]
    pub align: TextAlign,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<ClipRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<Anim>,
}

/// A solid or gradient-filled rectangle. `(x, y)` is the top-left corner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RectElement {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    #[serde(default = "default_text_color")]
    pub color: Color,
    /// When present, the fill is this gradient (it wins over `color`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<GradientDef>,
    /// Corner radius in pixels, clamped at render to half the shorter side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<ClipRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<Anim>,
}

/// An image drawn into a destination box. `(x, y)` is the top-left corner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImageElement {
    /// Key of an `image` asset.
    pub asset: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    #[serde(default = "default_fit")]
    pub fit: Fit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<ClipRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<Anim>,
}

/// A video drawn into a destination box, trimmed from its source (plan §5.1,
/// §12 v0.3). `(x, y)` is the top-left corner, like an image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VideoElement {
    /// Key of a `video` asset.
    pub asset: String,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    #[serde(default = "default_fit")]
    pub fit: Fit,
    /// Source window in ticks; absent means the whole clip. `from == to`
    /// freezes that single frame across the element's presence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_range: Option<SourceRange>,
    /// Playback speed multiplier (retime); 1.0 is real time.
    #[serde(default = "default_speed", skip_serializing_if = "is_default_speed")]
    pub speed: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<ClipRect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anim: Option<Anim>,
}

fn default_speed() -> f32 {
    1.0
}

fn is_default_speed(speed: &f32) -> bool {
    *speed == 1.0
}

/// A scene element. Rendered bottom-to-top in document order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Element {
    Text(TextElement),
    Rect(RectElement),
    Image(ImageElement),
    Video(VideoElement),
}

impl Element {
    pub fn anim(&self) -> Option<&Anim> {
        match self {
            Element::Text(e) => e.anim.as_ref(),
            Element::Rect(e) => e.anim.as_ref(),
            Element::Image(e) => e.anim.as_ref(),
            Element::Video(e) => e.anim.as_ref(),
        }
    }
}

/// A scene: `duration` ticks, elements stacked bottom-to-top.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// Unique, non-empty, at most 128 bytes.
    pub id: String,
    /// In ticks, `> 0`.
    pub duration: i64,
    /// How this scene enters; `None` means a hard cut. Only meaningful from
    /// the second scene on (the first scene cannot fade from anything).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<Transition>,
    #[serde(default)]
    pub elements: Vec<Element>,
}

impl Scene {
    /// Duration of the incoming crossfade, if this scene has one.
    pub fn crossfade_in(&self) -> Option<i64> {
        match self.transition_in {
            Some(Transition {
                kind: TransitionKind::Crossfade,
                duration,
            }) if duration > 0 => Some(duration),
            _ => None,
        }
    }
}

/// Direction of an audio fade: `in` rises from silence, `out` falls to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FadeKind {
    In,
    Out,
}

/// A linear gain fade applied to an audio track, in the tick domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FadeDef {
    /// Track-local start tick, `>= 0`.
    pub at: i64,
    /// Fade length in ticks, `> 0`.
    pub len: i64,
    pub kind: FadeKind,
}

/// One audio track of the mix: an audio asset placed on the document
/// timeline with gain, fades, and an optional source window (plan §5.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioTrack {
    /// Key of an `audio` asset.
    pub asset: String,
    /// Linear gain, `>= 0` (1.0 = unity).
    #[serde(default = "default_gain", skip_serializing_if = "is_default_gain")]
    pub gain: f32,
    /// Placement on the document timeline in ticks.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start_at: i64,
    /// Source window in ticks; absent means the whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_range: Option<SourceRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fades: Vec<FadeDef>,
}

fn default_gain() -> f32 {
    1.0
}

fn is_default_gain(gain: &f32) -> bool {
    *gain == 1.0
}

fn is_zero(ticks: &i64) -> bool {
    *ticks == 0
}

fn default_audio_rate() -> u32 {
    48_000
}

/// The root document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Document {
    /// Format version; `1` for this build.
    pub v: u32,
    #[serde(default = "default_timebase")]
    pub timebase: i64,
    pub size: Size,
    #[serde(default = "default_fps")]
    pub fps: u32,
    #[serde(default)]
    pub color_space: ColorSpace,
    #[serde(default = "default_bit_depth")]
    pub bit_depth: u8,
    #[serde(default = "default_bg")]
    pub bg: Color,
    #[serde(default)]
    pub assets: BTreeMap<String, Asset>,
    /// Audio mix rate in Hz; every track is resampled to it. Flicks divide
    /// it exactly for the standard rates (48k, 44.1k).
    #[serde(
        default = "default_audio_rate",
        skip_serializing_if = "is_default_audio_rate"
    )]
    pub audio_rate: u32,
    /// The audio mix: tracks are summed, each with gain, fades, and placement.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audio: Vec<AudioTrack>,
    pub scenes: Vec<Scene>,
}

fn is_default_audio_rate(rate: &u32) -> bool {
    *rate == 48_000
}

impl Document {
    /// Total duration in ticks (sum of scene durations).
    pub fn total_duration(&self) -> i64 {
        self.scenes.iter().map(|s| s.duration).sum()
    }

    /// Scene index containing `tick`, and the tick relative to the scene
    /// start. `None` when the tick is past the end.
    pub fn scene_at(&self, tick: i64) -> Option<(usize, i64)> {
        let mut start = 0i64;
        for (i, scene) in self.scenes.iter().enumerate() {
            if tick < start + scene.duration {
                return Some((i, tick - start));
            }
            start += scene.duration;
        }
        None
    }

    /// Frame count of the full render at the document's fps.
    pub fn frame_count(&self) -> u64 {
        let frame = crate::time::frame_tick(self.fps);
        if frame <= 0 {
            return 0;
        }
        (self.total_duration() as u64).div_ceil(frame as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_two_scenes() -> Document {
        Document {
            v: 1,
            timebase: crate::time::FLICKS_PER_SECOND,
            size: Size { w: 1920, h: 1080 },
            fps: 30,
            color_space: ColorSpace::Srgb,
            bit_depth: 8,
            bg: default_bg(),
            assets: BTreeMap::new(),
            audio_rate: 48_000,
            audio: Vec::new(),
            scenes: vec![
                Scene {
                    id: "a".into(),
                    duration: 2 * 705_600_000,
                    transition_in: None,
                    elements: vec![],
                },
                Scene {
                    id: "b".into(),
                    duration: 705_600_000,
                    transition_in: None,
                    elements: vec![],
                },
            ],
        }
    }

    #[test]
    fn total_duration_sums_scenes() {
        assert_eq!(doc_two_scenes().total_duration(), 3 * 705_600_000);
    }

    #[test]
    fn scene_at_maps_tick_to_scene_local_time() {
        let doc = doc_two_scenes();
        assert_eq!(doc.scene_at(0), Some((0, 0)));
        assert_eq!(doc.scene_at(705_600_000), Some((0, 705_600_000)));
        // First tick of the second scene.
        assert_eq!(doc.scene_at(2 * 705_600_000), Some((1, 0)));
        // One tick past the end is out of range.
        assert_eq!(doc.scene_at(3 * 705_600_000), None);
    }

    #[test]
    fn frame_count_at_30fps_is_exact() {
        let doc = doc_two_scenes();
        assert_eq!(doc.frame_count(), 90);
    }
}
