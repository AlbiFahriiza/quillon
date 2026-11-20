//! Parallel pipeline parity (plan §4 F7, research/R): the chunked parallel
//! renderer must produce byte-identical frames, in frame order, to the
//! single-threaded `Renderer`.

use quillon_core::builder::DocumentBuilder;
use quillon_core::color::Color;
use quillon_core::document::{
    Anim, Element, Key, RectElement, Scene, Size, TextElement, Track, Transition, TransitionKind,
};
use quillon_core::frame_source::NullFrameSource;
use quillon_core::time::FLICKS_PER_SECOND;
use quillon_render::{Renderer, render_document_parallel};
use std::path::Path;

fn doc() -> quillon_core::Document {
    let anim = Anim {
        opacity: Some(Track {
            keys: vec![
                Key {
                    t: 0,
                    v: 0.0,
                    ease: None,
                },
                Key {
                    t: FLICKS_PER_SECOND / 2,
                    v: 1.0,
                    ease: Some(quillon_core::easing::Ease::OutCubic),
                },
            ],
        }),
        ..Default::default()
    };
    DocumentBuilder::new(Size { w: 96, h: 54 })
        .asset(
            "body",
            quillon_core::document::Asset::Font {
                src: "assets/font.ttf".into(),
                sha256: None,
            },
        )
        .scene(Scene {
            id: "one".into(),
            duration: FLICKS_PER_SECOND,
            transition_in: None,
            elements: vec![
                Element::Rect(
                    RectElement::new(10, 10, 40, 30)
                        .color(Color::from_rgb8(30, 90, 200))
                        .anim(anim.clone()),
                ),
                Element::Text(TextElement::new("parity", "body").at(48, 27).size(16.0)),
            ],
        })
        .scene(Scene {
            id: "two".into(),
            duration: FLICKS_PER_SECOND,
            transition_in: Some(Transition {
                kind: TransitionKind::Crossfade,
                duration: FLICKS_PER_SECOND / 2,
            }),
            elements: vec![Element::Rect(
                RectElement::new(20, 5, 60, 44).color(Color::from_rgb8(200, 90, 30)),
            )],
        })
        .build()
}

#[test]
fn parallel_pipeline_matches_sequential_frames_in_order() {
    let base = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/corpus"));
    let doc = doc();
    let renderer = Renderer::new(&doc, base).expect("assets prepare");

    let mut sequential = Vec::new();
    for i in 0..doc.frame_count() {
        let tick = i as i64 * (FLICKS_PER_SECOND / 30);
        sequential.push(
            renderer
                .render_tick(tick, &NullFrameSource)
                .expect("sequential render"),
        );
    }

    let rx = render_document_parallel(
        &doc,
        base,
        4,
        std::sync::Arc::new(quillon_core::NullFrameSource),
        None,
        None,
    )
    .expect("pipeline starts");
    let parallel: Vec<_> = rx
        .into_iter()
        .map(|job| job.expect("parallel render"))
        .collect();

    assert_eq!(parallel.len(), sequential.len(), "frame counts match");
    for (i, (job, frame)) in parallel.iter().zip(sequential.iter()).enumerate() {
        assert_eq!(job.index, i as u64, "frames must arrive in order");
        assert_eq!(job.frame.width, frame.width);
        assert_eq!(job.frame.height, frame.height);
        assert_eq!(
            job.frame.rgba, frame.rgba,
            "frame {i} must be byte-identical to the sequential render"
        );
    }
}

#[test]
fn r3_zero_fps_is_a_named_error_not_a_divide_by_zero() {
    // R3-F3: frame_tick(fps) divides by fps, and the pipeline called it
    // before its own `frame <= 0` guard — a Document built through the
    // library (no validator) with fps 0 panicked. Watched failing first.
    let base = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/corpus"));
    let mut doc = doc();
    doc.fps = 0;
    let result = render_document_parallel(
        &doc,
        base,
        1,
        std::sync::Arc::new(quillon_core::NullFrameSource),
        None,
        None,
    );
    match result {
        Err(err) => assert!(err.to_string().contains("fps"), "names the problem: {err}"),
        // A pipeline that started would hang the test on recv; any
        // success is the bug it is.
        Ok(_) => panic!("fps 0 must not start a pipeline"),
    }
}
