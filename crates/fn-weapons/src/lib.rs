//! Original serialized weapon data and narrowly recovered native operations.
//! No invented weapon scheduling, inventory or firing implementation.
pub mod timeline;
pub mod recovered_reload;
pub mod reload_ammo; // AMMO HOOK
pub mod recovered_inventory;
pub mod recovered_panel; // WIDGETS3 HOOK: AthenaInventoryPanel / EquipSlot / DropSlot / TileButton bytecode decisions
pub mod recovered_healing;
pub mod native_inventory;
pub mod native_quickbar;
pub mod native_attachment;
pub mod recovered_ads; // ADS HOOK
pub mod recovered_recoil; // RECOIL HOOK

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trigger { Automatic, OnPress, OnRelease, NotSerialized, Unsupported(String) }

impl Trigger {
    /// Value of `UFortWeaponRangedItemDefinition::TriggerType` (+0x568) when the WID does not serialize it, recovered from the 1.8 exe:
    /// the class ctor (RVA 0x9164b0) never stores to +0x568 so the default is enum value 0, and `EFortWeaponTriggerType`'s registration
    /// (`Z_Construct_UEnum`, RVA 0xaa9780) lists OnPress (0), Automatic, OnRelease. The data agrees: of 55 Athena WIDs only Automatic
    /// and OnRelease are ever serialized, never OnPress (every pistol / pump / six-shooter / bolt sniper omits it).
    pub const NATIVE_DEFAULT: Trigger = Trigger::OnPress;
}

#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    pub item: String,
    pub display_name: Option<String>,
    pub rarity: Option<String>,
    pub stat_row: String,
    pub ammo: String,
    pub clip_size: u32,
    pub ammo_cost: u32,
    pub cartridges: u32,
    pub bullets_per_cartridge: u32,
    pub firing_rate: f32,
    pub reload_time: f32,
    pub individual_reload: bool,
    pub damage_pb: f32,
    pub critical_multiplier: f32,
    pub trigger: Trigger,
    pub assets: Assets,
}

/// Keep player montage, weapon montage and underlying sequence references distinct.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Assets {
    pub weapon_mesh: Option<String>, pub icon: Option<String>, pub actor: Option<String>,
    pub player_equip: Option<String>, pub player_fire: Option<String>, pub player_fire_ads: Option<String>,
    pub player_reload: Option<String>, pub weapon_fire: Option<String>, pub weapon_fire_ads: Option<String>,
    pub weapon_reload: Option<String>, pub projectile: Option<String>,
    pub camera: Option<String>, pub targeting_camera: Option<String>,
    pub muzzle: Option<String>, pub fire_sound: Option<String>, pub empty_sound: Option<String>,
}

impl Definition {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.item.is_empty() || self.stat_row.is_empty() || self.ammo.is_empty() { return Err("missing item/stat/ammo identity"); }
        if self.clip_size == 0 || self.ammo_cost == 0 || self.cartridges == 0 || self.bullets_per_cartridge == 0 { return Err("invalid ammunition dimensions"); }
        if !self.firing_rate.is_finite() || self.firing_rate <= 0. || !self.reload_time.is_finite() || self.reload_time <= 0. { return Err("invalid firing/reload time"); }
        if !self.damage_pb.is_finite() || self.damage_pb < 0. || !self.critical_multiplier.is_finite() || self.critical_multiplier < 0. { return Err("invalid damage"); }
        Ok(())
    }
}

