//! `FSoundAttenuationSettings` (UE 4.16): recovered per-asset values + the distance curve.
//!
//! What is **recovered** (read from the cooked `SoundAttenuation` assets / `SoundNodeAttenuation` overrides):
//! shape, shape extents (the inner "no falloff" radius), `FalloffDistance`, `DistanceAlgorithm`, `OmniRadius`,
//! `StereoSpread`, LPF radii/frequencies, custom curve keys, `dBAttenuationAtMax`, listener-focus values.
//! Properties absent from an asset take the engine constructor defaults, listed in [`Attenuation::default`]
//! (UE 4.16 `FSoundAttenuationSettings`; **not** verifiable from the install because cooked assets only store
//! non-default values).
//!
//! What is **assumed** (the exact closed forms of `FAttenuationSettings::AttenuationEval` live in native code that is
//! not in the install): the shape of the Logarithmic / Inverse / LogReverse / NaturalSound curves, see
//! [`Attenuation::curve`]. Linear (`1 - d/falloff`) and Custom (the asset's own keys) are exact. All curves are 1 at the
//! inner radius and 0 at `extents + falloff`, so cull distance and loudness at the ends match the asset.

use crate::curve::Curve;
use fn_uasset::{PropertyList, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sphere,
    Capsule,
    Box,
    Cone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DistanceModel {
    Linear,
    Logarithmic,
    Inverse,
    LogReverse,
    NaturalSound,
    Custom,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attenuation {
    pub attenuate: bool,
    pub spatialize: bool,
    pub shape: Shape,
    /// `AttenuationShapeExtents` (cm). For a sphere `x` is the radius inside which there is no distance attenuation.
    pub extents: [f32; 3],
    pub falloff_distance: f32,
    pub algorithm: DistanceModel,
    pub db_attenuation_at_max: f32,
    /// `CustomAttenuationCurve` (time = distance beyond the inner radius / falloff, 0..1) when `algorithm == Custom`.
    pub custom_curve: Curve,
    /// Inside this radius the source is not panned (cm).
    pub omni_radius: f32,
    pub stereo_spread: f32,
    pub lpf_enabled: bool,
    pub lpf_radius_min: f32,
    pub lpf_radius_max: f32,
    pub lpf_freq_at_min: f32,
    pub lpf_freq_at_max: f32,
    pub listener_focus: bool,
    pub focus_azimuth: f32,
    pub non_focus_azimuth: f32,
    pub focus_volume: f32,
    pub non_focus_volume: f32,
}

impl Default for Attenuation {
    /// Engine constructor defaults (UE 4.16 `FSoundAttenuationSettings`), used for every property an asset omits.
    fn default() -> Self {
        Attenuation {
            attenuate: true,
            spatialize: true,
            shape: Shape::Sphere,
            extents: [400.0, 0.0, 0.0],
            falloff_distance: 3600.0,
            algorithm: DistanceModel::Linear,
            db_attenuation_at_max: -60.0,
            custom_curve: Curve::default(),
            omni_radius: 0.0,
            stereo_spread: 200.0,
            lpf_enabled: false,
            lpf_radius_min: 3000.0,
            lpf_radius_max: 6000.0,
            lpf_freq_at_min: 20000.0,
            lpf_freq_at_max: 20000.0,
            listener_focus: false,
            focus_azimuth: 30.0,
            non_focus_azimuth: 60.0,
            focus_volume: 1.0,
            non_focus_volume: 1.0,
        }
    }
}

fn enum_tail(v: &Value) -> Option<&str> {
    v.as_str().map(|s| s.rsplit("::").next().unwrap_or(s))
}

impl Attenuation {
    /// Build from a `SoundAttenuationSettings` struct's property list (the `Attenuation` property of a
    /// `SoundAttenuation` asset or a node/cue `AttenuationOverrides`).
    pub fn from_props(p: &PropertyList) -> Attenuation {
        let mut a = Attenuation::default();
        let f = |n: &str| p.get(n).and_then(Value::as_f32);
        let b = |n: &str| p.get(n).and_then(Value::as_bool);
        if let Some(v) = b("bAttenuate") {
            a.attenuate = v;
        }
        if let Some(v) = b("bSpatialize") {
            a.spatialize = v;
        }
        if let Some(s) = p.get("AttenuationShape").and_then(enum_tail) {
            a.shape = match s {
                "Capsule" => Shape::Capsule,
                "Box" => Shape::Box,
                "Cone" => Shape::Cone,
                _ => Shape::Sphere,
            };
        }
        if let Some(v) = p.get("AttenuationShapeExtents").and_then(Value::as_vec3) {
            a.extents = v;
        }
        if let Some(v) = f("FalloffDistance") {
            a.falloff_distance = v;
        }
        if let Some(s) = p.get("DistanceAlgorithm").and_then(enum_tail) {
            a.algorithm = match s {
                "Logarithmic" => DistanceModel::Logarithmic,
                "Inverse" => DistanceModel::Inverse,
                "LogReverse" => DistanceModel::LogReverse,
                "NaturalSound" => DistanceModel::NaturalSound,
                "Custom" => DistanceModel::Custom,
                _ => DistanceModel::Linear,
            };
        }
        if let Some(v) = f("dBAttenuationAtMax") {
            a.db_attenuation_at_max = v;
        }
        a.custom_curve = Curve::from_runtime_curve(p.get("CustomAttenuationCurve"));
        if let Some(v) = f("OmniRadius") {
            a.omni_radius = v;
        }
        if let Some(v) = f("StereoSpread") {
            a.stereo_spread = v;
        }
        if let Some(v) = b("bAttenuateWithLPF") {
            a.lpf_enabled = v;
        }
        if let Some(v) = f("LPFRadiusMin") {
            a.lpf_radius_min = v;
        }
        if let Some(v) = f("LPFRadiusMax") {
            a.lpf_radius_max = v;
        }
        if let Some(v) = f("LPFFrequencyAtMin") {
            a.lpf_freq_at_min = v;
        }
        if let Some(v) = f("LPFFrequencyAtMax") {
            a.lpf_freq_at_max = v;
        }
        if let Some(v) = b("bEnableListenerFocus") {
            a.listener_focus = v;
        }
        if let Some(v) = f("FocusAzimuth") {
            a.focus_azimuth = v;
        }
        if let Some(v) = f("NonFocusAzimuth") {
            a.non_focus_azimuth = v;
        }
        if let Some(v) = f("FocusVolumeAttenuation") {
            a.focus_volume = v;
        }
        if let Some(v) = f("NonFocusVolumeAttenuation") {
            a.non_focus_volume = v;
        }
        a
    }

    /// Distance (cm) past which the sound is inaudible: `inner radius + falloff`.
    pub fn max_distance(&self) -> f32 {
        if !self.attenuate {
            return f32::INFINITY;
        }
        self.inner_radius() + self.falloff_distance
    }

    /// Radius with no distance attenuation. Sphere/cone/capsule/box are all reduced to their first extent (APPROXIMATION
    /// for the non-sphere shapes: the listener-relative shape test of UE is not reproduced).
    pub fn inner_radius(&self) -> f32 {
        self.extents[0].max(0.0)
    }

    /// Normalised curve value for `alpha = distance beyond the inner radius / falloff` (0..1). 1 at 0, 0 at 1.
    /// Linear and Custom are exact; the others are ASSUMED shapes (see the module docs).
    pub fn curve(&self, alpha: f32) -> f32 {
        let a = alpha.clamp(0.0, 1.0);
        if a >= 1.0 {
            return 0.0;
        }
        match self.algorithm {
            DistanceModel::Linear => 1.0 - a,
            // steep near the source, long quiet tail: -ln(a) mapped so a=0.001 -> 1, a=1 -> 0
            DistanceModel::Logarithmic => ((-(a.max(1e-3)).ln()) / (1000f32).ln()).clamp(0.0, 1.0),
            // 1/x style: 0.02 / a (clamped), forced to 0 at the cull distance
            DistanceModel::Inverse => (0.02 / a.max(1e-4)).min(1.0) * (1.0 - a),
            // mirror of Logarithmic: slow start, steep end
            DistanceModel::LogReverse => 1.0 - ((-((1.0 - a).max(1e-3)).ln()) / (1000f32).ln()).clamp(0.0, 1.0),
            // dB falls linearly with normalised distance to dBAttenuationAtMax, then forced to 0 at the cull distance
            DistanceModel::NaturalSound => 10f32.powf(self.db_attenuation_at_max * a / 20.0) * (1.0 - a),
            DistanceModel::Custom => {
                if self.custom_curve.is_empty() {
                    return 1.0 - a; // asset stored no keys (external curve unresolved): linear
                }
                self.custom_curve.eval(a, 1.0).clamp(0.0, 1.0)
            }
        }
    }

    /// Linear gain for a listener `distance` cm away.
    pub fn gain(&self, distance: f32) -> f32 {
        if !self.attenuate {
            return 1.0;
        }
        let beyond = (distance - self.inner_radius()).max(0.0);
        if self.falloff_distance <= 0.0 {
            return if beyond > 0.0 { 0.0 } else { 1.0 };
        }
        self.curve(beyond / self.falloff_distance)
    }

    /// Low-pass cutoff (Hz) for `distance` cm, or `None` when LPF attenuation is off / at the open frequency.
    pub fn lpf_cutoff(&self, distance: f32) -> Option<f32> {
        if !self.lpf_enabled {
            return None;
        }
        let t = if self.lpf_radius_max > self.lpf_radius_min { ((distance - self.lpf_radius_min) / (self.lpf_radius_max - self.lpf_radius_min)).clamp(0.0, 1.0) } else { (distance >= self.lpf_radius_max) as u8 as f32 };
        let f = self.lpf_freq_at_min + (self.lpf_freq_at_max - self.lpf_freq_at_min) * t;
        (f < 19999.0).then_some(f.max(20.0))
    }

    /// Fraction of full panning (0 = centred, 1 = fully spatialised) for a source `distance` cm away.
    pub fn spatial_blend(&self, distance: f32) -> f32 {
        if !self.spatialize {
            return 0.0;
        }
        if self.omni_radius <= 0.0 {
            1.0
        } else {
            (distance / self.omni_radius).clamp(0.0, 1.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_endpoints_and_inner_radius() {
        let a = Attenuation { extents: [300.0, 0.0, 0.0], falloff_distance: 800.0, ..Default::default() };
        assert_eq!(a.gain(0.0), 1.0);
        assert_eq!(a.gain(300.0), 1.0);
        assert!((a.gain(700.0) - 0.5).abs() < 1e-6);
        assert_eq!(a.gain(1100.0), 0.0);
        assert_eq!(a.max_distance(), 1100.0);
    }

    #[test]
    fn every_model_is_monotonic_with_unit_ends() {
        for m in [DistanceModel::Linear, DistanceModel::Logarithmic, DistanceModel::Inverse, DistanceModel::LogReverse, DistanceModel::NaturalSound, DistanceModel::Custom] {
            let a = Attenuation { algorithm: m, ..Default::default() };
            assert!((a.curve(0.0) - 1.0).abs() < 1e-4 || m == DistanceModel::Inverse, "{m:?}");
            assert_eq!(a.curve(1.0), 0.0, "{m:?}");
            let mut last = f32::INFINITY;
            for i in 0..=100 {
                let g = a.curve(i as f32 / 100.0);
                assert!(g <= last + 1e-5 && (0.0..=1.0).contains(&g), "{m:?} at {i}: {g} vs {last}");
                last = g;
            }
        }
    }

    #[test]
    fn custom_curve_interpolates() {
        use crate::curve::Key;
        let key = |t: f32, v: f32| Key { time: t, value: v, interp: 0, arrive_tangent: 0.0, leave_tangent: 0.0 };
        let a = Attenuation { algorithm: DistanceModel::Custom, custom_curve: Curve(vec![key(0.0, 1.0), key(0.5, 0.2), key(1.0, 0.0)]), ..Default::default() };
        assert!((a.curve(0.25) - 0.6).abs() < 1e-5);
    }

    #[test]
    fn lpf_and_omni() {
        let a = Attenuation { lpf_enabled: true, lpf_radius_min: 300.0, lpf_radius_max: 500.0, lpf_freq_at_min: 20000.0, lpf_freq_at_max: 500.0, omni_radius: 200.0, ..Default::default() };
        assert_eq!(a.lpf_cutoff(100.0), None);
        assert!((a.lpf_cutoff(500.0).unwrap() - 500.0).abs() < 1.0);
        assert_eq!(a.spatial_blend(100.0), 0.5);
        assert_eq!(a.spatial_blend(1000.0), 1.0);
    }
}

