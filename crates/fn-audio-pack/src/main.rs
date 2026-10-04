use fn_audio::library::Library;
use fn_audio::mixer::{AudioEngine, PlayParams};
use fn_audio_pack::{collect, default_cues, open_pack, write_pack, PACK_NAME};
use std::{env, fs, process};

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

fn usage() -> ! {
    eprintln!(
        "usage: fn-audio-pack [--root <FortniteGame>] <command>\n\
         \x20 build <out-dir-or-file> [--manifest m.json] [--cue /Game/..]...   export the sounds the game needs ({PACK_NAME})\n\
         \x20 list <pack>                                                       entries of a pack\n\
         \x20 verify <pack>                                                     every cue renders bit-identically from the pack and from the install\n\
         root defaults to $FN_ROOT or the Fortnite 1.8 FortniteGame directory (read-only)"
    );
    process::exit(2)
}
fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("fn-audio-pack: {e}");
    process::exit(1)
}

fn render(lib: &mut Library, cue: &str) -> Vec<f32> {
    let mut eng = AudioEngine::new(48000);
    let h = eng.handle();
    lib.prepare_cue(&h, cue).unwrap_or_else(|e| fail(format!("{cue}: {e}")));
    h.play_cue(cue, PlayParams::default()).unwrap_or_else(|e| fail(e));
    eng.render_offline(48000 * 2, 1)
}

fn main() {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut root = env::var("FN_ROOT").unwrap_or_else(|_| DEFAULT_ROOT.to_string());
    if args.first().map(String::as_str) == Some("--root") {
        if args.len() < 2 {
            usage()
        }
        root = args[1].clone();
        args.drain(..2);
    }
    if args.is_empty() {
        usage()
    }
    match args[0].as_str() {
        "build" if args.len() >= 2 => {
            let mut out = std::path::PathBuf::from(&args[1]);
            if out.extension().is_none() {
                out = out.join(PACK_NAME);
            }
            let mut cues: Vec<String> = default_cues().iter().map(|s| s.to_string()).collect();
            let mut manifest = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--manifest" => {
                        manifest = args.get(i + 1).cloned();
                        i += 1;
                    }
                    "--cue" => {
                        cues.push(args.get(i + 1).cloned().unwrap_or_else(|| usage()));
                        i += 1;
                    }
                    _ => usage(),
                }
                i += 1;
            }
            let mut lib = Library::open(&root).unwrap_or_else(|e| fail(e));
            let cue_refs: Vec<&str> = cues.iter().map(String::as_str).collect();
            let t = std::time::Instant::now();
            let cl = collect(&mut lib, &cue_refs);
            let install = &lib.install;
            for (p, why) in &cl.problems {
                eprintln!("warning: {p}: {why}");
            }
            let st = write_pack(install, &cl.files, &out).unwrap_or_else(|e| fail(e));
            println!("{}: {} entries from {} packages, {} bytes ({} payload) in {:.1}s", out.display(), st.entries, cl.packages.len(), st.bytes, st.payload_bytes, t.elapsed().as_secs_f64());
            if let Some(m) = manifest {
                let mut by_ext: std::collections::BTreeMap<String, (usize, u64)> = Default::default();
                for (n, s) in &cl.files {
                    let e = by_ext.entry(n.rsplit('.').next().unwrap_or("").to_ascii_lowercase()).or_default();
                    e.0 += 1;
                    e.1 += s;
                }
                let v = serde_json::json!({
                    "pack": out.file_name().map(|s| s.to_string_lossy().into_owned()),
                    "format": "UE pak v3, stored/unencrypted, mount ../../../ (fn_pak::PakSet opens it directly)",
                    "cues": cues, "packages": cl.packages.len(), "entries": st.entries, "pack_bytes": st.bytes,
                    "by_extension": by_ext.iter().map(|(k, v)| (k.clone(), serde_json::json!({"files": v.0, "bytes": v.1}))).collect::<serde_json::Map<_, _>>(),
                    "problems": cl.problems.iter().map(|(a, b)| format!("{a}: {b}")).collect::<Vec<_>>(),
                    "files": cl.files.keys().collect::<Vec<_>>(),
                });
                fs::write(&m, serde_json::to_string_pretty(&v).unwrap()).unwrap_or_else(|e| fail(e));
            }
        }
        "list" if args.len() == 2 => {
            let inst = open_pack(&args[1]).unwrap_or_else(|e| fail(e));
            let mut v: Vec<(String, u64)> = inst.paks.effective().map(|(_, e)| (e.rel_path().to_string(), e.size)).collect();
            v.sort();
            for (n, s) in &v {
                println!("{s:>10}  {n}");
            }
            println!("{} entries, {} bytes", v.len(), v.iter().map(|x| x.1).sum::<u64>());
        }
        "verify" if args.len() == 2 => {
            let mut a = Library::open(&root).unwrap_or_else(|e| fail(e));
            let mut b = Library::new(open_pack(&args[1]).unwrap_or_else(|e| fail(e)));
            let (ca, cb) = (a.classes().clone(), b.classes().clone());
            let mut bad = 0;
            if ca.len() != cb.len() || ca.master_volume() != cb.master_volume() {
                println!("SoundClass tree differs: install {} classes, pack {}", ca.len(), cb.len());
                bad += 1;
            } else {
                println!("SoundClass tree: {} classes, master {} (identical)", cb.len(), cb.master_volume());
            }
            for cue in default_cues() {
                let (ra, rb) = (render(&mut a, cue), render(&mut b, cue));
                let same = ra.len() == rb.len() && ra.iter().zip(&rb).all(|(x, y)| x.to_bits() == y.to_bits());
                let peak = rb.iter().fold(0f32, |m, &x| m.max(x.abs()));
                println!("{:<86} {} peak {:.3}", cue.rsplit('/').next().unwrap(), if same { "identical" } else { "DIFFERENT" }, peak);
                // a cue that is silent in its first 2 s (random-delay / long-lead ambience loops) is only a problem when the install renders it differently: identical silence is still a faithful copy
                if !same {
                    bad += 1;
                }
            }
            let mix = fn_audio::soundclass::SoundMix::load(&b.install, "/Game/Sounds/FortSoundClassesAndModes/Fort_WeaponMelee_Duck_MixMod");
            println!("duck mix from pack: {}", if matches!(mix, Ok(Some(_))) { "ok" } else { "MISSING" });
            if !matches!(mix, Ok(Some(_))) {
                bad += 1;
            }
            if bad > 0 {
                fail(format!("{bad} problem(s)"));
            }
        }
        _ => usage(),
    }
}

