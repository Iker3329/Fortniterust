//! `fn-pose-dump`: dumps the evaluated pose (bone locals, mesh matrices, skin matrices, sockets) of one frame of a scripted scenario as JSON.
//!
//!   fn-pose-dump <scenario> <frame> [options]
//!     scenario    pickaxe-test | anim-test | dir-test | face-test | jump-air-test:<0..7>   (frames are the old logs' 0-based loop indices)
//!     --clips     md5 (old exports via fn-legacy, default if the old tree is found) | packages (raw extracted packages) | fnanim DIR | pak DIR
//!     --meshes    gltf (old exports, default with --clips md5) | packages | pak (default otherwise)
//!     --assets    old tree `assets` directory (default: fn-testdata's)         --data  raw package root (default $FN_TESTDATA / Temp\fn-testdata)
//!     --mode      ue (exact UE sampling, the DEFAULT for every clip source) | legacy (emulation of the OLD wrong md5 playback, explicit comparisons only)
//!     --legacy-additive   pre-face-fix mesh-space additive          --part NAME   only that part (body | head | master)
//!     --compact   skip the per-bone listing (matrices only)         --out FILE    write there instead of stdout
//!     --bench N   time N full evaluations (all parts) of that frame instead of dumping
use fn_animgraph::ClipLibrary;
use fn_pose::clip::{load_player_skeleton_info, ClipSourceDir, ClipStore, SampleMode};
use fn_pose::loader::{self, AssetIndex};
use fn_pose::scenario::{self, Scenario};
use fn_pose::{CharacterRig, EvalOptions};
use serde_json::{json, Value};
use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()) }
fn flag(args: &[String], name: &str) -> bool { args.iter().any(|a| a == name) }
fn die(msg: impl std::fmt::Display) -> ! { eprintln!("fn-pose-dump: {msg}"); std::process::exit(2) }

fn mat(m: &glam::Mat4) -> Value { json!(m.to_cols_array()) }

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 2 || flag(&args, "--help") { eprintln!("usage: fn-pose-dump <scenario> <frame> [--clips md5|packages|fnanim DIR|pak DIR] [--mode ue|legacy] [--part body|head|master] [--out FILE] [--bench N]  (see source header)"); std::process::exit(2); }
    let Some(scn) = Scenario::parse(&args[0]) else { die(format!("unknown scenario {}", args[0])) };
    let want: u32 = args[1].parse().unwrap_or_else(|_| die("frame must be a number"));
    let clips_kind = arg(&args, "--clips").unwrap_or_else(|| "md5".into());
    let mode = match arg(&args, "--mode").as_deref() { Some("legacy") => SampleMode::LegacyMd5, Some("ue") => SampleMode::Ue, None => SampleMode::Ue, Some(o) => die(format!("bad --mode {o}")) };
    let assets = arg(&args, "--assets").map(PathBuf::from).unwrap_or_else(fn_testdata_assets);
    let data = arg(&args, "--data").map(PathBuf::from).or_else(loader::default_package_root);

    // clips
    let pak_set;
    let src: Option<ClipSourceDir> = match clips_kind.as_str() {
        "packages" => Some(ClipSourceDir::RawPackages(Box::leak(data.clone().unwrap_or_else(|| die("no raw package root (--data / FN_TESTDATA)")).into_boxed_path()))),
        "pak" => { pak_set = fn_pak::PakSet::open_install(arg(&args, "--pak").unwrap_or_else(|| die("--clips pak needs --pak DIR"))).unwrap_or_else(|e| die(e)); Some(ClipSourceDir::Pak(&pak_set)) }
        _ => None,
    };
    // the clip names the scenario needs: the graph's required clips plus the montage clips
    let names = fn_legacy::prototype_clip_names();
    let library: Box<dyn ClipLibrary> = if clips_kind == "md5" {
        let all = fn_legacy::files(&assets);
        Box::new(fn_legacy::load_clip_set(&all, &names).unwrap_or_else(|e| die(e)))
    } else {
        let src = src.as_ref().unwrap();
        let skel = load_player_skeleton_info(src).unwrap_or_else(|e| die(e));
        Box::new(ClipStore::load(src, &names, &skel, mode).unwrap_or_else(|e| die(e)))
    };
    // meshes
    let meshes = arg(&args, "--meshes").unwrap_or_else(|| if clips_kind == "md5" { "gltf".into() } else { "packages".into() });
    let mut parts = if meshes == "gltf" { loader::old_tree_parts(&AssetIndex::scan(&assets)).unwrap_or_else(|e| die(e)) } else {
        let s = match &src { Some(s) => s, None => &ClipSourceDir::RawPackages(Box::leak(data.clone().unwrap_or_else(|| die("no package root")).into_boxed_path())) };
        loader::package_parts(s).unwrap_or_else(|e| die(e))
    };
    if let Some(only) = arg(&args, "--part") { let name = match only.as_str() { "body" => 0, "head" => 1, "master" => 2, o => die(format!("bad --part {o}")) }; parts = vec![parts.swap_remove(name)]; }
    let mut rig = CharacterRig::new(parts);
    rig.opts = EvalOptions { legacy_mesh_additive: flag(&args, "--legacy-additive") };

    let mut dumped: Option<Value> = None;
    let mut bench_tree = None;
    scenario::run(scn, Some(want + 1), &mut |ctx| {
        if ctx.frame != want { return; }
        bench_tree = Some(ctx.tree.clone());
        let pose = rig.evaluate(&ctx.tree, library.as_ref());
        let compact = flag(&args, "--compact");
        let parts: Vec<Value> = rig.parts.iter().zip(&pose.parts).map(|(p, r)| {
            let bones: Vec<Value> = if compact { vec![] } else { (0..p.tree.len()).map(|i| json!({
                "i": i, "name": p.tree.names[i], "parent": p.tree.parents[i], "local_t": r.bone_local[i].0.to_array(), "local_q": r.bone_local[i].1.to_array(), "mesh": mat(&r.bone_mesh[i]) })).collect() };
            json!({ "name": p.name, "bone_count": p.tree.len(), "palette_len": p.palette_len(), "bones": bones,
                "joints": p.joints, "skin_matrices": r.skin_matrices.iter().map(mat).collect::<Vec<_>>() })
        }).collect();
        dumped = Some(json!({
            "scenario": scn.name(), "frame": ctx.frame, "clips": clips_kind, "sample_mode": format!("{mode:?}"), "legacy_additive": rig.opts.legacy_mesh_additive,
            "graph": ctx.anim.describe(), "capture": ctx.capture, "ik_stats_fk_gap_r_l_residual_r_l_m": pose.ik_stats, "missing_clips": pose.missing_clips,
            "sockets": pose.sockets.iter().map(|(n, m)| json!({ "name": n, "matrix": mat(m) })).collect::<Vec<_>>(), "parts": parts }));
    });
    let Some(v) = dumped else { die(format!("scenario {} has no frame {want}", scn.name())) };
    if let Some(n) = arg(&args, "--bench").and_then(|s| s.parse::<usize>().ok()) {
        let tree = bench_tree.unwrap();
        let mut out = fn_pose::CharacterPose::default();
        for _ in 0..20 { rig.evaluate_into(&tree, library.as_ref(), &mut out); }
        let mut times = vec![];
        for _ in 0..n { let t = std::time::Instant::now(); rig.evaluate_into(&tree, library.as_ref(), &mut out); times.push(t.elapsed().as_secs_f64() * 1e3); }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mean = times.iter().sum::<f64>() / n as f64;
        println!("{} frame {want}: {} parts, {n} evaluations: mean {mean:.4} ms, median {:.4} ms, min {:.4} ms, p99 {:.4} ms", scn.name(), rig.parts.len(), times[n / 2], times[0], times[(n * 99 / 100).min(n - 1)]);
        return;
    }
    let text = serde_json::to_string_pretty(&v).unwrap();
    match arg(&args, "--out") { Some(p) => std::fs::write(&p, text).unwrap_or_else(|e| die(format!("{p}: {e}"))), None => println!("{text}") }
}

fn fn_testdata_assets() -> PathBuf {
    if let Some(p) = std::env::var_os("FN_OLD_ROOT") { return PathBuf::from(p).join("assets"); }
    let here = std::fs::canonicalize(env!("CARGO_MANIFEST_DIR")).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    here.ancestors().nth(3).map(|p| p.join("FortniteRuntime").join("assets")).unwrap_or_else(|| PathBuf::from("assets"))
}
