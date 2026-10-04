//! Performance comparison: frame-time percentiles (from each executable's own smoke-test report), peak memory and CPU time (measured by the
//! process helper) for the same 880-frame smoke timeline.
use crate::config::*;
use crate::exec::Ctx;
use crate::logs::number_after;
use crate::model::*;

#[derive(Clone, Debug, Default)]
pub struct FrameStats { pub frames: f64, pub mean: f64, pub p50: f64, pub p95: f64, pub p99: f64, pub max: f64, pub vsync: Option<bool> }

/// Old `smoke-perf.txt`: `frames 780 (steady subset)` / `frame ms: mean 6.942 median 6.945 p95 7.013 p99 7.055 max 27.744` / `vsync on`.
pub fn parse_old(text: &str) -> Option<FrameStats> {
    let l = text.lines().find(|l| l.starts_with("frame ms:"))?;
    let frames = text.lines().find(|l| l.starts_with("frames "))
        .and_then(|l| l[7..].split_whitespace().next()?.parse::<f64>().ok()).unwrap_or(0.);
    let vsync = text.lines().find(|l| l.trim() == "vsync on" || l.trim() == "vsync off").map(|l| l.trim() == "vsync on");
    Some(FrameStats { frames, mean: number_after(l, "mean ")?, p50: number_after(l, "median ")?, p95: number_after(l, "p95 ")?, p99: number_after(l, "p99 ")?, max: number_after(l, "max ")?, vsync })
}

/// New `<script>-report.txt`: `frame interval (wall, incl. present): 861 frames (first 5 skipped) mean 4.90 ms ... median 3.96 p95 9.20 p99 11.02 max 104.28`.
pub fn parse_new(text: &str) -> Option<FrameStats> {
    let l = text.lines().find(|l| l.starts_with("frame interval"))?;
    let frames = number_after(l, "): ")?;
    let vsync = text.lines().find(|l| l.contains("vsync")).map(|l| !l.contains("off"));
    Some(FrameStats { frames, mean: number_after(l, "mean ")?, p50: number_after(l, "median ")?, p95: number_after(l, "p95 ")?, p99: number_after(l, "p99 ")?, max: number_after(l, "max ")?, vsync })
}

pub fn parse_new_total_frames(text: &str) -> Option<f64> { let l = text.lines().find(|l| l.starts_with("frames "))?; l[7..].split_whitespace().next()?.parse().ok() }
pub fn parse_old_total_frames(text: &str) -> Option<f64> { number_after(text, "ms/frame:").and_then(|_| { let i = text.find(" over ")?; text[i + 6..].split_whitespace().next()?.parse().ok() }) }

fn run<'a>(ctx: &'a Ctx, side: &str, id: &str) -> Option<&'a RunRecord> { ctx.runs.iter().rev().find(|r| r.side == side && r.id == id) }
fn read(p: &std::path::Path) -> String { std::fs::read_to_string(p).unwrap_or_default() }

fn ratio_item(ctx: &Ctx, id: &str, label: &str, key: &str, new: f64, old: f64, unit: &str, note: &str) -> Item {
    let t = ctx.cfg.pair_or(&format!("perf.{key}"), (1.5, 3.));
    let mut it = Item::new(format!("perf:{id}"), Area::Performance, "perf", label);
    if old <= 0. || new <= 0. { return it.na("a side did not report this number"); }
    let r = new / old; let s = score_lower(r, t);
    it.headline = format!("new {new:.2}{unit} vs old {old:.2}{unit} (x{r:.2})"); it.note = note.to_owned();
    it.metric("new", format!("{new:.2}{unit}"), Grade::Na).metric("old", format!("{old:.2}{unit}"), Grade::Na).metric("ratio new/old", format!("{r:.2}"), item_grade(s)).scored(s)
}

pub fn perf_items(ctx: &Ctx) -> Vec<Item> {
    let mut out = vec![];
    let old_dir = ctx.work_old("smoke-test"); let oldu_dir = ctx.work_old("smoke-test-uncapped");
    let (new_nv, new_v) = (ctx.work_new("smoke"), ctx.work_new("smoke-vsync"));
    let report = |d: &std::path::Path| { let r = std::fs::read_dir(d).ok().and_then(|rd| rd.flatten().find(|e| e.file_name().to_string_lossy().ends_with("-report.txt")).map(|e| e.path())); r.map(|p| read(&p)).unwrap_or_default() };
    let (old_v, old_u) = (parse_old(&read(&old_dir.join("smoke-perf.txt"))), parse_old(&read(&oldu_dir.join("smoke-perf.txt"))));
    let (new_vs, new_us) = (parse_new(&report(&new_v)), parse_new(&report(&new_nv)));
    let n = "Old: its smoke-perf steady-state subset; new: frame interval incl. present, first 5 frames skipped. The 880-frame timelines are the same script, the old one is driven in real time.";
    match (&old_v, &new_vs) {
        (Some(o), Some(nw)) => { out.push(ratio_item(ctx, "p50-vsync", "frame time p50, vsync on", "p50_ratio", nw.p50, o.p50, " ms", n)); out.push(ratio_item(ctx, "p99-vsync", "frame time p99, vsync on", "p99_ratio", nw.p99, o.p99, " ms", n)); }
        _ => out.push(Item::new("perf:frametime-vsync", Area::Performance, "perf", "frame time, vsync on").na("old smoke-perf.txt or the new smoke report is missing")),
    }
    match (&old_u, &new_us) {
        (Some(o), Some(nw)) if o.vsync == Some(false) => { out.push(ratio_item(ctx, "p50-uncapped", "frame time p50, vsync off", "p50_ratio", nw.p50, o.p50, " ms", n)); out.push(ratio_item(ctx, "p99-uncapped", "frame time p99, vsync off", "p99_ratio", nw.p99, o.p99, " ms", n)); }
        (Some(_), Some(_)) => out.push(Item::new("perf:frametime-uncapped", Area::Performance, "perf", "frame time, vsync off").na("the old exe ignored the scratch ini (still vsync on): no uncapped number")),
        _ => out.push(Item::new("perf:frametime-uncapped", Area::Performance, "perf", "frame time, vsync off").na("the uncapped smoke run of a side is missing")),
    }
    // memory / CPU from the measured processes (vsync-on runs: same pacing)
    if let (Some(o), Some(nw)) = (run(ctx, "old", "smoke-test"), run(ctx, "new", "smoke-vsync")) {
        out.push(ratio_item(ctx, "peak-ws", "peak working set", "mem_ratio", nw.peak_ws_mb, o.peak_ws_mb, " MB", "Whole process incl. loading and the 14 capture read-backs."));
        let (of, nf) = (parse_old_total_frames(&read(&old_dir.join("smoke-perf.txt"))).unwrap_or(0.), parse_new_total_frames(&report(&new_v)).unwrap_or(0.));
        if of > 0. && nf > 0. && o.cpu_s > 0. && nw.cpu_s > 0. {
            out.push(ratio_item(ctx, "cpu-per-frame", "CPU time per frame (process total / frames)", "cpu_ratio", nw.cpu_s * 1000. / nf, o.cpu_s * 1000. / of, " ms", "Includes start-up (shader compile, asset load) and capture work on both sides."));
        } else { out.push(Item::new("perf:cpu", Area::Performance, "perf", "CPU time per frame").na("CPU time or frame count unavailable")); }
    } else { out.push(Item::new("perf:memory", Area::Performance, "perf", "peak working set / CPU time").na("process measurements unavailable (--reuse or a run failed)")); }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn old_report_parses() {
        let t = "frames 780 (steady subset)\nframe ms: mean 6.942 median 6.945 p95 7.013 p99 7.055 max 27.744\nvsync on\nauto-exposure metering (mip generation + readback, CPU time) ms/frame: 0.157 over 958 frames\n";
        let s = parse_old(t).unwrap(); assert_eq!((s.p50, s.p99, s.vsync), (6.945, 7.055, Some(true))); assert_eq!(parse_old_total_frames(t), Some(958.));
    }
    #[test] fn new_report_parses() {
        let t = "frames 880 in 8.49 s, fixed dt 0.01667 s\nframe interval (wall, incl. present): 861 frames (first 5 skipped) mean 4.90 ms (204 fps) median 3.96 p95 9.20 p99 11.02 max 104.28 min 0.85\n";
        let s = parse_new(t).unwrap(); assert_eq!((s.frames, s.p50, s.p99), (861., 3.96, 11.02)); assert_eq!(parse_new_total_frames(t), Some(880.));
    }
}
