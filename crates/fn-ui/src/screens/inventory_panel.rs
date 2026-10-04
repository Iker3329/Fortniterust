//! `AthenaInventoryPanel_C` (`/Game/Athena/HUD/Inventory/AthenaInventoryPanel`): the full inventory screen of the Athena HUD (Toggle Inventory, Tab), as pure state -> [`RenderList`].
//!
//! SOURCES (fn-inspect `props` / `disasm` of `AthenaInventoryPanel`, `AthenaInventoryEquipSlot`, `AthenaInventoryDropSlot`, `AthenaInventoryFortItemTileButton`, `AthenaInventoryDragVisual`,
//! `AthenaInventoryLimitStatusIndicator`, the text styles, `RarityData`, the item definitions; widget tree dumped to `reports`-style notes in the task report):
//! * Root `CommonBorder_2` (Image brush `VerticalRibbonGradient-Inventory`, full screen) > `SafeZone_0` (no top / bottom pad) > `SizeBox_0` (HAlign Center) > `VerticalBox_0` =
//!   [header `HorizontalBox_1` > `BorderSeasonHeader` > `SizeBox_2` (Height 48) > `CommonBorder_3` (padding L30 R50, bottom aligned) > `PageTitle` ("INVENTORY", `TextStyle-Header-M-70pc` =
//!   Burbank 32, white 0.7, slot padding L30 R50), `HorizontalBox_9` (Fill) = [`SizeBox_22` (MinDesiredWidth 1100, Fill) > `VerticalBox_3` > `VerticalBox_2` (padding top 16, Fill) = [
//!   `CommonBorderEquipSlots` (style `Border-MainL-Black`, HAlign Center, slot padding L40 R40 B40) > `EquipSlots` (5 x `AthenaInventoryEquipSlot`, 130 x 130 `SizeBox_8`, slot padding L4 R4),
//!   `BackpackHeader` (VAlign Center) = [`InventoryLimitStatusBackpack` ("Backpack " `TextStyle-Base-S` + `{Numerator}` `-S-B` + "/{Denominator}" `-S`, colours below), `Image_2` (white, 3 high, padding
//!   T4 B8 R8)], `TileViewLoadGuard` (Fill, padding B8) > `TileView` (`FortItemTileView`, entries `AthenaInventoryFortItemTileButton`), `SizeBox_4` (MinDesiredHeight 175) >
//!   `AthenaInventoryDropSlot` (118 x 118)], `VerticalBox_1` (padding L12 T16 B4) > `SizeBox_1` (MinDesiredWidth 700, HAlign Right, VAlign Center) > `ItemDetails`]].
//! * Equip slot (130 box): Overlay = [Selection, EmptyImage (padding 6), ItemCard (`FortMultiSizeItemCard` M, padding 3), DropTarget (`Icon-QuickbarSlot-Highlight` Border brush, margin
//!   0.14), InputActionOverlay]. `RefreshItem`: the item slotted at (primary bar, SlotIndex) -> card visible + EmptyImage hidden, else the opposite. `Tick/TryAndShowDropTarget`: DropTarget is
//!   visible while a drag with an item payload of type 16 / 17 is running. `OnMouseButtonDown`: mouse = `DetectDragIfPressed(LeftMouseButton)`, gamepad = `OnEquipRequested`.
//!   `OnDragDetected` (card valid): drag visual = `AthenaInventoryDragVisual` (card size 2), operation `AthenaInventoryDragDrop` Tag "FromQuickbar", Payload = the item.
//!   `OnDrop` -> `FortInventoryContext.DropItemOnQuickBar(item, 0, SlotIndex)` (native, see `fn_weapons::native_inventory`).
//! * Tile (`AthenaInventoryFortItemTileButton`, style `ItemManagementTileButtonStyle-Base`, `ButtonPadding` 4): card `M`, `EquipSlotBorder` (black 0.33 over the card, with the
//!   `quickbar_s<N>` texture centred at the top) while the item is slotted at equip slot N (1..=5); `OnDragDetected` as above with `SplitTheStack = IsShiftDown`.
//! * Drop slot (118 box): `DropTargetOverlay` (EmptyImage + `T_AthenaDropItem` with padding 15) is Collapsed until a drag with an item payload runs (`Tick`); `OnDrop` ->
//!   `FortInventoryContext.DropItem(item, quantity)` with `fn_weapons::recovered_inventory::drag_drop_quantity`.
//! * Selection: clicking a tile selects it (`OnInventoryItemSelected` -> `AthenaHUDContext.SetSelectedInventoryItem`, `ItemDetails.SetItemToDetail`, `ItemDetails` Hidden -> HitTestInvisible
//!   once something is selected: `SetVisibility(2)` before, `(3)` after). `Equip` input action = `PerformEquip(selected)` (`fn_weapons::recovered_panel::perform_equip`).
//! * Item details (`ItemInspectionMainItemDetailsHostPanel` > `ItemDetails`): header `RarityBorder` (padding 8 / 16) = rarity text (`TextStyle-Base-XS-B`) + type text, name
//!   (`TextStyle-Header-M` = Burbank 32, slot padding -2 / -2), tag list; body `CommonBorder_8` (style `Border_DarkNavy_VGrad` = M-Border-MainL with RGBA0 (0.0284, 0.0382, 0.0823),
//!   RGBA1 (0.0137, 0.0176, 0.0452), Global Alpha 0.7) > ScrollBox > description (`TextStyle-Base-XS-I-40pc`, auto wrap, padding 16 / 8).
//!
//! DRAWN FROM THE ORIGINAL: the background texture, the equip-row and details-body borders (the cooked `M-Border-MainL` shader of `material::uimat` with the instances' parameter overrides
//! `M-Border-MainL-Black` and `M_Border_DarkNavy_VGrad`), the item cards (the translated `M-Item-Inst-S` card shader), `T_AthenaDropItem`, `Icon-QuickbarSlot-Highlight`, `quickbar_s1..5`, the
//! text styles, RarityData names / colours, item names / descriptions (from the item definitions).
//!
//! NOT RECOVERED / NOT DRAWN (reported, nothing substituted): the header materials `M_AthenaStore_OfferHeader(Backing)` (`M_Button_List_Master` instances; not translated), the empty equip slot
//! frame `M_Slot_People_M_Empty`, the slot / tile selection and hover materials (`M_Solid_Square_S_Edit`, `M_Slot_Selected`, `M_Slot_Pressed`), the `Border-ItemInfo-Header` rarity backing and the
//! LevelBar (`M-Border-Item-DetailsHeader-Inst`, `M-Border-ItemInfoHeader_BrightBar`), the item cards use the `M-Item-Inst-S` instance parameters (the M instance differs in `gStrokeThickness`
//! 0.465, `ItemBackDensity` 1.25, `ItemBackRadius` 0.65), the native `FortItemTileView` cell size (`ItemWidth` 254 / `ItemHeight` 439 are NOT applied: a tile is the 124 x 124 card + the 4 unit
//! `ButtonPadding`), the native text of the header (type name, tag list, stack counter), the card count text style (the quickbar's is reused), the `NotoSans-Italic` face (Regular is used),
//! the keyboard key of the `Equip` input action (CommonUI row data not decoded: Enter), the quantity selector modal of `HandleDrop` and the gamepad paths.
use crate::assets::{Sampler, Texture};
use crate::draw::{Cmd, MaterialKind, RenderList, TextureId};
use crate::layout::{dpi_scale, Rect};
use crate::material::uimat::{self, Draw, Mat, MatParams, MatState};
use crate::scene::UiContext;
use crate::screens::quickbar::RARITY;
use crate::screens::{art, atlas_px, centred_baseline, line_height, mat_tex, put_text, srgb3, text_width, Snap, WHITE};
use crate::text::FontId;

// ---- decoded numbers (Slate units) ----
pub const HEADER_H: f32 = 48.;
pub const COLUMN_MIN_W: f32 = 1100.; // SizeBox_22 MinDesiredWidth
pub const DETAILS_W: f32 = 700.; // SizeBox_1 MinDesiredWidth
pub const DETAILS_PAD: (f32, f32, f32) = (12., 16., 4.); // VerticalBox_1 slot padding Left, Top, Bottom
pub const COLUMN_TOP_PAD: f32 = 16.; // VerticalBox_2 slot padding Top
pub const EQUIP_BOX: f32 = 130.; // AthenaInventoryEquipSlot SizeBox_8
pub const EQUIP_SLOT_PAD: f32 = 4.; // EquipSlots HorizontalBoxSlot Padding L/R
pub const EQUIP_CARD_PAD: f32 = 3.; // ItemCard overlay slot padding
pub const EQUIP_OUTER_PAD: (f32, f32) = (40., 40.); // CommonBorderEquipSlots slot padding L/R, bottom
pub const DROP_BOX: f32 = 118.; // AthenaInventoryDropSlot SizeBox_8
pub const DROP_IMAGE_PAD: f32 = 15.; // DropTarget overlay slot padding
pub const DROP_BAND_MIN_H: f32 = 175.; // SizeBox_4 MinDesiredHeight
pub const TILE_BOTTOM_PAD: f32 = 8.; // TileViewLoadGuard slot padding Bottom
pub const TILE_BUTTON_PAD: f32 = 4.; // ItemManagementTileButtonStyle-Base ButtonPadding
pub const CARD_M: f32 = EQUIP_BOX - 2. * EQUIP_CARD_PAD; // the M card as the equip slot shows it (Fill inside the 130 box, padding 3)
pub const TILE: f32 = CARD_M + 2. * TILE_BUTTON_PAD;
pub const BACKPACK_LINE: (f32, f32, f32, f32) = (3., 4., 8., 8.); // Image_2 height, padding Top, Bottom, Right
pub const TITLE_PAD_L: f32 = 30. + 30.; // CommonBorder_3 padding Left + PageTitle slot padding Left
pub const TITLE_SIZE: f32 = 32.; // TextStyle-Header-M
pub const TITLE_ALPHA: f32 = 0.7; // TextStyle-Header-M-70pc
pub const BASE_S: f32 = 18.; // TextStyle-Base-S
pub const BASE_XS: f32 = 16.; // TextStyle-Base-XS(-B / -I-40pc)
pub const DRAG_START_UNITS: f32 = 5.; // UE default drag trigger distance
/// `InventoryLimitStatusBackpack` colours (linear).
pub const LABEL_COLOR: [f32; 4] = [0.008023, 0.01096, 0.026241, 0.7];
pub const COUNT_COLOR: [f32; 3] = [0.001821, 0.051269, 0.304987];
pub const EQUIP_TILE_BORDER: [f32; 4] = [0., 0., 0., 0.33]; // EquipSlotBorder TintColor
pub const QUICKBAR_OVERLAY: (f32, f32) = (128., 32.); // EquipSlotOverlay ImageSize
pub const PAGE_TITLE: &str = "INVENTORY";
/// `RarityData` `RarityCollection[N].Name`.
pub const RARITY_NAMES: [&str; 5] = ["Common", "Uncommon", "Rare", "Epic", "Legendary"];

/// One inventory entry as the panel shows it (the game side fills it from its live inventory, see `fn-app/src/inventory.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct InvItem {
    pub id: u32,
    /// Item definition `DisplayName` / `Description` texts.
    pub name: String,
    pub description: String,
    /// 0 Common .. 4 Legendary (`RarityData` order).
    pub rarity: u8,
    /// PNG name of the `SmallPreviewImage` texture.
    pub icon: String,
    pub count: u32,
    /// `EFortItemType` number (declaration order of the exe's enum name table).
    pub item_type: u8,
    /// `IsEquipable(controller)` and "matches the search string `Resource`" (the two inputs of the `Equip` action's enabled state).
    pub equipable: bool,
    pub is_resource: bool,
    /// `FortWorldItem::CanBeDropped()`.
    pub can_be_dropped: bool,
    /// Equip slot 1..=5 (primary quickbar slot index + 0 ... 1..5; the pickaxe slot 0 is not an equip slot) holding this item.
    pub slot: Option<u8>,
}

/// Everything the panel shows: the entries and the backpack counter (`{Numerator}` / `{Denominator}` of `InventoryLimitStatusBackpack`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InventoryView { pub items: Vec<InvItem>, pub backpack: (u32, u32) }

/// What the panel asks the host to do (the host owns the inventory: the native context calls are not part of the screen).
#[derive(Clone, Debug, PartialEq)]
pub enum PanelAction {
    /// A tile was clicked: `AthenaHUDContext.SetSelectedInventoryItem` + details.
    Select(u32),
    /// The `Equip` input action on the selected item: the host runs `PerformEquip` (`fn_weapons::recovered_panel::perform_equip`).
    Equip(u32),
    /// `AthenaInventoryEquipSlot.OnDrop`: `DropItemOnQuickBar(item, 0, slot)` with `slot` = SlotIndex 1..=5.
    DropOnSlot { item: u32, slot: u8 },
    /// `AthenaInventoryDropSlot.OnDrop`: `DropItem(item, quantity)`; `split` = `SplitTheStack || IsShiftDown` (the host applies `drag_drop_quantity`).
    Drop { item: u32, split: bool },
    /// `HandleClose` (`EnterCameraMode`): Back / LeaveInventory.
    Close,
}

#[derive(Clone, Copy, Debug)]
struct Press { item: u32, at: (f32, f32), shift: bool }
/// `from_slot` = the operation's Tag "FromQuickbar" (set by `AthenaInventoryEquipSlot.OnDragDetected`).
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
struct Drag { item: u32, split: bool, from_slot: bool }

/// Resolved geometry in pixels.
#[derive(Clone, Debug)]
pub struct Layout {
    pub scale: f32,
    pub panel_left: f32,
    pub title: Rect,
    pub equip_border: Rect,
    pub equip: [Rect; 5],
    pub backpack_row: Rect,
    pub backpack_line: Rect,
    pub tiles: Rect,
    pub drop: Rect,
    pub details: Rect,
}

/// Geometry for a client size. `backpack_text_h` = line height of `TextStyle-Base-S` in pixels, `details_h` = height of the details widget in pixels (0 = none).
pub fn layout(w: f32, h: f32, backpack_text_h: f32, details_h: f32) -> Layout {
    let s = dpi_scale(w, h);
    let (sw, sh) = (w / s, h / s);
    let panel_w = (COLUMN_MIN_W + DETAILS_PAD.0 + DETAILS_W).min(sw);
    let left = ((sw - panel_w) * 0.5) * s;
    let col_w = (panel_w - DETAILS_PAD.0 - DETAILS_W) * s;
    let top = (HEADER_H + COLUMN_TOP_PAD) * s;
    let title = Rect::new(left, 0., panel_w * s, HEADER_H * s);
    let row_w = 5. * (EQUIP_BOX + 2. * EQUIP_SLOT_PAD) * s;
    let eq_x = left + (col_w - row_w) * 0.5;
    let equip_border = Rect::new(eq_x, top, row_w, EQUIP_BOX * s);
    let equip: [Rect; 5] = std::array::from_fn(|i| Rect::new(eq_x + (i as f32 * (EQUIP_BOX + 2. * EQUIP_SLOT_PAD) + EQUIP_SLOT_PAD) * s, top, EQUIP_BOX * s, EQUIP_BOX * s));
    let bp_y = top + (EQUIP_BOX + EQUIP_OUTER_PAD.1) * s;
    let backpack_row = Rect::new(left, bp_y, col_w, backpack_text_h);
    let backpack_line = Rect::new(left, bp_y + backpack_text_h + BACKPACK_LINE.1 * s, col_w - BACKPACK_LINE.3 * s, BACKPACK_LINE.0 * s);
    let band_y = (sh - DROP_BAND_MIN_H) * s;
    let drop = Rect::new(left + (col_w - DROP_BOX * s) * 0.5, band_y + (DROP_BAND_MIN_H - DROP_BOX) * 0.5 * s, DROP_BOX * s, DROP_BOX * s);
    let tiles_y = backpack_line.bottom() + BACKPACK_LINE.2 * s;
    let tiles = Rect::new(left, tiles_y, col_w, (band_y - TILE_BOTTOM_PAD * s - tiles_y).max(0.));
    let col_h = (sh - HEADER_H - DETAILS_PAD.1 - DETAILS_PAD.2) * s;
    let d_h = details_h.min(col_h);
    let details = Rect::new(left + (col_w / s + DETAILS_PAD.0 + DETAILS_W - DETAILS_W) * s, top + (col_h - d_h) * 0.5, DETAILS_W * s, d_h);
    Layout { scale: s, panel_left: left, title, equip_border, equip, backpack_row, backpack_line, tiles, drop, details }
}

/// Tile rectangles of the visible window, row-major in cells of `TILE` units, scrolled by `scroll` pixels.
pub fn tile_rects(area: Rect, s: f32, n: usize, scroll: f32) -> Vec<Rect> {
    let cell = TILE * s;
    if area.w < cell || cell <= 0. { return vec![]; }
    let cols = ((area.w / cell).floor() as usize).max(1);
    (0..n).map(|i| Rect::new(area.x + (i % cols) as f32 * cell, area.y + (i / cols) as f32 * cell - scroll, cell, cell)).collect()
}
/// Largest scroll offset (pixels) for `n` tiles.
pub fn max_scroll(area: Rect, s: f32, n: usize) -> f32 {
    let cell = TILE * s;
    if area.w < cell { return 0.; }
    let cols = ((area.w / cell).floor() as usize).max(1);
    let rows = (n + cols - 1) / cols;
    (rows as f32 * cell - area.h).max(0.)
}

#[derive(Clone, Copy, Debug, Default)]
struct Metrics { s_line: f32, xs_line: f32, title_line: f32 }

pub struct InventoryPanel {
    pub open: bool,
    view: InventoryView,
    selected: Option<u32>,
    cursor: (f32, f32),
    press: Option<Press>,
    drag: Option<Drag>,
    scroll: f32,
    size: (f32, f32),
    m: Metrics,
    actions: Vec<PanelAction>,
}
impl Default for InventoryPanel { fn default() -> Self { InventoryPanel { open: false, view: InventoryView::default(), selected: None, cursor: (0., 0.), press: None, drag: None, scroll: 0., size: (1920., 1080.), m: Metrics { s_line: 25., xs_line: 22., title_line: 40. }, actions: vec![] } } }

impl InventoryPanel {
    pub fn new() -> Self { Self::default() }
    /// `ItemDetails.SetVisibility`: Hidden while nothing is selected.
    pub fn selected(&self) -> Option<u32> { self.selected }
    pub fn dragging(&self) -> Option<u32> { self.drag.map(|d| d.item) }
    pub fn view(&self) -> &InventoryView { &self.view }
    pub fn scroll(&self) -> f32 { self.scroll }
    /// Open (`OnActivated`: `UpdateEquipSlots`, `TrySetupInputHandling`) / close (`OnDeactivated`: handlers removed; the drag and press state is dropped).
    pub fn set_open(&mut self, open: bool) { self.open = open; if !open { self.press = None; self.drag = None; } }
    pub fn toggle(&mut self) { let o = !self.open; self.set_open(o); }
    /// Hand over the live inventory of this frame. A selection whose item vanished is cleared.
    pub fn set_view(&mut self, v: InventoryView) {
        self.view = v;
        if let Some(id) = self.selected { if !self.view.items.iter().any(|i| i.id == id) { self.selected = None; } }
        if self.drag.map_or(false, |d| !self.view.items.iter().any(|i| i.id == d.item)) { self.drag = None; }
        if self.press.map_or(false, |p| !self.view.items.iter().any(|i| i.id == p.item)) { self.press = None; }
    }
    pub fn select(&mut self, id: u32) { if self.view.items.iter().any(|i| i.id == id) { self.selected = Some(id); } }
    pub fn take_actions(&mut self) -> Vec<PanelAction> { std::mem::take(&mut self.actions) }
    pub fn set_viewport(&mut self, w: f32, h: f32) { self.size = (w.max(1.), h.max(1.)); }

    fn item(&self, id: u32) -> Option<&InvItem> { self.view.items.iter().find(|i| i.id == id) }
    fn equip_item(&self, slot: u8) -> Option<&InvItem> { self.view.items.iter().find(|i| i.slot == Some(slot)) }
    /// Geometry for hit tests (the details rectangle is not hit-tested, so its height is not needed here).
    fn lay(&self) -> Layout { layout(self.size.0, self.size.1, self.m.s_line, 0.) }

    // ---------------------------------------------------------------- input
    pub fn mouse_move(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
        if self.drag.is_none() {
            if let Some(p) = self.press {
                let s = dpi_scale(self.size.0, self.size.1);
                if (x - p.at.0).hypot(y - p.at.1) >= DRAG_START_UNITS * s {
                    if let Some(it) = self.item(p.item) { let from_slot = it.slot.is_some() && self.lay().equip.iter().enumerate().any(|(i, r)| r.contains(p.at.0, p.at.1) && it.slot == Some(i as u8 + 1)); self.drag = Some(Drag { item: p.item, split: p.shift, from_slot }); }
                }
            }
        }
    }
    /// Left button down at (x, y): arms a drag on an item tile / filled equip slot (`DetectDrag` / `DetectDragIfPressed`).
    pub fn mouse_down(&mut self, x: f32, y: f32, shift: bool) {
        self.cursor = (x, y);
        let l = self.lay();
        let item = self.hit_tile(&l, x, y).or_else(|| (0..5).find(|i| l.equip[*i].contains(x, y)).and_then(|i| self.equip_item(i as u8 + 1).map(|it| it.id)));
        self.press = item.map(|id| Press { item: id, at: (x, y), shift });
    }
    /// Left button up: a drag is resolved on the equip slot / drop slot under the cursor, a plain click on a tile selects it.
    pub fn mouse_up(&mut self, x: f32, y: f32, shift: bool) {
        self.cursor = (x, y);
        let l = self.lay();
        if let Some(d) = self.drag.take() {
            self.press = None;
            if let Some(i) = (0..5).find(|i| l.equip[*i].contains(x, y)) { self.actions.push(PanelAction::DropOnSlot { item: d.item, slot: i as u8 + 1 }); }
            else if l.drop.contains(x, y) { self.actions.push(PanelAction::Drop { item: d.item, split: d.split || shift }); }
            return;
        }
        if let Some(p) = self.press.take() {
            if let Some(id) = self.hit_tile(&l, x, y).filter(|id| *id == p.item) { self.selected = Some(id); self.actions.push(PanelAction::Select(id)); }
        }
    }
    pub fn wheel(&mut self, notches: f32) {
        let l = self.lay();
        if !l.tiles.contains(self.cursor.0, self.cursor.1) { return; }
        let m = max_scroll(l.tiles, l.scale, self.view.items.len());
        self.scroll = (self.scroll - notches * TILE * l.scale).clamp(0., m);
    }
    /// The `Equip` input action (`RegisterEquip`): only live for an equipable, non-resource selected item.
    pub fn key_equip(&mut self) {
        let id = self.selected.and_then(|id| self.item(id)).filter(|it| it.equipable && !it.is_resource).map(|it| it.id);
        if let Some(id) = id { self.actions.push(PanelAction::Equip(id)); }
    }
    /// Back / LeaveInventory (`HandleClose`).
    pub fn key_close(&mut self) { self.actions.push(PanelAction::Close); }

    fn hit_tile(&self, l: &Layout, x: f32, y: f32) -> Option<u32> {
        if !l.tiles.contains(x, y) { return None; }
        tile_rects(l.tiles, l.scale, self.view.items.len(), self.scroll).iter().position(|r| r.contains(x, y)).map(|i| self.view.items[i].id)
    }

    // ---------------------------------------------------------------- drawing
    fn border(ctx: &mut UiContext, out: &mut RenderList, r: Rect, s: f32, navy: bool) {
        if r.w < 1. || r.h < 1. { return; }
        let key = format!("inv-border:{}:{}x{}", if navy { "navy" } else { "black" }, r.w.round() as u32, r.h.round() as u32);
        let id = match ctx.assets.store.named(&key) {
            Some(id) => id,
            None => {
                let (pw, ph) = (r.w.round().max(1.) as u32, r.h.round().max(1.) as u32);
                let mut p = MatParams { size_px: (r.w / s, r.h / s), ..Default::default() };
                if navy { p.overrides = vec![("Global Alpha", [0.7; 4]), ("Rotation", [0.; 4]), ("RGBA0", [0.028426, 0.038204, 0.082283, 1.]), ("RGBA1", [0.013702, 0.017642, 0.045186, 1.])]; }
                else { p.overrides = vec![("Global Alpha", [0.4; 4]), ("RGBA0", [0., 0., 0., 1.]), ("RGBA1", [0.01, 0.01, 0.01, 1.])]; }
                // the style brushes Border-MainL-Black / Border_DarkNavy_VGrad carry no margin and no ImageSize: a plain Image draw of the material
                let mut b = uimat::default_brush(Mat::BorderMainL, MatState::Base).expect("Border-MainL brush");
                b.draw = Draw::Image; b.margin = [0.; 4]; b.tint = [1.; 4];
                let px = uimat::raster_brush(&b, pw, ph, &p);
                ctx.assets.store.insert_named(&key, Texture::new(pw, ph, px, Sampler::LINEAR))
            }
        };
        out.sprite(id, r, WHITE);
    }

    fn card(ctx: &mut UiContext, out: &mut RenderList, r: Rect, it: &InvItem, hero: TextureId) {
        let icon = if it.icon.is_empty() { TextureId::NONE } else { mat_tex(ctx, &it.icon, false).id };
        let c = RARITY[it.rarity.min(4) as usize];
        out.material(MaterialKind::ItemCard { colors: [c[0], c[1], c[2], c[3]], icon, hero_tri: hero }, r, WHITE, false);
    }

    fn count_text(ctx: &mut UiContext, out: &mut RenderList, r: Rect, n: u32, s: f32) {
        let px = atlas_px(crate::screens::quickbar::KEY_FONT, s);
        let txt = n.to_string();
        let tw = text_width(ctx, FontId::Bold, &txt, px);
        let lh = line_height(ctx, FontId::Bold, px);
        let base = centred_baseline(ctx, FontId::Bold, px, r.bottom() - 2. * s - lh, lh);
        put_text(ctx, out, FontId::Bold, px, r.right() - 4. * s - tw, base, &txt, WHITE, None, Snap::PerGlyph);
    }

    /// Word-wrapped lines of `text` for a width in pixels.
    pub fn wrap(ctx: &UiContext, font: FontId, px: u32, text: &str, width: f32) -> Vec<String> {
        let mut lines = vec![]; let mut cur = String::new();
        for word in text.split_whitespace() {
            let cand = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
            if cur.is_empty() || text_width(ctx, font, &cand, px) <= width { cur = cand; } else { lines.push(std::mem::take(&mut cur)); cur = word.to_owned(); }
        }
        if !cur.is_empty() { lines.push(cur); }
        lines
    }

    /// Draw the open panel. Returns nothing; missing optional art is recorded by `UiAssets::missing`.
    pub fn build(&mut self, ctx: &mut UiContext, w: f32, h: f32, out: &mut RenderList) {
        if !self.open { return; }
        self.size = (w, h);
        let s = dpi_scale(w, h);
        let (spx, xspx, tpx) = (atlas_px(BASE_S, s), atlas_px(BASE_XS, s), atlas_px(TITLE_SIZE, s));
        self.m = Metrics { s_line: line_height(ctx, FontId::Regular, spx), xs_line: line_height(ctx, FontId::Regular, xspx), title_line: line_height(ctx, FontId::Title, tpx) };
        let sel = self.selected.and_then(|id| self.item(id)).cloned();
        let desc_lines = sel.as_ref().map(|it| Self::wrap(ctx, FontId::Regular, xspx, &it.description, (DETAILS_W - 32.) * s)).unwrap_or_default();
        let desc_h = if desc_lines.is_empty() { 0. } else { desc_lines.len() as f32 * self.m.xs_line + 16. * s };
        let head_h = (8. + 32.) * s + (self.m.title_line - 4. * s) + 8. * s;
        let l = layout(w, h, self.m.s_line, if sel.is_some() { head_h + desc_h } else { 0. });
        let hero = mat_tex(ctx, "T-Card-Hero-Anim-Triangle.png", true).id;

        // CommonBorder_2: Image brush VerticalRibbonGradient-Inventory over the whole screen
        let bg = art(ctx, "VerticalRibbonGradient-Inventory.png"); out.sprite(bg.id, Rect::new(0., 0., w, h), WHITE);

        // header: PageTitle (the OfferHeader materials are not drawn)
        let tbase = centred_baseline(ctx, FontId::Title, tpx, l.title.bottom() - self.m.title_line, self.m.title_line); // VAlign_Bottom in the 48 unit row
        put_text(ctx, out, FontId::Title, tpx, l.panel_left + TITLE_PAD_L * s, tbase, PAGE_TITLE, [1., 1., 1., TITLE_ALPHA], None, Snap::PerGlyph);

        // equip slots
        Self::border(ctx, out, l.equip_border, s, false);
        let dragged = self.drag.and_then(|d| self.item(d.item)).cloned();
        for i in 0..5 {
            let r = l.equip[i];
            if let Some(it) = self.equip_item(i as u8 + 1) {
                let c = Rect::new(r.x + EQUIP_CARD_PAD * s, r.y + EQUIP_CARD_PAD * s, r.w - 2. * EQUIP_CARD_PAD * s, r.h - 2. * EQUIP_CARD_PAD * s);
                Self::card(ctx, out, c, it, hero);
                if it.count > 1 { Self::count_text(ctx, out, c, it.count, s); }
            }
            if dragged.as_ref().map_or(false, |d| fn_drop_target_visible(d.item_type)) {
                let hl = art(ctx, "Icon-QuickbarSlot-Highlight.png");
                out.nine_slice(hl, r, [0.14; 4], s, WHITE);
            }
        }

        // backpack header: "Backpack " + {Numerator} + "/{Denominator}", then the white line (Image_2)
        let row = l.backpack_row;
        let base = centred_baseline(ctx, FontId::Regular, spx, row.y, row.h);
        let label = [srgb3([LABEL_COLOR[0], LABEL_COLOR[1], LABEL_COLOR[2]])[0], srgb3([LABEL_COLOR[0], LABEL_COLOR[1], LABEL_COLOR[2]])[1], srgb3([LABEL_COLOR[0], LABEL_COLOR[1], LABEL_COLOR[2]])[2], LABEL_COLOR[3]];
        let cc = srgb3(COUNT_COLOR); let count_col = [cc[0], cc[1], cc[2], 1.];
        let (num, den) = (self.view.backpack.0.to_string(), format!("/{}", self.view.backpack.1));
        let mut x = row.x;
        put_text(ctx, out, FontId::Regular, spx, x, base, "Backpack ", label, None, Snap::PerGlyph); x += text_width(ctx, FontId::Regular, "Backpack ", spx) + 12. * s;
        put_text(ctx, out, FontId::Bold, spx, x, base, &num, count_col, None, Snap::PerGlyph); x += text_width(ctx, FontId::Bold, &num, spx);
        put_text(ctx, out, FontId::Regular, spx, x, base, &den, count_col, None, Snap::PerGlyph);
        out.fill(l.backpack_line, WHITE);

        // tiles (clipped to the TileView area)
        let n = self.view.items.len();
        self.scroll = self.scroll.clamp(0., max_scroll(l.tiles, s, n));
        out.push_clip(l.tiles);
        for (r, it) in tile_rects(l.tiles, s, n, self.scroll).into_iter().zip(self.view.items.iter()) {
            if r.bottom() < l.tiles.y || r.y > l.tiles.bottom() { continue; }
            let c = r.deflate(TILE_BUTTON_PAD * s, TILE_BUTTON_PAD * s, TILE_BUTTON_PAD * s, TILE_BUTTON_PAD * s);
            Self::card(ctx, out, c, it, hero);
            if it.count > 1 { Self::count_text(ctx, out, c, it.count, s); }
            if let Some(name) = it.slot.and_then(|sl| fn_tile_overlay(sl as i32)) {
                // EquipSlotBorder (black 0.33 over the card) + EquipSlotOverlay (quickbar_s<N>, 128 x 32, centred at the top)
                out.fill(c, EQUIP_TILE_BORDER);
                let t = art(ctx, &format!("{name}.png"));
                let (tw, th) = (QUICKBAR_OVERLAY.0 * s, QUICKBAR_OVERLAY.1 * s);
                out.sprite(t.id, Rect::new(c.x + (c.w - tw) * 0.5, c.y, tw, th), WHITE);
            }
        }
        out.pop_clip();

        // drop slot: the overlay is only there while an item is dragged
        if self.drag.is_some() {
            let t = art(ctx, "T_AthenaDropItem.png");
            out.sprite(t.id, l.drop.deflate(DROP_IMAGE_PAD * s, DROP_IMAGE_PAD * s, DROP_IMAGE_PAD * s, DROP_IMAGE_PAD * s), WHITE);
        }

        // item details of the selected item
        if let Some(it) = sel {
            let d = l.details;
            let head = Rect::new(d.x, d.y, d.w, head_h);
            let body = Rect::new(d.x, d.y + head_h, d.w, (d.h - head_h).max(0.));
            if body.h > 0. { Self::border(ctx, out, body, s, true); }
            // RarityBorder content: padding 8 / 16, SizeBox_3 (32) with the rarity text, then the item name
            let pad = Rect::new(head.x + 16. * s, head.y + 8. * s, head.w - 32. * s, head.h - 16. * s);
            let rbase = centred_baseline(ctx, FontId::Bold, xspx, pad.y, 32. * s);
            put_text(ctx, out, FontId::Bold, xspx, pad.x, rbase, RARITY_NAMES[it.rarity.min(4) as usize], WHITE, None, Snap::PerGlyph);
            let nbase = centred_baseline(ctx, FontId::Title, tpx, pad.y + 32. * s - 2. * s, self.m.title_line);
            put_text(ctx, out, FontId::Title, tpx, pad.x, nbase, it.name.trim(), WHITE, None, Snap::PerGlyph);
            // ItemDescriptionDetailWidget: TextStyle-Base-XS-I-40pc, padding 16 / 8
            for (k, line) in desc_lines.iter().enumerate() {
                let by = body.y + 8. * s + k as f32 * self.m.xs_line;
                let b = centred_baseline(ctx, FontId::Regular, xspx, by, self.m.xs_line);
                put_text(ctx, out, FontId::Regular, xspx, body.x + 16. * s, b, line, [1., 1., 1., 0.4], None, Snap::PerGlyph);
            }
        }

        // the drag visual (AthenaInventoryDragVisual: the item card at the cursor)
        if let (Some(_), Some(it)) = (self.drag, dragged) {
            let c = Rect::new(self.cursor.0 - CARD_M * s * 0.5, self.cursor.1 - CARD_M * s * 0.5, CARD_M * s, CARD_M * s);
            Self::card(ctx, out, c, &it, hero);
        }
    }

    /// Number of draw commands that are quads (test helper).
    pub fn quads(out: &RenderList) -> usize { out.cmds.iter().filter(|c| matches!(c, Cmd::Quad(_))).count() }
}

/// `AthenaInventoryEquipSlot.TryAndShowDropTarget` (item types 16 / 17).
fn fn_drop_target_visible(item_type: u8) -> bool { item_type == 16 || item_type == 17 }
/// `AthenaInventoryFortItemTileButton.SetEquipSlotOverlay`.
fn fn_tile_overlay(slot: i32) -> Option<&'static str> { match slot { 1 => Some("quickbar_s1"), 2 => Some("quickbar_s2"), 3 => Some("quickbar_s3"), 4 => Some("quickbar_s4"), 5 => Some("quickbar_s5"), _ => None } }

#[cfg(test)]
mod tests {
    use super::*;
    fn it(id: u32, ty: u8, slot: Option<u8>, count: u32) -> InvItem { InvItem { id, name: format!("Item {id}"), description: "A description that is long enough to wrap onto more than one line of the details panel body text.".into(), rarity: (id % 5) as u8, icon: String::new(), count, item_type: ty, equipable: ty != 19, is_resource: ty == 19, can_be_dropped: true, slot } }
    fn view() -> InventoryView { InventoryView { items: vec![it(1, 18, None, 1), it(2, 17, Some(1), 1), it(3, 17, Some(2), 1), it(4, 17, Some(3), 1), it(5, 19, None, 120)], backpack: (4, 5) } }
    fn panel() -> InventoryPanel { let mut p = InventoryPanel::new(); p.set_open(true); p.set_viewport(1920., 1080.); p.set_view(view()); p }

    #[test] fn decoded_constants() {
        assert_eq!((HEADER_H, COLUMN_MIN_W, DETAILS_W, EQUIP_BOX, DROP_BOX, DROP_BAND_MIN_H), (48., 1100., 700., 130., 118., 175.));
        assert_eq!(CARD_M, 124.); assert_eq!(TILE, 132.);
        assert_eq!(RARITY_NAMES, ["Common", "Uncommon", "Rare", "Epic", "Legendary"]);
    }
    #[test] fn layout_is_centred_and_stacked_like_the_widget_tree() {
        let l = layout(1920., 1080., 25., 0.);
        // panel = 1100 + 12 + 700 units, centred in 1920
        assert_eq!(l.scale, 1.); assert!((l.panel_left - 54.).abs() < 1e-3, "{}", l.panel_left);
        // equip row: 5 x (130 + 8) = 690 wide, centred in the 1100 column, 16 below the 48 header
        assert!((l.equip_border.w - 690.).abs() < 1e-3 && (l.equip_border.y - 64.).abs() < 1e-3 && (l.equip_border.h - 130.).abs() < 1e-3);
        assert!((l.equip_border.x + l.equip_border.w * 0.5 - (54. + 550.)).abs() < 1e-3);
        for i in 0..5 { assert!((l.equip[i].w - 130.).abs() < 1e-3 && (l.equip[i].x - (l.equip_border.x + i as f32 * 138. + 4.)).abs() < 1e-3); }
        // backpack header 40 below the equip row, the white line 4 under the text, tiles 8 under the line
        assert!((l.backpack_row.y - (64. + 130. + 40.)).abs() < 1e-3);
        assert!((l.backpack_line.y - (l.backpack_row.y + 25. + 4.)).abs() < 1e-3 && l.backpack_line.h == 3.);
        assert!((l.tiles.y - (l.backpack_line.bottom() + 8.)).abs() < 1e-3);
        // drop band: 175 high at the bottom, the 118 box centred in it
        assert!((l.drop.w - 118.).abs() < 1e-3 && (l.drop.y + 59. - (1080. - 87.5)).abs() < 1e-3);
        assert!(l.tiles.bottom() <= 1080. - 175. - 8. + 1e-3);
        // details: right of the column + 12, 700 wide
        let d = layout(1920., 1080., 25., 300.).details; assert!((d.x - (54. + 1100. + 12.)).abs() < 1e-3 && (d.w - 700.).abs() < 1e-3 && d.h == 300.);
    }
    #[test] fn layout_scales_and_shrinks_on_small_windows() {
        let l = layout(1280., 720., 17., 0.); let s = dpi_scale(1280., 720.);
        assert!((l.equip[0].w - 130. * s).abs() < 1e-2);
        let panel_w = (1812f32).min(1280. / s); assert!((l.panel_left - (1280. / s - panel_w) * 0.5 * s).abs() < 1e-2);
        assert!(l.tiles.w > 0. && l.tiles.h >= 0.);
    }
    #[test] fn tiles_fill_rows_and_scroll() {
        let area = Rect::new(0., 0., 400., 300.); let r = tile_rects(area, 1., 7, 0.);
        assert_eq!(r.len(), 7); let cols = (400. / TILE).floor() as usize; assert_eq!(cols, 3);
        assert_eq!(r[3], Rect::new(0., TILE, TILE, TILE)); assert_eq!(r[2].x, 2. * TILE);
        assert_eq!(tile_rects(area, 1., 7, 50.)[0].y, -50.);
        assert_eq!(max_scroll(area, 1., 7), 3. * TILE - 300.); assert_eq!(max_scroll(area, 1., 2), 0.);
        assert!(tile_rects(Rect::new(0., 0., 100., 300.), 1., 3, 0.).is_empty(), "narrower than one tile: nothing");
    }
    #[test] fn click_selects_and_drag_to_an_equip_slot_asks_for_drop_item_on_quickbar() {
        let mut p = panel(); let l = layout(1920., 1080., p.m.s_line, 0.); let t = tile_rects(l.tiles, 1., 5, 0.);
        let c = t[0].center();
        p.mouse_down(c.0, c.1, false); p.mouse_up(c.0, c.1, false);
        assert_eq!(p.selected(), Some(1)); assert_eq!(p.take_actions(), vec![PanelAction::Select(1)]);
        // drag the first tile onto equip slot 4 (index 3)
        p.mouse_down(c.0, c.1, false); p.mouse_move(c.0 + 3., c.1); assert!(p.dragging().is_none(), "below the 5 unit trigger distance");
        p.mouse_move(c.0 + 40., c.1 + 40.); assert_eq!(p.dragging(), Some(1));
        let e = layout(1920., 1080., p.m.s_line, 0.).equip[3].center(); p.mouse_move(e.0, e.1); p.mouse_up(e.0, e.1, false);
        assert_eq!(p.take_actions(), vec![PanelAction::DropOnSlot { item: 1, slot: 4 }]); assert!(p.dragging().is_none());
    }
    #[test] fn dragging_a_slotted_item_out_onto_the_drop_slot_asks_for_drop_item_with_the_split_flag() {
        let mut p = panel(); let l = layout(1920., 1080., p.m.s_line, 0.);
        let e = l.equip[0].center(); p.mouse_down(e.0, e.1, true); // shift at the drag detection = SplitTheStack
        p.mouse_move(e.0 + 30., e.1 + 30.); assert_eq!(p.dragging(), Some(2));
        let d = l.drop.center(); p.mouse_up(d.0, d.1, false);
        assert_eq!(p.take_actions(), vec![PanelAction::Drop { item: 2, split: true }]);
        // a drop on nothing does nothing; releasing over the drop slot with shift also splits
        p.mouse_down(e.0, e.1, false); p.mouse_move(e.0 + 30., e.1); p.mouse_up(5., 5., false); assert!(p.take_actions().is_empty());
        p.mouse_down(e.0, e.1, false); p.mouse_move(e.0 + 30., e.1); p.mouse_up(d.0, d.1, true); assert_eq!(p.take_actions(), vec![PanelAction::Drop { item: 2, split: true }]);
    }
    #[test] fn the_equip_action_needs_a_selected_equipable_non_resource_item() {
        let mut p = panel(); p.key_equip(); assert!(p.take_actions().is_empty(), "nothing selected");
        p.select(5); p.key_equip(); assert!(p.take_actions().is_empty(), "a resource has no Equip action");
        p.select(2); p.key_equip(); assert_eq!(p.take_actions(), vec![PanelAction::Equip(2)]);
        p.key_close(); assert_eq!(p.take_actions(), vec![PanelAction::Close]);
    }
    #[test] fn a_vanished_item_clears_selection_and_drag() {
        let mut p = panel(); p.select(2); p.set_view(InventoryView { items: vec![it(1, 18, None, 1)], backpack: (1, 5) }); assert_eq!(p.selected(), None);
        p.set_open(false); assert!(p.dragging().is_none() && !p.open);
    }
    #[test] fn wheel_scrolls_only_over_the_tiles() {
        let mut p = panel(); p.set_view(InventoryView { items: (0..60).map(|i| it(i, 17, None, 1)).collect(), backpack: (60, 60) });
        let l = layout(1920., 1080., p.m.s_line, 0.); p.mouse_move(5., 5.); p.wheel(-1.); assert_eq!(p.scroll(), 0.);
        let c = l.tiles.center(); p.mouse_move(c.0, c.1); p.wheel(-1.); assert_eq!(p.scroll(), TILE); p.wheel(2.); assert_eq!(p.scroll(), 0.);
    }
    #[test] fn build_draws_cards_title_and_details() {
        use crate::screens::testutil::ctx;
        let Some(mut c) = ctx() else { return };
        let mut p = panel(); p.select(2); let mut out = RenderList::new(); p.build(&mut c, 1920., 1080., &mut out);
        let cards = out.cmds.iter().filter(|x| matches!(x, Cmd::Material(m) if matches!(m.kind, MaterialKind::ItemCard { .. }))).count();
        assert_eq!(cards, 3 + 5 + 0, "3 slotted equip cards + 5 tiles"); assert!(InventoryPanel::quads(&out) > 40, "texts and borders");
        let mut closed = RenderList::new(); p.set_open(false); p.build(&mut c, 1920., 1080., &mut closed); assert!(closed.is_empty());
        let mut a = RenderList::new(); p.set_open(true); p.build(&mut c, 1920., 1080., &mut a); let mut b = RenderList::new(); p.build(&mut c, 1920., 1080., &mut b); assert_eq!(a.cmds, b.cmds, "deterministic");
    }
}

