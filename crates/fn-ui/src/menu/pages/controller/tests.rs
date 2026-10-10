//! Controller tab tests (headless page rig; hardware polling itself is covered in inputdev.rs and by the live checks).
use super::*;
use crate::layout::Viewport;
use crate::settings::{self, Settings};

fn fonts() -> Fonts { crate::testutil::fonts() }
struct Rig { st: UiState, sx: Session, draw: DrawList, fonts: Fonts, w: f32, h: f32, page: ControllerPage }
impl Rig {
    fn new(w: f32, h: f32) -> Self {
        let mut sx = Session::new(); sx.sync_from(&Settings::default());
        let mut r = Rig { st: UiState::new(), sx, draw: DrawList::default(), fonts: fonts(), w, h, page: ControllerPage::new() };
        r.frames(2); r
    }
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.draw.clear(); self.st.begin_frame(1. / 60.);
            {
                let vp = Viewport::new(self.w, self.h);
                let mut ui = Ui::new(vp, &self.fonts, &mut self.draw, &mut self.st, &mut self.sx, 1. / 60., 1., false, id_of("rig"));
                ui.push_id("controller");
                let a = Rect::new(self.w * 0.04, self.h * 0.12, self.w * 0.92, self.h * 0.72); ui.push_clip(a);
                self.page.ui(&mut ui, a);
                ui.pop_clip(); ui.pop_id(); ui.finish();
            }
            self.st.end_frame();
        }
    }
    fn scroll(&mut self, units: f32) { self.frames(1); for s in self.st.scroll.values_mut() { s.offset = units; } self.frames(1); }
    fn find(&self, text: &str) -> Option<(f32, f32)> {
        self.draw.cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text: t, font, px, .. } = c { if t == text { Some((x + self.fonts.measure(*font, t, *px as f32) * 0.5, top + box_h * 0.5)) } else { None } } else { None })
    }
    fn texts(&self) -> String { self.draw.text_strings().join(" | ") }
}

#[test] fn registers_as_controller_and_replaces_the_generic_tab() {
    let p = ControllerPage::new();
    assert_eq!((p.id(), p.title(), p.order()), ("controller", "Controller", 60)); assert!(!p.is_fallback());
}
#[test] fn lists_the_two_1_8_rotator_rows_and_nothing_invented() {
    // 1.8 Controller tab = two TextRotator rows (Configuration, Controller Platform) over the XB1/PS4 diagram; the sensitivity, vibration and
    // invert options live on the Game tab in 1.8 (see the Game tab tests / settings catalog).
    let r = Rig::new(1920., 1080.);
    let all = r.texts();
    for want in ["Configuration", "Controller Platform", "XBox One"] { assert!(all.contains(want), "{want} missing: {all}"); }
    for bad in ["Not available", "NOT IMPLEMENTED", "XInput", "runtime", "native", "pak", "fixed pad mapping"] { assert!(!all.contains(bad), "developer text {bad:?} on the Controller tab: {all}"); }
    for gone in ["Controller Sensitivity X", "Vibration"] { assert!(!all.contains(gone), "{gone} is not a Controller tab row in 1.8: {all}"); }
}
#[test] fn controller_sensitivity_options_are_game_tab_sliders_in_1_8() {
    // The 1.8 sliders are bare AnalogSliders (0..1, step 0.01); the controller sensitivity slider is (sens - 1) / 9 (@142f0a300) with sens 5
    // (UFortClientSettingsRecord ctor @140539c00), so it defaults to 4/9 (shown 0.44).
    let mut sx = Session::new(); sx.sync_from(&Settings::default());
    for id in ["input.controller_sensitivity_x", "input.controller_sensitivity_y"] {
        let e = settings::entry(id).unwrap();
        assert!(matches!(e.kind, settings::KindDef::Slider { min, max, .. } if min == 0. && (max - 1.).abs() < 1e-6), "{id}: 1.8 range 0..1");
    }
    assert!((sx.settings_clone().input.controller_sensitivity_x - 4. / 9.).abs() < 1e-6, "1.8 default");
    let i = Settings::default().input; assert_eq!((i.controller_ads_multiplier, i.controller_scope_multiplier), (1., 1.), "GamepadTargetingSlow / GamepadScopedSlow 1.0 (@140539c00)");
    assert!(tab_reset_ids().is_empty(), "no settings entry belongs to the Controller tab any more, and 1.8 has no Reset To Default there");
}
#[test] fn configuration_rotator_cycles_the_three_athena_presets_and_deadzones_default_to_the_cvars() {
    let mut r = Rig::new(1920., 1080.);
    let all = r.texts();
    for want in ["Standard", "Left Stick Deadzone", "Right Stick Deadzone", "0.24", "0.27"] { assert!(all.contains(want), "{want} missing: {all}"); }
    for (i, want) in ["Quick Builder", "Combat Pro", "Standard"].iter().enumerate() {
        r.sx.with_settings(|s| settings::extras::set_configuration(s, (i + 1) % 3));
        r.frames(2);
        assert!(r.texts().contains(want), "{want}: {}", r.texts());
    }
    r.sx.with_settings(|s| { settings::extras::set_configuration(s, 2); settings::extras::set_deadzone(s, settings::extras::CONTROLLER_LEFT_DEADZONE, 0.5); });
    r.page.reset_to_default(&mut r.sx);
    let s = r.sx.settings_clone();
    assert_eq!((settings::extras::configuration(&s), settings::extras::deadzones(&s)), (0, (0.24, 0.27)));
}
#[test] fn toggle_mode_switches_between_combat_and_building_labels() {
    let mut r = Rig::new(1920., 1080.);
    let before = r.texts();
    r.page.toggle_mode(); r.frames(2);
    assert_ne!(before, r.texts(), "the diagram's mode text changes");
}
#[test] fn readout_formats_a_state() {
    use crate::inputdev::pad::*;
    let t = pad_readout(&State { buttons: A | L3, rt: 200, lx: 32767, ry: -32768, ..Default::default() });
    assert!(t.contains("LS +1.00 +0.00") && t.contains("RS +0.00 -1.00") && t.contains("RT 200") && t.contains("A L3"), "{t}");
    assert!(pad_readout(&State::default()).contains("no buttons"));
}
#[test] fn renders_at_every_size_inside_the_client() {
    for (w, h) in [(1920., 1080.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.), (320., 200.), (1., 1.)] {
        let mut r = Rig::new(w, h);
        let fits = Viewport::new(w, h).slate_size().0 >= 1280.; // fixed 1.8 layout: only required not to panic when narrower
        for off in [0., 500., 1200., 9000.] {
            r.scroll(off);
            for c in &r.draw.cmds { if let Cmd::Text { x, text, font, px, .. } = c { let tw = r.fonts.measure(*font, text, *px as f32); assert!(!fits || (*x >= -1. && x + tw <= w + 1.), "{w}x{h}: {text:?} x {x} w {tw}"); } }
        }
    }
}
