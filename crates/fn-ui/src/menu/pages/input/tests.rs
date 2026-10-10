//! Input tab tests: pure binding logic and headless page interaction (real NotoSans fonts, no GL, per-test settings bridge).
use super::*;
use crate::settings::{A_CROUCH, A_FIRE, A_JUMP, A_MOVE_FORWARD};
use crate::layout::Viewport;

fn fonts() -> Fonts { crate::testutil::fonts() }
fn k(n: &str) -> Key { Key::from_name(n).unwrap_or_else(|| panic!("key {n}")) }

// ---- pure ---------------------------------------------------------------------------------------------------------------
#[test] fn key_labels_are_readable_and_unique() {
    let mut seen = std::collections::HashSet::new();
    for n in Key::all_names() {
        let key = Key::from_name(n).unwrap();
        let l = key_label(key);
        assert!(!l.is_empty() && l != "Not Bound", "{n}");
        assert!(seen.insert(l.clone()), "label {l:?} used twice ({n})");
        assert!(l.is_ascii(), "{l}: the font atlas is printable ASCII only");
    }
    for (n, want) in [("LeftMouseButton", "Left Mouse Button"), ("ThumbMouseButton2", "Thumb Mouse Button 2"), ("SpaceBar", "Space Bar"), ("LeftControl", "Left Control"), ("One", "1"), ("Zero", "0"),
        ("NumPadSeven", "Num 7"), ("MouseScrollUp", "Mouse Wheel Up"), ("MouseScrollDown", "Mouse Wheel Down"), ("PageDown", "Page Down"), ("F12", "F12"), ("W", "W"), ("BackSpace", "Backspace"), ("CapsLock", "Caps Lock"), ("Multiply", "Num *")] {
        assert_eq!(key_label(k(n)), want);
    }
    assert_eq!(key_label(Key::NONE), "Not Bound");
}
#[test] fn capture_rules() {
    assert_eq!(capture_key(0x1B), Capture::Cancel, "Esc is reserved for the menu");
    assert_eq!(capture_key(0x08), Capture::Clear); assert_eq!(capture_key(0x2E), Capture::Clear);
    assert_eq!(capture_key(0x41), Capture::Bind(k("A"))); assert_eq!(capture_key(0x31), Capture::Bind(k("One"))); assert_eq!(capture_key(0x20), Capture::Bind(k("SpaceBar")));
    assert_eq!(capture_key(0xA0), Capture::Bind(k("LeftShift"))); assert_eq!(capture_key(0xA1), Capture::Bind(k("RightShift")));
    assert_eq!(capture_key(0xA2), Capture::Bind(k("LeftControl"))); assert_eq!(capture_key(0xA3), Capture::Bind(k("RightControl")));
    assert_eq!(capture_key(0xA4), Capture::Bind(k("LeftAlt"))); assert_eq!(capture_key(0xA5), Capture::Bind(k("RightAlt")));
    assert_eq!(capture_key(0x10), Capture::Bind(k("LeftShift")), "generic modifier codes fall back to the left key");
    assert_eq!(capture_key(0x02), Capture::Bind(k("RightMouseButton"))); assert_eq!(capture_key(0x04), Capture::Bind(k("MiddleMouseButton")));
    assert_eq!(capture_key(0x05), Capture::Bind(k("ThumbMouseButton"))); assert_eq!(capture_key(0x06), Capture::Bind(k("ThumbMouseButton2")));
    assert_eq!(capture_key(0x70), Capture::Bind(k("F1"))); assert_eq!(capture_key(0x13), Capture::Bind(k("Pause"))); assert_eq!(capture_key(0x90), Capture::Bind(k("NumLock")));
    assert_eq!(capture_key(0x5B), Capture::Unsupported(0x5B), "Windows key is not bindable");
    assert_eq!(capture_key(0xFF), Capture::Unsupported(0xFF));
    // every vk the table knows round-trips except Escape (never offered)
    for n in Key::all_names() { let key = k(n); if let Some(vk) = key.vk() { if n != "Escape" && !matches!(vk, 0x08 | 0x2E) { assert_eq!(capture_key(vk), Capture::Bind(Key::from_vk(vk).unwrap()), "{n}"); assert_eq!(Key::from_vk(vk).and_then(|x| x.vk()), Some(vk)); } } }
    assert_eq!(wheel_key(1.), k("MouseScrollUp")); assert_eq!(wheel_key(-3.), k("MouseScrollDown"));
}
#[test] fn assign_plain_move_into_free_key() {
    let mut b = Bindings::default();
    let o = assign(&mut b, A_JUMP, 0, k("C"));
    assert_eq!(b.get(A_JUMP), [k("C"), Key::NONE]);
    assert_eq!(o.changes, vec![BindChange { action: A_JUMP, before: [k("SpaceBar"), Key::NONE], after: [k("C"), Key::NONE] }]);
    assert!(!o.warn && o.message.unwrap().contains("Jump"));
    assert!(b.conflicts().is_empty());
}
#[test] fn assign_same_key_is_a_no_op() {
    let mut b = Bindings::default();
    let o = assign(&mut b, A_JUMP, 0, k("SpaceBar"));
    assert!(o.changes.is_empty() && o.message.is_none()); assert_eq!(b, Bindings::default());
}
/// 1.8 ChangeBinding (FUN_140542480): the key leaves every row that may not share it; nothing is swapped.
#[test] fn conflict_removes_the_key_from_exclusive_rows() {
    let mut b = Bindings::default();
    let skip = Bindings::action_index("Skip Cutscene").unwrap();
    let o = assign(&mut b, A_CROUCH, 0, k("SpaceBar")); // Space: Jump (Combat) and Skip Cutscene (Cinematic); Crouch has no input group
    assert_eq!(b.get(A_CROUCH), [k("SpaceBar"), Key::NONE]);
    assert_eq!(b.get(A_JUMP), [Key::NONE, Key::NONE], "Jump lost the key, it did not receive Left Control");
    assert_eq!(b.get(skip), [Key::NONE, k("Escape")]);
    assert!(o.warn); assert_eq!(o.changes.len(), 3);
    let m = o.message.unwrap(); assert!(m.contains("Space Bar") && m.contains("Jump"), "{m}");
    assert!(b.conflicts().is_empty());
}
/// Rows in different InputActionGroupContexts groups keep a shared key (FUN_1405fc190).
#[test] fn rows_of_different_input_groups_share_keys() {
    let mut b = Bindings::default();
    let reload = Bindings::action_index("Reload").unwrap(); let rotate = Bindings::action_index("Rotate").unwrap();
    let o = assign(&mut b, A_FIRE, 1, k("R")); // Fire is Combat: Reload (Combat) loses R, Rotate (Building) keeps it
    assert_eq!(b.get(reload)[0], Key::NONE); assert_eq!(b.get(rotate)[0], k("R")); assert_eq!(o.changes.len(), 2);
    assert!(b.conflicts().is_empty());
}
#[test] fn conflict_with_an_empty_slot_leaves_the_other_action_unbound() {
    let mut b = Bindings::default();
    assign(&mut b, A_JUMP, 1, k("Q")); // secondary of Jump <- Q (Switch Quickbar's primary); Jump.secondary was None
    assert_eq!(b.get(A_JUMP), [k("SpaceBar"), k("Q")]);
    let sq = Bindings::action_index("SwitchQuickbar").unwrap();
    assert_eq!(b.get(sq), [Key::NONE, k("ThumbMouseButton")], "Switch Quickbar lost Q and kept its other key");
    assert!(b.conflicts().is_empty());
}
#[test] fn moving_a_key_inside_one_action_just_moves_it() {
    let mut b = Bindings::default();
    assign(&mut b, A_JUMP, 1, k("C"));
    let o = assign(&mut b, A_JUMP, 0, k("C"));
    assert_eq!(b.get(A_JUMP), [k("C"), k("C")], "1.8 leaves the row's own other slot alone");
    assert_eq!(o.changes.len(), 1); assert!(!o.warn);
}
#[test] fn clearing_and_wheel_and_mouse_bindings() {
    let mut b = Bindings::default();
    let o = assign(&mut b, A_JUMP, 0, Key::NONE);
    assert_eq!(b.get(A_JUMP), [Key::NONE, Key::NONE]); assert!(o.message.unwrap().contains("cleared"));
    assert!(assign(&mut b, A_JUMP, 0, Key::NONE).changes.is_empty(), "clearing an empty slot is a no-op");
    assign(&mut b, A_JUMP, 0, k("MouseScrollUp"));
    assert_eq!(b.get(A_JUMP)[0].poll_code(), Some(settings::WHEEL_UP_CODE));
    assign(&mut b, A_JUMP, 1, k("ThumbMouseButton"));
    assert_eq!(b.get(A_JUMP)[1].poll_code(), Some(5));
    // the default Slot Down is wheel-up (ConfigD_Athena SlotDown = MouseScrollUp): taking it for Jump removed it from Slot Down
    let down = Bindings::action_index("SlotDown").unwrap();
    assert_eq!(b.get(down), [Key::NONE, Key::NONE]);
    assert!(b.conflicts().is_empty());
}
#[test] fn runtime_shortcut_keys_carry_a_note() {
    let mut b = Bindings::default();
    let o = assign(&mut b, A_JUMP, 0, k("R"));
    let m = o.message.unwrap(); assert!(m.contains("respawns") && o.warn, "{m}");
    let o = assign(&mut b, A_MOVE_FORWARD, 1, k("F8")); assert!(o.message.unwrap().contains("help overlay")); // help card moved F1 -> F8 (CookedIniVersion.txt L2677)
    let o = assign(&mut b, A_CROUCH, 1, k("Down")); let m = o.message.unwrap(); assert!(!m.contains("looks") && m.contains("Move Backward"), "arrows are movement keys in 1.8: {m}");
}
#[test] fn stray_duplicates_are_cleared() {
    let mut b = Bindings::default();
    b.keys[Bindings::action_index("Reload").unwrap()][1] = k("C"); b.keys[Bindings::action_index("Use").unwrap()][1] = k("C"); // hand-edited ini: C twice
    assign(&mut b, A_JUMP, 0, k("C"));
    let owners = b.keys.iter().flatten().filter(|x| **x == k("C")).count();
    assert_eq!(owners, 1, "C is bound exactly once afterwards");
}
#[test] fn reset_one_and_all() {
    let mut b = Bindings::default();
    assign(&mut b, A_JUMP, 0, k("C")); assign(&mut b, A_JUMP, 1, k("V"));
    let o = reset_action(&mut b, A_JUMP);
    assert_eq!(b.get(A_JUMP), default_pair(A_JUMP)); assert!(o.message.unwrap().contains("reset to default"));
    assert!(reset_action(&mut b, A_JUMP).changes.is_empty() && reset_action(&mut b, A_JUMP).message.is_none(), "already default: silent");
    // reset when the default key is taken by another action: it is removed there, no duplicates
    assign(&mut b, A_JUMP, 0, k("C")); assign(&mut b, A_CROUCH, 0, k("SpaceBar"));
    let o = reset_action(&mut b, A_JUMP);
    assert_eq!(b.get(A_JUMP)[0], k("SpaceBar")); assert!(b.conflicts().is_empty(), "{:?}", b.conflicts()); assert!(o.warn);
    let mut b2 = Bindings::default(); assign(&mut b2, A_FIRE, 0, k("G")); assign(&mut b2, A_JUMP, 0, k("H"));
    let o = reset_all(&mut b2); assert_eq!(b2, Bindings::default()); assert_eq!(o.changes.len(), 3, "Fire, Jump and Building Edit (which lost G)");
}
/// Random edit sequences must never produce a duplicate key (starting from the conflict-free defaults) and never lose the key just bound.
#[test] fn fuzz_assignments_never_duplicate_keys() {
    let names: Vec<&str> = Key::all_names().filter(|n| *n != "Escape").collect();
    let mut seed = 0x1234_5678u32;
    let mut rnd = |n: usize| { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; (seed as usize) % n };
    let mut b = Bindings::default();
    for step in 0..4000 {
        let a = rnd(b.keys.len()); let s = rnd(2);
        let key = if rnd(8) == 0 { Key::NONE } else { k(names[rnd(names.len())]) };
        match rnd(12) {
            0 => { reset_action(&mut b, a); }
            1 if step % 50 == 0 => { reset_all(&mut b); }
            _ => { assign(&mut b, a, s, key); if key != Key::NONE { assert_eq!(b.keys[a][s], key, "step {step}: the new key sits on the chosen slot"); } }
        }
        assert!(b.conflicts().is_empty(), "step {step}: duplicate key {:?}", b.conflicts());
    }
}
#[test] fn groups_list_every_action_exactly_once() {
    let mut seen = vec![0; settings::actions().len()];
    for (_, names) in GROUPS { for n in names.iter() { seen[Bindings::action_index(n).unwrap_or_else(|| panic!("unknown action {n}"))] += 1; } }
    // GROUPS only fixes the order of the first rows; the 1.8 list appends every other action of the namespace (binding_order)
    for (i, c) in seen.iter().enumerate() { assert!(*c <= 1, "{} listed {c} times", settings::actions()[i].name); }
    assert_eq!(settings::actions().len(), 46, "46 Fort.InputOptions actions in the 1.8 catalog");
    let order = binding_order();
    let mut sorted = order.clone(); sorted.sort(); sorted.dedup();
    assert_eq!((order.len(), sorted.len()), (46, 46), "every action is shown exactly once");
}
#[test] fn tab_reset_covers_the_shown_settings() {
    let ids = tab_reset_ids();
    for id in ["input.crouch_mode", "input.mouse_sensitivity", "input.mouse_sensitivity_y", "input.invert_y", "input.toggle_sprint", "bind.Jump", "bind.ToggleCursorMode"] { assert!(ids.contains(&id), "{id}"); }
    for id in &ids { assert!(settings::entry(id).is_some(), "{id}"); }
    assert!(!ids.contains(&"input.vibration"), "controller options are reset on the Controller tab");
}
// ---- headless page rig ----------------------------------------------------------------------------------------------------
pub struct Rig { st: UiState, sx: Session, draw: DrawList, fonts: Fonts, w: f32, h: f32, page: InputPage }
impl Rig {
    fn new(w: f32, h: f32) -> Self {
        let mut sx = Session::new(); sx.sync_from(&Settings::default());
        let mut r = Rig { st: UiState::new(), sx, draw: DrawList::default(), fonts: fonts(), w, h, page: InputPage::new() };
        r.frame(); r.frame(); r
    }
    fn area(&self) -> Rect { Rect::new(self.w * 0.04, self.h * 0.12, self.w * 0.92, self.h * 0.72) }
    fn frame(&mut self) { self.frames(1); }
    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.draw.clear();
            self.st.begin_frame(1. / 60.);
            {
                let vp = Viewport::new(self.w, self.h);
                let a = self.area();
                let mut ui = Ui::new(vp, &self.fonts, &mut self.draw, &mut self.st, &mut self.sx, 1. / 60., 1., false, id_of("rig"));
                ui.push_id("input");
                ui.push_clip(a);
                self.page.ui(&mut ui, a);
                ui.pop_clip(); ui.pop_id();
                ui.finish();
            }
            self.st.end_frame();
        }
    }
    fn ev(&mut self, e: Event) { self.st.push_event(e); }
    fn scroll(&mut self, units: f32) { self.frame(); for s in self.st.scroll.values_mut() { s.offset = units; } self.frame(); }
    /// Centre of the first drawn text equal to `text`.
    fn find(&self, text: &str) -> Option<(f32, f32)> {
        self.draw.cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text: t, font, px, .. } = c { if t == text { Some((x + self.fonts.measure(*font, t, *px as f32) * 0.5, top + box_h * 0.5)) } else { None } } else { None })
    }
    fn all(&self, text: &str) -> Vec<(f32, f32)> {
        self.draw.cmds.iter().filter_map(|c| if let Cmd::Text { x, top, box_h, text: t, font, px, .. } = c { if t == text { Some((x + self.fonts.measure(*font, t, *px as f32) * 0.5, top + box_h * 0.5)) } else { None } } else { None }).collect()
    }
    fn must(&self, text: &str) -> (f32, f32) { self.find(text).unwrap_or_else(|| panic!("text {text:?} not drawn; drawn: {:?}", self.draw.text_strings())) }
    fn click_at(&mut self, p: (f32, f32)) { self.ev(Event::MouseMove(p.0, p.1)); self.frame(); self.ev(Event::MouseDown(MouseButton::Left, p.0, p.1)); self.frame(); self.ev(Event::MouseUp(MouseButton::Left, p.0, p.1)); self.frames(2); }
    fn click(&mut self, text: &str) { let p = self.must(text); self.click_at(p); }
    fn key(&mut self, vk: u16) { self.ev(Event::RawKey(vk)); self.frames(2); }
    fn bindings(&self) -> Bindings { self.sx.settings_clone().input.bindings }
    fn has_text(&self, t: &str) -> bool { self.draw.has_text(t) }
    fn listening(&self) -> bool { self.st.is_listening() }
}

const SZ: (f32, f32) = (1920., 1080.);
fn rig() -> Rig { let mut r = Rig::new(SZ.0, SZ.1); r.scroll(0.); r } // 1.8 list: Movement + Combat rows (Jump, Crouch, Reload) are in view at the top

#[test] fn page_registers_as_input_and_replaces_the_generic_tab() {
    let p = InputPage::new();
    assert_eq!((p.id(), p.title(), p.order()), ("input", "Input", 50)); assert!(!p.is_fallback());
    let mut m = MenuSystem::new(fonts()); super::super::register_all_into(&mut m); m.open();
    let pages: Vec<(String, bool)> = m.pages().iter().map(|p| (p.id().to_owned(), p.is_fallback())).collect();
    assert!(pages.contains(&("input".to_owned(), false)) && pages.contains(&("controller".to_owned(), false)), "{pages:?}");
    assert_eq!(pages.iter().filter(|p| p.0 == "input").count(), 1);
}
#[test] fn click_then_press_a_key_rebinds() {
    let mut r = rig();
    assert_eq!(r.bindings().get(A_JUMP), [k("SpaceBar"), Key::NONE]);
    r.click("Space Bar");
    assert!(r.listening() && r.page.listening() == Some((A_JUMP, 0)), "listening");
    assert!(r.has_text("PRESS ANY KEY..."));
    r.key(0x43);
    assert!(!r.listening() && r.page.listening().is_none());
    assert_eq!(r.bindings().get(A_JUMP), [k("C"), Key::NONE]);
    assert!(r.has_text("C"));
    assert!(r.sx.is_dirty() && r.sx.changed_ids().contains(&"bind.Jump".to_owned()), "the bridge sees the edit so Apply lights up");
}
#[test] fn secondary_binding_is_independent() {
    let mut r = rig();
    let unbound = r.all("Not Bound");
    assert!(!unbound.is_empty());
    // Jump's secondary is the Unbound button on the Jump row (same y as "Space Bar")
    let y = r.must("Space Bar").1;
    let sec = unbound.iter().find(|p| (p.1 - y).abs() < 2.).copied().expect("secondary of Jump");
    r.click_at(sec); assert_eq!(r.page.listening(), Some((A_JUMP, 1)));
    r.key(0x56);
    assert_eq!(r.bindings().get(A_JUMP), [k("SpaceBar"), k("V")]);
}
#[test] fn escape_cancels_without_binding_and_cannot_be_bound() {
    let mut r = rig();
    r.click("Space Bar"); assert!(r.listening());
    r.ev(Event::Nav(NavKey::Back)); r.ev(Event::RawKey(0x1B)); r.frames(2);
    assert!(!r.listening()); assert_eq!(r.bindings().get(A_JUMP), [k("SpaceBar"), Key::NONE]);
    // the only Escape in the defaults is ConfigD_Athena's SkipCutscene second key (CookedIniVersion.txt L2751); the capture never binds it
    let skip = Bindings::action_index("Skip Cutscene").unwrap();
    assert!(Bindings::default().keys.iter().enumerate().all(|(a, ks)| a == skip || ks.iter().all(|x| x.name() != "Escape")));
    assert_eq!(capture_key(0x1B), Capture::Cancel);
}
#[test] fn navigation_keys_do_not_leak_while_listening() {
    let mut r = rig();
    r.click("Space Bar");
    for n in [NavKey::Down, NavKey::Up, NavKey::Activate, NavKey::Back, NavKey::TabNext, NavKey::Clear] { r.ev(Event::Nav(n)); }
    r.frames(2);
    assert!(r.listening(), "toolkit navigation is suppressed while a key prompt is open");
}
#[test] fn conflict_removes_and_announces_nothing_on_screen() {
    let mut r = rig();
    r.click("Left Mouse Button"); // Fire primary (Crouch sits at the end of the 1.8 list, out of view)
    r.key(0x20);
    let b = r.bindings();
    assert_eq!(b.get(A_FIRE)[0], k("SpaceBar")); assert_eq!(b.get(A_JUMP)[0], Key::NONE, "Jump lost Space (same Combat group), no swap");
    let all = r.draw.text_strings().join(" | ");
    for bad in ["already used", "Swapped", "Undo", "Binding changed", "Not available in this build", "NOT IMPLEMENTED"] { assert!(!all.contains(bad), "{bad:?} is not original text: {all}"); }
    assert!(!r.has_text("Undo"), "no Undo widget");
}
#[test] fn mouse_buttons_bind() {
    for (b, want) in [(MouseButton::Right, "RightMouseButton"), (MouseButton::Middle, "MiddleMouseButton"), (MouseButton::X1, "ThumbMouseButton"), (MouseButton::X2, "ThumbMouseButton2")] {
        let mut r = rig();
        r.click("Space Bar");
        let p = r.must("PRESS ANY KEY...");
        r.ev(Event::MouseDown(b, p.0, p.1)); r.ev(Event::MouseUp(b, p.0, p.1)); r.frames(2);
        assert_eq!(r.bindings().get(A_JUMP)[0], k(want), "{want}");
        assert!(!r.listening());
    }
}
#[test] fn clicking_the_listening_button_again_binds_the_left_mouse_button() {
    let mut r = rig();
    r.click("Space Bar"); r.frames(30);
    r.click("PRESS ANY KEY...");
    assert_eq!(r.bindings().get(A_JUMP)[0], k("LeftMouseButton"));
    assert_eq!(r.bindings().get(A_FIRE)[0], Key::NONE, "Fire (same Combat group) lost the left button; nothing is swapped");
    assert!(!r.listening());
}
#[test] fn an_accidental_double_click_does_not_bind_the_left_button() {
    let mut r = rig();
    let p = r.must("Space Bar");
    r.click_at(p); r.click_at(p); // second click inside the grace period
    assert!(r.listening() && r.bindings().get(A_JUMP)[0] == k("SpaceBar"));
}
#[test] fn wheel_binds_and_does_not_scroll_the_list() {
    let mut r = rig();
    let off = |r: &Rig| r.st.scroll.values().next().map(|s| s.offset).unwrap();
    let before = off(&r);
    r.click("Space Bar");
    let p = r.must("PRESS ANY KEY..."); r.ev(Event::MouseMove(p.0, p.1)); r.frame();
    r.ev(Event::Wheel(-1.)); r.frames(2);
    assert_eq!(r.bindings().get(A_JUMP)[0], k("MouseScrollDown"));
    assert_eq!(off(&r), before, "the notch was consumed by the capture, not by the scroll area");
    // without a listening button the wheel still scrolls
    r.ev(Event::MouseMove(p.0, p.1)); r.frame(); r.ev(Event::Wheel(-1.)); r.frames(2);
    assert!(off(&r) > before, "wheel scrolls the list as usual when not listening");
}
#[test] fn modifier_keys_bind_left_and_right() {
    for (vk, want) in [(0xA0u16, "LeftShift"), (0xA1, "RightShift"), (0xA2, "LeftControl"), (0xA3, "RightControl"), (0xA4, "LeftAlt"), (0xA5, "RightAlt")] {
        let mut r = rig(); r.click("Space Bar"); r.key(vk);
        assert_eq!(r.bindings().get(A_JUMP)[0], k(want), "{want}");
    }
}
#[test] fn backspace_and_delete_clear_the_binding() {
    for vk in [0x08u16, 0x2E] {
        let mut r = rig(); r.click("Space Bar"); r.key(vk);
        assert_eq!(r.bindings().get(A_JUMP), [Key::NONE, Key::NONE]);
    }
}
#[test] fn unsupported_key_is_refused_politely() {
    let mut r = rig(); r.click("Space Bar"); r.key(0x5B);
    assert!(!r.listening()); assert_eq!(r.bindings().get(A_JUMP)[0], k("SpaceBar"));
}
#[test] fn clicking_elsewhere_cancels() {
    let mut r = rig();
    r.click("Space Bar"); assert!(r.listening());
    let label = r.must("Move Backward"); r.click_at(label);
    assert!(!r.listening() && r.page.listening().is_none()); assert_eq!(r.bindings().get(A_JUMP)[0], k("SpaceBar"));
}
#[test] fn focus_loss_and_timeout_cancel() {
    let mut r = rig(); r.click("Space Bar"); r.ev(Event::FocusLost); r.frames(2);
    assert!(!r.listening() && r.page.listening().is_none());
    let mut r = rig(); r.click("Space Bar"); r.frames((LISTEN_TIMEOUT * 60.) as usize + 30);
    assert!(!r.listening() && r.page.listening().is_none(), "gamepad users cannot press Esc: the prompt times out");
    assert_eq!(r.bindings().get(A_JUMP)[0], k("SpaceBar"));
}
#[test] fn scrolling_the_prompt_out_of_view_cancels_instead_of_freezing_the_menu() {
    let mut r = rig();
    r.click("Space Bar"); assert!(r.listening());
    r.scroll(3000.);
    assert!(!r.listening(), "a listening button that is no longer drawn must not leave the toolkit listening");
    r.ev(Event::Nav(NavKey::Down)); r.frame();
    assert!(r.st.focused_id().is_some(), "navigation works again");
}
#[test] fn the_unbind_button_clears_both_slots_of_a_row() {
    // 1.8 rows end with a Secondary-M unbind button (T-SlotIcon-Remove-32); there is no per-row Reset and no Reset All button.
    let mut r = rig();
    let jump_y = r.must("Jump").1;
    let x_at = |r: &Rig| r.draw.brushes().into_iter().filter(|(b, _)| *b == BrushId::Tex(TexId::RemoveIcon)).map(|(_, rc)| (rc.x + rc.w * 0.5, rc.y + rc.h * 0.5))
        .find(|p| (p.1 - jump_y).abs() < 4.).expect("unbind button icon on the Jump row");
    assert_eq!(r.bindings().get(A_JUMP), [k("SpaceBar"), Key::NONE]);
    let p = x_at(&r); r.click_at(p);
    assert_eq!(r.bindings().get(A_JUMP), [Key::NONE, Key::NONE]);
    assert!(!r.has_text("Reset") && !r.has_text("Reset All Bindings"), "the old Reset buttons do not exist in 1.8");
}
#[test] fn reset_to_default_button_text_is_in_the_footer_not_the_page() {
    let mut r = rig();
    r.click("Space Bar"); r.key(0x43);
    r.page.reset_to_default(&mut r.sx); r.frames(2);
    assert_eq!(r.bindings(), Bindings::default(), "footer Reset To Default restores the bindings (working copy)");
}
#[test] fn keyboard_focus_clear_key_unbinds() {
    let mut r = rig();
    let id = id_with(id_with(id_with(id_of("rig"), id_of("input")), id_of("binds")), id_of(&format!("key.{A_JUMP}.0")));
    r.st.focus = Some(id); r.st.focus_visible = true;
    r.ev(Event::Nav(NavKey::Clear)); r.frames(2);
    assert_eq!(r.bindings().get(A_JUMP), [Key::NONE, Key::NONE]);
}
#[test] fn keyboard_activation_starts_listening() {
    let mut r = rig();
    let id = id_with(id_with(id_with(id_of("rig"), id_of("input")), id_of("binds")), id_of(&format!("key.{A_JUMP}.0")));
    r.st.focus = Some(id); r.st.focus_visible = true;
    r.ev(Event::Nav(NavKey::Activate)); r.frames(2);
    assert_eq!(r.page.listening(), Some((A_JUMP, 0)));
    r.key(0x45);
    assert_eq!(r.bindings().get(A_JUMP)[0], k("E"));
}
#[test] fn every_binding_row_looks_the_same_wired_or_not() {
    // the original has no greyed rows, badges or "not available" lines: a binding without a backend yet is an ordinary working row
    let r = rig();
    let color = |t: &str| r.draw.cmds.iter().find_map(|c| if let Cmd::Text { text, color, .. } = c { if text == t { Some(*color) } else { None } } else { None }).unwrap_or_else(|| panic!("{t} not drawn"));
    assert_eq!(color("Jump"), super::super::slate::label_color());
    assert_eq!(color("Reload"), super::super::slate::label_color());
}
#[test] fn hovering_a_binding_describes_it() {
    let mut r = rig();
    let p = r.must("Jump"); r.ev(Event::MouseMove(p.0, p.1)); r.frames(2);
    assert!(r.draw.text_strings().iter().any(|t| t.contains("Bindings for Jump")), "{:?}", r.draw.text_strings());
    let reload = { r.scroll(0.); r.must("Reload") }; r.ev(Event::MouseMove(reload.0, reload.1)); r.frames(2);
    assert!(r.draw.text_strings().iter().any(|t| t.contains("Bindings for Reload")), "{:?}", r.draw.text_strings());
    assert!(!r.draw.text_strings().iter().any(|t| t.contains("Not available") || t.contains("Applies when") || t.contains("Fort.InputOptions") || t.contains("assumed") || t.contains("Default:")), "only the original tooltip text");
}
#[test] fn reset_to_default_restores_the_tab_in_the_working_copy() {
    let mut r = rig();
    r.click("Space Bar"); r.key(0x43);
    assert!(r.page.reset_prompt().is_some());
    r.page.reset_to_default(&mut r.sx);
    assert_eq!(r.sx.settings_clone().input.bindings, Settings::default().input.bindings);
    r.frames(2);
}
#[test] fn the_input_tab_lists_only_key_bindings_in_1_8() {
    // Mouse Sensitivity, Invert View, Crouch Mode and Toggle Sprint are Game tab rows in 1.8 (OptionsMenuInput only has binding rows)
    let mut r = Rig::new(SZ.0, SZ.1); r.scroll(0.);
    for gone in ["Mouse Sensitivity X", "Mouse Sensitivity Y", "Invert View", "Crouch Mode", "Toggle Sprint", "Reset All Bindings"] { assert!(!r.has_text(gone), "{gone} is not an Input tab row"); }
    assert!(r.has_text("Move Forward") || r.has_text("Jump"));
}
#[test] fn renders_at_every_size_inside_the_client() {
    for (w, h) in [(1920., 1080.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.), (320., 200.), (1., 1.)] {
        let mut r = Rig::new(w, h);
        let fits = Viewport::new(w, h).slate_size().0 >= 1280.; // the 1.8 layout is fixed (no reflow): only required not to panic when narrower
        for off in [0., 300., 900., 2000., 9000.] {
            r.scroll(off);
            r.click_at((w * 0.5, h * 0.5)); r.key(0x1B);
            for c in &r.draw.cmds {
                if let Cmd::Text { x, text, font, px, .. } = c { let tw = r.fonts.measure(*font, text, *px as f32); assert!(!fits || (*x >= -1. && x + tw <= w + 1.), "{w}x{h}: {text:?} at x {x} width {tw}"); }
            }
        }
        r.frames(3);
    }
}
#[test] fn the_tooltip_is_never_empty_while_a_binding_is_hovered() {
    let mut r = rig();
    let p = r.must("Move Backward"); r.ev(Event::MouseMove(p.0, p.1)); r.frames(2);
    let all = r.draw.text_strings().join(" | ");
    assert!(all.contains("Bindings for Move Backward"), "{all}");
}


// ---- end to end through the real Settings screen, footer Apply and the apply layer --------------------------------------------
struct Mock { live: settings::InputLive, applies: usize }
impl bridge::ApplyBackend for Mock {
    fn apply(&mut self, s: &mut Settings, _p: &mut dyn settings::PlatformHost, _save: bool) -> settings::ApplyReport {
        struct V; impl settings::VideoHooks for V {}
        self.applies += 1;
        let mut r = s.apply_video(&mut V);
        r.merge(s.apply_input(&mut self.live));
        r.merge(s.apply_audio(&mut settings::StoredOnlyAudio));
        r.merge(s.apply_game());
        s.drain_changes();
        r
    }
    fn write_file(&mut self, _s: &Settings) -> Result<(), String> { Ok(()) }
}
struct Full { m: MenuSystem, w: f32, h: f32, s: Settings, platform: settings::NullPlatform, game: Mock }
impl Full {
    fn new(w: f32, h: f32) -> Self {
        let game = Mock { live: settings::InputLive::default(), applies: 0 };
        let mut s = Settings::default(); s.drain_changes();
        let mut m = MenuSystem::new(fonts()); super::super::register_all_into(&mut m); m.fade_time = 0.; m.open();
        m.push(Box::new(crate::menu::pause::SettingsScreen::new()));
        let mut f = Full { m, w, h, s, platform: settings::NullPlatform::static_16_9(), game }; f.frames(2);
        let i = f.m.pages().iter().position(|p| p.id() == "input").unwrap(); f.m.set_settings_tab(i); f.frames(4); f
    }
    fn frame(&mut self) -> Vec<Cmd> { let c = self.m.frame_with(&mut UiContext::new(&mut self.s, &mut self.platform, &mut self.game), self.w, self.h, 1. / 60.).cmds.clone(); c }
    fn frames(&mut self, n: usize) { for _ in 0..n { self.frame(); } }
    fn ev(&mut self, e: Event) { self.m.event(e); self.frame(); }
    fn find(&mut self, text: &str) -> Option<(f32, f32)> {
        let cmds = self.frame();
        cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text: t, font, px, .. } = c { if t == text { Some((x + self.m.fonts().measure(*font, t, *px as f32) * 0.5, top + box_h * 0.5)) } else { None } } else { None })
    }
    fn click_at(&mut self, p: (f32, f32)) { self.ev(Event::MouseMove(p.0, p.1)); self.ev(Event::MouseDown(MouseButton::Left, p.0, p.1)); self.ev(Event::MouseUp(MouseButton::Left, p.0, p.1)); self.frames(2); }
    fn click(&mut self, text: &str) { let p = self.find(text).unwrap_or_else(|| panic!("no text {text:?}")); self.click_at(p); }
    fn scroll_down(&mut self, notches: f32) { self.ev(Event::MouseMove(self.w * 0.5, self.h * 0.4)); self.ev(Event::Wheel(-notches)); self.frames(3); }
}
#[test] fn rebinding_through_the_settings_screen_reaches_the_live_input_after_apply() {
    let mut f = Full::new(1920., 1080.);
    assert!(f.m.is_open() && f.m.stack_names().contains(&"settings".to_owned()));
    f.click("Space Bar");
    assert!(f.m.state().is_listening());
    // Esc while listening only cancels the prompt: the Settings screen stays
    f.ev(Event::Nav(NavKey::Back)); f.ev(Event::RawKey(0x1B)); f.frames(2);
    assert!(!f.m.state().is_listening() && f.m.is_open() && f.m.stack_names().contains(&"settings".to_owned()), "Esc cancelled the prompt but kept the menu");
    f.click("Space Bar"); f.ev(Event::RawKey(0x43)); f.frames(4);
    // 1.8 Input tab: no Apply handler (footer = Reset To Default + Back), a rebind takes effect at once
    assert!(f.find("Apply").is_none(), "the Input tab has no Apply button");
    let live = &f.game.live.bindings;
    assert_eq!(live.get(A_JUMP), [k("C"), Key::NONE], "the live input snapshot has the new key");
    let down_with = |pressed: u16| live.down(A_JUMP, |vk| vk == pressed);
    assert!(down_with(0x43) && !down_with(0x20), "jump now polls C and no longer Space");
    assert!(f.game.applies >= 1);
    // wheel as a binding becomes a pollable pseudo key in the live input
    f.scroll_down(0.);
    f.click("C"); let p = f.find("PRESS ANY KEY...").unwrap(); f.ev(Event::MouseMove(p.0, p.1)); f.ev(Event::Wheel(1.)); f.frames(4);
    let live = &f.game.live.bindings;
    assert!(live.down(A_JUMP, |vk| vk == settings::WHEEL_UP_CODE), "wheel-up is polled through the pseudo code");
}
#[test] fn escape_after_cancelling_still_leaves_the_settings_screen() {
    let mut f = Full::new(1920., 1080.);
    f.click("Space Bar"); f.ev(Event::Nav(NavKey::Back)); f.ev(Event::RawKey(0x1B)); f.frames(2);
    assert!(f.m.stack_names().contains(&"settings".to_owned()));
    f.ev(Event::Nav(NavKey::Back)); f.frames(3);
    assert!(!f.m.stack_names().contains(&"settings".to_owned()), "a second Esc goes back as usual (no unsaved changes)");
}
#[test] fn a_binding_edit_is_already_applied_when_back_leaves_without_a_prompt() {
    // 1.8: the Input tab has no Apply handler, so the rebind is live before Back; Back pops the screen and there is no unsaved-changes dialog
    let mut f = Full::new(1920., 1080.);
    f.click("Space Bar"); f.ev(Event::RawKey(0x43)); f.frames(4);
    f.ev(Event::Nav(NavKey::Back)); f.frames(3);
    assert!(!f.m.stack_names().iter().any(|n| n == "settings" || n == "settings-modal"), "left without a prompt: {:?}", f.m.stack_names());
    assert!(!f.m.session().is_dirty(), "nothing pending");
    assert_eq!(f.game.live.bindings.get(A_JUMP)[0], k("C"), "the rebind survived Back");
}

// DROPPED (inherently Win32): `windows_modifier_messages_resolve_left_and_right` resolved VK_SHIFT/CONTROL/MENU to the left/right virtual keys from the
// WM_KEYDOWN lParam scan code / extended bit (`translate_win32`). The window layer owns that message and now sends `Event::RawKey` with the resolved code.
// What the page does with the resolved codes is still checked here:
#[test] fn resolved_modifier_codes_are_bindable_keys() {
    for vk in [0xA0u16, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5] { assert!(matches!(capture_key(vk), Capture::Bind(_))); }
}