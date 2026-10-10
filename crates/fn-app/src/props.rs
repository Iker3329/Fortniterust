//! The REAL Fortnite 1.8 trees / rocks / bushes in the game: what is loaded, where they are placed, what they collide with and what the renderer draws.
//!
//! Data (all from `<assets>/props`, written by `fn-export props export`, read through `fn-propdata`):
//! * `catalog.json` + `<Kind>/props.json`: category, hit points (`health.max`, the `AttributesBuildingProps` row), resource type + rate rows;
//! * `<Kind>/collision.json`: the cooked BodySetup convex hulls (vertices only; `fn-hull` rebuilds the polytopes);
//! * `layout/all.json` (9609 `Athena_Terrain` placements, UE world cm) and `layout/sample.json` (the densest tree + rock cluster, used as the centre);
//! * the glTF LODs + materials, uploaded through `fn_render::static_loader` (see `loader.rs`).
//!
//! RECOVERED (exact): meshes, collision hulls, placements (position / yaw / uniform scale), per-kind health and resource type.
//! ASSUMED / CHOSEN here (labelled in the README): see the constants below, and in `Library::spawn_layout`.
use fn_combat::{Harvestable, Hull, HullSet, Kind, Resource};
use fn_propdata::{Collision, PropCatalog, PropInstance as Placement, PropKind, ResourceInfo, ResourceKind, Shape};
use fn_render::StaticMeshHandle;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

/// Radius (metres) of the part of `Athena_Terrain` that is placed around the anchor (`--prop-radius`). CHOSEN: 150 m keeps ~hundreds of props around the
/// spawn (the layout is ~9600 harvestables over the whole island) while the baseplate (512 m square) and the draw distances stay meaningful.
pub const DEFAULT_RADIUS_M: f32 = 150.;
/// WORLD (2026-10-07): the default `--prop-radius`: with the POIs on, the WHOLE island (1.8 places every layout actor; the far ones are drawn by their HLOD
/// proxy, `crate::poi::HlodCluster`, and the rest of the island's actors are the POI files'), else [`DEFAULT_RADIUS_M`].
pub fn default_radius_m() -> f32 { if crate::poi::enabled() { f32::INFINITY } else { DEFAULT_RADIUS_M } }
/// World position the centre of the layout (the `sample.json` centre: the harvestable with the most trees AND rocks within 40 m) lands on. CHOSEN: 22 m
/// in front of the spawn (the spawn stays where `fn_game::spawn()` puts it, on the baseplate looking toward -Z: the 1.8 data has no player start, the
/// players drop from the battle bus). TERRAIN: with the landscape installed every prop stands at its real UE height (`Library::spawn_layout_on`); without it on y = 0.
pub const ANCHOR: Vec3 = Vec3::new(0., 0., SPAWN_Z - 22.);
/// Placements whose origin is nearer than this to the spawn (or whose hull is within `SPAWN_CLEAR_HULL_M` of it) are dropped so the pawn never starts inside one.
pub const SPAWN_CLEAR_M: f32 = 3.;
pub const SPAWN_CLEAR_HULL_M: f32 = 1.5;

/// World z of the spawn (`fn_game::spawn().2`; a test keeps them equal).
const SPAWN_Z: f32 = 12.;

/// How the world is filled when the real props are available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Keep whatever the sim already has (the three procedural placeholders, or what the script plants).
    Off,
    /// The real layout around the anchor (the default in the live game and for `--screenshot`).
    Layout,
    /// A single real prop of the given kind id in front of the spawn, placed so its hull surface is `ONE_PROP_GAP_M` from the pawn (scripted chop tests: `props realtree` / `realrock`).
    One(&'static str),
    /// Remove every prop.
    Empty,
}

/// Distance from the spawn pawn to the hull surface of the test prop (the old placeholder tree stood 1.8 m - 0.4 m = 1.4 m away).
pub const ONE_PROP_GAP_M: f32 = 1.4;
pub const ONE_TREE_ID: &str = "Athena_Tree_Large_1";
pub const ONE_ROCK_ID: &str = "Athena_Prop_Rocks_StoneLarge01";

impl Mode {
    /// True when the real assets are needed to carry it out (everything but `Off` / `Empty`).
    pub fn needs_real(self) -> bool { matches!(self, Mode::Layout | Mode::One(_)) }
}

/// One prop kind: what the catalog says plus its collision in prop-local game space.
#[derive(Clone, Debug)]
pub struct Proto {
    pub id: String,
    pub display: String,
    pub category: String,
    /// Audio / HUD family: `Rock` for the `rock` category, `Tree` for everything else (bushes and hedges sound like trees: ASSUMED, there is no separate hit cue).
    pub kind: Kind,
    pub res: Resource,
    /// `health.max` of the catalog (RECOVERED: 600 large oak, 450 large rock, 300 pines, 200 medium trees, 30 shrubs, ...).
    pub health: f32,
    /// The catalog's resource rows, exposed unmodified (their meaning is native code, see `ResourceInfo`): the amount a hit awards stays the provisional
    /// `fn_combat::HARVEST_DAMAGE` (50 per swing) rule.
    pub resource: ResourceInfo,
    /// Hull pieces in prop-local game space (metres, Y up, unit scale, yaw 0). Empty = no collision (the export has no usable hull).
    pub hull: HullSet,
    pub height_m: f32,
    /// BUILDING: `ABuildingActor` +0x564 bits of the class ([`build_flags_of`]) and its `CentroidOffset` (UE cm, actor local).
    pub build_flags: u8,
    pub centroid_offset_cm: [f32; 3],
}

/// The +0x564 bits a building placement reads (`fn_building::placement::BlockFlags`), from a prop's exported `props.json`: the native class
/// at the end of `class_chain` gives the default (`ABuildingSMActor` ctor @14045b960 `&= ~3; |= 4`; `ABuildingProp` @14045adb0 /
/// `ABuildingContainer` @140458c60 clear bit2), then the merged class default object's `bDoNotBlockBuildings` (bit0),
/// `bDestroyOnPlayerBuildingPlacement` (bit1) and `bUseCentroidForBlockBuildingsCheck` (bit2) override it. Also returns `CentroidOffset`.
pub fn build_flags_of(raw: &serde_json::Value) -> (u8, [f32; 3]) {
    let native = raw.get("class_chain").and_then(|c| c.as_array()).and_then(|c| c.last()).and_then(|v| v.as_str()).unwrap_or("/Script/FortniteGame.BuildingProp");
    let mut flags: u8 = if native.ends_with(".BuildingProp") || native.ends_with(".BuildingContainer") { 0 } else { 4 };
    let cdo = raw.get("ue_properties").and_then(|u| u.get("class_default_object"));
    for (name, bit) in [("bDoNotBlockBuildings", 1u8), ("bDestroyOnPlayerBuildingPlacement", 2), ("bUseCentroidForBlockBuildingsCheck", 4)] {
        if let Some(b) = cdo.and_then(|c| c.get(name)).and_then(|v| v.as_bool()) { if b { flags |= bit } else { flags &= !bit } }
    }
    let off = cdo.and_then(|c| c.get("CentroidOffset")).and_then(|v| v.as_array()).map(|a| [0, 1, 2].map(|i| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.) as f32)).unwrap_or([0.; 3]);
    (flags, off)
}

/// The loaded catalog + the placements to spawn. Pure CPU data (no GPU): built on the loader's worker thread.
#[derive(Clone, Debug)]
pub struct Library {
    pub protos: Vec<Proto>,
    by_id: HashMap<String, u32>,
    placements: Vec<Placement>,
    /// UE world position (cm) of the layout centre, `[0; 3]` when no layout was readable.
    pub center_cm: [f32; 3],
    pub radius_m: f32,
    /// Human-readable load notes (layout fallbacks, kinds without a hull, ...), printed once.
    pub notes: Vec<String>,
    /// POI: a placement that is a loaded POI actor (`crate::poi::occupies`: same class, same spot) is left to `crate::poi` (drawn, colliding and harvestable there).
    poi: bool,
}

pub fn resource_of(r: ResourceKind, kind: Kind) -> Resource {
    match r { ResourceKind::Wood => Resource::Wood, ResourceKind::Stone => Resource::Stone, ResourceKind::Metal => Resource::Metal, ResourceKind::None => if kind == Kind::Rock { Resource::Stone } else { Resource::Wood } }
}

/// Points approximating a sphere / capsule end for the (unused by the 1.8 trees and rocks, but valid in the schema) sphere and capsule primitives.
fn sphere_points(c: Vec3, r: f32) -> Vec<Vec3> {
    let mut v = vec![c + Vec3::Z * r, c - Vec3::Z * r];
    for lat in 1..4 { let a = std::f32::consts::FRAC_PI_2 * (lat as f32 / 4.); for lon in 0..8 { let b = std::f32::consts::TAU * lon as f32 / 8.; for s in [-1., 1.] { v.push(c + Vec3::new(r * a.cos() * b.cos(), r * a.cos() * b.sin(), s * r * a.sin())); } } }
    v
}

/// The hull pieces of a `collision.json` in prop-local GAME space. UE cm, Z up, left handed -> `(x, z, y) * 0.01` (the same axis map as the glTF meshes and the layout).
pub fn hulls_of(c: &Collision) -> Vec<Hull> {
    let ue = |p: Vec3| Vec3::new(p.x, p.z, p.y) * 0.01;
    let quat = |q: [f32; 4]| Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize();
    let mut out = vec![];
    for s in &c.shapes {
        let pts: Vec<Vec3> = match s {
            Shape::Convex { vertices, transform_translation, transform_rotation, transform_scale, .. } => {
                let (r, sc, t) = (quat(*transform_rotation), Vec3::from(*transform_scale), Vec3::from(*transform_translation));
                vertices.iter().map(|v| ue(r * (Vec3::from(*v) * sc) + t)).collect()
            }
            Shape::Box { center, rotation, half_extent } => {
                let (r, c, h) = (quat(*rotation), Vec3::from(*center), Vec3::from(*half_extent));
                let mut v = vec![];
                for x in [-1., 1.] { for y in [-1., 1.] { for z in [-1., 1.] { v.push(ue(c + r * (h * Vec3::new(x, y, z)))); } } }
                v
            }
            Shape::Sphere { center, radius } => sphere_points(ue_unscaled(*center), radius * 0.01),
            Shape::Capsule { center, rotation, radius, half_length } => {
                let (r, c) = (quat(*rotation), Vec3::from(*center));
                let axis = r * Vec3::Z * *half_length;
                let mut v = sphere_points(Vec3::ZERO, 1.).iter().map(|p| ue(c + axis + *p * *radius)).collect::<Vec<_>>();
                v.extend(sphere_points(Vec3::ZERO, 1.).iter().map(|p| ue(c - axis + *p * *radius)));
                v
            }
        };
        if let Some(h) = Hull::from_points(&pts) { out.push(h); }
    }
    out
}
fn ue_unscaled(p: [f32; 3]) -> Vec3 { Vec3::new(p[0], p[2], p[1]) * 0.01 }

/// A prop-local hull set placed in the world: scale, rotation about +Y by `yaw`, translation `pos`.
pub fn place_hulls(local: &HullSet, pos: Vec3, yaw: f32, scale: f32) -> HullSet { place_hulls_mirrored(local, pos, yaw, scale, false) }
/// [`place_hulls`] of a mirrored placement (`fn_propdata::PropInstance::mirror_x`: local X negated before the yaw, a reflection, so the hull winding flips).
pub fn place_hulls_mirrored(local: &HullSet, pos: Vec3, yaw: f32, scale: f32, mirror_x: bool) -> HullSet {
    let r = Quat::from_rotation_y(yaw);
    let m = if mirror_x { Vec3::new(-1., 1., 1.) } else { Vec3::ONE };
    local.mapped(move |v| pos + r * (v * m * scale), mirror_x)
}

impl Library {
    /// Builds the library from a loaded catalog: a `Proto` for every kind of the loaded categories, the layout filtered to `radius_m` around the sample centre.
    pub fn from_catalog(cat: &PropCatalog, radius_m: f32) -> Library { Self::from_catalog_poi(cat, radius_m, true) }
    /// [`Library::from_catalog`]; `poi` false = the layout keeps the placements that are POI actors too (as with `FN_POI=0`; the layout tests).
    pub fn from_catalog_poi(cat: &PropCatalog, radius_m: f32, poi: bool) -> Library {
        let mut notes = vec![];
        let mut protos = vec![];
        let mut by_id = HashMap::new();
        for k in cat.kinds.iter().filter(|k| fn_render::static_loader::DEFAULT_CATEGORIES.contains(&k.category.as_str())) {
            let kind = if k.is_rock() { Kind::Rock } else { Kind::Tree };
            let hull = HullSet::new(hulls_of(&k.collision));
            if hull.is_empty() { notes.push(format!("{}: no usable collision hull (the prop is drawn but does not block)", k.id)); }
            by_id.insert(k.id.clone(), protos.len() as u32);
            let (build_flags, centroid_offset_cm) = build_flags_of(&k.raw);
            protos.push(Proto { id: k.id.clone(), display: k.display.clone(), category: k.category.clone(), kind, res: resource_of(k.resource.kind, kind), health: if k.health > 0. { k.health } else { Harvestable::max_health(kind) }, resource: k.resource.clone(), hull, height_m: k.height_m(), build_flags, centroid_offset_cm });
        }
        // the layout: all of Athena_Terrain around the sample centre; sample.json alone when all.json is unreadable
        let mut center_cm = [0.; 3];
        let sample = cat.load_sample_layout();
        if let Ok(s) = &sample { center_cm = s.center_cm; }
        let mut placements = vec![];
        match cat.load_full_layout() {
            Ok(all) => {
                if sample.is_err() { if let Some(f) = all.instances.iter().find(|p| by_id.contains_key(&p.kind)) { center_cm = f.abs_cm; notes.push("layout/sample.json unreadable: centred on the first placement of all.json".into()); } }
                if let Some(c) = crate::poi::world_center() { center_cm = c; notes.push(format!("--world-center: layout centred on ({:.0}, {:.0}) cm", c[0], c[1])); } // POI (dev)
                let r2 = (radius_m * 100.) * (radius_m * 100.);
                placements = all.instances.into_iter().filter(|p| by_id.contains_key(&p.kind) && (p.abs_cm[0] - center_cm[0]).powi(2) + (p.abs_cm[1] - center_cm[1]).powi(2) <= r2).collect();
            }
            Err(e) => match sample {
                Ok(s) => { notes.push(format!("layout/all.json unreadable ({e}): the 40-placement sample is used")); placements = s.instances.into_iter().filter(|p| by_id.contains_key(&p.kind)).collect(); }
                Err(e2) => notes.push(format!("no layout readable (all.json: {e}; sample.json: {e2}): no real props are placed")),
            },
        }
        Library { protos, by_id, placements, center_cm, radius_m, notes, poi }
    }

    pub fn proto_index(&self, id: &str) -> Option<u32> { self.by_id.get(id).copied() }
    pub fn proto(&self, i: u32) -> Option<&Proto> { self.protos.get(i as usize) }
    /// The kind ids a run in `mode` places (catalog order): the distinct kinds of the layout inside the radius, or the single test prop. Only these are loaded.
    pub fn kinds_needed(&self, mode: Mode) -> Vec<String> {
        match mode {
            Mode::Layout => { let used: std::collections::HashSet<&str> = self.placements.iter().map(|p| p.kind.as_str()).collect(); self.protos.iter().filter(|p| used.contains(p.id.as_str())).map(|p| p.id.clone()).collect() }
            Mode::One(id) => self.proto_index(id).map(|_| vec![id.to_owned()]).unwrap_or_default(),
            Mode::Off | Mode::Empty => vec![],
        }
    }
    /// Placements inside the radius (before the spawn clearance).
    pub fn placement_count(&self) -> usize { self.placements.len() }

    fn harvestable(&self, proto: u32, pos: Vec3, yaw: f32, scale: f32) -> Harvestable { self.harvestable_mirrored(proto, pos, yaw, scale, false) }
    /// MIRRORFIX: a placement with a negative-determinant scale keeps its mirror for the render instance and the collision.
    fn harvestable_mirrored(&self, proto: u32, pos: Vec3, yaw: f32, scale: f32, mirror_x: bool) -> Harvestable {
        let p = &self.protos[proto as usize];
        let hull = Arc::new(place_hulls_mirrored(&p.hull, pos, yaw, scale, mirror_x));
        let mut h = Harvestable::real(p.kind, p.res, pos, yaw, scale, p.health, proto, Some(hull));
        h.mirror_x = mirror_x;
        // GetCentroid: the actor transform applied to CentroidOffset (UE cm -> game m like the hulls)
        let o = p.centroid_offset_cm;
        let local = Vec3::new(o[0], o[2], o[1]) * 0.01 * if mirror_x { Vec3::new(-1., 1., 1.) } else { Vec3::ONE };
        h.build_flags = p.build_flags;
        h.centroid = pos + Quat::from_rotation_y(yaw) * (local * scale);
        h
    }

    /// The real placements around [`ANCHOR`]. UE world cm (left handed, Z up) -> game metres: `(x, z, y) * 0.01`, yaw `-yaw` about +Y (`fn_propdata::PropInstance`),
    /// relative to the layout centre, moved to `ANCHOR`; y = the real height on the installed landscape (TERRAIN), else 0; placements near the spawn are dropped.
    pub fn spawn_layout(&self) -> Vec<Harvestable> { self.spawn_layout_on(fn_terrain::world().as_deref()) }
    /// [`Library::spawn_layout`] on an explicit landscape. TERRAIN: with one, every prop stands at its REAL UE z through the landscape's mapping
    /// (`y = (z - ref_z) / 100`, the same mapping as the ground the pawn walks on) and the spawn clearance is measured from the spawn's ground; the clip is
    /// the landscape (minus `fn_game::EDGE_MARGIN`). Without one: the flat plate (`y = 0`, clip +-HALF - 1).
    pub fn spawn_layout_on(&self, terrain: Option<&fn_terrain::World>) -> Vec<Harvestable> {
        let ground = |x: f32, z: f32| terrain.and_then(|w| w.ground_y(x, z)).unwrap_or(0.);
        let spawn = Vec3::new(0., ground(0., SPAWN_Z), SPAWN_Z);
        let chest = spawn + Vec3::Y;
        let (lo, hi) = match terrain { Some(_) => fn_game::world_box(terrain), None => (glam::Vec2::splat(-fn_game::HALF + 1.), glam::Vec2::splat(fn_game::HALF - 1.)) };
        let mut out = Vec::with_capacity(self.placements.len());
        for pl in &self.placements {
            let Some(&pi) = self.by_id.get(&pl.kind) else { continue };
            if self.poi && crate::poi::occupies(&pl.kind, pl.abs_cm) { continue; } // ALL POIS: this placement is a POI actor (crate::poi draws / collides / harvests it)
            let y = terrain.map_or(0., |w| (pl.abs_cm[2] - w.map.ref_z_cm) * 0.01);
            let rel = Vec3::new((pl.abs_cm[0] - self.center_cm[0]) * 0.01, y, (pl.abs_cm[1] - self.center_cm[1]) * 0.01);
            let pos = ANCHOR + rel;
            if (Vec3::new(pos.x - spawn.x, 0., pos.z - spawn.z)).length() < SPAWN_CLEAR_M { continue; }
            let h = self.harvestable_mirrored(pi, pos, pl.game_yaw(), pl.uniform_scale(), pl.mirror_x());
            if h.hull.as_ref().map_or(false, |s| s.sdf(chest) < SPAWN_CLEAR_HULL_M) { continue; }
            // nothing is placed outside the world (the plate +-256 m / the landscape)
            if pos.x < lo.x || pos.x > hi.x || pos.z < lo.y || pos.z > hi.y { continue; }
            out.push(h);
        }
        out
    }

    /// One prop of kind `id` straight ahead of the spawn pawn: centred on the pawn's line (x) and slid along -Z until the hull surface is `gap` metres from the pawn's
    /// chest (or, for props lower than the chest, from its knee), measured along -Z. `None` when the kind is unknown or has no hull there.
    pub fn spawn_one_ahead(&self, id: &str, gap: f32) -> Option<Harvestable> {
        let pi = self.proto_index(id)?;
        let spawn = Vec3::new(0., fn_terrain::ground_y(0., SPAWN_Z), SPAWN_Z); // TERRAIN: on the spawn's ground
        let first = self.harvestable(pi, spawn + Vec3::new(0., 0., -6.), 0., 1.);
        // hull sets are not centred on their origin: put the middle of the hulls on the pawn's line
        let dx = -(first.hull.as_ref()?.center.x - first.pos.x);
        let probe = self.harvestable(pi, first.pos + Vec3::new(dx, 0., 0.), 0., 1.);
        let set = probe.hull.as_ref()?;
        let reach = 14.;
        let ahead = |height: f32| { let o = spawn + Vec3::Y * height; set.trace_sphere(o, o + Vec3::NEG_Z * reach, 0.).map(|f| f * reach) };
        let surface = ahead(fn_combat::CHEST_HEIGHT).or_else(|| ahead(fn_combat::LOW_TRACE_HEIGHT))?;
        Some(self.harvestable(pi, probe.pos + Vec3::new(0., 0., surface - gap), 0., 1.)) // +z brings it toward the pawn
    }

    /// Fills `world` according to `mode`; returns the number of props placed.
    pub fn populate(&self, world: &mut fn_combat::World, mode: Mode) -> usize {
        match mode {
            Mode::Off => return world.props.len(),
            Mode::Empty => world.props.clear(),
            // nothing placeable (no layout / no matching kind): the sandbox placeholders stay rather than an empty world
            Mode::Layout => { let mut v = self.spawn_layout(); v.extend(crate::poi::harvestables()); /* POI: the town's actors (collision + building data) */ if !v.is_empty() { world.props = v; } }
            Mode::One(id) => { let v: Vec<Harvestable> = self.spawn_one_ahead(id, ONE_PROP_GAP_M).into_iter().collect(); if !v.is_empty() { world.props = v; } }
        }
        world.props.len()
    }

    /// One log line per kind actually placed: id, count, hit points, resource.
    pub fn summary(&self, props: &[Harvestable]) -> String {
        let mut count: HashMap<u32, usize> = HashMap::new();
        for p in props { if p.proto != fn_combat::NO_PROTO { *count.entry(p.proto).or_insert(0) += 1; } }
        let mut v: Vec<_> = count.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v.iter().filter_map(|(i, n)| self.proto(*i).map(|p| format!("{n:4} x {} ({} hp, {:?}, rows {:?})", p.id, p.health, p.res, p.resource.amount_row))).collect::<Vec<_>>().join("\n")
    }
}

/// What the scene needs at draw time: which uploaded mesh belongs to which `Proto`.
pub struct Real {
    pub lib: Arc<Library>,
    /// Indexed like `lib.protos`; `None` = that kind failed to load (its props are drawn as the procedural placeholder instead).
    pub meshes: Vec<Option<StaticMeshHandle>>,
    /// Kinds the loader could not decode / upload: (id, reason).
    pub skipped: Vec<(String, String)>,
}
impl Real {
    pub fn mesh_of(&self, h: &Harvestable) -> Option<StaticMeshHandle> { self.meshes.get(h.proto as usize).copied().flatten() }
}

/// VRAM (2026-10-07): with the real-shader path on, the kinds were uploaded with only their custom-depth textures (`fn_render::static_loader::
/// set_prop_textures_depth_only`). Once that path has processed every kind, a PLACED kind it does not draw (its build failed: it stays on the static-mesh
/// path) is decoded and uploaded again in full and its handle replaced, so its fallback draw looks as before. Once per run; true when a handle changed.
pub fn upgrade_depth_only_kinds(renderer: &mut fn_render::Renderer, real: &mut Real, props: &[Harvestable]) -> bool {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.load(std::sync::atomic::Ordering::Relaxed) || !fn_render::static_loader::prop_textures_depth_only() || !crate::statics_render::props_built() { return false; }
    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut changed = false;
    for pi in 0..real.meshes.len() {
        let Some(h) = real.meshes[pi] else { continue };
        if crate::statics_render::handles(h) || !props.iter().any(|p| p.proto as usize == pi && p.alive()) { continue; }
        let id = real.lib.protos[pi].id.clone();
        let Some(kind) = crate::statics_render::catalog_kind(&id) else { continue };
        match fn_render::static_loader::prepare_kind_ext(&kind, &Default::default(), false) {
            Ok(prep) => { let lp = fn_render::static_loader::upload_prepared(renderer, prep, &mut HashMap::new()); real.meshes[pi] = Some(lp.mesh); changed = true; eprintln!("[props] {id}: not on the real-shader path: re-uploaded with all its textures"); }
            Err(e) => eprintln!("[props] {id}: full re-upload failed ({e}): drawn with its custom-depth textures only"),
        }
    }
    changed
}

/// The exported props folder: `<root>/props` of the first asset root holding a `catalog.json` (or `FN_PROPS_DIR`).
pub fn find_dir(roots: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    if let Some(d) = fn_live::var_os("FN_PROPS_DIR").map(std::path::PathBuf::from) { if d.join("catalog.json").is_file() { return Some(d); } }
    roots.iter().map(|r| r.join("props")).find(|d| d.join("catalog.json").is_file())
}

/// Loads the catalog (never panics): `None` + a log line when the folder is missing or unreadable, so the caller keeps the procedural path.
pub fn discover(roots: &[std::path::PathBuf]) -> Option<Arc<PropCatalog>> {
    // LOAD TIME (2026-10-06): one parse per folder for the process. The catalog is read-only data, and it was parsed twice per match (start_loading
    // and df_hook's first frame): 278 JSON files, ~1 s each time in the browser (verification/browser/launch5), less natively.
    static CACHE: std::sync::Mutex<Option<(std::path::PathBuf, Arc<PropCatalog>)>> = std::sync::Mutex::new(None);
    let dir = find_dir(roots)?;
    if let Some(c) = CACHE.lock().ok().and_then(|g| g.as_ref().filter(|(d, _)| *d == dir).map(|(_, c)| c.clone())) { return Some(c); }
    match PropCatalog::load(&dir) {
        Ok(c) => {
            let c = Arc::new(c);
            if let Ok(mut g) = CACHE.lock() { *g = Some((dir, c.clone())); }
            Some(c)
        }
        Err(e) => { eprintln!("[props] {}: {e}; the procedural trees / rocks stay", dir.display()); None }
    }
}

/// The catalog kinds named in `ids`, in catalog order (what the loader decodes + uploads).
pub fn select_kinds<'a>(cat: &'a PropCatalog, ids: &[String]) -> Vec<&'a PropKind> {
    cat.kinds.iter().filter(|k| ids.contains(&k.id)).collect()
}

/// Every kind of the loaded categories.
pub fn loadable_kinds(cat: &PropCatalog) -> Vec<&PropKind> {
    cat.kinds.iter().filter(|k| fn_render::static_loader::DEFAULT_CATEGORIES.contains(&k.category.as_str())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_hull::Hull;

    fn cube(c: Vec3, h: f32) -> Hull { let mut v = vec![]; for x in [-h, h] { for y in [-h, h] { for z in [-h, h] { v.push(c + Vec3::new(x, y, z)); } } } Hull::from_points(&v).unwrap() }

    fn props_dir() -> Option<std::path::PathBuf> {
        let d = fn_live::var_os("FN_PROPS_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().join("FortniteRuntime").join("assets").join("props"));
        d.join("catalog.json").is_file().then_some(d)
    }
    fn real_lib() -> Option<Library> { let d = props_dir()?; Some(Library::from_catalog_poi(&PropCatalog::load(d).ok()?, DEFAULT_RADIUS_M, false)) }

    #[test] fn the_spawn_z_constant_is_the_games_spawn() { assert_eq!(SPAWN_Z, fn_game::spawn().2); }
    /// BUILDING: the +0x564 bits from the exported class defaults: trees / big rocks block (native BuildingProp), every bush (a BuildingContainer)
    /// and hedge is destroyed by a placement, BridgeGirder_Lamp never blocks, a BuildingWall (fence) keeps the SM actor centroid test.
    #[test] fn build_block_flags_come_from_the_class_chain_and_the_cooked_defaults() {
        let Some(lib) = real_lib() else { eprintln!("no exported props: skipped"); return };
        let f = |id: &str| lib.proto(lib.proto_index(id).unwrap()).unwrap().build_flags;
        assert_eq!(f("Athena_Tree_Large_1"), 0);
        assert_eq!(f("Athena_Prop_Rocks_StoneLarge01"), 0);
        assert_eq!(f("Plant_Shrub_09"), 2);
        assert_eq!(f("Hedge_Low_Rectangle"), 2);
        assert_eq!(f("Prop_Rocks_05"), 2);
        let raw = |chain: &str, cdo: serde_json::Value| serde_json::json!({ "class_chain": [chain], "ue_properties": { "class_default_object": cdo } });
        assert_eq!(build_flags_of(&raw("/Script/FortniteGame.BuildingProp", serde_json::json!({ "bDoNotBlockBuildings": true }))).0, 1);
        assert_eq!(build_flags_of(&raw("/Script/FortniteGame.BuildingWall", serde_json::json!({ "CentroidOffset": [0, 0, 96] }))), (4, [0., 0., 96.]));
    }
    #[test] fn a_ue_box_becomes_a_game_space_hull_with_the_axis_swap_and_centimetres() {
        let c = Collision { shapes: vec![Shape::Box { center: [100., 200., 50.], rotation: [0., 0., 0., 1.], half_extent: [10., 20., 30.] }], ..Default::default() };
        let h = hulls_of(&c);
        assert_eq!(h.len(), 1);
        // UE (x, y, z) cm -> game (x, z, y) m: centre (1, 0.5, 2), half extents (0.1, 0.3, 0.2)
        assert!((h[0].center - Vec3::new(1., 0.5, 2.)).length() < 1e-4, "{:?}", h[0].center);
        assert!((h[0].max - h[0].min - Vec3::new(0.2, 0.6, 0.4)).length() < 1e-4);
    }
    #[test] fn convex_transforms_apply_scale_then_rotation_then_translation() {
        let q = std::f32::consts::FRAC_1_SQRT_2; // 90 degrees about UE Z
        let verts: Vec<[f32; 3]> = [[0., 0., 0.], [100., 0., 0.], [0., 100., 0.], [0., 0., 100.]].to_vec();
        let c = Collision { shapes: vec![Shape::Convex { vertices: verts, transform_translation: [500., 0., 0.], transform_rotation: [0., 0., q, q], transform_scale: [2., 2., 2.], bounds_min: [0.; 3], bounds_max: [0.; 3] }], ..Default::default() };
        let h = &hulls_of(&c)[0];
        // vertex (100,0,0) -> scale (200,0,0) -> rotate 90 deg about Z -> (0,200,0) -> +t -> (500,200,0) UE = (5, 0, 2) m
        assert!(h.verts.iter().any(|v| (*v - Vec3::new(5., 0., 2.)).length() < 1e-3), "{:?}", h.verts);
    }
    #[test] fn placing_a_hull_uses_the_game_yaw_scale_and_position() {
        let local = HullSet::new(vec![cube(Vec3::new(2., 1., 0.), 0.5)]);
        let yaw = std::f32::consts::FRAC_PI_2;
        let p = place_hulls(&local, Vec3::new(10., 0., 10.), yaw, 2.);
        // glam rotation_y(90 deg): (x, y, z) -> (z, y, -x): (2,1,0)*2 = (4,2,0) -> (0,2,-4) + (10,0,10)
        assert!((p.center - Vec3::new(10., 2., 6.)).length() < 1e-4, "{:?}", p.center);
        assert!(p.sdf(Vec3::new(10., 2., 6.)) < -0.9, "scaled cube half extent 1.0");
    }
    #[test] fn dead_and_hull_less_real_props_do_not_collide_or_get_hit() {
        let set = Arc::new(HullSet::new(vec![cube(Vec3::new(0., 1., 8.), 1.)]));
        let mut p = Harvestable::real(Kind::Tree, Resource::Wood, Vec3::new(0., 0., 8.), 0., 1., 600., 3, Some(set));
        let hit = fn_combat::trace_cylinders(&[p.clone()], Vec3::new(0., 1.1, 12.), Vec3::NEG_Z, fn_combat::MELEE_REACH, fn_combat::MELEE_RADIUS);
        assert!(hit.is_some(), "hull in reach is hit");
        p.health = 0.;
        assert!(fn_combat::trace_cylinders(&[p.clone()], Vec3::new(0., 1.1, 12.), Vec3::NEG_Z, 3., 0.2).is_none(), "destroyed: gone");
        let empty = Harvestable::real(Kind::Tree, Resource::Wood, Vec3::new(0., 0., 8.), 0., 1., 30., 4, Some(Arc::new(HullSet::default())));
        assert!(fn_combat::trace_cylinders(&[empty.clone()], Vec3::new(0., 1.1, 12.), Vec3::NEG_Z, 9., 0.2).is_none(), "no hull = nothing to hit and nothing to block");
        assert!(fn_props::solids(&empty).is_empty() && fn_props::player_blockers(&[empty], 12.).is_empty());
    }
    #[test] fn real_props_use_their_catalog_health_resource_and_kind() {
        let Some(lib) = real_lib() else { eprintln!("skipped: no exported props"); return };
        let get = |id: &str| lib.proto(lib.proto_index(id).unwrap()).unwrap();
        assert_eq!((get("Athena_Tree_Large_1").health, get("Athena_Tree_Large_1").res, get("Athena_Tree_Large_1").kind), (600., Resource::Wood, Kind::Tree));
        assert_eq!((get("Athena_Prop_Rocks_StoneLarge01").health, get("Athena_Prop_Rocks_StoneLarge01").res, get("Athena_Prop_Rocks_StoneLarge01").kind), (450., Resource::Stone, Kind::Rock));
        assert_eq!(get("Athena_Tree_Medium_01_").health, 200.);
        assert_eq!(get("Athena_Tree_Pine_3").health, 300.);
        assert!(lib.protos.iter().all(|p| p.health > 0.));
        assert!(get("Athena_Tree_Large_1").resource.amount_row.is_some(), "the resource rows are exposed");
    }
    #[test] fn every_real_hull_lies_inside_the_mesh_bounds_so_collision_and_mesh_share_one_frame() {
        let Some(dir) = props_dir() else { eprintln!("skipped: no exported props"); return };
        let cat = PropCatalog::load(&dir).unwrap();
        let mut checked = 0;
        for k in loadable_kinds(&cat) {
            let hulls = hulls_of(&k.collision);
            if hulls.is_empty() { continue; }
            let Ok(lods) = k.load_lods() else { continue };
            let (lo, hi) = lods[0].bounds();
            let (lo, hi) = (Vec3::from(lo), Vec3::from(hi));
            let size = (hi - lo).max_element().max(1.);
            // the collision is a coarse envelope: it may poke out of the visible mesh a little (rocks) but must overlap it, not sit in a mirrored / swapped frame
            let (hmin, hmax) = hulls.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), h| (a.min(h.min), b.max(h.max)));
            let tol = 0.35 * size + 0.5;
            assert!(hmin.x >= lo.x - tol && hmin.y >= lo.y - tol && hmin.z >= lo.z - tol && hmax.x <= hi.x + tol && hmax.y <= hi.y + tol && hmax.z <= hi.z + tol, "{}: hull {hmin:?}..{hmax:?} vs mesh {lo:?}..{hi:?}", k.id);
            checked += 1;
        }
        assert!(checked > 20, "{checked}");
    }
    #[test] fn the_layout_is_centred_on_the_anchor_and_leaves_the_spawn_clear() {
        let Some(lib) = real_lib() else { eprintln!("skipped: no exported props"); return };
        let props = lib.spawn_layout();
        assert!(props.len() > 100, "{} placed ({} in the radius)", props.len(), lib.placement_count());
        let spawn = Vec3::new(0., 1., 12.);
        assert!(props.iter().all(|p| (Vec3::new(p.pos.x, 0., p.pos.z) - Vec3::new(0., 0., 12.)).length() >= SPAWN_CLEAR_M && p.hull.as_ref().unwrap().sdf(spawn) >= SPAWN_CLEAR_HULL_M));
        assert!(props.iter().all(|p| p.pos.y == 0. && p.proto != fn_combat::NO_PROTO && p.max_hp > 0. && !p.regrow && p.alive()));
        // the centre of the sample (the Large oak at relative 0,0) is at the anchor
        assert!(props.iter().any(|p| (p.pos - ANCHOR).length() < 1e-3 && lib.proto(p.proto).unwrap().id == "Athena_Tree_Large_1"));
        // no placement farther than the radius
        assert!(props.iter().all(|p| (p.pos - ANCHOR).length() <= DEFAULT_RADIUS_M + 0.01));
    }
    /// The same walk as `--script props real` (sprint ahead, deflecting around the real hulls), timed per sim step and checked for penetration.
    #[test] fn walking_through_the_real_layout_never_penetrates_a_hull_and_stays_cheap() {
        use fn_game::{FrameInput, GameSim};
        let Some(lib) = real_lib() else { eprintln!("skipped: no exported props"); return };
        let mut g = GameSim::new();
        lib.populate(&mut g.world, Mode::Layout);
        let n = g.world.props.len();
        let (mut worst_gap, mut times, mut min_dist_blocked) = (f32::MAX, vec![], f32::MAX);
        for run in 0..6u32 {
            g.respawn();
            let yaw = [0., 40., -35., 90., 160., 250.][run as usize];
            for f in 0..420u32 {
                let inp = FrameInput { forward: 1., sprint_key: true, set_yaw: Some(yaw + (f as f32 * 0.4).sin() * 30.), jump: f % 150 == 100, ..Default::default() };
                let t = std::time::Instant::now();
                { let held = g.begin_frame(&inp, 1. / 60.); let a = std::time::Instant::now(); g.refresh_blockers(); let rb = a.elapsed().as_secs_f32() * 1000.; let b = std::time::Instant::now(); g.tick(&held, 1. / 60.); let tk = b.elapsed().as_secs_f32() * 1000.; let c = std::time::Instant::now(); let s = g.snapshot(); g.apply_view(&s); g.frames += 1; let av = c.elapsed().as_secs_f32() * 1000.; if rb + tk + av > 5. { eprintln!("  parts: refresh {rb:.2} tick {tk:.2} view {av:.2} hulls near {}", g.controller.hulls.len()); } }
                times.push(t.elapsed().as_secs_f32() * 1000.); if times.last().copied().unwrap_or(0.) > 5. { eprintln!("slow step run {run} frame {f}: {:.2} ms", times.last().unwrap()); }
                // capsule axis (feet + radius .. feet + height - radius) against every live hull
                let feet = g.feet();
                let h = if g.controller.crouched { 0.9 } else { 1.5 };
                let r = fn_game::movement::scene::PLAYER_RADIUS; let gap = (0..=8).map(|k| feet + Vec3::Y * (r + (h - 2. * r) * k as f32 / 8.)).map(|p| g.world.props.iter().filter_map(|pr| pr.hull.as_ref().filter(|_| pr.alive())).map(|s| s.sdf(p)).fold(f32::MAX, f32::min) - r).fold(f32::MAX, f32::min);
                worst_gap = worst_gap.min(gap);
                if g.controller.velocity.length() < 0.05 { min_dist_blocked = min_dist_blocked.min(gap); }
            }
        }
        times.sort_by(|a, b| a.total_cmp(b));
        let mean = times.iter().sum::<f32>() / times.len() as f32;
        eprintln!("{n} props, {} sim steps: mean {mean:.3} ms p95 {:.3} p99 {:.3} max {:.3}; worst capsule gap {worst_gap:.4} m", times.len(), times[times.len() * 95 / 100], times[times.len() * 99 / 100], times[times.len() - 1]);
        assert!(worst_gap > -0.02, "the capsule penetrated a real hull by {} m", -worst_gap);
    }
    /// Holds the attack key in front of one real prop of kind `id` and returns (swings that hit, wood, stone, audio surfaces of the harvest moments, destroyed flags, still alive).
    fn chop(lib: &Library, id: &str, frames: u32) -> (u32, u32, u32, Vec<fn_game::audio_events::HitSurface>, Vec<bool>, bool) {
        use fn_game::{audio_events::{AudioMoment, MomentDetector}, FrameInput, GameSim};
        let mut g = GameSim::new();
        g.world.props = lib.spawn_one_ahead(id, ONE_PROP_GAP_M).into_iter().collect();
        assert_eq!(g.world.props.len(), 1, "{id} has a hull at chest height");
        let mut d = MomentDetector::new();
        let (mut surfaces, mut destroyed) = (vec![], vec![]);
        for _ in 0..frames {
            g.step(&FrameInput { attack: true, ..Default::default() }, 1. / 60.);
            let mut o = vec![]; d.observe(&g, None, 1. / 60., &mut o);
            for m in o { if let AudioMoment::Harvest { surface, destroyed: x, .. } = m { surfaces.push(surface); destroyed.push(x); } }
        }
        (g.world.hits, g.world.wood, g.world.stone, surfaces, destroyed, g.world.props[0].alive())
    }
    #[test] fn chopping_the_real_oak_takes_12_swings_for_600_wood_sounds_like_a_tree_and_the_tree_stays_gone() {
        let Some(lib) = real_lib() else { eprintln!("skipped: no exported props"); return };
        let (hits, wood, stone, surf, destroyed, alive) = chop(&lib, ONE_TREE_ID, 60 * 20);
        assert_eq!((hits, wood, stone), (12, 600, 0), "600 hp / 50 per swing");
        assert!(!alive, "no regrow after 8 s: the real tree is gone for good (20 s of holding the key)");
        assert_eq!(surf.len(), 12); assert!(surf.iter().all(|s| *s == fn_game::audio_events::HitSurface::Tree));
        assert_eq!(destroyed.iter().filter(|d| **d).count(), 1); assert!(destroyed[11]);
    }
    #[test] fn chopping_a_real_rock_takes_9_swings_for_450_stone_and_sounds_like_a_rock() {
        let Some(lib) = real_lib() else { eprintln!("skipped: no exported props"); return };
        let (hits, wood, stone, surf, destroyed, alive) = chop(&lib, "Athena_Prop_Rocks_StoneLarge01", 60 * 12);
        assert_eq!((hits, wood, stone), (9, 0, 450), "450 hp / 50 per swing, stone not wood");
        assert!(!alive); assert_eq!(surf.len(), 9); assert!(surf.iter().all(|s| *s == fn_game::audio_events::HitSurface::Rock), "{surf:?}");
        assert!(destroyed[8] && destroyed.iter().filter(|d| **d).count() == 1);
    }
    #[test] fn every_catalog_health_is_what_a_kind_takes_to_destroy() {
        let Some(lib) = real_lib() else { eprintln!("skipped: no exported props"); return };
        // pure combat arithmetic on the real numbers: a prop of `health` hp dies on swing ceil(health / 50)
        for p in &lib.protos {
            let mut w = fn_combat::World { props: vec![Harvestable::real(p.kind, p.res, Vec3::new(0., 0., 9.), 0., 1., p.health, 0, Some(Arc::new(HullSet::new(vec![cube(Vec3::new(0., 1., 9.), 1.)]))))], ..Default::default() };
            let mut n = 0; while w.props[0].alive() { assert!(w.strike(Vec3::new(0., 0., 12.), Vec3::NEG_Z)); n += 1; assert!(n < 100); }
            assert_eq!(n, (p.health / fn_combat::HARVEST_DAMAGE).ceil() as u32, "{}", p.id);
        }
    }
    #[test] fn the_scene_splits_props_into_real_instances_and_procedural_fallbacks_and_drops_the_destroyed() {
        let mut real = Harvestable::real(Kind::Tree, Resource::Wood, Vec3::new(1., 0., 2.), 0.5, 1.2, 600., 1, Some(Arc::new(HullSet::default())));
        real.shake = 0.1;
        let missing = Harvestable::real(Kind::Rock, Resource::Stone, Vec3::new(3., 0., 4.), 0., 1., 450., 2, None); // proto 2 has no mesh
        let placeholder = Harvestable::new(Kind::Tree, Vec3::new(5., 0., 6.));
        let mut dead = Harvestable::real(Kind::Tree, Resource::Wood, Vec3::ZERO, 0., 1., 600., 1, None); dead.health = 0.;
        let meshes = [None, Some(StaticMeshHandle(7)), None];
        let (statics, procedural) = crate::scene::split_props(&[real, missing, placeholder, dead], |h| meshes.get(h.proto as usize).copied().flatten());
        assert_eq!(statics.len(), 1); let s = statics[0];
        assert_eq!((s.mesh, s.pos, s.yaw, s.scale, s.shake), (StaticMeshHandle(7), Vec3::new(1., 0., 2.), 0.5, 1.2, 0.1));
        assert_eq!(procedural.len(), 2, "the kind without a mesh and the placeholder stay procedural, the destroyed prop is in neither list");
    }
    #[test] fn missing_or_broken_prop_assets_never_panic_and_leave_the_procedural_path() {
        let tmp = std::env::temp_dir().join(format!("fn-app-props-{}", std::process::id())); let _ = std::fs::remove_dir_all(&tmp);
        // no props folder at all
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(find_dir(&[tmp.clone()]).is_none() && discover(&[tmp.clone()]).is_none());
        // a folder with a garbage catalog: found, but discover logs and returns None
        std::fs::create_dir_all(tmp.join("props")).unwrap(); std::fs::write(tmp.join("props").join("catalog.json"), b"{ not json").unwrap();
        assert!(find_dir(&[tmp.clone()]).is_some() && discover(&[tmp.clone()]).is_none());
        // a wrong schema version
        std::fs::write(tmp.join("props").join("catalog.json"), br#"{"schema": 9, "kinds": []}"#).unwrap();
        assert!(discover(&[tmp.clone()]).is_none());
        // an empty catalog loads (zero kinds): an empty library, no layout, nothing placed, no panic
        std::fs::write(tmp.join("props").join("catalog.json"), br#"{"schema": 1, "kinds": []}"#).unwrap();
        let cat = discover(&[tmp.clone()]).unwrap(); let lib = Library::from_catalog(&cat, 100.);
        assert!(lib.protos.is_empty() && lib.spawn_layout().is_empty() && !lib.notes.is_empty());
        let mut w = fn_combat::World::sandbox(); assert_eq!(lib.populate(&mut w, Mode::Layout), 3, "an empty library leaves the three sandbox placeholders in place");
        let _ = std::fs::remove_dir_all(&tmp);
    }
    #[test] fn a_kind_whose_mesh_file_is_missing_is_a_decode_error_not_a_panic() {
        let Some(dir) = props_dir() else { eprintln!("skipped: no exported props"); return };
        let tmp = std::env::temp_dir().join(format!("fn-app-props-bad-{}", std::process::id())); let _ = std::fs::remove_dir_all(&tmp);
        let kind = "Athena_Tree_Pine_3";
        std::fs::create_dir_all(tmp.join(kind)).unwrap();
        for f in std::fs::read_dir(dir.join(kind)).unwrap().flatten() { if f.path().is_file() && f.path().extension().map_or(true, |e| e != "bin") { std::fs::copy(f.path(), tmp.join(kind).join(f.file_name())).unwrap(); } }
        let cat_json = std::fs::read_to_string(dir.join("catalog.json")).unwrap();
        let mut j: serde_json::Value = serde_json::from_str(&cat_json).unwrap();
        j["kinds"] = serde_json::Value::Array(j["kinds"].as_array().unwrap().iter().filter(|k| k["id"] == kind).cloned().collect());
        std::fs::write(tmp.join("catalog.json"), j.to_string()).unwrap();
        let cat = PropCatalog::load(&tmp).unwrap();
        let k = cat.get(kind).unwrap();
        let e = fn_render::static_loader::prepare_kind(k, &Default::default()).err().expect("the .bin is missing");
        assert!(!e.is_empty());
        // the CPU side (hulls, health) does not need the mesh: the kind is still placed and collides, only drawn as the procedural placeholder
        let lib = Library::from_catalog(&cat, 50.);
        assert_eq!((lib.protos.len(), lib.protos[0].health), (1, 300.)); assert!(!lib.protos[0].hull.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
    #[test] fn only_the_kinds_a_run_places_are_loaded() {
        let Some(d) = props_dir() else { eprintln!("skipped: no exported props"); return };
        let cat = PropCatalog::load(&d).unwrap();
        let t = std::time::Instant::now(); let lib = Library::from_catalog_poi(&cat, DEFAULT_RADIUS_M, false); eprintln!("Library::from_catalog: {:.1} ms", t.elapsed().as_secs_f32() * 1000.);
        let all = loadable_kinds(&cat).len(); let layout = lib.kinds_needed(Mode::Layout);
        assert!(layout.len() < all && layout.len() > 10, "{} of {all}", layout.len());
        assert!(lib.spawn_layout().iter().all(|p| layout.contains(&lib.proto(p.proto).unwrap().id)), "every placed prop has its kind loaded");
        assert_eq!(lib.kinds_needed(Mode::One(ONE_ROCK_ID)), vec![ONE_ROCK_ID.to_owned()]);
        assert!(lib.kinds_needed(Mode::One("Nope")).is_empty() && lib.kinds_needed(Mode::Off).is_empty() && lib.kinds_needed(Mode::Empty).is_empty());
        assert_eq!(select_kinds(&cat, &layout).len(), layout.len());
    }
    #[test] fn the_single_test_tree_is_the_requested_distance_from_the_pawn_and_a_swing_reaches_it() {
        let Some(lib) = real_lib() else { eprintln!("skipped: no exported props"); return };
        let t = lib.spawn_one_ahead(ONE_TREE_ID, ONE_PROP_GAP_M).expect("the oak has a hull at chest height");
        let (origin, set) = (Vec3::new(0., fn_combat::CHEST_HEIGHT, 12.), t.hull.as_ref().unwrap());
        let d = set.trace_sphere(origin, origin + Vec3::NEG_Z * 12., 0.).or_else(|| set.trace_sphere(origin - Vec3::Y * 0.75, origin - Vec3::Y * 0.75 + Vec3::NEG_Z * 12., 0.)).unwrap() * 12.;
        assert!((d - ONE_PROP_GAP_M).abs() < 0.01, "surface {d} m from the pawn's chest along -z");
        let hit = fn_combat::trace_cylinders(&[t], origin, Vec3::NEG_Z, fn_combat::MELEE_REACH, fn_combat::MELEE_RADIUS);
        assert!(hit.is_some(), "the pickaxe sweep reaches it");
    }
}
