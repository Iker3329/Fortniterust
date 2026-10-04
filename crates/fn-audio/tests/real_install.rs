//! Verification against the real Fortnite 1.8 install (skipped with a message when it is not present; set `FN_ROOT`).

use fn_audio::analysis::{loop_seam_ratio, spectrum};
use fn_audio::codec::{decode_asset, ogg, opus, panics_caught};
use fn_audio::events;
use fn_audio::library::Library;
use fn_audio::mixer::{AudioEngine, PlayParams, Vec3};
use fn_audio::sweep::duration_matches;
use fn_audio::{read_wav, wav_bytes, DecodedWave, Install};
use std::path::Path;
use std::sync::OnceLock;

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

fn root() -> Option<String> {
    let r = std::env::var("FN_ROOT").unwrap_or_else(|_| DEFAULT_ROOT.to_string());
    if Path::new(&r).join("Content").join("Paks").exists() {
        Some(r)
    } else {
        eprintln!("SKIP: no Fortnite 1.8 install at {r} (set FN_ROOT)");
        None
    }
}

fn install() -> Option<&'static Install> {
    static I: OnceLock<Option<Install>> = OnceLock::new();
    I.get_or_init(|| root().map(|r| Install::open(r).expect("open install"))).as_ref()
}

const PICKAXE_ENEMY: &str = "FortniteGame/Content/Sounds/Fort_Weapons/PickAxe/PickAxe_Enemy_001";
const MINE_LOOP: &str = "FortniteGame/Content/Sounds/Trans_AmbientLoops/RoomTones/MineA019_Loop_Stereo";
const MISSION_STINGER: &str = "FortniteGame/Content/Sounds/Fort_Music/Missions/mission_success_03";

/// The pickaxe sound that blocked the first (Codex) attempt: an inline `OGG` format block after bCooked/NumFormats.
#[test]
fn pickaxe_enemy_decodes_with_exact_length() {
    let Some(inst) = install() else { return };
    let (w, a) = inst.decode_soundwave(PICKAXE_ENEMY).unwrap();
    assert_eq!(a.format_name(), Some("OGG"));
    assert_eq!((w.sample_rate, w.channels), (24000, 2));
    // RawPCMDataSize = 46396 bytes = 11599 stereo frames
    assert_eq!(w.frames(), 11599);
    assert_eq!(a.info.raw_pcm_data_size, Some(46396));
    assert_eq!(a.leftover, 0);
}

/// >= 50 distinct SoundWaves across both formats and many sample rates: decoded length within one frame (or 0.5 %) of
/// the asset's Duration, rate/channels equal the properties (Opus: rate is the Opus rate, checked by duration only).
#[test]
fn sixty_waves_across_formats_match_their_properties() {
    let Some(inst) = install() else { return };
    let mut paths = inst.sound_paths();
    paths.sort();
    let mut picked: Vec<String> = Vec::new();
    let mut rates: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let (mut ogg_n, mut opus_n) = (0, 0);
    // deterministic stride through the sorted list, at most 14 OGG waves per sample rate so rare rates are represented
    for p in paths.iter().step_by(7) {
        let Ok(a) = inst.load_soundwave(p) else { continue };
        let is_opus = a.streamed.is_some();
        let rate = a.info.sample_rate.unwrap_or(0);
        // pure-Rust Opus decodes at ~8x real time (slower in dev builds): keep the test to short streams
        if is_opus && (opus_n >= 5 || a.info.duration.unwrap_or(1e9) > 22.0) {
            continue;
        }
        if !is_opus {
            let c = rates.entry(rate).or_default();
            if *c >= 14 {
                continue;
            }
            *c += 1;
        }
        picked.push(p.clone());
        if is_opus {
            opus_n += 1
        } else {
            ogg_n += 1
        }
        if ogg_n >= 56 && rates.len() >= 5 {
            break;
        }
    }    // top up Opus with the streaming waves (they are rare, ~0.2 % of all waves)
    for p in paths.iter().filter(|p| p.contains("RoomTones") || p.contains("Fort_Music") || p.contains("Ambient")) {
        if opus_n >= 5 {
            break;
        }
        if picked.contains(p) {
            continue;
        }
        if let Ok(a) = inst.load_soundwave(p) {
            if a.streamed.is_some() && a.info.duration.unwrap_or(1e9) <= 22.0 {
                picked.push(p.clone());
                opus_n += 1;
            }
        }
    }
    assert!(picked.len() >= 50, "only {} waves picked", picked.len());
    assert!(opus_n >= 3, "opus waves {opus_n}");
    assert!(rates.len() >= 4, "sample rates {rates:?}");
    assert!(ogg_n >= 40, "ogg waves {ogg_n}");
    let mut worst = 0i64;
    for p in &picked {
        let (w, a) = inst.decode_soundwave(p).unwrap_or_else(|e| panic!("{p}: {e}"));
        let dur = a.info.duration.unwrap_or_else(|| panic!("{p}: no Duration"));
        let (delta, ok) = duration_matches(w.frames(), w.sample_rate, dur);
        assert!(ok, "{p}: {} frames @ {} Hz vs Duration {dur}s (delta {delta})", w.frames(), w.sample_rate);
        worst = worst.max(delta.abs());
        assert_eq!(Some(w.channels), a.info.channels, "{p}");
        if a.streamed.is_none() {
            assert_eq!(Some(w.sample_rate), a.info.sample_rate, "{p}");
        }
        let st = w.stats();
        assert_eq!(st.non_finite, 0, "{p}");
    }
    assert!(worst <= 1, "worst frame delta {worst}");
    eprintln!("{} waves verified ({ogg_n} OGG, {opus_n} OPUS), sample rates {rates:?}, worst delta {worst}", picked.len());
}

/// Write three decoded waves to WAV and verify them by analysis (peak/RMS/NaN/header/spectrum/clipping).
#[test]
fn three_waves_wav_analysis() {
    let Some(inst) = install() else { return };
    let dir = std::env::temp_dir().join("fn-audio-wav-check");
    std::fs::create_dir_all(&dir).unwrap();
    for (path, min_rms, max_dom_hz) in [(PICKAXE_ENEMY, 0.05, 400.0), (MINE_LOOP, 0.02, 600.0), (MISSION_STINGER, 0.05, 600.0)] {
        let (w, _) = inst.decode_soundwave(path).unwrap();
        let bytes = wav_bytes(&w).unwrap();
        let name = path.rsplit('/').next().unwrap();
        std::fs::write(dir.join(format!("{name}.wav")), &bytes).unwrap();
        // header sanity
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize, bytes.len() - 8);
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), w.channels);
        assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), w.sample_rate);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize, w.samples.len() * 2);
        // round trip
        let back = read_wav(&bytes).unwrap();
        assert_eq!(back.frames(), w.frames());
        let max_err = w.samples.iter().zip(&back.samples).fold(0f32, |a, (x, y)| a.max((x - y).abs()));
        assert!(max_err < 1.0 / 16000.0, "{name}: wav quantisation error {max_err}");
        // levels
        let st = w.stats();
        assert_eq!(st.non_finite, 0);
        assert!(st.peak <= 1.0 && st.peak > 0.05, "{name}: peak {}", st.peak);
        assert!(st.rms >= min_rms && st.rms < 0.7, "{name}: rms {}", st.rms);
        assert!(st.max_clip_run <= 8, "{name}: clip run {}", st.max_clip_run);
        // spectrum: band-limited, bass-heavy material with no broadband noise floor
        let sp = spectrum(&w, w.frames() / 3, 8192).unwrap();
        assert!(sp.dominant_hz > 0.0 && sp.dominant_hz < max_dom_hz, "{name}: dominant {} Hz", sp.dominant_hz);
        assert!(sp.high_band_fraction < 0.05, "{name}: high band {}", sp.high_band_fraction);
        eprintln!("{name}: {:?} stats {st:?}", sp);
    }
}

/// Looped samples (bLooping SoundWave, OGG) have a seam as smooth as the rest of the signal (the sweep reports
/// 29 of the 31 looping waves in the install with a seam ratio <= 3, median 0.73).
#[test]
fn known_looped_sample_loops_seamlessly() {
    let Some(inst) = install() else { return };
    let mut paths: Vec<String> = inst.sound_paths().into_iter().filter(|p| p.contains("Loop") && !p.contains("L10N")).collect();
    paths.sort();
    let mut ratios = Vec::new();
    for p in &paths {
        let Ok(a) = inst.load_soundwave(p) else { continue };
        if !a.info.looping || a.streamed.is_some() {
            continue;
        }
        let w = decode_asset(&a).unwrap();
        assert_eq!((w.loop_start, w.loop_end), (Some(0), Some(w.frames() as u64)));
        if let Some(r) = loop_seam_ratio(&w) {
            ratios.push((r, p.clone()));
        }
        if ratios.len() >= 12 {
            break;
        }
    }
    assert!(ratios.len() >= 4, "only {} looping OGG waves found", ratios.len());
    let good = ratios.iter().filter(|(r, _)| *r < 3.0).count();
    assert!(good * 10 >= ratios.len() * 7, "seam ratios {ratios:?}");
    assert!(ratios.iter().any(|(r, _)| *r < 1.0), "no loop with a seam smoother than average: {ratios:?}");
}
/// Offline render of the pickaxe swing cue with a fixed seed: deterministic, golden length / peak / RMS.
#[test]
fn pickaxe_swing_cue_golden_render() {
    let Some(r) = root() else { return };
    let mut lib = Library::open(&r).unwrap();
    let mut render = |seed: u64| -> Vec<f32> {
        let mut eng = AudioEngine::new(48000);
        let h = eng.handle();
        assert_eq!(lib.prepare_cue(&h, events::SWING).unwrap(), 0, "waves missing");
        h.play_cue(events::SWING, PlayParams::default()).unwrap();
        eng.render_offline(72000, seed)
    };
    let a = render(7);
    let b = render(7);
    assert_eq!(a, b, "same seed must be bit-identical");
    let c = render(8);
    assert_ne!(a, c, "different seeds pick different variations");
    let w = DecodedWave::new(a, 48000, 2);
    let st = w.stats();
    assert_eq!(w.frames(), 72000);
    assert_eq!(st.non_finite, 0);
    // golden values (seed 7): see GOLDEN below
    assert!((st.peak - GOLDEN_PEAK).abs() < GOLDEN_PEAK * 0.01, "peak {}", st.peak);
    assert!((st.rms - GOLDEN_RMS).abs() < GOLDEN_RMS * 0.01, "rms {:.9}", st.rms);
    // audible tail ends where the cue's own duration (1.37 s) says
    let last = w.samples.chunks(2).rposition(|f| f[0].abs() > 1e-4 || f[1].abs() > 1e-4).unwrap();
    let secs = last as f32 / 48000.0;
    assert!((0.3..1.4).contains(&secs), "audible until {secs}s");
}
const GOLDEN_PEAK: f32 = 0.088005126;
const GOLDEN_RMS: f32 = 0.008343246;

/// 3D: closer = louder, left/right pan by azimuth, culled beyond the attenuation's max distance (inner 300 + falloff 800).
#[test]
fn swing_cue_spatialisation() {
    let Some(r) = root() else { return };
    let mut lib = Library::open(&r).unwrap();
    let mut render = |pos: Vec3| -> (f32, f32, f32) {
        let mut eng = AudioEngine::new(48000);
        let h = eng.handle();
        lib.prepare_cue(&h, events::SWING).unwrap();
        h.set_listener(Vec3::default(), Vec3::new(0.0, 0.0, -1.0), Vec3::new(0.0, 1.0, 0.0));
        h.play_cue(events::SWING, PlayParams { position: Some(pos), ..Default::default() }).unwrap();
        let out = eng.render_offline(48000, 3);
        let (mut l, mut rr, mut p) = (0f64, 0f64, 0f32);
        for f in out.chunks(2) {
            l += (f[0] * f[0]) as f64;
            rr += (f[1] * f[1]) as f64;
            p = p.max(f[0].abs()).max(f[1].abs());
        }
        (l.sqrt() as f32, rr.sqrt() as f32, p)
    };
    let (nl, nr, _) = render(Vec3::new(0.0, 0.0, -250.0)); // inside the 300 cm inner radius, in front
    let (rl, rr_, _) = render(Vec3::new(250.0, 0.0, 0.0)); // hard right (+x)
    let (ll, lr, _) = render(Vec3::new(-250.0, 0.0, 0.0));
    let (_, _, far) = render(Vec3::new(0.0, 0.0, -700.0));
    let (_, _, mid) = render(Vec3::new(0.0, 0.0, -2000.0)); // only the distant (3000 cm falloff) layer is left
    let (_, _, cull) = render(Vec3::new(0.0, 0.0, -3500.0));
    assert!(rr_ > rl * 1.5, "right source should be louder on the right ({rl} vs {rr_})");
    assert!(ll > lr * 1.5, "left source should be louder on the left ({ll} vs {lr})");
    assert!((nl / nr - 1.0).abs() < 0.2, "front source is centred ({nl} vs {nr})");
    assert!(far < nl.max(nr), "700 cm quieter than 250 cm");
    assert!(mid > 0.0 && mid < far, "2000 cm: close layers culled (inner 300 + falloff 800), distant layer still audible ({mid} vs {far})");
    assert_eq!(cull, 0.0, "beyond 3000 cm every layer is culled");
}

/// Truncated / corrupted payloads of every real codec never panic.
#[test]
fn fuzzed_payloads_never_panic() {
    let Some(inst) = install() else { return };
    let before = panics_caught();
    let mut rng = 0x1234_5678_9abc_def1u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let ogg_payload = inst.load_soundwave(PICKAXE_ENEMY).unwrap().formats[0].payload.clone();
    let opus_stream = inst.load_soundwave(MINE_LOOP).unwrap().streamed.unwrap().stream_bytes();
    let opus_head = &opus_stream[..opus_stream.len().min(6_000)];
    let mut errors = 0;
    // truncations
    for i in 0..40 {
        let cut = (next() as usize) % ogg_payload.len();
        errors += ogg::decode_ogg(&ogg_payload[..cut]).is_err() as usize;
        let cut = (next() as usize) % opus_head.len();
        errors += opus::decode_ue_opus(&opus_head[..cut]).is_err() as usize;
        let _ = i;
    }
    // byte flips
    for _ in 0..30 {
        let mut o = ogg_payload.clone();
        for _ in 0..(1 + next() % 8) {
            let at = (next() as usize) % o.len();
            o[at] ^= (next() & 0xff) as u8 | 1;
        }
        let _ = ogg::decode_ogg(&o);
        let mut p = opus_head.to_vec();
        for _ in 0..(1 + next() % 8) {
            let at = 17 + (next() as usize) % (p.len() - 17);
            p[at] ^= (next() & 0xff) as u8 | 1;
        }
        let _ = opus::decode_ue_opus(&p);
        // header corruption too
        let mut h = opus_head.to_vec();
        let at = (next() as usize) % 17;
        h[at] ^= (next() & 0xff) as u8 | 1;
        let _ = opus::decode_ue_opus(&h);
    }
    assert!(errors > 20, "truncation should usually be rejected ({errors})");
    assert_eq!(panics_caught(), before, "a decoder panicked");
}

/// Corrupted package bytes (header + .uexp) through the whole parse -> decode chain: no panics.
#[test]
fn fuzzed_packages_never_panic() {
    let Some(inst) = install() else { return };
    use fn_uasset::{Package, PackageData};
    let ap = "FortniteGame/Content/Sounds/Fort_Weapons/PickAxe/PickAxe_Enemy_001.uasset";
    let d = PackageData::load(&inst.paks, ap).unwrap();
    let (asset, uexp) = (d.asset.to_vec(), d.uexp.to_vec());
    let mut rng = 0xdead_beef_1234_5678u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let mut parsed_ok = 0;
    for i in 0..600 {
        let (mut a, mut u) = (asset.clone(), uexp.clone());
        match i % 4 {
            0 => u.truncate((next() as usize) % u.len()),
            1 => {
                for _ in 0..(1 + next() % 6) {
                    let at = (next() as usize) % u.len();
                    u[at] ^= (next() & 0xff) as u8 | 1;
                }
            }
            2 => a.truncate((next() as usize) % a.len()),
            _ => {
                for _ in 0..(1 + next() % 4) {
                    let at = (next() as usize) % a.len();
                    a[at] ^= (next() & 0xff) as u8 | 1;
                }
            }
        }
        let r = std::panic::catch_unwind(|| {
            let Ok(pkg) = Package::parse(&a, &u) else { return false };
            let idx: Vec<usize> = pkg.exports_of_class("SoundWave").collect();
            for i in idx {
                if let Ok(sw) = fn_audio::soundwave::parse_soundwave(&pkg, i, &[]) {
                    let _ = decode_asset(&sw);
                }
            }
            true
        });
        match r {
            Ok(ok) => parsed_ok += ok as usize,
            Err(_) => panic!("panic on mutation #{i}"),
        }
    }
    assert!(parsed_ok > 30);
}

/// Decoding a code-3 packet frame by frame (the >1500-byte workaround) equals decoding it whole.
#[test]
fn opus_split_matches_whole_packet_decode() {
    let Some(inst) = install() else { return };
    let s = inst.load_soundwave(MINE_LOOP).unwrap().streamed.unwrap().stream_bytes();
    let h = opus::parse_header(&s).unwrap();
    let mut whole = opus_decoder::OpusDecoder::new(h.sample_rate, h.channels as usize).unwrap();
    let mut split = opus_decoder::OpusDecoder::new(h.sample_rate, h.channels as usize).unwrap();
    let mut p = 17;
    let (mut a, mut b) = (vec![0f32; 5760 * 2], vec![0f32; 5760 * 2]);
    for _ in 0..6 {
        let size = u16::from_le_bytes([s[p], s[p + 1]]) as usize;
        let packet = &s[p + 2..p + 2 + size];
        p += 2 + size;
        let n = whole.decode_float(packet, &mut a, false).unwrap();
        let mut got = Vec::new();
        for part in opus::split_code3(packet).expect("code-3 packet") {
            let m = split.decode_float(&part, &mut b, false).unwrap();
            got.extend_from_slice(&b[..m * 2]);
        }
        assert_eq!(got.len(), n * 2);
        let max = got.iter().zip(&a[..n * 2]).fold(0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(max < 1e-6, "split decode differs by {max}");
    }
}

/// Corrupted SoundCue packages never panic the graph parser.
#[test]
fn fuzzed_cue_packages_never_panic() {
    let Some(inst) = install() else { return };
    use fn_audio::cue::{parse_cue, NoResolver};
    use fn_uasset::{Package, PackageData};
    let d = PackageData::load(&inst.paks, "FortniteGame/Content/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue.uasset").unwrap();
    let (asset, uexp) = (d.asset.to_vec(), d.uexp.to_vec());
    let mut rng = 0x0bad_cafe_f00d_1234u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let mut parsed = 0;
    for i in 0..1500 {
        let (mut a, mut u) = (asset.clone(), uexp.clone());
        match i % 4 {
            0 => u.truncate((next() as usize) % u.len()),
            1 => {
                for _ in 0..(1 + next() % 6) {
                    let at = (next() as usize) % u.len();
                    u[at] ^= (next() & 0xff) as u8 | 1;
                }
            }
            2 => a.truncate((next() as usize) % a.len()),
            _ => {
                for _ in 0..(1 + next() % 4) {
                    let at = (next() as usize) % a.len();
                    a[at] ^= (next() & 0xff) as u8 | 1;
                }
            }
        }
        let r = std::panic::catch_unwind(|| match Package::parse(&a, &u) {
            Ok(pkg) => parse_cue(&pkg, &mut NoResolver).is_ok(),
            Err(_) => false,
        });
        match r {
            Ok(ok) => parsed += ok as usize,
            Err(_) => panic!("panic on mutation #{i}"),
        }
    }
    assert!(parsed > 20, "{parsed}");
}



