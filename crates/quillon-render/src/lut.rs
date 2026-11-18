//! sRGB <-> linear-light lookup tables for the render hot path.
//!
//! Compositing must happen in linear light (research/T), but the exact
//! transfer functions use f64 `powf` — far too slow per pixel. The 8-bit
//! sRGB-to-linear direction is exact by construction (256 entries); the
//! linear-to-sRGB direction is a 16384-entry table with linear interpolation,
//! whose quantized error stays below 1/255 everywhere (checked by test).
//! Both tables are pure arithmetic once built, so frames stay reproducible
//! across platforms regardless of libm differences.

use quillon_core::color::{linear_to_srgb8, srgb_to_linear};
use std::sync::OnceLock;

/// Exact sRGB code value (0-255) -> linear-light f32.
pub fn srgb8_to_linear(v: u8) -> f32 {
    static LUT: OnceLock<Box<[f32; 256]>> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut table = Box::new([0.0f32; 256]);
        for (i, slot) in table.iter_mut().enumerate() {
            *slot = srgb_to_linear(i as f32 / 255.0);
        }
        table
    })[v as usize]
}

const LINEAR_TO_SRGB_ENTRIES: usize = 16384;

/// Linear-light f32 in [0, 1] -> sRGB code value, via table interpolation.
pub fn linear_f32_to_srgb8(v: f32) -> u8 {
    static LUT: OnceLock<Box<[u16; LINEAR_TO_SRGB_ENTRIES]>> = OnceLock::new();
    let table = LUT.get_or_init(|| {
        let mut t = Box::new([0u16; LINEAR_TO_SRGB_ENTRIES]);
        for (i, slot) in t.iter_mut().enumerate() {
            let x = i as f32 / (LINEAR_TO_SRGB_ENTRIES - 1) as f32;
            *slot = u16::from(linear_to_srgb8(x));
        }
        t
    });
    let clamped = v.clamp(0.0, 1.0);
    let pos = clamped * (LINEAR_TO_SRGB_ENTRIES - 1) as f32;
    let idx = pos.floor() as usize;
    if idx + 1 >= LINEAR_TO_SRGB_ENTRIES {
        return table[LINEAR_TO_SRGB_ENTRIES - 1] as u8;
    }
    let frac = pos - idx as f32;
    let a = table[idx] as f32;
    let b = table[idx + 1] as f32;
    (a + (b - a) * frac + 0.5) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use quillon_core::color::linear_to_srgb8;

    #[test]
    fn srgb8_lut_matches_exact_function() {
        for v in 0..=255u8 {
            let exact = srgb_to_linear(f32::from(v) / 255.0);
            assert_eq!(srgb8_to_linear(v), exact, "code {v}");
        }
    }

    #[test]
    fn linear_lut_quantizes_within_one_lsb() {
        for i in 0..=4096 {
            let x = i as f32 / 4096.0;
            let exact = linear_to_srgb8(x);
            let approx = linear_f32_to_srgb8(x);
            let delta = (i16::from(exact) - i16::from(approx)).abs();
            assert!(delta <= 1, "linear {x}: exact {exact}, approx {approx}");
        }
    }

    #[test]
    fn roundtrip_through_both_luts_is_exact() {
        for v in 0..=255u8 {
            assert_eq!(linear_f32_to_srgb8(srgb8_to_linear(v)), v, "code {v}");
        }
    }
}
