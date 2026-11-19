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
    DOCUMENT_VERSION, MAX_ASSETS, MAX_AUDIO_RATE, MAX_AUDIO_TRACKS, MAX_BLUR_PX,
    MAX_CAPTION_TRACKS, MAX_CUES_PER_TRACK, MAX_DIMENSION, MAX_ELEMENTS_PER_SCENE,
    MAX_FADES_PER_TRACK, MAX_FPS, MAX_GAIN, MAX_GRADIENT_STOPS, MAX_ID_LEN, MAX_KEYS_PER_TRACK,
    MAX_RADIUS_PX, MAX_SCENES, MAX_SPEED, MAX_TEXT_LEN, MAX_TEXT_SIZE, MIN_AUDIO_RATE,
    MIN_DIMENSION, time,
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
    validate_audio(doc, &mut errors);
    validate_captions(doc, &mut errors);
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
        validate_scene(
            scene,
            &format!("/scenes/{i}"),
            &mut ids,
            i == 0,
            doc,
            errors,
        );
    }
}

fn validate_scene<'a>(
    scene: &'a Scene,
    base: &str,
    ids: &mut HashSet<&'a str>,
    is_first: bool,
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
    validate_transition(scene, base, is_first, errors);
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

/// Plan §5.2: a crossfade needs a previous scene to fade from and must fit
/// inside the scene it opens; a cut has no duration.
fn validate_transition(
    scene: &Scene,
    base: &str,
    is_first: bool,
    errors: &mut Vec<ValidationError>,
) {
    let Some(t) = &scene.transition_in else {
        return;
    };
    let path = format!("{base}/transition_in");
    if is_first && t.kind == crate::document::TransitionKind::Crossfade {
        errors.push(ValidationError::new(
            &path,
            "transition",
            "the first scene has nothing to crossfade from; use a cut or drop transition_in",
        ));
    }
    match t.kind {
        crate::document::TransitionKind::Cut => {
            if t.duration != 0 {
                errors.push(ValidationError::new(
                    format!("{path}/duration"),
                    "transition",
                    "a cut takes no time; duration must be 0",
                ));
            }
        }
        crate::document::TransitionKind::Crossfade => {
            if t.duration <= 0 || t.duration > scene.duration {
                errors.push(ValidationError::new(
                    format!("{path}/duration"),
                    "transition",
                    "crossfade duration must be within (0, the scene duration]",
                ));
            }
        }
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
        Element::Video(v) => validate_video(v, path, doc, errors),
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
    if let Some(s) = &t.shadow {
        validate_shadow(s, &format!("{path}/shadow"), errors);
    }
    if let Some(c) = &t.clip {
        validate_clip(c, &format!("{path}/clip"), errors);
    }
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
    if let Some(g) = &r.gradient {
        validate_gradient(g, &format!("{path}/gradient"), errors);
    }
    if let Some(radius) = r.radius {
        if !(0.0..=MAX_RADIUS_PX).contains(&radius) {
            errors.push(ValidationError::new(
                format!("{path}/radius"),
                "radius",
                format!("corner radius must be within [0, {MAX_RADIUS_PX}]"),
            ));
        }
    }
    if let Some(s) = &r.shadow {
        validate_shadow(s, &format!("{path}/shadow"), errors);
    }
    if let Some(c) = &r.clip {
        validate_clip(c, &format!("{path}/clip"), errors);
    }
}

/// Gradient stops must be at least two, strictly increasing, inside [0, 1].
fn validate_gradient(
    g: &crate::document::GradientDef,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    if g.stops.len() < 2 {
        errors.push(ValidationError::new(
            format!("{path}/stops"),
            "gradient",
            "a gradient needs at least two stops",
        ));
    }
    if g.stops.len() > MAX_GRADIENT_STOPS {
        errors.push(ValidationError::new(
            format!("{path}/stops"),
            "limit",
            format!("more than {MAX_GRADIENT_STOPS} gradient stops are not accepted"),
        ));
    }
    for (i, stop) in g.stops.iter().enumerate() {
        if !(0.0..=1.0).contains(&stop.at) {
            errors.push(ValidationError::new(
                format!("{path}/stops/{i}/at"),
                "gradient",
                "gradient stop positions must be within [0, 1]",
            ));
        }
        if i > 0 && stop.at <= g.stops[i - 1].at {
            errors.push(ValidationError::new(
                format!("{path}/stops/{i}/at"),
                "gradient",
                "gradient stop positions must strictly increase",
            ));
        }
        push_color_error(&stop.color, &format!("{path}/stops/{i}/color"), errors);
    }
}

fn validate_shadow(s: &crate::document::ShadowDef, path: &str, errors: &mut Vec<ValidationError>) {
    if !(0.0..=MAX_BLUR_PX).contains(&s.blur) {
        errors.push(ValidationError::new(
            format!("{path}/blur"),
            "shadow",
            format!("shadow blur must be within [0, {MAX_BLUR_PX}]"),
        ));
    }
    if !(0.0..=1.0).contains(&s.alpha) {
        errors.push(ValidationError::new(
            format!("{path}/alpha"),
            "shadow",
            "shadow alpha must be within [0, 1]",
        ));
    }
    push_color_error(&s.color, &format!("{path}/color"), errors);
}

fn validate_clip(c: &crate::document::ClipRect, path: &str, errors: &mut Vec<ValidationError>) {
    if c.w < 1 || c.h < 1 {
        errors.push(ValidationError::new(
            path,
            "clip",
            "clip width and height must be at least 1",
        ));
    }
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
    if let Some(s) = &img.shadow {
        validate_shadow(s, &format!("{path}/shadow"), errors);
    }
    if let Some(c) = &img.clip {
        validate_clip(c, &format!("{path}/clip"), errors);
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

/// Video elements: destination box, source window, retime speed, and the
/// asset kind (plan §12 v0.3; research/M §2: `from == to` is a freeze).
fn validate_video(
    v: &crate::document::VideoElement,
    path: &str,
    doc: &Document,
    errors: &mut Vec<ValidationError>,
) {
    if v.w < 1 || v.h < 1 {
        errors.push(ValidationError::new(
            path,
            "video-size",
            "video width and height must be at least 1",
        ));
    }
    if let Some(range) = &v.source_range {
        push_source_range_error(range, &format!("{path}/source_range"), errors);
    }
    if !(0.0 < v.speed && v.speed <= MAX_SPEED) {
        errors.push(ValidationError::new(
            format!("{path}/speed"),
            "video-speed",
            format!("playback speed must be within (0, {MAX_SPEED}]"),
        ));
    }
    if let Some(s) = &v.shadow {
        validate_shadow(s, &format!("{path}/shadow"), errors);
    }
    if let Some(c) = &v.clip {
        validate_clip(c, &format!("{path}/clip"), errors);
    }
    match doc.assets.get(&v.asset) {
        Some(Asset::Video { .. }) => {}
        Some(_) => errors.push(ValidationError::new(
            format!("{path}/asset"),
            "asset-type",
            format!("asset \"{}\" exists but is not a video", v.asset),
        )),
        None => errors.push(ValidationError::new(
            format!("{path}/asset"),
            "video-asset",
            format!("video references unknown asset \"{}\"", v.asset),
        )),
    }
}

/// A source window must be well-formed: non-negative, `to >= from`, and
/// `to > from` unless it freezes a single frame.
fn push_source_range_error(
    range: &crate::document::SourceRange,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    if range.from < 0 || range.to < range.from {
        errors.push(ValidationError::new(
            path,
            "source-range",
            "source range must satisfy 0 <= from <= to (from == to freezes one frame)",
        ));
    }
}

/// The audio mix: rate divisibility, track count, per-track gain, fades, and
/// asset kinds (plan §5.1 audio array, §12 v0.3).
fn validate_audio(doc: &Document, errors: &mut Vec<ValidationError>) {
    if doc.audio.is_empty() {
        return;
    }
    if !(MIN_AUDIO_RATE..=MAX_AUDIO_RATE).contains(&doc.audio_rate) {
        errors.push(ValidationError::new(
            "/audio_rate",
            "audio-rate",
            format!("audio_rate must be between {MIN_AUDIO_RATE} and {MAX_AUDIO_RATE} Hz"),
        ));
    } else if doc.timebase % i64::from(doc.audio_rate) != 0 {
        errors.push(ValidationError::new(
            "/audio_rate",
            "audio-rate",
            format!(
                "audio_rate {} does not divide the timebase {} exactly",
                doc.audio_rate, doc.timebase
            ),
        ));
    }
    if doc.audio.len() > MAX_AUDIO_TRACKS {
        errors.push(ValidationError::new(
            "/audio",
            "limit",
            format!("more than {MAX_AUDIO_TRACKS} audio tracks are not accepted"),
        ));
    }
    for (i, track) in doc.audio.iter().enumerate() {
        let path = format!("/audio/{i}");
        if !(0.0..=MAX_GAIN).contains(&track.gain) {
            errors.push(ValidationError::new(
                format!("{path}/gain"),
                "audio-gain",
                format!("gain must be within [0, {MAX_GAIN}]"),
            ));
        }
        if track.start_at < 0 {
            errors.push(ValidationError::new(
                format!("{path}/start_at"),
                "audio-placement",
                "start_at must be a non-negative number of ticks",
            ));
        }
        if let Some(range) = &track.source_range {
            push_source_range_error(range, &format!("{path}/source_range"), errors);
        }
        if track.fades.len() > MAX_FADES_PER_TRACK {
            errors.push(ValidationError::new(
                format!("{path}/fades"),
                "limit",
                format!("more than {MAX_FADES_PER_TRACK} fades per track are not accepted"),
            ));
        }
        for (j, fade) in track.fades.iter().enumerate() {
            let fade_path = format!("{path}/fades/{j}");
            if fade.at < 0 {
                errors.push(ValidationError::new(
                    format!("{fade_path}/at"),
                    "audio-fade",
                    "fade start must be a non-negative number of ticks",
                ));
            }
            if fade.len <= 0 {
                errors.push(ValidationError::new(
                    format!("{fade_path}/len"),
                    "audio-fade",
                    "fade length must be a positive number of ticks",
                ));
            }
        }
        match doc.assets.get(&track.asset) {
            Some(Asset::Audio { .. }) => {}
            Some(_) => errors.push(ValidationError::new(
                format!("{path}/asset"),
                "asset-type",
                format!("asset \"{}\" exists but is not an audio file", track.asset),
            )),
            None => errors.push(ValidationError::new(
                format!("{path}/asset"),
                "audio-asset",
                format!("audio track references unknown asset \"{}\"", track.asset),
            )),
        }
    }
}

/// Caption tracks (plan §12 v0.5): style sanity, cue ordering, and timing
/// inside the document. A cue past the end is an error, not a warning — it
/// could never appear on screen.
fn validate_captions(doc: &Document, errors: &mut Vec<ValidationError>) {
    if doc.captions.is_empty() {
        return;
    }
    if doc.captions.len() > MAX_CAPTION_TRACKS {
        errors.push(ValidationError::new(
            "/captions",
            "limit",
            format!("more than {MAX_CAPTION_TRACKS} caption tracks are not accepted"),
        ));
    }
    let duration = doc.total_duration();
    for (i, track) in doc.captions.iter().enumerate() {
        validate_caption_style(&track.style, &format!("/captions/{i}/style"), doc, errors);
        if track.cues.len() > MAX_CUES_PER_TRACK {
            errors.push(ValidationError::new(
                format!("/captions/{i}/cues"),
                "limit",
                format!("more than {MAX_CUES_PER_TRACK} cues per caption track are not accepted"),
            ));
        }
        for (j, cue) in track.cues.iter().enumerate() {
            let path = format!("/captions/{i}/cues/{j}");
            if cue.text.trim().is_empty() {
                errors.push(ValidationError::new(
                    format!("{path}/text"),
                    "caption-text",
                    "caption text must not be empty",
                ));
            }
            if cue.text.len() > MAX_TEXT_LEN {
                errors.push(ValidationError::new(
                    format!("{path}/text"),
                    "limit",
                    format!("caption text longer than {MAX_TEXT_LEN} bytes is not accepted"),
                ));
            }
            if cue.from < 0 {
                errors.push(ValidationError::new(
                    format!("{path}/from"),
                    "caption-timing",
                    "cue start must be a non-negative number of ticks",
                ));
            }
            if cue.to <= cue.from {
                errors.push(ValidationError::new(
                    format!("{path}/to"),
                    "caption-timing",
                    "cue end must be after its start (from < to)",
                ));
            }
            if cue.to > duration {
                errors.push(ValidationError::new(
                    format!("{path}/to"),
                    "caption-timing",
                    format!(
                        "cue ends at {} ticks but the document is only {duration} ticks long",
                        cue.to
                    ),
                ));
            }
            if j > 0 {
                let prev = &track.cues[j - 1];
                if cue.from < prev.from {
                    errors.push(ValidationError::new(
                        format!("{path}/from"),
                        "caption-order",
                        "cues in one track must be sorted by start tick",
                    ));
                } else if cue.from < prev.to {
                    errors.push(ValidationError::new(
                        format!("{path}/from"),
                        "caption-overlap",
                        format!(
                            "cues in one track must not overlap; the previous cue ends at {} ticks",
                            prev.to
                        ),
                    ));
                }
            }
        }
    }
}

fn validate_caption_style(
    style: &crate::caption::CaptionStyle,
    path: &str,
    doc: &Document,
    errors: &mut Vec<ValidationError>,
) {
    if style.size <= 0.0 || style.size > MAX_TEXT_SIZE {
        errors.push(ValidationError::new(
            format!("{path}/size"),
            "text-size",
            format!("font size must be in (0, {MAX_TEXT_SIZE}]"),
        ));
    }
    if style.margin < 0 {
        errors.push(ValidationError::new(
            format!("{path}/margin"),
            "caption-margin",
            "margin must be zero or a positive number of pixels",
        ));
    }
    push_color_error(&style.color, &format!("{path}/color"), errors);
    if let Some(s) = &style.shadow {
        validate_shadow(s, &format!("{path}/shadow"), errors);
    }
    if let Some(bg) = &style.bg {
        push_color_error(bg, &format!("{path}/bg"), errors);
    }
    match doc.assets.get(&style.font) {
        Some(Asset::Font { .. }) => {}
        Some(_) => errors.push(ValidationError::new(
            format!("{path}/font"),
            "asset-type",
            format!("asset \"{}\" exists but is not a font", style.font),
        )),
        None => errors.push(ValidationError::new(
            format!("{path}/font"),
            "font-asset",
            format!(
                "caption track references unknown font asset \"{}\"",
                style.font
            ),
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
    // Documents are platform-neutral, so these rules must not depend on the
    // host OS: `Path::is_absolute()` alone would accept "/etc/passwd" on
    // Windows (it lacks a drive prefix) and reject nothing that matters.
    // Leading `/` and `<letter>:` drive prefixes are banned by string shape.
    let has_drive_prefix =
        src.len() >= 2 && src.as_bytes()[0].is_ascii_alphabetic() && src.as_bytes()[1] == b':';
    if src.starts_with('/')
        || has_drive_prefix
        || candidate.is_absolute()
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
    const KNOWN_TYPES: [&str; 6] = ["text", "rect", "image", "video", "font", "audio"];
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
