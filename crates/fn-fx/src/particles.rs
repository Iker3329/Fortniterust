//! The `ParticleSystem` of the weak-point effect (`P_WeakPointEffect_01`) decoded from the package and simulated on the CPU: the emitters' required / spawn / type-data
//! modules, lifetime, size, size-multiply-life, size scale (instance parameter `SizeScale`), colour, colour scale over life, rotation, rotation rate and the
//! `ParticleModuleParameterDynamic` values that feed the materials' `DynamicParameter` node. The mesh-particle placement (camera facing with a locked up axis) is
//! [`mesh_transform`]. Everything the data states is taken from the data; the engine's native rules this relies on are listed in the crate README / report.
use crate::data::PackageJson;
use glam::{DMat3, DVec3};
use serde_json::Value;

// ---- distributions -----------------------------------------------------------------------------------------------------------
/// `FRawDistribution` lookup table (`DistributionLookupTable`).
#[derive(Clone, Debug, Default)]
pub struct Table { pub entry_count: usize, pub entry_stride: usize, pub op: u32, pub time_scale: f32, pub time_bias: f32, pub sub_entry_stride: usize, pub values: Vec<f32> }
#[derive(Clone, Debug)]
pub enum Dist {
    /// A table (constants have one entry); `Op` 2 = random between the two sub-entries.
    Table(Table),
    /// `DistributionVectorParticleParameter`: an instance parameter by name with the table's constant as default.
    Param { name: String, default: [f32; 3] },
}
impl Default for Dist { fn default() -> Self { Dist::Table(Table::default()) } }

fn f(v: &Value) -> f32 { v.as_f64().unwrap_or(0.) as f32 }
fn table_of(v: &Value) -> Table {
    let t = v.get("Table").unwrap_or(&Value::Null);
    Table {
        entry_count: t.get("EntryCount").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        entry_stride: t.get("EntryStride").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        op: t.get("Op").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        time_scale: t.get("TimeScale").map_or(0., f), time_bias: t.get("TimeBias").map_or(0., f),
        sub_entry_stride: t.get("SubEntryStride").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        values: t.get("Values").and_then(|x| x.as_array()).map(|a| a.iter().map(f).collect()).unwrap_or_default(),
    }
}
fn dist_of(v: Option<&Value>, pkg: &PackageJson) -> Dist {
    let Some(v) = v else { return Dist::default() };
    if let Some(path) = v.get("Distribution").and_then(|d| d.as_str()) {
        if let Some(e) = pkg.get(path) {
            if e.class == "DistributionVectorParticleParameter" {
                let c = e.props.get("Constant").and_then(|c| c.as_array()).map(|a| [f(&a[0]), f(&a[1]), f(&a[2])]).unwrap_or([0.; 3]);
                return Dist::Param { name: e.props.get("ParameterName").and_then(|n| n.as_str()).unwrap_or("").to_owned(), default: c };
            }
        }
    }
    Dist::Table(table_of(v))
}

impl Table {
    /// Value at `time` (`FDistributionLookupTable::GetValue`): `(time - bias) * scale` indexes the entries with linear interpolation, clamped at both ends. `rand` in
    /// [0,1) picks between the two sub-entries of a random distribution (`Op` 2). Returns the first `dims` components.
    pub fn eval(&self, time: f32, rand: f32, dims: usize) -> [f32; 4] {
        let mut out = [0f32; 4];
        if self.entry_count == 0 || self.values.is_empty() { return out; }
        let stride = self.entry_stride.max(1);
        let t = (time - self.time_bias) * self.time_scale;
        let (i0, i1, a) = if self.entry_count == 1 || t <= 0. { (0, 0, 0.) } else if t >= (self.entry_count - 1) as f32 { (self.entry_count - 1, self.entry_count - 1, 0.) } else { let i = t.floor() as usize; (i, i + 1, t - i as f32) };
        let get = |e: usize, c: usize, sub: usize| -> f32 { self.values.get(e * stride + sub * self.sub_entry_stride.max(0) + c).copied().unwrap_or(0.) };
        for c in 0..dims.min(4) {
            let one = |sub: usize| get(i0, c, sub) * (1. - a) + get(i1, c, sub) * a;
            out[c] = if self.op == 2 && self.sub_entry_stride > 0 { let (lo, hi) = (one(0), one(1)); lo + (hi - lo) * rand } else { one(0) };
        }
        out
    }
}
impl Dist {
    pub fn eval_f(&self, time: f32, rand: f32) -> f32 { match self { Dist::Table(t) => t.eval(time, rand, 1)[0], Dist::Param { default, .. } => default[0] } }
    pub fn eval_v(&self, time: f32, rand: f32, params: &InstanceParams) -> [f32; 3] {
        match self {
            Dist::Table(t) => { let v = t.eval(time, rand, 3); [v[0], v[1], v[2]] }
            Dist::Param { name, default } => params.vector(name).unwrap_or(*default),
        }
    }
}

/// Instance parameters of the component (`SetVectorParameter`).
#[derive(Clone, Debug, Default)]
pub struct InstanceParams { pub vectors: Vec<(String, [f32; 3])> }
impl InstanceParams {
    pub fn vector(&self, name: &str) -> Option<[f32; 3]> { self.vectors.iter().find(|(n, _)| n == name).map(|(_, v)| *v) }
    pub fn set_vector(&mut self, name: &str, v: [f32; 3]) { match self.vectors.iter_mut().find(|(n, _)| n == name) { Some(e) => e.1 = v, None => self.vectors.push((name.to_owned(), v)) } }
}

// ---- modules -----------------------------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct DynParam { pub name: String, pub value: Dist, pub use_emitter_time: bool, pub spawn_time_only: bool, pub user_set: bool }
#[derive(Clone, Debug)]
pub enum Module {
    Lifetime(Dist),
    Size(Dist),
    SizeMultiplyLife(Dist),
    SizeScale(Dist),
    Color { color: Dist, alpha: Dist },
    ColorScaleOverLife { color: Dist, alpha: Dist, emitter_time: bool },
    Rotation(Dist),
    RotationRate(Dist),
    Dynamic(Vec<DynParam>),
}
#[derive(Clone, Debug)]
pub struct Burst {
    /// Normalised time of the emitter duration.
    pub time: f32,
    pub count: i32,
    pub count_low: i32,
}
#[derive(Clone, Debug)]
pub struct Required {
    pub material: String,
    pub delay: f32, pub duration: f32, pub loops: i32, pub delay_first_loop_only: bool,
    pub kill_on_deactivate: bool, pub kill_on_completed: bool, pub use_local_space: bool,
}
#[derive(Clone, Debug)]
pub struct MeshType { pub mesh: String, pub alignment: String, pub camera_facing_option: String, pub camera_facing: bool, pub face_camera_direction: bool, pub apply_rotation_as_spin: bool }
#[derive(Clone, Debug)]
pub struct Spawn { pub rate: Dist, pub rate_scale: Dist, pub bursts: Vec<Burst>, pub burst_scale: Dist }
#[derive(Clone, Debug)]
pub struct EmitterLod { pub level: i32, pub enabled: bool, pub required: Required, pub spawn: Spawn, pub mesh: Option<MeshType>, pub modules: Vec<Module> }
#[derive(Clone, Debug)]
pub struct Emitter { pub name: String, pub lods: Vec<EmitterLod> }
#[derive(Clone, Debug)]
pub struct ParticleSystem { pub emitters: Vec<Emitter>, pub lod_distances: Vec<f32>, /** `FixedRelativeBoundingBox` (min, max, cm) when `bUseFixedRelativeBoundingBox` (UE4.16 ParticleComponents.cpp:4262) */ pub fixed_bounds: Option<([f32; 3], [f32; 3])> }

fn b(v: Option<&Value>, d: bool) -> bool { v.and_then(|x| x.as_bool()).unwrap_or(d) }

impl ParticleSystem {
    pub fn load(set: &fn_pak::PakSet, long: &str) -> Result<ParticleSystem, String> {
        let pkg = PackageJson::load(set, long)?;
        let sys = pkg.exports.iter().find(|e| e.class == "ParticleSystem").ok_or("no ParticleSystem export")?;
        let lod_distances = sys.props.get("LODDistances").and_then(|a| a.as_array()).map(|a| a.iter().map(f).collect()).unwrap_or_default();
        let mut emitters = vec![];
        for ep in sys.props.get("Emitters").and_then(|e| e.as_array()).ok_or("no emitters")? {
            let em = pkg.get(ep.as_str().unwrap_or("")).ok_or_else(|| format!("emitter {ep} missing"))?;
            let name = em.props.get("EmitterName").and_then(|n| n.as_str()).unwrap_or(&em.name).to_owned();
            let mut lods = vec![];
            for lp in em.props.get("LODLevels").and_then(|l| l.as_array()).cloned().unwrap_or_default() {
                let lod = pkg.get(lp.as_str().unwrap_or("")).ok_or("LOD level missing")?;
                let level = lod.props.get("Level").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
                let enabled = b(lod.props.get("bEnabled"), true);
                let req = pkg.get(lod.props.get("RequiredModule").and_then(|x| x.as_str()).unwrap_or("")).ok_or("required module missing")?;
                let rp = &req.props;
                let required = Required {
                    material: rp.get("Material").and_then(|m| m.as_str()).unwrap_or("").to_owned(),
                    delay: rp.get("EmitterDelay").map_or(0., f), duration: rp.get("EmitterDuration").map_or(1., f), loops: rp.get("EmitterLoops").and_then(|x| x.as_i64()).unwrap_or(0) as i32,
                    delay_first_loop_only: b(rp.get("bDelayFirstLoopOnly"), false), kill_on_deactivate: b(rp.get("bKillOnDeactivate"), false), kill_on_completed: b(rp.get("bKillOnCompleted"), false), use_local_space: b(rp.get("bUseLocalSpace"), false),
                };
                let sp = pkg.get(lod.props.get("SpawnModule").and_then(|x| x.as_str()).unwrap_or("")).ok_or("spawn module missing")?;
                let spawn = Spawn {
                    rate: dist_of(sp.props.get("Rate"), &pkg), rate_scale: dist_of(sp.props.get("RateScale"), &pkg), burst_scale: dist_of(sp.props.get("BurstScale"), &pkg),
                    bursts: sp.props.get("BurstList").and_then(|a| a.as_array()).map(|a| a.iter().map(|x| Burst { time: x.get("Time").map_or(0., f), count: x.get("Count").and_then(|c| c.as_i64()).unwrap_or(0) as i32, count_low: x.get("CountLow").and_then(|c| c.as_i64()).unwrap_or(-1) as i32 }).collect()).unwrap_or_default(),
                };
                let mesh = lod.props.get("TypeDataModule").and_then(|x| x.as_str()).and_then(|p| pkg.get(p)).filter(|e| e.class == "ParticleModuleTypeDataMesh").map(|e| MeshType {
                    mesh: e.props.get("Mesh").and_then(|m| m.as_str()).unwrap_or("").to_owned(),
                    alignment: e.props.get("MeshAlignment").and_then(|m| m.as_str()).unwrap_or("PSMA_MeshFaceCameraWithLockedAxis").to_owned(),
                    camera_facing_option: e.props.get("CameraFacingOption").and_then(|m| m.as_str()).unwrap_or("XAxisFacing_NoUp").to_owned(),
                    camera_facing: b(e.props.get("bCameraFacing"), false), face_camera_direction: b(e.props.get("bFaceCameraDirectionRatherThanPosition"), false), apply_rotation_as_spin: b(e.props.get("bApplyParticleRotationAsSpin"), false),
                });
                let mut modules = vec![];
                for mp in lod.props.get("Modules").and_then(|m| m.as_array()).cloned().unwrap_or_default() {
                    let Some(m) = pkg.get(mp.as_str().unwrap_or("")) else { continue };
                    let p = &m.props;
                    match m.class.as_str() {
                        "ParticleModuleLifetime" => modules.push(Module::Lifetime(dist_of(p.get("Lifetime"), &pkg))),
                        "ParticleModuleSize" => modules.push(Module::Size(dist_of(p.get("StartSize"), &pkg))),
                        "ParticleModuleSizeMultiplyLife" => modules.push(Module::SizeMultiplyLife(dist_of(p.get("LifeMultiplier"), &pkg))),
                        "ParticleModuleSizeScale" => modules.push(Module::SizeScale(dist_of(p.get("SizeScale"), &pkg))),
                        "ParticleModuleColor" => modules.push(Module::Color { color: dist_of(p.get("StartColor"), &pkg), alpha: dist_of(p.get("StartAlpha"), &pkg) }),
                        "ParticleModuleColorScaleOverLife" => modules.push(Module::ColorScaleOverLife { color: dist_of(p.get("ColorScaleOverLife"), &pkg), alpha: dist_of(p.get("AlphaScaleOverLife"), &pkg), emitter_time: b(p.get("bEmitterTime"), false) }),
                        "ParticleModuleRotation" => modules.push(Module::Rotation(dist_of(p.get("StartRotation"), &pkg))),
                        "ParticleModuleRotationRate" => modules.push(Module::RotationRate(dist_of(p.get("StartRotationRate"), &pkg))),
                        "ParticleModuleParameterDynamic" => {
                            let list = p.get("DynamicParams").and_then(|d| d.as_array()).cloned().unwrap_or_default();
                            modules.push(Module::Dynamic(list.iter().map(|d| DynParam {
                                name: d.get("ParamName").and_then(|n| n.as_str()).unwrap_or("").to_owned(), value: dist_of(d.get("ParamValue"), &pkg),
                                use_emitter_time: b(d.get("bUseEmitterTime"), false), spawn_time_only: b(d.get("bSpawnTimeOnly"), false),
                                user_set: d.get("ValueMethod").and_then(|v| v.as_str()).map_or(true, |v| v == "EDPV_UserSet"),
                            }).collect()));
                        }
                        _ => {}
                    }
                }
                lods.push(EmitterLod { level, enabled, required, spawn, mesh, modules });
            }
            lods.sort_by_key(|l| l.level);
            emitters.push(Emitter { name, lods });
        }
        let v3 = |v: Option<&Value>| -> Option<[f32; 3]> { let a = v?.as_array()?; Some([f(a.first()?), f(a.get(1)?), f(a.get(2)?)]) };
        let fixed_bounds = if b(sys.props.get("bUseFixedRelativeBoundingBox"), false) { sys.props.get("FixedRelativeBoundingBox").and_then(|x| Some((v3(x.get("Min"))?, v3(x.get("Max"))?))) } else { None };
        Ok(ParticleSystem { emitters, lod_distances, fixed_bounds })
    }
    /// `PARTICLESYSTEMLODMETHOD_Automatic`: the highest LOD whose distance is <= the camera distance (cm).
    pub fn lod_index(&self, distance_cm: f32) -> usize { self.lod_distances.iter().enumerate().filter(|(_, d)| **d <= distance_cm).map(|(i, _)| i).last().unwrap_or(0) }
}

// ---- simulation --------------------------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Particle {
    pub age: f32,
    pub one_over_life: f32,
    pub size0: [f32; 3],
    pub rotation0: f32,
    pub rotation_rate: f32,
    pub color0: [f32; 3],
    pub alpha0: f32,
    pub rand: [f32; 4],
    /// Dynamic parameter slots evaluated at spawn for `bSpawnTimeOnly` entries.
    pub dyn_spawn: [f32; 4],
}
/// Per-frame values of one live particle, ready for placement.
#[derive(Clone, Copy, Debug)]
pub struct ParticleView { pub size: [f32; 3], pub rotation: f32, pub color: [f32; 4], pub dynamic: [f32; 4], pub rel_time: f32 }

pub struct EmitterInstance {
    pub emitter: usize,
    pub particles: Vec<Particle>,
    pub spawn_fraction: f32,
    pub rng: u64,
    /// Seconds since the component was activated (the emitter delay counts from here).
    pub system_time: f32,
    pub halted: bool,
    pub enabled: bool,
    pub lod: usize,
    last_loop: i64,
}
fn next(r: &mut u64) -> f32 { *r = r.wrapping_add(0x9E37_79B9_7F4A_7C15); let mut z = *r; z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9); z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB); ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32 }

impl EmitterInstance {
    pub fn new(emitter: usize, seed: u64) -> EmitterInstance { EmitterInstance { emitter, particles: vec![], spawn_fraction: 0., rng: seed ^ (emitter as u64).wrapping_mul(0xA24B_AED4_963E_E407), system_time: 0., halted: false, enabled: true, lod: 0, last_loop: -1 } }

    fn lod_data<'a>(&self, sys: &'a ParticleSystem) -> Option<&'a EmitterLod> { let e = &sys.emitters[self.emitter]; e.lods.iter().find(|l| l.level == self.lod as i32).filter(|l| l.enabled) }

    /// Emitter time within the current loop and the loop index, or `None` while the emitter delay runs / after the last loop.
    pub fn emitter_time(&self, req: &Required) -> Option<(f32, i64)> {
        let elapsed = self.system_time - req.delay;
        if elapsed < 0. { return None; }
        if req.duration <= 0. { return Some((elapsed, 0)); }
        let l = (elapsed / req.duration).floor() as i64;
        if req.loops > 0 && l >= req.loops as i64 { return None; }
        Some((elapsed - l as f32 * req.duration, l))
    }

    /// Advances by `dt`: spawn (bursts at their normalised time, spawn rate), age, kill.
    pub fn update(&mut self, sys: &ParticleSystem, dt: f32, params: &InstanceParams) {
        let _ = params;
        let Some(lod) = self.lod_data(sys) else { self.particles.clear(); self.system_time += dt; return };
        let t0 = self.system_time; let t1 = t0 + dt;
        for p in &mut self.particles { p.age += dt; }
        // spawn between t0 and t1 (system time); the emitter time of each sub-interval follows the loop structure
        if !self.halted && self.enabled {
            let req = &lod.required;
            let mut spawn_at: Vec<f32> = vec![];
            // bursts: every loop, at `burst.time * duration` of the emitter time
            let dur = if req.duration > 0. { req.duration } else { f32::MAX };
            for burst in &lod.spawn.bursts {
                let at_in_loop = burst.time * if req.duration > 0. { req.duration } else { 0. };
                let first_loop = ((t0 - req.delay) / dur).floor().max(0.) as i64;
                let last_loop = ((t1 - req.delay) / dur).floor().max(0.) as i64;
                for l in first_loop..=last_loop {
                    if req.loops > 0 && l >= req.loops as i64 { break; }
                    let t = req.delay + l as f32 * if req.duration > 0. { req.duration } else { 0. } + at_in_loop;
                    // fire when the frame interval (t0, t1] (the very first frame includes t0) contains the burst
                    if (t > t0 || (t0 == 0. && t >= 0.)) && t <= t1 + 1e-6 {
                        let n = if burst.count_low >= 0 && burst.count_low < burst.count { burst.count_low + (next(&mut self.rng) * (burst.count - burst.count_low + 1) as f32) as i32 } else { burst.count };
                        let scale = lod.spawn.burst_scale.eval_f(0., 0.).max(0.);
                        for _ in 0..((n as f32) * if scale > 0. { scale } else { 1. }).round().max(0.) as i32 { spawn_at.push(t); }
                    }
                }
            }
            // continuous rate while the emitter runs
            if let Some((et, _)) = self.emitter_time(req) {
                let rate = lod.spawn.rate.eval_f(et, 0.) * lod.spawn.rate_scale.eval_f(et, 0.).max(0.);
                if rate > 0. {
                    self.spawn_fraction += rate * dt;
                    let n = self.spawn_fraction.floor() as i32; self.spawn_fraction -= n as f32;
                    for k in 0..n { spawn_at.push(t0 + dt * (k as f32 + 1.) / (n as f32 + 1.)); }
                }
            }
            for ts in spawn_at { let age = (t1 - ts).max(0.); let mut p = self.spawn(lod, age, ts - req.delay); p.age = age; self.particles.push(p); }
        }
        self.system_time = t1;
        // kill the dead
        self.particles.retain(|p| p.one_over_life <= 0. || p.age * p.one_over_life < 1.);
        let _ = self.last_loop;
    }

    fn spawn(&mut self, lod: &EmitterLod, _age: f32, emitter_time_at_spawn: f32) -> Particle {
        let r = [next(&mut self.rng), next(&mut self.rng), next(&mut self.rng), next(&mut self.rng)];
        let mut p = Particle { age: 0., one_over_life: 0., size0: [1.; 3], rotation0: 0., rotation_rate: 0., color0: [1.; 3], alpha0: 1., rand: r, dyn_spawn: [0.; 4] };
        let et = emitter_time_at_spawn.max(0.);
        for m in &lod.modules {
            match m {
                Module::Lifetime(d) => { let life = d.eval_f(et, r[0]); p.one_over_life = if life > 0. { 1. / life } else { 0. }; }
                Module::Size(d) => { p.size0 = d.eval_v(et, r[1], &InstanceParams::default()); }
                Module::Color { color, alpha } => { p.color0 = color.eval_v(et, r[2], &InstanceParams::default()); p.alpha0 = alpha.eval_f(et, r[3]); }
                Module::Rotation(d) => { p.rotation0 = d.eval_f(et, r[1]) * std::f32::consts::TAU; }
                Module::RotationRate(d) => { p.rotation_rate = d.eval_f(et, r[2]) * std::f32::consts::TAU; }
                Module::Dynamic(list) => for (i, dp) in list.iter().enumerate().take(4) { if dp.user_set && dp.spawn_time_only { p.dyn_spawn[i] = dp.value.eval_f(if dp.use_emitter_time { et } else { 0. }, r[i]); } },
                _ => {}
            }
        }
        p
    }

    /// Current per-particle values (`emitter_time` = the emitter's time in its loop, for `bUseEmitterTime` parameters).
    pub fn views(&self, sys: &ParticleSystem, params: &InstanceParams) -> Vec<ParticleView> {
        let Some(lod) = self.lod_data(sys) else { return vec![] };
        let et = self.emitter_time(&lod.required).map(|(t, _)| t).unwrap_or_else(|| if self.system_time - lod.required.delay > 0. { lod.required.duration.max(0.) } else { 0. });
        self.particles.iter().map(|p| {
            let rel = if p.one_over_life > 0. { (p.age * p.one_over_life).min(1.) } else { 0. };
            let mut size = p.size0; let mut color = [p.color0[0], p.color0[1], p.color0[2], p.alpha0]; let mut dynamic = [0f32; 4];
            let mut have_dyn = false;
            for m in &lod.modules {
                match m {
                    Module::SizeMultiplyLife(d) => { let k = d.eval_v(rel, p.rand[0], params); for c in 0..3 { size[c] *= k[c]; } }
                    Module::SizeScale(d) => { let k = d.eval_v(rel, p.rand[0], params); for c in 0..3 { size[c] *= k[c]; } }
                    Module::ColorScaleOverLife { color: cd, alpha, emitter_time } => { let t = if *emitter_time { et } else { rel }; let k = cd.eval_v(t, p.rand[1], params); for c in 0..3 { color[c] *= k[c]; } color[3] *= alpha.eval_f(t, p.rand[1]); }
                    Module::Dynamic(list) => { have_dyn = true; for (i, dp) in list.iter().enumerate().take(4) { dynamic[i] = if !dp.user_set { 0. } else if dp.spawn_time_only { p.dyn_spawn[i] } else { dp.value.eval_f(if dp.use_emitter_time { et } else { rel }, p.rand[i]) }; } }
                    _ => {}
                }
            }
            let _ = have_dyn;
            ParticleView { size, rotation: p.rotation0 + p.rotation_rate * p.age, color, dynamic, rel_time: rel }
        }).collect()
    }
}

/// Mesh particle placement (`FDynamicMeshEmitterData::CalculateParticleTransform` for `PSMA_MeshFaceCameraWithLockedAxis`): the mesh X axis faces the camera, the up
/// axis (`ZUp`: world Z) stays up, the particle rotation is a spin about the facing axis when `bApplyParticleRotationAsSpin`. Unreal space (cm). Returns the 3x3 whose
/// columns are the scaled local axes in world space, i.e. world = R * local + location.
pub fn mesh_transform(m: &MeshType, size: [f32; 3], rotation: f32, location: DVec3, eye: DVec3, view_forward: DVec3) -> DMat3 {
    let dir_to_camera = if m.face_camera_direction { -view_forward } else { eye - location };
    let d = dir_to_camera.normalize_or_zero();
    let d = if d == DVec3::ZERO { DVec3::X } else { d };
    let (up_world, locked) = match m.camera_facing_option.as_str() {
        "XAxisFacing_ZUp" => (DVec3::Z, true), "XAxisFacing_NegativeZUp" => (-DVec3::Z, true), "XAxisFacing_YUp" => (DVec3::Y, true), "XAxisFacing_NegativeYUp" => (-DVec3::Y, true),
        _ => (DVec3::Z, false),
    };
    let _ = locked;
    // right = up x facing; fall back when looking straight along the up axis
    let mut y = up_world.cross(d);
    if y.length_squared() < 1e-12 { y = DVec3::Y; }
    let y = y.normalize();
    let z = d.cross(y).normalize();
    let facing = DMat3::from_cols(d, y, z);
    let spin = if m.apply_rotation_as_spin { let (s, c) = (rotation as f64).sin_cos(); DMat3::from_cols(DVec3::X, DVec3::new(0., c, s), DVec3::new(0., -s, c)) } else { DMat3::IDENTITY };
    facing * spin * DMat3::from_diagonal(DVec3::new(size[0] as f64, size[1] as f64, size[2] as f64))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(entries: &[f32], stride: usize, scale: f32, bias: f32) -> Table { Table { entry_count: entries.len() / stride, entry_stride: stride, op: 1, time_scale: scale, time_bias: bias, sub_entry_stride: 0, values: entries.to_vec() } }
    #[test] fn lookup_tables_interpolate_clamp_and_bias() {
        // Wiggle Bias of the weak-point effect: [1, 0], TimeBias 0.2, TimeScale 2.5 -> 1 until 0.2 s, 0 from 0.6 s
        let w = t(&[1., 0.], 1, 2.5, 0.2);
        assert_eq!(w.eval(0.0, 0., 1)[0], 1.); assert_eq!(w.eval(0.2, 0., 1)[0], 1.);
        assert!((w.eval(0.4, 0., 1)[0] - 0.5).abs() < 1e-6); assert_eq!(w.eval(0.6, 0., 1)[0], 0.); assert_eq!(w.eval(5., 0., 1)[0], 0.);
        let c = t(&[1.3, 1.3, 1.3], 3, 0., 0.); assert_eq!(c.eval(9., 0., 3)[..3], [1.3, 1.3, 1.3]);
    }
    #[test] fn random_distribution_picks_between_the_sub_entries() {
        // StartRotationRate -0.4..-0.25 (Op 2, EntryStride 2, SubEntryStride 1)
        let r = Table { entry_count: 1, entry_stride: 2, op: 2, time_scale: 0., time_bias: 0., sub_entry_stride: 1, values: vec![-0.4, -0.25] };
        assert!((r.eval(0., 0., 1)[0] + 0.4).abs() < 1e-6 && (r.eval(0., 1., 1)[0] + 0.25).abs() < 1e-6 && (r.eval(0., 0.5, 1)[0] + 0.325).abs() < 1e-6);
    }
    #[test] fn mesh_faces_the_camera_with_z_up() {
        let m = MeshType { mesh: String::new(), alignment: String::new(), camera_facing_option: "XAxisFacing_ZUp".into(), camera_facing: true, face_camera_direction: true, apply_rotation_as_spin: false };
        // camera looking along +X: the ring (X axis = thin axis) must face back at the camera: X axis = -forward
        let r = mesh_transform(&m, [2., 3., 4.], 0., DVec3::ZERO, DVec3::new(-100., 0., 0.), DVec3::X);
        assert!((r.x_axis - DVec3::new(-2., 0., 0.)).length() < 1e-9, "{:?}", r.x_axis);
        assert!(r.z_axis.z > 0. && (r.z_axis.length() - 4.).abs() < 1e-9, "up axis stays up and scaled: {:?}", r.z_axis);
        assert!((r.y_axis.length() - 3.).abs() < 1e-9 && r.y_axis.z.abs() < 1e-12, "{:?}", r.y_axis);
    }
}

