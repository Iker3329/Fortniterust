//! The START-UP loading screen of the 1.8 client (the one with the yellow "EARLY ACCESS" plate top right): `SInitialFortniteLoadingScreen`.
//!
//! It is Slate C++ in the 1.8 exe, NOT a UMG widget: there is no widget blueprint for it in the paks (a pak search for `*LoadingScreen*` / `*Loading*.uasset` finds only the
//! loading-tip tables and the unrelated chunk-download dialog). Everything on it is therefore rebuilt from (a) the recovered Slate construction code, read statically
//! from the exe (`crate::screens::startup_layout`, evidence in `crates/fn-exe-slate/README.md`) and (b) the textures / font the exe loads, read from the paks:
//!
//! * `T-Loading-EarlyAccessPlate` (584x160, `/Game/UI/Foundation/Textures/LoadingScreens/`), drawn at its native size, slot Right/Top, padding (0,40,60,0);
//! * "EARLY ACCESS" (`NSLOCTEXT("FortniteLoadingScreen","EarlyAccess",..)`), `Burbank` typeface `Burbank-PlayerPower` (= `BurbankBigRegular-Bold`) 48 units, black, centred in the
//!   plate inset by (0,7,7,0);
//! * "LOADING" (same font, white), bottom left, with the `SCircularThrobber` (radius 20, 6 pieces, 0.75 s, piece brush `Throbber.CircleChunk` = Engine `Slate/Common/Throbber_Piece`)
//!   16 units to its right;
//! * the `SProgressBar` (6 units tall, x 16..W-16, 16 units above the bottom) with the `/Game/UI/CodeReferenced/ProgressBar_Background` / `_Fill` Box brushes (12x12, margin 5/12);
//!   its value is the REAL fraction `clamp(Progress * 0.98, 0, 1)` and 1.0 while the fade curve plays; no marquee;
//! * fade: curve of 0.2 s, alpha `1 - lerp`; plate, EARLY ACCESS and the LOADING subtree fade together (`bFlag == 0`), the key art is a separate layer (`bFlag == 1`).
//!
//! DEVIATION FROM THE EXE (rule of this port: Battle Royale / Athena only): the exe loads `FORT_Load_Atlas` as the full-window art, which is Save-the-World key art (the four
//! STW heroes and the husks, checked by decoding it). This runtime draws the Athena key art `T_BattleRoyaleKeyArt` (`/Game/UI/Login/SubGameSelect/Textures/`, 1920x1080) in the very
//! same slot (Fill/Fill, stretched, no padding). `FORT_Load_BattleRoyale` (also in the paks) is a masked mission-card layer, not a full-window picture.
//!
//! INFERRED / ASSUMED (not in the exe): the throbber piece layout (see [`throbber_pieces`]); the exact Slate line metrics (fontdue instead of FreeType); the 1 unit
//! `BorderPadding` of the bar fill. The old prototype's invented parts (cover-scaled art, 292-wide plate, 36 px Condensed-Black text, 240x4 bar, text shadow) are gone.
use crate::draw::RenderList;
use crate::layout::Rect;
use crate::scene::UiContext;
use crate::screens::startup_layout::{self as sl, StartupLayout};
use crate::screens::{art, put_text, Snap};
use crate::text::FontId;

/// Textures this screen draws (decoded before it appears): Athena key art, the plate, the progress-bar brushes, the throbber piece.
pub const PAK_TEXTURES: [&str; 5] = [ART_FILE, BADGE_FILE, BAR_BACKGROUND_FILE, BAR_FILL_FILE, THROBBER_PIECE_FILE];
/// Athena / Battle Royale key art in the slot where the exe draws `FORT_Load_Atlas` (Save the World art: never decoded by this BR runtime).
pub const ART_FILE: &str = "T_BattleRoyaleKeyArt.png";
pub const BADGE_FILE: &str = sl::BADGE_FILE;
/// `/Game/UI/CodeReferenced/ProgressBar_Background` and `_Fill` (rva 0x124b08c).
pub const BAR_BACKGROUND_FILE: &str = "ProgressBar_Background.png";
pub const BAR_FILL_FILE: &str = "ProgressBar_Fill.png";
/// `Throbber.CircleChunk` of the engine core style = `Engine/Content/Slate/Common/Throbber_Piece.png` (INFERRED mapping of the style name to the file).
pub const THROBBER_PIECE_FILE: &str = "Throbber_Piece.png";
pub const BADGE_TEXT: &str = sl::BADGE_TEXT;
pub const LOADING_TEXT: &str = sl::LOADING_TEXT;
/// `Burbank` / `Burbank-PlayerPower` (= BurbankBigRegular-Bold).
pub const FONT: FontId = FontId::BurbankBold;
pub const BADGE_COLOR: [f32; 4] = sl::BADGE_TEXT_COLOR;
pub const LOADING_COLOR: [f32; 4] = sl::LOADING_COLOR;

/// One throbber piece: centre (window px), rotation (radians, clockwise) and side of the square drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrobberPiece { pub centre: [f32; 2], pub angle: f32 }

/// Piece layout of `SCircularThrobber` (INFERRED from the engine's `OnPaint`, which is not in the exe analysis: `NumPieces` pieces `2*PI/NumPieces` apart, each drawn at the
/// top of the widget (its outer edge on the circle of `radius`) and rotated about the widget centre by `i * 2*PI/NumPieces + phase * 2*PI`; one revolution per `Period`).
/// `piece_h` = the piece texture height in px.
pub fn throbber_pieces(center: [f32; 2], radius: f32, piece_h: f32, t: f32) -> Vec<ThrobberPiece> {
    let delta = std::f32::consts::TAU / sl::THROBBER_PIECES as f32;
    let phase = sl::throbber_phase(t) * std::f32::consts::TAU;
    (0..sl::THROBBER_PIECES).map(|i| {
        let angle = delta * i as f32 + phase;
        let r = radius - piece_h * 0.5;
        ThrobberPiece { centre: [center[0] + r * angle.sin(), center[1] - r * angle.cos()], angle }
    }).collect()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StartupLoadingScreen;
impl StartupLoadingScreen {
    /// Appends one opaque frame. `progress` = the raw `Progress` (0..1: the fraction of REAL start-up work done; the bar shows `progress * 0.98`, 1.0 while fading),
    /// `art_ready` = the pak textures were delivered, `fade` = black overlay (0..1) while the screen leaves (the flow's own fade), `t` = seconds since the screen appeared
    /// (drives the throbber).
    #[allow(clippy::too_many_arguments)]
    pub fn build(&self, ctx: &mut UiContext, w: f32, h: f32, t: f32, progress: f32, art_ready: bool, fade: f32, out: &mut RenderList) {
        self.build_layers(ctx, w, h, t, progress, art_ready, 1., 1., out);
        if fade > 0. { out.fill(Rect::new(0., 0., w, h), [0., 0., 0., fade.min(1.)]); }
    }

    /// The exe's two fade layers: `fg_alpha` = plate + EARLY ACCESS + LOADING subtree (`bFlag == 0`, curve 0.2 s), `atlas_alpha` = the key art (`bFlag == 1`). While
    /// `fg_alpha < 1` the fade curve is playing and the bar shows 1.0. Nothing is drawn behind the screen when `atlas_alpha < 1` (the caller composites it over its scene).
    #[allow(clippy::too_many_arguments)]
    pub fn build_layers(&self, ctx: &mut UiContext, w: f32, h: f32, t: f32, progress: f32, art_ready: bool, fg_alpha: f32, atlas_alpha: f32, out: &mut RenderList) {
        let l = StartupLayout::new(w, h);
        let s = l.scale;
        if atlas_alpha >= 1. { out.fill(Rect::new(0., 0., w, h), [0., 0., 0., 1.]); }
        let a = |c: [f32; 4]| [c[0], c[1], c[2], c[3] * fg_alpha];
        if art_ready && atlas_alpha > 0. { out.sprite(art(ctx, ART_FILE).id, l.atlas, [1., 1., 1., atlas_alpha]); }
        if fg_alpha <= 0. { return; }
        let px = l.font_px;
        if art_ready {
            out.sprite(art(ctx, BADGE_FILE).id, l.plate, [1., 1., 1., fg_alpha]);
            let asc = crate::screens::ascent(ctx, FONT, px);
            let desc = crate::screens::line_height(ctx, FONT, px) - asc;
            let tw = crate::screens::text_width(ctx, FONT, BADGE_TEXT, px);
            let [x, base] = l.badge_text_origin(tw, asc, desc);
            put_text(ctx, out, FONT, px, x, base, BADGE_TEXT, a(BADGE_COLOR), None, Snap::PerGlyph);
        }
        // LOADING + throbber
        let asc = crate::screens::ascent(ctx, FONT, px);
        let desc = crate::screens::line_height(ctx, FONT, px) - asc;
        let tw = crate::screens::text_width(ctx, FONT, LOADING_TEXT, px);
        let row = l.loading_row(tw, asc, desc);
        put_text(ctx, out, FONT, px, row.text_x, row.baseline, LOADING_TEXT, a(LOADING_COLOR), None, Snap::PerGlyph);
        if art_ready {
            let piece = art(ctx, THROBBER_PIECE_FILE);
            let size = (piece.w as f32 * s, piece.h as f32 * s);
            for p in throbber_pieces(row.throbber_center, row.throbber_radius, size.1, t) { out.rotated(piece.id, (p.centre[0], p.centre[1]), size, p.angle, [1., 1., 1., fg_alpha]); }
            // the bar: Box brushes, margin 5/12 on every side, drawn at texture size * scale
            let frac = sl::bar_fraction(progress, fg_alpha < 1.);
            let (bg, fill) = (art(ctx, BAR_BACKGROUND_FILE), art(ctx, BAR_FILL_FILE));
            let m = [sl::BAR_BOX_MARGIN; 4];
            out.nine_slice(bg, l.bar, m, s, [1., 1., 1., fg_alpha]);
            if frac > 0. { out.nine_slice(fill, l.bar_fill(frac), m, s, [1., 1., 1., fg_alpha]); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn texts_and_files_are_the_exe_ones() {
        assert_eq!((BADGE_TEXT, LOADING_TEXT), ("EARLY ACCESS", "LOADING"));
        assert_eq!(FONT, FontId::BurbankBold);
        assert_eq!(PAK_TEXTURES, ["T_BattleRoyaleKeyArt.png", "T-Loading-EarlyAccessPlate.png", "ProgressBar_Background.png", "ProgressBar_Fill.png", "Throbber_Piece.png"]);
        assert_eq!(BADGE_COLOR, [0., 0., 0., 1.]);
    }
    #[test] fn throbber_has_six_pieces_on_one_circle_and_makes_one_turn_per_period() {
        let c = [100., 100.];
        let p0 = throbber_pieces(c, 20., 12., 0.);
        assert_eq!(p0.len(), 6);
        for p in &p0 { let d = ((p.centre[0] - c[0]).powi(2) + (p.centre[1] - c[1]).powi(2)).sqrt(); assert!((d - 14.).abs() < 1e-3); }
        let p1 = throbber_pieces(c, 20., 12., sl::THROBBER_PERIOD);
        for (a, b) in p0.iter().zip(&p1) { assert!((a.centre[0] - b.centre[0]).abs() < 1e-3 && (a.centre[1] - b.centre[1]).abs() < 1e-3); }
        let q = throbber_pieces(c, 20., 12., sl::THROBBER_PERIOD / 6.);
        assert!((q[0].centre[0] - p0[1].centre[0]).abs() < 1e-3, "a sixth of a period moves every piece onto its neighbour");
    }
}
