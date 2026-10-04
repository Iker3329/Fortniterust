//! Self-locating game folders (replaces the old `dist\Launch FortniteRuntime.cmd`, which only passed `--assets` / `--paks` to the exe).
//!
//! When `--assets` / `--paks` are absent the exe looks, in this order, at
//!
//! 1. a small `FortniteRuntime.toml` (or `paths.txt`) **next to the exe**:
//!    ```text
//!    assets = "C:\\path\\to\\assets"          # exported asset tree (glTF / PNG / md5anim)
//!    paks   = 'C:\path\to\FortniteGame'       # Fortnite 1.8 install or its Paks folder
//!    ```
//!    (`key = value`, `#` comments, double quotes with `\\` escapes or single quotes or bare; relative paths are relative to the exe; in `paths.txt` two bare lines
//!    mean assets then paks; the keys `install` / `fortnite` / `game` also name the paks entry);
//! 2. for an **interactive** launch only (never for scripted / diagnostic runs, which must stay reproducible): the known default locations of the developer's machine
//!    ([`DEFAULT_ASSETS`], [`DEFAULT_PAKS`]) plus a `FortniteRuntime\assets` folder next to any parent of the exe, used only when nothing else was found.
//!
//! Explicit flags always win (the value is only filled in when the flag is absent), and the environment (`FN_ASSETS`, `FN_PAKS`, `FN_ROOT`) and the `assets` /
//! `Paks` folders beside the exe keep their place in `assets.rs`. `assets.rs` itself stays free of any baked-in path (a test pins that); the defaults live here.
use crate::cli::Options;
use std::path::{Path, PathBuf};

pub const CONFIG_NAMES: [&str; 2] = ["FortniteRuntime.toml", "paths.txt"];
/// The developer machine's shared exported asset tree (the old launcher's `--assets`).
pub const DEFAULT_ASSETS: &str = r"C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\assets";
/// The developer machine's Fortnite 1.8 install (the old launcher's `--paks`).
pub const DEFAULT_PAKS: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Configured { pub assets: Option<PathBuf>, pub paks: Option<PathBuf>, /// the file the values came from
    pub source: Option<PathBuf> }

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        let mut out = String::new(); let mut it = v[1..v.len() - 1].chars();
        while let Some(c) = it.next() { if c == '\\' { match it.next() { Some('\\') => out.push('\\'), Some('"') => out.push('"'), Some(o) => { out.push('\\'); out.push(o); } None => out.push('\\') } } else { out.push(c); } }
        out
    } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') { v[1..v.len() - 1].to_owned() }
    else { v.split(" #").next().unwrap_or(v).trim().to_owned() }
}

/// Parses the text of `FortniteRuntime.toml` / `paths.txt`; relative paths are resolved against `base` (the exe's folder).
pub fn parse(text: &str, base: &Path) -> Configured {
    let mut c = Configured::default();
    let mut bare = 0;
    let full = |s: String| -> Option<PathBuf> { if s.is_empty() { None } else { let p = PathBuf::from(s); Some(if p.is_absolute() { p } else { base.join(p) }) } };
    for line in text.trim_start_matches('\u{feff}').lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with('[') { continue; }
        match l.split_once('=') {
            Some((k, v)) => match k.trim().to_ascii_lowercase().as_str() {
                "assets" | "assets_dir" | "asset_dir" => c.assets = full(unquote(v)),
                "paks" | "paks_dir" | "install" | "fortnite" | "fortnite_game" | "game" => c.paks = full(unquote(v)),
                _ => {}
            },
            None => { match bare { 0 => c.assets = full(unquote(l)), 1 => c.paks = full(unquote(l)), _ => {} } bare += 1; }
        }
    }
    c
}

/// The first of [`CONFIG_NAMES`] that exists next to the exe, parsed.
pub fn load(exe_dir: &Path) -> Configured {
    for n in CONFIG_NAMES {
        let p = exe_dir.join(n);
        if let Ok(t) = std::fs::read_to_string(&p) { let mut c = parse(&t, exe_dir); c.source = Some(p); return c; }
    }
    Configured::default()
}

/// The known default places, injectable for tests.
#[derive(Clone, Debug)]
pub struct Known { pub assets: PathBuf, pub paks: PathBuf }
impl Known { pub fn developer() -> Known { Known { assets: DEFAULT_ASSETS.into(), paks: DEFAULT_PAKS.into() } } }

/// `FortniteRuntime\assets` next to the exe's folder or one of its parents (a `dist\` exe inside a workspace that sits beside the prototype), else `known.assets`.
fn known_default_assets(exe_dir: &Path, known: &Known) -> Option<PathBuf> {
    exe_dir.ancestors().take(5).map(|a| a.join("FortniteRuntime").join("assets")).chain(std::iter::once(known.assets.clone())).find(|p| p.is_dir())
}

/// Fills in `--assets` / `--paks` when they are absent (see the module docs). Explicit flags are never touched.
pub fn apply_defaults(o: Options, exe_dir: &Path, known: &Known) -> Options {
    let mut o = o;
    let cfg = load(exe_dir);
    let interactive = !o.scripted() && !o.live_test && !o.startup_capture;
    if o.assets.is_none() {
        if let Some(a) = cfg.assets.clone().filter(|p| p.is_dir()) { eprintln!("[paths] assets from {}: {}", cfg.source.as_deref().unwrap_or(exe_dir).display(), a.display()); o.assets = Some(a); }
        else if interactive && crate::assets::candidate_roots(None).is_empty() {
            if let Some(a) = known_default_assets(exe_dir, known) { eprintln!("[paths] assets from the known default location: {}", a.display()); o.assets = Some(a); }
        }
    }
    if o.paks.is_none() {
        if let Some(p) = cfg.paks.clone().filter(|p| crate::assets::find_paks_dir(p).is_some()) { eprintln!("[paths] paks from {}: {}", cfg.source.as_deref().unwrap_or(exe_dir).display(), p.display()); o.paks = Some(p); }
        else if interactive && crate::assets::candidate_paks(None).is_empty() && crate::assets::find_paks_dir(&known.paks).is_some() {
            eprintln!("[paths] paks from the known default location: {}", known.paks.display()); o.paks = Some(known.paks.clone());
        }
    }
    o
}

/// "Looked in" lines for the missing-assets message.
pub fn describe(exe_dir: &Path) -> String {
    let c = load(exe_dir);
    let mut s = String::new();
    match &c.source { Some(p) => s.push_str(&format!("Paths file: {} (assets: {}, paks: {})\n", p.display(), c.assets.as_deref().map_or("not set".into(), |p| p.display().to_string()), c.paks.as_deref().map_or("not set".into(), |p| p.display().to_string()))),
        None => s.push_str(&format!("Paths file: none ({} or {} beside the executable can set `assets = ` / `paks = `)\n", CONFIG_NAMES[0], CONFIG_NAMES[1])) }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(name: &str) -> PathBuf { let d = std::env::temp_dir().join(format!("fn-app-paths-{name}-{}", std::process::id())); let _ = std::fs::remove_dir_all(&d); std::fs::create_dir_all(&d).unwrap(); d }
    fn nowhere() -> Known { Known { assets: PathBuf::from("Z:/fn/none/assets"), paks: PathBuf::from("Z:/fn/none/paks") } }

    #[test] fn parses_toml_and_bare_forms() {
        let base = Path::new("C:/game");
        let c = parse("# paths\n[paths]\nassets = \"D:\\\\fn\\\\assets\"\npaks = 'E:\\Fortnite\\FortniteGame'  \n", base);
        assert_eq!(c.assets, Some(PathBuf::from("D:\\fn\\assets"))); assert_eq!(c.paks, Some(PathBuf::from("E:\\Fortnite\\FortniteGame")));
        let r = parse("ASSETS = shared/assets # comment\ninstall = ../fortnite\n", base);
        assert_eq!(r.assets, Some(base.join("shared/assets"))); assert_eq!(r.paks, Some(base.join("../fortnite")));
        let b = parse("\u{feff}D:/a\nD:/p\nD:/ignored\n", base);
        assert_eq!((b.assets, b.paks), (Some(PathBuf::from("D:/a")), Some(PathBuf::from("D:/p"))));
        assert_eq!(parse("unknown = 1\n", base), Configured::default());
    }
    #[test] fn the_file_beside_the_exe_is_found_toml_first() {
        let d = tmp("load"); assert!(load(&d).source.is_none());
        std::fs::write(d.join("paths.txt"), "assets = A\n").unwrap(); assert_eq!(load(&d).assets, Some(d.join("A")));
        std::fs::write(d.join("FortniteRuntime.toml"), "assets = B\n").unwrap(); assert_eq!(load(&d).assets, Some(d.join("B")));
        let _ = std::fs::remove_dir_all(d);
    }
    #[test] fn explicit_flags_win_over_the_file_and_the_defaults() {
        let d = tmp("flags"); let (fa, fp) = (d.join("flag-assets"), d.join("flag-paks")); let (ca, cp) = (d.join("cfg-assets"), d.join("cfg-paks"));
        for p in [&fa, &ca, &cp] { std::fs::create_dir_all(p).unwrap(); } std::fs::create_dir_all(&fp).unwrap(); std::fs::write(cp.join("x.pak"), b"x").unwrap();
        std::fs::write(d.join("FortniteRuntime.toml"), format!("assets = '{}'\npaks = '{}'\n", ca.display(), cp.display())).unwrap();
        let o = apply_defaults(Options { assets: Some(fa.clone()), paks: Some(fp.clone()), ..Default::default() }, &d, &nowhere());
        assert_eq!((o.assets, o.paks), (Some(fa), Some(fp)), "flags are never replaced");
        let o = apply_defaults(Options::default(), &d, &nowhere());
        assert_eq!((o.assets, o.paks), (Some(ca), Some(cp)), "absent flags are filled from the file");
        let _ = std::fs::remove_dir_all(d);
    }
    #[test] fn a_file_entry_that_does_not_exist_is_ignored() {
        let d = tmp("missing"); std::fs::write(d.join("FortniteRuntime.toml"), "assets = nope\npaks = nope\n").unwrap();
        let o = apply_defaults(Options::default(), &d, &nowhere()); assert!(o.assets.is_none() && o.paks.is_none());
        let _ = std::fs::remove_dir_all(d);
    }
    #[test] fn known_defaults_apply_to_interactive_launches_only() {
        let d = tmp("known"); let (ka, kp) = (d.join("ka"), d.join("kp")); std::fs::create_dir_all(&ka).unwrap(); std::fs::create_dir_all(&kp).unwrap(); std::fs::write(kp.join("x.pak"), b"x").unwrap();
        let exe = d.join("bin"); std::fs::create_dir_all(&exe).unwrap();
        let known = Known { assets: ka.clone(), paks: kp.clone() };
        let o = apply_defaults(Options::default(), &exe, &known);
        assert_eq!(o.assets.as_deref(), Some(ka.as_path())); assert_eq!(o.paks.as_deref(), Some(kp.as_path()));
        for scripted in [Options { smoke: true, ..Default::default() }, Options { live_test: true, ..Default::default() }, Options { startup_capture: true, ..Default::default() }] {
            let o = apply_defaults(scripted, &exe, &known); assert!(o.assets.is_none() && o.paks.is_none(), "scripted runs never use baked defaults");
        }
        let _ = std::fs::remove_dir_all(d);
    }
    #[test] fn a_sibling_prototype_assets_folder_is_found_from_a_parent_of_the_exe() {
        let d = tmp("sibling"); let a = d.join("FortniteRuntime").join("assets"); std::fs::create_dir_all(&a).unwrap(); let exe = d.join("rs").join("dist"); std::fs::create_dir_all(&exe).unwrap();
        assert_eq!(known_default_assets(&exe, &nowhere()).as_deref(), Some(a.as_path()));
        let _ = std::fs::remove_dir_all(d);
    }
}
