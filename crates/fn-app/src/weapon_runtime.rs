//! WEAPON RUNTIME: any Battle Royale firearm in game, loaded lazily on first use. Everything a weapon needs (item data, stat row, mesh + textures, actor Blueprint chain montages, ADS cameras,
//! hit data, muzzle flash, audio, HUD art) is read from the 1.8 data by `fn_pose::loadout_gun::load_gun_pack` on a worker thread, then installed on the UI thread by [`WeaponRuntime::poll`]
//! (one install per frame is a few ms: a GPU upload of the mesh and its textures, a few vector pushes). The three starter firearms are gun indices 0..3 and work the same way as every other weapon.
//!
//! # API for the F2 console / other front-ends (all on [`WeaponRuntime`], held by the app as `App::weapons`)
//! * [`WeaponRuntime::catalog`]: every BR ranged weapon (45 WIDs) as [`WeaponInfo`] (`wid`, short name, display name once the background read finished, rarity).
//! * [`WeaponRuntime::resolve`]: user text -> WID package path. Accepts the full path, the asset name (`WID_Assault_Auto_Athena_R_Ore_T03`, `Assault_Auto_Athena_R_Ore_T03`, any case) or the display
//!   name; a display name shared by several rarities is an error that lists them (nothing is picked on the caller's behalf).
//! * [`WeaponRuntime::request_give`]`(query, equip)`: "give weapon by WID / display name". Queues the load and returns immediately; when the weapon is ready (typically a few frames, the first one of a
//!   class may take ~0.5 s on the worker, never on the UI thread) the app installs it, puts it on the quickbar (see `Arms::give_gun` for the slot rules) with a full magazine and, with `equip`, selects it
//!   (the equip montage + sound play). Giving an already installed weapon refills its magazine and equips it. Ammo is the player's existing stack (the data grants none: use the ammo command).
//! * [`WeaponRuntime::request_equip`]`(query)`: "equip weapon": selects the weapon's quickbar slot when it is on the bar; otherwise it behaves as `request_give(query, true)` (the sandbox has no
//!   backpack, so an equipped weapon is always a quickbar weapon).
//! * [`WeaponRuntime::take_events`]: log lines (`Info` / `Error`) of what the runtime did since the last call, for the console log.
//! The app calls [`WeaponRuntime::poll`] every frame and applies the returned [`Installed`] HUD changes (`App::apply_installed_weapons`).
//!
//! What is NOT in the data (never invented, logged at install as `[weapon] ... data lacks ...`): see `GunPack::gaps` (montage roles a Blueprint chain does not set, core-set assets the AnimBlueprint leaves null).
use crate::arms::{Arms, ArmsData, GunData, QuickbarItem};
use fn_animgraph::weapon_assets::BR_WEAPON_ASSETS;
use fn_pose::loadout::Resources;
use fn_pose::loadout_gun::{self, GunPack};
use fn_pose::ClipSourceDir;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Where the BR weapon item definitions live.
pub const WID_DIR: &str = "/Game/Athena/Items/Weapons/";

/// One BR ranged weapon of the catalog.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponInfo {
    /// `/Game/Athena/Items/Weapons/WID_...` package path.
    pub wid: String,
    /// Asset name (`WID_Assault_Auto_Athena_R_Ore_T03`).
    pub short: String,
    /// `DisplayName` of the item definition (filled by a background read of all WIDs, `None` until it finished or when the item has no name).
    pub display_name: Option<String>,
    /// 0 Handmade .. 4 Fine, as the HUD card uses it (`None` until the read finished).
    pub rarity: Option<u8>,
}

/// What `poll` did for the HUD: the new quickbar card and the HUD textures to register, one entry per installed / given weapon.
#[derive(Clone, Debug)]
pub struct Installed {
    pub gun: usize,
    pub slot: usize,
    pub wid: String,
    pub replaced: Option<QuickbarItem>,
    pub selected: bool,
    pub rarity: u8,
    pub icon: String,
    /// Inventory-screen texts of a newly installed weapon (`None` when the weapon was installed before: already in the catalog).
    pub catalog: Option<fn_pose::loadout_inventory::CatalogItem>,
    /// HUD textures not registered before: (PNG name, w, h, RGBA8).
    pub ui_images: Vec<(String, u32, u32, Vec<u8>)>,
}

/// A weapon read by a worker: the resources part and the hit data.
pub struct Built { pub pack: GunPack, pub hit: fn_hitreg::GunHit, pub secs: f32 }

enum Msg { Catalog(Vec<WeaponInfo>), Built { wid: String, result: Result<Box<Built>, String> } }

struct Ask { wid: String, equip: bool, give: bool, pickup: Option<u64> }

type Prefetched = (Option<fn_pose::loadout_inventory::CatalogItem>, Vec<(String, u32, u32, Vec<u8>)>);

pub struct WeaponRuntime {
    install: Option<PathBuf>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    catalog: Vec<WeaponInfo>,
    catalog_started: bool,
    /// Requests not handled yet.
    wants: Vec<Ask>,
    /// WID -> the requests waiting for its load.
    inflight: HashMap<String, Vec<Ask>>,
    prefetched: HashMap<String, Prefetched>,
    pickup_results: Vec<(u64, bool)>,
    events: Vec<(bool, String)>,
    /// Load times of finished packs (WID short name, seconds on the worker, seconds of the UI-thread install), for reports.
    pub timings: Vec<(String, f32, f32)>,
}

static PAKS: Mutex<Option<(PathBuf, Arc<fn_pak::PakSet>)>> = Mutex::new(None);
/// The shared pak set (opened once per install directory).
pub(crate) fn paks(install: &std::path::Path) -> Result<Arc<fn_pak::PakSet>, String> {
    let mut g = PAKS.lock().map_err(|e| e.to_string())?;
    if let Some((dir, set)) = g.as_ref() { if dir == install { return Ok(set.clone()); } }
    let set = Arc::new(fn_pak::PakSet::open_install(install).map_err(|e| e.to_string())?);
    *g = Some((install.to_path_buf(), set.clone()));
    Ok(set)
}

fn short_of(wid: &str) -> &str { wid.rsplit('/').next().unwrap_or(wid) }
fn norm(s: &str) -> String { s.chars().filter(|c| !c.is_whitespace() && *c != '_' && *c != '-').flat_map(|c| c.to_lowercase()).collect() }

impl Default for WeaponRuntime { fn default() -> Self { Self::new() } }

impl WeaponRuntime {
    pub fn new() -> WeaponRuntime {
        let (tx, rx) = channel();
        let catalog = BR_WEAPON_ASSETS.iter().map(|w| WeaponInfo { wid: format!("{WID_DIR}{}", w.wid), short: w.wid.to_owned(), display_name: None, rarity: None }).collect();
        WeaponRuntime { install: None, tx, rx, catalog, catalog_started: false, wants: vec![], inflight: HashMap::new(), prefetched: HashMap::new(), pickup_results: vec![], events: vec![], timings: vec![] }
    }
    /// The game install (called once when the loadout resources arrive); starts the background read of the display names.
    pub fn set_install(&mut self, dir: Option<PathBuf>) {
        self.install = dir;
        if self.catalog_started { return; }
        let Some(dir) = self.install.clone() else { return };
        self.catalog_started = true;
        let (tx, wids) = (self.tx.clone(), self.catalog.iter().map(|w| w.wid.clone()).collect::<Vec<_>>());
        std::thread::spawn(move || {
            let Ok(set) = paks(&dir) else { return };
            let src = ClipSourceDir::Pak(&set);
            let mut out = vec![];
            for wid in wids {
                let (display_name, rarity) = match loadout_gun::weapon_texts(&src, &wid) { Ok((n, r)) => (n, Some(r)), Err(_) => (None, None) };
                out.push(WeaponInfo { short: short_of(&wid).to_owned(), wid, display_name, rarity });
            }
            let _ = tx.send(Msg::Catalog(out));
        });
    }
    pub fn catalog(&self) -> &[WeaponInfo] { &self.catalog }

    /// All catalog weapons a query names: full path / asset name (exact, any case, `WID_` optional) first, else display name (any case, spaces / underscores ignored).
    pub fn resolve_all(&self, query: &str) -> Vec<&WeaponInfo> {
        let q = query.trim();
        let q = q.strip_prefix(WID_DIR).unwrap_or(q);
        let q = q.split('.').next().unwrap_or(q);
        let nq = norm(q);
        if nq.is_empty() { return vec![]; }
        let by_name: Vec<&WeaponInfo> = self.catalog.iter().filter(|w| { let s = norm(&w.short); s == nq || s.strip_prefix("wid") == Some(&nq) }).collect();
        if !by_name.is_empty() { return by_name; }
        self.catalog.iter().filter(|w| w.display_name.as_deref().is_some_and(|d| norm(d) == nq)).collect()
    }
    /// The one weapon a query names, else why not (unknown, or ambiguous with the candidates listed).
    pub fn resolve(&self, query: &str) -> Result<String, String> {
        let m = self.resolve_all(query);
        match m.as_slice() {
            [] => Err(format!("no Battle Royale weapon matches '{query}'{}", if self.catalog.iter().any(|w| w.display_name.is_some()) { "" } else { " (display names are still being read: use the WID asset name)" })),
            [w] => Ok(w.wid.clone()),
            many => Err(format!("'{query}' matches {} weapons: {}", many.len(), many.iter().map(|w| w.short.as_str()).collect::<Vec<_>>().join(", "))),
        }
    }
    /// "give weapon": see the module docs. Returns the resolved WID.
    pub fn request_give(&mut self, query: &str, equip: bool) -> Result<String, String> {
        let wid = self.resolve(query)?;
        self.wants.push(Ask { wid: wid.clone(), equip, give: true, pickup: None });
        Ok(wid)
    }
    pub fn request_prefetch(&mut self, query: &str) -> Result<String, String> {
        let wid = self.resolve(query)?;
        self.wants.push(Ask { wid: wid.clone(), equip: false, give: false, pickup: None });
        Ok(wid)
    }
    pub fn request_pickup(&mut self, query: &str, pickup: u64) -> Result<String, String> {
        let wid = self.resolve(query)?;
        self.wants.push(Ask { wid: wid.clone(), equip: false, give: true, pickup: Some(pickup) });
        Ok(wid)
    }
    pub fn take_pickup_results(&mut self) -> Vec<(u64, bool)> { std::mem::take(&mut self.pickup_results) }
    /// "equip weapon": see the module docs. Returns the resolved WID.
    pub fn request_equip(&mut self, query: &str) -> Result<String, String> { self.request_give(query, true) }
    /// Log lines since the last call: (is_error, text).
    pub fn take_events(&mut self) -> Vec<(bool, String)> { std::mem::take(&mut self.events) }
    fn info(&mut self, s: String) { eprintln!("[weapon] {s}"); self.events.push((false, s)); }
    fn error(&mut self, s: String) { eprintln!("[weapon] ERROR {s}"); self.events.push((true, s)); }
    fn refuse(&mut self, ask: &Ask, why: &str) {
        match ask.pickup {
            Some(id) => { self.pickup_results.push((id, false)); self.info(format!("{}: {why}: the pickup stays on the ground", short_of(&ask.wid))); }
            None if ask.give => self.error(format!("{}: {why}", short_of(&ask.wid))),
            None => eprintln!("[weapon] prefetch of {} failed: {why}", short_of(&ask.wid)),
        }
    }
    /// Drops the queued requests with an error line each (no live loadout: preview / non-pak mode).
    pub fn fail_all(&mut self, why: &str) { for ask in std::mem::take(&mut self.wants) { self.refuse(&ask, why); } }
    fn give(&mut self, res: &Resources, arms: &mut Arms, g: usize, ask: Ask, out: &mut Vec<Installed>) {
        let owned_before = arms.gun_loc(g).is_some();
        match arms.give_gun(g, ask.equip) {
            Ok((slot, replaced)) => {
                let (catalog, ui_images) = self.prefetched.remove(&ask.wid).unwrap_or_default();
                if let Some(id) = ask.pickup { self.pickup_results.push((id, true)); }
                out.push(Installed { gun: g, slot, wid: ask.wid, replaced, selected: ask.equip, rarity: res.gun_item(g).map_or(0, |i| i.rarity), icon: res.gun_item(g).map(|i| i.icon.clone()).unwrap_or_default(), catalog, ui_images });
            }
            Err(_) if !owned_before && arms.gun_loc(g).is_some() => { if let Some(id) = ask.pickup { self.pickup_results.push((id, true)); } }
            Err(e) => self.refuse(&ask, &e),
        }
    }
    /// True while a load is running or queued.
    pub fn busy(&self) -> bool { !self.inflight.is_empty() || !self.wants.is_empty() }

    /// Once per frame: starts the loads that were requested, and installs the ones that finished. `res` is the live resource set (inside the scene), `arms` the player's arms, `ads` the ADS driver.
    /// `gpu` uploads a freshly installed firearm into the renderer (`Scene::install_extra_gun`) and extends the player's clip library; `Err` rolls the install back.
    pub fn poll(&mut self, res: &mut Resources, arms: &mut Arms, mut ads: Option<&mut crate::ads::AdsDriver>, gpu: &mut dyn FnMut(&mut Resources, &Installation) -> Result<(), String>) -> Vec<Installed> {
        let mut out = vec![];
        // 1. requests
        for ask in std::mem::take(&mut self.wants) {
            if let Some(g) = res.gun_index_of(&ask.wid) {
                if ask.give { self.give(res, arms, g, ask, &mut out); }
                continue;
            }
            if let Some(v) = self.inflight.get_mut(&ask.wid) { v.push(ask); continue; }
            let Some(dir) = self.install.clone() else { self.refuse(&ask, "no game install known, the weapon cannot be loaded"); continue };
            let wid = ask.wid.clone();
            let (known, socket, tx, w) = (res.known(), res.player_hand_socket.clone(), self.tx.clone(), wid.clone());
            self.inflight.insert(wid.clone(), vec![ask]);
            self.info(format!("loading {} on a worker thread", short_of(&wid)));
            std::thread::spawn(move || {
                let t = std::time::Instant::now();
                let result = (|| -> Result<Box<Built>, String> {
                    let set = paks(&dir)?;
                    let src = ClipSourceDir::Pak(&set);
                    let pack = loadout_gun::load_gun_pack(&src, &w, &known, &socket)?;
                    let hit = fn_hitreg::load(&src, &w).map_err(|e| format!("{w}: hit data: {e}"))?;
                    // FIRST USE (2026-10-06): the original-material gun model's CPU half (mesh parse, material instances, texture mips), so that the install's
                    // `OrigGuns::add` on the UI thread only uploads
                    if let Err(e) = fn_matshader::gun::prepare_weapon(&set, &w, &pack.gun.item.mesh_materials) { eprintln!("[weapon] {w}: gun-pass prepare failed ({e}): built at install"); }
                    Ok(Box::new(Built { pack, hit, secs: t.elapsed().as_secs_f32() }))
                })();
                let _ = tx.send(Msg::Built { wid: w, result });
            });
        }
        // 2. results (one install per call keeps the UI-thread cost to a single weapon's upload)
        let mut installed_one = false;
        let mut pending = vec![];
        while let Ok(m) = self.rx.try_recv() { pending.push(m); }
        for m in pending {
            match m {
                Msg::Catalog(c) => { self.catalog = c; }
                Msg::Built { wid, result } if !installed_one => {
                    let asks = self.inflight.remove(&wid).unwrap_or_default();
                    match result {
                        Err(e) => { for a in &asks { self.refuse(a, &e); } }
                        Ok(b) => {
                            installed_one = true;
                            let t = std::time::Instant::now();
                            let worker = b.secs;
                            match install_data(res, arms, ads.as_deref_mut(), *b, gpu) {
                                Ok((g, catalog, ui_images, notes)) => {
                                    let ui = t.elapsed().as_secs_f32();
                                    self.timings.push((short_of(&wid).to_owned(), worker, ui));
                                    self.info(format!("{} installed as firearm {g} (worker {:.2}s, UI thread {:.1} ms)", short_of(&wid), worker, ui * 1000.));
                                    for n in notes { self.info(n); }
                                    self.prefetched.insert(wid.clone(), (Some(catalog), ui_images));
                                    for a in asks { if a.give { self.give(res, arms, g, a, &mut out); } }
                                }
                                Err(e) => { for a in &asks { self.refuse(a, &e); } }
                            }
                        }
                    }
                }
                late @ Msg::Built { .. } => { let _ = self.tx.send(late); } // a second weapon finished this frame: installed next frame
            }
        }
        out
    }
}

/// What the GPU / clip step of an install needs to know.
pub struct Installation { pub gun: usize, pub new_sequences: Vec<String>, pub new_cores: Vec<String> }


/// CPU side of an install, shared with the headless tests: merges the pack into `res`, builds the arms data, appends the firearm to the arms / ADS driver / hit-reg / muzzle state, puts it on the
/// quickbar. `gpu` runs after the resources are merged and before anything else is touched; its `Err` undoes the merge. Returns the HUD change and notes about missing data.
pub fn install(res: &mut Resources, arms: &mut Arms, ads: Option<&mut crate::ads::AdsDriver>, built: Built, gpu: &mut dyn FnMut(&mut Resources, &Installation) -> Result<(), String>, equip: bool) -> Result<(Installed, Vec<String>), String> {
    let wid = built.pack.wid.clone();
    let (g, catalog, ui_images, notes) = install_data(res, arms, ads, built, gpu)?;
    let (rarity, icon) = res.gun_item(g).map(|i| (i.rarity, i.icon.clone())).unwrap_or_default();
    let (slot, replaced) = arms.give_gun(g, equip)?;
    Ok((Installed { gun: g, slot, wid, replaced, selected: equip, rarity, icon, catalog: Some(catalog), ui_images }, notes))
}

pub fn install_data(res: &mut Resources, arms: &mut Arms, ads: Option<&mut crate::ads::AdsDriver>, built: Built, gpu: &mut dyn FnMut(&mut Resources, &Installation) -> Result<(), String>) -> Result<(usize, fn_pose::loadout_inventory::CatalogItem, Vec<(String, u32, u32, Vec<u8>)>, Vec<String>), String> {
    let Built { pack, hit, .. } = built;
    let wid = pack.wid.clone();
    let mut notes = vec![];
    if !pack.gaps.is_empty() { notes.push(format!("{} data lacks: {}", short_of(&wid), pack.gaps.join(", "))); }
    { let e = &pack.gun.extras; if e.charge != (0., 0., 0.) { notes.push(format!("{} has a charge row (MinChargeTime {}, MaxChargeTime {}, ChargeDownTime {}): the charge is NOT modelled, the shot fires on release", short_of(&wid), e.charge.0, e.charge.1, e.charge.2)); } }
    let ui_images = pack.ui_images.clone();
    let catalog = pack.inventory.clone();
    let ammo_setup = pack.ammo.clone();
    let profile = pack.gun.ads.clone();
    let inst = res.install_gun(pack)?;
    let fail = |res: &mut Resources, e: String| -> Result<_, String> { res.uninstall_last_gun(); Err(e) };
    if inst.gun != arms.data.guns.len() { return fail(res, format!("gun index {} does not follow the arms' {} firearms", inst.gun, arms.data.guns.len())); }
    let gun = match ArmsData::gun_from_resources(res, inst.gun) { Ok(g) => g, Err(e) => return fail(res, e) };
    if let Err(e) = gpu(res, &Installation { gun: inst.gun, new_sequences: inst.new_sequences.clone(), new_cores: inst.new_cores.clone() }) { return fail(res, e); }
    let (max_stack, drop_count, starting) = ammo_rules(&ammo_setup, &gun);
    let g = arms.add_gun(gun, max_stack, drop_count, starting);
    debug_assert_eq!(g, inst.gun);
    if let Some(a) = ads { if a.profiles.len() == g { a.add_profile(profile); } }
    crate::hitreg_hook::add_gun(g, hit);
    match crate::muzzle_hook::add_gun(g, &wid) { Ok(n) => notes.push(format!("muzzle: {n}")), Err(e) => notes.push(format!("muzzle: {e}")) }
    Ok((g, catalog, ui_images, notes))
}

/// (`MaxStackSize`, `DropCount`, starting count) of a firearm's ammo item, from its pack's ammo setup (the same rules `AmmoRules::from_setup` applies to the starters).
fn ammo_rules(setup: &fn_pose::loadout_ammo::AmmoSetup, gun: &GunData) -> (u32, u32, u32) {
    let r = crate::arms::AmmoRules::from_setup(setup, std::slice::from_ref(gun));
    (r.max_stack[0], r.drop_count[0], r.starting[0])
}

/// A clip library that answers from `base` first and from `extra` for what `base` lacks (clips added at run time: the animation sets of weapons loaded after the character was built).
pub struct LayeredClips { pub base: Box<dyn fn_animgraph::ClipLibrary + Send>, pub extra: fn_pose::ClipStore }
impl fn_animgraph::ClipLibrary for LayeredClips {
    fn clip(&self, name: &str) -> Option<&dyn fn_animgraph::ClipSource> { self.base.clip(name).or_else(|| self.extra.clip(name)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arms::{ArmsInput, ArmsSound, Policy};
    use fn_audio::weapon_events as we;
    use fn_game::{FrameInput, GameSim};
    use fn_pose::{ClipStore, SampleMode};

    const DT: f32 = 1. / 60.;
    fn set_ok(v: &str) -> bool { v.starts_with("/Game/") }
    fn root() -> PathBuf { PathBuf::from(fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install")) }

    /// The runtime the game builds, without a window: arms + ADS driver + sim + the character's clip library (prototype clips + the loadout's player clips, extended per installed weapon exactly like
    /// `Scene::install_extra_gun` does, base library first).
    struct World { res: Resources, arms: Arms, ads: crate::ads::AdsDriver, sim: GameSim, store: ClipStore, aim: bool }
    impl World {
        fn new(src: &ClipSourceDir) -> World {
            let res = Resources::load(src).unwrap();
            let arms = Arms::new(ArmsData::from_resources(&res).unwrap(), Policy::default());
            let ads = crate::ads::AdsDriver::from_resources(&res).expect("starter ADS profiles");
            let skel = fn_pose::clip::load_player_skeleton_info(src).unwrap();
            let mut store = ClipStore::load(src, &fn_legacy::prototype_clip_names(), &skel, SampleMode::Ue).unwrap();
            res.install_player_clips(&mut store);
            World { res, arms, ads, sim: GameSim::new(), store, aim: false }
        }
        /// One frame of `App::drive_arms` (arms step, ADS step + apply, held core, montages, sim step).
        fn tick(&mut self, fire: bool, reload: bool) -> Vec<ArmsSound> {
            let mut inp = FrameInput::default();
            self.arms.update(DT, &ArmsInput { fire_held: fire, reload_press: reload, ..Default::default() });
            let g = self.arms.gun_index();
            let f = self.ads.step(DT, self.aim, false, g, g.is_some(), 1., 1.);
            self.ads.apply(&f, &mut self.sim, &mut inp, true);
            self.arms.set_aiming(f.aiming);
            self.sim.set_held(self.arms.held_core().map(|(c, _)| (c, f.aiming)));
            self.sim.arms = self.arms.montages();
            self.sim.step(&inp, DT);
            self.arms.take_sounds()
        }
        fn run(&mut self, secs: f32, fire: bool) -> Vec<ArmsSound> { let mut v = vec![]; for _ in 0..(secs / DT).round() as u32 { v.extend(self.tick(fire, false)); } v }
        /// Clip names the pose tree reads this frame that the library lacks.
        fn missing_clips(&self) -> Vec<String> {
            let mut names = vec![]; self.sim.pose_tree().clip_names(&mut names);
            names.sort(); names.dedup();
            names.into_iter().filter(|n| !n.is_empty() && !self.store.clips.contains_key(n)).collect()
        }
        fn install(&mut self, src: &ClipSourceDir, wid: &str) -> Result<usize, String> {
            let pack = loadout_gun::load_gun_pack(src, wid, &self.res.known(), &self.res.player_hand_socket)?;
            let hit = fn_hitreg::load(src, wid).map_err(|e| format!("hit data: {e}"))?;
            let store = &mut self.store;
            let (inst, _) = install(&mut self.res, &mut self.arms, Some(&mut self.ads), Built { pack, hit, secs: 0. }, &mut |res, i| {
                let (extra, _) = res.new_player_clips(&i.new_sequences, &i.new_cores);
                for (k, v) in extra.clips { store.clips.entry(k).or_insert(v); }
                Ok(())
            }, true)?;
            Ok(inst.gun)
        }
    }
    /// The real path: `request_give` -> worker thread (pak read, hit data) -> `poll` installs on the calling thread, puts the weapon on the quickbar and equips it; a second request refills and re-equips.
    #[test]
    fn request_give_loads_on_a_worker_and_poll_installs_and_equips() {
        let set = fn_pak::PakSet::open_install(root()).unwrap();
        let src = ClipSourceDir::Pak(&set);
        let mut w = World::new(&src);
        let mut rt = WeaponRuntime::new();
        rt.set_install(Some(root()));
        let guns0 = w.arms.data.guns.len();
        let wid = rt.request_give("WID_Sniper_Standard_Scope_Athena_SR_Ore_T03", true).unwrap();
        let t = std::time::Instant::now();
        let mut calls = 0;
        let installed = loop {
            let v = rt.poll(&mut w.res, &mut w.arms, Some(&mut w.ads), &mut |_, i| { calls += 1; assert_eq!(i.gun, guns0); Ok(()) });
            if !v.is_empty() { break v; }
            assert!(t.elapsed().as_secs() < 120, "worker did not finish: {:?}", rt.take_events());
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        eprintln!("worker load + install of {wid}: {:.2}s wall; events {:?}; timings {:?}", t.elapsed().as_secs_f32(), rt.take_events(), rt.timings);
        assert_eq!(calls, 1);
        let i = &installed[0];
        assert_eq!((i.gun, i.wid.as_str(), i.selected), (guns0, wid.as_str(), true));
        assert_eq!(w.arms.gun_index(), Some(guns0), "equipped");
        assert!(i.catalog.is_some() && w.arms.data.guns.len() == guns0 + 1 && w.res.gun_count() == guns0 + 1 && w.ads.profiles.len() == guns0 + 1);
        // again: installed already -> refill + equip, nothing is loaded
        w.arms.mags[guns0] = 0;
        rt.request_equip(&wid).unwrap();
        let again = rt.poll(&mut w.res, &mut w.arms, Some(&mut w.ads), &mut |_, _| panic!("installed twice"));
        assert_eq!(again.len(), 1);
        assert!(again[0].catalog.is_none() && w.arms.mags[guns0] == w.arms.data.guns[guns0].def.clip_size);
        // unknown weapon: an error line, nothing queued
        assert!(rt.request_give("WID_Does_Not_Exist", true).is_err());
        // the display-name catalog fills in from the background read
        let t = std::time::Instant::now();
        while rt.catalog().iter().all(|c| c.display_name.is_none()) && t.elapsed().as_secs() < 60 { rt.poll(&mut w.res, &mut w.arms, None, &mut |_, _| Ok(())); std::thread::sleep(std::time::Duration::from_millis(20)); }
        let named = rt.catalog().iter().filter(|c| c.display_name.is_some()).count();
        eprintln!("display names read for {named} of 45 weapons: {:?}", rt.catalog().iter().take(6).map(|c| (&c.short, &c.display_name, c.rarity)).collect::<Vec<_>>());
        assert_eq!(named, 45);
    }
    #[test]
    fn queries_resolve_by_asset_name_or_display_name_and_ambiguity_is_reported() {
        let mut rt = WeaponRuntime::new();
        assert_eq!(rt.catalog().len(), 45);
        assert_eq!(rt.resolve("wid_assault_auto_athena_r_ore_t03").unwrap(), format!("{WID_DIR}WID_Assault_Auto_Athena_R_Ore_T03"));
        assert_eq!(rt.resolve("Assault_Auto_Athena_R_Ore_T03").unwrap(), format!("{WID_DIR}WID_Assault_Auto_Athena_R_Ore_T03"));
        assert_eq!(rt.resolve(&format!("{WID_DIR}WID_Assault_Auto_Athena_R_Ore_T03.WID_Assault_Auto_Athena_R_Ore_T03")).unwrap(), format!("{WID_DIR}WID_Assault_Auto_Athena_R_Ore_T03"));
        assert!(rt.resolve("assault").unwrap_err().contains("no Battle Royale weapon"));
        for w in rt.catalog.iter_mut() { w.display_name = Some(if w.short.contains("Assault_Auto_Athena") { "Assault Rifle".to_owned() } else { w.short.clone() }); }
        let e = rt.resolve("assault rifle").unwrap_err();
        assert!(e.contains("matches 3 weapons") && e.contains("Assault_Auto_Athena_UC_Ore_T03"), "{e}");
        assert_eq!(rt.resolve_all("ASSAULT RIFLE").len(), 3);
        assert!(rt.request_give("nothing", true).is_err() && !rt.busy());
        assert!(rt.request_equip("WID_Sniper_Standard_Scope_Athena_SR_Ore_T03").is_ok() && rt.busy());
    }
    fn ids(events: &[crate::audio::SoundEvent]) -> String { events.iter().map(|e| format!("{e:?}")).collect::<Vec<_>>().join(" | ") }

    /// EVERY one of the 45 BR firearms, equipped in a sim: equip montage -> fire (hip) -> reload -> ADS -> fire (ADS). Asserts no panic, that the montages the weapon's own Blueprint chain names are the ones
    /// requested (player montage on the sim, weapon-mesh clip from the weapon's own skeleton library), that every clip the pose tree reads exists in the character's library, and that the sounds the data
    /// defines (fire / loop / reload stages / out-of-ammo) reach the audio mapping. Data the weapon lacks is printed as `DATA-ABSENT`, never substituted.
    #[test]
    fn every_br_weapon_equips_fires_reloads_and_aims() {
        let set = fn_pak::PakSet::open_install(root()).unwrap();
        let src = ClipSourceDir::Pak(&set);
        let mut w = World::new(&src);
        let (w0, p0) = (glam::Vec3::new(1., 1., 1.), glam::Vec3::ZERO);
        let mut problems: Vec<String> = vec![];
        let mut absent: Vec<String> = vec![];
        let mut report: Vec<String> = vec![];
        let t_all = std::time::Instant::now();
        for row in BR_WEAPON_ASSETS {
            let wid = format!("{WID_DIR}{}", row.wid);
            let name = row.wid.trim_start_matches("WID_").trim_end_matches("_Ore_T03").trim_end_matches("_Ore_T02");
            let g = match w.res.gun_index_of(&wid) { Some(g) => g, None => match w.install(&src, &wid) { Ok(g) => g, Err(e) => { problems.push(format!("{name}: install failed: {e}")); continue; } } };
            let mut bad = |what: String| problems.push(format!("{name}: {what}"));
            let gun = w.arms.data.guns[g].clone();
            assert_eq!(gun.def.item, wid);
            // ---- equip ----
            w.aim = false; w.arms.mags[g] = gun.def.clip_size; w.sim = GameSim::new();
            w.arms.update(0., &ArmsInput { give_ammo: Some((g, 500)), ..Default::default() });
            if let Err(e) = w.arms.give_gun(g, true) { bad(format!("give_gun: {e}")); continue; }
            if w.arms.gun_index() != Some(g) { bad("not held after give_gun(select)".into()); continue; }
            if w.arms.held_core().map(|c| c.0) != Some(gun.core) { bad(format!("held core {:?} != {}", w.arms.held_core(), gun.core)); }
            let equip_clips: Vec<&str> = gun.equip.segs.iter().map(|s| s.clip.as_str()).collect();
            let mut equip_seen = false; let mut sounds = vec![];
            for _ in 0..(gun.equip.length / DT).ceil() as u32 + 6 {
                sounds.extend(w.tick(false, false));
                if w.sim.arms.iter().any(|m| equip_clips.contains(&m.clip.as_str())) { equip_seen = true; }
            }
            if !equip_seen { bad(format!("equip montage {:?} never reached the sim", equip_clips)); }
            let equip_fx = gun.equip.fx.len();
            let equip_sounds = sounds.iter().filter(|s| matches!(s, ArmsSound::Notify { .. } | ArmsSound::ReloadStage { .. })).count();
            if equip_sounds != equip_fx { bad(format!("equip montage has {equip_fx} sound notifies, {equip_sounds} sounds were requested")); }
            if equip_fx == 0 { absent.push(format!("{name}: equip montage carries no sound notify")); }
            let miss = w.missing_clips();
            if !miss.is_empty() { bad(format!("pose tree reads clips the library lacks (hip): {miss:?}")); }
            // ---- fire (hip) ----
            let bps: Vec<Option<String>> = w.arms.data.guns.iter().map(|g| g.def.assets.actor.clone()).collect();
            let shots0 = w.arms.shots[g];
            let mut fire_sounds = vec![]; let mut fire_montage = false; let mut weapon_clip = None;
            for _ in 0..(1.5 / DT) as u32 {
                fire_sounds.extend(w.tick(true, false));
                if w.sim.arms.iter().any(|m| gun.fire.segs.iter().any(|s| s.clip == m.clip)) { fire_montage = true; }
                if !w.arms.is_reloading() { if let Some((_, c, _)) = w.arms.weapon_clip() { weapon_clip = Some(c); } }
            }
            for _ in 0..20 { // the trigger is let go: OnRelease weapons fire now
                fire_sounds.extend(w.tick(false, false));
                if w.sim.arms.iter().any(|m| gun.fire.segs.iter().any(|s| s.clip == m.clip)) { fire_montage = true; }
                if !w.arms.is_reloading() { if let Some((_, c, _)) = w.arms.weapon_clip() { weapon_clip = Some(c); } }
            }
            let shots = w.arms.shots[g] - shots0;
            if shots == 0 { bad(format!("pulled the trigger for 1.5 s: no shot (trigger {:?}, clip {}, rate {})", gun.def.trigger, gun.def.clip_size, gun.def.firing_rate)); }
            else {
                if !fire_montage { bad(format!("fire montage {:?} never reached the sim", gun.fire.segs.iter().map(|s| &s.clip).collect::<Vec<_>>())); }
                let persistent = we::bp_persistent_fire(row.actor);
                let want_sound = set_ok(row.fire_sound[0]);
                let ev: Vec<_> = fire_sounds.iter().flat_map(|s| crate::audio::arms_to_sounds(s, &bps, false, w0, p0)).collect();
                if want_sound && ev.is_empty() { bad(format!("the data names a fire sound ({}) but no sound event was requested ({fire_sounds:?})", row.fire_sound[0])); }
                if !want_sound { absent.push(format!("{name}: PrimaryFireSound not serialized: silent shot")); }
                if want_sound && !persistent { if let Some(id) = we::bp_event(row.actor, "fire") { if !ids(&ev).contains(id) { bad(format!("fire event {id} missing from {}", ids(&ev))); } } }
                if persistent && want_sound && !ev.iter().any(|e| format!("{e:?}").starts_with("WeaponLoop")) { bad(format!("persistent-fire weapon requested no fire loop: {}", ids(&ev))); }
                match (&gun.weapon_fire, &weapon_clip) {
                    (Some(_), None) => bad("the weapon has a fire montage but the held mesh got no clip".into()),
                    (None, Some(c)) => bad(format!("the data has no weapon fire montage but a clip {c} was requested")),
                    (None, None) => absent.push(format!("{name}: no WeaponFireMontage in the data: the mesh does not animate when firing")),
                    (Some(_), Some(c)) => { match w.res.gun_clips(g) { Ok(lib) => if !lib.clips.contains_key(c) { bad(format!("weapon fire clip {c} is not in the weapon's own skeleton library")); }, Err(e) => bad(format!("weapon clips: {e}")) } }
                }
            }
            let miss = w.missing_clips();
            if !miss.is_empty() { bad(format!("pose tree reads clips the library lacks (firing): {miss:?}")); }
            // out of ammo: the click, when the data names one
            w.run(1.0, false);
            w.arms.mags[g] = 0; w.arms.reserve.iter_mut().for_each(|r| *r = 0); w.arms.update(0., &ArmsInput::default());
            let empty: Vec<_> = w.run(0.1, true).iter().filter(|s| matches!(s, ArmsSound::Empty { .. })).flat_map(|s| crate::audio::arms_to_sounds(s, &bps, false, w0, p0)).collect();
            w.run(0.2, false);
            if set_ok(row.out_of_ammo) { if empty.is_empty() { bad("the data names an out-of-ammo sound; none requested".into()); } } else { absent.push(format!("{name}: OutOfAmmoSound not serialized")); }
            // ---- reload ----
            for _ in 0..600 { if !w.arms.is_reloading() { break; } w.tick(false, false); } // an auto reload armed by the last shot finishes first
            w.run(0.5, false);
            w.arms.update(0., &ArmsInput { give_ammo: Some((g, 500)), ..Default::default() });
            w.arms.mags[g] = 0;
            let mut rsounds = vec![]; let mut reload_seen = false; let mut weapon_reload = false;
            rsounds.extend(w.tick(false, true));
            if !w.arms.is_reloading() { bad(format!("reload press did not start a reload (mag {} reserve {})", w.arms.mags[g], w.arms.reserve[g])); }
            let reload_clips: Vec<&str> = gun.reload.segs.iter().map(|s| s.clip.as_str()).collect();
            for _ in 0..((gun.def.reload_time + 1.0) / DT) as u32 {
                rsounds.extend(w.tick(false, false));
                if w.sim.arms.iter().any(|m| reload_clips.contains(&m.clip.as_str())) { reload_seen = true; }
                if w.arms.weapon_clip().is_some() { weapon_reload = true; }
                if !w.arms.is_reloading() { break; }
            }
            if !reload_seen { bad(format!("reload montage {reload_clips:?} never reached the sim")); }
            if w.arms.is_reloading() { bad("the reload never finished".into()); }
            if w.arms.mags[g] == 0 { bad("the reload put no ammo in the magazine".into()); }
            match (&gun.weapon_reload, weapon_reload) { (Some(_), false) => bad("the weapon has a reload montage but the mesh got no clip".into()), (None, true) => bad("no WeaponReloadMontage in the data but a clip was requested".into()), (None, false) => absent.push(format!("{name}: no WeaponReloadMontage in the data")), _ => {} }
            let stages: Vec<we::ReloadStage> = rsounds.iter().filter_map(|s| if let ArmsSound::ReloadStage { stage, .. } = s { Some(*stage) } else { None }).collect();
            let has_fx = gun.reload.fx.iter().any(|f| matches!(f.fx, crate::arms::Fx::Reload(_)));
            if has_fx && stages.is_empty() { bad(format!("the reload montage carries PlayReloadFX notifies but no reload sound stage was requested: fx {:?}, montage length {}, ReloadTime {}, segs {:?}", gun.reload.fx, gun.reload.length, gun.def.reload_time, gun.reload.segs.iter().map(|s| (s.start, s.anim_start, s.anim_end, s.rate, s.loops)).collect::<Vec<_>>())); }
            if !has_fx { absent.push(format!("{name}: the reload montage carries no PlayReloadFX notify: silent reload")); }
            for st in &stages {
                let i = match st { we::ReloadStage::Start => 0, we::ReloadStage::Cartridge => 1, we::ReloadStage::End => 2 };
                let ev = crate::audio::arms_to_sounds(&ArmsSound::ReloadStage { gun: g, stage: *st }, &bps, false, w0, p0);
                if set_ok(row.reload_sound[i]) && ev.is_empty() { bad(format!("ReloadSounds[{i}] is set but the {st:?} stage made no event")); }
                if !set_ok(row.reload_sound[i]) { absent.push(format!("{name}: ReloadSounds[{i}] absent in the data, the {st:?} stage is silent")); }
            }
            // ---- ADS ----
            w.aim = true; w.run(2.0, false);
            let prof = w.ads.profiles[g].clone();
            if !w.ads.machine.is_targeting() { bad("the ADS machine is not targeting after holding Target for 2 s".into()); }
            if w.sim.ads.weight < 0.99 { bad(format!("ADS camera weight {} after 2 s", w.sim.ads.weight)); }
            if (w.sim.ads.fov - prof.targeting.fov).abs() > 0.5 { bad(format!("ADS FOV {} != the weapon's targeting camera FOV {}", w.sim.ads.fov, prof.targeting.fov)); }
            let miss = w.missing_clips();
            if !miss.is_empty() { bad(format!("pose tree reads clips the library lacks (aiming): {miss:?}")); }
            w.arms.mags[g] = gun.def.clip_size;
            w.run(1.5, false);
            let before = w.arms.shots[g];
            let mut ads_player = false; let mut ads_weapon = None;
            for _ in 0..(1.5 / DT) as u32 {
                w.tick(true, false);
                let want: Vec<&str> = gun.fire_ads.as_ref().unwrap_or(&gun.fire).segs.iter().map(|s| s.clip.as_str()).collect();
                if w.sim.arms.iter().any(|m| want.contains(&m.clip.as_str())) { ads_player = true; }
                if !w.arms.is_reloading() { if let Some((_, c, _)) = w.arms.weapon_clip() { ads_weapon = Some(c); } }
            }
            w.run(0.4, false); // release (OnRelease weapons fire here)
            if w.arms.shots[g] == before { bad("no shot while aiming".into()); }
            else {
                if !ads_player { bad("the ADS fire montage never reached the sim".into()); }
                if gun.fire_ads.is_none() { absent.push(format!("{name}: no player FireDownsightsAnimation")); }
                if gun.weapon_fire_ads.is_none() && gun.weapon_fire.is_some() { absent.push(format!("{name}: no WeaponFireDownsightsMontage in the data: the hip montage is used while aiming (existing starter rule, INFERENCE)")); }
                if let (Some(a), Some(wf)) = (&ads_weapon, gun.weapon_fire_ads.as_ref().or(gun.weapon_fire.as_ref())) { if !wf.segs.iter().any(|s| &s.clip == a) { bad(format!("aimed weapon clip {a} is not in its fire montage")); } }
            }
            w.aim = false; w.run(1.0, false);
            report.push(format!("{name:48} core {:15} trigger {:?} clip {:3} equip {:.2}s fire {:.2}s reload {:.2}s | weapon-mesh montages fire {} ads {} reload {}", gun.core, gun.def.trigger, gun.def.clip_size, gun.equip.length, gun.fire.length, gun.reload.length, gun.weapon_fire.is_some(), gun.weapon_fire_ads.is_some(), gun.weapon_reload.is_some()) + &format!(" | cartridges {} burst rate {} charge (min,max,down) {:?}", gun.def.cartridges, gun.cadence.burst_firing_rate, w.res.gun_extras(g).map(|e| e.charge)));
        }
        for r in &report { eprintln!("{r}"); }
        for a in &absent { eprintln!("DATA-ABSENT {a}"); }
        for p in &problems { eprintln!("PROBLEM {p}"); }
        eprintln!("{} weapons stepped in {:.1}s", BR_WEAPON_ASSETS.len(), t_all.elapsed().as_secs_f32());
        assert!(problems.is_empty(), "{} problems", problems.len());
    }
}
