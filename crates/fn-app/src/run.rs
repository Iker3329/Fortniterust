//! Bookkeeping of a scripted run (`--script`, `--smoke-test`, `--screenshot`): the per-frame log, captures, frame-time statistics, the final report
//! and the comparison with fn-sim.
use crate::script::Driver;
use crate::stats::FrameStats;
use fn_game::GameSim;
use std::path::{Path, PathBuf};

pub struct ScriptRun {
    pub driver: Box<dyn Driver>,
    pub spec: String,
    pub frame: u32,
    pub end: u32,
    pub out: PathBuf,
    /// One `fn_sim::scenarios::frame_line` per frame (the log fn-sim writes), with the body pose hash.
    pub log: String,
    pub shots: Vec<PathBuf>,
    /// Interval between frame starts (wall clock) and CPU time spent in sim + pose + encode.
    pub interval: FrameStats,
    pub cpu: FrameStats,
    /// Frames that rendered a capture (offscreen render + read-back + PNG encode): reported separately so they do not pollute the percentiles.
    pub capture_ms: FrameStats,
    pub captured_frame: bool,
    /// `--screenshot FILE`: the single capture goes exactly here.
    pub shot_override: Option<PathBuf>,
    pub started: std::time::Instant,
    pub failures: Vec<String>,
    pub sizes_seen: Vec<(u32, u32)>,
    /// PERF: CPU ms of the scene draw (renderer encode + submit), the UI render + present, and the GPU timestamps per pass.
    pub draw_ms: FrameStats,
    pub ui_present_ms: FrameStats,
    pub gpu_passes: std::collections::BTreeMap<String, (&'static str, FrameStats)>,
    pub gpu_sum: FrameStats,
    /// PERF: CPU encode ms per graph pass (+ "prepare" = renderer CPU before the graph).
    pub cpu_passes: std::collections::BTreeMap<String, FrameStats>,
}

impl ScriptRun {
    pub fn new(spec: &str, driver: Box<dyn Driver>, out: PathBuf, max_frames: Option<u32>) -> ScriptRun {
        let end = driver.end_frame().min(max_frames.unwrap_or(u32::MAX));
        ScriptRun { driver, spec: spec.to_owned(), frame: 0, end, out, log: String::new(), shots: vec![], interval: FrameStats::default(), cpu: FrameStats::default(), capture_ms: FrameStats::default(), captured_frame: false, shot_override: None, started: std::time::Instant::now(), failures: vec![], sizes_seen: vec![], draw_ms: FrameStats::default(), ui_present_ms: FrameStats::default(), gpu_passes: Default::default(), gpu_sum: FrameStats::default(), cpu_passes: Default::default() }
    }
    pub fn done(&self) -> bool { self.frame >= self.end || self.driver.quit(self.frame.saturating_sub(1)) && self.frame > 0 }
    pub fn shot_path(&self, name: &str, size: (u32, u32)) -> PathBuf {
        match &self.shot_override { Some(p) => p.clone(), None => self.out.join(format!("{name}-{}x{}.png", size.0, size.1)) }
    }
    /// PERF: GPU pass timings that arrived this frame (passes of the same name in one frame are summed).
    pub fn push_gpu(&mut self, t: Vec<fn_render::profiler::PassTimings>) {
        for f in t {
            let mut per: std::collections::BTreeMap<String, (&'static str, f32)> = Default::default();
            for p in &f.passes { let e = per.entry(p.name.clone()).or_insert((p.group, 0.)); e.1 += p.ms; }
            for (k, (g, ms)) in per { self.gpu_passes.entry(k).or_insert((g, FrameStats::default())).1.push(ms); }
            self.gpu_sum.push(f.passes.iter().filter(|p| p.group != "node").map(|p| p.ms).sum());
            let nodes: f32 = f.passes.iter().filter(|p| p.group == "node").map(|p| p.ms).sum(); if nodes > 0. { self.gpu_passes.entry("(all graph nodes)".into()).or_insert(("node", FrameStats::default())).1.push(nodes); }
        }
    }
    /// PERF: the renderer's CPU split of this frame.
    pub fn push_cpu_passes(&mut self, s: &fn_render::FrameStats) {
        let enc: f32 = s.passes.iter().map(|p| p.encode_ms).sum();
        self.cpu_passes.entry("(prepare, before the graph)".into()).or_default().push(s.cpu_ms - enc);
        let mut per: std::collections::BTreeMap<&str, f32> = Default::default();
        for p in &s.passes { *per.entry(p.name.as_str()).or_default() += p.encode_ms; }
        for (k, v) in per { self.cpu_passes.entry(k.to_owned()).or_default().push(v); }
    }
    /// PERF: per-pass GPU table, sorted by mean.
    pub fn gpu_report(&self) -> String {
        let mut out = String::new();
        let mut rows: Vec<(String, crate::stats::Summary)> = self.cpu_passes.iter().filter_map(|(k, s)| s.summary(5.min(s.ms.len().saturating_sub(1))).map(|m| (k.clone(), m))).collect();
        rows.sort_by(|a, b| b.1.mean.partial_cmp(&a.1.mean).unwrap());
        for (k, m) in rows.into_iter().take(25) { out.push_str(&format!("  cpu {:<34} mean {:7.3} ms  p95 {:7.3}  max {:7.3}  ({} frames)\n", k, m.mean, m.p95, m.max, m.frames)); }
        out.push_str(&self.gpu_sum.report("gpu sum of timed passes", 5));
        let mut rows: Vec<(String, &'static str, crate::stats::Summary)> = self.gpu_passes.iter().filter_map(|(k, (g, s))| s.summary(5.min(s.ms.len().saturating_sub(1))).map(|m| (k.clone(), *g, m))).collect();
        rows.sort_by(|a, b| b.2.mean.partial_cmp(&a.2.mean).unwrap());
        for (k, g, m) in rows { out.push_str(&format!("  gpu {:<34} [{:<6}] mean {:7.3} ms  p95 {:7.3}  max {:7.3}  ({} frames)\n", k, g, m.mean, m.p95, m.max, m.frames)); }
        // PERF (2026-10-06, optimisation pass): the whole GPU frame (FN_GPU_NODES=1: every graph node timed) as a report line with p99 and the share of
        // frames over one 60 Hz vsync interval
        if let Some((_, (_, s))) = self.gpu_passes.iter().find(|(k, _)| k.starts_with("(all graph nodes)")) { out.push_str(&s.report("gpu frame (all graph nodes)", 5)); }
        out
    }
    pub fn log_frame(&mut self, f: u32, g: &GameSim, pose_hash: Option<u64>) { self.log.push_str(&frame_line(f, g, pose_hash)); self.log.push('\n'); }
}

/// The frame log line (fn-sim's format when built with `sim-compare`, so the two logs can be diffed).
pub fn frame_line(f: u32, g: &GameSim, pose: Option<u64>) -> String {
    #[cfg(feature = "sim-compare")]
    { fn_sim::scenarios::frame_line(f, g, pose) }
    #[cfg(not(feature = "sim-compare"))]
    { format!("f{f:05} pos {:.5} {:.5} {:.5} yaw {:.3} pitch {:.3} pose {}", g.x, g.y, g.z, g.yaw, g.pitch, pose.map_or("-".to_owned(), |h| format!("{h:016x}"))) }
}

pub fn save_png(path: &Path, w: u32, h: u32, rgba: &[u8]) -> Result<(), String> {
    if let Some(d) = path.parent() { std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?; }
    // captures are written opaque (the swapchain alpha is whatever the post pass leaves)
    let mut px = rgba.to_vec(); for p in px.chunks_exact_mut(4) { p[3] = 255; }
    image::save_buffer(path, &px, w, h, image::ColorType::Rgba8).map_err(|e| format!("{}: {e}", path.display()))
}

/// Result of diffing two frame logs: (report text, sim columns identical, whole lines identical).
pub struct Diff { pub text: String, pub sim_ok: bool, pub full_ok: bool }

#[cfg(feature = "sim-compare")]
pub fn compare_logs(actual: &str, reference: &str, label: &str) -> Diff {
    use fn_sim::compare::compare_text;
    let full = compare_text(actual, reference, 0., None, 0);
    // pose hashes come from the evaluator; compare the movement / animation-state / camera columns separately (everything before " pose ")
    let sim = compare_text(actual, reference, 0., Some((" pose ", "\u{0}")), 0);
    let mut t = format!("compare vs {label}\n  whole lines : {}\n  without pose: {}\n", full.summary(), sim.summary());
    for (n, a, b, why) in sim.first_mismatches.iter().chain(if sim.ok() { full.first_mismatches.iter() } else { [].iter() }) { t.push_str(&format!("  line {n}: {why}\n    app: {a}\n    ref: {b}\n")); }
    Diff { text: t, sim_ok: sim.ok(), full_ok: full.ok() }
}
#[cfg(not(feature = "sim-compare"))]
pub fn compare_logs(_a: &str, _b: &str, _label: &str) -> Diff { Diff { text: "compare: built without the sim-compare feature\n".into(), sim_ok: false, full_ok: false } }

/// Headless reference for a text/builtin script: a fresh `GameSim` stepped with the driver's inputs at the script's fixed rate (what fn-sim's
/// harness does), with the md5 rig of the asset tree for the pose hash when it can be loaded.
#[cfg(feature = "sim-compare")]
pub fn reference_log(mut driver: Box<dyn Driver>, assets: Option<&Path>, max_frames: Option<u32>) -> String {
    let rig = assets.and_then(|a| fn_sim::rig::Rig::load(a).ok());
    let mut g = GameSim::new();
    let dt = 1. / driver.fps();
    let mut out = String::new();
    for f in 0..driver.end_frame().min(max_frames.unwrap_or(u32::MAX)) {
        driver.prepare(f, &mut g);
        let inp = driver.input(f);
        g.step(&inp, dt);
        let h = rig.as_ref().map(|r| fn_sim::rig::pose_hash(&r.eval(&g.pose_tree()).pose));
        out.push_str(&fn_sim::scenarios::frame_line(f, &g, h)); out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn run_ends_at_the_script_end_or_the_frame_limit() {
        let d = crate::script::builtin("walk").unwrap().unwrap(); let end = d.end_frame();
        let r = ScriptRun::new("walk", d, PathBuf::from("o"), None); assert_eq!(r.end, end);
        let r = ScriptRun::new("walk", crate::script::builtin("walk").unwrap().unwrap(), PathBuf::from("o"), Some(10)); assert_eq!(r.end, 10);
    }
    #[test] fn shot_paths_carry_the_size_unless_overridden() {
        let mut r = ScriptRun::new("w", crate::script::builtin("walk").unwrap().unwrap(), PathBuf::from("out"), None);
        assert_eq!(r.shot_path("idle", (1280, 720)), PathBuf::from("out").join("idle-1280x720.png"));
        r.shot_override = Some(PathBuf::from("x.png")); assert_eq!(r.shot_path("idle", (1, 1)), PathBuf::from("x.png"));
    }
    #[test] fn png_capture_is_written_opaque() {
        let p = std::env::temp_dir().join(format!("fn-app-shot-{}.png", std::process::id()));
        save_png(&p, 2, 1, &[10, 20, 30, 0, 40, 50, 60, 7]).unwrap();
        let img = image::open(&p).unwrap().to_rgba8(); assert_eq!(img.get_pixel(0, 0).0, [10, 20, 30, 255]); assert_eq!(img.get_pixel(1, 0).0, [40, 50, 60, 255]); let _ = std::fs::remove_file(p);
    }
    #[cfg(feature = "sim-compare")]
    #[test] fn the_headless_reference_of_a_text_script_equals_itself_and_differs_when_the_input_differs() {
        let a = reference_log(crate::script::builtin("walk").unwrap().unwrap(), None, Some(120));
        let b = reference_log(crate::script::builtin("walk").unwrap().unwrap(), None, Some(120));
        assert_eq!(a.lines().count(), 120); let d = compare_logs(&a, &b, "self"); assert!(d.sim_ok && d.full_ok, "{}", d.text);
        let other = reference_log(Box::new(crate::script::Script::parse("t", "end 120").unwrap()), None, None);
        assert!(!compare_logs(&a, &other, "idle").sim_ok);
    }
}
