//! INVENTORY HOOK: the inventory rules' data, read from the install: the Athena primary quickbar slot definitions (`QuickBarDefinitions[0]` of the Athena game mode CDO) and the
//! `UFortItemDefinition` flags of an item (`MaxStackSize`, `bAllowMultipleStacks`, `bInventorySizeLimited`, `bCanBeDropped`) with the class constructor defaults recovered from the exe.
//! The rule functions themselves are `fn_weapons::inventory_model`.
use crate::loadout_inventory::ITEM_TYPE_NAMES;
use crate::{loader, ClipSourceDir};
use fn_ueio::props::PropList;
use fn_weapons::inventory_model::{item_type, ItemRules};

/// `AcceptedItemTypes` (as `EFortItemType` numbers) of every primary slot definition of the Athena game mode, in slot order.
pub fn read_primary_slots(src: &ClipSourceDir) -> Result<Vec<Vec<u8>>, String> {
    use fn_uasset::Value;
    let path = crate::loadout_ammo::GAME_MODE;
    let (asset, exp) = match src {
        ClipSourceDir::Pak(set) => (set.read_game(path, "uasset").map_err(|e| format!("{path}: {e}"))?.into_owned(), set.read_game(path, "uexp").map(|c| c.into_owned()).unwrap_or_default()),
        ClipSourceDir::RawPackages(root) => { let base = root.join("FortniteGame/Content").join(path.trim_start_matches("/Game/")); (std::fs::read(format!("{}.uasset", base.display())).map_err(|e| e.to_string())?, std::fs::read(format!("{}.uexp", base.display())).unwrap_or_default()) }
        ClipSourceDir::FnAnimDir(_) => return Err("a .fnanim directory has no game mode".into()),
    };
    let pkg = fn_uasset::Package::parse(&asset, &exp).map_err(|e| format!("{path}: {e}"))?;
    let idx = pkg.exports.iter().position(|e| e.name.starts_with("Default__")).ok_or("game mode has no class default object")?;
    let obj = pkg.read_export(idx).map_err(|e| format!("{path}: {e}"))?;
    let bar = obj.props.get("QuickBarDefinitions").ok_or("game mode lacks QuickBarDefinitions")?;
    let slots = bar.field("QuickbarSlots").and_then(Value::as_array).ok_or("QuickBarDefinitions lacks QuickbarSlots")?;
    let mut out = vec![];
    for s in slots {
        let types = s.field("AcceptedItemTypes").and_then(Value::as_array).ok_or("slot lacks AcceptedItemTypes")?;
        let mut v = vec![];
        for t in types {
            let name = t.as_str().ok_or("AcceptedItemTypes entry is not an enum")?;
            let short = name.rsplit("::").next().unwrap_or(name);
            v.push(ITEM_TYPE_NAMES.iter().position(|n| *n == short).ok_or_else(|| format!("unknown EFortItemType {name}"))? as u8);
        }
        out.push(v);
    }
    Ok(out)
}

/// The inventory rules of an item definition: the class constructor defaults (`fn_weapons::inventory_model::ItemRules`), overridden by whatever the definition serializes.
pub fn rules_of(class: &str, p: &PropList) -> ItemRules {
    let mut r = if class.contains("Ammo") { ItemRules::ammo(i32::MAX as u32) }
        else if class.contains("Resource") { ItemRules::resource(i32::MAX as u32) }
        else if class.contains("WeaponRanged") || class.contains("WeaponMelee") || class.contains("WeaponItem") { ItemRules::weapon(1) }
        else { ItemRules { item_type: item_type::GADGET, max_stack: 999, allow_multiple_stacks: true, size_limited: false, can_be_dropped: true } };
    if let Some(t) = p.text("ItemType") { if let Some(i) = ITEM_TYPE_NAMES.iter().position(|n| *n == t.rsplit("::").next().unwrap_or(t)) { r.item_type = i as u8; } }
    if let Some(v) = p.int("MaxStackSize") { r.max_stack = v.max(1) as u32; }
    if let Some(v) = p.bool("bAllowMultipleStacks") { r.allow_multiple_stacks = v; }
    if let Some(v) = p.bool("bInventorySizeLimited") { r.size_limited = v; }
    if let Some(v) = p.bool("bCanBeDropped") { r.can_be_dropped = v; }
    r
}

/// Rules of the item definition at `path` (loads the package).
pub fn read_rules(src: &ClipSourceDir, path: &str) -> Result<ItemRules, String> {
    let long = path.split('.').next().unwrap_or(path);
    let pkg = loader::package(src, long)?;
    let (idx, class) = (0..pkg.exports.len()).find_map(|n| { let c = pkg.export_class(n); c.contains("ItemDefinition").then_some((n, c)) }).ok_or_else(|| format!("{long}: no item definition export"))?;
    let (start, end) = pkg.export_range(idx);
    let p = fn_ueio::props::read_props(&mut fn_ueio::Reader::at(&pkg.data[..end], start), &fn_ueio::props::Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())?;
    Ok(rules_of(&class, &p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_weapons::inventory_model::{accepted_types, PRIMARY_DEFINITIONS};
    fn src() -> Option<fn_pak::PakSet> {
        let root = fn_live::var_os("FN_TEST_ROOT").map(std::path::PathBuf::from)?;
        fn_pak::PakSet::open_install(root).ok()
    }
    /// The slot rules of `fn_weapons::inventory_model` are exactly the Athena game mode's `QuickBarDefinitions[0]`.
    #[test] fn slot_rules_equal_the_game_mode_data() {
        let Some(set) = src() else { eprintln!("FN_TEST_ROOT not set: skipped"); return };
        let slots = read_primary_slots(&ClipSourceDir::Pak(&set)).unwrap();
        assert_eq!(slots.len(), PRIMARY_DEFINITIONS);
        for (i, s) in slots.iter().enumerate() { assert_eq!(s.as_slice(), accepted_types(i), "slot {i}"); }
    }
    /// The class-default fallbacks agree with what the shipped definitions say.
    #[test] fn item_rules_of_the_real_definitions() {
        let Some(set) = src() else { return };
        let s = ClipSourceDir::Pak(&set);
        assert_eq!(read_rules(&s, "/Game/Athena/Items/Weapons/WID_Harvest_Pickaxe_Athena_C_T01").unwrap(), ItemRules::pickaxe());
        assert_eq!(read_rules(&s, "/Game/Items/ResourcePickups/WoodItemData").unwrap(), ItemRules::resource(999));
        assert_eq!(read_rules(&s, "/Game/Athena/Items/Ammo/AthenaAmmoDataBulletsLight").unwrap(), ItemRules::ammo(999));
        assert_eq!(read_rules(&s, "/Game/Athena/Items/Consumables/Bandage/Athena_Bandage").unwrap(), ItemRules::weapon(15));
        assert_eq!(read_rules(&s, "/Game/Athena/Items/Consumables/Medkit/Athena_Medkit").unwrap(), ItemRules::weapon(3));
        assert_eq!(read_rules(&s, "/Game/Athena/Items/Consumables/Shields/Athena_Shields").unwrap(), ItemRules::weapon(2));
    }
}

/// The inventory screen texts of the item definition at `path` (display name, description, rarity, type, stack size, icon), plus the object path of its `SmallPreviewImage` texture.
pub fn read_catalog_item(src: &ClipSourceDir, path: &str) -> Result<crate::loadout_inventory::CatalogItem, String> {
    let long = path.split('.').next().unwrap_or(path);
    let pkg = loader::package(src, long)?;
    let (idx, class) = (0..pkg.exports.len()).find_map(|n| { let c = pkg.export_class(n); c.contains("ItemDefinition").then_some((n, c)) }).ok_or_else(|| format!("{long}: no item definition export"))?;
    let (start, end) = pkg.export_range(idx);
    let p = fn_ueio::props::read_props(&mut fn_ueio::Reader::at(&pkg.data[..end], start), &fn_ueio::props::Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())?;
    let icon_path = p.object("SmallPreviewImage").filter(|i| *i < 0).map(|i| pkg.import_path(i)).unwrap_or_default();
    Ok(crate::loadout_inventory::entry(long, &class, &p, &icon_path))
}
