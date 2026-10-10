//! `--settings-screen-test` scenes: scripted Settings-screen states (the old runtime captured them over the live frozen game scene).
//! Pure: operates on an explicit [`MenuSystem`]. Uses [`NullHost`] (applies through no-op hooks, writes nothing), never a real window,
//! so no display mode is ever changed.
use super::bridge::NullHost;
use crate::menu::pause::SettingsScreen;
use crate::menu::{Cmd, Event, MenuSystem, MouseButton, NavKey};
use crate::settings::Value;

pub const SCENES: &[&str] = &[
    "video", "video-hover", "video-dropdown", "game", "game-gated", "hud", "brightness", "audio", "input", "controller", "livestream",
    "dirty", "unsaved", "keep-video", "reset-confirm", "applied",
];

fn text_pos(cmds: &[Cmd], label: &str) -> Option<(f32, f32)> {
    cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text, .. } = c { if text == label { Some((*x + 6., *top + *box_h * 0.5)) } else { None } } else { None })
}

/// Put `m` (with the Settings pages registered, see `pages::register_all_into`) into the named Settings-screen state at a `w`x`h` client.
pub fn scene(m: &mut MenuSystem, name: &str, w: f32, h: f32) { scene_with(m, name, w, h, NullHost::new()) }
/// [`scene`] over a given game host (its settings are what the screen syncs from; e.g. a test pinning the values an old capture was taken with).
pub fn scene_with(m: &mut MenuSystem, name: &str, w: f32, h: f32, game: NullHost) {
    {
        m.close(); m.take_requests();
        m.remove_settings_pages_prefixed("gallery.");
        let mut game = game;
        m.session_mut().reset_transients(); // drop what the previous scene's teardown queued; the first frame syncs from `game`
        m.set_settings_tab(0);
        m.open();
        m.push(Box::new(SettingsScreen::new()));
        let frame = |m: &mut MenuSystem, game: &mut NullHost| -> Vec<Cmd> { m.frame_with(&mut game.cx(), w, h, 1. / 60.).cmds.clone() };
        let idx = |m: &MenuSystem, id: &str| m.pages().iter().position(|p| p.id() == id).unwrap_or(0);
        for _ in 0..3 { frame(m, &mut game); }
        let tab = |m: &mut MenuSystem, game: &mut NullHost, id: &str| { let i = idx(m, id); m.set_settings_tab(i); for _ in 0..4 { frame(m, game); } };
        let hover = |m: &mut MenuSystem, game: &mut NullHost, label: &str, dx: f32| {
            let c = frame(m, game);
            if let Some((x, y)) = text_pos(&c, label) { m.event(Event::MouseMove(x + dx, y)); for _ in 0..3 { frame(m, game); } }
        };
        let click = |m: &mut MenuSystem, game: &mut NullHost, label: &str, dx: f32| {
            let c = frame(m, game);
            if let Some((x, y)) = text_pos(&c, label) {
                let (x, y) = (x + dx, y);
                m.event(Event::MouseMove(x, y)); frame(m, game);
                m.event(Event::MouseDown(MouseButton::Left, x, y)); frame(m, game);
                m.event(Event::MouseUp(MouseButton::Left, x, y)); for _ in 0..3 { frame(m, game); }
            }
        };
        match name {
            "video" => tab(m, &mut game, "video"),
            "video-hover" => { tab(m, &mut game, "video"); hover(m, &mut game, "3D Resolution", 40.); }
            "video-dropdown" => { tab(m, &mut game, "video"); click(m, &mut game, "Frame Rate Limit", w * 0.3); }
            "game" => tab(m, &mut game, "game"),
            "game-gated" => {
                tab(m, &mut game, "game");
                m.event(Event::MouseMove(w * 0.5, h * 0.45)); m.event(Event::Wheel(-40.)); for _ in 0..4 { frame(m, &mut game); }
                hover(m, &mut game, "Show Backpack", 30.);
            }
            "hud" => { tab(m, &mut game, "game"); click(m, &mut game, "HUD", 6.); hover(m, &mut game, "Reticle", 30.); }
            "brightness" | "audio" | "input" | "controller" | "livestream" => tab(m, &mut game, name),
            "dirty" => {
                tab(m, &mut game, "video");
                m.session_mut().with_settings(|s| { let _ = s.set("video.show_fps", Value::Bool(true)); let _ = s.set("video.vsync", Value::Bool(false)); });
                for _ in 0..3 { frame(m, &mut game); }
            }
            "unsaved" => {
                tab(m, &mut game, "video");
                m.session_mut().with_settings(|s| { let _ = s.set("video.show_fps", Value::Bool(true)); });
                frame(m, &mut game); m.event(Event::Nav(NavKey::Back)); for _ in 0..3 { frame(m, &mut game); }
            }
            "keep-video" => {
                tab(m, &mut game, "video");
                m.session_mut().with_settings(|s| { let _ = s.set("video.window_mode", Value::Index(1)); });
                m.session_mut().request_apply(); for _ in 0..5 { frame(m, &mut game); }
            }
            "reset-confirm" => { tab(m, &mut game, "game"); click(m, &mut game, "Reset To Default", 20.); }
            "applied" => {
                tab(m, &mut game, "audio");
                m.session_mut().with_settings(|s| { let _ = s.set("audio.music_volume", Value::Float(0.25)); });
                m.session_mut().request_apply(); for _ in 0..4 { frame(m, &mut game); }
            }
            _ => {}
        }
        for _ in 0..2 { frame(m, &mut game); }
        m.settle();
    }
}
