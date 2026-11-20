//! v0.2 element visuals: gradients, rounded corners, shadows, clipping
//! (myquillonplan.md §12 v0.2). All color math is linear-light (research/T):
//! gradient stops and shadow colors are converted to linear before any
//! interpolation or blur, and encoded back to sRGB exactly once per pixel.

use crate::blend::RgbaLayer;
use crate::lut::{linear_f32_to_srgb8, srgb8_to_linear};
use quillon_core::color::Color;
use quillon_core::document::{ClipRect, GradientDef, ShadowDef};

/// The cubic-approximation constant for a quarter circle: the control-point
/// offset that makes one cubic Bézier reproduce a 90° arc.
const KAPPA: f32 = 0.552_284_7;

/// Hard cap on shadow buffer pixels (premultiplied linear f32 costs 16 bytes
/// per pixel); blur is clamped to fit so a huge layer cannot request
/// gigabytes. The clamp is a pure function of the sizes, so renders stay
/// reproducible.
const MAX_SHADOW_PIXELS: u64 = 1 << 25; // 32M px

fn rgb_linear(color: &Color) -> [f32; 3] {
    let (r, g, b) = color.rgb8();
    [srgb8_to_linear(r), srgb8_to_linear(g), srgb8_to_linear(b)]
}

/// A rounded-rect path; radius clamped to half the shorter side (what a
/// designer expects of an over-large radius instead of a broken shape).
pub fn rounded_rect_path(w: f32, h: f32, radius: f32) -> Option<tiny_skia::Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = radius.clamp(0.0, w.min(h) / 2.0);
    if r <= 0.0 {
        let rect = tiny_skia::Rect::from_xywh(0.0, 0.0, w, h)?;
        return Some(tiny_skia::PathBuilder::from_rect(rect));
    }
    let k = KAPPA * r;
    let mut b = tiny_skia::PathBuilder::new();
    b.move_to(r, 0.0);
    b.line_to(w - r, 0.0);
    b.cubic_to(w - r + k, 0.0, w, r - k, w, r);
    b.line_to(w, h - r);
    b.cubic_to(w, h - r + k, w - r + k, h, w - r, h);
    b.line_to(r, h);
    b.cubic_to(r - k, h, 0.0, h - r + k, 0.0, h - r);
    b.line_to(0.0, r);
    b.cubic_to(0.0, r - k, r - k, 0.0, r, 0.0);
    b.close();
    b.finish()
}

/// Projection range of a `(w, h)` box onto the gradient axis, measured at
/// pixel centers: the axis runs from the first pixel center to the last, so
/// the first and last pixels land exactly on the first and last stops.
/// `angle` is degrees clockwise in screen space: 0 is left-to-right, 90
/// top-to-bottom.
fn axis_range(w: f32, h: f32, angle_deg: f32) -> (f32, f32) {
    let rad = angle_deg.to_radians();
    let (cos, sin) = (rad.cos(), rad.sin());
    let (x0, x1) = (0.5, w - 0.5);
    let (y0, y1) = (0.5, h - 0.5);
    let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
    let mut min = f32::MAX;
    let mut max = f32::MIN;
    for (x, y) in corners {
        let p = x * cos + y * sin;
        min = min.min(p);
        max = max.max(p);
    }
    (min, max)
}

/// Normalized position of pixel `(x, y)` (sampled at its center) along the
/// gradient axis, in [0, 1].
fn axis_position(x: f32, y: f32, angle_deg: f32, (min, max): (f32, f32)) -> f32 {
    let rad = angle_deg.to_radians();
    let p = (x + 0.5) * rad.cos() + (y + 0.5) * rad.sin();
    if max <= min {
        return 0.0;
    }
    ((p - min) / (max - min)).clamp(0.0, 1.0)
}

/// Gradient color at `t`, stops interpolated in linear light and encoded to
/// sRGB once (interpolating gamma-encoded values is the muddy-gradient bug,
/// research/T).
pub fn gradient_srgb8(g: &GradientDef, t: f32) -> [u8; 3] {
    let stops = &g.stops;
    // Validation guarantees >= 2 strictly increasing stops; clamp defensively
    // so a bad document can still never panic the renderer.
    let t = t.clamp(
        stops.first().map_or(0.0, |s| s.at),
        stops.last().map_or(1.0, |s| s.at),
    );
    let (i, next) = stops
        .windows(2)
        .enumerate()
        .find_map(|(i, w)| (t < w[1].at).then_some((i, &w[1])))
        .unwrap_or((stops.len() - 2, &stops[stops.len() - 1]));
    let prev = &stops[i];
    let span = next.at - prev.at;
    let f = if span <= 0.0 {
        0.0
    } else {
        ((t - prev.at) / span).clamp(0.0, 1.0)
    };
    let [pr, pg, pb] = rgb_linear(&prev.color);
    let [nr, ng, nb] = rgb_linear(&next.color);
    [
        linear_f32_to_srgb8(pr + (nr - pr) * f),
        linear_f32_to_srgb8(pg + (ng - pg) * f),
        linear_f32_to_srgb8(pb + (nb - pb) * f),
    ]
}

/// Fill a shape path with a gradient: the path is rasterized once as an
/// alpha mask (the fill color is irrelevant; only coverage is read), then
/// each covered pixel takes its color from the linear-light ramp along the
/// gradient axis.
pub fn fill_gradient_mask(
    path: &tiny_skia::Path,
    g: &GradientDef,
    layer_w: u32,
    layer_h: u32,
) -> RgbaLayer {
    let mut mask = tiny_skia::Pixmap::new(layer_w, layer_h).expect("layer size was validated");
    let mut paint = tiny_skia::Paint::default();
    paint.set_color_rgba8(0, 0, 0, 255);
    mask.fill_path(
        path,
        &paint,
        tiny_skia::FillRule::Winding,
        tiny_skia::Transform::identity(),
        None,
    );
    let mut layer = RgbaLayer::new(layer_w, layer_h);
    let range = axis_range(layer_w as f32, layer_h as f32, g.angle);
    for y in 0..layer_h as i32 {
        for x in 0..layer_w as i32 {
            let a = mask.pixel(x as u32, y as u32).map_or(0, |c| c.alpha());
            if a == 0 {
                continue;
            }
            let t = axis_position(x as f32, y as f32, g.angle, range);
            let [r, gg, b] = gradient_srgb8(g, t);
            layer.set(x, y, [r, gg, b, a]);
        }
    }
    layer
}

/// Zero the alpha of every pixel outside `clip` (given in layer coordinates);
/// color channels are left alone because straight-alpha compositing ignores
/// them where alpha is 0.
pub fn apply_clip(layer: &mut RgbaLayer, clip: ClipRect, offset: (f32, f32)) {
    let (ox, oy) = (offset.0.round() as i64, offset.1.round() as i64);
    // Saturating adds: geometry is bounded by validation, but the renderer
    // is also reachable with unvalidated documents (library use) and must
    // not panic there (r1 F9).
    let x0 = clip.x.saturating_add(ox).max(0);
    let y0 = clip.y.saturating_add(oy).max(0);
    let x1 = clip
        .x
        .saturating_add(ox)
        .saturating_add(clip.w)
        .min(layer.width as i64);
    let y1 = clip
        .y
        .saturating_add(oy)
        .saturating_add(clip.h)
        .min(layer.height as i64);
    for y in 0..layer.height as i64 {
        for x in 0..layer.width as i64 {
            if x < x0 || y < y0 || x >= x1 || y >= y1 {
                let i = ((y as u32 * layer.width + x as u32) * 4) as usize + 3;
                layer.data[i] = 0;
            }
        }
    }
}

/// The shadow layer of an element, plus its origin offset relative to the
/// element layer (the buffer grows to cover blur and the shadow offset).
/// The silhouette is offset, tinted with the shadow color at its alpha, and
/// blurred with three box-blur passes in premultiplied linear light.
pub fn shadow_layer(layer: &RgbaLayer, s: &ShadowDef) -> (RgbaLayer, (f32, f32)) {
    let blur = blur_radius(layer.width, layer.height, s.blur);
    let b = blur as i64;
    let (ox, oy) = (s.offset_x.round() as i64, s.offset_y.round() as i64);
    // Buffer bounds: union of the layer box and its offset copy, padded by
    // the blur radius on every side.
    let min_x = (0.min(ox) - b).max(i64::MIN / 4);
    let min_y = (0.min(oy) - b).max(i64::MIN / 4);
    let max_x = (layer.width as i64).max(ox + layer.width as i64) + b;
    let max_y = (layer.height as i64).max(oy + layer.height as i64) + b;
    let (sw, sh) = ((max_x - min_x).max(1) as u32, (max_y - min_y).max(1) as u32);
    let mut buf = vec![0f32; (sw * sh * 4) as usize];

    let [cr, cg, cb] = rgb_linear(&s.color);
    for y in 0..layer.height as i64 {
        for x in 0..layer.width as i64 {
            let a = f32::from(layer.pixel(x as u32, y as u32)[3]) / 255.0;
            if a <= 0.0 {
                continue;
            }
            let strength = a * s.alpha;
            let (px, py) = (x + ox - min_x, y + oy - min_y);
            if px < 0 || py < 0 || px >= sw as i64 || py >= sh as i64 {
                continue;
            }
            let i = ((py as u32 * sw + px as u32) * 4) as usize;
            buf[i] = cr * strength;
            buf[i + 1] = cg * strength;
            buf[i + 2] = cb * strength;
            buf[i + 3] = strength;
        }
    }

    for _ in 0..3 {
        box_blur_pass(&mut buf, sw, sh, blur, true);
        box_blur_pass(&mut buf, sw, sh, blur, false);
    }

    let mut out = RgbaLayer::new(sw, sh);
    for (src, dst) in buf.chunks_exact(4).zip(out.data.chunks_exact_mut(4)) {
        let a = src[3];
        if a <= 0.0 {
            continue;
        }
        let inv = 1.0 / a;
        dst[0] = linear_f32_to_srgb8(src[0] * inv);
        dst[1] = linear_f32_to_srgb8(src[1] * inv);
        dst[2] = linear_f32_to_srgb8(src[2] * inv);
        dst[3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    (out, (min_x as f32, min_y as f32))
}

/// Effective blur radius: the requested radius, clamped so the padded shadow
/// buffer cannot exceed [`MAX_SHADOW_PIXELS`].
fn blur_radius(w: u32, h: u32, requested: f32) -> u32 {
    if requested <= 0.0 {
        return 0;
    }
    let mut r = requested.round().clamp(0.0, 512.0) as u32;
    while r > 0
        && (u64::from(w) + 2 * u64::from(r)) * (u64::from(h) + 2 * u64::from(r)) > MAX_SHADOW_PIXELS
    {
        r -= 1;
    }
    r
}

/// One horizontal or vertical box-blur pass over a premultiplied linear f32
/// RGBA buffer. Edges clamp (the window shrinks at the border and divides by
/// the actual sample count), which keeps the shadow energy bounded. The
/// operation order is fixed, so results are identical on every platform.
fn box_blur_pass(buf: &mut [f32], w: u32, h: u32, radius: u32, horizontal: bool) {
    if radius == 0 {
        return;
    }
    let r = radius as usize;
    let (outer_count, inner_count) = if horizontal {
        (h as usize, w as usize)
    } else {
        (w as usize, h as usize)
    };
    let row_len = w as usize * 4;
    // Along the pass direction, samples sit `sample_stride` bytes apart; each
    // line of the pass starts every `line_stride` bytes.
    let (line_stride, sample_stride) = if horizontal {
        (row_len, 4)
    } else {
        (4, row_len)
    };
    let mut line = vec![0f32; inner_count * 4];
    let mut blurred = vec![0f32; inner_count * 4];
    for o in 0..outer_count {
        let base = o * line_stride;
        for k in 0..inner_count {
            for c in 0..4 {
                line[k * 4 + c] = buf[base + k * sample_stride + c];
            }
        }
        for i in 0..inner_count {
            let lo = i.saturating_sub(r);
            let hi = (i + r).min(inner_count - 1);
            let n = (hi - lo + 1) as f32;
            let mut sums = [0f32; 4];
            for j in lo..=hi {
                for c in 0..4 {
                    sums[c] += line[j * 4 + c];
                }
            }
            for c in 0..4 {
                blurred[i * 4 + c] = sums[c] / n;
            }
        }
        for k in 0..inner_count {
            for c in 0..4 {
                buf[base + k * sample_stride + c] = blurred[k * 4 + c];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quillon_core::document::GradientStop;

    fn hex(c: &str) -> Color {
        Color::parse(c).unwrap()
    }

    fn gradient(angle: f32, from: &str, to: &str) -> GradientDef {
        GradientDef {
            angle,
            stops: vec![
                GradientStop {
                    at: 0.0,
                    color: hex(from),
                },
                GradientStop {
                    at: 1.0,
                    color: hex(to),
                },
            ],
        }
    }

    #[test]
    fn gradient_midpoint_is_linear_light() {
        // White -> black at t=0.5 must encode to ~188, not 128 (research/T).
        let g = gradient(0.0, "#ffffff", "#000000");
        let [r, gg, b] = gradient_srgb8(&g, 0.5);
        assert!((i16::from(r) - 188).abs() <= 1, "r={r}");
        assert_eq!(r, gg);
        assert_eq!(gg, b);
    }

    #[test]
    fn gradient_endpoints_are_exact() {
        let g = gradient(90.0, "#102030", "#304050");
        assert_eq!(gradient_srgb8(&g, 0.0), [0x10, 0x20, 0x30]);
        assert_eq!(gradient_srgb8(&g, 1.0), [0x30, 0x40, 0x50]);
    }

    #[test]
    fn axis_position_runs_left_to_right_at_zero_degrees() {
        let range = axis_range(100.0, 10.0, 0.0);
        assert!(axis_position(0.0, 5.0, 0.0, range) < 0.01);
        assert!((axis_position(50.0, 5.0, 0.0, range) - 0.5).abs() < 0.01);
        assert!(axis_position(100.0, 5.0, 0.0, range) > 0.99);
    }

    #[test]
    fn axis_position_runs_top_to_bottom_at_ninety_degrees() {
        let range = axis_range(10.0, 100.0, 90.0);
        assert!(axis_position(5.0, 0.0, 90.0, range) < 0.01);
        assert!(axis_position(5.0, 100.0, 90.0, range) > 0.99);
    }

    #[test]
    fn rounded_rect_path_is_buildable_and_clamps_oversize_radius() {
        assert!(rounded_rect_path(200.0, 100.0, 12.0).is_some());
        // Radius larger than half the height clamps instead of breaking.
        assert!(rounded_rect_path(200.0, 100.0, 90.0).is_some());
        assert!(rounded_rect_path(0.0, 100.0, 12.0).is_none());
    }

    #[test]
    fn clip_masks_only_outside_the_rect() {
        let mut layer = RgbaLayer::new(10, 10);
        for y in 0..10 {
            for x in 0..10 {
                layer.set(x, y, [200, 100, 50, 255]);
            }
        }
        apply_clip(
            &mut layer,
            ClipRect {
                x: 2,
                y: 3,
                w: 4,
                h: 5,
            },
            (0.0, 0.0),
        );
        assert_eq!(layer.pixel(3, 4)[3], 255, "inside the clip stays");
        assert_eq!(layer.pixel(0, 0)[3], 0, "outside is masked");
        assert_eq!(layer.pixel(6, 4)[3], 0, "x past the clip is masked");
        assert_eq!(layer.pixel(3, 9)[3], 0, "y past the clip is masked");
    }

    #[test]
    fn shadow_blur_spreads_and_keeps_color() {
        // A single opaque white pixel with a black shadow: after blur the
        // neighborhood gains partial alpha, the center keeps some too, and
        // the tint stays the shadow color.
        let mut layer = RgbaLayer::new(21, 21);
        layer.set(10, 10, [255, 255, 255, 255]);
        let (shadow, offset) = shadow_layer(
            &layer,
            &ShadowDef {
                offset_x: 0.0,
                offset_y: 0.0,
                blur: 2.0,
                color: hex("#000000"),
                alpha: 1.0,
            },
        );
        // The buffer is padded by the blur radius on every side, so the
        // element's (0,0) sits at (-2,-2) in shadow-layer coordinates.
        assert_eq!(offset, (-2.0, -2.0));
        let center = shadow.pixel(12, 12);
        assert!(center[3] > 0, "center still has shadow");
        let near = shadow.pixel(14, 12);
        assert!(near[3] > 0, "blur spreads the shadow");
        assert!(near[3] < 255, "blur softens the shadow");
        // Black tint at partial alpha: color channels stay ~0.
        assert!(near[0] <= 2, "tint stays black, got r={}", near[0]);
    }

    #[test]
    fn blur_radius_is_clamped_for_huge_layers() {
        // A 7680x4320 layer with a 512px blur would need a ~1.6 Gpx buffer;
        // the clamp keeps it bounded and deterministic.
        let r = blur_radius(7680, 4320, 512.0);
        assert!(r < 512, "expected clamped radius, got {r}");
        assert_eq!(blur_radius(10, 10, 0.0), 0);
        assert_eq!(blur_radius(10, 10, 3.2), 3);
    }
}
