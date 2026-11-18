//! Document validation (myquillonplan.md §5.2, §11).
//!
//! Pipeline, in order:
//! 1. JSON syntax parse.
//! 2. serde deserialization — field-qualified errors ("missing field `h`").
//! 3. jsonschema validation against the derived schema — supplies JSON-pointer
//!    locations; raw `oneOf` errors are translated, never surfaced as-is
//!    (research/J: they are not actionable for agents).
//! 4. Semantic checks over the parsed document (the boundary list in plan §9.5).
//! 5. Asset probe: existence and sha256 pin, relative to the document's
//!    directory, with path confinement (anti path traversal, plan §11).
//!
//! Every failure is a [`ValidationError`] with a JSON-pointer `path`.

use crate::color::Color;
use crate::document::{Anim, Asset, Document, Element, Scene, Track};
use crate::schema::document_schema;
use crate::{
    DOCUMENT_VERSION, MAX_ASSETS, MAX_DIMENSION, MAX_ELEMENTS_PER_SCENE, MAX_FPS, MAX_ID_LEN,
    MAX_KEYS_PER_TRACK, MAX_SCENES, MAX_TEXT_LEN, MAX_TEXT_SIZE, MIN_DIMENSION, time,
};
use jsonschema::validator_for;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A validation failure. `path` is a JSON pointer (e.g. `/scenes/0/elements/3`)
/// into the submitted document; `""` means the document root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationError {
    pub path: String,
    pub code: String,
    pub message: String,
}

impl ValidationError {
    fn new(path: impl Into<String>, code: &str, message: impl Into<String>) -> Self {
        ValidationError {
            path: path.into(),
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let path = if self.path.is_empty() {
            "<root>"
        } else {
            &self.path
        };
        write!(f, "{path}: {} ({})", self.code, self.message)
    }
}

/// Parse and fully validate a document. `base_dir` is the directory the
/// document was read from; asset paths resolve against it and must stay
/// inside it.
pub fn validate_str(input: &str, base_dir: &Path) -> Result<Document, Vec<ValidationError>> {
    let value: Value = match serde_json::from_str(input) {
        Ok(value) => value,
        Err(err) => {
            return Err(vec![ValidationError::new(
                "",
                "syntax",
                format!("not valid JSON: {err}"),
            )]);
        }
    };

    let mut errors = Vec::new();
    match serde_json::from_value::<Document>(value.clone()) {
        Ok(doc) => {
            errors.extend(schema_errors(&value));
            errors.extend(validate_document(&doc, base_dir));
            if errors.is_empty() {
                return Ok(doc);
            }
        }
        Err(err) => {
            // serde names the field; jsonschema supplies the pointer.
            errors.extend(schema_errors(&value));
            errors.push(ValidationError::new("", "schema", err.to_string()));
        }
    }
    Err(errors)
}

/// Semantic validation over an already-parsed document (steps 4-5).
pub fn validate_document(doc: &Document, base_dir: &Path) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_header(doc, &mut errors);
    validate_assets(doc, base_dir, &mut errors);
    validate_scenes(doc, &mut errors);
    errors
}

fn validate_header(doc: &Document, errors: &mut Vec<ValidationError>) {
    if doc.v != DOCUMENT_VERSION {
        errors.push(ValidationError::new(
            "/v",
            "version",
            format!(
                "document version {} is not supported; this build reads version {DOCUMENT_VERSION}",
                doc.v
            ),
        ));
    }
    if doc.timebase != time::FLICKS_PER_SECOND {
        errors.push(ValidationError::new(
            "/timebase",
            "timebase",
            format!(
                "timebase must be {} ticks/second for format v1",
                time::FLICKS_PER_SECOND
            ),
        ));
    }
    if doc.fps == 0 || doc.fps > MAX_FPS {
        errors.push(ValidationError::new(
            "/fps",
            "fps",
            format!("fps must be between 1 and {MAX_FPS}"),
        ));
    } else if doc.timebase % i64::from(doc.fps) != 0 {
        errors.push(ValidationError::new(
            "/fps",
            "fps",
            format!(
                "fps {} does not divide the timebase {} exactly; use a whole-frame rate",
                doc.fps, doc.timebase
            ),
        ));
    }
    for (field, side) in [("/size/w", doc.size.w), ("/size/h", doc.size.h)] {
        if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&side) {
            errors.push(ValidationError::new(
                field,
                "size",
                format!(
                    "canvas side must be between {MIN_DIMENSION} and {MAX_DIMENSION} pixels, got {side}"
                ),
            ));
        }
    }
    if doc.bit_depth != 8 {
        errors.push(ValidationError::new(
            "/bit_depth",
            "bit_depth",
            "format v1 renders at 8 bits per channel",
        ));
    }
    push_color_error(&doc.bg, "/bg", errors);
}

fn validate_assets(doc: &Document, base_dir: &Path, errors: &mut Vec<ValidationError>) {
    if doc.assets.len() > MAX_ASSETS {
        errors.push(ValidationError::new(
            "/assets",
            "limit",
            format!("more than {MAX_ASSETS} assets are not accepted"),
        ));
    }
    for (id, asset) in &doc.assets {
        let path = format!("/assets/{id}");
        if id.is_empty() || id.len() > MAX_ID_LEN {
            errors.push(ValidationError::new(
                path.clone(),
                "asset-id",
                format!("asset id must be 1..={MAX_ID_LEN} bytes"),
            ));
        }
        push_asset_path_error(asset.src(), &path, errors);
        // Probe: cheapest-first (plan §3 P4) — a missing asset is a document
        // error before any pixel is rendered (research/R).
        match resolve_confined(base_dir, asset.src()) {
            Some(file) => verify_asset_hash(asset, &file, &path, errors),
            None => errors.push(ValidationError::new(
                path,
                "asset-missing",
                format!("asset file \"{}\" does not exist", asset.src()),
            )),
        }
    }
}

/// Check the optional sha256 pin of a located asset file.
fn verify_asset_hash(asset: &Asset, file: &Path, path: &str, errors: &mut Vec<ValidationError>) {
    let Some(expected) = asset.sha256() else {
        return;
    };
    if !is_sha256_hex(expected) {
        errors.push(ValidationError::new(
            format!("{path}/sha256"),
            "asset-hash",
            "sha256 must be 64 lowercase hex characters",
        ));
        return;
    }
    match std::fs::read(file) {
        Ok(bytes) => {
            let actual = hex_sha256(&bytes);
            if actual != expected {
                errors.push(ValidationError::new(
                    path.to_string(),
                    "asset-hash",
                    format!("asset file hash is {actual}, document pins {expected}"),
                ));
            }
        }
        Err(err) => errors.push(ValidationError::new(
            path.to_string(),
            "asset-missing",
            format!("asset file cannot be read: {err}"),
        )),
    }
}

fn validate_scenes(doc: &Document, errors: &mut Vec<ValidationError>) {
    if doc.scenes.is_empty() || doc.scenes.len() > MAX_SCENES {
        errors.push(ValidationError::new(
            "/scenes",
            "limit",
            format!("a document needs 1..={MAX_SCENES} scenes"),
        ));
        return;
    }
    let mut ids: HashSet<&str> = HashSet::new();
    for (i, scene) in doc.scenes.iter().enumerate() {
        validate_scene(scene, &format!("/scenes/{i}"), &mut ids, doc, errors);
    }
}

fn validate_scene<'a>(
    scene: &'a Scene,
    base: &str,
    ids: &mut HashSet<&'a str>,
    doc: &Document,
    errors: &mut Vec<ValidationError>,
) {
    if scene.id.is_empty() || scene.id.len() > MAX_ID_LEN {
        errors.push(ValidationError::new(
            format!("{base}/id"),
            "scene-id",
            format!("scene id must be 1..={MAX_ID_LEN} bytes"),
        ));
    }
    if !ids.insert(scene.id.as_str()) && !scene.id.is_empty() {
        errors.push(ValidationError::new(
            format!("{base}/id"),
            "scene-id",
            format!("scene id \"{}\" is used more than once", scene.id),
        ));
    }
    if scene.duration <= 0 {
        errors.push(ValidationError::new(
            format!("{base}/duration"),
            "scene-duration",
            "scene duration must be a positive number of ticks",
        ));
    }
    if scene.elements.len() > MAX_ELEMENTS_PER_SCENE {
        errors.push(ValidationError::new(
            format!("{base}/elements"),
            "limit",
            format!("more than {MAX_ELEMENTS_PER_SCENE} elements per scene are not accepted"),
        ));
    }
    for (j, element) in scene.elements.iter().enumerate() {
        validate_element(element, &format!("{base}/elements/{j}"), scene, doc, errors);
    }
}

fn validate_element(
    element: &Element,
    path: &str,
    scene: &Scene,
    doc: &Document,
    errors: &mut Vec<ValidationError>,
) {
    match element {
        Element::Text(t) => validate_text(t, path, doc, errors),
        Element::Rect(r) => validate_rect(r, path, errors),
        Element::Image(img) => validate_image(img, path, doc, errors),
    }
    if let Some(anim) = element.anim() {
        validate_anim(anim, path, scene, errors);
    }
}

fn validate_text(
    t: &crate::document::TextElement,
    path: &str,
    doc: &Document,
    errors: &mut Vec<ValidationError>,
) {
    if t.text.is_empty() {
        errors.push(ValidationError::new(
            format!("{path}/text"),
            "text",
            "text must not be empty",
        ));
    }
    if t.text.len() > MAX_TEXT_LEN {
        errors.push(ValidationError::new(
            format!("{path}/text"),
            "limit",
            format!("text longer than {MAX_TEXT_LEN} bytes is not accepted"),
        ));
    }
    if t.size <= 0.0 || t.size > MAX_TEXT_SIZE {
        errors.push(ValidationError::new(
            format!("{path}/size"),
            "text-size",
            format!("font size must be in (0, {MAX_TEXT_SIZE}]"),
        ));
    }
    push_color_error(&t.color, &format!("{path}/color"), errors);
    match doc.assets.get(&t.font) {
        Some(Asset::Font { .. }) => {}
        Some(_) => errors.push(ValidationError::new(
            format!("{path}/font"),
            "asset-type",
            format!("asset \"{}\" exists but is not a font", t.font),
        )),
        None => errors.push(ValidationError::new(
            format!("{path}/font"),
            "font-asset",
            format!("text references unknown font asset \"{}\"", t.font),
        )),
    }
}

fn validate_rect(r: &crate::document::RectElement, path: &str, errors: &mut Vec<ValidationError>) {
    if r.w < 1 || r.h < 1 {
        errors.push(ValidationError::new(
            path,
            "rect-size",
            "rectangle width and height must be at least 1",
        ));
    }
    push_color_error(&r.color, &format!("{path}/color"), errors);
}

fn validate_image(
    img: &crate::document::ImageElement,
    path: &str,
    doc: &Document,
    errors: &mut Vec<ValidationError>,
) {
    if img.w < 1 || img.h < 1 {
        errors.push(ValidationError::new(
            path,
            "image-size",
            "image width and height must be at least 1",
        ));
    }
    match doc.assets.get(&img.asset) {
        Some(Asset::Image { .. }) => {}
        Some(_) => errors.push(ValidationError::new(
            format!("{path}/asset"),
            "asset-type",
            format!("asset \"{}\" exists but is not an image", img.asset),
        )),
        None => errors.push(ValidationError::new(
            format!("{path}/asset"),
            "image-asset",
            format!("image references unknown asset \"{}\"", img.asset),
        )),
    }
}

fn validate_anim(anim: &Anim, path: &str, scene: &Scene, errors: &mut Vec<ValidationError>) {
    let tracks = [
        ("opacity", anim.opacity.as_ref()),
        ("translate_x", anim.translate_x.as_ref()),
        ("translate_y", anim.translate_y.as_ref()),
        ("scale", anim.scale.as_ref()),
        ("rotate", anim.rotate.as_ref()),
    ];
    for (name, track) in tracks {
        let Some(track) = track else { continue };
        let track_path = format!("{path}/anim/{name}");
        validate_track(track, &track_path, scene, errors);
        if name == "opacity" {
            for (i, key) in track.keys.iter().enumerate() {
                if !(0.0..=1.0).contains(&key.v) {
                    errors.push(ValidationError::new(
                        format!("{track_path}/keys/{i}/v"),
                        "opacity-range",
                        "opacity values must be within [0, 1]",
                    ));
                }
            }
        }
    }
}

fn validate_track(track: &Track, path: &str, scene: &Scene, errors: &mut Vec<ValidationError>) {
    if track.keys.len() < 2 {
        errors.push(ValidationError::new(
            format!("{path}/keys"),
            "anim-keys",
            "an animation track needs at least two keys",
        ));
    }
    if track.keys.len() > MAX_KEYS_PER_TRACK {
        errors.push(ValidationError::new(
            format!("{path}/keys"),
            "limit",
            format!("more than {MAX_KEYS_PER_TRACK} keys per track are not accepted"),
        ));
    }
    for (i, key) in track.keys.iter().enumerate() {
        let key_path = format!("{path}/keys/{i}");
        if key.t < 0 || key.t > scene.duration {
            errors.push(ValidationError::new(
                format!("{key_path}/t"),
                "anim-t",
                format!(
                    "keyframe time must be within the scene (0..={} ticks)",
                    scene.duration
                ),
            ));
        }
        if i > 0 && key.t <= track.keys[i - 1].t {
            errors.push(ValidationError::new(
                format!("{key_path}/t"),
                "anim-keys",
                "keyframe times must strictly increase",
            ));
        }
    }
}

fn push_color_error(color: &Color, path: &str, errors: &mut Vec<ValidationError>) {
    if Color::parse(color.as_hex()).is_none() {
        errors.push(ValidationError::new(
            path,
            "color",
            format!("color \"{}\" is not #rrggbb", color.as_hex()),
        ));
    }
}

fn push_asset_path_error(src: &str, path: &str, errors: &mut Vec<ValidationError>) {
    if src.is_empty() {
        errors.push(ValidationError::new(
            format!("{path}/src"),
            "asset-src",
            "asset path must not be empty",
        ));
        return;
    }
    let candidate = Path::new(src);
    if candidate.is_absolute()
        || src.contains('\\')
        || candidate
            .components()
            .any(|c| c == std::path::Component::ParentDir)
    {
        errors.push(ValidationError::new(
            format!("{path}/src"),
            "asset-src",
            "asset path must be relative to the document and stay inside its directory",
        ));
    }
}

/// Map jsonschema findings onto our error shape, translating raw `oneOf`
/// failures into named element problems (plan §5.2 forbids leaking them).
fn schema_errors(value: &Value) -> Vec<ValidationError> {
    let Ok(validator) = validator_for(document_schema()) else {
        // A schema that fails to compile is a build bug caught by the schema
        // unit tests; validation still runs its serde and semantic layers.
        return Vec::new();
    };
    validator
        .iter_errors(value)
        .map(|err| {
            let pointer = err.instance_path().to_string();
            if err.kind().keyword() == "oneOf" {
                let message = translate_one_of(value, &pointer);
                ValidationError::new(pointer, "schema", message)
            } else {
                ValidationError::new(pointer, "schema", err.to_string())
            }
        })
        .collect()
}

fn translate_one_of(value: &Value, pointer: &str) -> String {
    let node = value.pointer(pointer).unwrap_or(value);
    const KNOWN_TYPES: [&str; 4] = ["text", "rect", "image", "font"];
    match node.get("type").and_then(Value::as_str) {
        Some(tag) if KNOWN_TYPES.contains(&tag) => {
            format!("the fields of this \"{tag}\" do not match the type's requirements; a required field is missing or has the wrong type")
        }
        Some(tag) => {
            format!("type \"{tag}\" is not one of the element or asset types this format defines")
        }
        None => "the fields at this position do not match any defined type; check the required fields of the type you intended".to_string(),
    }
}

/// Resolve `src` under `base_dir`, confined: the joined path must not escape
/// `base_dir`. Validation has already rejected absolute paths and `..`.
fn resolve_confined(base_dir: &Path, src: &str) -> Option<PathBuf> {
    let joined = base_dir.join(src);
    let canonical = joined.canonicalize().ok()?;
    let base = base_dir.canonicalize().ok()?;
    canonical.starts_with(&base).then_some(canonical)
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
