//! INVENTORY FIX2: the Athena inventory screen drawn from the COOKED widget trees, nothing recreated.
//!
//! `AthenaInventoryPanel_C` (`/Game/Athena/HUD/Inventory/AthenaInventoryPanel`) is composed with the generic UMG runtime of `athena_lobby` (`umg::Ui`:
//! the `WidgetArchetype` tree with every nested user widget: `AthenaInventoryEquipSlot_C` x5, `AthenaInventoryDropSlot_C`,
//! `AthenaInventoryLimitStatusIndicator_C`, `ItemInspectionMainItemDetailsHostPanel_C` > `ItemDetailsHeader_C` ...), laid out by its Slate rules
//! (`layout`) and painted by its brush / text / material painter (`paint`: cooked textures, the cooked Slate pixel shaders of the UI materials run on the
//! CPU). The header (`BorderSeasonHeader` / `CommonBorder_3` with `Border_AthenaStore_OfferHeader(Backing)`), the equip-slot frames (`EmptyImage`
//! `M_Slot_People_M_Empty`, `Selection` `M_Solid_Square_S_Edit`), the drop slot (`DropTargetOverlay` > `M_Slot_People_M_Border` + `T_AthenaDropItem` 80 x 80
//! Left / Top + 15) and the item-details widgets are therefore exactly the data.
//!
//! What this file adds is what 1.8 does at runtime around those trees, each piece with its source:
//! * the Blueprint behaviours (Kismet, `fn-inspect disasm`): `AthenaInventoryEquipSlot.RefreshItem` / `TryAndShowDropTarget` / `SetSelected`,
//!   `AthenaInventoryDropSlot` Tick, `AthenaInventoryFortItemTileButton.SetEquipSlotOverlay` / `UpdateInteractionWidgetVisibility` / `UpdateStyle`,
//!   `AthenaInventoryLimitStatusIndicator.RefreshWorldInventoryStatus`, `ItemDetailsHeader.UpdateItemsForWidgets`, `ItemDetailsHeaderRarityTypeText`,
//!   `ItemDetailsHeaderItemDisplayText`, `ItemDetailsHeaderTagListText.FormatItemDisplayTags`, `ItemDetailsStackCounter.Set Stack Count`,
//!   `ItemDescriptionDetailWidget`, `QuantitySelector.SetSplitData` / `SliderValueChanged` / `HandleMaxPressed`;
//! * the native fills (exe, `C:\FortniteRef\decomp\named`): the `FortItemTileView` grid (`UCommonTileView` RebuildWidget 0x143949510: cell =
//!   `ItemWidth` + `DesiredItemPadding` L/R by `ItemHeight` + T/B, `EListItemAlignment` 0 = EvenlyDistributed; UE 4.16 `SListPanel::OnArrangeChildren`),
//!   the entry padding (`UCommonListView` generate-row 0x14393a820: `UUserWidget::SetPadding(DesiredItemPadding)`), the tile button style padding
//!   (`ItemManagementTileButtonStyle-Base` `ButtonPadding` 4) and its Normal / Hovered / Pressed / Selected brushes, `FortItemTypeFunctionLibrary::ToText`
//!   (0x14061d7f0, LOCTEXT ns `EFortItemType`), `FortInventoryContext::GetUserFriendlyTags` (0x142e34410: `ItemCategories` primary category by item
//!   type, secondary by the definition's gameplay tags for ranged / trap weapons), `UCommonActionWidget` (0x1439592c0: collapses itself without an
//!   `InputActionDataRow`), `UFortMultiSizeItemCard` card sizes (`DefaultUIDataConfiguration.OtherItemCardSizes`: S 88, M 128).
//! INVENTORY FIX4: the header's `CategoryIndicator` (Secondary category icon) and `TierIndicator` (tier pips) are drawn (see `load` / `apply_details`).
//! NOT drawn: `FortLevelIndicator` (Hidden: BR items carry level 0), `ItemRatingIndicator_C` (Collapsed), `CategoryIndicatorTertiary` (row Collapsed:
//! `Show Ammo Header` false). INVENTORY FIX3 findings: `SFortLevelIndicator` update (0x143065040) sets itself Hidden when the item's level (`UFortWorldItem` vfunc +0x230 =
//! `ItemEntry.Level`) is 0; the tier pips come from `UFortItem::GetTier` / `GetMaxTier` (definition vfunc +0x278) and the category text / icon from item
//! vfunc +0x2d0 (0x142fee080, collapsed when it returns false); none of these are ported yet. The `BackgroundBlur` of the quantity selector is drawn
//! (`Cmd::Blur`, backend pass). The item cards keep the port's `MaterialKind::ItemCard`.
use super::athena_lobby::layout::{self as ul, Ctx};
use super::athena_lobby::paint::{Painter, Tf};
use super::athena_lobby::slate_mat::SlateMat;
use super::athena_lobby::store::{jf, Store};
use super::athena_lobby::styles::{Brush, BtnState};
use super::athena_lobby::umg::{Ui, Vis};
use crate::draw::RenderList;
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::inventory_panel::InvItem;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

pub const PANEL_CLASS: &str = "/Game/Athena/HUD/Inventory/AthenaInventoryPanel.AthenaInventoryPanel_C";
pub const TILE_CLASS: &str = "/Game/Athena/HUD/Inventory/AthenaInventoryFortItemTileButton.AthenaInventoryFortItemTileButton_C";
pub const QSEL_CLASS: &str = "/Game/UI/Foundation/Items/QuantitySelector.QuantitySelector_C";
/// `DefaultUIDataConfiguration.OtherItemCardSizes` (the square table: BR items are not cosmetics): S, M.
pub const CARD_S: f32 = 88.;
pub const CARD_M: f32 = 128.;
/// `Slate.GlobalScrollAmount` default (UE 4.16 `SlateCoreClasses.cpp`): `STableViewBase::OnMouseWheel` scrolls `WheelDelta * 32` Slate units.
pub const WHEEL_SCROLL: f32 = 32.;

/// One backpack tile of this frame.
#[derive(Clone, Debug)]
pub struct TileState { pub item: InvItem, pub btn: BtnState, pub selected: bool }

/// The `QuantitySelector_C` modal (`CreateQuantitySelectorWidget`).
#[derive(Clone, Debug, PartialEq)]
pub struct QselState { pub item: InvItem, pub current: i32, pub max: i32, pub slider: f32, pub hover: Option<QselButton>, pub pressed: Option<QselButton> }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QselButton { Confirm, Cancel, Max }

pub struct DrawState<'a> {
    pub equip: [Option<&'a InvItem>; 5],
    pub tiles: &'a [TileState],
    pub scroll: f32,
    /// the dragged item and whether the drag started on an equip slot (`AthenaInventoryDragVisual.ItemCardSize`: 2 = S from a slot, the tile card's M otherwise)
    pub drag: Option<(&'a InvItem, bool)>,
    pub cursor: (f32, f32),
    pub backpack: (u32, u32),
    pub selected: Option<&'a InvItem>,
    pub qsel: Option<&'a QselState>,
}

/// Pixel geometry of the drawn frame (hit tests of the panel).
#[derive(Clone, Debug, Default)]
pub struct Geo {
    pub scale: f32,
    pub equip: [Rect; 5],
    pub tile_view: Rect,
    /// one rect per tile (pixels, scrolled; may lie outside `tile_view`)
    pub tiles: Vec<Rect>,
    /// total height of the tile rows (pixels)
    pub content_h: f32,
    pub drop: Rect,
    pub qsel: Option<QselGeo>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct QselGeo { pub confirm: Rect, pub cancel: Rect, pub max: Rect, pub slider: Rect, pub thumb_w: f32 }


struct EquipW { root: usize, card: Option<usize>, empty: Option<usize>, drop_target: Option<usize>, selection: Option<usize>, input_overlay: Option<usize> }
struct TileW { root: usize, overlay: Option<usize>, card: Option<usize>, equip_border: Option<usize>, equip_overlay: Option<usize>, input_wrapper: Option<usize>, input_widget: Option<usize> }
struct QselW { root: usize, title: Option<usize>, current: Option<usize>, max_amount: Option<usize>, additional: Option<usize>, slider: Option<usize>, card: Option<usize>, confirm: Option<usize>, cancel: Option<usize>, max_btn: Option<usize> }

pub struct CookedInventory {
    ui: Ui,
    root: usize,
    mats: HashMap<String, Option<Rc<SlateMat>>>,
    equip: Vec<EquipW>,
    drop_overlay: Option<usize>,
    drop_root: Option<usize>,
    tile_view: Option<usize>,
    tiles: Vec<TileW>,
    count_label: Option<usize>,
    limit_label: Option<usize>,
    details: DetailsW,
    /// `ItemDetails` (the panel hosting the header)
    details_root: Option<usize>,
    qsel: Option<QselW>,
    hdr: HeaderData,
    /// assets / strings the painter could not resolve (reported once each)
    pub missing: Vec<String>,
}

use super::item_header::{group, DetailsW, HeaderData};
#[cfg(test)]
use super::item_header::item_type_index;
pub use super::item_header::item_type_text;

impl CookedInventory {
    pub fn load(install: &Path) -> Result<CookedInventory, String> {
        let store = Rc::new(Store::open(install)?);
        let mut ui = Ui::new(store.clone());
        let root = ui.add_class_instance(PANEL_CLASS, None, "", json!({}), None).ok_or("cannot compose AthenaInventoryPanel_C")?;
        let v = |ui: &Ui, owner: usize, n: &str| ui.var(owner, n).or_else(|| ui.find_in(owner, n));
        let mut equip = vec![];
        for i in 1..=5 {
            let s = v(&ui, root, &format!("EquipSlot{i}")).ok_or("no EquipSlot")?;
            equip.push(EquipW { root: s, card: v(&ui, s, "ItemCard"), empty: v(&ui, s, "EmptyImage"), drop_target: v(&ui, s, "DropTarget"), selection: v(&ui, s, "Selection"), input_overlay: v(&ui, s, "InputActionOverlay") });
        }
        let drop_root = v(&ui, root, "AthenaInventoryDropSlot");
        let drop_overlay = drop_root.and_then(|d| v(&ui, d, "DropTargetOverlay"));
        let tile_view = v(&ui, root, "TileView");
        let limit = v(&ui, root, "InventoryLimitStatusBackpack");
        let (count_label, limit_label) = match limit { Some(l) => (v(&ui, l, "ItemStorageCountLabel"), v(&ui, l, "ItemStorageLimitLabel")), None => (None, None) };
        let det = v(&ui, root, "ItemDetails");
        let details = DetailsW::find(&mut ui, det, "HeaderWidget");
        let hdr = HeaderData::load(&store);
        Ok(CookedInventory { ui, root, mats: HashMap::new(), equip, drop_overlay, drop_root, tile_view, tiles: vec![], count_label, limit_label, details, details_root: det, qsel: None, hdr, missing: vec![] })
    }

    /// The cooked brushes / material textures of these trees (`T-Slot-SelectSquare`, `T-SlotIcon-Add-64`, `128gray`, ...) are not all in the exported art
    /// folder the in-game UI reads: the paks are added BEHIND the existing source (exported / pre-decoded images keep priority), the way
    /// `UiContext::from_install` reads them for the lobby.
    #[cfg(feature = "pak")]
    pub fn attach_paks(uic: &mut UiContext, install: &Path) -> Result<(), String> {
        let pak: Box<dyn crate::assets::AssetSource> = Box::new(crate::pak_assets::PakAssets::open(install)?);
        let old = std::mem::replace(&mut uic.assets.source, Box::new(crate::assets::MemAssets::default()));
        uic.assets.source = Box::new(crate::assets::LayeredAssets::new(vec![old, pak]));
        Ok(())
    }
    /// Localized text: `Game.locres`, else the source string (what a native `LOCTEXT` / an unlocalized `FText` shows in English).
    fn loc(&self, ns: &str, key: &str, source: &str) -> String {
        self.ui.store.locres().and_then(|l| l.get(ns, key).map(|s| s.to_owned())).unwrap_or_else(|| source.to_owned())
    }
    /// `GetUserFriendlyTags` (see `item_header::HeaderData::user_friendly_tags`).
    pub fn user_friendly_tags(&mut self, it: &InvItem) -> Vec<String> { let s = self.ui.store.clone(); self.hdr.user_friendly_tags(&s, it) }
    /// `UFortItem::GetTier` (see `item_header::HeaderData::tier_of`).
    pub fn tier_of(&mut self, def: &str) -> u8 { let s = self.ui.store.clone(); self.hdr.tier_of(&s, def) }

    fn set_vis(&mut self, id: Option<usize>, v: Vis) { if let Some(i) = id { self.ui.w[i].vis = v; } }
    fn set_text(&mut self, id: Option<usize>, t: &str) { if let Some(i) = id { self.ui.w[i].text = Some(t.to_owned()); } }
    fn set_color(&mut self, id: Option<usize>, c: [f32; 4]) { if let Some(i) = id { self.ui.w[i].props["ColorAndOpacity"] = json!(c); } }

    fn ensure_tiles(&mut self, n: usize) {
        while self.tiles.len() < n {
            let Some(r) = self.ui.add_class_instance(TILE_CLASS, None, "", json!({}), None) else { return };
            let v = |ui: &Ui, n: &str| ui.var(r, n).or_else(|| ui.find_in(r, n));
            let t = TileW { root: r, overlay: v(&self.ui, "RootOverlay"), card: v(&self.ui, "ItemWidget"), equip_border: v(&self.ui, "EquipSlotBorder"), equip_overlay: v(&self.ui, "EquipSlotOverlay"), input_wrapper: v(&self.ui, "InputActionWrapper"), input_widget: v(&self.ui, "InputActionWidget") };
            // `UCommonActionWidget` without an `InputActionDataRow` collapses itself (0x1439592c0: row handle zero -> SetVisibility(Collapsed)); the tile's
            // widget has no row in the data and nothing native sets one (`SetInputAction` has no native caller)
            if let Some(w) = t.input_widget { self.ui.w[w].vis = Vis::Collapsed; }
            self.tiles.push(t);
        }
    }

    fn ensure_qsel(&mut self) {
        if self.qsel.is_some() { return; }
        let Some(r) = self.ui.add_class_instance(QSEL_CLASS, None, "", json!({}), None) else { self.missing.push("cannot compose QuantitySelector_C".into()); return };
        let v = |ui: &Ui, n: &str| ui.var(r, n).or_else(|| ui.find_in(r, n));
        let q = QselW { root: r, title: v(&self.ui, "WindowTitle"), current: v(&self.ui, "CurrentAmount"), max_amount: v(&self.ui, "MaxAmount"), additional: v(&self.ui, "AdditionalInfo"), slider: v(&self.ui, "ValueSlider"),
            card: v(&self.ui, "ItemCard"), confirm: v(&self.ui, "ConfirmButton"), cancel: v(&self.ui, "CancelButton"), max_btn: v(&self.ui, "MaxButton") };
        // `SetupForInputMode(false)` (mouse + keyboard): `ValueSlider.WidgetStyle = SliderStyle_MouseKeyboard`
        if let (Some(s), Some(st)) = (q.slider, self.ui.w[r].props.get("SliderStyle_MouseKeyboard").cloned()) { self.ui.w[s].props["WidgetStyle"] = st; }
        // the item card (`FortMultiSizeItemCard`, BPItemCardSize) is an `SBox` of the card size (`UFortMultiSizeItemCard::RebuildWidget`); laid out as that box
        if let Some(c) = q.card {
            let sz = match self.ui.w[c].props.get("BPItemCardSize").and_then(|s| s.as_str()).map(|s| s.rsplit("::").next().unwrap_or(s)) { Some("S") => CARD_S, Some("XS") => 60., Some("XXS") => 44., _ => CARD_M };
            let p = &mut self.ui.w[c].props;
            p["WidthOverride"] = json!(sz); p["HeightOverride"] = json!(sz); p["bOverride_WidthOverride"] = json!(true); p["bOverride_HeightOverride"] = json!(true);
            self.ui.w[c].class = "SizeBox".into();
        }
        // Construct: `bShowConfirmButton` / `bShowCancelButton` false -> Hidden
        for (flag, b) in [("bShowConfirmButton", q.confirm), ("bShowCancelButton", q.cancel)] {
            if !self.ui.w[r].props.get(flag).and_then(|b| b.as_bool()).unwrap_or(true) { self.set_vis(b, Vis::Hidden); }
        }
        self.qsel = Some(q);
    }

    fn apply(&mut self, st: &DrawState) {
        // ---- equip slots: RefreshItem / TryAndShowDropTarget; Selection + InputActionOverlay only when focused with a gamepad (not focusable on mouse + keyboard)
        let drag_ty = st.drag.map(|(d, _)| d.item_type);
        for i in 0..5 {
            let (card, empty, dt, sel, io) = (self.equip[i].card, self.equip[i].empty, self.equip[i].drop_target, self.equip[i].selection, self.equip[i].input_overlay);
            let filled = st.equip[i].is_some();
            self.set_vis(card, if filled { Vis::Visible } else { Vis::Collapsed });
            self.set_vis(empty, if filled { Vis::Collapsed } else { Vis::Visible });
            self.set_vis(dt, if matches!(drag_ty, Some(16) | Some(17)) { Vis::HitTestInvisible } else { Vis::Collapsed });
            self.set_vis(sel, Vis::Collapsed);
            self.set_vis(io, Vis::Collapsed);
        }
        // ---- drop slot Tick: DropTargetOverlay HitTestInvisible while a FortItem payload is dragged, else Collapsed
        let ov = self.drop_overlay;
        self.set_vis(ov, if st.drag.is_some() { Vis::HitTestInvisible } else { Vis::Collapsed });
        // ---- AthenaInventoryLimitStatusIndicator.RefreshWorldInventoryStatus: "{Numerator}" / " / {Denominator}", colour by fill (< 0.6 white, < 0.9
        // Theme-Emphasis Normal, else Theme-Critical Normal: the StyleLibrary stylesheet literal of the call, `Get Theme Color(Theme, ColorType 3 = Normal)`)
        let (n, m) = st.backpack;
        let num = self.loc("[DB67AFF647917A2EF27E2AB5F54DDA42]", "3BC1E91144DD430302E946AFA508D2DF", "{Numerator}").replace("{Numerator}", &group(n as i64));
        let den = self.loc("[DB67AFF647917A2EF27E2AB5F54DDA42]", "67083CC548597B35BCF59E9F136391EE", " / {Denominator}").replace("{Denominator}", &group(m as i64));
        let (cl, ll) = (self.count_label, self.limit_label);
        self.set_text(cl, &num); self.set_text(ll, &den);
        let pct = n as f32 / (m as f32).max(1.);
        let col = if pct < 0.6 { [1., 1., 1., 1.] } else if pct < 0.9 { [0.7519, 0.33446, 0.03026, 1.] } else { [0.19752, 0.01518, 0.00064, 1.] };
        self.set_color(cl, col); self.set_color(ll, col);
        // ---- tiles
        self.ensure_tiles(st.tiles.len());
        for k in 0..self.tiles.len() {
            let t = &self.tiles[k];
            let (root, eb, eo, iw) = (t.root, t.equip_border, t.equip_overlay, t.input_wrapper);
            let Some(ts) = st.tiles.get(k) else { self.ui.w[root].vis = Vis::Collapsed; continue };
            self.ui.w[root].vis = Vis::Visible;
            self.ui.w[root].btn_state = ts.btn;
            // SetEquipSlotOverlay(slot): 1..=5 -> EquipSlotBorder HitTestInvisible + EquipSlotOverlay.SetBrushFromTexture(quickbar_sN), else Collapsed
            match ts.item.slot.filter(|s| (1..=5).contains(s)) {
                Some(s) => {
                    self.set_vis(eb, Vis::HitTestInvisible);
                    if let Some(o) = eo { let mut b = self.ui.w[o].props.get("Brush").and_then(Brush::from_json); if let Some(b) = b.as_mut() { b.resource = Some(format!("/Game/Athena/HUD/Art/quickbar_s{s}.quickbar_s{s}")); } self.ui.w[o].brush = b; }
                }
                None => self.set_vis(eb, Vis::Collapsed),
            }
            // UpdateInteractionWidgetVisibility (OnSelected / OnFocusReceived): IsEquipable && !DoesMatchSearchString("Resource") -> SelfHitTestInvisible, else Collapsed
            if ts.selected { self.set_vis(iw, if ts.item.equipable && !ts.item.is_resource { Vis::SelfHitTestInvisible } else { Vis::Collapsed }); }
        }
        // ---- item details
        self.apply_details(st.selected);
        // ---- quantity selector
        if let Some(q) = st.qsel {
            self.ensure_qsel();
            let Some(w) = self.qsel.as_ref().map(|w| (w.root, w.title, w.current, w.max_amount, w.additional, w.confirm, w.cancel, w.max_btn)) else { return };
            let (root, title, cur, mx, add, confirm, cancel, maxb) = w;
            self.ui.w[root].vis = Vis::Visible;
            // HandleDrop -> CreateQuantitySelectorWidget(item, "Drop") -> SetSplitData(stack / 2, stack, item, "Drop Items", "Drop", empty)
            let t = self.loc("[1B2AE9714898B5DD0D7E3AA25758AAF9]", "928B60E844C6555615B1B983CC793AD3", "Drop Items");
            self.set_text(title, &t);
            let b = self.loc("[1B2AE9714898B5DD0D7E3AA25758AAF9]", "2BD365F444D2DFF7CEC53AAD8FB56DF8", "Drop");
            if let Some(c) = confirm { let tw = self.ui.var(c, "CenterButtonTextWidget"); self.set_text(tw, &b); }
            self.set_vis(add, Vis::Collapsed); // SetAdditionalText(empty): Collapsed
            self.set_text(cur, &group(q.current as i64)); self.set_text(mx, &group(q.max as i64));
            for (btn, which) in [(confirm, QselButton::Confirm), (cancel, QselButton::Cancel), (maxb, QselButton::Max)] {
                let Some(b) = btn else { continue };
                self.ui.w[b].btn_state = if q.pressed == Some(which) { BtnState::Pressed } else if q.hover == Some(which) { BtnState::Hovered } else { BtnState::Normal };
                self.ui.refresh_button_text(b);
            }
        } else if let Some(w) = &self.qsel { let r = w.root; self.ui.w[r].vis = Vis::Collapsed; }
    }

    fn apply_details(&mut self, sel: Option<&InvItem>) {
        let d = self.details_root;
        // OnInventoryItemSelected: ItemDetails Hidden -> HitTestInvisible once something is selected
        self.set_vis(d, if sel.is_some() { Vis::HitTestInvisible } else { Vis::Hidden });
        let Some(it) = sel else { return };
        self.details.apply(&mut self.ui, &mut self.hdr, it);
    }

    /// Lay out and paint the screen. `card` draws an item card (`MaterialKind::ItemCard`) over a pixel rectangle.
    pub fn draw(&mut self, uic: &mut UiContext, w: f32, h: f32, st: &DrawState, card: &mut dyn FnMut(&mut UiContext, &mut RenderList, Rect, &InvItem, bool)) -> (RenderList, Geo) {
        self.apply(st);
        let s = dpi_scale(w, h);
        let size = [w / s, h / s];
        let lay = ul::run(&mut self.ui, uic, self.root, size, s, &[]);
        let px = |r: [f32; 4]| Rect::new(r[0] * s, r[1] * s, r[2] * s, r[3] * s);
        let mut c = Ctx { ui: &mut self.ui, uic, lay };
        // ---- FortItemTileView cells (SListPanel, EvenlyDistributed) in the TileView geometry
        let tv = self.tile_view.map(|t| c.lay.rect[t]).unwrap_or([0.; 4]);
        let tvp = self.tile_view.map(|t| c.ui.w[t].props.clone()).unwrap_or(Value::Null);
        let pad = super::athena_lobby::store::jmargin(tvp.get("DesiredItemPadding"));
        let cw = tvp.get("ItemWidth").and_then(jf).unwrap_or(128.) + pad[0] + pad[2];
        let chh = tvp.get("ItemHeight").and_then(jf).unwrap_or(128.) + pad[1] + pad[3];
        let n = st.tiles.len();
        let cells = tile_cells([tv[2], tv[3]], cw, chh, n, st.scroll / s);
        let content_h = rows_height(tv[2], cw, chh, n);
        for (k, cell) in cells.iter().enumerate() {
            let t = &self.tiles[k];
            let r = [tv[0] + cell[0], tv[1] + cell[1], cell[2], cell[3]];
            // the entry gets `DesiredItemPadding` (UUserWidget::SetPadding), the CommonButton content its style's `ButtonPadding`
            let inner = [r[0] + pad[0], r[1] + pad[1], (r[2] - pad[0] - pad[2]).max(0.), (r[3] - pad[1] - pad[3]).max(0.)];
            c.arrange(t.root, inner);
            let bp = c.ui.w[t.root].props.get("Style").and_then(|s| s.as_str()).map(|s| s.to_owned()).map(|cls| { let store = c.ui.store.clone(); c.ui.styles.button(&store, &cls).padding }).unwrap_or([0.; 4]);
            if let Some(o) = t.overlay { c.arrange(o, [inner[0] + bp[0], inner[1] + bp[1], (inner[2] - bp[0] - bp[2]).max(0.), (inner[3] - bp[1] - bp[3]).max(0.)]); }
        }
        // ---- the modal layer
        if let (Some(q), Some(_)) = (&self.qsel, st.qsel) { c.arrange(q.root, [0., 0., size[0], size[1]]); }
        let lay = c.lay;
        let mut geo = Geo { scale: s, tile_view: px(tv), content_h: content_h * s, ..Default::default() };
        for i in 0..5 { geo.equip[i] = px(lay.rect[self.equip[i].root]); }
        geo.drop = self.drop_root.map(|d| px(lay.rect[d])).unwrap_or(Rect::ZERO);
        geo.tiles = cells.iter().map(|c| px([tv[0] + c[0], tv[1] + c[1], c[2], c[3]])).collect();
        let equip_cards: Vec<Option<Rect>> = (0..5).map(|i| self.equip[i].card.filter(|&c| self.ui.w[c].vis.draws()).map(|c| px(lay.rect[c]))).collect();
        let tile_cards: Vec<Option<Rect>> = (0..n).map(|k| self.tiles[k].card.map(|c| px(lay.rect[c]))).collect();
        let tile_rects: Vec<Rect> = (0..n).map(|k| px(lay.rect[self.tiles[k].root])).collect();
        let drop_targets: Vec<usize> = (0..5).filter_map(|i| self.equip[i].drop_target).filter(|&d| self.ui.w[d].vis.draws()).collect();
        for &d in &drop_targets { self.ui.w[d].vis = Vis::Collapsed; }
        let qsel_ids = self.qsel.as_ref().map(|q| (q.root, q.slider, q.card, q.confirm, q.cancel, q.max_btn));
        let tiles: Vec<(usize, Option<usize>)> = self.tiles.iter().take(n).map(|t| (t.root, None)).collect();
        let mut p = Painter { c: Ctx { ui: &mut self.ui, uic, lay }, out: RenderList::new(), mats: &mut self.mats, blur: vec![], missing: vec![], scale: s, disabled: false };
        p.paint(self.root, Tf::ID, [1.; 4]);
        self.details.paint_tier(&mut p, &self.hdr);
        for &d in &drop_targets { p.c.ui.w[d].vis = Vis::HitTestInvisible; }
        // equip-slot cards (Overlay order: Selection, EmptyImage, ItemCard, DropTarget, InputActionOverlay)
        for (i, r) in equip_cards.iter().enumerate() { if let (Some(r), Some(it)) = (r, st.equip[i]) { card(&mut *p.c.uic, &mut p.out, *r, it, true); } }
        for &d in &drop_targets { p.paint(d, Tf::ID, [1.; 4]); }
        // tiles, clipped to the TileView: button brush (style of the state), card, then the RootOverlay children above it
        p.out.push_clip(geo.tile_view);
        for (k, (root, _)) in tiles.iter().enumerate() {
            let r = tile_rects[k];
            if r.bottom() < geo.tile_view.y || r.y > geo.tile_view.bottom() { continue; }
            let state = p.c.ui.w[*root].btn_state;
            let cls = p.c.ui.w[*root].props.get("Style").and_then(|s| s.as_str()).map(|s| s.to_owned());
            if let Some(cls) = cls {
                let store = p.c.ui.store.clone();
                let bs = p.c.ui.styles.button(&store, &cls);
                if let Some(b) = bs.brush(state).cloned() { let rr = p.c.lay.rect[*root]; p.draw_brush(&b, rr, [1.; 4], Tf::ID, state == BtnState::Disabled); }
            }
            if let Some(cr) = tile_cards[k] { card(&mut *p.c.uic, &mut p.out, cr, &st.tiles[k].item, true); }
            p.paint(*root, Tf::ID, [1.; 4]);
        }
        p.out.pop_clip();
        // the drag visual (AthenaInventoryDragVisual: the card, S from an equip slot / M from a tile, centred on the cursor: UDragDropOperation Pivot CenterCenter)
        if let Some((it, from_slot)) = st.drag {
            let sz = if from_slot { CARD_S } else { CARD_M } * s;
            card(&mut *p.c.uic, &mut p.out, Rect::new(st.cursor.0 - sz * 0.5, st.cursor.1 - sz * 0.5, sz, sz), it, false);
        }
        // the quantity selector on top (modal layer)
        if let (Some((qroot, slider, qcard, confirm, cancel, maxb)), Some(q)) = (qsel_ids, st.qsel) {
            p.paint(qroot, Tf::ID, [1.; 4]);
            if let Some(cr) = qcard { let r = p.c.lay.rect[cr]; card(&mut *p.c.uic, &mut p.out, px(r), &q.item, false); }
            let mut qg = QselGeo::default();
            if let Some(sl) = slider {
                let r = p.c.lay.rect[sl];
                qg.slider = px(r);
                qg.thumb_w = paint_slider(&mut p, sl, r, q.slider) * s;
            }
            let g = |id: Option<usize>, p: &Painter| id.filter(|&i| p.c.ui.w[i].vis.draws()).map(|i| px(p.c.lay.rect[i])).unwrap_or(Rect::ZERO);
            qg.confirm = g(confirm, &p); qg.cancel = g(cancel, &p); qg.max = g(maxb, &p);
            geo.qsel = Some(qg);
        }
        for m in std::mem::take(&mut p.missing) { if !self.missing.contains(&m) { eprintln!("[inventory] not drawn: {m}"); self.missing.push(m); } }
        (std::mem::take(&mut p.out), geo)
    }
}

/// `SSlider::OnPaint` (UE 4.16, `IndentHandle` true by default in UMG): bar `BarThickness` high between the half-thumb insets, thumb `NormalThumbImage.ImageSize`
/// at `value * (width - thumb) `. Returns the thumb width (Slate units).
fn paint_slider(p: &mut Painter, id: usize, r: [f32; 4], value: f32) -> f32 {
    let style = p.c.ui.w[id].props.get("WidgetStyle").cloned().unwrap_or(Value::Null);
    let bar = style.get("NormalBarImage").and_then(Brush::from_json);
    let thumb = style.get("NormalThumbImage").and_then(Brush::from_json);
    let thick = style.get("BarThickness").and_then(jf).unwrap_or(2.);
    let hs = thumb.as_ref().map(|t| t.image_size).unwrap_or([0., 0.]);
    let v = value.clamp(0., 1.);
    let (w, h) = (r[2], r[3]);
    let len = w - hs[0];
    let y = 0.5 * h;
    if let Some(b) = bar { p.draw_brush(&b, [r[0] + hs[0] * 0.5, r[1] + y - thick * 0.5, w - hs[0], thick], [1.; 4], Tf::ID, false); }
    if let Some(t) = thumb { p.draw_brush(&t, [r[0] + v * len - hs[0] * v + 0.5 * hs[0], r[1] + y - hs[1] * 0.5, hs[0], hs[1]], [1.; 4], Tf::ID, false); }
    let _ = len;
    hs[0]
}

/// `SListPanel::OnArrangeChildren` for a tile view (UE 4.16) with `EListItemAlignment::EvenlyDistributed`: cells of `cw` x `ch` (Slate units) left to right,
/// padding `(W - 0.001 - NumItemsWide * cw) / NumItemsWide` between them only when there are more items than fit on one row, first row at `-scroll`.
pub fn tile_cells(area: [f32; 2], cw: f32, ch: f32, n: usize, scroll: f32) -> Vec<[f32; 4]> {
    if cw <= 0. { return vec![]; }
    let wide = (area[0] / cw).floor() as usize;
    let pad = if wide > 0 && n > wide { (area[0] - 0.001 - wide as f32 * cw) / wide as f32 } else { 0. };
    let (mut x, mut y) = (0f32, -scroll.floor());
    let mut out = vec![];
    for _ in 0..n {
        out.push([x + pad * 0.5, y, cw, ch]);
        x += cw + pad;
        if x + cw + pad > area[0] { x = 0.; y += ch; }
    }
    out
}
/// Height of all tile rows (Slate units).
pub fn rows_height(w: f32, cw: f32, ch: f32, n: usize) -> f32 {
    if cw <= 0. || n == 0 { return 0.; }
    let wide = ((w / cw).floor() as usize).max(1);
    ((n + wide - 1) / wide) as f32 * ch
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn evenly_distributed_cells() {
        // 1100 wide, 254 cells: 4 per row; padding only once there are more than 4 items
        let c = tile_cells([1100., 600.], 254., 439., 3, 0.);
        assert_eq!(c.iter().map(|c| c[0]).collect::<Vec<_>>(), [0., 254., 508.]);
        let c = tile_cells([1100., 600.], 254., 439., 6, 0.);
        let pad = (1100. - 0.001 - 4. * 254.) / 4.;
        assert!((c[0][0] - pad * 0.5).abs() < 1e-3 && (c[1][0] - (254. + pad * 1.5)).abs() < 1e-3);
        assert_eq!((c[4][0], c[4][1]), (pad * 0.5, 439.));
        assert_eq!(rows_height(1100., 254., 439., 6), 878.);
        assert_eq!(tile_cells([1100., 600.], 254., 439., 1, 32.)[0][1], -32.);
    }
    #[test] fn item_type_names() { assert_eq!(item_type_text(17), ("WeaponRanged", "Ranged Weapon")); assert_eq!(item_type_index("EFortItemType::WorldResource"), Some(19)); }
    #[test] fn cooked_panel_composes_and_draws() {
        let p = crate::pak_assets::default_install();
        if !p.exists() { return; }
        let mut uic = UiContext::from_install(&p, None).expect("ui");
        let mut ci = CookedInventory::load(&p).expect("compose");
        let it = |id: u32, ty: u8, slot: Option<u8>, count: u32, def: &str| InvItem { id, name: format!("Item {id}"), description: "desc".into(), rarity: 1, icon: String::new(), count, item_type: ty, equipable: ty != 19, is_resource: ty == 19, can_be_dropped: true, slot, def_path: def.into() };
        let ar = it(1, 17, Some(1), 1, "/Game/Athena/Items/Weapons/WID_Assault_Auto_Athena_C_Ore_T02");
        assert_eq!(ci.user_friendly_tags(&ar), ["Ranged", "Assault"], "PrimaryCategories(WeaponRanged) + SecondaryCategories(weapon.ranged.assault)");
        let wood = it(5, 19, None, 400, "");
        let tiles = vec![TileState { item: ar.clone(), btn: BtnState::Selected, selected: true }, TileState { item: wood.clone(), btn: BtnState::Normal, selected: false }];
        let q = QselState { item: wood.clone(), current: 200, max: 400, slider: 199. / 399., hover: None, pressed: None };
        let st = DrawState { equip: [Some(&ar), None, None, None, None], tiles: &tiles, scroll: 0., drag: None, cursor: (0., 0.), backpack: (1, 5), selected: Some(&ar), qsel: Some(&q) };
        let mut cards = 0;
        let (list, geo) = ci.draw(&mut uic, 1920., 1080., &st, &mut |_, _, _, _, _| cards += 1);
        assert!(!list.is_empty());
        assert_eq!(geo.tiles.len(), 2); assert!((geo.tiles[0].w - 254.).abs() < 0.01 && (geo.tiles[0].h - 439.).abs() < 0.01, "{:?}", geo.tiles[0]);
        assert_eq!(cards, 1 + 2 + 1, "equip slot 1, two tiles, the selector's card");
        let qg = geo.qsel.expect("selector geometry"); assert!(qg.confirm.w > 0. && qg.cancel.w > 0. && qg.slider.w > 0.);
        eprintln!("geo {geo:?}\nmissing {:?}", ci.missing);
        // INVENTORY FIX4 (UE 4.16 SBoxPanel): `VerticalBox_22` has three Fill slots (padding 14 / 0 / 20) and one Auto slot (`SizeBox_87` 40 + 20) in its
        // 222.7 units: every slot's padding is fixed space, so Max (HorizontalBox_2, VAlign Fill) and Drop / Cancel (HorizontalBox_0, VAlign_Bottom, desired
        // 48 > the share) are all (222.7 - 94) / 3 = 42.9 tall
        assert!((qg.max.h - qg.confirm.h).abs() < 0.01 && (qg.confirm.h - qg.cancel.h).abs() < 0.01 && (qg.max.h - 49.65).abs() < 0.05, "{qg:?}"); // TEXT SIZE (2026-10-06): 42.89 before the texts were drawn at Slate points (96/72): the taller texts grow the selector
        // INVENTORY FIX4 (verify3 V6): a Rare sniper (`Tier` EFortItemTier::III) shows 3 `T-Stars-Full-S` pips (24 x 24) centred in the LevelBar and the
        // Sniper Rifle category icon (`T-Icon-Sniper-32`, Brush_M 32 x 32) before the tag list
        let sn = it(9, 17, None, 1, "/Game/Athena/Items/Weapons/WID_Sniper_BoltAction_Scope_Athena_R_Ore_T03");
        assert_eq!(ci.tier_of(&sn.def_path), 3);
        assert_eq!(ci.tier_of(&wood.def_path), 0);
        let st = DrawState { equip: [None; 5], tiles: &[], scroll: 0., drag: None, cursor: (0., 0.), backpack: (1, 5), selected: Some(&sn), qsel: None };
        let (list, _) = ci.draw(&mut uic, 1920., 1080., &st, &mut |_, _, _, _, _| {});
        let (t, c, lb) = (ci.details.tier.unwrap(), ci.details.category.unwrap(), ci.details.level_bar.unwrap());
        let lay = ul::run(&mut ci.ui, &mut uic, ci.root, [1920., 1080.], 1., &[]);
        let (tr, cr, lr) = (lay.rect[t], lay.rect[c], lay.rect[lb]);
        eprintln!("tier {tr:?} category {cr:?} level bar {lr:?}");
        assert_eq!((tr[2], tr[3]), (72., 24.), "3 pips of 24");
        assert!(((tr[0] + tr[2] * 0.5) - (lr[0] + lr[2] * 0.5)).abs() < 0.6, "centred in the level bar");
        assert_eq!((cr[2], cr[3]), (32., 32.));
        assert!(ci.ui.w[c].brush.as_ref().and_then(|b| b.resource.clone()).unwrap_or_default().ends_with("T-Icon-Sniper-32"));
        let star = uic.assets.texture("/Game/UI/Foundation/Textures/Icons/Stars/T-Stars-Full-S.png").id;
        assert_ne!(star, crate::draw::TextureId::NONE);
        let n = list.cmds.iter().filter(|c| matches!(c, crate::draw::Cmd::Quad(q) if q.tex == star)).count();
        assert_eq!(n, 3, "three star quads");
    }
}
