//! MATERIAL PARAMETERS (2026-10-06): `UMaterialParameterCollection` assets and their per-world instance (`UMaterialParameterCollectionInstance`), packed into the
//! `MaterialCollectionN` uniform buffer the cooked programs read.
//!
//! * Layout (UE4.16 ParameterCollection.cpp:323 `GetParameterIndex` / :431 `GetDefaultParameterData` / :607 `GetParameterData`; the 1.8 exe's
//!   `UMaterialParameterCollectionInstance::GetParameterData` @14227f350 does the same): the scalar parameters packed four per float4 in their array order (a new
//!   float4 at every 4th), then one float4 per vector parameter. A parameter the instance has set (`ScalarParameterValues` / `VectorParameterValues`, by name)
//!   replaces the asset's `DefaultValue`.
//! * Which collection a program's `MaterialCollectionN` is: entry N of the shader map's `FUniformExpressionSet::ParameterCollections` (collection StateId GUIDs,
//!   `crate::uniform::UniformSet::parameter_collections`), resolved through the root material's `MaterialParameterCollectionInfos` (StateId -> collection),
//!   see `crate::material::Material::collections`.
//! * `SetScalarParameterValue` / `SetVectorParameterValue` only store a name the collection has (UE4.16 ParameterCollection.cpp `UMaterialParameterCollectionInstance::
//!   SetScalarParameterValue`: `Collection->GetScalarParameterByName`), [`Instance::set_scalar`] / [`Instance::set_vector`] do the same.
use crate::data::{long_package, PackageJson};
use serde_json::Value;
use std::collections::HashMap;

/// One `UMaterialParameterCollection` asset.
#[derive(Clone, Debug, PartialEq)]
pub struct Collection {
    /// Object path (`/Game/.../Name.Name`).
    pub path: String,
    /// `StateId` (hex of the 16 serialized bytes, the cooked JSON form).
    pub state_id: String,
    /// `ScalarParameters` (name, `DefaultValue`) in array order.
    pub scalars: Vec<(String, f32)>,
    /// `VectorParameters` (name, `DefaultValue` RGBA) in array order.
    pub vectors: Vec<(String, [f32; 4])>,
}

fn num(v: &Value) -> Option<f32> { v.as_f64().map(|x| x as f32) }
fn color(v: &Value) -> Option<[f32; 4]> {
    if let Some(a) = v.as_array() { return (a.len() == 4).then(|| [num(&a[0]).unwrap_or(0.), num(&a[1]).unwrap_or(0.), num(&a[2]).unwrap_or(0.), num(&a[3]).unwrap_or(0.)]); }
    let c = |k: &str, l: &str| v.get(k).or_else(|| v.get(l)).and_then(num).unwrap_or(0.);
    Some([c("R", "r"), c("G", "g"), c("B", "b"), c("A", "a")])
}

impl Collection {
    /// Loads the collection export of `object_path` from the paks.
    pub fn load(set: &fn_pak::PakSet, object_path: &str) -> Result<Collection, String> {
        let pkg = PackageJson::load(set, long_package(object_path))?;
        let name = object_path.rsplit('.').next().unwrap_or(object_path);
        let ex = pkg.by_name(name).filter(|e| e.class == "MaterialParameterCollection").or_else(|| pkg.of_class("MaterialParameterCollection").next())
            .ok_or_else(|| format!("{object_path}: no MaterialParameterCollection export"))?;
        Ok(Self::from_props(object_path, &ex.props))
    }
    /// From the export's tagged properties (a parameter without `DefaultValue` keeps the struct default 0 / (0, 0, 0, 0)).
    pub fn from_props(path: &str, p: &Value) -> Collection {
        let list = |k: &str| p.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let pname = |e: &Value| e.get("ParameterName").and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let scalars = list("ScalarParameters").iter().map(|e| (pname(e), e.get("DefaultValue").and_then(num).unwrap_or(0.))).collect();
        let vectors = list("VectorParameters").iter().map(|e| (pname(e), e.get("DefaultValue").and_then(color).unwrap_or([0.; 4]))).collect();
        Collection { path: path.to_owned(), state_id: p.get("StateId").and_then(|v| v.as_str()).unwrap_or("").to_owned(), scalars, vectors }
    }
    /// float4s of the uniform buffer: `DivideAndRoundUp(ScalarParameters.Num(), 4) + VectorParameters.Num()`.
    pub fn float4s(&self) -> usize { self.scalars.len().div_ceil(4) + self.vectors.len() }
    /// float4 index (and component for a scalar) of a parameter: `GetParameterIndex`.
    pub fn index_of(&self, name: &str) -> Option<(usize, Option<usize>)> {
        if let Some(i) = self.scalars.iter().position(|(n, _)| n == name) { return Some((i / 4, Some(i % 4))); }
        self.vectors.iter().position(|(n, _)| n == name).map(|i| (self.scalars.len().div_ceil(4) + i, None))
    }
    /// The uniform buffer contents for `inst` (`GetParameterData`; `GetDefaultParameterData` with an empty instance).
    pub fn data(&self, inst: &Instance) -> Vec<[f32; 4]> {
        let mut out: Vec<[f32; 4]> = Vec::with_capacity(self.float4s());
        for (i, (n, d)) in self.scalars.iter().enumerate() {
            if i % 4 == 0 { out.push([0.; 4]); }
            let last = out.len() - 1;
            out[last][i % 4] = inst.scalars.get(n).copied().unwrap_or(*d);
        }
        for (n, d) in &self.vectors { out.push(inst.vectors.get(n).copied().unwrap_or(*d)); }
        out
    }
}

/// The world's `UMaterialParameterCollectionInstance` values (what `SetScalarParameterValue` / `SetVectorParameterValue` stored).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Instance { pub scalars: HashMap<String, f32>, pub vectors: HashMap<String, [f32; 4]> }
impl Instance {
    /// `SetScalarParameterValue`: stored only when `c` has the parameter; returns whether it did.
    pub fn set_scalar(&mut self, c: &Collection, name: &str, v: f32) -> bool { let ok = c.scalars.iter().any(|(n, _)| n == name); if ok { self.scalars.insert(name.to_owned(), v); } ok }
    /// `SetVectorParameterValue`: stored only when `c` has the parameter; returns whether it did.
    pub fn set_vector(&mut self, c: &Collection, name: &str, v: [f32; 4]) -> bool { let ok = c.vectors.iter().any(|(n, _)| n == name); if ok { self.vectors.insert(name.to_owned(), v); } ok }
}

/// Collections loaded so far (by object path), shared by the buffer builder (`crate::gpu`) and the per-frame writers.
static LOADED: std::sync::Mutex<std::collections::BTreeMap<String, std::sync::Arc<Collection>>> = std::sync::Mutex::new(std::collections::BTreeMap::new());
/// [`Collection::load`] once per path.
pub fn load_cached(set: &fn_pak::PakSet, object_path: &str) -> Result<std::sync::Arc<Collection>, String> {
    if let Some(c) = loaded(object_path) { return Ok(c); }
    let c = std::sync::Arc::new(Collection::load(set, object_path)?);
    if let Ok(mut m) = LOADED.lock() { m.insert(object_path.to_owned(), c.clone()); }
    Ok(c)
}
/// A collection [`load_cached`] has loaded.
pub fn loaded(object_path: &str) -> Option<std::sync::Arc<Collection>> { LOADED.lock().ok().and_then(|m| m.get(object_path).cloned()) }

/// `/Game/Packages/Fortress_SharedMaterials/GlobalMaterialParameters/FortniteMaterialParameters`: the collection `AFortTimeOfDayManager` writes
/// (`TODM_A`'s `MaterialParameterCollection` property; the manager's +0xaa0 pointer in the exe).
pub const FORTNITE_MATERIAL_PARAMETERS: &str = "/Game/Packages/Fortress_SharedMaterials/GlobalMaterialParameters/FortniteMaterialParameters.FortniteMaterialParameters";

// ---- what the time-of-day manager writes into FortniteMaterialParameters -------------------------------------------------------------------------------------
// NOT WRITTEN (no program the port draws reads them; each is listed with its 1.8 writer):
// * PlayerPosition / PlayerCapsuleBottomPosition / PlayerDirection (FUN_14064da00: the local pawn's root location, its capsule bottom, its rotation vector);
// * SunLightColor / FogColor / FogDirectionalInscatteringColor (FUN_14064da00: CurrentTimeOfDayDirectionalLightColor +0xa90, CurrentTimeOfDayFogValues
//   .FogInscatteringColor +0xa70 / .DirectionalInscatteringColor +0xa60; the manager's own blend of those is not ported);
// * SunlightYTransformVector / SunlightZTransformVector (FUN_14064da00: the Y / Z axes of DirectionalLightRotation +0x400, roll included);
// * CloudsDirectSunLightColor / CloudsZenithColor (FUN_14063eba0: the cloud MID's "Clouds - Direct Sun Light Color" / "Clouds - Zenith Color" after the phase
//   blend, alpha included);
// * Foliage Directional Wind Intensity / StormStrength (TODM_A ubergraph 0x86 / 0x202: only inside the weather-storm timeline, which Athena does not play);
// * the wind values (AFortWindManager, a separate item). Every parameter not written keeps its DefaultValue.

/// `TODM_A` `UserConstructionScript` (offsets 0x0 / 0x1c): `World border cloud diffuse brightness` = the manager's `World Border Cloud Base Color Brightness`,
/// `World Border Cloud Sub Surface Color and Opacity` = its namesake (TODM_A CDO values; TODM_A_Child_Athena does not override them).
pub const WORLD_BORDER_CLOUD_BASE_COLOR_BRIGHTNESS: f32 = 0.5;
pub const WORLD_BORDER_CLOUD_SUB_SURFACE_COLOR_AND_OPACITY: [f32; 4] = [0.5, 0.475_136_01, 0.332_892, 0.2];
/// The construction script's FortniteMaterialParameters writes (the actor is constructed when the world spawns it, before it begins play).
pub fn construction_script(c: &Collection, inst: &mut Instance) {
    inst.set_scalar(c, "World border cloud diffuse brightness", WORLD_BORDER_CLOUD_BASE_COLOR_BRIGHTNESS);
    inst.set_vector(c, "World Border Cloud Sub Surface Color and Opacity", WORLD_BORDER_CLOUD_SUB_SURFACE_COLOR_AND_OPACITY);
}
/// `TODM_A` `ReceiveBeginPlay` (ubergraph 0x22f..0x3ae): `AreDistanceFieldsEnabled?` = `GetGameUserSettings()->GetShadowQuality() > 2` as float (every switch case
/// of the level lands on the same 0x307 block), `ShadowDistance` / `ShadowDistanceFadeoutFraction` = the manager's `DirectionalLightComponent`
/// `DynamicShadowDistanceMovableLight` / `ShadowDistanceFadeoutFraction` (`fn_render::lighting::SHADOW_DISTANCE_M` * 100 = 4500 cm, serialized in TODM_A;
/// `SHADOW_DISTANCE_FADEOUT_FRACTION` = the component default 0.1). Written once: a later settings change keeps the begin-play values.
#[cfg(feature = "render")]
pub fn begin_play(c: &Collection, inst: &mut Instance, shadow_quality: u32) {
    inst.set_scalar(c, "AreDistanceFieldsEnabled?", if shadow_quality > 2 { 1. } else { 0. });
    inst.set_scalar(c, "ShadowDistance", fn_render::lighting::SHADOW_DISTANCE_M * 100.);
    inst.set_scalar(c, "ShadowDistanceFadeoutFraction", fn_render::lighting::SHADOW_DISTANCE_FADEOUT_FRACTION);
}
/// `TimeOfDay_0-1` of FUN_14064da00: `TimeOfDay * 0.041666668`, clamped to 1 when not negative, else 0.
pub fn time_of_day_0_1(hour: f32) -> f32 { let x = hour * 0.041_666_668; if x >= 0. { x.min(1.) } else { 0. } }
/// One manager tick (`FUN_14064e920`) at `hour` with the world's `RealTimeSeconds` = `real_time_s`:
/// * `PositionTheSunAndMoonModels` (@0x140641e30): `SunlightDirectionalVector` = `SunAndMoonModelDirectionalVector` = the model direction,
///   `SunAndMoonModelWorldPosition` = direction * `DistanceToSunOrMoon` (UE cm, world origin; `FLinearColor(FVector)`: alpha 1);
/// * FUN_14064f050: `SkyLightColorAndIntensity` = the blended `SkyLightColor` (rgb + alpha);
/// * FUN_14064da00: `FortniteActiveTimeOfDay` = (FUN_140635510(p), p = 0..3) = the phase weights, `RealGameTime`, `TimeOfDay_0-1`.
/// LIMITS: `SunAndMoonModelWorldPosition` is UE world cm without the level -> FX `offset_cm` (right while the TOD actor sits at the FX origin, as the sky port
/// assumes; a cooked reader drawn with a level offset would need it added). `real_time_s` is the caller's clock (fn-app: time since the app started, not since
/// the world began play; scripted runs use the frame clock).
#[cfg(feature = "render")]
pub fn tick(c: &Collection, inst: &mut Instance, hour: f32, real_time_s: f32) {
    use fn_render::time_of_day as tod;
    let st = tod::state_at(hour);
    let d = tod::sun_model_direction_ue(hour);
    let p = d * tod::DISTANCE_TO_SUN_OR_MOON_CM;
    inst.set_vector(c, "SunlightDirectionalVector", [d.x, d.y, d.z, 1.]);
    inst.set_vector(c, "SunAndMoonModelDirectionalVector", [d.x, d.y, d.z, 1.]);
    inst.set_vector(c, "SunAndMoonModelWorldPosition", [p.x, p.y, p.z, 1.]);
    inst.set_vector(c, "SkyLightColorAndIntensity", [st.sky_light_color[0], st.sky_light_color[1], st.sky_light_color[2], st.sky_light_alpha]);
    inst.set_vector(c, "FortniteActiveTimeOfDay", st.weights);
    inst.set_scalar(c, "RealGameTime", real_time_s);
    inst.set_scalar(c, "TimeOfDay_0-1", time_of_day_0_1(hour));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Collection {
        Collection { path: "x".into(), state_id: String::new(), scalars: (0..6).map(|i| (format!("s{i}"), i as f32)).collect(), vectors: vec![("v0".into(), [1., 2., 3., 4.]), ("v1".into(), [5., 6., 7., 8.])] }
    }
    /// Scalars four per float4 first (6 scalars -> 2 float4s, the last half empty), then one float4 per vector; instance values replace defaults by name.
    #[test] fn layout_matches_get_parameter_data() {
        let c = sample();
        assert_eq!(c.float4s(), 4);
        assert_eq!(c.data(&Instance::default()), vec![[0., 1., 2., 3.], [4., 5., 0., 0.], [1., 2., 3., 4.], [5., 6., 7., 8.]]);
        let mut i = Instance::default();
        assert!(i.set_scalar(&c, "s5", 9.) && i.set_vector(&c, "v1", [0.; 4]));
        assert!(!i.set_scalar(&c, "nope", 1.) && !i.set_vector(&c, "s1", [1.; 4]), "unknown names are not stored");
        assert_eq!(c.data(&i), vec![[0., 1., 2., 3.], [4., 9., 0., 0.], [1., 2., 3., 4.], [0.; 4]]);
        assert_eq!((c.index_of("s5"), c.index_of("v1"), c.index_of("x")), (Some((1, Some(1))), Some((3, None)), None));
    }
    /// The cooked FortniteMaterialParameters: 59 scalars + 48 vectors = 63 float4s; ShadowDistance at 10.w, AreDistanceFieldsEnabled? at 11.x,
    /// ShadowDistanceFadeoutFraction at 11.y (what the foliage base pass reads), FortniteActiveTimeOfDay = float4 15, SkyLightColorAndIntensity = float4 41.
    #[test] fn fortnite_material_parameters_layout() {
        let Ok(set) = fn_pak::PakSet::open_install(crate::default_install()) else { eprintln!("no 1.8 install: skipped"); return };
        let c = Collection::load(&set, FORTNITE_MATERIAL_PARAMETERS).unwrap();
        assert_eq!((c.scalars.len(), c.vectors.len(), c.float4s()), (59, 48, 63));
        assert_eq!(c.state_id, "b3369f45e60e794497e281f5c5b5f955");
        assert_eq!(c.index_of("ShadowDistance"), Some((10, Some(3))));
        assert_eq!(c.index_of("AreDistanceFieldsEnabled?"), Some((11, Some(0))));
        assert_eq!(c.index_of("ShadowDistanceFadeoutFraction"), Some((11, Some(1))));
        assert_eq!(c.index_of("FortniteActiveTimeOfDay"), Some((15, None)));
        assert_eq!(c.index_of("SkyLightColorAndIntensity"), Some((41, None)));
        let d = c.data(&Instance::default());
        assert_eq!((d[10][3], d[11][0], d[11][1], d[15]), (4500., 0., 0.1, [0., 1., 0., 0.]));
        // every parameter the writers set exists in the cooked collection (set_* would silently drop an unknown name)
        let mut i = Instance::default();
        construction_script(&c, &mut i);
        assert_eq!((i.scalars.len(), i.vectors.len()), (1, 1));
        #[cfg(feature = "render")] {
            begin_play(&c, &mut i, 3);
            tick(&c, &mut i, 12., 5.);
            assert_eq!((i.scalars.len(), i.vectors.len()), (1 + 3 + 2, 1 + 5));
            let d = c.data(&i);
            assert_eq!((d[10][3], d[11][0], d[11][1]), (4500., 1., 0.1), "ShadowDistance, AreDistanceFieldsEnabled? (Epic), ShadowDistanceFadeoutFraction");
            assert_eq!(c.index_of("SunAndMoonModelWorldPosition"), Some((27, None)), "the float4 the cloud dome's pixel program reads (cb2[27])");
        }
    }
    /// The construction-script values are the TODM_A CDO's (the child does not override them).
    #[test] fn construction_script_values_are_the_todm_cdo() {
        let Ok(set) = fn_pak::PakSet::open_install(crate::default_install()) else { eprintln!("no 1.8 install: skipped"); return };
        let num = |v: &Value| v.as_f64().unwrap() as f32;
        for (pkg, own) in [("/Game/TimeOfDay/TODM/TODM_A", true), ("/Game/TimeOfDay/TODM/TODM_A_Child_Athena", false)] {
            let p = PackageJson::load(&set, pkg).unwrap();
            let cdo = p.exports.iter().find(|e| e.name.starts_with("Default__")).unwrap();
            let b = cdo.props.get("World Border Cloud Base Color Brightness");
            let s = cdo.props.get("World Border Cloud Sub Surface Color and Opacity");
            if !own { assert!(b.is_none() && s.is_none(), "{pkg} overrides the world border cloud values"); continue; }
            assert_eq!(num(b.unwrap()), WORLD_BORDER_CLOUD_BASE_COLOR_BRIGHTNESS);
            assert_eq!(color(s.unwrap()).unwrap(), WORLD_BORDER_CLOUD_SUB_SURFACE_COLOR_AND_OPACITY);
        }
    }
    #[test] fn time_of_day_0_1_clamps_like_fun_14064da00() {
        assert_eq!((time_of_day_0_1(12.), time_of_day_0_1(24.), time_of_day_0_1(30.), time_of_day_0_1(-1.)), (0.5, 1., 1., 0.));
        assert_eq!(time_of_day_0_1(6.), 6. * 0.041_666_668);
    }
    /// The begin-play shadow values follow the sg.ShadowQuality level (`> 2`), the tick values the clock.
    #[cfg(feature = "render")]
    #[test] fn begin_play_and_tick_values() {
        let names = |v: &[&str]| v.iter().map(|n| (n.to_string(), 0.)).collect::<Vec<_>>();
        let c = Collection { path: "x".into(), state_id: String::new(), scalars: names(&["AreDistanceFieldsEnabled?", "ShadowDistance", "ShadowDistanceFadeoutFraction", "RealGameTime", "TimeOfDay_0-1"]),
            vectors: ["FortniteActiveTimeOfDay", "SunlightDirectionalVector", "SunAndMoonModelDirectionalVector", "SunAndMoonModelWorldPosition", "SkyLightColorAndIntensity"].iter().map(|n| (n.to_string(), [0.; 4])).collect() };
        for (level, df) in [(0, 0.), (1, 0.), (2, 0.), (3, 1.)] {
            let mut i = Instance::default();
            begin_play(&c, &mut i, level);
            assert_eq!((i.scalars["AreDistanceFieldsEnabled?"], i.scalars["ShadowDistance"], i.scalars["ShadowDistanceFadeoutFraction"]), (df, 4500., 0.1), "level {level}");
        }
        let mut i = Instance::default();
        let h = 17.25;
        tick(&c, &mut i, h, 42.);
        let st = fn_render::time_of_day::state_at(h);
        assert_eq!(i.vectors["FortniteActiveTimeOfDay"], st.weights);
        assert_eq!((i.scalars["RealGameTime"], i.scalars["TimeOfDay_0-1"]), (42., time_of_day_0_1(h)));
        let d = fn_render::time_of_day::sun_model_direction_ue(h);
        let p = i.vectors["SunAndMoonModelWorldPosition"];
        assert!((glam::Vec3::new(p[0], p[1], p[2]) - d * 7_920_000.).length() < 1., "position = direction * DistanceToSunOrMoon");
        assert_eq!((i.vectors["SunlightDirectionalVector"][3], p[3]), (1., 1.), "FLinearColor(FVector) alpha");
        assert_eq!(i.vectors["SkyLightColorAndIntensity"], [st.sky_light_color[0], st.sky_light_color[1], st.sky_light_color[2], st.sky_light_alpha]);
    }
}
