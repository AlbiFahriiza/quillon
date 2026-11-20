//! Boundary tests for the validation pipeline (myquillonplan.md §9.5,
//! acceptance v0.1 #1). Fixtures live in `tests/fixtures/assets`.

use quillon_core::validate_str;
use std::path::Path;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

/// A minimal valid document: one text element on a font asset, one image.
fn valid_json() -> String {
    r##"{
            "v": 1,
            "size": { "w": 320, "h": 240 },
            "assets": {
                "f": { "type": "font", "src": "assets/font.ttf" },
                "img": { "type": "image", "src": "assets/dot.png",
                          "sha256": "cbb729b15b71376cbac4faa4db68f0093cc43da48cbc246f4ece869874561d7b" }
            },
            "scenes": [
                {
                    "id": "s1",
                    "duration": 705600000,
                    "elements": [
                        { "type": "text", "text": "Quillon", "font": "f", "x": 160, "y": 120, "size": 24.0 },
                        { "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }
                    ]
                }
            ]
        }"##
        .to_string()
}

fn errors_for(json: &str) -> Vec<quillon_core::ValidationError> {
    match validate_str(json, Path::new(FIXTURES)) {
        Ok(_) => panic!("expected validation errors, got Ok"),
        Err(errors) => errors,
    }
}

fn first_code(json: &str) -> String {
    let errors = errors_for(json);
    assert!(!errors.is_empty(), "expected at least one error");
    errors[0].code.clone()
}

#[test]
fn valid_document_passes() {
    let doc = validate_str(&valid_json(), Path::new(FIXTURES))
        .unwrap_or_else(|e| panic!("valid document rejected: {e:?}"));
    assert_eq!(doc.scenes.len(), 1);
    assert_eq!(doc.fps, 30, "fps default");
    assert_eq!(doc.timebase, 705_600_000, "timebase default");
}

#[test]
fn syntax_error_is_reported_as_syntax() {
    let code = first_code("{ not json");
    assert_eq!(code, "syntax");
}

#[test]
fn missing_required_field_names_the_field() {
    // rect without "h": serde names the field, jsonschema points at the element.
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r#"{ "type": "rect", "x": 1, "y": 2, "w": 10 }"#,
    );
    let errors = errors_for(&json);
    let text = format!("{errors:?}");
    assert!(
        text.contains("`h`"),
        "serde must name the missing field: {text}"
    );
}

#[test]
fn missing_field_error_carries_a_json_pointer() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r#"{ "type": "rect", "x": 1, "y": 2, "w": 10 }"#,
    );
    let errors = errors_for(&json);
    let with_pointer = errors.iter().any(|e| e.path == "/scenes/0/elements/1");
    assert!(
        with_pointer,
        "at least one error must point at the broken element, got {errors:?}"
    );
}

#[test]
fn unknown_element_type_is_translated_not_raw_oneof() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r#"{ "type": "circle", "x": 1, "y": 2, "r": 5 }"#,
    );
    let errors = errors_for(&json);
    let joined = format!("{errors:?}");
    assert!(
        !joined.contains("not valid under any of the schemas"),
        "raw oneOf error must not leak: {joined}"
    );
    assert!(
        joined.contains("circle"),
        "translation must name the unknown type: {joined}"
    );
}

#[test]
fn unknown_field_inside_element_is_rejected() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r##"{ "type": "rect", "x": 1, "y": 2, "w": 10, "h": 10, "clor": "#ffffff" }"##,
    );
    let errors = errors_for(&json);
    let joined = format!("{errors:?}");
    assert!(
        joined.contains("clor") || joined.contains("unknown"),
        "typo fields must be rejected, got {joined}"
    );
}

#[test]
fn version_must_be_one() {
    assert_eq!(
        first_code(&valid_json().replace(r#""v": 1"#, r#""v": 2"#)),
        "version"
    );
}

#[test]
fn wrong_timebase_is_rejected() {
    let json = valid_json().replacen("{\n", "{\n\"timebase\": 100,\n", 1);
    assert_eq!(first_code(&json), "timebase");
}

#[test]
fn fps_must_divide_timebase() {
    assert_eq!(
        first_code(&valid_json().replace(r#""size": { "w""#, r#""fps": 29, "size": { "w""#)),
        "fps"
    );
}

#[test]
fn fps_out_of_range_is_rejected() {
    assert_eq!(
        first_code(&valid_json().replace(r#""size": { "w""#, r#""fps": 500, "size": { "w""#)),
        "fps"
    );
}

#[test]
fn canvas_too_small_is_rejected() {
    let json = valid_json().replace(r#"{ "w": 320, "h": 240 }"#, r#"{ "w": 8, "h": 240 }"#);
    assert_eq!(first_code(&json), "size");
}

#[test]
fn bit_depth_other_than_8_is_rejected() {
    assert_eq!(
        first_code(&valid_json().replace(r#""size": { "w""#, r#""bit_depth": 10, "size": { "w""#)),
        "bit_depth"
    );
}

#[test]
fn malformed_background_color_is_rejected() {
    assert_eq!(
        first_code(&valid_json().replace(r#""size": { "w""#, r##""bg": "#00", "size": { "w""##)),
        "color"
    );
}

#[test]
fn zero_scene_duration_is_rejected() {
    let json = valid_json().replace(r#""duration": 705600000"#, r#""duration": 0"#);
    let errors = errors_for(&json);
    assert!(
        errors.iter().any(|e| e.code == "scene-duration"),
        "{errors:?}"
    );
}

#[test]
fn duplicate_scene_ids_are_rejected() {
    let json = valid_json().replace(r#""id": "s1""#, r#""id": "s1", "extra": 1"#);
    // Add a second scene with the same id by cloning the scenes array.
    let json = json.replace(r#""extra": 1"#, r#""duration": 705600000, "elements": []"#);
    let with_dup = json.replace(
        "]\n        }",
        ", { \"id\": \"s1\", \"duration\": 705600000, \"elements\": [] } ] }",
    );
    let errors = errors_for(&with_dup);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "scene-id" && e.path.starts_with("/scenes/")),
        "{errors:?}"
    );
}

#[test]
fn empty_text_is_rejected() {
    let json = valid_json().replace(r#""text": "Quillon""#, r#""text": """#);
    assert_eq!(first_code(&json), "text");
}

#[test]
fn zero_font_size_is_rejected() {
    let json = valid_json().replace(r#""size": 24.0"#, r#""size": 0.0"#);
    assert_eq!(first_code(&json), "text-size");
}

#[test]
fn unknown_font_asset_is_rejected() {
    let json = valid_json().replace(r#""font": "f""#, r#""font": "nope""#);
    assert_eq!(first_code(&json), "font-asset");
}

#[test]
fn font_reference_to_image_asset_is_rejected() {
    let json = valid_json().replace(r#""font": "f""#, r#""font": "img""#);
    assert_eq!(first_code(&json), "asset-type");
}

#[test]
fn rect_with_zero_width_is_rejected() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r#"{ "type": "rect", "x": 1, "y": 2, "w": 0, "h": 10 }"#,
    );
    assert_eq!(first_code(&json), "rect-size");
}

#[test]
fn track_with_single_key_is_rejected() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r##"{ "type": "rect", "x": 1, "y": 2, "w": 10, "h": 10,
             "anim": { "opacity": { "keys": [ { "t": 0, "v": 1.0 } ] } } }"##,
    );
    assert_eq!(first_code(&json), "anim-keys");
}

#[test]
fn non_increasing_key_times_are_rejected() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r##"{ "type": "rect", "x": 1, "y": 2, "w": 10, "h": 10,
             "anim": { "opacity": { "keys": [ { "t": 100, "v": 1.0 }, { "t": 50, "v": 0.5 } ] } } }"##,
    );
    assert_eq!(first_code(&json), "anim-keys");
}

#[test]
fn keyframe_time_past_scene_end_is_rejected() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r##"{ "type": "rect", "x": 1, "y": 2, "w": 10, "h": 10,
             "anim": { "opacity": { "keys": [ { "t": 0, "v": 1.0 }, { "t": 999999999, "v": 0.5 } ] } } }"##,
    );
    assert_eq!(first_code(&json), "anim-t");
}

#[test]
fn opacity_above_one_is_rejected() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r##"{ "type": "rect", "x": 1, "y": 2, "w": 10, "h": 10,
             "anim": { "opacity": { "keys": [ { "t": 0, "v": 1.5 }, { "t": 100, "v": 0.5 } ] } } }"##,
    );
    assert_eq!(first_code(&json), "opacity-range");
}

#[test]
fn negative_keyframe_time_is_rejected() {
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r##"{ "type": "rect", "x": 1, "y": 2, "w": 10, "h": 10,
             "anim": { "opacity": { "keys": [ { "t": -1, "v": 1.0 }, { "t": 100, "v": 0.5 } ] } } }"##,
    );
    assert_eq!(first_code(&json), "anim-t");
}

#[test]
fn parent_dir_asset_path_is_rejected() {
    let json = valid_json().replace(r#""src": "assets/font.ttf""#, r#""src": "../secrets.ttf""#);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "asset-src"), "{errors:?}");
}

#[test]
fn absolute_asset_path_is_rejected() {
    let json = valid_json().replace(r#""src": "assets/font.ttf""#, r#""src": "/etc/passwd""#);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "asset-src"), "{errors:?}");
}

#[test]
fn windows_drive_asset_path_is_rejected() {
    // `Path::is_absolute()` reports false for "C:/x" on Unix and true on
    // Windows; the rejection must hold on every OS because documents are
    // platform-neutral (found by the Windows leg of CI run 44296673474).
    let json = valid_json().replace(
        r#""src": "assets/font.ttf""#,
        r#""src": "C:/Windows/fonts/arial.ttf""#,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "asset-src"), "{errors:?}");
}

#[test]
fn unc_asset_path_is_rejected() {
    let json = valid_json().replace(
        r#""src": "assets/font.ttf""#,
        r#""src": "\\\\server\\share\\font.ttf""#,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "asset-src"), "{errors:?}");
}

#[test]
fn missing_asset_file_is_rejected() {
    let json = valid_json().replace(
        r#""src": "assets/font.ttf""#,
        r#""src": "assets/ghost.ttf""#,
    );
    let errors = errors_for(&json);
    assert!(
        errors.iter().any(|e| e.code == "asset-missing"),
        "{errors:?}"
    );
}

#[test]
fn pinned_hash_mismatch_is_rejected() {
    let json = valid_json().replace(
        r#""sha256": "cbb729b15b71376cbac4faa4db68f0093cc43da48cbc246f4ece869874561d7b""#,
        r#""sha256": "0000000000000000000000000000000000000000000000000000000000000000""#,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "asset-hash"), "{errors:?}");
}

#[test]
fn unpinned_asset_skips_hash_check() {
    // Same file, no sha256 pin: still valid (hash check is opt-in per asset).
    let pinned = r#""src": "assets/dot.png",
                          "sha256": "cbb729b15b71376cbac4faa4db68f0093cc43da48cbc246f4ece869874561d7b""#;
    let json = valid_json().replace(pinned, r#""src": "assets/dot.png""#);
    let doc = validate_str(&json, Path::new(FIXTURES))
        .unwrap_or_else(|e| panic!("unpinned asset should pass: {e:?}"));
    assert!(doc.assets.contains_key("img"));
}

// --- v0.2 boundary tests: transitions, gradient, radius, shadow, clip ---

/// The base document with a second scene carrying a valid crossfade.
fn crossfade_json() -> String {
    let json = valid_json().replace(
        r##"            ]
                }
            ]
        }"##,
        r##"            ]
                },
                {
                    "id": "s2",
                    "duration": 705600000,
                    "transition_in": { "kind": "crossfade", "duration": 352800000 },
                    "elements": []
                }
            ]
        }"##,
    );
    assert!(json.contains("s2"), "second scene must be injected");
    json
}

#[test]
fn valid_crossfade_on_second_scene_passes() {
    let doc = validate_str(&crossfade_json(), Path::new(FIXTURES))
        .unwrap_or_else(|e| panic!("valid crossfade rejected: {e:?}"));
    assert_eq!(doc.scenes[1].crossfade_in(), Some(352_800_000));
}

#[test]
fn crossfade_on_first_scene_is_rejected() {
    let json = valid_json().replace(
        r##"                    "id": "s1",
                    "duration": 705600000,"##,
        r##"                    "id": "s1",
                    "duration": 705600000,
                    "transition_in": { "kind": "crossfade", "duration": 705600000 },"##,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "transition" && e.path == "/scenes/0/transition_in"),
        "{errors:?}"
    );
}

#[test]
fn crossfade_duration_past_scene_end_is_rejected() {
    let json = crossfade_json().replace(r#""duration": 352800000"#, r#""duration": 999999999"#);
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "transition" && e.path == "/scenes/1/transition_in/duration"),
        "{errors:?}"
    );
}

#[test]
fn crossfade_duration_zero_is_rejected() {
    let json = crossfade_json().replace(r#""duration": 352800000"#, r#""duration": 0"#);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "transition"), "{errors:?}");
}

#[test]
fn cut_with_nonzero_duration_is_rejected() {
    let json = crossfade_json().replace(
        r#"{ "kind": "crossfade", "duration": 352800000 }"#,
        r#"{ "kind": "cut", "duration": 5 }"#,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "transition"), "{errors:?}");
}

#[test]
fn gradient_with_fewer_than_two_stops_is_rejected() {
    let json = valid_json().replace(
        r##"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"##,
        r##"{ "type": "rect", "x": 16, "y": 16, "w": 64, "h": 64,
             "gradient": { "angle": 0, "stops": [ { "at": 0.0, "color": "#000000" } ] } }"##,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "gradient" && e.path.ends_with("/gradient/stops")),
        "{errors:?}"
    );
}

#[test]
fn gradient_with_unsorted_stops_is_rejected() {
    let json = valid_json().replace(
        r##"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"##,
        r##"{ "type": "rect", "x": 16, "y": 16, "w": 64, "h": 64,
             "gradient": { "angle": 0, "stops": [ { "at": 1.0, "color": "#ffffff" }, { "at": 0.0, "color": "#000000" } ] } }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "gradient"), "{errors:?}");
}

#[test]
fn gradient_stop_outside_unit_range_is_rejected() {
    let json = valid_json().replace(
        r##"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"##,
        r##"{ "type": "rect", "x": 16, "y": 16, "w": 64, "h": 64,
             "gradient": { "angle": 0, "stops": [ { "at": -0.5, "color": "#000000" }, { "at": 1.0, "color": "#ffffff" } ] } }"##,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "gradient" && e.path.ends_with("/stops/0/at")),
        "{errors:?}"
    );
}

#[test]
fn oversize_radius_is_rejected() {
    let json = valid_json().replace(
        r##"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"##,
        r##"{ "type": "rect", "x": 16, "y": 16, "w": 64, "h": 64, "radius": 20000.0 }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "radius"), "{errors:?}");
}

#[test]
fn oversize_shadow_blur_is_rejected() {
    let json = valid_json().replace(
        r##""size": 24.0 }"##,
        r##""size": 24.0, "shadow": { "offset_x": 0.0, "offset_y": 0.0, "blur": 4096.0, "color": "#000000" } }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "shadow"), "{errors:?}");
}

#[test]
fn shadow_alpha_above_one_is_rejected() {
    let json = valid_json().replace(
        r##""size": 24.0 }"##,
        r##""size": 24.0, "shadow": { "offset_x": 0.0, "offset_y": 0.0, "blur": 4.0, "color": "#000000", "alpha": 1.5 } }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "shadow"), "{errors:?}");
}

#[test]
fn empty_clip_rect_is_rejected() {
    let json = valid_json().replace(
        r##"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"##,
        r##"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64, "clip": { "x": 0, "y": 0, "w": 0, "h": 10 } }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "clip"), "{errors:?}");
}

// ---------------------------------------------------------------------------
// v0.3 media boundaries (plan §12 v0.3, §9.5): video elements and the audio
// mix. Validation checks structure, not media content — decoding belongs to
// the render path (cheapest-first, plan §3 P4), so the fixtures only need to
// exist on disk.
// ---------------------------------------------------------------------------

/// A valid document whose scene shows a trimmed video element.
fn video_json() -> String {
    r##"{
            "v": 1,
            "size": { "w": 320, "h": 240 },
            "assets": {
                "clip": { "type": "video", "src": "assets/dot.png" }
            },
            "scenes": [
                {
                    "id": "s1",
                    "duration": 705600000,
                    "elements": [
                        { "type": "video", "asset": "clip", "x": 0, "y": 0, "w": 320, "h": 240,
                          "fit": "cover",
                          "source_range": { "from": 705600000, "to": 1411200000 },
                          "speed": 2.0 }
                    ]
                }
            ]
        }"##
    .to_string()
}

/// A valid document with one audio track carrying a fade.
fn audio_json() -> String {
    r##"{
            "v": 1,
            "size": { "w": 320, "h": 240 },
            "assets": {
                "tone": { "type": "audio", "src": "assets/dot.png" }
            },
            "audio_rate": 48000,
            "audio": [
                { "asset": "tone", "gain": 0.8, "start_at": 705600000,
                  "fades": [ { "at": 0, "len": 88200000, "kind": "in" } ] }
            ],
            "scenes": [ { "id": "s1", "duration": 705600000, "elements": [] } ]
        }"##
    .to_string()
}

#[test]
fn video_element_passes_validation() {
    assert!(validate_str(&video_json(), Path::new(FIXTURES)).is_ok());
}

#[test]
fn video_freeze_range_is_valid() {
    let json = video_json().replace(
        r##"{ "from": 705600000, "to": 1411200000 }"##,
        r##"{ "from": 705600000, "to": 705600000 }"##,
    );
    assert!(validate_str(&json, Path::new(FIXTURES)).is_ok());
}

#[test]
fn video_reversed_source_range_is_rejected() {
    let json = video_json().replace(
        r##"{ "from": 705600000, "to": 1411200000 }"##,
        r##"{ "from": 1411200000, "to": 705600000 }"##,
    );
    let errors = errors_for(&json);
    assert!(
        errors.iter().any(|e| e.code == "source-range"),
        "{errors:?}"
    );
}

#[test]
fn video_negative_source_range_is_rejected() {
    let json = video_json().replace(
        r##"{ "from": 705600000, "to": 1411200000 }"##,
        r##"{ "from": -705600000, "to": 705600000 }"##,
    );
    let errors = errors_for(&json);
    assert!(
        errors.iter().any(|e| e.code == "source-range"),
        "{errors:?}"
    );
}

#[test]
fn video_speed_zero_is_rejected() {
    let json = video_json().replace(r##"speed": 2.0"##, r##"speed": 0.0"##);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "video-speed"), "{errors:?}");
}

#[test]
fn video_speed_above_max_is_rejected() {
    let json = video_json().replace(r##"speed": 2.0"##, r##"speed": 11.0"##);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "video-speed"), "{errors:?}");
}

#[test]
fn video_zero_width_is_rejected() {
    let json = video_json().replace(
        r##"x": 0, "y": 0, "w": 320, "h": 240,"##,
        r##"x": 0, "y": 0, "w": 0, "h": 240,"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "video-size"), "{errors:?}");
}

#[test]
fn video_with_unknown_asset_is_rejected() {
    let json = video_json().replace(r##"asset": "clip""##, r##"asset": "missing""##);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "video-asset"), "{errors:?}");
}

#[test]
fn video_with_wrong_asset_kind_is_rejected() {
    let json = video_json().replace(
        r##"{ "type": "video", "src": "assets/dot.png" }"##,
        r##"{ "type": "image", "src": "assets/dot.png" }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "asset-type"), "{errors:?}");
}

#[test]
fn audio_track_passes_validation() {
    assert!(validate_str(&audio_json(), Path::new(FIXTURES)).is_ok());
}

#[test]
fn audio_with_unknown_asset_is_rejected() {
    let json = audio_json().replace(r##"asset": "tone""##, r##"asset": "missing""##);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "audio-asset"), "{errors:?}");
}

#[test]
fn audio_with_wrong_asset_kind_is_rejected() {
    let json = audio_json().replace(
        r##"{ "type": "audio", "src": "assets/dot.png" }"##,
        r##"{ "type": "font", "src": "assets/font.ttf" }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "asset-type"), "{errors:?}");
}

#[test]
fn audio_gain_out_of_range_is_rejected() {
    for bad in ["-0.5", "8.5"] {
        let json = audio_json().replace(r##"gain": 0.8"##, &format!(r##"gain": {bad}"##));
        let errors = errors_for(&json);
        assert!(errors.iter().any(|e| e.code == "audio-gain"), "{errors:?}");
    }
}

#[test]
fn audio_fade_zero_len_is_rejected() {
    let json = audio_json().replace(
        r##"{ "at": 0, "len": 88200000, "kind": "in" }"##,
        r##"{ "at": 0, "len": 0, "kind": "in" }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "audio-fade"), "{errors:?}");
}

#[test]
fn audio_fade_negative_start_is_rejected() {
    let json = audio_json().replace(
        r##"{ "at": 0, "len": 88200000, "kind": "in" }"##,
        r##"{ "at": -1, "len": 88200000, "kind": "in" }"##,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "audio-fade"), "{errors:?}");
}

#[test]
fn audio_negative_start_at_is_rejected() {
    let json = audio_json().replace(r##"start_at": 705600000"##, r##"start_at": -705600000"##);
    let errors = errors_for(&json);
    assert!(
        errors.iter().any(|e| e.code == "audio-placement"),
        "{errors:?}"
    );
}

#[test]
fn audio_rate_not_dividing_timebase_is_rejected() {
    let json = audio_json().replace(r##"audio_rate": 48000"##, r##"audio_rate": 44101"##);
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "audio-rate"), "{errors:?}");
}

#[test]
fn audio_reversed_source_range_is_rejected() {
    let json = audio_json().replace(
        r##"fades": [ { "at": 0"##,
        r##"source_range": { "from": 10, "to": 5 }, "fades": [ { "at": 0"##,
    );
    let errors = errors_for(&json);
    assert!(
        errors.iter().any(|e| e.code == "source-range"),
        "{errors:?}"
    );
}

// ---------------------------------------------------------------------------
// v0.5 caption boundaries (plan §12 v0.5, §9.5): one bottom track whose cues
// stay inside the single 1s scene and reference the font fixture.
// ---------------------------------------------------------------------------

fn caption_json() -> String {
    r##"{
            "v": 1,
            "size": { "w": 320, "h": 240 },
            "assets": {
                "f": { "type": "font", "src": "assets/font.ttf" }
            },
            "captions": [
                {
                    "style": { "font": "f", "size": 24.0 },
                    "cues": [
                        { "from": 0, "to": 352800000, "text": "hello" },
                        { "from": 352800000, "to": 705600000, "text": "captions" }
                    ]
                }
            ],
            "scenes": [ { "id": "s1", "duration": 705600000, "elements": [] } ]
        }"##
    .to_string()
}

#[test]
fn caption_track_passes_validation() {
    let doc = validate_str(&caption_json(), Path::new(FIXTURES))
        .unwrap_or_else(|e| panic!("valid caption track rejected: {e:?}"));
    assert_eq!(doc.captions.len(), 1);
    assert_eq!(doc.captions[0].cue_at(0).unwrap().text, "hello");
}

#[test]
fn caption_style_takes_documented_defaults() {
    let doc = validate_str(&caption_json(), Path::new(FIXTURES)).unwrap();
    let style = &doc.captions[0].style;
    assert_eq!(style.margin, 48);
    assert_eq!(
        style.position,
        quillon_core::CaptionPosition::Bottom,
        "position default"
    );
}

#[test]
fn caption_unknown_font_is_rejected() {
    let json = caption_json().replace(
        r#""font": "f", "size": 24.0"#,
        r#""font": "nope", "size": 24.0"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "font-asset" && e.path == "/captions/0/style/font"),
        "{errors:?}"
    );
}

#[test]
fn caption_font_of_wrong_kind_is_rejected() {
    let json = caption_json().replace(
        r##""f": { "type": "font", "src": "assets/font.ttf" }"##,
        r##""f": { "type": "image", "src": "assets/dot.png" }"##,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "asset-type" && e.path == "/captions/0/style/font"),
        "{errors:?}"
    );
}

#[test]
fn caption_empty_text_is_rejected() {
    let json = caption_json().replace(r#""text": "hello""#, r#""text": "   ""#);
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "caption-text" && e.path == "/captions/0/cues/0/text"),
        "{errors:?}"
    );
}

#[test]
fn caption_zero_size_is_rejected() {
    let json = caption_json().replace(
        r#""font": "f", "size": 24.0"#,
        r#""font": "f", "size": 0.0"#,
    );
    let errors = errors_for(&json);
    assert!(errors.iter().any(|e| e.code == "text-size"), "{errors:?}");
}

#[test]
fn caption_negative_margin_is_rejected() {
    let json = caption_json().replace(
        r#""font": "f", "size": 24.0"#,
        r#""font": "f", "size": 24.0, "margin": -1"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors.iter().any(|e| e.code == "caption-margin"),
        "{errors:?}"
    );
}

#[test]
fn caption_negative_from_is_rejected() {
    let json = caption_json().replace(
        r#"{ "from": 0, "to": 352800000"#,
        r#"{ "from": -1, "to": 352800000"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "caption-timing" && e.path == "/captions/0/cues/0/from"),
        "{errors:?}"
    );
}

#[test]
fn caption_reversed_cue_is_rejected() {
    let json = caption_json().replace(
        r#"{ "from": 0, "to": 352800000"#,
        r#"{ "from": 352800000, "to": 0"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "caption-timing" && e.path == "/captions/0/cues/0/to"),
        "{errors:?}"
    );
}

#[test]
fn caption_past_document_end_is_rejected() {
    let json = caption_json().replace(
        r#"{ "from": 352800000, "to": 705600000, "text": "captions" }"#,
        r#"{ "from": 352800000, "to": 705600001, "text": "captions" }"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "caption-timing" && e.path == "/captions/0/cues/1/to"),
        "{errors:?}"
    );
}

#[test]
fn caption_overlap_is_rejected() {
    let json = caption_json().replace(
        r#"{ "from": 352800000, "to": 705600000, "text": "captions" }"#,
        r#"{ "from": 100000000, "to": 705600000, "text": "captions" }"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "caption-overlap" && e.path == "/captions/0/cues/1/from"),
        "{errors:?}"
    );
}

#[test]
fn caption_unsorted_cues_are_rejected() {
    let json = caption_json().replace(
        r#"{ "from": 0, "to": 352800000, "text": "hello" },
                        { "from": 352800000, "to": 705600000, "text": "captions" }"#,
        r#"{ "from": 100000000, "to": 200000000, "text": "hello" },
                        { "from": 0, "to": 50000000, "text": "earlier" }"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "caption-order" && e.path == "/captions/0/cues/1/from"),
        "{errors:?}"
    );
}

#[test]
fn caption_unknown_style_field_is_rejected() {
    let json = caption_json().replace(
        r#""style": { "font": "f", "size": 24.0 }"#,
        r##""style": { "font": "f", "size": 24.0, "colour": "#ffffff" }"##,
    );
    let errors = errors_for(&json);
    let joined = format!("{errors:?}");
    assert!(
        joined.contains("colour") || joined.contains("unknown"),
        "typo fields must be rejected, got {joined}"
    );
}

#[test]
fn scene_duration_over_the_cap_is_a_named_error() {
    // r1 F1: three legal-per-old-rules scenes whose sum overflows i64
    // panicked `inspect` via total_duration(). The bound must name the
    // limit and the offending value instead.
    let half = (i64::MAX - 1) / 2;
    let json = format!(
        r##"{{
            "v": 1,
            "size": {{"w": 320, "h": 240}},
            "scenes": [
                {{"id": "a", "duration": {half}, "elements": []}},
                {{"id": "b", "duration": {half}, "elements": []}},
                {{"id": "c", "duration": {half}, "elements": []}}
            ]
        }}"##
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "scene-duration" && e.path == "/scenes/0/duration"),
        "each over-cap scene is named: {errors:?}"
    );
}

#[test]
fn document_total_over_the_cap_is_a_named_error() {
    // r1 F1: scenes within the per-scene cap can still sum past the
    // document cap; the total must be checked with a named error too.
    let day = quillon_core::MAX_SCENE_DURATION_TICKS;
    let json = format!(
        r##"{{
            "v": 1,
            "size": {{"w": 320, "h": 240}},
            "scenes": [
                {{"id": "a", "duration": {day}, "elements": []}},
                {{"id": "b", "duration": {day}, "elements": []}}
            ]
        }}"##
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "document-duration" && e.path == "/scenes"),
        "the document total is bounded: {errors:?}"
    );
}

#[test]
fn element_geometry_over_the_cap_is_a_named_error() {
    // r1 F9: w/h at i64::MAX passed the old checks and panicked the lint
    // bbox math; the geometry bound must name the field.
    let json = valid_json().replace(
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 64, "h": 64 }"#,
        r#"{ "type": "image", "asset": "img", "x": 16, "y": 16, "w": 9223372036854775807, "h": 64 }"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "geometry" && e.path == "/scenes/0/elements/1/w"),
        "huge geometry is a named error: {errors:?}"
    );
}

#[test]
fn clip_geometry_over_the_cap_is_a_named_error() {
    let json = valid_json().replace(
        r#"{ "type": "text", "text": "Quillon", "font": "f", "x": 160, "y": 120, "size": 24.0 }"#,
        r#"{ "type": "text", "text": "Quillon", "font": "f", "x": 160, "y": 120, "size": 24.0,
             "clip": { "x": 0, "y": -9223372036854775808, "w": 10, "h": 10 } }"#,
    );
    let errors = errors_for(&json);
    assert!(
        errors
            .iter()
            .any(|e| e.code == "geometry" && e.path == "/scenes/0/elements/0/clip/y"),
        "clip geometry is bounded: {errors:?}"
    );
}

#[test]
fn total_duration_saturates_instead_of_panicking_on_unvalidated_docs() {
    // The renderer is a library too: an unvalidated document with huge
    // durations must produce a saturated total, never a panic (r1 F1
    // defense in depth).
    let half = (i64::MAX - 1) / 2;
    let doc = quillon_core::DocumentBuilder::new(quillon_core::document::Size { w: 320, h: 240 })
        .scene(quillon_core::document::Scene {
            id: "a".into(),
            duration: half,
            transition_in: None,
            elements: Vec::new(),
        })
        .scene(quillon_core::document::Scene {
            id: "b".into(),
            duration: half,
            transition_in: None,
            elements: Vec::new(),
        })
        .build();
    // half = (i64::MAX - 1) / 2, so half + half = i64::MAX - 1 exactly:
    // no saturation reached, but the point is no panic.
    assert_eq!(doc.total_duration(), i64::MAX - 1);
    assert_eq!(doc.scene_at(i64::MAX - 1), None);
}
