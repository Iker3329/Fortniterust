//! BUILD ANIM HOOK: the player side of Battle Royale building mode (the held "building blueprint", its pencil and every player animation of build / edit mode),
//! resolved from the 1.8 data only. Every link below is a serialized 1.8 property or an exe fact (RVAs of the 1.8 `FortniteClient-Win64-Shipping.exe`):
//!
//!  * `/Game/Athena/Athena_GameMode` CDO `StartingItems` (the fresh Athena player's inventory) names the build pieces (`FortBuildingItemDefinition`
//!    `BuildingItemData_Wall` / `_Floor` / `_Stair_W` / `_RoofS`) and the edit tool (`FortEditToolItemDefinition` `EditTool`). They live under
//!    `/Game/Items/Weapons/BuildingTools/`, but the Athena game mode itself grants them (there is no Athena-only copy in 1.8).
//!  * item `WeaponActorClass` -> `DefaultBuildingTool_C` (native `FortWeap_BuildingTool`) / `DefaultEditingTool_C` (native `FortWeap_EditingTool`). Their CDOs serialize:
//!    `EquipAnimation` = montage `Blueprint_Equip`; `WeaponMesh` = component `WeaponMesh0` (`SkeletalMesh` `SK_Blueprint_Paper`, `AnimClass` `Blueprint_Paper_VIM_C`);
//!    `PencilMeshComponent` = `PencilMeshComponent0` (`StaticMesh` `Pencil`); `BlueprintPaperSocket` `BluePrintPaper`, `BlueprintPencilSocket` `BluePrintPen`; the building
//!    tool also `BlueprintPaperMaterial` `ArchitectTools`, `BlueprintDiffuseTextures[Wood, Stone, Metal]`, `BlueprintEmissiveMaskTextures[..]`, `BlueprintPaperPulseCurve`.
//!  * `WeaponCoreAnimation`: no CDO serializes it; both native ctors (0x140914770 / 0x140914890) write byte +0xA80 = 3 = `PaperBlueprint`
//!    (`loadout_weapon::native_core_default`). -> `CoreAnims_Male[PaperBlueprint]` of the player AnimBlueprint.
//!  * `AFortWeap_BuildingToolBase::AttachWeaponToPawn` (0x140918690): `AFortWeapon::AttachWeaponToPawn(mesh, BlueprintPaperSocket)` (the paper on `BluePrintPaper`), then the
//!    pencil component `AttachToComponent(pawn mesh, BlueprintPencilSocket)` with relative location `DAT_146683fe0` / scale `DAT_146683ff0` (FVector::ZeroVector / OneVector:
//!    bss globals compared as the identity transform by FUN_142242520) and its relative rotation only normalised.
//!  * Pawn `BuildingState` (+0x104A, `EFortBuildingState`) via RVA 0x1407ffb90: BuildingTool::OnEquip (0x14092f5c0) 0 = Placement, EditingTool::OnEquip (0x14092f7f0)
//!    1 = EditMode, both OnUnEquip (0x140932630 / 0x140932760) 2 = None.
//!  * Pawn montages (`AFortPlayerPawn` +0x10F0 / +0x10F8, CDO of `PlayerPawn_Athena_Generic`): `BluePrintPlaceAnimation` = `Blueprint_Place`, played by RVA 0x1407f6a70
//!    (from the local placement 0x1408336f0 and `ServerCreateBuildingActor` 0x140845d70) and `BluePrintEditAnimation` = `Blueprint_Edit`, played by RVA 0x1407f69f0
//!    (from `LocalOnBlueprintEdit` 0x140834c70, bound by the begin-edit vfunc 0x140836d70, and from the edit-mode exit 0x140837140 when the edit is not cancelled).
//!    Both: `PlayAnimMontage(montage, 1.0 (float at 0x143fe36fc), None)` only when `Montage_IsPlaying(montage)` is false.
use crate::{loader, loadout_weapon::{self, ActorChain, WeaponAnims}, ClipSourceDir};
use fn_ueio::{props::{read_props, Ctx, PropList, PropValue}, Package, Reader};

pub const PLAYER_PAWN: &str = "/Game/Athena/PlayerPawn_Athena";
pub const PLAYER_SKELETON: &str = crate::loadout::PLAYER_SKELETON;

fn pkg_of(r: &str) -> &str { r.split('.').next().unwrap_or(r) }

/// One building-mode tool (the build-piece tool or the edit tool) and its held meshes.
#[derive(Clone, Debug)]
pub struct Tool {
    /// Item definition package and class (`FortBuildingItemDefinition` / `FortEditToolItemDefinition`).
    pub item: String, pub item_class: String,
    /// Everything `loadout_weapon::weapon_anims` reads for the item (actor class, core enum + its provenance, `CoreAnims_Male` assets, the montage roles).
    pub anims: WeaponAnims,
    /// Actor Blueprint chain (most derived first) and the native class it ends in.
    pub actor_chain: Vec<String>, pub native: String,
    /// `WeaponMesh` component: skeletal mesh + its `AnimClass`.
    pub weapon_mesh: String, pub weapon_anim_class: Option<String>,
    /// `PencilMeshComponent` static mesh.
    pub pencil_mesh: Option<String>,
    pub paper_socket: String, pub pencil_socket: String,
    /// Building tool only (None / empty for the edit tool): the MID parent and its per-resource `Diffuse` / `EmissiveMask` textures (index = `EFortResourceType`
    /// Wood 0, Stone 1, Metal 2; the 4th slot is unset in the data), and the `EmissiveMultiply` pulse curve.
    pub paper_material: Option<String>, pub diffuse: [Option<String>; 4], pub emissive: [Option<String>; 4], pub pulse_curve: Option<String>,
}
impl Tool {
    pub fn equip_montage(&self) -> Option<&str> { self.anims.montage("EquipAnimation") }
}

/// The whole player side of build mode.
#[derive(Clone, Debug)]
pub struct BuildToolSet {
    pub build: Tool,
    pub edit: Tool,
    /// `PlayerPawn_Athena` chain package that serializes the two montages, and the montages (full object paths).
    pub pawn: String, pub place_montage: String, pub edit_montage: String,
    /// Player skeleton sockets of the two attach points.
    pub paper_socket: fn_mesh::Socket, pub pencil_socket: fn_mesh::Socket,
    /// The male blueprint pose sequences of AnimBP node 6 (players 210 / 209), as the AnimBlueprint imports them.
    pub place_pose: String, pub edit_pose: String,
}
impl BuildToolSet {
    /// Every animation package of the set (core assets of both tools, equip / place / edit montages, the two pose sequences).
    pub fn anim_packages(&self) -> Vec<String> {
        let mut v = self.build.anims.packages();
        v.extend(self.edit.anims.packages());
        for p in [&self.place_montage, &self.edit_montage, &self.place_pose, &self.edit_pose] { v.push(pkg_of(p).to_owned()); }
        v.sort(); v.dedup(); v
    }
}

fn export_props(pkg: &Package, idx: usize) -> Result<PropList, String> {
    let (start, end) = pkg.export_range(idx);
    read_props(&mut Reader::at(&pkg.data[..end], start), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())
}
/// External object path of property `key` (index < 0) in `pkg`.
fn ext(pkg: &Package, p: &PropList, key: &str) -> Option<String> { p.object(key).filter(|i| *i < 0).map(|i| pkg.import_path(i)) }
/// Element `index` of a static array property (`Name[index]`), as an external object path.
fn ext_at(pkg: &Package, p: &PropList, key: &str, index: i32) -> Option<String> {
    p.0.iter().find(|x| x.name == key && x.array_index == index).and_then(|x| match x.value { PropValue::Object(i) if i < 0 => Some(pkg.import_path(i)), _ => None })
}
/// First link of the chain whose CDO serializes `key` as an internal component reference: (that link's package, the component's props).
fn component(chain: &ActorChain, key: &str) -> Result<Option<(usize, PropList)>, String> {
    for (n, l) in chain.links.iter().enumerate() {
        let Some(v) = l.cdo.get(key) else { continue };
        let PropValue::Object(i) = v else { return Err(format!("{}: {key} is not an object", l.path)) };
        if *i <= 0 { return Ok(None); }
        return Ok(Some((n, export_props(&l.pkg, (*i - 1) as usize).map_err(|e| format!("{}: {key}: {e}", l.path))?)));
    }
    Ok(None)
}
/// First CDO of the chain that serializes `key` (any value).
fn first<'a>(chain: &'a ActorChain, key: &str) -> Option<(&'a Package, &'a PropList)> { chain.links.iter().find(|l| l.cdo.get(key).is_some()).map(|l| (&l.pkg, &l.cdo)) }

fn tool(src: &ClipSourceDir, item: &str, item_class: &str) -> Result<Tool, String> {
    let anims = loadout_weapon::weapon_anims(src, item, false)?;
    let chain = loadout_weapon::actor_chain(src, &anims.actor)?;
    let (mn, mesh) = component(&chain, "WeaponMesh")?.ok_or_else(|| format!("{}: no WeaponMesh component", anims.actor))?;
    let mpkg = &chain.links[mn].pkg;
    let weapon_mesh = ext(mpkg, &mesh, "SkeletalMesh").ok_or_else(|| format!("{}: WeaponMesh has no SkeletalMesh", anims.actor))?;
    let weapon_anim_class = ext(mpkg, &mesh, "AnimClass");
    let pencil_mesh = component(&chain, "PencilMeshComponent")?.and_then(|(n, p)| ext(&chain.links[n].pkg, &p, "StaticMesh"));
    let name = |key: &str| first(&chain, key).and_then(|(_, p)| p.text(key).map(str::to_owned)).ok_or_else(|| format!("{}: {key} not serialized", anims.actor));
    let (paper_socket, pencil_socket) = (name("BlueprintPaperSocket")?, name("BlueprintPencilSocket")?);
    let obj = |key: &str| first(&chain, key).and_then(|(pkg, p)| ext(pkg, p, key));
    let arr = |key: &str| -> [Option<String>; 4] { std::array::from_fn(|i| first(&chain, key).and_then(|(pkg, p)| ext_at(pkg, p, key, i as i32))) };
    Ok(Tool { item: pkg_of(item).to_owned(), item_class: item_class.to_owned(), actor_chain: chain.links.iter().map(|l| l.path.clone()).collect(), native: chain.native.clone(),
        weapon_mesh, weapon_anim_class, pencil_mesh, paper_socket, pencil_socket, paper_material: obj("BlueprintPaperMaterial"), diffuse: arr("BlueprintDiffuseTextures"),
        emissive: arr("BlueprintEmissiveMaskTextures"), pulse_curve: obj("BlueprintPaperPulseCurve"), anims })
}

/// The blueprint pose sequence the AnimBlueprint imports under `name` (exactly one import; full object path).
fn animbp_import(pkg: &Package, name: &str) -> Result<String, String> {
    let hits: Vec<String> = (0..pkg.imports.len() as i32).map(|i| pkg.import_path(-(i + 1))).filter(|p| p.starts_with("/Game/") && pkg_of(p).rsplit('/').next() == Some(name) && p.contains('.')).collect();
    let mut hits = hits; hits.sort(); hits.dedup();
    match hits.as_slice() { [one] => Ok(one.clone()), [] => Err(format!("player AnimBlueprint does not import {name}")), _ => Err(format!("ambiguous AnimBlueprint import {name}: {hits:?}")) }
}

/// `RawCurveData.FloatCurves` of a montage (the montage-level curves UE blends into the AnimInstance curves by the montage weight): (curve name, keys (time, value)).
/// `fn_anim::montage` does not decode them; `Blueprint_Equip` carries `BothHandFK` = 1 (one constant key), `Blueprint_Place` / `Blueprint_Edit` none.
pub fn montage_curves(src: &ClipSourceDir, montage: &str) -> Result<Vec<(String, Vec<(f32, f32)>)>, String> {
    // complete tagged-property decoder (`fn_uasset`, what fn-inspect prints): FSmartName / FRichCurveKey arrays are not decoded by `fn_ueio`
    use fn_uasset::Value;
    let long = pkg_of(montage);
    let (asset, exp) = match src {
        ClipSourceDir::Pak(set) => (set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned(), set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default()),
        ClipSourceDir::RawPackages(root) => { let base = root.join("FortniteGame/Content").join(long.trim_start_matches("/Game/")); (std::fs::read(format!("{}.uasset", base.display())).map_err(|e| e.to_string())?, std::fs::read(format!("{}.uexp", base.display())).unwrap_or_default()) }
        ClipSourceDir::FnAnimDir(_) => return Err("a .fnanim directory has no montages".into()),
    };
    let pkg = fn_uasset::Package::parse(&asset, &exp).map_err(|e| format!("{long}: {e}"))?;
    let name = long.rsplit('/').next().unwrap_or(long);
    let obj = pkg.read_export_named(name).ok_or_else(|| format!("{long}: no {name} export"))?.map_err(|e| format!("{long}: {e}"))?;
    let mut out = vec![];
    let Some(curves) = obj.get("RawCurveData").and_then(|r| r.field("FloatCurves")).and_then(Value::as_array) else { return Ok(out) };
    for c in curves {
        let name = c.field("Name").and_then(|n| n.field("DisplayName")).and_then(Value::as_str).ok_or_else(|| format!("{long}: curve without a DisplayName"))?.to_owned();
        let keys = c.field("FloatCurve").and_then(|f| f.field("Keys")).and_then(Value::as_array).unwrap_or(&[]).iter()
            .map(|k| Some((k.field("Time")?.as_f32()?, k.field("Value")?.as_f32()?))).collect::<Option<Vec<_>>>().ok_or_else(|| format!("{long}: {name}: undecoded key"))?;
        out.push((name, keys));
    }
    Ok(out)
}
/// One `AnimNode_SpringBone` of the paper's AnimBlueprint (`Blueprint_Paper_VIM_C` CDO). Unserialized members keep the UE 4.16 constructor defaults
/// (`FAnimNode_SpringBone::FAnimNode_SpringBone`: stiffness 50, damping 4, ErrorResetThresh 256, translate XYZ on, rotate off, no displacement limit;
/// `FAnimNode_SkeletalControlBase`: Alpha 1, `FInputScaleBias` scale 1 / bias 0).
#[derive(Clone, Debug, PartialEq)]
pub struct SpringBoneNode {
    pub bone: String,
    /// `AlphaScaleBias.ApplyTo(Alpha)` = clamp(Alpha * Scale + Bias, 0, 1) (UE 4.16 `FInputScaleBias::ApplyTo`).
    pub alpha: f32,
    pub stiffness: f32, pub damping: f32, pub limit_displacement: bool, pub max_displacement: f32, pub error_reset: f32,
    pub translate: [bool; 3], pub rotate: [bool; 3],
}
/// The SpringBone chain of an AnimBlueprint generated class (`anim_class` = `/Game/...X.X_C`), in EVALUATION order (the node next to the input pose first).
/// The anim nodes are the CDO's `AnimGraphNode_*` struct properties; a pose link `LinkID` resolves to `AnimNodeProperties[N - 1 - LinkID]` (UE 4.16), which is
/// serialized node `LinkID` (the serialized order is the reverse of that array), starting at `AnimNode_Root.Result`. Only `ConvertComponentToLocalSpace`, `SpringBone` and a `MeshSpaceRefPose` input are accepted (anything else is an error:
/// that graph is not ported).
pub fn spring_bones(src: &ClipSourceDir, anim_class: &str) -> Result<(Vec<SpringBoneNode>, String), String> {
    let long = pkg_of(anim_class);
    let pkg = loader::package(src, long)?;
    let cdo = pkg.exports.iter().position(|e| e.object_name.starts_with("Default__")).ok_or_else(|| format!("{long}: no CDO"))?;
    let props = export_props(&pkg, cdo).map_err(|e| format!("{long}: {e}"))?;
    let nodes: Vec<(&str, &PropList)> = props.0.iter().filter(|p| p.name.starts_with("AnimGraphNode_")).filter_map(|p| match &p.value { PropValue::Struct { name, fields } => Some((name.as_str(), fields)), _ => None }).collect();
    let n = nodes.len() as i64;
    let link = |f: &PropList, key: &str| -> Result<usize, String> {
        let id = f.fields(key).and_then(|l| l.int("LinkID")).ok_or_else(|| format!("{long}: pose link {key} without LinkID"))?;
        // UE 4.16 resolves LinkID as AnimNodeProperties[N - 1 - LinkID]; the tagged properties are serialized in the REVERSE of that array's order
        // (Blueprint_Paper_VIM: SB_9858, SB_88B6, SB_6213, SB_D9C2, C2L, Root, MeshRefPose), so in serialized order the node is index LinkID
        let i = id;
        if !(0..n).contains(&i) { return Err(format!("{long}: LinkID {id} out of range")); }
        Ok(i as usize)
    };
    let root = nodes.iter().position(|(s, _)| *s == "AnimNode_Root").ok_or_else(|| format!("{long}: no AnimNode_Root"))?;
    let mut cur = link(nodes[root].1, "Result")?;
    let (mut chain, mut input) = (vec![], String::new());
    for _ in 0..nodes.len() {
        let (kind, f) = nodes[cur];
        match kind {
            "AnimNode_ConvertComponentToLocalSpace" => cur = link(f, "ComponentPose")?,
            "AnimNode_SpringBone" => {
                let fl = |k: &str, d: f32| f.float(k).map(|v| v as f32).unwrap_or(d);
                let bl = |k: &str, d: bool| f.bool(k).unwrap_or(d);
                let bone = f.fields("SpringBone").and_then(|b| b.text("BoneName")).ok_or_else(|| format!("{long}: SpringBone without BoneName"))?.to_owned();
                let (scale, bias) = f.fields("AlphaScaleBias").map(|s| (s.float("Scale").unwrap_or(1.) as f32, s.float("Bias").unwrap_or(0.) as f32)).unwrap_or((1., 0.));
                chain.push(SpringBoneNode { bone, alpha: (fl("Alpha", 1.) * scale + bias).clamp(0., 1.), stiffness: fl("SpringStiffness", 50.), damping: fl("SpringDamping", 4.),
                    limit_displacement: bl("bLimitDisplacement", false), max_displacement: fl("MaxDisplacement", 0.), error_reset: fl("ErrorResetThresh", 256.),
                    translate: [bl("bTranslateX", true), bl("bTranslateY", true), bl("bTranslateZ", true)], rotate: [bl("bRotateX", false), bl("bRotateY", false), bl("bRotateZ", false)] });
                cur = link(f, "ComponentPose")?;
            }
            "AnimNode_MeshSpaceRefPose" | "AnimNode_RefPose" => { input = kind.to_owned(); break; }
            other => return Err(format!("{long}: anim node {other} is not ported")),
        }
    }
    if input.is_empty() { return Err(format!("{long}: the pose chain does not end in a reference pose")); }
    chain.reverse();
    Ok((chain, input))
}
/// Keys of a `CurveFloat` asset (`FloatCurve.Keys`): (time, value, InterpMode) (InterpMode 0 = RCIM_Linear, 1 Constant, 2 Cubic).
pub fn curve_float(src: &ClipSourceDir, path: &str) -> Result<Vec<(f32, f32, u8)>, String> {
    use fn_uasset::Value;
    let long = pkg_of(path);
    let (asset, exp) = match src {
        ClipSourceDir::Pak(set) => (set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned(), set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default()),
        ClipSourceDir::RawPackages(root) => { let base = root.join("FortniteGame/Content").join(long.trim_start_matches("/Game/")); (std::fs::read(format!("{}.uasset", base.display())).map_err(|e| e.to_string())?, std::fs::read(format!("{}.uexp", base.display())).unwrap_or_default()) }
        ClipSourceDir::FnAnimDir(_) => return Err("a .fnanim directory has no curves".into()),
    };
    let pkg = fn_uasset::Package::parse(&asset, &exp).map_err(|e| format!("{long}: {e}"))?;
    let name = long.rsplit('/').next().unwrap_or(long);
    let obj = pkg.read_export_named(name).ok_or_else(|| format!("{long}: no {name} export"))?.map_err(|e| format!("{long}: {e}"))?;
    let keys = obj.get("FloatCurve").and_then(|f| f.field("Keys")).and_then(Value::as_array).ok_or_else(|| format!("{long}: no FloatCurve keys"))?;
    keys.iter().map(|k| {
        let im = k.field("InterpMode");
        if let Some(b) = im.and_then(Value::as_i64) { return Ok((k.field("Time").and_then(Value::as_f32).ok_or_else(|| format!("{long}: key time"))?, k.field("Value").and_then(Value::as_f32).ok_or_else(|| format!("{long}: key value"))?, b as u8)); }
        let mode = match im.and_then(Value::as_str) { None => 0, Some(s) if s.ends_with("RCIM_Linear") => 0, Some(s) if s.ends_with("RCIM_Constant") => 1, Some(s) if s.ends_with("RCIM_Cubic") => 2, Some(s) => return Err(format!("{long}: key mode {s}")) };
        Ok((k.field("Time").and_then(Value::as_f32).ok_or_else(|| format!("{long}: key time"))?, k.field("Value").and_then(Value::as_f32).ok_or_else(|| format!("{long}: key value"))?, mode))
    }).collect()
}
pub fn load(src: &ClipSourceDir) -> Result<BuildToolSet, String> {
    let (items, _) = crate::loadout_ammo::game_mode_starting_items(src)?;
    let pick = |class: &str| items.iter().find(|s| s.class == class).map(|s| s.item.clone()).ok_or_else(|| format!("Athena StartingItems has no {class}"));
    // the first building piece of StartingItems (every piece names the same WeaponActorClass, checked in the tests)
    let build = tool(src, &pick("FortBuildingItemDefinition")?, "FortBuildingItemDefinition")?;
    let edit = tool(src, &pick("FortEditToolItemDefinition")?, "FortEditToolItemDefinition")?;
    let pawn_chain = loadout_weapon::actor_chain(src, PLAYER_PAWN)?;
    let (place_montage, pawn) = pawn_chain.object("BluePrintPlaceAnimation")?.ok_or("PlayerPawn_Athena chain serializes no BluePrintPlaceAnimation")?;
    let (edit_montage, _) = pawn_chain.object("BluePrintEditAnimation")?.ok_or("PlayerPawn_Athena chain serializes no BluePrintEditAnimation")?;
    let skel = fn_mesh::Skeleton::parse(&loader::package(src, PLAYER_SKELETON)?).map_err(|e| format!("player skeleton: {e}"))?;
    let socket = |n: &str| -> Result<fn_mesh::Socket, String> {
        let mut it = skel.sockets.iter().filter(|s| s.name == n);
        let s = it.next().cloned().ok_or_else(|| format!("player skeleton has no {n} socket"))?;
        if it.next().is_some() { return Err(format!("duplicate {n} socket")); }
        Ok(s)
    };
    if build.paper_socket != edit.paper_socket || build.pencil_socket != edit.pencil_socket { return Err("build / edit tools name different sockets".into()); }
    let (paper_socket, pencil_socket) = (socket(&build.paper_socket)?, socket(&build.pencil_socket)?);
    let abp = loader::package(src, loadout_weapon::PLAYER_ANIM_BP)?;
    let (place_pose, edit_pose) = (animbp_import(&abp, "M_BluePrint_Place")?, animbp_import(&abp, "M_BluePrint_Edit")?);
    Ok(BuildToolSet { build, edit, pawn, place_montage, edit_montage, paper_socket, pencil_socket, place_pose, edit_pose })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn paks() -> fn_pak::PakSet { fn_pak::PakSet::open_install(fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install")).unwrap() }
    #[test]
    fn build_mode_assets_come_from_the_athena_data() {
        let p = paks();
        let src = ClipSourceDir::Pak(&p);
        let s = load(&src).unwrap();
        let b = &s.build; let e = &s.edit;
        assert_eq!(b.item, "/Game/Items/Weapons/BuildingTools/BuildingItemData_Wall");
        assert_eq!(e.item, "/Game/Items/Weapons/BuildingTools/EditTool");
        assert_eq!((b.native.as_str(), e.native.as_str()), ("/Script/FortniteGame.FortWeap_BuildingTool", "/Script/FortniteGame.FortWeap_EditingTool"));
        assert_eq!((b.anims.core.as_str(), e.anims.core.as_str()), ("PaperBlueprint", "PaperBlueprint"));
        assert!(b.anims.core_source.contains("0x140914770") && e.anims.core_source.contains("0x140914890"), "{} / {}", b.anims.core_source, e.anims.core_source);
        for t in [b, e] {
            assert_eq!(t.equip_montage(), Some("/Game/Animation/Game/MainPlayer/Montages/Blueprint/Blueprint_Equip.Blueprint_Equip"));
            assert_eq!(t.weapon_mesh, "/Game/Weapons/FORT_BuildingTools/Meshes/SK_Blueprint_Paper.SK_Blueprint_Paper");
            assert_eq!(t.weapon_anim_class.as_deref(), Some("/Game/Weapons/FORT_BuildingTools/Meshes/Blueprint_Paper_VIM.Blueprint_Paper_VIM_C"));
            assert_eq!(t.pencil_mesh.as_deref(), Some("/Game/Weapons/FORT_BuildingTools/Meshes/Pencil.Pencil"));
            assert_eq!((t.paper_socket.as_str(), t.pencil_socket.as_str()), ("BluePrintPaper", "BluePrintPen"));
        }
        assert_eq!(b.paper_material.as_deref(), Some("/Game/Weapons/FORT_BuildingTools/Materials/ArchitectTools.ArchitectTools"));
        assert!(b.diffuse[0].as_deref().unwrap().ends_with("T_ArchitectTools_D_Wood") && b.diffuse[1].as_deref().unwrap().ends_with("T_ArchitectTools_D_Stone") && b.diffuse[2].as_deref().unwrap().ends_with("T_ArchitectTools_D_Metal") && b.diffuse[3].is_none());
        assert!(b.emissive[2].as_deref().unwrap().ends_with("T_ArchitectTools_E_Metal"));
        assert!(e.paper_material.is_none(), "the editing tool keeps the mesh's own material");
        assert_eq!(s.pawn, "/Game/Athena/PlayerPawn_Athena_Generic");
        assert_eq!(s.place_montage, "/Game/Animation/Game/MainPlayer/Montages/Blueprint/Blueprint_Place.Blueprint_Place");
        assert_eq!(s.edit_montage, "/Game/Animation/Game/MainPlayer/Montages/Blueprint/Blueprint_Edit.Blueprint_Edit");
        assert_eq!((s.paper_socket.bone.as_str(), s.pencil_socket.bone.as_str()), ("weapon_l", "weapon_r"));
        for so in [&s.paper_socket, &s.pencil_socket] { assert_eq!((so.location, so.rotation, so.scale), ([0.; 3], [0.; 3], [1.; 3])); }
        assert_eq!(s.place_pose, "/Game/Animation/Game/MainPlayer/Poses/Medium/Male/M_BluePrint_Place.M_BluePrint_Place");
        assert_eq!(s.edit_pose, "/Game/Animation/Game/MainPlayer/Poses/Medium/Male/M_BluePrint_Edit.M_BluePrint_Edit");
        // CoreAnims_Male[PaperBlueprint] = the generated graph table
        let baked = fn_animgraph::core_anims("PaperBlueprint").unwrap();
        let short = |f: &str| b.anims.core_asset(f).map(|p| pkg_of(p).rsplit('/').next().unwrap().to_owned()).unwrap_or_default();
        assert_eq!(short("TargetingPose"), baked.targeting_pose);
        assert_eq!(short("SprintAnimation"), baked.sprint);
        assert_eq!(short("CrouchTargetingPose"), baked.crouch_targeting_pose);
        // every building piece of StartingItems uses the same tool actor
        let (items, _) = crate::loadout_ammo::game_mode_starting_items(&src).unwrap();
        let pieces: Vec<_> = items.iter().filter(|i| i.class == "FortBuildingItemDefinition").collect();
        assert_eq!(pieces.len(), 4);
        for it in pieces { assert_eq!(loadout_weapon::weapon_anims(&src, &it.item, false).unwrap().actor, b.anims.actor, "{}", it.item); }
        for pk in s.anim_packages() { assert!(p.read_game(&pk, "uasset").is_ok(), "{pk}"); }
    }
    /// `Blueprint_Paper_VIM`: MeshSpaceRefPose -> Spring_1 -> Spring_2 -> Spring_3 -> Spring_4 -> ConvertComponentToLocalSpace -> Root, identical nodes;
    /// the glow pulse curve `HandheldCharacterBlueprintGlowAnimation` (linear 1 -> 7 -> 1 over 0.30 s).
    #[test]
    fn paper_spring_chain_and_pulse_curve() {
        let p = paks();
        let src = ClipSourceDir::Pak(&p);
        let s = load(&src).unwrap();
        let (chain, input) = spring_bones(&src, s.build.weapon_anim_class.as_deref().unwrap()).unwrap();
        assert_eq!(input, "AnimNode_MeshSpaceRefPose");
        assert_eq!(chain.iter().map(|n| n.bone.as_str()).collect::<Vec<_>>(), ["Spring_1", "Spring_2", "Spring_3", "Spring_4"]);
        for n in &chain {
            assert_eq!((n.alpha, n.stiffness, n.damping, n.limit_displacement, n.max_displacement, n.error_reset, n.translate, n.rotate), (0.5, 100., 10., true, 40., 256., [true; 3], [false; 3]));
        }
        let keys = curve_float(&src, s.build.pulse_curve.as_deref().unwrap()).unwrap();
        assert!(s.build.pulse_curve.as_deref().unwrap().ends_with("HandheldCharacterBlueprintGlowAnimation"));
        assert_eq!(keys.len(), 3);
        for ((t, v, m), (et, ev)) in keys.iter().zip([(0.00153, 1.), (0.15, 7.), (0.30, 1.)]) { assert!((t - et).abs() < 1e-4 && (v - ev).abs() < 1e-5 && *m == 0, "{keys:?}"); }
    }
    /// The montage numbers the runtime plays (read back from the packages, pinned here).
    #[test]
    fn blueprint_montages_and_animbp_node() {
        let p = paks();
        let src = ClipSourceDir::Pak(&p);
        let s = load(&src).unwrap();
        let m = |path: &str| fn_anim::montage::AnimMontage::parse(&loader::package(&src, pkg_of(path)).unwrap()).unwrap();
        let eq = m(s.build.equip_montage().unwrap());
        assert!((eq.length - 0.533333).abs() < 1e-4 && (eq.blend_in.time - 0.667).abs() < 1e-4 && (eq.blend_out.time - 0.2).abs() < 1e-4);
        assert_eq!(eq.slots[0].slot_name, "UpperBody");
        assert_eq!(montage_curves(&src, s.build.equip_montage().unwrap()).unwrap(), vec![("BothHandFK".to_owned(), vec![(0., 1.)])]);
        assert!(montage_curves(&src, &s.place_montage).unwrap().is_empty());
        for path in [&s.place_montage, &s.edit_montage] {
            let x = m(path);
            assert!((x.length - 0.533333).abs() < 1e-4 && (x.blend_in.time - 0.09).abs() < 1e-4 && (x.blend_out.time - 0.09).abs() < 1e-4);
            assert_eq!(x.slots[0].slot_name, "UpperBody");
            assert!(x.slots[0].segments[0].animation_path.as_deref().unwrap().ends_with("BluePrint_Edit_ConfirmCheck"));
        }
        // AnimBP node 6: BlendListByEnum<BuildingState>, EnumToPoseIndex [1,2,0], BlendTime 0.2 x3, Linear (the graph's BUILDING_STATE_BLEND)
        let abp = loader::package(&src, loadout_weapon::PLAYER_ANIM_BP).unwrap();
        let cdo = export_props(&abp, abp.exports.iter().position(|e| e.object_name.starts_with("Default__")).unwrap()).unwrap();
        let node = cdo.fields("AnimGraphNode_BlendListByEnum_0835E37E4BE8CE8DD6C449A998E91383").expect("node 6");
        let src_prop = node.fields("EvaluateGraphExposedInputs").and_then(|e| e.array("CopyRecords")).and_then(|a| a.first()).and_then(|r| r.as_fields()).and_then(|r| r.text("SourcePropertyName")).unwrap();
        assert_eq!(src_prop, "BuildingState");
        let ints: Vec<i64> = node.array("EnumToPoseIndex").unwrap().iter().map(|v| match v { PropValue::Int(i) => *i, PropValue::Byte(b) => *b as i64, _ => -1 }).collect();
        assert_eq!(&ints[..3], &[1, 2, 0]);
        let times: Vec<f64> = node.array("BlendTime").unwrap().iter().filter_map(|v| v.as_f64()).collect();
        assert_eq!(times.len(), 3);
        for t in times { assert!((t as f32 - fn_animgraph::BUILDING_STATE_BLEND.dur).abs() < 1e-6); }
        assert_eq!(node.text("BlendType"), Some("EAlphaBlendOption::Linear"));
    }
}
