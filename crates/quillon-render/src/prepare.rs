//! Asset preparation, done once per render: image decode + fit, font load.
//! Font determinism (plan §3 P5, §9.4): the FontSystem contains ONLY the
//! document's fonts — no system font fallback can leak into frames.

use crate::RenderError;
use crate::blend::RgbaLayer;
use image::imageops::{FilterType, crop_imm, resize};
use quillon_core::document::{Asset, Document, Fit};
use quillon_core::frame_source::Frame;
use std::collections::BTreeMap;
use std::path::Path;

/// A fitted image: the pixels that land on the canvas and their offset
/// within the element's destination box.
pub struct ImageContent {
    pub layer: RgbaLayer,
    pub offset: (i64, i64),
}

pub struct PreparedAssets {
    /// Per image element, keyed by (scene index, element index): the fitted
    /// content. Keyed lookups only — no iteration order reaches the output.
    pub images: BTreeMap<(usize, usize), ImageContent>,
    /// Font asset id -> family name for shaping.
    pub fonts: BTreeMap<String, String>,
}

/// Fixed resampling filter: deterministic and documented (changing it shifts
/// golden hashes — RENDERER_VERSION territory).
const RESIZE_FILTER: FilterType = FilterType::CatmullRom;

pub fn prepare(doc: &Document, base_dir: &Path) -> Result<PreparedAssets, RenderError> {
    let mut fonts = BTreeMap::new();
    let mut decoded: BTreeMap<String, image::RgbaImage> = BTreeMap::new();
    for (id, asset) in &doc.assets {
        match asset {
            Asset::Image { .. } => {
                let bytes = read_asset(base_dir, asset.src(), id)?;
                let img = image::load_from_memory(&bytes).map_err(|e| RenderError::Asset {
                    id: id.clone(),
                    reason: e.to_string(),
                })?;
                decoded.insert(id.clone(), img.to_rgba8());
            }
            Asset::Font { .. } => {
                let bytes = read_asset(base_dir, asset.src(), id)?;
                fonts.insert(id.clone(), probe_family_name(&bytes, id)?);
            }
            // Media assets are not decoded here: video frames arrive through
            // the document's FrameSource at render time (plan §6.2 — render
            // must not depend on media), audio through the audio pipeline.
            // Validation has already checked their existence.
            Asset::Video { .. } | Asset::Audio { .. } => {}
        }
    }

    let mut images = BTreeMap::new();
    for (si, scene) in doc.scenes.iter().enumerate() {
        for (ei, element) in scene.elements.iter().enumerate() {
            if let quillon_core::Element::Image(el) = element {
                let source = decoded.get(&el.asset).ok_or_else(|| RenderError::Asset {
                    id: el.asset.clone(),
                    reason: "not an image asset".into(),
                })?;
                let content = fit_image(source, el.w, el.h, el.fit);
                images.insert((si, ei), content);
            }
        }
    }

    Ok(PreparedAssets { images, fonts })
}

/// Build the deterministic FontSystem: only the document's fonts, no system
/// fonts, no locale-dependent scanning.
pub fn font_system(
    doc: &Document,
    base_dir: &Path,
) -> Result<cosmic_text::FontSystem, RenderError> {
    let mut db = cosmic_text::fontdb::Database::new();
    for (id, asset) in &doc.assets {
        if let Asset::Font { .. } = asset {
            let bytes =
                std::fs::read(base_dir.join(asset.src())).map_err(|e| RenderError::Asset {
                    id: id.clone(),
                    reason: e.to_string(),
                })?;
            db.load_font_data(bytes);
        }
    }
    Ok(cosmic_text::FontSystem::new_with_locale_and_db(
        "en".to_string(),
        db,
    ))
}

/// Read the family name out of a font file with a throwaway database.
fn probe_family_name(bytes: &[u8], id: &str) -> Result<String, RenderError> {
    let mut db = cosmic_text::fontdb::Database::new();
    db.load_font_data(bytes.to_vec());
    let face = db.faces().next().ok_or_else(|| RenderError::Asset {
        id: id.to_string(),
        reason: "no font face found in file".into(),
    })?;
    // First family is English US per fontdb's FaceInfo contract.
    face.families
        .first()
        .map(|(name, _)| name.clone())
        .ok_or_else(|| RenderError::Asset {
            id: id.to_string(),
            reason: "font has no family name".into(),
        })
}

/// Read an asset file under `base_dir`.
fn read_asset(base_dir: &Path, src: &str, id: &str) -> Result<Vec<u8>, RenderError> {
    std::fs::read(base_dir.join(src)).map_err(|e| RenderError::Asset {
        id: id.to_string(),
        reason: e.to_string(),
    })
}

/// Fit a decoded video frame into a destination box: the same math, filter,
/// and offset contract as image content, so both element kinds place their
/// pixels identically.
pub fn fit_frame(source: &Frame, box_w: i64, box_h: i64, fit: Fit) -> ImageContent {
    let img = image::RgbaImage::from_raw(source.width, source.height, source.rgba.clone())
        .expect("frame buffer length matches its dimensions");
    fit_image(&img, box_w, box_h, fit)
}

/// Fit a decoded image into a destination box.
fn fit_image(source: &image::RgbaImage, box_w: i64, box_h: i64, fit: Fit) -> ImageContent {
    let (sw, sh) = (source.width(), source.height());
    let (bw, bh) = (box_w.max(1) as u32, box_h.max(1) as u32);
    let (fw, fh) = match fit {
        Fit::Stretch => (bw, bh),
        Fit::Contain => fit_within(sw, sh, bw, bh),
        Fit::Cover => cover_dims(sw, sh, bw, bh),
    };
    let resized = if (fw, fh) == (sw, sh) {
        source.clone()
    } else {
        resize(source, fw, fh, RESIZE_FILTER)
    };
    // Contain leaves empty space around the content; cover overshoots and
    // needs a center crop back to the box.
    let (final_img, offset) = match fit {
        Fit::Contain => (resized, centered_offset(fw, fh, bw, bh)),
        Fit::Cover => {
            let crop = crop_imm(&resized, (fw - bw) / 2, (fh - bh) / 2, bw, bh);
            (crop.to_image(), (0, 0))
        }
        Fit::Stretch => (resized, (0, 0)),
    };
    ImageContent {
        layer: rgba_image_to_layer(final_img),
        offset,
    }
}

fn fit_within(sw: u32, sh: u32, bw: u32, bh: u32) -> (u32, u32) {
    if sw == 0 || sh == 0 {
        return (bw.max(1), bh.max(1));
    }
    let scale = (f64::from(bw) / f64::from(sw)).min(f64::from(bh) / f64::from(sh));
    (
        (f64::from(sw) * scale).round() as u32,
        (f64::from(sh) * scale).round() as u32,
    )
}

fn cover_dims(sw: u32, sh: u32, bw: u32, bh: u32) -> (u32, u32) {
    if sw == 0 || sh == 0 {
        return (bw.max(1), bh.max(1));
    }
    let scale = (f64::from(bw) / f64::from(sw)).max(f64::from(bh) / f64::from(sh));
    (
        (f64::from(sw) * scale).round() as u32,
        (f64::from(sh) * scale).round() as u32,
    )
}

fn centered_offset(w: u32, h: u32, bw: u32, bh: u32) -> (i64, i64) {
    (
        (bw.saturating_sub(w)) as i64 / 2,
        (bh.saturating_sub(h)) as i64 / 2,
    )
}

fn rgba_image_to_layer(img: image::RgbaImage) -> RgbaLayer {
    let (w, h) = img.dimensions();
    RgbaLayer {
        width: w,
        height: h,
        data: img.into_raw(),
    }
}
