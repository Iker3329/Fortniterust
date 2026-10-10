//! `fn-particles-export`: decodes the Cascade particle systems the harvestable props and the pickaxe impact reference from the 1.8 paks into `<out>` (default `assets/particles`).
//!
//! ```text
//! fn-particles-export --paks <Paks dir> --props <assets/props dir> [--out assets/particles] [--describe] [--sim] [extra /Game/... particle system packages]
//! ```
//! `--props` supplies the `DeathParticles` of every exported prop (`props.json` -> `destruction.death_particles`); the pickaxe impact effects are read from the CDO chain of
//! `B_Melee_Impact_Pickaxe_Athena` (`ImpactPhysicalSurfaceEffects`, child element first, parents fill the rest). `--describe` prints LOD 0 of every emitter, `--sim` runs each system for 3 s and prints the counts.
use fn_uasset::PackageData;
use std::path::PathBuf;
use std::sync::Arc;

const PICKAXE_IMPACT_BP: &str = "/Game/Weapons/FORT_Melee/Blueprints/Impact/B_Melee_Impact_Pickaxe_Athena";

fn main() {
    let mut paks = PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame/Content/Paks");
    let mut out = PathBuf::from("assets/particles");
    let mut props: Option<PathBuf> = None;
    let (mut describe, mut sim) = (false, false);
    let mut dump: Option<String> = None;
    let mut extra: Vec<String> = vec![];
    let mut materials: Vec<String> = vec![];
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--paks" => paks = PathBuf::from(it.next().expect("--paks needs a value")),
            "--out" => out = PathBuf::from(it.next().expect("--out needs a value")),
            "--props" => props = it.next().map(PathBuf::from),
            "--describe" => describe = true,
            "--sim" => sim = true,
            "--dump" => dump = it.next(),
            "--material" => materials.push(it.next().expect("--material needs a value")),
            other => extra.push(other.to_string()),
        }
    }
    let set = match fn_pak::PakSet::open_install(&paks) { Ok(s) => s, Err(e) => { eprintln!("cannot open the paks at {}: {e}", paks.display()); std::process::exit(2) } };
    if let Some(p) = &dump {
        match fn_particles::import::dump_package(&set, p) { Ok(t) => println!("{t}"), Err(e) => { eprintln!("{e}"); std::process::exit(1) } }
        return;
    }
    let mut packages: Vec<String> = extra;
    if let Some(dir) = &props {
        let mut found = 0;
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let Ok(text) = std::fs::read_to_string(e.path().join("props.json")) else { continue };
            let Ok(j) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
            if let Some(p) = j["destruction"]["death_particles"].as_str().filter(|s| !s.is_empty()) { packages.push(p.split('.').next().unwrap().to_string()); found += 1; }
        }
        println!("{found} props with DeathParticles in {}", dir.display());
    }
    // pickaxe impact effects. `ImpactPhysicalSurfaceEffects` is a fixed array indexed by EPhysicalSurface (AFortWeapon::PlayNativeImpactFX helper 0x92dc60 reads
    // `this[surface + 0xdb]`); a Blueprint CDO serialises only the elements that differ from its parent, so the class chain is walked child first
    // (B_Melee_Impact_Pickaxe_Athena -> B_Melee_Impact_Generic -> B_Melee_Generic; the parent supplies Wood [1] / Stone [2] / Metal [3]).
    let mut impact = serde_json::Map::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut cur = PICKAXE_IMPACT_BP.to_string();
    for _ in 0..16 {
        let Ok(data) = PackageData::load(&set, &cur) else { break };
        let Ok(pkg) = data.package() else { break };
        if let Some(Ok(cdo)) = pkg.read_default_object() {
            for p in cdo.props.iter().filter(|p| p.name == "ImpactPhysicalSurfaceEffects") {
                if !seen.insert(p.index) { continue; } // the child's element (even None) hides the parent's
                if let Some(i) = p.value.as_object() { let path = pkg.object_path(i); if !path.is_empty() { impact.insert(p.index.to_string(), serde_json::json!(path)); packages.push(path.split('.').next().unwrap().to_string()); } }
            }
        }
        // the Blueprint parent: the class export's super (a /Script/ native parent ends the chain)
        let cls = pkg.default_object_export().map(|i| pkg.exports[i].name.trim_start_matches("Default__").to_string());
        let sup = cls.and_then(|c| pkg.exports.iter().find(|e| e.name == c)).map(|e| pkg.object_path(e.super_index)).unwrap_or_default();
        let next = sup.split('.').next().unwrap_or("").to_string();
        if !next.starts_with("/Game/") || next == cur { break; }
        cur = next;
    }
    packages.sort(); packages.dedup();
    println!("{} particle systems", packages.len());
    if describe || sim {
        for p in &packages {
            match fn_particles::import::import_system(&set, p) {
                Ok(s) => {
                    if describe { print!("{}", s.describe()); }
                    if sim {
                        let mut inst = fn_particles::Instance::new(Arc::new(s), fn_particles::Transform::identity(), 1, 100.0);
                        let env = fn_particles::Env::none();
                        inst.run(3.0, 1.0 / 60.0, &env);
                        println!("  sim 3s: spawned {} live {} finished {}", inst.spawned_total(), inst.live_count(), inst.finished());
                        if describe { for e in &inst.emitters { println!("    {}: spawned {} dropped {} bursts {} completed {}", inst.system.emitters[e.index].name, e.spawned_total, e.dropped, e.bursts_fired, e.completed); } }
                    }
                }
                Err(e) => println!("{p}: ERROR {e}"),
            }
        }
        return;
    }
    match fn_particles::import::export_all_ex(&set, &packages, &materials, &out) {
        Ok(rep) => {
            println!("exported {} systems, {} meshes, {} materials, {} textures to {}", rep.systems.len(), rep.meshes, rep.materials, rep.textures, out.display());
            for (s, c, n) in &rep.unsupported { println!("UNSUPPORTED in {s}: {c} x{n}"); }
            for f in &rep.failures { println!("FAILED {f}"); }
            let mut idx: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(out.join("index.json")).unwrap()).unwrap();
            idx["pickaxe_impact"] = serde_json::json!({ "blueprint": PICKAXE_IMPACT_BP, "surface_effects": impact });
            std::fs::write(out.join("index.json"), serde_json::to_string_pretty(&idx).unwrap()).unwrap();
            if !rep.failures.is_empty() { std::process::exit(1); }
        }
        Err(e) => { eprintln!("{e}"); std::process::exit(1) }
    }
}
