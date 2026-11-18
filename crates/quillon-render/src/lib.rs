//! Deterministic CPU renderer for Quillon documents (plan §3 P2, §6.3).
//!
//! `render_tick` is a pure function of `(document, tick)`: no inter-frame
//! state, no wall clock, no randomness, fonts pinned to document assets.
//! Compositing happens in linear light; sRGB conversion happens once, at
//! output (research/T).

mod blend;
mod lut;
mod prepare;
mod text;
mod transform;

use blend::{LinearCanvas, RgbaLayer};
use quillon_core::document::{Document, RectElement};
use quillon_core::evaluate_anim;
use quillon_core::frame_source::{Frame, FrameSource};
use quillon_core::time::Tick;
use std::cell::RefCell;
use std::path::Path;
use transform::{Placement, composite};

/// Bumped when rendering logic changes pixels intentionally; part of the
/// cache key in later phases and the golden-refresh trigger (plan §14).
pub const RENDERER_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("asset \"{id}\" cannot be used: {reason}")]
    Asset { id: String, reason: String },
    #[error("tick {tick} is outside the document (0..{end})")]
    TickOutOfRange { tick: i64, end: i64 },
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
    /// Not `Sync`: the font caches are `RefCell` (single-threaded v0.1;
    /// parallel rendering arrives in v0.2 with per-worker caches).
    pub fn render_tick(&self, tick: Tick, _src: &dyn FrameSource) -> Result<Frame, RenderError> {
        let (scene_idx, local) = self.doc.scene_at(tick).ok_or(RenderError::TickOutOfRange {
            tick,
            end: self.doc.total_duration(),
        })?;
        let scene = &self.doc.scenes[scene_idx];
        let (w, h) = (self.doc.size.w, self.doc.size.h);
        let bg = self.doc.bg.rgb8();
        let mut canvas = LinearCanvas::filled_with(w, h, [bg.0, bg.1, bg.2]);

        for (ei, element) in scene.elements.iter().enumerate() {
            self.draw_element(&mut canvas, scene_idx, ei, element, local)?;
        }

        Ok(Frame {
            width: w,
            height: h,
            rgba: canvas.into_rgba8(),
        })
    }

    /// Rasterize one element and composite it. Rect and text layers are
    /// built per frame; image layers come from the prepared assets.
    fn draw_element(
        &self,
        canvas: &mut LinearCanvas,
        scene_idx: usize,
        ei: usize,
        element: &quillon_core::Element,
        local: Tick,
    ) -> Result<(), RenderError> {
        let state = evaluate_anim(element.anim(), local);
        let place = |origin: (f32, f32)| placement(origin, &state);
        match element {
            quillon_core::Element::Rect(el) => {
                let layer = rasterize_rect(el)?;
                composite(canvas, &layer, place((el.x as f32, el.y as f32)));
            }
            quillon_core::Element::Image(el) => {
                let content = self.prepared_image(scene_idx, ei, el)?;
                let origin = (
                    (el.x + content.offset.0) as f32,
                    (el.y + content.offset.1) as f32,
                );
                composite(canvas, &content.layer, place(origin));
            }
            quillon_core::Element::Text(el) => {
                let layer = self.rasterize_text(el)?;
                let origin = (
                    el.x as f32 - layer.width as f32 / 2.0,
                    el.y as f32 - layer.height as f32 / 2.0,
                );
                composite(canvas, &layer, place(origin));
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

    fn rasterize_text(&self, el: &quillon_core::TextElement) -> Result<RgbaLayer, RenderError> {
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

/// Rasterize a rect at rest through tiny-skia (analytic edge anti-aliasing);
/// transforms are applied by the shared placement compositor.
fn rasterize_rect(el: &RectElement) -> Result<RgbaLayer, RenderError> {
    let (w, h) = (el.w.max(1) as u32, el.h.max(1) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or_else(|| RenderError::Asset {
        id: "rect".into(),
        reason: "rectangle too large to rasterize".into(),
    })?;
    let rect = tiny_skia::Rect::from_xywh(0.0, 0.0, el.w as f32, el.h as f32).ok_or_else(|| {
        RenderError::Asset {
            id: "rect".into(),
            reason: "degenerate rectangle".into(),
        }
    })?;
    let path = tiny_skia::PathBuilder::from_rect(rect);
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
    Ok(RgbaLayer {
        width: w,
        height: h,
        data,
    })
}
