//! Rust builder API (plan §4 F9, acceptance v0.1 #4): a builder-produced
//! document serializes to exactly the same canonical JSON as a hand-written
//! document. The builder does not validate; run the document through
//! [`crate::validate_document`] before rendering.

use crate::color::Color;
use crate::document::{
    Anim, Asset, ColorSpace, Document, Fit, ImageElement, RectElement, Scene, Size, TextAlign,
    TextElement,
};
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

    pub fn build(self) -> Document {
        self.doc
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
            anim: None,
        }
    }

    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
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
            anim: None,
        }
    }

    pub fn fit(mut self, fit: Fit) -> Self {
        self.fit = fit;
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
    use crate::document::Element;

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
}
