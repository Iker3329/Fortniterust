//! FOLIAGE HOOK: the ambient ground cover of `Athena_Terrain` (the map's `FoliageInstancedStaticMeshComponent`s: 555 419 `S_WavyGrass_01` instances) drawn through
//! the renderer's instanced static-mesh path.
//!
//! Data (`<props>/foliage/`, written by `fn-export props foliage`): `foliage.json` (mesh, material, cull distances, instance format) and `instances.bin`
//! (UE world cm position, rotation, uniform scale, `PerInstanceRandom` of every instance, parent + `LevelTransform` already applied).
//!
//! RECOVERED (exact): the instance transforms (position, yaw, scale), the mesh (glTF of `S_WavyGrass_01`, 204 triangles, one LOD), the cull distances
//! (`InstanceStartCullDistance` 4096 cm, `InstanceEndCullDistance` 5120 cm: the mesh is culled at the end distance with a fade of `end - start`), no collision,
//! no shadow casting. There is no `bEnableDensityScaling` on the components, so every baked instance is used (the density is the baked one).
//! NOT RECOVERED / left out: the wind (the `WorldPositionOffset` graph of `M_FieldGrassMatching`: only its parameter values are exported), the blade opacity and the
//! world-projected terrain colour of the material graph (drawn opaque with the exported terrain grass texture on the mesh UVs), the instance tilt (only the yaw of the
//! instance rotation is used by the static-mesh path; tilt reaches 43 degrees on slopes, median 0.3), the terrain height (the world is the flat baseplate: y = 0, like the props).
//!
//! Placement matches the props: UE cm -> game m `(x, z, y) * 0.01` relative to the layout centre, moved to [`crate::props::ANCHOR`]; only the baseplate is filled.
//! Cost: instances live in 32 m cells; only the blades within the cull distance (+ a margin) of the camera are handed to the renderer, rebuilt when the camera moved 8 m.
use fn_render::{StaticMeshHandle, StaticMeshInstance};
use glam::Vec3;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Side of a spatial cell (metres).
pub const CELL_M: f32 = 32.;
/// The visible set is rebuilt when the camera moved this far (metres, ground plane) from where it was built.
pub const REBUILD_STEP_M: f32 = 8.;
pub const MAGIC: &[u8; 4] = b"FNGR";
pub const RECORD_BYTES: usize = 28;

/// One instance in game space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Blade { pub pos: Vec3, pub yaw: f32, pub scale: f32, pub random: f32 }

/// Everything read from `<props>/foliage/foliage.json` that the loader needs.
#[derive(Clone, Debug)]
pub struct Spec {
    pub dir: PathBuf,
    pub name: String,
    pub gltf: PathBuf,
    pub materials: Vec<(usize, PathBuf)>,
    pub bounds_extent_cm: [f32; 3],
    pub start_cm: f32,
    pub end_cm: f32,
    pub cast_shadow: bool,
    pub instances_file: PathBuf,
}

fn f(v: Option<&Value>) -> Option<f32> { v.and_then(|v| v.as_f64()).map(|x| x as f32) }

impl Spec {
    /// `<props_dir>/foliage/foliage.json`; `Err` when the export is missing (the game then runs without ground cover).
    pub fn read(props_dir: &Path) -> Result<Spec, String> {
        let dir = props_dir.join("foliage");
        let text = std::fs::read_to_string(dir.join("foliage.json")).map_err(|e| format!("{}: {e}", dir.join("foliage.json").display()))?;
        let j: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if j.get("schema").and_then(|v| v.as_u64()) != Some(1) { return Err("foliage.json: unknown schema".into()); }
        let mesh = j.get("mesh").ok_or("foliage.json: no mesh")?;
        let name = mesh.get("name").and_then(|v| v.as_str()).ok_or("foliage.json: mesh.name")?.to_owned();
        let mdir = dir.join(mesh.get("dir").and_then(|v| v.as_str()).unwrap_or(&name));
        let gltf = mdir.join(mesh.get("gltf").and_then(|v| v.as_str()).ok_or("foliage.json: mesh.gltf")?);
        let materials = j.get("materials").and_then(|v| v.as_array()).ok_or("foliage.json: materials")?.iter()
            .filter_map(|m| Some((m.get("slot")?.as_u64()? as usize, mdir.join(m.get("mat_json")?.as_str()?)))).collect::<Vec<_>>();
        if materials.is_empty() { return Err("foliage.json: no material".into()); }
        let ext = mesh.pointer("/bounds_cm/extent").and_then(|v| v.as_array()).filter(|a| a.len() == 3).map(|a| [f(a.first()).unwrap_or(0.), f(a.get(1)).unwrap_or(0.), f(a.get(2)).unwrap_or(0.)]).unwrap_or([0.; 3]);
        let cull = j.get("cull").ok_or("foliage.json: no cull")?;
        let (start_cm, end_cm) = (f(cull.get("instance_start_cull_distance_cm")).ok_or("cull start")?, f(cull.get("instance_end_cull_distance_cm")).ok_or("cull end")?);
        let cast_shadow = j.pointer("/render_flags/cast_shadow").and_then(|v| v.as_bool()).unwrap_or(false);
        let instances_file = dir.join(j.pointer("/instances/file").and_then(|v| v.as_str()).unwrap_or("instances.bin"));
        Ok(Spec { dir, name, gltf, materials, bounds_extent_cm: ext, start_cm, end_cm, cast_shadow, instances_file })
    }
    pub fn cull_m(&self) -> f32 { self.end_cm * 0.01 }
}

/// `(pitch-free) yaw` of a UE quaternion in game space: UE yaw about +Z, turned into the rotation about +Y of the `(x, z, y)` axis map (the sign flips).
pub fn game_yaw(q: [f32; 4]) -> f32 {
    let [x, y, z, w] = q;
    -(2. * (w * z + x * y)).atan2(1. - 2. * (y * y + z * z))
}

/// Reads `instances.bin` and keeps the instances inside the baseplate around `anchor`, mapped like the props: `anchor + (x - cx, 0, y - cy) * 0.01`.
pub fn read_instances(bytes: &[u8], center_cm: [f32; 2], anchor: Vec3, half_m: f32) -> Result<Vec<Blade>, String> {
    if bytes.len() < 12 || &bytes[..4] != MAGIC { return Err("instances.bin: bad magic".into()); }
    let u32at = |p: usize| u32::from_le_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]);
    if u32at(4) != 1 { return Err(format!("instances.bin: version {}", u32at(4))); }
    let n = u32at(8) as usize;
    if bytes.len() != 12 + n * RECORD_BYTES { return Err("instances.bin: size does not match the count".into()); }
    let fl = |p: usize| f32::from_le_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]);
    let sn = |p: usize| i16::from_le_bytes([bytes[p], bytes[p + 1]]) as f32 / 32767.;
    let mut out = Vec::new();
    for i in 0..n {
        let o = 12 + i * RECORD_BYTES;
        let (x, y) = ((fl(o) - center_cm[0]) * 0.01, (fl(o + 4) - center_cm[1]) * 0.01);
        let pos = Vec3::new(anchor.x + x, 0., anchor.z + y);
        if pos.x.abs() > half_m || pos.z.abs() > half_m { continue; }
        let q = [sn(o + 12), sn(o + 14), sn(o + 16), sn(o + 18)];
        out.push(Blade { pos, yaw: game_yaw(q), scale: fl(o + 20), random: fl(o + 24) });
    }
    Ok(out)
}

/// Loader step: reads the export next to the props, decodes + uploads the mesh / material through `fn_render::static_loader` and chunks the instances. `None` + a log
/// line when the export is missing or unreadable (the world simply has no ground cover). `lib.center_cm` is the layout centre the props are placed around.
pub fn load(renderer: &mut fn_render::Renderer, props_dir: &Path, lib: &crate::props::Library) -> Option<Grass> {
    let t = std::time::Instant::now();
    let spec = Spec::read(props_dir).map_err(|e| eprintln!("[foliage] {e}: no ground cover")).ok()?;
    let bytes = std::fs::read(&spec.instances_file).map_err(|e| eprintln!("[foliage] {}: {e}: no ground cover", spec.instances_file.display())).ok()?;
    let blades = read_instances(&bytes, [lib.center_cm[0], lib.center_cm[1]], crate::props::ANCHOR, fn_game::HALF - 1.).map_err(|e| eprintln!("[foliage] {e}: no ground cover")).ok()?;
    let gc = fn_render::static_loader::GroundCoverSpec { name: &spec.name, gltf: &spec.gltf, materials: &spec.materials, bounds_extent_cm: spec.bounds_extent_cm, cull_cm: spec.end_cm, fade_cm: spec.end_cm - spec.start_cm, cast_shadow: spec.cast_shadow };
    let prep = fn_render::static_loader::prepare_ground_cover(&gc).map_err(|e| eprintln!("[foliage] {}: {e}: no ground cover", spec.name)).ok()?;
    let mut cache = HashMap::new();
    let loaded = fn_render::static_loader::upload_prepared(renderer, prep, &mut cache);
    let grass = Grass::new(loaded.mesh, spec.cull_m(), blades);
    eprintln!("[foliage] {}: {} instances of the map's {} placed in the baseplate, cull {:.1} m (fade {:.1} m), {:.0} ms", spec.name, grass.total(), (bytes.len() - 12) / RECORD_BYTES, spec.cull_m(), (spec.end_cm - spec.start_cm) * 0.01, t.elapsed().as_secs_f32() * 1000.);
    Some(grass)
}

/// The spatially chunked blades + the list currently handed to the renderer.
pub struct Grass {
    pub mesh: StaticMeshHandle,
    cull_m: f32,
    cells: HashMap<(i32, i32), Vec<Blade>>,
    total: usize,
    built_at: Option<Vec3>,
    list: Vec<StaticMeshInstance>,
}

fn cell_of(p: Vec3) -> (i32, i32) { ((p.x / CELL_M).floor() as i32, (p.z / CELL_M).floor() as i32) }

impl Grass {
    pub fn new(mesh: StaticMeshHandle, cull_m: f32, blades: Vec<Blade>) -> Grass {
        let total = blades.len();
        let mut cells: HashMap<(i32, i32), Vec<Blade>> = HashMap::new();
        for b in blades { cells.entry(cell_of(b.pos)).or_default().push(b); }
        Grass { mesh, cull_m, cells, total, built_at: None, list: vec![] }
    }
    /// Blades placed (inside the baseplate).
    pub fn total(&self) -> usize { self.total }
    /// The instances for the renderer (the blades near the camera at the last rebuild).
    pub fn instances(&self) -> &[StaticMeshInstance] { &self.list }

    /// Rebuilds the near list when the camera moved [`REBUILD_STEP_M`] (or on the first call); returns true when the list changed.
    pub fn update(&mut self, eye: Vec3) -> bool {
        if let Some(b) = self.built_at { if (Vec3::new(eye.x - b.x, 0., eye.z - b.z)).length() < REBUILD_STEP_M { return false; } }
        self.built_at = Some(eye);
        let reach = self.cull_m + REBUILD_STEP_M;
        let (lo, hi) = (cell_of(Vec3::new(eye.x - reach, 0., eye.z - reach)), cell_of(Vec3::new(eye.x + reach, 0., eye.z + reach)));
        self.list.clear();
        for cz in lo.1..=hi.1 {
            for cx in lo.0..=hi.0 {
                let Some(v) = self.cells.get(&(cx, cz)) else { continue };
                for b in v {
                    let d = Vec3::new(b.pos.x - eye.x, 0., b.pos.z - eye.z);
                    if d.length_squared() <= reach * reach { self.list.push(StaticMeshInstance::new(self.mesh, b.pos).with_yaw(b.yaw).with_scale(b.scale).with_sway(0.)); }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rec(x: f32, y: f32, q: [i16; 4], s: f32, r: f32) -> Vec<u8> {
        let mut v = vec![]; for a in [x, y, 0.] { v.extend_from_slice(&a.to_le_bytes()); } for a in q { v.extend_from_slice(&a.to_le_bytes()); } v.extend_from_slice(&s.to_le_bytes()); v.extend_from_slice(&r.to_le_bytes()); v
    }
    fn file(recs: &[Vec<u8>]) -> Vec<u8> { let mut b = MAGIC.to_vec(); b.extend_from_slice(&1u32.to_le_bytes()); b.extend_from_slice(&(recs.len() as u32).to_le_bytes()); for r in recs { b.extend_from_slice(r); } b }
    #[test] fn ue_yaw_90_is_game_yaw_minus_90() {
        let s = std::f32::consts::FRAC_1_SQRT_2;
        assert!((game_yaw([0., 0., s, s]) + std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!(game_yaw([0., 0., 0., 1.]).abs() < 1e-6);
    }
    #[test] fn instances_map_like_the_props_and_the_baseplate_clips() {
        let ident = [0, 0, 0, 32767];
        let b = file(&[rec(1000., 2000., ident, 1.5, 0.25), rec(1000. + 100. * 300., 2000., ident, 1., 0.), rec(1100., 1900., ident, 1., 0.5)]);
        let v = read_instances(&b, [1000., 2000.], Vec3::new(0., 0., -10.), 256.).unwrap();
        assert_eq!(v.len(), 2, "the instance 300 m away is outside the 256 m baseplate half-width... of the anchored map");
        assert_eq!((v[0].pos, v[0].scale, v[0].random), (Vec3::new(0., 0., -10.), 1.5, 0.25));
        assert!((v[1].pos - Vec3::new(1., 0., -11.)).length() < 1e-5, "{:?}", v[1].pos);
        assert!(read_instances(&b[..20], [0., 0.], Vec3::ZERO, 1.).is_err() && read_instances(b"nope", [0., 0.], Vec3::ZERO, 1.).is_err());
    }
    #[test] fn only_blades_near_the_camera_are_listed_and_the_list_is_rebuilt_per_step() {
        let blades: Vec<Blade> = (-40..40).flat_map(|x| (-40..40).map(move |z| Blade { pos: Vec3::new(x as f32 * 5., 0., z as f32 * 5.), yaw: 0., scale: 1., random: 0. })).collect();
        let n = blades.len();
        let mut g = Grass::new(StaticMeshHandle(0), 51.2, blades);
        assert!(g.update(Vec3::new(0., 1., 0.)));
        let near = g.instances().len();
        assert!(near > 100 && near < n / 4, "{near} of {n}");
        assert!(g.instances().iter().all(|i| (i.pos - Vec3::ZERO).length() <= 51.2 + REBUILD_STEP_M + 1e-3 && i.sway == 0.));
        assert!(!g.update(Vec3::new(3., 1., 0.)), "inside the rebuild step: nothing changes");
        assert!(g.update(Vec3::new(100., 1., 0.)) && g.instances().iter().all(|i| (i.pos.x - 100.).abs() <= 60. + 1e-3));
    }
}
