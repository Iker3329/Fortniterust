//! Deterministic capture scenes of the HUD and the loading screen, named after the old runtime's BMP captures.
//!
//! Provenance of the inputs (old `main.rs` `--vitals-test`, `--quickbar-test`, `--hud-extra-test`, `loading::Loader::gallery/finish`):
//! * `vit-*`   health/shield bars: (name, from, to, 60 Hz ticks after the change) = full (100,0)->(100,0) 0 | shield-full (100,100) 0 | mid (63,27) 0 | low (12,0) 0 |
//!             empty (0,0) 0 | one-hp (1,1) 0 | damage-early (100,100)->(40,20) 6 | damage-late (100,100)->(40,20) 30 | heal (30,0)->(80,60) 3.
//!             The whole HUD is drawn (old captures show quickbar slot 1, compass, minimap, counters and packs at their defaults, yaw 0 = spawn yaw, over the 3D world).
//!             Only the 1.10 captures of these exist (`verification-1.10/vit-*`), taken with 1.10 data/layout: expect layout differences vs the 1.8 constants.
//! * `qb-*`    quickbar: default (slot 1 only) | full-sel0 | full-sel3 | full-sel5 (all six filled, rarities [0,1,2,3,4,0], counts [-,3,120,7,24,-]) |
//!             anim-half (selection animation moved 0.05 s from slot 1 toward slot 3, i.e. half way). No old BMPs exist in the tree (`old_capture` None).
//! * `hudx0..3` hud_extra test variants 0..3 (`hud_extra::test_variant`) on the default quickbar/vitals. No old BMPs exist in the tree.
//! * `loading*` loading screen: `loading` = the smoke gallery (progress by size, longest tip, label "Loading animation: M_Jog_Melee_Twohanded_NE (n/100)"),
//!             `loading-complete` = 100% "Loading complete", `loading-fade` = the first frame with fade >= 0.5, `loading-live` = the half-way live frame (label/step total
//!             ASSUMED: the real label depends on the clip list of that run). Tip index 0 for the automated captures.
use crate::draw::RenderList;
use crate::scene::{SceneDef, UiContext};
use crate::screens::hud::Hud;
use crate::screens::hud_extra::test_variant;
use crate::screens::loading::{longest_tip, LoadingScreen, Progress};

/// Fixed animation step of the capture scenes (the old tests ticked at 60 Hz).
pub const DT: f32 = 1. / 60.;

/// Vitals capture: snap to `from`, set the target `to`, advance `ticks` 60 Hz steps (0 = snap to `to`), draw the full HUD.
pub fn vitals_scene(ctx: &mut UiContext, w: f32, h: f32, from: (u16, u16), to: (u16, u16), ticks: u32) -> RenderList {
    let mut hud = Hud::new();
    hud.vitals.snap(from.0, from.1);
    hud.state.health = to.0; hud.state.shield = to.1;
    for _ in 0..ticks { hud.vitals.tick(DT, to.0, to.1); }
    if ticks == 0 { hud.vitals.snap(to.0, to.1); }
    let mut out = RenderList::new(); hud.build(ctx, w, h, 0., &mut out); out
}

const RARITIES: [usize; 6] = [0, 1, 2, 3, 4, 0];
const COUNTS: [Option<u32>; 6] = [None, Some(3), Some(120), Some(7), Some(24), None];

/// Quickbar capture: `from` = Some(slot) starts the selection animation on that slot and advances 0.05 s toward `selected`.
pub fn quickbar_scene(ctx: &mut UiContext, w: f32, h: f32, selected: usize, occupied: [bool; 6], rarity: [usize; 6], count: [Option<u32>; 6], from: Option<usize>) -> RenderList {
    let mut hud = Hud::new();
    for i in 0..6 { hud.state.items[i] = occupied[i].then(|| crate::screens::hud::HudItem::new(0, "", None)); } hud.state.selected = selected; hud.qb.rarity = rarity; hud.qb.count = count;
    match from { Some(f) => { hud.qb.snap(f); hud.qb.tick(selected, 0.05); } None => hud.qb.snap(selected) }
    let mut out = RenderList::new(); hud.build(ctx, w, h, 0., &mut out); out
}

pub fn hudx_scene(ctx: &mut UiContext, w: f32, h: f32, variant: u8) -> RenderList {
    let mut hud = Hud::new();
    hud.extra.state = test_variant(variant);
    let mut out = RenderList::new(); hud.build(ctx, w, h, 0., &mut out); out
}

/// Smoke-gallery progress (percent done of 100) per client size; unknown sizes use 50%.
pub fn gallery_progress(w: f32, h: f32) -> usize {
    match (w as i32, h as i32) { (1920, 1080) => 0, (1280, 720) => 50, (2560, 1080) => 75, (1024, 768) => 50, (640, 360) => 90, (600, 900) | (640, 900) => 25, (2400, 1350) | (2400, 1080) => 100, _ => 50 }
}

fn loading_frame(ctx: &mut UiContext, w: f32, h: f32, progress: Progress, label: &str, tip: Option<usize>, fade: f32) -> RenderList {
    let mut s = LoadingScreen::new();
    let mut out = RenderList::new();
    // `build` loads the tips on first use; pick the longest tip for the gallery by loading them first.
    if tip.is_none() { s = LoadingScreen::with_tips(LoadingScreen::load_tips(ctx)); }
    let idx = tip.unwrap_or_else(|| longest_tip(&s.tips));
    s.build(ctx, w, h, progress, label, idx, fade, &mut out); out
}

macro_rules! vit { ($f:ident, $from:expr, $to:expr, $ticks:expr) => { fn $f(c: &mut UiContext, w: f32, h: f32) -> RenderList { vitals_scene(c, w, h, $from, $to, $ticks) } }; }
vit!(vit_full, (100, 0), (100, 0), 0);
vit!(vit_shield_full, (100, 100), (100, 100), 0);
vit!(vit_mid, (63, 27), (63, 27), 0);
vit!(vit_low, (12, 0), (12, 0), 0);
vit!(vit_empty, (0, 0), (0, 0), 0);
vit!(vit_one_hp, (1, 1), (1, 1), 0);
vit!(vit_damage_early, (100, 100), (40, 20), 6);
vit!(vit_damage_late, (100, 100), (40, 20), 30);
vit!(vit_heal, (30, 0), (80, 60), 3);

fn qb_default(c: &mut UiContext, w: f32, h: f32) -> RenderList { quickbar_scene(c, w, h, 0, [true, false, false, false, false, false], [0; 6], [None; 6], None) }
fn qb_full_sel0(c: &mut UiContext, w: f32, h: f32) -> RenderList { quickbar_scene(c, w, h, 0, [true; 6], RARITIES, COUNTS, None) }
fn qb_full_sel3(c: &mut UiContext, w: f32, h: f32) -> RenderList { quickbar_scene(c, w, h, 3, [true; 6], RARITIES, COUNTS, None) }
fn qb_full_sel5(c: &mut UiContext, w: f32, h: f32) -> RenderList { quickbar_scene(c, w, h, 5, [true; 6], RARITIES, COUNTS, None) }
fn qb_anim_half(c: &mut UiContext, w: f32, h: f32) -> RenderList { quickbar_scene(c, w, h, 2, [true; 6], RARITIES, COUNTS, Some(0)) }

fn hudx0(c: &mut UiContext, w: f32, h: f32) -> RenderList { hudx_scene(c, w, h, 0) }
fn hudx1(c: &mut UiContext, w: f32, h: f32) -> RenderList { hudx_scene(c, w, h, 1) }
fn hudx2(c: &mut UiContext, w: f32, h: f32) -> RenderList { hudx_scene(c, w, h, 2) }
fn hudx3(c: &mut UiContext, w: f32, h: f32) -> RenderList { hudx_scene(c, w, h, 3) }

/// The reference gallery's level-travel capture (`loading-travel-WxH`): label "LOADING", longest tip, progress by size.
fn loading(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let done = gallery_progress(w, h);
    loading_frame(c, w, h, Progress { total: 100, done }, "LOADING", None, 0.)
}
fn loading_complete(c: &mut UiContext, w: f32, h: f32) -> RenderList { loading_frame(c, w, h, Progress { total: 20, done: 20 }, "Loading complete", Some(0), 0.) }
fn loading_fade(c: &mut UiContext, w: f32, h: f32) -> RenderList { loading_frame(c, w, h, Progress { total: 20, done: 20 }, "Loading complete", Some(0), 0.5) }
fn loading_live(c: &mut UiContext, w: f32, h: f32) -> RenderList { loading_frame(c, w, h, Progress { total: 20, done: 10 }, "Loading HUD", Some(0), 0.) }

/// The reference's START-UP screen (`loading-WxH`, `loading-complete`, `loading-fade`, `loading-live`): key art, plate, LOADING, progress line.
fn startup_frame(c: &mut UiContext, w: f32, h: f32, progress: f32, fade: f32) -> RenderList {
    let mut out = RenderList::new();
    crate::screens::startup_loading::StartupLoadingScreen.build(c, w, h, 0., progress, true, fade, &mut out); out
}
fn startup_loading(c: &mut UiContext, w: f32, h: f32) -> RenderList { startup_frame(c, w, h, gallery_progress(w, h) as f32 / 100., 0.) }
fn startup_complete(c: &mut UiContext, w: f32, h: f32) -> RenderList { startup_frame(c, w, h, 1., 0.) }
fn startup_fade(c: &mut UiContext, w: f32, h: f32) -> RenderList { startup_frame(c, w, h, 1., 0.5) }
fn startup_live(c: &mut UiContext, w: f32, h: f32) -> RenderList { startup_frame(c, w, h, 0.5, 0.) }
fn picker_main(c: &mut UiContext, w: f32, h: f32) -> RenderList {
    let mut out = RenderList::new(); crate::screens::picker::Picker::new().build(c, w, h, true, &mut out); out
}

pub fn all() -> Vec<SceneDef> {
    let hud = |name: &'static str, old: Option<&'static str>, build: fn(&mut UiContext, f32, f32) -> RenderList| SceneDef { name, old_capture: old, over_world: true, build };
    let full = |name: &'static str, old: Option<&'static str>, build: fn(&mut UiContext, f32, f32) -> RenderList| SceneDef { name, old_capture: old, over_world: false, build };
    vec![
        hud("vit-full", Some("vit-full-{w}x{h}"), vit_full),
        hud("vit-shield-full", Some("vit-shield-full-{w}x{h}"), vit_shield_full),
        hud("vit-mid", Some("vit-mid-{w}x{h}"), vit_mid),
        hud("vit-low", Some("vit-low-{w}x{h}"), vit_low),
        hud("vit-empty", Some("vit-empty-{w}x{h}"), vit_empty),
        hud("vit-one-hp", Some("vit-one-hp-{w}x{h}"), vit_one_hp),
        hud("vit-damage-early", Some("vit-damage-early-{w}x{h}"), vit_damage_early),
        hud("vit-damage-late", Some("vit-damage-late-{w}x{h}"), vit_damage_late),
        hud("vit-heal", Some("vit-heal-{w}x{h}"), vit_heal),
        hud("qb-default", None, qb_default),
        hud("qb-full-sel0", None, qb_full_sel0),
        hud("qb-full-sel3", None, qb_full_sel3),
        hud("qb-full-sel5", None, qb_full_sel5),
        hud("qb-anim-half", None, qb_anim_half),
        hud("hudx0", None, hudx0),
        hud("hudx1", None, hudx1),
        hud("hudx2", None, hudx2),
        hud("hudx3", None, hudx3),
        full("loading", Some("loading-travel-{w}x{h}"), loading),
        full("loading-complete", None, loading_complete),
        full("loading-fade", None, loading_fade),
        full("loading-live", None, loading_live),
        full("startup-loading", Some("loading-{w}x{h}"), startup_loading),
        full("startup-complete", Some("loading-complete"), startup_complete),
        full("startup-fade", Some("loading-fade"), startup_fade),
        full("startup-live", Some("loading-live"), startup_live),
        full("picker", None, picker_main),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Cmd;
    use crate::screens::testutil::ctx;
    #[test] fn scene_names_are_unique_and_registered() {
        let v = all(); let mut n: Vec<_> = v.iter().map(|s| s.name).collect(); n.sort(); n.dedup(); assert_eq!(n.len(), v.len());
        let reg = crate::scene::all_scenes();
        for s in &v { assert!(reg.iter().any(|r| r.name == s.name), "{} not wired into all_scenes", s.name); }
        assert!(v.iter().filter(|s| s.name.starts_with("loading")).all(|s| !s.over_world));
        assert!(v.iter().filter(|s| !s.name.starts_with("loading") && !s.name.starts_with("startup-") && !s.name.starts_with("picker")).all(|s| s.over_world));
    }
    #[test] fn every_scene_builds_nonempty_finite_lists_at_several_sizes() {
        let Some(mut c) = ctx() else { return };
        for s in all() {
            for (w, h) in [(1920., 1080.), (1280., 720.), (1024., 768.), (640., 360.), (800., 1280.)] {
                let l = (s.build)(&mut c, w, h);
                assert!(!l.is_empty(), "{} {w}x{h}", s.name);
                for cmd in &l.cmds { match cmd {
                    Cmd::Quad(q) => assert!(q.pos.iter().flatten().chain(q.uv.iter()).chain(q.tint.iter()).all(|v| v.is_finite()), "{} {w}x{h} NaN", s.name),
                    Cmd::Material(m) => assert!([m.rect.x, m.rect.y, m.rect.w, m.rect.h].iter().all(|v| v.is_finite()), "{}", s.name),
                    _ => {}
                } }
                let l2 = (s.build)(&mut c, w, h); assert_eq!(l.cmds, l2.cmds, "{} not deterministic", s.name);
            }
        }
    }
    #[test] fn vitals_scenes_differ_by_state() {
        let Some(mut c) = ctx() else { return };
        let full = vitals_scene(&mut c, 1920., 1080., (100, 0), (100, 0), 0);
        let mid = vitals_scene(&mut c, 1920., 1080., (63, 27), (63, 27), 0);
        let early = vitals_scene(&mut c, 1920., 1080., (100, 100), (40, 20), 6);
        let late = vitals_scene(&mut c, 1920., 1080., (100, 100), (40, 20), 30);
        assert_ne!(full.cmds, mid.cmds); assert_ne!(early.cmds, late.cmds);
        let bar_widths = |l: &RenderList| l.cmds.iter().filter_map(|c| { if let Cmd::Material(m) = c { if matches!(m.kind, crate::draw::MaterialKind::HudBar { .. }) { return Some(m.rect.w); } } None }).collect::<Vec<_>>();
        let (e, l) = (bar_widths(&early), bar_widths(&late));
        assert!(e.iter().cloned().fold(0., f32::max) > l.iter().cloned().fold(0., f32::max) - 1e-3, "delta trail shrinks over time");
    }
    #[test] fn quickbar_scenes_selection_and_counts() {
        let Some(mut c) = ctx() else { return };
        let d = qb_default(&mut c, 1920., 1080.); let f = qb_full_sel0(&mut c, 1920., 1080.);
        let mats = |l: &RenderList| l.cmds.iter().filter(|c| matches!(c, Cmd::Material(m) if matches!(m.kind, crate::draw::MaterialKind::ItemCard { .. }))).count();
        assert_eq!((mats(&d), mats(&f)), (1, 6));
        assert_ne!(qb_full_sel3(&mut c, 1920., 1080.).cmds, qb_full_sel5(&mut c, 1920., 1080.).cmds);
        assert_ne!(qb_anim_half(&mut c, 1920., 1080.).cmds, qb_full_sel3(&mut c, 1920., 1080.).cmds);
    }
    /// End-to-end through the CPU rasteriser (materials baked by `material::hud::bake`): every scene paints pixels. Set `FN_UI_DUMP=<dir>` to also write PNGs.
    #[test] fn scenes_rasterise_through_the_cpu_backend() {
        let Some(mut c) = ctx() else { return };
        let (w, h) = (1280u32, 720u32);
        for s in all() {
            let l = (s.build)(&mut c, w as f32, h as f32);
            let px = crate::raster::render(&l, &mut c.assets, w, h, Some([0.2, 0.3, 0.2, 1.]));
            assert_eq!(px.len(), (w * h * 4) as usize);
            let changed = px.chunks_exact(4).filter(|p| p[..3] != [51, 77, 51]).count();
            assert!(changed > 2000, "{}: only {changed} px painted", s.name);
            if let Some(dir) = fn_live::var_os("FN_UI_DUMP") {
                let dir = std::path::PathBuf::from(dir); let _ = std::fs::create_dir_all(&dir);
                image::save_buffer(dir.join(format!("{}-{w}x{h}.png", s.name)), &px, w, h, image::ColorType::Rgba8).unwrap();
            }
        }
    }
    #[test] fn gallery_progress_follows_old_smoke_table() { assert_eq!(gallery_progress(1920., 1080.), 0); assert_eq!(gallery_progress(640., 360.), 90); assert_eq!(gallery_progress(2400., 1350.), 100); assert_eq!(gallery_progress(333., 333.), 50); }
    #[test] fn old_capture_templates_name_existing_files() {
        // The templates must resolve to real old BMPs for the sizes the old runtime captured (when the old tree is present).
        let root = crate::screens::testutil::old_root();
        if !root.is_dir() { eprintln!("old tree not found, skipping"); return; }
        for (tpl, w, h, dir) in [("vit-full-{w}x{h}", 1920, 1080, "verification-1.10"), ("vit-damage-early-{w}x{h}", 1280, 720, "verification-1.10"), ("loading-{w}x{h}", 1280, 720, "verification"), ("loading-complete", 0, 0, "verification"), ("loading-fade", 0, 0, "verification")] {
            let stem = tpl.replace("{w}", &w.to_string()).replace("{h}", &h.to_string());
            assert!(root.join(dir).join(format!("{stem}.bmp")).is_file(), "{dir}/{stem}.bmp");
        }
    }
}

