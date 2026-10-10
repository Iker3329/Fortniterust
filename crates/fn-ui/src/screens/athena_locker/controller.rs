//! Pointer / keyboard interaction of the BR lobby screens, routed through the UMG hit test (`hit`): hover / press button states, tab switching, the matchmaking
//! widget's rotators and Play button, the Locker's slots, picker tiles and `Cancel`. Pure state (no window, no audio): it mutates a [`LobbyState`] and returns
//! [`UiEvent`]s for the application (sounds, Play, equipping an item).
//!
//! Behaviour sources: `CommonButton` click = press and release over the same enabled button (`UCommonButton::HandleButtonClicked`); the tab list selects the tab whose
//! button was clicked (`FortTabListWidgetBase::HandleTabButtonClicked`); `AthenaRotator` arrows step the rotator selection with wrap-around (`UCommonRotator::ShiftTextLeft/Right`);
//! the slot / tile / cancel flows are the Kismet of `AthenaCustomizationTab` (see `athena_locker`). Button sounds are the widget's serialized
//! `HoveredSlateSoundOverride` / `PressedSlateSoundOverride` (the object path is returned, the application plays it).
use super::hit;
use super::{Slot, INTRO_PICKER_END};
use crate::layout::Rect;
use crate::screens::athena_lobby::{AthenaLobbyUi, Frame, LobbyState};

#[derive(Clone, Debug, PartialEq)]
pub enum UiEvent {
    TabSelected(usize),
    Play,
    Cancel,
    PlaylistChanged(usize),
    FillChanged(usize),
    PickerOpened(Slot),
    PickerClosed,
    /// an item tile was clicked: `index` into `LockerView::items[slot]`
    Equip { slot: Slot, index: usize },
    /// `HoveredSlateSoundOverride.ResourceObject` of the button the pointer entered
    HoverSound(String),
    /// `PressedSlateSoundOverride.ResourceObject` of the button that was pressed
    PressSound(String),
}

/// What a button is (the widget roles the screens react to).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role { Tab(usize), Play, Cancel, PlaylistArrow(i8), FillArrow(i8), Slot(Slot), Tile(Slot, usize), Other }

#[derive(Default)]
pub struct Controller {
    hover: Option<usize>,
    pressed: Option<usize>,
    pub events: Vec<UiEvent>,
}

fn sound_of(lobby: &AthenaLobbyUi, id: usize, key: &str) -> Option<String> {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if let Some(p) = lobby.ui.w[c].props.get(key).and_then(|s| s.get("ResourceObject")).and_then(|v| v.as_str()) { return Some(p.to_owned()); }
        cur = lobby.ui.w[c].parent;
    }
    None
}

impl Controller {
    pub fn new() -> Controller { Controller::default() }
    pub fn take_events(&mut self) -> Vec<UiEvent> { std::mem::take(&mut self.events) }

    /// Role of a button widget.
    pub fn role(lobby: &AthenaLobbyUi, id: usize) -> Role {
        if let Some(i) = lobby.tab_widgets().iter().position(|&t| t == id) { return Role::Tab(i); }
        if let Some(l) = lobby.locker() {
            if let Some(s) = l.slot_of(id) { return Role::Slot(s); }
            if let Some((s, i)) = l.tile_of(id) { return Role::Tile(s, i); }
        }
        let ui = &lobby.ui;
        if ui.find_by_name("PlayButton") == Some(id) { return Role::Play; }
        if ui.find_by_name("CancelButton") == Some(id) { return Role::Cancel; }
        for (name, fill) in [("PlaylistRotator", false), ("TeamFillRotator", true)] {
            if let Some(r) = ui.find_by_name(name) {
                if ui.find_in(r, "ButtonLeft") == Some(id) { return if fill { Role::FillArrow(-1) } else { Role::PlaylistArrow(-1) }; }
                if ui.find_in(r, "ButtonRight") == Some(id) { return if fill { Role::FillArrow(1) } else { Role::PlaylistArrow(1) }; }
            }
        }
        Role::Other
    }

    fn button_at(lobby: &AthenaLobbyUi, frame: &Frame, x: f32, y: f32) -> Option<usize> { hit::button_under(&lobby.ui, lobby.root, &frame.id_rects, x, y) }

    /// The pointer moved (pixels): hover state + hover sound.
    pub fn pointer_move(&mut self, lobby: &AthenaLobbyUi, frame: &Frame, st: &mut LobbyState, x: f32, y: f32) {
        let over = Self::button_at(lobby, frame, x, y);
        if over != self.hover {
            if let Some(id) = over { if let Some(c) = sound_of(lobby, id, "HoveredSlateSoundOverride") { self.events.push(UiEvent::HoverSound(c)); } }
            self.hover = over;
        }
        st.hover_widget = over;
        st.hovered_tab = over.and_then(|id| match Self::role(lobby, id) { Role::Tab(i) => Some(i), _ => None });
        st.play_hovered = over.is_some_and(|id| Self::role(lobby, id) == Role::Play);
        st.cancel_hovered = over.is_some_and(|id| Self::role(lobby, id) == Role::Cancel);
    }

    /// Left mouse button (pixels). A click is a press and a release over the same button.
    pub fn pointer_button(&mut self, lobby: &AthenaLobbyUi, frame: &Frame, st: &mut LobbyState, x: f32, y: f32, down: bool) {
        let over = Self::button_at(lobby, frame, x, y);
        if down {
            self.pressed = over;
            st.press_widget = over;
            if let Some(id) = over { if let Some(c) = sound_of(lobby, id, "PressedSlateSoundOverride") { self.events.push(UiEvent::PressSound(c)); } }
            return;
        }
        let was = self.pressed.take();
        st.press_widget = None;
        if let (Some(a), Some(b)) = (was, over) { if a == b { self.click(lobby, st, b); } }
    }

    fn click(&mut self, lobby: &AthenaLobbyUi, st: &mut LobbyState, id: usize) {
        match Self::role(lobby, id) {
            Role::Tab(i) => {
                if st.selected_tab != i {
                    // `OnDeactivated` of the Locker tab: `EndPickingCustomization(false)`
                    st.locker.picking = None;
                    st.locker.picker_t = 0.0;
                    st.selected_tab = i;
                    self.events.push(UiEvent::TabSelected(i));
                }
            }
            Role::Play => self.events.push(UiEvent::Play),
            Role::Cancel => self.events.push(UiEvent::Cancel),
            Role::PlaylistArrow(d) => {
                st.playlist = (st.playlist as i32 + d as i32).rem_euclid(3) as usize;
                self.events.push(UiEvent::PlaylistChanged(st.playlist));
            }
            Role::FillArrow(_) => {
                // only the Squad playlist lets the player change the fill (`team_fill_rule`: the rotator is disabled otherwise, and disabled buttons are never returned by the hit test)
                st.team_fill = 1 - st.team_fill.min(1);
                self.events.push(UiEvent::FillChanged(st.team_fill));
            }
            Role::Slot(s) => self.open_picker(lobby, st, s),
            Role::Tile(s, i) => {
                st.locker.equipped[s.index()] = Some(i);
                self.events.push(UiEvent::Equip { slot: s, index: i });
                // `OnCosmeticChangeSaveEvent` -> `EndPickingCustomization(true)`
                self.close_picker(st);
            }
            Role::Other => {}
        }
    }

    /// `BeginPickingCustomization(category)`: the picker slides in, centred on the equipped item (`CenterSelectedItemTileWidget`).
    pub fn open_picker(&mut self, lobby: &AthenaLobbyUi, st: &mut LobbyState, slot: Slot) {
        st.locker.picking = Some(slot);
        let cols = lobby.locker().map(|l| l.grid.0).unwrap_or(1).max(1);
        let vis = lobby.locker().map(|l| l.grid.1).unwrap_or(1).max(1);
        let row = st.locker.equipped[slot.index()].map(|i| i / cols).unwrap_or(0);
        st.locker.scroll_row = row.saturating_sub(vis / 2);
        self.events.push(UiEvent::PickerOpened(slot));
    }

    /// `EndPickingCustomization`.
    pub fn close_picker(&mut self, st: &mut LobbyState) {
        if st.locker.picking.take().is_some() { self.events.push(UiEvent::PickerClosed); }
    }

    /// `HandleCancel` (the "Cancel" input action, Escape): ends picking. Returns whether the key was used.
    pub fn cancel(&mut self, st: &mut LobbyState) -> bool {
        if st.locker.picking.is_some() { self.close_picker(st); true } else { false }
    }

    /// Mouse wheel over the picker: whole rows. `rows` > 0 scrolls down.
    pub fn wheel(&mut self, lobby: &AthenaLobbyUi, frame: &Frame, st: &mut LobbyState, x: f32, y: f32, rows: i32) {
        let Some(l) = lobby.locker() else { return };
        let Some(tv) = l.tile_view() else { return };
        let inside = frame.id_rects.get(tv).is_some_and(|r: &Rect| r.contains(x, y));
        if !inside || st.locker.picking.is_none() { return; }
        let (_, vis, total) = l.grid;
        let max = total.saturating_sub(vis) as i32;
        st.locker.scroll_row = (st.locker.scroll_row as i32 + rows).clamp(0, max) as usize;
    }

    /// Advance the `IntroPicker` animation (it plays forward while picking, backward otherwise).
    pub fn tick(&mut self, st: &mut LobbyState, dt: f32) {
        let l = &mut st.locker;
        l.picker_t = if l.picking.is_some() { (l.picker_t + dt).min(INTRO_PICKER_END) } else { (l.picker_t - dt).max(0.0) };
    }
}
