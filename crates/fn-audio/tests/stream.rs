//! Incremental decode (`fn_audio::stream`) against whole-wave decode on real waves of the Fortnite 1.8 install
//! (skipped with a message when it is not present; set `FN_ROOT`).

use fn_audio::codec::decode_asset;
use fn_audio::library::Library;
use fn_audio::mixer::{AudioEngine, PlayParams};
use fn_audio::stream::{StreamDecoder, StreamSource, StreamedWave};
use fn_audio::{events, Install};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";
/// 10.9 s stereo Ogg: the shortest of the three waves of the Day ambience cue.
const OGG_DAY: &str = "FortniteGame/Content/Sounds/Fort_Ambient_Looping/Grassland_Exterior/Grassland_Day11c";
const OGG_WIND: &str = "FortniteGame/Content/Sounds/Fort_Ambient_Looping/Wind_Exterior/Grasslands_Wind04";
/// Streaming-chunk UE4OPUS wave (20 s stereo room tone).
const OPUS_MINE: &str = "FortniteGame/Content/Sounds/Trans_AmbientLoops/RoomTones/MineA019_Loop_Stereo";

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

fn bits_equal(a: &[f32], b: &[f32]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter().zip(b).position(|(x, y)| x.to_bits() != y.to_bits())
}

fn load(path: &str) -> (fn_audio::soundwave::SoundWaveAsset, Vec<f32>, Arc<StreamSource>) {
    let inst = install().unwrap();
    let a = inst.load_soundwave(path).unwrap();
    let t = Instant::now();
    let whole = decode_asset(&a).unwrap();
    eprintln!("{path}: whole-wave decode {:.0} ms ({} frames, {:.1} s)", t.elapsed().as_secs_f64() * 1e3, whole.frames(), whole.duration_seconds());
    let src = Arc::new(StreamSource::from_asset(&a).unwrap().expect("streamable format"));
    assert_eq!((src.sample_rate, src.channels as u16, src.frames as usize), (whole.sample_rate, whole.channels, whole.frames()), "stream facts equal the whole decode");
    (a, whole.samples, src)
}

#[test]
fn ogg_stream_is_bit_identical_to_the_whole_decode_and_wraps_sample_exactly() {
    let Some(_) = install() else { return };
    for p in [OGG_DAY, OGG_WIND] {
        let (_, whole, src) = load(p);
        let mut d = StreamDecoder::new(src.clone()).unwrap();
        // packet by packet, the way the worker does it
        let mut got = Vec::new();
        let mut units = 0;
        while d.next(&mut got).unwrap() {
            units += 1;
        }
        assert_eq!(bits_equal(&got, &whole), None, "{p}: first pass");
        // the loop wrap: a rewound decoder produces the very same frames again
        d.rewind().unwrap();
        let mut again = Vec::new();
        while d.next(&mut again).unwrap() {}
        assert_eq!(bits_equal(&again, &whole), None, "{p}: pass after the wrap");
        eprintln!("{p}: {units} packets, {} samples bit-identical, 2 passes", whole.len());
    }
}

#[test]
fn opus_stream_is_bit_identical_to_the_whole_decode_and_wraps_sample_exactly() {
    let Some(_) = install() else { return };
    let (a, whole, src) = load(OPUS_MINE);
    assert!(a.streamed.is_some());
    let t = Instant::now();
    let got = StreamDecoder::decode_all(&src).unwrap();
    eprintln!("opus stream decode {:.0} ms", t.elapsed().as_secs_f64() * 1e3);
    assert_eq!(bits_equal(&got, &whole), None);
    let mut d = StreamDecoder::new(src.clone()).unwrap();
    let mut first_units = Vec::new();
    while d.position() < 100_000 {
        assert!(d.next(&mut first_units).unwrap());
    }
    // wrap in the middle of the wave: rewind and the first frames equal frame 0.. of the whole decode
    d.rewind().unwrap();
    let mut again = Vec::new();
    while d.position() < 50_000 {
        assert!(d.next(&mut again).unwrap());
    }
    assert_eq!(bits_equal(&again, &whole[..again.len()]), None);
    // the stream's last packet is trimmed to the true sample count
    let mut d = StreamDecoder::new(src.clone()).unwrap();
    let mut n = 0u64;
    let mut v = Vec::new();
    while d.next(&mut v).unwrap() {
        n = d.position();
    }
    assert_eq!(n, src.frames);
}

#[test]
fn seeking_to_any_offset() {
    let Some(_) = install() else { return };
    // Ogg: exact at any offset (decode from 0 and discard)
    let (_, whole, src) = load(OGG_DAY);
    let ch = src.channels as usize;
    for off in [1u64, 1023, 1024, 4095, 100_000, 261_000, src.frames - 1] {
        let mut d = StreamDecoder::new(src.clone()).unwrap();
        d.seek(off, 0).unwrap();
        let mut got = Vec::new();
        while d.next(&mut got).unwrap() {}
        assert_eq!(bits_equal(&got, &whole[off as usize * ch..]), None, "ogg offset {off}");
    }
    // Opus: exact when decoding from packet 0; with the default pre-roll the decoder state has converged (report the error)
    let (_, whole, src) = load(OPUS_MINE);
    let ch = src.channels as usize;
    for off in [1u64, 1441, 123_457, 400_000] {
        let mut d = StreamDecoder::new(src.clone()).unwrap();
        d.seek(off, usize::MAX).unwrap();
        let mut got = Vec::new();
        while d.next(&mut got).unwrap() {}
        assert_eq!(bits_equal(&got, &whole[off as usize * ch..]), None, "opus exact offset {off}");
        let mut d = StreamDecoder::new(src.clone()).unwrap();
        d.seek(off, fn_audio::stream::DEFAULT_PREROLL).unwrap();
        let mut fast = Vec::new();
        while d.next(&mut fast).unwrap() {}
        let want = &whole[off as usize * ch..];
        assert_eq!(fast.len(), want.len());
        let (mut max_first, mut max_all, mut identical_from) = (0f32, 0f32, want.len());
        for (i, (x, y)) in fast.iter().zip(want).enumerate() {
            let e = (x - y).abs();
            if i < 4800 {
                max_first = max_first.max(e);
            }
            max_all = max_all.max(e);
            if e != 0.0 {
                identical_from = i + 1;
            }
        }
        eprintln!("opus seek {off} preroll {}: max |err| in the first 100 ms {max_first:.2e}, overall {max_all:.2e}, bit-identical from sample {identical_from} of {}", fn_audio::stream::DEFAULT_PREROLL, want.len());
        assert!(max_all < 0.05, "pre-roll seek error {max_all}");
    }
}

/// Read `total` frames of a cursor in order the way the mixer does (one acquire load per block), return them.
fn drain(c: &mut fn_audio::stream::StreamCursor, total: u64, block: u64) -> Vec<f32> {
    let mut out = Vec::with_capacity(total as usize * 2);
    let mut pos = 0u64;
    let t = Instant::now();
    while pos < total {
        c.refresh();
        let upto = c.available().min(total).min(pos + block);
        if upto <= pos {
            assert!(t.elapsed() < Duration::from_secs(30), "worker stalled at {pos}");
            std::thread::sleep(Duration::from_micros(300));
            continue;
        }
        while pos < upto {
            let (l, r) = c.frame(pos);
            out.push(l);
            out.push(r);
            pos += 1;
        }
        c.consumed(pos);
    }
    out
}

#[test]
fn worker_ring_delivers_the_loop_bit_exactly_and_slots_recycle() {
    let Some(_) = install() else { return };
    let (_, whole, src) = load(OGG_DAY);
    let n = src.frames;
    let sw = StreamedWave::new(src.clone()).unwrap();
    let t = Instant::now();
    assert!(sw.wait_primed(1, Duration::from_secs(5)));
    eprintln!("first slot primed {:.1} ms after StreamedWave::new", t.elapsed().as_secs_f64() * 1e3);
    // two concurrent looping playheads (the Day cue plays Wind04 twice), 2.6 passes each: ring wraps 10 times, loop wraps twice
    let total = n * 2 + n * 6 / 10;
    let mut a = sw.open(0, true).expect("slot a");
    let mut b = sw.open(0, true).expect("slot b");
    let (ta, tb) = std::thread::scope(|s| {
        let ha = s.spawn(|| drain(&mut a, total, 480));
        let hb = s.spawn(|| drain(&mut b, total, 1024));
        (ha.join().unwrap(), hb.join().unwrap())
    });
    let looped: Vec<f32> = whole.iter().cycle().take(total as usize * 2).copied().collect();
    assert_eq!(bits_equal(&ta, &looped), None, "playhead a");
    assert_eq!(bits_equal(&tb, &looped), None, "playhead b");
    let st = sw.stats();
    eprintln!("stats after 2 looping playheads x {total} frames: {st:?}");
    assert!(st.loop_wraps >= 4, "{st:?}");
    // recycle: dropping the cursors frees the slots; a new one starts at frame 0 again
    drop(a);
    drop(b);
    assert!(sw.wait_primed(2, Duration::from_secs(5)), "released slots are reset and primed again");
    let mut c = sw.open(0, false).expect("recycled slot");
    let tc = drain(&mut c, n, 700);
    assert_eq!(bits_equal(&tc, &whole), None, "recycled slot replays from frame 0");
    // a non-looping cursor sees the end of the wave
    c.refresh();
    let mut waited = Instant::now();
    while c.end() == u64::MAX && waited.elapsed() < Duration::from_secs(5) {
        c.refresh();
        std::thread::sleep(Duration::from_millis(1));
        waited = waited;
    }
    assert_eq!(c.end(), n);
    drop(c);
    // start at an arbitrary offset (Ogg: exact)
    let off = 123_457u64;
    let mut d = sw.open(off, false).expect("offset slot");
    let td = drain(&mut d, n - off, 512);
    assert_eq!(bits_equal(&td, &whole[off as usize * 2..]), None, "start at {off}");
    // every slot busy -> None, counted, no allocation or panic
    let held: Vec<_> = (0..16).filter_map(|_| sw.open(0, true)).collect();
    assert!(held.len() <= fn_audio::stream::SLOTS);
    assert!(sw.open(0, true).is_none());
    assert!(sw.stats().starved >= 1);
}

#[test]
fn offline_render_of_the_ambience_cue_is_bit_identical_streamed_or_whole() {
    let Some(r) = root() else { return };
    let render = |streamed: bool| {
        let mut lib = Library::open(&r).unwrap();
        if streamed {
            lib.stream_min_seconds = Some(4.0);
        }
        let mut eng = AudioEngine::new(48000);
        let h = eng.handle();
        assert_eq!(lib.prepare_cue(&h, events::AMBIENCE_DAY).unwrap(), 0);
        h.play_cue(events::AMBIENCE_DAY, PlayParams::default()).unwrap();
        // 26 s covers two loop wraps of the 10.9 s / 12.2 s / 13.9 s waves, pitch-shifted voices included
        let out = eng.render_offline(48000 * 26, 7);
        (out, eng.mixer().stats)
    };
    let t = Instant::now();
    let (whole, _) = render(false);
    let t_whole = t.elapsed();
    let t = Instant::now();
    let (streamed, st) = render(true);
    let t_streamed = t.elapsed();
    assert_eq!(st.stream_starved, 0);
    let peak = whole.iter().fold(0f32, |a, &b| a.max(b.abs()));
    assert!(peak > 1e-3, "ambience is audible ({peak})");
    let differing = whole.iter().zip(&streamed).filter(|(a, b)| a.to_bits() != b.to_bits()).count();
    let max = whole.iter().zip(&streamed).fold(0f32, |m, (a, b)| m.max((a - b).abs()));
    eprintln!("26 s offline render of the Day ambience: whole {:.2} s (load+render), streamed {:.2} s; peak {peak:.4}; differing samples {differing} of {}, max |diff| {max:e}", t_whole.as_secs_f64(), t_streamed.as_secs_f64(), whole.len());
    assert_eq!(differing, 0, "streamed render equals the whole-decode render sample for sample");
}

fn pct(v: &mut Vec<f64>, p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p) as usize]
}

/// Time to first sound of the ambience cue + the real-time behaviour (paced like the audio callback).
#[test]
fn ambience_time_to_first_sound_and_paced_playback() {
    let Some(r) = root() else { return };
    // content onset: the first non-silent frame of the cue itself (an Enveloper may fade in), from a whole-decode render
    let onset_frames = {
        let mut lib = Library::open(&r).unwrap();
        let mut eng = AudioEngine::new(48000);
        let h = eng.handle();
        lib.prepare_cue(&h, events::AMBIENCE_DAY).unwrap();
        h.play_cue(events::AMBIENCE_DAY, PlayParams::default()).unwrap();
        let o = eng.render_offline(48000, 7);
        o.chunks(2).position(|f| f[0].abs() > 1e-6 || f[1].abs() > 1e-6).unwrap_or(usize::MAX)
    };
    eprintln!("content onset of the cue: {} frames at 48 kHz ({:.1} ms)", onset_frames, onset_frames as f64 / 48.0);
    for streamed in [false, true] {
        let t_mount = Instant::now();
        let mut lib = Library::open(&r).unwrap();
        let mount = t_mount.elapsed();
        if streamed {
            lib.stream_min_seconds = Some(4.0);
        }
        let eng = AudioEngine::new(48000);
        let (h, mut mixer) = eng.into_parts();
        // the sink loads its gameplay cues first, so the SoundClass tree is already cached when the ambience loads
        let t_cls = Instant::now();
        lib.classes();
        let classes = t_cls.elapsed();
        let t0 = Instant::now();
        let t_cue = Instant::now();
        lib.load_cue(events::AMBIENCE_DAY).unwrap();
        let cue_ms = t_cue.elapsed();
        assert_eq!(lib.prepare_cue(&h, events::AMBIENCE_DAY).unwrap(), 0);
        let prepared = t0.elapsed();
        eprintln!("  phases: SoundClass tree {:.1} ms (before t0), cue parse {:.1} ms, cue parse + waves + queue {:.1} ms", classes.as_secs_f64() * 1e3, cue_ms.as_secs_f64() * 1e3, prepared.as_secs_f64() * 1e3);
        h.play_cue(events::AMBIENCE_DAY, PlayParams::default()).unwrap();
        // render 10 ms blocks at the callback cadence
        let mut buf = vec![0f32; 480 * 2];
        let mut first: Option<Duration> = None;
        let mut cost_us = Vec::new();
        let start = Instant::now();
        let mut block = 0u32;
        while start.elapsed() < Duration::from_secs(8) {
            let t = Instant::now();
            mixer.render(&mut buf);
            cost_us.push(t.elapsed().as_secs_f64() * 1e6);
            if first.is_none() && buf.iter().any(|s| s.abs() > 1e-6) {
                first = Some(t0.elapsed());
            }
            block += 1;
            let due = start + Duration::from_millis(10) * block;
            if let Some(w) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(w);
            }
        }
        let first = first.expect("sound within 8 s");
        let mut c = cost_us.clone();
        let (p50, p99, mx) = (pct(&mut c, 0.5), pct(&mut c, 0.99), cost_us.iter().cloned().fold(0.0, f64::max));
        eprintln!(
            "{}: mount {:.0} ms; prepare_cue {:.1} ms; first sound {:.1} ms after the load started (onset in the content {:.1} ms); audio callback (480 frames) p50 {p50:.0} us p99 {p99:.0} us max {mx:.0} us; mixer stats {:?}",
            if streamed { "STREAMED" } else { "WHOLE   " },
            mount.as_secs_f64() * 1e3,
            prepared.as_secs_f64() * 1e3,
            first.as_secs_f64() * 1e3,
            onset_frames as f64 / 48.0,
            mixer.stats
        );
        if streamed {
            for (k, s) in lib.stream_stats() {
                eprintln!("  {k}: {s:?}");
            }
        }
        if streamed {
            assert!(first < Duration::from_millis(100), "time to first sound {first:?}");
            assert_eq!(mixer.stats.stream_starved, 0);
        }
    }
}
