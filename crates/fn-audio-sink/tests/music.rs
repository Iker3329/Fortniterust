//! MUSIC HOOK: the front-end music + loading-screen mix on the engine sink, against the real 1.8 install (skipped when it is missing; `FN_ROOT` overrides).
//! Logic checks only: levels, fades, sliders, ordering of requests vs the loader. Rendered offline (deterministic), no device.

use fn_audio::library::Library;
use fn_audio::mixer::AudioEngine;
use fn_audio_sink::{AudioSink, EngineSink, MusicCue};
use std::path::Path;

const ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";
const RATE: usize = 48000;

fn root() -> Option<String> {
    let r = std::env::var("FN_ROOT").unwrap_or_else(|_| ROOT.to_string());
    Path::new(&r).join("Content").join("Paks").exists().then_some(r)
}

/// rms of the stereo block `[from, to)` seconds of a render
fn rms(o: &[f32], from: f32, to: f32) -> f32 {
    let (a, b) = ((from * RATE as f32) as usize * 2, (to * RATE as f32) as usize * 2);
    let s: f64 = o[a..b.min(o.len())].iter().map(|&x| (x as f64) * (x as f64)).sum();
    (s / (b.min(o.len()) - a) as f64).sqrt() as f32
}
fn peak(o: &[f32]) -> f32 {
    o.iter().fold(0f32, |m, &x| m.max(x.abs()))
}
fn secs(eng: &mut AudioEngine, s: f32) -> Vec<f32> {
    eng.render_offline((s * RATE as f32) as usize, 1)
}

/// A sink whose loader (the game's worker thread) ran `load_music`; requests made BEFORE it finished are carried out when it does.
fn sink_with_music(requests_first: impl FnOnce(&mut EngineSink)) -> Option<(AudioEngine, EngineSink)> {
    let r = root()?;
    let mut eng = AudioEngine::new(RATE as u32);
    let mut sink = EngineSink::new(eng.handle());
    requests_first(&mut sink);
    assert!(!sink.music_status().0, "nothing is loaded yet");
    let loader = sink.loader();
    let t = std::thread::spawn(move || {
        let mut lib = Library::open(r).unwrap();
        let missing = loader.load_music(&mut lib);
        assert!(lib.is_streamed("/Game/Sounds/Fort_Music/Menu/fort_lobby_music_athena"), "the 115 s lobby track is streamed, not decoded whole on the loader");
        assert!(lib.is_streamed("/Game/Sounds/Fort_Music/Menu/fortnite_login_screen"));
        assert!(!lib.is_streamed("/Game/Sounds/Fort_Ambient_Looping/Mountain_Exterior/Mountain_Wind06"), "the 9 s wind loop decodes whole");
        missing
    });
    let missing = t.join().unwrap();
    assert!(missing.is_empty(), "{missing:?}");
    let _ = &mut eng;
    Some((eng, sink))
}

#[test]
fn lobby_music_requested_before_the_loader_finishes_plays_once_it_is_ready() {
    let Some((mut eng, mut sink)) = sink_with_music(|s| s.set_music(Some(MusicCue::LobbyAthena))) else { return eprintln!("SKIP: no install") };
    assert_eq!(sink.music_status(), (true, true, false));
    let o = secs(&mut eng, 3.0);
    let p = peak(&o);
    // class chain 0.35 (Master 0.7 x Music 0.5, Fort_Music_Menu_PSM 1.0) x cue VolumeMultiplier 0.9, 2D (no attenuation)
    assert!(p > 0.01 && p <= 0.35 * 0.9 + 1e-3, "lobby music peak {p}");
    sink.set_music(None);
}

#[test]
fn music_slider_master_slider_and_the_sfx_slider() {
    // The lobby cue is a 115-second arranged music track, so adjacent one
    // second windows naturally have different RMS levels.  Render every
    // slider case from a fresh engine at the same cue position instead of
    // treating a later musical phrase as a volume regression.
    let level = |master, music, sfx, dialog, voice| -> Option<f32> {
        let (mut eng, mut sink) = sink_with_music(|s| s.set_music(Some(MusicCue::LobbyAthena)))?;
        sink.set_volumes(master, music, sfx, dialog, voice);
        secs(&mut eng, 1.0);
        Some(rms(&secs(&mut eng, 1.0), 0.25, 1.0))
    };
    let Some(base) = level(1.0, 1.0, 1.0, 1.0, 1.0) else { return eprintln!("SKIP: no install") };
    assert!(base > 1e-3, "{base}");
    let half = level(1.0, 0.5, 1.0, 1.0, 1.0).unwrap(); // Music slider 0.5
    assert!((half / base - 0.5).abs() < 0.1, "music slider 0.5 -> x{}", half / base);
    let other = level(1.0, 1.0, 0.0, 0.0, 0.0).unwrap(); // SFX / Dialog / Voice chat sliders do not touch music
    assert!((other / base - 1.0).abs() < 0.1, "other sliders: x{}", other / base);
    let master = level(0.5, 1.0, 1.0, 1.0, 1.0).unwrap(); // Master 0.5
    assert!((master / base - 0.5).abs() < 0.1, "master 0.5 -> x{}", master / base);
    let (mut eng, mut sink) = sink_with_music(|s| s.set_music(Some(MusicCue::LobbyAthena))).unwrap();
    sink.set_volumes(1.0, 0.0, 1.0, 1.0, 1.0); // Music 0
    assert!(peak(&secs(&mut eng, 0.5)) < 1e-6, "music slider 0 is silent");
}

#[test]
fn stopping_fades_out_over_one_second_and_a_new_track_crossfades() {
    let Some((mut eng, mut sink)) = sink_with_music(|_| {}) else { return eprintln!("SKIP: no install") };
    sink.set_music(Some(MusicCue::LobbyAthena));
    secs(&mut eng, 1.0);
    let full = rms(&secs(&mut eng, 0.5), 0.0, 0.5);
    sink.set_music(None);
    let o = secs(&mut eng, 1.3);
    // linear gain 1 -> 0 over FadeOutTime = 1.0 s: the middle of the fade is about half level, the end is silence
    let mid = rms(&o, 0.45, 0.55);
    assert!(mid > full * 0.25 && mid < full * 0.75, "mid fade {mid} vs full {full}");
    assert!(peak(&o[(1.1 * RATE as f32) as usize * 2..]) < 1e-5, "silent after the fade");
    assert_eq!(sink.music_status().1, false);

    // login -> lobby: the new track is at full level at once, the old one fades under it for 1 s; afterwards only the new one plays
    sink.set_music(Some(MusicCue::Login));
    secs(&mut eng, 1.5);
    sink.set_music(Some(MusicCue::LobbyAthena));
    let o = secs(&mut eng, 2.0);
    assert!(rms(&o, 0.0, 0.2) > 1e-3 && rms(&o, 1.5, 2.0) > 1e-3);
    assert_eq!(sink.music_status().1, true);
    sink.set_music(None);
    secs(&mut eng, 1.3);
}

#[test]
fn the_loading_screen_mix_silences_music_with_the_recovered_fades() {
    let Some((mut eng, mut sink)) = sink_with_music(|s| {
        s.set_loading_screen(true); // pushed while the loader has not even started: remembered
        s.set_music(Some(MusicCue::LobbyAthena));
    }) else {
        return eprintln!("SKIP: no install");
    };
    assert_eq!(sink.music_status(), (true, true, true));
    // Fort_Loading_Mix: Music -> 0.0, fade in 0.5 s: audible at the very start, gone after the fade, silent while the screen is up
    let o = secs(&mut eng, 1.5);
    assert!(rms(&o, 0.0, 0.1) > 1e-3, "starts audible (the mix fades in over 0.5 s)");
    assert!(peak(&o[(0.6 * RATE as f32) as usize * 2..]) < 1e-5, "loading screen: music is silent");
    // popped: fade out 1.0 s -> the music comes back linearly
    sink.set_loading_screen(false);
    let o = secs(&mut eng, 1.5);
    let early = rms(&o, 0.0, 0.1);
    let mid = rms(&o, 0.45, 0.55);
    let late = rms(&o, 1.2, 1.5);
    assert!(early < mid && mid < late, "music returns over the 1.0 s fade: {early} < {mid} < {late}");
    assert!(late > 1e-3);
    assert_eq!(sink.music_status().2, false);
    // pushing twice / popping twice is harmless
    sink.set_loading_screen(false);
    sink.set_loading_screen(true);
    sink.set_loading_screen(true);
    secs(&mut eng, 0.8);
    assert!(peak(&secs(&mut eng, 0.5)) < 1e-5);
}
