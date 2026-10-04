//! Capture scenes of the world feedback HUD pieces (reticle, harvest popups, pack highlight / count / cap, Target Info, interaction prompt, equip progress).
//!
//! The `ret-*` scenes reproduce the old prototype's `--reticle-test` matrix (old `main.rs`, `verification/ret-<scene>-<W>x<H>.bmp`, state log `verification/reticle-test.txt`):
//! the HUD at its defaults (pickaxe in slot 1, packs, backpack row, compass, minimap, counters) with the reticle forced white / yellow, the `HUD.Center.Reticle` toggle off, or
//! the popup driven through the recorded event script. The old captures have the 3D world behind the HUD, so only the pixels our UI covers opaquely are comparable
//! (`SceneDef::over_world`). The old script, replayed cumulatively here (`HARVEST_SCRIPT`), each step = (capture name, pack counts, hit fed before advancing, seconds advanced in
//! 1/60 s steps):
//!   hit1 (wood, 20, packs [20,0,0]) t0.05 / +0.2 -> t0.25 / +0.45 -> t0.70 | hit2 (packs [40,0,0]) +0.1 | weak point (packs [60,0,0]) +0.1 then +0.4 | +2.8 -> outro t0.30
//!   | +0.6 -> outro t0.90 | +0.5 -> hidden | stone (object 2, destroyed) +0.1, +1.3 -> stone outro t0.40.
//! The old captures never ticked the count-up (the counters show the target at once), and the reticle was forced white during the popup captures: both reproduced.
//!
//! The scenes without an old capture (`pack-*`, `target-info-*`, `interaction-*`, `equip-progress-*`) use TEST inputs, not game data.
use crate::draw::RenderList;
use crate::scene::{SceneDef, UiContext};
use crate::screens::hud::Hud;
use crate::screens::reticle_state::{EquipProgress, HarvestHit, HarvestResource, HealthDisplayRule, InteractionPrompt, TargetInfo};

/// One step of the old `--reticle-test` popup script.
pub struct Step { pub name: &'static str, pub packs: [u32; 3], pub hit: Option<HarvestHit>, pub advance: f32 }
const fn hit(object: u64, resource: HarvestResource, destroyed: bool, weakpoint: bool) -> Option<HarvestHit> { Some(HarvestHit { object, resource, amount: 20, destroyed, weakpoint }) }
pub const HARVEST_SCRIPT: [Step; 11] = [
    Step { name: "harvest-hit1-t0.05", packs: [20, 0, 0], hit: hit(1, HarvestResource::Wood, false, false), advance: 0.05 },
    Step { name: "harvest-hit1-t0.25", packs: [20, 0, 0], hit: None, advance: 0.2 },
    Step { name: "harvest-hit1-t0.70", packs: [20, 0, 0], hit: None, advance: 0.45 },
    Step { name: "harvest-hit2-t0.10", packs: [40, 0, 0], hit: hit(1, HarvestResource::Wood, false, false), advance: 0.1 },
    Step { name: "harvest-weakpoint-t0.10", packs: [60, 0, 0], hit: hit(1, HarvestResource::Wood, false, true), advance: 0.1 },
    Step { name: "harvest-weakpoint-t0.50", packs: [60, 0, 0], hit: None, advance: 0.4 },
    Step { name: "harvest-outro-t0.30", packs: [60, 0, 0], hit: None, advance: 2.8 },
    Step { name: "harvest-outro-t0.90", packs: [60, 0, 0], hit: None, advance: 0.6 },
    Step { name: "harvest-hidden", packs: [60, 0, 0], hit: None, advance: 0.5 },
    Step { name: "harvest-stone-destroyed-t0.10", packs: [60, 0, 0], hit: hit(2, HarvestResource::Stone, true, false), advance: 0.1 },
    Step { name: "harvest-stone-outro-t0.40", packs: [60, 0, 0], hit: None, advance: 1.3 },
];

/// A fresh HUD in the state the old captures start from (pickaxe equipped, nothing harvested, reticle white).
pub fn base_hud() -> Hud { Hud::new() }

fn draw(ctx: &mut UiContext, w: f32, h: f32, hud: &mut Hud) -> RenderList { let mut out = RenderList::new(); hud.present(ctx, w, h, &mut out); out }

/// `ret-white` / `ret-yellow` / `ret-toggle-off`.
pub fn reticle_scene(ctx: &mut UiContext, w: f32, h: f32, on_target: bool, toggle_on: bool) -> RenderList {
    let mut hud = base_hud(); hud.state.world.reticle.on_target = on_target; hud.state.visible.reticle = toggle_on;
    draw(ctx, w, h, &mut hud)
}
/// The popup capture `name` of [`HARVEST_SCRIPT`]: replays the script from its start up to and including that step.
pub fn harvest_scene(ctx: &mut UiContext, w: f32, h: f32, name: &str) -> RenderList {
    let mut hud = base_hud();
    for st in HARVEST_SCRIPT.iter() {
        hud.state.resources = st.packs;
        if let Some(e) = st.hit { hud.reticle.on_hit(e); }
        hud.reticle.popup.advance(st.advance);
        if st.name == name { return draw(ctx, w, h, &mut hud); }
    }
    panic!("unknown harvest capture {name}");
}

fn ret_white(c: &mut UiContext, w: f32, h: f32) -> RenderList { reticle_scene(c, w, h, false, true) }
fn ret_yellow(c: &mut UiContext, w: f32, h: f32) -> RenderList { reticle_scene(c, w, h, true, true) }
fn ret_toggle_off(c: &mut UiContext, w: f32, h: f32) -> RenderList { reticle_scene(c, w, h, false, false) }
macro_rules! harvest { ($f:ident, $name:expr) => { fn $f(c: &mut UiContext, w: f32, h: f32) -> RenderList { harvest_scene(c, w, h, $name) } }; }
harvest!(hv_hit1_005, "harvest-hit1-t0.05");
harvest!(hv_hit1_025, "harvest-hit1-t0.25");
harvest!(hv_hit1_070, "harvest-hit1-t0.70");
harvest!(hv_hit2_010, "harvest-hit2-t0.10");
harvest!(hv_weak_010, "harvest-weakpoint-t0.10");
harvest!(hv_weak_050, "harvest-weakpoint-t0.50");
harvest!(hv_outro_030, "harvest-outro-t0.30");
harvest!(hv_outro_090, "harvest-outro-t0.90");
harvest!(hv_hidden, "harvest-hidden");
harvest!(hv_stone_010, "harvest-stone-destroyed-t0.10");
harvest!(hv_stone_outro, "harvest-stone-outro-t0.40");

// ---- scenes without an old capture (test inputs) -------------------------------------------------------------------------------
/// Packs at the stack cap with the stone pack as the current resource: count text at alpha 0.6 for 999, highlighted border on stone.
fn pack_cap(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut hud = base_hud(); hud.state.resources = [999, 45, 3]; hud.state.world.current_resource = HarvestResource::Stone; draw(c, w, h, &mut hud)
}
/// Pack count-up half way (0.25 s of the 0.5 s interpolation from 0 to 100 on wood).
fn pack_countup(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut hud = base_hud(); hud.state.resources = [0; 3]; hud.tick(1. / 60.); hud.state.resources = [100, 0, 0]; hud.tick(0.25); draw(c, w, h, &mut hud)
}
fn target(health: f32, rule: HealthDisplayRule) -> TargetInfo { TargetInfo { id: 1, health, max_health: 600., rule, interaction: 0, screen: Some((960., 420.)) } }
fn target_info_damaged(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut hud = base_hud();
    // focus a full-health prop (rule OnDamage: hidden), then hit it: the bar appears with the 3 s hide timer running
    hud.state.world.target = Some(target(600., HealthDisplayRule::OnDamage)); let _ = draw(c, w, h, &mut hud);
    hud.state.world.target = Some(target(330., HealthDisplayRule::OnDamage)); hud.tick(0.5); draw(c, w, h, &mut hud)
}
fn target_info_full(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut hud = base_hud(); hud.state.world.target = Some(target(600., HealthDisplayRule::Always)); draw(c, w, h, &mut hud)
}
fn interaction_prompt(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut hud = base_hud();
    hud.state.world.interaction = Some(InteractionPrompt { key: "E".into(), text: "Open Chest".into(), screen: Some((w * 0.5, h * 0.55)) }); draw(c, w, h, &mut hud)
}
fn equip_progress_25(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut hud = base_hud(); hud.state.world.equip = Some(EquipProgress { elapsed: 0.5, duration: 2. }); draw(c, w, h, &mut hud)
}
fn equip_progress_75(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut hud = base_hud(); hud.state.world.equip = Some(EquipProgress { elapsed: 1.5, duration: 2. }); draw(c, w, h, &mut hud)
}

pub fn all() -> Vec<SceneDef> {
    let hud = |name: &'static str, old: Option<&'static str>, build: fn(&mut UiContext, f32, f32) -> RenderList| SceneDef { name, old_capture: old, over_world: true, build };
    vec![
        hud("ret-white", Some("ret-white-{w}x{h}"), ret_white),
        hud("ret-yellow", Some("ret-yellow-{w}x{h}"), ret_yellow),
        hud("ret-toggle-off", Some("ret-toggle-off-{w}x{h}"), ret_toggle_off),
        hud("ret-harvest-hit1-t0.05", Some("ret-harvest-hit1-t0.05-{w}x{h}"), hv_hit1_005),
        hud("ret-harvest-hit1-t0.25", Some("ret-harvest-hit1-t0.25-{w}x{h}"), hv_hit1_025),
        hud("ret-harvest-hit1-t0.70", Some("ret-harvest-hit1-t0.70-{w}x{h}"), hv_hit1_070),
        hud("ret-harvest-hit2-t0.10", Some("ret-harvest-hit2-t0.10-{w}x{h}"), hv_hit2_010),
        hud("ret-harvest-weakpoint-t0.10", Some("ret-harvest-weakpoint-t0.10-{w}x{h}"), hv_weak_010),
        hud("ret-harvest-weakpoint-t0.50", Some("ret-harvest-weakpoint-t0.50-{w}x{h}"), hv_weak_050),
        hud("ret-harvest-outro-t0.30", Some("ret-harvest-outro-t0.30-{w}x{h}"), hv_outro_030),
        hud("ret-harvest-outro-t0.90", Some("ret-harvest-outro-t0.90-{w}x{h}"), hv_outro_090),
        hud("ret-harvest-hidden", Some("ret-harvest-hidden-{w}x{h}"), hv_hidden),
        hud("ret-harvest-stone-destroyed-t0.10", Some("ret-harvest-stone-destroyed-t0.10-{w}x{h}"), hv_stone_010),
        hud("ret-harvest-stone-outro-t0.40", Some("ret-harvest-stone-outro-t0.40-{w}x{h}"), hv_stone_outro),
        hud("pack-cap-highlight", None, pack_cap),
        hud("pack-countup-half", None, pack_countup),
        hud("target-info-damaged", None, target_info_damaged),
        hud("target-info-full", None, target_info_full),
        hud("interaction-prompt", None, interaction_prompt),
        hud("equip-progress-25", None, equip_progress_25),
        hud("equip-progress-75", None, equip_progress_75),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{Cmd, MaterialCmd, MaterialKind};
    use crate::screens::testutil::ctx;

    #[test] fn scene_names_are_unique_registered_and_over_world() {
        let v = all(); let mut n: Vec<_> = v.iter().map(|s| s.name).collect(); n.sort(); n.dedup(); assert_eq!(n.len(), v.len());
        let reg = crate::scene::all_scenes();
        for s in &v { assert!(reg.iter().any(|r| r.name == s.name), "{} not wired into all_scenes", s.name); assert!(s.over_world); }
        assert_eq!(v.iter().filter(|s| s.old_capture.is_some()).count(), 14, "3 reticle + 11 popup captures have an old BMP");
    }
    #[test] fn every_script_name_has_a_scene() {
        let v = all(); for st in HARVEST_SCRIPT.iter() { assert!(v.iter().any(|s| s.name == format!("ret-{}", st.name)), "{}", st.name); }
    }
    #[test] fn old_capture_templates_resolve_to_the_old_bmps() {
        let root = crate::screens::testutil::old_root(); if !root.is_dir() { eprintln!("old tree not found, skipping"); return; }
        for s in all() { let Some(t) = s.old_capture else { continue };
            for (w, h) in [(1920, 1080), (1280, 720)] { let stem = t.replace("{w}", &w.to_string()).replace("{h}", &h.to_string()); assert!(root.join("verification").join(format!("{stem}.bmp")).is_file(), "{stem}"); }
        }
    }
    #[test] fn every_scene_builds_finite_deterministic_lists_at_several_sizes() {
        let Some(mut c) = ctx() else { return };
        for s in all() { for (w, h) in [(1920., 1080.), (1280., 720.), (1024., 768.), (640., 480.), (2560., 1080.)] {
            let l = (s.build)(&mut c, w, h); assert!(!l.is_empty(), "{} {w}x{h}", s.name);
            for cmd in &l.cmds { match cmd {
                Cmd::Quad(q) => assert!(q.pos.iter().flatten().chain(q.uv.iter()).chain(q.tint.iter()).all(|v| v.is_finite()), "{} NaN", s.name),
                Cmd::Material(m) => assert!([m.rect.x, m.rect.y, m.rect.w, m.rect.h].iter().all(|v| v.is_finite()), "{}", s.name), _ => {} } }
            assert_eq!(l.cmds, (s.build)(&mut c, w, h).cmds, "{} not deterministic", s.name);
        } }
    }
    // ---- numeric comparison against the old captures (the same metrics as `fn-ui-demo --diff`, CPU rasteriser) ---------------------------------------------
    fn old_bmp(name: &str, w: u32, h: u32) -> Option<image::RgbImage> {
        let p = crate::screens::testutil::old_root().join("verification").join(format!("ret-{name}-{w}x{h}.bmp"));
        image::open(p).ok().map(|i| i.to_rgb8())
    }
    /// Mean abs RGB difference (0..255) over the pixels our premultiplied RGBA covers with alpha >= 252 (valid over a 3D world), fraction of them off by > 8, and their count.
    fn masked_diff(ours: &[u8], old: &image::RgbImage) -> (f64, f64, usize) {
        let (mut sum, mut bad, mut n) = (0u64, 0usize, 0usize);
        for (o, p) in ours.chunks_exact(4).zip(old.as_raw().chunks_exact(3)) {
            if o[3] < 252 { continue; }
            let d = [o[0].abs_diff(p[0]), o[1].abs_diff(p[1]), o[2].abs_diff(p[2])];
            sum += d.iter().map(|v| *v as u64).sum::<u64>(); n += 1; if d.iter().any(|v| *v > 8) { bad += 1; }
        }
        (if n > 0 { sum as f64 / (n as f64 * 3.) } else { f64::NAN }, if n > 0 { bad as f64 / n as f64 } else { f64::NAN }, n)
    }
    /// Composite our premultiplied `ours` over the old capture `bg` (RGB) and compare with `want` inside the bounding box (grown by 3 px) of what `ours` covers at all:
    /// returns (mean abs RGB difference, max channel difference, pixels compared). This also checks the translucent edge pixels, unlike `masked_diff`.
    fn overlay_diff(ours: &[u8], bg: &image::RgbImage, want: &image::RgbImage) -> (f64, u8, usize) {
        let (w, h) = (bg.width() as i32, bg.height() as i32);
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, -1, -1);
        for y in 0..h { for x in 0..w { if ours[((y * w + x) * 4 + 3) as usize] > 0 { x0 = x0.min(x); y0 = y0.min(y); x1 = x1.max(x); y1 = y1.max(y); } } }
        if x1 < 0 { return (0., 0, 0); }
        let (mut sum, mut mx, mut n) = (0u64, 0u8, 0usize);
        for y in (y0 - 3).max(0)..=(y1 + 3).min(h - 1) { for x in (x0 - 3).max(0)..=(x1 + 3).min(w - 1) {
            let i = ((y * w + x) * 4) as usize; let a = ours[i + 3] as u32; let (b, t) = (bg.get_pixel(x as u32, y as u32), want.get_pixel(x as u32, y as u32));
            for k in 0..3 { let c = (ours[i + k] as u32 + (b[k] as u32 * (255 - a) + 127) / 255).min(255) as u8; let d = c.abs_diff(t[k]); sum += d as u64; mx = mx.max(d); }
            n += 1;
        } }
        (sum as f64 / (n as f64 * 3.), mx, n)
    }
    /// Reticle / popup layer alone as premultiplied RGBA (what `overlay_diff` composites over the old world).
    fn layer(c: &mut UiContext, w: u32, h: u32, on_target: bool, script_until: Option<&str>) -> Vec<u8> {
        let mut r = crate::screens::reticle::ReticleHud::new(); let mut out = RenderList::new();
        if let Some(name) = script_until {
            for st in HARVEST_SCRIPT.iter() { if let Some(e) = st.hit { r.on_hit(e); } r.popup.advance(st.advance); if st.name == name { break; } }
            r.build_popup(c, w as f32, h as f32, &mut out); // (the old `harvest-hidden` background already holds the white reticle)
        } else { r.build_reticle(c, w as f32, h as f32, on_target, &mut out); }
        crate::raster::render(&out, &mut c.assets, w, h, None)
    }
    /// Recorded numbers (`cargo test -p fn-ui old_ret_captures -- --nocapture`): the reticle and every popup capture, composited over the old world, differ from the old
    /// captures by well under 1/255 mean on the covered region; the whole-HUD opaque-pixel metric is bounded by the (approximate, fontdue) text and 1.10-vs-1.8-free layers.
    #[test] fn old_ret_captures_match_numerically() {
        let Some(mut c) = ctx() else { return };
        if !crate::screens::testutil::old_root().join("verification").is_dir() { eprintln!("old tree not found, skipping"); return; }
        let mut worst_overlay = 0f64; let mut rows = Vec::new();
        for (w, h) in [(1920u32, 1080u32), (1280, 720)] {
            let Some(base) = old_bmp("toggle-off", w, h) else { continue };
            let hidden = old_bmp("harvest-hidden", w, h).unwrap();
            for (name, on) in [("white", false), ("yellow", true)] {
                let want = old_bmp(name, w, h).unwrap(); let l = layer(&mut c, w, h, on, None);
                // (the old `ret-white-1280x720` capture was taken before the 3D world drew its first frame: its world is black, so the reticle is composited over black there)
                let lum = |im: &image::RgbImage| -> u32 { (0..20).flat_map(|i| (0..20).map(move |j| (i, j))).map(|(i, j)| im.get_pixel(w / 5 + i, h * 7 / 10 + j).0.iter().map(|v| *v as u32).sum::<u32>()).sum::<u32>() / 400 };
                let black = image::RgbImage::new(w, h); let world_missing = lum(&want) < 8 && lum(&base) > 60;
                let (m, mx, n) = overlay_diff(&l, if world_missing { &black } else { &base }, &want); worst_overlay = worst_overlay.max(m); rows.push(format!("overlay ret-{name:<34} {w}x{h} mean {m:.3} max {mx:>3} px {n}"));
            }
            for st in HARVEST_SCRIPT.iter() {
                let want = old_bmp(st.name, w, h).unwrap(); let l = layer(&mut c, w, h, false, Some(st.name));
                // popup over the old hidden capture of the same size (the packs differ only outside the popup region): crop to the popup area (left of centre, y > 380)
                let (m, mx, n) = overlay_diff(&l, &hidden, &want); rows.push(format!("overlay ret-{:<34} {w}x{h} mean {m:.3} max {mx:>3} px {n}", st.name));
                if st.name != "harvest-hidden" { worst_overlay = worst_overlay.max(m); }
            }
        }
        for r in &rows { eprintln!("{r}"); }
        assert!(worst_overlay < 2., "worst composite mean difference {worst_overlay} (recorded: 1.2 for the black-world ret-white-1280x720, <= 0.06 for every other capture)");
    }
    #[test] fn old_ret_hud_scenes_opaque_pixels_match() {
        let Some(mut c) = ctx() else { return };
        if !crate::screens::testutil::old_root().join("verification").is_dir() { return; }
        let mut worst = 0f64;
        for (w, h) in [(1920u32, 1080u32), (1280, 720)] {
            for s in all() {
                let Some(t) = s.old_capture else { continue };
                let stem = t.replace("-{w}x{h}", "").replacen("ret-", "", 1);
                let Some(old) = old_bmp(&stem, w, h) else { continue };
                let l = (s.build)(&mut c, w as f32, h as f32); let px = crate::raster::render(&l, &mut c.assets, w, h, None);
                let (m, bad, n) = masked_diff(&px, &old); eprintln!("opaque {:<36} {w}x{h} mean {m:.3} bad>8 {:.1}% px {n}", s.name, bad * 100.); worst = worst.max(m);
            }
        }
        assert!(worst < 0.5, "worst opaque-pixel mean difference {worst} (recorded 0.065 over 28 captures)");
    }
    /// End-to-end through the CPU rasteriser (materials baked by `material::hud::bake`); `FN_UI_DUMP=<dir>` also writes PNGs (1920x1080 and 1280x720, over a green backdrop).
    #[test] fn world_scenes_rasterise_through_the_cpu_backend() {
        let Some(mut c) = ctx() else { return };
        for (w, h) in [(1280u32, 720u32), (1920, 1080)] {
            for s in all() {
                if w == 1920 && std::env::var_os("FN_UI_DUMP").is_none() { continue; }
                let l = (s.build)(&mut c, w as f32, h as f32);
                let px = crate::raster::render(&l, &mut c.assets, w, h, Some([0.2, 0.3, 0.2, 1.]));
                let changed = px.chunks_exact(4).filter(|p| p[..3] != [51, 77, 51]).count();
                assert!(changed > 2000, "{}: only {changed} px painted", s.name);
                if let Some(dir) = std::env::var_os("FN_UI_DUMP") {
                    let dir = std::path::PathBuf::from(dir); let _ = std::fs::create_dir_all(&dir);
                    image::save_buffer(dir.join(format!("{}-{w}x{h}.png", s.name)), &px, w, h, image::ColorType::Rgba8).unwrap();
                }
            }
        }
    }
    fn quads(l: &RenderList) -> Vec<crate::draw::Quad> { l.cmds.iter().filter_map(|c| if let Cmd::Quad(q) = c { Some(*q) } else { None }).collect() }
    #[test] fn reticle_scenes_differ_only_by_the_reticle_quad() {
        let Some(mut c) = ctx() else { return };
        let white = ret_white(&mut c, 1920., 1080.); let yellow = ret_yellow(&mut c, 1920., 1080.); let off = ret_toggle_off(&mut c, 1920., 1080.);
        assert_eq!(white.cmds.len(), yellow.cmds.len()); assert_eq!(white.cmds.len(), off.cmds.len() + 1);
        let diff: Vec<usize> = (0..white.cmds.len()).filter(|i| white.cmds[*i] != yellow.cmds[*i]).collect(); assert_eq!(diff.len(), 1, "only the reticle changes colour");
        let Cmd::Quad(y) = &yellow.cmds[diff[0]] else { panic!() }; assert_eq!(y.tint, crate::screens::reticle::reticle_tint(true));
        assert!(off.cmds.iter().all(|cmd| !matches!(cmd, Cmd::Quad(q) if q.bounds() == crate::screens::reticle::reticle_rect(1920., 1080.))), "the toggle removes the reticle");
    }
    #[test] fn popup_scenes_follow_the_old_state_log() {
        let Some(mut c) = ctx() else { return };
        let n = |l: &RenderList| quads(l).len();
        // the popup scenes keep the (white) reticle and the packs of their step: the hidden step equals a plain HUD with 60 wood and no popup
        let mut plain = base_hud(); plain.state.resources = [60, 0, 0]; let plain = draw(&mut c, 1920., 1080., &mut plain);
        assert_eq!(hv_hidden(&mut c, 1920., 1080.).cmds, plain.cmds, "hidden: no popup glyphs");
        let hit1 = hv_hit1_025(&mut c, 1920., 1080.); assert!(n(&hit1) > n(&plain) + 8, "icon + '+20' glyph quads (aggregate x2, potential x2)");
        assert!(n(&hv_outro_030(&mut c, 1920., 1080.)) > n(&plain), "outro in progress at t0.30");
        let m = |l: &RenderList| l.cmds.iter().filter(|x| matches!(x, Cmd::Material(MaterialCmd { kind: MaterialKind::PanelVGradHighlighted, .. }))).count();
        assert_eq!(m(&hit1), 1, "the current (wood) pack is highlighted");
        // weak point: cyan potential text
        let w = crate::screens::reticle::weakpoint_color(); assert!(quads(&hv_weak_010(&mut c, 1920., 1080.)).iter().any(|q| (q.tint[0] - w[0]).abs() < 1e-5 && (q.tint[2] - w[2]).abs() < 1e-5));
        assert!(!quads(&hv_hit1_025(&mut c, 1920., 1080.)).iter().any(|q| (q.tint[0] - w[0]).abs() < 1e-5 && (q.tint[1] - w[1]).abs() < 1e-5 && q.tint[3] > 0.5), "no cyan on a normal hit");
    }
    #[test] fn packs_show_cap_dimming_highlight_and_count_up() {
        let Some(mut c) = ctx() else { return };
        let cap = pack_cap(&mut c, 1920., 1080.);
        assert_eq!(cap.cmds.iter().filter(|x| matches!(x, Cmd::Material(m) if m.kind == MaterialKind::PanelVGradHighlighted)).count(), 1);
        // the 999 text is dimmed to 0.6: some glyph quads carry alpha 0.6, and the unlimited 45 / 3 ones carry 1
        let dim = quads(&cap).iter().filter(|q| (q.tint[3] - 0.6).abs() < 1e-6 && q.tint[0] == 1.).count(); assert!(dim >= 3, "three digits of '999' at alpha 0.6: {dim}");
        // count-up: halfway through 0.5 s the wood pack shows 50 (linear, 0 -> 100)
        let mut hud = base_hud(); hud.state.resources = [0; 3]; hud.tick(1. / 60.); hud.state.resources = [100, 0, 0]; hud.tick(0.25);
        assert_eq!(hud.extra.count.shown(0, hud.state.resources), 50); hud.tick(0.3); assert_eq!(hud.extra.count.shown(0, hud.state.resources), 100);
    }
    #[test] fn target_info_scene_shows_the_bar_after_damage_only() {
        let Some(mut c) = ctx() else { return };
        let bars = |l: &RenderList| l.cmds.iter().filter(|x| matches!(x, Cmd::Material(m) if matches!(m.kind, MaterialKind::HudBar { .. }))).count();
        let base = bars(&ret_white(&mut c, 1920., 1080.));
        assert_eq!(bars(&target_info_damaged(&mut c, 1920., 1080.)), base + 2, "delta + fill after the hit");
        assert_eq!(bars(&target_info_full(&mut c, 1920., 1080.)), base + 2);
        // toggled off: the HUD row hides it
        let mut hud = base_hud(); hud.state.world.target = Some(target(600., HealthDisplayRule::Always)); hud.state.visible.target_info = false;
        assert_eq!(bars(&draw(&mut c, 1920., 1080., &mut hud)), base);
    }
    #[test] fn interaction_and_equip_scenes_respect_their_toggles() {
        let Some(mut c) = ctx() else { return };
        let base = ret_white(&mut c, 1920., 1080.).cmds.len();
        assert!(interaction_prompt(&mut c, 1920., 1080.).cmds.len() > base);
        let mut hud = base_hud(); hud.state.world.interaction = Some(InteractionPrompt { key: "E".into(), text: "Open Chest".into(), screen: None }); hud.state.visible.interaction_info = false;
        assert_eq!(draw(&mut c, 1920., 1080., &mut hud).cmds.len(), base, "Interaction Info off hides the prompt");
        assert!(equip_progress_25(&mut c, 1920., 1080.).cmds.len() > base);
    }
    #[test] fn the_reticle_follows_the_equipped_item_and_the_settings_toggle() {
        let Some(mut c) = ctx() else { return };
        let ret = crate::screens::reticle::reticle_rect(1920., 1080.);
        let has = |l: &RenderList| l.cmds.iter().any(|x| matches!(x, Cmd::Quad(q) if q.bounds() == ret));
        let mut hud = base_hud(); assert!(has(&draw(&mut c, 1920., 1080., &mut hud)), "pickaxe equipped: reticle");
        // another item selected (no weapon data for it): no pickaxe reticle
        hud.state.items[1] = Some(crate::screens::hud::HudItem::new(2, "", Some(3))); hud.state.selected = 1; assert!(!has(&draw(&mut c, 1920., 1080., &mut hud)));
        hud.state.selected = 0; assert!(has(&draw(&mut c, 1920., 1080., &mut hud)));
        // the Settings row HUD.Center.Reticle -> HudVisibility::reticle (from_tags is what fn-hud-adapter feeds from the [HUD] extras)
        hud.state.visible = crate::screens::hud::HudVisibility::from_tags(|t| t != "HUD.Center.Reticle"); assert!(!has(&draw(&mut c, 1920., 1080., &mut hud)));
        // every row now drives its piece: Settings > HUD tags map one to one onto the visibility fields the draw code reads
        for (i, tag) in crate::screens::hud::HudVisibility::TAGS.iter().enumerate() {
            let v = crate::screens::hud::HudVisibility::from_tags(|t| t != *tag);
            let flags = [v.reticle, v.player_health, v.pickup_info, v.minimap, v.quick_bar, v.target_info, v.interaction_info];
            assert_eq!(flags.iter().filter(|f| !**f).count(), 1); assert!(!flags[i], "{tag}");
        }
    }
    #[test] fn a_hit_in_the_state_is_consumed_once() {
        let Some(mut c) = ctx() else { return };
        let mut hud = Hud::new(); let mut s = crate::screens::hud::HudState::default();
        s.world.harvest_hits.push(HarvestHit { object: 5, resource: HarvestResource::Wood, amount: 20, destroyed: false, weakpoint: false });
        hud.set_state(&s); assert_eq!(hud.reticle.popup.aggregate, 20); assert!(hud.state.world.harvest_hits.is_empty());
        hud.set_state(&crate::screens::hud::HudState::default()); hud.tick(0.1); assert_eq!(hud.reticle.popup.aggregate, 20, "no replay");
        let mut o = RenderList::new(); hud.present(&mut c, 1920., 1080., &mut o); hud.present(&mut c, 1920., 1080., &mut o); assert_eq!(hud.reticle.popup.aggregate, 20);
    }
}
