//! COSMETIC SWAP: run-time selection of the Battle Royale (Athena) skin, harvesting tool, back bling and glider.
//!
//! * Enumeration (`list_*`): every item definition under `/Game/Athena/Items/Cosmetics/**` of the 1.8 paks whose main export class is
//!   `AthenaCharacterItemDefinition` / `AthenaPickaxeItemDefinition` / `AthenaBackpackItemDefinition` / `AthenaGliderItemDefinition`, read once
//!   (cached for the process) with the same property decoding as `fn_pose::cosmetics::Catalog` (INFERENCE: re-implemented here because fn-pose
//!   optionally depends on fn-game, so fn-game cannot depend on fn-pose). Development assets (`00_TestAssets`) are excluded, Save the World
//!   (`/Game/Items`, `/Game/Heroes`) cannot appear: the root is the Athena cosmetics folder and a skin's `HeroDefinition` must be under `/Game/Athena/Heroes/`.
//! * `set_*`: resolves a query (id or display name, case-insensitive: exact, then prefix, then substring; several candidates = `Err` listing them) and
//!   records the item in [`GameSim::cosmetics`] ([`Selection`]). Position, animation state, loadout and swing state are not touched. The sim has
//!   no GPU / mesh objects, so the actual rebuild is done by the front-end from [`Selection::take_pending`]: for a skin it must rebuild ALL parts from
//!   `Selection::skin.hero` (`fn_pose::br_character::BrCharacter::load(src, hero)` builds the part list from scratch: head / body / hat / face accessories / materials,
//!   so no part of the previous skin survives); for a pickaxe `fn_pose::loadout_melee::load_for(src, pickaxe.package)` resolves the mesh / anims / swing data.
use crate::GameSim;
use fn_pak::PakSet;
use fn_uasset::PackageData;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

pub const COSMETICS_ROOT: &str = "fortnitegame/content/athena/items/cosmetics";
pub const LOCRES_PATH: &str = "FortniteGame/Content/Localization/Game/en/Game.locres";

/// One selectable cosmetic (what the console lists).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CosmeticEntry { pub id: String, pub display_name: String, pub rarity: String }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind { Skin, Pickaxe, Backbling, Glider }
impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Skin, Kind::Pickaxe, Kind::Backbling, Kind::Glider];
    fn class(self) -> &'static str {
        match self { Kind::Skin => "AthenaCharacterItemDefinition", Kind::Pickaxe => "AthenaPickaxeItemDefinition", Kind::Backbling => "AthenaBackpackItemDefinition", Kind::Glider => "AthenaGliderItemDefinition" }
    }
    pub fn name(self) -> &'static str { match self { Kind::Skin => "skin", Kind::Pickaxe => "pickaxe", Kind::Backbling => "back bling", Kind::Glider => "glider" } }
}

/// A resolved BR cosmetic item definition: the data the front-end needs to rebuild.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub display_name: String,
    pub rarity: String,
    /// `/Game/Athena/Items/Cosmetics/...` package of the item definition
    pub package: String,
    /// skins: `HeroDefinition` object path (`/Game/Athena/Heroes/HID_..`) the pawn's parts are assembled from
    pub hero: Option<String>,
    /// pickaxes: `WeaponDefinition` object path (`WID_Harvest_*`)
    pub weapon: Option<String>,
    /// gliders / back bling: every object path property of the definition that names a mesh / class (`SkeletalMesh`, `AnimClass`, `MaterialOverrides` ...), as `(property, path)`
    pub assets: Vec<(String, String)>,
}
impl Item {
    fn entry(&self) -> CosmeticEntry { CosmeticEntry { id: self.id.clone(), display_name: self.display_name.clone(), rarity: self.rarity.clone() } }
}

/// The cosmetics of the running game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Selection {
    pub skin: Option<Item>,
    pub pickaxe: Option<Item>,
    pub backbling: Option<Item>,
    pub glider: Option<Item>,
    /// bumped by every applied swap
    pub generation: u32,
    pending: Vec<Kind>,
}
impl Selection {
    pub fn get(&self, k: Kind) -> Option<&Item> { match k { Kind::Skin => self.skin.as_ref(), Kind::Pickaxe => self.pickaxe.as_ref(), Kind::Backbling => self.backbling.as_ref(), Kind::Glider => self.glider.as_ref() } }
    fn set(&mut self, k: Kind, i: Item) {
        match k { Kind::Skin => self.skin = Some(i), Kind::Pickaxe => self.pickaxe = Some(i), Kind::Backbling => self.backbling = Some(i), Kind::Glider => self.glider = Some(i) }
        self.pending.retain(|p| *p != k);
        self.pending.push(k);
        self.generation += 1;
    }
    /// Kinds changed since the last call (the front-end rebuilds those); each kind appears once, most recent swap last.
    pub fn take_pending(&mut self) -> Vec<Kind> { std::mem::take(&mut self.pending) }
    pub fn has_pending(&self) -> bool { !self.pending.is_empty() }
}

// ---- catalog (read once) ----

struct Catalog { set: Arc<PakSet>, items: HashMap<Kind, Vec<Item>>, load_ms: u128 }

/// The 1.8 `FortniteGame` directory: `FN_ROOT`, else `FN_TEST_ROOT`, else the dev install (same default as `fn_ui::pak_assets::default_install`).
fn install_dir() -> PathBuf {
    if let Some(p) = fn_live::var_os("FN_ROOT").or_else(|| fn_live::var_os("FN_TEST_ROOT")) { return PathBuf::from(p); }
    PathBuf::from(r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame")
}

fn catalog() -> Result<&'static Catalog, String> {
    static C: OnceLock<Result<Catalog, String>> = OnceLock::new();
    C.get_or_init(|| {
        let t = Instant::now();
        let dir = install_dir();
        let set = PakSet::open_install(&dir).map_err(|e| format!("cannot open the 1.8 paks in {}: {e}", dir.display()))?;
        let items = read_all(&set);
        Ok(Catalog { set: Arc::new(set), items, load_ms: t.elapsed().as_millis() })
    }).as_ref().map_err(|e| e.clone())
}

/// The mounted pak set the catalog was read from (for the front-end rebuilds).
pub fn paks() -> Result<Arc<PakSet>, String> { catalog().map(|c| c.set.clone()) }

fn read_all(set: &PakSet) -> HashMap<Kind, Vec<Item>> {
    let locres = Locres::open(set);
    let mut paths: Vec<String> = set.glob(&format!("{COSMETICS_ROOT}/**/*.uasset")).iter()
        .filter_map(|(_, e)| fn_pak::pak_path_to_game(e.rel_path()).or_else(|| fn_pak::pak_path_to_game(&e.path))).map(|g| g.trim_end_matches(".uasset").to_owned()).collect();
    paths.sort();
    paths.dedup();
    let mut out: HashMap<Kind, Vec<Item>> = HashMap::new();
    for long in paths {
        if long.to_ascii_lowercase().contains("/00_testassets/") { continue; }
        if let Some((kind, item)) = read_item(set, &long, locres.as_ref()) { out.entry(kind).or_default().push(item); }
    }
    for v in out.values_mut() { v.sort_by(|a, b| a.id.cmp(&b.id)); }
    out
}

fn read_item(set: &PakSet, long: &str, locres: Option<&Locres>) -> Option<(Kind, Item)> {
    let data = PackageData::load(set, long).ok()?;
    let pkg = data.package().ok()?;
    let file = long.rsplit('/').next().unwrap_or(long);
    let idx = (0..pkg.exports.len()).find(|&i| pkg.exports[i].name == file).or(if pkg.exports.is_empty() { None } else { Some(0) })?; // same lookup as fn_pose::cosmetics (BoltonPickaxe.uasset names its export "BoltOnPickaxe")
    let class = pkg.export_class_name(idx);
    let kind = Kind::ALL.into_iter().find(|k| k.class() == class)?;
    let p = pkg.read_export(idx).ok()?.to_json(&pkg).get("properties").cloned().unwrap_or(Value::Null);
    let hero = path_of(p.get("HeroDefinition"));
    // Battle Royale only: a skin's hero is an Athena hero (never /Game/Heroes or /Game/Items)
    if kind == Kind::Skin && !hero.as_deref().is_some_and(|h| h.starts_with("/Game/Athena/Heroes/")) { return None; }
    let weapon = path_of(p.get("WeaponDefinition"));
    if kind == Kind::Pickaxe && !weapon.as_deref().is_some_and(|w| w.starts_with("/Game/Athena/")) { return None; }
    let mut assets = vec![];
    if let Some(o) = p.as_object() {
        for (k, v) in o { if let Some(s) = path_of(Some(v)) { if matches!(kind, Kind::Glider | Kind::Backbling) { assets.push((k.clone(), s)); } } }
    }
    assets.sort();
    let tail = |v: Option<&Value>| v.and_then(|x| x.as_str()).map(|s| s.rsplit("::").next().unwrap_or(s).to_owned());
    Some((kind, Item {
        id: file.to_owned(),
        display_name: text_of(p.get("DisplayName"), locres).map(|s| s.trim().to_owned()).unwrap_or_else(|| file.to_owned()),
        rarity: tail(p.get("Rarity")).unwrap_or_else(|| "Handmade".to_owned()),
        package: long.to_owned(), hero, weapon, assets,
    }))
}

fn path_of(v: Option<&Value>) -> Option<String> { v.and_then(|x| x.as_str()).filter(|s| s.starts_with('/') && !s.is_empty()).map(str::to_owned) }

fn text_of(v: Option<&Value>, locres: Option<&Locres>) -> Option<String> {
    let o = v?.as_object()?;
    let key = o.get("key").and_then(|k| k.as_str()).unwrap_or("");
    let ns = o.get("namespace").and_then(|k| k.as_str()).unwrap_or("");
    if key.is_empty() { return o.get("text").and_then(|t| t.as_str()).map(str::to_owned); }
    locres.and_then(|l| l.map.get(&(ns.to_owned(), key.to_owned()))).map(|s| s.to_owned())
}

/// `Game.locres` (UE 4.16 legacy v0), same format as `fn_pose::cosmetics::Locres`.
struct Locres { map: HashMap<(String, String), String> }
impl Locres {
    fn open(set: &PakSet) -> Option<Locres> { Locres::parse(&set.read(LOCRES_PATH).ok()?) }
    fn parse(b: &[u8]) -> Option<Locres> {
        let mut o = 0usize;
        fn u32_(b: &[u8], o: &mut usize) -> Option<u32> { let v = u32::from_le_bytes(b.get(*o..*o + 4)?.try_into().ok()?); *o += 4; Some(v) }
        fn fstr(b: &[u8], o: &mut usize) -> Option<String> {
            let n = i32::from_le_bytes(b.get(*o..*o + 4)?.try_into().ok()?); *o += 4;
            if n == 0 { return Some(String::new()); }
            if n < 0 {
                let len = (-n) as usize;
                let raw = b.get(*o..*o + len * 2)?; *o += len * 2;
                let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                Some(String::from_utf16_lossy(&u[..u.len().saturating_sub(1)]))
            } else {
                let len = n as usize;
                let raw = b.get(*o..*o + len)?; *o += len;
                Some(String::from_utf8_lossy(&raw[..raw.len().saturating_sub(1)]).into_owned())
            }
        }
        let nns = u32_(b, &mut o)?;
        let mut map = HashMap::new();
        for _ in 0..nns {
            let ns = fstr(b, &mut o)?;
            let nk = u32_(b, &mut o)?;
            for _ in 0..nk {
                let key = fstr(b, &mut o)?;
                let _hash = u32_(b, &mut o)?;
                let val = fstr(b, &mut o)?;
                map.insert((ns.clone(), key), val);
            }
        }
        Some(Locres { map })
    }
}

// ---- API ----

/// Every shipped BR item of `kind` (empty when the paks cannot be opened).
pub fn items(kind: Kind) -> Vec<Item> { catalog().ok().and_then(|c| c.items.get(&kind).cloned()).unwrap_or_default() }
fn list(kind: Kind) -> Vec<CosmeticEntry> { items(kind).iter().map(Item::entry).collect() }
pub fn list_skins() -> Vec<CosmeticEntry> { list(Kind::Skin) }
pub fn list_pickaxes() -> Vec<CosmeticEntry> { list(Kind::Pickaxe) }
pub fn list_backblings() -> Vec<CosmeticEntry> { list(Kind::Backbling) }
pub fn list_gliders() -> Vec<CosmeticEntry> { list(Kind::Glider) }

/// Query rules: case-insensitive match of the id or the display name: exact (one item), else prefix, else substring. Several candidates at the best level
/// (or none) -> `Err` (listing the candidates).
pub fn resolve<'a>(items: &'a [Item], query: &str) -> Result<&'a Item, String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() { return Err("empty query".into()); }
    let levels: [&dyn Fn(&str) -> bool; 3] = [&|s| s == q, &|s| s.starts_with(&q), &|s| s.contains(&q)];
    for level in levels {
        let hits: Vec<&Item> = items.iter().filter(|i| level(&i.id.to_lowercase()) || level(&i.display_name.to_lowercase())).collect();
        match hits.len() {
            0 => continue,
            1 => return Ok(hits[0]),
            n => {
                let mut names: Vec<String> = hits.iter().take(12).map(|i| format!("{} ({})", i.id, i.display_name)).collect();
                if n > 12 { names.push(format!("... {} more", n - 12)); }
                return Err(format!("'{query}' is ambiguous, {n} matches: {}", names.join(", ")));
            }
        }
    }
    Err(format!("no cosmetic matches '{query}'"))
}

/// Applies `query` of `kind` to the sim's selection. Ok string = what was applied (+ catalog load time on first use).
pub fn set(sim: &mut GameSim, kind: Kind, query: &str) -> Result<String, String> {
    let t = Instant::now();
    let first = catalog().map(|c| c.load_ms).map_err(|e| e.clone())?;
    let cat = catalog()?;
    let list = cat.items.get(&kind).map(|v| v.as_slice()).unwrap_or(&[]);
    if list.is_empty() { return Err(format!("the 1.8 paks ship no {} item definitions ({}: none under /Game/Athena/Items/Cosmetics)", kind.name(), kind.class())); }
    let item = resolve(list, query)?.clone();
    let msg = format!("{} -> {} ({}, {})", kind.name(), item.display_name, item.id, item.rarity);
    sim.cosmetics.set(kind, item);
    Ok(format!("{msg} [selected in {:.2} ms; catalog read once in {first} ms; the front-end loads the parts on demand]", t.elapsed().as_secs_f64() * 1000.))
}
pub fn set_skin(sim: &mut GameSim, query: &str) -> Result<String, String> { set(sim, Kind::Skin, query) }
pub fn set_pickaxe(sim: &mut GameSim, query: &str) -> Result<String, String> { set(sim, Kind::Pickaxe, query) }
pub fn set_backbling(sim: &mut GameSim, query: &str) -> Result<String, String> { set(sim, Kind::Backbling, query) }
pub fn set_glider(sim: &mut GameSim, query: &str) -> Result<String, String> { set(sim, Kind::Glider, query) }

#[cfg(test)]
mod tests {
    use super::*;
    fn have_paks() -> bool { catalog().is_ok() }
    fn item(id: &str, name: &str) -> Item { Item { id: id.into(), display_name: name.into(), ..Default::default() } }

    #[test]
    fn query_rules() {
        let v = vec![item("CID_001_Athena_Commando_F_Default", "Default"), item("CID_029_Athena_Commando_F_Halloween", "Ghoul Trooper"), item("CID_030_Athena_Commando_M_Halloween", "Skull Trooper"), item("CID_005", "Default")];
        assert_eq!(resolve(&v, "ghoul trooper").unwrap().id, "CID_029_Athena_Commando_F_Halloween"); // exact display name, case-insensitive
        assert_eq!(resolve(&v, "cid_030_athena_commando_m_halloween").unwrap().id, "CID_030_Athena_Commando_M_Halloween"); // exact id
        assert_eq!(resolve(&v, "skull").unwrap().display_name, "Skull Trooper"); // prefix
        assert_eq!(resolve(&v, "trooper").is_err(), true); // substring, two candidates
        let e = resolve(&v, "trooper").unwrap_err();
        assert!(e.contains("Ghoul Trooper") && e.contains("Skull Trooper") && e.contains("ambiguous"), "{e}");
        assert_eq!(resolve(&v, "oul tro").unwrap().id, "CID_029_Athena_Commando_F_Halloween"); // substring, one
        assert!(resolve(&v, "default").unwrap_err().contains("ambiguous")); // exact on two display names
        assert!(resolve(&v, "zzz").unwrap_err().contains("no cosmetic"));
        assert_eq!(resolve(&v, "CID_001").unwrap().display_name, "Default"); // id prefix
    }

    #[test]
    fn lists_are_br_only_and_non_empty() {
        if !have_paks() { eprintln!("1.8 install not found: skipped"); return }
        // 1.8 ships NO AthenaBackpackItemDefinition (probed: no such class and no *ack* file under Cosmetics): back blings are legitimately empty
        for (n, l) in [("skins", list_skins()), ("pickaxes", list_pickaxes()), ("gliders", list_gliders())] {
            eprintln!("{n}: {}", l.len());
            assert!(!l.is_empty(), "{n} empty");
        }
        assert!(list_backblings().is_empty());
        for k in Kind::ALL {
            for i in items(k) {
                assert!(i.package.starts_with("/Game/Athena/Items/Cosmetics/"), "{}", i.package);
                assert!(!i.package.to_lowercase().contains("testassets"));
                for p in i.hero.iter().chain(i.weapon.iter()).chain(i.assets.iter().map(|a| &a.1)) { assert!(!p.starts_with("/Game/Items/") && !p.starts_with("/Game/Heroes/"), "STW path {p} in {}", i.id); }
            }
        }
        let skins = list_skins();
        assert!(skins.iter().any(|s| s.display_name == "Skull Trooper"));
        assert!(skins.iter().all(|s| s.id.starts_with("CID_")), "{:?}", skins.iter().map(|s| &s.id).collect::<Vec<_>>());
        assert!(list_pickaxes().iter().any(|s| s.display_name == "Reaper"));
        assert_eq!((skins.len(), list_pickaxes().len(), list_gliders().len()), (30, 10, 23));
        assert!(list_gliders().iter().any(|s| s.id == "Glider_Voyager"));
    }

    #[test]
    fn headless_swap_changes_the_selection_and_keeps_the_sim() {
        if !have_paks() { return }
        let mut sim = GameSim::new();
        for _ in 0..30 { sim.step(&crate::FrameInput { forward: 1., ..Default::default() }, 1. / 60.); }
        let (x, y, z, yaw, frames, state) = (sim.x, sim.y, sim.z, sim.yaw, sim.frames, sim.anim_time);
        let r = set_skin(&mut sim, "skull trooper").unwrap();
        eprintln!("{r}");
        assert_eq!(sim.cosmetics.skin.as_ref().unwrap().id, "CID_030_Athena_Commando_M_Halloween");
        assert!(sim.cosmetics.skin.as_ref().unwrap().hero.as_deref().unwrap().contains("HID_030_Athena_Commando_M_Halloween"));
        set_skin(&mut sim, "ghoul trooper").unwrap();
        assert_eq!(sim.cosmetics.skin.as_ref().unwrap().id, "CID_029_Athena_Commando_F_Halloween");
        eprintln!("{}", set_pickaxe(&mut sim, "Reaper").unwrap());
        assert!(sim.cosmetics.pickaxe.as_ref().unwrap().weapon.as_deref().unwrap().contains("WID_Harvest_HalloweenScythe"));
        eprintln!("{}", set_glider(&mut sim, "Glider_Voyager").unwrap());
        let e = set_backbling(&mut sim, "any").unwrap_err();
        eprintln!("{e}");
        assert!(e.contains("ship no back bling"));
        assert_eq!(sim.cosmetics.generation, 4);
        assert_eq!(sim.cosmetics.take_pending(), vec![Kind::Skin, Kind::Pickaxe, Kind::Glider]);
        assert!(!sim.cosmetics.has_pending());
        assert_eq!((sim.x, sim.y, sim.z, sim.yaw, sim.frames, sim.anim_time), (x, y, z, yaw, frames, state)); // position / animation untouched
        assert!(set_skin(&mut sim, "").is_err());
        assert!(set_skin(&mut sim, "no such skin zzz").is_err());
        assert_eq!(sim.cosmetics.generation, 4);
    }

    #[test]
    #[ignore]
    fn dump_backbling_and_glider_properties() {
        if !have_paks() { return }
        for k in [Kind::Backbling, Kind::Glider] { for i in items(k).iter().take(3) { eprintln!("{:?} {} {:?}", k, i.id, i.assets); } }
    }
}
