//! Turns pairs of captures / logs into scored `Item`s (+ evidence images).
use crate::config::*;
use crate::exec::Ctx;
use crate::img::{self, Img, Rect};
use crate::logs;
use crate::model::*;
use crate::scenarios::Topic;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub enum JobKind { World(Topic), Ui(Area) }

#[derive(Clone, Debug, Default)]
pub struct UiRow { pub full: f64, pub masked: Option<f64>, pub bad8: Option<f64>, pub covered: usize, pub total: usize }

#[derive(Clone, Debug)]
pub struct ImageJob {
    pub id: String, pub group: String, pub label: String,
    pub old: PathBuf, pub new: PathBuf, pub kind: JobKind,
    /// spawn framing: sky / ground / HUD regions are meaningful
    pub std_view: bool,
    /// the player stands at the spawn point with the default camera: the props regions (trees, rock, shadow) are meaningful
    pub spawn: bool,
    pub weight: f64,
    pub note: String,
    /// the producer of the new capture ran and left outputs (otherwise a missing capture is "n/a", not a failure of the game)
    pub new_available: bool,
    pub ui: Option<UiRow>,
}

pub fn sanitize(id: &str) -> String { id.chars().map(|c| if c.is_ascii_alphanumeric() || "._+-".contains(c) { c } else { '_' }).collect() }

#[derive(Default)]
pub struct RegionAgg { pub rows: BTreeMap<String, Vec<(String, f64, f64, Option<String>)>> }

pub fn region_area(name: &str) -> Area { match name { "sky" | "ground" | "tree_canopy" | "tree_trunk" | "rock" | "tree_shadow" => Area::Lighting, _ => Area::Hud } }

/// Compare one image pair. Returns the topic item (None for pairs that only feed the regional aggregates).
pub fn compare_image(ctx: &Ctx, job: &ImageJob, agg: &mut RegionAgg) -> Item {
    let cfg = &ctx.cfg;
    let area = match &job.kind { JobKind::World(Topic::Anim) => Area::Animation, JobKind::World(Topic::Light) => Area::Lighting, JobKind::World(Topic::Camera) => Area::Camera, JobKind::Ui(a) => *a };
    let mut it = Item::new(format!("img:{}", job.id), area, job.group.clone(), job.label.clone());
    it.weight = job.weight; it.note = job.note.clone();
    let old = match img::load(&job.old) { Ok(i) => i, Err(e) => return it.na(format!("old capture missing ({e})")) };
    let new = match img::load(&job.new) {
        Ok(i) => i,
        Err(_) if !job.new_available => return it.na("the new game did not run this scenario (see Runs)"),
        Err(_) => { it.headline = "new capture missing".into(); return it.scored(0.).metric("capture", "missing in the new run".into(), Grade::Red); }
    };
    let (a, b, cropped) = img::align(&old, &new);
    let full = img::metrics(&a, &b, Rect::FULL);
    let heat = img::heatmap(&a, &b);
    let sheet = img::side_by_side(&a, &b, &heat, 400);
    let rel = format!("evidence/{}.png", sanitize(&job.id));
    let _ = img::save_png(&ctx.o.parity.join(&rel), &sheet);
    it.evidence = Some(rel.clone());
    it.thumb = Some(img::data_uri(&img::side_by_side(&a, &b, &heat, 120)));

    let (mae_t, ssim_t, struct_t, bad_t) = (cfg.pair_or("image.mae", (4., 12.)), cfg.pair_or("image.ssim", (0.95, 0.85)), cfg.pair_or("image.structure", (0.9, 0.7)), cfg.pair_or("image.bad_pct", (5., 20.)));
    let g = |s: f64| item_grade(s);
    let mut metrics_added: Vec<Metric> = vec![];
    let mut push = |n: &str, v: String, s: f64| metrics_added.push(Metric { name: n.to_owned(), value: v, grade: g(s) });
    let mut score;
    if cropped { push("size", format!("old {}x{} vs new {}x{}", old.w, old.h, new.w, new.h), 0.); }

    // regional statistics for world captures
    let mut region_stats: BTreeMap<String, img::Metrics> = BTreeMap::new();
    if matches!(job.kind, JobKind::World(_)) {
        for name in cfg.region_names() { if let Some(r) = cfg.region(&name) { region_stats.insert(name, img::metrics(&a, &b, r)); } }
        if job.std_view { for (name, m) in &region_stats { agg.rows.entry(name.clone()).or_default().push((job.id.clone(), m.mae_all, m.ssim, Some(rel.clone()))); } }
        if job.spawn { for name in cfg.names_in("spawn_regions") { if let Some(r) = cfg.region_in("spawn_regions", &name) { let m = img::metrics(&a, &b, r); agg.rows.entry(name.clone()).or_default().push((job.id.clone(), m.mae_all, m.ssim, Some(rel.clone()))); region_stats.insert(name, m); } } }
    }
    match &job.kind {
        JobKind::World(Topic::Anim) => {
            let c = &region_stats["character"];
            let t = cfg.pair_or("image.region_mae.character", (8., 24.));
            let (s1, s2) = (score_lower(c.mae_all, t), score_higher(c.ssim, ssim_t));
            score = s1.min(s2);
            it.headline = format!("character MAE {:.1}, SSIM {:.3}", c.mae_all, c.ssim);
            push("character MAE", format!("{:.2}", c.mae_all), s1); push("character SSIM", format!("{:.3}", c.ssim), s2);
            push("frame MAE", format!("{:.2}", full.mae_all), score_lower(full.mae_all, mae_t)); push("frame SSIM", format!("{:.3}", full.ssim), score_higher(full.ssim, ssim_t));
        }
        JobKind::World(Topic::Light) => {
            let dl = (full.mean_luma_a - full.mean_luma_b).abs();
            let lt = cfg.pair_or("image.luma_delta", (3., 10.));
            let (s1, s2) = (score_lower(full.mae_all, mae_t), score_lower(dl, lt));
            score = s1.min(s2);
            push("frame MAE", format!("{:.2}", full.mae_all), s1); push("mean luma delta", format!("{:.2} (old {:.1}, new {:.1})", dl, full.mean_luma_a, full.mean_luma_b), s2);
            for rn in ["sky", "ground"] { if job.std_view { if let Some(m) = region_stats.get(rn) { let s = score_lower(m.mae_all, cfg.pair_or(&format!("image.region_mae.{rn}"), mae_t)); score = score.min(s); push(&format!("{rn} MAE"), format!("{:.2}", m.mae_all), s); } } }
            it.headline = format!("MAE {:.1}, luma delta {:.1}", full.mae_all, dl);
        }
        JobKind::World(Topic::Camera) => {
            let s1 = score_higher(full.structure, struct_t);
            let s2 = score_lower(full.mae_all, mae_t);
            score = s1;
            it.headline = format!("structure {:.3}, MAE {:.1}", full.structure, full.mae_all);
            push("frame structure", format!("{:.3}", full.structure), s1); push("frame MAE", format!("{:.2}", full.mae_all), s2); push("frame SSIM", format!("{:.3}", full.ssim), score_higher(full.ssim, ssim_t));
        }
        JobKind::Ui(_) => {
            let over_world = job.ui.as_ref().map_or(false, |u| u.total > 0 && (u.covered as f64) < 0.98 * u.total as f64);
            if over_world {
                let u = job.ui.as_ref().unwrap();
                let Some(m) = u.masked.filter(|v| v.is_finite()) else { it.evidence = Some(rel); return it.na("the new UI draws no opaque pixels in this scene: nothing to compare against the old capture with the 3D world behind it"); };
                let s1 = score_lower(m, mae_t);
                score = s1;
                it.headline = format!("UI-covered pixels MAE {:.1} ({:.0}% covered)", m, u.covered as f64 * 100. / u.total as f64);
                push("masked MAE", format!("{:.2}", m), s1);
                if let Some(b) = u.bad8 { push("bad>8", format!("{:.1}%", b * 100.), score_lower(b * 100., bad_t)); }
                it.note = format!("{} Compared only where the new UI is opaque (the old capture has the 3D world behind the UI).", it.note).trim().to_owned();
            } else {
                let (s1, s2) = (score_lower(full.mae_all, mae_t), score_higher(full.ssim, ssim_t));
                score = s1.min(s2);
                it.headline = format!("MAE {:.1}, SSIM {:.3}", full.mae_all, full.ssim);
                push("frame MAE", format!("{:.2}", full.mae_all), s1); push("frame SSIM", format!("{:.3}", full.ssim), s2); push("bad pixels", format!("{:.1}%", full.bad_pct), score_lower(full.bad_pct, bad_t));
            }
        }
    }
    // a blank capture is never a pass
    if full.mean_luma_b < 2. && full.mean_luma_a > 10. { score = 0.; push("new capture", "blank (black)".into(), 0.); it.headline = format!("{} - NEW CAPTURE IS BLANK", it.headline); }
    if cropped { score = score.min(50.); }
    it.metrics = metrics_added;
    it.scored(score)
}

/// Aggregated items: per named region, the mean over every standard-view world capture (worst capture in the note and as the evidence).
pub fn region_items(ctx: &Ctx, agg: &RegionAgg) -> Vec<Item> {
    let cfg = &ctx.cfg; let mut out = vec![];
    for (name, rows) in &agg.rows {
        if rows.is_empty() { continue; }
        let mean = rows.iter().map(|r| r.1).sum::<f64>() / rows.len() as f64;
        let mean_ssim = rows.iter().map(|r| r.2).sum::<f64>() / rows.len() as f64;
        let worst = rows.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).unwrap();
        if name == "character" { continue; } // graded per capture (Animation)
        let t = cfg.pair_or(&format!("image.region_mae.{name}"), cfg.pair_or("image.mae", (4., 12.)));
        let s1 = score_lower(mean, t);
        let area = region_area(name);
        let mut it = Item::new(format!("region:{name}"), area, "regions", format!("{name} region over {} standard-view captures", rows.len()));
        it.headline = format!("mean MAE {:.1}, worst {:.1} ({})", mean, worst.1, worst.0);
        it.evidence = worst.3.clone();
        it = it.metric("mean MAE", format!("{:.2}", mean), item_grade(s1)).metric("mean SSIM", format!("{:.3}", mean_ssim), item_grade(score_higher(mean_ssim, cfg.pair_or("image.ssim", (0.95, 0.85))))).metric("worst MAE", format!("{:.2}", worst.1), item_grade(score_lower(worst.1, t)));
        out.push(it.scored(s1));
    }
    out
}

// ------------------------------------------------------------------------------------------------------------------------------------------------
// logs

pub struct LogJob {
    pub id: String, pub group: String, pub label: String,
    pub old: Option<String>, pub new: Option<String>,
    pub drop: Option<(&'static str, &'static str)>, pub skip_lines: Option<&'static str>,
    pub note: String,
    pub new_available: bool,
}

fn log_area(group: &str, cat: &str) -> Area {
    match group {
        "camera" => Area::Camera,
        "movement" => Area::Movement,
        "face" | "pickaxe-frames" => Area::Animation,
        _ => match cat { "velocity" | "position_cm" | "angle_deg" => Area::Movement, "eye_cm" => Area::Camera, _ => Area::Animation },
    }
}

pub fn compare_log(ctx: &Ctx, job: &LogJob) -> Vec<Item> {
    let cfg = &ctx.cfg;
    let mut out = vec![];
    let (Some(a), Some(b)) = (&job.old, &job.new) else {
        let why = if job.old.is_none() { "old log not available" } else if job.new_available { "new log missing" } else { "the new tool did not run" };
        let mut it = Item::new(format!("log:{}", job.id), log_area(&job.group, "other"), job.group.clone(), job.label.clone()).na(why);
        if job.old.is_some() && job.new_available { it = it.scored(0.); it.note = why.into(); }
        return vec![it];
    };
    let d = logs::diff(a, b, job.drop, job.skip_lines);
    let ev_rel = format!("evidence/log_{}.txt", sanitize(&job.id));
    if d.compared > 0 { let _ = std::fs::create_dir_all(&ctx.evidence); let _ = std::fs::write(ctx.o.parity.join(&ev_rel), logs::excerpt(&d, &job.label)); }
    if d.compared == 0 { return vec![Item::new(format!("log:{}", job.id), log_area(&job.group, "other"), job.group.clone(), job.label.clone()).na("empty log")]; }
    for (cat, c) in &d.cats {
        if cat == "frame" || c.n == 0 { continue; }
        let t = cfg.pair_or(&format!("logs.{cat}"), (0.001, 0.1));
        let s = score_lower(c.max, t);
        let unit = match cat.as_str() { "position_cm" | "bone_cm" | "eye_cm" => "cm", "angle_deg" => "deg", "velocity" => "u/s", _ => "" };
        let mut it = Item::new(format!("log:{}:{cat}", job.id), log_area(&job.group, cat), job.group.clone(), format!("{} - {cat}", job.label));
        it.headline = format!("max {:.4}{unit} (line {}, {}), mean {:.5}{unit}, {} of {} values differ", c.max, c.max_line, if c.max_label.is_empty() { "-" } else { &c.max_label }, c.mean(), c.over, c.n);
        it.note = job.note.clone();
        out.push(it.metric("max error", format!("{:.4}{unit}", c.max), item_grade(s)).metric("mean error", format!("{:.5}{unit}", c.mean()), item_grade(s)).scored(s));
    }
    if d.word_tokens > 0 {
        let pct = d.word_mismatch_pct();
        let s = score_lower(pct, cfg.pair_or("logs.anim_state_mismatch_pct", (0., 2.)));
        let mut it = Item::new(format!("log:{}:states", job.id), log_area(&job.group, "state"), job.group.clone(), format!("{} - state / label tokens", job.label));
        it.headline = format!("{} of {} word tokens differ ({pct:.2}%){}", d.word_mismatches, d.word_tokens, d.first_word_mismatch.as_ref().map_or(String::new(), |(l, x, y)| format!("; first at line {l}: {x:?} vs {y:?}")));
        it.note = job.note.clone();
        out.push(it.metric("mismatching tokens", format!("{pct:.2}%"), item_grade(s)).scored(s));
    }
    let lp = d.length_pct();
    for it in out.iter_mut() { it.evidence = Some(ev_rel.clone()); }
    if lp > 0. || d.lines_a != d.lines_b {
        let s = score_lower(lp, cfg.pair_or("logs.length_pct", (0., 2.)));
        let mut it = Item::new(format!("log:{}:length", job.id), log_area(&job.group, "other"), job.group.clone(), format!("{} - line count", job.label));
        it.headline = format!("old {} lines vs new {} lines", d.lines_a, d.lines_b);
        out.push(it.metric("length mismatch", format!("{lp:.2}%"), item_grade(s)).scored(s));
    }
    out
}

/// Time to liftoff / airtime / apex of a standing jump: parsed from the `--jump-feel-test` text of both sides.
pub fn jump_items(ctx: &Ctx, old: &str, new: &str) -> Vec<Item> {
    let cfg = &ctx.cfg; let mut out = vec![];
    for (key, label, ms) in [("liftoff t=", "time to liftoff", true), ("airtime ", "airtime", true), ("apex height ", "apex height", false)] {
        let (Some(x), Some(y)) = (logs::number_after(old, key), logs::number_after(new, key)) else { continue };
        let (e, unit, t) = if ms { ((x - y).abs() * 1000., "ms", cfg.pair_or("logs.liftoff_ms", (8., 33.))) } else { ((x - y).abs() * 100., "cm", cfg.pair_or("logs.position_cm", (0.5, 5.))) };
        let s = score_lower(e, t);
        let mut it = Item::new(format!("jump:{label}"), Area::Movement, "jump", format!("standing jump - {label}"));
        it.headline = format!("old {x} vs new {y}: error {e:.2}{unit}");
        out.push(it.metric("error", format!("{e:.2}{unit}"), item_grade(s)).scored(s));
    }
    out
}

/// "N lines vs M lines: C compared, I identical, W within tolerance, X mismatching" -> (compared, mismatching)
pub fn parse_compare_line(line: &str) -> Option<(usize, usize)> {
    let nums = |key: &str| -> Option<usize> { let i = line.find(key)?; line[..i].trim_end().rsplit(|c: char| !c.is_ascii_digit()).next()?.parse().ok() };
    Some((nums(" compared")?, nums(" mismatching")?))
}

pub fn read_text(p: &Path) -> Option<String> { std::fs::read_to_string(p).ok().filter(|t| !t.trim().is_empty()) }
pub fn load_img(p: &Path) -> Option<Img> { img::load(p).ok() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn compare_line_parses() { assert_eq!(parse_compare_line("  without pose: 880 lines vs 880 lines: 880 compared, 880 identical, 880 within tolerance, 0 mismatching, max |diff| 0.000000"), Some((880, 0))); assert_eq!(parse_compare_line("nothing"), None); }
    #[test] fn sanitize_keeps_names_readable() { assert_eq!(sanitize("img:pick-idle@1280x720"), "img_pick-idle_1280x720"); }
}
