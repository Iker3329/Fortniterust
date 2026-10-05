//! The Fortnite 1.8 Battle Royale Settings screen (`/Game/UI/Frontend/Settings/OptionsMenu`) composed from the cooked widget blueprints with the generic UMG
//! interpreter (`athena_lobby::{umg, layout, paint}`).
//!
//! What is read from the data: the widget trees (layout, sizes, padding, styles, brushes, fonts), the tab registrations (`OptionsMenu::Initialize` Kismet),
//! every row's label / options / hover text / visibility (`Initialize Data`, `Update Data`, `Construct` Kismet of each tab, run by [`kismet::Vm`]), the
//! footer action states (`SetActionHandlerState` of the tab-changed handler) and the texts (`Game.locres`). What is native in the 1.8 client (the setting
//! getters / setters, display mode lists, platform queries) is answered by the host through [`Backing`]; each native name is the one the Kismet calls
//! (see `host.rs`).
//!
//! Composition (all from data): the state layout `/Game/UI/Foundation/StateLayouts/HUD` hosts the screen. `HUD.CursorModeContent.OverlayManagementMenu` is
//! `VerticalBox { MainContentStack (fill), BottomBarSizedSpacer }` and `BottomBarWidget` (InputReflector with the action buttons) sits over it; `OptionsMenu_C`
//! is pushed into `MainContentStack` (`UIManagerWidget::PushContentWidget`). Only the cursor mode subtree is laid out and painted.
pub mod host;
pub mod kismet;

use crate::draw::RenderList;
use crate::layout::Rect;
use crate::scene::UiContext;
use crate::screens::athena_lobby::{layout, paint, slate_mat, store::Store, styles::BtnState, umg::{Ui, Vis}};
use kismet::{Natives, Val, Vm};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::rc::Rc;

pub const HUD: &str = "/Game/UI/Foundation/StateLayouts/HUD.HUD_C";
pub const OPTIONS_MENU: &str = "/Game/UI/Frontend/Settings/OptionsMenu.OptionsMenu_C";
pub const ICON_TAB_BUTTON: &str = "/Game/UI/Foundation/Buttons/IconTabButton.IconTabButton_C";
const INPUT_ACTIONS: &str = "/Game/UI/Foundation/Input/InputActionDataTable";

/// A row value as the native getters / setters of the options tab see it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RowValue { Index(i64), Float(f32) }

/// What the host (the game's settings model) answers for the natives of the Settings Blueprints.
pub trait Backing {
    /// Native function `func` called by the Blueprint `page` (a tab package name such as `TabVideoOptions`); `None` = not answered.
    fn native(&self, page: &str, func: &str, args: &[Val]) -> Option<Val>;
    /// Current value of the row `key` (widget variable) of `page`.
    fn value(&self, page: &str, key: &str) -> Option<RowValue>;
}

/// What the user did (the Blueprint handler the host must run natively).
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// a row changed: the new selected index / slider value (the `...Changed` native of the tab)
    Set { page: String, key: String, value: RowValue },
    Apply, Restore, ResetToDefault, ResetHud, ToggleMode, Back,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind { Rotator, Segments, Slider }

pub struct Row {
    pub page: usize,
    pub key: String,
    pub widget: usize,
    pub kind: RowKind,
    pub label: String,
    pub options: Vec<String>,
    /// hover texts: one per option (segments) or a single one
    pub hovers: Vec<String>,
    pub selected: i64,
    pub value: f32,
    pub buttons: Vec<usize>,
    pub tooltip: Option<usize>,
}

pub struct Page {
    /// tab id registered by `AddTab` / `RegisterFortTab` (`Video`, `Game`, `HUD`, ...)
    pub name: String,
    pub label: String,
    /// package of the content widget (`/Game/UI/Frontend/Settings/TabVideoOptions`)
    pub long: String,
    pub widget: usize,
    pub button: usize,
    /// the owning page of a sub tab (the Game tab hosts `Game` / `HUD`)
    pub parent: Option<usize>,
}

/// One drawn frame.
pub struct Frame {
    pub list: RenderList,
    pub blur: Vec<paint::BlurRegion>,
    /// pixel rectangle of every widget by composed-tree id
    pub id_rects: Vec<Rect>,
    pub missing: Vec<String>,
}

/// Pointer state of one frame (pixels).
#[derive(Clone, Copy, Debug, Default)]
pub struct Pointer { pub x: f32, pub y: f32, pub present: bool, pub down: bool, pub clicked: bool }

pub struct SettingsUi {
    pub ui: Ui,
    root: usize,
    cursor: usize,
    options: usize,
    switcher: usize,
    pub pages: Vec<Page>,
    pub rows: Vec<Row>,
    /// selected top level tab (index into the pages without a parent) and, in the Game tab, the selected sub tab
    pub active: usize,
    pub sub_active: usize,
    /// footer actions (InputReflector buttons) in registration order: (action name, button widget, handler state 0 enabled .. 3 hidden and disabled)
    pub actions: Vec<(String, usize, u8)>,
    /// `OptionsMenu.Apply Visible`: set when a setting was changed since the last apply
    pub apply_visible: bool,
    in_zone: bool,
    mats: HashMap<String, Option<Rc<slate_mat::SlateMat>>>,
    /// widget under the pointer / pressed
    hover: Option<usize>,
    press: Option<usize>,
    /// slider being dragged (row index)
    drag: Option<usize>,
    last_btn: Vec<usize>,
    pub log: Vec<String>,
}

struct VmNatives<'a> { b: &'a dyn Backing, page: &'a str, ui: &'a Ui, owner: usize }
impl Natives for VmNatives<'_> {
    fn call(&self, func: &str, args: &[Val]) -> Option<Val> { self.b.native(self.page, func, args) }
    fn widget(&self, on: &str, func: &str, _args: &[Val]) -> Option<Val> {
        let w = self.ui.var(self.owner, on).or_else(|| self.ui.find_in(self.owner, on))?;
        match func { "IsVisible" => Some(Val::Bool(self.ui.w[w].vis.draws())), _ => None }
    }
}

fn pkg_name(long: &str) -> &str { long.rsplit('/').next().unwrap_or(long) }
fn text_json(s: &str) -> Value { json!({ "key": "", "namespace": "", "text": s }) }
fn vis_of(i: i64) -> Vis { match i { 1 => Vis::Collapsed, 2 => Vis::Hidden, 3 => Vis::HitTestInvisible, 4 => Vis::SelfHitTestInvisible, _ => Vis::Visible } }
fn class_name(path: &str) -> &str { path.rsplit('.').next().unwrap_or(path) }

impl SettingsUi {
    pub fn load(store: Rc<Store>, b: &dyn Backing, in_zone: bool) -> Result<SettingsUi, String> {
        let t0 = std::time::Instant::now();
        let mut ui = Ui::new(store.clone());
        let root = ui.add_class_instance(HUD, None, "", json!({}), None).ok_or("cannot compose the HUD state layout")?;
        let t_hud = t0.elapsed();
        let cursor = ui.find_in(root, "[Overlay] Cursor Mode Content").ok_or("HUD has no cursor mode content")?;
        let stack = ui.find_in(root, "MainContentStack").ok_or("HUD has no MainContentStack")?;
        // the inner switcher of the cursor mode content shows OverlayManagementMenu (the screen host) while a management screen is open
        if let Some(sw) = ui.find_in(root, "[Switcher] CursorModeContent") { ui.w[sw].props["ActiveWidgetIndex"] = json!(0); }
        let options = ui.add_class_instance(OPTIONS_MENU, Some(stack), "PanelSlot", json!({}), None).ok_or("cannot compose OptionsMenu")?;
        let switcher = ui.var(options, "CurrentTabSwitcher").or_else(|| ui.find_in(options, "CurrentTabSwitcher")).ok_or("OptionsMenu has no CurrentTabSwitcher")?;
        let mut me = SettingsUi { ui, root, cursor, options, switcher, pages: vec![], rows: vec![], active: 0, sub_active: 0, actions: vec![], apply_visible: false, in_zone, mats: HashMap::new(), hover: None, press: None, drag: None, last_btn: vec![], log: vec![] };
        let t_opt = t0.elapsed();
        me.build_tabs(&store, b)?;
        let t_tabs = t0.elapsed();
        me.build_footer();
        me.refresh(b);
        me.select_tab(b, 0);
        me.log.push(format!("timing: HUD {t_hud:?}, +OptionsMenu {t_opt:?}, +tabs {t_tabs:?}, total {:?}; {} widgets", t0.elapsed(), me.ui.w.len()));
        Ok(me)
    }

    fn run_with(&mut self, b: &dyn Backing, long: &str, f: &str, params: Vec<(String, Val)>, vars: &[(&str, Val)]) -> kismet::Run {
        let store = self.ui.store.clone();
        let owner = if long == OPTIONS_MENU.split('.').next().unwrap() { self.options } else { self.pages.iter().find(|p| p.long == long).map(|p| p.widget).unwrap_or(self.options) };
        let r = {
            let n = VmNatives { b, page: pkg_name(long), ui: &self.ui, owner };
            let mut vm = Vm::new(&store, long, &n);
            for (k, v) in vars { vm.vars.insert((*k).to_owned(), v.clone()); }
            vm.exec_with(f, None, params);
            vm.run
        };
        self.log.extend(r.notes.iter().map(|n| format!("{}.{f}: {n}", pkg_name(long))));
        r
    }

    /// Execute the tab registrations a Blueprint made (`AddChild(Create(class))` into the content switcher, `RegisterFortTab(id, IconTabButton_C, label info,
    /// content)` on the tab list) into the widget tree. Returns the pages created, in order.
    fn compose_tabs(&mut self, store: &Rc<Store>, owner: usize, owner_long: &str, calls: &[kismet::Call], parent: Option<usize>) -> Result<Vec<usize>, String> {
        let mut made = vec![];
        let mut contents: Vec<(String, usize)> = vec![];
        let mut lists: HashMap<String, usize> = HashMap::new();
        for c in calls {
            match (c.func.as_str(), c.on.as_deref()) {
                ("AddChild", Some(sw)) => {
                    let Some(Val::Created(class)) = c.args.first() else { continue };
                    let Some(swid) = self.ui.var(owner, sw).or_else(|| self.ui.find_in(owner, sw)) else { continue };
                    let Some(class_path) = self.ui.find_class_path(class) else { self.log.push(format!("tab class {class} not found")); continue };
                    let widget = self.ui.add_class_instance(&class_path, Some(swid), "WidgetSwitcherSlot", json!({}), None).ok_or_else(|| format!("cannot compose {class}"))?;
                    contents.push((class.clone(), widget));
                }
                ("RegisterFortTab", Some(list)) => {
                    let id = c.args.first().and_then(|a| a.as_name()).unwrap_or("").to_owned();
                    let label = match c.args.get(2) { Some(Val::Struct(s)) => s.iter().find(|(k, _)| k == "DisplayName").and_then(|(_, v)| kismet::resolve_text(store, v)).unwrap_or_default(), _ => String::new() };
                    let Some(Val::Created(class)) = c.args.get(3) else { continue };
                    let Some((_, widget)) = contents.iter().rev().find(|(cl, _)| cl == class).cloned() else { continue };
                    let list_id = match lists.get(list) { Some(&l) => l, None => { let Some(l) = self.ui.var(owner, list).or_else(|| self.ui.find_in(owner, list)) else { continue }; lists.insert(list.to_owned(), l); l } };
                    let tab_box = self.ui.var(list_id, "TabButtonBox").or_else(|| self.ui.find_in(list_id, "TabButtonBox")).ok_or("no TabButtonBox")?;
                    let pad = self.ui.w[list_id].props.get("TabButtonPadding").cloned().unwrap_or(json!({}));
                    // the list's `ButtonStyle` (OptionsMenu additionally calls `SetStyle(ButtonStyle-Tab-Manage_C)` from `OnTabButtonCreated`, the same style)
                    let style = self.ui.w[list_id].props.get("ButtonStyle").cloned().unwrap_or(Value::Null);
                    let inst = json!({ "Style": style, "ButtonText": text_json(&label) });
                    let button = self.ui.add_class_instance(ICON_TAB_BUTTON, Some(tab_box), "HorizontalBoxSlot", json!({ "Padding": pad }), Some(inst)).ok_or("cannot compose a tab button")?;
                    // `ShowText()`
                    if let Some(t) = self.ui.var(button, "CenterButtonTextWidget") { self.ui.w[t].text = Some(label.clone()); self.ui.w[t].vis = if label.is_empty() { Vis::Collapsed } else { Vis::SelfHitTestInvisible }; }
                    let long = self.ui.w[widget].user.as_ref().and_then(|u| u.class_path.clone()).and_then(|p| p.split('.').next().map(|s| s.to_owned())).unwrap_or_default();
                    self.pages.push(Page { name: id, label, long, widget, button, parent });
                    made.push(self.pages.len() - 1);
                }
                _ => {}
            }
        }
        let _ = owner_long;
        Ok(made)
    }

    /// `OptionsMenu::Initialize`: `AddTab(WidgetClass, DisplayName, NameId)` for every tab the platform shows; then the tab content widgets run their Construct.
    fn build_tabs(&mut self, store: &Rc<Store>, b: &dyn Backing) -> Result<(), String> {
        let long = OPTIONS_MENU.split('.').next().unwrap().to_owned();
        let run = self.run_with(b, &long, "Initialize", vec![], &[]);
        // the handler states `Initialize` leaves (Apply / ResetToDefault hidden, Restore shown, ToggleMode hidden)
        self.record_states(&run.calls);
        let owner = self.options;
        let made = self.compose_tabs(store, owner, &long, &run.calls, None)?;
        // the tab content widgets' `Construct` (rows) runs when they enter the tree; the Game tab registers its sub tabs there
        for pi in made { self.construct_page(store, b, pi)?; }
        Ok(())
    }

    fn construct_page(&mut self, store: &Rc<Store>, b: &dyn Backing, pi: usize) -> Result<(), String> {
        let long = self.pages[pi].long.clone();
        for (f, params) in [("PreConstruct", vec![("IsDesignTime".to_owned(), Val::Bool(false))]), ("Construct", vec![])] {
            if self.ui.store.function(&long, f).is_none() { continue; }
            let run = self.run_with(b, &long, f, params, &[]);
            // sub tabs (the Game tab): `AddChild(Create(..))` + `RegisterFortTab`
            let owner = self.pages[pi].widget;
            let subs = self.compose_tabs(store, owner, &long, &run.calls, Some(pi))?;
            self.apply_calls(pi, &run.calls);
            for s in subs { self.construct_page(store, b, s)?; }
        }
        self.update_page(b, pi);
        Ok(())
    }

    fn update_page(&mut self, b: &dyn Backing, pi: usize) {
        let long = self.pages[pi].long.clone();
        for f in ["Update Data", "UpdateData"] {
            if self.ui.store.function(&long, f).is_none() { continue; }
            let params = self.ui.store.function_params(&long, f).into_iter().map(|p| (p, Val::Bool(false))).collect();
            let run = self.run_with(b, &long, f, params, &[]);
            self.apply_calls(pi, &run.calls);
            break;
        }
    }

    /// Re-run every page's `Update Data` (the host's settings changed).
    pub fn refresh(&mut self, b: &dyn Backing) {
        for pi in 0..self.pages.len() { self.update_page(b, pi); }
    }

    /// Apply the calls a Blueprint function made on the widgets of page `pi` (the widget methods `RotatorSelector::Initialize`, `OptionsMenuSlider::Update Slider`, ...).
    fn apply_calls(&mut self, pi: usize, calls: &[kismet::Call]) {
        let store = self.ui.store.clone();
        let page_w = self.pages[pi].widget;
        for c in calls {
            let Some(var) = c.on.as_deref() else { continue };
            let Some(w) = self.ui.var(page_w, var).or_else(|| self.ui.find_in(page_w, var)) else { continue };
            let txt = |v: Option<&Val>| v.and_then(|v| kismet::resolve_text(&store, v)).unwrap_or_default();
            let row = self.rows.iter().position(|r| r.page == pi && r.key == var);
            match c.func.as_str() {
                // RotatorSelector::Initialize(Display Text, Options, Hover Text, Tab Tooltip)
                "Initialize" if c.args.len() >= 3 => {
                    let options: Vec<String> = c.args[1].as_array().iter().map(|v| txt(Some(v))).collect();
                    let r = Row { page: pi, key: var.to_owned(), widget: w, kind: RowKind::Rotator, label: txt(c.args.first()), options, hovers: vec![txt(c.args.get(2))], selected: 0, value: 0.0, buttons: vec![], tooltip: self.tooltip_widget(pi, c.args.get(3)) };
                    match row { Some(i) => { let old = self.rows.remove(i); self.rows.insert(i, Row { selected: old.selected, ..r }); } None => self.rows.push(r) }
                }
                // OptionsMenuRowSelector::Initialize Row Selector(Row Text, Buttons, Hover Texts, Require Selection, Tab Tooltip)
                "Initialize Row Selector" if c.args.len() >= 3 => {
                    if row.is_some() { continue; }
                    let options: Vec<String> = c.args[1].as_array().iter().map(|v| txt(Some(v))).collect();
                    let hovers: Vec<String> = c.args[2].as_array().iter().map(|v| txt(Some(v))).collect();
                    let tooltip = self.tooltip_widget(pi, c.args.get(4));
                    self.rows.push(Row { page: pi, key: var.to_owned(), widget: w, kind: RowKind::Segments, label: txt(c.args.first()), options, hovers, selected: -1, value: 0.0, buttons: vec![], tooltip });
                    let ri = self.rows.len() - 1;
                    self.build_segments(ri);
                }
                // OptionsMenuSlider::Update Slider(Slider Text, Slider Value, Hover Text, Tooltip Text Block)
                "Update Slider" if c.args.len() >= 2 => {
                    let value = c.args.get(1).and_then(|v| v.as_float()).unwrap_or(0.0);
                    let label = txt(c.args.first());
                    let hover = txt(c.args.get(2));
                    let tip = self.tooltip_widget(pi, c.args.get(3));
                    match row {
                        Some(i) => { let r = &mut self.rows[i]; r.label = label; r.value = value; r.hovers = vec![hover]; r.tooltip = tip; }
                        None => self.rows.push(Row { page: pi, key: var.to_owned(), widget: w, kind: RowKind::Slider, label, options: vec![], hovers: vec![hover], selected: 0, value, buttons: vec![], tooltip: tip }),
                    }
                }
                "Update Row Selector" => { if let (Some(i), Some(v)) = (row, c.args.first().and_then(|v| v.as_int())) { self.rows[i].selected = v; } }
                "Deselect All Rows" => { if let Some(i) = row { self.rows[i].selected = -1; } }
                "SetVisibility" => { if let Some(v) = c.args.first().and_then(|v| v.as_int()) { self.ui.w[w].vis = vis_of(v); } }
                "SetIsEnabled" => { if let Some(b) = c.args.first().and_then(|v| v.as_bool()) { self.ui.w[w].enabled = b; } }
                "SetText" => { let t = txt(c.args.first()); self.ui.w[w].text = Some(t); }
                _ => {}
            }
        }
    }

    fn tooltip_widget(&self, pi: usize, v: Option<&Val>) -> Option<usize> {
        // the tab passes its tooltip text block (`V:TooltipDisplay`, or `V:TooltipDisplayReference` that `TabGameOptions` points at its own `TooltipDisplay`)
        let name = match v { Some(Val::Var(n)) => n.as_str(), _ => return None };
        let page_w = self.pages[pi].widget;
        if let Some(w) = self.ui.var(page_w, name).filter(|&w| self.ui.w[w].class.ends_with("TextBlock")) { return Some(w); }
        let mut anc = self.pages[pi].parent.map(|p| self.pages[p].widget);
        while let Some(t) = anc { if let Some(w) = self.ui.find_in(t, "TooltipDisplay") { return Some(w); } anc = self.pages[pi].parent.and_then(|p| self.pages[p].parent).map(|p| self.pages[p].widget); }
        self.ui.find_in(page_w, "TooltipDisplay")
    }

    /// `OptionsMenuRowSelector::Initialize Row Selector`: one `IconTextButton_C` per option in `SelectorButtonsBox` (Style = the row's `ButtonStyle`, Center aligned,
    /// `HorizontalBoxSlot` size Fill 1).
    fn build_segments(&mut self, ri: usize) {
        let (w, options) = (self.rows[ri].widget, self.rows[ri].options.clone());
        let Some(boxw) = self.ui.var(w, "SelectorButtonsBox").or_else(|| self.ui.find_in(w, "SelectorButtonsBox")) else { return };
        let style = self.ui.w[w].props.get("ButtonStyle").cloned().unwrap_or(Value::Null);
        let Some(cls) = self.ui.find_class_path("IconTextButton_C") else { return };
        let mut btns = vec![];
        for o in &options {
            let inst = json!({ "ButtonText": text_json(o), "Style": style, "ContentAlignment": "HAlign_Center" });
            let slot = json!({ "Size": { "SizeRule": "ESlateSizeRule::Fill", "Value": 1.0 } });
            if let Some(b) = self.ui.add_class_instance(&cls, Some(boxw), "HorizontalBoxSlot", slot, Some(inst)) { btns.push(b); }
        }
        self.rows[ri].buttons = btns;
    }

    /// `InputReflector` of `BottomBarWidget`: one `IconTextButton_C` per input action the OptionsMenu registered (`Set Input Action Handlers`, in registration order).
    fn build_footer(&mut self) {
        let ui = &mut self.ui;
        let Some(refl) = ui.find_in(self.root, "InputReflector") else { return };
        let Some(bbox) = ui.var(refl, "ButtonBox").or_else(|| ui.find_in(refl, "ButtonBox")) else { return };
        let margin = ui.w[refl].props.get("DefaultButtonMargin").cloned().unwrap_or(json!({}));
        let style = ui.find_class_path("ButtonStyle-BottomBar_C");
        let Some(btn_class) = ui.find_class_path("IconTextButton_C") else { return };
        let table = ui.store.pkg(INPUT_ACTIONS).and_then(|p| p.named("InputActionDataTable").map(|e| e.props.clone())).unwrap_or(Value::Null);
        let store = ui.store.clone();
        let mut vm = Vm::new(&store, OPTIONS_MENU, &kismet::NoNatives);
        vm.exec_with("Set Input Action Handlers", None, vec![]);
        for c in vm.run.calls.iter().filter(|c| c.func == "AddInputActionHandler") {
            let Some(name) = c.args.get(1).and_then(|a| a.as_name()) else { continue };
            let Some(r) = table.get("Rows").and_then(|t| t.get(name)) else { continue };
            let text = r.get("DisplayName").cloned().unwrap_or(Value::Null);
            let inst = json!({ "ButtonText": text, "Style": style });
            let slot = json!({ "HorizontalAlignment": "HAlign_Center", "VerticalAlignment": "VAlign_Center", "Padding": margin });
            if let Some(b) = ui.add_class_instance(&btn_class, Some(bbox), "HorizontalBoxSlot", slot, Some(inst)) {
                // default handler state: the row's keyboard state (`OverrrideStateForInputType[0]`)
                let st = match r.get("OverrrideStateForInputType").and_then(|s| s.as_str()).map(|s| s.rsplit("::").next().unwrap_or(s)) { Some("Enabled") => 0, Some("Disabled") => 1, Some("Hidden") => 2, _ => 3 };
                self.actions.push((name.to_owned(), b, st));
            }
        }
        // `Initialize` ran before the footer existed: re-apply its states
        let long = OPTIONS_MENU.split('.').next().unwrap().to_owned();
        let mut vm = Vm::new(&store, &long, &kismet::NoNatives);
        vm.exec_with("Initialize", None, vec![]);
        let calls = vm.run.calls;
        self.record_states(&calls);
    }

    /// `SetActionHandlerState(InputActionDataTable, Name, State)` calls -> the footer button states.
    fn record_states(&mut self, calls: &[kismet::Call]) {
        for c in calls.iter().filter(|c| c.func == "SetActionHandlerState") {
            let (Some(name), Some(st)) = (c.args.get(1).and_then(|a| a.as_name()), c.args.get(2).and_then(|a| a.as_int())) else { continue };
            if let Some(a) = self.actions.iter_mut().find(|a| a.0 == name) { a.2 = st as u8; }
        }
    }

    /// Top level pages (the OptionsMenu tabs).
    pub fn top_pages(&self) -> Vec<usize> { (0..self.pages.len()).filter(|&i| self.pages[i].parent.is_none()).collect() }
    pub fn sub_pages(&self, parent: usize) -> Vec<usize> { (0..self.pages.len()).filter(|&i| self.pages[i].parent == Some(parent)).collect() }
    pub fn page_by_name(&self, n: &str) -> Option<usize> { self.pages.iter().position(|p| p.name == n && p.parent.is_none()) }

    /// Select a top level tab: `CurrentTabSwitcher` shows its page and the `OnActiveWidgetChanged` handler of the OptionsMenu sets the footer action states.
    pub fn select_tab(&mut self, b: &dyn Backing, i: usize) {
        let tops = self.top_pages();
        if i >= tops.len() { return; }
        self.active = i;
        let pi = tops[i];
        let mut chain: Vec<String> = self.ui.w[self.pages[pi].widget].user.as_ref().map(|u| u.chain.iter().map(|c| class_name(c).to_owned()).collect()).unwrap_or_default();
        // native hierarchy of the option tab classes (reflection SDK `FN_FortniteUI_classes.hpp`: `UFortAccountOptions` / `UFortAudioOptions` / `UFortGameOptions` /
        // `UFortInputOptions` / `UFortVideoOptions` : `UFortOptionsTab`)
        if chain.iter().any(|c| matches!(c.as_str(), "FortAccountOptions" | "FortAudioOptions" | "FortGameOptions" | "FortInputOptions" | "FortVideoOptions")) { chain.push("FortOptionsTab".to_owned()); }
        let long = OPTIONS_MENU.split('.').next().unwrap().to_owned();
        let Some(name) = self.ui.store.function_names(&long).into_iter().find(|n| n.starts_with("BndEvt__CurrentTabSwitcher") && n.contains("OnActiveWidgetChanged")) else { return };
        let params = vec![("ActiveWidget".to_owned(), Val::Inst(chain)), ("ActiveWidgetIndex".to_owned(), Val::Int(i as i64))];
        let run = self.run_with(b, &long, &name, params, &[("Apply Visible", Val::Bool(self.apply_visible))]);
        self.record_states(&run.calls);
    }

    /// Pages shown now: the active top page and, for the Game tab, the active sub page.
    fn shown_pages(&self) -> Vec<usize> {
        let tops = self.top_pages();
        let Some(&p) = tops.get(self.active) else { return vec![] };
        let mut v = vec![p];
        let subs = self.sub_pages(p);
        if let Some(&s) = subs.get(self.sub_active.min(subs.len().saturating_sub(1))) { v.push(s); }
        v
    }

    /// `WidgetSwitcher` activation, button states, row texts, footer: everything the Blueprints keep in widget state, written into the tree.
    fn apply_state(&mut self) {
        let tops = self.top_pages();
        // CurrentTabSwitcher / CommonWidgetSwitcher_0: the active page's widget
        let show = |ui: &mut Ui, sw: usize, page_w: usize| { if let Some(i) = ui.w[sw].kids.iter().position(|&k| k == page_w) { ui.w[sw].props["ActiveWidgetIndex"] = json!(i); } };
        if let Some(&p) = tops.get(self.active) {
            let w = self.pages[p].widget;
            show(&mut self.ui, self.switcher, w);
            let subs = self.sub_pages(p);
            if !subs.is_empty() {
                let sub_w = self.pages[subs[self.sub_active.min(subs.len() - 1)]].widget;
                if let Some(sw) = self.ui.w[sub_w].parent { show(&mut self.ui, sw, sub_w); }
            }
        }
        // tab buttons
        let hover = self.hover;
        let shown = self.shown_pages();
        for (pi, p) in self.pages.iter().enumerate() {
            let sel = shown.contains(&pi);
            let h = hover == Some(p.button);
            let st = match (sel, h, self.press == Some(p.button)) { (true, _, true) => BtnState::SelectedPressed, (true, true, _) => BtnState::SelectedHovered, (true, ..) => BtnState::Selected, (false, _, true) => BtnState::Pressed, (false, true, _) => BtnState::Hovered, _ => BtnState::Normal };
            self.ui.w[p.button].btn_state = st;
            self.ui.refresh_button_text(p.button);
        }
        // rows
        let (hover, press) = (self.hover, self.press);
        let btn_state = |id: usize, selected: bool| match (selected, hover == Some(id), press == Some(id)) { (true, _, true) => BtnState::SelectedPressed, (true, true, _) => BtnState::SelectedHovered, (true, ..) => BtnState::Selected, (false, _, true) => BtnState::Pressed, (false, true, _) => BtnState::Hovered, _ => BtnState::Normal };
        for ri in 0..self.rows.len() {
            let r = &self.rows[ri];
            let w = r.widget;
            match r.kind {
                RowKind::Rotator => {
                    if let Some(t) = self.ui.var(w, "DisplayText") { self.ui.w[t].text = Some(r.label.clone()); }
                    let sel = r.options.get(r.selected.max(0) as usize).cloned().unwrap_or_default();
                    if let Some(t) = self.ui.var(w, "MyText").or_else(|| self.ui.find_in(w, "MyText")) { self.ui.w[t].text = Some(sel); }
                    let enabled = self.ui.w[w].enabled;
                    for n in ["ButtonLeft", "ButtonRight"] {
                        if let Some(bn) = self.ui.find_in(w, n) { let st = if enabled { btn_state(bn, false) } else { BtnState::Disabled }; self.ui.w[bn].btn_state = st; self.ui.w[bn].enabled = enabled; self.ui.refresh_button_text(bn); }
                    }
                }
                RowKind::Segments => {
                    if let Some(t) = self.ui.var(w, "RowSelectorText") { self.ui.w[t].text = Some(r.label.clone()); }
                    let btns = r.buttons.clone();
                    let sel = r.selected;
                    for (i, &bn) in btns.iter().enumerate() { self.ui.w[bn].btn_state = btn_state(bn, i as i64 == sel); self.ui.refresh_button_text(bn); }
                }
                RowKind::Slider => {
                    if let Some(t) = self.ui.var(w, "SliderText") { self.ui.w[t].text = Some(r.label.clone()); }
                    // `Conv_FloatToText(value, HalfToEven, grouping, minInt 1, maxInt 1, minFrac 2, maxFrac 2)` of `Update Slider`
                    if let Some(t) = self.ui.var(w, "SliderValue") { self.ui.w[t].text = Some(format!("{:.2}", r.value)); }
                    if let Some(bg) = self.ui.var(w, "SliderBackground") { self.ui.w[bg].mat_scalars = vec![("fillPercent".to_owned(), r.value)]; }
                }
            }
        }
        // footer: handler states (0 enabled, 1 disabled, 2 hidden, 3 hidden and disabled)
        for (_, w, st) in self.actions.clone() {
            let w = w;
            self.ui.w[w].vis = if st <= 1 { Vis::Visible } else { Vis::Collapsed };
            self.ui.w[w].enabled = st == 0;
            self.ui.w[w].btn_state = if st != 0 { BtnState::Disabled } else { btn_state(w, false) };
            self.ui.refresh_button_text(w);
        }
        // tooltips: the hovered row's hover text (OnMouseEnter / `Button Hovered`), empty otherwise (OnMouseLeave)
        let mut tips: Vec<(usize, String)> = vec![];
        for r in &self.rows {
            if let Some(t) = r.tooltip { tips.push((t, String::new())); }
        }
        if let Some(hv) = hover {
            for r in &self.rows {
                let hit = r.widget == hv || self.ui.is_descendant(hv, r.widget);
                if !hit { continue; }
                let text = match r.kind {
                    RowKind::Segments => r.buttons.iter().position(|&b| b == hv).and_then(|i| r.hovers.get(i).cloned()).unwrap_or_default(),
                    _ => r.hovers.first().cloned().unwrap_or_default(),
                };
                if let Some(t) = r.tooltip { if let Some(e) = tips.iter_mut().find(|(w, _)| *w == t) { e.1 = text; } }
            }
        }
        for (t, s) in tips { self.ui.w[t].text = Some(s); }
    }

    /// Is widget `id` painted (all ancestors up to the cursor mode content draw and the switchers show it)?
    fn shown(&self, id: usize) -> bool {
        let mut cur = id;
        loop {
            let w = &self.ui.w[cur];
            if !w.vis.draws() || !w.vis.takes_space() { return false; }
            let Some(p) = w.parent else { return cur == self.cursor || cur == self.root };
            let pw = &self.ui.w[p];
            if matches!(pw.class.as_str(), "WidgetSwitcher" | "CommonWidgetSwitcher") {
                let idx = pw.props.get("ActiveWidgetIndex").and_then(|v| v.as_i64()).unwrap_or(0).max(0) as usize;
                if pw.kids.get(idx) != Some(&cur) { return false; }
            }
            if cur == self.cursor { return true; }
            cur = p;
        }
    }

    /// Interactive widget under the pixel (`x`, `y`): tab buttons, row buttons, sliders, footer buttons (the last created = painted on top wins).
    fn widget_at(&self, rects: &[Rect], x: f32, y: f32) -> Option<usize> {
        let mut best = None;
        for (i, w) in self.ui.w.iter().enumerate() {
            if i >= rects.len() || !rects[i].contains(x, y) { continue; }
            let interactive = (self.ui.is_common_button(i) && w.enabled) || self.rows.iter().any(|r| r.kind == RowKind::Slider && r.widget == i);
            if interactive && self.shown(i) { best = Some(i); }
        }
        best
    }

    /// One frame: layout, pointer interaction, widget state, paint. Returns the frame and what the user did.
    pub fn frame(&mut self, uic: &mut UiContext, b: &dyn Backing, w: f32, h: f32, ptr: Pointer) -> (Frame, Vec<Event>) {
        let scale = crate::layout::dpi_scale(w, h);
        let mut events = vec![];
        self.refresh(b);
        self.apply_state();
        let lay = layout::run(&mut self.ui, uic, self.cursor, [w / scale, h / scale], scale, &[]);
        let px = |r: [f32; 4]| Rect::new(r[0] * scale, r[1] * scale, r[2] * scale, r[3] * scale);
        let rects: Vec<Rect> = lay.rect.iter().map(|r| px(*r)).collect();
        // pointer
        self.hover = if ptr.present { self.widget_at(&rects, ptr.x, ptr.y) } else { None };
        if !ptr.down { self.drag = None; self.press = None; } else if ptr.clicked { self.press = self.hover; }
        if let Some(hv) = self.hover { if ptr.clicked { self.on_click(b, hv, &rects, ptr, &mut events); } }
        // slider drag (`SSlider`: the value follows the pointer while the mouse is captured)
        if ptr.down { if ptr.clicked { self.drag = self.rows.iter().position(|r| r.kind == RowKind::Slider && Some(r.widget) == self.hover); } }
        if let Some(ri) = self.drag { self.slide(ri, &rects, ptr.x, &mut events); }
        self.apply_state();
        let lay = layout::run(&mut self.ui, uic, self.cursor, [w / scale, h / scale], scale, &[]);
        let id_rects: Vec<Rect> = lay.rect.iter().map(|r| px(*r)).collect();
        let mut p = paint::Painter { c: layout::Ctx { ui: &mut self.ui, uic, lay }, out: RenderList::new(), mats: &mut self.mats, blur: vec![], missing: vec![], scale, disabled: false };
        p.paint(self.cursor, paint::Tf::ID, [1.0; 4]);
        let (list, blur, mut missing) = (std::mem::take(&mut p.out), std::mem::take(&mut p.blur), std::mem::take(&mut p.missing));
        missing.extend(self.ui.unresolved.borrow().iter().cloned());
        (Frame { list, blur, id_rects, missing }, events)
    }

    fn slide(&mut self, ri: usize, rects: &[Rect], x: f32, events: &mut Vec<Event>) {
        let r = &self.rows[ri];
        let Some(sl) = self.ui.var(r.widget, "MenuSlider").or_else(|| self.ui.find_in(r.widget, "MenuSlider")) else { return };
        let g = rects[sl];
        if g.w <= 0.0 { return; }
        let v = ((x - g.x) / g.w).clamp(0.0, 1.0);
        let (page, key) = (pkg_name(&self.pages[r.page].long).to_owned(), r.key.clone());
        if (self.rows[ri].value - v).abs() > f32::EPSILON { self.rows[ri].value = v; events.push(Event::Set { page, key, value: RowValue::Float(v) }); }
    }

    fn on_click(&mut self, b: &dyn Backing, hv: usize, rects: &[Rect], ptr: Pointer, events: &mut Vec<Event>) {
        // tab buttons
        if let Some(pi) = self.pages.iter().position(|p| p.button == hv) {
            match self.pages[pi].parent {
                None => { if let Some(i) = self.top_pages().iter().position(|&t| t == pi) { if i != self.active { self.sub_active = 0; self.select_tab(b, i); } } }
                Some(par) => { if let Some(i) = self.sub_pages(par).iter().position(|&t| t == pi) { self.sub_active = i; } }
            }
            return;
        }
        // footer
        if let Some((name, _, st)) = self.actions.iter().find(|a| a.1 == hv).cloned() {
            if st == 0 {
                events.push(match name.as_str() { "Apply" => Event::Apply, "Restore" => Event::Restore, "ResetToDefault" => Event::ResetToDefault, "ResetHUDToDefault" => Event::ResetHud, "ToggleMode" => Event::ToggleMode, _ => Event::Back });
            }
            return;
        }
        // rows
        let shown = self.shown_pages();
        for ri in 0..self.rows.len() {
            let r = &self.rows[ri];
            if !shown.contains(&r.page) { continue; }
            let (page, key) = (pkg_name(&self.pages[r.page].long).to_owned(), r.key.clone());
            match r.kind {
                RowKind::Rotator => {
                    let n = r.options.len() as i64;
                    if n == 0 { continue; }
                    let left = self.ui.find_in(r.widget, "ButtonLeft");
                    let right = self.ui.find_in(r.widget, "ButtonRight");
                    let d = if Some(hv) == left { -1 } else if Some(hv) == right { 1 } else { continue };
                    let v = (r.selected + d).rem_euclid(n);
                    events.push(Event::Set { page, key, value: RowValue::Index(v) });
                    self.rows[ri].selected = v;
                }
                RowKind::Segments => {
                    if let Some(i) = r.buttons.iter().position(|&b| b == hv) { events.push(Event::Set { page, key, value: RowValue::Index(i as i64) }); self.rows[ri].selected = i as i64; }
                }
                RowKind::Slider => { if r.widget == hv { self.slide(ri, rects, ptr.x, events); } }
            }
        }
        if !events.is_empty() { self.apply_visible = true; }
    }
}
