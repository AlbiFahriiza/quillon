//! Draft scaling (plan §12 v0.2, research/D §4): a half-resolution copy of
//! a document for the preview filmstrip. Geometry halves; the timeline and
//! all time values (durations, keyframe `t`, crossfades) stay untouched, so
//! a draft renders the same motion at a quarter of the pixel cost. Draft
//! frames are for preview only — they are never golden-hashed.

use crate::document::{
    Anim, ClipRect, Document, Element, Key, RectElement, Scene, ShadowDef, Size, TextElement, Track,
};

/// Scale `doc` by 1/`divisor` (rounded, floor of half-px at 2). The canvas
/// never shrinks below the format's minimum dimension.
pub fn scale_document(doc: &Document, divisor: u32) -> Document {
    assert!(divisor >= 1, "draft divisor must be >= 1");
    let mut out = doc.clone();
    if divisor == 1 {
        return out;
    }
    let f = 1.0 / f64::from(divisor);
    out.size = Size {
        w: ((f64::from(doc.size.w) * f).round() as u32).max(crate::MIN_DIMENSION),
        h: ((f64::from(doc.size.h) * f).round() as u32).max(crate::MIN_DIMENSION),
    };
    out.scenes = doc
        .scenes
        .iter()
        .map(|scene| Scene {
            elements: scene
                .elements
                .iter()
                .map(|el| scale_element(el, f))
                .collect(),
            ..scene.clone()
        })
        .collect();
    out
}

fn scale_element(el: &Element, f: f64) -> Element {
    match el {
        Element::Rect(r) => Element::Rect(RectElement {
            x: ((f64::from(r.x as i32)) * f).round() as i64,
            y: ((f64::from(r.y as i32)) * f).round() as i64,
            w: ((f64::from(r.w as i32)) * f).round().max(1.0) as i64,
            h: ((f64::from(r.h as i32)) * f).round().max(1.0) as i64,
            radius: r.radius.map(|r| (f64::from(r) * f) as f32),
            gradient: r.gradient.clone(),
            shadow: r.shadow.as_ref().map(|s| scale_shadow(s, f)),
            clip: r.clip.map(|c| scale_clip(c, f)),
            anim: r.anim.as_ref().map(|a| scale_anim(a, f)),
            ..r.clone()
        }),
        Element::Text(t) => Element::Text(TextElement {
            x: ((f64::from(t.x as i32)) * f).round() as i64,
            y: ((f64::from(t.y as i32)) * f).round() as i64,
            size: ((f64::from(t.size as i32)) * f) as f32,
            shadow: t.shadow.as_ref().map(|s| scale_shadow(s, f)),
            clip: t.clip.map(|c| scale_clip(c, f)),
            anim: t.anim.as_ref().map(|a| scale_anim(a, f)),
            ..t.clone()
        }),
        Element::Image(i) => Element::Image(crate::document::ImageElement {
            x: ((f64::from(i.x as i32)) * f).round() as i64,
            y: ((f64::from(i.y as i32)) * f).round() as i64,
            w: ((f64::from(i.w as i32)) * f).round().max(1.0) as i64,
            h: ((f64::from(i.h as i32)) * f).round().max(1.0) as i64,
            shadow: i.shadow.as_ref().map(|s| scale_shadow(s, f)),
            clip: i.clip.map(|c| scale_clip(c, f)),
            anim: i.anim.as_ref().map(|a| scale_anim(a, f)),
            ..i.clone()
        }),
        Element::Video(v) => Element::Video(crate::document::VideoElement {
            x: ((f64::from(v.x as i32)) * f).round() as i64,
            y: ((f64::from(v.y as i32)) * f).round() as i64,
            w: ((f64::from(v.w as i32)) * f).round().max(1.0) as i64,
            h: ((f64::from(v.h as i32)) * f).round().max(1.0) as i64,
            shadow: v.shadow.as_ref().map(|s| scale_shadow(s, f)),
            clip: v.clip.map(|c| scale_clip(c, f)),
            anim: v.anim.as_ref().map(|a| scale_anim(a, f)),
            ..v.clone()
        }),
    }
}

/// Translation scales with the canvas; rotation, scale factors and opacity
/// are dimensionless and stay; key times are the same timeline.
fn scale_anim(anim: &Anim, f: f64) -> Anim {
    let scale_track = |t: &Option<Track>, spatial: bool| {
        t.as_ref().map(|track| Track {
            keys: track
                .keys
                .iter()
                .map(|k| Key {
                    v: if spatial {
                        (f64::from(k.v) * f) as f32
                    } else {
                        k.v
                    },
                    ..*k
                })
                .collect(),
        })
    };
    Anim {
        opacity: scale_track(&anim.opacity, false),
        translate_x: scale_track(&anim.translate_x, true),
        translate_y: scale_track(&anim.translate_y, true),
        scale: scale_track(&anim.scale, false),
        rotate: scale_track(&anim.rotate, false),
    }
}

fn scale_shadow(s: &ShadowDef, f: f64) -> ShadowDef {
    ShadowDef {
        offset_x: (f64::from(s.offset_x) * f) as f32,
        offset_y: (f64::from(s.offset_y) * f) as f32,
        blur: (f64::from(s.blur) * f) as f32,
        ..s.clone()
    }
}

fn scale_clip(c: ClipRect, f: f64) -> ClipRect {
    ClipRect {
        x: ((f64::from(c.x as i32)) * f).round() as i64,
        y: ((f64::from(c.y as i32)) * f).round() as i64,
        w: ((f64::from(c.w as i32)) * f).round().max(1.0) as i64,
        h: ((f64::from(c.h as i32)) * f).round().max(1.0) as i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builder::DocumentBuilder;
    use crate::color::Color;
    use crate::document::{GradientDef, GradientStop};
    use crate::time::FLICKS_PER_SECOND;

    fn doc() -> Document {
        DocumentBuilder::new(Size { w: 1920, h: 1080 })
            .asset(
                "body",
                crate::document::Asset::Font {
                    src: "assets/font.ttf".into(),
                    sha256: None,
                },
            )
            .scene(Scene {
                id: "s".into(),
                duration: 2 * FLICKS_PER_SECOND,
                transition_in: None,
                elements: vec![
                    Element::Rect(
                        RectElement::new(100, 200, 400, 300)
                            .color(Color::from_rgb8(255, 0, 0))
                            .radius(24.0)
                            .gradient(GradientDef {
                                angle: 90.0,
                                stops: vec![
                                    GradientStop {
                                        at: 0.0,
                                        color: Color::from_rgb8(0, 0, 0),
                                    },
                                    GradientStop {
                                        at: 1.0,
                                        color: Color::from_rgb8(255, 255, 255),
                                    },
                                ],
                            })
                            .shadow(ShadowDef {
                                offset_x: 8.0,
                                offset_y: 8.0,
                                blur: 16.0,
                                color: Color::from_rgb8(0, 0, 0),
                                alpha: 0.5,
                            }),
                    ),
                    Element::Text(TextElement::new("draft", "body").at(960, 540).size(96.0)),
                ],
            })
            .build()
    }

    #[test]
    fn halves_geometry_and_keeps_timeline() {
        let scaled = scale_document(&doc(), 2);
        assert_eq!(scaled.size, Size { w: 960, h: 540 });
        let scene = &scaled.scenes[0];
        assert_eq!(scene.duration, 2 * FLICKS_PER_SECOND, "timeline untouched");
        let Element::Rect(rect) = &scene.elements[0] else {
            panic!("first element must stay a rect");
        };
        assert_eq!((rect.x, rect.y, rect.w, rect.h), (50, 100, 200, 150));
        assert_eq!(rect.radius, Some(12.0));
        assert_eq!(rect.shadow.as_ref().unwrap().blur, 8.0);
        // Gradient geometry (angle, stop positions) is resolution-free.
        let gradient = rect.gradient.as_ref().unwrap();
        assert_eq!(gradient.angle, 90.0);
        assert_eq!(gradient.stops[1].at, 1.0);
        let Element::Text(text) = &scene.elements[1] else {
            panic!("second element must stay text");
        };
        assert_eq!(text.size, 48.0);
        assert_eq!((text.x, text.y), (480, 270));
    }

    #[test]
    fn scaling_anim_keeps_time_and_dimensionless_values() {
        let mut d = doc();
        d.scenes[0].elements[0] = Element::Rect(RectElement::new(0, 0, 100, 100).anim(Anim {
            translate_x: Some(Track {
                keys: vec![
                    Key {
                        t: 0,
                        v: 100.0,
                        ease: None,
                    },
                    Key {
                        t: FLICKS_PER_SECOND,
                        v: 200.0,
                        ease: None,
                    },
                ],
            }),
            rotate: Some(Track {
                keys: vec![
                    Key {
                        t: 0,
                        v: 45.0,
                        ease: None,
                    },
                    Key {
                        t: FLICKS_PER_SECOND,
                        v: 90.0,
                        ease: None,
                    },
                ],
            }),
            ..Default::default()
        }));
        let scaled = scale_document(&d, 2);
        let Element::Rect(rect) = &scaled.scenes[0].elements[0] else {
            panic!("must stay a rect");
        };
        let anim = rect.anim.as_ref().unwrap();
        let tx = anim.translate_x.as_ref().unwrap();
        assert_eq!(tx.keys[0].v, 50.0, "translation halves");
        assert_eq!(tx.keys[0].t, 0, "key times are the same timeline");
        assert_eq!(
            tx.keys[1].t, FLICKS_PER_SECOND,
            "key times are the same timeline"
        );
        let rotate = anim.rotate.as_ref().unwrap();
        assert_eq!(rotate.keys[0].v, 45.0, "rotation is dimensionless");
    }

    #[test]
    fn divisor_one_is_identity_and_small_canvases_clamp() {
        let d = doc();
        assert_eq!(scale_document(&d, 1), d);
        let mut tiny = d;
        tiny.size = Size { w: 32, h: 32 };
        let scaled = scale_document(&tiny, 2);
        assert_eq!(
            scaled.size,
            Size { w: 16, h: 16 },
            "halves down to the format minimum"
        );
    }
}
