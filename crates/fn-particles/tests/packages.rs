//! Module parsing against the real 1.8 packages, spawn counts and lifetimes of the simulator. Needs the paks (`FN_PAKS`, `FN_ROOT`, or the developer's install) and the
//! `import` feature; without them every test prints a note and passes.
#![cfg(feature = "import")]
use fn_particles::model::*;
use fn_particles::{Env, Instance, Transform};
use std::path::PathBuf;
use std::sync::Arc;

const PINE: &str = "/Game/Effects/Fort_Effects/World/Destruction/Trees/P_Pine_Tree_Med_Destruction";
const IMPACT: &str = "/Game/Weapons/FORT_Melee/Effects/Impacts/P_Melee_Impact_Default";
const OAK: &str = "/Game/Effects/Fort_Effects/World/Destruction/Trees/P_OakTree_Large_Destruction";

fn paks() -> Option<fn_pak::PakSet> {
    let cands = [std::env::var_os("FN_PAKS").map(PathBuf::from), std::env::var_os("FN_ROOT").map(PathBuf::from), Some(PathBuf::from(r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame"))];
    for c in cands.into_iter().flatten() { if let Ok(s) = fn_pak::PakSet::open_install(&c) { return Some(s); } }
    eprintln!("no Fortnite 1.8 paks: skipped"); None
}
fn load(set: &fn_pak::PakSet, p: &str) -> Arc<ParticleSystem> { Arc::new(fn_particles::import::import_system(set, p).unwrap_or_else(|e| panic!("{p}: {e}"))) }
fn lod0<'a>(s: &'a ParticleSystem, emitter: &str) -> &'a Lod { &s.emitters.iter().find(|e| e.name == emitter).unwrap_or_else(|| panic!("no emitter {emitter}")).lods[0] }

#[test] fn pine_destruction_decodes_exactly_what_the_package_says() {
    let Some(set) = paks() else { return };
    let s = load(&set, PINE);
    let names: Vec<&str> = s.emitters.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Smoke", "Wood", "Needles", "LeafGroup", "Branch", "ChunksFast", "Flash"]);
    assert_eq!(s.lod_distances, [0., 400., 800., 7500.]);
    assert_eq!(s.emitters.iter().map(|e| e.lods.len()).collect::<Vec<_>>(), [4; 7]);
    // bursts: (count, count_low) of LOD 0 straight from ParticleModuleSpawn.BurstList
    let bursts: Vec<(i32, i32)> = s.emitters.iter().map(|e| { let b = &e.lods[0].spawn.bursts; assert_eq!(b.len(), 1, "{}", e.name); assert_eq!(b[0].time, 0.0); (b[0].count, b[0].count_low) }).collect();
    assert_eq!(bursts, [(5, 4), (6, -1), (5, 4), (6, 3), (4, 3), (32, 16), (1, -1)]);
    // required module
    let smoke = lod0(&s, "Smoke");
    assert!(smoke.required.material.ends_with("M_Smoke_Destruction_01_Inst.M_Smoke_Destruction_01_Inst"));
    assert_eq!((smoke.required.sub_images_h, smoke.required.sub_images_v), (8, 8));
    assert_eq!(smoke.required.interpolation, SubUvInterp::LinearBlend);
    assert_eq!(smoke.required.sort_mode, SortMode::ViewProjDepth);
    assert!(smoke.required.use_local_space && matches!(smoke.type_data, TypeData::Sprite));
    assert!(smoke.required.cutout_texture.as_deref().unwrap().ends_with("T_SmokePoof_8x8_01.T_SmokePoof_8x8_01"));
    // type data
    assert!(matches!(&lod0(&s, "Wood").type_data, TypeData::Mesh { mesh, .. } if mesh.ends_with("S_Wood_Chip_01.S_Wood_Chip_01")));
    let TypeData::Gpu(g) = &lod0(&s, "ChunksFast").type_data else { panic!("ChunksFast is a GPU sprite emitter") };
    assert_eq!((g.max_particle_count, g.collision, g.constant_acceleration), (33, true, [0., 0., -900.]));
    assert!(g.collision_mode.ends_with("DistanceField"));
    // modules of the ChunksFast LOD 0: the collision module is the GPU one, with its resilience bias 0.2 baked in the cooked data
    assert!(lod0(&s, "ChunksFast").modules.iter().any(|m| matches!(m.kind, ModuleKind::CollisionGpu { .. })));
    // the mesh emitters swap their material through the MeshMaterial module
    assert!(lod0(&s, "Wood").modules.iter().any(|m| matches!(&m.kind, ModuleKind::MeshMaterial { materials } if materials[0].ends_with("M_Destruct_PrimaryColor_Wood_Inst.M_Destruct_PrimaryColor_Wood_Inst"))));
    // lifetime of the GPU chunks: uniform 2..3 s (ParticleModuleLifetime_26)
    let life = lod0(&s, "ChunksFast").modules.iter().find_map(|m| if let ModuleKind::Lifetime { lifetime } = &m.kind { Some(lifetime.range_at(0., 1)) } else { None }).unwrap();
    assert_eq!((life.0[0], life.1[0]), (2.0, 3.0));
    assert!(s.unsupported().is_empty(), "{:?}", s.unsupported());
}

#[test] fn every_system_the_props_and_the_pickaxe_reference_decodes_and_only_the_oak_vector_fields_are_left_out() {
    let Some(set) = paks() else { return };
    let props = PathBuf::from(r"C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\assets\props");
    let mut paths: Vec<String> = vec![IMPACT.to_string()];
    for e in std::fs::read_dir(&props).into_iter().flatten().flatten() {
        let Ok(t) = std::fs::read_to_string(e.path().join("props.json")) else { continue };
        let j: serde_json::Value = serde_json::from_str(&t).unwrap();
        if let Some(p) = j["destruction"]["death_particles"].as_str().filter(|s| !s.is_empty()) { paths.push(p.split('.').next().unwrap().to_string()); }
    }
    paths.sort(); paths.dedup();
    if paths.len() < 10 { eprintln!("props not exported here: only the pickaxe impact checked"); }
    let mut unsupported = vec![];
    for p in &paths {
        let s = load(&set, p);
        assert!(!s.emitters.is_empty(), "{p}");
        for e in &s.emitters { assert!(!e.lods.is_empty(), "{p}/{}: no LOD levels", e.name); }
        assert!(s.emitters.iter().any(|e| e.lods[0].enabled), "{p}: every emitter is disabled at LOD 0");
        for (c, n) in s.unsupported() { unsupported.push((s.name.clone(), c, n)); }
    }
    for (s, c, _) in &unsupported { assert_eq!(s, "P_OakTree_Large_Destruction", "{c}"); assert!(c.starts_with("ParticleModuleVectorField"), "{c}"); }
}

#[test] fn spawn_counts_follow_the_bursts_and_stay_inside_count_low_count() {
    let Some(set) = paks() else { return };
    let s = load(&set, PINE);
    let mut totals = vec![];
    for seed in 0..40u64 {
        let mut inst = Instance::new(s.clone(), Transform::identity(), seed, 100.0);
        inst.run(0.5, 1.0 / 60.0, &Env::none()); // the Wood emitter has an EmitterDelay
        let per: Vec<u64> = inst.emitters.iter().map(|e| e.spawned_total).collect();
        assert!((4..=5).contains(&per[0]) && per[1] == 6 && (4..=5).contains(&per[2]) && (3..=6).contains(&per[3]) && (3..=4).contains(&per[4]) && (16..=32).contains(&per[5]) && per[6] == 1, "{per:?}");
        totals.push(inst.spawned_total());
    }
    assert!(totals.iter().min() != totals.iter().max(), "CountLow makes the totals vary with the seed");
    // same seed -> identical run
    let run = |seed| { let mut i = Instance::new(s.clone(), Transform::identity(), seed, 100.0); i.run(2.0, 1.0 / 60.0, &Env::none()); (i.spawned_total(), i.live_count(), i.emitters.iter().map(|e| e.particles.iter().map(|p| p.loc.x + p.loc.y * 3. + p.loc.z * 7.).sum::<f32>()).collect::<Vec<_>>()) };
    assert_eq!(run(5), run(5)); assert_ne!(run(5).2, run(6).2);
}

#[test] fn lifetimes_match_the_lifetime_modules_and_the_system_finishes() {
    let Some(set) = paks() else { return };
    let s = load(&set, PINE);
    let env = Env::none();
    // the GPU chunks live 2..3 s: none alive at 3.1 s, all alive at 1.9 s (they spawn at t = 0 within the first 1/60 s)
    let mut inst = Instance::new(s.clone(), Transform::identity(), 3, 100.0);
    let chunks = 5;
    inst.run(1.9, 1.0 / 60.0, &env); assert_eq!(inst.emitters[chunks].particles.len() as u64, inst.emitters[chunks].spawned_total, "no chunk dies before 2 s");
    inst.run(1.3, 1.0 / 60.0, &env); assert!(inst.emitters[chunks].particles.is_empty(), "every chunk is gone after 3 s");
    // the Flash mesh lives 0.15 s
    let mut f = Instance::new(s.clone(), Transform::identity(), 3, 100.0);
    f.run(0.10, 1.0 / 60.0, &env); assert_eq!(f.emitters[6].particles.len(), 1);
    f.run(0.12, 1.0 / 60.0, &env); assert_eq!(f.emitters[6].particles.len(), 0);
    // everything ends: the longest emitter lifetime bounds the effect
    let mut all = Instance::new(s, Transform::identity(), 3, 100.0);
    all.run(8.0, 1.0 / 60.0, &env); assert!(all.finished() && all.live_count() == 0);
}

#[test] fn the_pickaxe_impact_is_a_short_burst_of_smoke_and_a_flash() {
    let Some(set) = paks() else { return };
    let s = load(&set, IMPACT);
    assert_eq!(s.emitters.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Smoke", "Flash"]);
    let mut inst = Instance::new(s.clone(), Transform::identity(), 9, 100.0);
    inst.tick(1.0 / 60.0, &Env::none());
    assert!((2..=3).contains(&inst.emitters[0].spawned_total) && inst.emitters[1].spawned_total == 1);
    inst.run(4.0, 1.0 / 60.0, &Env::none()); assert!(inst.finished());
    // LOD by distance: 6 m -> LOD 1 of the 3-level system
    assert_eq!(s.lod_for_distance(100.0), 0); assert_eq!(s.lod_for_distance(700.0), 1); assert_eq!(s.lod_for_distance(5000.0), 2);
}

#[test] fn the_chunks_collide_with_the_ground_and_stop_bouncing_up() {
    let Some(set) = paks() else { return };
    let s = load(&set, PINE);
    let ground = |_: f32, _: f32| 0.0f32; let env = Env { ground: Some(&ground) };
    let mut inst = Instance::new(s, Transform::identity(), 11, 100.0);
    inst.run(1.2, 1.0 / 60.0, &env);
    // GPU chunks: spawned in a sphere around the origin with -900 cm/s^2 gravity, resilience 0.2: after 1.2 s none is below the ground
    for p in &inst.emitters[5].particles { assert!(p.loc.z >= -1.0, "chunk below the ground: {:?}", p.loc); }
    assert!(inst.emitters[5].particles.iter().any(|p| p.collisions > 0 || p.stopped || p.loc.z < 30.0), "at least one chunk reached the ground");
}

#[test] fn the_oak_reports_its_vector_field_modules_instead_of_pretending() {
    let Some(set) = paks() else { return };
    let s = load(&set, OAK);
    let u = s.unsupported(); assert!(u.iter().any(|(c, n)| c == "ParticleModuleVectorFieldLocal" && *n == 6), "{u:?}");
    // the system still simulates (the vector field only perturbs the velocity)
    let mut inst = Instance::new(s, Transform::identity(), 2, 100.0);
    inst.run(1.0, 1.0 / 60.0, &Env::none()); assert!(inst.spawned_total() > 20);
}
