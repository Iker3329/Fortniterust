//! Runner: build, prepare the old exe's scratch copy, run the scripted modes of both executables, collect their outputs in `parity/work`.
use crate::exec::{mirror_dir, sync_file, Ctx};
use crate::scenarios;
use std::path::{Path, PathBuf};

struct OldMode { id: &'static str, args: &'static [&'static str], timeout: f64, stable_file: Option<&'static str> }
const OLD_MODES: &[OldMode] = &[
    OldMode { id: "smoke-test", args: &["--smoke-test"], timeout: 90., stable_file: None },
    OldMode { id: "dir-test", args: &["--dir-test"], timeout: 150., stable_file: Some("dir-frames.txt") },
    OldMode { id: "pickaxe-test", args: &["--pickaxe-test"], timeout: 120., stable_file: Some("pickaxe-frames.txt") },
    OldMode { id: "lighting-test", args: &["--lighting-test"], timeout: 90., stable_file: None },
    OldMode { id: "sky-test", args: &["--sky-test"], timeout: 150., stable_file: None },
    OldMode { id: "menu-test", args: &["--menu-test"], timeout: 90., stable_file: None },
    OldMode { id: "settings18-capture", args: &["--settings18-capture"], timeout: 120., stable_file: None },
    OldMode { id: "reticle-test", args: &["--reticle-test"], timeout: 45., stable_file: None },
    OldMode { id: "jump-feel-test", args: &["--jump-feel-test"], timeout: 30., stable_file: None },
    OldMode { id: "time-test", args: &["--time-test"], timeout: 90., stable_file: None },
    OldMode { id: "face-test", args: &["--face-test"], timeout: 90., stable_file: None },
];

pub fn dir_has_files(d: &Path) -> bool { std::fs::read_dir(d).map_or(false, |mut r| r.next().is_some()) }

fn rm_dir(d: &Path) { for _ in 0..5 { if !d.exists() || std::fs::remove_dir_all(d).is_ok() { return; } std::thread::sleep(std::time::Duration::from_millis(300)); } }

fn copy_tree(from: &Path, to: &Path) {
    let _ = std::fs::create_dir_all(to);
    if let Ok(rd) = std::fs::read_dir(from) { for e in rd.flatten() { let (p, q) = (e.path(), to.join(e.file_name())); if p.is_dir() { copy_tree(&p, &q); } else { let _ = std::fs::copy(&p, &q); } } }
}

/// Build the binaries the dashboard needs; on failure the previous binaries are kept (and flagged) because other agents edit the crates concurrently.
pub fn stage_build(ctx: &mut Ctx) {
    if ctx.o.no_build { eprintln!("[build] skipped (--no-build)"); return; }
    for (pkg, exe) in [("fn-app", "fortnite-rs"), ("fn-sim", "fn-sim"), ("fn-ui-demo", "fn-ui-demo")] {
        let (ok, err) = ctx.cargo_build(&[pkg], 3600);
        if !ok {
            let p = ctx.new_exe(exe);
            if p.exists() {
                let age = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map_or("?".to_owned(), |d| format!("{:.0} min", d.as_secs_f64() / 60.));
                ctx.warn(format!("build of {pkg} FAILED ({err}); using the stale {exe}.exe built {age} ago, so results lag the sources"));
            } else { ctx.warn(format!("build of {pkg} FAILED ({err}) and no previous {exe}.exe exists: its scenarios are skipped")); }
        }
    }
}

/// Idempotent scratch copy of the old exe (never run from the original folder).
pub fn prepare_old(ctx: &mut Ctx) -> bool {
    let (src, dst) = (ctx.o.old_tree.clone(), ctx.o.scratch_old.clone());
    if src == dst { ctx.warn("scratch folder equals the old tree: refusing to run the old exe from its original location"); return false; }
    let mut exe = ctx.o.old_exe.clone().unwrap_or_else(|| src.join("FortniteRuntime.exe"));
    if ctx.o.old_build {
        match build_old_snapshot(ctx) { Some(p) => exe = p, None => ctx.warn("--old-build: the snapshot build of the old sources failed; falling back to the old tree's exe") }
    }
    if !exe.exists() { ctx.warn(format!("old exe not found at {}", exe.display())); return false; }
    // staleness: the old tree is still being edited; an exe older than its sources is not what the rewrite was ported from
    if exe == src.join("FortniteRuntime.exe") {
        let newest = newest_mtime(&src.join("src")).max(newest_mtime_file(&src.join("Cargo.toml")));
        if let (Some(n), Ok(m)) = (newest, std::fs::metadata(&exe).and_then(|m| m.modified())) {
            if let Ok(d) = n.duration_since(m) { if d.as_secs() > 120 { ctx.warn(format!("the old exe ({}) is {:.1} h older than the newest file in the old tree's src: its behaviour can differ from the sources the rewrite was ported from (e.g. harvest HP, pelvis offsets); use --old-build to build a fresh snapshot exe in the scratch folder or --old-exe to pick another one", exe.display(), d.as_secs_f64() / 3600.)); } }
        }
    }
    if let Err(e) = std::fs::create_dir_all(&dst) { ctx.warn(format!("cannot create {}: {e}", dst.display())); return false; }
    match sync_file(&exe, &dst.join("FortniteRuntime.exe")) { Ok(c) => if c { eprintln!("[old] copied {} to scratch", exe.display()) }, Err(e) => { ctx.warn(e); return false; } }
    let scratch_exe = dst.join("FortniteRuntime.exe");
    let (size, mtime) = std::fs::metadata(&exe).map(|m| (m.len(), m.modified().ok())).unwrap_or((0, None));
    let _ = std::fs::write(ctx.work.join("old-exe-source.json"), serde_json::json!({ "source": exe.display().to_string(), "size": size, "source_mtime_unix": mtime.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()), "scratch_fnv64": crate::exec::fingerprint(&scratch_exe) }).to_string());
    let _ = sync_file(&src.join("FortniteSettings.ini"), &dst.join("FortniteSettings.ini.orig"));
    if !dst.join("FortniteSettings.ini").exists() { let _ = std::fs::copy(src.join("FortniteSettings.ini"), dst.join("FortniteSettings.ini")); }
    // always start from the original settings
    let _ = std::fs::copy(dst.join("FortniteSettings.ini.orig"), dst.join("FortniteSettings.ini"));
    if !mirror_dir(&src.join("assets"), &dst.join("assets")) { ctx.warn("robocopy of the old assets reported an error"); }
    true
}

fn read(p: &Path) -> String { std::fs::read_to_string(p).unwrap_or_default() }

fn newest_mtime_file(p: &Path) -> Option<std::time::SystemTime> { std::fs::metadata(p).and_then(|m| m.modified()).ok() }
fn newest_mtime(d: &Path) -> Option<std::time::SystemTime> {
    let mut best: Option<std::time::SystemTime> = None;
    if let Ok(rd) = std::fs::read_dir(d) { for e in rd.flatten() { let p = e.path(); let t = if p.is_dir() { newest_mtime(&p) } else if p.extension().map_or(false, |x| x == "rs" || x == "glsl" || x == "vert" || x == "frag") { newest_mtime_file(&p) } else { None }; if t > best { best = t; } } }
    best
}

/// Snapshot build of the old prototype: its sources (+ Cargo files, + a copy of `reports`, which the code `include_str!`s) are mirrored into a scratch
/// folder and built there with a separate target dir. Nothing in the old tree is touched. The old tree is edited concurrently, so a snapshot can fail
/// to compile; the caller then falls back to the tree's exe.
pub fn build_old_snapshot(ctx: &mut Ctx) -> Option<PathBuf> {
    let tmp = ctx.o.scratch_old.parent().map(|p| p.to_path_buf()).unwrap_or_else(std::env::temp_dir);
    let (snap, target) = (tmp.join("parity-old-src"), tmp.join("cargo-old-parity"));
    let src = ctx.o.old_tree.clone();
    if !crate::exec::mirror_dir_purge(&src.join("src"), &snap.join("src")) { ctx.warn("snapshot: robocopy of the old src failed"); return None; }
    for f in ["Cargo.toml", "Cargo.lock"] { if std::fs::copy(src.join(f), snap.join(f)).is_err() { ctx.warn(format!("snapshot: cannot copy {f}")); return None; } }
    if !crate::exec::mirror_dir(&src.join("reports"), &snap.join("reports")) { ctx.warn("snapshot: robocopy of the old reports failed"); return None; }
    let (ok, err) = ctx.cargo_build_in(&snap, &target, &[], 1800);
    if !ok { ctx.warn(format!("snapshot build failed: {err}")); return None; }
    let exe = target.join("release").join("fortnite-runtime.exe");
    exe.exists().then_some(exe)
}

fn run_old_once(ctx: &mut Ctx, m: &OldMode, tag: &str, dest: &Path) -> bool {
    let scratch = ctx.o.scratch_old.clone();
    rm_dir(&scratch.join("verification"));
    let args: Vec<String> = m.args.iter().map(|s| s.to_string()).collect();
    let id = format!("{}{}", m.id, tag);
    let exe = ctx.old_exe();
    let rec = ctx.run_proc(&id, "old", &exe, &args, &scratch, m.timeout, &[], &["FNRT_*"], dest);
    rm_dir(dest);
    let v = scratch.join("verification");
    if v.exists() { copy_tree(&v, dest); }
    rm_dir(&scratch.join("verification"));
    rec.ok
}

pub fn stage_old(ctx: &mut Ctx) {
    if ctx.o.skip_old { return; }
    if !prepare_old(ctx) { return; }
    let repeat = ctx.cfg.f("run.old_repeat", 2.) as usize;
    for m in OLD_MODES {
        let dest = ctx.work_old(m.id);
        if m.stable_file.is_some() && repeat >= 2 {
            // window-mode runs read the real keyboard while their window is foreground (see testdata/old-logs/README.txt): keep a run only when the
            // frame log is identical to the run before it
            let a = ctx.work.join("old").join(format!("{}.run1", m.id));
            run_old_once(ctx, m, "#1", &a);
            run_old_once(ctx, m, "#2", &dest);
            let f = m.stable_file.unwrap();
            if read(&a.join(f)) != read(&dest.join(f)) {
                let second = read(&dest.join(f));
                run_old_once(ctx, m, "#3", &dest);
                if read(&dest.join(f)) == second { ctx.warn(format!("old {}: run 1 differed ({f}) but runs 2 and 3 are identical: kept run 3 (the first run after another window run is contaminated by real keyboard state)", m.id)); }
                else { ctx.warn(format!("old {}: three runs, three different {f}: the old exe is NOT deterministic here (a real key was probably pressed, or another window is stealing focus); numbers from this mode are unreliable", m.id)); }
            }
        } else { run_old_once(ctx, m, "", &dest); }
    }
    // uncapped smoke run for the frame-time comparison (Vsync off, framerate uncapped)
    {
        let scratch = ctx.o.scratch_old.clone();
        let ini = read(&scratch.join("FortniteSettings.ini.orig"));
        let patched: String = ini.lines().map(|l| { let t = l.trim().to_ascii_lowercase(); if t.starts_with("vsync=") { "Vsync=False".to_owned() } else if t.starts_with("uncapframerate=") { "UncapFramerate=True".to_owned() } else { l.to_owned() } }).collect::<Vec<_>>().join("\r\n");
        let _ = std::fs::write(scratch.join("FortniteSettings.ini"), patched);
        let m = OldMode { id: "smoke-test-uncapped", args: &["--smoke-test"], timeout: 90., stable_file: None };
        let dest = ctx.work_old(m.id);
        run_old_once(ctx, &m, "", &dest);
        let _ = std::fs::copy(scratch.join("FortniteSettings.ini.orig"), scratch.join("FortniteSettings.ini"));
    }
}

/// Client size of the old default window (the smoke captures), read from the old idle capture.
pub fn old_smoke_size(ctx: &Ctx) -> (u32, u32) {
    crate::img::load(&ctx.work_old("smoke-test").join("idle.bmp")).map(|i| (i.w, i.h)).unwrap_or((1264, 761))
}

fn app_args(ctx: &Ctx, out: &Path) -> Vec<String> {
    vec!["--out".into(), out.display().to_string(), "--assets".into(), ctx.o.scratch_old.join("assets").display().to_string(), "--no-gamepad".into()]
}

fn run_app(ctx: &mut Ctx, id: &str, extra: &[String], out: &Path, timeout: f64) -> bool {
    rm_dir(out); let _ = std::fs::create_dir_all(out);
    let mut args = extra.to_vec(); args.extend(app_args(ctx, out));
    let exe = ctx.new_exe("fortnite-rs");
    ctx.run_proc(id, "new", &exe, &args, &ctx.o.scratch_old.clone(), timeout, &[], &["FN_*"], out).ok
}

pub fn stage_new(ctx: &mut Ctx) {
    if ctx.o.skip_new { return; }
    let (sw, sh) = old_smoke_size(ctx);
    let size = format!("{sw}x{sh}");
    // --- the game itself (window + wgpu), driven by its own scripted facilities
    if ctx.wanted("smoke") { run_app(ctx, "smoke", &["--smoke-test".into(), "--size".into(), size.clone()], &ctx.work_new("smoke"), 150.); }
    if ctx.wanted("smoke") { run_app(ctx, "smoke-vsync", &["--smoke-test".into(), "--size".into(), size.clone(), "--vsync".into()], &ctx.work_new("smoke-vsync"), 150.); }
    if ctx.wanted("pickaxe") { run_app(ctx, "pickaxe", &["--script".into(), "builtin:pickaxe".into(), "--compare-sim".into()], &ctx.work_new("pickaxe"), 240.); }
    for (id, g, timeout) in [("dir", scenarios::dir_script(), 420.), ("light", scenarios::light_script(), 300.), ("sky", scenarios::sky_script(), 600.)] {
        if !ctx.wanted(id) { continue; }
        let out = ctx.work_new(id);
        rm_dir(&out); let _ = std::fs::create_dir_all(&out);
        let sp = out.join(format!("{id}.script"));
        let _ = std::fs::write(&sp, &g.script);
        let mut args = vec!["--script".to_owned(), sp.display().to_string()];
        args.extend(app_args(ctx, &out));
        let exe = ctx.new_exe("fortnite-rs");
        ctx.run_proc(id, "new", &exe, &args, &ctx.o.scratch_old.clone(), timeout, &[], &["FN_*"], &out);
    }
    // --- the headless simulation (same scenarios, numeric logs in the old exe's formats)
    if ctx.wanted("sim") {
        let out = ctx.work_new("sim"); rm_dir(&out); let _ = std::fs::create_dir_all(&out);
        // The old exe (and the shipped game loop, fn-game TickDriver) simulate on the fixed SIM_HZ = 30 tick, so the scripted modes must be replayed at
        // 30 Hz too (`--hz 30`); fn-sim's default per-frame mode (= FNRT_SIM_HZ=0) steps velocity / animation state on different frames.
        let args = vec!["all".to_owned(), "--out".into(), out.display().to_string(), "--assets".into(), ctx.o.scratch_old.join("assets").display().to_string(), "--hz".into(), "30".into()];
        let exe = ctx.new_exe("fn-sim");
        ctx.run_proc("sim", "new", &exe, &args, &ctx.o.scratch_old.clone(), 600., &[], &["FN_*"], &out);
    }
    if ctx.wanted("ui") { stage_ui(ctx); }
}

/// UI scenes of fn-ui-demo (Esc menu, Settings tabs, loading, HUD) rendered at the two fixed sizes.
pub fn stage_ui(ctx: &mut Ctx) {
    let exe = ctx.new_exe("fn-ui-demo");
    let out = ctx.work_new("ui"); rm_dir(&out); let _ = std::fs::create_dir_all(&out);
    if !exe.exists() { ctx.warn("fn-ui-demo.exe is not built: the menu / settings / loading / HUD scene comparisons are skipped"); return; }
    let merged = build_merged_old(ctx);
    let assets = ctx.o.scratch_old.join("assets");
    let rec = ctx.run_proc("ui-list", "new", &exe, &["--list".to_owned()], &ctx.o.scratch_old.clone(), 60., &[], &[], &out);
    let _ = std::fs::write(out.join("scenes.txt"), read(&ctx.work.join("logs").join("new-ui-list.log")));
    if !rec.ok { return; }
    let args = vec!["--diff".to_owned(), "all".into(), "1280x720,1920x1080".into(), "--out".into(), out.display().to_string(), "--assets".into(), "dir".into(), "--folder".into(), assets.display().to_string()];
    let rec = ctx.run_proc("ui-diff", "new", &exe, &args, &ctx.o.scratch_old.clone(), 600., &[("FN_UI_OLD_ROOT", &merged.display().to_string())], &[], &out);
    let _ = std::fs::write(out.join("diff-table.txt"), read(&ctx.work.join("logs").join("new-ui-diff.log")));
    if !rec.ok { ctx.warn("fn-ui-demo --diff failed or timed out: partial UI results"); }
}

/// Folder `work/old-merged/verification` holding (hard links of) every fresh old capture plus the old tree's stored captures for scenes no allowed mode
/// regenerates (quickbar / vitals / HUD-extra tests); `fn-ui-demo --diff` looks its old captures up there through FN_UI_OLD_ROOT.
pub fn build_merged_old(ctx: &mut Ctx) -> PathBuf {
    let root = ctx.work.join("old-merged"); let v = root.join("verification");
    rm_dir(&v); let _ = std::fs::create_dir_all(&v);
    let link = |from: &Path, to: &Path| { if to.exists() { return; } if std::fs::hard_link(from, to).is_err() { let _ = std::fs::copy(from, to); } };
    let is_img = |p: &Path| p.extension().map_or(false, |e| e == "bmp" || e == "png");
    if let Ok(rd) = std::fs::read_dir(ctx.work.join("old")) {
        for d in rd.flatten() { let dn = d.file_name().to_string_lossy().into_owned(); if dn.contains("uncapped") || dn.contains(".run") { continue; } if d.path().is_dir() { if let Ok(fs) = std::fs::read_dir(d.path()) { for f in fs.flatten() { let p = f.path(); if is_img(&p) { link(&p, &v.join(f.file_name())); } } } } }
    }
    // stored captures of the old tree (read-only source) for scenes no allowed scripted mode regenerates
    if let Ok(rd) = std::fs::read_dir(ctx.o.old_tree.join("verification")) { for f in rd.flatten() { let p = f.path(); if is_img(&p) { link(&p, &v.join(f.file_name())); } } }
    root
}

/// `--list` lines: `<name padded>  old capture: <template or ->`
pub fn parse_scene_list(text: &str) -> Vec<(String, Option<String>)> {
    text.lines().filter_map(|l| {
        let (name, rest) = l.split_once("old capture:")?;
        let name = name.trim().to_owned(); let t = rest.trim();
        if name.is_empty() || name.contains(' ') { return None; }
        Some((name, if t == "-" || t.is_empty() { None } else { Some(t.to_owned()) }))
    }).collect()
}

pub fn find_root(start: &Path) -> Option<PathBuf> {
    let mut d = Some(start.to_path_buf());
    while let Some(p) = d { if p.join("crates").join("fn-parity").is_dir() { return Some(p); } d = p.parent().map(|x| x.to_path_buf()); }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn scene_list_parses_names_and_templates() {
        let v = parse_scene_list("backend: cpu\nmenu-pause                        old capture: menu-pause-{w}x{h}\nloading                           old capture: -\n");
        assert_eq!(v.len(), 2); assert_eq!(v[0].0, "menu-pause"); assert_eq!(v[0].1.as_deref(), Some("menu-pause-{w}x{h}")); assert!(v[1].1.is_none());
    }
}
