//! The items the sandbox can hold, as the console sees them. The front-end builds the list from the REAL 1.8 item definitions it loaded (WID_* firearms, the
//! healing consumables, the pickaxe, the wood / stone / metal resource items): ids and display names are the paks' own, rarities are the `Rarity` property.
//! Nothing here is invented; an item the sandbox has no gameplay for (another weapon, a trap, ...) is simply not in the catalog.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource { Wood, Stone, Metal }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// Firearm `n` of the starter loadout (`fn_pose::loadout::ITEMS` order).
    Gun(usize),
    /// Healing consumable `n` (`fn_pose::loadout_stats::HEALABLE_ITEMS` order: Bandage, Medkit, Shields, PurpleStuff).
    Healable(usize),
    /// The harvesting tool (quickbar slot 1).
    Pickaxe,
    Resource(Resource),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemInfo {
    /// Short asset name (`WID_Assault_Auto_Athena_C_Ore_T02`, `Athena_Bandage`, `WoodItemData`).
    pub id: String,
    /// The definition's `DisplayName` (or, when it could not be read, the asset name; the front-end says so).
    pub name: String,
    /// 0 Handmade (Common) .. 4 Fine (Legendary), the loadout's mapping of `EFortRarity`; None when the definition has no rarity.
    pub rarity: Option<u8>,
    pub kind: ItemKind,
    /// `MaxStackSize` of the definition.
    pub max_stack: Option<u32>,
}

/// Rarity name as the game shows it (`EFortRarity` Handmade / Ordinary / Sturdy / Quality / Fine = Common / Uncommon / Rare / Epic / Legendary).
pub fn rarity_name(r: u8) -> &'static str {
    match r { 0 => "Common", 1 => "Uncommon", 2 => "Rare", 3 => "Epic", 4 => "Legendary", _ => "Unknown rarity" }
}

/// Rarity of an `EFortRarity::X` enum string of a definition (same mapping as `fn_pose::loadout_inventory`).
pub fn rarity_of(v: &str) -> Option<u8> {
    match v.rsplit("::").next().unwrap_or(v) { "Handmade" | "Common" => Some(0), "Ordinary" | "Uncommon" => Some(1), "Sturdy" | "Rare" => Some(2), "Quality" | "Epic" => Some(3), "Fine" | "Legendary" => Some(4), _ => None }
}

/// Lower-case alphanumerics only: `Pump Shotgun`, `pump_shotgun` and `PumpShotgun` all compare equal.
pub fn norm(s: &str) -> String { s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect() }

/// Match quality of `q` (already normalised) against a normalised id / name: 0 exact, 1 prefix, 2 substring.
pub fn rank_text(text: &str, q: &str) -> Option<u8> {
    if q.is_empty() { return Some(2); }
    if text == q { Some(0) } else if text.starts_with(q) { Some(1) } else if text.contains(q) { Some(2) } else { None }
}

/// One Battle Royale weapon the front-end's weapon runtime can load and give (the whole BR catalog, not only the starter loadout). Mirrors `WeaponInfo` of fn-app.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponEntry {
    /// `/Game/Athena/Items/Weapons/WID_...` package path (what the runtime resolves).
    pub wid: String,
    /// Asset name (`WID_Assault_Auto_Athena_R_Ore_T03`).
    pub id: String,
    /// `DisplayName`, `None` until the background read of the definitions finished.
    pub name: Option<String>,
    pub rarity: Option<u8>,
}
impl WeaponEntry {
    pub fn describe(&self) -> String { format!("{} ({})", self.name.as_deref().unwrap_or("name not read yet"), self.rarity.map(rarity_name).unwrap_or("no rarity")) }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub items: Vec<ItemInfo>,
    /// Every BR weapon the weapon runtime can give (empty in preview mode / until the front-end fills it).
    pub weapons: Vec<WeaponEntry>,
}

impl Catalog {
    pub fn new(items: Vec<ItemInfo>) -> Catalog { Catalog { items, weapons: vec![] } }
    /// Weapons a query names, same rule as the runtime: full path / asset name (any case, `WID_` optional) exactly, else the display name exactly (case / spaces / punctuation ignored).
    pub fn weapon_matches(&self, query: &str) -> Vec<&WeaponEntry> {
        let q = query.trim().trim_matches('"');
        let q = q.strip_prefix("/Game/Athena/Items/Weapons/").unwrap_or(q);
        let q = norm(q.split('.').next().unwrap_or(q));
        if q.is_empty() { return vec![]; }
        let by_id: Vec<&WeaponEntry> = self.weapons.iter().filter(|w| { let n = norm(&w.id); n == q || n.strip_prefix("wid") == Some(&q) }).collect();
        if !by_id.is_empty() { return by_id; }
        self.weapons.iter().filter(|w| w.name.as_deref().is_some_and(|n| norm(n) == q)).collect()
    }
    /// The one weapon a query names; an ambiguous display name (several rarities) is an error listing every option.
    pub fn weapon_resolve(&self, query: &str) -> Result<&WeaponEntry, String> {
        let m = self.weapon_matches(query);
        match m.as_slice() {
            [] => Err(format!("no Battle Royale weapon matches '{}' (list weapons shows all {}){}", query.trim(), self.weapons.len(), if self.weapons.iter().any(|w| w.name.is_some()) { "" } else { "; display names are still being read: use the WID asset name" })),
            [w] => Ok(w),
            many => Err(format!("'{}' matches {} weapons, name one: {}", query.trim(), many.len(), many.iter().map(|w| format!("{} [{}]", w.id, w.rarity.map(rarity_name).unwrap_or("no rarity"))).collect::<Vec<_>>().join(", "))),
        }
    }
    /// Weapons for completion / listing: best matches (id or display name) first.
    pub fn weapon_search(&self, query: &str) -> Vec<&WeaponEntry> {
        let q = norm(query);
        let mut v: Vec<(u8, &WeaponEntry)> = self.weapons.iter().filter_map(|w| {
            let a = rank_text(&norm(&w.id), &q); let b = w.name.as_deref().and_then(|n| rank_text(&norm(n), &q));
            match (a, b) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) }.map(|r| (r, w))
        }).collect();
        v.sort_by(|a, b| (a.0, &a.1.id).cmp(&(b.0, &b.1.id)));
        v.into_iter().map(|(_, w)| w).collect()
    }
    pub fn rank(i: &ItemInfo, q: &str) -> Option<u8> {
        match (rank_text(&norm(&i.id), q), rank_text(&norm(&i.name), q)) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) }
    }
    /// Items matching `query` (best matches first), restricted by `filter`.
    pub fn search<'a>(&'a self, query: &str, filter: impl Fn(&ItemInfo) -> bool) -> Vec<&'a ItemInfo> {
        let q = norm(query);
        let mut v: Vec<(u8, &ItemInfo)> = self.items.iter().filter(|i| filter(i)).filter_map(|i| Self::rank(i, &q).map(|r| (r, i))).collect();
        v.sort_by(|a, b| (a.0, &a.1.id).cmp(&(b.0, &b.1.id)));
        v.into_iter().map(|(_, i)| i).collect()
    }
    /// One item: exact match of the id or display name, else the unique best prefix / substring match; several candidates at the best level are an error that lists them.
    pub fn resolve<'a>(&'a self, query: &str, filter: impl Fn(&ItemInfo) -> bool) -> Result<&'a ItemInfo, String> {
        let q = norm(query);
        if q.is_empty() { return Err("missing item (give an id or a display name; Tab completes)".into()); }
        let mut best: Option<u8> = None;
        let mut hits: Vec<&ItemInfo> = vec![];
        for i in self.items.iter().filter(|i| filter(i)) {
            let Some(r) = Self::rank(i, &q) else { continue };
            match best { Some(b) if r > b => {}, Some(b) if r == b => hits.push(i), _ => { best = Some(r); hits.clear(); hits.push(i); } }
        }
        match hits.len() {
            0 => Err(format!("no item matches '{}' (list items shows what this sandbox can hold)", query.trim())),
            1 => Ok(hits[0]),
            n => Err(format!("'{}' matches {n} items: {}", query.trim(), hits.iter().take(8).map(|i| format!("{} ({})", i.id, i.name)).collect::<Vec<_>>().join(", "))),
        }
    }
    pub fn guns(&self) -> impl Iterator<Item = &ItemInfo> { self.items.iter().filter(|i| matches!(i.kind, ItemKind::Gun(_))) }
    pub fn describe(i: &ItemInfo) -> String {
        format!("{} ({}{})", i.name, i.rarity.map(rarity_name).unwrap_or("no rarity"), i.max_stack.map(|m| format!(", stack {m}")).unwrap_or_default())
    }
}

#[cfg(test)]
pub(crate) fn test_weapons() -> Catalog {
    let w = |id: &str, name: &str, r: u8| WeaponEntry { wid: format!("/Game/Athena/Items/Weapons/{id}"), id: id.into(), name: Some(name.into()), rarity: Some(r) };
    let mut c = test_catalog();
    c.weapons = vec![
        w("WID_Assault_Auto_Athena_C_Ore_T02", "Assault Rifle", 0), w("WID_Assault_Auto_Athena_UC_Ore_T03", "Assault Rifle", 1), w("WID_Assault_Auto_Athena_R_Ore_T03", "Assault Rifle", 2),
        w("WID_Launcher_Rocket_Athena_R_Ore_T03", "Rocket Launcher", 2), w("WID_Pistol_SemiAuto_Athena_C_Ore_T02", "Pistol", 0),
    ];
    c
}

#[cfg(test)]
pub(crate) fn test_catalog() -> Catalog {
    let it = |id: &str, name: &str, r: Option<u8>, kind, max| ItemInfo { id: id.into(), name: name.into(), rarity: r, kind, max_stack: max };
    Catalog::new(vec![
        it("WID_Assault_Auto_Athena_C_Ore_T02", "Assault Rifle", Some(0), ItemKind::Gun(0), None),
        it("WID_Shotgun_Standard_Athena_C_Ore_T03", "Pump Shotgun", Some(0), ItemKind::Gun(1), None),
        it("WID_Sniper_BoltAction_Scope_Athena_R_Ore_T03", "Bolt-Action Sniper Rifle", Some(2), ItemKind::Gun(2), None),
        it("Athena_Bandage", "Bandage", Some(0), ItemKind::Healable(0), Some(15)),
        it("Athena_Medkit", "Med Kit", Some(1), ItemKind::Healable(1), Some(3)),
        it("Athena_Shields", "Shield Potion", Some(2), ItemKind::Healable(2), Some(2)),
        it("WID_Harvest_Pickaxe_Athena_C_T01", "Pickaxe", Some(0), ItemKind::Pickaxe, None),
        it("WoodItemData", "Wood", Some(0), ItemKind::Resource(Resource::Wood), Some(999)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn names_compare_without_case_spaces_and_punctuation() {
        let c = test_catalog();
        assert_eq!(c.resolve("pump shotgun", |_| true).unwrap().id, "WID_Shotgun_Standard_Athena_C_Ore_T03");
        assert_eq!(c.resolve("PumpShotgun", |_| true).unwrap().kind, ItemKind::Gun(1));
        assert_eq!(c.resolve("bolt-action", |_| true).unwrap().kind, ItemKind::Gun(2));
        assert_eq!(c.resolve("athena_bandage", |_| true).unwrap().kind, ItemKind::Healable(0));
    }
    #[test] fn exact_beats_prefix_and_ambiguity_is_listed() {
        let c = test_catalog();
        assert_eq!(c.resolve("wood", |_| true).unwrap().name, "Wood");
        let e = c.resolve("wid", |_| true).unwrap_err(); assert!(e.contains("matches 4 items") && e.contains("Pump Shotgun"), "{e}");
        assert!(c.resolve("nothing", |_| true).unwrap_err().contains("no item matches"));
        assert!(c.resolve("  ", |_| true).unwrap_err().contains("missing item"));
    }
    #[test] fn weapons_resolve_by_path_id_and_display_name_and_list_ambiguity() {
        let c = test_weapons();
        assert_eq!(c.weapon_resolve("rocket launcher").unwrap().id, "WID_Launcher_Rocket_Athena_R_Ore_T03");
        assert_eq!(c.weapon_resolve("/Game/Athena/Items/Weapons/WID_Pistol_SemiAuto_Athena_C_Ore_T02.WID_Pistol_SemiAuto_Athena_C_Ore_T02").unwrap().rarity, Some(0));
        assert_eq!(c.weapon_resolve("assault_auto_athena_uc_ore_t03").unwrap().rarity, Some(1));
        let e = c.weapon_resolve("Assault Rifle").unwrap_err(); assert!(e.contains("matches 3 weapons") && e.contains("_UC_") && e.contains("[Rare]"), "{e}");
        assert!(c.weapon_resolve("bazooka").unwrap_err().contains("no Battle Royale weapon"));
        assert_eq!(c.weapon_search("rock")[0].id, "WID_Launcher_Rocket_Athena_R_Ore_T03");
    }
    #[test] fn rarity_strings_map_to_the_game_names() {
        assert_eq!(rarity_of("EFortRarity::Quality"), Some(3)); assert_eq!(rarity_name(rarity_of("Fine").unwrap()), "Legendary"); assert_eq!(rarity_of("Elegant"), None);
    }
    #[test] fn search_orders_by_match_quality() {
        let c = test_catalog();
        let v = c.search("sh", |_| true); assert_eq!(v[0].name, "Shield Potion", "prefix of a name beats an id substring"); assert!(v.iter().any(|i| i.name == "Pump Shotgun"));
    }
}
