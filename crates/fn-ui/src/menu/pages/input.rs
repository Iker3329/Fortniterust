//! Settings "Input" tab: mouse look, crouch/sprint behaviour and the full key-binding list with rebinding.
//!
//! RECOVERED from the 1.8 pak (reports/settings-catalog.json, menu-layout-settings.json): the tab exists (`ShowInputOptions`, keyboard
//! platforms only), the 40 action display names ("Move Forward", "Weapon Slot 1", ... namespace `Fort.InputOptions` in Game.locres),
//! the slider range of the mouse sensitivity (0.01..1, 2 digits) and the labels "Mouse Sensitivity", "Invert View", "Toggle Sprint".
//! The original key list itself is built natively from AES-encrypted config, so it is NOT recoverable. ASSUMED / this runtime's choices:
//! the order and grouping of the actions (Movement, Combat, Weapons and Quickbar, Building, Interface), every default key, two bindings
//! per action (primary / secondary), the rebinding flow below, mouse X/Y sliders and Crouch Mode (extensions), and the wheel as a binding.
//!
//! Rebinding UX (all assumed; the original keybind widget is native):
//! * click a primary/secondary button -> "Press a key..."; the next key or mouse button (right, middle, thumb 1/2) or wheel notch binds;
//!   modifier keys (left/right Shift, Ctrl, Alt) bind; Esc cancels (Esc is reserved for the menu and can never be bound);
//!   Backspace/Delete clears the slot; clicking the listening button again binds the left mouse button; clicking elsewhere, losing
//!   focus, or waiting 10 s cancels.
//! * a key already used by another binding is SWAPPED (the other binding receives the key this slot had); a key moved within one
//!   action just moves. Nothing is announced on screen: the original shows no toast, warning or Undo (only the tooltip and "PRESS ANY KEY...").
//! * footer "Reset To Default" for the whole tab; a rebind takes effect at once (the Input tab has no Apply handler in 1.8).
use super::{bridge, PageState};
use crate::menu::*;
use crate::settings::{self, Bindings, Key, Settings, Value};

/// Seconds a "Press a key..." prompt waits before it cancels itself (gamepad users cannot press Esc while listening).
pub const LISTEN_TIMEOUT: f32 = 10.;
/// Clicks on the listening button inside this window are ignored (an accidental double click must not bind the left mouse button).
const CLICK_GRACE: f32 = 0.3;

// ---- key naming and capture -----------------------------------------------------------------------------------------------

/// Text for a key (Unreal display-name style: "Left Mouse Button", "Space Bar", "1", "Num 1", "F5"); "Not Bound" (`KeyboardKeyNames.KeyNotBound`) for none.
pub fn key_label(k: Key) -> String {
    let n = k.name();
    let fixed = match n {
        "None" => "Not Bound", /* KeyboardKeyNames `KeyNotBound`, the only unbound-key text in the 1.8 strings */ "MouseScrollUp" => "Mouse Wheel Up", "MouseScrollDown" => "Mouse Wheel Down", "BackSpace" => "Backspace",
        "Multiply" => "Num *", "Add" => "Num +", "Subtract" => "Num -", "Decimal" => "Num .", "Divide" => "Num /",
        "Zero" => "0", "One" => "1", "Two" => "2", "Three" => "3", "Four" => "4", "Five" => "5", "Six" => "6", "Seven" => "7", "Eight" => "8", "Nine" => "9",
        _ => "",
    };
    if !fixed.is_empty() { return fixed.to_owned(); }
    if let Some(d) = n.strip_prefix("NumPad") {
        let digit = ["Zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine"].iter().position(|x| *x == d);
        if let Some(i) = digit { return format!("Num {i}"); }
    }
    // CamelCase -> words ("LeftMouseButton" -> "Left Mouse Button", "ThumbMouseButton2" -> "Thumb Mouse Button 2")
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in n.chars() {
        if let Some(p) = prev {
            if (c.is_ascii_uppercase() && p.is_ascii_lowercase()) || (c.is_ascii_digit() && p.is_ascii_lowercase()) { out.push(' '); }
        }
        out.push(c); prev = Some(c);
    }
    out
}

/// What a raw key press means while a binding is listening.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capture { Bind(Key), Clear, Cancel, Unsupported(u16) }
/// Interpret a Windows virtual-key code (or mouse button code 2/4/5/6): Esc cancels (reserved for the menu), Backspace/Delete clear,
/// generic Shift/Ctrl/Alt become the left key (the menu already resolves left/right from the scan code), unknown keys are refused.
pub fn capture_key(vk: u16) -> Capture {
    match vk {
        0x1B => Capture::Cancel,
        0x08 | 0x2E => Capture::Clear,
        0x10 => Capture::Bind(Key::from_vk(0xA0).unwrap()), 0x11 => Capture::Bind(Key::from_vk(0xA2).unwrap()), 0x12 => Capture::Bind(Key::from_vk(0xA4).unwrap()),
        v => match Key::from_vk(v) { Some(k) if k.name() != "Escape" => Capture::Bind(k), _ => Capture::Unsupported(v) },
    }
}
fn wheel_key(delta: f32) -> Key { Key::from_name(if delta > 0. { "MouseScrollUp" } else { "MouseScrollDown" }).unwrap() }

/// Keys the runtime ALSO reads directly regardless of bindings (main.rs): binding them works but doubles up.
pub const RUNTIME_SHORTCUTS: &[(&str, &str)] = &[
    ("R", "also respawns the character at the start point (built-in runtime shortcut)"),
    ("F1", "also toggles the help overlay (built-in runtime shortcut)"),
    ("Up", "also looks up (built-in fallback look key)"), ("Down", "also looks down (built-in fallback look key)"),
    ("Left", "also turns left (built-in fallback look key)"), ("Right", "also turns right (built-in fallback look key)"),
];
fn shortcut_note(k: Key) -> Option<&'static str> { RUNTIME_SHORTCUTS.iter().find(|(n, _)| Key::from_name(n) == Some(k)).map(|(_, t)| *t) }

// ---- binding edits (pure) --------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct BindChange { pub action: usize, pub before: [Key; 2], pub after: [Key; 2] }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome { pub changes: Vec<BindChange>, pub message: Option<String>, pub warn: bool }

pub fn bind_id(action: usize) -> String { format!("bind.{}", settings::actions()[action].name) }
pub fn action_label(action: usize) -> &'static str { settings::actions()[action].label }
pub fn default_pair(action: usize) -> [Key; 2] { let a = &settings::actions()[action]; [Key::from_name(a.primary).unwrap(), Key::from_name(a.secondary).unwrap()] }
fn slot_name(slot: usize) -> &'static str { if slot == 0 { "primary" } else { "secondary" } }

fn touch(changes: &mut Vec<BindChange>, b: &Bindings, action: usize) {
    if !changes.iter().any(|c| c.action == action) { changes.push(BindChange { action, before: b.keys[action], after: b.keys[action] }); }
}
fn finish(changes: &mut Vec<BindChange>, b: &Bindings) {
    for c in changes.iter_mut() { c.after = b.keys[c.action]; }
    changes.retain(|c| c.before != c.after);
}
fn merge(into: &mut Outcome, o: Outcome) {
    for c in o.changes {
        match into.changes.iter_mut().find(|x| x.action == c.action) { Some(x) => x.after = c.after, None => into.changes.push(c) }
    }
    into.changes.retain(|c| c.before != c.after);
    if let Some(m) = o.message { into.message = Some(match into.message.take() { Some(p) => format!("{p} {m}"), None => m }); }
    into.warn |= o.warn;
}

/// Put `key` on binding `slot` (0 primary, 1 secondary) of `action`. A key already bound elsewhere is swapped with what this slot held
/// (inside one action it just moves); stray duplicates (hand-edited ini) are cleared so afterwards `key` is bound exactly once.
pub fn assign(b: &mut Bindings, action: usize, slot: usize, key: Key) -> Outcome {
    let slot = slot.min(1);
    let mut o = Outcome::default();
    if b.keys[action][slot] == key { return o; }
    let label = action_label(action);
    if key == Key::NONE {
        touch(&mut o.changes, b, action);
        b.keys[action][slot] = Key::NONE;
        finish(&mut o.changes, b);
        o.message = Some(format!("{label}: {} binding cleared.", slot_name(slot)));
        return o;
    }
    let owner = (0..b.keys.len()).flat_map(|a| [(a, 0usize), (a, 1usize)]).find(|&(a, s)| (a, s) != (action, slot) && b.keys[a][s] == key);
    let displaced = b.keys[action][slot];
    touch(&mut o.changes, b, action);
    let mut msg = format!("{label}: {} binding is now {}.", slot_name(slot), key_label(key));
    if let Some((oa, os)) = owner {
        touch(&mut o.changes, b, oa);
        b.keys[oa][os] = displaced;
        if oa == action {
            msg = format!("{} moved from the {} to the {} binding of {label}.", key_label(key), slot_name(os), slot_name(slot));
        } else {
            let now = if displaced == Key::NONE { "nothing".to_owned() } else { key_label(displaced) };
            msg = format!("{} was already used by {} ({}). Swapped: {} now uses {now}.", key_label(key), action_label(oa), slot_name(os), action_label(oa));
            o.warn = true;
        }
    }
    b.keys[action][slot] = key;
    // stray duplicates elsewhere
    for a in 0..b.keys.len() { for s in 0..2 { if (a, s) != (action, slot) && b.keys[a][s] == key { touch(&mut o.changes, b, a); b.keys[a][s] = Key::NONE; } } }
    if let Some(n) = shortcut_note(key) { msg.push_str(&format!(" Note: {} {n}.", key_label(key))); o.warn = true; }
    finish(&mut o.changes, b);
    o.message = Some(msg);
    o
}
/// Restore the default pair of one action (primary then secondary, each with the swap rule, so no key ends up bound twice).
pub fn reset_action(b: &mut Bindings, action: usize) -> Outcome {
    let d = default_pair(action);
    let mut o = Outcome::default();
    let mut swaps: Vec<String> = Vec::new();
    for slot in 0..2 {
        let part = assign(b, action, slot, d[slot]);
        if part.warn { if let Some(m) = &part.message { swaps.push(m.clone()); } }
        merge(&mut o, part);
    }
    // an action already at its defaults reports nothing; otherwise one concise message plus any swap explanations
    o.message = if o.changes.is_empty() { None } else { Some(std::iter::once(format!("{}: reset to default.", action_label(action))).chain(swaps).collect::<Vec<_>>().join(" ")) };
    o
}
/// Every binding back to its default (the defaults never conflict; see the settings tests).
pub fn reset_all(b: &mut Bindings) -> Outcome {
    let mut o = Outcome::default();
    let def = Bindings::default();
    for a in 0..b.keys.len() { if b.keys[a] != def.keys[a] { o.changes.push(BindChange { action: a, before: b.keys[a], after: def.keys[a] }); } }
    *b = def;
    o.message = Some("All key bindings reset to their defaults.".to_owned());
    o
}

/// Display order of the binding list. RECOVERED from the 1.8 EXE: `ConstructKeyBindList` (FortOptionsTab native, 0x142ee4070) appends
/// these 46 action names with their `Fort.InputOptions` display texts in exactly this order (the list is read from that name table; the
/// per-action key defaults live in the AES-encrypted DefaultInput.ini and stay unrecoverable). Names are this runtime's ids
/// (`Rotate` = RotatePrimitiveClockwise, `PrimaryQuickbarSlotN` = HotbarKeyN, `SecondaryQuickbarSlotFN` = HotbarKeyFN, `SpotTarget` = SpotActor,
/// `Respawn` = RespawnAtStart, `Resurrect` = ResurrectHere).
pub const GROUPS: &[(&str, &[&str])] = &[
    ("", &["MoveForward", "MoveBackward", "MoveRight", "MoveLeft", "ToggleCursorMode", "Jump", "Fire", "Target", "Reload", "Use", "TrapPicker",
        "PerformBuildingEditInteraction", "PerformBuildingImprovementInteraction", "Rotate", "Sprint", "ChangeMaterial", "BuildingEditReset",
        "PrimaryQuickbarSlot1", "PrimaryQuickbarSlot2", "PrimaryQuickbarSlot3", "PrimaryQuickbarSlot4", "PrimaryQuickbarSlot5", "PrimaryQuickbarSlot6",
        "PrimaryQuickbarSlot7", "PrimaryQuickbarSlot8", "PrimaryQuickbarSlot9",
        "SecondaryQuickbarSlotF1", "SecondaryQuickbarSlotF2", "SecondaryQuickbarSlotF3", "SecondaryQuickbarSlotF4", "SecondaryQuickbarSlotF5",
        "SwitchQuickbar", "SlotUp", "SlotDown", "Chat", "ChatPicker", "NotePicker", "SpotTarget", "SwitchToSelectedBuildingType",
        "ToggleFullScreenMap", "ToggleInventory", "Skip Cutscene", "Resurrect", "Respawn", "Crouch", "PushToTalk"]),
];
/// Ids of the settings the "Reset To Default" footer button restores on the Input tab (the whole tab plus the rows shown here).
pub fn tab_reset_ids() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = settings::entries().iter().filter(|e| e.tab == "Input").map(|e| e.id).collect();
    for id in ["input.mouse_sensitivity", "input.mouse_sensitivity_y", "input.invert_y", "input.toggle_sprint"] { if !v.contains(&id) { v.push(id); } }
    v
}


// ---- the page --------------------------------------------------------------------------------------------------------------
#[derive(Clone, Copy)]
enum Desc { Bind(usize) }
#[derive(Clone, Copy, Debug)]
struct Listen { action: usize, slot: usize, id: Id, age: f32 }
#[derive(Clone, Copy, Debug, PartialEq)]
enum Op { Assign { action: usize, slot: usize, key: Key }, Clear { action: usize, slot: usize } }

pub struct InputPage {
    ps: PageState,
    listen: Option<Listen>,
    desc: Option<Desc>,
}
impl InputPage {
    pub fn new() -> Self { Self { ps: PageState::default(), listen: None, desc: None } }
    /// Binding currently waiting for a key: (action, slot).
    pub fn listening(&self) -> Option<(usize, usize)> { self.listen.map(|l| (l.action, l.slot)) }
    fn stop(&mut self, ui: &mut Ui) { if self.listen.map_or(false, |l| ui.st.listening == Some(l.id)) { ui.st.listening = None; } self.listen = None; }

    /// Consume the key / mouse button / wheel notch a listening binding is waiting for.
    fn capture(&mut self, ui: &mut Ui, dt: f32, ops: &mut Vec<Op>) {
        let Some(mut l) = self.listen else { return };
        if ui.is_inert() { self.stop(ui); return; }
        if ui.st.listening != Some(l.id) { self.listen = None; return; } // the toolkit cancelled it (focus lost, screen change)
        l.age += dt;
        if l.age > LISTEN_TIMEOUT { self.stop(ui); return; }
        self.listen = Some(l);
        if let Some(vk) = ui.st.raw_key.take() {
            match capture_key(vk) {
                Capture::Bind(key) => ops.push(Op::Assign { action: l.action, slot: l.slot, key }),
                Capture::Clear => ops.push(Op::Clear { action: l.action, slot: l.slot }),
                Capture::Cancel => {}
                Capture::Unsupported(_) => {}
            }
            self.stop(ui);
        } else if ui.st.wheel != 0. {
            // taken BEFORE the page's scroll area runs, so binding the wheel never scrolls the list
            let key = wheel_key(ui.st.wheel);
            ui.st.wheel = 0.;
            ops.push(Op::Assign { action: l.action, slot: l.slot, key });
            self.stop(ui);
        }
    }

    fn exec(&mut self, op: Op, sx: &mut bridge::Session) {
        sx.with_settings(|st| {
            let mut b = st.input.bindings.clone();
            let outcome = match op {
                Op::Assign { action, slot, key } => assign(&mut b, action, slot, key),
                Op::Clear { action, slot } => assign(&mut b, action, slot, Key::NONE),
            };
            for c in &outcome.changes { let _ = st.set(&bind_id(c.action), Value::Keys(c.after)); }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn slot_button(&mut self, ui: &mut Ui, rect: Rect, action: usize, slot: usize, key: Key, dup: bool, ops: &mut Vec<Op>) -> Id {
        let id = ui.id(&format!("key.{action}.{slot}"));
        let it = ui.interact(id, rect, true, true);
        let inert = ui.is_inert();
        let mut listening = !inert && ui.st.listening == Some(id) && self.listen.map_or(false, |l| l.id == id);
        if listening {
            if ui.st.pressed_any && ui.st.press_target != Some(id) { self.stop(ui); listening = false; }
            else if it.clicked && self.listen.map_or(false, |l| l.age >= CLICK_GRACE) {
                ops.push(Op::Assign { action, slot, key: Key::from_name("LeftMouseButton").unwrap() });
                self.stop(ui); listening = false;
            }
        } else if it.clicked && !inert {
            if let Some(old) = self.listen { if ui.st.listening == Some(old.id) { ui.st.listening = None; } }
            self.listen = Some(Listen { action, slot, id, age: 0. });
            ui.st.listening = Some(id);
            listening = true;
        }
        if !listening && !inert && ui.st.focused_id() == Some(id) && key != Key::NONE && ui.take_nav(NavKey::Clear) { ops.push(Op::Clear { action, slot }); }
        if ui.clip().map_or(true, |c| rect_intersect(rect, c).is_some()) {
            // ScrollingTextButton with ButtonStyle-Secondary-M (M-Button-Secondary-*), text TextStyle-Button-Primary-M (NotoSans Bold 18, dark navy)
            use super::uimat::{Mat, MatState};
            let st = if listening || it.down { MatState::Pressed } else if it.hot || it.focused { MatState::Hovered } else { MatState::Base };
            ui.material(Mat::SecondaryM, st, rect, 0., [1., 1., 1., 1.]);
            let txt = if listening { "PRESS ANY KEY...".to_owned() } else { key_label(key) };
            let navy = crate::menu::outline::srgba([0.008023, 0.01096, 0.026241], 1.);
            let col = navy; let _ = dup; /* the original draws no conflict colouring */
            let pad = ui.u(6.);
            ui.text(Rect::new(rect.x + pad, rect.y, (rect.w - 2. * pad).max(0.), rect.h), &txt, TextStyle::new(FontId::Bold, 18., col), HAlign::Center);
        }
        id
    }


    /// Exact 1.8 key-binding row (OptionsMenuInput): label SizeBox 300 (right aligned, XS-Blue) | Primary (Fill, pad [15,5,5,5]) | Secondary (Fill,
    /// pad [0,5,5,5]) | unbind IconTextButton (SizeBox max 48, pad [0,5,15,5]); Secondary-M buttons are 48 high -> the row is 58 high.
    #[allow(clippy::too_many_arguments)]
    fn bind_row18(&mut self, ui: &mut Ui, r: Rect, snap: &Settings, a: usize, dups: &[[bool; 2]], list: Rect, ops: &mut Vec<Op>, hot: &mut Option<Desc>, foc: &mut Option<Desc>) {
        let s = ui.vp.scale;
        let g = bind_row_geo(r, s);
        let def = &settings::actions()[a];
        let keys = snap.input.bindings.get(a);
        let (mx, my) = ui.mouse().unwrap_or((-1., -1.));
        let hot_row = rect_contains(r, mx, my) && rect_contains(list, mx, my);
        let visible = ui.clip().map_or(true, |cl| rect_intersect(r, cl).is_some());
        let id0 = self.slot_button(ui, g.primary, a, 0, keys[0], dups.get(a).map_or(false, |d| d[0]), ops);
        let id1 = self.slot_button(ui, g.secondary, a, 1, keys[1], dups.get(a).map_or(false, |d| d[1]), ops);
        // unbind button (Secondary-M with the red remove icon)
        let uid = ui.id(&format!("unbind.{a}"));
        let ub = ui.interact(uid, g.unbind, keys != [Key::NONE, Key::NONE], true);
        if ub.clicked { ops.push(Op::Clear { action: a, slot: 1 }); ops.push(Op::Clear { action: a, slot: 0 }); }
        let mut group = vec![id0, id1]; group.push(uid);
        ui.horizontal_focus(&group);
        let focused = group.iter().any(|x| ui.st.focused_id() == Some(*x) && ui.st.focus_visible);
        if visible {
            use super::uimat::{Mat, MatState};
            let st = if !ub.enabled { MatState::Disabled } else if ub.down { MatState::Pressed } else if ub.hot || ub.focused { MatState::Hovered } else { MatState::Base };
            ui.material(Mat::SecondaryM, st, g.unbind, 0., [1., 1., 1., 1.]);
            // IconTextButton IconBrush: T-SlotIcon-Remove-32, 32x32, tint (0.6, 0.036, 0.036, 1), centred (ContentAlignment HAlign_Center)
            let isz = 32. * ui.vp.scale;
            let ir = Rect::new(g.unbind.x + (g.unbind.w - isz) * 0.5, g.unbind.y + (g.unbind.h - isz) * 0.5, isz, isz);
            let tint = crate::menu::outline::srgba([0.6, 0.036, 0.036], if ub.enabled { 1. } else { 0.5 }); // linear colour -> drawn sRGB, like every other recovered tint
            ui.sprite(BrushId::Tex(TexId::RemoveIcon), ir, false, tint);
            ui.text(g.label, def.label, TextStyle::new(FontId::Regular, 16., super::slate::label_color()), HAlign::Right);
        }
        if hot_row || group.iter().any(|x| ui.st.hot_id() == Some(*x)) { *hot = Some(Desc::Bind(a)); }
        if focused { *foc = Some(Desc::Bind(a)); }
    }

}

impl SettingsPage for InputPage {
    fn id(&self) -> &str { "input" }
    fn title(&self) -> &str { "Input" }
    fn order(&self) -> i32 { super::ORDER_INPUT }
    fn on_show(&mut self) { self.desc = None; }
    fn on_hide(&mut self) { self.listen = None; }
    fn on_close(&mut self) { self.on_hide(); }
    fn reset_prompt(&self) -> Option<Option<String>> { Some(Some("Reset input options to default?".to_owned())) }
    fn reset_to_default(&mut self, sx: &mut bridge::Session) {
        sx.with_settings(|s| for id in tab_reset_ids() { let _ = s.reset(id); });
        sx.request_apply(); // no Apply button on this tab in 1.8: the reset takes effect at once
    }
    fn ui(&mut self, ui: &mut Ui, area: Rect) {
        let dt = ui.dt.min(0.25);
        let snap = ui.session.settings_clone();
        let mut ops: Vec<Op> = Vec::new();
        self.capture(ui, dt, &mut ops);
        self.ui18(ui, area, &snap, ops);
    }
}
impl InputPage {
    /// Exact TabInputOptions: HBox[SizeBox(padding left 200, min 900 / max 1200){CommonListView of OptionsMenuInput rows}, TooltipDisplay (Fill, padding [30,30,230,30])].
    fn ui18(&mut self, ui: &mut Ui, area: Rect, snap: &Settings, mut ops: Vec<Op>) {
        let s = ui.vp.scale;
        let list = Rect::new(area.x + 200. * s, area.y, 900. * s, area.h);
        let tip = Rect::new(list.x + list.w + 30. * s, area.y + 30. * s, (area.x + area.w - (list.x + list.w + 30. * s) - 230. * s).max(0.), (area.h - 60. * s).max(0.));
        let (mut hot, mut foc): (Option<Desc>, Option<Desc>) = (None, None);
        let conflicts = snap.input.bindings.conflicts();
        let mut dups = vec![[false; 2]; snap.input.bindings.keys.len()];
        for (k, owners) in &conflicts { for &a in owners { for sl in 0..2 { if snap.input.bindings.keys[a][sl] == *k { dups[a][sl] = true; } } } }
        let order = binding_order();
        {
            let this = &mut *self;
            ui.push_id("binds");
            ui.scroll_area("scroll", list, |ui, view| {
                let mut y = view.y;
                for &a in &order {
                    let r = Rect::new(view.x, y, view.w, BIND_ROW_H * s);
                    this.bind_row18(ui, r, snap, a, &dups, list, &mut ops, &mut hot, &mut foc);
                    y += BIND_ROW_H * s;
                }
                y - view.y
            });
            ui.pop_id();
        }
        if let Some(d) = hot.or(foc) { self.desc = Some(d); }
        self.draw_tooltip18(ui, tip);
        if self.listen.is_some() {
            // OptionsMenu Message overlay: Border-Bottom-Box-Rounded-DkBlue, padding 25, text TextStyle-Header-L (Burbank 48) "PRESS ANY KEY..."
            let px = ui.text_px(48.) as f32;
            let tw = ui.fonts.measure(FontId::Title, "PRESS ANY KEY...", px);
            let th = ui.fonts.line_height(FontId::Title, px);
            let (w, h) = (tw + 100. * s, th + 100. * s);
            let r = Rect::new((ui.vp.w - w) * 0.5, (ui.vp.h - h) * 0.5, w, h);
            ui.material(super::uimat::Mat::BottomBoxRounded, super::uimat::MatState::Base, r, 0., [0., 0., 0., 0.7]);
            ui.text(Rect::new(r.x + 50. * s, r.y + 50. * s, tw, th), "PRESS ANY KEY...", TextStyle::new(FontId::Title, 48., [1., 1., 1., 1.]), HAlign::Left);
        }
        let had_ops = !ops.is_empty();
        for op in ops { self.exec(op, &mut *ui.session); }
        // The 1.8 Input tab registers no Apply handler (footer = Reset To Default + Back): a rebind takes effect at once.
        if had_ops && ui.session.is_dirty() { ui.session.request_apply(); }
    }
    fn draw_tooltip18(&mut self, ui: &mut Ui, rect: Rect) {
        if let Some(Desc::Bind(a)) = self.desc {
            // original text (OptionsMenuInput.SetData): "These options allow you to change ... Bindings for {Input Row}."
            let t = format!("These options allow you to change the primary and secondary key bindings for game actions. \n\nBindings for {}.", action_label(a));
            super::slate::tooltip_h(ui, rect, &t);
        }
    }
}

/// Row height of OptionsMenuInput: Secondary-M MinHeight 48 + slot padding 5 + 5.
pub const BIND_ROW_H: f32 = 58.;
pub struct BindRowGeo { pub label: Rect, pub primary: Rect, pub secondary: Rect, pub unbind: Rect }
/// Geometry of one binding row (pixels): label SizeBox 300 | Primary (Fill, pad [15,5,5,5]) | Secondary (Fill, pad [0,5,5,5]) | unbind (48 wide, pad [0,5,15,5]).
pub fn bind_row_geo(r: Rect, s: f32) -> BindRowGeo {
    let fill = (r.w / s - 300. - 48. - 15.).max(0.) / 2.;
    let y = r.y + 5. * s; let h = 48. * s;
    let label = Rect::new(r.x, r.y, 300. * s, r.h);
    let primary = Rect::new(r.x + (300. + 15.) * s, y, (fill - 20.).max(0.) * s, h);
    let secondary = Rect::new(r.x + (300. + fill) * s, y, (fill - 5.).max(0.) * s, h);
    let unbind = Rect::new(r.x + (300. + 2. * fill) * s, y, 48. * s, h);
    BindRowGeo { label, primary, secondary, unbind }
}
/// All actions in display order: the assumed groups flattened, then any action of the namespace not listed there.
pub fn binding_order() -> Vec<usize> {
    let mut v: Vec<usize> = Vec::new();
    for (_, names) in GROUPS { for n in names.iter() { if let Some(a) = Bindings::action_index(n) { v.push(a); } } }
    for a in 0..settings::actions().len() { if !v.contains(&a) { v.push(a); } }
    v
}

// ---- scripted captures (main.rs `--input-tab-test`) ---------------------------------------------------------------------
pub const TEST_SCENES: &[&str] = &["input-top", "input-binds", "input-listen", "input-conflict", "input-hover-stored", "controller-top", "controller-layout", "controller-hover"];
/// Put the menu into the Settings screen on the Input/Controller tab in a scripted state at a `w`x`h` client (events are interleaved with
/// headless frames so hit regions exist; the caller renders once more and captures).
pub fn test_scene(m: &mut MenuSystem, name: &str, w: f32, h: f32) {
    use crate::menu::pause::SettingsScreen;
    {
        m.session_mut().discard_changes(); // unapplied edits of the previous scene must not show up in this one
        m.close(); m.take_requests(); m.open();
        m.st.scroll.clear(); // scroll offsets are keyed by widget id and would otherwise carry over from the previous scene
        m.push(Box::new(SettingsScreen::new()));
        let frame = |m: &mut MenuSystem| { m.frame(w, h, 1. / 60.); };
        let ev = |m: &mut MenuSystem, e: Event| { m.event(e); m.frame(w, h, 1. / 60.); };
        for _ in 0..2 { frame(m); }
        let id = if name.starts_with("controller") { "controller" } else { "input" };
        if let Some(i) = m.pages().iter().position(|p| p.id() == id) { m.set_settings_tab(i); }
        for _ in 0..4 { frame(m); }
        let find = |m: &mut MenuSystem, text: &str| -> Option<(f32, f32)> {
            let (list, fonts) = (m.frame(w, h, 1. / 60.).cmds.clone(), m.fonts());
            list.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text: t, font, px, .. } = c { if t == text { Some((x + fonts.measure(*font, t, *px as f32) * 0.5, top + box_h * 0.5)) } else { None } } else { None })
        };
        let click = |m: &mut MenuSystem, p: (f32, f32)| { ev(m, Event::MouseMove(p.0, p.1)); ev(m, Event::MouseDown(MouseButton::Left, p.0, p.1)); ev(m, Event::MouseUp(MouseButton::Left, p.0, p.1)); frame(m); };
        let scroll = |m: &mut MenuSystem, notches: f32| { ev(m, Event::MouseMove(w * 0.5, h * 0.4)); ev(m, Event::Wheel(-notches)); for _ in 0..3 { frame(m); } };
        match name {
            "input-binds" => scroll(m, 8.),
            "input-listen" => { scroll(m, 8.); if let Some(p) = find(m, "Space Bar") { click(m, p); } for _ in 0..2 { frame(m); } }
            "input-conflict" => {
                scroll(m, 8.);
                if let Some(p) = find(m, "Left Control") { click(m, p); }
                ev(m, Event::RawKey(0x20)); for _ in 0..3 { frame(m); }
            }
            "input-hover-stored" => { scroll(m, 12.); if let Some(p) = find(m, "Reload") { ev(m, Event::MouseMove(p.0, p.1)); } }
            "controller-layout" => scroll(m, 12.),
            "controller-hover" => { if let Some(p) = find(m, "Controller Sensitivity X") { ev(m, Event::MouseMove(p.0, p.1)); } }
            _ => {}
        }
        for _ in 0..2 { frame(m); }
        m.settle();
    }
}
/// Evidence of the menu state (listening, scroll offsets) and every text drawn, so a scene can be checked without opening an image.
/// (The old runtime wrote this next to the capture; here the caller decides what to do with the string.)
pub fn state_summary(m: &mut MenuSystem, w: f32, h: f32) -> String {
    let off: Vec<f32> = m.st.scroll.values().map(|s| s.offset).collect();
    let listening = m.state().is_listening();
    let texts = m.frame(w, h, 0.).text_strings().join(" | ");
    format!("listening={listening} scroll_offsets={off:?}\n{texts}\n")
}

// ---- tests ------------------------------------------------------------------------------------------------------------------
#[cfg(test)]
mod tests;
