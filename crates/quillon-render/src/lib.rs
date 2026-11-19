//! Deterministic CPU renderer for Quillon documents (plan §3 P2, §6.3).
//!
//! `render_tick` is a pure function of `(document, tick)`: no inter-frame
//! state, no wall clock, no randomness, fonts pinned to document assets.
//! Compositing happens in linear light; sRGB conversion happens once, at
//! output (research/T).

mod blend;
mod lut;
mod pipeline;
mod prepare;
mod text;
mod transform;
mod visuals;

pub use pipeline::{CacheGet, CachePut, FrameJob, render_document_parallel};

use blend::LinearCanvas;
use quillon_core::caption::CaptionPosition;
use quillon_core::document::{Document, RectElement, Scene, TextAlign};
use quillon_core::evaluate_anim;
use quillon_core::frame_source::{Frame, FrameSource};
use quillon_core::time::Tick;
use std::cell::RefCell;
use std::path::Path;
use transform::{Placement, composite};

/// Bumped when rendering logic changes pixels intentionally; part of the
/// cache key (research/G: content and renderer version are independent
/// invalidation axes) and the golden-refresh trigger (plan §14).
pub const RENDERER_VERSION: u32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("asset \"{id}\" cannot be used: {reason}")]
    Asset { id: String, reason: String },
    #[error("tick {tick} is outside the document (0..{end})")]
    TickOutOfRange { tick: i64, end: i64 },
    #[error("media source failed at source tick {tick}: {reason}")]
    Media { tick: i64, reason: String },
    #[error("cannot run the render pipeline: {0}")]
    Pool(String),
}

/// Map a scene-local tick to the source-media tick of a video element:
/// `from + local × speed`, clamped to the source window. `from == to`
/// (a freeze) pins the tick to `from`; speed and window are validated
/// positive and ordered by the document validator.
fn video_source_tick(el: &quillon_core::VideoElement, local: Tick) -> i64 {
    let from = el.source_range.map_or(0, |r| r.from);
    let raw = local as f64 * f64::from(el.speed) + from as f64;
    let tick = raw.round() as i64;
    match el.source_range {
        Some(range) => tick.clamp(range.from, range.to),
        None => tick.max(from),
    }
}

/// A prepared document: assets decoded, fonts loaded once.
pub struct Renderer {
    doc: Document,
    prepared: prepare::PreparedAssets,
    font_system: RefCell<cosmic_text::FontSystem>,
    swash: RefCell<cosmic_text::SwashCache>,
}

impl Renderer {
    /// Load every asset the document references. Fails before any pixel is
    /// rendered if an asset is unreadable, undecodable, or not a font.
    pub fn new(doc: &Document, base_dir: &Path) -> Result<Renderer, RenderError> {
        let prepared = prepare::prepare(doc, base_dir)?;
        let font_system = prepare::font_system(doc, base_dir)?;
        Ok(Renderer {
            doc: doc.clone(),
            prepared,
            font_system: RefCell::new(font_system),
            swash: RefCell::new(cosmic_text::SwashCache::new()),
        })
    }

    /// Render one frame. Pure with respect to the document and tick.
    ///
    /// Not `Sync`: the font caches are `RefCell` (per-worker instances once
    /// the v0.2 parallel pipeline builds one `Renderer` per rayon worker).
    pub fn render_tick(&self, tick: Tick, src: &dyn FrameSource) -> Result<Frame, RenderError> {
        let (scene_idx, local) = self.doc.scene_at(tick).ok_or(RenderError::TickOutOfRange {
            tick,
            end: self.doc.total_duration(),
        })?;
        let scene = &self.doc.scenes[scene_idx];
        let mut canvas = match scene.crossfade_in() {
            // During the head of a crossfading scene the previous scene keeps
            // playing (its tracks hold their final values past the end), and
            // the two frames blend per-pixel in premultiplied linear light.
            Some(duration) if scene_idx > 0 && local < duration => {
                let prev = &self.doc.scenes[scene_idx - 1];
                let over = self.render_scene_canvas(scene_idx, scene, local, src)?;
                let under =
                    self.render_scene_canvas(scene_idx - 1, prev, prev.duration + local, src)?;
                let p = local as f32 / duration as f32;
                blend_canvases(under, over, p)
            }
            _ => self.render_scene_canvas(scene_idx, scene, local, src)?,
        };
        // Captions burn in over the fully composited frame, so a cue is
        // never blended by a scene transition and always reads on top.
        self.draw_captions(&mut canvas, tick)?;
        Ok(Frame {
            width: self.doc.size.w,
            height: self.doc.size.h,
            rgba: canvas.into_rgba8(),
        })
    }

    /// Draw every caption track's active cue onto the canvas (plan §12 v0.5,
    /// design note D6). Documents without captions take an empty-loop no-op,
    /// so their pixels — and the golden corpus — are untouched.
    fn draw_captions(&self, canvas: &mut LinearCanvas, tick: Tick) -> Result<(), RenderError> {
        for track in &self.doc.captions {
            let Some(cue) = track.cue_at(tick) else {
                continue;
            };
            let style = &track.style;
            // Reuse the exact text rasterization path elements use; only the
            // origin differs (derived from the position, not an author x/y).
            let el = quillon_core::TextElement {
                text: cue.text.clone(),
                font: style.font.clone(),
                x: 0,
                y: 0,
                size: style.size,
                color: style.color.clone(),
                align: style.align,
                shadow: style.shadow.clone(),
                clip: None,
                anim: None,
            };
            let layer = self.rasterize_text(&el)?;
            let (w, h) = (self.doc.size.w as f32, self.doc.size.h as f32);
            let y = match style.position {
                CaptionPosition::Bottom => h - style.margin as f32 - layer.height as f32,
                CaptionPosition::Top => style.margin as f32,
                CaptionPosition::Center => (h - layer.height as f32) / 2.0,
            };
            let x = match style.align {
                TextAlign::Center => (w - layer.width as f32) / 2.0,
                TextAlign::Left => style.margin as f32,
                TextAlign::Right => w - style.margin as f32 - layer.width as f32,
            };
            if let Some(bg) = &style.bg {
                // Backing box: the text block plus padding derived from the
                // font size (0.35 em horizontal, 0.15 em vertical).
                let (pad_x, pad_y) = (style.size * 0.35, style.size * 0.15);
                let rect = RectElement {
                    x: 0,
                    y: 0,
                    w: (layer.width as f32 + 2.0 * pad_x).round() as i64,
                    h: (layer.height as f32 + 2.0 * pad_y).round() as i64,
                    color: bg.clone(),
                    gradient: None,
                    radius: None,
                    shadow: None,
                    clip: None,
                    anim: None,
                };
                let box_layer = rasterize_rect(&rect)?;
                composite(
                    canvas,
                    &box_layer,
                    Placement {
                        origin: (x - pad_x, y - pad_y),
                        ..default_placement()
                    },
                );
            }
            let state = evaluate_anim(None, 0);
            composite_layer(
                canvas,
                &layer,
                placement((x, y), &state),
                style.shadow.as_ref(),
                None,
                (0.0, 0.0),
            );
        }
        Ok(())
    }

    /// Rasterize one scene at a scene-local tick onto a fresh canvas.
    fn render_scene_canvas(
        &self,
        scene_idx: usize,
        scene: &Scene,
        local: Tick,
        src: &dyn FrameSource,
    ) -> Result<LinearCanvas, RenderError> {
        let (w, h) = (self.doc.size.w, self.doc.size.h);
        let bg = self.doc.bg.rgb8();
        let mut canvas = LinearCanvas::filled_with(w, h, [bg.0, bg.1, bg.2]);
        for (ei, element) in scene.elements.iter().enumerate() {
            self.draw_element(&mut canvas, scene_idx, ei, element, local, src)?;
        }
        Ok(canvas)
    }

    /// Rasterize one element and composite it, with its optional shadow
    /// under it and its optional clip applied to both. Rect and text layers
    /// are built per frame; image layers come from the prepared assets.
    fn draw_element(
        &self,
        canvas: &mut LinearCanvas,
        scene_idx: usize,
        ei: usize,
        element: &quillon_core::Element,
        local: Tick,
        src: &dyn FrameSource,
    ) -> Result<(), RenderError> {
        let state = evaluate_anim(element.anim(), local);
        let place = |origin: (f32, f32)| placement(origin, &state);
        match element {
            quillon_core::Element::Rect(el) => {
                let layer = rasterize_rect(el)?;
                let place = place((el.x as f32, el.y as f32));
                composite_layer(
                    canvas,
                    &layer,
                    place,
                    el.shadow.as_ref(),
                    el.clip,
                    (0.0, 0.0),
                );
            }
            quillon_core::Element::Video(el) => {
                let source_tick = video_source_tick(el, local);
                let frame =
                    src.frame_at(&el.asset, source_tick)
                        .map_err(|e| RenderError::Media {
                            tick: source_tick,
                            reason: e.to_string(),
                        })?;
                let content = prepare::fit_frame(&frame, el.w, el.h, el.fit);
                let origin = (
                    (el.x + content.offset.0) as f32,
                    (el.y + content.offset.1) as f32,
                );
                composite_layer(
                    canvas,
                    &content.layer,
                    place(origin),
                    el.shadow.as_ref(),
                    el.clip,
                    (0.0, 0.0),
                );
            }
            quillon_core::Element::Image(el) => {
                let content = self.prepared_image(scene_idx, ei, el)?;
                let origin = (
                    (el.x + content.offset.0) as f32,
                    (el.y + content.offset.1) as f32,
                );
                composite_layer(
                    canvas,
                    &content.layer,
                    place(origin),
                    el.shadow.as_ref(),
                    el.clip,
                    (0.0, 0.0),
                );
            }
            quillon_core::Element::Text(el) => {
                let layer = self.rasterize_text(el)?;
                // Text clips are authored around the text center; the
                // layer's (0,0) is its top-left corner.
                let half = (layer.width as f32 / 2.0, layer.height as f32 / 2.0);
                let origin = (
                    el.x as f32 - layer.width as f32 / 2.0,
                    el.y as f32 - layer.height as f32 / 2.0,
                );
                composite_layer(
                    canvas,
                    &layer,
                    place(origin),
                    el.shadow.as_ref(),
                    el.clip,
                    half,
                );
            }
        }
        Ok(())
    }

    fn prepared_image(
        &self,
        scene_idx: usize,
        ei: usize,
        el: &quillon_core::ImageElement,
    ) -> Result<&prepare::ImageContent, RenderError> {
        self.prepared
            .images
            .get(&(scene_idx, ei))
            .ok_or_else(|| RenderError::Asset {
                id: el.asset.clone(),
                reason: "image content was not prepared".into(),
            })
    }

    fn rasterize_text(
        &self,
        el: &quillon_core::TextElement,
    ) -> Result<blend::RgbaLayer, RenderError> {
        let family = self
            .prepared
            .fonts
            .get(&el.font)
            .ok_or_else(|| RenderError::Asset {
                id: el.font.clone(),
                reason: "font was not prepared".into(),
            })?;
        let mut font_system = self.font_system.borrow_mut();
        let mut swash = self.swash.borrow_mut();
        Ok(text::rasterize(&mut font_system, &mut swash, el, family))
    }
}

/// Build the placement of an element from its evaluated animation state.
fn placement(origin: (f32, f32), state: &quillon_core::ElementState) -> Placement {
    Placement {
        origin,
        translate: (state.translate_x, state.translate_y),
        scale: state.scale,
        rotate_deg: state.rotate_deg,
        opacity: state.opacity,
    }
}

/// An unanimated placement for layers that are positioned directly (the
/// caption backing box).
fn default_placement() -> Placement {
    Placement {
        origin: (0.0, 0.0),
        translate: (0.0, 0.0),
        scale: 1.0,
        rotate_deg: 0.0,
        opacity: 1.0,
    }
}

/// Composite an element layer under its placement, applying the optional
/// clip to the element and to its shadow (the blur must not spread past the
/// clip), and drawing the shadow first so the element lands on top of it.
fn composite_layer(
    canvas: &mut LinearCanvas,
    layer: &blend::RgbaLayer,
    place: Placement,
    shadow: Option<&quillon_core::document::ShadowDef>,
    clip: Option<quillon_core::document::ClipRect>,
    clip_offset: (f32, f32),
) {
    match clip {
        None => draw_with_shadow(canvas, layer, place, shadow, None),
        Some(c) => {
            let mut masked = layer.clone();
            visuals::apply_clip(&mut masked, c, clip_offset);
            draw_with_shadow(canvas, &masked, place, shadow, Some((c, clip_offset)));
        }
    }
}

/// Draw the shadow (when present) and then the element. When a clip is
/// active, the shadow buffer — whose origin differs from the element
/// layer's — is masked with the clip translated into its coordinates.
fn draw_with_shadow(
    canvas: &mut LinearCanvas,
    layer: &blend::RgbaLayer,
    place: Placement,
    shadow: Option<&quillon_core::document::ShadowDef>,
    clip: Option<(quillon_core::document::ClipRect, (f32, f32))>,
) {
    if let Some(s) = shadow {
        let (mut shadow_layer, origin_offset) = visuals::shadow_layer(layer, s);
        if let Some((c, off)) = clip {
            visuals::apply_clip(
                &mut shadow_layer,
                c,
                (off.0 - origin_offset.0, off.1 - origin_offset.1),
            );
        }
        let mut shadow_place = place;
        shadow_place.origin = (
            place.origin.0 + origin_offset.0,
            place.origin.1 + origin_offset.1,
        );
        composite(canvas, &shadow_layer, shadow_place);
    }
    composite(canvas, layer, place);
}

/// Crossfade blend of two complete frames in premultiplied linear light:
/// `out = (1-p)·under + p·over`. Both canvases are opaque (the document
/// background fills them), so alpha stays 1 and the blend is a plain
/// weighted sum per channel (research/T: the gamma-correct crossfade).
fn blend_canvases(under: LinearCanvas, over: LinearCanvas, p: f32) -> LinearCanvas {
    let keep = 1.0 - p;
    let mut px = under.px;
    for (dst, src) in px.iter_mut().zip(over.px.iter()) {
        *dst = *dst * keep + *src * p;
    }
    LinearCanvas {
        width: under.width,
        height: under.height,
        px,
    }
}

/// The render contract entry point (plan §6.3). Prepares assets on every
/// call; callers rendering more than one frame should hold a [`Renderer`].
pub fn render_tick(
    doc: &Document,
    tick: Tick,
    src: &dyn FrameSource,
    base_dir: &Path,
) -> Result<Frame, RenderError> {
    let renderer = Renderer::new(doc, base_dir)?;
    renderer.render_tick(tick, src)
}

/// Rasterize a rect at rest through tiny-skia (analytic edge
/// anti-aliasing); transforms are applied by the shared placement
/// compositor. Solid fills keep the exact v0.1 pixel path (golden-hash
/// stability); gradients rasterize the shape as an alpha mask and color it
/// in linear light (research/T).
fn rasterize_rect(el: &RectElement) -> Result<blend::RgbaLayer, RenderError> {
    let (w, h) = (el.w.max(1) as u32, el.h.max(1) as u32);
    let path = match el.radius {
        Some(r) if r > 0.0 => visuals::rounded_rect_path(el.w as f32, el.h as f32, r),
        _ => tiny_skia::Rect::from_xywh(0.0, 0.0, el.w as f32, el.h as f32)
            .map(tiny_skia::PathBuilder::from_rect),
    }
    .ok_or_else(|| RenderError::Asset {
        id: "rect".into(),
        reason: "degenerate rectangle".into(),
    })?;
    if let Some(g) = &el.gradient {
        return Ok(visuals::fill_gradient_mask(&path, g, w, h));
    }
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| RenderError::Asset {
        id: "rect".into(),
        reason: "rectangle too large to rasterize".into(),
    })?;
    let (r, g, b) = el.color.rgb8();
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(r, g, b, 255);
    pixmap.fill_path(
        &path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );

    // tiny-skia stores premultiplied sRGB; the compositor wants straight.
    let mut data = pixmap.data().to_vec();
    for px in data.chunks_exact_mut(4) {
        let a = px[3];
        if a == 0 || a == 255 {
            continue;
        }
        for c in px.iter_mut().take(3) {
            *c = (u16::from(*c) * 255 / u16::from(a)).min(255) as u8;
        }
    }
    Ok(blend::RgbaLayer {
        width: w,
        height: h,
        data,
    })
}
