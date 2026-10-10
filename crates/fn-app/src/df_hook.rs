//! DISTANCE FIELD HOOK (runtime half): reads the COOKED mesh distance fields of the real props from the 1.8 paks (`FDistanceFieldVolumeData` of LOD 0,
//! decoded by `fn_mesh`) and hands them to the renderer (`fn_render::distance_field`). Runs once, after the scene with the real props exists.
//!
//! Per placed component (UE4.16 `FStaticMeshSceneProxy::SupportsDistanceFieldRepresentation` / `AffectsDistanceFieldLighting`):
//! * `CastShadow` (catalog `render.cast_shadow`) and `bAffectDistanceFieldLighting` (UPrimitiveComponent default true; a serialized `false` on the
//!   prop's StaticMeshComponent template is honoured);
//! * the 1.8 exe's `ABuildingActor::PostInitializeComponents` (0x140487b00) clears `bAffectDistanceFieldLighting` (bit 6 of +0x2a9) on every
//!   primitive of a building actor whose `bIsMovable` or (client) `bIsDynamic` is set (+0x554 bits 28 / 26); the ctor leaves both clear, so only a
//!   serialized `true` on the prop's class default object turns the distance field off.
use crate::scene::Scene;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

static DONE: AtomicBool = AtomicBool::new(false);

pub fn pending() -> bool { !DONE.load(Ordering::Relaxed) }

pub fn frame(scene: &mut Scene, install: Option<PathBuf>, roots: &[PathBuf]) {
    if DONE.swap(true, Ordering::Relaxed) { return; }
    let Some(real) = scene.real.as_ref() else { return };
    let Some(dir) = install else { eprintln!("[df] no install / paks: distance-field shadows and AO stay off"); return };
    let Some(cat) = crate::props::discover(roots) else { return };
    let set = match fn_pak::PakSet::open_install(&dir) { Ok(s) => s, Err(e) => { eprintln!("[df] paks: {e}"); return } };
    let t0 = std::time::Instant::now();
    let (mut ok, mut skipped) = (0usize, vec![]);
    let jobs: Vec<(usize, fn_render::StaticMeshHandle)> = real.meshes.iter().enumerate().filter_map(|(i, m)| m.map(|h| (i, h))).collect();
    let protos: Vec<String> = real.lib.protos.iter().map(|p| p.id.clone()).collect();
    for (pi, handle) in jobs {
        let Some(kind) = cat.kinds.iter().find(|k| k.id == protos[pi]) else { continue };
        match load_one(&set, kind) {
            Ok(df) => { scene.renderer.set_static_mesh_distance_field(handle, df); ok += 1; }
            Err(e) => skipped.push(format!("{}: {e}", kind.id)),
        }
    }
    eprintln!("[df] {ok} cooked mesh distance fields loaded in {:.0} ms{}", t0.elapsed().as_secs_f32() * 1000., if skipped.is_empty() { String::new() } else { format!("; skipped: {}", skipped.join("; ")) });
}

fn load_one(set: &fn_pak::PakSet, kind: &fn_propdata::PropKind) -> Result<fn_render::distance_field::MeshDistanceField, String> {
    let long = kind.mesh_ue_path.split('.').next().unwrap_or("");
    let rd = |ext: &str| set.read_game(long, ext).map(|c| c.into_owned());
    let a = rd("uasset").map_err(|e| e.to_string())?;
    let x = rd("uexp").unwrap_or_default();
    let b = rd("ubulk").unwrap_or_default();
    let pkg = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| e.to_string())?;
    let mesh = fn_mesh::StaticMesh::parse(&pkg).map_err(|e| e.to_string())?;
    let d = mesh.distance_fields.first().cloned().flatten().ok_or("no cooked distance field for LOD 0")?;
    let (raw, fmt) = d.texels().ok_or("empty / undecodable distance field volume")?;
    let texels = match fmt {
        fn_mesh::static_mesh::DistanceFieldFormat::G8 => raw,
        fn_mesh::static_mesh::DistanceFieldFormat::R16F => return Err("PF_R16F distance field (the 1.8 cook is PF_G8)".into()),
    };
    let v = &kind.raw;
    let flag = |ptr: &str| v.pointer(ptr).and_then(|x| x.as_bool());
    let affects = kind.render.cast_shadow
        && flag("/ue_properties/static_mesh_component/bAffectDistanceFieldLighting").unwrap_or(true)
        && flag("/ue_properties/static_mesh_component/CastShadow").unwrap_or(true)
        && !flag("/ue_properties/class_default_object/bIsMovable").unwrap_or(false)
        && !flag("/ue_properties/class_default_object/bIsDynamic").unwrap_or(false);
    Ok(fn_render::distance_field::MeshDistanceField {
        size: [d.size[0].max(0) as u32, d.size[1].max(0) as u32, d.size[2].max(0) as u32],
        texels,
        box_min: d.local_box_min,
        box_max: d.local_box_max,
        distance_min_max: d.distance_min_max,
        built_as_if_two_sided: d.built_as_if_two_sided,
        mesh_was_plane: d.mesh_was_plane,
        self_shadow_bias: fn_render::distance_field::ue::DEFAULT_SELF_SHADOW_BIAS,
        affects_distance_field_lighting: affects,
    })
}
