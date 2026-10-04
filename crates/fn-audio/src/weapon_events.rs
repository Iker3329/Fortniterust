//! Which SoundCue the weapons and consumables of the starter loadout use in Fortnite 1.8 (Athena), with provenance: the Assault Rifle
//! (`WID_Assault_Auto_Athena_C_Ore_T02` -> `B_Assault_Auto_Athena_C`), the Pump Shotgun (`WID_Shotgun_Standard_Athena_C_Ore_T03` ->
//! `B_Shotgun_Standard_Athena_C`), the Bolt-Action Sniper (`WID_Sniper_BoltAction_Scope_Athena_R_Ore_T03` -> `B_Rifle_Sniper_Athena_C`) and the
//! four healing consumables. Every row was read from cooked data (the Blueprint class default objects walked up their `SuperStruct` chain
//! `B_Xxx_Athena -> B_Xxx_Generic -> B_Ranged_Generic`, the item definitions, the `AnimNotify_PlaySound` notifies of the montages, the
//! `BulletWhipTrackerComponent` / `TracerGeneric_Athena` Blueprints). Paths are long package names (`/Game/...`).
//!
//! What the data does NOT give (left out, see `UNRECOVERED`): the native selection rule between `PrimaryFireSound[0..2]` (Normal / LowAmmo /
//! Degraded), when `PrimaryFireStopSound` plays, a separate bolt-cycle sound (the sniper montages carry no sound notifies; the bolt work is
//! inside `Weapon_RFL_Sniper_Athena_Cue`'s own waves), a separate pump sound (same for the shotgun), impact sounds of the sniper (its CDO
//! nulls `ImpactPhysicalSurfaceSounds[0..10]`).
use crate::events::{EventCue, Source, Source::Recovered};

/// The three firearms of the starter loadout (quickbar slots 2..4, loadout items 0..3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gun {
    Assault,
    Shotgun,
    Sniper,
}
impl Gun {
    pub const ALL: [Gun; 3] = [Gun::Assault, Gun::Shotgun, Gun::Sniper];
    /// Loadout item index -> gun.
    pub fn from_item(i: usize) -> Option<Gun> {
        Gun::ALL.get(i).copied()
    }
    pub fn key(self) -> &'static str {
        match self {
            Gun::Assault => "ar",
            Gun::Shotgun => "shotgun",
            Gun::Sniper => "sniper",
        }
    }
}

/// `EFortReloadFXState` (registration order in the 1.8 executable: ReloadStart, ReloadCartridge, ReloadEnd). `AnimNotify_PlayReloadFX` calls
/// `FortWeapon::PlayReloadFX(stage)`; `ReloadSounds[0..3]` of the weapon are Start / Cartridge / End (the array names agree: Open2 / InsertSingle2 / Cock2,
/// Reload_Start / Reload_Insert / Reload_End). The native stage -> array index function is not in the install: index = enum value is INFERRED.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadStage {
    Start = 0,
    Cartridge = 1,
    End = 2,
}
impl ReloadStage {
    /// `EFortReloadFXState::ReloadEnd` -> `End` (the enum text of the notify property).
    pub fn from_enum_text(t: &str) -> Option<ReloadStage> {
        match t.rsplit("::").next()? {
            "ReloadStart" => Some(ReloadStage::Start),
            "ReloadCartridge" => Some(ReloadStage::Cartridge),
            "ReloadEnd" => Some(ReloadStage::End),
            _ => None,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            ReloadStage::Start => "start",
            ReloadStage::Cartridge => "cartridge",
            ReloadStage::End => "end",
        }
    }
}

/// One event -> cue row.
#[derive(Clone, Copy, Debug)]
pub struct WeaponCue {
    pub event: &'static str,
    pub cue: &'static str,
    pub source: Source,
    pub provenance: &'static str,
    /// `PitchMultiplier` of the `AnimNotify_PlaySound` that plays it (1.0 when none).
    pub pitch: f32,
}
impl WeaponCue {
    pub fn as_event_cue(&self) -> EventCue {
        EventCue { event: self.event, cue: self.cue, source: self.source, provenance: self.provenance }
    }
}

const fn row(event: &'static str, cue: &'static str, provenance: &'static str) -> WeaponCue {
    WeaponCue { event, cue, source: Recovered, provenance, pitch: 1.0 }
}
const fn row_pitch(event: &'static str, cue: &'static str, provenance: &'static str, pitch: f32) -> WeaponCue {
    WeaponCue { event, cue, source: Recovered, provenance, pitch }
}

const FIRE_AR: &str = "B_Assault_Auto_Athena_C CDO .PrimaryFireSound[0] (played on its `Normal` AudioComponent, attached to WeaponMesh0 = at the weapon). Close / medium / distant layers (the fire tail / echo) are inside the cue.";
const FIRE_SG: &str = "B_Shotgun_Standard_Athena_C CDO .PrimaryFireSound[0]. Close / medium / distant layers inside the cue.";
const FIRE_SN: &str = "B_Rifle_Sniper_Athena_C CDO .PrimaryFireSound[0]. Close / medium / distant layers inside the cue.";
const LOW_AR: &str = "B_Assault_Auto_Athena_C CDO .PrimaryFireSound[1] (`LowAmmo` AudioComponent). The native threshold that selects it is not in the install: NOT played.";
const DEG_AR: &str = "B_Assault_Auto_Athena_C CDO .PrimaryFireSound[2] (`Degraded` AudioComponent). Durability: the WID has bItemHasDurability=false: NOT played.";
const LOW_SG: &str = "B_Shotgun_Standard_Athena_C CDO .PrimaryFireSound[1] (`LowAmmo`). Selection rule not recovered: NOT played.";
const DEG_SG: &str = "B_Shotgun_Standard_Athena_C CDO .PrimaryFireSound[2] (`Degraded`). Not played (no durability).";
const LOW_SN: &str = "B_Rifle_Sniper_Athena_C CDO (inherited B_Rifle_Generic) .PrimaryFireSound[1]. Selection rule not recovered: NOT played.";
const DEG_SN: &str = "B_Rifle_Sniper_Athena_C CDO (inherited B_Rifle_Generic) .PrimaryFireSound[2]. Not played (no durability).";
const EMPTY_R: &str = "B_Rifle_Generic CDO .OutOfAmmoSound (inherited by B_Assault_Auto_Athena_C via B_Assault_Generic and by B_Rifle_Sniper_Athena_C). Played at the weapon when the trigger is pulled on an empty magazine (the native call site is not in the install: the property name is the evidence).";
const EMPTY_SG: &str = "B_Shotgun_Generic CDO .OutOfAmmoSound (inherited by B_Shotgun_Standard_Athena_C). Played at the weapon when the trigger is pulled on an empty magazine (property name is the evidence).";
const RELOAD_AR: &str = "B_Assault_Generic CDO .ReloadSounds[N] (inherited by B_Assault_Auto_Athena_C); played by FortWeapon::PlayReloadFX(stage) which the AnimNotify_PlayReloadFX notifies of AssaultRifle_FCBA_Reload_CMM (Start 0.4447 s, Cartridge 1.3753 s, End 1.9062 s) and of Shotgun_Default_Equip_CMM (End, 0.2410 s) call. Stage -> index: INFERRED (enum order = array order).";
const RELOAD_SG: &str = "B_Shotgun_Generic CDO .ReloadSounds[N]; stage notifies of Shotgun_RG_PABF_Reload_M (Start 0.0771 s, Cartridge 0.7406 s once per loop pass = one per shell, End 1.9331 s) and Shotgun_Default_Equip_CMM (End). Stage -> index: INFERRED.";
const RELOAD_SN: &str = "B_Rifle_Generic CDO .ReloadSounds[N] (inherited by B_Rifle_Sniper_Athena_C; the sniper uses the Assault Rifle reload cues); stage notifies of SniperRifle_FCSA_Re3Sec_CMM_M (Start 0.4861 s, Cartridge 1.6016 s, End 2.1187 s) and Shotgun_Default_Equip_CMM (End). Stage -> index: INFERRED.";
const STOP: &str = "B_Rifle_Generic CDO .PrimaryFireStopSound (the Sniper inherits it; the Assault Rifle CDO sets it null). The native rule that plays it is not recovered: NOT played.";
const PICKUP: &str = "WID .PickupSound (FortWorldItemDefinition) - played when the item is picked up (native; no pickup mechanic exists in the sandbox).";
const DROP: &str = "WID .DropSound.";
const IMPACT_AR: &str = "B_Assault_Auto_Athena_C CDO (+ B_Assault_Generic / B_Ranged_Generic) .ImpactPhysicalSurfaceSounds[N], N = EPhysicalSurface index (1 wood, 2 stone, 3 metal, 4 player, 5 opponent, 6 husk, 7 smasher, 8 water by the cue names; the project's surface enum names were not read).";
const IMPACT_SHOTGUN: &str = "B_Shotgun_Standard_Athena_C CDO (+ B_Shotgun_Generic / B_Ranged_Generic) .ImpactPhysicalSurfaceSounds[N], N = EPhysicalSurface index.";
const IMPACT_SNIPER: &str = "B_Rifle_Sniper_Athena_C CDO: nulls [0..10]; only [19] is inherited from B_Ranged_Generic.";

/// Every weapon / consumable event -> cue row (the canonical table; `fn-audio-pack` exports the closure of all of them).
pub const TABLE: &[WeaponCue] = &[
    row("weapon.ar.fire", "/Game/Athena/Sounds/Weapons/Striker_Auto/Weapon_ASLT_AutoSF_Striker_Athena_Cue", FIRE_AR),
    row("weapon.ar.fire.lowammo", "/Game/Sounds/Fort_Weapons/LowAmmo/Rifle_Auto_Low_Ammo_Cue", LOW_AR),
    row("weapon.ar.fire.degraded", "/Game/Sounds/Fort_Weapons/Degrades/Rifle_Standard_T00_Degrade_Cue", DEG_AR),
    row("weapon.ar.empty", "/Game/Sounds/Fort_Weapons/Rifle/Rifle_Empty_Cue", EMPTY_R),
    row("weapon.ar.reload.start", "/Game/Sounds/Fort_Weapons/AutoRifle/Weapon_ASLT_Auto_Reload_Start_Cue", RELOAD_AR),
    row("weapon.ar.reload.cartridge", "/Game/Sounds/Fort_Weapons/AutoRifle/Weapon_ASLT_Auto_Reload_Insert_Cue", RELOAD_AR),
    row("weapon.ar.reload.end", "/Game/Sounds/Fort_Weapons/AutoRifle/Weapon_ASLT_Auto_Reload_End_Cue", RELOAD_AR),
    row("weapon.ar.pickup", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/AR_Pickup_Cue", PICKUP),
    row("weapon.ar.drop", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/AR_Drop_Cue", DROP),
    row("weapon.shotgun.fire", "/Game/Sounds/Fort_Weapons/Shotgun/Weapon_Pump_Shotgun_Fire_Athena_Cue", FIRE_SG),
    row("weapon.shotgun.fire.lowammo", "/Game/Sounds/Fort_Weapons/LowAmmo/Shotgun_Standard_Low_Ammo_Cue", LOW_SG),
    row("weapon.shotgun.fire.degraded", "/Game/Sounds/Fort_Weapons/Degrades/Shotgun_Standard_T01_Degrade_Cue", DEG_SG),
    row("weapon.shotgun.empty", "/Game/Sounds/Fort_Weapons/Shotgun/Shotgun_Empty_Cue", EMPTY_SG),
    row("weapon.shotgun.reload.start", "/Game/Sounds/Fort_Weapons/Shotgun/Weapon_Shotgun_Open2_Cue", RELOAD_SG),
    row("weapon.shotgun.reload.cartridge", "/Game/Sounds/Fort_Weapons/Shotgun/Weapon_Shotgun_InsertSingle2_Cue", RELOAD_SG),
    row("weapon.shotgun.reload.end", "/Game/Sounds/Fort_Weapons/Shotgun/Weapon_Shotgun_Cock2_Cue", RELOAD_SG),
    row("weapon.shotgun.pickup", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/Shotgun_Pickup_Cue", PICKUP),
    row("weapon.shotgun.drop", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/Shotgun_Drop_Cue", DROP),
    row("weapon.sniper.fire", "/Game/Sounds/Fort_Weapons/Rifle/Weapon_RFL_Sniper_Athena_Cue", FIRE_SN),
    row("weapon.sniper.fire.lowammo", "/Game/Sounds/Fort_Weapons/LowAmmo/Rifle_Auto_Low_Ammo_Cue", LOW_SN),
    row("weapon.sniper.fire.degraded", "/Game/Sounds/Fort_Weapons/Degrades/Rifle_Standard_T00_Degrade_Cue", DEG_SN),
    row("weapon.sniper.fire.stop", "/Game/Sounds/Fort_Weapons/AutoRifle/Weapon_ASLT_Auto_Stop_Cue", STOP),
    row("weapon.sniper.empty", "/Game/Sounds/Fort_Weapons/Rifle/Rifle_Empty_Cue", EMPTY_R),
    row("weapon.sniper.reload.start", "/Game/Sounds/Fort_Weapons/AutoRifle/Weapon_ASLT_Auto_Reload_Start_Cue", RELOAD_SN),
    row("weapon.sniper.reload.cartridge", "/Game/Sounds/Fort_Weapons/AutoRifle/Weapon_ASLT_Auto_Reload_Insert_Cue", RELOAD_SN),
    row("weapon.sniper.reload.end", "/Game/Sounds/Fort_Weapons/AutoRifle/Weapon_ASLT_Auto_Reload_End_Cue", RELOAD_SN),
    row("weapon.sniper.pickup", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/Rifle_Pickup_Cue", PICKUP),
    row("weapon.sniper.drop", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/Rifle_Drop_Cue", DROP),
    row("weapon.sniper.scope.zoom_in", "/Game/Sounds/Fort_Foley_Sounds/Moves/Player_Scope_ZoomIn_Cue", "B_Ranged_Generic CDO .Sound_ScopeZoomIn; B_Ranged_Generic.OnSetTargeting spawns it attached to the WeaponMesh (SpawnSoundAttached) when scope targeting begins. The sandbox has no aim-down-sights state: NOT triggered."),
    row("weapon.sniper.scope.zoom_out", "/Game/Sounds/Fort_Foley_Sounds/Moves/Player_Scope_ZoomOut_Cue", "B_Ranged_Generic CDO .Sound_ScopeZoomOut (end of scope targeting). NOT triggered (no ADS)."),
    row("weapon.equip", "/Game/Sounds/Fort_GamePlay_Sounds/Player/Player_Equip_Projectile_Equip_Cue", "Shotgun_Default_Equip_CMM (the EquipAnimation of all three firearms) AnimNotify_PlaySound 'Player_Equip_Projectile_Equip_Cue' at 0.1803 s, bFollow = true (attached to the player). The same montage also has PlayReloadFX(ReloadEnd) at 0.2410 s = ReloadSounds[End] of the held weapon."),
    row_pitch("heal.apply", "/Game/Sounds/Fort_GamePlay_Sounds/Player/Bandage_Apply_Athena_Cue", "AnimNotify_PlaySound 'Bandage_Apply_Athena_Cue' of WrapBandages_Athena (0.0393 s), ApplyMedkit_Athena (0.1166 s), ApplyShield_Athena (0.3931 s) and ApplyPurple_Athena (0.3931 s).", 1.0),
    row_pitch("heal.restore.health", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/EatHealth_Restore_Athena_Cue", "AnimNotify_PlaySound 'EatHealth_Restore_Athena_Cue' PitchMultiplier 1.5 at the TriggerAbility notify of WrapBandages_Athena (3.5086 s) and ApplyMedkit_Athena (10.0667 s).", 1.5),
    row_pitch("heal.restore.shield", "/Game/Sounds/Fort_GamePlay_Sounds/Loot/Sheild_Restore_Athena_Cue", "AnimNotify_PlaySound 'Sheild_Restore_Athena_Cue' PitchMultiplier 1.5 at ApplyShield_Athena (5.1223 s) and ApplyPurple_Athena (1.9968 s).", 1.5),
    row("pickup.bandage", "/Game/Sounds/Fort_Gadgets/Teleporter/bandage_pickup_Athena_Cue", "Athena_Bandage .PickupSound."),
    row("pickup.medkit", "/Game/Sounds/Fort_Gadgets/Teleporter/medkit_pickup_Athena_Cue", "Athena_Medkit .PickupSound."),
    row("pickup.shield", "/Game/Sounds/Fort_Gadgets/Teleporter/shield_pickup_Athena_Cue", "Athena_Shields .PickupSound (Athena_PurpleStuff uses the same cue)."),
    row("whip.far", "/Game/Athena/Sounds/Weapons/BulletWhips/bullet_whip_Cue", "TracerGeneric_Athena / TracerGeneric .PassByFarSound (BulletWhipTrackerComponent .PassByFarSound). Played by PlayPassBySound at Location with VolumeMultiplier = 1 - closeness."),
    row("whip.close", "/Game/Athena/Sounds/Weapons/BulletWhips/bullet_whip_heavy_Cue", "TracerGeneric_Athena / TracerGeneric .PassByCloseSound. Played with VolumeMultiplier = closeness."),
    row("impact.ar.s0", "/Game/Sounds/Fort_Weapons/Impacts/Pistol_Ricochet_Generic_Cue", IMPACT_AR),
    row("impact.ar.s1", "/Game/Sounds/Fort_Weapons/Impacts/WOOD_Bullet_Impact_Ricochet_Light_Cue", IMPACT_AR),
    row("impact.ar.s2", "/Game/Sounds/Fort_Weapons/Impacts/STONE_Bullet_Impact_Ricochet_Light_Cue", IMPACT_AR),
    row("impact.ar.s3", "/Game/Sounds/Fort_Weapons/Impacts/MTL_Bullet_Impact_Ricochet_Light_Cue", IMPACT_AR),
    row("impact.ar.s4", "/Game/Sounds/Fort_Weapons/Impacts/Bullet_Player_Impact_Athena_Cue", IMPACT_AR),
    row("impact.ar.s5", "/Game/Sounds/Fort_Weapons/Impacts/Rifle_Opponent_Impact_Cue", IMPACT_AR),
    row("impact.ar.s6", "/Game/Sounds/Fort_Monster_Husk/SFX/Husk_RifleBullet_Impact_Cue", IMPACT_AR),
    row("impact.ar.s7", "/Game/Sounds/Fort_Monster_Smasher/SFX/Smasher_RifleBullet_Impact_Cue", IMPACT_AR),
    row("impact.ar.s8", "/Game/Sounds/Fort_Weapons/Impacts/Water_Bullet_Impact_Cue", IMPACT_AR),
    row("impact.ar.s9", "/Game/Sounds/Fort_Weapons/Impacts/Pistol_Ricochet_Generic_Cue", IMPACT_AR),
    row("impact.ar.s10", "/Game/Sounds/Fort_Weapons/Impacts/Pistol_Ricochet_Generic_Cue", IMPACT_AR),
    row("impact.ar.s19", "/Game/Sounds/Fort_Weapons/Impacts/MTL_Bullet_Impact_Ricochet_Med_Cue", IMPACT_AR),
    row("impact.shotgun.s0", "/Game/Sounds/Fort_Weapons/Impacts/Shotgun_Ricochet_Generic_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s1", "/Game/Sounds/Fort_Weapons/Impacts/WOOD_Bullet_Impact_Ricochet_Heavy_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s2", "/Game/Sounds/Fort_Weapons/Impacts/STONE_Bullet_Impact_Ricochet_Heavy_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s3", "/Game/Sounds/Fort_Weapons/Impacts/MTL_Bullet_Impact_Ricochet_Heavy_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s4", "/Game/Sounds/Fort_Weapons/Impacts/Bullet_Player_Impact_Athena_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s5", "/Game/Sounds/Fort_Weapons/Impacts/Shotgun_Opponent_Impact_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s6", "/Game/Sounds/Fort_Monster_Husk/SFX/Husk_PistolBullet_Impact_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s7", "/Game/Sounds/Fort_Monster_Smasher/SFX/Smasher_PistolBullet_Impact_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s8", "/Game/Sounds/Fort_Weapons/Impacts/Water_Bullet_Impact_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s9", "/Game/Sounds/Fort_Weapons/Impacts/Shotgun_Ricochet_Generic_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s10", "/Game/Sounds/Fort_Weapons/Impacts/Shotgun_Ricochet_Generic_Cue", IMPACT_SHOTGUN),
    row("impact.shotgun.s19", "/Game/Sounds/Fort_Weapons/Impacts/MTL_Bullet_Impact_Ricochet_Med_Cue", IMPACT_SHOTGUN),
    row("impact.sniper.s19", "/Game/Sounds/Fort_Weapons/Impacts/MTL_Bullet_Impact_Ricochet_Med_Cue", IMPACT_SNIPER),
];

/// What the data does not decide (reported, not played).
pub const UNRECOVERED: &[&str] = &[
    "PrimaryFireSound[1] (LowAmmo) / [2] (Degraded) selection: native, threshold unknown (cues exported but never played)",
    "PrimaryFireStopSound trigger rule (native)",
    "bolt-action cycle sound: no sound notifies in the sniper fire / reload montages or sequences and no BP property names one",
    "shotgun pump sound: the pump montage notify is ShellsOnPumpFX (particles only); no pump sound property",
    "sniper bullet-impact sounds: the CDO nulls ImpactPhysicalSurfaceSounds[0..10]; Athena/Sounds/Impacts/Sniper_Rifle_*_Cue are not referenced by any asset in the weapon class chain",
    "aim-down-sights / scope: no ADS state in the sandbox, so the scope zoom cues are not triggered",
    "PlayReloadFX stage -> ReloadSounds index (native; enum order assumed)",
    "OutOfAmmoSound call site (native; played on a trigger pull with an empty magazine)",
];

pub fn lookup(event: &str) -> Option<&'static WeaponCue> {
    TABLE.iter().find(|e| e.event == event)
}

/// Row whose cue is the package `path` (an object path `/Game/X/Y.Y` is accepted); the first such row wins.
pub fn by_cue_path(path: &str) -> Option<&'static WeaponCue> {
    let p = path.split('.').next().unwrap_or(path);
    TABLE.iter().find(|e| e.cue.eq_ignore_ascii_case(p))
}

pub fn fire_event(g: Gun) -> &'static str {
    match g {
        Gun::Assault => "weapon.ar.fire",
        Gun::Shotgun => "weapon.shotgun.fire",
        Gun::Sniper => "weapon.sniper.fire",
    }
}
pub fn empty_event(g: Gun) -> &'static str {
    match g {
        Gun::Assault => "weapon.ar.empty",
        Gun::Shotgun => "weapon.shotgun.empty",
        Gun::Sniper => "weapon.sniper.empty",
    }
}
pub fn reload_event(g: Gun, s: ReloadStage) -> &'static str {
    match (g, s) {
        (Gun::Assault, ReloadStage::Start) => "weapon.ar.reload.start",
        (Gun::Assault, ReloadStage::Cartridge) => "weapon.ar.reload.cartridge",
        (Gun::Assault, ReloadStage::End) => "weapon.ar.reload.end",
        (Gun::Shotgun, ReloadStage::Start) => "weapon.shotgun.reload.start",
        (Gun::Shotgun, ReloadStage::Cartridge) => "weapon.shotgun.reload.cartridge",
        (Gun::Shotgun, ReloadStage::End) => "weapon.shotgun.reload.end",
        (Gun::Sniper, ReloadStage::Start) => "weapon.sniper.reload.start",
        (Gun::Sniper, ReloadStage::Cartridge) => "weapon.sniper.reload.cartridge",
        (Gun::Sniper, ReloadStage::End) => "weapon.sniper.reload.end",
    }
}
/// `impact.<gun>.s<N>` for `EPhysicalSurface` index N, if the weapon CDO names a sound for it.
pub fn impact_event(g: Gun, surface: u8) -> Option<&'static str> {
    let want = format!("impact.{}.s{}", g.key(), surface);
    TABLE.iter().find(|e| e.event == want).map(|e| e.event)
}

/// Whether the front-end can reach this row (the sink only decodes those up front): the low-ammo / degraded / fire-stop variants, the scope
/// zoom cues, the drop / pickup cues and the monster impacts have no trigger in the sandbox (see [`UNRECOVERED`]) and stay in the pack only.
pub fn is_played(w: &WeaponCue) -> bool {
    let e = w.event;
    !(e.ends_with(".fire.lowammo") || e.ends_with(".fire.degraded") || e.ends_with(".fire.stop") || e.contains(".scope.") || e.ends_with(".drop") || e.ends_with(".pickup") || e.starts_with("pickup.") || e.ends_with(".s6") || e.ends_with(".s7"))
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
            assert!(e.pitch > 0.);
        }
        assert_eq!(lookup("weapon.equip").unwrap().cue, "/Game/Sounds/Fort_GamePlay_Sounds/Player/Player_Equip_Projectile_Equip_Cue");
        assert_eq!(lookup("heal.restore.health").unwrap().pitch, 1.5);
    }
    #[test]
    fn every_gun_has_fire_empty_and_all_reload_stages() {
        for g in Gun::ALL {
            assert!(lookup(fire_event(g)).is_some() && lookup(empty_event(g)).is_some());
            for s in [ReloadStage::Start, ReloadStage::Cartridge, ReloadStage::End] {
                assert!(lookup(reload_event(g, s)).is_some(), "{g:?} {s:?}");
            }
        }
        assert_eq!(Gun::from_item(2), Some(Gun::Sniper));
        assert_eq!(Gun::from_item(3), None);
    }
    #[test]
    fn impacts_follow_the_cdo_arrays() {
        assert_eq!(lookup(impact_event(Gun::Assault, 1).unwrap()).unwrap().cue, "/Game/Sounds/Fort_Weapons/Impacts/WOOD_Bullet_Impact_Ricochet_Light_Cue");
        assert_eq!(lookup(impact_event(Gun::Shotgun, 1).unwrap()).unwrap().cue, "/Game/Sounds/Fort_Weapons/Impacts/WOOD_Bullet_Impact_Ricochet_Heavy_Cue");
        assert!(impact_event(Gun::Sniper, 1).is_none(), "the sniper CDO nulls the surface sounds");
        assert!(impact_event(Gun::Sniper, 19).is_some());
    }
    #[test]
    fn notify_cue_paths_resolve_to_rows() {
        assert_eq!(by_cue_path("/Game/Sounds/Fort_GamePlay_Sounds/Player/Bandage_Apply_Athena_Cue.Bandage_Apply_Athena_Cue").unwrap().event, "heal.apply");
        assert_eq!(ReloadStage::from_enum_text("EFortReloadFXState::ReloadCartridge"), Some(ReloadStage::Cartridge));
    }
}