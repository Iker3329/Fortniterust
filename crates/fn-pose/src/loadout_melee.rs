//! BR (Athena) pickaxe: the 1.8 data path from the cosmetic to every animation asset, resolved by FULL package path.
//!
//! Chain (every link is a serialized 1.8 property, Battle Royale / Athena only):
//!  * `AthenaPickaxeItemDefinition` cosmetic (`DEFAULT_PICKAXE`) `.WeaponDefinition` -> `FortWeaponMeleeItemDefinition` (`DEFAULT_WID`, Athena folder)
//!  * WID `.WeaponActorClass` -> Blueprint chain (`B_Melee_Impact_Pickaxe_Athena` -> `B_Melee_Impact_Generic` -> `B_Melee_Generic`) ending in the native
//!    `/Script/FortniteGame.FortWeapon`; the first CDO of that chain serializing `EquipAnimation` gives the equip montage.
//!  * `WeaponCoreAnimation`: no Blueprint of the chain serializes it, so it is the native `AFortWeapon` constructor default (decomp: writes 0 = `Melee`; see [`native_core`]).
//!  * `Fortnite_M_Avg_Player_AnimBlueprint` CDO `.CoreAnims_Male[Melee]` -> `PlayerAnimAssets_Struct` (poses, aim offsets, sprint / crouch clips, additive blend spaces).
//!  * WID `.PrimaryFireAbility` (`GAB_Melee_ImpactCombo_Athena`) CDO `.MontageInfo.MontageToPlay` -> the swing montage; each of its composite sections is one swing variant.
//!
//! Nothing here globs by base name. [`basename_hits`] exists so callers (and the test) can detect the legacy `ClipStore::load` hazard: it takes the FIRST
//! pak entry matching `**/<name>.uasset`, which is only correct while the name is unique in the paks.
use crate::{loader, ClipSourceDir};
use fn_anim::{montage::AnimMontage, BlendSpace, BlendSpaceKind};
use fn_ueio::{props::{read_props, Ctx, PropList, PropValue}, Package, Reader};
use std::collections::HashMap;

/// Default pickaxe cosmetic of the BR loadout (`AthenaPickaxeItemDefinition`); its `WeaponDefinition` is [`DEFAULT_WID`].
pub const DEFAULT_PICKAXE: &str = "/Game/Athena/Items/Cosmetics/Pickaxes/DefaultPickaxe";
/// The Athena harvesting-tool weapon item definition (what `DefaultPickaxe.WeaponDefinition` points at).
pub const DEFAULT_WID: &str = "/Game/Athena/Items/Weapons/WID_Harvest_Pickaxe_Athena_C_T01";
pub const PLAYER_ANIMBP: &str = "/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint";
/// Roots of Save the World content in the 1.8 paks (weapon / schematic / hero data and STW-only animation trees). No BR pickaxe asset may live here.
/// `/Game/Items/Weapons/Melee/Harvest/WID_Harvest_Pickaxe_C_T01..` is the STW harvesting tool the task warned about.
pub const STW_ROOTS: [&str; 6] = ["/Game/Items/Weapons/", "/Game/Items/Schematics/", "/Game/Items/Heroes/", "/Game/Heroes/", "/Game/Abilities/Heroes/", "/Game/Animation/Game/Heroes/"];

fn pkg_of(reference: &str) -> &str { reference.split('.').next().unwrap_or(reference) }
fn short(path: &str) -> &str { pkg_of(path).rsplit('/').next().unwrap_or(path) }

/// Native constructor default of `WeaponCoreAnimation` for a Blueprint chain that ends in `native_class` (delegates to `loadout_weapon::native_core_default`,
/// exe-exact: `AFortWeapon::AFortWeapon` @0x140914b60 writes byte +0xA80 = 0 = `Melee`, re-verified in the decomp). `None` = no traced constructor: report, do not guess.
pub fn native_core(native_class: &str) -> Option<String> { crate::loadout_weapon::native_core_default(native_class).ok().map(|(n, _)| n.to_owned()) }

fn export_props(pkg: &Package, idx: usize) -> Result<PropList, String> {
    let (start, end) = pkg.export_range(idx);
    read_props(&mut Reader::at(&pkg.data[..end], start), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())
}
fn cdo_props(pkg: &Package) -> Result<PropList, String> {
    let idx = pkg.exports.iter().position(|e| e.object_name.starts_with("Default__")).ok_or("missing class default object")?;
    export_props(pkg, idx)
}
/// External object reference of property `key` as a full `/Game/..` object path.
fn object_path(pkg: &Package, p: &PropList, key: &str) -> Option<String> {
    if let Some(t) = p.text(key).filter(|s| s.starts_with("/Game/")) { return Some(t.to_owned()); }
    let i = p.object(key).filter(|i| *i < 0)?;
    Some(pkg.import_path(i))
}
/// `CoreAnims_Male` payload (removed count, entry count, enum FName + tagged struct), same encoding `loadout.rs::core_map` reads.
fn core_entries(pkg: &Package) -> Result<HashMap<String, PropList>, String> {
    let p = cdo_props(pkg)?;
    let Some(PropValue::Raw(bytes)) = p.get("CoreAnims_Male") else { return Err("missing original male core map".into()) };
    let mut r = Reader::new(bytes);
    let e = |x: fn_ueio::Error| x.to_string();
    if r.i32().map_err(e)? != 0 { return Err("unsupported core map removals".into()); }
    let count = r.i32().map_err(e)?;
    if !(0..=256).contains(&count) { return Err("invalid core map count".into()); }
    let mut out = HashMap::new();
    for _ in 0..count {
        let name = r.i32().map_err(e)?;
        if r.i32().map_err(e)? != 0 { return Err("numbered core enum".into()); }
        let key = pkg.names.get(usize::try_from(name).map_err(|_| "negative enum name")?).ok_or("core enum name out of bounds")?.rsplit("::").next().unwrap_or_default().to_owned();
        let value = read_props(&mut r, &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|x| x.to_string())?;
        out.insert(key, value);
    }
    Ok(out)
}

/// One `PlayerAnimAssets_Struct` field of the `Melee` core entry with its exact package path (None = explicit null in the data).
#[derive(Debug, Clone, PartialEq)]
pub struct CoreField { pub name: String, pub array_index: i32, pub path: Option<String> }

/// One swing variant: a composite section of the swing montage and the clip each slot track plays for it.
#[derive(Debug, Clone, PartialEq)]
pub struct SwingVariant {
    /// Montage section name (`Swing1`, `Swing2`, `Swing1_2Idle`, ... exactly as serialized).
    pub section: String,
    /// `NextSectionName` link of the section (the combo chain).
    pub next_section: String,
    /// Section start inside the montage (seconds).
    pub start: f32,
    /// Per slot track: (slot name, full sequence package path, AnimEndTime = played length, AnimPlayRate).
    pub tracks: Vec<SwingTrack>,
}
#[derive(Debug, Clone, PartialEq)]
pub struct SwingTrack { pub slot: String, pub sequence: String, pub length: f32, pub play_rate: f32 }

/// The resolved pickaxe asset set. Every path is a full package path from serialized 1.8 data.
#[derive(Debug)]
pub struct PickaxeSet {
    pub cosmetic: String,
    pub weapon_definition: String,
    pub weapon_actor_class: String,
    /// Blueprint chain, most derived first, ending with the native class (`/Script/FortniteGame.FortWeapon`).
    pub actor_chain: Vec<String>,
    /// `Melee` (native constructor default, see [`native_core`]).
    pub core_animation: String,
    /// Every field of `CoreAnims_Male[Melee]`.
    pub core_fields: Vec<CoreField>,
    /// Samples (full paths) of each BlendSpace / AimOffset referenced by `core_fields` (key = blend space package).
    pub blendspace_samples: HashMap<String, Vec<String>>,
    /// `EquipAnimation` of the actor chain (`Melee_Equip`) + its parsed montage.
    pub equip_montage: String,
    pub equip: AnimMontage,
    /// Sequences the equip montage plays (full paths).
    pub equip_sequences: Vec<String>,
    /// `PrimaryFireAbility` of the WID and its `MontageInfo.MontageToPlay`.
    pub primary_fire_ability: String,
    pub swing_montage: String,
    pub swing: AnimMontage,
    pub swing_variants: Vec<SwingVariant>,
    /// WID mesh data (SK_Pick).
    pub weapon_mesh: String,
}

impl PickaxeSet {
    /// `CoreAnims_Male[Melee]` field by name (and array index), non-null only.
    pub fn core_path(&self, name: &str, array_index: i32) -> Option<&str> {
        self.core_fields.iter().find(|f| f.name == name && f.array_index == array_index).and_then(|f| f.path.as_deref())
    }
    /// Swing variant by montage section name.
    pub fn variant(&self, section: &str) -> Option<&SwingVariant> { self.swing_variants.iter().find(|v| v.section == section) }
    /// Every package path of the pickaxe set (core fields, blend space samples, equip + swing montages and their sequences, mesh, WID, cosmetic, ability).
    pub fn all_paths(&self) -> Vec<String> {
        let mut v: Vec<String> = vec![self.cosmetic.clone(), self.weapon_definition.clone(), self.weapon_actor_class.clone(), self.primary_fire_ability.clone(), self.weapon_mesh.clone(), self.equip_montage.clone(), self.swing_montage.clone()];
        v.extend(self.actor_chain.iter().filter(|c| c.starts_with("/Game/")).cloned());
        v.extend(self.core_fields.iter().filter_map(|f| f.path.clone()));
        v.extend(self.blendspace_samples.values().flatten().cloned());
        v.extend(self.equip_sequences.iter().cloned());
        v.extend(self.swing_variants.iter().flat_map(|s| s.tracks.iter().map(|t| t.sequence.clone())));
        v.sort(); v.dedup(); v
    }
    /// Sequence base names (what the legacy name-keyed `ClipStore` / `required_clips()` use) of every sequence of the set: core poses, blend space samples, equip, swings.
    pub fn sequence_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.core_fields.iter().filter_map(|f| f.path.as_deref()).map(|p| short(p).to_owned()).collect();
        v.extend(self.blendspace_samples.values().flatten().map(|p| short(p).to_owned()));
        v.extend(self.equip_sequences.iter().map(|p| short(p).to_owned()));
        v.extend(self.swing_variants.iter().flat_map(|s| s.tracks.iter().map(|t| short(&t.sequence).to_owned())));
        v.sort(); v.dedup(); v
    }
    /// Paths that sit under a Save the World root ([`STW_ROOTS`]). Must be empty.
    pub fn stw_paths(&self) -> Vec<String> { self.all_paths().into_iter().filter(|p| is_stw(p)).collect() }
}

/// True when `path` is under a Save the World root.
pub fn is_stw(path: &str) -> bool { STW_ROOTS.iter().any(|r| path.starts_with(r)) }

/// Every pak entry (engine-relative package path without extension) whose file name is `<name>.uasset`, which is what `ClipStore::load`'s
/// `fortnitegame/content/**/<name>.uasset` glob matches (it silently takes the first). More than one hit = ambiguous base-name lookup.
pub fn basename_hits(src: &ClipSourceDir, name: &str) -> Result<Vec<String>, String> {
    let ClipSourceDir::Pak(set) = src else { return Err("basename_hits needs the pak source".into()) };
    Ok(set.glob(&format!("fortnitegame/content/**/{}.uasset", name.to_lowercase())).into_iter().map(|(_, e)| e.path.trim_end_matches(".uasset").to_owned()).collect())
}

fn montage_sequences(m: &AnimMontage, what: &str) -> Result<Vec<String>, String> {
    let mut v = vec![];
    for slot in &m.slots { for seg in &slot.segments {
        v.push(pkg_of(seg.animation_path.as_deref().ok_or_else(|| format!("{what}: segment lacks external animation reference"))?).to_owned());
    } }
    Ok(v)
}
fn parse_montage(src: &ClipSourceDir, path: &str) -> Result<AnimMontage, String> {
    let pkg = loader::package(src, pkg_of(path))?;
    if pkg.find_export("AnimMontage").is_none() { return Err(format!("{path}: not an AnimMontage")); }
    AnimMontage::parse(&pkg).map_err(|e| format!("{path}: {e}"))
}

/// Swing variants of a montage: one per `CompositeSections` entry. Start time = `SegmentBeginTime + LinkValue` for `Relative` links, `LinkValue` for `Absolute`
/// (UE `FAnimLinkableElement` semantics; `fn_anim::AnimMontage::sections[].start_time` only keeps `LinkValue`, which is 0 for every pickaxe section, so it is not used).
fn variants(pkg: &Package, m: &AnimMontage) -> Result<Vec<SwingVariant>, String> {
    let ei = pkg.find_export("AnimMontage").ok_or("no AnimMontage export")?;
    let props = export_props(pkg, ei)?;
    let mut out = vec![];
    for it in props.array("CompositeSections").ok_or("montage has no CompositeSections")? {
        let f = it.as_fields().ok_or("bad CompositeSection")?;
        let link = f.float("LinkValue").unwrap_or(0.) as f32;
        let start = match f.text("LinkMethod").map(|s| s.rsplit("::").next().unwrap_or(s)) {
            Some("Absolute") => link,
            Some("Relative") | None => f.float("SegmentBeginTime").unwrap_or(0.) as f32 + link,
            Some(o) => return Err(format!("unsupported section link method {o}")),
        };
        let mut tracks = vec![];
        for slot in &m.slots { for seg in &slot.segments {
            if (seg.start_pos - start).abs() < 1e-3 {
                tracks.push(SwingTrack { slot: slot.slot_name.clone(), sequence: pkg_of(seg.animation_path.as_deref().ok_or("swing segment lacks external animation reference")?).to_owned(), length: seg.anim_end_time - seg.anim_start_time, play_rate: seg.play_rate });
            }
        } }
        out.push(SwingVariant { section: f.text("SectionName").unwrap_or("").to_owned(), next_section: f.text("NextSectionName").unwrap_or("").to_owned(), start, tracks });
    }
    Ok(out)
}

/// Resolve the pickaxe set of a pickaxe cosmetic (`DEFAULT_PICKAXE` for the default loadout).
pub fn load_for(src: &ClipSourceDir, cosmetic: &str) -> Result<PickaxeSet, String> {
    let cpkg = loader::package(src, cosmetic)?;
    let ci = cpkg.exports.iter().position(|e| e.object_name.eq_ignore_ascii_case(short(cosmetic))).ok_or("cosmetic export missing")?;
    if cpkg.export_class(ci) != "AthenaPickaxeItemDefinition" { return Err(format!("{cosmetic}: not an AthenaPickaxeItemDefinition (got {})", cpkg.export_class(ci))); }
    let weapon_definition = object_path(&cpkg, &export_props(&cpkg, ci)?, "WeaponDefinition").ok_or("pickaxe cosmetic has no WeaponDefinition")?;
    let wid_pkg = loader::package(src, pkg_of(&weapon_definition))?;
    let wi = wid_pkg.exports.iter().position(|e| e.object_name == short(&weapon_definition)).ok_or("WID export missing")?;
    let wid = export_props(&wid_pkg, wi)?;
    let weapon_actor_class = object_path(&wid_pkg, &wid, "WeaponActorClass").ok_or("WID has no WeaponActorClass")?;
    let primary_fire_ability = object_path(&wid_pkg, &wid, "PrimaryFireAbility").ok_or("WID has no PrimaryFireAbility")?;
    let weapon_mesh = object_path(&wid_pkg, &wid, "WeaponMeshOverride").ok_or("WID has no WeaponMeshOverride")?;

    // actor Blueprint chain: EquipAnimation / WeaponCoreAnimation from the first CDO that serializes them
    let mut actor_chain = vec![];
    let (mut equip_montage, mut core_animation) = (None::<String>, None::<String>);
    let mut path = pkg_of(&weapon_actor_class).to_owned();
    for _ in 0..32 {
        let pkg = loader::package(src, &path)?;
        actor_chain.push(path.clone());
        let p = cdo_props(&pkg)?;
        if equip_montage.is_none() { equip_montage = object_path(&pkg, &p, "EquipAnimation"); }
        if core_animation.is_none() { core_animation = p.text("WeaponCoreAnimation").map(|c| c.rsplit("::").next().unwrap_or(c).to_owned()); }
        let class = pkg.exports.iter().enumerate().find(|(i, _)| pkg.export_class(*i) == "BlueprintGeneratedClass").map(|(_, e)| e).ok_or("missing generated class")?;
        if class.super_index >= 0 { return Err(format!("{path}: internal super class")); }
        let parent = pkg.import_path(class.super_index);
        if !parent.starts_with("/Game/") {
            actor_chain.push(parent.clone());
            if core_animation.is_none() { core_animation = Some(native_core(&parent).ok_or_else(|| format!("{path}: no serialized WeaponCoreAnimation before native parent {parent}, and no recovered constructor default for it"))?); }
            break;
        }
        path = pkg_of(&parent).to_owned();
    }
    let core_animation = core_animation.ok_or("actor chain did not terminate")?;
    let equip_montage = pkg_of(&equip_montage.ok_or("no EquipAnimation in the actor chain")?).to_owned();

    // CoreAnims_Male[core]: exact asset paths (indices only mean something inside the AnimBlueprint package)
    let abp = loader::package(src, PLAYER_ANIMBP)?;
    let entries = core_entries(&abp)?;
    let entry = entries.get(&core_animation).ok_or_else(|| format!("missing original CoreAnims_Male[{core_animation}]"))?;
    let mut core_fields = vec![];
    for f in &entry.0 {
        if let PropValue::Object(i) = f.value {
            let path = if i == 0 { None } else if i < 0 { Some(abp.import_path(i)) } else { return Err("internal core animation reference".into()) };
            core_fields.push(CoreField { name: f.name.clone(), array_index: f.array_index, path });
        }
    }
    // blend spaces / aim offsets referenced by the entry: their sample sequences
    let mut blendspace_samples = HashMap::new();
    for f in &core_fields {
        let Some(p) = &f.path else { continue };
        let bp = pkg_of(p);
        let pkg = loader::package(src, bp)?;
        if pkg.exports.iter().enumerate().any(|(i, _)| BlendSpaceKind::from_class(&pkg.export_class(i)).is_some()) {
            let space = BlendSpace::parse(&pkg).map_err(|e| format!("{bp}: {e}"))?;
            let mut s = vec![];
            for sample in &space.samples { if let Some(r) = &sample.animation_path { s.push(pkg_of(r).to_owned()); } }
            blendspace_samples.insert(bp.to_owned(), s);
        }
    }

    let equip = parse_montage(src, &equip_montage)?;
    let equip_sequences = montage_sequences(&equip, &equip_montage)?;

    // swing montage: GAB_*.MontageInfo.MontageToPlay
    let gpkg = loader::package(src, pkg_of(&primary_fire_ability))?;
    let gp = cdo_props(&gpkg)?;
    let info = gp.fields("MontageInfo").ok_or("ability has no MontageInfo")?;
    let swing_montage = pkg_of(&object_path(&gpkg, info, "MontageToPlay").ok_or("MontageInfo has no MontageToPlay")?).to_owned();
    let swing = parse_montage(src, &swing_montage)?;
    let swing_variants = variants(&loader::package(src, &swing_montage)?, &swing)?;
    Ok(PickaxeSet { cosmetic: cosmetic.to_owned(), weapon_definition: pkg_of(&weapon_definition).to_owned(), weapon_actor_class: pkg_of(&weapon_actor_class).to_owned(), actor_chain, core_animation, core_fields, blendspace_samples, equip_montage, equip, equip_sequences, primary_fire_ability: pkg_of(&primary_fire_ability).to_owned(), swing_montage, swing, swing_variants, weapon_mesh: pkg_of(&weapon_mesh).to_owned() })
}
/// The default loadout pickaxe (`DefaultPickaxe` -> `WID_Harvest_Pickaxe_Athena_C_T01`).
pub fn load(src: &ClipSourceDir) -> Result<PickaxeSet, String> { load_for(src, DEFAULT_PICKAXE) }

#[cfg(test)]
mod tests {
    use super::*;
    fn paks() -> fn_pak::PakSet {
        let root = fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install");
        fn_pak::PakSet::open_install(root).unwrap()
    }
    const GRIP: &str = "/Game/Animation/Game/MainPlayer/Combat/Melee/TwoHanded/WideGrip/UpperBody/";
    #[test]
    fn native_constructor_defaults() {
        assert_eq!(native_core("/Script/FortniteGame.FortWeapon").as_deref(), Some("Melee"));
        assert_eq!(native_core("/Script/FortniteGame.FortWeaponRanged").as_deref(), Some("Pistol"));
        assert_eq!(native_core("/Script/FortniteGame.FortSomethingElse"), None);
    }
    #[test]
    fn br_pickaxe_chain_and_variants_come_from_athena_data() {
        let p = paks();
        let s = load(&ClipSourceDir::Pak(&p)).unwrap();
        assert_eq!(s.weapon_definition, DEFAULT_WID);
        assert_eq!(s.weapon_actor_class, "/Game/Weapons/FORT_Melee/Blueprints/Impact/B_Melee_Impact_Pickaxe_Athena");
        assert_eq!(s.actor_chain.last().unwrap(), "/Script/FortniteGame.FortWeapon");
        assert_eq!(s.core_animation, "Melee");
        assert_eq!(s.primary_fire_ability, "/Game/Athena/Items/Weapons/Abilities/GAB_Melee_ImpactCombo_Athena");
        assert_eq!(s.equip_montage, "/Game/Animation/Game/MainPlayer/Montages/Melee/Melee_Equip");
        assert_eq!(s.equip_sequences, [format!("{}Melee_WideGrip_Equip_CMM", GRIP.replace("UpperBody/", ""))]);
        assert_eq!(s.swing_montage, "/Game/Animation/Game/MainPlayer/Montages/Melee/PlayerMeleeCombo");
        assert_eq!(s.weapon_mesh, "/Game/Weapons/FORT_Melee/Meshes/SK_Pick");
        // CoreAnims_Male[Melee] equals what the animation graph bakes (same names, now with full paths)
        let baked = fn_animgraph::core_anims("Melee").unwrap();
        let name = |prefix: &str| s.core_fields.iter().find(|f| f.name.starts_with(prefix)).and_then(|f| f.path.as_deref()).map(short).unwrap_or("");
        assert_eq!(name("TargetingPose"), baked.targeting_pose);
        assert_eq!(name("TargetingAimOffset"), baked.targeting_ao);
        assert_eq!(name("JogAdditiveBlendspace_"), baked.jog_additive);
        assert_eq!(name("SprintAnimation"), baked.sprint);
        assert_eq!(name("CrouchTargetingPose"), baked.crouch_targeting_pose);
        assert_eq!(name("CrouchJogAdditiveBlendspace_"), baked.crouch_jog_additive);
        assert_eq!(name("CrouchSprintAnimation"), baked.crouch_sprint);
        assert_eq!(s.core_path("TargetingPose_4_D2562A4A44A308F2ADE73F8A183BA29E", 0), Some("/Game/Animation/Game/MainPlayer/Poses/Medium/Male/M_Melee_TwoHanded.M_Melee_TwoHanded"));
        // swing variants: PlayerMeleeCombo composite sections, two slot tracks each (standing clip on FullBodyInPlace, in-motion clip on UpperBody)
        let expect: [(&str, &str, f32, &str, &str); 6] = [
            ("Swing1", "Swing2", 0., "TwoHanded_Loop_Swing1", "TwoHanded_Loop_Swing1_InMotion"),
            ("Swing2", "Swing1", 0.5333, "TwoHanded_Loop_Swing2", "TwoHanded_Loop_Swing2_InMotion"),
            ("End1", "None", 1.16663, "TwoHanded_Loop_Swing1_2Idle", "TwoHanded_Loop_Swing1_2Idle_InMotion"),
            ("End2", "None", 1.69997, "TwoHanded_Loop_Swing2_2Idle", "TwoHanded_Loop_Swing2_2Idle_InMotion"),
            ("Bounce1", "None", 1.99997, "TwoHanded_Loop_Swing1_Bounce", "TwoHanded_Loop_Swing1_Bounce"),
            ("Bounce2", "None", 2.39997, "TwoHanded_Loop_Swing2_Bounce", "TwoHanded_Loop_Swing2_Bounce"),
        ];
        assert_eq!(s.swing_variants.len(), expect.len());
        for (v, (sec, next, start, full, upper)) in s.swing_variants.iter().zip(expect) {
            assert_eq!((v.section.as_str(), v.next_section.as_str()), (sec, next));
            assert!((v.start - start).abs() < 1e-3, "{sec} start {}", v.start);
            assert_eq!(v.tracks.len(), 2, "{sec}");
            assert_eq!((v.tracks[0].slot.as_str(), v.tracks[0].sequence.as_str()), ("FullBodyInPlace", format!("{GRIP}{full}").as_str()));
            assert_eq!((v.tracks[1].slot.as_str(), v.tracks[1].sequence.as_str()), ("UpperBody", format!("{GRIP}{upper}").as_str()));
        }
        assert!(s.variant("Swing2").is_some() && s.variant("nope").is_none());
    }
    /// No resolved path may come from Save the World, every package must exist in the 1.8 paks, and every sequence base name must be unique in the paks
    /// (the legacy `ClipStore::load` globs `**/<name>.uasset` and takes the first hit, so a duplicate could silently load an STW clip).
    #[test]
    fn no_resolved_asset_is_save_the_world() {
        let p = paks();
        let src = ClipSourceDir::Pak(&p);
        let s = load(&src).unwrap();
        assert_eq!(s.stw_paths(), Vec::<String>::new());
        assert!(is_stw("/Game/Items/Weapons/Melee/Harvest/WID_Harvest_Pickaxe_C_T01"), "sanity: the STW tool path must be flagged");
    }
    #[test]
    fn every_path_exists_and_basenames_are_unambiguous() {
        let p = paks();
        let src = ClipSourceDir::Pak(&p);
        let s = load(&src).unwrap();
        for path in s.all_paths() {
            assert!(!is_stw(&path), "STW path {path}");
            assert!(p.read_game(&path, "uasset").is_ok(), "{path} not in the paks");
        }
        for n in s.sequence_names() {
            let hits = basename_hits(&src, &n).unwrap();
            assert_eq!(hits.len(), 1, "ambiguous base name {n}: {hits:?}");
            assert!(!is_stw(&format!("/Game/{}", hits[0].trim_start_matches("fortnitegame/content/"))), "{n} resolves to STW {hits:?}");
        }
    }
    #[test]
    fn a_non_pickaxe_cosmetic_is_refused() {
        let p = paks();
        assert!(load_for(&ClipSourceDir::Pak(&p), "/Game/Athena/Items/Weapons/WID_Harvest_Pickaxe_Athena_C_T01").is_err());
    }
}
