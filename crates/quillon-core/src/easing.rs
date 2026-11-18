//! Keyframe easing and track evaluation.
//!
//! The ten curves are polynomial only (no transcendentals), so evaluation is
//! bit-identical across platforms; libm `sin`/`pow` differ in the last ULP
//! between C libraries, which would break the reproducible-pixel contract.

use crate::document::{Anim, Key, Track};
use crate::time::Tick;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Easing curve applied to the segment that *starts* at the key carrying it
/// (the last key's `ease` is ignored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    Linear,
    InQuad,
    OutQuad,
    InCubic,
    OutCubic,
    InOutCubic,
    InQuart,
    OutQuart,
    InBack,
    OutBack,
}

/// Overshoot constant for the back curves (the standard value).
const BACK_C1: f32 = 1.701_58;
const BACK_C3: f32 = BACK_C1 + 1.0;

/// Apply an easing curve to progress `t` in [0, 1].
pub fn apply_ease(ease: Ease, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    match ease {
        Ease::Linear => t,
        Ease::InQuad => t * t,
        Ease::OutQuad => 1.0 - (1.0 - t) * (1.0 - t),
        Ease::InCubic => t * t * t,
        Ease::OutCubic => 1.0 - (1.0 - t).powi(3),
        Ease::InOutCubic => {
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        Ease::InQuart => t * t * t * t,
        Ease::OutQuart => 1.0 - (1.0 - t).powi(4),
        Ease::InBack => BACK_C3 * t * t * t - BACK_C1 * t * t,
        Ease::OutBack => {
            let u = t - 1.0;
            1.0 + BACK_C3 * u * u * u + BACK_C1 * u * u
        }
    }
}

/// Evaluate a validated, time-sorted keyframe track at `tick`.
///
/// Caller contract (enforced by document validation): at least two keys,
/// strictly increasing `t` within [0, scene duration]. The first key's value
/// holds before it; the last key's value holds at and after it.
pub fn evaluate_track(track: &Track, tick: Tick) -> f32 {
    let keys = &track.keys;
    // Unreachable on validated documents; hold-first is the safe fallback.
    if keys.len() < 2 {
        return keys.first().map_or(0.0, |k| k.v);
    }
    if tick <= keys[0].t {
        return keys[0].v;
    }
    let last = keys[keys.len() - 1];
    if tick >= last.t {
        return last.v;
    }
    let (i, next) = keys
        .windows(2)
        .enumerate()
        .find_map(|(i, w)| (tick < w[1].t).then_some((i, w[1])))
        .unwrap_or((keys.len() - 2, last));
    let Key { t: t0, v: v0, .. } = keys[i];
    let span = (next.t - t0) as f32;
    if span <= 0.0 {
        return v0;
    }
    let frac = (tick - t0) as f32 / span;
    let eased = apply_ease(keys[i].ease.unwrap_or(Ease::Linear), frac);
    v0 + (next.v - v0) * eased
}

/// Animated element properties at a tick; defaults describe an untouched
/// element (opaque, unmoved, unscaled, unrotated).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementState {
    pub opacity: f32,
    pub translate_x: f32,
    pub translate_y: f32,
    pub scale: f32,
    /// Rotation in degrees, clockwise, around the element's center.
    pub rotate_deg: f32,
}

impl Default for ElementState {
    fn default() -> Self {
        ElementState {
            opacity: 1.0,
            translate_x: 0.0,
            translate_y: 0.0,
            scale: 1.0,
            rotate_deg: 0.0,
        }
    }
}

/// Evaluate all animation tracks of an element at `tick` (scene-local).
pub fn evaluate_anim(anim: Option<&Anim>, tick: Tick) -> ElementState {
    let mut state = ElementState::default();
    let Some(anim) = anim else { return state };
    if let Some(track) = &anim.opacity {
        state.opacity = evaluate_track(track, tick);
    }
    if let Some(track) = &anim.translate_x {
        state.translate_x = evaluate_track(track, tick);
    }
    if let Some(track) = &anim.translate_y {
        state.translate_y = evaluate_track(track, tick);
    }
    if let Some(track) = &anim.scale {
        state.scale = evaluate_track(track, tick);
    }
    if let Some(track) = &anim.rotate {
        state.rotate_deg = evaluate_track(track, tick);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Key;

    fn track(pairs: &[(i64, f32, Option<Ease>)]) -> Track {
        Track {
            keys: pairs
                .iter()
                .map(|&(t, v, ease)| Key { t, v, ease })
                .collect(),
        }
    }

    #[test]
    fn ease_endpoints_are_exact() {
        for ease in [
            Ease::Linear,
            Ease::InQuad,
            Ease::OutQuad,
            Ease::InCubic,
            Ease::OutCubic,
            Ease::InOutCubic,
            Ease::InQuart,
            Ease::OutQuart,
            Ease::InBack,
            Ease::OutBack,
        ] {
            assert!((apply_ease(ease, 0.0)).abs() < 1e-6, "{ease:?} at 0");
            assert!((apply_ease(ease, 1.0) - 1.0).abs() < 1e-6, "{ease:?} at 1");
        }
    }

    #[test]
    fn linear_easing_is_identity_midway() {
        assert_eq!(apply_ease(Ease::Linear, 0.5), 0.5);
    }

    #[test]
    fn track_holds_before_first_and_after_last_key() {
        let tr = track(&[(100, 10.0, None), (200, 20.0, None)]);
        assert_eq!(evaluate_track(&tr, 0), 10.0);
        assert_eq!(evaluate_track(&tr, 150), 15.0);
        assert_eq!(evaluate_track(&tr, 250), 20.0);
    }

    #[test]
    fn ease_applies_to_segment_starting_at_its_key() {
        // out_quad on the first segment: 25% progress in, eased to 43.75%.
        let tr = track(&[(0, 0.0, Some(Ease::OutQuad)), (100, 100.0, None)]);
        assert!((evaluate_track(&tr, 25) - 43.75).abs() < 1e-4);
    }

    #[test]
    fn anim_evaluation_defaults_without_tracks() {
        let state = evaluate_anim(None, 0);
        assert_eq!(state, ElementState::default());
    }
}
