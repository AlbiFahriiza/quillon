//! The linear-light accumulator and straight-alpha layer blending.
//!
//! The frame is accumulated as premultiplied **linear** f32 RGBA (research/T:
//! blending in gamma-encoded space produces dark fringes). Layers are
//! straight-alpha sRGB u8 (tiny-skia, glyph masks, decoded images); each
//! layer converts to linear at blend time and src_overs onto the
//! accumulator. Conversion back to sRGB happens exactly once, at output.

use crate::lut::{linear_f32_to_srgb8, srgb8_to_linear};

/// A straight-alpha sRGB u8 layer: 4 bytes per pixel, row-major.
#[derive(Debug, Clone)]
pub struct RgbaLayer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaLayer {
    pub fn new(width: u32, height: u32) -> Self {
        RgbaLayer {
            width,
            height,
            data: vec![0; (width * height * 4) as usize],
        }
    }

    /// Set a pixel; out-of-range coordinates are ignored (clipping at layer
    /// edges during glyph rasterization).
    pub fn set(&mut self, x: i32, y: i32, rgba: [u8; 4]) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.data[i..i + 4].copy_from_slice(&rgba);
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        let mut out = [0u8; 4];
        out.copy_from_slice(&self.data[i..i + 4]);
        out
    }
}

/// The frame accumulator: premultiplied linear-light f32, `w * h * 4`.
pub struct LinearCanvas {
    pub width: u32,
    pub height: u32,
    /// (r, g, b, a) per pixel, premultiplied linear.
    pub px: Vec<f32>,
}

impl LinearCanvas {
    pub fn filled_with(width: u32, height: u32, color: [u8; 3]) -> Self {
        let (lr, lg, lb) = (
            srgb8_to_linear(color[0]),
            srgb8_to_linear(color[1]),
            srgb8_to_linear(color[2]),
        );
        let mut px = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..width * height {
            px.extend_from_slice(&[lr, lg, lb, 1.0]);
        }
        LinearCanvas { width, height, px }
    }

    /// Blend one straight-alpha layer pixel at exact integer canvas
    /// coordinates with the given element opacity. Fast path used by the
    /// blit (identity transform) compositor.
    pub fn blend_straight_pixel(
        &mut self,
        canvas_x: u32,
        canvas_y: u32,
        rgba: [u8; 4],
        opacity: f32,
    ) {
        let alpha = f32::from(rgba[3]) / 255.0 * opacity;
        if alpha <= 0.0 {
            return;
        }
        let (sr, sg, sb) = (
            srgb8_to_linear(rgba[0]),
            srgb8_to_linear(rgba[1]),
            srgb8_to_linear(rgba[2]),
        );
        let i = ((canvas_y * self.width + canvas_x) * 4) as usize;
        let keep = 1.0 - alpha;
        self.px[i] = sr * alpha + self.px[i] * keep;
        self.px[i + 1] = sg * alpha + self.px[i + 1] * keep;
        self.px[i + 2] = sb * alpha + self.px[i + 2] * keep;
        self.px[i + 3] = alpha + self.px[i + 3] * keep;
    }

    /// Blend a straight-alpha **linear premultiplied-by-opacity** source
    /// contribution. Used by the affine sampler, which already converted the
    /// sampled colors to linear.
    pub fn blend_linear_premult(
        &mut self,
        canvas_x: u32,
        canvas_y: u32,
        src: [f32; 3],
        alpha: f32,
    ) {
        if alpha <= 0.0 {
            return;
        }
        let i = ((canvas_y * self.width + canvas_x) * 4) as usize;
        let keep = 1.0 - alpha;
        self.px[i] = src[0] + self.px[i] * keep;
        self.px[i + 1] = src[1] + self.px[i + 1] * keep;
        self.px[i + 2] = src[2] + self.px[i + 2] * keep;
        self.px[i + 3] = alpha + self.px[i + 3] * keep;
    }

    /// Quantize to the output frame: un-premultiply, linear -> sRGB once.
    /// Alpha of 255 is forced for pixels that ended opaque; the output format
    /// is RGB video, and a fully transparent background cannot exist because
    /// the document background always fills the canvas.
    pub fn into_rgba8(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len());
        for chunk in self.px.chunks_exact(4) {
            let a = chunk[3];
            if a <= 0.0 {
                out.extend_from_slice(&[0, 0, 0, 255]);
                continue;
            }
            let inv = 1.0 / a;
            out.push(linear_f32_to_srgb8(chunk[0] * inv));
            out.push(linear_f32_to_srgb8(chunk[1] * inv));
            out.push(linear_f32_to_srgb8(chunk[2] * inv));
            out.push(255);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_layer_over_background_keeps_layer_color() {
        let mut canvas = LinearCanvas::filled_with(1, 1, [0, 0, 0]);
        canvas.blend_straight_pixel(0, 0, [255, 255, 255, 255], 1.0);
        assert_eq!(canvas.into_rgba8(), vec![255, 255, 255, 255]);
    }

    #[test]
    fn transparent_layer_leaves_background() {
        let mut canvas = LinearCanvas::filled_with(1, 1, [90, 90, 90]);
        canvas.blend_straight_pixel(0, 0, [255, 0, 0, 0], 1.0);
        assert_eq!(canvas.into_rgba8(), vec![90, 90, 90, 255]);
    }

    #[test]
    fn half_opacity_white_over_black_is_mid_gray() {
        let mut canvas = LinearCanvas::filled_with(1, 1, [0, 0, 0]);
        canvas.blend_straight_pixel(0, 0, [255, 255, 255, 255], 0.5);
        let out = canvas.into_rgba8();
        // Linear 0.5 -> sRGB is ~188, not 128: the whole point of the
        // linear pipeline (research/T).
        assert!((out[0] as i16 - 188).abs() <= 1, "got {}", out[0]);
        assert_eq!(out[0], out[1]);
    }
}
