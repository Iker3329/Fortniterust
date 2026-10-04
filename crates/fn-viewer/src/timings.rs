//! `--gpu-timings`: per-pass GPU time table (timestamp queries) over N frames, mean / p99 / max.
use crate::{Cli, Session};
use fn_render::{FrameParams, PassTimings};
use std::fmt::Write;

pub fn mean(v: &[f32]) -> f32 { if v.is_empty() { 0. } else { v.iter().sum::<f32>() / v.len() as f32 } }
/// p-th percentile (0..1) by the nearest-rank method.
pub fn percentile(v: &[f32], p: f32) -> f32 {
    if v.is_empty() { return 0.; }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((p * s.len() as f32).ceil() as usize).clamp(1, s.len()) - 1]
}

/// Builds the report from the per-frame timings.
pub fn report(frames: &[PassTimings], wall_ms: &[f32], cpu_ms: &[f32], size: (u32, u32), gpu: &str, prop_vertices: u32) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "GPU pass timings {}x{} on {gpu}, {} frames, {} prop vertices (ms)", size.0, size.1, frames.len(), prop_vertices);
    let _ = writeln!(out, "{:<20} {:>8} {:>8} {:>8} {:>8} {:>7}", "pass", "mean", "p50", "p99", "max", "% sum");
    // pass names in first-seen order (frames without shadows etc. simply have fewer rows)
    let mut names: Vec<(String, &'static str)> = vec![];
    for f in frames { for p in &f.passes { if !names.iter().any(|n| n.0 == p.name) { names.push((p.name.clone(), p.group)); } } }
    let series = |name: &str| -> Vec<f32> { frames.iter().map(|f| f.passes.iter().find(|p| p.name == name).map_or(0., |p| p.ms)).collect() };
    let sums: Vec<f32> = frames.iter().map(|f| f.sum_ms()).collect();
    let total_mean = mean(&sums).max(1e-9);
    for (n, _) in &names {
        let s = series(n);
        let _ = writeln!(out, "{:<20} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>6.1}%", n, mean(&s), percentile(&s, 0.5), percentile(&s, 0.99), s.iter().cloned().fold(0., f32::max), 100. * mean(&s) / total_mean);
    }
    let _ = writeln!(out, "-- groups --");
    let mut groups: Vec<&'static str> = vec![];
    for (_, g) in &names { if !groups.contains(g) { groups.push(g); } }
    for g in groups {
        let s: Vec<f32> = frames.iter().map(|f| f.group_ms(g)).collect();
        let _ = writeln!(out, "{:<20} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>6.1}%", g, mean(&s), percentile(&s, 0.5), percentile(&s, 0.99), s.iter().cloned().fold(0., f32::max), 100. * mean(&s) / total_mean);
    }
    let span: Vec<f32> = frames.iter().map(|f| f.span_ms).collect();
    let _ = writeln!(out, "{:<20} {:>8.3} {:>8.3} {:>8.3} {:>8.3}", "sum of passes", mean(&sums), percentile(&sums, 0.5), percentile(&sums, 0.99), sums.iter().cloned().fold(0., f32::max));
    let _ = writeln!(out, "{:<20} {:>8.3} {:>8.3} {:>8.3} {:>8.3}", "gpu span", mean(&span), percentile(&span, 0.5), percentile(&span, 0.99), span.iter().cloned().fold(0., f32::max));
    let _ = writeln!(out, "{:<20} {:>8.3} {:>8.3} {:>8.3} {:>8.3}", "wall (submit..idle)", mean(wall_ms), percentile(wall_ms, 0.5), percentile(wall_ms, 0.99), wall_ms.iter().cloned().fold(0., f32::max));
    let _ = writeln!(out, "{:<20} {:>8.3} {:>8.3} {:>8.3} {:>8.3}", "cpu encode+submit", mean(cpu_ms), percentile(cpu_ms, 0.5), percentile(cpu_ms, 0.99), cpu_ms.iter().cloned().fold(0., f32::max));
    out
}

pub fn run(cli: &Cli) -> Result<(), String> {
    let mut s = Session::open(cli)?;
    if !s.scene.renderer.gpu_timing_supported() { return Err("this device has no TIMESTAMP_QUERY support, cannot measure GPU pass times".into()); }
    let frames = if cli.frames <= 3 { 300 } else { cli.frames } as usize;
    let ids = s.scene.model_ids();
    let (mut all, mut wall, mut cpu) = (vec![], vec![], vec![]);
    // Frames are submitted in bursts of BURST (the GPU stays busy, clocks ramp like in a real frame loop) and drained after each burst;
    // wall time is burst time / BURST. 60 warm-up frames are not counted.
    const BURST: usize = 4;
    let warm = 60;
    let mut k = 0usize;
    while all.len() < frames {
        let t = std::time::Instant::now();
        for _ in 0..BURST {
            s.scene.animate(cli.anim_time + k as f32 / 60.);
            let fp = FrameParams { camera: &s.cam, time: cli.sky_time + k as f32 / 60., dt: 1. / 60., models: &ids, options: &cli.options };
            let stats = match s.hud.as_mut() {
                Some(h) => { h.update(s.size.0, s.size.1, cli.yaw, 1. / 60.); s.scene.renderer.render_with_overlay(&fp, &s.target.view, Some(&mut h.pass())) }
                None => s.scene.renderer.render(&fp, &s.target.view),
            };
            if k >= warm { cpu.push(stats.cpu_ms); }
            k += 1;
        }
        s.scene.renderer.wait_idle();
        let w = t.elapsed().as_secs_f32() * 1000. / BURST as f32;
        s.scene.renderer.flush_gpu_timings();
        let done = s.scene.renderer.take_pass_timings();
        if k > warm { for d in done { all.push(d); wall.push(w); } }
    }
    all.truncate(frames);
    wall.truncate(frames);
    cpu.truncate(frames);
    let ui = if s.scene.renderer.last_pass_timings().passes.iter().any(|p| p.group == "ui") { "" } else { " (no UI row: run with --demo-ui)" };
    print!("{}", report(&all, &wall, &cpu, s.size, &s.gpu.describe(), s.scene.renderer.prop_vertex_count()));
    println!("{ui}");
    let st = s.scene.renderer.static_stats();
    if st.instances > 0 {
        println!("static meshes: {} instances, {} visible (LOD {:?}), {} culled by distance, {} by the frustum; {} draw calls + {} shadow draw calls (3 cascades, {} shadow instances); {} triangles in the main view", st.instances, st.visible, &st.lod_counts[..3], st.culled_distance, st.culled_frustum, st.draw_calls, st.shadow_draw_calls, st.shadow_instances, st.triangles);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_render::GpuPassTime;
    #[test]
    fn percentile_is_nearest_rank() {
        let v: Vec<f32> = (1..=100).map(|x| x as f32).collect();
        assert_eq!(percentile(&v, 0.99), 99.);
        assert_eq!(percentile(&v, 1.0), 100.);
        assert_eq!(percentile(&v, 0.5), 50.);
        assert_eq!(percentile(&[], 0.99), 0.);
        assert!((mean(&[1., 2., 3.]) - 2.).abs() < 1e-6);
    }
    #[test]
    fn report_lists_passes_groups_and_totals() {
        let f = |a: f32, b: f32| PassTimings { supported: true, frame: 1, span_ms: a + b, passes: vec![
            GpuPassTime { name: "scene".into(), group: "scene", ms: a }, GpuPassTime { name: "post".into(), group: "post", ms: b }] };
        let r = report(&[f(1., 0.5), f(2., 0.5)], &[3., 4.], &[0.2, 0.2], (64, 64), "test gpu", 0);
        assert!(r.contains("scene") && r.contains("post") && r.contains("-- groups --") && r.contains("sum of passes"));
        assert!(r.contains("1.500"), "{r}"); // mean scene
    }
}
