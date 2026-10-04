//! Offline decoder audit for original starting-loadout assets. Not gameplay parity.
use fn_anim::{AnimSequence, SkeletonInfo};
use fn_pose::{loader, ClipSourceDir};
use serde_json::{json, Value};
use std::{collections::HashMap, path::Path};

fn package(reference: &str) -> &str { reference.split('.').next().unwrap_or(reference) }
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 { return Err("fn-loadout-audit <FortniteGame> <loadout-animations.json> <private-evidence.json> <out.json>".into()); }
    let paks = fn_pak::PakSet::open_install(&args[1])?;
    let source = ClipSourceDir::Pak(&paks);
    let manifest: Value = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let catalog: Value = serde_json::from_slice(&std::fs::read(&args[3])?)?;
    let mut skeletons: HashMap<String, SkeletonInfo> = HashMap::new();
    let mut animations = vec![];
    for value in manifest["sequencePackages"].as_array().ok_or("missing sequence packages")? {
        let path = value.as_str().ok_or("invalid sequence path")?;
        let pkg = loader::package(&source, path)?;
        let sequence = AnimSequence::parse(&pkg)?;
        let skeleton_path = package(sequence.skeleton_path.as_deref().ok_or("sequence missing skeleton")?).to_owned();
        if !skeletons.contains_key(&skeleton_path) {
            let pkg = loader::package(&source, &skeleton_path)?;
            skeletons.insert(skeleton_path.clone(), SkeletonInfo::from_package(&pkg)?);
        }
        let skeleton = &skeletons[&skeleton_path];
        if sequence.num_frames == 0 || skeleton.num_bones() == 0 { return Err(format!("empty animation {path}").into()); }
        // Decode every frame against THAT sequence's skeleton (weapon skeletons
        // are not substituted with the player's). Do not claim playback parity.
        for frame in 0..sequence.num_frames {
            let pose = sequence.sample_frame(frame, skeleton);
            for bone in &pose {
                if bone.translation.iter().chain(bone.rotation.iter()).chain(bone.scale.iter()).any(|v| !v.is_finite()) {
                    return Err(format!("non-finite decoded pose {path} frame {frame}").into());
                }
            }
        }
        animations.push(json!({"package":path,"skeleton":skeleton_path,"frames":sequence.num_frames,"length":sequence.length,"rateScale":sequence.rate_scale,"bones":skeleton.num_bones(),"tracks":sequence.tracks.len(),"allFramesFinite":true}));
        println!("decoded {} frames: {path}", sequence.num_frames);
    }
    let mut meshes = vec![];
    for item in ["WID_Assault_Auto_Athena_C_Ore_T02", "WID_Shotgun_Standard_Athena_C_Ore_T03", "WID_Sniper_BoltAction_Scope_Athena_R_Ore_T03"] {
        let definition = catalog["definitions"].as_array().ok_or("missing definitions")?.iter().find(|d| d["item"].as_str().is_some_and(|id| id.ends_with(item))).ok_or("missing requested firearm")?;
        let path = package(definition["mesh"].as_str().ok_or("missing mesh reference")?);
        let pkg = loader::package(&source, path)?;
        let mesh = fn_mesh::SkeletalMesh::parse(&pkg)?;
        let part = fn_pose::Part::from_skeletal_mesh(&mesh.name, &mesh, 0)?;
        let vertices: usize = part.prims.iter().map(|p| p.positions.len()).sum();
        if vertices == 0 { return Err(format!("empty mesh {path}").into()); }
        meshes.push(json!({"package":path,"bones":mesh.ref_skeleton.len(),"lod0Vertices":vertices,"materials":mesh.materials.len(),"skeleton":mesh.skeleton_path}));
        println!("decoded mesh {path}: {vertices} vertices");
    }
    let report = json!({"result":"decoder audit passed","warning":"Original package decoder coverage only, not original-runtime trace comparison, complete animation scheduling, rendering or gameplay parity. Locomotion/ADS graphs remain separate work.","sequences":animations,"meshes":meshes});
    if let Some(parent) = Path::new(&args[4]).parent() { std::fs::create_dir_all(parent)?; }
    std::fs::write(&args[4], serde_json::to_vec_pretty(&report)?)?;
    Ok(())
}
fn main() { if let Err(error) = run() { eprintln!("loadout audit failed: {error}"); std::process::exit(1); } }
