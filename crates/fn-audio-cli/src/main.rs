use fn_audio::analysis::{loop_seam_ratio, octave_bands_db, spectrum};
use fn_audio::cue::NodeKind;
use fn_audio::events;
use fn_audio::install::to_pak_asset_path;
use fn_audio::library::Library;
use fn_audio::mixer::{AudioEngine, PlayParams, Vec3};
use fn_audio::soundclass::Bus;
use fn_audio::{read_wav, sweep, write_wav, DecodedWave, Install};
use std::collections::BTreeMap;
use std::{env, fs, process};

fn usage() -> ! {
    eprintln!(
        "usage: fn-audio-cli [--root <FortniteGame>] <command>\n\
         \x20 list [substr]                      list package paths under Sounds/Audio\n\
         \x20 info <package>                     SoundWave properties + cooked format layout\n\
         \x20 decode <package> <out.wav>         decode a SoundWave to 16-bit WAV (prints stats + spectrum)\n\
         \x20 sweep [report.json [issues.tsv]]   decode every SoundWave in the install, verify, summarize\n\
         \x20 cue <package>                      dump a SoundCue graph\n\
         \x20 cues [report.json]                 parse every SoundCue, tabulate node classes / unresolved refs\n\
         \x20 classes                            SoundClass tree: chain volumes and buses\n\
         \x20 events                             verify every gameplay event cue loads, decodes and renders\n\
         \x20 render <cue> <out.wav> [--seed N] [--secs S] [--pos x,y,z] [--listener x,y,z] [--rate HZ]\n\
         \x20                                    offline render of a cue (positions in cm)\n\
         \x20 analyze <in.wav>                   peak / rms / spectrum / loop-seam of a WAV\n\
         \x20 device-smoke                       (build with --features device-output) play 0.3 s of the pickaxe swing\n\
         root defaults to $FN_ROOT or the Fortnite 1.8 FortniteGame directory"
    );
    process::exit(2)
}
fn fail(e: impl std::fmt::Display) -> ! {
    eprintln!("fn-audio-cli: {e}");
    process::exit(1)
}
const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

fn json(v: &serde_json::Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

fn opt<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn vec3(s: &str) -> Vec3 {
    let p: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    if p.len() != 3 {
        fail("expected x,y,z");
    }
    Vec3::new(p[0], p[1], p[2])
}

fn print_wave_stats(w: &DecodedWave) {
    let s = w.stats();
    print!("peak {:.4} rms {:.4} nan {} max-clip-run {}", s.peak, s.rms, s.non_finite, s.max_clip_run);
    if let Some(sp) = spectrum(w, w.frames() / 4, 8192) {
        print!("; dominant {:.0} Hz centroid {:.0} Hz rolloff95 {:.0} Hz high-band {:.4}", sp.dominant_hz, sp.centroid_hz, sp.rolloff95_hz, sp.high_band_fraction);
    }
    println!();
    let bands = octave_bands_db(w);
    if !bands.is_empty() {
        let s: Vec<String> = bands.iter().map(|(e, db)| if e.is_finite() { format!("<{e:.0}:{db:.0}") } else { format!("rest:{db:.0}") }).collect();
        println!("  octave-band dB: {}", s.join(" "));
    }
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
    let open = || Install::open(&root).unwrap_or_else(|e| fail(e));
    let open_lib = || Library::open(&root).unwrap_or_else(|e| fail(e));
    match args[0].as_str() {
        "list" => {
            let inst = open();
            let pat = args.get(1).map(|s| s.to_ascii_lowercase());
            for p in inst.sound_paths() {
                if pat.as_ref().map_or(true, |s| p.to_ascii_lowercase().contains(s)) {
                    println!("{p}");
                }
            }
        }
        "info" if args.len() == 2 => {
            let a = open().load_soundwave(&args[1]).unwrap_or_else(|e| fail(e));
            println!("{:#?}", a.info);
            println!("cooked={} guid={:02x?} leftover={} tail_error={:?}", a.cooked, a.guid, a.leftover, a.tail_error);
            for f in &a.formats {
                println!("format {} placement {:?} flags {:#x} size_on_disk {} offset {} payload {} bytes", f.name, f.placement, f.bulk.flags, f.bulk.size_on_disk, f.bulk.offset_in_file, f.payload.len());
            }
            if let Some(s) = &a.streamed {
                println!("streamed format {} chunks {}", s.format, s.chunks.len());
                for (i, c) in s.chunks.iter().enumerate() {
                    println!("  chunk {i}: {:?} flags {:#x} DataSize {} AudioDataSize {} payload {}", c.placement, c.bulk.flags, c.data_size, c.audio_data_size, c.payload.len());
                }
            }
        }
        "decode" if args.len() == 3 => {
            let (w, a) = open().decode_soundwave(&args[1]).unwrap_or_else(|e| fail(e));
            write_wav(&args[2], &w).unwrap_or_else(|e| fail(e));
            print!("{} {} Hz, {} ch, {} frames ({:.4} s); property duration {:?} s; ", a.format_name().unwrap_or("?"), w.sample_rate, w.channels, w.frames(), w.duration_seconds(), a.info.duration);
            print_wave_stats(&w);
        }
        "sweep" => {
            let inst = open();
            let t = std::time::Instant::now();
            let rows = sweep::sweep_install(&inst);
            let sum = sweep::summarize(&rows);
            println!("{}", json(&sum));
            eprintln!("swept {} SoundWaves in {:.1}s", rows.len(), t.elapsed().as_secs_f64());
            if let Some(out) = args.get(2) {
                let mut t = String::new();
                for r in rows.iter().filter(|r| !r.decoded || !r.duration_ok || !r.rate_ok || !r.channels_ok) {
                    t += &format!("{}\t{}\tstream={}\tprop {:?}Hz {:?}ch {:?}s\tdec {}Hz {}ch {}fr\tdelta {:?}\t{:?}\n", r.path, r.format, r.streaming, r.prop_rate, r.prop_channels, r.prop_duration, r.dec_rate, r.dec_channels, r.dec_frames, r.frame_delta, r.error);
                }
                fs::write(out, t).unwrap_or_else(|e| fail(e));
            }
            if let Some(out) = args.get(1) {
                fs::write(out, json(&sum)).unwrap_or_else(|e| fail(e));
            }
        }
        "cue" if args.len() == 2 => {
            let mut lib = open_lib();
            let cue = lib.load_cue(&args[1]).unwrap_or_else(|e| fail(e));
            print!("{}", cue.dump());
            let waves = cue.wave_paths();
            println!("waves: {}", waves.len());
            for w in waves {
                println!("  {w}{}", if lib.install.paks.contains(&to_pak_asset_path(&w)) { "" } else { "   <MISSING>" });
            }
        }
        "cues" => {
            let mut lib = open_lib();
            let all = args.iter().any(|a| a == "--all");
            let paths: Vec<String> = lib.install.all_asset_paths().into_iter().filter(|p| all || p.to_ascii_lowercase().contains("cue")).collect();
            let (mut ok, mut not_cue, mut failed) = (0usize, 0usize, 0usize);
            let mut classes: BTreeMap<String, usize> = BTreeMap::new();
            let (mut unresolved_att, mut null_waves, mut missing_waves, mut total_nodes, mut no_root, mut conc, mut cue_att) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
            let mut errs: BTreeMap<String, usize> = BTreeMap::new();
            let mut buses: BTreeMap<String, usize> = BTreeMap::new();
            let t = std::time::Instant::now();
            for p in &paths {
                match lib.load_cue(p) {
                    Ok(c) => {
                        ok += 1;
                        *buses.entry(format!("{:?}", c.class.bus)).or_default() += 1;
                        conc += c.concurrency.is_some() as usize;
                        cue_att += c.attenuation.is_some() as usize;
                        match &c.root {
                            None => no_root += 1,
                            Some(r) => r.walk(&mut |n| {
                                total_nodes += 1;
                                *classes.entry(n.class_label().to_string()).or_default() += 1;
                                match &n.kind {
                                    NodeKind::Attenuation(None) => {
                                        unresolved_att += 1;
                                        if args.iter().any(|a| a == "--verbose") && unresolved_att <= 12 {
                                            eprintln!("unresolved attenuation: {} / {}", c.path, n.name);
                                        }
                                    }
                                    NodeKind::WavePlayer { wave: None, .. } => null_waves += 1,
                                    NodeKind::WavePlayer { wave: Some(w), .. } if !lib.install.paks.contains(&to_pak_asset_path(w)) => missing_waves += 1,
                                    _ => {}
                                }
                            }),
                        }
                    }
                    Err(e) => {
                        let s = e.to_string();
                        if s.contains("no SoundCue export") {
                            not_cue += 1;
                        } else {
                            failed += 1;
                            *errs.entry(s.chars().take(100).collect()).or_default() += 1;
                        }
                    }
                }
            }
            let rep = serde_json::json!({
                "candidate_packages": paths.len(), "parsed_cues": ok, "not_a_cue": not_cue, "parse_failures": failed, "failures": errs,
                "cues_without_root": no_root, "total_nodes": total_nodes, "node_classes": classes,
                "attenuation_nodes_unresolved": unresolved_att, "wave_players_null": null_waves, "wave_players_missing_asset": missing_waves,
                "cues_with_concurrency": conc, "cues_with_cue_level_attenuation": cue_att, "cue_buses": buses,
                "seconds": t.elapsed().as_secs_f64(),
            });
            println!("{}", json(&rep));
            if let Some(out) = args.get(1) {
                fs::write(out, json(&rep)).unwrap_or_else(|e| fail(e));
            }
        }
        "classes" => {
            let mut lib = open_lib();
            let tree = lib.classes().clone();
            let mut names: Vec<&str> = tree.names().collect();
            names.sort();
            println!("{} SoundClasses; Master volume {}", tree.len(), tree.master_volume());
            for n in names {
                let i = tree.info(n);
                let c = tree.get(n).unwrap();
                println!("{:46} parent {:44} vol {:<5} -> chain-volume {:.4} bus {:?}{}", n, c.parent.as_deref().unwrap_or("-"), c.volume, i.base_volume, i.bus, if i.is_ui { " ui" } else { "" });
            }
            // SoundMix assets of the same folder
            let mut mixes = 0;
            let mut adjusters = 0;
            for p in lib.install.all_asset_paths().into_iter().filter(|p| p.contains("/FortSoundClassesAndModes/")) {
                if let Ok(Some(m)) = lib.load_mix(&p) {
                    mixes += 1;
                    adjusters += m.adjusters.len();
                    if m.name == "Fort_WeaponMelee_Duck_MixMod" {
                        println!("SoundMix {}: fade in {}s hold {}s fade out {}s, adjusters {:?}", m.name, m.fade_in_time, m.duration, m.fade_out_time, m.adjusters);
                    }
                }
            }
            println!("{mixes} SoundMix assets parsed ({adjusters} class adjusters)");
        }
        "events" => {
            let mut lib = open_lib();
            let eng = AudioEngine::new(48000);
            let h = eng.handle();
            let mut eng = eng;
            let mut bad = 0;
            for e in events::TABLE {
                match lib.prepare_cue(&h, e.cue) {
                    Ok(missing) => {
                        let cue = lib.load_cue(e.cue).unwrap();
                        h.play_cue(e.cue, PlayParams::default()).unwrap();
                        let out = eng.render_offline(48000 * 2, 1);
                        let peak = out.iter().fold(0f32, |a, &b| a.max(b.abs()));
                        let last = out.chunks(2).rposition(|f| f[0].abs() > 1e-4 || f[1].abs() > 1e-4).map_or(0.0, |i| i as f32 / 48000.0);
                        println!("{:30} {:9?} {:<78} waves {:2} missing {} peak {:.3} audible-until {:.2}s class {} bus {:?} base {:.3}", e.event, e.source, e.cue.rsplit('/').next().unwrap(), cue.wave_paths().len(), missing, peak, last, cue.class.name, cue.class.bus, cue.class.base_volume);
                        if missing > 0 || peak == 0.0 {
                            bad += 1;
                        }
                        eng.handle().stop_all();
                        eng.render_offline(2048, 1);
                    }
                    Err(err) => {
                        bad += 1;
                        println!("{:30} FAILED {}: {err}", e.event, e.cue);
                    }
                }
            }
            for (a, m) in &lib.warnings {
                println!("warning: {a}: {m}");
            }
            if bad > 0 {
                fail(format!("{bad} event cue(s) problematic"));
            }
        }
        "render" if args.len() >= 3 => {
            let mut lib = open_lib();
            let rate: u32 = opt(&args, "--rate").and_then(|s| s.parse().ok()).unwrap_or(48000);
            let seed: u64 = opt(&args, "--seed").and_then(|s| s.parse().ok()).unwrap_or(1);
            let secs: f32 = opt(&args, "--secs").and_then(|s| s.parse().ok()).unwrap_or(2.0);
            let mut eng = AudioEngine::new(rate);
            let h = eng.handle();
            let missing = lib.prepare_cue(&h, &args[1]).unwrap_or_else(|e| fail(e));
            if missing > 0 {
                eprintln!("warning: {missing} wave(s) could not be loaded: {:?}", lib.warnings);
            }
            if let Some(l) = opt(&args, "--listener") {
                h.set_listener(vec3(l), Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 1.0, 0.0));
            }
            let mut p = PlayParams::default();
            p.position = opt(&args, "--pos").map(vec3);
            h.play_cue(&args[1], p).unwrap_or_else(|e| fail(e));
            let out = eng.render_offline((secs * rate as f32) as usize, seed);
            let w = DecodedWave::new(out, rate, 2);
            write_wav(&args[2], &w).unwrap_or_else(|e| fail(e));
            let st = eng.mixer().stats;
            print!("rendered {} frames at {rate} Hz (seed {seed}), voices started {} missing waves {}; ", w.frames(), st.voices_started, st.missing_waves);
            print_wave_stats(&w);
        }
        "analyze" if args.len() == 2 => {
            let b = fs::read(&args[1]).unwrap_or_else(|e| fail(e));
            let w = read_wav(&b).unwrap_or_else(|e| fail(e));
            print!("{} Hz, {} ch, {} frames ({:.3} s); ", w.sample_rate, w.channels, w.frames(), w.duration_seconds());
            print_wave_stats(&w);
            if let Some(r) = loop_seam_ratio(&w) {
                println!("loop-seam ratio {r:.2}");
            }
        }
        "device-smoke" => device_smoke(&root),
        _ => usage(),
    }
    let _ = Bus::Master;
}

#[cfg(feature = "device-output")]
fn device_smoke(root: &str) {
    let mut lib = Library::open(root).unwrap_or_else(|e| fail(e));
    let (handle, stream) = fn_audio::device::open_default().unwrap_or_else(|e| fail(e));
    println!("output device: {} ({} Hz, {} ch)", stream.device_name, stream.sample_rate, stream.channels);
    lib.prepare_cue(&handle, events::SWING).unwrap_or_else(|e| fail(e));
    // low volume: master 0.15 x the class chain (~0.28 x 0.3 cue volume) -> well below normal listening level
    handle.set_bus_volume(Bus::Master, 0.15);
    handle.play_cue(events::SWING, PlayParams::default()).unwrap_or_else(|e| fail(e));
    std::thread::sleep(std::time::Duration::from_millis(300));
    handle.stop_all();
    std::thread::sleep(std::time::Duration::from_millis(40));
    println!("played 0.3 s of {} through the default device", events::SWING);
    drop(stream);
}

#[cfg(not(feature = "device-output"))]
fn device_smoke(_root: &str) {
    fail("built without the device-output feature (cargo run -p fn-audio-cli --features device-output -- device-smoke)")
}


