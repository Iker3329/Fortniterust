//! Settings screen core: generic descriptor-driven tabs, the settings session and the Apply / Reset / confirm flow.
//!
//! # STABLE API for page authors (pages REPLACE the generic ones by id)
//! ```text
//! // 1. Register a page into the MenuSystem (ids: "video" "game" "brightness" "audio" "input" "controller" "livestream", order 10,20,..,70).
//! //    A non-fallback page with the same id replaces the generic page in either registration order (`SettingsPage::is_fallback`).
//! pages::register_all_into(&mut menu);                        // the standard set
//! menu.register_settings_page(Box::new(MyPage::new()));       // or your own
//!
//! // 2. Inside `SettingsPage::ui(&mut self, ui, area)` read/modify the WORKING copy through the session on `ui` (nothing reaches the game until Apply):
//! ui.session.with_settings(|s: &mut Settings| s.set("video.vsync", Value::Bool(true)));
//! let snap: Settings = ui.session.settings_clone();           // cheap per-frame snapshot for drawing
//! ui.session.mark_dirty();                                    // only for page-private state; Settings edits are detected by diff
//! ui.session.is_dirty(); ui.session.changed_ids();
//!
//! // 3. Apply (same as pressing the footer button: confirmation dialog for risky video changes, atomic save):
//! ui.session.request_apply();
//! ui.session.request_preview();                               // apply live WITHOUT saving (e.g. while dragging Brightness)
//! ui.session.discard_changes();
//!
//! // 4. Reset To Default (footer button): implement SettingsPage::reset_prompt()/reset_to_default(sx) on your page
//! //    (reset_prompt() -> Some(None) immediate, Some(Some("Reset ...?")) with confirmation, None = disabled).
//! //    Helpers: pages::reset_row, pages::RowDef/row_info describe the generic rows.
//!
//! // 5. Dialogs: ui.session.push_screen(Box<dyn Screen>) (pushed by the menu after the current frame); pages::dialogs::Modal.
//! // 6. Status line: ui.session.set_status("text", StatusKind::Ok).
//! // 7. Display capabilities: ui.session.display_sizes() / current_display_mode() (filled from the host's `PlatformHost`).
//! ```
//! There are no globals: the [`Session`] is owned by the `MenuSystem` and reaches pages through `Ui::session`.
//!
//! # Host side (fn-app, fn-ui-demo)
//! Once per frame: `menu.frame_with(&mut UiContext::new(&mut game_settings, &mut platform, &mut backend), w, h, dt)`. The first frame after the
//! menu opens copies the game's settings and the platform's display list into the session; at the end of every frame the queued
//! Apply/Preview/Save request runs against the `UiContext` (`Session::pump`): working values are copied into `game_settings`, the
//! [`ApplyBackend`] runs the hooks and writes the ini. Without a game (splash/lobby):
//! `menu.session_mut().attach_standalone(Some(settings::default_path()))`; Apply then just saves the file.
//!
//! # Provenance
//! RECOVERED: tab order (Video, Game[Game, HUD], Brightness, Audio, Input, Controller, Live Streaming), row order, labels, hover
//! texts, visibility gates, "Apply" / "Reset To Default" / "Back" names, the three confirmation texts (`layout_table.rs`).
//! ASSUMED: the widget look (toolkit), the 15 s revert timeout, the unsaved-changes prompt, the widget->setting mapping, and
//! that Input/Controller rows come from the settings catalog (the original key list is natively built from encrypted data).
pub mod brightness;
pub mod bridge;
pub mod controller;
pub mod dialogs;
pub mod footer;
pub mod input;
pub mod layout_table;
pub mod rows;
pub mod scenes;
pub mod screen18;
pub mod slate;
pub use crate::material::uimat;
pub mod video;
pub mod visibility;

pub use bridge::*;
#[allow(unused_imports)]
pub use rows::{extra_keys, hud_set, hud_tags, reset_row, row_info, Bind, GenericPage, Gate, PageState, RowDef, RowInfo, SubTab, TabDef, Wid};

pub const ORDER_VIDEO: i32 = 10;
pub const ORDER_GAME: i32 = 20;
pub const ORDER_BRIGHTNESS: i32 = 30;
pub const ORDER_AUDIO: i32 = 40;
pub const ORDER_INPUT: i32 = 50;
pub const ORDER_CONTROLLER: i32 = 60;
pub const ORDER_LIVESTREAM: i32 = 70;

/// Only the generic descriptor-built pages (all seven tabs).
pub fn generic_pages() -> Vec<Box<dyn crate::menu::SettingsPage>> {
    rows::generic_tabs().into_iter().map(|t| Box::new(GenericPage::new(t)) as Box<dyn crate::menu::SettingsPage>).collect()
}
/// Every Settings page (generic fallbacks + the specialised Input / Controller / Video / Brightness pages). Register them into a `MenuSystem` with
/// `MenuSystem::register_settings_page` (specialised pages replace the generic ones by id).
pub fn all_pages() -> Vec<Box<dyn crate::menu::SettingsPage>> {
    let mut v = generic_pages();
    v.push(Box::new(input::InputPage::new()));
    v.push(Box::new(controller::ControllerPage::new()));
    v.push(Box::new(video::VideoPage::new()));
    v.push(Box::new(brightness::BrightnessPage::new()));
    v
}
/// Register [`all_pages`] directly into `m`.
pub fn register_all_into(m: &mut crate::menu::MenuSystem) { for p in all_pages() { m.register_settings_page(p); } }
/// Register only the generic pages into `m`.
pub fn register_generic_into(m: &mut crate::menu::MenuSystem) { for p in generic_pages() { m.register_settings_page(p); } }

#[cfg(test)]
mod tests;
#[cfg(test)]
mod layout_tests;
