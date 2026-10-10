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
//! * the full-window art is `FORT_Load_Atlas` (`/Game/UI/Foundation/Textures/LoadingScreens/`), the texture the exe loads (Fill/Fill, stretched, no padding),
//!   as the live 1.8 client shows it (FRONT END, 2026-10-06; it used to be replaced by the BR key art).
//! * FRONT END: when the start-up work is done only the `bFlag == 0` layer (plate, EARLY ACCESS, LOADING + throbber, bar) fades (0.2 s); the art stays up alone
//!   until the login screen replaces it (the live client: art only from ~0.2 s to ~3.0 s, then "Logging In...").
//!
//! INFERRED / ASSUMED (not in the exe): the exact Slate line metrics (fontdue instead of FreeType); the 1 unit
//! `BorderPadding` of the bar fill. The old prototype's invented parts (cover-scaled art, 292-wide plate, 36 px Condensed-Black text, 240x4 bar, text shadow) are gone.
use crate::draw::RenderList;
use crate::layout::Rect;
use crate::scene::UiContext;
use crate::screens::startup_layout::{self as sl, StartupLayout};
use crate::screens::{art, put_text, Snap};
use crate::text::FontId;

/// Textures this screen draws (decoded before it appears): Athena key art, the plate, the progress-bar brushes, the throbber piece.
pub const PAK_TEXTURES: [&str; 5] = [ART_FILE, BADGE_FILE, BAR_BACKGROUND_FILE, BAR_FILL_FILE, THROBBER_PIECE_FILE];
/// `FORT_Load_Atlas`: the full-window art the exe draws.
pub const ART_FILE: &str = "FORT_Load_Atlas.png";
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

/// One throbber piece: its box (window px).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThrobberPiece { pub rect: Rect }
/// `Throbber.CircleChunk` = `IMAGE_BRUSH("Common/Throbber_Piece", FVector2D(8,8))` (UE4.16 SlateCore CoreStyle.cpp:594): the piece's ImageSize.
pub const PIECE_SIZE: f32 = 8.;

/// `SCircularThrobber::OnPaint` (UE4.16 SThrobber.cpp:145): the widget is `2 * Radius` square; `LocalOffset = (Size - ImageSize) / 2`, `Phase = lerp * 2PI`;
/// piece i (0-based) is drawn at `LocalOffset + LocalOffset * (sin(a), cos(a))` with `a = i * 2PI/N + Phase`, scaled `(i + 1) / N` (no rotation).
/// `center` / `radius` in window px, `scale` = the DPI scale.
pub fn throbber_pieces(center: [f32; 2], radius: f32, scale: f32, t: f32) -> Vec<ThrobberPiece> {
    let n = sl::THROBBER_PIECES as f32;
    let delta = std::f32::consts::TAU / n;
    let phase = sl::throbber_phase(t) * std::f32::consts::TAU;
    let img = PIECE_SIZE * scale;
    let (x0, y0) = (center[0] - radius, center[1] - radius);
    let off = (2. * radius - img) * 0.5;
    (0..sl::THROBBER_PIECES).map(|i| {
        let angle = delta * i as f32 + phase;
        let k = (i + 1) as f32 / n;
        ThrobberPiece { rect: Rect::new(x0 + off + off * angle.sin(), y0 + off + off * angle.cos(), img * k, img * k) }
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
            for p in throbber_pieces(row.throbber_center, row.throbber_radius, s, t) { out.sprite(piece.id, p.rect, [1., 1., 1., fg_alpha]); }
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
        assert_eq!(PAK_TEXTURES, ["FORT_Load_Atlas.png", "T-Loading-EarlyAccessPlate.png", "ProgressBar_Background.png", "ProgressBar_Fill.png", "Throbber_Piece.png"]);
        assert_eq!(BADGE_COLOR, [0., 0., 0., 1.]);
    }
    #[test] fn throbber_is_the_engines_circular_throbber() {
        let c = [100., 100.];
        let p0 = throbber_pieces(c, 20., 1., 0.);
        assert_eq!(p0.len(), 6);
        // piece 0 at angle 0: LocalOffset (16,16) + (0, 16) -> (16, 32) from the widget's corner (80, 80), 8/6 units square; the last piece is full size
        assert_eq!(p0[0].rect, Rect::new(96., 112., 8. / 6., 8. / 6.));
        assert_eq!((p0[5].rect.w, p0[5].rect.h), (8., 8.));
        let p1 = throbber_pieces(c, 20., 1., sl::THROBBER_PERIOD);
        for (a, b) in p0.iter().zip(&p1) { assert!((a.rect.x - b.rect.x).abs() < 1e-3 && (a.rect.y - b.rect.y).abs() < 1e-3); }
    }
}
