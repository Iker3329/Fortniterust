//! WIDGETS3: what the inventory screen shows about an item, read from the original item definitions (`DisplayName`, `Description`, `Rarity`, `ItemType`, `MaxStackSize`, `SmallPreviewImage`).
//! The starter items (three firearms, the bandage) come from the loadout's own serialized properties; the pickaxe and the three build resources are read here.
use crate::{loader, ClipSourceDir};
use fn_ueio::props::{read_props, Ctx, PropList, PropValue};
use fn_ueio::{Package, Reader};

/// The pickaxe WID and the three `FortResourceItemDefinition`s of the starter inventory (wood, stone, metal).
pub const EXTRA_ITEMS: [&str; 4] = [
    "/Game/Athena/Items/Weapons/WID_Harvest_Pickaxe_Athena_C_T01",
    "/Game/Items/ResourcePickups/WoodItemData",
    "/Game/Items/ResourcePickups/StoneItemData",
    "/Game/Items/ResourcePickups/MetalItemData",
];
/// `EFortItemType` in the declaration order of the exe's enum name table (the value of `ItemType` is its index).
pub const ITEM_TYPE_NAMES: [&str; 20] = ["WorldItem", "Ammo", "Badge", "BackpackPickup", "BuildingPiece", "CharacterPart", "Deco", "EditTool", "Ingredient", "Food", "Gadget", "HomebaseGadget", "HeroAbility", "MissionItem", "Trap", "Weapon", "WeaponMelee", "WeaponRanged", "WeaponHarvest", "WorldResource"];

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogItem {
    pub path: String,
    pub name: String,
    pub description: String,
    /// 0 Handmade (Common) .. 4 Fine (Legendary), the loadout's mapping.
    pub rarity: u8,
    /// `EFortItemType` number: the serialized `ItemType` when the definition has one, else the type its class implies (`FortWeaponRangedItemDefinition` = WeaponRanged,
    /// `FortWeaponMeleeItemDefinition` = WeaponMelee, `FortResourceItemDefinition` = WorldResource; the class defaults live in the exe's native code and are ASSUMED from the names).
    pub item_type: u8,
    pub max_stack: Option<u32>,
    /// PNG name of the `SmallPreviewImage` texture (e.g. `T-Icon-Logs.png`) and its object path.
    pub icon: String,
    pub icon_path: String,
}

/// Source string of a serialized `FText` (`TextProperty` payload: flags u32, history type i8; type 0 = Base: namespace, key, source string).
pub fn ftext_source(raw: &[u8]) -> Option<String> {
    let mut r = Reader::new(raw);
    let _flags = r.u32().ok()?;
    let history = r.u8().ok()? as i8;
    if history != 0 { return None; }
    let _namespace = r.fstring().ok()?; let _key = r.fstring().ok()?;
    r.fstring().ok()
}
/// An FText property of `p` as text (`DisplayName`, `Description`, ...).
pub fn text_of(p: &PropList, key: &str) -> Option<String> {
    match p.get(key)? { PropValue::Raw(b) => ftext_source(b), PropValue::Str(s) => Some(s.clone()), _ => None }
}
fn rarity(v: &str) -> u8 { match v.rsplit("::").next().unwrap_or(v) { "Handmade" => 0, "Ordinary" => 1, "Sturdy" => 2, "Quality" => 3, "Fine" => 4, _ => 0 } }
fn type_of(class: &str, p: &PropList) -> u8 {
    if let Some(t) = p.text("ItemType") { if let Some(i) = ITEM_TYPE_NAMES.iter().position(|n| *n == t.rsplit("::").next().unwrap_or(t)) { return i as u8; } }
    if class.contains("Ranged") { 17 } else if class.contains("Melee") { 16 } else if class.contains("Resource") { 19 } else { 0 }
}
/// Catalog entry of a definition from its class name, serialized properties and the icon file name.
pub fn entry(path: &str, class: &str, p: &PropList, icon_path: &str) -> CatalogItem {
    let stem = icon_path.split('.').next().unwrap_or(icon_path).rsplit('/').next().unwrap_or_default();
    CatalogItem {
        path: path.to_owned(), name: text_of(p, "DisplayName").unwrap_or_default(), description: text_of(p, "Description").unwrap_or_default(),
        rarity: p.text("Rarity").map(rarity).unwrap_or(0), item_type: type_of(class, p), max_stack: p.int("MaxStackSize").map(|v| v.max(0) as u32),
        icon: format!("{stem}.png"), icon_path: icon_path.to_owned(),
    }
}
fn read_props_of(pkg: &Package, idx: usize) -> Result<PropList, String> {
    let (start, end) = pkg.export_range(idx);
    read_props(&mut Reader::at(&pkg.data[..end], start), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())
}
/// Read the pickaxe and the resources.
pub fn read_extras(src: &ClipSourceDir) -> Result<Vec<CatalogItem>, String> {
    let mut out = vec![];
    for path in EXTRA_ITEMS {
        let pkg = loader::package(src, path)?;
        let (idx, class) = (0..pkg.exports.len()).find_map(|n| { let c = pkg.export_class(n); c.contains("ItemDefinition").then(|| (n, c)) }).ok_or_else(|| format!("{path}: no item definition export"))?;
        let p = read_props_of(&pkg, idx)?;
        let icon = match p.object("SmallPreviewImage").filter(|i| *i < 0) { Some(i) => pkg.import_path(i), None => return Err(format!("{path}: SmallPreviewImage is not an external reference")) };
        out.push(entry(path, &class, &p, &icon));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ftext(ns: &str, key: &str, src: &str) -> Vec<u8> {
        let mut v = vec![]; v.extend(0u32.to_le_bytes()); v.push(0u8);
        for s in [ns, key, src] { let b: Vec<u8> = s.bytes().chain(std::iter::once(0)).collect(); v.extend((b.len() as i32).to_le_bytes()); v.extend(b); }
        v
    }
    #[test] fn base_history_text_yields_the_source_string() {
        assert_eq!(ftext_source(&ftext("", "DD9B2AEF", "Bandage")).as_deref(), Some("Bandage"));
        assert_eq!(ftext_source(&ftext("[50CC]", "K", "Has healing properties. Get some health back.")).as_deref(), Some("Has healing properties. Get some health back."));
        let mut utf16 = vec![]; utf16.extend(0u32.to_le_bytes()); utf16.push(0u8); for s in ["", "k"] { let b: Vec<u8> = s.bytes().chain(std::iter::once(0)).collect(); utf16.extend((b.len() as i32).to_le_bytes()); utf16.extend(b); }
        let w: Vec<u16> = "Sch\u{f6}n".encode_utf16().chain(std::iter::once(0)).collect(); utf16.extend((-(w.len() as i32)).to_le_bytes()); for c in w { utf16.extend(c.to_le_bytes()); }
        assert_eq!(ftext_source(&utf16).as_deref(), Some("Sch\u{f6}n"));
        assert_eq!(ftext_source(&[0, 0, 0, 0, 0xFF]), None, "history None");
    }
    #[test] fn item_types_follow_the_enum_name_table() { assert_eq!(ITEM_TYPE_NAMES[16], "WeaponMelee"); assert_eq!(ITEM_TYPE_NAMES[17], "WeaponRanged"); assert_eq!(ITEM_TYPE_NAMES[18], "WeaponHarvest"); assert_eq!(ITEM_TYPE_NAMES[14], "Trap"); }
}
