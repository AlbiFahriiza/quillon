//! sRGB hex colors and the sRGB <-> linear-light conversion used by the
//! compositing pipeline (plan §3 F6, research/T: blending must happen in
//! linear light; conversion happens once, at output).

use serde::{Deserialize, Serialize};

/// An sRGB color authored as `#rrggbb`. Normalized to lowercase on parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Color(String);

impl Color {
    /// Parse `#rrggbb` (case-insensitive). Returns `None` for any other shape;
    /// document validation reports the location, this only answers the shape.
    pub fn parse(hex: &str) -> Option<Color> {
        let bytes = hex.as_bytes();
        if bytes.len() != 7 || bytes[0] != b'#' {
            return None;
        }
        if !bytes[1..].iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        Some(Color(hex.to_ascii_lowercase()))
    }

    /// Build from 8-bit channels; the inverse of [`Color::rgb8`].
    pub fn from_rgb8(r: u8, g: u8, b: u8) -> Color {
        let mut hex = String::with_capacity(7);
        hex.push('#');
        for v in [r, g, b] {
            hex.push_str(&format!("{v:02x}"));
        }
        Color(hex)
    }

    /// The normalized `#rrggbb` string.
    pub fn as_hex(&self) -> &str {
        &self.0
    }

    /// Channels as u8 (r, g, b) in sRGB space. Infallible: every constructor
    /// guarantees two hex digits per channel.
    pub fn rgb8(&self) -> (u8, u8, u8) {
        let h = |from: usize| u8::from_str_radix(&self.0[from..from + 2], 16).unwrap_or(0);
        (h(1), h(3), h(5))
    }

    /// Channels as linear-light f32 in [0, 1].
    pub fn linear_rgb(&self) -> (f32, f32, f32) {
        let (r, g, b) = self.rgb8();
        (
            srgb_to_linear(f32::from(r) / 255.0),
            srgb_to_linear(f32::from(g) / 255.0),
            srgb_to_linear(f32::from(b) / 255.0),
        )
    }
}

impl std::fmt::Display for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// A Color serializes as its hex string, so its schema is a string schema.
impl schemars::JsonSchema for Color {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        <String as schemars::JsonSchema>::json_schema(generator)
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::from_rgb8(0, 0, 0)
    }
}

/// sRGB -> linear transfer function (IEC 61966-2-1). f64 internally for
/// precision; the result rounds to f32.
pub fn srgb_to_linear(c: f32) -> f32 {
    let c = f64::from(c);
    let v = if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    };
    v as f32
}

/// linear -> sRGB transfer function, the inverse of [`srgb_to_linear`].
pub fn linear_to_srgb(c: f32) -> f32 {
    let c = f64::from(c);
    let v = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    v as f32
}

/// Quantize a linear-light channel to an 8-bit sRGB code value.
pub fn linear_to_srgb8(c: f32) -> u8 {
    let v = linear_to_srgb(c) * 255.0 + 0.5;
    v.clamp(0.0, 255.0) as u8
}

/// WCAG-style relative-luminance contrast ratio, used by the low-contrast
/// lint. Inputs are linear channels.
pub fn contrast_ratio(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    let lum = |c: (f32, f32, f32)| 0.2126 * c.0 + 0.7152 * c.1 + 0.0722 * c.2;
    let (la, lb) = (lum(a), lum(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_accepts_only_7_char_hex() {
        assert!(Color::parse("#000000").is_some());
        assert!(Color::parse("#FFFFFF").is_some());
        assert!(Color::parse("000000").is_none());
        assert!(Color::parse("#00000").is_none());
        assert!(Color::parse("#0000000").is_none());
        assert!(Color::parse("#00gg00").is_none());
    }

    #[test]
    fn parse_normalizes_case() {
        assert_eq!(Color::parse("#AbCdEf").unwrap().as_hex(), "#abcdef");
    }

    #[test]
    fn from_rgb8_roundtrips_through_rgb8() {
        for (r, g, b) in [(0u8, 0, 0), (255, 255, 255), (18, 52, 86), (1, 2, 3)] {
            assert_eq!(Color::from_rgb8(r, g, b).rgb8(), (r, g, b));
        }
    }

    #[test]
    fn srgb_linear_roundtrip_hits_quantized_value() {
        for v in [0u8, 1, 64, 128, 200, 254, 255] {
            let c = f32::from(v) / 255.0;
            assert_eq!(linear_to_srgb8(srgb_to_linear(c)), v, "code value {v}");
        }
    }

    #[test]
    fn contrast_black_vs_white_is_21() {
        let ratio = contrast_ratio((0.0, 0.0, 0.0), (1.0, 1.0, 1.0));
        assert!((ratio - 21.0).abs() < 0.01, "ratio {ratio}");
    }
}
