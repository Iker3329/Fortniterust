//! The `EPhysicalSurface` of a hit on a building prop (tree, rock, car, ...), the index into the weapon's / pickaxe's `ImpactPhysicalSurfaceEffects` and
//! `ImpactPhysicalSurfaceSounds`.
//!
//! SOURCES
//!  * EXACT (exe) `UFortKismetLibrary::GetFortPhysicalSurface` body RVA 0x1405c4930: the surface of the hit's `PhysMaterial` (`FHitResult` +0x5c, `UPhysicalMaterial::SurfaceType`
//!    +0x50) when it is not `SurfaceType_Default` (0); otherwise the hit actor's `IFortDamageableActorInterface` getter (vtable +0x10).
//!  * EXACT (exe) that getter for `ABuildingSMActor`, RVA 0x140477250 (bytes read from the 1.8 exe, interface `this` = actor + 0x3e0): `PrimarySurfaceType` (+0x598) when
//!    non-zero, else by `ResourceType` (+0x77c): Wood 0 -> 1, Stone 1 -> 2, Metal 2 -> 3, 3 -> 3, anything else (None 4) -> 0.
//!  * EXACT (UE4.16 `FBodyInstance::GetSimplePhysicalMaterial`, BodyInstance.cpp L3495-3560; `UMaterialInstance::GetPhysicalMaterial` MaterialInstance.cpp L1406;
//!    `UMaterial::GetPhysicalMaterial` Material.cpp L2050) the hit's physical material on simple collision: the component's `BodyInstance.PhysMaterialOverride`, else the
//!    mesh `BodySetup::PhysMaterial`, else material 0's (`OverrideMaterials[0]`, else the mesh's `StaticMaterials[0]`) `PhysMaterial` walking `Parent` up to the root
//!    `UMaterial`, else the engine default (SurfaceType_Default).
//!  * EXACT (data) e.g. `MI_StoneLarge_01.PhysMaterial` = `/Game/FortressPhysicalMaterials/Stone` (`SurfaceType2`); the tree materials chain to
//!    `FortniteFoliage_MasterMaterial` with no physical material, so a tree hit falls through to its `ResourceType` (Wood -> 1).
//!  * INFERENCE: the hit is on the prop's simple collision (the 1.8 melee / weapon target filters serialize `bTraceComplexCollision` false); component-template properties
//!    are inherited child-first along the Blueprint chain like the CDO ones.
use crate::data::{cdo_chain, pkg_of};
use fn_pose::{loader, ClipSourceDir};
use fn_ueio::{props::{read_props, Ctx, PropList, PropValue}, Package, Reader};
use std::sync::Mutex;

/// `EPhysicalSurface` of an enum text (`SurfaceType2`, `EPhysicalSurface::SurfaceType2`, `SurfaceType_Default` = 0).
pub fn surface_enum(s: &str) -> u32 {
    let t = s.rsplit("::").next().unwrap_or(s);
    t.strip_prefix("SurfaceType").and_then(|n| n.parse().ok()).unwrap_or(0)
}
/// `ABuildingSMActor`'s damageable-interface surface (RVA 0x140477250): `PrimarySurfaceType` if non-zero, else the `EFortResourceType` map.
pub fn actor_surface(primary_surface_type: u32, resource_type: Option<u8>) -> u32 {
    if primary_surface_type != 0 { return primary_surface_type; }
    match resource_type { Some(0) => 1, Some(1) => 2, Some(2) | Some(3) => 3, _ => 0 }
}
/// `GetFortPhysicalSurface` (RVA 0x1405c4930): the physical material's surface unless Default, else the actor's.
pub fn hit_surface(phys_material_surface: u32, actor: u32) -> u32 { if phys_material_surface != 0 { phys_material_surface } else { actor } }
/// `EFortResourceType` value of an enum text (`EFortResourceType::Stone` -> 1).
pub fn resource_enum(s: &str) -> Option<u8> {
    match s.rsplit("::").next().unwrap_or(s) { "Wood" => Some(0), "Stone" => Some(1), "Metal" => Some(2), "Permanite" => Some(3), "None" => Some(4), _ => None }
}

/// What the cooked data say about one prop class.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PropSurface {
    /// The physical material a simple-collision hit reports (`None` = the engine default) and where it came from.
    pub phys_material: Option<String>, pub phys_material_from: &'static str,
    /// Its `SurfaceType` (0 when absent / default).
    pub phys_surface: u32,
    /// `PrimarySurfaceType` of the class chain (0 when not serialized) and `ResourceType` (`None` = not serialized: the native default, `EFortResourceType::None` 4).
    pub primary_surface_type: u32, pub resource_type: Option<u8>,
    /// The final `EPhysicalSurface` ([`hit_surface`] of the two).
    pub surface: u32,
}

fn props_of(pkg: &Package, i: usize) -> Option<PropList> {
    let (s, e) = pkg.export_range(i);
    read_props(&mut Reader::at(&pkg.data[..e], s), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).ok()
}
fn import(pkg: &Package, v: Option<&PropValue>) -> Option<String> { match v { Some(PropValue::Object(i)) if *i < 0 => Some(pkg.import_path(*i)), _ => None } }
/// The main export of an asset package (`/Game/X/Foo` -> export `Foo`) with its class and properties.
fn asset(src: &ClipSourceDir, path: &str) -> Option<(Package, usize)> {
    let p = pkg_of(path);
    let pkg = loader::package(src, p).ok()?;
    let name = p.rsplit('/').next().unwrap_or(p);
    let i = pkg.exports.iter().position(|e| e.object_name == name)?;
    Some((pkg, i))
}

/// `UMaterialInterface::GetPhysicalMaterial` of `material`: the first `PhysMaterial` along the `Parent` chain (`None` = default).
pub fn material_phys_material(src: &ClipSourceDir, material: &str) -> Option<String> {
    let mut cur = material.to_owned();
    for _ in 0..16 {
        let (pkg, i) = asset(src, &cur)?;
        let p = props_of(&pkg, i)?;
        if let Some(pm) = import(&pkg, p.get("PhysMaterial")) { return Some(pm); }
        cur = import(&pkg, p.get("Parent"))?; // a UMaterial has no Parent: the engine default
    }
    None
}
/// `UPhysicalMaterial::SurfaceType` of a physical material asset (0 when not serialized).
pub fn phys_material_surface(src: &ClipSourceDir, phys: &str) -> u32 {
    asset(src, phys).and_then(|(pkg, i)| props_of(&pkg, i)).and_then(|p| p.text("SurfaceType").map(surface_enum)).unwrap_or(0)
}

/// Resolves the surface of a hit on a placed `blueprint` (`/Game/...` class package); `mesh` is the static mesh when the class chain does not name one.
pub fn resolve(src: &ClipSourceDir, blueprint: &str, mesh: Option<&str>) -> Result<PropSurface, String> {
    let chain = cdo_chain(src, blueprint)?;
    let mut out = PropSurface {
        primary_surface_type: chain.iter().find_map(|(_, p)| p.text("PrimarySurfaceType")).map_or(0, surface_enum),
        resource_type: chain.iter().find_map(|(_, p)| p.text("ResourceType")).and_then(resource_enum),
        ..Default::default()
    };
    // the StaticMeshComponent template of each class in the chain (child first)
    let comps: Vec<(&Package, PropList)> = chain.iter().filter_map(|(pkg, p)| { let i = p.object("StaticMeshComponent").filter(|i| *i > 0)?; Some((pkg, props_of(pkg, i as usize - 1)?)) }).collect();
    let body_override = comps.iter().find_map(|(pkg, c)| import(pkg, c.fields("BodyInstance").and_then(|b| b.get("PhysMaterialOverride"))));
    let override_mat = comps.iter().find_map(|(pkg, c)| c.array("OverrideMaterials").and_then(|a| import(pkg, a.first())));
    let mesh = comps.iter().find_map(|(pkg, c)| import(pkg, c.get("StaticMesh")))
        .or_else(|| chain.iter().find_map(|(pkg, p)| import(pkg, p.get("StaticMesh"))))
        .or_else(|| mesh.map(str::to_owned));
    let (mut body_setup_pm, mut mesh_mat) = (None, None);
    if let Some(m) = &mesh { if let Ok(pkg) = loader::package(src, pkg_of(m)) {
        for i in 0..pkg.exports.len() {
            let class = pkg.export_class(i);
            if class == "BodySetup" { if let Some(p) = props_of(&pkg, i) { body_setup_pm = body_setup_pm.or_else(|| import(&pkg, p.get("PhysMaterial"))); } }
        }
        // `StaticMaterials[0].MaterialInterface`: the cooked mesh keeps the material references in the native tail (`fn_mesh::static_mesh`), not in the tagged props
        if let Ok(sm) = fn_mesh::static_mesh::StaticMesh::parse(&pkg) { mesh_mat = sm.materials.first().and_then(|m| m.material.clone()); }
    } }
    let material = override_mat.or(mesh_mat);
    (out.phys_material, out.phys_material_from) = if let Some(pm) = body_override { (Some(pm), "component BodyInstance.PhysMaterialOverride") }
        else if let Some(pm) = body_setup_pm { (Some(pm), "mesh BodySetup.PhysMaterial") }
        else if let Some(pm) = material.as_deref().and_then(|m| material_phys_material(src, m)) { (Some(pm), "material 0 PhysMaterial chain") }
        else { (None, "engine default (SurfaceType_Default)") };
    out.phys_surface = out.phys_material.as_deref().map_or(0, |p| phys_material_surface(src, p));
    // ResourceType not serialized anywhere in the chain = the native default, None (4): `ABuildingSMActor` constructor RVA 0x14045b960 stores 4 at +0x77c
    out.surface = hit_surface(out.phys_surface, actor_surface(out.primary_surface_type, Some(out.resource_type.unwrap_or(4))));
    Ok(out)
}

/// Per prop kind (`fn_combat::Harvestable::proto`) surfaces resolved by the app at load time; `None` until set.
static PROTO_SURFACES: Mutex<Option<Vec<Option<u32>>>> = Mutex::new(None);
pub fn set_proto_surfaces(v: Vec<Option<u32>>) { *PROTO_SURFACES.lock().unwrap() = Some(v); }
/// The surface of a hit on harvestable `p`: the resolved class surface of its proto, else (placeholders, unresolved kinds) the `ResourceType` rule of
/// [`actor_surface`] with no `PrimarySurfaceType` (`crate::shoot::surface_of`).
pub fn of_prop(p: &fn_combat::Harvestable) -> u32 {
    PROTO_SURFACES.lock().ok().and_then(|g| g.as_ref().and_then(|v| v.get(p.proto as usize).copied().flatten())).unwrap_or_else(|| crate::shoot::surface_of(p.res))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_matches_the_exe() {
        assert_eq!((surface_enum("SurfaceType2"), surface_enum("EPhysicalSurface::SurfaceType12"), surface_enum("SurfaceType_Default")), (2, 12, 0));
        assert_eq!([0u8, 1, 2, 3, 4].map(|r| actor_surface(0, Some(r))), [1, 2, 3, 3, 0]);
        assert_eq!(actor_surface(7, Some(1)), 7);
        assert_eq!((hit_surface(2, 1), hit_surface(0, 1)), (2, 1));
    }
    #[test]
    fn tree_rock_and_car_surfaces_from_the_install() {
        let Some(root) = fn_live::var_os("FN_TEST_ROOT") else { eprintln!("FN_TEST_ROOT not set, skipping"); return };
        let paks = fn_pak::PakSet::open_install(&root).unwrap();
        let src = ClipSourceDir::Pak(&paks);
        let tree = resolve(&src, "/Game/Athena/Environments/Blueprints/Athena_Tree_Large_1", None).unwrap();
        assert_eq!((tree.phys_material.as_deref(), tree.resource_type, tree.surface), (None, Some(0), 1), "{tree:?}");
        let rock = resolve(&src, "/Game/Athena/Environments/Blueprints/Athena_Prop_Rocks_StoneLarge01", None).unwrap();
        assert_eq!((rock.phys_material.as_deref(), rock.phys_surface, rock.surface), (Some("/Game/FortressPhysicalMaterials/Stone.Stone"), 2, 2), "{rock:?}");
        let car = resolve(&src, "/Game/Building/ActorBlueprints/Prop/Car_Pickup", None).unwrap();
        assert_eq!((car.phys_surface, car.resource_type, car.surface), (0, Some(2), 3), "Car_Pickup: default physical material, Metal (Car_Copper) -> 3: {car:?}");
    }
}
