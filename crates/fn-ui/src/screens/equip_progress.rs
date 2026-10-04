//! Equip progress (`AthenaHealProgress` = `AthenaEquipProgress_C`, tag `HUD.Athena.EquipProgress`): the radial timer shown while an item is being equipped.
//!
//! SOURCES (fn-inspect / old Kismet dump `AthenaEquipProgress.txt`, AthenaHUD slot data):
//! * Placement: `[Overlay] Persistent HUD Content` slot `OverlaySlot_10` (Padding Bottom 160, HAlign Center, VAlign Bottom) holds `Overlay_3` = [AthenaEquippedItemWidget,
//!   AthenaHealProgress]; the progress widget is Center / Bottom in it.
//! * Widget: `Root` ScaleBox (`EStretch::UserSpecified`, scale 0.75, initially Collapsed) -> `OverlayTimer` = [ImageTimerBackground, ImageTimer, TextTimeRemaining]:
//!   backing `Results_TimerBacking` (material instance of `Common1` over the texture `Results_TimerBacking`) and fill `M_HealProgress` (instance of `CircularProgressBar`,
//!   `Texture` = `Results_TimerFill`), both 105 x 105, centred; the text is `CommonNumericTextBlock` NotoSans-Regular 30, centred, one fractional digit.
//! * Behaviour (bytecode): `OnUIGameplayCue(CueName, EventType, Parameters)`: ignore every cue except `GameplayCue.Athena.Equipping` (and BeingRevivedFromDBNO); event
//!   type 1 (WhileActive) -> Root Visible(3), `StartTime = GameTime`, `Duration = Parameters.RawMagnitude`; type 3 (Removed) -> Root Collapsed. `Tick`: if Duration > 0:
//!   `Progress = clamp((GameTime - StartTime) / Duration, 0, 1)`, `M_HealProgress.Percentage = Progress`, `TextTimeRemaining = (1 - Progress) * Duration`.
//!
//! The material is translated from its cooked pixel shader (`material::hud::radial_progress`, WGSL twin in fn-ui-wgpu). APPROXIMATE: `Results_TimerBacking` is drawn as its
//! plain texture (the parent material `Common1` was not disassembled), the text colour is white (the block has no style), ScaleBox sizing (desired = 105 x 0.75 x DPI).
use crate::draw::{MaterialKind, RenderList};
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::reticle_state::EquipProgress;
use crate::screens::{art, atlas_px, centred_baseline, mat_tex, put_text, text_width, Snap, WHITE};
use crate::text::FontId;

pub const IMAGE_SIZE: f32 = 105.; // ImageTimer / ImageTimerBackground brush ImageSize
pub const SCALE: f32 = 0.75; // Root ScaleBox UserSpecifiedScale
pub const FONT_SIZE: f32 = 30.; // TextTimeRemaining Font Size (NotoSans-Regular)
pub const BOTTOM_PAD: f32 = 160.; // OverlaySlot_10 Padding.Bottom
pub const FILL_TEXTURE: &str = "Results_TimerFill.png";
pub const BACKING_TEXTURE: &str = "Results_TimerBacking.png";

/// `Progress` of the widget (the `FClamp(elapsed / Duration, 0, 1)`), None while `Duration` is not positive.
pub fn progress(e: &EquipProgress) -> Option<f32> { if e.duration > 0. { Some((e.elapsed / e.duration).clamp(0., 1.)) } else { None } }
/// `TextTimeRemaining` value: `(1 - Progress) * Duration`.
pub fn remaining(e: &EquipProgress) -> Option<f32> { progress(e).map(|p| (1. - p) * e.duration) }
/// `CommonNumericTextBlock` with `MinimumFractionalDigits = MaximumFractionalDigits = 1`.
pub fn time_text(v: f32) -> String { format!("{:.1}", v.max(0.)) }

/// Pixel square of the timer: `IMAGE_SIZE * SCALE * dpi`, centred horizontally, `BOTTOM_PAD` above the bottom.
pub fn rect(w: f32, h: f32) -> Rect {
    let s = dpi_scale(w, h); let size = IMAGE_SIZE * SCALE * s;
    Rect::new(((w - size) * 0.5).round(), (h - BOTTOM_PAD * s - size).round(), size, size)
}

/// Draw the timer when `e` has a positive duration.
pub fn build(ctx: &mut UiContext, w: f32, h: f32, e: &EquipProgress, out: &mut RenderList) {
    let (Some(p), Some(rem)) = (progress(e), remaining(e)) else { return };
    let r = rect(w, h); let s = dpi_scale(w, h);
    // overlay order: background, timer fill, text
    let back = art(ctx, BACKING_TEXTURE); out.sprite(back.id, r, WHITE);
    let fill = mat_tex(ctx, FILL_TEXTURE, false);
    if fill.id != crate::draw::TextureId::NONE { out.material(MaterialKind::RadialProgress { percentage: p, texture: fill.id }, r, WHITE, false); }
    let px = atlas_px(FONT_SIZE * SCALE, s);
    let txt = time_text(rem);
    let tw = text_width(ctx, FontId::Regular, &txt, px);
    let base = centred_baseline(ctx, FontId::Regular, px, r.y, r.h);
    put_text(ctx, out, FontId::Regular, px, r.x + (r.w - tw) * 0.5, base, &txt, WHITE, None, Snap::PerGlyph);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Cmd;
    use crate::material::hud::{radial_angle, radial_mask, radial_progress, Img};
    use crate::screens::testutil::{ctx, testdata};

    #[test] fn constants_are_the_decoded_widget_numbers() {
        assert_eq!((IMAGE_SIZE, SCALE, FONT_SIZE, BOTTOM_PAD), (105., 0.75, 30., 160.));
        // (the Kismet dump is in the old tree; the numbers above are quoted in the module docs and pinned here against the bytecode literals)
        let root = crate::screens::testutil::old_root().join("reports").join("hud-bytecode").join("AthenaEquipProgress.txt");
        if let Ok(t) = std::fs::read_to_string(&root) {
            assert!(t.contains("prop UserSpecifiedScale = 0.75") && t.contains("\"Size\":30") && t.contains("\"ImageSize\":[105,105]"));
            assert!(t.contains("GameplayCue.Athena.Equipping") && t.contains("SetScalarParameterValue#47(Percentage, self.Progress)"));
        }
        let _ = testdata("hud-1.8-evidence.json"); // the slot data (healProgress Center/Bottom, Overlay_3 padding 160) is quoted from this file's source dump
    }
    #[test] fn progress_follows_the_tick_bytecode() {
        assert_eq!(progress(&EquipProgress { elapsed: 0.5, duration: 2. }), Some(0.25));
        assert_eq!(progress(&EquipProgress { elapsed: 9., duration: 2. }), Some(1.), "FClamp");
        assert_eq!(progress(&EquipProgress { elapsed: 1., duration: 0. }), None, "Duration > 0 gate");
        assert_eq!(remaining(&EquipProgress { elapsed: 0.5, duration: 2. }), Some(1.5));
        assert_eq!(time_text(1.5), "1.5"); assert_eq!(time_text(0.04), "0.0"); assert_eq!(time_text(2.), "2.0");
    }
    #[test] fn timer_is_a_centred_square_160_above_the_bottom() {
        let r = rect(1920., 1080.); assert_eq!((r.w, r.h), (78.75, 78.75)); assert!((r.x + r.w * 0.5 - 960.).abs() <= 0.5); assert!((r.y + r.h - (1080. - 160.)).abs() <= 0.5);
        let r = rect(1280., 720.); assert!((r.w - 105. * 0.75 * 0.666).abs() < 1e-3);
    }
    #[test] fn the_radial_angle_is_atan2_of_the_cooked_expansion() {
        // The cooked shader computes HLSL atan2(first, second) with swapped roles; pin the actual mapping by value (rad, wrap at +-pi).
        let eps = 2e-3;
        let a = |dx: f32, dy: f32| radial_angle(dx, dy);
        assert!((a(0., -0.5) - 0.).abs() < eps, "straight up (v = 0) is angle 0: {}", a(0., -0.5));
        assert!((a(0.5, 0.) - -std::f32::consts::FRAC_PI_2).abs() < eps, "right is -pi/2: {}", a(0.5, 0.));
        assert!((a(-0.5, 0.) - std::f32::consts::FRAC_PI_2).abs() < eps, "left is +pi/2: {}", a(-0.5, 0.));
        assert!((a(0., 0.5).abs() - std::f32::consts::PI).abs() < eps, "straight down is +-pi: {}", a(0., 0.5));
        assert_eq!(a(0., 0.), 0.);
    }
    #[test] fn the_wipe_starts_at_twelve_oclock_and_sweeps_clockwise() {
        // frac(angle/2pi) + P >= 1: for P = 0.25 the covered sector is angle in [-pi/2, 0): up -> right (clockwise on screen, y down)
        let covered = |u: f32, v: f32, p: f32| radial_mask(u, v, p) >= 1.;
        assert!(covered(0.75, 0.25, 0.25), "up-right quadrant covered at 25%");
        assert!(!covered(0.25, 0.25, 0.25) && !covered(0.25, 0.75, 0.25) && !covered(0.75, 0.75, 0.25), "the other quadrants are not");
        assert!(covered(0.75, 0.75, 0.5) && covered(0.75, 0.25, 0.5) && !covered(0.25, 0.75, 0.5) && !covered(0.25, 0.25, 0.5), "50%: right half");
        assert!(covered(0.25, 0.75, 0.75) && !covered(0.25, 0.25, 0.75), "75%: all but the up-left quadrant");
        for (u, v) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] { assert!(radial_mask(u, v, 0.) == 0. && radial_mask(u, v, 1.) == 1., "0% empty, 100% full at ({u},{v})"); }
    }
    #[test] fn radial_progress_bakes_alpha_from_the_mask_and_texture() {
        let tex = Img::solid([1., 1., 1., 1.]);
        let px = radial_progress(8, 8, 0.5, &tex);
        let right = px[2 * 8 + 6]; let left = px[2 * 8 + 1];
        assert_eq!(right[3], 1.); assert!(right[0] > 0.99); assert_eq!(left[3], 0.);
        let faint = Img::solid([1., 1., 1., 0.4]); assert!((radial_progress(8, 8, 1., &faint)[10][3] - 0.4).abs() < 1e-6, "alpha = mask * texture alpha");
        let enc = radial_progress(4, 4, 1., &Img::solid([0.2, 0.2, 0.2, 1.]))[5];
        assert!((enc[0] - (1.055 * 0.2f32.powf(0.416667) - 0.055)).abs() < 1e-4, "exact sRGB encode of the linear texture value");
    }
    #[test] fn build_emits_backing_wipe_and_one_decimal_text() {
        let Some(mut c) = ctx() else { return };
        // the textures come from the paks (not in the old exported folder): skip the drawing assertions when the source has none
        if c.assets.texture(FILL_TEXTURE).id == crate::draw::TextureId::NONE { eprintln!("Results_TimerFill not available from this asset source, skipping"); return; }
        let mut out = RenderList::new(); build(&mut c, 1920., 1080., &EquipProgress { elapsed: 0.5, duration: 2. }, &mut out);
        let m: Vec<_> = out.cmds.iter().filter_map(|c| if let Cmd::Material(m) = c { Some(m) } else { None }).collect();
        assert_eq!(m.len(), 1); assert!(matches!(m[0].kind, MaterialKind::RadialProgress { percentage, .. } if (percentage - 0.25).abs() < 1e-6)); assert_eq!(m[0].rect, rect(1920., 1080.));
        let mut none = RenderList::new(); build(&mut c, 1920., 1080., &EquipProgress { elapsed: 0.5, duration: 0. }, &mut none); assert!(none.is_empty());
    }
}
