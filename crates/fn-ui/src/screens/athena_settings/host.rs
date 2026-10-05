//! The natives of the Settings Blueprints, answered from the game's settings model (`fn_settings::Settings`).
//!
//! Every native here is a function the Blueprint bytecode calls on itself (`call GetShadowQuality()` in `TabVideoOptions::Update Data`, `IsAthena()`,
//! `GetPossibleFPS()`, ...); the names are the ones in the Kismet. Which setting each getter reads is this runtime's mapping (the 1.8 client reads its
//! `UFortGameUserSettings` fields natively; the exe does not expose more than the function name), so the mapping table below is the only non-data glue.
#![cfg(feature = "menu")]
use super::kismet::Val;
use super::{Backing, RowValue};
use crate::menu::pages::video::resolution_label;
use crate::screens::athena_lobby::store::Locres;
use fn_settings::{self as settings, KindDef, Settings, Value};
use std::rc::Rc;

/// `(Blueprint package, row widget variable, settings id, getter native the Blueprint calls for it)`.
pub const ROWS: &[(&str, &str, &str, &str)] = &[
    ("TabVideoOptions", "WindowMode", "video.window_mode", "GetCurrentWindowMode"),
    ("TabVideoOptions", "DisplayResolution", "video.resolution", "GetCurrentDisplayResolution"),
    ("TabVideoOptions", "FrameRateLimit", "video.frame_rate_limit", "GetCurrentFPS"),
    ("TabVideoOptions", "Quality", "video.quality", ""),
    ("TabVideoOptions", "3DResolution", "video.resolution_quality", "GetDiscreteResolutionQuality"),
    ("TabVideoOptions", "ViewDistance", "video.view_distance", "GetViewDistanceQuality"),
    ("TabVideoOptions", "Shadows", "video.shadows", "GetShadowQuality"),
    ("TabVideoOptions", "Anti-Aliasing", "video.anti_aliasing", "GetAntiAliasingQuality"),
    ("TabVideoOptions", "Textures", "video.textures", "GetTextureQuality"),
    ("TabVideoOptions", "Effects", "video.effects", "GetEffectsQuality"),
    ("TabVideoOptions", "PostProcessing", "video.post_processing", "GetPostProcessQuality"),
    ("TabVideoOptions", "VSync", "video.vsync", "GetUseVsync"),
    ("TabVideoOptions", "MotionBlur", "video.motion_blur", "GetMotionBlur"),
    ("TabVideoOptions", "ShowGrass", "video.show_grass", "GetShowGrass"),
    ("TabVideoOptions", "ShowFPS", "video.show_fps", "GetShowFPS"),
    ("TabGameOptionsMain", "MouseSensitivity", "input.mouse_sensitivity", "GetMouseSensitivityValue"),
    ("TabGameOptionsMain", "MouseTargetingMultiplier", "input.mouse_ads_multiplier", "GetTargetingMultiplierValue"),
    ("TabGameOptionsMain", "MouseScopedMultiplier", "input.mouse_scope_multiplier", "GetScopedMultiplierValue"),
    ("TabGameOptionsMain", "GamepadTargetingMultiplier", "input.controller_ads_multiplier", "GetGamepadTargetingMultiplierValue"),
    ("TabGameOptionsMain", "GamepadScopedMultiplier", "input.controller_scope_multiplier", "GetGamepadScopedMultiplierValue"),
    ("TabGameOptionsMain", "ControllerLookSensitivityX", "input.controller_sensitivity_x", ""),
    ("TabGameOptionsMain", "ControllerLookSensitivityY", "input.controller_sensitivity_y", ""),
    ("TabGameOptionsMain", "Language", "game.language", "GetCurrentLanguage"),
    ("TabGameOptionsMain", "InvertMouse", "input.invert_y", "GetLookInversionState"),
    ("TabGameOptionsMain", "ToggleSprint", "input.toggle_sprint", "GetSprintToggleState"),
    ("TabGameOptionsMain", "SprintCancelsReload", "input.sprint_cancels_reload", "GetSprintCancelsReloadState"),
    ("TabGameOptionsMain", "TapInteract", "input.tap_to_interact", "GetTapInteractState"),
    ("TabGameOptionsMain", "ToggleTargeting", "input.toggle_targeting", "GetTargetingToggleState"),
    ("TabGameOptionsMain", "AutoEquipBetterItems", "game.auto_equip_better_items", "GetAutoEquipState"),
    ("TabGameOptionsMain", "First-PersonCamera", "game.first_person_camera", "GetFirstPersonCameraState"),
    ("TabGameOptionsMain", "ForceFeedback", "input.vibration", "GetForceFeedbackState"),
    ("TabGameOptionsMain", "EquipFirstBuildingPieceWhenSwappingQuickbars", "game.reset_building_choice", "GetFocusOnFirstBuildingPieceWhenQuickbarSwappedState"),
    ("TabGameOptionsMain", "ShowHeadAccessories", "game.show_head_accessories", "GetShowHeroHeadAccessoriesForLocalPlayer"),
    ("TabGameOptionsMain", "ShowBackpack", "game.show_backpack", "GetShowHeroBackpackForLocalPlayer"),
    ("TabBrightnessOptions", "BrightnessValue", "video.brightness", "GetGammaSettings"),
    ("TabAudioOptions", "MusicVolume", "audio.music_volume", "GetMusicVolumeValue"),
    ("TabAudioOptions", "SoundFXVolume", "audio.sfx_volume", "GetSoundFXVolumeValue"),
    ("TabAudioOptions", "DialogVolume", "audio.dialog_volume", "GetDialogVolumeValue"),
    ("TabAudioOptions", "ChatVolume", "audio.voice_chat_volume", "GetChatVolumeValue"),
    ("TabAudioOptions", "Subtitles", "accessibility.subtitles", "GetSubtitlesEnabled"),
    ("TabAudioOptions", "Quality", "audio.quality", "GetQuality"),
    ("TabAudioOptions", "VoiceChatEnable", "audio.voice_chat_enabled", "GetVoiceChatEnabled"),
    ("TabAudioOptions", "VoiceChatPushToTalk", "audio.push_to_talk", "GetVoiceChatPTTEnabled"),
];

/// `Matchmaking Region` (`GetRegionDisplayNames` / `GetCurrentRegion`): the `MMRegion` locres strings; stored as `[Game] MatchmakingRegion`.
pub const REGIONS: &[&str] = &["Europe", "North America", "Oceania"];
pub const REGION_DEFAULT: usize = 1;

pub struct SettingsBacking {
    pub settings: Settings,
    /// display modes of the monitor
    pub sizes: Vec<(u32, u32)>,
    pub in_zone: bool,
    pub locres: Option<Rc<Locres>>,
}

fn lit(s: &str) -> Val { Val::Text { src: s.to_owned(), key: String::new(), ns: String::new() } }
fn loc(ns: &str, key: &str, src: &str) -> Val { Val::Text { src: src.to_owned(), key: key.to_owned(), ns: ns.to_owned() } }

impl SettingsBacking {
    pub fn row_setting(page: &str, key: &str) -> Option<&'static str> { ROWS.iter().find(|r| r.0 == page && r.1 == key).map(|r| r.2) }

    /// Display modes of the Display Resolution row: the monitor's modes (>= 640x360) plus the current size.
    pub fn resolution_modes(&self) -> Vec<(u32, u32)> {
        let mut m: Vec<(u32, u32)> = self.sizes.iter().copied().filter(|&(w, h)| w >= 640 && h >= 360).collect();
        m.sort(); m.dedup();
        let cur = (self.settings.video.width, self.settings.video.height);
        if cur.0 > 0 && !m.contains(&cur) { m.push(cur); m.sort(); }
        m
    }

    fn fps_text(&self, fps: u32) -> Val {
        if fps == 0 { return loc("Fortnite.VideoOptions", "UnlimitedFrameRate", "Unlimited"); }
        let f = self.locres.as_ref().and_then(|l| l.get("Fortnite.VideoOptions", "FrameRateLimitText").map(|s| s.to_owned()));
        lit(&f.map(|s| s.replace("{0}", &fps.to_string())).unwrap_or_else(|| format!("{fps}")))
    }

    fn entry_options(id: &str) -> Vec<&'static str> {
        match settings::entry(id).map(|e| e.kind) { Some(KindDef::Enum(o)) => o.iter().filter(|x| x.0 != "custom").map(|x| x.1).collect(), _ => vec![] }
    }

    fn region_index(&self) -> i64 {
        self.settings.extra("Game", "MatchmakingRegion").and_then(|v| REGIONS.iter().position(|o| o.eq_ignore_ascii_case(v.trim()))).unwrap_or(REGION_DEFAULT) as i64
    }

    /// Current value of a setting as the row sees it.
    fn row_value(&self, id: &str) -> Option<RowValue> {
        let e = settings::entry(id)?;
        Some(match (e.kind, self.settings.get(id)?) {
            (KindDef::Toggle, Value::Bool(b)) => RowValue::Index(b as i64),
            (KindDef::Enum(_), Value::Index(i)) => RowValue::Index(i as i64),
            (KindDef::Slider { .. }, Value::Float(f)) => RowValue::Float(f),
            (KindDef::Resolution, Value::Size(w, h)) => RowValue::Index(self.resolution_modes().iter().position(|m| *m == (w, h)).unwrap_or(0) as i64),
            _ => return None,
        })
    }

    /// Turn a row event into a `Settings` write: `(settings id, value)`.
    pub fn to_setting(&self, page: &str, key: &str, v: RowValue) -> Option<(&'static str, Value)> {
        if key == "Region" { return None; }
        let id = Self::row_setting(page, key)?;
        let e = settings::entry(id)?;
        let val = match (e.kind, v) {
            (KindDef::Toggle, RowValue::Index(i)) => Value::Bool(i != 0),
            (KindDef::Enum(o), RowValue::Index(i)) => Value::Index((i.max(0) as usize).min(o.len().saturating_sub(1))),
            (KindDef::Slider { min, max, .. }, RowValue::Float(f)) => Value::Float(f.clamp(min, max)),
            (KindDef::Resolution, RowValue::Index(i)) => { let m = self.resolution_modes(); let s = *m.get(i.max(0) as usize)?; Value::Size(s.0, s.1) }
            _ => return None,
        };
        Some((id, val))
    }
}

impl Backing for SettingsBacking {
    fn value(&self, page: &str, key: &str) -> Option<RowValue> {
        if key == "Region" { return Some(RowValue::Index(self.region_index())); }
        self.row_value(Self::row_setting(page, key)?)
    }

    fn native(&self, page: &str, func: &str, args: &[Val]) -> Option<Val> {
        // platform / context queries: Battle Royale (`IsAthena`) match on the PC (no Xbox / PS4, mouse + keyboard)
        match func {
            "IsAthena" | "ShowVideoOptions" | "ShowGameOptions" | "ShowInputOptions" | "ShowControllerOptions" => return Some(Val::Bool(true)),
            "IsInZone" => return Some(Val::Bool(self.in_zone)),
            "IsXboxPlatform" | "IsPS4Platform" | "ShouldShowSafeZoneOption" | "IsUsingGamepad" | "ShowChatVolume" | "ShowQuality" => return Some(Val::Bool(false)),
            "GetPlatformName" => return Some(Val::Str("Windows".into())),
            "IsAccountLinkingEnabled" => return Some(Val::Bool(true)),
            "GetAccountLinkingUIConfig" => return Some(Val::Int(1)),
            "GetPossibleWindowModes" => return Some(Val::Array(vec![loc("Fortnite.VideoOptions", "WindowedModeStringsFullscreen", "Fullscreen"), loc("Fortnite.VideoOptions", "WindowedModeStringsWindowedFullscreen", "WindowedFullscreen"), loc("Fortnite.VideoOptions", "WindowedModeStringsWindowed", "Windowed")])),
            // EWindowMode: Fullscreen 0, WindowedFullscreen 1, Windowed 2 (the index order of `GetPossibleWindowModes`)
            "IsWindowedFullscreen" => return args.first().and_then(|a| a.as_int()).map(|i| Val::Bool(i == 1)),
            "GetPossibleFPS" => return Some(Val::Array([30u32, 60, 120, 144, 165, 240, 0].iter().map(|&f| self.fps_text(f)).collect())),
            "GetPossibleDisplayResolutions" => return Some(Val::Array(self.resolution_modes().iter().map(|&(w, h)| lit(&resolution_label(w, h))).collect())),
            "GetPossibleLanguages" => return Some(Val::Array(Self::entry_options("game.language").iter().map(|s| lit(s)).collect())),
            "GetRegionDisplayNames" => return Some(Val::Array(REGIONS.iter().map(|r| loc("MMRegion", r, r)).collect())),
            "GetCurrentRegion" => return Some(Val::Int(self.region_index())),
            "GetGamepadLookSensitivityValue" => {
                let f = |id| match self.settings.get(id) { Some(Value::Float(v)) => v, _ => 0.0 };
                return Some(Val::Array(vec![Val::Float(f("input.controller_sensitivity_x")), Val::Float(f("input.controller_sensitivity_y"))]));
            }
            "GetSafeZoneValue" => return Some(Val::Float(0.0)),
            _ => {}
        }
        let (_, _, id, _) = ROWS.iter().find(|r| r.0 == page && r.3 == func)?;
        let e = settings::entry(id)?;
        Some(match (e.kind, self.row_value(id)?) {
            // `Conv_BoolToInt(GetX())` rows: the getter returns a bool
            (KindDef::Toggle, RowValue::Index(i)) => Val::Bool(i != 0),
            (_, RowValue::Index(i)) => Val::Int(i),
            (_, RowValue::Float(f)) => Val::Float(f),
        })
    }
}
