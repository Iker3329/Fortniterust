//! Original ammo data read from the install at load time (no values typed in here): the `FortAmmoItemDefinition` of every ammo type the
//! loadout's firearms use (`MaxStackSize`, `DropCount`, tags, icon) and the fresh Athena player's `StartingItems` (the Athena game mode's
//! class default object), from which the starting ammo is derived: the list is read and every entry is classified by its item class, so
//! "no starting ammo" is a result of the data, not an assumption.
use crate::{loader, ClipSourceDir};
use fn_ueio::{props::{read_props, Ctx, PropList}, Package, Reader};

/// The Athena game mode Blueprint whose CDO carries `StartingItems` / `bOverrideQuickBars` / the quickbar slot layout.
pub const GAME_MODE: &str = "/Game/Athena/Athena_GameMode";

/// One `FortAmmoItemDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub struct AmmoDef {
    /// Package path (`/Game/Athena/Items/Ammo/AthenaAmmoDataBulletsMedium`) and short name.
    pub item: String, pub name: String,
    /// `MaxStackSize`: the largest stack of this ammo one inventory slot holds.
    pub max_stack: u32,
    /// `DropCount`: ammo granted by one floor pickup of this type.
    pub drop_count: u32,
    /// `GameplayTags` entry (`supplies.ammo.bulletmedium`).
    pub tag: String,
}

/// One `ItemAndCount` of the game mode's `StartingItems` with the class of the item it names.
#[derive(Clone, Debug, PartialEq)]
pub struct StartingItem { pub item: String, pub count: i64, pub class: String }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AmmoSetup {
    /// One entry per distinct ammo type used by the loadout firearms, in first-use order.
    pub defs: Vec<AmmoDef>,
    /// The game mode's `StartingItems`, in data order.
    pub starting_items: Vec<StartingItem>,
    /// `bOverrideQuickBars` of the game mode (the starting items replace the default quickbar contents).
    pub override_quickbars: bool,
}
impl AmmoSetup {
    /// Index into `defs` of the ammo item with this package path (object suffix ignored).
    pub fn def_index(&self, ammo_path: &str) -> Option<usize> { let p = ammo_path.split('.').next().unwrap_or(ammo_path); self.defs.iter().position(|d| d.item == p) }
    /// Ammo granted by `StartingItems`: (index into `defs`, count) for every starting item that is a `FortAmmoItemDefinition` of a used ammo type.
    pub fn starting_ammo(&self) -> Vec<(usize, i64)> {
        self.starting_items.iter().filter(|s| s.class.contains("AmmoItemDefinition")).filter_map(|s| self.def_index(&s.item).map(|i| (i, s.count))).collect()
    }
}

fn package_name(r: &str) -> &str { r.split('.').next().unwrap_or(r) }
fn props_of(pkg: &Package, class: &str) -> Result<PropList, String> {
    let idx = pkg.find_export(class).ok_or_else(|| format!("missing {class}"))?;
    let (start, end) = pkg.export_range(idx);
    read_props(&mut Reader::at(&pkg.data[..end], start), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())
}

/// `FortAmmoItemDefinition` of one ammo package.
pub fn ammo_def(src: &ClipSourceDir, ammo_path: &str) -> Result<AmmoDef, String> {
    let long = package_name(ammo_path);
    let pkg = loader::package(src, long)?;
    let class = pkg.exports.iter().enumerate().find_map(|(i, _)| { let c = pkg.export_class(i); c.contains("AmmoItemDefinition").then_some(c) }).ok_or_else(|| format!("{long}: no FortAmmoItemDefinition export"))?;
    let p = props_of(&pkg, &class)?;
    let count = |k: &str| -> Result<u32, String> { p.int(k).filter(|v| *v >= 0).map(|v| v as u32).ok_or_else(|| format!("{long}: lacks {k}")) };
    let tag = p.fields("GameplayTags").and_then(|t| t.array("GameplayTags")).and_then(|a| a.first()).and_then(|v| v.as_str()).unwrap_or_default().to_owned();
    Ok(AmmoDef { item: long.to_owned(), name: long.rsplit('/').next().unwrap_or_default().to_owned(), max_stack: count("MaxStackSize")?, drop_count: count("DropCount")?, tag })
}

/// `StartingItems` + `bOverrideQuickBars` of the Athena game mode CDO (complete property decoder: `ItemAndCount` is a nested struct array).
pub fn game_mode_starting_items(src: &ClipSourceDir) -> Result<(Vec<StartingItem>, bool), String> {
    use fn_uasset::Value;
    let (asset, exp) = match src {
        ClipSourceDir::Pak(set) => (set.read_game(GAME_MODE, "uasset").map_err(|e| format!("{GAME_MODE}: {e}"))?.into_owned(), set.read_game(GAME_MODE, "uexp").map(|c| c.into_owned()).unwrap_or_default()),
        ClipSourceDir::RawPackages(root) => { let base = root.join("FortniteGame/Content").join(GAME_MODE.trim_start_matches("/Game/")); (std::fs::read(format!("{}.uasset", base.display())).map_err(|e| e.to_string())?, std::fs::read(format!("{}.uexp", base.display())).unwrap_or_default()) }
        ClipSourceDir::FnAnimDir(_) => return Err("a .fnanim directory has no game mode".into()),
    };
    let pkg = fn_uasset::Package::parse(&asset, &exp).map_err(|e| format!("{GAME_MODE}: {e}"))?;
    let idx = pkg.exports.iter().position(|e| e.name.starts_with("Default__")).ok_or("game mode has no class default object")?;
    let obj = pkg.read_export(idx).map_err(|e| format!("{GAME_MODE}: {e}"))?;
    let mut out = vec![];
    for s in obj.props.get("StartingItems").and_then(Value::as_array).ok_or("game mode lacks StartingItems")? {
        let count = s.field("Count").and_then(Value::as_i64).ok_or("starting item lacks Count")?;
        let item = s.field("Item").and_then(Value::as_object).map(|o| pkg.object_path(o)).ok_or("starting item lacks Item")?;
        let item = package_name(&item).to_owned();
        let class = match loader::package(src, &item) { Ok(p) => (0..p.exports.len()).map(|i| p.export_class(i)).find(|c| c.contains("ItemDefinition")).unwrap_or_default(), Err(_) => String::new() };
        out.push(StartingItem { item, count, class });
    }
    let override_quickbars = obj.props.get("bOverrideQuickBars").and_then(Value::as_bool).unwrap_or(false);
    Ok((out, override_quickbars))
}

/// Everything above for the ammo types named by the loadout firearms (`ammo_paths`, duplicates collapsed).
pub fn read(src: &ClipSourceDir, ammo_paths: &[String]) -> Result<AmmoSetup, String> {
    let mut defs: Vec<AmmoDef> = vec![];
    for p in ammo_paths { let long = package_name(p); if !defs.iter().any(|d| d.item == long) { defs.push(ammo_def(src, long)?); } }
    let (starting_items, override_quickbars) = game_mode_starting_items(src)?;
    Ok(AmmoSetup { defs, starting_items, override_quickbars })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn def(item: &str) -> AmmoDef { AmmoDef { item: item.into(), name: "x".into(), max_stack: 999, drop_count: 12, tag: String::new() } }
    #[test] fn starting_ammo_is_derived_from_the_item_classes_of_starting_items() {
        let s = AmmoSetup { defs: vec![def("/Game/A/Medium")], override_quickbars: true, starting_items: vec![
            StartingItem { item: "/Game/B/Wall".into(), count: 1, class: "BuildingItemData".into() },
            StartingItem { item: "/Game/A/Medium".into(), count: 30, class: "FortAmmoItemDefinition".into() },
            StartingItem { item: "/Game/A/Other".into(), count: 9, class: "FortAmmoItemDefinition".into() } ] };
        assert_eq!(s.starting_ammo(), vec![(0, 30)], "only ammo-class entries of used ammo types");
        assert_eq!(s.def_index("/Game/A/Medium.Medium"), Some(0));
    }
}
