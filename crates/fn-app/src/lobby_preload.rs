//! LOBBY PRELOAD: everything the Battle Royale lobby needs is loaded behind the 1.8 start-up loading screen (the "EARLY ACCESS" one,
//! `fn_ui::screens::startup_loading`) so that nothing loads lazily or hitches once the lobby is shown.
//!
//! The work is split by what may run where:
//!
//! | unit | where | what |
//! |---|---|---|
//! | `game files, catalog, profile` | worker | `PakSet`, `fn_pose::cosmetics::Catalog` (every Athena item definition), the offline profile |
//! | `lobby scene assets` | worker | `LobbyAssets::load`: static meshes, materials, textures, sky SH, LUT, the equipped hero and every AnimBP clip |
//! | `skin meshes + materials` | worker pool, then GPU upload | `load_hero_parts` of every outfit (equipped first) within [`hero_budget_bytes`], then `HeroBuilder::build` |
//! | `lobby UI widgets` | main (the UMG store is `Rc`) | `LobbyUi::load` |
//! | `UI renderer` | main | `UiRenderer` (quad pipeline) |
//! | `lobby scene GPU` | main | `LobbyScene::from_assets`: buffers, textures, every material pipeline, shadow maps, lights |
//! | `Locker, tabs, item icons` | main, sliced | warm-up draws through the real `LobbyUi::draw` + `UiRenderer::render` into an off-screen target: every tab page is composed, every hover / press state, every Locker picker page with ALL catalog items of the three slots (icons decoded, cards baked, textures uploaded, material pipelines built) |
//! | `scene warm-up` | main | two real `LobbyScene::render` frames at the window size (render targets, first-use paths) |
//! | `audio` | audio worker | `DeviceSink::open_streaming`: front-end music, then the UI cues; the loading-screen mix (`Fort_Loading_Mix`) is on until the hand-off |
//! | extra tasks | main | [`Preloader::add_task`]: the hook for the shop / console (see the module docs of `lobby_shop` / `lobby_console`) |
//!
//! PROGRESS is the fraction of COMPLETED units by weight (the loading screen's `Progress` getter of the exe: sum of finished job weights / sum of all weights); the bar moves
//! only when a unit completes. The weights are the measured seconds of each unit on the reference machine (INFERENCE: they only shape the bar, not the behaviour).
//!
//! The main thread never blocks for long in a slice (`Preloader::frame` gives each frame a 25 ms budget) except inside the three calls that are one driver call each
//! (`LobbyUi::load`, `LobbyScene::from_assets`, a pipeline compile): the throbber pauses there.
//!
//! LOADS AFTER THE HAND-OFF: [`Counters`] (pak entry reads, UI texture store size, unresolved UI assets, UI renderer texture cache and baked-material cache) are sampled
//! at the hand-off ([`Products::counters`]) and can be sampled again later with [`counters`]; a lobby whose first frames add nothing has preloaded everything.
#![allow(dead_code)]
use crate::lobby_locker::LockerGlue;
use crate::lobby_profile::Profile;
use crate::lobby_scene::{HeroBuilder, HeroParts, HeroReady, LobbyAssets, LobbyScene};
use crate::lobby_ui::{Controller, LobbyState, LobbyUi, LockerSlot, Role};
use fn_audio_sink::{AudioSink, DeviceSink, LoadLog, MusicCue, SoundEvent};
use fn_pose::cosmetics::Category;
use fn_ui::draw::RenderList;
use fn_ui::scene::UiContext;
use fn_ui::screens::startup_layout::FADE_SECONDS;
use fn_ui::screens::startup_loading::{self, StartupLoadingScreen};
use fn_ui_wgpu::UiRenderer;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Time one `Preloader::frame` may spend on main-thread work.
pub const FRAME_BUDGET: Duration = Duration::from_millis(25);

/// Memory the pre-built skins may take (CPU copy of the decoded character textures, estimated from the RGBA buffers). `FN_LOBBY_HERO_BUDGET_MB` overrides.
pub fn hero_budget_bytes() -> usize {
    std::env::var("FN_LOBBY_HERO_BUDGET_MB").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(768) * 1024 * 1024
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// progress
// ------------------------------------------------------------------------------------------------------------------------------------------
pub const U_CPU: usize = 0;
pub const U_SCENE_ASSETS: usize = 1;
pub const U_HEROES: usize = 2;
pub const U_UI: usize = 3;
pub const U_RENDERER: usize = 4;
pub const U_SCENE_GPU: usize = 5;
pub const U_UI_WARM: usize = 6;
pub const U_SCENE_WARM: usize = 7;
pub const U_AUDIO: usize = 8;
const BASE_UNITS: [(&str, f32); 9] = [
    ("game files, cosmetic catalog, profile", 0.50),
    ("lobby scene assets (meshes, textures, materials, hero, AnimBP clips)", 3.00),
    ("skin meshes: background after the hand-off", 0.0),
    ("lobby UI widgets", 0.60),
    ("UI renderer", 0.05),
    ("lobby scene GPU (pipelines, shadow maps, lights)", 0.80),
    ("Locker, tabs, item icons (warm-up draws)", 3.00),
    ("lobby scene warm-up frames", 0.30),
    ("audio (music + UI sounds)", 0.60),
];

#[derive(Clone, Debug)]
pub struct Unit { pub label: String, pub weight: f32, pub started: Option<f32>, pub finished: Option<f32> }
pub struct Progress { units: Mutex<Vec<Unit>>, t0: Instant }
impl Progress {
    pub fn new(units: Vec<(String, f32)>) -> Arc<Progress> {
        Arc::new(Progress { units: Mutex::new(units.into_iter().map(|(label, weight)| Unit { label, weight: weight.max(1e-4), started: None, finished: None }).collect()), t0: Instant::now() })
    }
    fn with<R>(&self, f: impl FnOnce(&mut Vec<Unit>, f32) -> R) -> R { let now = self.t0.elapsed().as_secs_f32(); f(&mut self.units.lock().unwrap_or_else(|e| e.into_inner()), now) }
    pub fn begin(&self, i: usize) { self.with(|u, t| if let Some(x) = u.get_mut(i) { x.started.get_or_insert(t); }) }
    pub fn end(&self, i: usize) { self.with(|u, t| if let Some(x) = u.get_mut(i) { x.started.get_or_insert(t); x.finished.get_or_insert(t); }) }
    pub fn done(&self, i: usize) -> bool { self.with(|u, _| u.get(i).map_or(true, |x| x.finished.is_some())) }
    pub fn all_done(&self) -> bool { self.with(|u, _| u.iter().all(|x| x.finished.is_some())) }
    /// Completed weight / total weight (a running unit counts for nothing).
    pub fn fraction(&self) -> f32 { self.with(|u, _| { let total: f32 = u.iter().map(|x| x.weight).sum(); let done: f32 = u.iter().filter(|x| x.finished.is_some()).map(|x| x.weight).sum(); if u.iter().all(|x| x.finished.is_some()) { 1. } else { (done / total.max(1e-6)).min(0.999) } }) }
    pub fn add(&self, label: &str, weight: f32) -> usize { self.with(|u, _| { u.push(Unit { label: label.to_owned(), weight, started: None, finished: None }); u.len() - 1 }) }
    pub fn units(&self) -> Vec<Unit> { self.with(|u, _| u.clone()) }
    pub fn label(&self) -> String { self.with(|u, _| u.iter().filter(|x| x.started.is_some() && x.finished.is_none()).map(|x| x.label.clone()).next().unwrap_or_default()) }
    pub fn elapsed(&self) -> f32 { self.t0.elapsed().as_secs_f32() }
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// counters (loads after the hand-off)
// ------------------------------------------------------------------------------------------------------------------------------------------
/// Everything whose growth after the hand-off means "something loaded lazily".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// `fn_pak::read_count()`: pak entries read by the whole process
    pub pak_reads: u64,
    /// textures in the UI store (decoded art, baked cards / materials)
    pub ui_textures: usize,
    /// UI assets that could not be resolved
    pub ui_missing: usize,
    /// textures resident in the UI renderer
    pub renderer_textures: usize,
}
pub fn counters(ui: &mut LobbyUi, renderer: &UiRenderer) -> Counters {
    let a = &ui.ctx_mut().assets;
    Counters { pak_reads: fn_pak::read_count(), ui_textures: a.store.len(), ui_missing: a.missing.len(), renderer_textures: renderer.cached_texture_count() }
}
impl Counters {
    /// Human readable growth since `before` (empty when nothing grew).
    pub fn grew_since(&self, before: &Counters) -> Vec<String> {
        let mut v = vec![];
        if self.pak_reads > before.pak_reads { v.push(format!("pak reads +{}", self.pak_reads - before.pak_reads)); }
        if self.ui_textures > before.ui_textures { v.push(format!("UI textures +{}", self.ui_textures - before.ui_textures)); }
        if self.ui_missing > before.ui_missing { v.push(format!("UI unresolved +{}", self.ui_missing - before.ui_missing)); }
        if self.renderer_textures > before.renderer_textures { v.push(format!("renderer textures +{}", self.renderer_textures - before.renderer_textures)); }
        v
    }
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// skin cache
// ------------------------------------------------------------------------------------------------------------------------------------------
/// Every outfit that was pre-built, GPU-ready. `take` removes one (the scene owns it while it is on the pad); `put` returns the hero that left the pad.
#[derive(Default)]
pub struct HeroCache { store: Option<Arc<Mutex<HeroStore>>>, ready: HashMap<String, HeroReady>, pub bytes: usize, pub total: usize, pub skipped_over_budget: usize, pub failed: Vec<(String, String)> }
impl HeroCache {
    /// Collects the skins the background builder finished (cheap: moves handles, no GPU work). Call once per frame.
    pub fn pump(&mut self) {
        let Some(st) = self.store.as_ref() else { return };
        let Ok(mut g) = st.try_lock() else { return };
        for (k, v) in g.ready.drain() { self.ready.insert(k, v); }
        self.bytes = g.bytes; self.total = g.total; self.skipped_over_budget = g.skipped; self.failed = g.failed.clone();
    }
    pub fn take(&mut self, hero: &str) -> Option<HeroReady> { self.ready.remove(hero) }
    pub fn put(&mut self, r: HeroReady) { self.ready.insert(r.hero().to_owned(), r); }
    pub fn contains(&self, hero: &str) -> bool { self.ready.contains_key(hero) }
    pub fn len(&self) -> usize { self.ready.len() }
    /// Swaps `hero` onto the pad of `scene` from the cache; the hero that was there goes into the cache. `false` = not cached (load it the slow way).
    pub fn swap_in(&mut self, scene: &mut LobbyScene, hero: &str) -> bool {
        let Some(r) = self.take(hero) else { return false };
        match scene.swap_hero_ready(r) { Ok(prev) => { self.put(prev); true } Err(r) => { self.put(r); false } }
    }
}

fn parts_bytes(p: &HeroParts) -> usize {
    p.parts.iter().flat_map(|b| b.sections.iter()).map(|s| [&s.diffuse, &s.normal, &s.spec, &s.mask].iter().filter_map(|t| t.as_ref()).map(|t| t.rgba.len()).sum::<usize>()).sum()
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// products / report
// ------------------------------------------------------------------------------------------------------------------------------------------
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// (unit label, seconds it took, seconds after the start it began)
    pub steps: Vec<(String, f32, f32)>,
    pub total_secs: f32,
    pub heroes_cached: usize, pub heroes_total: usize, pub heroes_bytes: usize, pub heroes_skipped: usize, pub heroes_failed: usize,
    pub locker_items: [usize; 3],
    pub ui_textures_before_warm: usize, pub ui_textures_after_warm: usize, pub warm_draws: usize,
    pub audio_log: Vec<String>,
    pub notes: Vec<String>,
}
impl Report {
    pub fn lines(&self) -> Vec<String> {
        let mut v = vec![format!("[preload] ready after {:.2} s", self.total_secs)];
        for (l, s, at) in &self.steps { v.push(format!("[preload]   {s:6.2} s  (from {at:5.2} s)  {l}")); }
        v.push(format!("[preload] skins: the equipped outfit is in the scene, the others build in the background after the hand-off"));
        v.push(format!("[preload] Locker items listed: {} outfits / {} harvesting tools / {} gliders; UI textures {} -> {} over {} warm-up draws", self.locker_items[0], self.locker_items[1], self.locker_items[2], self.ui_textures_before_warm, self.ui_textures_after_warm, self.warm_draws));
        for n in &self.notes { v.push(format!("[preload] note: {n}")); }
        v
    }
}

/// What the lobby gets when the preload is done.
pub struct Products {
    pub set: Arc<fn_pak::PakSet>,
    pub glue: LockerGlue,
    pub profile: Profile,
    pub ui: LobbyUi,
    pub renderer: UiRenderer,
    pub scene: LobbyScene,
    pub audio: Option<Box<dyn AudioSink>>,
    pub heroes: HeroCache,
    /// the key art fading out over the first lobby frames (`overlay.draw` after the lobby is drawn, until it returns `true`)
    pub overlay: Option<AtlasOverlay>,
    pub report: Report,
    /// counters at the hand-off
    pub counters: Counters,
}

/// The `bFlag == 1` layer of the exe's loading screen: the key art alone, fading over `FADE_SECONDS` with the lobby already drawn behind it.
pub struct AtlasOverlay { ctx: UiContext, renderer: UiRenderer, t0: Instant }
impl AtlasOverlay {
    /// Draws the fading art over `target`; `true` when it is gone.
    pub fn draw(&mut self, target: &wgpu::TextureView, w: u32, h: u32) -> bool {
        let a = 1. - (self.t0.elapsed().as_secs_f32() / FADE_SECONDS).clamp(0., 1.);
        if a <= 0. { return true; }
        let mut list = RenderList::new();
        StartupLoadingScreen.build_layers(&mut self.ctx, w as f32, h as f32, 0., 1., true, 0., a, &mut list);
        self.renderer.render(&list, &mut self.ctx.assets, target, w, h, None);
        false
    }
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// worker side
// ------------------------------------------------------------------------------------------------------------------------------------------
struct CpuReady { set: Arc<fn_pak::PakSet>, glue: LockerGlue, profile: Profile }

#[derive(Default)]
pub struct HeroStore { ready: HashMap<String, HeroReady>, bytes: usize, skipped: usize, failed: Vec<(String, String)>, total: usize }

/// BACKGROUND skin builder: starts only when the lobby is on screen (the `HeroBuilder` arrives on `builder_rx` at the hand-off), then decodes + uploads every other
/// outfit on two threads, the equipped one first, while the lobby runs (the main thread only collects finished skins with `HeroCache::pump`). Stops at `budget`.
fn hero_worker(set: Arc<fn_pak::PakSet>, heroes: Vec<String>, builder_rx: Receiver<Option<HeroBuilder>>, store: Arc<Mutex<HeroStore>>, budget: usize) {
    let n = heroes.len();
    let Ok(builder) = builder_rx.recv() else { return };
    let started = Instant::now();
    store.lock().unwrap_or_else(|e| e.into_inner()).total = n;
    let Some(b) = builder else { store.lock().unwrap_or_else(|e| e.into_inner()).failed.push(("(all)".into(), "the scene has no character pipelines: skins not uploaded".into())); return };
    let next = AtomicUsize::new(0);
    let used = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..2 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n { break; }
                if used.load(Ordering::Relaxed) >= budget { store.lock().unwrap_or_else(|e| e.into_inner()).skipped += 1; continue; }
                match crate::lobby_scene::load_hero_parts(&set, &heroes[i]) {
                    Ok(p) => {
                        let bytes = parts_bytes(&p);
                        used.fetch_add(bytes, Ordering::Relaxed);
                        let ready = b.build(&p);
                        drop(p);
                        let mut g = store.lock().unwrap_or_else(|e| e.into_inner());
                        g.bytes += bytes;
                        g.ready.insert(heroes[i].clone(), ready);
                    }
                    Err(e) => store.lock().unwrap_or_else(|e| e.into_inner()).failed.push((heroes[i].clone(), e)),
                }
            });
        }
    });
    { let g = store.lock().unwrap_or_else(|e| e.into_inner()); eprintln!("[lobby] background skin builder finished: {}/{} skins built ({:.0} MiB of decoded textures, {} left to on-demand loads over the {} MiB GPU budget, {} failed) in {:.1} s", g.ready.len(), n, g.bytes as f64 / 1048576., g.skipped, budget / 1048576, g.failed.len(), started.elapsed().as_secs_f32()); }
}
// ------------------------------------------------------------------------------------------------------------------------------------------
// the preloader
// ------------------------------------------------------------------------------------------------------------------------------------------
/// What an extra task may touch (main thread only).
pub struct TaskCtx<'a> {
    pub ui: &'a mut LobbyUi,
    pub renderer: &'a mut UiRenderer,
    pub glue: &'a LockerGlue,
    pub profile: &'a Profile,
    pub set: &'a Arc<fn_pak::PakSet>,
    pub size: (u32, u32),
    /// an off-screen target of the window's size / format: draw a screen into it with `renderer.render` to upload its textures and build its pipelines
    pub target: &'a wgpu::TextureView,
}
struct Task { unit: usize, run: Box<dyn FnMut(&mut TaskCtx) -> bool> }

struct Boot { ctx: UiContext, renderer: UiRenderer }

enum Job { Tab(usize), Picker(LockerSlot, usize), Hover { tab: usize, picking: Option<LockerSlot>, widget: usize, press: bool } }
struct Warm { jobs: VecDeque<Job>, view: fn_ui::screens::athena_locker::LockerView, locker_tab: Option<usize>, draws: usize, tex_before: usize, seen_roles: Vec<Role> }

pub struct Preloader {
    install: PathBuf,
    size: (u32, u32),
    device: wgpu::Device,
    queue: wgpu::Queue,
    format: wgpu::TextureFormat,
    progress: Arc<Progress>,
    boot: Option<Boot>,
    cpu_rx: Receiver<Result<CpuReady, String>>,
    cpu: Option<CpuReady>,
    scene_rx: Receiver<Result<LobbyAssets, String>>,
    builder_tx: Option<Sender<Option<HeroBuilder>>>,
    heroes: Arc<Mutex<HeroStore>>,
    audio: Option<(DeviceSink, LoadLog)>,
    ui: Option<LobbyUi>,
    renderer: Option<UiRenderer>,
    scene: Option<LobbyScene>,
    offscreen: Option<(wgpu::Texture, wgpu::TextureView)>,
    warm: Option<Warm>,
    warm_done: bool,
    scene_warmed: bool,
    tasks: Vec<Task>,
    loaded_at: Option<Instant>,
    notes: Vec<String>,
    t0: Instant,
    counters_ui_before: usize,
    warm_draws: usize,
}

impl Preloader {
    /// Starts the workers and decodes the loading screen's own art (needed for its first frame). `size` = the window's client size (the render targets and the
    /// Locker cards depend on it); `format` = the surface format.
    pub fn start(install: &Path, device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat, size: (u32, u32)) -> Result<Preloader, String> {
        let t0 = Instant::now();
        let progress = Progress::new(BASE_UNITS.iter().map(|(l, w)| (l.to_string(), *w)).collect());
        // loading screen art first: this is what the user sees while everything else loads
        let mut ctx = UiContext::from_install(install, None)?;
        for n in startup_loading::PAK_TEXTURES { ctx.assets.texture(n); }
        let renderer = UiRenderer::with_device(device.clone(), queue.clone(), format);
        // the audio device opens now: its worker loads the music and the UI cues in parallel with everything else, the loading mix keeps it silent until the hand-off
        progress.end(U_HEROES);
        progress.begin(U_AUDIO);
        let audio = match DeviceSink::open_streaming(install.to_path_buf()) {
            Ok((mut sink, log)) => { sink.set_loading_screen(true); Some((sink, log)) }
            Err(e) => { eprintln!("[lobby] audio unavailable: {e}"); progress.end(U_AUDIO); None }
        };
        // worker 1: pak set, catalog, profile; then it starts the scene-asset worker and the skin worker
        let (cpu_tx, cpu_rx) = channel();
        let (scene_tx, scene_rx) = channel();
        let (builder_tx, builder_rx) = channel();
        let heroes = Arc::new(Mutex::new(HeroStore::default()));
        {
            let (install, progress, heroes) = (install.to_path_buf(), progress.clone(), heroes.clone());
            std::thread::Builder::new().name("lobby-preload-cpu".into()).spawn(move || {
                progress.begin(U_CPU);
                let set = match fn_pak::PakSet::open_install(&install) { Ok(s) => Arc::new(s), Err(e) => { let _ = cpu_tx.send(Err(format!("cannot open the paks under {}: {e}", install.display()))); return; } };
                let glue = LockerGlue::load(&set);
                let profile = Profile::load_or_new(&crate::lobby_profile::default_profile_path(), glue.default_loadout());
                let hero = glue.equipped_hero(&profile);
                // every outfit's hero, the equipped one first, no duplicates
                let mut list = vec![hero.clone()];
                for c in glue.catalog.of(Category::Outfit, false) { if let Some(h) = &c.hero { if !list.contains(h) { list.push(h.clone()); } } }
                eprintln!("[lobby] {} outfits / {} harvesting tools / {} gliders in the paks; profile owns {} items, equipped outfit {:?}", glue.catalog.count(Category::Outfit, false), glue.catalog.count(Category::Pickaxe, false), glue.catalog.count(Category::Glider, false), profile.owned_count(), profile.equipped(crate::lobby_profile::Slot::Outfit));
                progress.end(U_CPU);
                // scene assets
                { let (install, progress, tx, hero) = (install.clone(), progress.clone(), scene_tx, hero);
                  std::thread::Builder::new().name("lobby-preload-scene".into()).spawn(move || { progress.begin(U_SCENE_ASSETS); let r = LobbyAssets::load(&install, Some(&hero)); progress.end(U_SCENE_ASSETS); let _ = tx.send(r); }).ok(); }
                // skins
                { let (set, progress, heroes) = (set.clone(), progress.clone(), heroes);
                  std::thread::Builder::new().name("lobby-preload-heroes".into()).spawn(move || hero_worker(set, list, builder_rx, heroes, hero_budget_bytes())).ok(); }
                let _ = cpu_tx.send(Ok(CpuReady { set, glue, profile }));
            }).map_err(|e| e.to_string())?;
        }
        Ok(Preloader {
            install: install.to_path_buf(), size, device, queue, format, progress, boot: Some(Boot { ctx, renderer }), cpu_rx, cpu: None, scene_rx, builder_tx: Some(builder_tx), heroes, audio,
            ui: None, renderer: None, scene: None, offscreen: None, warm: None, warm_done: false, scene_warmed: false, tasks: vec![], loaded_at: None, notes: vec![], t0, counters_ui_before: 0, warm_draws: 0,
        })
    }

    pub fn progress(&self) -> &Arc<Progress> { &self.progress }
    /// Registers an extra main-thread task run after the Locker warm-up (the shop / console hook). `run` is called once per slice until it returns `true`; its unit
    /// counts `weight` of the bar. Must be called before the first [`frame`](Self::frame).
    pub fn add_task(&mut self, label: &str, weight: f32, run: Box<dyn FnMut(&mut TaskCtx) -> bool>) {
        let unit = self.progress.add(label, weight);
        self.tasks.push(Task { unit, run });
    }

    fn offscreen_view(&mut self) -> wgpu::TextureView {
        if self.offscreen.is_none() {
            let (w, h) = self.size;
            let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some("lobby preload target"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: self.format, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.offscreen = Some((tex, view));
        }
        self.offscreen.as_ref().unwrap().1.clone()
    }

    // ---------------------------------------------------------------------------------------------------------------------------- main thread slices
    /// Runs main-thread work until `deadline` or until there is nothing runnable right now. `Ok(true)` = everything is loaded.
    pub fn poll(&mut self, deadline: Instant) -> Result<bool, String> {
        loop {
            self.poll_audio();
            if self.progress.all_done() && self.ui.is_some() && self.scene.is_some() && self.cpu.is_some() { return Ok(true); }
            let did = self.step(deadline)?;
            if !did || Instant::now() >= deadline { break; }
        }
        self.poll_audio();
        Ok(self.progress.all_done() && self.ui.is_some() && self.scene.is_some() && self.cpu.is_some())
    }

    fn poll_audio(&mut self) {
        if self.progress.done(U_AUDIO) { return; }
        if let Some((_, log)) = &self.audio {
            if self.progress.units().get(U_AUDIO).and_then(|u| u.started).map_or(false, |s| self.progress.elapsed() - s > 60.) { self.notes.push("audio: the cue loader did not finish within 60 s".into()); self.progress.end(U_AUDIO); return; }
            // `open_streaming` loads the music first, then the gameplay + UI cues, then the (in-game) ambience: the lobby needs the first two
            if log.lines().iter().any(|l| l.starts_with("gameplay + UI cues ready") || l.contains("audio stays silent")) { self.progress.end(U_AUDIO); }
        }
    }

    fn step(&mut self, deadline: Instant) -> Result<bool, String> {
        if self.cpu.is_none() {
            match self.cpu_rx.try_recv() { Ok(Ok(c)) => self.cpu = Some(c), Ok(Err(e)) => return Err(e), Err(TryRecvError::Empty) => {}, Err(TryRecvError::Disconnected) => return Err("the preload worker died".into()) }
        }
        if self.ui.is_none() {
            self.progress.begin(U_UI);
            let ui = LobbyUi::load(&self.install).map_err(|e| format!("UI: {e}"))?;
            self.ui = Some(ui);
            self.progress.end(U_UI);
            return Ok(true);
        }
        if self.renderer.is_none() {
            self.progress.begin(U_RENDERER);
            self.renderer = Some(UiRenderer::with_device(self.device.clone(), self.queue.clone(), self.format));
            self.progress.end(U_RENDERER);
            return Ok(true);
        }
        if self.scene.is_none() {
            match self.scene_rx.try_recv() {
                Ok(Ok(assets)) => {
                    self.progress.begin(U_SCENE_GPU);
                    let scene = LobbyScene::from_assets(assets, &self.device, &self.queue, self.format).map_err(|e| format!("scene: {e}"))?;
                    self.scene = Some(scene);
                    self.progress.end(U_SCENE_GPU);
                    return Ok(true);
                }
                Ok(Err(e)) => return Err(format!("scene assets: {e}")),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => return Err("the scene loader died".into()),
            }
        }
        if self.cpu.is_some() && !self.warm_done { self.warm_slice(deadline)?; return Ok(true); }
        if self.warm_done && !self.tasks.is_empty() {
            let view = self.offscreen_view();
            let (cpu, ui, renderer) = (self.cpu.as_ref().unwrap(), self.ui.as_mut().unwrap(), self.renderer.as_mut().unwrap());
            let mut i = 0;
            while i < self.tasks.len() {
                self.progress.begin(self.tasks[i].unit);
                let mut cx = TaskCtx { ui: &mut *ui, renderer: &mut *renderer, glue: &cpu.glue, profile: &cpu.profile, set: &cpu.set, size: self.size, target: &view };
                if (self.tasks[i].run)(&mut cx) { self.progress.end(self.tasks[i].unit); self.tasks.remove(i); } else { i += 1; }
                if Instant::now() >= deadline { break; }
            }
            return Ok(true);
        }
        if self.scene.is_some() && self.warm_done && self.tasks.is_empty() && !self.scene_warmed {
            self.progress.begin(U_SCENE_WARM);
            let view = self.offscreen_view();
            let (w, h) = self.size;
            let scene = self.scene.as_mut().unwrap();
            // two real frames: the render targets / bind groups of this window size, the AnimBP state, every pipeline's first use
            scene.render(&view, w, h, 0.0);
            scene.render(&view, w, h, 1.0 / 60.0);
            let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
            self.scene_warmed = true;
            self.progress.end(U_SCENE_WARM);
            return Ok(true);
        }
        Ok(false)
    }

    /// One slice of the Locker / tabs / icons warm-up (every draw is the real `LobbyUi::draw` followed by the real `UiRenderer::render`).
    fn warm_slice(&mut self, deadline: Instant) -> Result<(), String> {
        let (w, h) = self.size;
        let target = self.offscreen_view();
        if self.warm.is_none() {
            self.progress.begin(U_UI_WARM);
            let cpu = self.cpu.as_ref().unwrap();
            // the user owns every Battle Royale item: the picker lists the whole catalog (the same `LockerGlue::sync` conversion as the live Locker, so the card keys match)
            let mut all = Profile::new(cpu.glue.default_loadout());
            for cat in [Category::Outfit, Category::Pickaxe, Category::Glider] { for c in cpu.glue.catalog.of(cat, false) { all.grant(&c.id); } }
            let mut view = fn_ui::screens::athena_locker::LockerView::default();
            cpu.glue.sync(&all, &mut view);
            let ui = self.ui.as_mut().unwrap();
            ui.inner_mut().set_card_budget(usize::MAX);
            self.counters_ui_before = ui.ctx_mut().assets.store.len();
            let n_tabs = ui.tab_labels().len();
            // only the Lobby tab blocks the loading screen; every other tab / the Locker pages are warmed after the hand-off (`FN_LOBBY_WARM_ALL=1` restores the full warm-up)
            let jobs = (0..if std::env::var_os("FN_LOBBY_WARM_ALL").is_some() { n_tabs } else { 1.min(n_tabs) }).map(Job::Tab).collect();
            self.warm = Some(Warm { jobs, view, locker_tab: ui.tab_index_of(fn_ui::screens::athena_locker::TAB_CLASS), draws: 0, tex_before: self.counters_ui_before, seen_roles: vec![] });
        }
        loop {
            let warm = self.warm.as_mut().unwrap();
            let Some(job) = warm.jobs.pop_front() else {
                // finished: restore the live card budget, leave a settled state, wait for the GPU
                let ui = self.ui.as_mut().unwrap();
                ui.inner_mut().set_card_budget(fn_ui::screens::athena_locker::card::CardMats::default().max_per_frame);
                let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
                self.warm_draws = warm.draws;
                self.warm_done = true;
                self.progress.end(U_UI_WARM);
                return Ok(());
            };
            let ui = self.ui.as_mut().unwrap();
            let renderer = self.renderer.as_mut().unwrap();
            let (tab, picking, scroll, hover, press) = match &job {
                Job::Tab(t) => (*t, None, 0, None, None),
                Job::Picker(s, r) => (warm.locker_tab.unwrap_or(0), Some(*s), *r, None, None),
                Job::Hover { tab, picking, widget, press } => (*tab, *picking, 0, Some(*widget), press.then_some(*widget)),
            };
            let mut st = LobbyState { selected_tab: tab, mtx_balance: self.cpu.as_ref().map_or(0, |c| c.profile.balance()), ..Default::default() };
            st.locker = warm.view.clone();
            if let Some(s) = picking { st.locker.picking = Some(s); st.locker.picker_t = fn_ui::screens::athena_locker::INTRO_PICKER_END; st.locker.scroll_row = scroll; }
            st.hover_widget = hover; st.press_widget = press;
            let frame = ui.draw(w as f32, h as f32, &st);
            renderer.render(&frame.list, &mut ui.ctx_mut().assets, &target, w, h, Some([0., 0., 0., 0.]));
            warm.draws += 1;
            // follow-up jobs
            match job {
                Job::Tab(t) => {
                    // every distinct interactive element in its hover and pressed state
                    for id in 0..frame.id_rects.len() {
                        if frame.id_rects[id].w <= 0. { continue; }
                        let role = Controller::role(ui.inner(), id);
                        if role == Role::Other || matches!(role, Role::Tile(..)) { continue; }
                        if warm.seen_roles.contains(&role) { continue; }
                        warm.seen_roles.push(role);
                        for press in [false, true] { warm.jobs.push_back(Job::Hover { tab: t, picking: None, widget: id, press }); }
                    }
                    if Some(t) == warm.locker_tab {
                        for s in LockerSlot::ALL { warm.jobs.push_back(Job::Picker(s, 0)); }
                    }
                }
                Job::Picker(s, row) => {
                    if let Some(l) = ui.inner().locker() {
                        let (_, vis, total) = l.grid;
                        if row == 0 {
                            // scroll through every page of the grid
                            let max = total.saturating_sub(vis);
                            let mut r = vis.max(1);
                            while r < max { warm.jobs.push_back(Job::Picker(s, r)); r += vis.max(1); }
                            if max > 0 { warm.jobs.push_back(Job::Picker(s, max)); }
                            // the first tile's hover / pressed state
                            for id in 0..frame.id_rects.len() {
                                if frame.id_rects[id].w <= 0. { continue; }
                                if let Role::Tile(ts, _) = Controller::role(ui.inner(), id) {
                                    if ts == s && !warm.seen_roles.contains(&Role::Tile(s, 0)) {
                                        warm.seen_roles.push(Role::Tile(s, 0));
                                        for press in [false, true] { warm.jobs.push_back(Job::Hover { tab: warm.locker_tab.unwrap_or(0), picking: Some(s), widget: id, press }); }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                Job::Hover { .. } => {}
            }
            if Instant::now() >= deadline { return Ok(()); }
        }
    }

    // ---------------------------------------------------------------------------------------------------------------------------- frame
    /// One frame of the loading screen: advances the preload by [`FRAME_BUDGET`] and draws the screen into `target` (the window's frame). Returns the products once
    /// everything is loaded and the screen's 0.2 s foreground fade is over (the first lobby frame follows).
    pub fn frame(&mut self, target: &wgpu::TextureView, w: u32, h: u32) -> Result<Option<Products>, String> {
        if self.loaded_at.is_none() && self.poll(Instant::now() + FRAME_BUDGET)? { self.loaded_at = Some(Instant::now()); }
        let fg = self.loaded_at.map_or(1., |t| 1. - (t.elapsed().as_secs_f32() / FADE_SECONDS).clamp(0., 1.));
        self.draw_boot(target, w, h, fg, 1.);
        if self.loaded_at.is_some() && fg <= 0. { return self.finish().map(Some); }
        Ok(None)
    }

    /// Draws the loading screen (`fg` = alpha of plate / texts / throbber / bar, `atlas` = alpha of the key art).
    pub fn draw_boot(&mut self, target: &wgpu::TextureView, w: u32, h: u32, fg: f32, atlas: f32) {
        let t = self.t0.elapsed().as_secs_f32();
        let progress = self.progress.fraction();
        let Some(b) = self.boot.as_mut() else { return };
        let mut list = RenderList::new();
        StartupLoadingScreen.build_layers(&mut b.ctx, w as f32, h as f32, t, progress, true, fg, atlas, &mut list);
        b.renderer.render(&list, &mut b.ctx.assets, target, w, h, Some([0., 0., 0., 1.]));
    }

    /// CPU raster of the loading screen at the current progress (premultiplied RGBA8), for captures.
    pub fn raster_boot(&mut self, w: u32, h: u32) -> Vec<u8> {
        let t = self.t0.elapsed().as_secs_f32();
        let progress = self.progress.fraction();
        let b = self.boot.as_mut().expect("boot screen");
        let mut list = RenderList::new();
        StartupLoadingScreen.build_layers(&mut b.ctx, w as f32, h as f32, t, progress, true, 1., 1., &mut list);
        fn_ui::raster::render(&list, &mut b.ctx.assets, w, h, None)
    }

    pub fn fraction(&self) -> f32 { self.progress.fraction() }
    pub fn loaded(&self) -> bool { self.loaded_at.is_some() }

    fn finish(&mut self) -> Result<Products, String> {
        let cpu = self.cpu.take().ok_or("preload: no catalog")?;
        let mut ui = self.ui.take().ok_or("preload: no UI")?;
        let renderer = self.renderer.take().ok_or("preload: no UI renderer")?;
        let scene = self.scene.take().ok_or("preload: no scene")?;
        let boot = self.boot.take().ok_or("preload: no boot screen")?;
        // the lobby is complete: now the background builder may start on the other skins
        if let Some(tx) = self.builder_tx.take() { let _ = tx.send(scene.hero_builder()); }
        let mut cache = HeroCache { store: Some(self.heroes.clone()), ..Default::default() };
        let mut report = Report::default();
        let units = self.progress.units();
        report.steps = units.iter().filter_map(|u| Some((u.label.clone(), u.finished? - u.started?, u.started?))).collect();
        report.steps.sort_by(|a, b| a.2.total_cmp(&b.2));
        report.total_secs = self.t0.elapsed().as_secs_f32();
        report.heroes_cached = cache.len(); report.heroes_total = cache.total; report.heroes_bytes = cache.bytes; report.heroes_skipped = cache.skipped_over_budget; report.heroes_failed = cache.failed.len();
        report.notes.push("skins of the other outfits build in the background after the hand-off (HeroCache::pump)".into());
        report.locker_items = [Category::Outfit, Category::Pickaxe, Category::Glider].map(|c| cpu.glue.catalog.count(c, false));
        report.ui_textures_before_warm = self.counters_ui_before; report.ui_textures_after_warm = ui.ctx_mut().assets.store.len();
        report.warm_draws = self.warm_draws;
        let mut audio: Option<Box<dyn AudioSink>> = None;
        if let Some((mut sink, log)) = self.audio.take() {
            report.audio_log = log.lines();
            // the loading screen is over: pop the loading mix, start the front-end music (what the lobby did itself before) and the menu-open sound
            sink.set_loading_screen(false);
            sink.set_music(Some(MusicCue::LobbyAthena));
            sink.event(SoundEvent::MenuOpen);
            audio = Some(Box::new(sink));
        }
        cache.failed.shrink_to_fit();
        let counters = counters(&mut ui, &renderer);
        let overlay = Some(AtlasOverlay { ctx: boot.ctx, renderer: boot.renderer, t0: Instant::now() });
        let _ = &self.notes;
        Ok(Products { set: cpu.set, glue: cpu.glue, profile: cpu.profile, ui, renderer, scene, audio, heroes: cache, overlay, report, counters })
    }
}




