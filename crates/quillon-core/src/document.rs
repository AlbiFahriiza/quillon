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
}

impl Asset {
    pub fn src(&self) -> &str {
        match self {
            Asset::Image { src, .. } | Asset::Font { src, .. } => src,
        }
    }

    pub fn sha256(&self) -> Option<&str> {
        match self {
            Asset::Image { sha256, .. } | Asset::Font { sha256, .. } => sha256.as_deref(),
        }
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
    pub anim: Option<Anim>,
}

/// A solid rectangle. `(x, y)` is the top-left corner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RectElement {
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    #[serde(default = "default_text_color")]
    pub color: Color,
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
    pub anim: Option<Anim>,
}

/// A scene element. Rendered bottom-to-top in document order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Element {
    Text(TextElement),
    Rect(RectElement),
    Image(ImageElement),
}

impl Element {
    pub fn anim(&self) -> Option<&Anim> {
        match self {
            Element::Text(e) => e.anim.as_ref(),
            Element::Rect(e) => e.anim.as_ref(),
            Element::Image(e) => e.anim.as_ref(),
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
    #[serde(default)]
    pub elements: Vec<Element>,
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
    pub scenes: Vec<Scene>,
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
            scenes: vec![
                Scene {
                    id: "a".into(),
                    duration: 2 * 705_600_000,
                    elements: vec![],
                },
                Scene {
                    id: "b".into(),
                    duration: 705_600_000,
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
