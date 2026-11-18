//! Crossfade acceptance tests (myquillonplan.md §12 v0.2): the blend must
//! happen in linear light — at the midpoint of a white-to-black crossfade
//! the pixel is linear 0.5, which encodes to sRGB ~188, not 128 (research/T).

use quillon_core::builder::DocumentBuilder;
use quillon_core::color::Color;
use quillon_core::document::{
    ClipRect, Element, RectElement, Scene, ShadowDef, Size, Transition, TransitionKind,
};
use quillon_core::frame_source::NullFrameSource;
use quillon_core::time::FLICKS_PER_SECOND;
use quillon_render::Renderer;
use std::path::Path;

fn white_rect_scene(id: &str) -> Scene {
    Scene {
        id: id.into(),
        duration: FLICKS_PER_SECOND,
        transition_in: None,
        elements: vec![Element::Rect(RectElement::new(0, 0, 64, 64))],
    }
}

fn empty_scene(id: &str, transition_in: Option<Transition>) -> Scene {
    Scene {
        id: id.into(),
        duration: FLICKS_PER_SECOND,
        transition_in,
        elements: vec![],
    }
}

/// Like [`empty_scene`] but 2s long, so a 1s crossfade ends inside it.
fn long_empty_scene(id: &str, transition_in: Option<Transition>) -> Scene {
    Scene {
        duration: 2 * FLICKS_PER_SECOND,
        ..empty_scene(id, transition_in)
    }
}

fn crossfade_doc() -> quillon_core::Document {
    DocumentBuilder::new(Size { w: 64, h: 64 })
        .bg(Color::from_rgb8(0, 0, 0))
        .scene(white_rect_scene("white"))
        .scene(empty_scene(
            "black",
            Some(Transition {
                kind: TransitionKind::Crossfade,
                duration: FLICKS_PER_SECOND,
            }),
        ))
        .build()
}

fn frame_at(doc: &quillon_core::Document, tick: i64) -> quillon_core::Frame {
    let renderer = Renderer::new(doc, Path::new(".")).expect("assets prepare");
    renderer
        .render_tick(tick, &NullFrameSource)
        .expect("tick renders")
}

fn center_pixel(frame: &quillon_core::Frame) -> [u8; 4] {
    let i = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    [
        frame.rgba[i],
        frame.rgba[i + 1],
        frame.rgba[i + 2],
        frame.rgba[i + 3],
    ]
}

#[test]
fn crossfade_midpoint_is_linear_light() {
    let doc = crossfade_doc();
    // The second scene starts at 1s and crossfades for 1s; its midpoint is
    // doc tick 1.5s.
    let frame = frame_at(&doc, FLICKS_PER_SECOND + FLICKS_PER_SECOND / 2);
    let px = center_pixel(&frame);
    // Linear 0.5 encodes to sRGB 188; blending in gamma space would give 128.
    assert!(
        (i16::from(px[0]) - 188).abs() <= 1,
        "crossfade must blend in linear light, got r={}",
        px[0]
    );
    assert_eq!(px[0], px[1]);
    assert_eq!(px[1], px[2]);
    assert_eq!(px[3], 255);
}

#[test]
fn crossfade_start_shows_outgoing_scene() {
    let doc = crossfade_doc();
    // The first frame of the crossfading scene is (almost) pure outgoing.
    let frame = frame_at(&doc, FLICKS_PER_SECOND);
    let px = center_pixel(&frame);
    assert!(
        px[0] > 250,
        "expected ~white at crossfade start, got {}",
        px[0]
    );
}

#[test]
fn after_crossfade_only_incoming_scene_renders() {
    let doc = DocumentBuilder::new(Size { w: 64, h: 64 })
        .bg(Color::from_rgb8(0, 0, 0))
        .scene(white_rect_scene("white"))
        .scene(long_empty_scene(
            "black",
            Some(Transition {
                kind: TransitionKind::Crossfade,
                duration: FLICKS_PER_SECOND,
            }),
        ))
        .build();
    // Past the 1s crossfade window (1.1s into the second scene) the frame is
    // the incoming scene alone.
    let frame = frame_at(&doc, FLICKS_PER_SECOND + 11 * FLICKS_PER_SECOND / 10);
    let px = center_pixel(&frame);
    assert_eq!(px[0], 0, "expected black past the crossfade, got {}", px[0]);
}

#[test]
fn cut_transition_is_a_hard_switch() {
    let doc = DocumentBuilder::new(Size { w: 64, h: 64 })
        .bg(Color::from_rgb8(0, 0, 0))
        .scene(white_rect_scene("white"))
        .scene(empty_scene(
            "black",
            Some(Transition {
                kind: TransitionKind::Cut,
                duration: 0,
            }),
        ))
        .build();
    // First frame of the second scene: no blend, straight to black.
    let frame = frame_at(&doc, FLICKS_PER_SECOND);
    let px = center_pixel(&frame);
    assert_eq!(px[0], 0, "a cut must not blend, got {}", px[0]);
}

#[test]
fn crossfade_holds_outgoing_animation() {
    // The outgoing scene keeps "playing" during the crossfade: an element
    // animating opacity to 0 holds its end value, so the crossfade blends
    // against that held frame, not against the scene start.
    use quillon_core::document::{Anim, Key, Track};
    let mut scene = white_rect_scene("fade");
    scene.elements[0] = Element::Rect(RectElement::new(0, 0, 64, 64).anim(Anim {
        opacity: Some(Track {
            keys: vec![
                Key {
                    t: 0,
                    v: 1.0,
                    ease: None,
                },
                Key {
                    t: FLICKS_PER_SECOND / 2,
                    v: 0.0,
                    ease: None,
                },
            ],
        }),
        ..Default::default()
    }));
    let doc = DocumentBuilder::new(Size { w: 64, h: 64 })
        .bg(Color::from_rgb8(0, 0, 0))
        .scene(scene)
        .scene(empty_scene(
            "black",
            Some(Transition {
                kind: TransitionKind::Crossfade,
                duration: FLICKS_PER_SECOND,
            }),
        ))
        .build();
    // Halfway into the crossfade the outgoing scene has faded its rect to
    // nothing, so the frame is pure black regardless of the blend factor.
    let frame = frame_at(&doc, FLICKS_PER_SECOND + FLICKS_PER_SECOND / 2);
    let px = center_pixel(&frame);
    assert!(px[0] <= 2, "held end value must be black, got {}", px[0]);
}

#[test]
fn gradient_rect_render_is_smooth() {
    use quillon_core::document::{GradientDef, GradientStop};
    let doc = DocumentBuilder::new(Size { w: 64, h: 8 })
        .bg(Color::from_rgb8(0, 0, 0))
        .scene(Scene {
            id: "g".into(),
            duration: FLICKS_PER_SECOND,
            transition_in: None,
            elements: vec![Element::Rect(RectElement::new(0, 0, 64, 8).gradient(
                GradientDef {
                    angle: 0.0,
                    stops: vec![
                        GradientStop {
                            at: 0.0,
                            color: Color::from_rgb8(255, 255, 255),
                        },
                        GradientStop {
                            at: 1.0,
                            color: Color::from_rgb8(0, 0, 0),
                        },
                    ],
                },
            ))],
        })
        .build();
    let frame = frame_at(&doc, 0);
    let left = frame.rgba[0];
    let mid = frame.rgba[(4 * 32) as usize];
    let right = frame.rgba[(4 * 63) as usize];
    assert!(left > 250, "gradient starts ~white, got {left}");
    assert!(
        (i16::from(mid) - 188).abs() <= 2,
        "midpoint ~188, got {mid}"
    );
    assert!(right <= 2, "gradient ends black, got {right}");
}

#[test]
fn shadow_renders_under_element_and_clip_hides() {
    let doc = DocumentBuilder::new(Size { w: 64, h: 64 })
        .bg(Color::from_rgb8(0, 0, 0))
        .scene(Scene {
            id: "s".into(),
            duration: FLICKS_PER_SECOND,
            transition_in: None,
            elements: vec![Element::Rect(
                RectElement::new(8, 8, 32, 32)
                    .color(Color::from_rgb8(255, 255, 255))
                    .shadow(ShadowDef {
                        offset_x: 6.0,
                        offset_y: 6.0,
                        blur: 4.0,
                        color: Color::from_rgb8(255, 255, 255),
                        alpha: 1.0,
                    })
                    .clip(ClipRect {
                        x: 0,
                        y: 0,
                        w: 16,
                        h: 48,
                    }),
            )],
        })
        .build();
    let frame = frame_at(&doc, 0);
    // Element at canvas [8,40)x[8,40); clip at element-local x in [0,16)
    // means canvas x in [8,24).
    // Inside the clip: white element on black bg.
    assert_eq!(frame.rgba[(12 * 64 + 12) * 4], 255);
    // The element itself is clipped away past canvas x=24.
    assert_eq!(frame.rgba[(30 * 64 + 30) * 4], 0, "clip hides the element");
    // Shadow (offset +6, blur 4) of the clipped element: at y=42 the element
    // is gone (element ends at y=40) but the shadow reaches y in [8,46);
    // x=20 is inside the clip, so the shadow shows there.
    assert!(
        frame.rgba[(42 * 64 + 20) * 4] > 0,
        "shadow shows inside the clip past the element"
    );
    // Same shadow, beyond the clip edge: masked.
    assert_eq!(
        frame.rgba[(42 * 64 + 28) * 4],
        0,
        "shadow is clipped like the element"
    );
}
