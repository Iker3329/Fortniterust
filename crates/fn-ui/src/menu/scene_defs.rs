//! Registered scenes of the menu port (`menu::scenes`), named after the old runtime's captures so the pixel diff helper can find them.
//!
//! * `settings18-*`: the Fortnite 1.8 Settings screen, produced by the old `--settings18-capture` driver (1920x1080 and 1280x720, no input beyond the
//!   script, a no-op apply host over `Settings::default()`); old captures live in `verification/settings18-<scene>-<W>x<H>.bmp` and show the screen on
//!   its own background (the Settings screen is opaque), so `over_world = false`.
//!   `game-hover` = old scene `game-gated` (scrolled Game tab with "Show Backpack" hovered), `input-listen` = `pages::input::test_scene`.
//! * `menu-pause*`: the Esc menu (`--menu-test` scenes `pause*`), captured over the frozen 3D scene (verification-1.10), so `over_world = true`.
//!   `menu-pause-fading` shows the fade at 0.45 visibility (plus the two capture frames the old driver rendered).
//!
//! Each scene builds a fresh [`MenuSystem`] (cloning `ctx.fonts`), plays the old script, runs two headless frames (the old driver rendered twice and
//! captured the second) and lowers the draw list with [`lower::lower`] (Procedural boxes as `Fill` runs, the behaviour of the old captures).
//! Nothing touches the disk. Every scene owns its menu and thus its settings session (no globals).
use super::{lower, pages, scripts, MenuSystem};
use crate::draw::RenderList;
use crate::scene::{SceneDef, UiContext};

/// Frame time of the scripted frames.
const DT: f32 = 1. / 60.;

fn finish(m: &mut MenuSystem, ctx: &mut UiContext, w: f32, h: f32) -> RenderList {
    // The old drivers rendered twice (the first frame is discarded); the second one is what the capture shows.
    m.frame(w, h, DT);
    m.frame(w, h, DT);
    let mut out = RenderList::new();
    lower::lower(m.parts().0, ctx, w, h, &mut out);
    out
}

/// Settings18 scene: `script` is an old scene name (`pages::scenes::SCENES`, `input-listen`, `game-hover`).
pub fn settings18(ctx: &mut UiContext, w: f32, h: f32, script: &str) -> RenderList {
    let mut m = scripts::new_menu(ctx.fonts.clone(), false);
    match script {
        "input-listen" => pages::input::test_scene(&mut m, "input-listen", w, h),
        "game-hover" => pages::scenes::scene(&mut m, "game-gated", w, h),
        s => pages::scenes::scene(&mut m, s, w, h),
    }
    finish(&mut m, ctx, w, h)
}

/// Esc-menu scene: `script` is an old `--menu-test` scene name (`pause`, `pause-hover`, ...).
pub fn menu(ctx: &mut UiContext, w: f32, h: f32, script: &str) -> RenderList {
    let mut m = scripts::new_menu(ctx.fonts.clone(), false);
    scripts::test_scene(&mut m, script, w, h);
    finish(&mut m, ctx, w, h)
}

macro_rules! scene {
    ($name:literal, $old:literal, $world:literal, $f:ident, $script:literal) => {
        SceneDef { name: $name, old_capture: Some($old), over_world: $world, build: { fn b(c: &mut UiContext, w: f32, h: f32) -> RenderList { $f(c, w, h, $script) } b } }
    };
}

/// All scenes of the menu port.
pub fn scenes() -> Vec<SceneDef> {
    vec![
        scene!("settings18-video", "settings18-video-{w}x{h}", true, settings18, "video"),
        scene!("settings18-game", "settings18-game-{w}x{h}", true, settings18, "game"),
        scene!("settings18-hud", "settings18-hud-{w}x{h}", true, settings18, "hud"),
        scene!("settings18-brightness", "settings18-brightness-{w}x{h}", true, settings18, "brightness"),
        scene!("settings18-audio", "settings18-audio-{w}x{h}", true, settings18, "audio"),
        scene!("settings18-input", "settings18-input-{w}x{h}", true, settings18, "input"),
        scene!("settings18-input-listen", "settings18-input-listen-{w}x{h}", true, settings18, "input-listen"),
        scene!("settings18-controller", "settings18-controller-{w}x{h}", true, settings18, "controller"),
        scene!("settings18-livestream", "settings18-livestream-{w}x{h}", true, settings18, "livestream"),
        scene!("settings18-dirty", "settings18-dirty-{w}x{h}", true, settings18, "dirty"),
        scene!("settings18-keep-video", "settings18-keep-video-{w}x{h}", true, settings18, "keep-video"),
        scene!("settings18-game-hover", "settings18-game-hover-{w}x{h}", true, settings18, "game-hover"),
        scene!("settings18-video-hover", "settings18-video-hover-{w}x{h}", true, settings18, "video-hover"),
        scene!("menu-pause", "menu-pause-{w}x{h}", true, menu, "pause"),
        scene!("menu-pause-hover", "menu-pause-hover-{w}x{h}", true, menu, "pause-hover"),
        scene!("menu-pause-focus", "menu-pause-focus-{w}x{h}", true, menu, "pause-focus"),
        scene!("menu-pause-pressed", "menu-pause-pressed-{w}x{h}", true, menu, "pause-pressed"),
        scene!("menu-pause-disabled-hover", "menu-pause-disabled-hover-{w}x{h}", true, menu, "pause-disabled-hover"),
        scene!("menu-pause-toast", "menu-pause-toast-{w}x{h}", true, menu, "pause-toast"),
        scene!("menu-pause-leave-hover", "menu-pause-leave-hover-{w}x{h}", true, menu, "pause-leave-hover"),
        scene!("menu-pause-gamepad", "menu-pause-gamepad-{w}x{h}", true, menu, "pause-gamepad"),
        scene!("menu-pause-keyboard-column", "menu-pause-keyboard-column-{w}x{h}", true, menu, "pause-keyboard-column"),
        scene!("menu-pause-fading", "menu-pause-fading-{w}x{h}", true, menu, "pause-fading"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil;

    /// Old capture BMP of a scene (`verification/` then `verification-1.10/`) as straight RGB, or None.
    fn old_capture(def: &SceneDef, w: u32, h: u32) -> Option<(u32, u32, Vec<u8>)> {
        let stem = def.old_capture?.replace("{w}", &w.to_string()).replace("{h}", &h.to_string());
        for dir in ["verification", "verification-1.10"] {
            if let Some(p) = testutil::find_old(&format!("{dir}/{stem}.bmp")) { let im = image::open(p).ok()?.to_rgb8(); return Some((im.width(), im.height(), im.into_raw())); }
        }
        None
    }
    /// Over-world scenes: only pixels the UI covers (almost) opaquely are comparable. Returns (compared fraction, mean abs RGB difference,
    /// fraction of compared pixels with any channel off by more than `tol`) of a premultiplied render against an old capture.
    fn compare(px: &[u8], old: &[u8], tol: i32, opaque_only: bool) -> (f32, f32, f32) {
        let (mut sum, mut bad, mut cnt, n) = (0u64, 0usize, 0usize, old.len() / 3);
        for i in 0..n {
            if opaque_only && px[i * 4 + 3] < 250 { continue; }
            cnt += 1; let mut worst = 0;
            for k in 0..3 { let d = (px[i * 4 + k] as i32 - old[i * 3 + k] as i32).abs(); sum += d as u64; worst = worst.max(d); }
            if worst > tol { bad += 1; }
        }
        let c = cnt.max(1);
        (cnt as f32 / n as f32, sum as f32 / (c * 3) as f32, bad as f32 / c as f32)
    }
    fn dump(name: &str, w: u32, h: u32, px: &[u8]) {
        let Some(dir) = fn_live::var_os("FN_UI_DUMP_DIR") else { return };
        let rgb: Vec<u8> = px.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let _ = image::RgbImage::from_raw(w, h, rgb).map(|im| im.save(std::path::Path::new(&dir).join(format!("{name}-{w}x{h}.png"))));
    }
    #[test] fn every_scene_builds_and_names_are_unique() {
        let Some(mut ctx) = testutil::ui_context() else { return };
        let defs = scenes();
        let mut names: Vec<&str> = defs.iter().map(|d| d.name).collect(); names.sort(); let n = names.len(); names.dedup(); assert_eq!(names.len(), n);
        assert_eq!(n, 23);
        for d in &defs {
            for (w, h) in [(1920., 1080.), (1280., 720.)] {
                let l = (d.build)(&mut ctx, w, h);
                assert!(l.quad_count() > 20, "{} {w}x{h}: {} quads", d.name, l.quad_count());
                assert!(l.cmds.iter().filter(|c| matches!(c, crate::draw::Cmd::PushClip(_))).count() == l.cmds.iter().filter(|c| matches!(c, crate::draw::Cmd::PopClip)).count(), "{}: clip stack balanced", d.name);
            }
        }
        assert!(ctx.assets.missing.is_empty(), "missing assets: {:?}", ctx.assets.missing);
    }
    #[test] fn scenes_are_deterministic() {
        let Some(mut ctx) = testutil::ui_context() else { return };
        for d in scenes().iter().filter(|d| ["settings18-video-hover", "menu-pause-toast", "settings18-input-listen"].contains(&d.name)) {
            let a = (d.build)(&mut ctx, 1280., 720.); let b = (d.build)(&mut ctx, 1280., 720.);
            assert_eq!(a.cmds, b.cmds, "{}", d.name);
        }
    }
    /// Pixel diff of every scene against the old BMP captures (where they exist), over the pixels the UI covers (almost) opaquely.
    /// The settings18 scenes were captured from the same code path and must match nearly exactly (the 1280x720 `input` capture still carries the
    /// previous scene's scroll/hover state in the old driver, so it is excluded). The `menu-pause*` captures in verification-1.10 predate the 1.8
    /// MainMenu rebuild: they are only reported.
    /// A settings18 scene over a host pinned to the settings the old captures were taken with: the old runtime's assumed defaults Mouse Sensitivity
    /// (and Y) 0.07 and Mouse ADS / Scope Sensitivity 0.5 (fn-settings `table.rs` at the initial commit cc25a1e). The current defaults are the
    /// recovered 1.8 ones (0.28 / 1.0, see `table.rs`), so the captures stay a fixed reference instead of being regenerated.
    fn settings18_as_captured(ctx: &mut UiContext, w: f32, h: f32, scene: &str) -> RenderList {
        let mut game = pages::NullHost::new();
        (game.s.input.mouse_sensitivity, game.s.input.mouse_sensitivity_y, game.s.input.mouse_ads_multiplier, game.s.input.mouse_scope_multiplier) = (0.07, 0.07, 0.5, 0.5);
        // the old captures predate the recovered gamepad defaults (sensitivity 4/9, ADS / scope 1.0): draw the 0.5 they show
        (game.s.input.controller_sensitivity_x, game.s.input.controller_sensitivity_y, game.s.input.controller_ads_multiplier, game.s.input.controller_scope_multiplier) = (0.5, 0.5, 0.5, 0.5);
        game.s.drain_changes();
        let mut m = scripts::new_menu(ctx.fonts.clone(), false);
        pages::scenes::scene_with(&mut m, if scene == "game-hover" { "game-gated" } else { scene }, w, h, game);
        finish(&mut m, ctx, w, h)
    }
    #[test] fn scenes_resemble_the_old_captures() {
        let Some(mut ctx) = testutil::ui_context() else { return };
        let mut report = Vec::new();
        for d in scenes() {
            for (w, h) in [(1920u32, 1080u32), (1280, 720), (1024, 768), (2560, 1080), (800, 1280), (640, 480)] {
                let Some((ow, oh, old)) = old_capture(&d, w, h) else { continue };
                assert_eq!((ow, oh), (w, h));
                let l = match d.name.strip_prefix("settings18-") { Some(s) if s != "input-listen" => settings18_as_captured(&mut ctx, w as f32, h as f32, s), _ => (d.build)(&mut ctx, w as f32, h as f32) };
                let px = crate::raster::render(&l, &mut ctx.assets, w, h, if d.over_world { None } else { Some([0., 0., 0., 1.]) });
                dump(d.name, w, h, &px);
                let (cov, mean, bad) = compare(&px, &old, 24, d.over_world);
                report.push(format!("{} {w}x{h}: compared {:.0}% of pixels, mean abs {mean:.2}, {:.1}% off by > 24", d.name, cov * 100., bad * 100.));
                // the old settings18-input* captures show the earlier prototype's "Unbound" and an invented key order; 1.8 draws
                // KeyboardKeyNames.KeyNotBound = "Not Bound" in the ConstructKeyBindList order, so those are only reported; the old livestream capture shows the prototype's invented text stack, 1.8 draws the TabAccountLinkage website configuration (header, EpicLogo, AccountLinkText)
                // asserted at 1080 px high only: the old captures were taken with the old ShortestSide DPI curve, which equals the cooked Vertical curve
                // (CookedIniVersion.txt L531-532) only there (720p: 0.666 vs 2/3; 1024x768, 800x1280, 640x480 differ more)
                if h == 1080 && d.name.starts_with("settings18-") && !d.name.starts_with("settings18-input") && d.name != "settings18-livestream" { assert!(bad < 0.02 && mean < 1.0, "{}", report.last().unwrap()); }
            }
        }
        for r in &report { eprintln!("{r}"); }
    }
}
