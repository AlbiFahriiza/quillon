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
