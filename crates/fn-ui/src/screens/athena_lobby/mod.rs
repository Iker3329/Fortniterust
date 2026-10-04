//! The Fortnite 1.8 Battle Royale lobby screen (UMG) decoded from the cooked widget blueprints and rendered through the UI draw list.
//!
//! The generic interpreter (`umg`, `layout`, `paint`, `styles`, `slate_mat`) renders the packages as they are. The code in THIS file only reproduces what the
//! 1.8 client does natively around them; each piece names its source:
//! * composition: `FrontEnd::InitForSubGame` (Kismet) creates `AthenaTabsScreen_C` for `ESubGame::Athena` and pushes it into `MainContentStack`;
//! * tab buttons: `HorizontalTabList_C` (`FortTabListWidgetBase`) creates one `TabButtonType` per `PreregisteredTabInfoArray` entry that is not `bHidden`,
//!   `AthenaTabsScreen::HandleTabCreated` (Kismet) then calls `SetStyle(ButtonStyle-Tab-Main)`, `ButtonText = TextToUpper(ButtonText)` and `ShowText`;
//! * the first tab content is `AthenaLobby_C` (`TabContentType` of the first registration) in `TopTabContentWidgetSwitcher`;
//! * bottom bar buttons: `InputReflector::OnButtonAdded` (Kismet) for the actions `AthenaLobby::InitializeInput` / `RefreshInput` (Kismet) registered, text from
//!   `InputActionDataTable` (`DisplayName`), visibility from the row's keyboard state (`OverrrideStateForInputType[0]`) unless the handler state was set;
//! * rotator labels: `AthenaMatchmakingWidget::SetAutoFillOptions` (Kismet) and the native `FortAthenaMatchmakingWidget` playlist strings (locres ns
//!   `Fortnite.FortAthenaMatchmakingWidget`, keys also present as UTF-16 literals in the exe at RVA 0x4ab1d68 / 0x4ade360);
//! * player pads: `LobbyPlayerPadTop` / `LobbyPlayerAddPlayer` (the widgets the lobby pad actors host; placed by the caller at projected screen points).
pub mod layout;
pub mod paint;
pub mod slate_mat;
pub mod store;
pub mod styles;
pub mod umg;
#[cfg(test)]
mod tests;

use crate::draw::RenderList;
use crate::layout::Rect;
use crate::scene::UiContext;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use store::Store;
use styles::BtnState;
use umg::{Ui, Vis};

pub const FRONTEND: &str = "/Game/UI/Foundation/StateLayouts/FrontEnd.FrontEnd_C";
pub const TABS_SCREEN: &str = "/Game/Athena/UI/Frontend/AthenaTabsScreen.AthenaTabsScreen_C";
pub const LOBBY: &str = "/Game/Athena/UI/Frontend/AthenaLobby.AthenaLobby_C";
pub const ICON_TAB_BUTTON: &str = "/Game/UI/Foundation/Buttons/IconTabButton.IconTabButton_C";
pub const PAD_TOP: &str = "/Game/UI/Frontend/Lobby/PlayerPad/LobbyPlayerPadTop.LobbyPlayerPadTop_C";
pub const PAD_ADD: &str = "/Game/UI/Frontend/Lobby/PlayerPad/LobbyPlayerAddPlayer.LobbyPlayerAddPlayer_C";
const INPUT_ACTIONS: &str = "/Game/UI/Foundation/Input/InputActionDataTable";

/// A world-space lobby pad widget (screen-space `UWidgetComponent`) placed by the caller at the projected component location (pixels).
#[derive(Clone, Debug)]
pub enum PadKind {
    /// `LobbyPlayerPadTop`: crown (party leader), name (None = the serialized design-time text), ready state
    Player { name: Option<String>, leader: bool, ready: bool },
    /// `LobbyPlayerAddPlayer`: the empty party slot "+" marker
    Empty,
}
#[derive(Clone, Debug)]
pub struct Pad {
    /// projected location of the pad's `UWidgetComponent` (pixels)
    pub x: f32, pub y: f32,
    /// `UWidgetComponent.Pivot` (read from the pad blueprint: name plate (0.5, 1.0), "+" marker default (0.5, 0.5)); the widget's desired-size box is placed at `anchor - size * pivot`
    pub pivot: [f32; 2],
    pub kind: PadKind }

/// Runtime-driven parts of the screen (everything else is the widget data). Defaults = the settled state of a freshly entered lobby.
#[derive(Clone, Debug)]
pub struct LobbyState {
    /// index into the visible tab registrations (0 = Lobby)
    pub selected_tab: usize,
    pub hovered_tab: Option<usize>,
    /// `AthenaRotator` of the playlist: 0 = Solo, 1 = Duo, 2 = Squad
    pub playlist: usize,
    /// `TeamFillRotator` selection the player chose for Squad (`bCurrentSquadFill`): 0 = first `SetAutoFillOptions` entry ("No Fill"), 1 = second ("Fill").
    /// Solo / Duo force their own index (see [`team_fill_rule`]).
    pub team_fill: usize,
    /// `PartyGameState::IsLocalPlayerPartyLeader` (a local party of one is its own leader): non-leaders get both rotators disabled
    pub party_leader: bool,
    /// `FortPrivateAccountInfo.MtxBalance` (V-Bucks) shown by `MTXButton.Amount` (`CommonNumericTextBlock::SetCurrentValue(float)` from `Construct` /
    /// `UpdateAmount`); runtime account data, default 0
    pub mtx_balance: i64,
    /// `PlayerPanel` ("PLAYER MENU"): hidden until a lobby player pad is hovered / clicked (`AthenaLobby` Kismet)
    pub player_panel: bool,
    pub play_hovered: bool,
    /// world-space pad widgets (name plates / empty party slots) at projected screen points
    pub pads: Vec<Pad>,
    /// the `CommonButton` under the pointer / pressed (widget ids of the composed tree, set by `athena_locker::controller`): hover and press button states
    pub hover_widget: Option<usize>,
    pub press_widget: Option<usize>,
    /// the Locker tab (`athena_locker`): owned items, equipped items, the open picker
    pub locker: crate::screens::athena_locker::LockerView,
}
impl Default for LobbyState {
    fn default() -> Self { LobbyState { selected_tab: 0, hovered_tab: None, playlist: 0, team_fill: 0, party_leader: true, mtx_balance: 0, player_panel: false, play_hovered: false, pads: vec![], hover_widget: None, press_widget: None, locker: Default::default() } }
}

/// The native `FortAthenaMatchmakingWidget` refresh (exe RVA 0x2eb0a20, called from `OnPlaylistRotatorChanged` 0x2e96470 / the party-state updates) for
/// `CurrentPlaylist` (`+0x250`: 0 Solo, 1 Duo, 2 Squad) and `bCurrentSquadFill` (`+0x251`), on `TeamFillRotator` (`+0x288`) / `PlaylistRotator` (`+0x280`):
/// * Squad (`cmp [this+0x250], 2`): `TeamFillRotator.SetIsEnabled(true)` (0x3934ad0) + `SetSelectedItem(bCurrentSquadFill)` (0x3954d20);
/// * otherwise `SetSelectedItem(CurrentPlaylist == 1)` (Duo = "Fill", Solo = "No Fill") then `SetIsEnabled(false)` (0x39341c0);
/// * then, unless the local player is the party leader (`PartyGameState::IsLocalPlayerPartyLeader`, 0x3bd0ca0), both rotators are disabled.
/// Returns (selected index, enabled).
pub fn team_fill_rule(playlist: usize, squad_fill: usize, party_leader: bool) -> (usize, bool) {
    let (idx, en) = match playlist { 2 => (squad_fill.min(1), true), 1 => (1, false), _ => (0, false) };
    (idx, en && party_leader)
}

/// `FText::AsNumber` of an integer (default formatting options: digit grouping with ",").
fn group_digits(v: i64) -> String {
    let s = v.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); }
        out.push(c);
    }
    if v < 0 { format!("-{out}") } else { out }
}

/// One drawn frame.
pub struct Frame {
    pub list: RenderList,
    /// `BackgroundBlur` regions (pixels) the backend may blur behind the UI
    pub blur: Vec<paint::BlurRegion>,
    /// rectangle (pixels) of every widget by plain widget name (first match wins)
    pub rects: HashMap<String, Rect>,
    pub tabs: Vec<(String, Rect)>,
    /// rectangle of each pad widget, in the order of `LobbyState::pads`
    pub pads: Vec<Rect>,
    /// pixel rectangle of every widget of the composed tree by widget id (for hit testing, `athena_locker::hit`)
    pub id_rects: Vec<Rect>,
    /// assets / materials / strings that could not be resolved (empty when everything was found)
    pub missing: Vec<String>,
}
impl Frame {
    pub fn rect(&self, name: &str) -> Option<Rect> { self.rects.get(name).copied() }
}

pub struct AthenaLobbyUi {
    pub ui: Ui,
    pub root: usize,
    tabs: Vec<usize>,
    tab_labels: Vec<String>,
    /// `TopTabContentWidgetSwitcher` of `AthenaTabsScreen` and, per visible tab registration, its `TabContentType` class and the composed page (pages are composed
    /// the first time their tab is selected, like the tab list creating the content widget on demand)
    switcher: usize,
    tab_content: Vec<String>,
    pages: Vec<Option<usize>>,
    /// the Locker page (`AthenaCustomizationTab_C`) once composed
    locker: Option<crate::screens::athena_locker::LockerWidgets>,
    last_hover: [Option<usize>; 2],
    /// pooled pad widgets: (pad index, is_player) -> root widget
    pad_roots: HashMap<(usize, bool), usize>,
    mats: HashMap<String, Option<Rc<slate_mat::SlateMat>>>,
    card_mats: crate::screens::athena_locker::card::CardMats,
    pub composition_log: Vec<String>,
}

impl AthenaLobbyUi {
    pub fn load(install: &Path) -> Result<AthenaLobbyUi, String> {
        let store = Rc::new(Store::open(install)?);
        Self::from_store(store)
    }

    pub fn from_store(store: Rc<Store>) -> Result<AthenaLobbyUi, String> {
        let mut ui = Ui::new(store);
        let root = ui.add_class_instance(FRONTEND, None, "", json!({}), None).ok_or("cannot compose FrontEnd")?;
        // FrontEnd::InitForSubGame (Athena): AthenaTabsScreen_C pushed into MainContentStack
        let stack = ui.find_in(root, "MainContentStack").ok_or("FrontEnd has no MainContentStack")?;
        let tabs_screen = ui.add_class_instance(TABS_SCREEN, Some(stack), "PanelSlot", json!({}), None).ok_or("cannot compose AthenaTabsScreen")?;
        // the first tab content (`TabContentType` of the Lobby registration): AthenaLobby_C in the TopTabContentWidgetSwitcher
        let switcher = ui.var(tabs_screen, "TopTabContentWidgetSwitcher").or_else(|| ui.find_in(tabs_screen, "TopTabContentWidgetSwitcher")).ok_or("no TopTabContentWidgetSwitcher")?;
        let lobby_page = ui.add_class_instance(LOBBY, Some(switcher), "WidgetSwitcherSlot", json!({}), None).ok_or("cannot compose AthenaLobby")?;
        // HorizontalTabList_C: one TabButtonType (IconTabButton_C) per registration that is not bHidden
        let list = ui.var(tabs_screen, "TopTabList").or_else(|| ui.find_in(tabs_screen, "TopTabList")).ok_or("no TopTabList")?;
        let tab_box = ui.var(list, "TabButtonBox").or_else(|| ui.find_in(list, "TabButtonBox")).ok_or("no TabButtonBox")?;
        let style = ui.w[list].props.get("ButtonStyle").cloned().unwrap_or(json!(null));
        let pad = ui.w[list].props.get("TabButtonPadding").cloned().unwrap_or(json!({}));
        let regs = ui.w[list].props.get("PreregisteredTabInfoArray").and_then(|a| a.as_array()).cloned().unwrap_or_default();
        let mut tabs = vec![];
        let mut labels = vec![];
        let mut content = vec![];
        for r in regs {
            if r.get("bHidden").and_then(|b| b.as_bool()).unwrap_or(false) { continue; }
            content.push(r.get("TabContentType").and_then(|c| c.as_str()).unwrap_or("").to_owned());
            let dn = r.get("TabLabelInfo").and_then(|l| l.get("DisplayName")).cloned().unwrap_or(json!(null));
            // AthenaTabsScreen::HandleTabCreated: SetStyle(ButtonStyle-Tab-Main); ButtonText = TextToUpper(ButtonText); ShowText()
            let upper = ui.text_of(&dn).unwrap_or_default().to_uppercase();
            let inst = json!({ "Style": style, "ButtonText": null });
            let t = ui.add_class_instance(ICON_TAB_BUTTON, Some(tab_box), "HorizontalBoxSlot", json!({ "Padding": pad }), Some(inst)).ok_or("cannot compose IconTabButton")?;
            if let Some(tw) = ui.var(t, "CenterButtonTextWidget") { ui.w[tw].text = Some(upper.clone()); ui.w[tw].vis = if upper.is_empty() { Vis::Collapsed } else { Vis::SelfHitTestInvisible }; }
            tabs.push(t);
            labels.push(upper);
        }
        let mut pages = vec![None; tabs.len()];
        if let Some(p) = pages.first_mut() { *p = Some(lobby_page); }
        let mut me = AthenaLobbyUi { ui, root, tabs, tab_labels: labels, switcher, tab_content: content, pages, locker: None, last_hover: [None; 2], pad_roots: HashMap::new(), mats: HashMap::new(), card_mats: Default::default(), composition_log: vec![] };
        me.reflect_input_actions()?;
        me.composition_log = me.ui.log.clone();
        Ok(me)
    }

    /// `InputReflector` (bottom bar): one `IconTextButton_C` per registered input action that is visible for mouse + keyboard.
    /// Actions registered by `AthenaLobby::InitializeInput` (Kismet): Scroll (no handler), Cancel, PlaylistChange, FillChange, AthenaNews. `RefreshInput`
    /// (Kismet) sets the handler state of Cancel (`ShouldAddCancel ? 2 : 3`), Scroll (`ShouldAddScroll ? 2 : 3`) and News (0 = Enabled); the others keep the
    /// row state of the `InputActionDataTable` (`OverrrideStateForInputType[0]`, mouse + keyboard). States: 0 Enabled, 1 Disabled, 2 Hidden, 3 HiddenAndDisabled.
    /// `ShouldAddCancel` / `ShouldAddScroll` are read as false in the idle lobby (matchmaking / scrolling lists only).
    fn reflect_input_actions(&mut self) -> Result<(), String> {
        let ui = &mut self.ui;
        let Some(refl) = ui.find_by_name("InputReflector") else { return Ok(()) };
        let Some(bbox) = ui.var(refl, "ButtonBox") else { return Ok(()) };
        let margin = ui.w[refl].props.get("DefaultButtonMargin").cloned().unwrap_or(json!({}));
        let style = ui.find_class_path("ButtonStyle-BottomBar_C");
        let Some(btn_class) = ui.find_class_path("IconTextButton_C") else { return Ok(()) };
        let table = ui.store.pkg(INPUT_ACTIONS).and_then(|p| p.named("InputActionDataTable").map(|e| e.props.clone())).unwrap_or(Value::Null);
        let handler: [(&str, u8); 5] = [("Cancel", 3), ("Scroll", 3), ("AthenaNews", 0), ("PlaylistChange", 255), ("SquadFillChange", 255)];
        for (row, hs) in handler {
            let Some(r) = table.get("Rows").and_then(|t| t.get(row)) else { continue };
            let state = if hs != 255 { hs } else {
                match r.get("OverrrideStateForInputType").and_then(|s| s.as_str()).map(|s| s.rsplit("::").next().unwrap_or(s)) { Some("Enabled") => 0, Some("Disabled") => 1, Some("Hidden") => 2, _ => 3 }
            };
            if state > 1 { continue; }
            let text = r.get("DisplayName").cloned().unwrap_or(Value::Null);
            // OnButtonAdded: SetMouseKeyboardStyle(ButtonStyle-BottomBar); AddChildToHorizontalBox -> VAlign Center, HAlign Center, padding DefaultButtonMargin
            let inst = json!({ "ButtonText": text, "Style": style });
            let slot = json!({ "HorizontalAlignment": "HAlign_Center", "VerticalAlignment": "VAlign_Center", "Padding": margin });
            ui.add_class_instance(&btn_class, Some(bbox), "HorizontalBoxSlot", slot, Some(inst)).ok_or("cannot compose a reflected input button")?;
        }
        Ok(())
    }

    pub fn tab_labels(&self) -> &[String] { &self.tab_labels }
    /// How many new item cards one `draw` may bake (see `athena_locker::card::CardMats`); `usize::MAX` for captures.
    pub fn set_card_budget(&mut self, n: usize) { self.card_mats.max_per_frame = n; }
    /// Some item cards were not baked yet this frame (budget): draw again.
    pub fn cards_pending(&self) -> bool { self.card_mats.pending }
    /// The tab button widgets (one per visible tab registration, in order).
    pub fn tab_widgets(&self) -> &[usize] { &self.tabs }
    /// The Locker page once it has been composed (first time its tab is selected).
    pub fn locker(&self) -> Option<&crate::screens::athena_locker::LockerWidgets> { self.locker.as_ref() }
    /// Content class path (`TabContentType`) of a visible tab.
    pub fn tab_content(&self, i: usize) -> Option<&str> { self.tab_content.get(i).map(|s| s.as_str()) }
    /// The composed page widget of a tab (None until it was selected, or when its class could not be composed).
    pub fn page(&self, i: usize) -> Option<usize> { self.pages.get(i).copied().flatten() }

    /// Compose the content widget of tab `i` into `TopTabContentWidgetSwitcher` the first time it is needed (`FortTabListWidgetBase` creates the
    /// `TabContentType` widget of a registration on demand and the switcher activates it).
    pub fn ensure_page(&mut self, i: usize) {
        if i >= self.pages.len() || self.pages[i].is_some() { return; }
        let class = self.tab_content[i].clone();
        if class == crate::screens::athena_locker::TAB_CLASS {
            if let Some(lw) = crate::screens::athena_locker::LockerWidgets::install(&mut self.ui, self.switcher) { self.pages[i] = Some(lw.tab); self.locker = Some(lw); }
            return;
        }
        let id = self.ui.add_class_instance(&class, Some(self.switcher), "WidgetSwitcherSlot", json!({}), None);
        if id.is_none() { self.composition_log.push(format!("tab {i}: cannot compose {class}")); }
        self.pages[i] = id;
    }

    fn apply_state(&mut self, uic: &mut UiContext, st: &LobbyState, viewport: [f32; 2], scale: f32) {
        // the previous frame's hover / press overlay is undone first (the managed states below are reassigned every frame)
        for id in self.last_hover.into_iter().flatten() { if self.ui.w[id].btn_state != BtnState::Disabled { self.ui.w[id].btn_state = BtnState::Normal; self.ui.refresh_button_text(id); } }
        // the tab list activates the page of the selected tab in the content switcher
        self.ensure_page(st.selected_tab);
        let active = self.pages.get(st.selected_tab).copied().flatten().and_then(|pg| self.ui.w[self.switcher].kids.iter().position(|&k| k == pg));
        self.ui.w[self.switcher].props["ActiveWidgetIndex"] = json!(active.map_or(9999, |i| i as i64));
        if self.tab_content.get(st.selected_tab).is_some_and(|c| c == crate::screens::athena_locker::TAB_CLASS) {
            if let Some(l) = self.locker.as_mut() { l.apply(&mut self.ui, uic, &st.locker, viewport, scale); }
        }
        let locres = self.ui.store.locres();
        let find = |ui: &Ui, n: &str| ui.find_by_name(n);
        let miss = |ui: &Ui, ns: &str, key: &str| { let m = format!("locres miss: {ns} / {key}"); let mut u = ui.unresolved.borrow_mut(); if !u.contains(&m) { u.push(m); } };
        for (i, &t) in self.tabs.iter().enumerate() {
            let state = if i == st.selected_tab { if st.hovered_tab == Some(i) { BtnState::SelectedHovered } else { BtnState::Selected } } else if st.hovered_tab == Some(i) { BtnState::Hovered } else { BtnState::Normal };
            self.ui.w[t].btn_state = state;
            self.ui.refresh_button_text(t);
        }
        // matchmaking rotators (no fallback strings: a failed lookup shows nothing and is reported)
        if let Some(pr) = find(&self.ui, "PlaylistRotator") {
            let ns = "Fortnite.FortAthenaMatchmakingWidget";
            let key = ["SquadPlaylistSolo", "SquadPlaylistDuo", "SquadPlaylistSquad"][st.playlist.min(2)];
            let label = match locres.as_ref().and_then(|l| l.get(ns, key)) { Some(s) => s.to_owned(), None => { miss(&self.ui, ns, key); String::new() } };
            if let Some(t) = self.ui.var(pr, "MyText") { self.ui.w[t].text = Some(label); }
        }
        if let Some(fr) = find(&self.ui, "TeamFillRotator") {
            // SetAutoFillOptions: PopulateTextLabels(["No Fill", "Fill"]) (FText literals of the Kismet, ns [570C64F3...])
            let ns = "[570C64F344756CC8FFA9A3B51340E2A4]";
            let (fill_idx, fill_enabled) = team_fill_rule(st.playlist, st.team_fill, st.party_leader);
            let key = if fill_idx == 0 { "5FB65AF549F646DD0AB3AAA5E705C869" } else { "B5C6FB4943BBF71C5310128DBF9C6968" };
            let label = match locres.as_ref().and_then(|l| l.get(ns, key)) { Some(s) => s.to_owned(), None => { miss(&self.ui, ns, key); String::new() } };
            if let Some(t) = self.ui.var(fr, "MyText") { self.ui.w[t].text = Some(label); }
            // SetIsEnabled(false): CommonButton state Disabled (own disabled brush / text styles) and the Slate disabled effect on the subtree
            self.ui.w[fr].enabled = fill_enabled;
            self.ui.w[fr].btn_state = if fill_enabled { BtnState::Normal } else { BtnState::Disabled };
        }
        if let Some(pr) = find(&self.ui, "PlaylistRotator") {
            self.ui.w[pr].enabled = st.party_leader;
            self.ui.w[pr].btn_state = if st.party_leader { BtnState::Normal } else { BtnState::Disabled };
        }
        // MTXButton.Amount.SetCurrentValue(Conv_IntToFloat(MtxBalance)); NumericType Number -> FText::AsNumber (digit grouping)
        if let Some(m) = find(&self.ui, "MTXButton") {
            if let Some(a) = self.ui.var(m, "Amount") { self.ui.w[a].text = Some(group_digits(st.mtx_balance)); }
        }
        if let Some(pp) = find(&self.ui, "PlayerPanel") { self.ui.w[pp].vis = if st.player_panel { Vis::SelfHitTestInvisible } else { Vis::Hidden }; }
        if let Some(pb) = find(&self.ui, "PlayButton") { self.ui.w[pb].btn_state = if st.play_hovered { BtnState::Hovered } else { BtnState::Normal }; }
        // pointer overlay: `UCommonButton` hovered / pressed state (selected buttons keep their selected variants)
        self.last_hover = [st.hover_widget, st.press_widget];
        for (id, pressed) in [(st.hover_widget, false), (st.press_widget, true)] {
            let Some(id) = id.filter(|&i| i < self.ui.w.len()) else { continue };
            let cur = self.ui.w[id].btn_state;
            if cur == BtnState::Disabled || !self.ui.w[id].enabled { continue; }
            let sel = matches!(cur, BtnState::Selected | BtnState::SelectedHovered | BtnState::SelectedPressed);
            self.ui.w[id].btn_state = match (sel, pressed) { (false, false) => BtnState::Hovered, (false, true) => BtnState::Pressed, (true, false) => BtnState::SelectedHovered, (true, true) => BtnState::SelectedPressed };
            self.ui.refresh_button_text(id);
        }
    }

    /// Pad roots for this frame (composed once per slot, reused), with their anchors in Slate units.
    fn prepare_pads(&mut self, st: &LobbyState, scale: f32) -> Vec<(usize, [f32; 2], [f32; 2])> {
        let mut out = vec![];
        for (i, p) in st.pads.iter().enumerate() {
            let player = matches!(p.kind, PadKind::Player { .. });
            let id = match self.pad_roots.get(&(i, player)) {
                Some(&id) => id,
                None => {
                    let class = if player { PAD_TOP } else { PAD_ADD };
                    let Some(id) = self.ui.add_class_instance(class, None, "", json!({}), None) else { continue };
                    self.pad_roots.insert((i, player), id);
                    id
                }
            };
            if let PadKind::Player { name, leader, ready } = &p.kind { self.ui.hook_pad_top(id, name.as_deref(), *leader, *ready); }
            out.push((id, [p.x / scale, p.y / scale], p.pivot));
        }
        out
    }

    /// Lay out and paint the screen for a `w` x `h` pixel viewport (transparent: nothing is drawn behind the widgets).
    pub fn draw(&mut self, uic: &mut UiContext, w: f32, h: f32, st: &LobbyState) -> Frame {
        let scale = crate::layout::dpi_scale(w, h);
        self.card_mats.baked = 0;
        self.card_mats.pending = false;
        self.apply_state(uic, st, [w / scale, h / scale], scale);
        let pads = self.prepare_pads(st, scale);
        let lay = layout::run(&mut self.ui, uic, self.root, [w / scale, h / scale], scale, &pads);
        let px = |r: [f32; 4]| Rect::new(r[0] * scale, r[1] * scale, r[2] * scale, r[3] * scale);
        let id_rects: Vec<Rect> = lay.rect.iter().map(|r| px(*r)).collect();
        let mut p = paint::Painter { c: layout::Ctx { ui: &mut self.ui, uic, lay }, out: RenderList::new(), mats: &mut self.mats, blur: vec![], missing: vec![], scale, disabled: false };
        p.paint(self.root, paint::Tf::ID, [1.0; 4]);
        for &(id, _, _) in &pads { p.paint(id, paint::Tf::ID, [1.0; 4]); }
        // the native item cards of the Locker (slot buttons and picker tiles) are painted over their (empty) `FortMultiSizeItemCard` rectangles
        if let Some(l) = self.locker.as_ref() { l.paint_cards(&*p.c.ui, &mut *p.c.uic, &mut self.card_mats, &mut p.out, &id_rects, &mut p.missing); }
        let (list, blur, mut missing) = (std::mem::take(&mut p.out), std::mem::take(&mut p.blur), std::mem::take(&mut p.missing));
        let lay = p.c.lay;
        missing.extend(self.ui.unresolved.borrow().iter().cloned());
        let mut rects = HashMap::new();
        for wd in &self.ui.w { if wd.id < lay.rect.len() { rects.entry(wd.name.clone()).or_insert(px(lay.rect[wd.id])); } }
        let tabs = self.tabs.iter().zip(&self.tab_labels).map(|(&t, n)| (n.clone(), px(lay.rect[t]))).collect();
        let pad_rects = pads.iter().map(|&(id, _, _)| px(lay.rect[id])).collect();
        Frame { list, blur, rects, tabs, pads: pad_rects, id_rects, missing }
    }

    /// Text dump of the composed tree with arranged rectangles (Slate units) for a viewport.
    pub fn dump(&mut self, uic: &mut UiContext, w: f32, h: f32, st: &LobbyState) -> String {
        let scale = crate::layout::dpi_scale(w, h);
        self.apply_state(uic, st, [w / scale, h / scale], scale);
        let pads = self.prepare_pads(st, scale);
        let lay = layout::run(&mut self.ui, uic, self.root, [w / scale, h / scale], scale, &pads);
        let mut s = String::new();
        self.ui.dump(self.root, &mut s, 0, Some(&lay.rect));
        for &(id, _, _) in &pads { self.ui.dump(id, &mut s, 0, Some(&lay.rect)); }
        s
    }
}

