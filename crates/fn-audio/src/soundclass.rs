//! `USoundClass` hierarchy and `USoundMix` assets.
//!
//! Recovered from `/Game/Sounds/FortSoundClassesAndModes/**` (103 classes, 50 mixes). The class tree is
//! `Master (0.7) -> { Music (0.5), SFX (0.5), Voice (0.8), Fort_01_VOIP }` with ~100 game classes below those
//! (`Fort_UI -> SFX`, `Fort_WeaponMelee -> Fort_Weapons (0.8) -> Fort_06_Player_Attacks_Abilities -> SFX`, ...).
//! A wave's base loudness from its class is the product of `Properties.Volume` (default 1) along the parent chain.

use crate::error::Result;
use crate::install::Install;
use fn_uasset::{PackageData, Value};
use std::collections::HashMap;

/// The user-facing volume buses the Settings > Audio sliders map to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bus {
    /// Master volume (root `Master` class; the 1.8 Audio tab has no Master slider, it is an extension).
    Master,
    /// Music Volume slider (`Music` class subtree).
    Music,
    /// Sound FX Volume slider (`SFX` subtree, including UI sounds).
    Sfx,
    /// Dialog Volume slider (`Voice` subtree).
    Voice,
    /// Voice Chat Volume slider (`Fort_01_VOIP`).
    VoiceChat,
}

impl Bus {
    pub const ALL: [Bus; 5] = [Bus::Master, Bus::Music, Bus::Sfx, Bus::Voice, Bus::VoiceChat];
    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug)]
pub struct SoundClass {
    pub name: String,
    pub parent: Option<String>,
    pub volume: f32,
    pub pitch: f32,
    pub is_ui: bool,
    pub is_music: bool,
    pub always_play: bool,
    pub apply_effects: bool,
    pub reverb: bool,
}

/// What a cue/node needs to know about its class at play time.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassInfo {
    pub name: String,
    /// Product of `Volume` along the whole chain up to and including `Master` (0.7). The user's sliders multiply on top.
    pub base_volume: f32,
    pub pitch: f32,
    pub bus: Bus,
    pub is_ui: bool,
    pub always_play: bool,
    /// Lower-case class names from this class up to the root (used by SoundMix adjusters).
    pub chain: Vec<String>,
}

impl Default for ClassInfo {
    fn default() -> Self {
        ClassInfo { name: "SFX".into(), base_volume: 1.0, pitch: 1.0, bus: Bus::Sfx, is_ui: false, always_play: false, chain: vec!["sfx".into(), "master".into()] }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SoundClassTree {
    classes: HashMap<String, SoundClass>,
}

/// Last path component of an object path (`/Game/X/Foo.Foo` -> `Foo`).
pub fn object_name(path: &str) -> &str {
    path.rsplit(['.', '/', ':']).next().unwrap_or(path)
}

impl SoundClassTree {
    pub fn insert(&mut self, c: SoundClass) {
        self.classes.insert(c.name.to_ascii_lowercase(), c);
    }
    pub fn get(&self, name: &str) -> Option<&SoundClass> {
        self.classes.get(&object_name(name).to_ascii_lowercase())
    }
    pub fn len(&self) -> usize {
        self.classes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.classes.values().map(|c| c.name.as_str())
    }

    /// Class names from `name` up to the root (cycle-safe).
    pub fn chain(&self, name: &str) -> Vec<&SoundClass> {
        let mut out = Vec::new();
        let mut cur = self.get(name);
        while let Some(c) = cur {
            if out.iter().any(|o: &&SoundClass| o.name == c.name) || out.len() > 64 {
                break;
            }
            out.push(c);
            cur = c.parent.as_deref().and_then(|p| self.get(p));
        }
        out
    }

    /// Resolve a class path/name to the bus + base volume used by the mixer. Unknown classes act like plain SFX.
    pub fn info(&self, name: &str) -> ClassInfo {
        let chain = self.chain(name);
        if chain.is_empty() {
            return ClassInfo { name: object_name(name).to_string(), ..Default::default() };
        }
        let mut bus = Bus::Sfx;
        for c in &chain {
            let b = match c.name.as_str() {
                "Music" => Some(Bus::Music),
                "SFX" => Some(Bus::Sfx),
                "Voice" => Some(Bus::Voice),
                "Fort_01_VOIP" => Some(Bus::VoiceChat),
                "Master" => Some(Bus::Master),
                _ => None,
            };
            if let Some(b) = b {
                bus = b;
                break;
            }
        }
        let mut vol = 1.0;
        let mut pitch = 1.0;
        for c in &chain {
            vol *= c.volume;
            pitch *= c.pitch;
        }
        ClassInfo {
            name: chain[0].name.clone(),
            base_volume: vol,
            pitch,
            bus,
            is_ui: chain.iter().any(|c| c.is_ui),
            always_play: chain.iter().any(|c| c.always_play),
            chain: chain.iter().map(|c| c.name.to_ascii_lowercase()).collect(),
        }
    }

    /// Volume of the `Master` class (0.7 in 1.8), applied on top of the user's master slider.
    pub fn master_volume(&self) -> f32 {
        self.get("Master").map(|c| c.volume).unwrap_or(1.0)
    }

    /// Load every `SoundClass` asset of the install (only the `FortSoundClassesAndModes` folder is scanned).
    pub fn load(install: &Install) -> Result<SoundClassTree> {
        let mut t = SoundClassTree::default();
        for p in install.all_asset_paths().into_iter().filter(|p| p.contains("/FortSoundClassesAndModes/")) {
            let Ok(data) = PackageData::load(&install.paks, &p) else { continue };
            let Ok(pkg) = data.package() else { continue };
            for i in pkg.exports_of_class("SoundClass").collect::<Vec<_>>() {
                let Ok(o) = pkg.read_export(i) else { continue };
                let props = o.get("Properties");
                let f = |n: &str| props.and_then(|p| p.field(n)).and_then(Value::as_f32);
                let b = |n: &str, d: bool| props.and_then(|p| p.field(n)).and_then(Value::as_bool).unwrap_or(d);
                t.insert(SoundClass {
                    name: o.name.clone(),
                    parent: o.get("ParentClass").and_then(Value::as_object).filter(|i| !i.is_null()).map(|i| object_name(&pkg.object_path(i)).to_string()),
                    volume: f("Volume").unwrap_or(1.0),
                    pitch: f("Pitch").unwrap_or(1.0),
                    is_ui: b("bIsUISound", false),
                    is_music: b("bIsMusic", false),
                    always_play: b("bAlwaysPlay", false),
                    apply_effects: b("bApplyEffects", true),
                    reverb: b("bReverb", true),
                });
            }
        }
        Ok(t)
    }
}

/// One `FSoundClassAdjuster` of a `SoundMix`.
#[derive(Clone, Debug)]
pub struct ClassAdjuster {
    pub class: String,
    pub volume: f32,
    pub pitch: f32,
    pub apply_to_children: bool,
}

/// `USoundMix`: temporary per-class volume/pitch adjusters with fade in / hold / fade out.
#[derive(Clone, Debug)]
pub struct SoundMix {
    pub name: String,
    pub initial_delay: f32,
    pub fade_in_time: f32,
    /// Seconds to hold before fading out; negative = until popped.
    pub duration: f32,
    pub fade_out_time: f32,
    pub adjusters: Vec<ClassAdjuster>,
}

impl SoundMix {
    pub fn load(install: &Install, path: &str) -> Result<Option<SoundMix>> {
        let ap = crate::install::to_pak_asset_path(path);
        let data = PackageData::load(&install.paks, &ap)?;
        let pkg = data.package()?;
        let Some(i) = pkg.exports_of_class("SoundMix").next() else { return Ok(None) };
        let o = pkg.read_export(i)?;
        let f = |n: &str, d: f32| o.get(n).and_then(Value::as_f32).unwrap_or(d);
        let adjusters = o
            .get("SoundClassEffects")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|e| {
                        let class = e.field("SoundClassObject").and_then(Value::as_object).filter(|i| !i.is_null()).map(|i| object_name(&pkg.object_path(i)).to_string())?;
                        Some(ClassAdjuster {
                            class,
                            volume: e.field("VolumeAdjuster").and_then(Value::as_f32).unwrap_or(1.0),
                            pitch: e.field("PitchAdjuster").and_then(Value::as_f32).unwrap_or(1.0),
                            apply_to_children: e.field("bApplyToChildren").and_then(Value::as_bool).unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Engine defaults of USoundMix (UE 4.16): FadeInTime 1.0, Duration -1, FadeOutTime 1.0.
        Ok(Some(SoundMix { name: o.name.clone(), initial_delay: f("InitialDelay", 0.0), fade_in_time: f("FadeInTime", 1.0), duration: f("Duration", -1.0), fade_out_time: f("FadeOutTime", 1.0), adjusters }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> SoundClassTree {
        let mut t = SoundClassTree::default();
        let mk = |n: &str, p: Option<&str>, v: f32| SoundClass { name: n.into(), parent: p.map(str::to_string), volume: v, pitch: 1.0, is_ui: n == "Fort_UI", is_music: false, always_play: false, apply_effects: true, reverb: false };
        t.insert(mk("Master", None, 0.7));
        t.insert(mk("SFX", Some("Master"), 0.5));
        t.insert(mk("Music", Some("Master"), 0.5));
        t.insert(mk("Fort_Weapons", Some("SFX"), 0.8));
        t.insert(mk("Fort_WeaponMelee", Some("Fort_Weapons"), 1.0));
        t.insert(mk("Fort_UI", Some("SFX"), 1.0));
        t
    }

    #[test]
    fn chain_product_and_bus() {
        let t = tree();
        let i = t.info("/Game/Sounds/FortSoundClassesAndModes/Fort_WeaponMelee.Fort_WeaponMelee");
        assert_eq!(i.bus, Bus::Sfx);
        assert!((i.base_volume - 0.28).abs() < 1e-6, "{}", i.base_volume); // 0.7 x 0.5 x 0.8 x 1.0
        assert!(t.info("Fort_UI").is_ui);
        assert_eq!(t.info("Music").bus, Bus::Music);
        assert_eq!(t.info("NoSuchClass").bus, Bus::Sfx);
        assert!((t.master_volume() - 0.7).abs() < 1e-6);
    }

    #[test]
    fn cycles_do_not_hang() {
        let mut t = SoundClassTree::default();
        for (n, p) in [("A", "B"), ("B", "A")] {
            t.insert(SoundClass { name: n.into(), parent: Some(p.into()), volume: 0.5, pitch: 1.0, is_ui: false, is_music: false, always_play: false, apply_effects: true, reverb: false });
        }
        assert_eq!(t.chain("A").len(), 2);
    }
}

