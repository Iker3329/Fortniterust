//! Settings "Controller" tab (Wireless Controller on PS4 in the original).
//!
//! RECOVERED from the 1.8 pak (menu-layout-settings.json, TabGamePadConfig): the tab (`ShowControllerOptions`), the rows
//! "Configuration" (option names are filled natively: NOT recovered), "Controller Platform" with the options "XBox One" and
//! "PlayStation 4" (selects button glyphs), a button-mapping diagram widget (`GamepadMappingInfo`, only partly recovered), and, from
//! the options screens, Controller Sensitivity X/Y (0.1..10, 0 digits), Controller ADS/Scope Sensitivity (0.01..1) and Vibration.
//! There is NO dead-zone, controller-invert or button-remap option in the data: the dead zones are fixed XInput constants and Invert View
//! is the single shared setting (the Input tab row). Nothing but the original rows and the diagram is drawn.
//!
//! IMPLEMENTED (src/inputdev.rs, wired in `App::update`): XInput controller 0 -> move (left stick), look (right stick, Controller
//! Sensitivity X/Y, Invert View), jump (A), crouch (B, Hold/Toggle per Crouch Mode), sprint (left stick click, Toggle Sprint applies),
//! fire / pickaxe swing (right trigger), Start -> pause menu (on release; the menu itself then follows the pad). All of that mapping, the dead zones, the response curve and the degrees-per-second scale are
//! ASSUMED (not 1:1). STORED ONLY: ADS and scope multipliers (no aiming), Vibration (no rumble output), Configuration and Controller
//! Platform (one fixed Xbox-style layout).
use super::bridge;
use crate::inputdev;
use crate::menu::*;
use crate::settings;

/// Short live readout of the pad (only for the status row; pure for tests).
pub fn pad_readout(s: &inputdev::pad::State) -> String {
    use inputdev::pad::*;
    let mut b: Vec<&str> = Vec::new();
    for (m, n) in [(A, "A"), (B, "B"), (X, "X"), (Y, "Y"), (LB, "LB"), (RB, "RB"), (L3, "L3"), (R3, "R3"), (START, "Start"), (BACK, "Back")] { if s.buttons & m != 0 { b.push(n); } }
    let (lx, ly) = radial_deadzone(s.lx, s.ly, LEFT_DEADZONE);
    let (rx, ry) = radial_deadzone(s.rx, s.ry, RIGHT_DEADZONE);
    format!("LS {lx:+.2} {ly:+.2}   RS {rx:+.2} {ry:+.2}   LT {}  RT {}   {}", s.lt, s.rt, if b.is_empty() { "no buttons".to_owned() } else { b.join(" ") })
}

pub struct ControllerPage { desc: Option<()>, platform: usize, combat: bool }
impl ControllerPage {
    pub fn new() -> Self { Self { desc: None, platform: 0, combat: true } }
}

/// Ids restored by the footer "Reset To Default" on this tab (Invert View is shared with the Input tab and is left alone).
pub fn tab_reset_ids() -> Vec<&'static str> { settings::entries().iter().filter(|e| e.tab == "Controller").map(|e| e.id).collect() }

impl SettingsPage for ControllerPage {
    fn id(&self) -> &str { "controller" }
    fn title(&self) -> &str { "Controller" }
    fn order(&self) -> i32 { super::ORDER_CONTROLLER }
    fn on_show(&mut self) { self.desc = None; }
    fn reset_prompt(&self) -> Option<Option<String>> { Some(Some("Reset controller options to default?".to_owned())) }
    fn reset_to_default(&mut self, sx: &mut bridge::Session) { sx.with_settings(|s| for id in tab_reset_ids() { let _ = s.reset(id); }); }
    fn toggle_mode(&mut self) { self.combat = !self.combat; }
    fn ui(&mut self, ui: &mut Ui, area: Rect) {
        use super::slate;
        let s = ui.vp.scale;
        // TabGamePadConfig: VBox[ SizeBox(935){GamepadConfig}, SizeBoxControllerPlatform(935){ControllerPlatformConfig}, GamepadMappingInfo (padding top 30) ] centred
        let rw = 935. * s;
        let rx = area.x + (area.w - rw) * 0.5;
        let mut col = slate::Col::new(rx, area.y, rw, s);
        let (mx, my) = ui.mouse().unwrap_or((-1., -1.));
        // Configuration: the option list is native (GetAllPlayerInputPresetNamesForSubGame): not available in this runtime
        let r1 = col.slot(slate::ROTATOR_H, [0.; 4]);
        let a1 = slate::rotator_row(ui, "cfg", r1, "Configuration", "Default", false);
        // Controller Platform: "XBox One" / "PlayStation(R)4" selects the button pictures (UI state only, nothing is stored)
        let r2 = col.slot(slate::ROTATOR_H, [0.; 4]);
        let names = ["XBox One", "PlayStation\u{00AE}4"];
        let a2 = slate::rotator_row(ui, "plat", r2, "Controller Platform", &sanitize_label(names[self.platform]), true);
        if a2.delta != 0 { self.platform = (self.platform as i32 + a2.delta).rem_euclid(2) as usize; }
        // GamepadMappingInfo: 1024x672 picture + line overlay, centred, 30 below the rows
        let top = col.y + 30. * s;
        let pic = Rect::new(area.x + (area.w - 1024. * s) * 0.5, top, 1024. * s, 672. * s);
        let (img, lines) = if self.platform == 0 { (TexId::Xb1Config, TexId::Xb1Lines) } else { (TexId::Ps4Config, TexId::Ps4Lines) };
        ui.sprite(BrushId::Tex(img), pic, false, [1., 1., 1., 1.]);
        ui.sprite(BrushId::Tex(lines), pic, false, [1., 1., 1., 1.]);
        // mode (Image_Mode 32? + Text_Mode TextStyle ...): "Combat Controls" / "Build Controls"
        let (mt, mi) = if self.combat { ("Combat Controls", TexId::ModeCombat) } else { ("Build Controls", TexId::ModeBuild) };
        let tw = ui.fonts.measure(FontId::Bold, mt, ui.text_px(24.) as f32);
        let ir = Rect::new(pic.x + (pic.w - tw) * 0.5 - 40. * s, pic.y + 4. * s, 32. * s, 32. * s);
        ui.sprite(BrushId::Tex(mi), ir, false, slate::label_color());
        ui.text(Rect::new(ir.x + 40. * s, pic.y + 4. * s, tw + 8. * s, 32. * s), mt, TextStyle::new(FontId::Bold, 24., slate::label_color()), HAlign::Left);
        let _ = (a1, mx, my);
    }
}
fn sanitize_label(s: &str) -> String { s.replace('\u{00AE}', "(R)") }
#[cfg(test)]
mod tests;

