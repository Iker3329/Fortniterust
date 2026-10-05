//! `fn-vfx-export`: reads the muzzle flash data of every BR firearm from the 1.8 paks.
//!
//! ```text
//! fn-vfx-export [--paks <Paks dir>] [--out <dir>] [--list]
//! ```
//! For every `/Game/Athena/Items/Weapons/WID_*` ranged weapon: `WeaponActorClass` -> the Blueprint chain (`B_*_Athena` -> ... -> `B_Ranged_Generic`) -> the CDO properties
//! `MuzzleParticleSystem`, `MuzzleSocketName`, `bPersistentFireFX` (first Blueprint in the chain that sets them), every other particle reference of those CDOs, the `*Light*`
//! numbers, and the muzzle socket of the weapon mesh. Then the referenced particle systems (with their materials / textures / meshes) are decoded by `fn-particles` into
//! `<out>` (`index.json`, `systems/`, ...) next to `<out>/muzzle.json`.
use fn_uasset::{Package, PackageData, PackageIndex, Value};
use fn_vfx::table::{MuzzleTable, SocketInfo, WeaponMuzzle};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const WID_DIR: &str = "FortniteGame/Content/Athena/Items/Weapons/";

fn pkg_of(path: &str) -> String { path.split('.').next().unwrap_or(path).to_string() }
fn strip_class(p: &str) -> String { pkg_of(p) }

/// Hard object reference, or a soft object / soft class path string (`WeaponActorClass`, `WeaponMeshOverride` are soft).
fn object_path(pkg: &Package, v: &Value) -> Option<String> {
    if let Some(i) = v.as_object() { let p = pkg.object_path(i); return if p.is_empty() { None } else { Some(p) }; }
    v.as_str().filter(|s| !s.is_empty()).map(String::from)
}

struct Cdo { props: Vec<(String, Value)>, pkg_particles: BTreeMap<String, String>, super_path: Option<String> }

/// CDO properties of a Blueprint package (+ its resolved super class package, `None` for native supers).
fn load_cdo(set: &fn_pak::PakSet, long: &str) -> Result<Cdo, String> {
    let data = PackageData::load(set, long).map_err(|e| format!("{long}: {e}"))?;
    let pkg = data.package().map_err(|e| format!("{long}: {e}"))?;
    let super_path = pkg.super_class().map(|r| r.path().to_string()).filter(|p| p.starts_with("/Game/")).map(|p| pkg_of(&p));
    let mut props = vec![];
    let mut particles = BTreeMap::new();
    if let Some(Ok(cdo)) = pkg.read_default_object() {
        for p in cdo.props.iter() {
            let key = if p.index > 0 { format!("{}[{}]", p.name, p.index) } else { p.name.clone() };
            // particle system references (class of the referenced object = ParticleSystem)
            if let Value::Object(i) = &p.value {
                if let ObjectClass::Particle = object_class(&pkg, *i) { if let Some(path) = object_path(&pkg, &p.value) { particles.insert(key.clone(), pkg_of(&path)); } }
            }
            props.push((key, p.value.clone()));
        }
    }
    Ok(Cdo { props, pkg_particles: particles, super_path })
}
enum ObjectClass { Particle, Other }
fn object_class(pkg: &Package, i: PackageIndex) -> ObjectClass {
    match pkg.resolve(i) { fn_uasset::ObjectRef::Import { class_name, .. } if class_name == "ParticleSystem" => ObjectClass::Particle, _ => ObjectClass::Other }
}

fn find_socket(set: &fn_pak::PakSet, mesh_long: &str, socket: &str) -> Option<SocketInfo> {
    let load = |long: &str| -> Option<fn_ueio::Package> {
        let ua = set.read_game(long, "uasset").ok()?.into_owned();
        let ue = set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default();
        fn_ueio::Package::parse(&ua, &ue, &[]).ok()
    };
    let pkg = load(mesh_long)?;
    let sm = fn_mesh::SkeletalMesh::parse(&pkg).ok()?;
    let conv = |s: &fn_mesh::skeleton::Socket, owner: &str| SocketInfo { name: s.name.clone(), bone: s.bone.clone(), location: s.location, rotation: s.rotation, scale: s.scale, owner: owner.into() };
    if let Some(s) = sm.sockets.iter().find(|s| s.name == socket) { return Some(conv(s, "mesh")); }
    let sp = sm.skeleton_path.as_deref()?;
    let spkg = load(sp.split('.').next()?)?;
    let sk = fn_mesh::Skeleton::parse(&spkg).ok()?;
    sk.sockets.iter().find(|s| s.name == socket).map(|s| conv(s, "skeleton"))
}

fn main() {
    let mut paks = PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame/Content/Paks");
    let mut out = PathBuf::from("assets/particles-muzzle");
    let mut list = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--paks" => paks = PathBuf::from(it.next().expect("--paks needs a value")),
            "--out" => out = PathBuf::from(it.next().expect("--out needs a value")),
            "--list" => list = true,
            other => { eprintln!("unknown argument {other}"); std::process::exit(2) }
        }
    }
    let set = match fn_pak::PakSet::open_install(&paks) { Ok(s) => s, Err(e) => { eprintln!("cannot open the paks at {}: {e}", paks.display()); std::process::exit(2) } };
    let mut wids: Vec<String> = set.glob(&format!("{WID_DIR}WID_*.uasset")).into_iter().filter_map(|(_, e)| fn_pak::pak_path_to_game(e.rel_path())).collect();
    wids.sort(); wids.dedup();
    let mut table = MuzzleTable { schema: 1, notes: vec![
        "muzzle_system = MuzzleParticleSystem of the first Blueprint in the chain that sets it (CDO); the Muzzle(Empty) component of B_Ranged_Generic is attached to the weapon mesh socket MuzzleSocketName and uses it as its template (UserConstructionScript)".into(),
        "muzzle_socket falls back to `Muzzle` when no Blueprint sets MuzzleSocketName: INFERENCE (the native default lives in the exe); `socket` is the real socket of the mesh / skeleton with that name".into(),
    ], weapons: BTreeMap::new() };
    let mut systems: BTreeSet<String> = BTreeSet::new();
    for wid in wids {
        let name = wid.rsplit('/').next().unwrap_or("").to_string();
        if name.contains("Harvest") { continue; }
        let Ok(data) = PackageData::load(&set, &wid) else { continue };
        let Ok(pkg) = data.package() else { continue };
        let Some(Ok(def)) = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i).ends_with("ItemDefinition")).map(|i| pkg.read_export(i)) else { continue };
        if !pkg.export_class_name(0).contains("WeaponRanged") { continue; }
        let Some(actor) = def.get("WeaponActorClass").and_then(|v| object_path(&pkg, v)) else { eprintln!("{name}: no WeaponActorClass"); continue };
        let mesh = def.get("WeaponMeshOverride").and_then(|v| object_path(&pkg, v)).map(|p| pkg_of(&p)).unwrap_or_default();
        let mut w = WeaponMuzzle { wid: wid.clone(), actor_class: strip_class(&actor), mesh: mesh.clone(), muzzle_socket: "Muzzle".into(), ..Default::default() };
        let mut cur = Some(strip_class(&actor)); let mut seen_persistent = false;
        while let Some(bp) = cur.take() {
            if w.chain.len() > 12 { break; }
            let cdo = match load_cdo(&set, &bp) { Ok(c) => c, Err(e) => { eprintln!("{name}: {e}"); break } };
            w.chain.push(bp.clone());
            for (k, v) in &cdo.props {
                match k.as_str() {
                    "MuzzleParticleSystem" if w.muzzle_system.is_none() => { if let Some(p) = cdo.pkg_particles.get(k) { w.muzzle_system = Some(p.clone()); } }
                    "MuzzleSocketName" if !w.socket_from_data => { if let Some(s) = v.as_str() { w.muzzle_socket = s.to_string(); w.socket_from_data = true; } }
                    "bPersistentFireFX" if !seen_persistent => { if let Some(b) = v.as_bool() { w.persistent_fire = b; seen_persistent = true; } }
                    _ => {}
                }
                if k.contains("Light") { if let Some(f) = v.as_f32() { w.light_properties.entry(k.clone()).or_insert(f); } }
            }
            for (k, p) in &cdo.pkg_particles { if k != "MuzzleParticleSystem" && !k.contains("ImpactPhysicalSurface") { w.other_particle_refs.entry(k.clone()).or_insert_with(|| p.clone()); } }
            cur = cdo.super_path;
        }
        if let Some(s) = &w.muzzle_system { systems.insert(s.clone()); }
        // beam / shells / reload / destroy effects are listed in the table but not exported; the muzzle variants (`MuzzleFlash(Charged)`, `MuzzleWindParticleSystem`, ...) are
        for (k, p) in &w.other_particle_refs { if k.contains("Muzzle") { systems.insert(p.clone()); } }
        if !mesh.is_empty() { w.socket = find_socket(&set, &mesh, &w.muzzle_socket); }
        table.weapons.insert(wid, w);
    }
    // report
    for w in table.weapons.values() {
        println!("{:<46} {:<36} {:<62} socket {:<10}{} persistent {:<5} {}", w.wid.rsplit('/').next().unwrap_or(""), w.actor_class.rsplit('/').next().unwrap_or(""), w.muzzle_system.as_deref().map_or("-", |s| s.rsplit('/').next().unwrap_or("")), w.muzzle_socket, if w.socket_from_data { "" } else { "*" }, w.persistent_fire,
            match &w.socket { Some(s) => format!("[{} on bone {} at {:?} rot {:?}]", s.owner, s.bone, s.location, s.rotation), None => "[socket NOT FOUND]".into() });
        for (k, p) in &w.other_particle_refs { println!("      {k}: {p}"); }
        for (k, v) in &w.light_properties { println!("      {k} = {v}"); }
    }
    println!("{} weapons, {} particle systems; `*` = MuzzleSocketName not set by any Blueprint (default `Muzzle` inferred)", table.weapons.len(), systems.len());
    if list { return; }
    let packages: Vec<String> = systems.into_iter().collect();
    match fn_particles::import::export_all(&set, &packages, &out) {
        Ok(rep) => {
            println!("exported {} systems, {} meshes, {} materials, {} textures to {}", rep.systems.len(), rep.meshes, rep.materials, rep.textures, out.display());
            for (s, c, n) in &rep.unsupported { println!("UNSUPPORTED in {s}: {c} x{n}"); }
            for f in &rep.failures { println!("FAILED {f}"); }
        }
        Err(e) => { eprintln!("{e}"); std::process::exit(1) }
    }
    std::fs::create_dir_all(&out).ok();
    std::fs::write(out.join("muzzle.json"), serde_json::to_string_pretty(&table).expect("table json")).expect("write muzzle.json");
    println!("wrote {}", out.join("muzzle.json").display());
}
