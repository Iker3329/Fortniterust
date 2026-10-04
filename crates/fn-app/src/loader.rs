//! The in-match loading flow: real load work drives the loading screen's progress bar without ever blocking the UI thread.
//!
//! * **Worker side** (`build_world`, one thread plus scoped helpers): mount the paks (pak mode) -> character + pickaxe models -> every animation clip on a
//!   pool of worker threads -> ground texture + the whole `Renderer` (shader modules, pipelines, targets, sky: ONE call into the driver per phase, on its own
//!   thread so the window keeps animating) -> character textures -> upload the models into the renderer -> the finished `Scene`.
//! * **UI-thread side** (`Loader::poll`, called once per frame with a time budget): the work that needs the `UiLayer` (HUD art, menu brush textures, the
//!   offscreen warm-up render of the HUD / Esc menu / Settings so their pipelines exist before the first real frame) is cut into small resumable slices.
//!   A slice that has to compile a pipeline is the only thing that can run longer than the budget (the loader records the longest slice).
//! * **Progress** is a set of weighted units ([`Progress`]): every unit is announced BEFORE it runs (`begin`), timed (`end`) and counts for its
//!   weight (estimated seconds of work, calibrated from the timings the log prints); like the user's reference loading screen the bar moves only when a step
//!   COMPLETES (a running unit counts for nothing), never goes backwards and only reaches 100 % when everything is done.
use crate::assets::AssetRoots;
use crate::scene::{make_rigid, part_primitives, Character, Scene};
use crate::ui::UiLayer;
use fn_animgraph::ClipLibrary;
use fn_legacy::Md5Clip;
use fn_pose::{CharacterRig, ClipSourceDir, ClipStore, FnClip, Part, SampleMode};
use fn_propdata::PropCatalog;
use fn_render::{MaterialKind, Renderer, RendererAssets, SkyAssets};
use fn_ui::draw::{Cmd, RenderList};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub enum Source { Dir, Paks(PathBuf) }

enum Loaded { Md5(Md5Clip), Fn(Box<FnClip>) }

// ------------------------------------------------------------------------------------------------------------------------------------------
// progress
// ------------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Unit { pub label: String, pub weight: f32, pub started: Option<f32>, pub finished: Option<f32> }
impl Unit { pub fn secs(&self) -> Option<f32> { Some(self.finished? - self.started?) } }

/// Pure progress bookkeeping on an explicit clock (seconds), so it can be tested without sleeping.
#[derive(Clone, Debug, Default)]
pub struct Core { pub units: Vec<Unit>, shown: f32, last_label: String }
/// The fraction of its weight a running unit may contribute before it finishes. 0 = the bar moves only when a step COMPLETES: the user's reference screens are driven
/// by completed load steps (`Progress { total, done }`), never by wall-clock time (the earlier 0.9 interpolation was an invention).
const RUNNING_CAP: f32 = 0.;
/// While anything is unfinished the bar stays below this.
const NOT_DONE_CAP: f32 = 0.995;
impl Core {
    pub fn new(units: Vec<(String, f32)>) -> Core { Core { units: units.into_iter().map(|(label, weight)| Unit { label, weight: weight.max(1e-4), started: None, finished: None }).collect(), shown: 0., last_label: String::new() } }
    pub fn begin(&mut self, i: usize, now: f32) { if let Some(u) = self.units.get_mut(i) { if u.started.is_none() { u.started = Some(now); self.last_label = u.label.clone(); } } }
    pub fn end(&mut self, i: usize, now: f32) { if let Some(u) = self.units.get_mut(i) { if u.started.is_none() { u.started = Some(now); } if u.finished.is_none() { u.finished = Some(now); } } }
    pub fn all_done(&self) -> bool { self.units.iter().all(|u| u.finished.is_some()) }
    pub fn total_weight(&self) -> f32 { self.units.iter().map(|u| u.weight).sum() }
    /// Monotonic fraction 0..1 of the work done at `now`.
    pub fn fraction(&mut self, now: f32) -> f32 {
        let total = self.total_weight().max(1e-6);
        let done: f32 = self.units.iter().map(|u| match (u.started, u.finished) {
            (_, Some(_)) => u.weight,
            (Some(s), None) => u.weight * ((now - s).max(0.) / u.weight).min(RUNNING_CAP),
            _ => 0.,
        }).sum();
        let f = if self.all_done() { 1. } else { (done / total).min(NOT_DONE_CAP) };
        self.shown = self.shown.max(f);
        self.shown
    }
    /// The unit announced last (what the screen says), or `Starting` before anything began.
    pub fn label(&self) -> String { if self.last_label.is_empty() { "Starting".to_owned() } else { self.last_label.clone() } }
    pub fn index_of(&self, label: &str) -> Option<usize> { self.units.iter().position(|u| u.label == label) }
}

/// Thread-safe progress shared between the worker threads and the UI thread.
pub struct Progress { core: Mutex<Core>, t0: Instant }
impl Progress {
    pub fn new(units: Vec<(String, f32)>) -> Arc<Progress> { Arc::new(Progress { core: Mutex::new(Core::new(units)), t0: Instant::now() }) }
    fn now(&self) -> f32 { self.t0.elapsed().as_secs_f32() }
    fn with<R>(&self, f: impl FnOnce(&mut Core, f32) -> R) -> R { let now = self.now(); let mut g = self.core.lock().unwrap_or_else(|e| e.into_inner()); f(&mut g, now) }
    pub fn begin(&self, i: usize) { self.with(|c, t| c.begin(i, t)) }
    pub fn end(&self, i: usize) { self.with(|c, t| c.end(i, t)) }
    pub fn fraction(&self) -> f32 { self.with(|c, t| c.fraction(t)) }
    pub fn label(&self) -> String { self.with(|c, _| c.label()) }
    pub fn all_done(&self) -> bool { self.with(|c, _| c.all_done()) }
    pub fn index_of(&self, label: &str) -> Option<usize> { self.with(|c, _| c.index_of(label)) }
    /// (label, seconds, start offset) of every finished unit, in the order they started.
    pub fn timings(&self) -> Vec<(String, f32, f32)> {
        self.with(|c, _| { let mut v: Vec<_> = c.units.iter().filter_map(|u| Some((u.label.clone(), u.secs()?, u.started?))).collect(); v.sort_by(|a, b| a.2.total_cmp(&b.2)); v })
    }
    pub fn elapsed(&self) -> f32 { self.now() }
}

/// Phases `Renderer::new_with_progress` announces (labels must match fn-render/src/renderer.rs) with their estimated weights in seconds.
pub const RENDERER_PHASES: [(&str, f32); 10] = [
    ("Creating bind group layouts", 0.02), ("Compiling shader modules", 0.15), ("Compiling shaders: ground, character and prop pipelines", 0.55),
    ("Compiling shaders: shadow pipelines", 0.25), ("Compiling shaders: post-processing pipelines", 0.55), ("Creating render buffers and shadow maps", 0.10),
    ("Uploading the ground texture", 0.10), ("Compiling shaders: sky", 0.80), ("Starting the GPU profiler", 0.05), ("Compiling shaders: textured static meshes", 0.30),
];
/// Estimated seconds per animation clip, per pak / tree texture etc. (calibrated from a measured run, see the log).
const W_PAKS: f32 = 0.15;
const W_MODELS: f32 = 0.10;
const W_CLIP: f32 = 0.02;
const W_TEXTURE: f32 = 0.10;
const W_CHAR_TEX: f32 = 0.35;
const W_ASSEMBLE: f32 = 0.30;
const W_HUD: f32 = 0.10;
const W_BRUSHES: f32 = 0.20;
const W_WARM: f32 = 0.45;
/// Estimated seconds to upload one real prop kind (textures + mips + mesh), calibrated from the load log.
const W_PROP: f32 = 0.04;
/// FOLIAGE HOOK: estimated seconds to read the 15.6 MB instance file, chunk it and upload the mesh + texture.
const W_FOLIAGE: f32 = 0.15;

/// Indices of the units in the [`Progress`] (the plan).
#[derive(Clone, Debug)]
pub struct Plan { pub paks: Option<usize>, pub loadout: Option<usize>, pub models: usize, pub clips: Vec<usize>, pub texture: usize, pub renderer: Vec<usize>, pub char_tex: usize, pub assemble: usize, pub hud: usize, pub brushes: usize, pub warm: usize, pub props: Vec<usize>, pub foliage: Option<usize> }

pub fn plan(source: &Source, clip_names: &[String]) -> (Vec<(String, f32)>, Plan) { plan_with_props(source, clip_names, &[]) }

/// The plan with one unit per real prop kind (`prop_ids`, in `props::loadable_kinds` order) between the renderer phases and the character upload: the unit of a kind
/// covers its GPU upload (its decode runs on a pool in parallel with the renderer's shader compilation and is not a unit of its own).
pub fn plan_with_props(source: &Source, clip_names: &[String], prop_ids: &[String]) -> (Vec<(String, f32)>, Plan) { plan_with_foliage(source, clip_names, prop_ids, None) }

/// FOLIAGE HOOK: [`plan_with_props`] plus one unit for the ambient ground cover (`Some(label)`) right after the prop kinds.
pub fn plan_with_foliage(source: &Source, clip_names: &[String], prop_ids: &[String], foliage: Option<String>) -> (Vec<(String, f32)>, Plan) {
    let mut u: Vec<(String, f32)> = vec![];
    let mut add = |label: String, w: f32| { u.push((label, w)); u.len() - 1 };
    let paks = matches!(source, Source::Paks(_)).then(|| add("Mounting the pak files".into(), W_PAKS));
    let loadout = matches!(source, Source::Paks(_)).then(|| add("Loading original starter weapons and animations".into(), 0.5));
    let models = add("Loading character and pickaxe models".into(), W_MODELS);
    let clips = clip_names.iter().map(|n| add(format!("Loading animation: {n}"), W_CLIP)).collect();
    let texture = add("Loading ground texture".into(), W_TEXTURE);
    let renderer = RENDERER_PHASES.iter().map(|(l, w)| add((*l).to_owned(), *w)).collect();
    let props = prop_ids.iter().map(|id| add(format!("Loading prop: {id}"), W_PROP)).collect();
    let foliage = foliage.map(|label| add(label, W_FOLIAGE)); // FOLIAGE HOOK
    let char_tex = add("Loading character textures".into(), W_CHAR_TEX);
    let assemble = add("Uploading character models".into(), W_ASSEMBLE);
    let hud = add("Loading HUD".into(), W_HUD);
    let brushes = add("Loading menu art".into(), W_BRUSHES);
    let warm = add("Preparing the menus".into(), W_WARM);
    (u, Plan { paks, loadout, models, clips, texture, renderer, char_tex, assemble, hud, brushes, warm, props, foliage })
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// worker side
// ------------------------------------------------------------------------------------------------------------------------------------------
/// Number of worker threads decoding clips.
fn workers() -> usize { std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8) }

/// The real props to load: the catalog and the library built from it (`Loader::start` does both: they are small), and the kind ids to decode + upload (only the
/// kinds the run will place: the layout's kinds within the radius, or the single test prop; the 49 kinds of the export take ~4 s to upload, the ~20 near the spawn ~1.5 s).
#[derive(Clone)]
pub struct PropsJob { pub cat: Arc<PropCatalog>, pub lib: Arc<crate::props::Library>, pub kinds: Vec<String> }

struct Job { source: Source, roots: Arc<AssetRoots>, device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat, clip_names: Vec<String>, props: Option<PropsJob> }

fn parts_from(source: &Source, roots: &AssetRoots, paks: Option<&Arc<fn_pak::PakSet>>) -> Result<(CharacterRig, Part), String> {
    match source {
        Source::Dir => {
            let idx = fn_pose::loader::AssetIndex { files: roots.files.clone() };
            let mut parts = fn_pose::loader::old_tree_parts(&idx)?;
            parts.truncate(2); // body, head: the master mesh only donates its IK bones
            let pick = Part::from_gltf_path(idx.find("SK_Pick.gltf")?)?;
            Ok((CharacterRig::new(parts), pick))
        }
        Source::Paks(_) => {
            let set = paks.ok_or("paks not mounted")?;
            let src = ClipSourceDir::Pak(set);
            let mut parts = fn_pose::loader::package_parts(&src)?;
            parts.truncate(2);
            let pkg = fn_pose::loader::package(&src, fn_pose::loader::PICK_PKG)?;
            let mesh = fn_mesh::SkeletalMesh::parse(&pkg).map_err(|e| e.to_string())?;
            Ok((CharacterRig::new(parts), Part::from_skeletal_mesh("SK_Pick", &mesh, 0)?))
        }
    }
}

fn build_world(job: Job, p: Arc<Progress>, plan: Plan) -> Result<Box<Scene>, String> {
    let Job { source, roots, device, queue, format, clip_names, props: props_job } = job;
    // 1. mount (pak mode)
    let paks = match &source {
        Source::Paks(dir) => {
            let i = plan.paks.ok_or("no pak unit")?; p.begin(i);
            let set = Arc::new(fn_pak::PakSet::open_install(dir).map_err(|e| format!("Cannot open the paks in {}: {e}", dir.display()))?);
            p.end(i); Some(set)
        }
        Source::Dir => None,
    };
    let loadout = if let Some(set) = &paks {
        let i = plan.loadout.ok_or("no starter loadout progress unit")?;
        p.begin(i);
        let resources = fn_pose::loadout::Resources::load(&ClipSourceDir::Pak(set))?;
        eprintln!("[loadout] loaded {} original item definitions, {} role bindings, {} montages, {} composites, {} sequences", resources.items.len(), resources.bindings.len(), resources.montages.len(), resources.composites.len(), resources.sequences.len());
        eprintln!("[loadout] recovered {} inherited weapon core groups and {} original blendspaces",resources.core_anims.len(),resources.blendspaces.len());
        eprintln!("[loadout] retained {} original gun material packages with full section/parent references",resources.materials.len());
        eprintln!("[loadout] decoded {} original material parameter textures with sampler/colour metadata",resources.material_textures.len());
        eprintln!("[loadout] parsed {} original material shader variants; pass/vertex-factory selection remains unresolved",resources.materials.values().map(|m|m.shaders.len()).sum::<usize>());
        p.end(i);
        Some(resources)
    } else { None };
    // 2. the clip loader: a closure over the source, shared by the pool
    let clip_job: Arc<dyn Fn(&str) -> Result<Loaded, String> + Send + Sync> = match &source {
        Source::Dir => { let files = Arc::new(roots.files.clone()); Arc::new(move |name: &str| fn_legacy::load_clip(&files, &format!("{name}.md5anim")).map(Loaded::Md5)) }
        Source::Paks(_) => {
            let set = paks.clone().ok_or("paks not mounted")?;
            let skel = Arc::new(fn_pose::clip::load_player_skeleton_info(&ClipSourceDir::Pak(&set))?);
            Arc::new(move |name: &str| {
                let mut store = ClipStore::load(&ClipSourceDir::Pak(&set), &[name.to_owned()], &skel, SampleMode::Ue)?;
                store.clips.remove(name).map(|c| Loaded::Fn(Box::new(c))).ok_or_else(|| format!("{name}: decoded no clip"))
            })
        }
    };
    let queue_idx: Arc<Mutex<VecDeque<usize>>> = Arc::new(Mutex::new((0..clip_names.len()).collect()));
    let (tx, rx) = channel::<(usize, Result<Loaded, String>)>();
    let renderer_units = plan.renderer.clone();

    // real props: the kinds to decode (CPU, on a small pool, bounded so only a few kinds' textures are in memory) and upload once the renderer exists
    let prop_kinds: Vec<&fn_propdata::PropKind> = props_job.as_ref().map(|j| crate::props::select_kinds(&j.cat, &j.kinds)).unwrap_or_default();
    let next_prop = std::sync::atomic::AtomicUsize::new(0);
    let (renderer, rig, pick, body_head, real) = std::thread::scope(|s| -> Result<(Renderer, CharacterRig, Part, (Vec<fn_render::SkinnedPrimitive>, Vec<fn_render::SkinnedPrimitive>, Vec<fn_render::SkinnedPrimitive>), Option<crate::props::Real>), String> {
        let prop_rx = if prop_kinds.is_empty() { None } else {
            let (ptx, prx) = std::sync::mpsc::sync_channel::<(usize, Result<fn_render::static_loader::PreparedProp, String>)>(6);
            for _ in 0..workers().min(4) {
                let (ptx, kinds, next) = (ptx.clone(), &prop_kinds, &next_prop);
                s.spawn(move || loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if i >= kinds.len() { break; }
                    let known = std::collections::HashSet::new();
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fn_render::static_loader::prepare_kind(kinds[i], &known))).unwrap_or_else(|_| Err("panicked while decoding".to_owned()));
                    if ptx.send((i, r)).is_err() { break; }
                });
            }
            Some(prx)
        };
        // clip pool
        for _ in 0..workers() {
            let (queue_idx, tx, job, p, names, units) = (queue_idx.clone(), tx.clone(), clip_job.clone(), p.clone(), &clip_names, &plan.clips);
            s.spawn(move || loop {
                let Some(i) = queue_idx.lock().ok().and_then(|mut q| q.pop_front()) else { break };
                p.begin(units[i]);
                let r = job(&names[i]);
                p.end(units[i]);
                if tx.send((i, r)).is_err() { break; }
            });
        }
        drop(tx);
        // renderer thread: ground texture, then every renderer phase (each announced before it runs)
        let rh = {
            let (p, roots, device, queue, tex_unit) = (p.clone(), roots.clone(), device.clone(), queue.clone(), plan.texture);
            s.spawn(move || -> Result<Renderer, String> {
                p.begin(tex_unit);
                let ground = roots.load_rgba("T_Grasslands_D.png")?;
                p.end(tex_unit);
                let sky_dir = roots.find("dome_optimized.bin").and_then(|f| f.parent()).ok_or("Missing asset: dome_optimized.bin")?;
                let sky = SkyAssets::load(sky_dir)?;
                let mut current: Option<usize> = None;
                let r = Renderer::new_with_progress(&device, &queue, format, &RendererAssets { sky, ground_texture: ground }, &mut |label| {
                    if let Some(c) = current.take() { p.end(c); }
                    if let Some(i) = renderer_units.iter().copied().find(|&i| p.with(|c, _| c.units[i].label == label)) { p.begin(i); current = Some(i); }
                })?;
                if let Some(c) = current.take() { p.end(c); }
                Ok(r)
            })
        };
        // this thread: models, then the characters' textures (disk decode)
        p.begin(plan.models);
        let (rig, pick) = parts_from(&source, &roots, paks.as_ref())?;
        p.end(plan.models);
        p.begin(plan.char_tex);
        // BR CHARACTER HOOK: the real Athena body + head from the packages replace the current body / head (same master pose, same two slots); None = keep the current ones
        let br = crate::br_character::load_for(&source, paks.as_ref());
        let (rig, body, head) = match br {
            Some(m) => (fn_pose::CharacterRig { parts: m.rig_parts, ..rig }, m.body, m.head),
            None => { let b = part_primitives(&rig.parts[0], &roots, MaterialKind::Character); let h = part_primitives(&rig.parts[1], &roots, MaterialKind::Character); (rig, b, h) }
        };
        let mut pickp = part_primitives(&pick, &roots, MaterialKind::Prop);
        make_rigid(&mut pickp);
        p.end(plan.char_tex);
        let lib = props_job.as_ref().map(|j| j.lib.clone());
        let mut renderer = rh.join().map_err(|_| "the renderer thread panicked".to_owned())??;
        // upload the decoded kinds (one progress unit each); a kind that fails stays a procedural placeholder, nothing here can fail the load
        let real = match (lib, prop_rx) {
            (Some(lib), Some(prx)) => {
                let mut cache = HashMap::new();
                let mut meshes: Vec<Option<fn_render::StaticMeshHandle>> = vec![None; lib.protos.len()];
                let mut skipped = vec![];
                for _ in 0..prop_kinds.len() {
                    let Ok((i, r)) = prx.recv() else { break };
                    let unit = plan.props.get(i).copied();
                    if let Some(u) = unit { p.begin(u); }
                    match r {
                        Ok(prep) => { let id = prep.id.clone(); let lp = fn_render::static_loader::upload_prepared(&mut renderer, prep, &mut cache); if let Some(pi) = lib.proto_index(&id) { meshes[pi as usize] = Some(lp.mesh); } }
                        Err(e) => skipped.push((prop_kinds[i].id.clone(), e)),
                    }
                    if let Some(u) = unit { p.end(u); }
                }
                for &u in &plan.props { p.end(u); }
                Some(crate::props::Real { lib, meshes, skipped })
            }
            _ => { for &u in &plan.props { p.end(u); } None }
        };
        Ok((renderer, rig, pick, (body, head, pickp), real))
    })?;
    let _ = pick;
    // FOLIAGE HOOK: the ambient ground cover (one progress unit), uploaded through the same static-mesh path as the props; a failure only logs and leaves the world without grass
    let mut renderer = renderer;
    let grass = match (plan.foliage, props_job.as_ref()) {
        (Some(u), Some(j)) => { p.begin(u); let g = crate::foliage::load(&mut renderer, &j.cat.dir, &j.lib); p.end(u); g }
        _ => None,
    };
    // 3. collect the clips (all workers are done once the scope ended: the channel holds every result)
    let total = clip_names.len();
    let mut md5: HashMap<String, Md5Clip> = HashMap::new();
    let mut fnclips = ClipStore::new();
    let mut got = 0;
    while got < total {
        match rx.recv() {
            Ok((i, r)) => {
                let name = &clip_names[i];
                match r.map_err(|e| format!("Cannot load animation {name}: {e}"))? { Loaded::Md5(c) => { md5.insert(name.clone(), c); } Loaded::Fn(c) => { fnclips.insert(name, *c); } }
                got += 1;
            }
            Err(_) => return Err("the animation workers stopped unexpectedly".into()),
        }
    }
    if let Some(resources)=&loadout {
        let added=resources.install_player_clips(&mut fnclips);
        eprintln!("[loadout] installed {added} additional original player clips; weapon-skeleton sequences remain separate");
    }
    // 4. upload the models
    p.begin(plan.assemble);
    let (body, head, pickp) = body_head;
    let clips: Box<dyn ClipLibrary + Send> = match source { Source::Dir => Box::new(md5), Source::Paks(_) => Box::new(fnclips) };
    let mut scene = Scene::new(renderer, Character::new(rig, clips), &body, &head, &pickp);
    scene.character.part_anim = crate::part_anim_glue::take(); // PART ANIM HOOK
    scene.real = real;
    scene.grass = grass; // FOLIAGE HOOK
    if let Some(resources)=&loadout {
        let hand=fn_pose::SocketDef::from_original_yaw_socket(0,&resources.player_hand_socket)?;
        if scene.character.rig.parts[0].tree.bone_index(&hand.bone).is_none(){return Err("original hand socket bone missing from character".into());}
        // Pak mode uses the actual serialized socket, not the legacy default.
        // Scene evaluates this socket twice: once on the displayed post-process
        // pose and once immediately before CopyBone / hand IK for the weapon.
        scene.character.rig.sockets=vec![hand];
        eprintln!("[loadout] installed original player RightHand socket for final-pose weapon attachment");
    }
    scene.loadout = loadout;
    p.end(plan.assemble);
    Ok(Box::new(scene))
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// UI-thread side
// ------------------------------------------------------------------------------------------------------------------------------------------
/// Command ranges of at most `max` commands that never split a clip scope (so each range renders on its own).
pub fn slice_ranges(list: &RenderList, max: usize) -> Vec<(usize, usize)> {
    let (mut out, mut start, mut depth) = (vec![], 0usize, 0i32);
    for (i, c) in list.cmds.iter().enumerate() {
        match c { Cmd::PushClip(_) => depth += 1, Cmd::PopClip => depth -= 1, _ => {} }
        if depth <= 0 && i + 1 - start >= max { out.push((start, i + 1)); start = i + 1; }
    }
    if start < list.cmds.len() { out.push((start, list.cmds.len())); }
    out
}

/// Commands per warm-up render slice: small enough that a slice which has to build a pipeline stays short.
const WARM_SLICE_CMDS: usize = 24;
const WARM_PASSES: usize = 3;
const WARM_SIZE: (u32, u32) = (1280, 720);
/// Brush textures decoded per poll at most (also bounded by the time budget).
const BRUSH_CHUNK: usize = 6;

enum Ui {
    Hud,
    Brushes { files: Vec<String>, i: usize },
    WarmBuildHud { pass: usize },
    WarmBuildMenu { pass: usize },
    WarmRender { pass: usize, ranges: Vec<(usize, usize)>, at: usize },
    WarmFinish,
    Done,
}

pub enum State { Running, Done(Box<Scene>) }

pub struct Loader {
    progress: Arc<Progress>,
    plan: Plan,
    result: Receiver<Result<Box<Scene>, String>>,
    scene: Option<Box<Scene>>,
    ui: Ui,
    /// The next UI-thread unit was announced (its label has been drawn once) but has not run yet.
    announced: bool,
    target: Option<fn_render::OffscreenTarget>,
    /// The warm-up pass being built / rendered (the UI's own list is rebuilt every frame by the screen that is showing, so the loader keeps its own).
    warm_list: RenderList,
    host: fn_ui::menu::pages::NullHost,
    pub started: Instant,
    /// Longest single UI-thread slice (ms) and the number of slices over 50 ms.
    pub max_slice_ms: f32,
    pub slices_over_50ms: u32,
    /// UI-thread operations that took more than 15 ms: (what, ms).
    pub slow_ops: Vec<(String, f32)>,
    device: wgpu::Device,
    format: wgpu::TextureFormat,
}

impl Loader {
    /// Spawns the worker side. `device` / `queue` are clones of the window's (the renderer is built on a worker thread).
    pub fn start(source: Source, roots: Arc<AssetRoots>, device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat, props: Option<PropsJob>) -> Loader {
        let clip_names = fn_legacy::prototype_clip_names();
        let prop_ids: Vec<String> = props.as_ref().map(|j| crate::props::select_kinds(&j.cat, &j.kinds).iter().map(|k| k.id.clone()).collect()).unwrap_or_default();
        // FOLIAGE HOOK: one extra unit when the ground cover export exists next to the props
        let foliage_label = props.as_ref().and_then(|j| crate::foliage::Spec::read(&j.cat.dir).ok()).map(|s| format!("Loading ground cover: {}", s.name));
        let (units, plan_) = plan_with_foliage(&source, &clip_names, &prop_ids, foliage_label);
        let progress = Progress::new(units);
        let (tx, rx) = channel();
        let job = Job { source, roots, device: device.clone(), queue, format, clip_names, props };
        let (p2, plan2) = (progress.clone(), plan_.clone());
        std::thread::Builder::new().name("load-world".into()).spawn(move || { let r = build_world(job, p2, plan2); let _ = tx.send(r); }).ok();
        Loader { progress, plan: plan_, result: rx, scene: None, ui: Ui::Hud, announced: false, target: None, warm_list: RenderList::new(), host: fn_ui::menu::pages::NullHost::new(), started: Instant::now(), max_slice_ms: 0., slices_over_50ms: 0, slow_ops: vec![], device, format }
    }
    pub fn fraction(&self) -> f32 { self.progress.fraction() }
    pub fn label(&self) -> String { self.progress.label() }
    pub fn progress(&self) -> &Arc<Progress> { &self.progress }
    pub fn finished(&self) -> bool { self.scene.is_some() && matches!(self.ui, Ui::Done) }
    /// "seconds  label  (start offset)" per finished unit, the longest UI-thread slice and the total.
    pub fn timings_text(&self) -> String {
        let mut s: String = self.progress.timings().iter().map(|(l, secs, start)| format!("{secs:8.3} s  {l}  (started at {start:.3} s)\n")).collect();
        s.push_str(&format!("{:8.3} s  total   longest UI-thread slice {:.1} ms, slices over 50 ms: {}\n", self.started.elapsed().as_secs_f32(), self.max_slice_ms, self.slices_over_50ms));
        for (what, ms) in &self.slow_ops { s.push_str(&format!("          UI-thread op over 15 ms: {what}: {ms:.1} ms\n")); }
        s
    }

    /// One frame's worth of UI-thread work (<= `budget` plus at most one slice) and the poll of the worker. `Done` once everything finished.
    pub fn poll(&mut self, ui: &mut UiLayer, budget: Duration) -> Result<State, String> {
        match self.result.try_recv() {
            Ok(Ok(scene)) => self.scene = Some(scene),
            Ok(Err(e)) => return Err(e),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => if self.scene.is_none() { return Err("the loading thread stopped unexpectedly".into()) },
        }
        let t0 = Instant::now();
        while !matches!(self.ui, Ui::Done) {
            // announce a unit's first operation one frame before it runs, so the screen already says what is being done
            let unit = match &self.ui { Ui::Hud => Some(self.plan.hud), Ui::Brushes { i: 0, .. } => Some(self.plan.brushes), Ui::WarmBuildHud { pass: 0 } => Some(self.plan.warm), _ => None };
            if let Some(u) = unit { if !self.announced { self.progress.begin(u); self.announced = true; return Ok(self.state_now()); } }
            self.announced = false;
            let slice = Instant::now();
            let what = self.ui_op_name();
            self.step_ui(ui);
            let ms = slice.elapsed().as_secs_f32() * 1000.;
            if ms > 15. { self.slow_ops.push((what, ms)); }
            self.max_slice_ms = self.max_slice_ms.max(ms);
            if ms > 50. { self.slices_over_50ms += 1; }
            if t0.elapsed() >= budget { break; }
        }
        Ok(self.state_now())
    }
    fn ui_op_name(&self) -> String {
        match &self.ui { Ui::Hud => "HUD scratch present".into(), Ui::Brushes { i, files } => format!("menu brushes {i}..{}", (*i + BRUSH_CHUNK).min(files.len())), Ui::WarmBuildHud { pass } => format!("warm pass {pass}: build HUD"), Ui::WarmBuildMenu { pass } => format!("warm pass {pass}: build menu"),
            Ui::WarmRender { pass, ranges, at } => format!("warm pass {pass}: render slice {at}/{}", ranges.len()), Ui::WarmFinish => "warm finish".into(), Ui::Done => "done".into() }
    }
    fn state_now(&mut self) -> State {
        if self.scene.is_some() && matches!(self.ui, Ui::Done) { State::Done(self.scene.take().expect("checked")) } else { State::Running }
    }

    fn step_ui(&mut self, ui: &mut UiLayer) {
        let (w, h) = (WARM_SIZE.0 as f32, WARM_SIZE.1 as f32);
        let ui_state = std::mem::replace(&mut self.ui, Ui::Done);
        self.ui = match ui_state {
            Ui::Hud => {
                // build a HUD at the reference size into a scratch list: loads the HUD art and builds its glyph atlases
                let mut scratch = RenderList::new();
                ui.hud.set_state(&fn_ui::screens::hud::HudState::default());
                ui.hud.present(&mut ui.ctx, 1920., 1080., &mut scratch);
                self.progress.end(self.plan.hud);
                Ui::Brushes { files: fn_ui::menu::lower::brush_files().into_iter().collect(), i: 0 }
            }
            Ui::Brushes { files, mut i } => {
                let end = (i + BRUSH_CHUNK).min(files.len());
                while i < end { ui.ctx.assets.texture(&files[i]); i += 1; }
                if i >= files.len() { self.progress.end(self.plan.brushes); Ui::WarmBuildHud { pass: 0 } } else { Ui::Brushes { files, i } }
            }
            Ui::WarmBuildHud { pass } => {
                if pass == 0 {
                    self.target = Some(fn_render::OffscreenTarget::new(&self.device, WARM_SIZE.0, WARM_SIZE.1, self.format));
                    ui.menu.open();
                    ui.menu.push(Box::new(fn_ui::menu::pause::SettingsScreen::new()));
                }
                self.warm_list.clear();
                ui.hud.set_state(&fn_ui::screens::hud::HudState::default());
                ui.hud.present(&mut ui.ctx, w, h, &mut self.warm_list);
                Ui::WarmBuildMenu { pass }
            }
            Ui::WarmBuildMenu { pass } => {
                let draw = ui.menu.frame_with(&mut self.host.cx(), w, h, 0.016);
                fn_ui::menu::lower::lower(draw, &mut ui.ctx, w, h, &mut self.warm_list);
                let ranges = slice_ranges(&self.warm_list, WARM_SLICE_CMDS);
                Ui::WarmRender { pass, ranges, at: 0 }
            }
            Ui::WarmRender { pass, ranges, at } => {
                if let (Some(&(a, b)), Some(target)) = (ranges.get(at), self.target.as_ref()) {
                    let sub = RenderList { cmds: self.warm_list.cmds[a..b].to_vec() };
                    ui.renderer.render(&sub, &mut ui.ctx.assets, &target.view, WARM_SIZE.0, WARM_SIZE.1, if at == 0 { Some([0., 0., 0., 1.]) } else { None });
                }
                if at + 1 < ranges.len() { Ui::WarmRender { pass, ranges, at: at + 1 } }
                else if pass + 1 < WARM_PASSES { Ui::WarmBuildHud { pass: pass + 1 } } else { Ui::WarmFinish }
            }
            Ui::WarmFinish => {
                ui.menu.close();
                ui.menu.set_visibility(0.);
                let _ = ui.menu.take_requests();
                ui.begin();
                self.warm_list = RenderList::new();
                self.target = None;
                self.progress.end(self.plan.warm);
                Ui::Done
            }
            Ui::Done => Ui::Done,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn core() -> Core { Core::new(vec![("a".into(), 1.), ("b".into(), 2.), ("c".into(), 1.)]) }
    #[test] fn fraction_is_proportional_to_the_weights_of_finished_units() {
        let mut c = core();
        c.begin(0, 0.); c.end(0, 1.); assert!((c.fraction(1.) - 0.25).abs() < 1e-6);
        c.begin(1, 1.); c.end(1, 3.); assert!((c.fraction(3.) - 0.75).abs() < 1e-6);
        c.begin(2, 3.); c.end(2, 4.); assert_eq!(c.fraction(4.), 1.);
    }
    #[test] fn a_running_unit_counts_for_nothing_until_it_completes() {
        let mut c = core(); c.begin(1, 0.);
        assert_eq!(c.fraction(0.5), 0.); assert_eq!(c.fraction(1000.), 0., "progress is step-driven, never wall-clock driven (reference behaviour)");
        c.end(1, 1000.); assert!((c.fraction(1000.) - 0.5).abs() < 1e-6);
    }
    #[test] fn the_bar_never_goes_backwards_and_never_shows_100_before_everything_is_done() {
        let mut c = core(); let mut last = 0.;
        for (i, t) in [(0, 0.), (1, 0.1), (2, 0.2)] { c.begin(i, t); }
        for k in 0..50 { let f = c.fraction(k as f32 * 0.05); assert!(f >= last); assert!(f < 1.); last = f; }
        c.end(0, 3.); c.end(1, 3.); assert!(c.fraction(3.) < 1.); c.end(2, 3.); assert_eq!(c.fraction(3.), 1.);
    }
    #[test] fn the_label_is_the_unit_announced_last_and_starting_before_that() {
        let mut c = core(); assert_eq!(c.label(), "Starting"); c.begin(0, 0.); assert_eq!(c.label(), "a"); c.begin(2, 0.1); assert_eq!(c.label(), "c");
        c.begin(2, 0.2); assert_eq!(c.label(), "c", "beginning twice changes nothing"); c.end(2, 1.); assert_eq!(c.label(), "c");
    }
    #[test] fn timings_are_recorded_for_every_finished_unit_including_instant_ones() {
        let mut c = core(); c.begin(0, 0.5); c.end(0, 0.75); c.end(1, 2.); assert_eq!(c.units[0].secs(), Some(0.25)); assert_eq!(c.units[1].secs(), Some(0.), "a unit ended without begin is zero seconds, not missing");
        assert!(c.units[2].secs().is_none());
    }
    #[test] fn the_plan_has_one_unit_per_clip_and_per_renderer_phase_and_no_fake_steps() {
        let names: Vec<String> = (0..7).map(|i| format!("clip{i}")).collect();
        let (units, p) = plan(&Source::Dir, &names);
        assert_eq!(p.clips.len(), 7); assert_eq!(p.renderer.len(), RENDERER_PHASES.len()); assert!(p.paks.is_none());
        let (units2, p2) = plan(&Source::Paks(PathBuf::from("x")), &names); assert!(p2.paks.is_some() && p2.loadout.is_some() && units2.len() == units.len() + 2);
        let mut labels: Vec<&String> = units.iter().map(|u| &u.0).collect(); labels.sort(); let n = labels.len(); labels.dedup(); assert_eq!(labels.len(), n, "labels are unique (the renderer maps announced labels to units)");
        assert!(units.iter().all(|u| u.1 > 0.));
        // the "Compiling shaders" work is no longer one 3 s unit that follows a 98 % bar
        let renderer_w: f32 = p.renderer.iter().map(|&i| units[i].1).sum(); let total: f32 = units.iter().map(|u| u.1).sum();
        assert!(units[p.renderer[2]].1 < 0.2 * total && renderer_w < 0.7 * total + 1.0);
    }
    #[test] fn renderer_phase_labels_match_the_ones_fn_render_announces() {
        let src = include_str!("../../fn-render/src/renderer.rs");
        for (label, _) in RENDERER_PHASES { assert!(src.contains(&format!("progress(\"{label}\")")), "{label} is not announced by Renderer::new_with_progress"); }
        assert_eq!(src.matches("progress(\"").count(), RENDERER_PHASES.len(), "every announced phase has a unit");
    }
    #[test] fn slices_split_at_the_limit_never_inside_a_clip_scope_and_cover_every_command() {
        let mut l = RenderList::new(); let r = fn_ui::layout::Rect::new(0., 0., 4., 4.);
        for _ in 0..5 { l.fill(r, [1.; 4]); } l.push_clip(r); for _ in 0..7 { l.fill(r, [1.; 4]); } l.pop_clip(); for _ in 0..3 { l.fill(r, [1.; 4]); }
        let s = slice_ranges(&l, 4);
        assert_eq!(s.first().unwrap().0, 0); assert_eq!(s.last().unwrap().1, l.cmds.len());
        for w in s.windows(2) { assert_eq!(w[0].1, w[1].0); }
        for &(a, b) in &s { let d = l.cmds[a..b].iter().fold(0i32, |d, c| d + match c { Cmd::PushClip(_) => 1, Cmd::PopClip => -1, _ => 0 }); assert_eq!(d, 0, "slice {a}..{b} is balanced"); }
        assert!(slice_ranges(&RenderList::new(), 4).is_empty());
    }
    #[test] fn progress_wrapper_is_thread_safe_and_reports_timings_in_start_order() {
        let p = Progress::new(vec![("x".into(), 1.), ("y".into(), 1.)]);
        let p2 = p.clone(); std::thread::spawn(move || { p2.begin(1); std::thread::sleep(Duration::from_millis(5)); p2.end(1); }).join().unwrap();
        p.begin(0); p.end(0);
        let t = p.timings(); assert_eq!(t.len(), 2); assert!(t[0].2 <= t[1].2); assert!(p.all_done() && p.fraction() == 1.);
        assert!(t.iter().find(|x| x.0 == "y").unwrap().1 >= 0.004, "{t:?}");
    }
}

