//! Canonical JSON: the same semantic document always serializes to the same
//! bytes (sorted keys via BTreeMap-backed `serde_json::Value`, compact form),
//! so git diffs and agent edit-verification are meaningful (plan §5.2).

use crate::document::Document;

/// Serialize a document to its canonical JSON form.
///
/// Infallible in practice: the document type has no non-string map keys and
/// no unserializable values, but the error is still propagated rather than
/// swallowed.
pub fn canonical_json(doc: &Document) -> Result<String, serde_json::Error> {
    let value = serde_json::to_value(doc)?;
    serde_json::to_string(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::DocumentBuilder;
    use crate::document::{Asset, Element, RectElement, Scene, Size};

    fn sample() -> Document {
        DocumentBuilder::new(Size { w: 1920, h: 1080 })
            .fps(30)
            .asset(
                "bg_rect",
                Asset::Image {
                    src: "assets/bg.png".into(),
                    sha256: None,
                },
            )
            .scene(Scene {
                id: "s1".into(),
                duration: 705_600_000,
                elements: vec![Element::Rect(RectElement {
                    x: 100,
                    y: 100,
                    w: 200,
                    h: 80,
                    color: crate::color::Color::from_rgb8(255, 0, 0),
                    anim: None,
                })],
            })
            .build()
    }

    #[test]
    fn canonical_form_is_idempotent() {
        let doc = sample();
        let once = canonical_json(&doc).unwrap();
        // Re-parse and re-canonicalize: identical bytes.
        let reparsed: Document = serde_json::from_str(&once).unwrap();
        let twice = canonical_json(&reparsed).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_form_has_no_whitespace() {
        let doc = sample();
        let json = canonical_json(&doc).unwrap();
        // The sample's string values contain no spaces, so compact form means
        // no whitespace at all between any two characters.
        assert_eq!(json.split_whitespace().collect::<String>(), json);
    }
}
