//! Drives the real binary: pure-WAV commands always, install-backed commands when the 1.8 install is present.

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_fn-audio-cli");
const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

fn root() -> Option<String> {
    let r = std::env::var("FN_ROOT").unwrap_or_else(|_| DEFAULT_ROOT.to_string());
    Path::new(&r).join("Content").join("Paks").exists().then_some(r)
}

fn run(args: &[&str]) -> (bool, String, String) {
    let o = Command::new(BIN).args(args).output().expect("spawn fn-audio-cli");
    (o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join("fn-audio-cli-tests");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name).to_string_lossy().into_owned()
}

#[test]
fn usage_on_no_args_and_unknown_command() {
    let (ok, _, err) = run(&[]);
    assert!(!ok && err.contains("usage"));
    let (ok, _, err) = run(&["frobnicate"]);
    assert!(!ok && err.contains("usage"));
}

#[test]
fn analyze_reads_a_wav_and_reports_the_dominant_frequency() {
    // 1 s, 24 kHz, mono 1 kHz sine written by hand
    let n = 24000usize;
    let mut f = Vec::new();
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&(36 + n as u32 * 2).to_le_bytes());
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&16u32.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&24000u32.to_le_bytes());
    f.extend_from_slice(&48000u32.to_le_bytes());
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(n as u32 * 2).to_le_bytes());
    for i in 0..n {
        let s = ((2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 24000.0).sin() * 16000.0) as i16;
        f.extend_from_slice(&s.to_le_bytes());
    }
    let p = tmp("sine.wav");
    std::fs::write(&p, f).unwrap();
    let (ok, out, err) = run(&["analyze", &p]);
    assert!(ok, "{err}");
    assert!(out.contains("24000 Hz, 1 ch, 24000 frames"), "{out}");
    assert!(out.contains("dominant 1000 Hz") || out.contains("dominant 999 Hz") || out.contains("dominant 1001 Hz") || out.contains("dominant 1003 Hz") || out.contains("dominant 998 Hz"), "{out}");
    assert!(run(&["analyze", "/definitely/not/there.wav"]).0 == false);
}

#[test]
fn install_commands_decode_dump_and_render() {
    let Some(r) = root() else {
        eprintln!("SKIP: no install");
        return;
    };
    let wav = tmp("pe.wav");
    let (ok, out, err) = run(&["--root", &r, "decode", "FortniteGame/Content/Sounds/Fort_Weapons/PickAxe/PickAxe_Enemy_001", &wav]);
    assert!(ok, "{err}");
    assert!(out.contains("OGG 24000 Hz, 2 ch, 11599 frames"), "{out}");
    let (ok, out, _) = run(&["analyze", &wav]);
    assert!(ok && out.contains("11599 frames"), "{out}");

    let (ok, out, err) = run(&["--root", &r, "cue", "/Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue"]);
    assert!(ok, "{err}");
    assert!(out.contains("QualityLevel") && out.contains("Random") && out.contains("Attenuation") && out.contains("waves: 12") && !out.contains("<MISSING>"), "{out}");

    let swing = tmp("swing.wav");
    let (ok, out, err) = run(&["--root", &r, "render", "/Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue", &swing, "--seed", "7", "--secs", "1.5"]);
    assert!(ok, "{err}");
    assert!(out.contains("rendered 72000 frames at 48000 Hz (seed 7)"), "{out}");
    let a = std::fs::read(&swing).unwrap();
    let (ok, _, _) = run(&["--root", &r, "render", "/Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue", &swing, "--seed", "7", "--secs", "1.5"]);
    assert!(ok);
    assert_eq!(a, std::fs::read(&swing).unwrap(), "same seed -> identical WAV bytes");
}
