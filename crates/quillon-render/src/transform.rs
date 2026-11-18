//! Element placement: identity blit and affine sampling.
//!
//! The animated transform maps layer coordinates to canvas coordinates:
//! scale around the content center, then rotate around it, then translate.
//! Sampling happens in linear light (the sRGB->linear LUT is applied to the
//! four bilinear taps before interpolation, so averaging itself is linear —
//! research/T). Elements without animation take the exact blit path and are
//! never resampled.

use crate::blend::{LinearCanvas, RgbaLayer};
use crate::lut::srgb8_to_linear;

/// The evaluated placement of one element.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Layer origin (top-left) on the canvas before animation.
    pub origin: (f32, f32),
    pub translate: (f32, f32),
    pub scale: f32,
    /// Degrees, clockwise.
    pub rotate_deg: f32,
    pub opacity: f32,
}

impl Placement {
    pub fn is_identity(&self) -> bool {
        self.translate == (0.0, 0.0) && self.scale == 1.0 && self.rotate_deg == 0.0
    }
}

/// Composite a layer onto the canvas under its placement.
pub fn composite(canvas: &mut LinearCanvas, layer: &RgbaLayer, place: Placement) {
    if place.opacity <= 0.0 {
        return;
    }
    if place.is_identity() {
        blit(canvas, layer, place);
    } else {
        affine(canvas, layer, place);
    }
}

/// Exact path: element at rest. Alpha still composites (opacity may be < 1).
fn blit(canvas: &mut LinearCanvas, layer: &RgbaLayer, place: Placement) {
    let origin_x = place.origin.0.round() as i64;
    let origin_y = place.origin.1.round() as i64;
    for y in 0..layer.height as i64 {
        for x in 0..layer.width as i64 {
            let (cx, cy) = (origin_x + x, origin_y + y);
            if cx < 0 || cy < 0 || cx >= canvas.width as i64 || cy >= canvas.height as i64 {
                continue;
            }
            let rgba = layer.pixel(x as u32, y as u32);
            if rgba[3] == 0 {
                continue;
            }
            canvas.blend_straight_pixel(cx as u32, cy as u32, rgba, place.opacity);
        }
    }
}

/// The scale-rotate-translate mapping of one element, in both directions.
struct Affine {
    /// Content center on the canvas, after translation.
    pivot: (f32, f32),
    scale: f32,
    sin: f32,
    cos: f32,
}

impl Affine {
    fn new(place: &Placement, layer_size: (f32, f32)) -> Affine {
        let center = (
            place.origin.0 + layer_size.0 / 2.0,
            place.origin.1 + layer_size.1 / 2.0,
        );
        let rad = place.rotate_deg.to_radians();
        Affine {
            pivot: (center.0 + place.translate.0, center.1 + place.translate.1),
            scale: place.scale,
            sin: rad.sin(),
            cos: rad.cos(),
        }
    }

    /// Layer point -> canvas point.
    fn forward(&self, x: f32, y: f32, layer_size: (f32, f32)) -> (f32, f32) {
        let dx = (x - layer_size.0 / 2.0) * self.scale;
        let dy = (y - layer_size.1 / 2.0) * self.scale;
        (
            dx * self.cos - dy * self.sin + self.pivot.0,
            dx * self.sin + dy * self.cos + self.pivot.1,
        )
    }

    /// Canvas point -> layer point.
    fn inverse(&self, cx: f32, cy: f32, layer_size: (f32, f32)) -> (f32, f32) {
        let vx = cx - self.pivot.0;
        let vy = cy - self.pivot.1;
        let ux = (vx * self.cos + vy * self.sin) / self.scale;
        let uy = (-vx * self.sin + vy * self.cos) / self.scale;
        (ux + layer_size.0 / 2.0, uy + layer_size.1 / 2.0)
    }
}

/// Transform path: scale/rotate/subpixel translate, bilinear in linear light.
fn affine(canvas: &mut LinearCanvas, layer: &RgbaLayer, place: Placement) {
    let layer_size = (layer.width as f32, layer.height as f32);
    let ctx = Affine::new(&place, layer_size);
    let bbox = layer_bbox(&ctx, layer_size);
    for cy in bbox.1..=bbox.3 {
        for cx in bbox.0..=bbox.2 {
            if cx < 0 || cy < 0 || cx >= canvas.width as i32 || cy >= canvas.height as i32 {
                continue;
            }
            if let Some(sample) = sample(layer, &ctx, layer_size, cx, cy) {
                canvas.blend_linear_premult(cx as u32, cy as u32, sample.rgb, sample.a);
            }
        }
    }
}

struct Sample {
    rgb: [f32; 3],
    a: f32,
}

/// Inverse-map one canvas pixel into layer space and bilinear-sample it.
fn sample(
    layer: &RgbaLayer,
    ctx: &Affine,
    layer_size: (f32, f32),
    cx: i32,
    cy: i32,
) -> Option<Sample> {
    let (lx, ly) = ctx.inverse(cx as f32, cy as f32, layer_size);
    let (x0, y0) = (lx.floor() as i32, ly.floor() as i32);
    let (w, h) = (layer.width as i32, layer.height as i32);
    if x0 < -1 || y0 < -1 || x0 > w || y0 > h {
        return None;
    }
    let (fx, fy) = (lx - x0 as f32, ly - y0 as f32);
    let top = lerp4(tap(layer, x0, y0), tap(layer, x0 + 1, y0), fx);
    let bottom = lerp4(tap(layer, x0, y0 + 1), tap(layer, x0 + 1, y0 + 1), fx);
    let c = lerp4(top, bottom, fy);
    if c[3] <= 0.0 {
        return None;
    }
    Some(Sample {
        rgb: [c[0], c[1], c[2]],
        a: c[3],
    })
}

/// One bilinear tap, converted to straight linear light.
fn tap(layer: &RgbaLayer, x: i32, y: i32) -> [f32; 4] {
    let (w, h) = (layer.width as i32, layer.height as i32);
    if x < 0 || y < 0 || x >= w || y >= h {
        return [0.0; 4];
    }
    let p = layer.pixel(x as u32, y as u32);
    [
        srgb8_to_linear(p[0]),
        srgb8_to_linear(p[1]),
        srgb8_to_linear(p[2]),
        f32::from(p[3]) / 255.0,
    ]
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
        a[3] + (b[3] - a[3]) * t,
    ]
}

/// Bounding box of the transformed layer on the canvas, as integers.
fn layer_bbox(ctx: &Affine, layer_size: (f32, f32)) -> (i32, i32, i32, i32) {
    let (lw, lh) = layer_size;
    let corners = [(0.0, 0.0), (lw, 0.0), (0.0, lh), (lw, lh)];
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for (dx, dy) in corners {
        let (rx, ry) = ctx.forward(dx, dy, layer_size);
        min_x = min_x.min(rx);
        max_x = max_x.max(rx);
        min_y = min_y.min(ry);
        max_y = max_y.max(ry);
    }
    (
        min_x.floor() as i32,
        min_y.floor() as i32,
        max_x.ceil() as i32,
        max_y.ceil() as i32,
    )
}
