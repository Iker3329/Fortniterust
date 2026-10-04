//! Assembles every comparison from the outputs in `parity/work` (works after a run or with `--reuse`).
use crate::compare::*;
use crate::config::*;
use crate::consts::{self, ConstRow};
use crate::exec::Ctx;
use crate::model::*;
use crate::perf;
use crate::run::{dir_has_files, old_smoke_size};
use crate::scenarios::{self, Topic};
use std::path::{Path, PathBuf};

fn rd(p: &Path) -> Option<String> { read_text(p) }

fn ui_area(name: &str) -> Area {
    let n = name.to_ascii_lowercase();
    if n.starts_with("loading") { Area::Loading }
    else if ["hud", "quickbar", "vitals", "reticle", "popup", "harvest", "minimap", "compass", "card", "backpack", "resource"].iter().any(|k| n.contains(k)) { Area::Hud }
    else { Area::Menus }
}

/// `fn-ui-demo --diff` table lines: `name  WxH  full  masked  bad>8  max  covered  oldfile`
fn parse_diff_table(text: &str) -> Vec<(String, (u32, u32), UiRow, String)> {
    let mut out = vec![];
    for l in text.lines() {
        if l.contains("(no old capture") || l.contains("skipped") { continue; }
        let t: Vec<&str> = l.split_whitespace().collect();
        if t.len() < 8 { continue; }
        let Some((w, h)) = t[1].split_once('x').and_then(|(a, b)| Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))) else { continue };
        let Ok(full) = t[2].parse::<f64>() else { continue };
        let masked = t[3].parse::<f64>().ok();
        let bad8 = t[4].strip_suffix('%').and_then(|v| v.parse::<f64>().ok()).map(|v| v / 100.);
        let Ok(covered) = t[6].parse::<usize>() else { continue };
        out.push((t[0].to_owned(), (w, h), UiRow { full, masked, bad8, covered, total: (w * h) as usize }, t[7].to_owned()));
    }
    out
}

pub fn analyze(ctx: &mut Ctx) -> (Vec<Item>, Vec<ConstRow>, Vec<String>) {
    let _ = std::fs::remove_dir_all(ctx.evidence.clone());
    let _ = std::fs::create_dir_all(&ctx.evidence);
    let mut items: Vec<Item> = vec![];
    let mut agg = RegionAgg::default();
    let mut gaps: Vec<String> = vec![];
    let (sw, sh) = old_smoke_size(ctx);

    let new_has = |ctx: &Ctx, d: &str| dir_has_files(&ctx.work_new(d));
    let mut jobs: Vec<ImageJob> = vec![];

    // pickaxe-test: every old capture with a counterpart name
    {
        let od = ctx.work_old("pickaxe-test"); let nd = ctx.work_new("pickaxe"); let avail = new_has(ctx, "pickaxe");
        let mut stems: Vec<String> = std::fs::read_dir(&od).map(|r| r.flatten().filter_map(|e| { let n = e.file_name().to_string_lossy().into_owned(); n.strip_suffix(".bmp").filter(|s| s.starts_with("pick-")).map(str::to_owned) }).collect()).unwrap_or_default();
        stems.sort();
        for s in stems {
            let std = s.ends_with("-1280x720") || s.ends_with("-1920x1080");
            jobs.push(ImageJob { id: format!("pickaxe/{s}"), group: "pickaxe-test".into(), label: s.clone(), old: od.join(format!("{s}.bmp")), new: nd.join(format!("{s}.png")), kind: JobKind::World(Topic::Anim), std_view: std, spawn: s == "pick-idle-1280x720", weight: 1., note: String::new(), new_available: avail, ui: None });
        }
    }
    let script_jobs = |ctx: &Ctx, jobs: &mut Vec<ImageJob>, g: scenarios::Generated, old_mode: &str, new_dir: &str, group: &str, weight: f64, note: &str| {
        let (od, nd, avail) = (ctx.work_old(old_mode), ctx.work_new(new_dir), new_has(ctx, new_dir));
        for p in g.pairs {
            jobs.push(ImageJob { id: format!("{group}/{}", p.old), group: group.to_owned(), label: p.old.clone(), old: od.join(format!("{}.bmp", p.old)), new: nd.join(&p.new), kind: JobKind::World(p.topic), std_view: p.std_view, spawn: p.old == "light-on-1280x720", weight, note: note.to_owned(), new_available: avail, ui: None });
        }
    };
    script_jobs(ctx, &mut jobs, scenarios::dir_script(), "dir-test", "dir", "dir-test", 0., "INFORMATIONAL (weight 0): the app is driven through its key mapping (sprint / crouch keys, diagonal input) while the old test forces run / crouch; speeds and camera lag differ by construction (e.g. diagonal sprint 479 vs 550 cm/s), so these captures do not measure the animation. Pose parity of the same scenario is graded from the frame logs.");
    script_jobs(ctx, &mut jobs, scenarios::light_script(), "lighting-test", "light", "lighting-test", 1., "");
    script_jobs(ctx, &mut jobs, scenarios::sky_script(), "sky-test", "sky", "sky-test", 1., "Time-of-day / cloud animation time differs (the old test pins View.RealTime).");
    {
        let (od, nd, avail) = (ctx.work_old("smoke-test"), ctx.work_new("smoke"), new_has(ctx, "smoke"));
        for n in scenarios::SMOKE_NAMES {
            jobs.push(ImageJob { id: format!("smoke/{n}"), group: "smoke-test".into(), label: n.to_owned(), old: od.join(format!("{n}.bmp")), new: nd.join(format!("{n}-{sw}x{sh}.png")), kind: JobKind::World(Topic::Anim), std_view: matches!(n, "idle" | "scene"), spawn: false, weight: 0.5, note: "The old smoke test runs in real time (vsync, ~144 fps), the new one at a fixed 60 Hz, so the same frame number is a different animation phase: informational (weight 0.5).".into(), new_available: avail, ui: None });
        }
    }
    // UI scenes
    {
        let ud = ctx.work_new("ui"); let avail = dir_has_files(&ud);
        let merged = ctx.work.join("old-merged").join("verification");
        if let Some(t) = rd(&ud.join("diff-table.txt")) {
            for (name, (w, h), row, oldf) in parse_diff_table(&t) {
                let area = ui_area(&name);
                jobs.push(ImageJob { id: format!("ui/{name}@{w}x{h}"), group: "ui-scenes".into(), label: format!("{name} {w}x{h}"), old: merged.join(&oldf), new: ud.join(format!("{name}-{w}x{h}-ours.png")), kind: JobKind::Ui(area), std_view: false, spawn: false, weight: if row.total > 0 && (row.covered as f64) < 0.98 * row.total as f64 { (row.covered as f64 / row.total as f64).clamp(0.25, 1.) } else { 1. }, note: format!("old capture: {oldf}"), new_available: avail, ui: Some(row) });
            }
        } else if !avail { gaps.push("Menus / Settings / Loading / HUD scenes: fn-ui-demo did not run (not built or --diff failed), so no UI scene was compared.".into()); }
    }
    for job in &jobs { items.push(compare_image(ctx, job, &mut agg)); }
    items.extend(region_items(ctx, &agg));

    // logs
    let sim = ctx.work_new("sim");
    let simf = |sc: &str, f: &str| rd(&sim.join(sc).join(f));
    let sim_ok = dir_has_files(&sim);
    let mut ljobs: Vec<LogJob> = vec![];
    let mut lj = |id: &str, group: &str, label: &str, old: Option<String>, new: Option<String>, drop: Option<(&'static str, &'static str)>, skip: Option<&'static str>, note: &str| {
        ljobs.push(LogJob { id: id.into(), group: group.into(), label: label.into(), old, new, drop, skip_lines: skip, note: note.into(), new_available: sim_ok });
    };
    let od = |m: &str, f: &str| rd(&ctx.work_old(m).join(f));
    lj("dir-frames", "dir-frames", "--dir-test frame log (old exe vs fn-sim)", od("dir-test", "dir-frames.txt"), simf("dir-test", "dir-frames.txt"), None, None, "bones in cm, anim graph state tokens");
    lj("dir-pickaxe-frames", "pickaxe-frames", "--dir-test pickaxe-frames.txt", od("dir-test", "pickaxe-frames.txt"), simf("dir-test", "pickaxe-frames.txt"), None, None, "");
    lj("dir-captures", "pickaxe-test", "--dir-test capture lines", od("dir-test", "pickaxe-test.txt"), simf("dir-test", "pickaxe-test.txt"), Some(("grip_r", "height")), Some("pick-size"), "mesh-based grip numbers dropped (only the old exe can produce them)");
    lj("pickaxe-frames", "pickaxe-frames", "--pickaxe-test frame log", od("pickaxe-test", "pickaxe-frames.txt"), simf("pickaxe-test", "pickaxe-frames.txt"), None, None, "");
    lj("pickaxe-captures", "pickaxe-test", "--pickaxe-test capture lines", od("pickaxe-test", "pickaxe-test.txt"), simf("pickaxe-test", "pickaxe-test.txt"), Some(("grip_r", "height")), Some("pick-size"), "mesh-based grip numbers dropped (only the old exe can produce them)");
    lj("face-test", "face", "--face-test head / camera numbers", od("face-test", "face-test.txt"), simf("face-test", "face-test.txt"), None, None, "");
    lj("jump-feel", "movement", "--jump-feel-test", od("jump-feel-test", "jump-feel-test.txt"), simf("jump-feel-test", "jump-feel-test.txt"), None, None, "");
    // golden logs written by the old exe in modes this harness may not run (headless / camera modes): used only when intact
    let golden_root: PathBuf = ctx.o.root.join("testdata").join("old-logs");
    let golden = |f: &str| rd(&golden_root.join(f)).filter(|t| t.len() > 200);
    let gnote = "old side = golden log in testdata/old-logs (that mode is not in the allowed scripted list, so it is not re-run); new side = fn-sim now";
    for (id, group, label, gf, sc, nf) in [
        ("movement-test", "movement", "--movement-test", "movement-test/movement-test.txt", "movement-test", "movement-test.txt"),
        ("jump-air-test", "movement", "--jump-air-test", "jump-air-test/jump-air-test.txt", "jump-air-test", "jump-air-test.txt"),
        ("camera-test", "camera", "--camera-test (eye / view)", "camera-test/camera-test.txt", "camera-test", "camera-test.txt"),
        ("camprop-after", "camera", "--camera-prop-test, prop collision on", "camera-prop-test/camprop-after.txt", "camera-prop-test", "camprop-after.txt"),
        ("camprop-before", "camera", "--camera-prop-test, collision off", "camera-prop-test/camprop-before.txt", "camera-prop-test", "camprop-before.txt"),
    ] {
        lj(id, group, label, golden(gf), simf(sc, nf), None, None, gnote);
    }
    if let Some(g) = golden("camera-video/camera-video-v2.txt") { let g: String = g.lines().map(|l| l.split(" size ").next().unwrap_or(l).to_owned()).collect::<Vec<_>>().join("\n"); lj("camera-video", "camera", "--camera-video (eye path)", Some(g), simf("camera-video", "camera-video-v2.txt"), None, None, gnote); }
    else { lj("camera-video", "camera", "--camera-video (eye path)", None, None, None, None, ""); }
    for j in &ljobs {
        let mut its = compare_log(ctx, j);
        if its.len() == 1 && its[0].score.is_none() && its[0].note.contains("old log not available") && j.note.contains("golden") { its[0].note = "golden log missing or truncated (another agent is regenerating testdata/old-logs?): skipped".into(); }
        items.extend(its);
    }
    // the headless series carry their own pass / fail checks
    for (sc, file) in [("movement-test", "movement-test.txt"), ("jump-air-test", "jump-air-test.txt")] {
        let Some(n) = simf(sc, file) else { continue };
        let fails: Vec<&str> = n.lines().filter(|l| l.contains("[FAIL]")).collect();
        let summary = n.lines().rev().find(|l| l.contains("checks,")).unwrap_or("");
        let gold = golden(&format!("{sc}/{file}"));
        let same_in_old = !fails.is_empty() && gold.as_ref().map_or(false, |g| fails.iter().all(|l| g.contains(*l)));
        let s = if fails.is_empty() || same_in_old { 100. } else { score_lower(fails.len() as f64, (0., 1.)) };
        let mut it = Item::new(format!("selfcheck:{sc}"), Area::Movement, "movement", format!("--{sc} own pass/fail checks (new code)"));
        it.headline = format!("{summary}{}", fails.first().map_or(String::new(), |f| format!(" first: {f}")));
        if same_in_old { it.note = "the golden log of the old exe contains the same [FAIL] line(s): identical behaviour, not a regression".into(); }
        items.push(it.metric("failed checks", fails.len().to_string(), item_grade(s)).scored(s));
    }
    if let (Some(o), Some(n)) = (od("jump-feel-test", "jump-feel-test.txt"), simf("jump-feel-test", "jump-feel-test.txt")) { items.extend(jump_items(ctx, &o, &n)); }

    // time handling: simulated flight time of a standing jump under the old real-time audit vs the new virtual-clock twin
    if let (Some(o), Some(n)) = (od("time-test", "time-audit-run.txt"), simf("time-test", "time-audit-144fps.txt")) {
        if let (Some(x), Some(y)) = (crate::logs::number_after(&o, "simulated "), crate::logs::number_after(&n, "simulated ")) {
            let e = (x - y).abs() * 1000.;
            let s = score_lower(e, ctx.cfg.pair_or("logs.liftoff_ms", (8., 33.)));
            let mut it = Item::new("time:flight", Area::Movement, "time-audit", "--time-test: simulated flight time of a standing jump");
            it.headline = format!("old (real clock, vsync) {x} s vs new (virtual 144 Hz clock) {y} s: {e:.1} ms apart");
            it.note = "the old audit measures the windowed loop with the wall clock, the new twin uses a virtual clock; only the simulated airtime is comparable".into();
            items.push(it.metric("airtime difference", format!("{e:.1} ms"), item_grade(s)).scored(s));
        }
    }
    // new app loop vs headless sim (self-consistency of the new code)
    for (dirn, label) in [("smoke", "smoke script"), ("pickaxe", "pickaxe series")] {
        let d = ctx.work_new(dirn);
        let rep = std::fs::read_dir(&d).ok().and_then(|r| r.flatten().find(|e| e.file_name().to_string_lossy().ends_with("-report.txt")).map(|e| e.path())).and_then(|p| rd(&p));
        let mut it = Item::new(format!("appsim:{dirn}"), Area::Movement, "app-vs-sim", format!("app loop vs headless sim: {label}"));
        match rep.as_ref().and_then(|t| t.lines().find(|l| l.contains("without pose:"))).and_then(parse_compare_line) {
            Some((compared, mism)) => {
                let pct = if compared == 0 { 100. } else { mism as f64 * 100. / compared as f64 };
                let s = score_lower(pct, ctx.cfg.pair_or("logs.app_vs_sim_mismatch_pct", (0., 0.5)));
                it.headline = format!("{mism} of {compared} frame-log lines differ"); it.note = "both sides are the new code: this checks that the windowed app loop and the headless fn-sim harness agree".into();
                items.push(it.metric("mismatching lines", format!("{pct:.2}%"), item_grade(s)).scored(s));
            }
            None => items.push(it.na("no compare line in the app report (the run did not finish)")),
        }
    }

    items.extend(perf::perf_items(ctx));
    let (cit, crow) = consts::const_items(ctx);
    items.extend(cit);

    // UI scenes the new code has but the old prototype never captured (or not at the two fixed sizes)
    if let Some(list) = rd(&ctx.work_new("ui").join("scenes.txt")) {
        let have: std::collections::BTreeSet<String> = jobs.iter().filter_map(|j| j.id.strip_prefix("ui/").and_then(|r| r.split('@').next()).map(str::to_owned)).collect();
        let missing: Vec<String> = crate::run::parse_scene_list(&list).into_iter().map(|(n, _)| n).filter(|n| !have.contains(n)).collect();
        if !missing.is_empty() { gaps.push(format!("UI scenes with no old capture at 1280x720 / 1920x1080 (new screens or states the old prototype never captured, so nothing to compare): {}.", missing.join(", "))); }
    }
    // honest coverage notes
    gaps.extend(coverage_notes(ctx, &items));
    (items, crow, gaps)
}

fn coverage_notes(ctx: &Ctx, items: &[Item]) -> Vec<String> {
    let mut g = vec![
        "Audio: the rewrite has only a null audio sink and the old prototype has no audio at all, so there is nothing to compare (area is n/a).".to_owned(),
        "The old exe has no scripting facility: only its fixed scripted modes can be replayed (smoke / dir / pickaxe / lighting / sky / menu / settings18 / jump-feel / time / face / reticle). Anything outside those programs (harvest of a tree to destruction beyond the pickaxe series, camera collision against a wall, a picker or lobby screen, other characters) has no old-exe capture and is not compared.".to_owned(),
        "Camera collision (--camera-prop-test) and the movement series (--movement-test, --jump-air-test) are headless modes outside the allowed list; their old side comes from the golden logs in testdata/old-logs (used only when intact) and is compared with the new code's headless run. Camera pitch up/down is covered through the old --sky-test / --lighting-test views, not through dedicated camera captures.".to_owned(),
        "Old-exe window runs read the real keyboard while their window is in the foreground and a hung mode (--reticle-test) never exits: log-bearing modes are run twice and kept only when identical; --reticle-test is given 45 s and then killed (its captures are not available).".to_owned(),
        "Lighting scenes with shadows switched off, the sky layer-isolation shots, the sky time-animation shots and the sun-disc time overrides need renderer switches the new game does not expose in script mode: skipped. Auto-exposure adaptation state differs between a 30-frame old scene and a continuous new timeline, which shows up as brightness differences.".to_owned(),
        "UI scenes are rendered by fn-ui-demo (the real menu / Settings / HUD / loading screens on a transparent background), not by the running game: scenes over a 3D world are compared only where the new UI is opaque. The in-game Esc menu composited over the live world, the Settings tabs inside the game window and the loading screen during real asset loading are exercised by fn-app --live-test, which cannot run unattended here (it resizes / minimises the window).".to_owned(),
        "Frame times: the old exe paces itself with vsync (144 Hz monitor) so its p50 / p99 sit at the refresh interval; an uncapped old run is attempted by editing the scratch ini only. GPU time, VRAM and shader-compile stalls are not measured (CPU time and peak working set come from the OS process counters).".to_owned(),
        "Recovered-data checks only cover constants a Rust crate exposes today (movement parameters and the camera modes); animation-graph, lighting, post, HUD, settings and sky constants stay 'not exposed' until their crates publish them.".to_owned(),
    ];
    let na: Vec<&str> = items.iter().filter(|i| i.score.is_none() && !i.id.starts_with("const:")).map(|i| i.id.as_str()).collect();
    if !na.is_empty() { g.push(format!("{} items could not be scored this run (an input was missing): {}", na.len(), na.iter().take(12).copied().collect::<Vec<_>>().join(", ") + if na.len() > 12 { ", ..." } else { "" })); }
    let reticle = ctx.runs.iter().rev().find(|r| r.id == "reticle-test");
    if let Some(r) = reticle { if r.timed_out { g.push("--reticle-test timed out and was killed (no reticle captures were produced by the old exe in this environment).".to_owned()); } }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn diff_table_rows_parse() {
        let t = "backend: gpu\nscreen size full masked bad>8 max covered  old capture\nsettings18-video 1280x720 3.21 3.21 4.5% 255 921600  settings18-video-1280x720.bmp\nhud 1280x720 - - - - -  (no old capture at this size)\n";
        let v = parse_diff_table(t); assert_eq!(v.len(), 1); assert_eq!(v[0].0, "settings18-video"); assert_eq!(v[0].1, (1280, 720)); assert_eq!(v[0].2.bad8, Some(0.045)); assert_eq!(v[0].3, "settings18-video-1280x720.bmp");
    }
    #[test] fn ui_areas() { assert_eq!(ui_area("loading-screen"), Area::Loading); assert_eq!(ui_area("hud-quickbar"), Area::Hud); assert_eq!(ui_area("settings18-video"), Area::Menus); }
}
