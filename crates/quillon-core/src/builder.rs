//! Rust builder API (plan §4 F9, acceptance v0.1 #4): a builder-produced
//! document serializes to exactly the same canonical JSON as a hand-written
//! document. The builder does not validate; run the document through
//! [`crate::validate_document`] before rendering.

use crate::caption::CaptionTrack;
use crate::color::Color;
use crate::document::{
    Anim, Asset, AudioTrack, ClipRect, ColorSpace, Document, FadeDef, FadeKind, Fit, GradientDef,
    ImageElement, RectElement, Scene, ShadowDef, Size, SourceRange, TextAlign, TextElement,
    VideoElement,
};
use crate::template::VariableDef;
use crate::time::FLICKS_PER_SECOND;
use std::collections::BTreeMap;

/// Fluent document builder. Defaults match the serde field defaults exactly,
/// so a document built without overrides equals the same document parsed
/// from JSON that omits those fields.
pub struct DocumentBuilder {
    doc: Document,
}

impl DocumentBuilder {
    pub fn new(size: Size) -> Self {
        DocumentBuilder {
            doc: Document {
                v: crate::DOCUMENT_VERSION,
                timebase: FLICKS_PER_SECOND,
                size,
                fps: 30,
                color_space: ColorSpace::Srgb,
                bit_depth: 8,
                bg: Color::from_rgb8(0, 0, 0),
                assets: BTreeMap::new(),
                audio_rate: 48_000,
                audio: Vec::new(),
                captions: Vec::new(),
                variables: BTreeMap::new(),
                scenes: Vec::new(),
            },
        }
    }

    pub fn fps(mut self, fps: u32) -> Self {
        self.doc.fps = fps;
        self
    }

    pub fn bg(mut self, bg: Color) -> Self {
        self.doc.bg = bg;
        self
    }

    pub fn asset(mut self, id: &str, asset: Asset) -> Self {
        self.doc.assets.insert(id.to_string(), asset);
        self
    }

    pub fn scene(mut self, scene: Scene) -> Self {
        self.doc.scenes.push(scene);
        self
    }

    /// Add a track to the document's audio mix (plan §5.1 audio array).
    pub fn audio_track(mut self, track: AudioTrack) -> Self {
        self.doc.audio.push(track);
        self
    }

    /// Add a caption track to be burned in over the rendered frames
    /// (plan §12 v0.5).
    pub fn caption_track(mut self, track: CaptionTrack) -> Self {
        self.doc.captions.push(track);
        self
    }

    /// Declare a typed template variable (plan §4 F12).
    pub fn variable(mut self, name: &str, def: VariableDef) -> Self {
        self.doc.variables.insert(name.to_string(), def);
        self
    }

    /// Set the audio mix rate in Hz (default 48 000).
    pub fn audio_rate(mut self, rate: u32) -> Self {
        self.doc.audio_rate = rate;
        self
    }

    pub fn build(self) -> Document {
        self.doc
    }
}

impl VideoElement {
    pub fn new(asset: impl Into<String>, x: i64, y: i64, w: i64, h: i64) -> Self {
        VideoElement {
            asset: asset.into(),
            x,
            y,
            w,
            h,
            fit: Fit::Contain,
            source_range: None,
            speed: 1.0,
            shadow: None,
            clip: None,
            anim: None,
        }
    }

    pub fn fit(mut self, fit: Fit) -> Self {
        self.fit = fit;
        self
    }

    /// Trim the source window in ticks; `from == to` freezes that frame.
    pub fn source_range(mut self, from: i64, to: i64) -> Self {
        self.source_range = Some(SourceRange { from, to });
        self
    }

    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    pub fn shadow(mut self, shadow: ShadowDef) -> Self {
        self.shadow = Some(shadow);
        self
    }

    pub fn clip(mut self, clip: ClipRect) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn anim(mut self, anim: Anim) -> Self {
        self.anim = Some(anim);
        self
    }
}

impl AudioTrack {
    pub fn new(asset: impl Into<String>) -> Self {
        AudioTrack {
            asset: asset.into(),
            gain: 1.0,
            start_at: 0,
            source_range: None,
            fades: Vec::new(),
        }
    }

    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Place the track at this document-timeline tick.
    pub fn start_at(mut self, at: i64) -> Self {
        self.start_at = at;
        self
    }

    /// Trim the source window in ticks.
    pub fn source_range(mut self, from: i64, to: i64) -> Self {
        self.source_range = Some(SourceRange { from, to });
        self
    }

    /// Add a linear fade (in or out) starting at a track-local tick.
    pub fn fade(mut self, kind: FadeKind, at: i64, len: i64) -> Self {
        self.fades.push(FadeDef { at, len, kind });
        self
    }
}

impl TextElement {
    pub fn new(text: impl Into<String>, font: impl Into<String>) -> Self {
        TextElement {
            text: text.into(),
            font: font.into(),
            x: 0,
            y: 0,
            size: 48.0,
            color: Color::from_rgb8(255, 255, 255),
            align: TextAlign::Center,
            shadow: None,
            clip: None,
            anim: None,
        }
    }

    pub fn at(mut self, x: i64, y: i64) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn shadow(mut self, shadow: ShadowDef) -> Self {
        self.shadow = Some(shadow);
        self
    }

    pub fn clip(mut self, clip: ClipRect) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn anim(mut self, anim: Anim) -> Self {
        self.anim = Some(anim);
        self
    }
}

impl RectElement {
    pub fn new(x: i64, y: i64, w: i64, h: i64) -> Self {
        RectElement {
            x,
            y,
            w,
            h,
            color: Color::from_rgb8(255, 255, 255),
            gradient: None,
            radius: None,
            shadow: None,
            clip: None,
            anim: None,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn gradient(mut self, gradient: GradientDef) -> Self {
        self.gradient = Some(gradient);
        self
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = Some(radius);
        self
    }

    pub fn shadow(mut self, shadow: ShadowDef) -> Self {
        self.shadow = Some(shadow);
        self
    }

    pub fn clip(mut self, clip: ClipRect) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn anim(mut self, anim: Anim) -> Self {
        self.anim = Some(anim);
        self
    }
}

impl ImageElement {
    pub fn new(asset: impl Into<String>, x: i64, y: i64, w: i64, h: i64) -> Self {
        ImageElement {
            asset: asset.into(),
            x,
            y,
            w,
            h,
            fit: Fit::Contain,
            shadow: None,
            clip: None,
            anim: None,
        }
    }

    pub fn fit(mut self, fit: Fit) -> Self {
        self.fit = fit;
        self
    }

    pub fn shadow(mut self, shadow: ShadowDef) -> Self {
        self.shadow = Some(shadow);
        self
    }

    pub fn clip(mut self, clip: ClipRect) -> Self {
        self.clip = Some(clip);
        self
    }

    pub fn anim(mut self, anim: Anim) -> Self {
        self.anim = Some(anim);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::canonical_json;
    use crate::document::{Element, Transition, TransitionKind};

    #[test]
    fn builder_output_equals_hand_written_json() {
        let built = DocumentBuilder::new(Size { w: 1920, h: 1080 })
            .fps(30)
            .asset(
                "body",
                Asset::Font {
                    src: "assets/font.ttf".into(),
                    sha256: None,
                },
            )
            .scene(Scene {
                id: "s1".into(),
                duration: FLICKS_PER_SECOND,
                transition_in: None,
                elements: vec![
                    Element::Rect(
                        RectElement::new(0, 0, 1920, 1080).color(Color::from_rgb8(20, 30, 40)),
                    ),
                    Element::Text(
                        TextElement::new("Quillon", "body")
                            .at(960, 540)
                            .size(120.0)
                            .color(Color::from_rgb8(240, 240, 240)),
                    ),
                ],
            })
            .build();

        // r## delimiters: the JSON contains "#rrggbb" colors, whose quote-hash
        // sequence would terminate an r#"..."# raw string early.
        let hand_written = r##"{
            "v": 1,
            "timebase": 705600000,
            "size": { "w": 1920, "h": 1080 },
            "fps": 30,
            "color_space": "srgb",
            "bit_depth": 8,
            "bg": "#000000",
            "assets": {
                "body": { "type": "font", "src": "assets/font.ttf" }
            },
            "scenes": [
                {
                    "id": "s1",
                    "duration": 705600000,
                    "elements": [
                        { "type": "rect", "x": 0, "y": 0, "w": 1920, "h": 1080, "color": "#141e28" },
                        { "type": "text", "text": "Quillon", "font": "body", "x": 960, "y": 540, "size": 120.0, "color": "#f0f0f0", "align": "center" }
                    ]
                }
            ]
        }"##;

        let parsed: Document = serde_json::from_str(hand_written).unwrap();
        assert_eq!(
            canonical_json(&built).unwrap(),
            canonical_json(&parsed).unwrap()
        );
        assert_eq!(built, parsed);
    }

    #[test]
    fn builder_output_equals_hand_written_json_v02_features() {
        let gradient = GradientDef {
            angle: 90.0,
            stops: vec![
                crate::document::GradientStop {
                    at: 0.0,
                    color: Color::from_rgb8(10, 10, 10),
                },
                crate::document::GradientStop {
                    at: 1.0,
                    color: Color::from_rgb8(240, 240, 240),
                },
            ],
        };
        let built = DocumentBuilder::new(Size { w: 640, h: 360 })
            .asset(
                "img",
                Asset::Image {
                    src: "assets/dot.png".into(),
                    sha256: None,
                },
            )
            .scene(Scene {
                id: "s1".into(),
                duration: FLICKS_PER_SECOND,
                transition_in: None,
                elements: vec![Element::Rect(
                    RectElement::new(40, 40, 400, 200)
                        .gradient(gradient)
                        .radius(12.0),
                )],
            })
            .scene(Scene {
                id: "s2".into(),
                duration: FLICKS_PER_SECOND,
                transition_in: Some(Transition {
                    kind: TransitionKind::Crossfade,
                    duration: FLICKS_PER_SECOND / 2,
                }),
                elements: vec![
                    Element::Text(TextElement::new("shadowed", "body").at(320, 180).shadow(
                        ShadowDef {
                            offset_x: 2.0,
                            offset_y: 4.0,
                            blur: 8.0,
                            color: Color::from_rgb8(0, 0, 0),
                            alpha: 0.5,
                        },
                    )),
                    Element::Image(ImageElement::new("img", 10, 10, 50, 50).clip(ClipRect {
                        x: 0,
                        y: 0,
                        w: 25,
                        h: 50,
                    })),
                ],
            })
            .build();

        // r## delimiters: the JSON contains "#rrggbb" colors.
        let hand_written = r##"{
            "v": 1,
            "timebase": 705600000,
            "size": { "w": 640, "h": 360 },
            "fps": 30,
            "color_space": "srgb",
            "bit_depth": 8,
            "bg": "#000000",
            "assets": {
                "img": { "type": "image", "src": "assets/dot.png" }
            },
            "scenes": [
                {
                    "id": "s1",
                    "duration": 705600000,
                    "elements": [
                        {
                            "type": "rect", "x": 40, "y": 40, "w": 400, "h": 200, "color": "#ffffff",
                            "gradient": { "angle": 90.0, "stops": [ { "at": 0.0, "color": "#0a0a0a" }, { "at": 1.0, "color": "#f0f0f0" } ] },
                            "radius": 12.0
                        }
                    ]
                },
                {
                    "id": "s2",
                    "duration": 705600000,
                    "transition_in": { "kind": "crossfade", "duration": 352800000 },
                    "elements": [
                        {
                            "type": "text", "text": "shadowed", "font": "body", "x": 320, "y": 180, "size": 48.0, "color": "#ffffff", "align": "center",
                            "shadow": { "offset_x": 2.0, "offset_y": 4.0, "blur": 8.0, "color": "#000000", "alpha": 0.5 }
                        },
                        {
                            "type": "image", "asset": "img", "x": 10, "y": 10, "w": 50, "h": 50, "fit": "contain",
                            "clip": { "x": 0, "y": 0, "w": 25, "h": 50 }
                        }
                    ]
                }
            ]
        }"##;

        let parsed: Document = serde_json::from_str(hand_written).unwrap();
        assert_eq!(
            canonical_json(&built).unwrap(),
            canonical_json(&parsed).unwrap(),
            "builder and hand-written v0.2 documents must canonicalize identically"
        );
        assert_eq!(built, parsed);
    }

    #[test]
    fn builder_output_equals_hand_written_json_v03_media() {
        // Same discipline as the v0.1 parity test, now covering the v0.3
        // media surface: a trimmed video element and an audio track with a
        // fade (plan §12 v0.3, acceptance v0.1 #4 pattern).
        let built = DocumentBuilder::new(Size { w: 1920, h: 1080 })
            .fps(30)
            .asset(
                "intro",
                Asset::Video {
                    src: "assets/intro.mp4".into(),
                    sha256: None,
                },
            )
            .asset(
                "music",
                Asset::Audio {
                    src: "assets/music.wav".into(),
                    sha256: None,
                },
            )
            .audio_track(
                AudioTrack::new("music")
                    .gain(0.8)
                    .start_at(705_600_000)
                    .fade(FadeKind::In, 0, 88_200_000),
            )
            .scene(Scene {
                id: "s1".into(),
                duration: 705_600_000,
                transition_in: None,
                elements: vec![Element::Video(
                    VideoElement::new("intro", 0, 0, 1920, 1080)
                        .fit(Fit::Cover)
                        .source_range(705_600_000, 1_411_200_000)
                        .speed(2.0),
                )],
            })
            .build();

        let hand_written: Document = serde_json::from_str(
            r##"{
                "v": 1,
                "timebase": 705600000,
                "size": { "w": 1920, "h": 1080 },
                "fps": 30,
                "color_space": "srgb",
                "bit_depth": 8,
                "bg": "#000000",
                "assets": {
                    "intro": { "type": "video", "src": "assets/intro.mp4" },
                    "music": { "type": "audio", "src": "assets/music.wav" }
                },
                "audio_rate": 48000,
                "audio": [
                    { "asset": "music", "gain": 0.8, "start_at": 705600000,
                      "fades": [ { "at": 0, "len": 88200000, "kind": "in" } ] }
                ],
                "scenes": [
                    { "id": "s1", "duration": 705600000,
                      "elements": [
                        { "type": "video", "asset": "intro", "x": 0, "y": 0,
                          "w": 1920, "h": 1080, "fit": "cover",
                          "source_range": { "from": 705600000, "to": 1411200000 },
                          "speed": 2.0 }
                      ] }
                ]
            }"##,
        )
        .expect("hand-written fixture");

        assert_eq!(
            canonical_json(&built).unwrap(),
            canonical_json(&hand_written).unwrap(),
            "builder and hand-written media documents must serialize identically"
        );
    }
}
