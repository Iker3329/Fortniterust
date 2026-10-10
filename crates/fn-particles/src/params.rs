//! Per-instance parameters of a `UParticleSystemComponent` (`InstanceParameters`): `SetFloatParameter`, `SetVectorParameter`, `SetColorParameter`.
//! Looked up by `FName`, i.e. case-insensitively (UE4.16 ParticleComponents.cpp:6319 `SetFloatParameter`, :6516 `GetFloatParameter`, :6580 `GetAnyVectorParameter`).
use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Params {
    scalars: HashMap<String, f32>,
    /// Vector parameters, colours included (a `PSPT_Color` parameter reads back as the vector `rgb` of the 8-bit sRGB quantised colour, see [`Params::set_color`]).
    vectors: HashMap<String, [f32; 3]>,
}

fn key(name: &str) -> String { name.to_ascii_lowercase() }

/// `FLinearColor::ToFColor(true)` followed by `FLinearColor(FColor)` (the `sRGBToLinearTable`): what `GetAnyVectorParameter` returns for a colour parameter.
/// RECALLED (UE4.16 Color.cpp / Color.h; the sRGB transfer function is standard).
pub fn quantize_color(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    let enc = if c <= 0.003_130_8 { c * 12.92 } else { c.powf(1.0 / 2.4) * 1.055 - 0.055 };
    let q = (enc * 255.999).floor().clamp(0.0, 255.0) / 255.0;
    if q <= 0.040_45 { q / 12.92 } else { ((q + 0.055) / 1.055).powf(2.4) }
}

impl Params {
    pub fn new() -> Params { Params::default() }
    pub fn is_empty(&self) -> bool { self.scalars.is_empty() && self.vectors.is_empty() }
    /// `SetFloatParameter` (an empty name is ignored like `NAME_None`).
    pub fn set_float(&mut self, name: &str, v: f32) { if !name.is_empty() { self.scalars.insert(key(name), v); } }
    /// `SetVectorParameter`.
    pub fn set_vector(&mut self, name: &str, v: [f32; 3]) { if !name.is_empty() { self.vectors.insert(key(name), v); } }
    /// `SetColorParameter(Name, FLinearColor)`: the colour is stored as an `FColor` (8-bit sRGB), so a vector distribution reads the quantised linear rgb back; alpha is not
    /// reachable through a distribution and is dropped.
    pub fn set_color(&mut self, name: &str, rgba: [f32; 4]) { self.set_vector(name, [quantize_color(rgba[0]), quantize_color(rgba[1]), quantize_color(rgba[2])]); }
    pub fn clear(&mut self) { self.scalars.clear(); self.vectors.clear(); }
    pub fn remove(&mut self, name: &str) { let k = key(name); self.scalars.remove(&k); self.vectors.remove(&k); }
    /// `GetFloatParameter`: scalar parameters only.
    pub fn get_float(&self, name: &str) -> Option<f32> { self.scalars.get(&key(name)).copied() }
    /// `GetAnyVectorParameter`: a vector / colour parameter, else a scalar replicated to the three components.
    pub fn get_any_vector(&self, name: &str) -> Option<[f32; 3]> {
        let k = key(name);
        self.vectors.get(&k).copied().or_else(|| self.scalars.get(&k).map(|s| [*s; 3]))
    }
}
