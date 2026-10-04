//! Menu toolkit + in-game pause menu (src/menu).
//!
//! A small *immediate-mode* UI layer on top of `ui_layout.rs` (DPI scale, `Rect`, `Viewport`, nine-slice brushes,
//! premultiplied-alpha batching). Widgets are plain function calls made every frame from `Screen::ui` /
//! `SettingsPage::ui`; they emit a retained-free `DrawList` (renderer-independent, unit-testable without OpenGL)
//! that `render.rs` replays through `UiBatch`.
//!
//! Layout contract: widget rects are PIXELS (build them from Slate units with `ui.u(units)`), like the rest of
//! `ui_layout`. Scroll offsets are stored in Slate units so a DPI change (resize) keeps the scroll position.
//!
//! Interaction model (see `Ui::interact`):
//! * Hit-testing uses the regions submitted LAST frame (topmost = last submitted wins), so overlapping widgets,
//!   clipped scroll content, disabled widgets (they still swallow the click) and modal dialogs (a full-screen
//!   blocker submitted after the screen below) all resolve consistently. Inert (non-top) screens submit nothing.
//! * Mouse events are processed per frame with their own positions (press target = region under the press
//!   point, release target = region under the release point), so a press+release inside one frame still clicks.
//! * Keyboard/controller: Up/Down/Tab (D-pad, stick) move a linear focus order (wraps), Enter/Space/A activate,
//!   Left/Right adjust sliders and selectors, Esc/B go back, Ctrl+Tab / Ctrl+PageUp/Down / LB/RB switch tabs.
//!   Focus is only DRAWN after a keyboard/pad event (mouse movement hides it), like CommonUI.
//! * Esc while a keybind button is listening cancels the capture; while a dropdown is open it closes the list;
//!   only then does it reach the screen's back handler.
//!
//! PROVENANCE: button/tab/checkbox/keybind-frame/close/slider-arrow textures and their nine-slice margins
//! (0.5 on every side), content padding (M 12, L 20 units) and the pressed/hover/disabled mapping come from the
//! original 1.10 `Button-*-SlateWidgetStyleAsset`, `Tab1-M-SlateWidgetStyleAsset`, `Checkbox_Glass` assets
//! (tagged properties read with tools/uasset-props.cjs). The text face is NotoSans (the face every inspected
//! 1.10 widget style references, see hud_extra.rs). APPROXIMATIONS (not recovered): all text sizes, the panel
//! colours/gradients, the slider track (the original is a material, `OptionsSliderFill`), the dim amount and
//! the open/close fade. The DPI curve is the UE4 default (see ui_layout.rs).
#![allow(dead_code)]
pub mod lower;
pub mod mainmenu;
pub mod outline;
pub mod pages;
pub mod pause;
pub mod scene_defs;
pub mod scripts;
pub mod widgets;
pub use scene_defs::scenes;
pub use pages::{ApplyBackend, NullBackend, NullHost, Session, StandardBackend, UiContext};

use crate::layout::{Rect, Viewport};
pub use crate::input::{key_name, pad_to_nav, Event, MouseButton, NavKey, PadButton, PadState};
pub use crate::text::{FontId, Fonts};
use std::collections::HashMap;

pub type Id = u64;

/// FNV-1a 64-bit hash of a widget name.
pub fn id_of(s: &str) -> Id {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); }
    h
}
/// Combine a parent id with a child id (order dependent).
pub fn id_with(parent: Id, child: Id) -> Id { (parent ^ child.wrapping_mul(0x9e3779b97f4a7c15)).rotate_left(23).wrapping_mul(0x100000001b3) ^ child }

// ---- rect helpers ----------------------------------------------------------------------------------------
pub fn rect_contains(r: Rect, x: f32, y: f32) -> bool { r.contains(x, y) }
pub fn rect_intersect(a: Rect, b: Rect) -> Option<Rect> { a.intersect(b) }
pub fn rect_deflate(r: Rect, l: f32, t: f32, rt: f32, b: f32) -> Rect { r.deflate(l, t, rt, b) }
pub fn rect_inflate(r: Rect, d: f32) -> Rect { r.inflate(d) }


/// The atlas only holds printable ASCII; anything else is drawn as '?' (measured identically).
pub fn sanitize(text: &str) -> String { text.chars().map(|c| if (' '..='~').contains(&c) { c } else if c == '\n' || c == '\t' { ' ' } else { '?' }).collect() }

/// Shorten `text` with "..." so it fits `max_w` pixels (empty if even the dots do not fit). Binary search on the
/// prefix length (advance widths are additive).
pub fn fit_text(fonts: &Fonts, font: FontId, text: &str, px: f32, max_w: f32) -> String {
    let text = sanitize(text);
    if fonts.measure(font, &text, px) <= max_w { return text; }
    let chars: Vec<char> = text.chars().collect();
    let dots = fonts.measure(font, "...", px);
    if dots > max_w { return String::new(); }
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let s: String = chars[..mid].iter().collect();
        if fonts.measure(font, s.trim_end(), px) + dots <= max_w { lo = mid; } else { hi = mid - 1; }
    }
    let s: String = chars[..lo].iter().collect();
    format!("{}...", s.trim_end())
}

/// Greedy word wrap to `max_w` pixels (words longer than a line are split by character).
pub fn wrap_text(fonts: &Fonts, font: FontId, text: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for para in text.split('\n') {
        let para = sanitize(para);
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let cand = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
            if fonts.measure(font, &cand, px) <= max_w { cur = cand; continue; }
            if !cur.is_empty() { lines.push(std::mem::take(&mut cur)); }
            let mut piece = String::new();
            for ch in word.chars() {
                let t = format!("{piece}{ch}");
                if fonts.measure(font, &t, px) > max_w && !piece.is_empty() { lines.push(std::mem::take(&mut piece)); piece.push(ch); } else { piece = t; }
            }
            cur = piece;
        }
        lines.push(cur);
    }
    lines
}

/// Result of [`fit_paragraph`]: the pixel font size, the (whole) lines to draw and the line height in pixels.
#[derive(Clone, Debug)]
pub struct FittedParagraph { pub px: u32, pub lines: Vec<String>, pub line_h: f32, pub truncated: bool }
/// Word-wrap `text` into a `w` x `h` pixel box WITHOUT ever cutting a line: the font shrinks (down to `MIN_TEXT_PX`) until every
/// wrapped line fits; if even the smallest size does not fit, only the lines that fully fit are kept and the last kept line ends
/// in "..." (`truncated`). `size` is in Slate units (scaled by `scale`). Pure; unit-tested with the longest hover strings.
pub fn fit_paragraph(fonts: &Fonts, font: FontId, text: &str, size: f32, scale: f32, w: f32, h: f32) -> FittedParagraph {
    let px_of = |s: f32| (s * scale).round().max(theme::MIN_TEXT_PX) as u32;
    let lay = |px: u32| { let lh = (fonts.line_height(font, px as f32) * 1.15).ceil(); (wrap_text(fonts, font, text, px as f32, w.max(1.)), lh) };
    let mut s = size;
    loop {
        let px = px_of(s);
        let (lines, lh) = lay(px);
        if lines.len() as f32 * lh <= h + 0.01 { return FittedParagraph { px, lines, line_h: lh, truncated: false }; }
        if px <= theme::MIN_TEXT_PX as u32 || s <= 1. {
            let keep = ((h / lh).floor() as usize).max(1).min(lines.len());
            let mut lines = lines; lines.truncate(keep);
            if let Some(last) = lines.last_mut() { let t = format!("{} ...", last.trim_end()); *last = fit_text(fonts, font, &t, px as f32, w.max(1.)); }
            return FittedParagraph { px, lines, line_h: lh, truncated: true };
        }
        s = (s - size * 0.04).max(1.);
    }
}
/// Minimum layout in Slate units: below this the menu shows a "window too small" notice instead of its screens.
pub const MIN_UI_SLATE: (f32, f32) = (480., 400.);
pub const TOO_SMALL_TEXT: &str = "Window too small for the menu. Enlarge the window to at least 640 x 360. Esc goes back.";
/// True when a `w` x `h` pixel client is too small to lay the menu screens out (see [`MIN_UI_SLATE`]).
pub fn too_small(w: f32, h: f32) -> bool { let (vw, vh) = Viewport::new(w, h).slate_size(); vw < MIN_UI_SLATE.0 || vh < MIN_UI_SLATE.1 }

// ---- brushes / draw list -----------------------------------------------------------------------------------
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum BtnKind { Default, Emphasis, Critical }
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum BtnSize { M, L }
pub use crate::material::types::Vis;
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)] pub enum TabVis { Passive, PassiveHover, Active }

/// Original 1.10 textures used by the toolkit (all nine-slice with margin 0.5 except arrow/checkbox/keybind frame).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BrushId { Button(BtnKind, BtnSize, Vis), Tab(TabVis), Check(bool, Vis), Arrow, KeyFrame, Close(bool), Icon(IconKind), Calibration, Tex(TexId) }

/// Plain textures of the 1.8 Settings screen (assets/exported/Game/UI/Menu/..., drawn 1:1 as images).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TexId { Xb1Config, Xb1Lines, Ps4Config, Ps4Lines, ModeCombat, ModeBuild, RemoveIcon, EpicLogo }
impl TexId {
    pub const ALL: [TexId; 8] = [TexId::Xb1Config, TexId::Xb1Lines, TexId::Ps4Config, TexId::Ps4Lines, TexId::ModeCombat, TexId::ModeBuild, TexId::RemoveIcon, TexId::EpicLogo];
    pub fn file_name(self) -> &'static str {
        match self { TexId::Xb1Config => "XB1_ControllerConfig.png", TexId::Xb1Lines => "XB1_ControllerConfig_LineOverlay.png", TexId::Ps4Config => "PS4_ControllerConfig.png", TexId::Ps4Lines => "PS4_ControllerConfig_LineOverlay.png", TexId::ModeCombat => "Icon-Score-Combat-L.png", TexId::ModeBuild => "Icon-Score-Building-L.png", TexId::RemoveIcon => "T-SlotIcon-Remove-32.png", TexId::EpicLogo => "EpicLogo.png" }
    }
}

/// 52x52 icon textures of the MainMenu icon column (`Game/UI/Foundation/Textures/Icons/{Social,Modal}`): 1.8 names `T-Icon-<name>-XL` (Config,
/// Logout, Exit, Feedback) and `-52` (LegalInfo, Credits); 1.10 used 64x64 `-64` files (the runtime no longer asks for those).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IconKind { Config, Legal, News, Credits, Logout, Exit }
impl IconKind {
    pub const ALL: [IconKind; 6] = [IconKind::Config, IconKind::Legal, IconKind::News, IconKind::Credits, IconKind::Logout, IconKind::Exit];
    pub fn file_name(self) -> &'static str {
        match self {
            IconKind::Config => "T-Icon-Config-XL.png", IconKind::Legal => "T-Icon-LegalInfo-52.png",
            // ButtonNews uses T-Icon-Feedback-XL in the original widget (Image_4).
            IconKind::News => "T-Icon-Feedback-XL.png", IconKind::Credits => "T-Icon-Credits-52.png",
            IconKind::Logout => "T-Icon-Logout-XL.png", IconKind::Exit => "T-Icon-Exit-XL.png",
        }
    }
}
impl BrushId {
    /// PNG file name under assets/exported (found by name through `assets::find`).
    pub fn file_name(self) -> String {
        match self {
            BrushId::Button(kind, size, vis) => {
                let k = match kind { BtnKind::Default => "Default", BtnKind::Emphasis => "Emphasis", BtnKind::Critical => "Critical" };
                let s = match size { BtnSize::M => "M", BtnSize::L => "L" };
                match vis {
                    Vis::Normal => format!("Button-{k}-{s}-_normal_.png"), Vis::Hover => format!("Button-{k}-{s}-_hover_.png"), Vis::Pressed => format!("Button-{k}-{s}-_click_.png"),
                    // The Emphasis style asset itself maps Disabled to Button-Default-M-_disabled_; Critical has no disabled texture.
                    Vis::Disabled => format!("Button-Default-{s}-_disabled_.png"),
                }
            }
            BrushId::Tab(v) => match v { TabVis::Passive => "Tab1-M-_passive_.png", TabVis::PassiveHover => "Tab1-M-_passive-hover_.png", TabVis::Active => "Tab1-M-_active_.png" }.to_owned(),
            BrushId::Check(on, v) => format!("CheckBox{}{}.png", if on { "Checked" } else { "Unchecked" }, match v { Vis::Hover => "Hovered", Vis::Pressed => "Pressed", _ => "" }),
            BrushId::Arrow => "T-HUD-Slider-Arrow.png".to_owned(),
            BrushId::KeyFrame => "Keybind-Frame-M.png".to_owned(),
            BrushId::Close(hover) => if hover { "CloseButton-M-_hover_.png" } else { "CloseButton-M-_normal_.png" }.to_owned(),
            BrushId::Icon(i) => i.file_name().to_owned(),
            // Brightness tab logo (original T_BrightnessCalibration, 512x512 RGBA, used by M_CalibrationSet_Master_Light/Dark).
            BrushId::Calibration => "T_BrightnessCalibration.png".to_owned(),
            BrushId::Tex(t) => t.file_name().to_owned(),
        }
    }
    /// Box-brush margin (fractions of the texture, as in the style assets) or None for plain sprites.
    pub fn box_margin(self) -> Option<[f32; 4]> {
        match self { BrushId::Button(..) | BrushId::Tab(_) | BrushId::Close(_) => Some([0.5; 4]), BrushId::KeyFrame => Some([0.35; 4]), _ => None }
    }
    /// Every brush the renderer must load.
    pub fn all() -> Vec<BrushId> {
        let mut v = Vec::new();
        for k in [BtnKind::Default, BtnKind::Emphasis, BtnKind::Critical] { for s in [BtnSize::M, BtnSize::L] { for vis in [Vis::Normal, Vis::Hover, Vis::Pressed, Vis::Disabled] { v.push(BrushId::Button(k, s, vis)); } } }
        for t in [TabVis::Passive, TabVis::PassiveHover, TabVis::Active] { v.push(BrushId::Tab(t)); }
        for on in [false, true] { for vis in [Vis::Normal, Vis::Hover, Vis::Pressed] { v.push(BrushId::Check(on, vis)); } }
        v.extend([BrushId::Arrow, BrushId::KeyFrame, BrushId::Close(false), BrushId::Close(true)]);
        v.extend(IconKind::ALL.map(BrushId::Icon));
        v.push(BrushId::Calibration);
        v.extend(TexId::ALL.map(BrushId::Tex));
        v
    }
}

#[derive(Clone, Debug)]
pub enum Cmd {
    Fill { rect: Rect, color: [f32; 4] },
    Brush { brush: BrushId, rect: Rect, tint: [f32; 4] },
    /// Plain sprite (optionally mirrored horizontally).
    Sprite { brush: BrushId, rect: Rect, flip_x: bool, tint: [f32; 4] },
    /// `x` = left of the first glyph, `top`/`box_h` = vertical box the line is centred in. `px` = atlas pixel size.
    Text { x: f32, top: f32, box_h: f32, text: String, font: FontId, px: u32, color: [f32; 4], shadow: bool },
    PushClip(Rect),
    PopClip,
    /// Material-driven box (see outline.rs): the renderer expands it with `outline::expand`; kept as one command so the
    /// widget state (`vis`) is visible to tests. `unit` = pixels per Slate unit, `alpha` = menu fade.
    Procedural { style: outline::ProcStyle, vis: Vis, rect: Rect, unit: f32, alpha: f32 },
    /// Gradient-panel material (`outline::PanelKind`): the renderer stretches `outline::panel_bitmap` over `rect`; `alpha` = menu fade.
    Panel { kind: outline::PanelKind, rect: Rect, alpha: f32 },
    /// Cooked 1.8 UI material (pages/uimat.rs: the pixel shader ported instruction by instruction), rasterised per size by the renderer and cached.
    /// `fill` = slider fillPercent, `tint` = widget colour/opacity (straight RGBA, already includes the menu fade), `scale` = DPI scale.
    Material { mat: pages::uimat::Mat, state: pages::uimat::MatState, rect: Rect, fill: f32, tint: [f32; 4], scale: f32, flip_x: bool },
}

#[derive(Default, Debug)]
pub struct DrawList { pub cmds: Vec<Cmd> }
impl DrawList {
    pub fn clear(&mut self) { self.cmds.clear(); }
    pub fn texts(&self) -> Vec<&Cmd> { self.cmds.iter().filter(|c| matches!(c, Cmd::Text { .. })).collect() }
    pub fn text_strings(&self) -> Vec<&str> { self.cmds.iter().filter_map(|c| if let Cmd::Text { text, .. } = c { Some(text.as_str()) } else { None }).collect() }
    pub fn brushes(&self) -> Vec<(BrushId, Rect)> { self.cmds.iter().filter_map(|c| if let Cmd::Brush { brush, rect, .. } | Cmd::Sprite { brush, rect, .. } = c { Some((*brush, *rect)) } else { None }).collect() }
    pub fn has_text(&self, s: &str) -> bool { self.cmds.iter().any(|c| matches!(c, Cmd::Text { text, .. } if text == s)) }
}

// ---- theme (Slate units; APPROXIMATE except where noted in the module docs) ---------------------------------
pub mod theme {
    pub const WHITE: [f32; 4] = [1., 1., 1., 1.];
    pub const TEXT: [f32; 4] = [1., 1., 1., 1.];
    pub const TEXT_DIM: [f32; 4] = [0.74, 0.82, 0.84, 1.];
    pub const TEXT_DISABLED: [f32; 4] = [0.55, 0.57, 0.58, 1.];
    /// Teal sampled from the original Button-Default-M textures (centre 0,99,94 / hover edge 88,234,185).
    pub const TEAL: [f32; 4] = [0., 99. / 255., 94. / 255., 1.];
    pub const TEAL_BRIGHT: [f32; 4] = [88. / 255., 234. / 255., 185. / 255., 1.];
    pub const PANEL: [f32; 4] = [0.012, 0.07, 0.10, 0.96];
    pub const PANEL_EDGE: [f32; 4] = [0.0, 0.40, 0.37, 1.];
    pub const TRACK: [f32; 4] = [0.0, 0.02, 0.04, 0.75];
    // (1.8: there is NO dim constant any more. MainMenu.Border_0 is NoDrawType, so its BrushColor 0,0,0,0.5 is never drawn; the Esc menu
    // backdrop is the ShellUberBG material panel drawn by `mainmenu::MainMenuScreen`, see outline::PanelKind::Backdrop.)
    pub const ROW_H: f32 = 44.;
    pub const ROW_GAP: f32 = 4.;
    pub const BTN_M_H: f32 = 36.;
    pub const BTN_L_H: f32 = 60.;
    pub const FONT_BODY: f32 = 17.;
    pub const FONT_BUTTON_M: f32 = 18.;
    pub const FONT_BUTTON_L: f32 = 24.;
    pub const FONT_HEADER: f32 = 20.;
    pub const FONT_TITLE: f32 = 34.;
    pub const MIN_TEXT_PX: f32 = 8.;
    pub const SCROLLBAR_W: f32 = 10.;
    pub const WHEEL_UNITS: f32 = 64.;
    pub const LABEL_FRAC: f32 = 0.42;
}

// ---- persistent UI state --------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
struct Region { id: Id, rect: Rect, scroll_owner: Option<Id> }
#[derive(Clone, Copy, Debug)]
struct Focusable { id: Id }
#[derive(Clone, Copy, Debug, Default)]
pub struct ScrollState { pub offset: f32, pub content_h: f32, pub view_h: f32 }

#[derive(Clone, Debug)]
pub(crate) struct PopupReq { pub id: Id, pub anchor: Rect, pub items: Vec<String>, pub selected: usize }

/// Most events a frame queues before new ones are dropped (see [`UiState::push_event`]).
pub const MAX_QUEUED_EVENTS: usize = 2048;

#[derive(Default)]
pub struct UiState {
    events: Vec<Event>,
    mouse: Option<(f32, f32)>, mouse_down: bool,
    regions: Vec<Region>, next_regions: Vec<Region>,
    focusables: Vec<Focusable>, next_focusables: Vec<Focusable>,
    hot: Option<Id>, active: Option<Id>, focus: Option<Id>, focus_visible: bool, focus_moved: bool, auto_focus: bool,
    /// Last input came from the gamepad (MainMenu `Set Icon Button List Column Width`: 90 when using a gamepad, else 48).
    pub(crate) pad_mode: bool,
    press_target: Option<Id>, release_target: Option<Id>, released: bool,
    wheel: f32, nav: Vec<NavKey>, raw_key: Option<u16>,
    listening: Option<Id>, open_dropdown: Option<Id>, dd_hi: usize, dd_scroll: f32,
    popup: Option<PopupReq>, popup_result: Option<(Id, usize, u8)>, popup_handled: bool, popup_press: Option<usize>,
    hot_owner: Option<Id>, mouse_moved: bool, pressed_any: bool,
    pub(crate) scroll: HashMap<Id, ScrollState>,
    flash: Option<(Id, f32)>,
    /// Modifier keys held right now (copied from the menu's input router at the start of every frame).
    pub(crate) mods: crate::router::Modifiers,
}
impl UiState {
    pub fn new() -> Self { Self { auto_focus: true, ..Default::default() } }
    /// Queue an event for the next frame. Consecutive pointer moves and wheel deltas coalesce (a flood of them costs nothing) and the queue is
    /// capped (everything but FocusLost is dropped beyond [`MAX_QUEUED_EVENTS`]), so an input storm can never grow memory or stall a frame.
    pub fn push_event(&mut self, e: Event) {
        match (self.events.last_mut(), e) {
            (Some(Event::MouseMove(x, y)), Event::MouseMove(nx, ny)) => { *x = nx; *y = ny; return; }
            (Some(Event::Wheel(d)), Event::Wheel(nd)) => { *d += nd; return; }
            _ => {}
        }
        if self.events.len() >= MAX_QUEUED_EVENTS && e != Event::FocusLost { return; }
        self.events.push(e);
    }
    /// Forget transient interaction (screen change, window deactivation, close).
    pub fn reset_interaction(&mut self) {
        self.active = None; self.listening = None; self.open_dropdown = None; self.popup = None; self.popup_result = None; self.flash = None;
        self.press_target = None; self.release_target = None; self.released = false; self.mouse_down = false; self.focus = None; self.auto_focus = true;
        self.regions.clear(); self.next_regions.clear(); self.focusables.clear(); self.next_focusables.clear();
    }
    pub fn focused_id(&self) -> Option<Id> { self.focus }
    pub fn hot_id(&self) -> Option<Id> { self.hot }
    pub fn active_id(&self) -> Option<Id> { self.active }
    pub fn is_listening(&self) -> bool { self.listening.is_some() }
    pub fn dropdown_open(&self) -> bool { self.open_dropdown.is_some() }
    pub fn mouse_pos(&self) -> Option<(f32, f32)> { self.mouse }
    pub fn pending_events(&self) -> usize { self.events.len() }
    fn hit(&self, x: f32, y: f32) -> Option<Id> { self.regions.iter().rev().find(|r| rect_contains(r.rect, x, y)).map(|r| r.id) }
    fn hit_region(&self, x: f32, y: f32) -> Option<Region> { self.regions.iter().rev().find(|r| rect_contains(r.rect, x, y)).copied() }
    fn take_nav(&mut self, k: NavKey) -> bool { if let Some(p) = self.nav.iter().position(|x| *x == k) { self.nav.remove(p); true } else { false } }
    fn count_nav(&mut self, k: NavKey) -> usize { let n = self.nav.iter().filter(|x| **x == k).count(); self.nav.retain(|x| *x != k); n }

    /// Fold queued events into this frame's input. Called once per frame before any widget runs.
    fn begin_frame(&mut self, dt: f32) {
        self.regions = std::mem::take(&mut self.next_regions);
        self.focusables = std::mem::take(&mut self.next_focusables);
        self.press_target = None; self.release_target = None; self.released = false; self.wheel = 0.; self.nav.clear(); self.raw_key = None; self.focus_moved = false;
        self.mouse_moved = false; self.pressed_any = false; self.popup_handled = false;
        if let Some((_, t)) = self.flash.as_mut() { *t -= dt; }
        if matches!(self.flash, Some((_, t)) if t <= 0.) { self.flash = None; }
        let events = std::mem::take(&mut self.events);
        for e in events {
            let listening = self.listening.is_some();
            match e {
                Event::MouseMove(x, y) => { if self.mouse != Some((x, y)) { self.mouse_moved = true; self.focus_visible = false; self.pad_mode = false; } self.mouse = Some((x, y)); }
                Event::MouseLeave => { self.mouse = None; }
                Event::MouseDown(b, x, y) => {
                    self.mouse = Some((x, y)); self.focus_visible = false;
                    if listening && b != MouseButton::Left {
                        // first wins: Esc and the key that was pressed with it in the same frame must not race for the binding
                        if self.raw_key.is_none() { self.raw_key = Some(match b { MouseButton::Right => 0x02, MouseButton::Middle => 0x04, MouseButton::X1 => 0x05, _ => 0x06 }); }
                    } else if b == MouseButton::Left { self.mouse_down = true; self.press_target = self.hit(x, y); self.released = false; self.pressed_any = true; }
                }
                Event::MouseUp(b, x, y) => { self.mouse = Some((x, y)); if b == MouseButton::Left { self.mouse_down = false; self.released = true; self.release_target = self.hit(x, y); } }
                Event::Wheel(d) => self.wheel += d,
                Event::Nav(k) => { self.pad_mode = false; if !listening { self.nav.push(k); } }
                Event::Pad(b) => { self.pad_mode = true; if !listening { self.nav.push(pad_to_nav(b)); } }
                Event::RawKey(vk) => { if listening && self.raw_key.is_none() { self.raw_key = Some(vk); } }
                Event::FocusLost => { self.mouse_down = false; self.active = None; self.listening = None; self.open_dropdown = None; self.popup = None; self.press_target = None; self.released = false; self.release_target = None; self.nav.clear(); self.raw_key = None; }
            }
        }
        let hr = self.mouse.and_then(|(x, y)| self.hit_region(x, y));
        self.hot = hr.map(|r| r.id); self.hot_owner = hr.and_then(|r| r.scroll_owner);
        if !self.nav.is_empty() { self.focus_visible = true; }
        // Linear focus navigation over last frame's focus order. Popups and key capture own Up/Down.
        if self.open_dropdown.is_none() && self.listening.is_none() {
            let order: Vec<Id> = self.focusables.iter().map(|f| f.id).collect();
            let mut keep = Vec::new();
            for k in std::mem::take(&mut self.nav) {
                match k {
                    NavKey::Up | NavKey::Down if !order.is_empty() => {
                        let cur = self.focus.and_then(|f| order.iter().position(|o| *o == f));
                        let next = match (cur, k) {
                            (None, NavKey::Down) => 0, (None, _) => order.len() - 1,
                            (Some(i), NavKey::Down) => (i + 1) % order.len(), (Some(i), _) => (i + order.len() - 1) % order.len(),
                        };
                        self.focus = Some(order[next]); self.focus_moved = true; self.auto_focus = false;
                    }
                    other => keep.push(other),
                }
            }
            self.nav = keep;
        }
        if let Some(f) = self.focus { if !self.focusables.iter().any(|x| x.id == f) && !self.focusables.is_empty() { self.focus = None; } }
    }
    fn end_frame(&mut self) {
        if self.released { if !self.mouse_down { self.active = None; } }
        // Active widget that vanished (e.g. tab switched mid-drag): never stay stuck.
        if let Some(a) = self.active { if !self.mouse_down && !self.next_regions.iter().any(|r| r.id == a) { self.active = None; } }
        self.nav.clear();
        // A key-capture whose button was not submitted (visibly) this frame (tab switched, scrolled away, screen changed) must not
        // leave the menu listening forever: while listening Nav/pad events are dropped, so a stale capture would freeze the keyboard.
        if let Some(l) = self.listening { if !self.next_regions.iter().any(|r| r.id == l) { self.listening = None; } }
        // A dropdown whose widget was not submitted this frame (tab/screen changed) must not leave a dead popup open.
        if self.open_dropdown.is_some() && !self.popup_handled { self.open_dropdown = None; self.popup = None; }
        if let Some((_, _, age)) = self.popup_result.as_mut() { *age += 1; }
        if matches!(self.popup_result, Some((_, _, age)) if age >= 2) { self.popup_result = None; }
    }
}

// ---- per-frame UI context ------------------------------------------------------------------------------------
#[derive(Clone, Copy)]
pub struct TextStyle { pub font: FontId, pub size: f32, pub color: [f32; 4], pub shadow: bool }
impl TextStyle {
    pub const fn new(font: FontId, size: f32, color: [f32; 4]) -> Self { Self { font, size, color, shadow: false } }
    pub const fn shadowed(mut self) -> Self { self.shadow = true; self }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HAlign { Left, Center, Right }

#[derive(Clone, Copy, Debug, Default)]
pub struct Interact { pub hot: bool, pub down: bool, pub focused: bool, pub clicked: bool, pub activated: bool, pub grabbed: bool, pub enabled: bool }
#[derive(Clone, Copy, Debug, Default)]
pub struct Response { pub clicked: bool, pub changed: bool, pub hot: bool, pub focused: bool }

pub(crate) struct ScrollCtx { pub id: Id, pub clip: Rect, pub want: Option<(f32, f32)> }

pub struct Ui<'a> {
    pub vp: Viewport,
    pub fonts: &'a Fonts,
    pub dt: f32,
    /// Global alpha (menu fade), multiplied into every tint.
    pub alpha: f32,
    pub(crate) out: &'a mut DrawList,
    pub(crate) st: &'a mut UiState,
    /// The settings session (working copy, apply requests, dialogs queue, platform-derived display list): the explicit replacement of
    /// the old thread-local bridge. Pages read and edit settings through it.
    pub session: &'a mut pages::Session,
    pub(crate) inert: bool,
    pub(crate) clips: Vec<Rect>,
    pub(crate) scrolls: Vec<ScrollCtx>,
    pub(crate) salt: Id,
    pub(crate) id_stack: Vec<Id>,
}
impl<'a> Ui<'a> {
    pub fn new(vp: Viewport, fonts: &'a Fonts, out: &'a mut DrawList, st: &'a mut UiState, session: &'a mut pages::Session, dt: f32, alpha: f32, inert: bool, salt: Id) -> Self {
        Self { vp, fonts, dt, alpha, out, st, session, inert, clips: Vec::new(), scrolls: Vec::new(), salt, id_stack: Vec::new() }
    }
    /// Slate units -> pixels.
    pub fn u(&self, v: f32) -> f32 { v * self.vp.scale }
    pub fn screen(&self) -> Rect { Rect::new(0., 0., self.vp.w, self.vp.h) }
    pub fn is_inert(&self) -> bool { self.inert }
    /// Modifier keys (left / right Shift, Ctrl, Alt) held right now, as tracked by the menu's input router.
    pub fn modifiers(&self) -> crate::router::Modifiers { self.st.mods }
    /// Atlas pixel size for a font size in Slate units.
    pub fn text_px(&self, size: f32) -> u32 { (size * self.vp.scale).round().max(theme::MIN_TEXT_PX) as u32 }
    pub fn id(&self, name: &str) -> Id { id_with(self.id_stack.last().copied().unwrap_or(self.salt), id_of(name)) }
    pub fn push_id(&mut self, name: &str) { let i = self.id(name); self.id_stack.push(i); }
    pub fn pop_id(&mut self) { self.id_stack.pop(); }
    pub(crate) fn clip(&self) -> Option<Rect> { self.clips.last().copied() }
    fn tint(&self, c: [f32; 4]) -> [f32; 4] { [c[0], c[1], c[2], c[3] * self.alpha] }

    // drawing primitives
    pub fn fill(&mut self, rect: Rect, color: [f32; 4]) { if rect.w > 0. && rect.h > 0. { let color = self.tint(color); self.out.cmds.push(Cmd::Fill { rect, color }); } }
    pub fn brush(&mut self, brush: BrushId, rect: Rect, tint: [f32; 4]) { if rect.w > 0. && rect.h > 0. { let tint = self.tint(tint); self.out.cmds.push(Cmd::Brush { brush, rect, tint }); } }
    pub fn sprite(&mut self, brush: BrushId, rect: Rect, flip_x: bool, tint: [f32; 4]) { if rect.w > 0. && rect.h > 0. { let tint = self.tint(tint); self.out.cmds.push(Cmd::Sprite { brush, rect, flip_x, tint }); } }
    /// Draw a cooked UI material box (see [`Cmd::Material`]).
    pub fn material(&mut self, mat: pages::uimat::Mat, state: pages::uimat::MatState, rect: Rect, fill: f32, tint: [f32; 4]) {
        self.material_flip(mat, state, rect, fill, tint, false);
    }
    /// [`Ui::material`] mirrored horizontally (RenderTransform scale -1,1 widgets).
    pub fn material_flip(&mut self, mat: pages::uimat::Mat, state: pages::uimat::MatState, rect: Rect, fill: f32, tint: [f32; 4], flip_x: bool) {
        if rect.w > 0. && rect.h > 0. { let tint = self.tint(tint); let scale = self.vp.scale; self.out.cmds.push(Cmd::Material { mat, state, rect, fill, tint, scale, flip_x }); }
    }
    pub fn push_clip(&mut self, r: Rect) {
        let r = match self.clip() { Some(c) => rect_intersect(c, r).unwrap_or(Rect::new(r.x, r.y, 0., 0.)), None => r };
        self.clips.push(r); self.out.cmds.push(Cmd::PushClip(r));
    }
    pub fn pop_clip(&mut self) { if self.clips.pop().is_some() { self.out.cmds.push(Cmd::PopClip); } }
    /// Single-line text in `rect` (ellipsised to fit, vertically centred).
    pub fn text(&mut self, rect: Rect, s: &str, style: TextStyle, align: HAlign) {
        if rect.w <= 0. || rect.h <= 0. { return; }
        let px = self.text_px(style.size);
        let t = fit_text(self.fonts, style.font, s, px as f32, rect.w);
        if t.is_empty() { return; }
        let w = self.fonts.measure(style.font, &t, px as f32);
        let x = match align { HAlign::Left => rect.x, HAlign::Center => rect.x + (rect.w - w) * 0.5, HAlign::Right => rect.x + rect.w - w };
        let color = self.tint(style.color);
        self.out.cmds.push(Cmd::Text { x, top: rect.y, box_h: rect.h, text: t, font: style.font, px, color, shadow: style.shadow });
    }
    /// Word-wrapped paragraph; returns the pixel height used.
    pub fn paragraph(&mut self, rect: Rect, s: &str, style: TextStyle, align: HAlign) -> f32 {
        let px = self.text_px(style.size);
        let lh = (self.fonts.line_height(style.font, px as f32) * 1.15).ceil();
        let lines = wrap_text(self.fonts, style.font, s, px as f32, rect.w);
        for (i, l) in lines.iter().enumerate() { self.text(Rect::new(rect.x, rect.y + i as f32 * lh, rect.w, lh), l, style, align); }
        lines.len() as f32 * lh
    }

    /// Paragraph that always fits `rect`: shrinks the font, then drops whole trailing lines (see [`fit_paragraph`]). Returns the pixel height used.
    pub fn paragraph_fit(&mut self, rect: Rect, s: &str, style: TextStyle, align: HAlign) -> f32 {
        let f = fit_paragraph(self.fonts, style.font, s, style.size, self.vp.scale, rect.w, rect.h);
        let size = f.px as f32 / self.vp.scale;
        let st = TextStyle { size, ..style };
        for (i, l) in f.lines.iter().enumerate() { self.text(Rect::new(rect.x, rect.y + i as f32 * f.line_h, rect.w, f.line_h), l, st, align); }
        f.lines.len() as f32 * f.line_h
    }

    // interaction ----------------------------------------------------------------------------------------------
    pub(crate) fn register_focusable(&mut self, id: Id) {
        self.st.next_focusables.push(Focusable { id });
        if self.st.auto_focus && self.st.focus.is_none() { self.st.focus = Some(id); self.st.auto_focus = false; }
    }
    /// Request initial focus for `id` (only honoured right after a screen change, before the user navigated).
    pub fn default_focus(&mut self, id: Id) { if self.st.auto_focus && !self.inert { self.st.focus = Some(id); self.st.auto_focus = false; } }
    pub fn is_focused(&self, id: Id) -> bool { self.st.focus == Some(id) && self.st.focus_visible }
    /// Register `rect` as a hit region + (optionally) a focus stop and resolve mouse/keyboard interaction.
    /// Disabled widgets still register a region so they block clicks from reaching widgets underneath.
    pub fn interact(&mut self, id: Id, rect: Rect, enabled: bool, focusable: bool) -> Interact { self.interact_ex(id, rect, enabled, focusable, true) }
    /// `accept_activate`: whether Enter/Space/A on the focused widget counts as a click (dropdowns hand it to their popup).
    pub fn interact_ex(&mut self, id: Id, rect: Rect, enabled: bool, focusable: bool, accept_activate: bool) -> Interact {
        let mut it = Interact { enabled, ..Default::default() };
        if self.inert { return it; }
        self.register_region(id, rect, false);
        if !enabled { return it; }
        if focusable { self.register_focusable(id); }
        it.focused = focusable && self.st.focus == Some(id) && self.st.focus_visible;
        // Keep the keyboard-focused widget inside its scroll area.
        if focusable && self.st.focus_moved && self.st.focus == Some(id) { if let Some(s) = self.scrolls.last_mut() { s.want = Some((rect.y, rect.y + rect.h)); } }
        it.hot = self.st.hot == Some(id);
        if self.st.press_target == Some(id) { self.st.active = Some(id); self.st.focus = Some(id); }
        it.grabbed = self.st.active == Some(id);
        if self.st.released && self.st.active == Some(id) { it.clicked = self.st.release_target == Some(id); self.st.active = None; }
        if accept_activate && focusable && self.st.focus == Some(id) && self.st.take_nav(NavKey::Activate) { it.activated = true; it.clicked = true; self.st.flash = Some((id, 0.12)); }
        it.down = (self.st.active == Some(id) && it.hot) || matches!(self.st.flash, Some((f, _)) if f == id);
        it
    }
    /// Submit a hit region (clipped to the current clip rect). `own_scroll`: the region belongs to the scroll area `id` itself.
    pub(crate) fn register_region(&mut self, id: Id, rect: Rect, own_scroll: bool) {
        if self.inert { return; }
        let vis = match self.clip() { Some(c) => rect_intersect(rect, c), None => Some(rect) };
        let owner = if own_scroll { Some(id) } else { self.scrolls.last().map(|s| s.id) };
        if let Some(v) = vis { self.st.next_regions.push(Region { id, rect: v, scroll_owner: owner }); }
    }
    /// Pressed look for the mouse-drag case where interact() already cleared `active` on release.
    pub fn mouse(&self) -> Option<(f32, f32)> { self.st.mouse }
    pub fn mouse_held(&self) -> bool { self.st.mouse_down }
    pub fn take_nav(&mut self, k: NavKey) -> bool { if self.inert { false } else { self.st.take_nav(k) } }
    pub fn count_nav(&mut self, k: NavKey) -> usize { if self.inert { 0 } else { self.st.count_nav(k) } }
    /// Call at the end of a screen's `ui` to consume the Back key (returns true once per press).
    /// An open dropdown popup gets the key first (it closes itself), so this stays false while one is open.
    pub fn back_pressed(&mut self) -> bool { if self.st.open_dropdown.is_some() { return false; } self.take_nav(NavKey::Back) }
    /// Left/Right move focus between a horizontal group of focusable ids (button rows). Returns true if focus moved.
    pub fn horizontal_focus(&mut self, ids: &[Id]) -> bool {
        if self.inert { return false; }
        let Some(cur) = self.st.focus.and_then(|f| ids.iter().position(|i| *i == f)) else { return false };
        let (l, r) = (self.st.count_nav(NavKey::Left) as i64, self.st.count_nav(NavKey::Right) as i64);
        if l + r == 0 { return false; }
        let next = (cur as i64 + r - l).clamp(0, ids.len() as i64 - 1) as usize;
        self.st.focus = Some(ids[next]); self.st.focus_moved = true; self.st.focus_visible = true; true
    }

    /// Called by the manager after a screen's `ui`: draws/handles overlay popups (dropdown lists).
    pub(crate) fn finish(&mut self) { widgets::finish_overlays(self); }
}

// ---- screens / manager ----------------------------------------------------------------------------------------
/// What the host application must react to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HostRequest {
    /// Menu closed (Resume / Esc at the root): host must re-capture the mouse and resume input.
    Closed,
    /// User confirmed quitting to desktop.
    QuitToDesktop,
}

pub enum Action {
    Push(Box<dyn Screen>),
    Pop,
    Resume,
    QuitToDesktop,
}

/// Mutable context handed to screens: queued stack actions + the registered settings pages.
pub struct Ctx<'a> {
    pub actions: Vec<Action>,
    pub pages: &'a mut Vec<Box<dyn SettingsPage>>,
    pub settings_tab: &'a mut usize,
    /// The settings session, available to `Screen::on_leave` (during `Screen::ui` it is reached through `Ui::session` instead).
    pub session: Option<&'a mut pages::Session>,
}
impl Ctx<'_> {
    pub fn push(&mut self, s: Box<dyn Screen>) { self.actions.push(Action::Push(s)); }
    pub fn pop(&mut self) { self.actions.push(Action::Pop); }
    pub fn resume(&mut self) { self.actions.push(Action::Resume); }
    pub fn quit_to_desktop(&mut self) { self.actions.push(Action::QuitToDesktop); }
}

/// One entry of the menu stack (pause menu, settings, confirm dialog, ...).
pub trait Screen {
    fn name(&self) -> &str;
    /// Build the screen: call widgets, queue actions on `cx`. Always called with the CURRENT viewport; derive all
    /// layout from `ui.vp` so resizing while open just works. Non-top screens run inert (draw only).
    fn ui(&mut self, ui: &mut Ui, cx: &mut Ctx);
    /// Opaque screens hide every screen below them (full-screen settings). Others are drawn over the one below.
    fn opaque(&self) -> bool { false }
    /// Called when the screen leaves the stack.
    fn on_leave(&mut self, _cx: &mut Ctx) {}
}

/// A tab of the Settings screen. Register it with `MenuSystem::register_settings_page` (the standard set: `pages::register_all_into`).
pub trait SettingsPage {
    /// Stable key (used for ids; must be unique among pages).
    fn id(&self) -> &str;
    /// Tab title.
    fn title(&self) -> &str;
    /// Sort key (lower first); ties keep registration order.
    fn order(&self) -> i32 { 0 }
    /// Draw the page into `area` (pixels, below the tab bar). Use `ui.form(...)` for scrolling rows.
    fn ui(&mut self, ui: &mut Ui, area: Rect);
    fn on_show(&mut self) {}
    fn on_hide(&mut self) {}
    /// The settings screen was closed (persist here).
    fn on_close(&mut self) {}
    /// True for the generic descriptor-built pages of `menu::pages`: a non-fallback page with the same `id()` replaces
    /// them (in either registration order), so specialised pages never need to know whether the generic one exists.
    fn is_fallback(&self) -> bool { false }
    /// "Reset to Default" support of this tab (footer button). `None` = not supported (button disabled);
    /// `Some(None)` = reset immediately; `Some(Some(text))` = ask `text` first (original: "Reset game options to default?").
    /// Resetting only edits the working copy of the settings; nothing changes in the game until Apply.
    fn reset_prompt(&self) -> Option<Option<String>> { None }
    /// Perform the reset announced by `reset_prompt` (edit the working copy with `sx.with_settings`).
    fn reset_to_default(&mut self, _sx: &mut pages::Session) {}
    /// Footer "Toggle Combat/Building Controls" (Controller tab only).
    fn toggle_mode(&mut self) {}
}

pub struct MenuSystem {
    open: bool,
    /// 0..1 fade (visual only; interaction state follows `open` immediately so rapid Esc toggling is consistent).
    anim: f32,
    stack: Vec<Box<dyn Screen>>,
    /// Focus of the screen below each pushed screen (id, ring visible): restored when the pushed screen is popped, so closing a dialog
    /// returns focus (and with it the hover description) to the control that opened it instead of re-running the default focus.
    focus_stack: Vec<(Option<Id>, bool)>,
    st: UiState,
    draw: DrawList,
    fonts: Fonts,
    pages: Vec<Box<dyn SettingsPage>>,
    settings_tab: usize,
    requests: Vec<HostRequest>,
    pad: Option<Box<dyn crate::input::PadSource>>,
    pub fade_time: f32,
    last_size: (f32, f32),
    /// Working copy of the settings + apply protocol (see `pages::Session`); owned here instead of in a thread-local.
    session: pages::Session,
    /// Raw input (pointer / keys / pad) -> toolkit events, with held-key / modifier / repeat bookkeeping; see [`MenuSystem::input`].
    router: crate::router::InputRouter,
}
impl MenuSystem {
    pub fn new(fonts: Fonts) -> Self {
        Self { open: false, anim: 0., stack: Vec::new(), focus_stack: Vec::new(), st: UiState::new(), draw: DrawList::default(), fonts, pages: Vec::new(), settings_tab: 0, requests: Vec::new(), pad: None, fade_time: 0.12, last_size: (0., 0.), session: pages::Session::new(), router: crate::router::InputRouter::new() }
    }
    /// Poll a gamepad source (the platform's `PadSource`) while the menu is open. Headless tests/scenes install none.
    pub fn set_gamepad(&mut self, src: Box<dyn crate::input::PadSource>) { self.pad = Some(src); }
    pub fn is_open(&self) -> bool { self.open }
    /// Fade value 0..1 (>0 while fading out after close).
    pub fn visibility(&self) -> f32 { self.anim }
    pub fn fonts(&self) -> &Fonts { &self.fonts }
    pub fn stack_names(&self) -> Vec<String> { self.stack.iter().map(|s| s.name().to_owned()).collect() }
    pub fn depth(&self) -> usize { self.stack.len() }
    pub fn state(&self) -> &UiState { &self.st }
    pub fn pages(&self) -> &[Box<dyn SettingsPage>] { &self.pages }
    /// The settings session (working copy, status line, queued dialogs).
    pub fn session(&self) -> &pages::Session { &self.session }
    pub fn session_mut(&mut self) -> &mut pages::Session { &mut self.session }
    pub fn router(&self) -> &crate::router::InputRouter { &self.router }
    /// Feed one raw, windowing-system-agnostic input event (see [`crate::router`]). The router always sees every event, also while the menu
    /// is closed, so held keys never turn into fresh presses when the menu opens (holding Esc cannot flicker it). The toolkit events only reach
    /// the screens while the menu is open. Returns what the host needs to know (e.g. escape_edge to open the menu when it is closed).
    pub fn input(&mut self, e: crate::router::InputEvent) -> crate::router::InputOutcome {
        let mut out = Vec::new();
        let res = self.router.push(e, &mut out);
        for ev in out { self.event(ev); }
        res
    }
    pub fn settings_tab(&self) -> usize { self.settings_tab }
    pub fn set_settings_tab(&mut self, i: usize) { self.settings_tab = i; }
    /// Remove pages whose id starts with `prefix` (capture scenes drop the toolkit-gallery demo pages).
    pub fn remove_settings_pages_prefixed(&mut self, prefix: &str) {
        self.pages.retain(|p| !p.id().starts_with(prefix));
        if self.settings_tab >= self.pages.len() { self.settings_tab = 0; }
    }
    pub fn register_settings_page(&mut self, p: Box<dyn SettingsPage>) {
        // Same id twice: a specialised page replaces a generic fallback; a fallback never replaces anything; otherwise last wins.
        if let Some(i) = self.pages.iter().position(|q| q.id() == p.id()) {
            if p.is_fallback() && !self.pages[i].is_fallback() { return; }
            self.pages.remove(i);
        }
        self.pages.push(p);
        // stable sort keeps registration order for equal `order()`
        self.pages.sort_by_key(|p| p.order());
    }
    /// Open at the pause root. Idempotent (rapid double Esc cannot stack two roots).
    pub fn open(&mut self) {
        if self.open { return; }
        self.open = true; self.stack.clear(); self.focus_stack.clear(); self.stack.push(Box::new(pause::PauseScreen::new()));
        self.st.reset_interaction();
        if let Some((x, y)) = self.router.pointer() { self.st.push_event(Event::MouseMove(x, y)); } // hover state without waiting for the next move
    }
    /// Close immediately (logical state); the fade-out is purely visual. Emits `HostRequest::Closed` once.
    pub fn close(&mut self) {
        if !self.open { return; }
        self.leave_all();
        self.open = false; self.st.reset_interaction();
        self.session.synced = false;
        self.requests.push(HostRequest::Closed);
    }
    pub fn toggle(&mut self) { if self.open { self.close(); } else { self.open(); } }
    fn leave_all(&mut self) {
        let mut acts = Vec::new();
        for s in self.stack.iter_mut().rev() { let mut cx = Ctx { actions: Vec::new(), pages: &mut self.pages, settings_tab: &mut self.settings_tab, session: Some(&mut self.session) }; s.on_leave(&mut cx); acts.append(&mut cx.actions); }
        self.stack.clear(); self.focus_stack.clear();
    }
    fn push_screen(&mut self, s: Box<dyn Screen>) {
        self.focus_stack.push((self.st.focus, self.st.focus_visible));
        self.stack.push(s); self.st.reset_interaction();
    }
    pub fn push(&mut self, s: Box<dyn Screen>) { if self.open { self.push_screen(s); } }
    pub fn pop(&mut self) {
        if !self.open { return; }
        if self.stack.len() <= 1 { self.close(); return; }
        if let Some(mut s) = self.stack.pop() { let mut cx = Ctx { actions: Vec::new(), pages: &mut self.pages, settings_tab: &mut self.settings_tab, session: Some(&mut self.session) }; s.on_leave(&mut cx); }
        self.st.reset_interaction();
        if let Some((f, vis)) = self.focus_stack.pop() { if f.is_some() { self.st.focus = f; self.st.focus_visible = vis || self.st.focus_visible; self.st.auto_focus = false; } }
    }
    /// Queue an input event. Ignored while closed (except FocusLost, which just clears stale state).
    pub fn event(&mut self, e: Event) {
        if self.open { self.st.push_event(e); } else if e == Event::FocusLost { self.st.reset_interaction(); }
    }
    pub fn take_requests(&mut self) -> Vec<HostRequest> { std::mem::take(&mut self.requests) }
    /// Skip the fade (deterministic captures).
    pub fn settle(&mut self) { self.anim = if self.open { 1. } else { 0. }; }
    /// Force the fade value (test captures of the transition).
    pub fn set_visibility(&mut self, v: f32) { self.anim = v.clamp(0., 1.); }
    /// The last built draw list and the fonts (for the GL renderer; `frame` borrows `&mut self`).
    pub fn parts(&self) -> (&DrawList, &Fonts) { (&self.draw, &self.fonts) }

    /// One frame with the host's context: fills the settings session on the first frame after the menu opened (`UiContext::settings`, the
    /// platform's display list), runs the UI pass and then processes the queued Apply/Preview/Save request against the host (`Session::pump`).
    pub fn frame_with(&mut self, cx: &mut UiContext, w: f32, h: f32, dt: f32) -> &DrawList {
        if self.open && !self.session.synced { self.session.sync_from(cx.settings); self.session.refresh_display(cx.platform); }
        self.session.pad_status = cx.pad;
        self.frame(w, h, dt);
        self.session.pump(cx);
        &self.draw
    }

    /// Run one UI pass for a `w` x `h` client and return the commands to draw. Cheap when closed. This is the host-less pass: it never
    /// touches the game's settings (an application uses [`MenuSystem::frame_with`]).
    pub fn frame(&mut self, w: f32, h: f32, dt: f32) -> &DrawList {
        self.draw.clear();
        let target = if self.open { 1. } else { 0. };
        let step = if self.fade_time <= 0. { 1. } else { dt / self.fade_time };
        // BUGFIX: the first version fell into the "fade out" branch whenever anim == target (open and fully faded in), so a
        // steady-state menu pulsed between 1.0 and 1-step every other frame (and with fade_time 0 between 1 and 0).
        self.anim = if self.anim < target { (self.anim + step).min(1.) } else if self.anim > target { (self.anim - step).max(0.) } else { self.anim };
        if !self.open && self.anim <= 0. { return &self.draw; }
        self.last_size = (w, h);
        let vp = Viewport::new(w, h);
        if self.open {
            let mut evs = Vec::new();
            self.router.tick(dt, &mut evs);
            for e in evs { self.st.push_event(e); }
            if let Some(p) = self.pad.as_mut() { for e in p.poll(dt) { self.st.push_event(e); } }
        } else { self.st.events.clear(); }
        self.st.mods = self.router.modifiers();
        self.st.begin_frame(dt);
        let alpha = self.anim;
        // Fade-out: the screens are gone as soon as the menu closes (the fade is purely visual), so only the ShellUberBG backdrop fades away.
        if !self.open { self.draw.cmds.push(Cmd::Panel { kind: outline::PanelKind::Backdrop, rect: Rect::new(0., 0., w, h), alpha }); }
        let n = self.stack.len();
        let first = (0..n).rev().find(|&i| self.stack[i].opaque()).unwrap_or(0);
        let mut actions: Vec<Action> = Vec::new();
        // Below the minimum the screens are not laid out at all (they would overlap or truncate to nothing): show a notice.
        let tiny = too_small(w, h);
        if tiny {
            let mut ui = Ui::new(vp, &self.fonts, &mut self.draw, &mut self.st, &mut self.session, dt, alpha, false, id_of("screen-too-small"));
            let pad = (ui.vp.w.min(ui.vp.h) * 0.06).max(4.);
            let panel = Rect::new(pad, pad, (ui.vp.w - 2. * pad).max(1.), (ui.vp.h - 2. * pad).max(1.));
            ui.fill(panel, [0.004, 0.03, 0.05, 0.96]);
            let inner = rect_deflate(panel, pad * 0.5, pad * 0.5, pad * 0.5, pad * 0.5);
            ui.push_clip(inner);
            ui.paragraph_fit(inner, TOO_SMALL_TEXT, TextStyle::new(FontId::Regular, 16., theme::WHITE), HAlign::Center);
            ui.pop_clip();
            if ui.back_pressed() { actions.push(Action::Pop); }
        }
        for i in first..n {
            if tiny { break; }
            let top = i == n - 1;
            let mut ui = Ui::new(vp, &self.fonts, &mut self.draw, &mut self.st, &mut self.session, dt, alpha, !top || !self.open, id_of(&format!("screen{i}:{}", self.stack[i].name())));
            let mut cx = Ctx { actions: Vec::new(), pages: &mut self.pages, settings_tab: &mut self.settings_tab, session: None };
            self.stack[i].ui(&mut ui, &mut cx);
            ui.finish();
            actions.append(&mut cx.actions);
        }
        self.st.end_frame();
        // Screens requested by settings pages (confirmation dialogs) through `pages::push_screen`.
        for s in self.session.take_screens() { actions.push(Action::Push(s)); }
        for a in actions {
            if !self.open { break; }
            match a {
                Action::Push(s) => self.push_screen(s),
                Action::Pop => self.pop(),
                Action::Resume => self.close(),
                Action::QuitToDesktop => { self.requests.push(HostRequest::QuitToDesktop); self.close_silently(); }
            }
        }
        &self.draw
    }
    /// Close without a `Closed` request (the app is quitting).
    fn close_silently(&mut self) { self.leave_all(); self.open = false; self.st.reset_interaction(); }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod live_tests;
