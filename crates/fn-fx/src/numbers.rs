//! The in-world damage numbers: `ImpactNumbers_C` (blueprint data + bytecode: `init a new number set`, `Set Material Pararmeters`) on the original `standard_DamageNumbers8` /
//! `newFontQuads` meshes with their original materials (`FlyingTextMat_Inst` -> `DamageNumbers_Mat`, `DamageNumbers_CriticalHitBase` / `Glow` / `TransparentMaterial`), the
//! per-digit `0a..8a` / `0b..8b` / `0c..8c` vector parameters the blueprint writes into the dynamic material instances, and the
//! `DamageNumberWPO` function running inside the recovered vertex shaders.
//!
//! Native values (`AFortDamageNumbersActor`, read from the exe by `fn-exe-defaults`' `classdump`): `ComponentLifespan` = 1.0 s ([`COMPONENT_LIFESPAN`]), the colour / font defaults equal the
//! blueprint's CDO. NOT recoverable from the data: the contents of the native `DamageNumberArray` for a Building number (the blueprint skips slot 0 for every type but Player; the
//! array is taken as `[0, d1, d2, ..]`, slot 0 = 0 = no sign glyph).
use crate::data::PackageJson;
use crate::gpu::{build_material, load_mesh, MaterialGpu, MeshGpu, SharedBuffers, TextureCache, Vf};
use crate::material::Material;
use crate::uniform::Overrides;
use crate::view::{bytes, primitive_buffer_local, ViewParams};
use fn_pak::PakSet;
use fn_render::fx::*;
use glam::{DVec3, Vec3};
use std::collections::HashMap;

pub const BP: &str = "/Game/UI/DamageText/Base/ImpactNumbers";
/// `AFortDamageNumbersActor::ComponentLifespan`, the constructor's store at +0x390 (exe, `classdump AFortDamageNumbersActor:ComponentLifespan`): 1.0 second.
pub const COMPONENT_LIFESPAN: f32 = 1.0;
/// `EFortDamageNumberType::Building` / `Player` as the blueprint bytecode compares them.
pub const TYPE_BUILDING: u8 = 2;
pub const TYPE_PLAYER: u8 = 3;
/// Concurrent numbers (pool slots); a new number past this reuses the oldest.
pub const SLOTS: usize = 8;
pub const BASEPASS_VS: &str = "TBasePassVSFNoLightMapPolicy";
pub const BASEPASS_PS: &str = "TBasePassPSFNoLightMapPolicy";

/// The `ImpactNumbers_C` class defaults used by the blueprint functions.
#[derive(Clone, Debug)]
pub struct NumberBp {
    pub standard_color: [f32; 4], pub crit_wall_color: [f32; 4], pub crit_enemy_color: [f32; 4], pub player_color: [f32; 4],
    pub crit_multiplier: f32, pub doubling_distance: f32, pub font_scale_max: f32, pub spacing_multiplier: f32, pub spacing_for_ones: f32,
    pub font_x: f32, pub font_y: f32, pub rotations: f32,
    pub standard_mesh: String, pub crit_building_mesh: String,
}
impl NumberBp {
    pub fn load(set: &PakSet) -> Result<NumberBp, String> {
        let pkg = PackageJson::load(set, BP)?;
        let cdo = pkg.exports.iter().find(|e| e.name == "Default__ImpactNumbers_C").ok_or("CDO")?;
        let f = |k: &str| cdo.props.get(k).and_then(|v| v.as_f64()).map(|v| v as f32).ok_or_else(|| format!("ImpactNumbers CDO.{k}"));
        let c = |k: &str| -> Result<[f32; 4], String> { let a = cdo.props.get(k).and_then(|v| v.as_array()).ok_or_else(|| format!("ImpactNumbers CDO.{k}"))?; Ok([a[0].as_f64().unwrap_or(0.) as f32, a[1].as_f64().unwrap_or(0.) as f32, a[2].as_f64().unwrap_or(0.) as f32, a[3].as_f64().unwrap_or(0.) as f32]) };
        let s = |k: &str| cdo.props.get(k).and_then(|v| v.as_str()).map(|s| s.to_owned()).ok_or_else(|| format!("ImpactNumbers CDO.{k}"));
        Ok(NumberBp {
            standard_color: c("Standard Hit Color")?, crit_wall_color: c("Critical Wall Hit Color")?, crit_enemy_color: c("Critical Enemy Hit Color")?, player_color: c("Player Damage Color")?,
            crit_multiplier: f("Critical Hit Multiplier")?, doubling_distance: f("Distance from Camera Before Doubling Size")?, font_scale_max: f("FontSizeScaleMaxPerc")?, spacing_multiplier: f("Number Spacing Multiplier")?, spacing_for_ones: f("Spacing percentage for 1s")?,
            font_x: f("FontXSize_0")?, font_y: f("FontYSize_0")?, rotations: f("Number of number rotations")?,
            standard_mesh: s("MeshesByElement")?, crit_building_mesh: s("CritBuildingImpactMesh")?,
        })
    }
}

/// The player camera at the moment the number was created (UE cm, axes forward / right / up).
#[derive(Clone, Copy, Debug)]
pub struct CamUe { pub loc: DVec3, pub fwd: DVec3, pub right: DVec3, pub up: DVec3 }
impl CamUe {
    pub fn from_view(v: &ViewParams) -> CamUe { let c = v.clip(); CamUe { loc: c.eye, fwd: c.fwd, right: c.right, up: c.up } }
    /// `TransformDirection(CameraTransform, (x, y, z))` with the rotator's axes (X forward, Y right, Z up).
    pub fn direction(&self, v: [f32; 3]) -> DVec3 { self.fwd * v[0] as f64 + self.right * v[1] as f64 + self.up * v[2] as f64 }
}

fn next(r: &mut u64) -> f32 { *r = r.wrapping_add(0x9E37_79B9_7F4A_7C15); let mut z = *r; z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9); z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB); ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32 }

/// What `Set Material Pararmeters` writes into every dynamic material instance of the number's mesh.
#[derive(Clone, Debug, Default)]
pub struct MidParams { pub vectors: Vec<(String, [f32; 4])>, pub scalars: Vec<(String, f32)> }
impl MidParams {
    pub fn apply(&self, base: &Overrides) -> Overrides {
        let mut o = base.clone();
        for (n, v) in &self.vectors { o.vectors.insert(n.clone(), *v); }
        for (n, v) in &self.scalars { o.scalars.insert(n.clone(), *v); }
        o
    }
    pub fn vector(&self, name: &str) -> Option<[f32; 4]> { self.vectors.iter().find(|(n, _)| n == name).map(|(_, v)| *v) }
}

/// `init a new number set`: the random camera-space offset and the number's location. Returns the component location (UE cm).
pub fn number_location(world_location: DVec3, cam: &CamUe, kind: u8, rng: &mut u64) -> DVec3 {
    // (X, Y) = (0, 0) for every type but Score (5); the random value (-5..5 cm) is multiplied by 1 for types 0..4 (5 for Score, 0 for DBNO) and added to BOTH axes
    let m = match kind { 5 => 5.0, 6 => 0.0, _ => 1.0 };
    let r = (next(rng) * 10.0 - 5.0) * m;
    world_location + cam.right * r as f64 + cam.up * r as f64
}

/// `Set Material Pararmeters` (blueprint bytecode): `digits` = the native `DamageNumberArray`.
pub fn mid_params(bp: &NumberBp, kind: u8, critical: bool, digits: &[u8], cam: &CamUe, number_location: DVec3, now: f32, rng: &mut u64) -> MidParams {
    let mut p = MidParams::default();
    p.scalars.push(("+Or-".into(), if kind == TYPE_PLAYER { 0.5 } else { 0.0 }));
    let wall_or_enemy = if critical && kind == TYPE_BUILDING { bp.crit_wall_color } else { bp.crit_enemy_color };
    let c2 = if critical { wall_or_enemy } else { bp.standard_color };
    let color = if kind == TYPE_PLAYER { bp.player_color } else { c2 };
    p.vectors.push(("Color".into(), color));
    p.scalars.push(("Animation Lifespan".into(), COMPONENT_LIFESPAN));
    p.scalars.push(("isCriticalHit?".into(), if critical { 1.0 } else { 0.0 }));
    let len = digits.len() as i32;
    let last = (len - 1).max(0);
    let mut offset = if kind == TYPE_PLAYER { -(len as f32) } else { -(len as f32) - 1.0 };
    let dist = (cam.loc - number_location).length() as f32;
    let dist_scale = (dist / bp.doubling_distance).clamp(1.0, 1.0e9);
    let size = (1.0 + (bp.font_scale_max - 1.0) * 0.0) * if critical { bp.crit_multiplier } else { 1.0 };
    let (sx, sy) = (bp.font_x * size * dist_scale, bp.font_y * size * dist_scale);
    let slots = if last > 3 { last + 1 } else { 4 };
    let digit = |i: i32| digits.get(i.clamp(0, last) as usize).copied().unwrap_or(0);
    for k in 0..=slots.min(8) {
        offset += if digit(k) == 1 || digit(k - 1) == 1 { bp.spacing_for_ones } else { 1.0 };
        let y = ((k as f32 / last as f32) - 0.5) * 2.0;
        let dir = cam.direction([0.0, y, 0.0]);
        p.vectors.push((format!("{k}a"), [dir.x as f32, dir.y as f32, dir.z as f32, offset * bp.spacing_multiplier]));
        let active = ((k == 0 && kind == TYPE_PLAYER) || k != 0) && k <= last;
        let sign = if dir.x > 0.0 { 1.0 } else if dir.x < 0.0 { -1.0 } else { 0.0 };
        p.vectors.push((format!("{k}b"), [if active { sx } else { 0.0 }, if active { sy } else { 0.0 }, digit(k) as f32, sign * bp.rotations]));
        p.vectors.push((format!("{k}c"), [now + COMPONENT_LIFESPAN, next(rng), 0.0, 0.0]));
    }
    p
}

/// One mesh section with the material it is drawn with.
pub struct SectionGpu { pub first_index: u32, pub index_count: u32, pub material: Material, pub gpu: MaterialGpu, pub path: String, pub slot_binds: Vec<Vec<(u32, FxBindId)>>, pub slot_material_bufs: Vec<FxBufId> }
pub struct KindGpu { pub mesh: MeshGpu, pub sections: Vec<SectionGpu> }
pub struct NumberAssets { pub bp: NumberBp, pub standard: KindGpu, pub crit: KindGpu, pub primitive_bufs: Vec<FxBufId> }

impl NumberAssets {
    pub fn load(set: &PakSet, fx: &mut FxRenderer, cache: &mut TextureCache, shared: SharedBuffers) -> Result<NumberAssets, String> {
        let bp = NumberBp::load(set)?;
        let primitive_bufs: Vec<FxBufId> = (0..SLOTS).map(|_| fx.create_buffer("number primitive", 16 * 16)).collect();
        let mut kind = |path: &str| -> Result<KindGpu, String> {
            let mesh = load_mesh(set, fx, Vf::Local, path)?;
            let mut sections = vec![];
            for (slot, first, count) in mesh.sections.clone() {
                let mpath = mesh.material_paths.get(slot).cloned().flatten().ok_or_else(|| format!("{path}: section material {slot} missing"))?;
                let material = Material::load(set, &mpath)?;
                let gpu = build_material(set, fx, cache, &material, Vf::Local, BASEPASS_VS, BASEPASS_PS, mesh.num_uv, false, shared)?;
                let regs = gpu.material_registers.max(97);
                let mut slot_binds = vec![]; let mut slot_material_bufs = vec![];
                for s in 0..SLOTS { let mb = fx.create_buffer("number material", regs * 16); slot_material_bufs.push(mb); slot_binds.push(gpu.bind_variant(fx, primitive_bufs[s], Some(mb))); }
                sections.push(SectionGpu { first_index: first, index_count: count, material, gpu, path: mpath, slot_binds, slot_material_bufs });
            }
            Ok(KindGpu { mesh, sections })
        };
        let standard = kind(&bp.standard_mesh)?;
        let crit = kind(&bp.crit_building_mesh)?;
        Ok(NumberAssets { bp, standard, crit, primitive_bufs })
    }
}

/// A live number (a pooled `StaticMeshComponent`).
#[derive(Clone, Debug)]
pub struct LiveNumber {
    pub slot: usize,
    pub spawn_time: f32,
    pub critical: bool,
    pub location: DVec3,
    pub cam: CamUe,
    pub params: MidParams,
    pub amount: u32,
}
impl LiveNumber {
    pub fn spawn(assets: &NumberAssets, slot: usize, world_location_m: Vec3, amount: u32, critical: bool, view: &ViewParams, now: f32, seed: u64) -> LiveNumber {
        let mut rng = seed;
        let cam = CamUe::from_view(view);
        let kind = TYPE_BUILDING;
        let location = number_location(crate::view::to_ue(world_location_m), &cam, kind, &mut rng);
        let mut digits: Vec<u8> = vec![0];
        digits.extend(amount.to_string().bytes().map(|b| b - b'0'));
        let params = mid_params(&assets.bp, kind, critical, &digits, &cam, location, now, &mut rng);
        LiveNumber { slot, spawn_time: now, critical, location, cam, params, amount }
    }
    pub fn alive(&self, now: f32) -> bool { now - self.spawn_time <= COMPONENT_LIFESPAN }
}

/// Adds the draws of `numbers` to `frame` (sections in mesh order) and writes their primitive / material buffers.
pub fn draws(assets: &NumberAssets, numbers: &[LiveNumber], now: f32, frame: &mut FxFrame) {
    for n in numbers.iter().filter(|n| n.alive(now)) {
        let kind = if n.critical { &assets.crit } else { &assets.standard };
        // LocalToWorld of the component: the camera rotation at spawn (X forward, Y right, Z up) and the number location
        frame.buffer_writes.push((assets.primitive_bufs[n.slot], bytes(&primitive_buffer_local(n.location, [n.cam.fwd, n.cam.right, n.cam.up]))));
        for s in &kind.sections {
            let ov = n.params.apply(&s.material.overrides);
            let packed = crate::uniform::evaluate(&s.material.uniform, &ov, now);
            frame.buffer_writes.push((s.slot_material_bufs[n.slot], bytes(&packed.cb)));
            frame.draws.push(FxDraw { pipe: s.gpu.pipe, binds: s.slot_binds[n.slot].clone(), geom: FxGeom::MeshRange { mesh: kind.mesh.id, first_index: s.first_index, index_count: s.index_count }, instances: None });
        }
    }
}

#[allow(dead_code)]
fn _unused(_: HashMap<u8, u8>) {}
