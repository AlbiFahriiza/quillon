//! Caption tracks (plan §4 F5, §12 v0.5): document-level cue lists with a
//! shared style, burned in on top of the composited frame. Cue timing is in
//! document-timeline ticks and may cross scene boundaries — an SRT/VTT
//! timeline is global to the video, which is why captions are tracks next
//! to `audio` and not scene elements.

use crate::color::Color;
use crate::document::{ShadowDef, TextAlign};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_caption_color() -> Color {
    Color::from_rgb8(255, 255, 255)
}

fn default_caption_margin() -> i64 {
    48
}

fn is_default_margin(margin: &i64) -> bool {
    *margin == 48
}

/// Vertical placement of the caption block on the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptionPosition {
    /// Near the bottom edge, above `margin` pixels (the subtitle default).
    #[default]
    Bottom,
    /// Vertically centered on the canvas.
    Center,
    /// Near the top edge, below `margin` pixels.
    Top,
}

/// How one caption track looks. One style per track; a second track (a
/// translation, say) brings its own style and position.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionStyle {
    /// Key of a `font` asset.
    pub font: String,
    /// Font size in pixels, (0, 512].
    pub size: f32,
    #[serde(default = "default_caption_color")]
    pub color: Color,
    #[serde(default)]
    pub align: TextAlign,
    #[serde(default)]
    pub position: CaptionPosition,
    /// Gap between the caption block and its canvas edge, in pixels.
    #[serde(
        default = "default_caption_margin",
        skip_serializing_if = "is_default_margin"
    )]
    pub margin: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowDef>,
    /// Optional backing box color drawn under the text block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<Color>,
}

/// One caption: `text` shown for the half-open tick range `[from, to)` on
/// the document timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionCue {
    /// Inclusive start tick, `>= 0`.
    pub from: i64,
    /// Exclusive end tick; `> from` and `<=` the document duration.
    pub to: i64,
    /// Non-empty; newlines split lines.
    pub text: String,
}

/// A caption track: one style, cues sorted and non-overlapping (enforced by
/// validation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptionTrack {
    pub style: CaptionStyle,
    pub cues: Vec<CaptionCue>,
}

impl CaptionTrack {
    /// The cue covering `tick`, if any. Cues are validated sorted and
    /// non-overlapping, so the only candidate is the last cue starting at
    /// or before `tick`.
    pub fn cue_at(&self, tick: i64) -> Option<&CaptionCue> {
        let idx = self.cues.partition_point(|c| c.from <= tick);
        let cue = self.cues.get(idx.wrapping_sub(1))?;
        (tick < cue.to).then_some(cue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(cues: &[(i64, i64, &str)]) -> CaptionTrack {
        CaptionTrack {
            style: CaptionStyle {
                font: "body".into(),
                size: 42.0,
                color: default_caption_color(),
                align: TextAlign::Center,
                position: CaptionPosition::Bottom,
                margin: default_caption_margin(),
                shadow: None,
                bg: None,
            },
            cues: cues
                .iter()
                .map(|(from, to, text)| CaptionCue {
                    from: *from,
                    to: *to,
                    text: text.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn cue_at_is_half_open() {
        let t = track(&[(10, 20, "a"), (30, 40, "b")]);
        assert!(t.cue_at(9).is_none());
        assert_eq!(t.cue_at(10).unwrap().text, "a");
        assert_eq!(t.cue_at(19).unwrap().text, "a");
        assert!(t.cue_at(20).is_none(), "the end tick is exclusive");
        assert!(t.cue_at(29).is_none(), "gaps between cues show nothing");
        assert_eq!(t.cue_at(39).unwrap().text, "b");
        assert!(t.cue_at(40).is_none());
    }

    #[test]
    fn cue_at_on_empty_track_is_none() {
        let t = track(&[]);
        assert!(t.cue_at(0).is_none());
        assert!(t.cue_at(123_456).is_none());
    }

    #[test]
    fn minimal_style_json_takes_the_documented_defaults() {
        let style: CaptionStyle =
            serde_json::from_str(r#"{"font":"body","size":42}"#).expect("minimal style");
        assert_eq!(style.color.as_hex(), "#ffffff");
        assert_eq!(style.align, TextAlign::Center);
        assert_eq!(style.position, CaptionPosition::Bottom);
        assert_eq!(style.margin, 48);
        assert!(style.shadow.is_none() && style.bg.is_none());
        // Defaults are skipped on serialize: the canonical form stays small.
        let value = serde_json::to_value(&style).unwrap();
        assert!(value.get("margin").is_none());
        assert!(value.get("bg").is_none());
    }
}
