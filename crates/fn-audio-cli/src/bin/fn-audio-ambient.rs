//! `fn-audio-ambient <out.json> [--root <FortniteGame>]`: scan Athena_Terrain (+ sublevels) for the ambient system and write the layout file
//! that `fn_audio::ambient` embeds (`crates/fn-audio/data/ambient_layout.json`). Read-only on the install.
use fn_pak::PakSet;
use std::{env, fs, process};

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut root = env::var("FN_ROOT").unwrap_or_else(|_| DEFAULT_ROOT.to_string());
    if let Some(i) = args.iter().position(|a| a == "--root") {
        root = args.get(i + 1).cloned().unwrap_or_else(|| process::exit(2));
        args.drain(i..i + 2);
    }
    let Some(out) = args.first() else {
        eprintln!("usage: fn-audio-ambient <out.json> [--root <FortniteGame>]");
        process::exit(2)
    };
    let set = PakSet::open_install(&root).unwrap_or_else(|e| {
        eprintln!("cannot open {root}: {e}");
        process::exit(1)
    });
    let mut j = fn_audio::ambient_scan::scan(&set).unwrap_or_else(|e| {
        eprintln!("scan failed: {e}");
        process::exit(1)
    });
    // per-cue facts the runtime needs without the paks (attenuation range, class volume, loop flag), read from the SoundCue assets
    let mut lib = fn_audio::library::Library::open(&root).unwrap_or_else(|e| {
        eprintln!("cannot open {root}: {e}");
        process::exit(1)
    });
    let cues = fn_audio::ambient_scan::referenced_cues(&j);
    let mut info = serde_json::Map::new();
    for c in &cues {
        match lib.load_cue(c) {
            Ok(cue) => {
                let a = cue.attenuation.as_ref();
                info.insert(
                    c.clone(),
                    serde_json::json!({
                        "volume_multiplier": cue.volume_multiplier, "class": cue.class_name,
                        "waves": cue.wave_paths().len(),
                        "attenuation": a.map(|a| serde_json::json!({"attenuate": a.attenuate, "spatialize": a.spatialize, "inner_cm": a.inner_radius(), "falloff_cm": a.falloff_distance, "max_distance_cm": a.max_distance(), "omni_radius": a.omni_radius})),
                    }),
                );
            }
            Err(e) => {
                info.insert(c.clone(), serde_json::json!({"error": e.to_string()}));
            }
        }
    }
    j["cues"] = serde_json::Value::Object(info);
    fs::write(out, serde_json::to_string_pretty(&j).unwrap()).unwrap_or_else(|e| {
        eprintln!("{out}: {e}");
        process::exit(1)
    });
    println!(
        "{out}: {} emitters, {} lakes, {} rivers, {} volumes",
        j["emitters"].as_array().map_or(0, |a| a.len()),
        j["lakes"].as_array().map_or(0, |a| a.len()),
        j["rivers"].as_array().map_or(0, |a| a.len()),
        j["volumes"].as_array().map_or(0, |a| a.len())
    );
}
