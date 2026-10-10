use crate::audio::SoundEvent;
use crate::lobby_locker::LockerGlue;
use crate::lobby_mm::Matchmaking;
use crate::lobby_profile::Profile;
use crate::lobby_scene::{HeroReady, LobbyAssets, LobbyScene, LobbyScreen, ScreenBackdrops};
use crate::lobby_shop::{ShopEvent, ShopScreen};
use crate::lobby_ui::{Controller, Frame, LobbyState, LobbyUi, Pad, PadKind, UiEvent};
use fn_ui::draw::RenderList;
use fn_ui::layout::Rect;
use fn_ui_wgpu::UiRenderer;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;

struct CpuReady { set: Arc<fn_pak::PakSet>, glue: LockerGlue, profile: Profile, assets: LobbyAssets }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LobbyOut { None, MatchReady }

pub fn sound_event(cue: &str, hover: bool) -> Option<SoundEvent> {
    use fn_audio_sink as s;
    if hover {
        match cue { s::TOP_BAR_HOVER_CUE => Some(SoundEvent::TopBarHover), s::LOCKER_ITEM_HOVER_CUE => Some(SoundEvent::LockerItemHover), _ => None }
    } else {
        match cue { s::TOP_BAR_PRESS_CUE => Some(SoundEvent::TopBarPress), s::LOBBY_PLAY_PRESS_CUE => Some(SoundEvent::LobbyPlayPress), s::LOCKER_ITEM_PRESS_CUE => Some(SoundEvent::LockerItemPress), _ => None }
    }
}

pub fn season_xp_for_level(level: i32) -> i32 {
    match level {
        1 => 100, 2 => 250, 3 => 300, 4 => 350,
        5..=14 => 700,
        15..=25 => 1700,
        l => 1800 + (l - 26).max(0) * 100,
    }
}

pub const SEASON_MAX_LEVEL: i32 = 100;
pub const SEASON_CHASE_REWARD_LEVELS: [i32; 2] = [10, 25];

pub struct LobbyHost {
    install: PathBuf,
    cpu_rx: Option<Receiver<Result<CpuReady, String>>>,
    set: Option<Arc<fn_pak::PakSet>>,
    glue: Option<LockerGlue>,
    profile: Option<Profile>,
    assets: Option<LobbyAssets>,
    ui: Option<LobbyUi>,
    renderer: Option<UiRenderer>,
    scene: Option<LobbyScene>,
    shop: Option<ShopScreen>,
    shop_tried: bool,
    backdrops: ScreenBackdrops,
    backdrop_job: Option<(&'static str, Receiver<Result<LobbyAssets, String>>)>,
    pub failed: Option<String>,
    controller: Controller,
    pub state: LobbyState,
    frame: Option<Frame>,
    mm: Option<Matchmaking>,
    hero_job: Option<Receiver<Result<crate::lobby_scene::HeroParts, String>>>,
    hero_queued: bool,
    t: f32,
    cursor: (f32, f32),
    name: String,
    sounds: Vec<SoundEvent>,
    quests: Vec<fn_ui::screens::athena_lobby::QuestRow>,
    fx: Option<crate::lobby_fx::LobbyFx>,
    quests_loaded: bool,
    fx_job: Option<Receiver<crate::lobby_fx::LobbyFxAssets>>,
}

impl LobbyHost {
    pub fn start(install: &Path, name: String) -> LobbyHost {
        let (tx, rx) = channel();
        let inst = install.to_path_buf();
        let spawned = std::thread::Builder::new().name("lobby-load".into()).spawn(move || {
            let r = (|| -> Result<CpuReady, String> {
                let set = Arc::new(fn_pak::PakSet::open_install(&inst).map_err(|e| format!("cannot open the paks under {}: {e}", inst.display()))?);
                let glue = LockerGlue::load(&set);
                let profile = Profile::load_or_new(&crate::lobby_profile::default_profile_path(), glue.default_loadout());
                let hero = glue.equipped_hero(&profile);
                let assets = LobbyAssets::load(&inst, Some(&hero))?;
                Ok(CpuReady { set, glue, profile, assets })
            })();
            let _ = tx.send(r);
        });
        let failed = spawned.err().map(|e| e.to_string());
        LobbyHost {
            install: install.to_path_buf(), cpu_rx: Some(rx), set: None, glue: None, profile: None, assets: None, ui: None, renderer: None, scene: None, shop: None, shop_tried: false,
            backdrops: ScreenBackdrops::new(), backdrop_job: None, failed, controller: Controller::new(), state: LobbyState::default(), frame: None, mm: None, hero_job: None, hero_queued: false,
            t: 0.0, cursor: (0.0, 0.0), name, sounds: vec![], quests: vec![], fx: None, fx_job: None, quests_loaded: false,
        }
    }

    pub fn ready(&self) -> bool { self.ui.is_some() && self.renderer.is_some() && self.scene.is_some() && self.profile.is_some() && self.quests_loaded && self.shop_tried }
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_owned();
        if let Some(p) = self.profile.as_mut() { if p.display_name() != Some(name) && !name.is_empty() { p.set_display_name(name); let _ = p.flush(); } }
    }
    pub fn take_sounds(&mut self) -> Vec<SoundEvent> { std::mem::take(&mut self.sounds) }
    pub fn equipped_hero(&self) -> Option<String> { match (self.glue.as_ref(), self.profile.as_ref()) { (Some(g), Some(p)) => Some(g.equipped_hero(p)), _ => None } }

    pub fn poll_load(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) {
        if self.failed.is_some() { return; }
        if let Some(rx) = self.cpu_rx.as_ref() {
            match rx.try_recv() {
                Ok(Ok(c)) => {
                    self.cpu_rx = None;
                    self.set = Some(c.set); self.glue = Some(c.glue); self.profile = Some(c.profile); self.assets = Some(c.assets);
                    if let Some(n) = self.profile.as_ref().and_then(|p| p.display_name()) { self.name = n.to_owned(); }
                }
                Ok(Err(e)) => { self.cpu_rx = None; self.failed = Some(e); return; }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => { self.cpu_rx = None; self.failed = Some("the lobby loader stopped".into()); return; }
            }
        }
        if self.renderer.is_none() { self.renderer = Some(UiRenderer::with_device(device.clone(), queue.clone(), format)); return; }
        if self.ui.is_none() {
            match LobbyUi::load(&self.install) { Ok(u) => self.ui = Some(u), Err(e) => self.failed = Some(format!("lobby UI: {e}")) }
            return;
        }
        if self.scene.is_none() {
            if let Some(a) = self.assets.take() {
                match LobbyScene::from_assets(a, device, queue, format) {
                    Ok(s) => {
                        for (m, why) in &s.report.substituted { eprintln!("[lobby] material not original: {m}: {why}"); }
                        if fn_live::var("FN_LOBBY_FX").map_or(true, |v| v != "0") {
                            if let Some(set) = self.set.clone() {
                                let data = s.data.clone();
                                let (tx, rx) = channel();
                                let _ = std::thread::Builder::new().name("lobby-fx".into()).spawn(move || { let _ = tx.send(crate::lobby_fx::load_assets(&set, &data)); });
                                self.fx_job = Some(rx);
                            }
                        }
                        self.scene = Some(s);
                    }
                    Err(e) => self.failed = Some(format!("lobby scene: {e}")),
                }
            }
            return;
        }
        if !self.quests_loaded && self.set.is_some() {
            self.quests_loaded = true;
            let locres = self.ui.as_ref().and_then(|u| u.locres());
            self.quests = load_daily_quests(self.set.as_deref(), locres.clone());
            if let Some(l) = locres.as_ref() {
                let g = l.get("FriendsList", "Global").unwrap_or("").to_owned();
                let hint = l.get("", "LostConnection").unwrap_or("").to_owned();
                if !g.is_empty() { self.state.chat = Some((g, hint)); }
            }
            return;
        }
        if !self.shop_tried {
            self.shop_tried = true;
            match fn_ui::screens::athena_lobby::store::Store::open(&self.install).map(std::rc::Rc::new).and_then(ShopScreen::load) {
                Ok(s) => self.shop = Some(s),
                Err(e) => eprintln!("[lobby] item shop unavailable: {e}"),
            }
            if let (Some(glue), Some(profile)) = (self.glue.as_ref(), self.profile.as_ref()) {
                glue.sync(profile, &mut self.state.locker);
                self.state.mtx_balance = profile.balance();
            }
        }
    }

    pub fn warm(&mut self, w: u32, h: u32) {
        let (Some(scene), Some(ui), Some(renderer)) = (self.scene.as_mut(), self.ui.as_mut(), self.renderer.as_mut()) else { return };
        let device = scene_device(scene);
        let tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("lobby warm target"), size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: scene_format(scene), usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        scene.render(&view, w, h, 0.0);
        let st = LobbyState { mtx_balance: self.state.mtx_balance, ..Default::default() };
        let frame = ui.draw(w as f32, h as f32, &st);
        renderer.render_texture(&frame.list, &mut ui.ctx_mut().assets, &tex, w, h, None);
    }

    fn shop_tab(&self) -> Option<usize> { self.shop.as_ref().map(|s| s.ui.item_shop_tab) }
    fn on_shop(&self) -> bool { self.shop_tab() == Some(self.state.selected_tab) }

    fn set_tab(&mut self, i: usize) {
        self.state.selected_tab = i;
        self.state.locker.picking = None;
        self.state.locker.picker_t = 0.0;
        if self.shop_tab() == Some(i) { if let Some(s) = self.shop.as_mut() { s.open(); } }
        self.backdrops.set_screen(LobbyScreen::from_tab_index(i));
    }

    fn poll_fx(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(rx) = self.fx_job.as_ref() else { return };
        match rx.try_recv() {
            Ok(a) => {
                self.fx_job = None;
                let eye = self.scene.as_ref().map_or(glam::Vec3::ZERO, |s| s.camera_xf().t);
                let fx = crate::lobby_fx::LobbyFx::new(device, queue, &self.install, a, eye);
                eprintln!("[lobby fx] {} particle systems in the lobby", fx.count());
                self.fx = Some(fx);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.fx_job = None,
        }
    }

    fn poll_backdrop(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) {
        if let Some((cam, rx)) = self.backdrop_job.as_ref() {
            match rx.try_recv() {
                Ok(r) => {
                    let cam = *cam;
                    self.backdrop_job = None;
                    match r.and_then(|a| LobbyScene::from_assets(a, device, queue, format)) {
                        Ok(s) => self.backdrops.insert(cam, s),
                        Err(e) => eprintln!("[lobby] backdrop {cam}: {e}"),
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.backdrop_job = None,
            }
            return;
        }
        if let Some(cam) = self.backdrops.pending() {
            let (tx, rx) = channel();
            let inst = self.install.clone();
            let ch = if cam == "Cosmetics" { self.scene.as_ref().and_then(|s| s.share_character()) } else { None };
            let _ = std::thread::Builder::new().name("lobby-backdrop".into()).spawn(move || { let _ = tx.send(LobbyAssets::load_backdrop(&inst, cam, ch)); });
            self.backdrop_job = Some((cam, rx));
        }
    }

    fn request_hero(&mut self) {
        let (Some(glue), Some(profile)) = (self.glue.as_ref(), self.profile.as_ref()) else { return };
        let hero = glue.equipped_hero(profile);
        if self.scene.as_ref().and_then(|s| s.hero()) == Some(hero.as_str()) && self.hero_job.is_none() { return; }
        if self.hero_job.is_some() { self.hero_queued = true; return; }
        let Some(set) = self.set.clone() else { return };
        let (tx, rx) = channel();
        let _ = std::thread::Builder::new().name("lobby-hero".into()).spawn(move || { let _ = tx.send(crate::lobby_scene::load_hero_parts(&set, &hero)); });
        self.hero_job = Some(rx);
    }

    fn poll_hero(&mut self) {
        let Some(rx) = self.hero_job.as_ref() else { return };
        let res = match rx.try_recv() { Ok(r) => r, Err(TryRecvError::Empty) => return, Err(TryRecvError::Disconnected) => Err("the outfit loader stopped".into()) };
        self.hero_job = None;
        match res {
            Ok(parts) => {
                if let Some(s) = self.scene.as_mut() { if let Some(b) = s.hero_builder() { let ready: HeroReady = b.build(&parts); s.set_hero_ready(ready); } }
                let screen = self.backdrops.screen();
                self.backdrops = ScreenBackdrops::new();
                self.backdrops.set_screen(screen);
                self.backdrop_job = None;
            }
            Err(e) => eprintln!("[lobby] cannot build the equipped outfit: {e}"),
        }
        if std::mem::take(&mut self.hero_queued) { self.request_hero(); }
    }

    fn handle_events(&mut self) {
        for ev in self.controller.take_events() {
            match ev {
                UiEvent::HoverSound(c) => { if let Some(e) = sound_event(&c, true) { self.sounds.push(e); } }
                UiEvent::PressSound(c) => { if let Some(e) = sound_event(&c, false) { self.sounds.push(e); } }
                UiEvent::TabSelected(i) => self.set_tab(i),
                UiEvent::Play => { if self.mm.is_none() && self.state.can_matchmake { self.mm = Some(Matchmaking::start()); } }
                UiEvent::Cancel => { if self.mm.as_ref().map_or(false, |m| m.can_cancel()) { self.mm = None; } }
                UiEvent::Equip { slot, index } => {
                    let (Some(glue), Some(profile)) = (self.glue.as_ref(), self.profile.as_mut()) else { continue };
                    if glue.equip(profile, slot, &self.state.locker, index).is_some() { let _ = profile.flush(); }
                    glue.sync(profile, &mut self.state.locker);
                    if slot == crate::lobby_ui::LockerSlot::Outfit { self.request_hero(); }
                }
                UiEvent::PickerOpened(_) | UiEvent::PickerClosed | UiEvent::PlaylistChanged(_) | UiEvent::FillChanged(_) => {}
            }
        }
    }

    fn season_view(&self) -> fn_ui::screens::athena_lobby::SeasonView {
        let (level, xp) = self.profile.as_ref().map_or((1, 0), |p| (p.season_level(), p.season_xp()));
        let level = level.clamp(1, SEASON_MAX_LEVEL);
        let chase = SEASON_CHASE_REWARD_LEVELS.iter().copied().find(|&l| l > level);
        fn_ui::screens::athena_lobby::SeasonView { level, max_level: SEASON_MAX_LEVEL, level_xp: xp, level_xp_for_level: season_xp_for_level(level), chase_reward_level: chase }
    }

    pub fn frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat, target: &wgpu::Texture, w: u32, h: u32, dt: f32, load_fraction: f32) -> LobbyOut {
        self.t += dt;
        self.poll_hero();
        self.poll_backdrop(device, queue, format);
        self.poll_fx(device, queue);
        let mut out = LobbyOut::None;
        if let Some(m) = self.mm.as_mut() {
            m.tick(dt, load_fraction);
            if m.done() { out = LobbyOut::MatchReady; }
        }
        let locres = self.ui.as_ref().and_then(|u| u.locres());
        self.state.matchmaking = self.mm.as_ref().map(|m| m.view(locres.as_deref(), load_fraction));
        let ready = self.mm.as_ref().map_or(false, |m| m.local_ready);
        self.state.mat_time = self.t;
        self.state.season = Some(self.season_view());
        self.state.quests = self.quests.clone();
        if let Some(p) = self.profile.as_ref() { self.state.mtx_balance = p.balance(); }
        let on_shop = self.on_shop();
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let (Some(ui), Some(renderer)) = (self.ui.as_mut(), self.renderer.as_mut()) else { return out };
        self.controller.tick(&mut self.state, dt);
        let lobby_cam = self.backdrops.camera() == "HeroSelect";
        let scene_drawn = match self.backdrops.scene_mut() {
            Some(bg) => { bg.render(&view, w, h, self.t); true }
            None => match self.scene.as_mut() {
                Some(scene) => {
                    scene.render_ext(&view, w, h, self.t, self.fx.as_mut().map(|f| f as &mut dyn crate::lobby_scene::LobbyTranslucent));
                    let mut pads = vec![];
                    if self.state.selected_tab == 0 && lobby_cam {
                        for s in scene.party_slots().iter().filter(|s| !s.hidden) {
                            if s.occupied {
                                if let Some([x, y]) = scene.project(s.top_widget_cm, w, h) { pads.push(Pad { x, y, pivot: s.top_pivot, kind: PadKind::Player { name: Some(self.name.clone()), leader: true, ready } }); }
                            } else if let Some([x, y]) = scene.project(s.add_widget_cm, w, h) { pads.push(Pad { x, y, pivot: s.add_pivot, kind: PadKind::Empty }); }
                        }
                    }
                    self.state.pads = pads;
                    true
                }
                None => false,
            },
        };
        if !scene_drawn { self.state.pads.clear(); }
        let clear = if scene_drawn { None } else { Some([0.0, 0.0, 0.0, 1.0]) };
        if on_shop {
            let (Some(shop), Some(profile)) = (self.shop.as_mut(), self.profile.as_ref()) else { return out };
            let mut list = RenderList::new();
            if let Some(c) = self.backdrops.darken() { list.fill(Rect::new(0.0, 0.0, w as f32, h as f32), c); }
            let sf = shop.draw(ui.ctx_mut(), w as f32, h as f32, profile, crate::lobby_shop::now_unix());
            list.extend(sf.list);
            renderer.render_texture(&list, &mut ui.ctx_mut().assets, target, w, h, clear);
            self.frame = None;
        } else {
            let frame = ui.draw(w as f32, h as f32, &self.state);
            renderer.render_texture(&frame.list, &mut ui.ctx_mut().assets, target, w, h, clear);
            self.frame = Some(frame);
        }
        self.handle_events();
        out
    }

    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        if self.on_shop() { if let Some(s) = self.shop.as_mut() { s.pointer_move(x, y); } return; }
        let (Some(ui), Some(frame)) = (self.ui.as_ref(), self.frame.as_ref()) else { return };
        self.controller.pointer_move(ui.inner(), frame, &mut self.state, x, y);
    }

    pub fn pointer_button(&mut self, down: bool) {
        let (x, y) = self.cursor;
        if self.on_shop() {
            if down { return; }
            let now = crate::lobby_shop::now_unix();
            let (Some(shop), Some(profile)) = (self.shop.as_mut(), self.profile.as_mut()) else { return };
            match shop.click(x, y, profile, now) {
                ShopEvent::TabSelected(i) => { self.sounds.push(SoundEvent::TopBarPress); self.set_tab(i); }
                ShopEvent::Bought { .. } => {
                    let _ = profile.flush();
                    if let Some(g) = self.glue.as_ref() { g.sync(profile, &mut self.state.locker); }
                    self.request_hero();
                }
                ShopEvent::Message(m) => eprintln!("[lobby] shop: {m}"),
                ShopEvent::None => {}
            }
            return;
        }
        let (Some(ui), Some(frame)) = (self.ui.as_ref(), self.frame.as_ref()) else { return };
        self.controller.pointer_button(ui.inner(), frame, &mut self.state, x, y, down);
        self.handle_events();
    }

    pub fn wheel(&mut self, notches: f32) {
        if self.on_shop() { return; }
        let rows = if notches < 0.0 { 1 } else if notches > 0.0 { -1 } else { 0 };
        let (x, y) = self.cursor;
        if let (Some(ui), Some(frame)) = (self.ui.as_ref(), self.frame.as_ref()) { self.controller.wheel(ui.inner(), frame, &mut self.state, x, y, rows); }
    }

    pub fn escape(&mut self) {
        if self.on_shop() && self.shop.as_mut().map_or(false, |s| s.back()) { return; }
        let _ = self.controller.cancel(&mut self.state);
    }

    pub fn enter(&mut self) {
        if !self.on_shop() { return; }
        let now = crate::lobby_shop::now_unix();
        if let (Some(shop), Some(profile)) = (self.shop.as_mut(), self.profile.as_mut()) {
            if let ShopEvent::Bought { .. } = shop.confirm(profile, now) {
                let _ = profile.flush();
                if let Some(g) = self.glue.as_ref() { g.sync(profile, &mut self.state.locker); }
                self.request_hero();
            }
        }
    }

    pub fn debug_play(&mut self) { if self.mm.is_none() { self.mm = Some(Matchmaking::start()); } }

    pub fn flush(&mut self) { if let Some(p) = self.profile.as_mut() { let _ = p.flush(); } }
}

fn scene_device(scene: &LobbyScene) -> wgpu::Device { scene.device().clone() }
fn scene_format(scene: &LobbyScene) -> wgpu::TextureFormat { scene.output_format() }

pub const DAILY_QUESTS: [&str; 3] = [
    "/Game/Athena/Items/Quests/DailyQuests/AthenaDailyQuest_InteractAmmoCrate",
    "/Game/Athena/Items/Quests/DailyQuests/AthenaDailyQuest_PlayerElimination",
    "/Game/Athena/Items/Quests/DailyQuests/AthenaDaily_Solo_Top25",
];

fn load_daily_quests(set: Option<&fn_pak::PakSet>, locres: Option<std::rc::Rc<fn_ui::screens::athena_lobby::store::Locres>>) -> Vec<fn_ui::screens::athena_lobby::QuestRow> {
    let Some(set) = set else { return vec![] };
    let text = |v: &serde_json::Value| -> String {
        let key = v.get("key").and_then(|x| x.as_str()).unwrap_or("");
        let ns = v.get("namespace").and_then(|x| x.as_str()).unwrap_or("");
        if key.is_empty() { return v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_owned(); }
        locres.as_ref().and_then(|l| l.get(ns, key)).unwrap_or("").to_owned()
    };
    let mut out = vec![];
    for path in DAILY_QUESTS {
        let pd = match fn_uasset::PackageData::load(set, path) { Ok(p) => p, Err(e) => { eprintln!("[lobby] daily quest {path}: {e}"); continue } };
        let pkg = match pd.package() { Ok(p) => p, Err(e) => { eprintln!("[lobby] daily quest {path}: {e}"); continue } };
        let Some(idx) = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "FortQuestItemDefinition") else { continue };
        let Ok(obj) = pkg.read_export(idx) else { continue };
        let props = obj.props.to_json(&pkg);
        let name = props.get("DisplayName").map(&text).unwrap_or_default();
        let obj0 = props.get("Objectives").and_then(|o| o.as_array()).and_then(|a| a.first()).cloned().unwrap_or(serde_json::Value::Null);
        let task = obj0.get("HudShortDescription").map(&text).unwrap_or_default();
        let required = obj0.get("Count").and_then(|c| c.as_i64()).unwrap_or(1) as i32;
        let reward_xp = props.get("Rewards").and_then(|r| r.as_array()).and_then(|a| a.first()).and_then(|r| r.get("Quantity")).and_then(|q| q.as_i64()).unwrap_or(0) as i32;
        out.push(fn_ui::screens::athena_lobby::QuestRow { name, task, achieved: 0, required, reward_xp });
    }
    eprintln!("[lobby] daily quests: {}", out.iter().map(|q| format!("{} ({}: 0/{}, {} XP)", q.name, q.task, q.required, q.reward_xp)).collect::<Vec<_>>().join(", "));
    out
}
