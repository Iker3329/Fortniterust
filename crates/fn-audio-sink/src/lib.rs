//! `fn_app`'s `AudioSink` seam implemented on `fn-audio`.
//!
//! This crate OWNS the definition of [`AudioSink`] and [`SoundEvent`]; `fn-app` re-exports them
//! (`pub use fn_audio_sink::{AudioSink, SoundEvent};` in `crates/fn-app/src/audio.rs`) so there is exactly one copy.
//!
//! [`EngineSink`] owns an [`EngineHandle`] only (every call from the game loop pushes a lock-free command; the cpal callback
//! does the mixing). The cues are decoded from the install by a [`SinkLoader`], which can run on any thread:
//! [`StreamingSink`] (the game's sink) opens the device immediately and fills the cues in from a worker thread
//! (core gameplay cues first, then the long ambience loops), so nothing ever blocks the game loop and an event whose cue is
//! not loaded yet is simply dropped by the mixer ("unknown cue").
//!
//! Event -> cue mapping (provenance in [`fn_audio::events::TABLE`], assumptions flagged there):
//! `Step{gait}` -> `Athena_Character_{Walk,Sprint,Crouch,Crouch_Sprint}_Footstep_Cue` (2D, the local pawn's own 1P cues),
//! `Jump`/`Land` -> `Athena_Character_{Jump,Land,LandHard}_Cue`, `SwingAt` -> `PickAxe_Swing_Athena_Cue` (3D at the weapon),
//! `Harvest` -> `PickAxe_Impact_Default_Cue` + the surface cue (`Demo_Tree_Impact_Cue`, `Demo_Rock/Wood/Metal_Impact_MLTI_Cue`)
//! + `Demo_Tree_Destroyed_Cue` on the killing hit (3D at the prop), `Equip` -> `PickAxe_Ready_Athena_Cue` +
//! `Player_Equip_Melee_Cue`, menu events -> the `Fort_UI_Sounds` cues, ambience -> `Temperate_Grasslands_Hills_Day_Athena_Cue`.

use fn_audio::events;
use fn_audio::weapon_events;
use fn_audio::library::Library;
use fn_audio::mixer::{EngineHandle, PlayId, PlayParams, Vec3};
use fn_audio::soundclass::{Bus, SoundMix};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

pub mod music; // MUSIC HOOK: front-end music + loading-screen sound mix
mod ambient_rt; // AMBIENCE HOOK: executes the fn_audio::ambient director's acts on the engine
pub use music::MusicCue;

/// Which footstep cue variant plays (`PlayerPawn_Athena_C` `Default{,Fast,Crouched,CrouchSprint}FootstepSound`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    /// `DefaultFootstepSound` (walk / jog).
    Walk,
    /// `DefaultFastFootstepSound` (sprint).
    Sprint,
    /// `DefaultCrouchedFootstepSound`.
    Crouch,
    /// `DefaultCrouchSprintFootstepSound`.
    CrouchSprint,
}

/// What a pickaxe hit struck (selects the surface cue).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Tree,
    Rock,
    Wood,
    Metal,
    /// Only the pickaxe's default impact cue.
    Default,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SoundEvent {
    /// Legacy: walk / sprint footstep (= `Step`).
    Footstep { sprint: bool },
    /// A foot plant (an `AnimNotify_FootStep` of the locomotion clip), 2D.
    Step { gait: Gait },
    Jump,
    /// Touch-down; `speed` = impact speed in m/s (>= [`HARD_LAND_SPEED`] plays the hard-landing cue).
    Land { speed: f32 },
    /// Legacy: swing 1.5 m in front of the listener.
    Swing,
    /// Pickaxe weapon fire (`PrimaryFireSound`) at `pos` (world metres).
    SwingAt { pos: glam::Vec3 },
    /// Legacy: a tree hit 2.5 m in front of the listener.
    HarvestHit,
    /// A pickaxe hit at `pos` (world metres); `destroyed` on the killing hit.
    Harvest { surface: Surface, destroyed: bool, pos: glam::Vec3 },
    /// Pickaxe equipped.
    Equip,
    // CRACK AUDIO HOOK: the weak-spot effect (`WeakSpot_C`) and the per-prop sounds of a harvest hit.
    /// A weak spot was generated at `pos` (world metres): `Crack_Spawn_Cue` (`CrackHitCount` 0).
    CrackSpawn { pos: glam::Vec3 },
    /// A weak spot was hit: `Sound_Crack_Level_0N_Cue` for `level` 0..=8 (clamped) at the spot.
    CrackLevel { level: u8, pos: glam::Vec3 },
    /// A weak spot was hit: `Crack_Impact_Cue` at the spot.
    CrackImpact { pos: glam::Vec3 },
    /// A pickaxe hit on a prop whose `props.json` names its sounds: the pickaxe impact + the prop's `PlayHitSound`, plus its `DeathSound` when `destroyed`.
    PropHit { sound: &'static fn_audio::events::PropSound, destroyed: bool, pos: glam::Vec3 },
    // WEAPON AUDIO HOOK: the firearms / consumables of `fn_audio::weapon_events::TABLE` (fire, empty, reload stages, equip, heal, pickup, bullet impact, whip).
    /// A weapon-table sound (`id` = a `weapon_events::TABLE` event id) at `pos` (world metres; `None` = 2D at the listener) with the call's
    /// `volume` multiplier (1.0 except the bullet whip, which scales by closeness). The pitch of the row (the `PitchMultiplier` of the
    /// `AnimNotify_PlaySound` that plays it) is applied.
    Weapon { id: &'static str, pos: Option<glam::Vec3>, volume: f32 },
    /// FIRE LOOP: `AFortWeapon::PlayWeaponFireFX` of a persistent-fire weapon (exe vfunc_201 RVA 0x9355d0): starts the weapon-table sound `id` on the fire audio channel `key`
    /// (gun * 3 + channel) ONCE (it keeps running, looping when its cue loops indefinitely); a still-running sound of the same `key` is replaced.
    WeaponLoop { id: &'static str, key: u32, pos: Option<glam::Vec3>, volume: f32 },
    /// FIRE LOOP stop: `StopWeaponFireFX` (exe vfunc_202 RVA 0x93e020): fades the channel `key` out over `fade` seconds (`PrimaryFireSoundFadeOutTime`). The stop sounds are ordinary `Weapon` events.
    WeaponLoopStop { key: u32, fade: f32 },
    MenuOpen,
    MenuClose,
    MenuClick,
    /// `TopBar.Social`'s serialized hover override.
    TopBarHover,
    /// `TopBar.Social`'s serialized press override.
    TopBarPress,
    /// `AthenaMatchmakingWidget.PlayButton`'s serialized hover override.
    LobbyPlayHover,
    /// `AthenaMatchmakingWidget.PlayButton`'s serialized press override.
    LobbyPlayPress,
    /// `AthenaCustomizationPickerTileButton`'s serialized hover override.
    LockerItemHover,
    /// `AthenaCustomizationPickerTileButton`'s serialized press override.
    LockerItemPress,
}

pub trait AudioSink {
    /// A gameplay / UI moment (called from the game loop thread; must not block).
    fn event(&mut self, _event: SoundEvent) {}
    /// Linear listener transform, once per frame (metres, world space, y up).
    fn listener(&mut self, _position: glam::Vec3, _forward: glam::Vec3, _up: glam::Vec3) {}
    /// Audio-tab volumes 0..1 (master, music, sfx, dialog, voice chat).
    fn set_volumes(&mut self, _master: f32, _music: f32, _sfx: f32, _dialog: f32, _voice_chat: f32) {}
    /// Focus loss / minimise mutes, regain unmutes.
    fn set_suspended(&mut self, _suspended: bool) {}
    /// Start / stop the in-world ambience bed (starts as soon as its waves are decoded).
    fn set_ambience(&mut self, _on: bool) {}
    /// Audio-tab Quality (`true` = High): `SoundNodeQualityLevel` child selection.
    fn set_quality(&mut self, _high: bool) {}
    /// MUSIC HOOK: front-end music (`Fort_Entry_Music_Controller_BP`): `Some(track)` starts it (the previous one fades out over 1.0 s), `None` fades the current one out.
    fn set_music(&mut self, _track: Option<MusicCue>) {}
    /// MUSIC HOOK: a loading screen is up (`DefaultGameData.LoadingScreenSoundMix` = `Fort_Loading_Mix`: SFX, Voice and Music to 0; 0.5 s in, 1.0 s out).
    fn set_loading_screen(&mut self, _on: bool) {}
    /// AMBIENCE HOOK: where the level's ambient emitters stand in the game world: `center_cm` = the UE world position (cm) the game's layout is centred on,
    /// `anchor_m` = where that centre sits in game metres (`audio = anchor + (ue.x - c.x, ue.z - c.z, ue.y - c.y)`). Call once; the default maps UE to `(x, z, y)`.
    fn ambient_map(&mut self, _center_cm: [f32; 3], _anchor_m: glam::Vec3) {}
    /// AMBIENCE HOOK: once per frame while playing. `pawn` = the pawn's location (game metres), `forward` its horizontal forward; `trace(from_m, dir, len_m)` is a
    /// line trace against world geometry that ignores pawns (used by the bed's "surrounded" test, a few times per 1.5 s).
    fn ambient_frame(&mut self, _dt: f32, _pawn: glam::Vec3, _forward: glam::Vec3, _trace: &mut dyn FnMut(glam::Vec3, glam::Vec3, f32) -> bool) {}
}

/// Drops everything.
#[derive(Default)]
pub struct NullSink;
impl AudioSink for NullSink {}

/// Vertical landing speed (m/s) from which the hard-landing cue plays. ASSUMED: the native threshold is not in the
/// install (a normal jump lands at about 6 m/s here).
pub const HARD_LAND_SPEED: f32 = 9.0;
/// Assumed distance (cm) in front of the camera of the legacy `Swing` / `HarvestHit` events.
const SWING_DISTANCE_CM: f32 = 150.0;
/// Waves of at least this many seconds (ambience beds, music) are streamed; shorter one-shots are decoded whole.
const STREAM_MIN_SECONDS: f64 = 4.0;
const HARVEST_DISTANCE_CM: f32 = 250.0;

fn cue_of(id: &str) -> &'static str {
    events::lookup(id).map(|e| e.cue).unwrap_or("")
}
const PLAYER_EQUIP_CUE: &str = "/Game/Sounds/Fort_GamePlay_Sounds/Player/Player_Equip_Melee_Cue";
/// `/Game/UI/Frontend/TopBar/TopBar.TopBar_C:WidgetTree_475:Social`
/// `HoveredSlateSoundOverride.ResourceObject` in the supplied 1.8 package.
pub const TOP_BAR_HOVER_CUE: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/New_FrontEnd/Fortnite_UI_Map_Hover_01_Cue.Fortnite_UI_Map_Hover_01_Cue";
/// The matching `PressedSlateSoundOverride.ResourceObject`.
pub const TOP_BAR_PRESS_CUE: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/New_FrontEnd/Fortnite_UI_Options_Menu_03_Cue.Fortnite_UI_Options_Menu_03_Cue";
/// `/Game/Athena/UI/AthenaMatchmakingWidget.AthenaMatchmakingWidget_C:WidgetTree:PlayButton`
/// `HoveredSlateSoundOverride.ResourceObject` in the supplied 1.8 package.
pub const LOBBY_PLAY_HOVER_CUE: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/New_FrontEnd/Fortnite_UI_Map_Hover_01_Cue.Fortnite_UI_Map_Hover_01_Cue";
/// The matching `PressedSlateSoundOverride.ResourceObject`.
pub const LOBBY_PLAY_PRESS_CUE: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/New_FrontEnd/Fortnite_UI_MainMenu_Generic_02_Cue.Fortnite_UI_MainMenu_Generic_02_Cue";
/// `/Game/Athena/UI/Customization/AthenaCustomizationPickerTileButton`
/// `Default__AthenaCustomizationPickerTileButton_C.HoveredSlateSoundOverride`.
pub const LOCKER_ITEM_HOVER_CUE: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/Fort_Athena_LockerItem_Hovered.Fort_Athena_LockerItem_Hovered";
/// The matching `PressedSlateSoundOverride`.
pub const LOCKER_ITEM_PRESS_CUE: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/Fort_Athena_LockerItem_Pressed.Fort_Athena_LockerItem_Pressed";

/// Every cue the sink may play except the ambience (prepared up front so no decoding happens during play).
fn core_cues() -> Vec<&'static str> {
    vec![
        events::FOOTSTEP_WALK,
        events::FOOTSTEP_SPRINT,
        cue_of("move.footstep.crouch"),
        cue_of("move.footstep.crouch_sprint"),
        events::JUMP,
        events::LAND,
        events::LAND_HARD,
        events::SWING,
        events::EQUIP_READY,
        PLAYER_EQUIP_CUE,
        events::IMPACT_DEFAULT,
        events::IMPACT_TREE,
        cue_of("harvest.tree.destroyed"),
        cue_of("harvest.rock.hit"),
        cue_of("harvest.wood.hit"),
        cue_of("harvest.metal.hit"),
        events::MENU_OPEN,
        events::MENU_CLOSE,
        events::MENU_CLICK,
        TOP_BAR_HOVER_CUE,
        TOP_BAR_PRESS_CUE,
        LOBBY_PLAY_HOVER_CUE,
        LOBBY_PLAY_PRESS_CUE,
        LOCKER_ITEM_HOVER_CUE,
        LOCKER_ITEM_PRESS_CUE,
    ]
    .into_iter()
    .chain(events::crack_and_prop_cues()) // CRACK AUDIO HOOK
    .chain(weapon_events::TABLE.iter().filter(|w| weapon_events::is_played(w)).map(|w| w.cue)) // WEAPON AUDIO HOOK
    .chain(weapon_events::bp_played_cues()) // WEAPON BP TABLE: every BR weapon Blueprint's cues + montage-notify cues
    .collect::<std::collections::BTreeSet<_>>().into_iter().collect()
}

/// The cues a weak-spot / prop [`SoundEvent`] starts (the single place that maps them; the sink and the tests use it).
/// Every other event keeps its direct handling in [`AudioSink::event`] and returns an empty list here.
pub fn crack_event_cues(e: &SoundEvent) -> Vec<&'static str> {
    // CRACK AUDIO HOOK
    match *e {
        SoundEvent::CrackSpawn { .. } => vec![events::CRACK_SPAWN],
        SoundEvent::CrackLevel { level, .. } => vec![events::crack_level_cue(level)],
        SoundEvent::CrackImpact { .. } => vec![events::CRACK_IMPACT],
        SoundEvent::PropHit { sound, destroyed, .. } => {
            let mut v = vec![events::IMPACT_DEFAULT];
            v.extend(sound.hit);
            if destroyed {
                v.extend(sound.death);
            }
            v
        }
        _ => Vec::new(),
    }
}

/// `Fort_WeaponMelee_PSM` (the swing cue's SoundClass) carries a `PassiveSoundMixModifier` for this SoundMix: while a
/// melee sound plays above the class' volume threshold the mix ducks music (x0.5), ambience (x0.5), gadgets, foley, ...
pub const MELEE_DUCK_MIX: &str = "/Game/Sounds/FortSoundClassesAndModes/Fort_WeaponMelee_Duck_MixMod";

#[derive(Default)]
struct Amb {
    ready: bool,
    want: bool,
    playing: Option<PlayId>,
}

/// State shared between the sink (game thread) and its loader (worker thread).
#[derive(Default)]
struct Shared {
    duck: OnceLock<SoundMix>,
    amb: Mutex<Amb>,
    music: music::MusicState, // MUSIC HOOK
    ambient: Mutex<ambient_rt::AmbientRt>, // AMBIENCE HOOK: bed / lakes / rivers / emitters director
}

/// AMBIENCE HOOK: start / stop the whole ambient system at a pose (a disabled system fades its loops out over 3 s).
fn ambient_enable(shared: &Shared, h: &EngineHandle, on: bool, at: Vec3, fwd: Vec3) {
    shared.ambient.lock().unwrap_or_else(|p| p.into_inner()).enable(h, on, at, fwd);
}

pub struct EngineSink {
    handle: EngineHandle,
    shared: Arc<Shared>,
    listener_pos: Vec3,
    forward: Vec3,
    /// FIRE LOOP: the running play of each fire audio channel (`key` = gun * 3 + channel).
    fire_loops: std::collections::HashMap<u32, fn_audio::mixer::PlayId>,
    /// Cues that failed to load (name, reason) when loaded through [`EngineSink::with_engine`]; the sink stays usable without them.
    pub missing: Vec<(String, String)>,
}

/// Decodes the cues from an install into an engine; `Send`, so it can run on a worker thread while the sink is already in use.
pub struct SinkLoader {
    handle: EngineHandle,
    shared: Arc<Shared>,
}

impl SinkLoader {
    /// Gameplay + UI cues and the melee duck mix. Returns what failed to load.
    pub fn load_core(&self, lib: &mut Library) -> Vec<(String, String)> {
        let mut missing = Vec::new();
        for c in core_cues() {
            match lib.prepare_cue(&self.handle, c) {
                Ok(0) => {}
                Ok(n) => missing.push((c.to_string(), format!("{n} wave(s) failed to load"))),
                Err(e) => missing.push((c.to_string(), e.to_string())),
            }
        }
        match lib.load_mix(MELEE_DUCK_MIX).ok().flatten() {
            Some(m) => {
                let _ = self.shared.duck.set(m);
            }
            None => missing.push((MELEE_DUCK_MIX.to_string(), "SoundMix not found".into())),
        }
        missing
    }

    /// The ambience bed (long OGG loops; can take a while). Starts playing at once when `set_ambience(true)` was already requested.
    pub fn load_ambience(&self, lib: &mut Library) -> Vec<(String, String)> {
        let mut missing = Vec::new();
        // long waves are decoded incrementally on a worker thread (fn_audio::stream), so the bed starts at once
        let prev = lib.stream_min_seconds.replace(STREAM_MIN_SECONDS);
        // AMBIENCE HOOK: every cue of the recovered ambient system (the four day-phase beds, the room tone, lakes, rivers, placed emitters)
        for c in fn_audio::ambient::cue_paths() {
            match lib.prepare_cue(&self.handle, c) {
                Ok(0) => {}
                Ok(n) => missing.push((c.to_string(), format!("{n} wave(s) failed to load"))),
                Err(e) => missing.push((c.to_string(), e.to_string())),
            }
        }
        match lib.load_mix(fn_audio::ambient::INSIDE_MIX).ok().flatten() {
            Some(m) => self.shared.ambient.lock().unwrap_or_else(|p| p.into_inner()).mix = Some(m),
            None => missing.push((fn_audio::ambient::INSIDE_MIX.to_string(), "SoundMix not found".into())),
        }
        lib.stream_min_seconds = prev;
        let mut a = self.shared.amb.lock().unwrap_or_else(|p| p.into_inner());
        a.ready = true;
        if a.want && a.playing.is_none() {
            a.playing = Some(0); // "running" marker: the director owns the plays
            drop(a);
            ambient_enable(&self.shared, &self.handle, true, Vec3::default(), Vec3::new(0.0, 0.0, -1.0));
        }
        missing
    }

    /// A music cue (long Opus / Ogg waves are streamed, short ones decoded whole); play it with `EngineHandle::play_cue`.
    pub fn load_music_cue(&self, lib: &mut Library, cue: &str) -> Vec<(String, String)> {
        let prev = lib.stream_min_seconds.replace(STREAM_MIN_SECONDS);
        let r = lib.prepare_cue(&self.handle, cue);
        lib.stream_min_seconds = prev;
        match r {
            Ok(0) => Vec::new(),
            Ok(n) => vec![(cue.to_string(), format!("{n} wave(s) failed to load"))],
            Err(e) => vec![(cue.to_string(), e.to_string())],
        }
    }

    /// Core cues, then the ambience.
    pub fn load(&self, lib: &mut Library) -> Vec<(String, String)> {
        let mut m = self.load_core(lib);
        m.extend(self.load_ambience(lib));
        m
    }
}

impl EngineSink {
    /// A sink on `handle` with nothing loaded yet (see [`EngineSink::loader`]).
    pub fn new(handle: EngineHandle) -> EngineSink {
        EngineSink { handle, shared: Arc::new(Shared::default()), fire_loops: Default::default(), listener_pos: Vec3::default(), forward: Vec3::new(0.0, 0.0, -1.0), missing: Vec::new() }
    }

    pub fn loader(&self) -> SinkLoader {
        SinkLoader { handle: self.handle.clone(), shared: self.shared.clone() }
    }

    /// Headless: drive an existing engine (tests, offline renders); loads everything before returning.
    pub fn with_engine(handle: EngineHandle, lib: &mut Library) -> EngineSink {
        let mut s = EngineSink::new(handle);
        s.missing = s.loader().load(lib);
        s
    }

    pub fn handle(&self) -> &EngineHandle {
        &self.handle
    }

    fn at(&self, dist_cm: f32) -> Vec3 {
        let f = self.forward;
        Vec3::new(self.listener_pos.x + f.x * dist_cm, self.listener_pos.y + f.y * dist_cm, self.listener_pos.z + f.z * dist_cm)
    }

    fn play(&self, cue: &str, position: Option<Vec3>) {
        if cue.is_empty() {
            return;
        }
        let _ = self.handle.play_cue(cue, PlayParams { position, ..Default::default() });
    }

    fn duck_melee(&self) {
        if let Some(m) = self.shared.duck.get() {
            self.handle.push_mix(m.clone());
        }
    }

    /// Whether the ambience bed has been decoded (it starts by itself once `set_ambience(true)` was asked for).
    pub fn ambience_ready(&self) -> bool {
        self.shared.amb.lock().unwrap_or_else(|p| p.into_inner()).ready
    }

    /// Play any prepared cue by event id from [`fn_audio::events::TABLE`] (`"harvest.rock.hit"`, ...), 2D.
    pub fn play_event_id(&self, id: &str) -> bool {
        match events::lookup(id) {
            Some(e) => {
                let _ = self.handle.play_cue(e.cue, PlayParams::default());
                true
            }
            None => false,
        }
    }
}

/// World metres -> UE centimetres (the engine's unit).
fn cm(p: glam::Vec3) -> Vec3 {
    Vec3::new(p.x * 100.0, p.y * 100.0, p.z * 100.0)
}

/// The cue(s) a [`Surface`] hit plays on top of the pickaxe's default impact.
fn surface_cue(s: Surface) -> Option<&'static str> {
    match s {
        Surface::Tree => Some(events::IMPACT_TREE),
        Surface::Rock => Some(cue_of("harvest.rock.hit")),
        Surface::Wood => Some(cue_of("harvest.wood.hit")),
        Surface::Metal => Some(cue_of("harvest.metal.hit")),
        Surface::Default => None,
    }
}

/// The footstep cue of a gait.
pub fn step_cue(g: Gait) -> &'static str {
    match g {
        Gait::Walk => events::FOOTSTEP_WALK,
        Gait::Sprint => events::FOOTSTEP_SPRINT,
        Gait::Crouch => cue_of("move.footstep.crouch"),
        Gait::CrouchSprint => cue_of("move.footstep.crouch_sprint"),
    }
}

impl AudioSink for EngineSink {
    fn event(&mut self, e: SoundEvent) {
        match e {
            // 1P footsteps / jump / land: the local pawn's own non-3P cues, played 2D at the listener
            SoundEvent::Footstep { sprint } => self.play(step_cue(if sprint { Gait::Sprint } else { Gait::Walk }), None),
            SoundEvent::Step { gait } => self.play(step_cue(gait), None),
            SoundEvent::Jump => self.play(events::JUMP, None),
            SoundEvent::Land { speed } => self.play(if speed >= HARD_LAND_SPEED { events::LAND_HARD } else { events::LAND }, None),
            SoundEvent::Swing => {
                let p = self.at(SWING_DISTANCE_CM);
                self.play(events::SWING, Some(p));
                self.duck_melee();
            }
            SoundEvent::SwingAt { pos } => {
                self.play(events::SWING, Some(cm(pos)));
                self.duck_melee();
            }
            SoundEvent::HarvestHit => {
                let p = self.at(HARVEST_DISTANCE_CM);
                self.play(events::IMPACT_DEFAULT, Some(p));
                self.play(events::IMPACT_TREE, Some(p));
            }
            SoundEvent::Harvest { surface, destroyed, pos } => {
                let p = Some(cm(pos));
                self.play(events::IMPACT_DEFAULT, p);
                if let Some(c) = surface_cue(surface) {
                    self.play(c, p);
                }
                if destroyed && surface == Surface::Tree {
                    self.play(cue_of("harvest.tree.destroyed"), p);
                }
            }
            SoundEvent::Equip => {
                self.play(events::EQUIP_READY, None);
                self.play(PLAYER_EQUIP_CUE, None);
            }
            // CRACK AUDIO HOOK: positional at the weak spot / prop
            SoundEvent::CrackSpawn { pos } | SoundEvent::CrackLevel { pos, .. } | SoundEvent::CrackImpact { pos } | SoundEvent::PropHit { pos, .. } => {
                let p = Some(cm(pos));
                for c in crack_event_cues(&e) {
                    self.play(c, p);
                }
            }
            // WEAPON AUDIO HOOK: positional at the weapon / pawn (None = 2D)
            SoundEvent::Weapon { id, pos, volume } => {
                if let Some(w) = weapon_events::lookup(id) {
                    let _ = self.handle.play_cue(w.cue, PlayParams { position: pos.map(cm), volume, pitch: w.pitch, ..Default::default() });
                }
            }
            SoundEvent::WeaponLoop { id, key, pos, volume } => {
                if let Some(old) = self.fire_loops.remove(&key) { self.handle.stop(old); }
                if let Some(w) = weapon_events::lookup(id) {
                    if let Ok(pid) = self.handle.play_cue(w.cue, PlayParams { position: pos.map(cm), volume, pitch: w.pitch, ..Default::default() }) { self.fire_loops.insert(key, pid); }
                }
            }
            SoundEvent::WeaponLoopStop { key, fade } => {
                if let Some(pid) = self.fire_loops.remove(&key) { if fade > 0. { self.handle.fade_play(pid, 0., fade, true) } else { self.handle.stop(pid) } }
            }
            SoundEvent::MenuOpen => self.play(events::MENU_OPEN, None),
            SoundEvent::MenuClose => self.play(events::MENU_CLOSE, None),
            SoundEvent::MenuClick => self.play(events::MENU_CLICK, None),
            SoundEvent::TopBarHover => self.play(TOP_BAR_HOVER_CUE, None),
            SoundEvent::TopBarPress => self.play(TOP_BAR_PRESS_CUE, None),
            SoundEvent::LobbyPlayHover => self.play(LOBBY_PLAY_HOVER_CUE, None),
            SoundEvent::LobbyPlayPress => self.play(LOBBY_PLAY_PRESS_CUE, None),
            SoundEvent::LockerItemHover => self.play(LOCKER_ITEM_HOVER_CUE, None),
            SoundEvent::LockerItemPress => self.play(LOCKER_ITEM_PRESS_CUE, None),
        }
    }

    fn listener(&mut self, position: glam::Vec3, forward: glam::Vec3, up: glam::Vec3) {
        // metres -> UE centimetres; both spaces are used right-handed y-up here
        self.listener_pos = cm(position);
        self.forward = Vec3::new(forward.x, forward.y, forward.z).normalized();
        self.handle.set_listener(self.listener_pos, self.forward, Vec3::new(up.x, up.y, up.z));
    }

    fn set_volumes(&mut self, master: f32, music: f32, sfx: f32, dialog: f32, voice_chat: f32) {
        self.handle.set_bus_volume(Bus::Master, master);
        self.handle.set_bus_volume(Bus::Music, music);
        self.handle.set_bus_volume(Bus::Sfx, sfx);
        self.handle.set_bus_volume(Bus::Voice, dialog);
        self.handle.set_bus_volume(Bus::VoiceChat, voice_chat);
    }

    fn set_suspended(&mut self, suspended: bool) {
        self.handle.set_suspended(suspended);
    }

    fn set_ambience(&mut self, on: bool) {
        let mut a = self.shared.amb.lock().unwrap_or_else(|p| p.into_inner());
        a.want = on;
        if !a.ready {
            return; // starts from `SinkLoader::load_ambience`
        }
        // AMBIENCE HOOK: the director (fn_audio::ambient) owns the bed: phase loop + cross-fades, lakes, rivers, emitters
        let running = a.playing.is_some();
        match (on, running) {
            (true, false) => {
                a.playing = Some(0);
                drop(a);
                ambient_enable(&self.shared, &self.handle, true, self.listener_pos, self.forward);
            }
            (false, true) => {
                a.playing = None;
                drop(a);
                ambient_enable(&self.shared, &self.handle, false, self.listener_pos, self.forward);
            }
            _ => {}
        }
    }

    // AMBIENCE HOOK
    fn ambient_map(&mut self, center_cm: [f32; 3], anchor_m: glam::Vec3) {
        self.shared.ambient.lock().unwrap_or_else(|p| p.into_inner()).set_map(center_cm, [anchor_m.x * 100.0, anchor_m.y * 100.0, anchor_m.z * 100.0]);
    }
    fn ambient_frame(&mut self, dt: f32, pawn: glam::Vec3, forward: glam::Vec3, trace: &mut dyn FnMut(glam::Vec3, glam::Vec3, f32) -> bool) {
        if !self.shared.amb.lock().unwrap_or_else(|p| p.into_inner()).playing.is_some() {
            return;
        }
        let mut t = |from: Vec3, dir: Vec3, len: f32| trace(glam::Vec3::new(from.x * 0.01, from.y * 0.01, from.z * 0.01), glam::Vec3::new(dir.x, dir.y, dir.z), len * 0.01);
        self.shared.ambient.lock().unwrap_or_else(|p| p.into_inner()).step(&self.handle, dt, self.listener_pos, cm(pawn), Vec3::new(forward.x, forward.y, forward.z).normalized(), &mut t);
    }

    fn set_quality(&mut self, high: bool) {
        self.handle.set_quality(if high { 0 } else { 1 });
    }

    // MUSIC HOOK
    fn set_music(&mut self, track: Option<MusicCue>) {
        self.set_music_cue(track);
    }
    fn set_loading_screen(&mut self, on: bool) {
        self.set_loading_mix(on);
    }
}

/// An [`EngineSink`] together with the output stream it plays through (keep it on the thread that created it).
#[cfg(feature = "device")]
pub struct DeviceSink {
    pub sink: EngineSink,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
    _device: fn_audio::device::DeviceStream,
}

#[cfg(feature = "device")]
impl AudioSink for DeviceSink {
    fn event(&mut self, e: SoundEvent) {
        self.sink.event(e)
    }
    fn listener(&mut self, p: glam::Vec3, f: glam::Vec3, u: glam::Vec3) {
        self.sink.listener(p, f, u)
    }
    fn set_volumes(&mut self, m: f32, mu: f32, s: f32, d: f32, v: f32) {
        self.sink.set_volumes(m, mu, s, d, v)
    }
    fn set_suspended(&mut self, s: bool) {
        self.sink.set_suspended(s)
    }
    fn set_ambience(&mut self, on: bool) {
        self.sink.set_ambience(on)
    }
    fn set_quality(&mut self, high: bool) {
        self.sink.set_quality(high)
    }
    // MUSIC HOOK
    fn set_music(&mut self, track: Option<MusicCue>) {
        self.sink.set_music(track)
    }
    fn set_loading_screen(&mut self, on: bool) {
        self.sink.set_loading_screen(on)
    }
    // AMBIENCE HOOK
    fn ambient_map(&mut self, center_cm: [f32; 3], anchor_m: glam::Vec3) {
        self.sink.ambient_map(center_cm, anchor_m)
    }
    fn ambient_frame(&mut self, dt: f32, pawn: glam::Vec3, forward: glam::Vec3, trace: &mut dyn FnMut(glam::Vec3, glam::Vec3, f32) -> bool) {
        self.sink.ambient_frame(dt, pawn, forward, trace)
    }
}

/// What the worker thread of a [`DeviceSink::open_streaming`] reports (lines are also printed to stderr).
#[cfg(feature = "device")]
#[derive(Clone, Default)]
pub struct LoadLog(pub Arc<Mutex<Vec<String>>>);
#[cfg(feature = "device")]
impl LoadLog {
    fn push(&self, line: String) {
        eprintln!("[audio] {line}");
        self.0.lock().unwrap_or_else(|p| p.into_inner()).push(line);
    }
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[cfg(feature = "device")]
impl DeviceSink {
    /// Open the default output device and load the cues from the install at `root` (the `FortniteGame` directory) before returning.
    pub fn open(root: impl AsRef<Path>) -> fn_audio::Result<DeviceSink> {
        let mut lib = Library::open(root)?;
        let (handle, stream) = fn_audio::device::open_default()?;
        let sink = EngineSink::with_engine(handle, &mut lib);
        Ok(DeviceSink::wrap(sink, stream))
    }

    fn wrap(sink: EngineSink, stream: fn_audio::device::DeviceStream) -> DeviceSink {
        DeviceSink { sink, device_name: stream.device_name.clone(), sample_rate: stream.sample_rate, channels: stream.channels, _device: stream }
    }

    /// Open the default output device NOW and return at once; a worker thread mounts the install, decodes the gameplay / UI cues and then the
    /// ambience loops (a long Opus / Ogg decode never touches the caller's thread). Events before a cue is loaded are dropped.
    pub fn open_streaming(root: PathBuf) -> fn_audio::Result<(DeviceSink, LoadLog)> {
        let (handle, stream) = fn_audio::device::open_default()?;
        let sink = EngineSink::new(handle);
        let loader = sink.loader();
        let log = LoadLog::default();
        let wlog = log.clone();
        let started = std::time::Instant::now();
        std::thread::Builder::new()
            .name("fn-audio-loader".into())
            .spawn(move || {
                let mut lib = match Library::open(&root) {
                    Ok(l) => l,
                    Err(e) => {
                        wlog.push(format!("cannot open the install {}: {e}; audio stays silent", root.display()));
                        return;
                    }
                };
                wlog.push(format!("install mounted in {:.2} s", started.elapsed().as_secs_f32()));
                // MUSIC HOOK: the loading-screen mix + the front-end music first (the first thing the user hears / the loading screen's silence)
                for (c, why) in loader.load_music(&mut lib) {
                    wlog.push(format!("music {c} unavailable: {why}"));
                }
                wlog.push(format!("music ready after {:.2} s", started.elapsed().as_secs_f32()));
                for (c, why) in loader.load_core(&mut lib) {
                    wlog.push(format!("cue {c} unavailable: {why}"));
                }
                wlog.push(format!("gameplay + UI cues ready after {:.2} s", started.elapsed().as_secs_f32()));
                for (c, why) in loader.load_ambience(&mut lib) {
                    wlog.push(format!("cue {c} unavailable: {why}"));
                }
                wlog.push(format!("ambience ready after {:.2} s", started.elapsed().as_secs_f32()));
            })
            .map_err(|e| fn_audio::AudioError::Device(format!("loader thread: {e}")))?;
        Ok((DeviceSink::wrap(sink, stream), log))
    }
}

/// Build a boxed sink for fn-app: the real engine when the device and the install work, otherwise a [`NullSink`]
/// (the reason is printed to stderr once). Blocks while the cues decode; the game uses [`DeviceSink::open_streaming`].
#[cfg(feature = "device")]
pub fn open_boxed(root: impl AsRef<Path>) -> Box<dyn AudioSink> {
    match DeviceSink::open(root) {
        Ok(s) => {
            for (c, why) in &s.sink.missing {
                eprintln!("fn-audio-sink: cue {c} unavailable: {why}");
            }
            Box::new(s)
        }
        Err(e) => {
            eprintln!("fn-audio-sink: audio disabled: {e}");
            Box::new(NullSink)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_audio::mixer::AudioEngine;

    const ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

    fn root() -> Option<String> {
        let r = std::env::var("FN_ROOT").unwrap_or_else(|_| ROOT.to_string());
        Path::new(&r).join("Content").join("Paks").exists().then_some(r)
    }

    #[test]
    fn null_sink_accepts_everything() {
        let mut n = NullSink;
        n.event(SoundEvent::Jump);
        n.set_volumes(1., 1., 1., 1., 1.);
        n.set_suspended(true);
        n.set_ambience(true);
        n.set_quality(false);
        n.listener(glam::Vec3::ZERO, glam::Vec3::NEG_Z, glam::Vec3::Y);
    }

    #[test]
    fn every_cue_the_sink_plays_is_in_the_event_table() {
        for c in core_cues() {
            assert!(c.starts_with("/Game/"), "{c}");
        }
        for g in [Gait::Walk, Gait::Sprint, Gait::Crouch, Gait::CrouchSprint] {
            assert!(step_cue(g).ends_with("Footstep_Cue"), "{g:?}");
        }
        for s in [Surface::Tree, Surface::Rock, Surface::Wood, Surface::Metal] {
            assert!(surface_cue(s).is_some_and(|c| c.starts_with("/Game/")), "{s:?}");
        }
    }

    #[test]
    fn events_make_sound_and_volumes_and_suspend_apply() {
        let Some(r) = root() else {
            eprintln!("SKIP: no install");
            return;
        };
        let mut lib = Library::open(r).unwrap();
        let mut eng = AudioEngine::new(48000);
        let mut sink = EngineSink::with_engine(eng.handle(), &mut lib);
        assert!(sink.missing.is_empty(), "{:?}", sink.missing);
        sink.listener(glam::Vec3::new(10.0, 2.0, -4.0), glam::Vec3::NEG_Z, glam::Vec3::Y);
        let peak = |eng: &mut AudioEngine| {
            let o = eng.render_offline(24000, 9);
            // skip the first 512 samples: suspend / volume changes ramp within the first 64-frame block
            let p = o[512..].iter().fold(0f32, |a, &b| a.max(b.abs()));
            eng.handle().stop_all();
            eng.render_offline(2048, 9);
            p
        };
        let at = glam::Vec3::new(10.5, 1.2, -7.0);
        for (ev, name) in [
            (SoundEvent::Footstep { sprint: false }, "walk"),
            (SoundEvent::Footstep { sprint: true }, "sprint"),
            (SoundEvent::Step { gait: Gait::Crouch }, "crouch step"),
            (SoundEvent::Step { gait: Gait::CrouchSprint }, "crouch sprint step"),
            (SoundEvent::Jump, "jump"),
            (SoundEvent::Land { speed: 3.0 }, "land"),
            (SoundEvent::Land { speed: 12.0 }, "hard land"),
            (SoundEvent::Swing, "swing"),
            (SoundEvent::SwingAt { pos: at }, "swing at"),
            (SoundEvent::HarvestHit, "harvest"),
            (SoundEvent::Harvest { surface: Surface::Tree, destroyed: false, pos: at }, "tree hit"),
            (SoundEvent::Harvest { surface: Surface::Tree, destroyed: true, pos: at }, "tree destroyed"),
            (SoundEvent::Harvest { surface: Surface::Rock, destroyed: false, pos: at }, "rock hit"),
            (SoundEvent::Harvest { surface: Surface::Wood, destroyed: false, pos: at }, "wood hit"),
            (SoundEvent::Harvest { surface: Surface::Metal, destroyed: false, pos: at }, "metal hit"),
            (SoundEvent::Equip, "equip"),
            (SoundEvent::MenuOpen, "menu open"),
            (SoundEvent::MenuClose, "menu close"),
            (SoundEvent::MenuClick, "menu click"),
            (SoundEvent::TopBarHover, "top bar hover"),
            (SoundEvent::TopBarPress, "top bar press"),
            (SoundEvent::LobbyPlayHover, "lobby play hover"),
            (SoundEvent::LobbyPlayPress, "lobby play press"),
            (SoundEvent::LockerItemHover, "locker item hover"),
            (SoundEvent::LockerItemPress, "locker item press"),
        ] {
            sink.event(ev);
            assert!(peak(&mut eng) > 1e-4, "{name} is silent");
        }
        // volume sliders: master 0 silences everything, sfx scales
        sink.event(SoundEvent::Swing);
        sink.set_volumes(1.0, 1.0, 1.0, 1.0, 1.0);
        let full = peak(&mut eng);
        sink.event(SoundEvent::Swing);
        sink.set_volumes(1.0, 1.0, 0.5, 1.0, 1.0);
        let half = peak(&mut eng);
        assert!(half < full * 0.75, "sfx slider halves the swing ({full} -> {half})");
        sink.set_volumes(0.0, 1.0, 1.0, 1.0, 1.0);
        sink.event(SoundEvent::Swing);
        assert_eq!(peak(&mut eng), 0.0, "master 0");
        sink.set_volumes(1.0, 1.0, 1.0, 1.0, 1.0);
        sink.set_suspended(true);
        sink.event(SoundEvent::Jump);
        assert!(peak(&mut eng) < 1e-6, "suspended sink is silent");
        sink.set_suspended(false);
        // ambience on/off
        sink.set_ambience(true);
        assert!(peak(&mut eng) > 1e-5, "ambience bed audible");
        sink.set_ambience(false);
    }

    // CRACK AUDIO HOOK: one test per cue of the weak-spot effect + the per-prop harvest sounds.
    fn crack_sink_peak(eng: &mut AudioEngine) -> f32 {
        let o = eng.render_offline(24000, 9);
        let p = o[512..].iter().fold(0f32, |a, &b| a.max(b.abs()));
        eng.handle().stop_all();
        eng.render_offline(2048, 9);
        p
    }

    #[test]
    fn crack_events_map_to_their_cues() {
        let pos = glam::Vec3::new(1.0, 1.0, -3.0);
        assert_eq!(crack_event_cues(&SoundEvent::CrackSpawn { pos }), vec![events::CRACK_SPAWN]);
        assert_eq!(crack_event_cues(&SoundEvent::CrackImpact { pos }), vec![events::CRACK_IMPACT]);
        for l in 0..9u8 {
            let c = crack_event_cues(&SoundEvent::CrackLevel { level: l, pos });
            assert_eq!(c.len(), 1);
            assert!(c[0].ends_with(&format!("Sound_Crack_Level_{:02}_Cue", l + 1)), "{l}: {c:?}");
        }
        assert_eq!(crack_event_cues(&SoundEvent::CrackLevel { level: 99, pos }), vec![events::CRACK_LEVELS[8]]);
        let oak = events::prop_sound("Athena_Tree_Large_1").unwrap();
        assert_eq!(crack_event_cues(&SoundEvent::PropHit { sound: oak, destroyed: false, pos }), vec![events::IMPACT_DEFAULT, events::OAK_IMPACT]);
        assert_eq!(crack_event_cues(&SoundEvent::PropHit { sound: oak, destroyed: true, pos }), vec![events::IMPACT_DEFAULT, events::OAK_IMPACT, events::OAK_DESTROYED]);
        let pine5 = events::prop_sound("Athena_Tree_Pine_05").unwrap();
        assert_eq!(crack_event_cues(&SoundEvent::PropHit { sound: pine5, destroyed: true, pos }), vec![events::IMPACT_DEFAULT, events::OAK_DESTROYED], "Pine_05 serialises no PlayHitSound");
        assert!(crack_event_cues(&SoundEvent::Jump).is_empty());
        let core = core_cues();
        for c in events::crack_and_prop_cues() {
            assert!(core.contains(&c), "{c} is prepared by the sink");
        }
    }

    #[test]
    fn every_crack_and_prop_cue_decodes_and_its_event_is_audible_at_the_weak_spot() {
        let Some(r) = root() else {
            eprintln!("SKIP: no install");
            return;
        };
        let mut lib = Library::open(r).unwrap();
        let mut eng = AudioEngine::new(48000);
        let mut sink = EngineSink::with_engine(eng.handle(), &mut lib);
        assert!(sink.missing.is_empty(), "{:?}", sink.missing);
        sink.listener(glam::Vec3::new(10.0, 2.0, -4.0), glam::Vec3::NEG_Z, glam::Vec3::Y);
        let at = glam::Vec3::new(10.5, 1.2, -6.0);
        let mut cases: Vec<(String, SoundEvent, &'static str)> = vec![
            ("crack spawn".into(), SoundEvent::CrackSpawn { pos: at }, events::CRACK_SPAWN),
            ("crack impact".into(), SoundEvent::CrackImpact { pos: at }, events::CRACK_IMPACT),
        ];
        for l in 0..9u8 {
            cases.push((format!("crack level {}", l + 1), SoundEvent::CrackLevel { level: l, pos: at }, events::CRACK_LEVELS[l as usize]));
        }
        for (name, ev, cue) in cases {
            assert_eq!(crack_event_cues(&ev), vec![cue], "{name}");
            assert_eq!(lib.prepare_cue(sink.handle(), cue).unwrap(), 0, "{name}: every wave of {cue} decodes");
            sink.event(ev);
            assert!(crack_sink_peak(&mut eng) > 1e-4, "{name} is silent");
        }
        // the destroy / death cues of every prop that names one decode and play
        for p in events::PROP_SOUNDS {
            for (what, cue) in [("hit", p.hit), ("death", p.death)] {
                let Some(cue) = cue else { continue };
                assert_eq!(lib.prepare_cue(sink.handle(), cue).unwrap(), 0, "{} {what}: {cue}", p.id);
            }
            sink.event(SoundEvent::PropHit { sound: p, destroyed: true, pos: at });
            assert!(crack_sink_peak(&mut eng) > 1e-4, "{} destroyed is silent", p.id);
        }
    }

    /// Every reachable weapon / consumable event is audible, and positional ones pan toward their side (WEAPON AUDIO HOOK).
    #[test]
    fn fire_loop_starts_once_keeps_running_and_stops_with_its_fade() {
        let Some(r) = root() else {
            eprintln!("SKIP: no install");
            return;
        };
        let mut lib = Library::open(r).unwrap();
        let mut eng = AudioEngine::new(48000);
        let mut sink = EngineSink::with_engine(eng.handle(), &mut lib);
        assert!(sink.missing.is_empty(), "{:?}", sink.missing);
        sink.listener(glam::Vec3::ZERO, glam::Vec3::NEG_Z, glam::Vec3::Y);
        let peak = |o: &[f32]| o.iter().fold(0f32, |a, &b| a.max(b.abs()));
        // Hydraulic Drum: bPersistentFireFX with indefinitely looping fire cues (BP_FIRE_FX): one start runs on
        sink.event(SoundEvent::WeaponLoop { id: "bp.B_Assault_Hydraulic_Drum_Athena.fire", key: 0, pos: Some(glam::Vec3::new(0., 0., -1.)), volume: 1.0 });
        let o = eng.render_offline(48000 * 6, 11);
        assert!(peak(&o[o.len() - 48000..]) > 1e-5, "the loop must still sound after 5 s");
        // a second start on the same channel replaces it (no stacking); the stop fades it out
        sink.event(SoundEvent::WeaponLoopStop { key: 0, fade: 0.15 });
        let o = eng.render_offline(48000, 11);
        assert!(peak(&o[o.len() - 24000..]) < 1e-6, "silent half a second after the 0.15 s fade-out");
        assert!(sink.fire_loops.is_empty());
        // its stop sound is an ordinary weapon event
        sink.event(SoundEvent::Weapon { id: "bp.B_Assault_Hydraulic_Drum_Athena.fire.stop", pos: Some(glam::Vec3::new(0., 0., -1.)), volume: 1.0 });
        assert!(peak(&eng.render_offline(48000, 11)) > 1e-5, "stop sound");
    }
    #[test]
    fn weapon_events_make_sound_and_the_positional_ones_are_panned() {
        let Some(r) = root() else {
            eprintln!("SKIP: no install");
            return;
        };
        let mut lib = Library::open(r).unwrap();
        let mut eng = AudioEngine::new(48000);
        let mut sink = EngineSink::with_engine(eng.handle(), &mut lib);
        assert!(sink.missing.is_empty(), "{:?}", sink.missing);
        sink.listener(glam::Vec3::ZERO, glam::Vec3::NEG_Z, glam::Vec3::Y);
        let render = |eng: &mut AudioEngine, secs: usize| {
            let o = eng.render_offline(48000 * secs, 11);
            eng.handle().stop_all();
            eng.render_offline(2048, 11);
            o
        };
        let at = Some(glam::Vec3::new(0.5, 0.0, -1.0));
        let mut n = 0;
        for w in weapon_events::TABLE.iter().filter(|w| weapon_events::is_played(w)) {
            sink.event(SoundEvent::Weapon { id: w.event, pos: if w.event.starts_with("weapon.equip") || w.event.starts_with("heal.") { None } else { at }, volume: 1.0 });
            let o = render(&mut eng, 2);
            assert!(o.iter().fold(0f32, |a, &b| a.max(b.abs())) > 1e-5, "{} is silent", w.event);
            n += 1;
        }
        assert!(n >= 40, "{n} weapon events checked");
        // a shot to the right (+X) is louder in the right channel; to the left in the left channel (interleaved stereo)
        let side = |sink: &mut EngineSink, eng: &mut AudioEngine, x: f32| {
            sink.event(SoundEvent::Weapon { id: "weapon.ar.fire", pos: Some(glam::Vec3::new(x, 0.0, -1.0)), volume: 1.0 });
            let o = render(eng, 1);
            let (mut l, mut r) = (0f64, 0f64);
            for f in o.chunks(2) {
                l += (f[0] as f64).powi(2);
                r += (f[1] as f64).powi(2);
            }
            (l, r)
        };
        let (l, r) = side(&mut sink, &mut eng, 3.0);
        assert!(r > l * 1.5, "right-hand shot: L {l} R {r}");
        let (l, r) = side(&mut sink, &mut eng, -3.0);
        assert!(l > r * 1.5, "left-hand shot: L {l} R {r}");
        // the whip closeness volume scales the cue
        sink.event(SoundEvent::Weapon { id: "whip.close", pos: at, volume: 1.0 });
        let loud = render(&mut eng, 1).iter().fold(0f32, |a, &b| a.max(b.abs()));
        sink.event(SoundEvent::Weapon { id: "whip.close", pos: at, volume: 0.25 });
        let quiet = render(&mut eng, 1).iter().fold(0f32, |a, &b| a.max(b.abs()));
        assert!(quiet < loud * 0.6 && quiet > 0.0, "volume {loud} -> {quiet}");
    }

    #[test]
    fn ambience_requested_before_it_is_loaded_starts_when_the_loader_finishes() {
        let Some(r) = root() else {
            eprintln!("SKIP: no install");
            return;
        };
        let mut eng = AudioEngine::new(48000);
        let mut sink = EngineSink::new(eng.handle());
        sink.set_ambience(true);
        assert!(!sink.ambience_ready());
        // nothing is loaded: events are dropped, no panic
        sink.event(SoundEvent::Jump);
        let silent = eng.render_offline(4800, 1).iter().fold(0f32, |a, &b| a.max(b.abs()));
        assert_eq!(silent, 0.0);
        let loader = sink.loader();
        let t = std::thread::spawn(move || {
            let mut lib = Library::open(r).unwrap();
            loader.load(&mut lib)
        });
        let missing = t.join().unwrap();
        assert!(missing.is_empty(), "{missing:?}");
        assert!(sink.ambience_ready());
        let o = eng.render_offline(24000, 3);
        assert!(o.iter().fold(0f32, |a, &b| a.max(b.abs())) > 1e-5, "ambience started by itself");
    }
}
