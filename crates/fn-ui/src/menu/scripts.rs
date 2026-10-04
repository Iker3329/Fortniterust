//! Scripted menu states: the old `--menu-test` driver (`menu::host::test_scene`) as pure functions over an explicit [`MenuSystem`] (no process-global
//! host, no window). Events are interleaved with headless frames so hit regions exist; the caller then runs one more frame (no new input) and lowers.
//!
//! DESIGN NOTE (kept from the old host): in the game, opening the menu PAUSES the simulation; real Fortnite is online and does not. Cursor contract:
//! menu open = cursor free; on `HostRequest::Closed` the platform re-captures the mouse.
use super::*;

pub const TEST_SCENES: [&str; 16] = ["pause", "pause-hover", "pause-focus", "pause-pressed", "pause-disabled-hover", "pause-toast", "pause-leave-hover", "pause-gamepad", "pause-keyboard-column", "confirm", "settings-widgets", "settings-dropdown", "settings-list", "settings-binds", "settings-disabled-hover", "pause-fading"];

/// A fresh menu with every real Settings tab registered (and, optionally, the toolkit gallery demo pages that the old `--menu-test` added).
pub fn new_menu(fonts: Fonts, demo_pages: bool) -> MenuSystem {
    let mut m = MenuSystem::new(fonts);
    pages::register_all_into(&mut m);
    if demo_pages { pause::register_demo_pages(&mut m); }
    m
}

/// Put `m` into a scripted state at a `w`x`h` client (the old `Host::test_scene`).
pub fn test_scene(m: &mut MenuSystem, name: &str, w: f32, h: f32) {
    m.close(); m.take_requests(); m.set_settings_tab(0); m.open();
    // Centre of a MainMenu entry (mouse/keyboard layout; the scripted scenes only hover/click before any pad input).
    let at = |widget: &str| { let r = mainmenu::entry_rect(w, h, mainmenu::MenuContext::Match, widget).expect("scene widget exists in the match menu"); (r.x + r.w * 0.5, r.y + r.h * 0.5) };
    let frame = |m: &mut MenuSystem| { m.frame(w, h, 1. / 60.); };
    let ev = |m: &mut MenuSystem, e: Event| { m.event(e); m.frame(w, h, 1. / 60.); };
    let click = |m: &mut MenuSystem, p: (f32, f32)| { ev(m, Event::MouseMove(p.0, p.1)); ev(m, Event::MouseDown(MouseButton::Left, p.0, p.1)); ev(m, Event::MouseUp(MouseButton::Left, p.0, p.1)); frame(m); };
    let key = |m: &mut MenuSystem, k: NavKey| ev(m, Event::Nav(k));
    m.event(Event::MouseLeave); // the previous scene's pointer must not leak into this one
    for _ in 0..2 { frame(m); }
    let to_settings = |m: &mut MenuSystem| { click(m, at("SettingsButton")); for _ in 0..3 { frame(m); } };
    match name {
        "pause-hover" => { let p = at("SettingsButton"); ev(m, Event::MouseMove(p.0, p.1)); }
        "pause-focus" => key(m, NavKey::Down),
        "pause-pressed" => { let p = at("SettingsButton"); ev(m, Event::MouseMove(p.0, p.1)); ev(m, Event::MouseDown(MouseButton::Left, p.0, p.1)); }
        "pause-disabled-hover" => { let p = at("Feedback"); ev(m, Event::MouseMove(p.0, p.1)); }
        // Click on a disabled entry: the "Not available in this build" toast shows while the hover description stays.
        "pause-toast" => click(m, at("Feedback")),
        "pause-leave-hover" => { let p = at("LeaveButton"); ev(m, Event::MouseMove(p.0, p.1)); }
        // A pad event switches the icon column to its 90-unit gamepad width (Set Icon Button List Column Width).
        "pause-gamepad" => { ev(m, Event::Pad(PadButton::DPadDown)); ev(m, Event::Pad(PadButton::DPadLeft)); }
        "pause-keyboard-column" => { key(m, NavKey::Left); key(m, NavKey::Down); }
        "confirm" => click(m, at("LeaveButton")),
        "settings-widgets" => to_settings(m),
        "settings-dropdown" => { to_settings(m); for _ in 0..7 { key(m, NavKey::Down); } key(m, NavKey::Activate); for _ in 0..3 { frame(m); } }
        "settings-list" => { to_settings(m); key(m, NavKey::TabNext); let (sx, sy) = (w * 0.5, h * 0.55); ev(m, Event::MouseMove(sx, sy)); ev(m, Event::Wheel(-7.)); for _ in 0..3 { frame(m); } }
        "settings-binds" => { to_settings(m); key(m, NavKey::TabNext); key(m, NavKey::TabNext); for _ in 0..3 { key(m, NavKey::Down); } key(m, NavKey::Activate); for _ in 0..3 { frame(m); } }
        "settings-disabled-hover" => { to_settings(m); let p = (w * 0.62, h * 0.3); ev(m, Event::MouseMove(p.0, p.1)); }
        _ => {}
    }
    for _ in 0..2 { frame(m); }
    m.settle();
    if name == "pause-fading" { m.close(); m.take_requests(); m.open(); frame(m); m.set_visibility(0.45); }
}
