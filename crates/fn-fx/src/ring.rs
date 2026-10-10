//! The weak-point ring: `WeakSpot_C` + `P_WeakPointEffect_01` from the original data, drawn through the FX pass with the original `M_Weakpoint_*` programs.
//!
//! `WeakSpot_C` (decoded from its blueprint, see [`WeakSpotBp`]): its `AnimatedBackgroundRingsParticleEffect` component is activated when the spot spawns; the first spot of a
//! prop (`CrackHitCount == 0`) scales the component by 1.5 (`SetWorldScale3D(1.5)`), later spots disable the `StarterRings` emitter instead. A hit (`OnHitCrack`) stops the
//! emitters (`SetActive(false)`) and starts `ScaleDownParticleTL` (length 0.35 s, the `ScaleDown` curve drives the `SizeScale` instance parameter); the actor is destroyed
//! when the timeline finishes. When a hit spot is replaced by a new one within `ConnectTheDotsTimeoutFromPreviousHit` and 30..500 cm away, the new spot draws the
//! `Sm_Weakpoint_Connector_01` mesh from the old location (see [`Connector`]).
use crate::data::PackageJson;
use crate::gpu::{build_material, load_mesh, MaterialGpu, MeshGpu, SharedBuffers, TextureCache, Vf, INSTANCE_STRIDE, SPRITE_VERTEX_STRIDE};
use crate::material::Material;
use crate::particles::{mesh_transform, EmitterInstance, InstanceParams, ParticleSystem};
use crate::view::{bytes, primitive_buffer, to_ue, ViewParams};
use fn_pak::PakSet;
use fn_render::fx::*;
use glam::{DVec3, Vec3};
use std::collections::HashMap;

pub const SYSTEM: &str = "/Game/Effects/Fort_Effects/BluePrints/WeakPointEffect/P_WeakPointEffect_01";
pub const WEAKSPOT_BP: &str = "/Game/Effects/Fort_Effects/BluePrints/WeakPointEffect/WeakSpot";
/// The base-pass permutation of an unlit translucent material on a mesh-particle vertex factory (the shader entries of the material package named it).
pub const BASEPASS_VS: &str = "TBasePassVSFNoLightMapPolicy";
pub const BASEPASS_PS: &str = "TBasePassPSFNoLightMapPolicy";

/// One key of a `RichCurve` (the `ScaleDown` track of `ScaleDownParticleTL`).
#[derive(Clone, Copy, Debug)]
pub struct CurveKey { pub time: f32, pub value: f32, pub arrive: f32, pub leave: f32, pub interp: u8 }
#[derive(Clone, Debug, Default)]
pub struct Curve { pub keys: Vec<CurveKey> }
impl Curve {
    /// `FRichCurve::Eval` with constant extrapolation; interpolation modes 0 linear, 1 constant, 2 cubic (Hermite with the stored tangents).
    pub fn eval(&self, t: f32) -> f32 {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else { return 0. };
        if t <= first.time { return first.value; }
        if t >= last.time { return last.value; }
        let i = self.keys.windows(2).position(|w| t < w[1].time).unwrap_or(0);
        let (a, b) = (self.keys[i], self.keys[i + 1]);
        let dt = b.time - a.time; let alpha = (t - a.time) / dt;
        match a.interp {
            1 => a.value,
            2 => { let (p0, m0, p1, m1) = (a.value, a.leave * dt, b.value, b.arrive * dt); let (a2, a3) = (alpha * alpha, alpha * alpha * alpha); (2. * a3 - 3. * a2 + 1.) * p0 + (a3 - 2. * a2 + alpha) * m0 + (a3 - a2) * m1 + (-2. * a3 + 3. * a2) * p1 }
            _ => a.value + (b.value - a.value) * alpha,
        }
    }
}

/// The data of `WeakSpot_C` the effect uses.
#[derive(Clone, Debug)]
pub struct WeakSpotBp {
    pub timeline_length: f32,
    pub scale_down: Curve,
    /// `SetWorldScale3D` of the ring component on the first spot of a prop (the constant in the blueprint's bytecode).
    pub first_spot_scale: f32,
    pub connect_min_length: f32, pub connect_max_length: f32, pub connect_timeout: f32, pub connect_width: f32, pub connect_tile_scale: f32,
    pub connect_duration_scale: f32, pub connect_max_duration: f32,
    /// `FClamp(.., 0.05, ConnectTheDotsMaxDuration)` lower bound (bytecode constant).
    pub connect_min_duration: f32,
}
impl WeakSpotBp {
    pub fn load(set: &PakSet) -> Result<WeakSpotBp, String> {
        let pkg = PackageJson::load(set, WEAKSPOT_BP)?;
        let tl = pkg.exports.iter().find(|e| e.class == "TimelineTemplate" && e.name == "ScaleDownParticleTL_Template").ok_or("ScaleDownParticleTL missing")?;
        let timeline_length = tl.props.get("TimelineLength").and_then(|v| v.as_f64()).ok_or("timeline length")? as f32;
        let track = tl.props.get("FloatTracks").and_then(|t| t.as_array()).and_then(|a| a.first()).ok_or("no float track")?;
        let curve_path = track.get("CurveFloat").and_then(|c| c.as_str()).ok_or("track curve")?;
        let curve_ex = pkg.get(curve_path).ok_or("curve export")?;
        let keys = curve_ex.props.get("FloatCurve").and_then(|c| c.get("Keys")).and_then(|k| k.as_array()).ok_or("curve keys")?;
        let interp = |s: Option<&serde_json::Value>| s.map(|v| if let Some(n) = v.as_u64() { n as u8 } else { match v.as_str().unwrap_or("") { "RCIM_Constant" => 1, "RCIM_Cubic" => 2, _ => 0 } }).unwrap_or(0);
        let scale_down = Curve { keys: keys.iter().map(|k| CurveKey { time: k["Time"].as_f64().unwrap_or(0.) as f32, value: k["Value"].as_f64().unwrap_or(0.) as f32, arrive: k["ArriveTangent"].as_f64().unwrap_or(0.) as f32, leave: k["LeaveTangent"].as_f64().unwrap_or(0.) as f32, interp: interp(k.get("InterpMode")) }).collect() };
        let cdo = pkg.exports.iter().find(|e| e.name == "Default__WeakSpot_C").ok_or("CDO")?;
        let fp = |k: &str| cdo.props.get(k).and_then(|v| v.as_f64()).map(|v| v as f32).ok_or_else(|| format!("CDO.{k}"));
        let (first_spot_scale, connect_min_duration) = bytecode_constants(set)?;
        Ok(WeakSpotBp {
            timeline_length, scale_down, first_spot_scale, connect_min_duration,
            connect_min_length: fp("ConnectTheDotsMinLength")?, connect_max_length: fp("ConnectTheDotsMaxLength")?, connect_timeout: fp("ConnectTheDotsTimeoutFromPreviousHit")?, connect_width: fp("ConnectTheDotsWidth")?,
            connect_tile_scale: fp("ConnectTheDotsTileScale")?, connect_duration_scale: fp("ConnectTheDotsDurationScale")?, connect_max_duration: fp("ConnectTheDotsMaxDuration")?,
        })
    }
    /// Seconds a connector mesh lives (`ConnectTheDotsDuration` = clamp(scale * length, 0.05, max)) for a connection of `length_cm`.
    pub fn connect_duration(&self, length_cm: f32) -> f32 { (self.connect_duration_scale * length_cm).clamp(self.connect_min_duration, self.connect_max_duration) }
}

/// The constants of the `ExecuteUbergraph_WeakSpot` bytecode the blueprint settings do not carry: the `SetWorldScale3D` vector and the lower bound of the connect duration
/// `FClamp`.
fn bytecode_constants(set: &PakSet) -> Result<(f32, f32), String> {
    use fn_uasset::kismet::{disassemble, Expr};
    let data = fn_uasset::PackageData::load(set, WEAKSPOT_BP).map_err(|e| e.to_string())?;
    let pkg = data.package().map_err(|e| e.to_string())?;
    let (mut scale, mut lo) = (None, None);
    fn walk(e: &Expr, scale: &mut Option<f32>, lo: &mut Option<f32>) {
        let text = e.render();
        if text.contains("SetWorldScale3D") { if let Expr::VectorConst(v) = find_vector(e) { *scale = Some(v[0]); } }
        if text.contains("FClamp(L:CallFunc_Multiply_FloatFloat_ReturnValue2") { if let Some(x) = find_float(e) { *lo = Some(x); } }
        for c in e.children() { walk(c, scale, lo); }
    }
    fn find_vector(e: &Expr) -> Expr { if let Expr::VectorConst(_) = e { return e.clone(); } for c in e.children() { if let Expr::VectorConst(v) = find_vector(c) { return Expr::VectorConst(v); } } Expr::Nothing }
    fn find_float(e: &Expr) -> Option<f32> { if let Expr::FloatConst(x) = e { return Some(*x); } e.children().into_iter().find_map(find_float) }
    for i in 0..pkg.exports.len() {
        if pkg.export_class_name(i) != "Function" { continue; }
        let Ok(f) = fn_uasset::classes::read_function(&pkg, i) else { continue };
        for s in disassemble(&pkg, f.script).statements { walk(&s.expr, &mut scale, &mut lo); }
    }
    Ok((scale.ok_or("SetWorldScale3D constant not found")?, lo.ok_or("FClamp lower bound not found")?))
}

/// Meshes + materials of the effect, uploaded once.
pub struct RingAssets {
    pub system: ParticleSystem,
    pub bp: WeakSpotBp,
    pub meshes: HashMap<String, MeshGpu>,
    pub materials: HashMap<String, MaterialGpu>,
    pub notes: Vec<String>,
}
impl RingAssets {
    pub fn load(set: &PakSet, fx: &mut FxRenderer, cache: &mut TextureCache, shared: SharedBuffers) -> Result<RingAssets, String> {
        let system = ParticleSystem::load(set, SYSTEM)?;
        let bp = WeakSpotBp::load(set)?;
        let (mut meshes, mut materials, mut notes) = (HashMap::new(), HashMap::new(), vec![]);
        for e in &system.emitters {
            for lod in e.lods.iter().filter(|l| l.level == 0) {
                let Some(mt) = &lod.mesh else {
                    // sprite emitter (FParticleSpriteVertexFactory)
                    if !materials.contains_key(&lod.required.material) {
                        let mat = Material::load(set, &lod.required.material)?;
                        let g = build_material(set, fx, cache, &mat, Vf::Sprite, BASEPASS_VS, BASEPASS_PS, 0, false, shared)?;
                        materials.insert(lod.required.material.clone(), g);
                    }
                    continue;
                };
                if !meshes.contains_key(&mt.mesh) { meshes.insert(mt.mesh.clone(), load_mesh(set, fx, Vf::MeshParticle, &mt.mesh)?); }
                if !materials.contains_key(&lod.required.material) {
                    let mat = Material::load(set, &lod.required.material)?;
                    let nuv = meshes[&mt.mesh].num_uv;
                    let g = build_material(set, fx, cache, &mat, Vf::MeshParticle, BASEPASS_VS, BASEPASS_PS, nuv, true, shared)?;
                    materials.insert(lod.required.material.clone(), g);
                }
            }
        }
        Ok(RingAssets { system, bp, meshes, materials, notes })
    }
}

/// One live `WeakSpot_C`.
pub struct RingEffect {
    pub object: u64,
    pub pos_ue: DVec3,
    pub scale: f32,
    pub age: f32,
    pub emitters: Vec<EmitterInstance>,
    pub params: InstanceParams,
    /// Age at which `OnHitCrack` ran (`SetActive(false)` + `ScaleDownParticleTL.PlayFromStart()`).
    pub hit_at: Option<f32>,
    /// Primitive uniform buffer / bind groups of this effect (a pool slot).
    pub slot: usize,
    pub finished: bool,
}
impl RingEffect {
    /// `first` = `CrackHitCount == 0`.
    pub fn spawn(assets: &RingAssets, object: u64, pos_m: Vec3, first: bool, slot: usize, seed: u64) -> RingEffect {
        let mut emitters: Vec<EmitterInstance> = (0..assets.system.emitters.len()).map(|i| EmitterInstance::new(i, seed)).collect();
        if !first { if let Some(s) = assets.system.emitters.iter().position(|e| e.name == "StarterRings") { emitters[s].enabled = false; } }
        RingEffect { object, pos_ue: to_ue(pos_m), scale: if first { assets.bp.first_spot_scale } else { 1.0 }, age: 0., emitters, params: InstanceParams::default(), hit_at: None, slot, finished: false }
    }
    pub fn hit(&mut self) { if self.hit_at.is_none() { self.hit_at = Some(self.age); for e in &mut self.emitters { e.halted = true; } } }
    pub fn update(&mut self, assets: &RingAssets, dt: f32, camera_distance_cm: f32) {
        let lod = assets.system.lod_index(camera_distance_cm);
        self.age += dt;
        if let Some(h) = self.hit_at {
            let t = (self.age - h).min(assets.bp.timeline_length);
            let v = assets.bp.scale_down.eval(t);
            self.params.set_vector("SizeScale", [v, v, v]);
            if self.age - h >= assets.bp.timeline_length { self.finished = true; }
        }
        for e in &mut self.emitters { e.lod = lod; e.update(&assets.system, dt, &self.params); }
    }
}

/// Draws of one effect for this frame (emitter order: the engine submits the emitters' mesh batches in emitter index order), plus the primitive buffer contents.
pub fn draws(assets: &RingAssets, effect: &RingEffect, view: &ViewParams, binds: &HashMap<(String, usize), Vec<(u32, FxBindId)>>, primitive_bufs: &[FxBufId], frame: &mut FxFrame) {
    let eye = to_ue(view.eye);
    let fwd = crate::view::dir_to_ue(view.forward).normalize();
    let live: Vec<_> = effect.emitters.iter().filter_map(|e| {
        let lod = assets.system.emitters[e.emitter].lods.iter().find(|l| l.level == e.lod as i32 && l.enabled)?;
        let views = e.views(&assets.system, &effect.params);
        (!views.is_empty()).then_some((lod, views))
    }).collect();
    // the component's bounds (UParticleSystemComponent::CalcBounds, ParticleComponents.cpp:4252: P_WeakPointEffect_01 has bUseFixedRelativeBoundingBox, +-40 cm):
    // the Primitive buffer's bounds and the translucency sort key (|Bounds.Origin - ViewOrigin|, TranslucentRendering.cpp:864) of every emitter's draw
    let at = [effect.pos_ue.x as f32, effect.pos_ue.y as f32, effect.pos_ue.z as f32];
    let l2w = glam::Mat4::from_translation(Vec3::from(at)) * glam::Mat4::from_scale(Vec3::splat(effect.scale));
    let boxes: Vec<_> = live.iter().filter_map(|(lod, views)| {
        let ext = lod.mesh.as_ref().and_then(|mt| assets.meshes.get(&mt.mesh)).map(|m| m.bounds.extent);
        crate::view::emitter_box_at(at, &views.iter().map(|v| v.size).collect::<Vec<_>>(), effect.scale, [-0.5, -0.5], ext)
    }).collect();
    let bounds = crate::view::particle_bounds(assets.system.fixed_bounds, &l2w, &boxes);
    let sort = view.sort(bounds.origin);
    frame.buffer_writes.push((primitive_bufs[effect.slot], bytes(&primitive_buffer(effect.pos_ue, effect.scale, &bounds))));
    for (lod, views) in live {
        let (Some(mat), Some(bg)) = (assets.materials.get(&lod.required.material), binds.get(&(lod.required.material.clone(), effect.slot))) else { continue };
        match &lod.mesh {
            Some(mt) => {
                let Some(mesh) = assets.meshes.get(&mt.mesh) else { continue };
                let mut b = Vec::with_capacity(views.len() * INSTANCE_STRIDE as usize);
                for v in &views {
                    // component scale (bUseLocalSpace): the particle's local transform is multiplied by the component's LocalToWorld
                    let size = [v.size[0] * effect.scale, v.size[1] * effect.scale, v.size[2] * effect.scale];
                    let r = mesh_transform(mt, size, v.rotation, effect.pos_ue, eye, fwd);
                    let rows = [[r.x_axis.x, r.y_axis.x, r.z_axis.x, effect.pos_ue.x], [r.x_axis.y, r.y_axis.y, r.z_axis.y, effect.pos_ue.y], [r.x_axis.z, r.y_axis.z, r.z_axis.z, effect.pos_ue.z]];
                    for row in rows { for c in row { b.extend((c as f32).to_le_bytes()); } }
                    for _ in 0..4 { b.extend(0i32.to_le_bytes()); }                                   // SubUVParams
                    for x in [0f32, v.rel_time, 0., 0.] { b.extend(x.to_le_bytes()); }                // SubUVLerpAndRelTime
                    for x in v.dynamic { b.extend(x.to_le_bytes()); }                                  // DynamicParameter
                    for x in v.color { b.extend(x.to_le_bytes()); }                                    // particle colour
                    for _ in 0..4 { b.extend(0f32.to_le_bytes()); }                                   // Velocity
                }
                frame.push(FxDraw { pipe: mat.pipe, binds: bg.clone(), geom: FxGeom::Mesh(mesh.id), instances: Some(FxInstances { count: views.len() as u32, bytes: b }), sort });
            }
            None => {
                // camera facing sprites: 4 vertices per particle (Position+RelativeTime, OldPosition+ParticleId, Size+Rotation+SubImageIndex, Color, corner TexCoord)
                let (mut vb, mut ib) = (Vec::with_capacity(views.len() * 4 * SPRITE_VERTEX_STRIDE as usize), Vec::with_capacity(views.len() * 6));
                for (n, v) in views.iter().enumerate() {
                    let size = [v.size[0] * effect.scale, v.size[1] * effect.scale];
                    for uv in [[0f32, 0.], [0., 1.], [1., 1.], [1., 0.]] {
                        for x in [0f32, 0., 0., v.rel_time, 0., 0., 0., n as f32, size[0], size[1], v.rotation, 0.] { vb.extend(x.to_le_bytes()); }
                        for x in v.color { vb.extend(x.to_le_bytes()); }
                        for x in uv { vb.extend(x.to_le_bytes()); }
                    }
                    let base = (n * 4) as u32;
                    ib.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                }
                frame.push(FxDraw { pipe: mat.pipe, binds: bg.clone(), geom: FxGeom::Dynamic { vertices: vb, indices: ib }, instances: None, sort });
            }
        }
    }
}


