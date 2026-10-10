//! Generic per-weapon animation set of ANY Battle Royale firearm, resolved from the 1.8 data only (no per-weapon tables):
//!
//!   WID (`WeaponActorClass`) -> weapon actor Blueprint chain (`BlueprintGeneratedClass` super chain, CDO of each link)
//!     * `WeaponCoreAnimation` (`AFortWeapon` +0xA80): first CDO in the chain that serializes it, else the constructor default of the
//!       native class the chain ends in (see [`native_core_default`]).
//!     * montages: `EquipAnimation`, `ReloadAnimation`, `WeaponEquipMontage`, `WeaponReloadMontage` (`AFortWeapon` +0xA60..0xA78) and
//!       `FireAnimation`, `FireDownsightsAnimation`, `WeaponFireMontage`, `WeaponFireDownsightsMontage` (`AFortWeaponRanged` +0xC78..0xC90),
//!       each taken from the first CDO of the chain that serializes it (UE property inheritance).
//!   -> `CoreAnims_Male` / `CoreAnims_Female` of `Fortnite_M_Avg_Player_AnimBlueprint` (its CDO; the female pawn uses the Female map,
//!      SDK `FN_Fortnite_M_Avg_Player_AnimBlueprint_classes.hpp`: CoreAnims_Male +0x139B0, CoreAnims_Female +0x13A00) -> the
//!      `PlayerAnimAssets_Struct` entry of that enum key (idle / aim offset / jog additive / sprint / crouch assets).
//!
//! Charge / burst: 1.8 has no animation property for them on the weapon classes (SDK `AFortWeapon` / `AFortWeaponRanged` carry only the eight
//! montage fields above), so burst / charge weapons play the same montages through their fire ability; nothing is invented for them.
use crate::{loader, ClipSourceDir};
use fn_ueio::{props::{read_props, Ctx, PropList, PropValue}, Package, Reader};

pub const PLAYER_ANIM_BP: &str = "/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint";
/// The eight montage properties of `AFortWeapon` / `AFortWeaponRanged` (SDK offsets in the module doc), in a fixed role order.
pub const MONTAGE_PROPS: [&str; 8] = ["EquipAnimation", "FireAnimation", "FireDownsightsAnimation", "ReloadAnimation", "WeaponEquipMontage", "WeaponFireMontage", "WeaponFireDownsightsMontage", "WeaponReloadMontage"];

fn package_name(reference: &str) -> &str { reference.split('.').next().unwrap_or(reference) }

/// The `WeaponCoreAnimation` value the exe's native constructors leave when no Blueprint CDO serializes the property.
/// `AFortWeapon::AFortWeapon` (RVA 0x140914b60) writes `*(u8*)(this+0xA80) = 0` (Melee); `AFortWeaponRanged::AFortWeaponRanged`
/// (RVA 0x140915090) calls it then writes `*(u8*)(this+0xA80) = 1` (Pistol). Enum values: SDK `EFortWeaponCoreAnimation` (0 Melee, 1 Pistol).
/// Returns (enum name, ctor RVA). Any other native parent is an error (it would need its own constructor traced).
pub fn native_core_default(class: &str) -> Result<(&'static str, u64), String> {
    match class {
        "/Script/FortniteGame.FortWeapon" => Ok(("Melee", 0x140914b60)),
        "/Script/FortniteGame.FortWeaponRanged" => Ok(("Pistol", 0x140915090)),
        // BUILD ANIM HOOK: `AFortWeap_BuildingTool::AFortWeap_BuildingTool` (RVA 0x140914770) and `AFortWeap_EditingTool::AFortWeap_EditingTool` (RVA 0x140914890)
        // run the BuildingToolBase ctor (-> AFortWeapon ctor, 0 = Melee) then write `*(u8*)(this + 0x150*8 = 0xA80) = 3` = `PaperBlueprint`.
        "/Script/FortniteGame.FortWeap_BuildingTool" => Ok(("PaperBlueprint", 0x140914770)),
        "/Script/FortniteGame.FortWeap_EditingTool" => Ok(("PaperBlueprint", 0x140914890)),
        other => Err(format!("native weapon class {other} has no traced constructor default for WeaponCoreAnimation")),
    }
}

pub struct ChainLink { pub path: String, pub pkg: Package, pub cdo: PropList }
/// Blueprint super chain of a weapon actor class (most derived first) and the native class it ends in.
pub struct ActorChain { pub links: Vec<ChainLink>, pub native: String }

fn cdo_of(pkg: &Package) -> Result<PropList, String> {
    let idx = pkg.exports.iter().position(|e| e.object_name.starts_with("Default__")).ok_or("missing original class default object")?;
    let (start, end) = pkg.export_range(idx);
    read_props(&mut Reader::at(&pkg.data[..end], start), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())
}

pub fn actor_chain(src: &ClipSourceDir, actor: &str) -> Result<ActorChain, String> {
    let mut path = package_name(actor).to_owned();
    let mut links: Vec<ChainLink> = vec![];
    for _ in 0..32 {
        if links.iter().any(|l| l.path == path) { return Err("weapon actor inheritance cycle".into()); }
        let pkg = loader::package(src, &path)?;
        let cdo = cdo_of(&pkg).map_err(|e| format!("{path}: {e}"))?;
        let class = pkg.exports.iter().enumerate().find(|(i, _)| pkg.export_class(*i) == "BlueprintGeneratedClass").map(|(_, e)| e).ok_or_else(|| format!("{path}: missing generated class"))?;
        if class.super_index >= 0 { return Err(format!("{path}: internal super class")); }
        let parent = pkg.import_path(class.super_index);
        links.push(ChainLink { path: path.clone(), pkg, cdo });
        if parent.starts_with("/Game/") { path = package_name(&parent).to_owned(); } else { return Ok(ActorChain { links, native: parent }); }
    }
    Err("weapon actor inheritance depth limit".into())
}

impl ActorChain {
    /// (value, defining package) of the first CDO in the chain serializing `key` as an object reference; `Ok(None)` for an explicit null
    /// or when no CDO serializes it (native default of every montage pointer is null, UObject* ctor zero-init).
    pub fn object(&self, key: &str) -> Result<Option<(String, String)>, String> {
        for l in &self.links {
            let Some(v) = l.cdo.get(key) else { continue };
            let PropValue::Object(i) = v else { return Err(format!("{}: {key} is not an object reference", l.path)) };
            if *i == 0 { return Ok(None); }
            if *i > 0 { return Err(format!("{}: {key} is an internal reference", l.path)); }
            return Ok(Some((l.pkg.import_path(*i), l.path.clone())));
        }
        Ok(None)
    }
    /// `WeaponCoreAnimation` enum name and its provenance (defining Blueprint package, or the native ctor RVA).
    pub fn core(&self) -> Result<(String, String), String> {
        for l in &self.links {
            if let Some(c) = l.cdo.text("WeaponCoreAnimation") { return Ok((c.rsplit("::").next().unwrap_or(c).to_owned(), format!("CDO {}", l.path))); }
        }
        let (name, rva) = native_core_default(&self.native)?;
        Ok((name.to_owned(), format!("{} constructor RVA {rva:#x}", self.native)))
    }
}

/// `CoreAnims_{Male,Female}` map payload of the player AnimBlueprint CDO: removed count, entry count, then enum FName + tagged struct.
pub fn core_map(pkg: &Package, key: &str) -> Result<Vec<(String, PropList)>, String> {
    let idx = pkg.exports.iter().position(|e| e.object_name.starts_with("Default__")).ok_or("missing AnimBP default object")?;
    let (start, end) = pkg.export_range(idx);
    let p = read_props(&mut Reader::at(&pkg.data[..end], start), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())?;
    let Some(PropValue::Raw(bytes)) = p.get(key) else { return Err(format!("missing original {key}")) };
    let mut r = Reader::new(bytes);
    if r.i32().map_err(|e| e.to_string())? != 0 { return Err("unsupported core map removals".into()); }
    let count = r.i32().map_err(|e| e.to_string())?;
    if !(0..=256).contains(&count) { return Err("invalid core map count".into()); }
    let mut out = vec![];
    for _ in 0..count {
        let name = r.i32().map_err(|e| e.to_string())?;
        if r.i32().map_err(|e| e.to_string())? != 0 { return Err("unexpected numbered core enum".into()); }
        let k = pkg.names.get(usize::try_from(name).map_err(|_| "negative enum name index")?).ok_or("core enum name out of bounds")?.rsplit("::").next().unwrap_or("").to_owned();
        let v = read_props(&mut r, &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())?;
        out.push((k, v));
    }
    if r.remaining() != 0 { return Err("trailing core map bytes".into()); }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CoreAsset {
    /// Field name without the Blueprint struct suffix (`TargetingPose`), and the serialized name (`TargetingPose_4_<GUID>`, the key of `Resources::core_asset_paths`).
    pub field: String, pub raw: String, pub index: i32, pub path: Option<String>,
}
/// `Name_<n>_<32 hex GUID>` -> `Name` (UE user-defined struct member naming).
fn field_base(n: &str) -> &str {
    let b = n.as_bytes();
    if b.len() > 34 && b[b.len() - 33] == b'_' && b[b.len() - 32..].iter().all(|c| c.is_ascii_hexdigit()) {
        let head = &n[..n.len() - 33];
        if let Some(p) = head.rfind('_') { if head[p + 1..].bytes().all(|c| c.is_ascii_digit()) { return &head[..p]; } }
    }
    n
}
/// Everything the data says about one weapon's animations.
#[derive(Clone, Debug)]
pub struct WeaponAnims {
    pub wid: String,
    pub actor: String,
    pub female: bool,
    /// `EFortWeaponCoreAnimation` key and where it came from (CDO package or native constructor RVA).
    pub core: String,
    pub core_source: String,
    /// The `PlayerAnimAssets_Struct` object fields of `CoreAnims_{Male|Female}[core]` in serialized order (explicit nulls kept as `None`).
    pub core_assets: Vec<CoreAsset>,
    /// `MONTAGE_PROPS` role -> (montage object path, Blueprint package whose CDO serializes it); `None` = the chain serializes no montage.
    pub montages: Vec<(&'static str, Option<(String, String)>)>,
}
impl WeaponAnims {
    pub fn montage(&self, prop: &str) -> Option<&str> { self.montages.iter().find(|(k, _)| *k == prop).and_then(|(_, v)| v.as_ref()).map(|(p, _)| p.as_str()) }
    pub fn core_asset(&self, field: &str) -> Option<&str> { self.core_assets.iter().find(|a| a.field == field && a.index == 0).and_then(|a| a.path.as_deref()) }
    /// Montage / core roles that are absent in the data (reported, never substituted).
    pub fn missing(&self) -> Vec<String> {
        let mut v: Vec<String> = self.montages.iter().filter(|(_, m)| m.is_none()).map(|(k, _)| format!("montage {k}")).collect();
        v.extend(self.core_assets.iter().filter(|a| a.path.is_none()).map(|a| format!("core {}", a.field)));
        v
    }
    /// Every external asset package this weapon's animation set refers to.
    pub fn packages(&self) -> Vec<String> {
        let mut v: Vec<String> = self.core_assets.iter().filter_map(|a| a.path.as_deref()).chain(self.montages.iter().filter_map(|(_, m)| m.as_ref().map(|(p, _)| p.as_str()))).map(|p| package_name(p).to_owned()).collect();
        v.sort(); v.dedup(); v
    }
}

/// Resolve the animation set of a WID. `female` picks `CoreAnims_Female`, else `CoreAnims_Male`.
pub fn weapon_anims(src: &ClipSourceDir, wid: &str, female: bool) -> Result<WeaponAnims, String> {
    let wid_pkg = loader::package(src, package_name(wid))?;
    let class = (0..wid_pkg.exports.len()).map(|i| wid_pkg.export_class(i)).find(|c| c.contains("ItemDefinition")).ok_or("WID has no item definition export")?;
    let idx = wid_pkg.find_export(&class).ok_or("WID export")?;
    let (s, e) = wid_pkg.export_range(idx);
    let wid_props = read_props(&mut Reader::at(&wid_pkg.data[..e], s), &Ctx { names: &wid_pkg.names, file_version: wid_pkg.summary.file_version }).map_err(|e| e.to_string())?;
    let actor = match wid_props.text("WeaponActorClass").filter(|s| s.starts_with("/Game/")) {
        Some(t) => t.to_owned(),
        None => wid_pkg.import_path(wid_props.object("WeaponActorClass").filter(|i| *i < 0).ok_or_else(|| format!("{wid}: no external WeaponActorClass"))?),
    };
    let chain = actor_chain(src, &actor)?;
    let (core, core_source) = chain.core()?;
    let graph = loader::package(src, PLAYER_ANIM_BP)?;
    let map_key = if female { "CoreAnims_Female" } else { "CoreAnims_Male" };
    let map = core_map(&graph, map_key)?;
    let (_, entry) = map.iter().find(|(k, _)| *k == core).ok_or_else(|| format!("{map_key} has no entry {core}"))?;
    let mut core_assets = vec![];
    for f in &entry.0 {
        if let PropValue::Object(i) = f.value {
            let path = match i { 0 => None, i if i < 0 => Some(graph.import_path(i)), _ => return Err(format!("{map_key}[{core}].{}: internal reference", f.name)) };
            if let Some(p) = &path { if !p.starts_with("/Game/") { return Err(format!("{map_key}[{core}].{}: asset outside game packages", f.name)); } }
            core_assets.push(CoreAsset { field: field_base(&f.name).to_owned(), raw: f.name.clone(), index: f.array_index, path });
        }
    }
    let mut montages = vec![];
    for k in MONTAGE_PROPS { montages.push((k, chain.object(k)?)); }
    Ok(WeaponAnims { wid: wid.to_owned(), actor, female, core, core_source, core_assets, montages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    /// Every `WID_*_Athena_*` ranged weapon of the 1.8 BR data (stat table AthenaRangedWeapons), same selection as `fn_hitreg::data::br_weapon_wids`.
    fn br_ranged_wids(set: &fn_pak::PakSet, src: &ClipSourceDir) -> Vec<String> {
        let mut v: Vec<String> = set.glob("*WID_*Athena*.uasset").into_iter().map(|(_, e)| e.rel_path().to_owned())
            .filter(|p| p.contains("/Athena/Items/Weapons/") && !p.contains("Harvest") && !p.contains("Schematics"))
            .map(|p| format!("/Game/{}", p.trim_start_matches("FortniteGame/Content/").trim_end_matches(".uasset"))).collect();
        v.sort(); v.dedup();
        v.retain(|w| {
            let pkg = loader::package(src, w).unwrap();
            let Some(c) = (0..pkg.exports.len()).map(|i| pkg.export_class(i)).find(|c| c.contains("ItemDefinition")) else { return false };
            let (s, e) = pkg.export_range(pkg.find_export(&c).unwrap());
            let p = read_props(&mut Reader::at(&pkg.data[..e], s), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).unwrap();
            p.fields("WeaponStatHandle").and_then(|h| h.object("DataTable")).filter(|i| *i < 0).map_or(false, |i| pkg.import_path(i).ends_with("AthenaRangedWeapons.AthenaRangedWeapons"))
        });
        v
    }
    #[test]
    fn every_br_weapon_resolves_a_complete_animation_set_male_and_female() {
        let root = fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install");
        let set = fn_pak::PakSet::open_install(root).unwrap();
        let src = ClipSourceDir::Pak(&set);
        let wids = br_ranged_wids(&set, &src);
        assert_eq!(wids.len(), 45, "45 BR ranged weapons");
        let mut problems: Vec<String> = vec![];
        let mut by_core: BTreeMap<String, (usize, String)> = BTreeMap::new();
        let mut checked = std::collections::HashSet::new();
        let mut absent: Vec<String> = vec![];
        for wid in &wids {
            let name = wid.rsplit('/').next().unwrap();
            let male = weapon_anims(&src, wid, false).unwrap_or_else(|e| panic!("{name}: {e}"));
            let female = weapon_anims(&src, wid, true).unwrap_or_else(|e| panic!("{name} (female): {e}"));
            assert_eq!(male.core, female.core, "{name}: core enum does not depend on gender");
            assert_eq!(male.montages, female.montages);
            // Female map picks female assets for the same fields.
            assert_eq!(male.core_assets.len(), female.core_assets.len(), "{name}: male/female core field count");
            let e = by_core.entry(male.core.clone()).or_insert((0, male.core_source.clone()));
            e.0 += 1;
            // Idle (pose + aim offset), jog additive, sprint and crouch assets of the core entry.
            for f in ["TargetingPose", "SprintAnimation", "CrouchTargetingPose"] {
                if male.core_asset(f).is_none() { problems.push(format!("{name}: core {} has no {f}", male.core)); }
                if female.core_asset(f).is_none() { problems.push(format!("{name}: female core {} has no {f}", female.core)); }
            }
            // Player-side montages (equip / fire / ADS fire / reload) must exist for every weapon. The weapon-mesh-side montages are optional data:
            // a weapon whose chain serializes none animates its mesh another way (AnimClass) or not at all; listed, never substituted.
            for m in male.missing().into_iter().filter(|m| m.starts_with("montage")) {
                if ["EquipAnimation", "FireAnimation", "FireDownsightsAnimation", "ReloadAnimation"].iter().any(|p| m.ends_with(p) && !m.contains("Weapon")) { problems.push(format!("{name}: {m} not serialized in the actor chain")); }
                else { absent.push(format!("{name}: {m}")); }
            }
            // every referenced package must exist with the right class
            for w in [&male, &female] {
                for p in w.packages() {
                    if !checked.insert(p.clone()) { continue; }
                    let pkg = loader::package(&src, &p).unwrap_or_else(|e| panic!("{name}: {e}"));
                    let ok = ["AnimMontage", "AnimComposite", "AnimSequence"].iter().any(|c| pkg.find_export(c).is_some()) || (0..pkg.exports.len()).any(|i| fn_anim::BlendSpaceKind::from_class(&pkg.export_class(i)).is_some());
                    if !ok { problems.push(format!("{name}: {p} is not an animation asset")); }
                }
            }
        }
        for (c, (n, s)) in &by_core { eprintln!("core {c}: {n} weapons ({s})"); }
        for a in &absent { eprintln!("DATA-ABSENT {a}"); }
        for p in &problems { eprintln!("PROBLEM {p}"); }
        assert!(problems.is_empty(), "{} problems", problems.len());
    }
}

#[cfg(test)]
mod closure_tests {
    use super::*;
    /// One weapon per core enum, male and female: the full closure (core assets + all montages with their sequences) loads through `Resources::add_weapon_anims`.
    #[test]
    fn weapon_anim_closure_loads_for_every_core_class_male_and_female() {
        let root = fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install");
        let set = fn_pak::PakSet::open_install(root).unwrap();
        let src = ClipSourceDir::Pak(&set);
        let base = "/Game/Athena/Items/Weapons/";
        let picks = [("WID_Assault_Auto_Athena_R_Ore_T03", false), ("WID_Pistol_SixShooter_Athena_R_Ore_T03", true), ("WID_Shotgun_Standard_Athena_UC_Ore_T03", false), ("WID_Shotgun_SemiAuto_Athena_R_Ore_T03", true), ("WID_Launcher_Rocket_Athena_R_Ore_T03", false), ("WID_Launcher_Grenade_Athena_R_Ore_T03", true)];
        let mut r = crate::loadout::Resources::load(&src).unwrap();
        for (w, female) in picks {
            let a = weapon_anims(&src, &format!("{base}{w}"), female).unwrap();
            r.add_weapon_anims(&src, &a).unwrap();
            for p in a.packages() {
                assert!(r.sequences.contains_key(&p) || r.blendspaces.contains_key(&p) || r.montages.contains_key(&p) || r.composites.contains_key(&p), "{w}: {p} not loaded");
            }
            assert!(r.core_anims.contains_key(&a.core));
        }
        eprintln!("closure: {} sequences, {} montages, {} blendspaces, cores {:?}", r.sequences.len(), r.montages.len(), r.blendspaces.len(), r.core_anims.keys().collect::<Vec<_>>());
    }
}
