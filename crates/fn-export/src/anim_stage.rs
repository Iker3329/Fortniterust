//! Animation stage: manifest `animations[]` -> `.fnanim` (lossless F32) at the manifest `out` path with the extension
//! replaced; with `--md5` also the old `.md5anim` text; with `--verify <old assets>` the generated md5 text is compared
//! byte-for-byte against the old UEViewer file and the `.fnanim` is read back and compared with the sequence.
use crate::source::Source;
use crate::{entry, parallel, write_file, Args, Stage};
use fn_anim::{AnimSequence, Precision, SkeletonInfo};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

type SkelCache = Mutex<HashMap<String, Result<Arc<SkeletonInfo>, String>>>;

fn skeleton_for(src: &Source, pkg: &fn_ueio::Package, cache: &SkelCache) -> Result<Arc<SkeletonInfo>, String> {
    let idx = pkg.imports.iter().position(|i| i.class_name == "Skeleton").ok_or("animation has no Skeleton import")?;
    let long = pkg.import_path(-(idx as i32) - 1);
    let pkg_path = long.split('.').next().unwrap_or(&long).to_string();
    if let Some(r) = cache.lock().unwrap().get(&pkg_path) {
        return r.clone();
    }
    let r = (|| {
        let sp = src.package(&pkg_path)?;
        SkeletonInfo::from_package(&sp).map(Arc::new).map_err(|e| format!("{pkg_path}: {e}"))
    })();
    cache.lock().unwrap().insert(pkg_path, r.clone());
    r
}

fn export_one(src: &Source, cache: &SkelCache, pkg_name: &str, out: &str, args: &Args) -> Result<Value, String> {
    let pkg = src.package(pkg_name)?;
    let skel = skeleton_for(src, &pkg, cache)?;
    let seq = AnimSequence::parse(&pkg).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    seq.write_fnanim(&skel, Precision::F32, &mut bytes);
    let fnanim_rel = std::path::Path::new(out).with_extension("fnanim");
    write_file(&args.out.join(&fnanim_rel), &bytes)?;
    let mut d = json!({"frames": seq.num_frames, "bones": skel.num_bones(), "fnanim_bytes": bytes.len()});
    // round trip
    let back = AnimSequence::read_fnanim(&bytes).map_err(|e| e.to_string())?;
    // what a consumer substitutes for channels the file has no data for: reference pose, or identity for additive clips
    let additive_defaults;
    let defaults: &[fn_anim::BoneTransform] = if seq.is_additive() {
        additive_defaults = vec![fn_anim::BoneTransform::ADDITIVE_IDENTITY; skel.num_bones()];
        &additive_defaults
    } else {
        &skel.ref_pose
    };
    // compare at an integer frame (the .fnanim stores one key per frame)
    let frame = (seq.num_frames as f32 * 0.37).floor().min(seq.num_frames.saturating_sub(1) as f32);
    let (a, b) = (seq.pose_at_frame(frame, &skel), back.pose_at_frame(frame, defaults));
    let mut max_pos = 0f32;
    let mut worst = (0usize, [0f32; 3], [0f32; 3]);
    for (bi, (x, y)) in a.iter().zip(&b).enumerate() {
        for k in 0..3 {
            let e = (x.translation[k] - y.translation[k]).abs();
            if e > max_pos {
                max_pos = e;
                worst = (bi, x.translation, y.translation);
            }
        }
    }
    d["worst_bone"] = json!([skel.bone_names[worst.0], worst.1, worst.2]);
    d["fnanim_roundtrip_max_pos_err_cm"] = json!(max_pos);
    if args.md5 || args.verify.is_some() {
        let mut text = Vec::new();
        fn_anim::md5::write_md5anim(&seq, &skel, &mut text).map_err(|e| e.to_string())?;
        if args.md5 {
            write_file(&args.out.join(out), &text)?;
        }
        if let Some(old) = &args.verify {
            match std::fs::read(old.join(out)) {
                Ok(o) => d["md5_identical_to_old"] = json!(o == text),
                Err(e) => d["md5_identical_to_old"] = json!(format!("old file unreadable: {e}")),
            }
        }
    }
    Ok(d)
}

pub fn run(src: &Source, manifest: &Value, args: &Args) -> Stage {
    let mut items: Vec<(String, String)> = manifest["animations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|a| (a["pkg"].as_str().unwrap_or("").to_string(), a["out"].as_str().unwrap_or("").to_string()))
        .collect();
    if let Some(f) = &args.only {
        items.retain(|(p, _)| p.to_lowercase().contains(f.as_str()));
    }
    println!("animations: {} clips", items.len());
    let cache: SkelCache = Mutex::new(HashMap::new());
    let results = parallel(&items, args.jobs, |(p, o)| {
        let t = Instant::now();
        let r = export_one(src, &cache, p, o, args);
        (r, t.elapsed().as_millis())
    });
    let (mut entries, mut failures, mut identical, mut different, mut total_bytes) = (vec![], 0, 0, 0, 0u64);
    for ((p, o), (r, ms)) in items.iter().zip(results) {
        match &r {
            Err(_) => failures += 1,
            Ok(d) => {
                total_bytes += d["fnanim_bytes"].as_u64().unwrap_or(0);
                match d.get("md5_identical_to_old") {
                    Some(Value::Bool(true)) => identical += 1,
                    Some(_) => different += 1,
                    None => {}
                }
            }
        }
        entries.push(entry("animation", p, &std::path::Path::new(o).with_extension("fnanim").to_string_lossy(), &r, ms));
    }
    if args.verify.is_some() {
        println!("animation verify: md5 text byte-identical to old export {identical}, different/unreadable {different}");
    }
    println!("animations: {} ok, {failures} failed, {:.2} MB of .fnanim", items.len() - failures, total_bytes as f64 / 1e6);
    Stage { entries, failures }
}
