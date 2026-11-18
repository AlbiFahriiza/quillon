//! Text rasterization through cosmic-text with document-pinned fonts only.
//!
//! The element's (x, y) is the CENTER of the laid-out block (plan §5.1:
//! title at 960,540 on 1920x1080). This module returns the block layer and
//! its measured size; the caller computes the top-left origin.

use crate::blend::RgbaLayer;
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap};
use quillon_core::document::{TextAlign, TextElement};

/// Shape and rasterize one text element into a straight-alpha layer.
pub fn rasterize(
    font_system: &mut FontSystem,
    swash: &mut SwashCache,
    el: &TextElement,
    family: &str,
) -> RgbaLayer {
    let line_height = el.size * 1.2;
    let mut buffer = Buffer::new(font_system, Metrics::new(el.size, line_height));
    let attrs = Attrs::new().family(Family::Name(family));
    buffer.set_text(&el.text, &attrs, Shaping::Advanced, None);
    buffer.set_wrap(Wrap::None);
    buffer.set_size(None, None);
    buffer.shape_until_scroll(font_system, false);

    let measurements = measure(&buffer);
    let layer_w = measurements.width.max(1.0).ceil() as u32;
    let layer_h = measurements.height.max(1.0).ceil() as u32;
    let mut layer = RgbaLayer::new(layer_w, layer_h);

    let (r, g, b) = el.color.rgb8();
    let ink = Color::rgba(r, g, b, 255);
    for run in buffer.layout_runs() {
        let line_off = line_offset(el.align, measurements.width, run.line_w);
        for glyph in run.glyphs {
            let physical = glyph.physical((0.0, run.line_y), 1.0);
            swash.with_pixels(font_system, physical.cache_key, ink, |dx, dy, pixel| {
                layer.set(
                    line_off + physical.x + dx,
                    physical.y + dy,
                    color_to_array(pixel),
                );
            });
        }
    }
    layer
}

struct Measured {
    width: f32,
    height: f32,
}

fn measure(buffer: &Buffer) -> Measured {
    let mut width = 0.0f32;
    let mut height = 0.0f32;
    for run in buffer.layout_runs() {
        width = width.max(run.line_w);
        height = run.line_top + run.line_height;
    }
    Measured { width, height }
}

fn line_offset(align: TextAlign, block_width: f32, line_width: f32) -> i32 {
    match align {
        TextAlign::Left => 0,
        TextAlign::Center => ((block_width - line_width) / 2.0).round() as i32,
        TextAlign::Right => (block_width - line_width).round() as i32,
    }
}

/// cosmic-text packs Color as (a << 24) | (r << 16) | (g << 8) | b.
fn color_to_array(c: Color) -> [u8; 4] {
    let bits = c.0;
    [
        ((bits >> 16) & 0xff) as u8,
        ((bits >> 8) & 0xff) as u8,
        (bits & 0xff) as u8,
        ((bits >> 24) & 0xff) as u8,
    ]
}
