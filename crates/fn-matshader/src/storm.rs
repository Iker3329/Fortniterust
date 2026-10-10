//! The BR storm as the original 1.8 data draws it: the safe-zone wall and the storm cloud sheet, both through the FX pass of `fn-render` with the ORIGINAL base-pass
//! programs of their materials (cooked `FMaterialShaderMap`s, DXBC translated by `fn-dxbc`, assembled by `fn-fx`).
//!
//! RECOVERED (package data, see `fn_render::storm` for the indicator / native actor numbers):
//! * wall: `SafeZoneIndicator.SafeZoneCylinder` = `StaticMeshComponent` of mesh `SafeZoneCylinder` with the override material `M_SafeZoneSphere` (`BLEND_Translucent`,
//!   `MSM_Unlit`, two sided, a blue scanline / grid / intersection-edge shield that reads the scene depth). Permutation: `FLocalVertexFactory`,
//!   `TBasePassVSFNoLightMapPolicy` + `TBasePassPSFNoLightMapPolicy` (a translucent unlit surface has no lightmap; the sky-light / fog variants belong to other passes).
//!   Its constant buffers are the program's own `View`, `Primitive`, `Material` (the material defaults: `HeightGradientScale` 16000 ...).
//! * clouds: `BP_SafeZoneStorm` components with mesh `StormQuadMain` (a 614 m wide displaced grid, bounds z -650 m .. +300 m) and material `M_SafeZoneStorm`
//!   (`BLEND_Masked`, `MSM_DefaultLit`, two sided, world position offset from three textures, reads `MPC_SafeZoneClouds` through the `MaterialCollection0` buffer).
//!   Masked materials draw in the FX opaque pass (depth written); the depth-only pixel shader clips (`fn_fx::program::assemble`).
//!
//! The `MaterialCollection0` `MasterOpacity` member is written but no wall program reads it: the wall is always drawn (the StormFadeIn timeline only reaches programs that read the collection).
//!
//! NOT DONE / APPROXIMATED (labelled where it happens):the lit cloud shading is the FX pass's own sun + sky-hemisphere shading of the program's GBuffer outputs, not the
//! deferred light accumulation; no exponential height fog is applied (the FX `View` buffer carries no fog parameters, so the programs see "no fog"); no reflection
//! captures / sky-light SH (zero in the FX `View` buffer: the sky share comes from the FX light buffer); the `FadeSafeZoneVisuals` timeline writes `HeightGradientScale`
//! = `Lerp(0, 0, alpha)` in the Blueprint bytecode (both literals are 0.0), whether / when the timeline plays was not recovered, so the material default is kept.
use fn_fx::data::PackageJson;
use fn_fx::gpu::{build_material_ext, load_mesh, MaterialGpu, MeshGpu, SharedBuffers, TextureCache, Vf};
use fn_fx::material::Material;
use fn_fx::uniform::MaterialTime;
use fn_fx::view::{bytes, ViewParams};
use fn_pak::PakSet;
use fn_render::fx::*;
use fn_render::primitive_ub::{mesh_primitive, PrimitiveFlags, PRIMITIVE_FLOAT4S};
use fn_render::storm::{StormParams, STORM_ACTOR_CM};
use glam::{Mat4, Vec3};

pub const SPHERE_MATERIAL: &str = "/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere";
pub const STORM_MATERIAL: &str = "/Game/Athena/SafeZone/M_SafeZoneStorm.M_SafeZoneStorm";
pub const CYLINDER_MESH: &str = "/Game/Athena/SafeZone/SafeZoneCylinder.SafeZoneCylinder";
pub const QUAD_MESH: &str = "/Game/VisualThreatManager/StormVisuals/LargeClouds/Mesh_NoLOD/StormQuadMain.StormQuadMain";
pub const STORM_BP: &str = "/Game/Athena/SafeZone/BP_SafeZoneStorm";
pub const BASEPASS_VS: &str = "TBasePassVSFNoLightMapPolicy";
pub const BASEPASS_PS: &str = "TBasePassPSFNoLightMapPolicy";
/// Name of the `MPC_SafeZoneClouds` uniform buffer in the programs' own buffer tables.
pub const COLLECTION_BUFFER: &str = "MaterialCollection0";
/// The collection `M_SafeZoneStorm`'s `MaterialCollection0` resolves to (`MaterialParameterCollectionInfos[0]`, StateId aadb8df1084f4d499a3c6acde694e449).
pub const SAFE_ZONE_CLOUDS: &str = "/Game/Athena/SafeZone/MPC_SafeZoneClouds.MPC_SafeZoneClouds";

/// One `StormQuadMain` component of `BP_SafeZoneStorm`: location relative to the actor root (cm) and scale.
#[derive(Clone, Debug, PartialEq)]
pub struct QuadInstance { pub name: String, pub rel_cm: [f32; 3], pub scale: [f32; 3] }

fn v3(v: Option<&serde_json::Value>, d: [f32; 3]) -> [f32; 3] {
    v.and_then(|a| a.as_array()).filter(|a| a.len() == 3).map(|a| [a[0].as_f64().unwrap_or(0.) as f32, a[1].as_f64().unwrap_or(0.) as f32, a[2].as_f64().unwrap_or(0.) as f32]).unwrap_or(d)
}

/// Every `StaticMeshComponent` of `BP_SafeZoneStorm` that shows `StormQuadMain` (the Blueprint's component templates; the level instance `BP_SafeZoneStorm_1322` only moves the
/// root, its components inherit the template values). Refuses a rotated component (none is serialized; a rotation would need the Rotator -> matrix port).
pub fn load_quads(set: &PakSet) -> Result<Vec<QuadInstance>, String> {
    let pkg = PackageJson::load(set, STORM_BP)?;
    let mut out = vec![];
    for e in pkg.of_class("StaticMeshComponent") {
        if e.props.get("StaticMesh").and_then(|v| v.as_str()) != Some(QUAD_MESH) { continue; }
        if e.props.get("RelativeRotation").is_some() { return Err(format!("{}: rotated storm quad components are not supported", e.name)); }
        out.push(QuadInstance { name: e.name.clone(), rel_cm: v3(e.props.get("RelativeLocation"), [0.; 3]), scale: v3(e.props.get("RelativeScale3D"), [1.; 3]) });
    }
    if out.is_empty() { return Err(format!("{STORM_BP}: no StormQuadMain component")); }
    Ok(out)
}

/// The GPU side of the storm: materials, meshes, buffers and the per-component bind groups.
pub struct StormWorld {
    view_buf: FxBufId,
    light_buf: FxBufId,
    collection_buf: FxBufId,
    cylinder_prim: FxBufId,
    quad_prims: Vec<FxBufId>,
    /// Per-primitive vertex globals of the programs that have them (the cloud base-pass VS: `PreviousLocalToWorld` cb0[2..5], `SkipOutputVelocity` cb0[6].x,
    /// read from the cooked parameters, `fn_fx::shader_params`): (cylinder, quads).
    cylinder_vs: Option<FxBufId>,
    quad_vs: Vec<Option<FxBufId>>,
    /// Last frame's `LocalToWorld` of the cylinder / each quad (`None` before the first frame: the current transform is written, BasePassRendering.inl:44).
    prev_cylinder: Option<Mat4>,
    prev_quads: Vec<Option<Mat4>>,
    /// CAMERA CUT: `View.bCameraCut` of the next frame ([`Self::camera_cut`]).
    camera_cut: bool,
    wall: (Material, MaterialGpu, Vec<(u32, FxBindId)>),
    cloud: (Material, MaterialGpu, Vec<Vec<(u32, FxBindId)>>),
    cylinder: MeshGpu,
    quad: MeshGpu,
    pub quads: Vec<QuadInstance>,
    /// Notes for the log (what the programs ask for / what was left out).
    pub notes: Vec<String>,
}

impl StormWorld {
    pub fn load(set: &PakSet, fx: &mut FxRenderer) -> Result<StormWorld, String> {
        let quads = load_quads(set)?;
        let view_buf = fx.create_buffer("storm view", fn_fx::view::VIEW_FLOAT4S * 16);
        let light_buf = fx.create_buffer("storm light", fn_render::sky_light::FX_LIGHT_BYTES); // SKY LIGHT: FxLight with the SH (fn-fx program.rs)
        // MATERIAL PARAMETERS (2026-10-06): the world has one MPC_SafeZoneClouds buffer: the one an earlier reader created (fn_fx::gpu), else this one, which
        // every later program reading the collection binds
        let collection_buf = match fx.collection_buffer(SAFE_ZONE_CLOUDS) {
            Some(b) => b,
            None => { let b = fx.create_buffer("storm MaterialCollection0 (MPC_SafeZoneClouds)", 64 * 16); fx.register_collection_buffer(SAFE_ZONE_CLOUDS, b); b }
        };
        // FPrimitiveUniformShaderParameters is 18 float4s (fn_render::primitive_ub)
        let cylinder_prim = fx.create_buffer("storm cylinder primitive", PRIMITIVE_FLOAT4S * 16);
        let quad_prims: Vec<FxBufId> = quads.iter().map(|_| fx.create_buffer("storm quad primitive", PRIMITIVE_FLOAT4S * 16)).collect();
        let sprite_vf = fx.create_buffer("storm sprite vf (unused)", 10 * 16);
        let shared = SharedBuffers { view: view_buf, primitive: cylinder_prim, sprite_vf, light: light_buf };
        let mut cache = TextureCache::default();
        let cylinder = load_mesh(set, fx, Vf::Local, CYLINDER_MESH)?;
        let quad = load_mesh(set, fx, Vf::Local, QUAD_MESH)?;
        let named = [(COLLECTION_BUFFER, collection_buf)];
        let wall_mat = Material::load(set, SPHERE_MATERIAL)?;
        let wall_gpu = build_material_ext(set, fx, &mut cache, &wall_mat, Vf::Local, BASEPASS_VS, BASEPASS_PS, cylinder.num_uv, false, shared, &named)?;
        // per-primitive vertex globals when the program declares them (read from the cooked parameters, not inferred)
        let cylinder_vs = wall_gpu.vs_globals().map(|g| fx.create_buffer("storm cylinder vertex globals", g.float4s * 16));
        let wall_binds = wall_gpu.bind_variant_globals(fx, cylinder_prim, None, cylinder_vs);
        let cloud_mat = Material::load(set, STORM_MATERIAL)?;
        let cloud_gpu = build_material_ext(set, fx, &mut cache, &cloud_mat, Vf::Local, BASEPASS_VS, BASEPASS_PS, quad.num_uv, false, shared, &named)?;
        let quad_vs: Vec<Option<FxBufId>> = quad_prims.iter().map(|_| cloud_gpu.vs_globals().map(|g| fx.create_buffer("storm quad vertex globals", g.float4s * 16))).collect();
        let cloud_binds: Vec<Vec<(u32, FxBindId)>> = quad_prims.iter().zip(&quad_vs).map(|(p, v)| cloud_gpu.bind_variant_globals(fx, *p, None, *v)).collect();
        let mut notes = vec![];
        notes.push(format!("wall {} ({}, translucent {}, separate translucency {}), {} quads {} ({}, masked)", SPHERE_MATERIAL.rsplit('.').next().unwrap(), wall_mat.flags.blend, wall_gpu.translucent, wall_mat.flags.separate_translucency, quads.len(), STORM_MATERIAL.rsplit('.').next().unwrap(), cloud_mat.flags.blend));
        for (n, m) in [("wall", &wall_gpu), ("clouds", &cloud_gpu)] { for g in &m.globals { notes.push(format!("{n}: {} globals cb{} ({} float4s) from the cooked parameters: per-frame scalars {:?} vectors {:?}, base-pass VS {:?}", if g.vertex { "vertex" } else { "pixel" }, g.slot, g.float4s, g.params.per_frame_scalars.iter().filter(|p| p.is_bound()).map(|p| p.float_index()).collect::<Vec<_>>(), g.params.per_frame_vectors.iter().filter(|p| p.is_bound()).map(|p| p.float_index()).collect::<Vec<_>>(), g.params.base_pass_vs(&g.shader_type).map(|v| (v.previous_local_to_world.float_index(), v.skip_output_velocity.float_index())))); } }
        notes.push(format!("cooked bounds (Primitive buffer): cylinder {:?}, quad {:?}", cylinder.bounds, quad.bounds));
        let n = quads.len();
        Ok(StormWorld { view_buf, light_buf, collection_buf, cylinder_prim, quad_prims, cylinder_vs, quad_vs, prev_cylinder: None, prev_quads: vec![None; n], camera_cut: false, wall: (wall_mat, wall_gpu, wall_binds), cloud: (cloud_mat, cloud_gpu, cloud_binds), cylinder, quad, quads, notes })
    }

    /// The FX draws of one rendered frame: wall (translucent pass; separate translucency when its material says so) + cloud quads (opaque pass). `params` is in the
    /// FX space (see `fn_render::storm::StormParams`); `offset_cm` shifts the Unreal level coordinates of the cloud actor into that same space; `t` = the material
    /// times (world time / real time / world delta, UE4.16 ShaderBaseClasses.cpp:287).
    /// The wall is drawn whatever `MasterOpacity` is: neither `M_SafeZoneSphere` nor `M_SafeZoneStorm`'s wall program reads that `MPC_SafeZoneClouds` member, so in UE
    /// the wall never fades (the old early-out at opacity 0 and its "the material fades" note were wrong). `MasterOpacity` still goes into the collection buffer.
    /// CAMERA CUT (2026-10-05): the next frame is a camera cut (`PlayerCameraManager::bGameCameraCutThisFrame` -> `FSceneView::bCameraCut`, LocalPlayer.cpp:690 /
    /// SceneView.cpp:601). On that frame `FVelocityDrawingPolicy::HasVelocityOnBasePass` returns false (VelocityRendering.cpp:436-439), so `SetMesh` writes an
    /// identity `PreviousLocalToWorld` and `SkipOutputVelocity` 1 (BasePassRendering.inl:44-59). The port has no camera-cut source yet: call this when one exists.
    pub fn camera_cut(&mut self) { self.camera_cut = true; }

    pub fn frame(&mut self, view: &ViewParams, params: &StormParams, offset_cm: [f32; 3], t: MaterialTime) -> FxFrame {
        let mut f = FxFrame::default();
        let cut = std::mem::take(&mut self.camera_cut);
        let time = t.world;
        f.buffer_writes.push((self.view_buf, bytes(&view.view_buffer())));
        f.buffer_writes.push((self.light_buf, bytes(&fn_render::sky_light::fx_light_block()))); // SKY LIGHT: sun + View.SkyIrradianceEnvironmentMap
        f.buffer_writes.push((self.collection_buf, bytes(&params.collection_buffer())));
        f.buffer_writes.push((self.wall.1.material_buf, bytes(&self.wall.0.packed(time).cb)));
        // per-frame expressions of both stages at the offsets the programs serialized (the wall's time panners: scanlines, grid)
        f.buffer_writes.extend(self.wall.1.per_frame_writes(&self.wall.0, t));
        f.buffer_writes.extend(self.cloud.1.per_frame_writes(&self.cloud.0, t));
        f.buffer_writes.push((self.cloud.1.material_buf, bytes(&self.cloud.0.packed(time).cb)));
        let flags = PrimitiveFlags::default();
        let actor = [STORM_ACTOR_CM[0] + offset_cm[0], STORM_ACTOR_CM[1] + offset_cm[1], STORM_ACTOR_CM[2] + offset_cm[2]];
        // clouds (opaque pass, depth written), then the wall (translucent pass, depth read-only, sorted by bounds distance)
        for (i, q) in self.quads.iter().enumerate() {
            let tr = Vec3::new(actor[0] + q.rel_cm[0], actor[1] + q.rel_cm[1], actor[2] + q.rel_cm[2]);
            let m = Mat4::from_translation(tr) * Mat4::from_scale(Vec3::from(q.scale));
            // shared UE4.16 Primitive builder from the cooked mesh bounds; ActorWorldPosition = BP_SafeZoneStorm_1322's root
            f.buffer_writes.push((self.quad_prims[i], bytes(&mesh_primitive(&m, &self.quad.bounds, actor, &flags))));
            // base-pass VS globals: PreviousLocalToWorld = last frame's transform (the component is movable: HasVelocityOnBasePass, VelocityRendering.cpp:458); on the
            // first frame there is no motion-blur info yet, so SetMesh writes the CURRENT LocalToWorld with SkipOutputVelocity 0 (BasePassRendering.inl:44-48)
            // CAMERA CUT: no velocity on the base pass (identity, SkipOutputVelocity 1)
            let prev = if cut { None } else { Some(self.prev_quads[i].unwrap_or(m)) };
            if let (Some(b), Some(bytes_)) = (self.quad_vs[i], self.cloud.1.vs_globals_bytes(&self.cloud.0, t, prev)) { f.buffer_writes.push((b, bytes_)); }
            self.prev_quads[i] = Some(m);
            let sort = view.sort(self.quad.bounds.transform_by(&m).origin);
            for &(slot, first, count) in &self.quad.sections { if slot == 0 { f.push(FxDraw { pipe: self.cloud.1.pipe, binds: self.cloud.2[i].clone(), geom: FxGeom::MeshRange { mesh: self.quad.id, first_index: first, index_count: count }, instances: None, sort }); } }
        }
        let cm = params.cylinder_local_to_world();
        // ActorWorldPosition = the indicator actor location = the zone centre (AFortSafeZoneIndicator::Tick)
        f.buffer_writes.push((self.cylinder_prim, bytes(&mesh_primitive(&cm, &self.cylinder.bounds, params.center_cm, &flags))));
        // same SetMesh rule as the clouds (first frame: the current transform, velocity on)
        let prev = if cut { None } else { Some(self.prev_cylinder.unwrap_or(cm)) }; // CAMERA CUT: as the clouds
        if let (Some(b), Some(bytes_)) = (self.cylinder_vs, self.wall.1.vs_globals_bytes(&self.wall.0, t, prev)) { f.buffer_writes.push((b, bytes_)); }
        self.prev_cylinder = Some(cm);
        // SortByDistance key: |Bounds.Origin - ViewOrigin| (TranslucentRendering.cpp:864), TranslucencySortPriority 0 (no SafeZoneIndicator component sets it)
        let sort = view.sort(self.cylinder.bounds.transform_by(&cm).origin);
        for &(slot, first, count) in &self.cylinder.sections { if slot == 0 { f.push(FxDraw { pipe: self.wall.1.pipe, binds: self.wall.2.clone(), geom: FxGeom::MeshRange { mesh: self.cylinder.id, first_index: first, index_count: count }, instances: None, sort }); } }
        f
    }
}

// (the position-box bounding sphere helper was removed: the Primitive buffer now uses the cooked mesh bounds, `MeshGpu::bounds`)

#[cfg(test)]
mod tests {
    use super::*;
    fn set() -> PakSet { PakSet::open_install(fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install")).unwrap() }
    #[test]
    fn the_blueprint_has_28_storm_quads_on_two_grids() {
        let q = load_quads(&set()).unwrap();
        assert_eq!(q.len(), 28);
        assert_eq!(q.iter().filter(|x| x.scale[0] == 2.0 && x.rel_cm[2] == 25000.0).count(), 16, "inner 4 x 4 grid: scale 2 at 250 m");
        assert_eq!(q.iter().filter(|x| x.scale[0] == 4.0 && x.rel_cm[2] == 26000.0).count(), 12, "outer ring: scale 4 at 260 m");
        assert!(q.iter().all(|x| x.scale[0] == x.scale[1] && x.scale[1] == x.scale[2]));
    }
    #[test]
    fn both_materials_resolve_to_the_expected_permutations() {
        let set = set();
        for (path, blend, shading) in [(SPHERE_MATERIAL, "BLEND_Translucent", "MSM_Unlit"), (STORM_MATERIAL, "BLEND_Masked", "MSM_DefaultLit")] {
            let m = Material::load(&set, path).unwrap();
            assert_eq!((m.flags.blend.as_str(), m.flags.shading.as_str(), m.flags.two_sided), (blend, shading, true), "{path}");
            for ty in [BASEPASS_VS, BASEPASS_PS] { assert!(m.cooked.find("FLocalVertexFactory", ty).is_some(), "{path}: {ty}"); }
        }
    }
}
