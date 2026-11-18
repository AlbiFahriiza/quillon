//! Semantic lint over a valid document (plan §5.2, model: Kineto's
//! `check_document`, ~20 tokens, no pixels). Lint findings are data: they do
//! not make a document invalid, but they are real problems worth failing CI
//! on (the CLI exits 3 unless `--allow-lint`).
//!
//! Text metrics are estimated (0.6 em average advance): core must stay free
//! of font dependencies so it cannot diverge from the pinned-font renderer.

use crate::color::contrast_ratio;
use crate::document::{Document, Element};
use serde::{Deserialize, Serialize};

/// Average glyph advance as a fraction of the font size; DejaVu-class fonts
/// average close to 0.6 em over mixed-case text.
const EM_ADVANCE: f32 = 0.6;
/// Reading speed for the "can a human read this?" check (research/D §2).
const WORDS_PER_MINUTE: f32 = 300.0;
/// A scene shorter than this can never be read, whatever the text.
const MIN_SCENE_SECONDS: f32 = 0.8;
/// WCAG-style threshold: below 3.0, text is hard to read on the background.
const CONTRAST_FLOOR: f32 = 3.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LintFinding {
    pub path: String,
    pub code: String,
    pub severity: String,
    pub message: String,
}

fn finding(path: impl Into<String>, code: &str, message: impl Into<String>) -> LintFinding {
    LintFinding {
        path: path.into(),
        code: code.to_string(),
        severity: "warn".to_string(),
        message: message.into(),
    }
}

/// Estimated pixel width of a single text line at `size`.
fn line_width(line: &str, size: f32) -> f32 {
    line.chars().count() as f32 * size * EM_ADVANCE
}

/// Estimated bounding box of a text block whose center is (x, y).
fn text_bbox(x: i64, y: i64, text: &str, size: f32) -> Bbox {
    let lines: Vec<&str> = text.lines().collect();
    let width = lines
        .iter()
        .map(|l| line_width(l, size))
        .fold(0.0_f32, f32::max);
    let line_height = size * 1.2;
    let height = lines.len() as f32 * line_height;
    let cx = x as f32;
    let cy = y as f32;
    Bbox {
        left: cx - width / 2.0,
        top: cy - height / 2.0,
        right: cx + width / 2.0,
        bottom: cy + height / 2.0,
    }
}

/// Axis-aligned bounding box in canvas pixels.
#[derive(Debug, Clone, Copy)]
struct Bbox {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Bbox {
    fn rect(x: i64, y: i64, w: i64, h: i64) -> Bbox {
        Bbox {
            left: x as f32,
            top: y as f32,
            right: (x + w) as f32,
            bottom: (y + h) as f32,
        }
    }
}

/// Run the four v0.1 lint classes over a validated document.
pub fn lint_document(doc: &Document) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    let canvas = Bbox {
        left: 0.0,
        top: 0.0,
        right: doc.size.w as f32,
        bottom: doc.size.h as f32,
    };
    for (i, scene) in doc.scenes.iter().enumerate() {
        for (j, element) in scene.elements.iter().enumerate() {
            let path = format!("/scenes/{i}/elements/{j}");
            match element {
                Element::Text(t) => {
                    lint_text_contrast(doc, i, j, t, &mut findings);
                    let bbox = text_bbox(t.x, t.y, &t.text, t.size);
                    lint_bbox(&path, bbox, canvas, &mut findings);
                }
                Element::Rect(r) => {
                    lint_bbox(&path, Bbox::rect(r.x, r.y, r.w, r.h), canvas, &mut findings);
                }
                Element::Image(img) => {
                    lint_bbox(
                        &path,
                        Bbox::rect(img.x, img.y, img.w, img.h),
                        canvas,
                        &mut findings,
                    );
                }
            }
        }
        lint_scene_read_time(doc, i, &mut findings);
    }
    findings
}

fn lint_bbox(path: &str, bbox: Bbox, canvas: Bbox, findings: &mut Vec<LintFinding>) {
    // Static position only; animated elements are checked at their rest pose
    // (documented v0.1 limitation).
    if bbox.right <= canvas.left
        || bbox.bottom <= canvas.top
        || bbox.left >= canvas.right
        || bbox.top >= canvas.bottom
    {
        findings.push(finding(
            path,
            "lint/off-canvas",
            "element sits entirely outside the canvas and will never be visible",
        ));
    } else if bbox.left < canvas.left
        || bbox.top < canvas.top
        || bbox.right > canvas.right
        || bbox.bottom > canvas.bottom
    {
        findings.push(finding(
            path,
            "lint/text-overflow",
            "element extends past the canvas edge; part of it will be cut off",
        ));
    }
}

fn lint_text_contrast(
    doc: &Document,
    scene_i: usize,
    element_j: usize,
    text: &crate::document::TextElement,
    findings: &mut Vec<LintFinding>,
) {
    let path = format!("/scenes/{scene_i}/elements/{element_j}");
    let text_linear = text.color.linear_rgb();
    // The background under the text: the document background, or the last
    // rect painted before the text whose box contains the text anchor.
    let scene = &doc.scenes[scene_i];
    let anchor = (text.x as f32, text.y as f32);
    let underlay = scene
        .elements
        .iter()
        .take(element_j)
        .find_map(|el| match el {
            Element::Rect(r)
                if anchor.0 >= r.x as f32
                    && anchor.0 < (r.x + r.w) as f32
                    && anchor.1 >= r.y as f32
                    && anchor.1 < (r.y + r.h) as f32 =>
            {
                Some(r.color.linear_rgb())
            }
            _ => None,
        });
    let bg_linear = underlay.unwrap_or_else(|| doc.bg.linear_rgb());
    let ratio = contrast_ratio(text_linear, bg_linear);
    if ratio < CONTRAST_FLOOR {
        findings.push(finding(
            path,
            "lint/low-contrast",
            format!(
                "text color {} on its background has contrast {ratio:.1}:1 (needs {CONTRAST_FLOOR}:1)",
                text.color
            ),
        ));
    }
}

fn lint_scene_read_time(doc: &Document, i: usize, findings: &mut Vec<LintFinding>) {
    let scene = &doc.scenes[i];
    let words: usize = scene
        .elements
        .iter()
        .map(|el| match el {
            Element::Text(t) => t.text.split_whitespace().count(),
            _ => 0,
        })
        .sum();
    let seconds = doc.duration_seconds(scene.duration);
    let needed = (words as f32 / WORDS_PER_MINUTE * 60.0).max(MIN_SCENE_SECONDS);
    if seconds < needed {
        findings.push(finding(
            format!("/scenes/{i}"),
            "lint/scene-too-short",
            format!(
                "scene shows for {seconds:.2}s but its text needs about {needed:.2}s at {WORDS_PER_MINUTE:.0} wpm",
            ),
        ));
    }
}

impl Document {
    /// Scene duration in fractional seconds (for lint heuristics only; the
    /// format itself never carries float time).
    fn duration_seconds(&self, ticks: i64) -> f32 {
        ticks as f32 / self.timebase as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::DocumentBuilder;
    use crate::color::Color;
    use crate::document::{Element, RectElement, Scene, Size, TextElement};

    fn doc_with(scene: Scene) -> Document {
        DocumentBuilder::new(Size { w: 1920, h: 1080 })
            .scene(scene)
            .build()
    }

    fn text_el(text: &str, x: i64, y: i64, color: &str) -> Element {
        Element::Text(TextElement {
            text: text.into(),
            font: "f".into(),
            x,
            y,
            size: 48.0,
            color: Color::parse(color).unwrap(),
            align: Default::default(),
            shadow: None,
            clip: None,
            anim: None,
        })
    }

    #[test]
    fn white_on_black_passes_contrast() {
        let doc = doc_with(Scene {
            id: "s".into(),
            duration: crate::time::FLICKS_PER_SECOND * 5,
            transition_in: None,
            elements: vec![text_el("hello", 960, 540, "#ffffff")],
        });
        assert!(lint_document(&doc).is_empty(), "{:?}", lint_document(&doc));
    }

    #[test]
    fn low_contrast_is_flagged() {
        let doc = doc_with(Scene {
            id: "s".into(),
            duration: crate::time::FLICKS_PER_SECOND * 5,
            transition_in: None,
            elements: vec![text_el("hello", 960, 540, "#0a0a0a")],
        });
        let findings = lint_document(&doc);
        assert!(
            findings.iter().any(|f| f.code == "lint/low-contrast"),
            "{findings:?}"
        );
    }

    #[test]
    fn contrast_checked_against_underlay_rect() {
        let doc = doc_with(Scene {
            id: "s".into(),
            duration: crate::time::FLICKS_PER_SECOND * 5,
            transition_in: None,
            elements: vec![
                Element::Rect(RectElement {
                    x: 0,
                    y: 0,
                    w: 1920,
                    h: 1080,
                    color: Color::parse("#f0f0f0").unwrap(),
                    gradient: None,
                    radius: None,
                    shadow: None,
                    clip: None,
                    anim: None,
                }),
                // White text over a light rect: fine against doc bg, wrong
                // against the actual underlay.
                text_el("hello", 960, 540, "#ffffff"),
            ],
        });
        let findings = lint_document(&doc);
        assert!(
            findings.iter().any(|f| f.code == "lint/low-contrast"),
            "underlay contrast must be checked, got {findings:?}"
        );
    }

    #[test]
    fn fully_off_canvas_element_is_flagged() {
        let doc = doc_with(Scene {
            id: "s".into(),
            duration: crate::time::FLICKS_PER_SECOND,
            transition_in: None,
            elements: vec![Element::Rect(RectElement {
                x: 5000,
                y: 5000,
                w: 100,
                h: 100,
                color: Color::from_rgb8(255, 255, 255),
                gradient: None,
                radius: None,
                shadow: None,
                clip: None,
                anim: None,
            })],
        });
        let findings = lint_document(&doc);
        assert!(
            findings.iter().any(|f| f.code == "lint/off-canvas"),
            "{findings:?}"
        );
    }

    #[test]
    fn partially_overflowing_text_is_flagged() {
        // 20 chars * 48px * 0.6 = 576px wide, centered at x=50: overflows left.
        let doc = doc_with(Scene {
            id: "s".into(),
            duration: crate::time::FLICKS_PER_SECOND,
            transition_in: None,
            elements: vec![text_el("01234567890123456789", 50, 540, "#ffffff")],
        });
        let findings = lint_document(&doc);
        assert!(
            findings.iter().any(|f| f.code == "lint/text-overflow"),
            "{findings:?}"
        );
    }

    #[test]
    fn unreadable_scene_is_flagged() {
        // ~30 words need ~6s at 300 wpm; scene gives 1s.
        let long_text = "word ".repeat(30);
        let doc = doc_with(Scene {
            id: "s".into(),
            duration: crate::time::FLICKS_PER_SECOND,
            transition_in: None,
            elements: vec![text_el(long_text.trim(), 960, 540, "#ffffff")],
        });
        let findings = lint_document(&doc);
        assert!(
            findings.iter().any(|f| f.code == "lint/scene-too-short"),
            "{findings:?}"
        );
    }
}
