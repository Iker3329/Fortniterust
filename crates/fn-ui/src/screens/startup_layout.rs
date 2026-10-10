//! Layout of the start-up loading screen: `SInitialFortniteLoadingScreen` (FortniteLoadingScreen.cpp) of the 1.8 client, recovered by static analysis of
//! `FortniteClient-Win64-Shipping.exe` (crate `fn-exe-slate`; evidence for every number: `reports/exe-loading-screen.json` and `crates/fn-exe-slate/README.md`).
//!
//! STATUS: [`PROVISIONAL`] = `false`. The Slate construction code was decoded; each constant below says where it comes from. Items marked ASSUMED could not be
//! recovered from the exe (reason given). RVAs are image offsets (VA = 0x140000000 + RVA).
//!
//! ORIGINAL WIDGET TREE (SNew execution order is innermost first; lines are FortniteLoadingScreen.cpp lines):
//! ```text
//! SDPIScaler (184)  DPIScale = UUserInterfaceSettings::GetDPIScaleBasedOnSize(system resolution), computed ONCE at construction
//!  SOverlay (187)   slots, back to front:
//!   [0] SImage (190)  FORT_Load_Atlas, slot Fill/Fill, padding 0   (brush 1920x1080, only created when ResX >= 1920)
//!   [1] SSafeZone (197) > SBorder (199; HAlign Fill, VAlign Bottom, padding 16,4,16,4, NoBorder) > SVerticalBox (206)
//!         slot HAlign Left, AutoHeight:  SHorizontalBox (210)
//!             slot AutoWidth VAlign Center padding 0:        STextBlock (216) "LOADING"
//!             slot AutoWidth VAlign Center padding L16:      SCircularThrobber (226) Radius 20
//!         slot AutoHeight padding L0 T6 R0 B12 (HAlign Fill): SBox (235) HeightOverride 6 > SProgressBar (238)
//!   [2] SImage (256)  T-Loading-EarlyAccessPlate, slot Right/Top, padding L0 T40 R60 B0   (brush = texture size 584x160)
//!   [3] SBorder (266) > SBox (271; Width/HeightOverride = plate size, content Center/Center, padding 0,7,7,0) > STextBlock (278) "EARLY ACCESS"
//!       slot Right/Top, padding L0 T40 R60 B0  (so the text box coincides with the plate rectangle)
//! ```
//! All lengths are Slate units; pixels = units x DPI scale (`crate::layout::dpi_scale`, the cooked Vertical-rule curve used by every fn-ui screen).
//!
//! THINGS THE OLD PROVISIONAL FILE GOT WRONG (the original has): no dots after LOADING (a throbber instead), no marquee (the bar is a real fraction), the art is
//! stretched (not cover-fitted) and is absent below 1920 px width, the EARLY ACCESS text is 48 units black, the font is `BurbankBigRegular-Bold` (typeface
//! `Burbank-PlayerPower`), not Condensed-Black.
use crate::layout::Rect;

/// `true` while the numbers are screen-author estimates. Recovered from the exe: `false`.
pub const PROVISIONAL: bool = false;

/// Reference canvas (the atlas brush size; also the 1080p point of the DPI curve).
pub const REF_W: f32 = 1920.;
pub const REF_H: f32 = 1080.;

// ------------------------------------------------------------------------------------------------------------------------- key art
/// FORT_Load_Atlas (`/Game/UI/Foundation/Textures/LoadingScreens/FORT_Load_Atlas`, 1920x1080). Path string: file offset 0x44B73A0 (UTF-16), loaded at rva 0x124af8c.
pub const ART_FILE: &str = "FORT_Load_Atlas.png";
/// RECOVERED (rva 0x124af66..0x124af6e): the atlas `FSlateBrush.ImageSize`.
pub const ART_BRUSH_W: f32 = 1920.;
pub const ART_BRUSH_H: f32 = 1080.;
/// RECOVERED: the SImage sits in a Fill/Fill overlay slot with no padding and the brush is drawn as a plain image -> it is STRETCHED over the whole window
/// (aspect not preserved). `false` = stretch. (Was PROVISIONAL `true` = cover.)
pub const ART_COVER: bool = false;
/// RECOVERED (rva 0x124af74, `comiss xmm1,[1920.0]`; xmm1 = GSystemResolution.ResX): below this width the original creates NO atlas brush (null) and the
/// SImage draws nothing; the plate, texts and bar still show over the window background. A runtime that wants the art at any size ignores this on purpose.
pub const ART_MIN_RES_X: f32 = 1920.;

// ------------------------------------------------------------------------------------------------------------------------- font
/// RECOVERED: both texts use `FSlateFontInfo(Burbank, 48, "Burbank-PlayerPower")` (rva 0x124890e / 0x12491cd, r8d = 0x30). Size is in Slate units.
pub const FONT_ASSET: &str = "/Game/UI/Foundation/Fonts/Burbank";
pub const FONT_TYPEFACE: &str = "Burbank-PlayerPower";
/// RECOVERED from the composite font asset (`Burbank.uasset`, typeface table decoded with fn-ueio): "Burbank-PlayerPower" -> FontFace import `BurbankBigRegular-Bold`.
pub const FONT_FILE: &str = "BurbankBigRegular-Bold.ufont";
pub const FONT_SIZE: f32 = 48.;

// ------------------------------------------------------------------------------------------------------------------------- EARLY ACCESS plate
/// T-Loading-EarlyAccessPlate (584x160, transparent corners; no text on the texture).
pub const BADGE_FILE: &str = "T-Loading-EarlyAccessPlate.png";
/// RECOVERED/INFERRED: the plate brush `ImageSize` is the texture size (rva 0x124b877 reads [texture+0xf8]); the SImage keeps its desired size = brush size.
pub const BADGE_W: f32 = 584.;
pub const BADGE_H: f32 = 160.;
/// RECOVERED (rva 0x124a22f / 0x124a25f): slot Padding(L,T,R,B) = (0, 40, 60, 0), slot alignment Right/Top (rva 0x124a1b2 / 0x124a1b9).
pub const BADGE_MARGIN_TOP: f32 = 40.;
pub const BADGE_MARGIN_RIGHT: f32 = 60.;
/// The text drawn over the plate: NSLOCTEXT("FortniteLoadingScreen","EarlyAccess","EARLY ACCESS") (file offset 0x44B7B00).
pub const BADGE_TEXT: &str = "EARLY ACCESS";
/// RECOVERED: 48 Slate units.
pub const BADGE_TEXT_PX: f32 = 48.;
/// RECOVERED (rva 0x124886a): FLinearColor::Black (0,0,0,1) from the constant-initialised global 0x145198810.
pub const BADGE_TEXT_COLOR: [f32; 4] = [0., 0., 0., 1.];
/// RECOVERED (rva 0x1248970, 0x1444b9470 = 0,7,7,0): padding (L,T,R,B) of the SBox that holds the text; the text is centred (Center/Center, rva 0x1248cbe) in
/// the remaining area.
pub const BADGE_TEXT_PAD: [f32; 4] = [0., 7., 7., 0.];
/// ASSUMED (derived): baseline as a fraction of the plate height for BurbankBigRegular-Bold at 48 units (ascent 45.6, descent 14.4, line 60):
/// (7 + (153 - 60) / 2 + 45.6) / 160. The exact Slate line metrics (FreeType, rounding) are not in the exe; use [`StartupLayout::badge_text_origin`] with the real atlas metrics.
pub const BADGE_TEXT_BASELINE: f32 = 0.619;

// ------------------------------------------------------------------------------------------------------------------------- LOADING row
/// NSLOCTEXT("FortniteLoadingScreen","Loading","LOADING") (file offset 0x44B7C18).
pub const LOADING_TEXT: &str = "LOADING";
/// RECOVERED: 48 Slate units.
pub const LOADING_PX: f32 = 48.;
/// No ColorAndOpacity is set on this STextBlock -> style "NormalText" (UseForeground) -> the inherited foreground = Slate's `DefaultForeground`
/// FLinearColor(0.72, 0.72, 0.72) (UE4.16 SlateCore CoreStyle.cpp:31), multiplied by the container's fading alpha. sRGB-encoded here (put_text takes
/// sRGB): 0.72 linear = 0.8613 (219); the live 1.8 client's LOADING pixels are 218.
pub const LOADING_COLOR: [f32; 4] = [0.8613, 0.8613, 0.8613, 1.];
/// RECOVERED (rva 0x12492b2, 0x1444b9490 = 16,4,16,4): SBorder padding. Left edge of the LOADING text (the row slot is HAlign Left).
pub const LOADING_MARGIN_LEFT: f32 = 16.;
/// ASSUMED (derived): distance from the bar slot's top (`h - (BAR_H + BAR_MARGIN_BOTTOM)` above) to the text BASELINE, in the old API's terms
/// (`baseline = h - LOADING_MARGIN_BOTTOM - BAR_H - BAR_MARGIN_BOTTOM`): bar slot top padding 6 + descent of the font at 48 units (14.4).
pub const LOADING_MARGIN_BOTTOM: f32 = 20.4;
/// Legacy (NOT in the original; the old screen author's cadence). The original draws NO dots: the animation next to LOADING is the throbber below.
pub const DOT_STEP_SECONDS: f32 = 0.33;
pub const MAX_DOTS: usize = 3;
/// RECOVERED (rva 0x1249658 for the radius; defaults from ctor 0x12478c0): SCircularThrobber next to LOADING, in its own slot with padding L=16 (rva 0x12497d9),
/// VAlign Center (0x12497b0).
pub const THROBBER_RADIUS: f32 = 20.;
pub const THROBBER_PAD_LEFT: f32 = 16.;
pub const THROBBER_PIECES: u32 = 6;
pub const THROBBER_PERIOD: f32 = 0.75;
/// INFERRED (engine: SCircularThrobber::ComputeDesiredSize = 2*Radius).
pub const THROBBER_DESIRED: f32 = 40.;

// ------------------------------------------------------------------------------------------------------------------------- progress bar
/// ProgressBar_* textures: 12x12 background and fill (Box brushes), 20x12 marquee (unused: the bar always has a Percent).
pub const SHOW_BAR: bool = true;
pub const BAR_H: f32 = 6.;
/// RECOVERED: bar SBox HeightOverride 6 (rva 0x124911e); slot Padding(0,6,0,12) (rva 0x1249606) inside the border padded 16,4,16,4:
/// bar spans `x = 16 .. W-16` (units), bottom edge `4 + 12 = 16` units above the window bottom.
pub const BAR_MARGIN_X: f32 = 16.;
pub const BAR_MARGIN_BOTTOM: f32 = 16.;
/// Space between the LOADING row and the bar (slot top padding).
pub const BAR_SLOT_PAD_TOP: f32 = 6.;
/// RECOVERED (rva 0x124bcb6, 0x1444b9480): Background and Fill brushes are DrawAs=Box with margin 0.41666667 (= 5/12, UV fractions) on every side.
pub const BAR_BOX_MARGIN: f32 = 5. / 12.;
pub const BAR_TEX_SIZE: f32 = 12.;
/// RECOVERED (rva 0x124bf21 / 0x124bf2d): marquee brush ImageSize (20, 12); never drawn (see [`BAR_PERCENT_SCALE`]).
pub const MARQUEE_TEX: [f32; 2] = [20., 12.];
/// RECOVERED (rva 0x124b223, double 0.98): displayed fraction = clamp(Progress * 0.98, 0, 1); Progress is the float the FortAssetManager startup jobs push through the module's
/// SetProgress (sum of finished job weights / sum of all weights). While the fade curve plays the getter returns 1.0 (rva 0x124b20e).
pub const BAR_PERCENT_SCALE: f32 = 0.98;
/// ENGINE default kept by the original (ctor 0x45c7c0): SProgressBar.BorderPadding = (1, 0): the fill is inset by 1 unit left and right (ASSUMED how OnPaint uses it).
pub const BAR_BORDER_PADDING_X: f32 = 1.;
/// Legacy (NOT in the original; the bar is never in marquee mode because the Percent getter always returns a value).
pub const MARQUEE_SPEED: f32 = 0.55;
pub const MARQUEE_BLOCK: f32 = 0.22;

// ------------------------------------------------------------------------------------------------------------------------- timing
/// RECOVERED (rva 0x12483b5): the fade curve length for both widget variants.
pub const FADE_SECONDS: f32 = 0.2;
/// RECOVERED (rva 0x12483c0): the second widget variant (bFlag = 1, added to the game viewport at ZOrder [`VIEWPORT_ZORDER`]) waits this long after the module's
/// "hide requested" before it starts the 0.2 s fade (the first variant, bFlag = 0, fades at once when the movie player reports loading finished: delay 0, rva 0x1248410).
pub const FLAG1_FADE_DELAY_SECONDS: f32 = 1.;
pub const VIEWPORT_ZORDER: i32 = 20000;
/// RECOVERED (rva 0x61b44d, "hard-coded... ick!"): UFortLoadingScreenManager keeps the in-game loading screen up until 2.0 s after TimeLoadingScreenFirstDismissed.
pub const MANAGER_MIN_SHOW_SECONDS: f32 = 2.;

// ------------------------------------------------------------------------------------------------------------------------- the layout
/// Pixel geometry of the screen for a window, per the Slate rules above. `scale` is the DPI scale; all `Rect`s are window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StartupLayout {
    pub scale: f32,
    pub w: f32,
    pub h: f32,
    /// the atlas image: the whole window (stretched)
    pub atlas: Rect,
    /// the plate image at its native 584x160 units, top-right
    pub plate: Rect,
    /// area of the SBox around the EARLY ACCESS text after its padding (the text is centred in it)
    pub plate_text_area: Rect,
    /// progress bar rect (Background brush)
    pub bar: Rect,
    /// y of the bottom edge of the LOADING row (= top of the bar slot)
    pub row_bottom: f32,
    /// font size in pixels for both texts (`atlas_px` rule: round(48 * scale), at least 6)
    pub font_px: u32,
}

impl StartupLayout {
    /// DPI scale from the shared curve (`crate::layout::dpi_scale`).
    pub fn new(w: f32, h: f32) -> Self { Self::with_scale(w, h, crate::layout::dpi_scale(w, h)) }

    pub fn with_scale(w: f32, h: f32, s: f32) -> Self {
        let plate = Rect::new(w - (BADGE_MARGIN_RIGHT + BADGE_W) * s, BADGE_MARGIN_TOP * s, BADGE_W * s, BADGE_H * s);
        let pad = BADGE_TEXT_PAD;
        let plate_text_area = Rect::new(plate.x + pad[0] * s, plate.y + pad[1] * s, plate.w - (pad[0] + pad[2]) * s, plate.h - (pad[1] + pad[3]) * s);
        let bar = Rect::new(BAR_MARGIN_X * s, h - (BAR_MARGIN_BOTTOM + BAR_H) * s, (w - 2. * BAR_MARGIN_X * s).max(1.), BAR_H * s);
        let row_bottom = bar.y - BAR_SLOT_PAD_TOP * s;
        Self { scale: s, w, h, atlas: Rect::new(0., 0., w, h), plate, plate_text_area, bar, row_bottom, font_px: crate::screens::atlas_px(FONT_SIZE, s) }
    }

    /// Whether the original would create the atlas brush at this system resolution width (see [`ART_MIN_RES_X`]).
    pub fn atlas_brush_exists(res_x: f32) -> bool { res_x >= ART_MIN_RES_X }

    /// EARLY ACCESS text: (pen x, baseline y) in pixels for a line of width `text_w` with the font's `ascent` and `descent` (positive) in pixels:
    /// centred horizontally and vertically in [`StartupLayout::plate_text_area`].
    pub fn badge_text_origin(&self, text_w: f32, ascent: f32, descent: f32) -> [f32; 2] {
        let a = self.plate_text_area;
        let lh = ascent + descent;
        [a.x + (a.w - text_w) * 0.5, a.y + (a.h - lh) * 0.5 + ascent]
    }

    /// The LOADING row (text + throbber), bottom-left above the bar. `text_w`, `ascent`, `descent` in pixels.
    pub fn loading_row(&self, text_w: f32, ascent: f32, descent: f32) -> LoadingRow {
        let s = self.scale;
        let lh = ascent + descent;
        let row_h = lh.max(THROBBER_DESIRED * s);
        let top = self.row_bottom - row_h;
        let x = LOADING_MARGIN_LEFT * s;
        let text_top = top + (row_h - lh) * 0.5;
        let throbber_x = x + text_w + THROBBER_PAD_LEFT * s;
        LoadingRow {
            text_x: x, baseline: text_top + ascent, row: Rect::new(x, top, text_w + (THROBBER_PAD_LEFT + THROBBER_DESIRED) * s, row_h),
            throbber_center: [throbber_x + THROBBER_RADIUS * s, top + row_h * 0.5], throbber_radius: THROBBER_RADIUS * s,
        }
    }

    /// Fill rect of the progress bar for a displayed fraction in 0..=1 (BorderPadding X inset, LeftToRight).
    pub fn bar_fill(&self, fraction: f32) -> Rect {
        let inset = BAR_BORDER_PADDING_X * self.scale;
        let inner_w = (self.bar.w - 2. * inset).max(0.);
        Rect::new(self.bar.x + inset, self.bar.y, inner_w * fraction.clamp(0., 1.), self.bar.h)
    }
}

/// Geometry of the LOADING row in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadingRow { pub text_x: f32, pub baseline: f32, pub row: Rect, pub throbber_center: [f32; 2], pub throbber_radius: f32 }

/// Displayed progress (the SProgressBar.Percent getter): `fading` = the fade curve is playing.
pub fn bar_fraction(progress: f32, fading: bool) -> f32 { if fading { 1. } else { (progress as f64 * BAR_PERCENT_SCALE as f64).clamp(0., 1.) as f32 } }

/// Alpha of a fading layer `t` seconds after `Play`: `1 - clamp((t - delay) / FADE_SECONDS, 0, 1)` (GetLerp of a linear curve; see `fade.alpha`).
pub fn fade_alpha(t: f32, delay: f32) -> f32 { 1. - ((t - delay) / FADE_SECONDS).clamp(0., 1.) }

/// Throbber revolution phase in 0..1 at time `t` (one revolution per [`THROBBER_PERIOD`]; ASSUMED how the engine maps it to piece positions).
pub fn throbber_phase(t: f32) -> f32 { (t / THROBBER_PERIOD).rem_euclid(1.) }

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f32, b: f32) -> bool { (a - b).abs() < 1e-3 }
    #[test] fn constants_are_the_recovered_slate_numbers() {
        assert!(!PROVISIONAL);
        assert_eq!((BADGE_W, BADGE_H, BADGE_MARGIN_TOP, BADGE_MARGIN_RIGHT), (584., 160., 40., 60.));
        assert_eq!(BADGE_TEXT_PAD, [0., 7., 7., 0.]);
        assert_eq!((FONT_SIZE, BADGE_TEXT_PX, LOADING_PX), (48., 48., 48.));
        assert_eq!((BAR_H, BAR_MARGIN_X, BAR_MARGIN_BOTTOM, BAR_SLOT_PAD_TOP), (6., 16., 16., 6.));
        assert_eq!((THROBBER_RADIUS, THROBBER_PAD_LEFT, THROBBER_PIECES, THROBBER_PERIOD), (20., 16., 6, 0.75));
        assert_eq!((FADE_SECONDS, FLAG1_FADE_DELAY_SECONDS, MANAGER_MIN_SHOW_SECONDS, VIEWPORT_ZORDER), (0.2, 1., 2., 20000));
        assert!(close(BAR_BOX_MARGIN, 0.416_666_67));
        assert!(!ART_COVER && ART_MIN_RES_X == 1920.);
    }
    #[test] fn plate_is_top_right_at_native_size_with_the_overlay_padding() {
        let l = StartupLayout::with_scale(1920., 1080., 1.);
        assert_eq!((l.plate.x, l.plate.y, l.plate.w, l.plate.h), (1920. - 60. - 584., 40., 584., 160.));
        assert_eq!((l.plate_text_area.x, l.plate_text_area.y, l.plate_text_area.w, l.plate_text_area.h), (l.plate.x, 47., 577., 153.));
        let l2 = StartupLayout::with_scale(2560., 1440., 1.5);
        assert!(close(l2.plate.right(), 2560. - 60. * 1.5) && close(l2.plate.w, 584. * 1.5) && close(l2.plate.y, 60.));
    }
    #[test] fn badge_text_is_centred_in_the_padded_box() {
        let l = StartupLayout::with_scale(1920., 1080., 1.);
        // BurbankBigRegular-Bold @ 48: advance("EARLY ACCESS") = 284.256, ascent 45.6, descent 14.4 -> baseline 99.1 below the plate top = 0.619 of 160
        let [x, base] = l.badge_text_origin(284.256, 45.6, 14.4);
        assert!(close(x, l.plate.x + (577. - 284.256) / 2.) && close(base, 40. + 7. + (153. - 60.) / 2. + 45.6));
        assert!(((base - l.plate.y) / l.plate.h - BADGE_TEXT_BASELINE).abs() < 1e-3);
    }
    #[test] fn bar_spans_the_window_minus_the_border_padding_and_sits_16_units_up() {
        let l = StartupLayout::with_scale(1920., 1080., 1.);
        assert_eq!((l.bar.x, l.bar.y, l.bar.w, l.bar.h), (16., 1080. - 22., 1920. - 32., 6.));
        assert!(close(l.bar.bottom(), 1080. - 16.));
        assert!(close(l.row_bottom, 1080. - 28.));
        let f = l.bar_fill(0.5);
        assert!(close(f.x, 17.) && close(f.w, (1888. - 2.) * 0.5));
    }
    #[test] fn loading_row_is_bottom_left_with_the_throbber_right_of_the_text() {
        let l = StartupLayout::with_scale(1920., 1080., 1.);
        let r = l.loading_row(168.528, 45.6, 14.4);
        assert!(close(r.text_x, 16.) && close(r.baseline, 1080. - 28. - 14.4));
        assert!(close(r.throbber_center[0], 16. + 168.528 + 16. + 20.) && close(r.throbber_center[1], 1080. - 28. - 30.) && r.throbber_radius == 20.);
        // the legacy margin constants reproduce the same baseline through the old formula (baseline = h - LOADING_MARGIN_BOTTOM - BAR_H - BAR_MARGIN_BOTTOM)
        assert!(close(1080. - LOADING_MARGIN_BOTTOM - BAR_H - BAR_MARGIN_BOTTOM, r.baseline));
    }
    #[test] fn progress_and_fade_follow_the_getters() {
        assert!(close(bar_fraction(0.5, false), 0.49) && bar_fraction(1.2, false) == 1. && bar_fraction(-1., false) == 0. && bar_fraction(0.1, true) == 1.);
        assert_eq!(fade_alpha(0., 0.), 1.);
        assert!(close(fade_alpha(0.1, 0.), 0.5) && fade_alpha(0.3, 0.) == 0.);
        assert_eq!(fade_alpha(0.9, 1.), 1.);
        assert!(close(fade_alpha(1.1, 1.), 0.5));
        assert!(close(throbber_phase(0.75 + 0.375), 0.5));
    }
    #[test] fn atlas_gate_matches_the_exe() { assert!(StartupLayout::atlas_brush_exists(1920.) && !StartupLayout::atlas_brush_exists(1919.)); }
    #[test] fn new_uses_the_shared_dpi_curve() {
        let l = StartupLayout::new(1280., 720.);
        assert!(close(l.scale, 0.666) && l.font_px == 43); // 48 pt * 96/72 * 0.666
    }
}
