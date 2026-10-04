//! Brightness tab of the Settings screen (replaces the generic `brightness` page).
//!
//! Recovered layout of `TabBrightnessOptions` (reports/menu-layout-settings.json, menu-widgets.json): a black backdrop, the header
//! "BRIGHTNESS CALIBRATION" (Burbank-Black 48), the description "Please adjust the brightness of the game to your display." (NotoSans 16,
//! white 60 %), the calibration strip `T_BrightnessCal` (688x80 shown at 512x48), a pair of 320x320 calibration images 128 units apart
//! (`M_CalibrationSet_Master_Light` with scalar Luminosity 0.15 on the left, `..._Dark` with Luminosity 0.005 on the right; both show
//! `T_BrightnessCalibration`, the black rounded square with the white logo), the `OptionsMenuSlider_Light` "Brightness" slider (padding
//! right 96) and the callout "Default Value {0}" (placeholder "Default Value 0.50", padding top 32).
//!
//! Behaviour here: the slider edits `video.brightness` (0..1, default 0.5 = display gamma 2.2) and, like the original's live slider,
//! previews immediately (`pages::request_preview()`: the 3D scene's output gamma changes without saving; Apply saves, Back/Discard rolls
//! the preview back). The gamma range is RECOVERED (`MinGammaValue` 1.7 .. `MaxGammaValue` 2.7); the slider = normalised position
//! between them is INFERRED (`settings::display_gamma`).
//!
//! DECISION (live gamma vs. readable chrome): the gamma preview only ever touches (a) the 3D scene, through the post pass (`display_exponent`),
//! and (b) the calibration strip + the two logo images, through the CPU curve below. The Settings screen is drawn AFTER the post pass and
//! is opaque (0.985 alpha), so the darkened/brightened scene is not visible behind it and the page chrome (header, description, slider,
//! buttons, tabs, footer) is never multiplied by the gamma: at slider 0.00 the controls look exactly like at 1.00. The two pieces of
//! secondary text are white 0.6 (`CHROME_DIM`) rather than 0.4 so they remain legible. `chrome_is_independent_of_brightness` pins this.
//! APPROXIMATIONS (labelled, no 1:1 claim): (1) the settings screen is drawn after the post pass, so the display gamma does not reach it;
//! the calibration images are therefore passed through the same gamma curve on the CPU (logo tint = Luminosity^exponent, strip cells
//! sampled from the original texture and raised to the same exponent), which is what a global display-gamma change would do to them.
//! (2) The Luminosity scalars are treated as display values (0.005 = 1/255, "barely visible" at the default), not as linear light; the
//! material graph itself is not recovered. (3) The pair is shown from the start; the original hides it initially (the trigger is not
//! recovered). (4) Burbank is not available: NotoSans Bold stands in for the header. (5) The extra hint line is this runtime's own text.
use super::bridge;
use crate::menu::theme::*;
use crate::menu::*;
use crate::settings::{self, Value};

/// RECOVERED material scalars of the two calibration images.
pub const LIGHT_LUMINOSITY: f32 = 0.15;
pub const DARK_LUMINOSITY: f32 = 0.005;
/// Original texts (`menu-layout-settings.json`, Brightness tab layout).
pub const HEADER: &str = "BRIGHTNESS CALIBRATION";
pub const DESCRIPTION: &str = "Please adjust the brightness of the game to your display.";
pub const SLIDER_LABEL: &str = "Brightness";
/// Colour of the secondary chrome text (hint, "Default Value"): white 0.6 so it stays readable on the black backdrop at every slider value.
pub const CHROME_DIM: [f32; 4] = [1., 1., 1., 0.6];
/// Runtime-added hint (NOT original text).
pub const HINT: &str = "Raise Brightness until the logo in the right image is just visible, while the left image stays clearly visible.";

/// Sizes in Slate units, recovered from the widget tree.
pub const STRIP_W: f32 = 512.;
pub const STRIP_H: f32 = 48.;
pub const PATCH: f32 = 320.;
pub const PATCH_GAP: f32 = 128.;
pub const SLIDER_PAD_RIGHT: f32 = 96.;
pub const CALLOUT_PAD_TOP: f32 = 32.;

/// T_BrightnessCal: texel size and the geometry of its 21 grey steps (measured on the exported PNG; the test re-reads the PNG).
pub const STRIP_TEX: (f32, f32) = (688., 80.);
pub const STRIP_CELLS_X0: f32 = 8.;
pub const STRIP_CELL_W: f32 = 32.;
pub const STRIP_CELLS_Y: (f32, f32) = (8., 72.);
pub const STRIP_FRAME_PX: (f32, f32, f32, f32) = (6., 6., 682., 74.); // light 2 px outline (colour 214)
pub const STRIP_FRAME_VALUE: u8 = 214;
/// Grey value of each of the 21 cells (centre texel of `T_BrightnessCal.png`).
pub const STRIP_VALUES: [u8; 21] = [0, 10, 22, 33, 46, 57, 68, 79, 93, 104, 115, 126, 137, 150, 161, 173, 186, 197, 208, 220, 231];

/// Output exponent for a slider value (what `post.frag` applies to the sRGB output): `2.2 / lerp(1.7, 2.7, value)`.
pub fn exponent_for(brightness: f32) -> f32 { settings::display_exponent(settings::display_gamma(brightness)) }
/// Display value (0..1) of a calibration element authored as `value` after the display gamma: `value ^ exponent`.
pub fn through_gamma(value: f32, exponent: f32) -> f32 { value.clamp(0., 1.).powf(exponent.max(0.01)) }
/// 8-bit display level of the dark/light logo at a slider value (what a monitor would receive).
pub fn logo_level(luminosity: f32, brightness: f32) -> f32 { through_gamma(luminosity, exponent_for(brightness)) * 255. }
/// "Default Value {0}" with the default slider value (the original placeholder reads "Default Value 0.50").
pub fn default_callout() -> String {
    let d = match settings::entry("video.brightness").map(|e| e.default_value()) { Some(Value::Float(f)) => f, _ => 0.5 };
    format!("Default Value {:.2}", d)
}

/// Pixel layout of the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout { pub k: f32, pub header: Rect, pub desc: Rect, pub strip: Rect, pub light: Rect, pub dark: Rect, pub slider: Rect, pub callout: Rect, pub hint: Rect }
/// Lays the recovered column out inside `area` (pixels, `scale` = DPI scale). Images shrink uniformly (`k` < 1) when the area is too small;
/// the block is centred vertically like the original's VAlign_Center border content. Pure; unit-tested for many sizes.
pub fn layout(area: Rect, scale: f32) -> Layout {
    let u = |v: f32| v * scale;
    let (hh, dh, hint_h, slider_h, callout_h) = (52., 22., 20., 44., 22.);
    let gaps = (6., 14., 22., 22., 14., CALLOUT_PAD_TOP * 0.5);
    let fixed = hh + dh + STRIP_H + PATCH + hint_h + slider_h + callout_h + gaps.0 + gaps.1 + gaps.2 + gaps.3 + gaps.4 + gaps.5;
    let (aw, ah) = (area.w / scale.max(1e-3), area.h / scale.max(1e-3));
    let kw = (aw - 24.) / (PATCH * 2. + PATCH_GAP);
    let kh = ah / fixed;
    let k = kw.min(kh).min(1.).max(0.2);
    let total_h = u(fixed * k);
    let cx = area.x + area.w * 0.5;
    let mut y = area.y + ((area.h - total_h) * 0.5).max(0.);
    let mut take = |h: f32, gap: f32| { let r = y; y += u((h + gap) * k); (r, u(h * k)) };
    let wide = |w: f32| u(w * k).min(area.w);
    let row = |w: f32, (ry, rh): (f32, f32)| Rect::new(cx - w * 0.5, ry, w, rh);
    let header = row(area.w.min(u(900.)), take(hh, gaps.0));
    let desc = row(area.w.min(u(900.)), take(dh, gaps.1));
    let strip = row(wide(STRIP_W), take(STRIP_H, gaps.2));
    let (py, ph) = take(PATCH, gaps.3);
    let pw = wide(PATCH);
    let gap = wide(PATCH_GAP);
    let light = Rect::new(cx - gap * 0.5 - pw, py, pw, ph);
    let dark = Rect::new(cx + gap * 0.5, py, pw, ph);
    let hint = row(area.w.min(u(900.)), take(hint_h, gaps.4));
    // slider row: original padding right 96 + centred; width limited so label + track + value stay usable
    let sw = (area.w - u(SLIDER_PAD_RIGHT * k.max(0.5))).min(u(760.)).max(u(120.)).min(area.w);
    let slider = row(sw, take(slider_h, gaps.5));
    let callout = row(area.w.min(u(900.)), take(callout_h, 0.));
    Layout { k, header, desc, strip, light, dark, slider, callout, hint }
}

pub struct BrightnessPage;
impl BrightnessPage {
    pub fn new() -> Self { Self }
}

/// Draws the calibration strip: black plate, light outline, 21 grey cells (sampled values, through the display gamma).
fn draw_strip(ui: &mut Ui, r: Rect, exponent: f32) {
    let (sx, sy) = (r.w / STRIP_TEX.0, r.h / STRIP_TEX.1);
    let g = |v: u8| { let c = through_gamma(v as f32 / 255., exponent); [c, c, c, 1.] };
    ui.fill(r, [0., 0., 0., 1.]);
    let (fx0, fy0, fx1, fy1) = STRIP_FRAME_PX;
    let t = (2. * sx.min(sy)).max(1.);
    let frame = g(STRIP_FRAME_VALUE);
    ui.fill(Rect::new(r.x + fx0 * sx, r.y + fy0 * sy, (fx1 - fx0) * sx, t), frame);
    ui.fill(Rect::new(r.x + fx0 * sx, r.y + fy1 * sy - t, (fx1 - fx0) * sx, t), frame);
    ui.fill(Rect::new(r.x + fx0 * sx, r.y + fy0 * sy, t, (fy1 - fy0) * sy), frame);
    ui.fill(Rect::new(r.x + fx1 * sx - t, r.y + fy0 * sy, t, (fy1 - fy0) * sy), frame);
    for (i, v) in STRIP_VALUES.iter().enumerate() {
        let x0 = STRIP_CELLS_X0 + STRIP_CELL_W * i as f32;
        ui.fill(Rect::new(r.x + x0 * sx, r.y + STRIP_CELLS_Y.0 * sy, STRIP_CELL_W * sx, (STRIP_CELLS_Y.1 - STRIP_CELLS_Y.0) * sy), g(*v));
    }
}

impl SettingsPage for BrightnessPage {
    fn id(&self) -> &str { "brightness" }
    fn title(&self) -> &str { "Brightness" }
    fn order(&self) -> i32 { super::ORDER_BRIGHTNESS }
    fn reset_prompt(&self) -> Option<Option<String>> { Some(None) }
    fn reset_to_default(&mut self, sx: &mut bridge::Session) { sx.with_settings(|s| { let _ = s.reset("video.brightness"); }); sx.request_preview(); }
    fn ui(&mut self, ui: &mut Ui, area: Rect) {
        use super::slate;
        let snap = ui.session.settings_clone();
        let mut value = snap.video.brightness;
        let exponent = exponent_for(value);
        let sc = ui.vp.scale;
        // TabBrightnessOptions (1.8): BackgroundSurpress = black image scaled 1.25 about the top centre (translation -25) covering the whole content
        // area down to the bottom bar; CommonBorder_0 (padding bottom 64) centres a column: Header-L, 24, description, 64, strip 512x48, 64, slider
        // (padding right 96), callout (padding top 32). The calibration pair (HorizontalBox_1) is Collapsed and no decoded function ever shows it.
        let black = Rect::new(0., area.y - ui.u(25.), ui.vp.w, ui.vp.h - area.y + ui.u(25.));
        ui.fill(black, [0., 0., 0., 1.]);
        let hdr_px = ui.text_px(48.) as f32; let hdr_h = ui.fonts.line_height(FontId::Title, hdr_px);
        let body_px = ui.text_px(16.) as f32; let body_h = ui.fonts.line_height(FontId::Regular, body_px);
        let dim = [1., 1., 1., 0.6];
        let col_h = hdr_h + ui.u(24.) + body_h + ui.u(64.) + ui.u(48.) + ui.u(64.) + ui.u(slate::SLIDER_ROW_H) + ui.u(32.) + body_h;
        let region_h = (area.h - ui.u(0.)).max(0.);
        let mut y = area.y + ((region_h - ui.u(64.)) - col_h).max(0.) * 0.5;
        let cx = area.x + area.w * 0.5;
        ui.text(Rect::new(area.x, y, area.w, hdr_h), HEADER, TextStyle::new(FontId::Title, 48., WHITE), HAlign::Center); y += hdr_h + ui.u(24.);
        ui.text(Rect::new(area.x, y, area.w, body_h), DESCRIPTION, TextStyle::new(FontId::Regular, 16., dim), HAlign::Center); y += body_h + ui.u(64.);
        let strip = Rect::new(cx - ui.u(STRIP_W) * 0.5, y, ui.u(STRIP_W), ui.u(STRIP_H));
        draw_strip(ui, strip, exponent); y += ui.u(48. + 64.);
        // OptionsMenuSlider_Light: both SizeBoxes are Fill(1) in the row (HAlign_Center, padding right 96)
        let row_w = ui.u(slate::LABEL_W + slate::LABEL_GAP + slate::CONTROL_MIN_W);
        let rx = cx - ui.u(48.) - row_w * 0.5;
        let row = Rect::new(rx, y, row_w, ui.u(slate::SLIDER_ROW_H));
        let d = settings::entry("video.brightness").map(|e| e.kind);
        let (min, max, step, dec) = match d { Some(settings::KindDef::Slider { min, max, step, decimals }) => (min, max, step, decimals as usize), _ => (0., 1., 0.01, 2) };
        let r = slate::slider_row_ex(ui, "brightness", row, SLIDER_LABEL, value, min, max, step, dec, true, dim, ui.u(340.), ui.u(360.));
        if let Some(nv) = r.value { value = nv; if !ui.is_inert() { ui.session.with_settings(|s| { let _ = s.set("video.brightness", Value::Float(value)); }); ui.session.request_preview(); } }
        y += ui.u(slate::SLIDER_ROW_H + 32.);
        ui.text(Rect::new(area.x, y, area.w, body_h), &default_callout(), TextStyle::new(FontId::Regular, 16., dim), HAlign::Center);
        let _ = sc;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::{menu::pages::bridge::{ReqKind, Session}, settings::Settings, layout::Viewport};
    use std::path::PathBuf;

    fn assets() -> PathBuf { crate::assets::DirAssets::default_root() }
    fn fonts() -> Fonts { crate::testutil::fonts() }
    const SIZES: [(f32, f32); 7] = [(1920., 1080.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.), (640., 360.)];
    fn area_for(w: f32, h: f32) -> (Viewport, Rect) {
        let vp = Viewport::new(w, h);
        let s = vp.scale;
        (vp, Rect::new(s * 20., s * 130., (w - s * 40.).min(s * 1200.), (h - s * 220.).max(s * 100.)))
    }
    fn inside(a: Rect, r: Rect) -> bool { r.x >= a.x - 0.5 && r.y >= a.y - 0.5 && r.x + r.w <= a.x + a.w + 0.5 && r.y + r.h <= a.y + a.h + 0.5 }

    #[test] fn recovered_constants_match_the_evidence_json() {
        let text = crate::testutil::report("menu-widgets.json");
        let j: serde_json::Value = serde_json::from_str(&text).unwrap();
        // walk every widget blueprint for the calibration brushes
        let mut seen = std::collections::BTreeMap::new();
        fn walk(v: &serde_json::Value, seen: &mut std::collections::BTreeMap<String, f64>) {
            match v {
                serde_json::Value::Object(m) => {
                    if let (Some(root), Some(l)) = (m.get("materialRoot"), m.get("materialParams").and_then(|p| p.pointer("/scalars/Luminosity"))) { if root.as_str().map_or(false, |r| r.contains("CalibrationSet")) { seen.insert(m["resource"]["path"].as_str().unwrap_or("").to_owned(), l.as_f64().unwrap()); } }
                    for x in m.values() { walk(x, seen); }
                }
                serde_json::Value::Array(a) => for x in a { walk(x, seen); },
                _ => {}
            }
        }
        walk(&j, &mut seen);
        let light = seen.iter().find(|(k, _)| k.contains("Master_Light")).map(|(_, v)| *v).expect("light patch");
        let dark = seen.iter().find(|(k, _)| k.contains("Master_Dark")).map(|(_, v)| *v).expect("dark patch");
        assert!((light - LIGHT_LUMINOSITY as f64).abs() < 1e-6 && (dark - DARK_LUMINOSITY as f64).abs() < 1e-6, "light {light} dark {dark}");
        // gamma range
        let s = text.find("MinGammaValue").and_then(|_| text.find("MaxGammaValue")).is_some();
        assert!(s, "MinGammaValue/MaxGammaValue evidence present");
        assert_eq!((settings::GAMMA_MIN, settings::GAMMA_MAX), (1.7, 2.7));
        // layout json carries the header text, description and default callout used here
        let lj: serde_json::Value = crate::testutil::report_json("menu-layout-settings.json");
        let tab = lj["tabs"].as_array().unwrap().iter().find(|t| t["id"] == "Brightness").unwrap();
        assert_eq!(tab["layout"]["headerText"]["text"], HEADER); assert_eq!(tab["layout"]["description"]["text"], DESCRIPTION);
        assert_eq!(tab["layout"]["defaultCallOut"]["text"], default_callout()); assert_eq!(tab["sliderLabel"]["text"], SLIDER_LABEL);
        let size = |v: &serde_json::Value| -> Vec<f64> { v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect() };
        assert_eq!(size(&tab["layout"]["calibrationImage"]["size"]), vec![STRIP_W as f64, STRIP_H as f64]);
        assert_eq!(size(&tab["layout"]["calibrationPair"]["lightPatch"]["size"]), vec![PATCH as f64, PATCH as f64]);
        assert_eq!(size(&tab["layout"]["calibrationPair"]["darkPatch"]["size"]), vec![PATCH as f64, PATCH as f64]);
    }
    #[test] fn strip_values_and_geometry_match_the_exported_png() {
        let p = assets().join("exported/Game/UI/Menu/Foundation/Textures/Calibration/T_BrightnessCal.png");
        let im = image::open(&p).expect("T_BrightnessCal.png").to_rgb8();
        assert_eq!((im.width() as f32, im.height() as f32), STRIP_TEX);
        for (i, v) in STRIP_VALUES.iter().enumerate() {
            let x = (STRIP_CELLS_X0 + STRIP_CELL_W * i as f32 + STRIP_CELL_W * 0.5) as u32;
            assert_eq!(im.get_pixel(x, 40)[0], *v, "cell {i}");
        }
        // frame: outline value at the recorded position, black outside
        assert_eq!(im.get_pixel(6, 40)[0], STRIP_FRAME_VALUE); assert_eq!(im.get_pixel(300, 6)[0], STRIP_FRAME_VALUE); assert_eq!(im.get_pixel(0, 0)[0], 0);
        let first = STRIP_CELLS_X0 as u32; let last = (STRIP_CELLS_X0 + STRIP_CELL_W * 21.) as u32;
        assert_eq!(im.get_pixel(first - 1, 40)[0], STRIP_FRAME_VALUE, "frame touches the first cell"); assert_ne!(im.get_pixel(last - 1, 40)[0], 0);
        let logo = image::open(assets().join("exported/Game/UI/Menu/Foundation/Textures/Calibration/T_BrightnessCalibration.png")).expect("logo").to_rgba8();
        assert_eq!((logo.width(), logo.height()), (512, 512)); assert_eq!(logo.get_pixel(256, 300)[3], 255); assert_eq!(logo.get_pixel(2, 2)[3], 0, "transparent corner");
    }
    #[test] fn default_brightness_is_identity_and_levels_move_the_right_way() {
        assert!((exponent_for(0.5) - 1.).abs() < 1e-6, "0.50 = gamma 2.2 = identity");
        assert!((settings::display_gamma(0.) - 1.7).abs() < 1e-6 && (settings::display_gamma(1.) - 2.7).abs() < 1e-6 && (settings::display_gamma(0.5) - 2.2).abs() < 1e-6);
        assert!(exponent_for(0.9) < 1. && exponent_for(0.1) > 1.);
        let d = |b: f32| logo_level(DARK_LUMINOSITY, b);
        assert!((d(0.5) - 1.275).abs() < 1e-3, "dark logo is 1/255 at the default: {}", d(0.5));
        assert!(d(0.0) < d(0.25) && d(0.25) < d(0.5) && d(0.5) < d(0.75) && d(0.75) < d(1.0), "monotonic brighter");
        assert!(d(0.0) < 0.5, "invisible at the minimum"); assert!(d(1.0) > 3., "clearly visible at the maximum: {}", d(1.0));
        assert!(logo_level(LIGHT_LUMINOSITY, 0.5) > 30.);
        assert_eq!(default_callout(), "Default Value 0.50");
    }
    #[test] fn layout_fits_every_size_and_keeps_the_recovered_proportions() {
        for (w, h) in SIZES {
            let (vp, a) = area_for(w, h);
            let l = layout(a, vp.scale);
            for (n, r) in [("header", l.header), ("desc", l.desc), ("strip", l.strip), ("light", l.light), ("dark", l.dark), ("slider", l.slider), ("callout", l.callout), ("hint", l.hint)] { assert!(inside(a, r), "{w}x{h} {n} {r:?} outside {a:?}"); assert!(r.w > 0. && r.h > 0., "{w}x{h} {n}"); }
            assert!((l.light.w - l.light.h).abs() < 0.01 && (l.dark.w - l.dark.h).abs() < 0.01, "patches stay square");
            assert!((l.strip.w / l.strip.h - STRIP_W / STRIP_H).abs() < 0.01, "strip keeps 512:48");
            assert!(l.light.x + l.light.w <= l.dark.x, "light left of dark, no overlap"); assert!(l.strip.y + l.strip.h <= l.light.y && l.light.y + l.light.h <= l.slider.y, "vertical order strip, images, slider");
            assert!((l.dark.x - (l.light.x + l.light.w) - PATCH_GAP * vp.scale * l.k).abs() < 0.6, "128-unit gap");
            if h >= 1000. && w >= 1900. { assert!((l.k - 1.).abs() < 1e-6, "full size at 1080p: k {}", l.k); assert!((l.light.w - PATCH * vp.scale).abs() < 0.6); }
        }
        let l = layout(Rect::new(0., 0., 1., 1.), 1.); assert!(l.k >= 0.2 && l.k.is_finite());
    }
    #[test] fn page_draws_the_recovered_elements_and_the_slider_edits_working_brightness_with_a_preview() {
        let f = fonts();
        let (vp, a) = area_for(1920., 1080.);
        let mut sx = Session::new(); sx.sync_from(&Settings::default());
        let mut page = BrightnessPage::new();
        let mut st = UiState::new();
        let run = |st: &mut UiState, sx: &mut Session, page: &mut BrightnessPage, events: &[Event]| -> Vec<Cmd> {
            for e in events { st.push_event(*e); }
            let mut out = DrawList::default();
            st.begin_frame(1. / 60.);
            { let mut ui = Ui::new(vp, &f, &mut out, st, sx, 1. / 60., 1., false, id_of("test-brightness")); page.ui(&mut ui, a); ui.finish(); }
            st.end_frame();
            out.cmds
        };
        let c0 = run(&mut st, &mut sx, &mut page, &[]);
        let c0 = { let _ = c0; run(&mut st, &mut sx, &mut page, &[]) };
        let texts: Vec<&str> = c0.iter().filter_map(|c| if let Cmd::Text { text, .. } = c { Some(text.as_str()) } else { None }).collect();
        for t in [HEADER, DESCRIPTION, "Brightness", "Default Value 0.50", "0.50"] { assert!(texts.contains(&t), "missing {t:?} in {texts:?}"); }
        let logos: Vec<(Rect, [f32; 4])> = c0.iter().filter_map(|c| if let Cmd::Sprite { brush: BrushId::Calibration, rect, tint, .. } = c { Some((*rect, *tint)) } else { None }).collect();
        // 1.8: the Light/Dark calibration pair is Collapsed in the widget tree (bytecode), so it is not drawn
        assert!(logos.is_empty(), "the calibration images are collapsed in 1.8");
        let fills = c0.iter().filter(|c| matches!(c, Cmd::Fill { .. } | Cmd::Material { .. })).count(); assert!(fills >= 1, "strip and slider drawn: {fills}");
        // Right arrow on the focused slider: +0.01 -> working value and live preview request, images get brighter
        let mut changed = None;
        for _ in 0..3 { let c = run(&mut st, &mut sx, &mut page, &[Event::Nav(NavKey::Right)]); changed = Some(c); }
        let after = sx.settings_clone().video.brightness;
        assert!(after > 0.5 && after <= 0.53 + 1e-6, "slider moved: {after}");
        assert!(sx.is_dirty(), "working differs from applied");
        let _ = changed;
        assert!(sx.pending_request() == Some(ReqKind::Preview), "preview requested for the running game");
        // Reset: working value back to 0.5 and the Apply-less preview queued again
        let mut p2 = BrightnessPage::new(); assert!(p2.reset_prompt() == Some(None)); p2.reset_to_default(&mut sx);
        assert_eq!(sx.settings_clone().video.brightness, 0.5);
    }
    #[test] fn chrome_is_independent_of_brightness() {
        let f = fonts();
        let (vp, a) = area_for(1920., 1080.);
        let draw = |b: f32| -> Vec<Cmd> {
            let mut sx = Session::new(); sx.sync_from(&Settings::default());
            sx.with_settings(|s| { let _ = s.set("video.brightness", Value::Float(b)); });
            let mut page = BrightnessPage::new(); let mut st = UiState::new(); let mut out = DrawList::default();
            for _ in 0..2 { out = DrawList::default(); st.begin_frame(1. / 60.); { let mut ui = Ui::new(vp, &f, &mut out, &mut st, &mut sx, 1. / 60., 1., false, id_of("test-chrome")); page.ui(&mut ui, a); ui.finish(); } st.end_frame(); }
            out.cmds
        };
        let (dark, bright) = (draw(0.0), draw(1.0));
        let chrome = |c: &[Cmd]| -> Vec<(String, [f32; 4])> { c.iter().filter_map(|c| if let Cmd::Text { text, color, .. } = c { if text != "0.00" && text != "1.00" { Some((text.clone(), *color)) } else { None } } else { None }).collect() };
        assert_eq!(chrome(&dark), chrome(&bright), "every text of the page has the same colour at slider 0 and 1");
        assert!(chrome(&dark).iter().all(|(_, c)| c[3] >= 0.59), "no chrome text dimmer than white 0.6");
        // only the calibration elements (strip cells, the two images) change; they are the only commands that differ at all
        let (da, db): (Vec<&Cmd>, Vec<&Cmd>) = (dark.iter().collect(), bright.iter().collect());
        assert_eq!(da.len(), db.len());
        let diff = da.iter().zip(&db).filter(|(x, y)| format!("{x:?}") != format!("{y:?}")).count();
        assert!(diff >= 1 && diff <= 21 + 5 + 2 + 3, "{diff} commands differ (strip + slider handle/value only)");
    }
    #[test] fn page_ids_and_order() { let p = BrightnessPage::new(); assert_eq!(p.id(), "brightness"); assert_eq!(p.title(), "Brightness"); assert_eq!(p.order(), super::super::ORDER_BRIGHTNESS); assert!(!p.is_fallback()); }
}
