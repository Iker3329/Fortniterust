//! COOKED SKY (2026-10-05): the 1.8 Athena sky drawn with the ORIGINAL base-pass programs of its materials (cooked `FMaterialShaderMap`s, DXBC translated by
//! `fn-dxbc`, assembled by `fn-fx`), replacing the hand-written WGSL ports of `fn_render::sky` (sky_dome / sky_clouds / sky_sun.wgsl) for the dome, the cloud
//! dome and the sun disc. Drawn by the renderer's own sky FX instance (`Renderer::sky_fx`), in the sky slot of the frame (before the backdrop / scene).
//!
//! RECOVERED (package data / exe):
//! * the map's sky actor: `Athena_Terrain` `WorldSettings.WorldTimeOfDayManager` = `TODM_A_Child_Athena_C` (child of `TODM_A`).
//! * dome: `StarMapMeshComponent` (mesh `FortniteSkyDome_Optimized`, `RelativeScale3D` (37500, 37500, 18750)), override material `M_AridSky_Inst_Day` (parent
//!   `M_AridSky`, opaque `MSM_Unlit`); the child's day phases name `M_AridSky_Inst_{Morning,Day,Evening,Night}` as `StarMapMaterialInstance`.
//! * cloud dome: `SkyDomeMeshComp` (same mesh, scale (2000, 2000, 167.7), `RelativeLocation` (0, 0, -100000)), `M_AridClouds_Inst_Day` (parent `M_AridClouds`,
//!   translucent); phases `M_AridClouds_Inst_*` as `SkyMaterialInstance`.
//! * phase blend: `FUN_14063eba0` calls `UMaterialInstanceDynamic::K2_InterpolateMaterialInstanceParams(MID, phase[from].Sky/StarMapMaterialInstance,
//!   phase[to]..., alpha)` on the two MIDs: every scalar / vector parameter named by either instance is lerped, a name missing from one side takes the BASE
//!   material's default (UE4.16 MaterialInstanceDynamic.cpp:185-260). Implemented exactly here ([`interpolate`]).
//! * sun: `SunMeshComponent` with the child's `SunMesh` `SM_FlatSunMoon_Arid`, `SunMaterialOverrides` `M_Sun_Arid` (translucent unlit, separate translucency),
//!   `SunScale` 2000. `AFortTimeOfDayManager::PositionTheSunAndMoonModels` (exe `140641e30`) sets its WORLD location to `dir * distance` and its world rotation to
//!   `dir.Rotation()` (roll 0), `dir` = the slerped sun-model direction.
//! * `M_Sun_Arid` reads `MaterialCollection0` = `FortniteMaterialParameters` float4 15 (59 scalars packed 4 per float4 -> 15 float4s, then vector 0 =
//!   `FortniteActiveTimeOfDay`): the phase weights.
//!
//! RECOVERED SINCE (MATERIAL PARAMETERS, 2026-10-06): the sun distance is the manager's `DistanceToSunOrMoon` (+0x428) = 7.92e6 cm
//! (`fn_render::time_of_day::DISTANCE_TO_SUN_OR_MOON_CM`; callers pass it as `SkyInputs::sun_distance_cm`); `FortniteActiveTimeOfDay` is written by the manager
//! tick FUN_14064da00 as exactly the phase weights used here (`fn_fx::mpc::tick`); +0xaa0 is the manager's `MaterialParameterCollection` (FortniteMaterialParameters).
//! This module still keeps its own collection buffer (defaults + the weights) instead of the FX renderer's shared one (`fn_fx::gpu` / `fn-app mpc_hook.rs`): it
//! is not drawn by the app (the sky is `fn_render::sky`), so the clouds' `SunAndMoonModelWorldPosition` keeps its default here.
//! NOT RECOVERED / ASSUMED (labelled): the TOD actor's location (taken as the level origin, mapped like the storm with `offset_cm`); the MID parent at creation
//! (the component's `*_Inst_Day` is assumed, so parameters named by neither phase keep the Day instance values). `FUN_14063eba0` has two parts not applied here:
//! the storm-time re-interpolation of the sky MID (gated on `StormStrength` > 0 and `StormMaterialInst`) and, when the collection is set (+0xaa0), the copy of
//! the cloud MID's "Clouds - Direct Sun Light Color" / "Clouds - Zenith Color" into the collection's `CloudsDirectSunLightColor` / `CloudsZenithColor`.
use fn_fx::data::PackageJson;
use fn_fx::gpu::{build_material_ext, load_mesh, MaterialGpu, MeshGpu, SharedBuffers, TextureCache, Vf};
use fn_fx::material::Material;
use fn_fx::uniform::{Expr, MaterialTime, Overrides};
use fn_fx::view::{bytes, ViewParams};
use fn_pak::PakSet;
use fn_render::fx::*;
use fn_render::primitive_ub::{mesh_primitive, PrimitiveFlags, PRIMITIVE_FLOAT4S};
use glam::{Mat4, Vec3};
use std::collections::HashMap;

pub const DOME_MESH: &str = "/Game/Packages/Fortress_Sky/Mesh_NoLOD/FortniteSkyDome_Optimized.FortniteSkyDome_Optimized";
pub const SUN_MESH: &str = "/Game/Packages/Fortress_Sky/MaterialInstance/Arid/SM_FlatSunMoon_Arid.SM_FlatSunMoon_Arid";
pub const SUN_MATERIAL: &str = "/Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_Sun_Arid.M_Sun_Arid";
pub const MPC: &str = "/Game/Packages/Fortress_SharedMaterials/GlobalMaterialParameters/FortniteMaterialParameters";
pub const PHASES: [&str; 4] = ["Morning", "Day", "Evening", "Night"];
pub fn sky_instance(phase: &str) -> String { format!("/Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_AridSky_Inst_{phase}.M_AridSky_Inst_{phase}") }
pub fn cloud_instance(phase: &str) -> String { format!("/Game/Packages/Fortress_Sky/MaterialInstance/Arid/M_AridClouds_Inst_{phase}.M_AridClouds_Inst_{phase}") }
pub const BASEPASS_VS: &str = "TBasePassVSFNoLightMapPolicy";
pub const BASEPASS_PS: &str = "TBasePassPSFNoLightMapPolicy";
/// `TODM_A_Child_Athena` component transforms (UE cm).
pub const STAR_DOME_SCALE: [f32; 3] = [37500., 37500., 18750.];
pub const CLOUD_DOME_SCALE: [f32; 3] = [2000., 2000., 167.699_97];
pub const CLOUD_DOME_Z_CM: f32 = -100000.;
pub const SUN_SCALE: f32 = 2000.;
/// Near / far (m) of the sky's own View buffer: the main pass far plane (3 km) would clip the 1557 km dome; UE draws it with an infinite far plane.
pub const SKY_NEAR_M: f32 = 100.;
pub const SKY_FAR_M: f32 = 4.0e6;

/// Default value of every scalar / vector parameter named in the material's uniform expression set (the BASE material defaults compiled into the shader map).
pub fn parameter_defaults(m: &Material) -> (HashMap<String, f32>, HashMap<String, [f32; 4]>) {
    fn walk(e: &Expr, s: &mut HashMap<String, f32>, v: &mut HashMap<String, [f32; 4]>) {
        match e {
            Expr::VectorParameter { name, default } => { v.entry(name.clone()).or_insert(*default); }
            Expr::ScalarParameter { name, default } => { s.entry(name.clone()).or_insert(*default); }
            Expr::ComponentSwizzle { e, .. } | Expr::Unary(_, e) => walk(e, s, v),
            Expr::FoldedMath { a, b, .. } | Expr::AppendVector { a, b, .. } | Expr::Max(a, b) | Expr::Min(a, b) | Expr::Fmod(a, b) => { walk(a, s, v); walk(b, s, v); }
            Expr::TrigMath { x, y, .. } => { walk(x, s, v); walk(y, s, v); }
            Expr::Clamp { x, lo, hi } => { walk(x, s, v); walk(lo, s, v); walk(hi, s, v); }
            _ => {}
        }
    }
    let (mut s, mut v) = (HashMap::new(), HashMap::new());
    let u = &m.uniform;
    for e in u.vectors.iter().chain(&u.scalars).chain(&u.per_frame_scalars).chain(&u.per_frame_vectors) { walk(e, &mut s, &mut v); }
    (s, v)
}

/// `K2_InterpolateMaterialInstanceParams(A, B, alpha)` applied to a MID whose current parameters are `base`: every name of either instance is set to
/// `Lerp(A or default, B or default, alpha)` (UE4.16 MaterialInstanceDynamic.cpp:185).
pub fn interpolate(base: &Overrides, a: &Overrides, b: &Overrides, alpha: f32, defaults: &(HashMap<String, f32>, HashMap<String, [f32; 4]>)) -> Overrides {
    let mut out = base.clone();
    let lerp = |x: f32, y: f32| x + (y - x) * alpha;
    for n in a.scalars.keys().chain(b.scalars.keys()) {
        let d = defaults.0.get(n).copied().unwrap_or(0.);
        out.scalars.insert(n.clone(), lerp(a.scalars.get(n).copied().unwrap_or(d), b.scalars.get(n).copied().unwrap_or(d)));
    }
    for n in a.vectors.keys().chain(b.vectors.keys()) {
        let d = defaults.1.get(n).copied().unwrap_or([0.; 4]);
        let (x, y) = (a.vectors.get(n).copied().unwrap_or(d), b.vectors.get(n).copied().unwrap_or(d));
        out.vectors.insert(n.clone(), std::array::from_fn(|k| lerp(x[k], y[k])));
    }
    out
}

/// `FVector::Rotation()` (pitch, yaw; roll 0) as the component rotation matrix (`FRotationMatrix`): X = the direction, Y horizontal.
pub fn rotation_of(dir: Vec3) -> Mat4 {
    let yaw = dir.y.atan2(dir.x);
    let pitch = dir.z.atan2((dir.x * dir.x + dir.y * dir.y).sqrt());
    // UE FRotator (P, Y, 0) = yaw about +Z, then pitch about -Y (left handed): X axis (CP CY, CP SY, SP), Y (-SY, CY, 0), Z (-SP CY, -SP SY, CP)
    let (sp, cp, sy, cy) = (pitch.sin(), pitch.cos(), yaw.sin(), yaw.cos());
    Mat4::from_cols(glam::Vec4::new(cp * cy, cp * sy, sp, 0.), glam::Vec4::new(-sy, cy, 0., 0.), glam::Vec4::new(-sp * cy, -sp * sy, cp, 0.), glam::Vec4::W)
}

/// `FortniteMaterialParameters` collection buffer: the scalar defaults packed 4 per float4, then one float4 per vector default (UE4.16
/// MaterialParameterCollection.cpp `GetDefaultParameterData`), with the vector names in order.
pub fn collection_defaults(set: &PakSet) -> Result<(Vec<[f32; 4]>, Vec<String>), String> {
    let pkg = PackageJson::load(set, MPC)?;
    let e = pkg.of_class("MaterialParameterCollection").next().ok_or_else(|| format!("{MPC}: no MaterialParameterCollection export"))?;
    let num = |v: &serde_json::Value| v.as_f64().unwrap_or(0.) as f32;
    let scalars: Vec<f32> = e.props.get("ScalarParameters").and_then(|v| v.as_array()).map(|a| a.iter().map(|p| p.get("DefaultValue").map(num).unwrap_or(0.)).collect()).unwrap_or_default();
    let mut out = vec![[0f32; 4]; scalars.len().div_ceil(4)];
    for (i, s) in scalars.iter().enumerate() { out[i / 4][i % 4] = *s; }
    let mut names = vec![];
    for p in e.props.get("VectorParameters").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
        let d = p.get("DefaultValue");
        let c = |k: &str, i: usize| d.and_then(|d| d.get(k).or_else(|| d.as_array().and_then(|a| a.get(i)))).map(num).unwrap_or(0.);
        out.push([c("R", 0), c("G", 1), c("B", 2), c("A", 3)]);
        names.push(p.get("ParameterName").and_then(|v| v.as_str()).unwrap_or("").to_owned());
    }
    Ok((out, names))
}

/// One frame's inputs (all from the port's clock).
#[derive(Clone, Copy, Debug)]
pub struct SkyInputs {
    /// Phase blend (`time_of_day::TodState` from / to / blend).
    pub from: usize,
    pub to: usize,
    pub alpha: f32,
    /// `FortniteActiveTimeOfDay` (morning, day, evening, night).
    pub weights: [f32; 4],
    /// Unit vector towards the sun MODEL, Unreal axes.
    pub sun_dir_ue: [f32; 3],
    pub sun_distance_cm: f32,
}

struct Part { mat: Material, gpu: MaterialGpu, binds: Vec<(u32, FxBindId)>, prim: FxBufId, vs: Option<FxBufId>, prev: Option<Mat4> }

pub struct SkyWorld {
    view_buf: FxBufId,
    light_buf: FxBufId,
    collection_buf: FxBufId,
    collection: Vec<[f32; 4]>,
    active_tod_slot: Option<usize>,
    dome: Part,
    clouds: Part,
    sun: Part,
    /// The four phase instances (Morning, Day, Evening, Night) of the dome / cloud materials and their base defaults.
    sky_phases: Vec<Overrides>,
    cloud_phases: Vec<Overrides>,
    sky_defaults: (HashMap<String, f32>, HashMap<String, [f32; 4]>),
    cloud_defaults: (HashMap<String, f32>, HashMap<String, [f32; 4]>),
    dome_mesh: MeshGpu,
    sun_mesh: MeshGpu,
    pub notes: Vec<String>,
}

fn part(set: &PakSet, fx: &mut FxRenderer, cache: &mut TextureCache, path: &str, mesh: &MeshGpu, shared: SharedBuffers, named: &[(&str, FxBufId)], label: &str) -> Result<Part, String> {
    let mat = Material::load(set, path)?;
    let prim = fx.create_buffer(&format!("sky {label} primitive"), PRIMITIVE_FLOAT4S * 16);
    let gpu = build_material_ext(set, fx, cache, &mat, Vf::Local, BASEPASS_VS, BASEPASS_PS, mesh.num_uv, false, SharedBuffers { primitive: prim, ..shared }, named)?;
    let vs = gpu.vs_globals().map(|g| fx.create_buffer(&format!("sky {label} vertex globals"), g.float4s * 16));
    let binds = gpu.bind_variant_globals(fx, prim, None, vs);
    Ok(Part { mat, gpu, binds, prim, vs, prev: None })
}

impl SkyWorld {
    pub fn load(set: &PakSet, fx: &mut FxRenderer) -> Result<SkyWorld, String> {
        let view_buf = fx.create_buffer("sky view", fn_fx::view::VIEW_FLOAT4S * 16);
        let light_buf = fx.create_buffer("sky light", fn_render::sky_light::FX_LIGHT_BYTES);
        let collection_buf = fx.create_buffer("sky MaterialCollection0 (FortniteMaterialParameters)", 128 * 16);
        let sprite_vf = fx.create_buffer("sky sprite vf (unused)", 10 * 16);
        let shared = SharedBuffers { view: view_buf, primitive: view_buf, sprite_vf, light: light_buf };
        let named = [("MaterialCollection0", collection_buf)];
        let mut cache = TextureCache::default();
        let dome_mesh = load_mesh(set, fx, Vf::Local, DOME_MESH)?;
        let sun_mesh = load_mesh(set, fx, Vf::Local, SUN_MESH)?;
        let dome = part(set, fx, &mut cache, &sky_instance("Day"), &dome_mesh, shared, &named, "dome")?;
        let clouds = part(set, fx, &mut cache, &cloud_instance("Day"), &dome_mesh, shared, &named, "clouds")?;
        let sun = part(set, fx, &mut cache, SUN_MATERIAL, &sun_mesh, shared, &named, "sun")?;
        let mut sky_phases = vec![];
        let mut cloud_phases = vec![];
        for p in PHASES {
            sky_phases.push(Material::load(set, &sky_instance(p))?.overrides);
            cloud_phases.push(Material::load(set, &cloud_instance(p))?.overrides);
        }
        let (collection, names) = collection_defaults(set)?;
        let active_tod_slot = names.iter().position(|n| n == "FortniteActiveTimeOfDay").map(|i| collection.len() - names.len() + i);
        let mut notes = vec![];
        for (n, p) in [("dome", &dome), ("clouds", &clouds), ("sun", &sun)] {
            notes.push(format!("{n}: {} ({} / {}, two sided {}, translucent {}, separate translucency {})", p.mat.path.rsplit('.').next().unwrap_or(""), p.mat.flags.blend, p.mat.flags.shading, p.mat.flags.two_sided, p.gpu.translucent, p.mat.flags.separate_translucency));
        }
        notes.push(format!("FortniteMaterialParameters: {} float4s, FortniteActiveTimeOfDay at {:?}", collection.len(), active_tod_slot));
        let sky_defaults = parameter_defaults(&dome.mat);
        let cloud_defaults = parameter_defaults(&clouds.mat);
        Ok(SkyWorld { view_buf, light_buf, collection_buf, collection, active_tod_slot, dome, clouds, sun, sky_phases, cloud_phases, sky_defaults, cloud_defaults, dome_mesh, sun_mesh, notes })
    }

    /// The sky's own View: the frame's camera with [`SKY_NEAR_M`] / [`SKY_FAR_M`].
    pub fn sky_view(view: &ViewParams) -> ViewParams { ViewParams { near_m: SKY_NEAR_M, far_m: SKY_FAR_M, ..*view } }

    /// The FX draws of one frame: dome (opaque), cloud dome (standard translucency), sun (separate translucency). `offset_cm` = UE level cm -> FX cm (the TOD
    /// actor sits at the level origin, ASSUMED).
    pub fn frame(&mut self, view: &ViewParams, offset_cm: [f32; 3], inp: &SkyInputs, t: MaterialTime) -> FxFrame {
        let mut f = FxFrame::default();
        let sv = Self::sky_view(view);
        f.buffer_writes.push((self.view_buf, bytes(&sv.view_buffer())));
        f.buffer_writes.push((self.light_buf, bytes(&fn_render::sky_light::fx_light_block())));
        let mut coll = self.collection.clone();
        if let Some(s) = self.active_tod_slot { coll[s] = inp.weights; }
        f.buffer_writes.push((self.collection_buf, bytes(&coll)));
        // phase blend of the two MIDs (FUN_14063eba0), the MID parents being the components' *_Inst_Day
        let (a, b) = (inp.from.min(3), inp.to.min(3));
        self.dome.mat.overrides = interpolate(&self.sky_phases[1], &self.sky_phases[a], &self.sky_phases[b], inp.alpha, &self.sky_defaults);
        self.clouds.mat.overrides = interpolate(&self.cloud_phases[1], &self.cloud_phases[a], &self.cloud_phases[b], inp.alpha, &self.cloud_defaults);
        let origin = Vec3::from(offset_cm);
        let d = Vec3::from(inp.sun_dir_ue).normalize_or_zero();
        let flags = PrimitiveFlags::default();
        let parts: [(&mut Part, &MeshGpu, Mat4); 3] = [
            (&mut self.dome, &self.dome_mesh, Mat4::from_translation(origin) * Mat4::from_scale(Vec3::from(STAR_DOME_SCALE))),
            (&mut self.clouds, &self.dome_mesh, Mat4::from_translation(origin + Vec3::new(0., 0., CLOUD_DOME_Z_CM)) * Mat4::from_scale(Vec3::from(CLOUD_DOME_SCALE))),
            // PositionTheSunAndMoonModels: world location dir * distance (absolute), world rotation dir.Rotation(); scale SunScale
            (&mut self.sun, &self.sun_mesh, Mat4::from_translation(origin + d * inp.sun_distance_cm) * rotation_of(d) * Mat4::from_scale(Vec3::splat(SUN_SCALE))),
        ];
        for (p, mesh, m) in parts {
            f.buffer_writes.push((p.gpu.material_buf, bytes(&p.mat.packed(t.world).cb)));
            f.buffer_writes.extend(p.gpu.per_frame_writes(&p.mat, t));
            f.buffer_writes.push((p.prim, bytes(&mesh_primitive(&m, &mesh.bounds, offset_cm, &flags))));
            let prev = Some(p.prev.unwrap_or(m));
            if let (Some(buf), Some(by)) = (p.vs, p.gpu.vs_globals_bytes(&p.mat, t, prev)) { f.buffer_writes.push((buf, by)); }
            p.prev = Some(m);
            let sort = sv.sort(mesh.bounds.transform_by(&m).origin);
            for &(slot, first, count) in &mesh.sections { if slot == 0 { f.push(FxDraw { pipe: p.gpu.pipe, binds: p.binds.clone(), geom: FxGeom::MeshRange { mesh: mesh.id, first_index: first, index_count: count }, instances: None, sort }); } }
        }
        f
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn set() -> PakSet { PakSet::open_install(fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install")).unwrap() }
    #[test]
    fn rotation_points_x_along_the_direction() {
        for d in [Vec3::new(1., 2., 3.).normalize(), Vec3::new(-0.3, 0.1, 0.9).normalize(), Vec3::X] {
            let m = rotation_of(d);
            assert!((m.transform_vector3(Vec3::X) - d).length() < 1e-5, "{d:?}");
            assert!(m.transform_vector3(Vec3::Y).z.abs() < 1e-6, "roll 0 keeps Y horizontal");
            assert!((glam::Mat3::from_mat4(m).determinant() - 1.).abs() < 1e-5);
        }
    }
    #[test]
    fn interpolation_takes_the_union_and_base_defaults() {
        let mut a = Overrides::default(); a.scalars.insert("x".into(), 2.);
        let mut b = Overrides::default(); b.scalars.insert("y".into(), 4.);
        let defaults = (HashMap::from([("x".to_owned(), 0.), ("y".to_owned(), 1.)]), HashMap::new());
        let o = interpolate(&Overrides::default(), &a, &b, 0.5, &defaults);
        assert_eq!(o.scalars["x"], 1.); // lerp(2, default 0)
        assert_eq!(o.scalars["y"], 2.5); // lerp(default 1, 4)
    }
    #[test]
    fn the_cooked_sky_materials_resolve() {
        let set = set();
        for (path, blend) in [(sky_instance("Day"), "BLEND_Opaque"), (cloud_instance("Day"), "BLEND_Translucent"), (SUN_MATERIAL.to_owned(), "BLEND_Translucent")] {
            let m = Material::load(&set, &path).unwrap();
            assert_eq!(m.flags.blend, blend, "{path}");
            for ty in [BASEPASS_VS, BASEPASS_PS] { assert!(m.cooked.find("FLocalVertexFactory", ty).is_some(), "{path}: {ty}"); }
        }
        let (c, names) = collection_defaults(&set).unwrap();
        let i = names.iter().position(|n| n == "FortniteActiveTimeOfDay").unwrap();
        assert_eq!(c.len() - names.len() + i, 15, "M_Sun_Arid reads MaterialCollection0[15]");
    }
}
