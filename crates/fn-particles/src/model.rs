//! Pure-Rust representation of a decoded `UParticleSystem` (Cascade, UE 4.16). Everything here is data read from the package; units are the original ones
//! (centimetres, seconds, turns for rotations, UE axes: X forward, Y right, Z up).
use crate::dist::Dist;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParticleSystem {
    /// Short object name (`P_Pine_Tree_Med_Destruction`).
    pub name: String,
    /// Long package path (`/Game/Effects/.../P_Pine_Tree_Med_Destruction`).
    pub package: String,
    pub emitters: Vec<Emitter>,
    /// `LODDistances` (cm); `ActivateAutomatic` picks the last level whose distance is <= the camera distance.
    pub lod_distances: Vec<f32>,
    pub lod_method: String,
    /// `FixedRelativeBoundingBox` (min, max) in cm when `bUseFixedRelativeBoundingBox`.
    pub fixed_bounds: Option<([f32; 3], [f32; 3])>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Emitter {
    pub name: String,
    /// `LODLevels`, index = LOD level.
    pub lods: Vec<Lod>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortMode { None, ViewProjDepth, DistanceToView, Age, Unsupported }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alignment { Square, Rectangle, Velocity, TypeSpecific }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubUvInterp { None, Linear, LinearBlend, Random, RandomBlend }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Required {
    /// Material object path (`/Game/.../M_X.M_X`); for mesh emitters the mesh's own materials are used unless a MeshMaterial module overrides them.
    pub material: String,
    pub use_local_space: bool,
    pub emitter_duration: f32,
    pub emitter_delay: f32,
    /// 0 = loop forever.
    pub emitter_loops: i32,
    pub sort_mode: SortMode,
    pub screen_alignment: Alignment,
    pub sub_images_h: u32,
    pub sub_images_v: u32,
    pub interpolation: SubUvInterp,
    pub random_image_time: f32,
    /// `OpacitySourceMode` (`OSM_RedChannel`...) of the cutout texture, kept as text.
    pub opacity_source: String,
    pub alpha_threshold: f32,
    pub cutout_texture: Option<String>,
    pub legacy_emitter_time: bool,
    /// `ParticleModuleRequired.bEnabled` (kept apart from `Lod.enabled`, which is `ParticleLODLevel.bEnabled`).
    pub required_enabled: bool,
}
impl Default for Required {
    fn default() -> Required {
        Required { material: String::new(), use_local_space: false, emitter_duration: 1.0, emitter_delay: 0.0, emitter_loops: 1, sort_mode: SortMode::None, screen_alignment: Alignment::Square, sub_images_h: 1, sub_images_v: 1,
            interpolation: SubUvInterp::None, random_image_time: 0.0, opacity_source: String::new(), alpha_threshold: 0.0, cutout_texture: None, legacy_emitter_time: true, required_enabled: true }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Burst { pub count: i32, pub count_low: i32, pub time: f32 }

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Spawn { pub rate: Dist, pub rate_scale: Dist, pub burst_scale: Dist, pub bursts: Vec<Burst> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis { X, Y, Z }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DynParam { pub name: String, pub value: Dist, pub use_emitter_time: bool, pub spawn_time_only: bool, pub scale_velocity: bool }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GpuInfo {
    pub max_particle_count: u32,
    pub collision: bool,
    /// `EParticleCollisionMode` text (DistanceField / SceneDepth).
    pub collision_mode: String,
    pub constant_acceleration: [f32; 3],
    pub max_lifetime: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TypeData {
    Sprite,
    Mesh { mesh: String, roll_pitch_yaw: Dist, override_material: bool },
    /// `ParticleModuleTypeDataGpu`: the same modules, simulated on the GPU in the original (sprites only).
    Gpu(GpuInfo),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ModuleKind {
    Lifetime { lifetime: Dist },
    Location { start: Dist },
    LocationSphere { start: Dist, radius: Dist, surface_only: bool, velocity: bool, velocity_scale: Dist, positive_z: bool, negative_z: bool },
    LocationCylinder { start: Dist, radius: Dist, height: Dist, height_axis: Axis, surface_only: bool, velocity: bool, velocity_scale: Dist, positive_z: bool, negative_z: bool },
    Velocity { start: Dist, radial: Dist, in_world_space: bool },
    Size { start: Dist },
    SizeMultiplyLife { mult: Dist, x: bool, y: bool, z: bool },
    SizeScaleBySpeed { speed_scale: [f32; 2], max_scale: [f32; 2] },
    Color { color: Dist, alpha: Dist },
    ColorOverLife { color: Dist, alpha: Dist, clamp_alpha: bool },
    ColorScaleOverLife { color: Dist, alpha: Dist },
    Rotation { start: Dist },
    RotationRate { start: Dist },
    MeshRotation { start: Dist },
    MeshRotationRate { start: Dist },
    Acceleration { accel: Dist, world: bool },
    AccelerationConstant { accel: [f32; 3], world: bool },
    Drag { coefficient: Dist },
    CameraOffset { offset: Dist, spawn_time_only: bool },
    Orbit { offset: Dist, rotation: Dist, rotation_rate: Dist },
    ParameterDynamic { params: Vec<DynParam> },
    PivotOffset { offset: [f32; 2] },
    SubUv { index: Dist },
    SubUvMovie { index: Dist, frame_rate: Dist, starting_frame: i32 },
    Collision { damping: Dist, damping_rotation: Dist, max_collisions: Dist, delay: Dist, particle_mass: Dist, halt_when_done: bool },
    CollisionGpu { friction: f32, resilience: Dist, resilience_scale_over_life: Dist, radius_scale: f32, stop: bool },
    MeshMaterial { materials: Vec<String> },
    /// A module class this crate does not simulate (listed so the gap is visible); `class` is the cooked class name.
    Unsupported { class: String, note: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Module {
    pub kind: ModuleKind,
    pub enabled: bool,
    /// `LODValidity` bit mask (bit n = valid in LOD n); 15 = all four.
    pub lod_validity: u32,
    /// `RandomSeedInfo.RandomSeeds[0]` of the `_Seeded` variants.
    pub seed: Option<i32>,
    /// Export name in the package (diagnostics).
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lod {
    pub level: i32,
    pub enabled: bool,
    pub required: Required,
    pub spawn: Spawn,
    pub type_data: TypeData,
    /// Modules in the order of `ParticleLODLevel.Modules` (spawn and update order of the original).
    pub modules: Vec<Module>,
    pub peak_active: i32,
}

/// A texture of an exported material (PNG relative to the particle asset root).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextureRef { pub name: String, pub ue_path: String, pub png: String, pub srgb: bool, pub compression: String, pub width: u32, pub height: u32, pub has_alpha: bool }

/// A material as far as the cooked package keeps it: the root `UMaterial` settings (blend mode, shading model, two sided, clip value), the merged parameter values of the
/// instance chain, and the input -> parameter wiring where the graph still has it (the arithmetic nodes between parameters are stripped from the cooked packages).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MaterialDef {
    pub name: String,
    pub ue_path: String,
    pub root: String,
    pub blend_mode: String,
    pub shading_model: String,
    pub two_sided: bool,
    pub clip: f32,
    pub textures: std::collections::BTreeMap<String, TextureRef>,
    pub scalars: std::collections::BTreeMap<String, f32>,
    pub vectors: std::collections::BTreeMap<String, [f32; 4]>,
    /// Material input (`BaseColor`, `EmissiveColor`, `Opacity`, `OpacityMask`) -> texture parameter, only when wired straight to a texture sample.
    pub direct: std::collections::BTreeMap<String, String>,
    /// Material inputs that have something connected.
    pub connected: Vec<String>,
    /// Parameter chosen as the colour texture of the particle (name heuristic when the graph is not recoverable) and how it was chosen.
    pub color_texture: Option<String>,
    pub color_source: String,
    pub notes: Vec<String>,
}

impl MaterialDef {
    pub fn is_translucent(&self) -> bool { self.blend_mode.contains("Translucent") }
    pub fn is_additive(&self) -> bool { self.blend_mode.contains("Additive") }
    pub fn is_masked(&self) -> bool { self.blend_mode.contains("Masked") }
    pub fn is_modulate(&self) -> bool { self.blend_mode.contains("Modulate") }
    pub fn is_unlit(&self) -> bool { self.shading_model.contains("Unlit") }
}

/// A static mesh of a mesh emitter in game space (metres, Y up) as written by the exporter.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MeshDef {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// (first index, index count, material slot)
    pub sections: Vec<(u32, u32, u32)>,
    /// Material object path per slot (the mesh's own materials).
    pub materials: Vec<String>,
}

impl ParticleSystem {
    /// LOD level for a camera distance (cm), `ActivateAutomatic`.
    pub fn lod_for_distance(&self, cm: f32) -> usize {
        let mut lod = 0;
        for (i, d) in self.lod_distances.iter().enumerate() { if cm >= *d { lod = i; } }
        lod
    }
    /// Human readable summary of LOD 0 of every emitter (tools / logs).
    pub fn describe(&self) -> String {
        let mut s = format!("{} ({} emitters, LOD distances {:?})\n", self.name, self.emitters.len(), self.lod_distances);
        for e in &self.emitters {
            let Some(l) = e.lods.first() else { s.push_str(&format!("  {}: no LOD\n", e.name)); continue };
            let td = match &l.type_data { TypeData::Sprite => "sprite".to_string(), TypeData::Mesh { mesh, .. } => format!("mesh {}", mesh.rsplit('/').next().unwrap_or(mesh)), TypeData::Gpu(g) => format!("gpu-sprite max {} collision {}", g.max_particle_count, g.collision) };
            let bursts: Vec<String> = l.spawn.bursts.iter().map(|b| format!("{}(low {})@{}", b.count, b.count_low, b.time)).collect();
            s.push_str(&format!("  {} [{}] mat {} bursts [{}] rate {:?} loops {} dur {} local {} align {:?} sort {:?} sub {}x{} lods {} enabled {} modules {} flags [{}]\n", e.name, td, l.required.material.rsplit('/').next().unwrap_or(""), bursts.join(","), l.spawn.rate.mean(0.0, 1)[0],
                l.required.emitter_loops, l.required.emitter_duration, l.required.use_local_space, l.required.screen_alignment, l.required.sort_mode, l.required.sub_images_h, l.required.sub_images_v, e.lods.len(), l.enabled, l.modules.len(), e.lods.iter().map(|l| format!("{}{}", if l.enabled { "L" } else { "l" }, if l.required.required_enabled { "R" } else { "r" })).collect::<Vec<_>>().join(" ")));
        }
        s
    }
    /// Unsupported module classes used by LOD 0 of any emitter (class, count).
    pub fn unsupported(&self) -> Vec<(String, usize)> {
        let mut v: Vec<(String, usize)> = vec![];
        for e in &self.emitters { if let Some(l) = e.lods.first() { for m in &l.modules { if let ModuleKind::Unsupported { class, .. } = &m.kind {
            match v.iter_mut().find(|x| &x.0 == class) { Some(x) => x.1 += 1, None => v.push((class.clone(), 1)) } } } } }
        v
    }
}
