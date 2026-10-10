//! Battle Royale-only front-end picker recovered from the 1.8 Athena screen.
//!
//! The local runtime deliberately contains no campaign page or campaign art:
//! choosing BATTLE ROYALE is the only game route, and the lobby follows it.
use crate::draw::{Cmd, Quad, RenderList, TextureId};
use crate::input::{pad_to_nav, Event, MouseButton, NavKey};
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::{art, put_text, Snap};
use crate::text::FontId;

pub const REF_W: f32 = 1920.;
pub const REF_H: f32 = 1080.;
pub const PAD_X: f32 = 100.;
pub const PAD_Y: f32 = 70.;
pub const LOGO_MAIN: (f32, f32) = (768., 240.);
pub const HEADER_PX: f32 = 32.;
pub const BASE_PX: f32 = 18.;
pub const BR_ART: &str = "T_BattleRoyaleKeyArt.png";
pub const LOGO_FILE: &str = "T-Logo-Wordmark-1024.png";
/// Only Athena front-end images are decoded for this screen.
pub const PAK_TEXTURES: [&str; 2] = [BR_ART, LOGO_FILE];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// FRONT END: `Campaign` = the SAVE THE WORLD button of the 1.8 picker (shown 1:1, inert: this port runs Battle Royale only).
pub enum Item { Campaign, BattleRoyale, Quit }
impl Item { pub fn label(self) -> &'static str { match self { Self::Campaign => "SAVE THE WORLD", Self::BattleRoyale => "BATTLE ROYALE", Self::Quit => "QUIT" } } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page { Main }
impl Page { pub fn items(self) -> &'static [Item] { &[Item::Campaign, Item::BattleRoyale, Item::Quit] } }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome { None, StartBattleRoyale, Quit }

#[derive(Clone, Debug, PartialEq)]
pub struct Layout { pub scale: f32, pub bg: Rect, pub logo: Rect, pub buttons: Vec<(Item, Rect)>, pub hint_baseline: f32, pub header_px: u32, pub base_px: u32 }
impl Layout {
    pub fn rect(&self, item: Item) -> Option<Rect> { self.buttons.iter().find(|(i, _)| *i == item).map(|(_, r)| *r) }
    pub fn hit(&self, x: f32, y: f32) -> Option<Item> { self.buttons.iter().find(|(_, r)| r.contains(x, y)).map(|(i, _)| *i) }
}
fn button_w(text_w: f32, s: f32) -> f32 { (text_w + (18. * 2. + 28.) * s).max(128. * s) }
pub fn layout(_page: Page, w: f32, h: f32, text_w: &dyn Fn(&str, f32) -> f32) -> Layout {
    let scale = dpi_scale(w, h);
    let fill = (w / REF_W).max(h / REF_H);
    let bg = Rect::new((w - REF_W * fill) * 0.5, (h - REF_H * fill) * 0.5, REF_W * fill, REF_H * fill);
    let header_px = (HEADER_PX * scale).round().max(8.) as u32;
    let base_px = (BASE_PX * scale).round().max(8.) as u32;
    let logo = Rect::new(PAD_X * scale, PAD_Y * scale + 100. * scale, LOGO_MAIN.0 * scale, LOGO_MAIN.1 * scale);
    let mut y = logo.bottom() + 30. * scale;
    let mut buttons = Vec::new();
    for &item in Page::Main.items() {
        let r = Rect::new(PAD_X * scale + 16. * scale, y, button_w(text_w(item.label(), header_px as f32), scale), 56. * scale);
        y += 66. * scale;
        buttons.push((item, r));
    }
    Layout { scale, bg, logo, buttons, hint_baseline: h - 18. * scale, header_px, base_px }
}

/// `SwitchMaterial`: `bGameMode` 0 -> 1 over its 0.25 s playback range.
const SWITCH_RATE: f32 = 4.;
#[derive(Clone, Debug)]
pub struct Picker {
    pub page: Page,
    pub focus: Item,
    pub pressed: Option<Item>,
    /// Kept at zero for capture compatibility: there is one Athena key-art layer.
    pub art_mix: f32,
    pub t: f32,
    /// FRONT END: a button was hovered / navigated to (`OnActivated` only centres `CampaignBtn` for the gamepad: no button shows hovered before that)
    pub highlighted: bool,
    /// `SetDescriptionForSubGame`: 0 Save the World, 1 Battle Royale, 2 = "Exit Fortnite." (the `ButtonExit` hover)
    pub description: u8,
    /// `M_SGS-Back` `bGameMode` (the `SwitchMaterial` animation: 0 -> 1 in 0.25 s on the Battle Royale hover, reversed on the Save the World hover)
    pub game_mode: f32,
    pub game_mode_target: f32,
}
impl Default for Picker { fn default() -> Self { Self::new() } }
impl Picker {
    /// `OnActivated`: `CenterWidget(CampaignBtn)`, `SetDescriptionForSubGame(0)`; `bGameMode` 0.
    /// Starts on BATTLE ROYALE with its art (bGameMode 1): Save the World is removed from the picker (user choice, 2026-10-07).
    pub fn new() -> Self { Self { page: Page::Main, focus: Item::BattleRoyale, pressed: None, art_mix: 0., t: 0., highlighted: false, description: 1, game_mode: 1., game_mode_target: 1. } }
    /// The hover / navigation handlers of `SubgameSelectScreen` (`CampaignBtn`: description 0 + `SwitchMaterial` reverse; `AthenaBtn`: description 1 + forward;
    /// `ButtonExit`: "Exit Fortnite.").
    pub fn set_focus(&mut self, item: Item) {
        self.focus = item; self.highlighted = true;
        match item { Item::Campaign => { self.description = 0; self.game_mode_target = 0.; } Item::BattleRoyale => { self.description = 1; self.game_mode_target = 1.; } Item::Quit => self.description = 2 }
    }
    /// A BR-only picker has no secondary page to dismiss.
    pub fn back(&mut self) -> bool { false }
    pub fn tick(&mut self, dt: f32) {
        self.t += dt.max(0.);
        let d = SWITCH_RATE * dt.max(0.);
        self.game_mode = if self.game_mode < self.game_mode_target { (self.game_mode + d).min(self.game_mode_target) } else { (self.game_mode - d).max(self.game_mode_target) };
    }
    fn step(&mut self, delta: i32) { let items = self.page.items(); let at = items.iter().position(|item| *item == self.focus).unwrap_or(0) as i32; self.set_focus(items[(at + delta).rem_euclid(items.len() as i32) as usize]); }
    /// `CampaignBtn` clicked would `SafeSetSubGame(0)` (Save the World): inert here.
    pub fn activate(&mut self, item: Item) -> Outcome { match item { Item::Campaign => Outcome::None, Item::BattleRoyale => Outcome::StartBattleRoyale, Item::Quit => Outcome::Quit } }
    pub fn event(&mut self, event: Event, layout: &Layout) -> Outcome {
        match event {
            Event::MouseMove(x, y) => { if let Some(item) = layout.hit(x, y) { if !self.highlighted || self.focus != item { self.set_focus(item); } } Outcome::None }
            Event::MouseDown(MouseButton::Left, x, y) => { self.pressed = layout.hit(x, y); if let Some(item) = self.pressed { self.set_focus(item); } Outcome::None }
            Event::MouseUp(MouseButton::Left, x, y) => { let (down, up) = (self.pressed.take(), layout.hit(x, y)); if down.is_some() && down == up { self.activate(up.unwrap()) } else { Outcome::None } }
            Event::MouseLeave | Event::FocusLost => { self.pressed = None; Outcome::None }
            Event::Nav(key) => self.nav(key, layout), Event::Pad(button) => self.nav(pad_to_nav(button), layout), _ => Outcome::None,
        }
    }
    /// FRONT END: a button the layout does not show (zero size: QUIT, collapsed on PC) is skipped.
    fn nav(&mut self, key: NavKey, layout: &Layout) -> Outcome {
        let shown = |l: &Layout, i: Item| l.rect(i).map_or(true, |r| r.w > 0. && r.h > 0.);
        let d = match key { NavKey::Up | NavKey::Left => -1, NavKey::Down | NavKey::Right => 1, _ => 0 };
        if d != 0 { for _ in 0..self.page.items().len() { self.step(d); if shown(layout, self.focus) { break; } } return Outcome::None; }
        match key { NavKey::Activate => self.activate(self.focus), NavKey::Back => Outcome::None, _ => Outcome::None } }
    pub fn build(&self, ctx: &mut UiContext, w: f32, h: f32, art_ready: bool, out: &mut RenderList) {
        let fonts = ctx.fonts.clone(); let l = layout(self.page, w, h, &|text, px| fonts.measure(FontId::Title, text, px));
        out.fill(Rect::new(0., 0., w, h), [0.02, 0.04, 0.12, 1.]);
        if art_ready { out.sprite(art(ctx, BR_ART).id, l.bg, [1., 1., 1., 1.]); }
        let bar_w = w * 0.40;
        for i in 0..24 { let a = i as f32 / 24.; out.fill(Rect::new(a * bar_w, 0., bar_w / 24. + 1., h), [0.01, 0.03, 0.10, 0.62 * (1. - a).powf(1.6)]); }
        out.sprite(art(ctx, LOGO_FILE).id, l.logo, [1., 1., 1., 1.]);
        for (item, rect) in &l.buttons { self.button(ctx, out, &l, *item, *rect); }
        put_text(ctx, out, FontId::Regular, l.base_px.max(11) * 3 / 4 + 4, PAD_X * l.scale + 16. * l.scale, l.hint_baseline, "ARROWS / MOUSE: CHOOSE    ENTER / CLICK: SELECT    ESC: EXIT", [1., 1., 1., 0.55], None, Snap::Pen);
    }
    fn button(&self, ctx: &mut UiContext, out: &mut RenderList, layout: &Layout, item: Item, rect: Rect) {
        let k = 0.30 * rect.h; let focused = self.focus == item; let pressed = self.pressed == Some(item);
        let (fill, text) = if pressed { ([0.72, 0.84, 1., 1.], [0., 0.12, 0.45, 1.]) } else if focused { ([1., 1., 1., 0.97], [0., 0.17, 0.62, 1.]) } else { ([0.04, 0.09, 0.25, 0.82], [1., 1., 1., 1.]) };
        if !focused { shear(out, rect.inflate(layout.scale), k, [1., 1., 1., 0.18]); } shear(out, rect, k, fill);
        put_text(ctx, out, FontId::Title, layout.header_px, rect.x + 32. * layout.scale, rect.y + (rect.h + layout.header_px as f32 * 0.62) * 0.5, item.label(), text, None, Snap::Pen);
    }
}
fn shear(out: &mut RenderList, r: Rect, k: f32, tint: [f32; 4]) { out.cmds.push(Cmd::Quad(Quad { tex: TextureId::WHITE, pos: [[r.x + k, r.y], [r.right(), r.y], [r.right() - k, r.bottom()], [r.x, r.bottom()]], uv: [0., 0., 1., 1.], tint })); }

#[cfg(test)]
mod tests {
    use super::*;
    fn tw(t: &str, px: f32) -> f32 { t.len() as f32 * px * 0.5 }
    #[test] fn br_only_buttons_fit_and_stack() { for (w, h) in [(1280., 720.), (1920., 1080.), (640., 360.)] { let l = layout(Page::Main, w, h, &tw); assert_eq!(l.buttons.iter().map(|x| x.0).collect::<Vec<_>>(), vec![Item::Campaign, Item::BattleRoyale, Item::Quit]); assert!(l.buttons.iter().all(|(_, r)| r.x >= 0. && r.right() <= w && r.y >= 0. && r.bottom() <= h)); } }
    #[test] fn navigation_and_activation_are_br_only() {
        let l = layout(Page::Main, 1280., 720., &tw); let mut p = Picker::new();
        assert_eq!((p.focus, p.highlighted, p.description, p.game_mode), (Item::BattleRoyale, false, 1, 1.));
        assert_eq!(p.activate(Item::Campaign), Outcome::None, "Save the World is inert");
        assert_eq!(p.event(Event::Nav(NavKey::Activate), &l), Outcome::StartBattleRoyale);
        p.event(Event::Nav(NavKey::Down), &l); assert_eq!((p.focus, p.description, p.game_mode_target), (Item::Quit, 2, 1.));
        assert_eq!(p.event(Event::Nav(NavKey::Activate), &l), Outcome::Quit);
    }
}
