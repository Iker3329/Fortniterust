//! Compare parsed BlendSpace assets with the old runtime's report (reports/animation-blendspaces.json).
//! usage: bs_check <testdata root> <old animation-blendspaces.json>
use fn_anim::BlendSpace;
use fn_ueio::Package;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(&args[1]);
    let report: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&args[2]).unwrap()).unwrap();
    let mut bad = 0;
    for b in report["blendSpaces"].as_array().unwrap() {
        let src = b["source"].as_str().unwrap().trim_start_matches("../../../");
        let base = root.join(src).with_extension("");
        let rd = |e: &str| std::fs::read(base.with_extension(e)).unwrap_or_default();
        let pkg = Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap();
        let bs = BlendSpace::parse(&pkg).unwrap();
        bs.validate().unwrap();
        let mut diffs: Vec<String> = Vec::new();
        let name = b["name"].as_str().unwrap();
        if bs.name != name {
            diffs.push(format!("name {} vs {}", bs.name, name));
        }
        if format!("{:?}", bs.kind) != b["className"].as_str().unwrap() {
            diffs.push(format!("class {:?} vs {}", bs.kind, b["className"]));
        }
        let axes = b["axes"].as_array().unwrap();
        for (i, a) in bs.axes.iter().enumerate() {
            let o = &axes[i];
            let f = |k: &str| o[k].as_f64().unwrap() as f32;
            if (a.min, a.max, a.grid_divisions as f32) != (f("min"), f("max"), f("grid")) || (a.interp_time - f("interpTime")).abs() > 1e-6 || a.interp_type != o["interpType"].as_str().unwrap() {
                diffs.push(format!("axis {i}: {a:?} vs {o}"));
            }
        }
        if (bs.target_weight_interpolation_speed_per_sec as f64 - b["targetWeightInterpolationSpeedPerSec"].as_f64().unwrap()).abs() > 1e-6 {
            diffs.push("targetWeight".into());
        }
        if bs.axis_to_scale_animation != b["axisToScaleAnimation"].as_str().unwrap() {
            diffs.push("axisToScale".into());
        }
        if bs.sample_index_with_markers as i64 != b["sampleIndexWithMarkers"].as_i64().unwrap() {
            diffs.push("sampleIndexWithMarkers".into());
        }
        if bs.rotation_blend_in_mesh_space != b["bRotationBlendInMeshSpace"].as_bool().unwrap() {
            diffs.push("meshSpace".into());
        }
        let nt = b["notifyTriggerMode"].as_str().map(|s| s.rsplit("::").next().unwrap().to_string());
        if bs.notify_trigger_mode != nt {
            diffs.push(format!("notifyTriggerMode {:?} vs {:?}", bs.notify_trigger_mode, nt));
        }
        let samples = b["samples"].as_array().unwrap();
        if samples.len() != bs.samples.len() {
            diffs.push("sample count".into());
        } else {
            for (s, o) in bs.samples.iter().zip(samples) {
                let p = o["pos"].as_array().unwrap();
                if s.animation.as_deref() != o["anim"].as_str() || (s.position[0] as f64 - p[0].as_f64().unwrap()).abs() > 1e-4 || (s.position[1] as f64 - p[1].as_f64().unwrap()).abs() > 1e-4 || (s.rate_scale as f64 - o["rate"].as_f64().unwrap()).abs() > 1e-6 {
                    diffs.push(format!("sample {s:?} vs {o}"));
                }
            }
        }
        let grid = b["grid"].as_array().unwrap();
        if grid.len() != bs.grid.len() {
            diffs.push(format!("grid count {} vs {}", bs.grid.len(), grid.len()));
        } else {
            for (g, o) in bs.grid.iter().zip(grid) {
                let (idx, w) = (o["idx"].as_array().unwrap(), o["w"].as_array().unwrap());
                for k in 0..3 {
                    let ow = w[k].as_f64().unwrap() as f32;
                    if (g.weights[k] - ow).abs() > 1e-6 || (ow > 0.0 && g.indices[k] as i64 != idx[k].as_i64().unwrap()) {
                        diffs.push(format!("grid element {g:?} vs {o}"));
                        break;
                    }
                }
            }
        }
        // weights of every sample position must be 1.0 on that sample's position
        let mut pos_errs = 0;
        for (i, s) in bs.samples.iter().enumerate() {
            let w = bs.sample_weights(s.position[0], s.position[1]);
            let own: f32 = w.iter().filter(|(j, _)| bs.samples[*j].position == s.position).map(|x| x.1).sum();
            if (own - 1.0).abs() > 1e-4 {
                pos_errs += 1;
                diffs.push(format!("sample {i} at {:?} gets weights {w:?}", s.position));
            }
        }
        println!(
            "{:22} {:40} axes {} samples {:2} grid {:2} twis {} -> {}{}",
            format!("{:?}", bs.kind),
            bs.name,
            bs.axes.len(),
            bs.samples.len(),
            bs.grid.len(),
            bs.target_weight_interpolation_speed_per_sec,
            if diffs.is_empty() { "MATCH".to_string() } else { format!("{} DIFFS", diffs.len()) },
            if pos_errs > 0 { format!(" ({pos_errs} samples not reproduced at their own position)") } else { String::new() }
        );
        for d in diffs.iter().take(3) {
            println!("    {d}");
        }
        bad += diffs.len();
    }
    println!("total diffs: {bad}");
}
