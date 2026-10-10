//! Which SoundCue each gameplay event uses in Fortnite 1.8 (Athena), with provenance.
//!
//! `Source::Recovered` rows were read straight from cooked data (the property that names the cue is given).
//! `Source::Assumed` rows are the closest content when the native code that picks the cue is not in the install; the
//! `note` says what the assumption is. All paths are long package names (`/Game/...`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Recovered,
    Assumed,
}

#[derive(Clone, Copy, Debug)]
pub struct EventCue {
    /// Stable event id (`"melee.swing"`).
    pub event: &'static str,
    pub cue: &'static str,
    pub source: Source,
    /// Where the reference was read (asset + property) or what was assumed.
    pub provenance: &'static str,
}

use Source::{Assumed, Recovered};

pub const SWING: &str = "/Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue";
pub const EQUIP_READY: &str = "/Game/Athena/Sounds/Weapons/PickAxe_Ready_Athena_Cue";
pub const IMPACT_DEFAULT: &str = "/Game/Sounds/Fort_Weapons/PickAxe/PickAxe_Impact_Default_Cue";
pub const IMPACT_TREE: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_Tree_Impact_Cue";
pub const FOOTSTEP_WALK: &str = "/Game/Athena/Sounds/Character/Athena_Character_Walk_Footstep_Cue";
pub const FOOTSTEP_SPRINT: &str = "/Game/Athena/Sounds/Character/Athena_Character_Sprint_Footstep_Cue";
pub const JUMP: &str = "/Game/Athena/Sounds/Character/Athena_Character_Jump_Cue";
pub const LAND: &str = "/Game/Athena/Sounds/Character/Athena_Character_Land_Cue";
pub const LAND_HARD: &str = "/Game/Athena/Sounds/Character/Athena_Character_LandHard_Cue";
pub const MENU_OPEN: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/Fort_UI_MenuOpen_Cue";
pub const MENU_CLOSE: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/New_FrontEnd/Fortnite_UI_Options_Menu_03_Cue";
pub const MENU_CLICK: &str = "/Game/Sounds/Fort_UI_Sounds/FrontEnd/Fort_UI_MenuButtonClick_Cue";
pub const AMBIENCE_DAY: &str = "/Game/Athena/Sounds/Ambient/Temperate_Grasslands_Hills_Day_Athena_Cue";

// CRACK AUDIO HOOK: weak-spot (`WeakSpot_C` / `FortCrackEffect`) sounds and the per-prop harvest hit / death sounds.
pub const CRACK_SPAWN: &str = "/Game/Sounds/Fort_GamePlay_Sounds/Demolition/Crack_Spawn_Cue";
pub const CRACK_IMPACT: &str = "/Game/Sounds/Fort_GamePlay_Sounds/Demolition/Crack_Impact_Cue";
/// `Sound_Crack_Level_01..09_Cue` (index = crack level 0..8).
pub const CRACK_LEVELS: [&str; 9] = [
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_01_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_02_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_03_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_04_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_05_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_06_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_07_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_08_Cue",
    "/Game/Sounds/Fort_UI_Sounds/Gameplay/Sound_Crack_Level_09_Cue",
];
/// The cue of crack level `level` (clamped to 0..=8, as `WeakSpot_C` clamps `CrackLevel`).
pub fn crack_level_cue(level: u8) -> &'static str {
    CRACK_LEVELS[(level as usize).min(CRACK_LEVELS.len() - 1)]
}
pub const OAK_IMPACT: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_OakTree_Impact_Cue";
pub const OAK_DESTROYED: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_OakTree_Destroyed_Cue";
pub const TREE_DESTROYED: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_Tree_Destroyed_Cue";
pub const PINE_IMPACT: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_PineTree_Impact_Cue";
pub const SMALLTREE_IMPACT: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_SmallTree_Impact_Cue";
pub const SMALLTREE_DESTROYED: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_SmallTree_Destroyed_Cue";
pub const BUSH_IMPACT: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_Bush_Impact_Cue";
pub const BUSH_DESTROYED: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_Bush_ImpactFinal_Cue";
pub const JUNK_LARGE: &str = "/Game/Sounds/Fort_Impact_Sounds/Demolition/Exterior_Props/Demo_Junk_Large_Impact_Cue";

/// `PlayHitSound` / `DeathSound` of one harvestable prop, as exported in its `props.json` (`destruction.hit_sound` / `destruction.death_sound`).
/// `None` = the class serialises no such sound (the rocks and bushes serialise neither).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropSound {
    pub id: &'static str,
    pub hit: Option<&'static str>,
    pub death: Option<&'static str>,
    pub impact: Option<&'static str>,
    pub damage: Option<&'static str>,
    pub container_death: Option<&'static str>,
}
const fn ps(id: &'static str, hit: Option<&'static str>, death: Option<&'static str>) -> PropSound {
    PropSound { id, hit, death, impact: Some(IMPACT_DEFAULT), damage: None, container_death: None }
}
/// Every prop of the harvest categories (`tree`, `rock`, `bush`, `hedge`) of the exported catalog that has a hit or a death sound
/// (RECOVERED from each `props.json`; the `Athena_Prop_Rocks_*`, `Prop_Rocks_*`, `Plant_Shrub_*`, `LCD_RubblePile_02` and `TelephonePole_*` exports have neither).
pub const PROP_SOUNDS: &[PropSound] = &[
    ps("Athena_Tree_Hero_02", Some(OAK_IMPACT), Some(OAK_DESTROYED)),
    ps("Athena_Tree_Large_1", Some(OAK_IMPACT), Some(OAK_DESTROYED)),
    ps("Athena_Tree_Medium_01_", Some(IMPACT_TREE), Some(TREE_DESTROYED)),
    ps("Athena_Tree_Medium_03_", Some(IMPACT_TREE), Some(TREE_DESTROYED)),
    ps("Athena_Tree_Medium_04_", Some(IMPACT_TREE), Some(TREE_DESTROYED)),
    ps("Athena_Tree_Pine_01", Some(PINE_IMPACT), Some(TREE_DESTROYED)),
    ps("Athena_Tree_Pine_05", None, Some(OAK_DESTROYED)),
    ps("Athena_Tree_Pine_3", Some(IMPACT_TREE), Some(TREE_DESTROYED)),
    ps("Athena_Tree_WeepingWillow_01", Some(IMPACT_TREE), Some(TREE_DESTROYED)),
    ps("Tree_04", Some(IMPACT_TREE), Some(TREE_DESTROYED)),
    ps("Tree_Bush_1a", Some(SMALLTREE_IMPACT), Some(SMALLTREE_DESTROYED)),
    ps("Tree_Bush_3", Some(SMALLTREE_IMPACT), Some(SMALLTREE_DESTROYED)),
    ps("Tree_Small_1", Some(SMALLTREE_IMPACT), Some(SMALLTREE_DESTROYED)),
    ps("Tree_StumpSet02", Some(IMPACT_TREE), Some(OAK_DESTROYED)),
    ps("Hedge_High_Rectangle", Some(BUSH_IMPACT), Some(BUSH_DESTROYED)),
    ps("Hedge_Low_Rectangle", Some(BUSH_IMPACT), Some(BUSH_DESTROYED)),
    ps("ToiletFactory_Frame_01", Some(JUNK_LARGE), Some(JUNK_LARGE)),
    ps("ToiletFactory_Frame_02", Some(JUNK_LARGE), Some(JUNK_LARGE)),
    ps("ToiletFactory_Frame_03", Some(JUNK_LARGE), Some(JUNK_LARGE)),
    ps("ToiletFactory_Frame_04", Some(JUNK_LARGE), Some(JUNK_LARGE)),
    ps("ToiletFactory_Frame_05", Some(JUNK_LARGE), Some(JUNK_LARGE)),
];
/// The hit / death sounds of a catalog prop id (`None` for props that have no sound property at all).
pub fn prop_sound(id: &str) -> Option<&'static PropSound> {
    PROP_SOUNDS.iter().find(|p| p.id == id)
}
/// Every cue of the weak-spot effect and of [`PROP_SOUNDS`], de-duplicated (what a game that plays the harvest feedback must prepare).
pub fn crack_and_prop_cues() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = vec![CRACK_SPAWN, CRACK_IMPACT];
    v.extend(CRACK_LEVELS);
    for p in PROP_SOUNDS {
        for c in [p.hit, p.death].into_iter().flatten() {
            if !v.contains(&c) {
                v.push(c);
            }
        }
    }
    v
}

/// Every event -> cue row (the canonical table; the CLI `events` command verifies each row loads and decodes).
pub const TABLE: &[EventCue] = &[
    EventCue { event: "crack.spawn", cue: CRACK_SPAWN, source: Recovered, provenance: "WeakSpot_C (= DefaultGameData.CrackEffectBlueprintClass, parent FortCrackEffect) .Crack_Spawn_Cue, played with CrackHitCount 0 (the hit that generates the weak spot). SoundCue graph: Mixer[Modulator(Random of Crack_Spawn_01..04, pitch 1.4..1.6), Modulator(crack_test_02)]; PitchMultiplier 1.5; Fort_WeaponMelee_PSM; Logarithmic 300 + 1000." },
    EventCue { event: "crack.impact", cue: CRACK_IMPACT, source: Recovered, provenance: "WeakSpot_C .Crack_Impact_Cue (a weak point was hit). QualityLevel(Mixer[Enveloper(Random of Crack_Impact_Sweet_ST_03..07), Random of crit_tune_*]); concurrency Crack_Impact_Cue (3, StopFarthestThenPreventNew)." },
    EventCue { event: "crack.level.1", cue: CRACK_LEVELS[0], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_01_Cue (CrackLevel 0)." },
    EventCue { event: "crack.level.2", cue: CRACK_LEVELS[1], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_02_Cue (CrackLevel 1)." },
    EventCue { event: "crack.level.3", cue: CRACK_LEVELS[2], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_03_Cue (CrackLevel 2)." },
    EventCue { event: "crack.level.4", cue: CRACK_LEVELS[3], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_04_Cue (CrackLevel 3)." },
    EventCue { event: "crack.level.5", cue: CRACK_LEVELS[4], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_05_Cue (CrackLevel 4)." },
    EventCue { event: "crack.level.6", cue: CRACK_LEVELS[5], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_06_Cue (CrackLevel 5)." },
    EventCue { event: "crack.level.7", cue: CRACK_LEVELS[6], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_07_Cue (CrackLevel 6)." },
    EventCue { event: "crack.level.8", cue: CRACK_LEVELS[7], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_08_Cue (CrackLevel 7)." },
    EventCue { event: "crack.level.9", cue: CRACK_LEVELS[8], source: Recovered, provenance: "WeakSpot_C .Sound_Crack_Level_09_Cue (CrackLevel 8)." },
    EventCue { event: "harvest.tree.oak.hit", cue: OAK_IMPACT, source: Recovered, provenance: "props.json destruction.hit_sound of Athena_Tree_Large_1 / Hero_02 (.PlayHitSound)." },
    EventCue { event: "harvest.tree.oak.destroyed", cue: OAK_DESTROYED, source: Recovered, provenance: "props.json destruction.death_sound of Athena_Tree_Large_1 / Hero_02 / Pine_05 / Tree_StumpSet02 (Athena_Tree_* CDO .DeathSound)." },
    EventCue { event: "harvest.tree.pine.hit", cue: PINE_IMPACT, source: Recovered, provenance: "props.json destruction.hit_sound of Athena_Tree_Pine_01 (.PlayHitSound)." },
    EventCue { event: "harvest.tree.small.hit", cue: SMALLTREE_IMPACT, source: Recovered, provenance: "props.json destruction.hit_sound of Tree_Bush_1a / Tree_Bush_3 / Tree_Small_1." },
    EventCue { event: "harvest.tree.small.destroyed", cue: SMALLTREE_DESTROYED, source: Recovered, provenance: "props.json destruction.death_sound of Tree_Bush_1a / Tree_Bush_3 / Tree_Small_1." },
    EventCue { event: "harvest.hedge.hit", cue: BUSH_IMPACT, source: Recovered, provenance: "props.json destruction.hit_sound of Hedge_High_Rectangle / Hedge_Low_Rectangle." },
    EventCue { event: "harvest.hedge.destroyed", cue: BUSH_DESTROYED, source: Recovered, provenance: "props.json destruction.death_sound of Hedge_High_Rectangle / Hedge_Low_Rectangle." },
    EventCue { event: "harvest.junk.large", cue: JUNK_LARGE, source: Recovered, provenance: "props.json destruction.hit_sound AND death_sound of ToiletFactory_Frame_01..05 (category rock, same cue for both)." },
    EventCue { event: "melee.swing", cue: SWING, source: Recovered, provenance: "B_Melee_Impact_Pickaxe_Athena_C CDO .PrimaryFireSound (WID_Harvest_Pickaxe_Athena_C_T01.WeaponActorClass). The PlayerMeleeCombo / TwoHanded_Loop_Swing montages carry no sound notifies, so it is played natively when the weapon fires = the FortAnimNotify_TriggerGameplayAbility notify (fn-combat's TriggerAbility moment). Played in 2D? No: SoundNodeAttenuation Fort_Weapon_Melee_Close/Medium_Athena_Att at the weapon." },
    EventCue { event: "melee.equip", cue: EQUIP_READY, source: Recovered, provenance: "B_Melee_Impact_Pickaxe_Athena_C CDO .ReloadSounds (ready/draw). Melee_Equip montage additionally plays /Game/Sounds/Fort_GamePlay_Sounds/Player/Player_Equip_Melee_Cue via AnimNotify_PlaySound." },
    EventCue { event: "melee.impact.default", cue: IMPACT_DEFAULT, source: Recovered, provenance: "B_Melee_Impact_Pickaxe_C CDO .ImpactPhysicalSurfaceSounds[0] (index = EPhysicalSurface; the 1.8 pickaxe array has 2 entries: [0] Default, [1] PickAxe_Impact_Husk_Cue). Any other surface index falls back to [0] (assumed native fallback)." },
    EventCue { event: "melee.impact.husk", cue: "/Game/Sounds/Fort_Weapons/PickAxe/PickAxe_Impact_Husk_Cue", source: Recovered, provenance: "B_Melee_Impact_Pickaxe_C CDO .ImpactPhysicalSurfaceSounds[1] (SurfaceType1; the base class B_Melee_Generic puts Wood at 1, the pickaxe overrides the array)." },
    EventCue { event: "harvest.tree.hit", cue: IMPACT_TREE, source: Recovered, provenance: "Athena_Tree_Medium_01_/Medium_03_/Medium_04_/Pine_3/WeepingWillow_01 CDO .PlayHitSound (oak variants Athena_Tree_Large_1/Hero_02: Demo_OakTree_Impact_Cue, Athena_Tree_Pine_01: Demo_PineTree_Impact_Cue). Played together with melee.impact.default." },
    EventCue { event: "harvest.tree.destroyed", cue: "/Game/Sounds/Fort_Impact_Sounds/Demolition/Wood/Demo_Tree_Destroyed_Cue", source: Recovered, provenance: "Athena_Tree_* CDO .DeathSound." },
    EventCue { event: "harvest.rock.hit", cue: "/Game/Sounds/Fort_Impact_Sounds/Demolition/Defaults/Demo_Rock_Impact_MLTI_Cue", source: Assumed, provenance: "Athena_Prop_Rocks_StoneLarge0x (BuildingProp, ResourceType Stone) serialises no hit-sound property; the cue is B_Melee_Generic .ImpactPhysicalSurfaceSounds[2] (Stone surface). The native BuildingProp default is not in the install." },
    EventCue { event: "harvest.wood.hit", cue: "/Game/Sounds/Fort_Impact_Sounds/Demolition/Defaults/Demo_Wood_Impact_MLTI_Cue", source: Assumed, provenance: "Wood-surface props (pallets, wood buildings): B_Melee_Generic .ImpactPhysicalSurfaceSounds[1]." },
    EventCue { event: "harvest.metal.hit", cue: "/Game/Sounds/Fort_Impact_Sounds/Demolition/Defaults/Demo_Metal_Impact_MLTI_Cue", source: Assumed, provenance: "Metal-surface props: B_Melee_Generic .ImpactPhysicalSurfaceSounds[3]." },
    EventCue { event: "move.footstep.walk", cue: FOOTSTEP_WALK, source: Recovered, provenance: "PlayerPawn_Athena_C CDO .DefaultFootstepSound (1P/local). The *_3P_Cue variants are for other players. Native FortPawn::PlayFootStepSound(FootIndex) is called from AnimNotify_FootStep_Left/Right." },
    EventCue { event: "move.footstep.sprint", cue: FOOTSTEP_SPRINT, source: Recovered, provenance: "PlayerPawn_Athena_C CDO .DefaultFastFootstepSound." },
    EventCue { event: "move.footstep.crouch", cue: "/Game/Athena/Sounds/Character/Athena_Character_Crouch_Footstep_Cue", source: Recovered, provenance: "PlayerPawn_Athena_C CDO .DefaultCrouchedFootstepSound." },
    EventCue { event: "move.footstep.crouch_sprint", cue: "/Game/Athena/Sounds/Character/Athena_Character_Crouch_Sprint_Footstep_Cue", source: Recovered, provenance: "PlayerPawn_Athena_C CDO .DefaultCrouchSprintFootstepSound." },
    EventCue { event: "move.footstep.grass", cue: FOOTSTEP_WALK, source: Assumed, provenance: "There is NO grass-specific footstep cue in the 1.8 Athena content: only Default{,Fast,Crouched,CrouchSprint}FootstepSound exist (grass = default surface). The only surface variant is Athena_Character_Bush_Footstep_Cue (inside a bush)." },
    EventCue { event: "move.jump", cue: JUMP, source: Recovered, provenance: "PlayerPawn_Athena_C CDO .DefaultJumpSound (PlayerPawn_Athena_Generic uses Player_Foley_Jump_Cue instead)." },
    EventCue { event: "move.land", cue: LAND, source: Recovered, provenance: "PlayerPawn_Athena_C CDO .DefaultLandingSound; hard landings use .DefaultHardLandingSound = Athena_Character_LandHard_Cue (the speed threshold is native and unknown; the sink uses 900 cm/s, ASSUMED)." },
    EventCue { event: "move.land_hard", cue: LAND_HARD, source: Recovered, provenance: "PlayerPawn_Athena_C CDO .DefaultHardLandingSound." },
    EventCue { event: "ui.menu.open", cue: MENU_OPEN, source: Assumed, provenance: "Name-matched: Fort_UI_Sounds/FrontEnd/Fort_UI_MenuOpen_Cue (the widget blueprints' UI sound bindings were not mapped)." },
    EventCue { event: "ui.menu.close", cue: MENU_CLOSE, source: Assumed, provenance: "Fortnite_UI_Options_Menu_03_Cue chosen as the options-menu dismiss sound (no explicit 'MenuClose' cue exists)." },
    EventCue { event: "ui.menu.click", cue: MENU_CLICK, source: Assumed, provenance: "Name-matched: Fort_UI_MenuButtonClick_Cue." },
    EventCue { event: "ambience.athena.day", cue: AMBIENCE_DAY, source: Assumed, provenance: "Athena/Sounds/Ambient/Temperate_Grasslands_Hills_{Morning,Day,Evening,Night}_Athena_Cue are the island-wide beds; which one plays is driven by the time-of-day Blueprint (not decoded). Day chosen." },
];

pub fn lookup(event: &str) -> Option<&'static EventCue> {
    TABLE.iter().find(|e| e.event == event)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_is_consistent() {
        let mut seen = std::collections::HashSet::new();
        for e in TABLE {
            assert!(seen.insert(e.event), "duplicate {}", e.event);
            assert!(e.cue.starts_with("/Game/") && !e.cue.contains('.'), "{}", e.cue);
        }
        assert_eq!(lookup("melee.swing").unwrap().cue, SWING);
    }
    // CRACK AUDIO HOOK
    #[test]
    fn crack_levels_map_one_to_one_and_clamp() {
        for l in 0..9u8 {
            assert!(crack_level_cue(l).ends_with(&format!("Sound_Crack_Level_{:02}_Cue", l + 1)), "{l}");
            assert_eq!(lookup(&format!("crack.level.{}", l + 1)).unwrap().cue, crack_level_cue(l));
        }
        assert_eq!(crack_level_cue(200), CRACK_LEVELS[8]);
        assert_eq!(lookup("crack.spawn").unwrap().cue, CRACK_SPAWN);
        assert_eq!(lookup("crack.impact").unwrap().cue, CRACK_IMPACT);
    }
    #[test]
    fn prop_sounds_are_unique_and_every_cue_is_in_the_table() {
        let mut ids = std::collections::HashSet::new();
        for p in PROP_SOUNDS {
            assert!(ids.insert(p.id), "duplicate {}", p.id);
            for c in [p.hit, p.death].into_iter().flatten() {
                assert!(TABLE.iter().any(|e| e.cue == c), "{c} has no TABLE row");
            }
        }
        assert_eq!(prop_sound("Athena_Tree_Large_1").unwrap().death, Some(OAK_DESTROYED));
        assert!(prop_sound("Prop_Rocks_01").is_none());
        let c = crack_and_prop_cues();
        let mut s = c.clone();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), c.len());
    }
}
