//! Primary quickbar (AthenaQuickbarPrimary / AthenaQuickbarSlot) laid out from the 1.8 widget data (re-read on 1.8).
//! 1.8 UPDATE: the item card is the cooked `M-Item-Inst-S` material (bIsItem=1 instance of M-Item-Card-Inst-Master) drawn by the translated pixel
//! shader (`material::hud::item_card`, rarity parameters "Rarity 1..4" = RarityData Color1..4, the item icon is the material's "Icon" texture parameter);
//! the keycap is KeybindWidget Brush Size Small (24x24, Keybind-Frame-S, NotoSans-Bold 12, no shadow); the Selected animation is a cubic RichCurve.
//! The notes below that mention a guessed gradient / 32px keycap / TextStyle shadow are SUPERSEDED by this.
//!
//! RECOVERED (reports/quickbar-evidence.json, tools/quickbar-evidence.cjs, from the original cooked packages):
//!  - anchor chain: AthenaHUD Persistent HUD overlay slot is HAlign_Right/VAlign_Bottom with Padding Right 20 / Bottom 10,
//!    the quickbar HorizontalBox slot adds Padding.Bottom 8. The primary bar is therefore BOTTOM-RIGHT (not bottom-centre).
//!  - AthenaQuickbarPrimary: slot 1 | gap 8 | slots 2..6 contiguous | trailing 8 (HorizontalBoxSlot Padding.Right 8 on both overlays).
//!  - AthenaQuickBarSlot: 80x80 item box (design-time SizeBox override; at runtime UpdateItemBoxSize makes it 92 maximised / 64 minimised, see `layout_box`); "Empty" frame (Quickbar-Frame-Default, box 0.25) inset 6 at 20% alpha; item card inset 2;
//!    "Selection" (Icon-BasicFrame-S, box 0.5) fills the slot; keybind widget box 45x45 BELOW the item (VerticalBox order top/item/bottom);
//!    keycap border (Keybind-Frame-L) brush alpha 0.6; "Selected" animation lifts the slot 7 units over 0.1 s.
//!  - Keybind text style TextStyle-Base-XS(-B-S): size 16, shadow offset 1.6 / colour (0,0,0,0.75).
//!  - Rarity colours: RarityData FortRarityItemData[N].Color1..5 (linear).
//! DPI: `layout::dpi_scale` (UIScaleRule Vertical, curve (540,0.5)(1080,1)(2160,2), CookedIniVersion.txt L531-532).
//! APPROXIMATE (not recovered): keycap size (square 32), which RarityData stop paints which
//! part of the item card (FortMultiSizeItemCard is native C++), selection tint (white), animation easing (linear), font face (Segoe UI Bold,
//! original is NotoSans-Bold which is not exported), item-count position (bottom-right of card), whether the top keybind row is hidden.
//!
//! PORT NOTES: the old per-label GL text textures are replaced by the shared glyph atlas; the label box geometry (`ceil` advance + 2, `ceil` ascent/descent + 2,
//! glyphs offset by 1) is reproduced and snapped to whole pixels. The old non-material gradient fallback (used only when the card bake failed) is gone: the
//! card is always a `MaterialKind::ItemCard` command. NotoSans-Bold is now the real face (the old runtime used Segoe UI Bold unless NotoSans was found).
//!
//! QUICKBAR AMMO (2026-10-06): the card's stack-count row, replacing the provisional bottom-right count. The 1.8 card is native Slate (new cards unless
//! `-UseOldItemCards`), so the layout comes from the decompile plus `DefaultUIDataConfiguration`:
//!  - S card `SFortItemCard_S_ItemInstance` (builder FUN_142fc5f50, FortItemCard_S.cpp) and XS card (FUN_142fd56c0, FortItemCard_XS.cpp):
//!    `SFortItemCard_BackgroundWithPortrait` (content padding `BackgroundPadding`) holds an SOverlay. Its slot HAlign_Right / VAlign_Bottom holds the
//!    row SHorizontalBox: [`SFortItemCard_StackCountBlock` (AutoWidth, VAlign_Center) | `SFortItemCard_AmmoIconImage` (AutoWidth, HAlign_Center,
//!    VAlign_Center, size `FirstTraitSize`)], padding `TraitBoxPadding`. On the S card the row sits in an SVerticalBox (AutoHeight, HAlign_Right) above
//!    the tier meter, which the quickbar suppresses (`ShouldSuppressTierMeter` -> Collapsed, FUN_142fa3150).
//!  - Config values: `ItemCardConfig_S_ItemInstance` (UFortUIDataConfiguration+0x26C8) and `ItemCardConfig_XS_ItemInstance` (+0x28B8).
//!    S: BackgroundPadding 3, FirstTraitSize 18. XS: FirstTraitSize 16. Members absent from the asset keep the struct ctor's zeros
//!    (FUN_142ec9e70 / FUN_142eca4d0); UFortUIDataConfiguration's ctor sets nothing else there.
//!    `StackCountBlock.TextStyle` = TextStyle-Base-XS-B-S: NotoSans-Bold 16, drop shadow (1.6, 1.6) black 0.75, white from TextStyle-Base.
//!  - Number (StackCountBlock refresh FUN_142fe9d10): a firearm whose ammo item differs from itself shows `GetRemainingAmmo` (UFortWorldItem vfunc_98
//!    @1406b7d10 = loaded + the ammo stack in the inventory), even 0. Any other item shows its stack count when its definition can stack
//!    (`MaxStackSize != 1`, @14068ec40), even 1, except types in `ItemTypesToHideCountWhenCountEqualsOne` (= Badge only). Otherwise the text is Hidden.
//!    The text has no grouping separators (`FNumberFormattingOptions::DefaultNoGrouping`).
//!  - Icon (AmmoIconImage refresh FUN_142fe5800): the ranged definition's tertiary category = the ammo item's `AmmoIconBrush` (vfunc_94 @14092b210). Its
//!    sub-brush is picked by FUN_140646e50 for `FirstTraitSize` and drawn at that size. No ammo item -> Collapsed.
//!  - Which card shows (QuickbarSlot ubergraph): AnimMaximize OnAnimationStarted -> Resize(S); AnimMinimize OnAnimationFinished -> Resize(XS).
//!  - Font size: Slate `Size` is points at 96 DPI, so the em is `Size * 96 / 72` px (see `athena_shop::layout::text_px`).
//!  - The text block's size includes the shadow offset (UE4.16 `FSlateTextRun::Measure`).
//!  NOT PORTED: the `StackCountInfinite` text (infinite-ammo cheat) and the XS card material (the S material still draws while minimised).
use crate::draw::{MaterialKind, RenderList, TextureId};
use crate::layout::{dpi_scale, Rect};
use crate::material::hud::cubic_ease;
use crate::scene::UiContext;
use crate::screens::{art, mat_tex, put_text, Snap, WHITE};
use crate::text::FontId;

pub const SLOT: f32 = 80.;
pub const EMPTY_PAD: f32 = 6.;
pub const CARD_PAD: f32 = 2.;
pub const KEYBIND_BOX: f32 = 45.;
/// Keycap = KeybindWidget TextSizeMinBox MinDesiredWidth/Height for Brush Size Small (KeybindWidget_C.UpdateFrameSize bytecode: Small -> 24; the slot's
/// KeybindBottom has no Brush Size override). Frame brush = Brush_S = Keybind-Frame-S 24x24, margin 0.25.
pub const KEYCAP: f32 = 24.;
pub const GROUP_GAP: f32 = 8.;
pub const HUD_PAD_RIGHT: f32 = 20.;
pub const HUD_PAD_BOTTOM: f32 = 10.;
pub const PRIMARY_PAD_BOTTOM: f32 = 8.;
pub const SELECTED_LIFT: f32 = 7.;
pub const SELECTED_TIME: f32 = 0.1;
/// KeybindWidget sets the key text font in UpdateFrameSize: NotoSans-Bold, Size = Round(base(24 for Small) * 1.5 * 96 / 300) = 12.
pub const KEY_FONT: f32 = 12.;
/// The key TextBlock has Style=null: no drop shadow (the earlier TextStyle-Base-XS-B-S shadow was wrong).
pub const TEXT_SHADOW: (f32, f32) = (0., 0.);
/// Total widget row width in Slate units: 6 slots + two 8-unit group paddings.
pub const ROW_WIDTH: f32 = 6. * SLOT + 2. * GROUP_GAP;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotLayout { pub item: Rect, pub empty: Rect, pub card: Rect, pub keybind: Rect, pub keycap: Rect }

fn inset(r: Rect, d: f32) -> Rect { Rect::new(r.x + d, r.y + d, r.w - 2. * d, r.h - 2. * d) }

/// Pixel rectangles (selection animation NOT applied) for the six slots at a client size, with the bar maximised (the combat bar focused).
pub fn layout(w: f32, h: f32) -> [SlotLayout; 6] { layout_box(w, h, crate::screens::build_quickbar::BOX_MAX) }

/// HUD FIX (weapon bar minimise): [`layout`] for an item box of `bx` Slate units. The design-time `[SizeBox] Item` override ([`SLOT`] = 80) is replaced at runtime by
/// `QuickbarSlot.UpdateItemBoxSize` (cooked Kismet, `fn-inspect disasm /Game/UI/InGame/HUD/QuickbarSlot`): width / height = Lerp(ItemCardMinimized card +
/// OverlaySlot padding, ItemCardMaximized card + padding, AnimatedBoxScaleFactor), called from PreConstruct and `SetAnimatedBoxScaleFactor`. The primary
/// slots' cards are `BPItemCardSize` S (maximised) / XS (minimised) (`fn-inspect props /Game/Athena/HUD/AthenaQuickbarPrimary`), the same as the building bar,
/// and a weapon is neither cosmetic nor schematic / personnel (`UFortItemCardUtilities::GetCardDimensions` @142fa68a0 -> selector FUN_142fa93c0 ->
/// `OtherItemCardSizes`), so the box runs 64 .. 92 exactly like `build_quickbar::item_box`.
pub fn layout_box(w: f32, h: f32, bx: f32) -> [SlotLayout; 6] {
    let s = dpi_scale(w, h);
    let right = w - HUD_PAD_RIGHT * s;
    let bottom = h - (HUD_PAD_BOTTOM + PRIMARY_PAD_BOTTOM) * s;
    let left = right - (6. * bx + 2. * GROUP_GAP) * s;
    let item_y = bottom - (KEYBIND_BOX + bx) * s;
    let mut out = [SlotLayout { item: Rect::ZERO, empty: Rect::ZERO, card: Rect::ZERO, keybind: Rect::ZERO, keycap: Rect::ZERO }; 6];
    for (i, o) in out.iter_mut().enumerate() {
        let x = left + if i == 0 { 0. } else { GROUP_GAP + bx * i as f32 } * s;
        let item = Rect::new(x, item_y, bx * s, bx * s);
        let keybind = Rect::new(x + (bx - KEYBIND_BOX) * 0.5 * s, item_y + bx * s, KEYBIND_BOX * s, KEYBIND_BOX * s);
        let keycap = Rect::new(keybind.x + (KEYBIND_BOX - KEYCAP) * 0.5 * s, keybind.y + (KEYBIND_BOX - KEYCAP) * 0.5 * s, KEYCAP * s, KEYCAP * s);
        *o = SlotLayout { item, empty: inset(item, EMPTY_PAD * s), card: inset(item, CARD_PAD * s), keybind, keycap };
    }
    out
}

/// Selection animation weight 0..1 -> upward translation in pixels (Selected animation: Translation Y 0 -> -7 in 0.1 s).
pub fn lift_px(weight: f32, scale: f32) -> f32 { weight.clamp(0., 1.) * SELECTED_LIFT * scale }

/// RarityData (linear) -> display sRGB for Slate's gamma-space compositing.
pub fn lin_to_srgb(c: f32) -> f32 { crate::screens::srgb(c) }
/// RarityData Color1..Color5 (linear RGB) for rarity 0..4 (Common, Uncommon, Rare, Epic, Legendary).
pub const RARITY: [[[f32; 3]; 5]; 5] = [
    [[0.6939, 0.6939, 0.6939], [0.4735, 0.521, 0.5583], [0.1746, 0.1946, 0.2159], [0.0513, 0.0612, 0.0742], [0.0203, 0.0212, 0.0343]],
    [[0.3564, 0.9911, 0.], [0.1195, 0.521, 0.], [0., 0.2159, 0.0015], [0.0006, 0.0782, 0.0009], [0.0006, 0.0331, 0.0006]],
    [[0., 1., 0.9647], [0., 0.4287, 1.], [0., 0.0976, 0.521], [0., 0.0595, 0.2542], [0.0103, 0.0232, 0.0409]],
    [[0.8367, 0.02, 1.], [0.6172, 0.0999, 1.], [0.1823, 0.026, 0.651], [0.0723, 0.0097, 0.1981], [0.0262, 0.0144, 0.0409]],
    [[0.9647, 0.545, 0.1254], [1., 0.2575, 0.01], [0.521, 0.0781, 0.], [0.2542, 0.0449, 0.0127], [0.04, 0.0121, 0.009]],
];
pub fn rarity_srgb(rarity: usize, stop: usize) -> [f32; 3] { let c = RARITY[rarity.min(4)][stop.min(4)]; [lin_to_srgb(c[0]), lin_to_srgb(c[1]), lin_to_srgb(c[2])] }

/// Textures resolved lazily on the first `build` (all optional: a missing frame/selection/keycap is skipped, as in the old runtime).
#[derive(Clone, Copy)]
struct Tex { frame: crate::assets::TexInfo, selection: crate::assets::TexInfo, keycap: crate::assets::TexInfo, hero_tri: TextureId }

/// Icon texture file name of the slot-1 pickaxe (the only item the runtime had when the quickbar was written).
pub const PICKAXE_ICON: &str = "T-Icon-Weapons-SK-Pick.png";

/// One `FSlateBrush` of an `FFortMultiSizeBrush`: (ImageSize, texture file name); None = the sub-brush names no resource.
pub type SubBrush = Option<([f32; 2], String)>;
/// QUICKBAR AMMO: `FFortMultiSizeBrush` (Brush_XXS, _XS, _S, _M, _L, _XL), e.g. `UFortAmmoItemDefinition::AmmoIconBrush` (+0x4D8).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MultiSizeBrush(pub [SubBrush; 6]);
impl MultiSizeBrush {
    /// FUN_140646e50: the first sub-brush (XXS .. XL) with a resource whose ImageSize is larger than `want` on BOTH axes (strictly); when none is,
    /// the last sub-brush with a resource.
    pub fn pick(&self, want: [f32; 2]) -> Option<&str> {
        let mut last = None;
        for (size, name) in self.0.iter().flatten() {
            last = Some(name.as_str());
            if size[0] - want[0] > 0. && size[1] - want[1] > 0. { return last; }
        }
        last
    }
}

/// QUICKBAR AMMO: the stack-count row values of one card size (`ItemCardConfig_S_ItemInstance` / `_XS_ItemInstance`, see the module docs).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardRow { pub background_padding: f32, pub trait_box_padding: f32, pub first_trait_size: f32 }
/// The maximised slot's card (`ItemCardMaximized`, BPItemCardSize S).
pub const ROW_S: CardRow = CardRow { background_padding: 3., trait_box_padding: 0., first_trait_size: 18. };
/// The minimised slot's card (`ItemCardMinimized`, BPItemCardSize XS).
pub const ROW_XS: CardRow = CardRow { background_padding: 0., trait_box_padding: 0., first_trait_size: 16. };
/// `StackCountBlock.TextStyle` = TextStyle-Base-XS-B-S: NotoSans-Bold, Size 16 (points at 96 DPI).
pub const COUNT_FONT: f32 = 16.;
/// TextStyle-Base-XS-B-S drop shadow: offset (1.6, 1.6) Slate units, colour (0, 0, 0, 0.75).
pub const COUNT_SHADOW: (f32, [f32; 4]) = (1.6, [0., 0., 0., 0.75]);

#[derive(Clone)]
pub struct Quickbar {
    /// Rarity per slot (0 Common .. 4 Legendary). Slot 1 is the Common pickaxe (WID_Harvest_Pickaxe_Athena_C_T01).
    pub rarity: [usize; 6],
    /// QUICKBAR AMMO: the number of the card's `SFortItemCard_StackCountBlock` per slot (a firearm's loaded + inventory ammo, a stackable item's count);
    /// None = the text is Hidden (the pickaxe). The game side applies the 1.8 rules (module docs).
    pub count: [Option<u32>; 6],
    /// Exported icon texture file name per slot (the item card material's "Icon" parameter); empty = no icon. Slot 1 defaults to the pickaxe.
    pub icon: [String; 6],
    /// QUICKBAR AMMO: the `AmmoIconBrush` of the slot firearm's ammo item, shown by `SFortItemCard_AmmoIconImage`; None = Collapsed (no ammo item).
    pub ammo: [Option<MultiSizeBrush>; 6],
    /// WIDGETS3: `QuickbarSlotCooldown` state per slot (`screens::slot_cooldown`).
    pub cooldown: [Option<crate::screens::slot_cooldown::SlotCooldown>; 6],
    pub slot_keys: [String; 6],
    anim: [f32; 6],
    /// Slot focused at the last `tick` (detects the deselect that restarts the lift reverse from its end).
    focused: usize,
    tex: Option<Tex>,
    /// HUD FIX: FocusedQuickbar == Primary. `AthenaHUD.SetQuickbarSizes` (cooked Kismet): FocusedQuickbar 1 -> QuickbarPrimary.Minimize, 0 -> Maximize; and
    /// `QuickbarBase.OnQuickbarSlotFocusChanged` only selects a slot of the focused bar (the other bar deselects: FocusedSlot = -1, SetSelected(false)).
    pub primary_focused: bool,
    /// AnimatedBoxScaleFactor timeline position 0..1 (AnimMaximize / AnimMinimize of QuickbarSlot: 0.1 s, cubic, see `build_quickbar`). Starts maximised.
    resize_t: f32,
    /// `primary_focused` at the last `tick` (a change starts AnimMaximize / AnimMinimize from time 0: `build_quickbar::resize_step`).
    focus_last: bool,
}
pub fn default_slot_keys() -> [String; 6] { ["One", "Two", "Three", "Four", "Five", "Six"].map(String::from) }
impl Default for Quickbar {
    fn default() -> Self {
        let mut icon: [String; 6] = Default::default();
        icon[0] = PICKAXE_ICON.to_owned();
        Self { rarity: [0; 6], count: [None; 6], icon, ammo: Default::default(), cooldown: [None; 6], slot_keys: default_slot_keys(), anim: [1., 0., 0., 0., 0., 0.], focused: 0, tex: None, primary_focused: true, resize_t: 1., focus_last: true }
    }
}

impl Quickbar {
    pub fn new() -> Self { Self::default() }
    /// Current Selected-animation weight per slot (0..1, linear; the cubic ease is applied when drawing).
    pub fn anim(&self) -> [f32; 6] { self.anim }
    /// Advance the Selected animation (0.1 s linear) toward the selected slot.
    /// Deselect = `PlayAnimation(Selected, StartAtTime = GetEndTime, Reverse)`: the slot that just lost focus restarts its lift reverse from the END, not from
    /// where a cut-short select left it (QuickbarSlot.SetSelected; docs/spec/smallfix_followups.md section 2).
    pub fn tick(&mut self, selected: usize, dt: f32) {
        let selected = self.shown_selection(selected);
        if self.focused != selected { if let Some(a) = self.anim.get_mut(self.focused) { *a = 1.; } self.focused = selected; }
        for (i, a) in self.anim.iter_mut().enumerate() { let target = if i == selected { 1. } else { 0. }; let step = dt / SELECTED_TIME; *a = if *a < target { (*a + step).min(target) } else { (*a - step).max(target) }; }
        // HUD FIX2: Maximize / Minimize (QuickbarSlot AnimMaximize / AnimMinimize, 0.1 s): a change stops the running one and plays the other FROM TIME 0
        // (the value jumps to its first key, not a smooth reversal), see build_quickbar::resize_step
        let dt = if dt.is_finite() { dt.max(0.) } else { 0. };
        self.resize_t = crate::screens::build_quickbar::resize_step(self.resize_t, self.primary_focused, &mut self.focus_last, dt);
    }
    /// Test hook: jump the animations to their end state.
    pub fn snap(&mut self, selected: usize) {
        let selected = self.shown_selection(selected);
        self.focused = selected; for (i, a) in self.anim.iter_mut().enumerate() { *a = if i == selected { 1. } else { 0. }; }
        self.resize_t = if self.primary_focused { 1. } else { 0. };
        self.focus_last = self.primary_focused;
    }
    /// The slot the bar shows selected: none (6) while the building bar has the focus (OnQuickbarSlotFocusChanged deselects the unfocused bar).
    fn shown_selection(&self, selected: usize) -> usize { if self.primary_focused { selected } else { 6 } }
    /// AnimatedBoxScaleFactor as drawn (eased).
    pub fn scale_factor(&self) -> f32 { cubic_ease(self.resize_t) }
    /// The item box (Slate units) this frame: `QuickbarSlot.UpdateItemBoxSize` (see [`layout_box`]).
    pub fn item_box(&self) -> f32 { crate::screens::build_quickbar::item_box(self.scale_factor()) }
    /// QUICKBAR AMMO: the card size shown (`QuickbarSlot.Resize` -> `UpdateItemCardsVisibility`). AnimMaximize's OnAnimationStarted shows the S card; only
    /// AnimMinimize's OnAnimationFinished shows the XS card, so the S card stays up while the box shrinks.
    pub fn card_row(&self) -> CardRow { if !self.primary_focused && self.resize_t <= 0. { ROW_XS } else { ROW_S } }

    /// QUICKBAR AMMO: the card's stack-count row (module docs), bottom-right inside `card`: the count text, then the ammo icon.
    fn count_row(&self, ctx: &mut UiContext, out: &mut RenderList, card: Rect, s: f32, slot: usize, row: CardRow) {
        let icon = self.ammo[slot].as_ref().and_then(|b| b.pick([row.first_trait_size; 2])).map(|n| art(ctx, n)).filter(|t| t.id != TextureId::NONE);
        let label = self.count[slot].map(|n| n.to_string());
        if icon.is_none() && label.is_none() { return; }
        let px = crate::screens::atlas_px(COUNT_FONT * 96. / 72., s);
        let (_, a) = ctx.atlases.atlas(&mut ctx.assets.store, &ctx.fonts, FontId::Bold, px);
        let (asc, desc) = (a.ascent.ceil(), a.descent.abs().ceil());
        let shadow = COUNT_SHADOW.0 * s;
        // STextBlock desired size = shaped text + |ShadowOffset| * scale (FSlateTextRun::Measure); the height is the font's line height
        let text = label.as_deref().map(|l| (l, crate::screens::text_width(ctx, FontId::Bold, l, px) + shadow, asc + desc + shadow));
        let icon_px = row.first_trait_size * s;
        let row_h = text.map_or(0., |t| t.2).max(if icon.is_some() { icon_px } else { 0. });
        let right = card.x + card.w - (row.background_padding + row.trait_box_padding) * s;
        let bottom = card.y + card.h - (row.background_padding + row.trait_box_padding) * s;
        let top = bottom - row_h;
        let mut x = right;
        if let Some(t) = icon {
            x -= icon_px;
            out.quad(t.id, Rect::new(x, top + (row_h - icon_px) * 0.5, icon_px, icon_px), [0., 0., 1., 1.], WHITE);
        }
        if let Some((l, tw, th)) = text {
            let base = (top + (row_h - th) * 0.5 + asc).round();
            put_text(ctx, out, FontId::Bold, px, x - tw, base, l, WHITE, Some(([shadow, shadow], COUNT_SHADOW.1)), Snap::PerGlyph);
        }
    }

    fn ensure_tex(&mut self, ctx: &mut UiContext) -> Tex {
        if let Some(t) = self.tex { return t; }
        let t = Tex { frame: art(ctx, "Quickbar-Frame-Default.png"), selection: art(ctx, "Icon-BasicFrame-S.png"), keycap: art(ctx, "Keybind-Frame-S.png"), hero_tri: mat_tex(ctx, "T-Card-Hero-Anim-Triangle.png", true).id };
        self.tex = Some(t); t
    }

    /// Text centred on (cx, cy) (or bottom-right anchored at (cx, cy) when `align_right`), reproducing the old label-texture geometry.
    #[allow(clippy::too_many_arguments)]
    fn text_centered(&self, ctx: &mut UiContext, out: &mut RenderList, label: &str, cx: f32, cy: f32, scale: f32, align_right: bool) {
        let px = crate::screens::atlas_px(KEY_FONT, scale);
        let font = ctx.fonts.font(FontId::Bold);
        let Some(lm) = font.horizontal_line_metrics(px as f32) else { return };
        let (asc, desc) = (lm.ascent.ceil(), (-lm.descent).ceil());
        let tw = ctx.fonts.measure(FontId::Bold, label, px as f32).ceil().max(1.) + 2.;
        let th = asc + desc + 2.;
        let (x, y) = if align_right { (cx - tw, cy - th) } else { (cx - tw * 0.5, cy - th * 0.5) };
        let (x, y) = (x.round(), y.round());
        let shadow = if TEXT_SHADOW.0 != 0. || TEXT_SHADOW.1 != 0. { Some(([TEXT_SHADOW.0 * scale, TEXT_SHADOW.1 * scale], [0., 0., 0., 0.75])) } else { None };
        put_text(ctx, out, FontId::Bold, px, x + 1., y + 1. + asc, label, WHITE, shadow, Snap::PerGlyph);
    }

    /// Append the quickbar. `occupied[i]` slots draw the item-card material, others the faint "Empty" frame.
    pub fn build(&mut self, ctx: &mut UiContext, w: f32, h: f32, selected: usize, occupied: [bool; 6], out: &mut RenderList) {
        let tex = self.ensure_tex(ctx);
        let icons: [TextureId; 6] = std::array::from_fn(|i| if self.icon[i].is_empty() { TextureId::NONE } else { mat_tex(ctx, &self.icon[i], false).id });
        let s = dpi_scale(w, h);
        let lay = layout_box(w, h, self.item_box());
        let selected = self.shown_selection(selected);
        for i in 0..6 {
            // Selected animation: Translation Y 0 -> -7 over 0.1 s, RichCurve interp = Cubic with zero tangents (reports/quickbar-evidence.json) = smoothstep.
            let dy = -lift_px(cubic_ease(self.anim[i]), s);
            let mv = |r: Rect| Rect::new(r.x, r.y + dy, r.w, r.h);
            let l = lay[i];
            if occupied[i] {
                let c = mv(l.card);
                let r = RARITY[self.rarity[i].min(4)];
                out.material(MaterialKind::ItemCard { colors: [r[0], r[1], r[2], r[3]], icon: icons[i], hero_tri: tex.hero_tri }, c, WHITE, false);
                self.count_row(ctx, out, c, s, i, self.card_row());
            } else {
                // "Empty" image: Quickbar-Frame-Default, box margin 0.25, ColorAndOpacity alpha 0.2.
                out.nine_slice(tex.frame, mv(l.empty), [0.25; 4], s, [1., 1., 1., 0.2]);
            }
            // `Selection` image: Visible / Collapsed in the same frame the focus changes (FUN_140698b20 -> AFortQuickBars+0x590 broadcast -> OnQuickbarSlotFocusChanged
            // -> QuickbarSlot.SetSelected), full opacity; only the lift above animates (docs/spec/smallfix_followups.md section 2)
            if i == selected { out.nine_slice(tex.selection, mv(l.item), [0.5; 4], s, WHITE); }
            // QuickbarSlotCooldown (WIDGETS3): the Item overlay's child above the cards and the selection frame, filling the 80 x 80 item box
            if let Some(cd) = self.cooldown[i] { crate::screens::slot_cooldown::build(ctx, out, mv(l.item), s, i, &cd); }
            // Keybind widget: keycap border (alpha 0.6) + slot number.
            let k = mv(l.keycap);
            let key = self.slot_keys[i].clone();
            if let Some(icon) = crate::screens::target_info::key_icon(&key) { crate::screens::target_info::draw_key_icon(ctx, out, icon, k, s); }
            else if !key.is_empty() {
                let t = crate::screens::target_info::hud_key_text(&key);
                let tw = ctx.fonts.measure(FontId::Bold, &t, crate::screens::atlas_px(KEY_FONT, s) as f32);
                let kw = (tw + 2. * KEYCAP * 0.1 * s).max(k.w);
                out.nine_slice(tex.keycap, Rect::new(k.x + (k.w - kw) * 0.5, k.y, kw, k.h), [0.25; 4], s, [1., 1., 1., 0.6]);
                self.text_centered(ctx, out, &t, k.x + k.w * 0.5, k.y + k.h * 0.5, s, false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Cmd;
    use crate::screens::testutil::{ctx, evidence};
    fn num(j: &serde_json::Value, key: &str) -> f32 { j[key].as_f64().unwrap_or_else(|| panic!("missing {key}")) as f32 }
    #[test] fn constants_match_recovered_widget_data() {
        let j = evidence("hud-quickbar-evidence.json");
        // Same numbers as the 1.10 widgets for the slot geometry (re-read from the 1.8 trees); keycap size/font/shadow are NEW 1.8 findings (bytecode).
        assert_eq!(num(&j, "slot_size"), SLOT); assert_eq!(num(&j, "empty_pad"), EMPTY_PAD); assert_eq!(num(&j, "card_pad"), CARD_PAD);
        assert_eq!(num(&j, "keybind_box"), KEYBIND_BOX); assert_eq!(num(&j, "group_gap"), GROUP_GAP);
        assert_eq!(num(&j, "hud_pad_right"), HUD_PAD_RIGHT); assert_eq!(num(&j, "hud_pad_bottom"), HUD_PAD_BOTTOM); assert_eq!(num(&j, "primary_pad_bottom"), PRIMARY_PAD_BOTTOM);
        assert_eq!(num(&j, "selected_lift"), SELECTED_LIFT); assert!((num(&j, "selected_anim_time") - SELECTED_TIME).abs() < 1e-4);
        assert_eq!(num(&j, "keycap_size"), KEYCAP); assert_eq!(num(&j, "keybind_font_size"), KEY_FONT);
        assert_eq!(num(&j, "empty_alpha"), 0.2); assert_eq!(num(&j, "keycap_alpha"), 0.6);
        assert_eq!(j["selected_interp_mode"], 2, "Selected animation is a CUBIC RichCurve, tangents all zero -> smoothstep");
        assert!(j["selected_tangents"].as_array().unwrap().iter().all(|t| t.as_f64() == Some(0.)));
    }
    #[test] fn rarity_table_matches_rarity_data() {
        let j = evidence("hud-quickbar-evidence.json");
        for r in 0..5 { for c in 0..5 { let a = j[format!("rarity{r}_color{}", c + 1)].as_array().unwrap(); for k in 0..3 { assert!((a[k].as_f64().unwrap() as f32 - RARITY[r][c][k]).abs() < 1e-4, "rarity {r} color {c} comp {k}"); } assert_eq!(a[3].as_f64().unwrap(), 1.); } }
    }
    #[test] fn row_width_and_anchor_bottom_right() {
        let l = layout(1920., 1080.);
        // Right edge: 1920 - 20. Maximised row width 6 * 92 + 16 = 568 (UpdateItemBoxSize: S card 88 + 2 + 2). Slot 6 ends at right-8 (trailing group gap).
        assert_eq!(l[0].item.x, 1900. - 568.);
        assert_eq!(l[5].item.x + l[5].item.w, 1900. - 8.);
        // Bottom: 10 + 8 padding, keybind row 45 under the 92 item.
        assert_eq!(l[0].keybind.y + l[0].keybind.h, 1080. - 18.);
        assert_eq!(l[0].item.y, 1080. - 18. - 45. - 92.);
        // the design-time 80 box is what the evidence file records; the runtime box replaces it
        assert_eq!(layout_box(1920., 1080., SLOT)[0].item.x, 1900. - ROW_WIDTH);
    }
    /// HUD FIX: AthenaHUD.SetQuickbarSizes minimises the primary bar while the building bar is focused (64 box, 0.1 s cubic) and deselects its slot.
    #[test] fn primary_bar_minimises_in_build_mode() {
        let mut q = Quickbar::default();
        assert_eq!(q.item_box(), 92.);
        q.primary_focused = false; q.tick(0, 0.05);
        assert!((q.item_box() - 78.).abs() < 1e-4, "smoothstep(0.5) halfway: {}", q.item_box());
        q.tick(0, 0.05); assert_eq!(q.item_box(), 64.);
        assert_eq!(q.anim(), [0.; 6], "no slot of the unfocused bar is selected");
        let l = layout_box(1920., 1080., 64.);
        assert_eq!((l[0].item.w, l[0].item.y, l[5].item.x + l[5].item.w), (64., 1080. - 18. - 45. - 64., 1892.));
        q.primary_focused = true; q.tick(0, 1.); assert_eq!(q.item_box(), 92.); assert_eq!(q.anim()[0], 1.);
        let Some(mut c) = ctx() else { return };
        q.primary_focused = false; q.snap(0);
        let mut out = RenderList::new(); q.build(&mut c, 1920., 1080., 0, [true; 6], &mut out);
        let sel = q.tex.unwrap().selection.id;
        assert!(!out.cmds.iter().any(|c| matches!(c, Cmd::Quad(q) if q.tex == sel)), "no selection frame on the minimised bar");
    }
    #[test] fn group_gap_only_after_first_slot() {
        let l = layout(1920., 1080.);
        assert_eq!(l[1].item.x - (l[0].item.x + l[0].item.w), 8.);
        for i in 2..6 { assert_eq!(l[i].item.x - (l[i - 1].item.x + l[i - 1].item.w), 0.); }
    }
    #[test] fn paddings_and_keybind_geometry() {
        let l = layout(1920., 1080.)[2];
        assert_eq!(l.empty, Rect::new(l.item.x + 6., l.item.y + 6., 80., 80.));
        assert_eq!(l.card, Rect::new(l.item.x + 2., l.item.y + 2., 88., 88.));
        assert_eq!(l.keybind.w, 45.); assert_eq!(l.keybind.x + 22.5, l.item.x + 46.);
        assert_eq!(l.keybind.y, l.item.y + l.item.h);
    }
    #[test] fn layout_scales_with_dpi_and_stays_on_screen() {
        for (w, h) in [(1920., 1080.), (1280., 720.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.), (3840., 2160.)] {
            let l = layout(w, h); let s = dpi_scale(w, h);
            assert!((l[0].item.w - 92. * s).abs() < 1e-3);
            for sl in &l { for r in [sl.item, sl.keybind] { assert!(r.x >= 0. && r.y >= 0. && r.x + r.w <= w + 1e-3 && r.y + r.h <= h + 1e-3, "{w}x{h} off-screen {r:?}"); } }
            assert!((l[5].item.x + l[5].item.w - (w - 28. * s)).abs() < 1e-2);
        }
    }
    #[test] fn selection_lifts_seven_units_over_a_tenth_of_a_second() {
        assert_eq!(lift_px(0., 1.), 0.); assert_eq!(lift_px(1., 1.), 7.); assert_eq!(lift_px(1., 2.), 14.); assert_eq!(lift_px(2., 1.), 7.);
        let mut q = Quickbar::default();
        q.tick(2, 0.05); assert!((q.anim[0] - 0.5).abs() < 1e-5 && (q.anim[2] - 0.5).abs() < 1e-5);
        q.tick(2, 0.05); assert!(q.anim[0] == 0. && (q.anim[2] - 1.).abs() < 1e-5);
        q.tick(2, 1.); assert_eq!(q.anim[2], 1.);
        // a select cut short at 0.3, then deselected: the reverse lift starts from the end (1), not from 0.3
        q.tick(3, 0.03); assert!((q.anim[3] - 0.3).abs() < 1e-5);
        q.tick(4, 0.02); assert!((q.anim[3] - 0.8).abs() < 1e-5 && (q.anim[4] - 0.2).abs() < 1e-5);
    }
    /// The selection frame is on (alpha 1) for the selected slot only, from the very frame the focus changes; no fade.
    #[test] fn selection_frame_switches_in_the_same_frame() {
        let Some(mut c) = ctx() else { return };
        let mut q = Quickbar::default();
        q.tick(2, 0.); // the frame of the switch: the lift has not moved yet
        let mut out = RenderList::new(); q.build(&mut c, 1920., 1080., 2, [false; 6], &mut out);
        let sel = q.tex.unwrap().selection.id;
        let frames: Vec<_> = out.cmds.iter().filter_map(|c| match c { Cmd::Quad(q) if q.tex == sel => Some(q.tint[3]), _ => None }).collect();
        assert!(!frames.is_empty() && frames.iter().all(|a| *a == 1.), "{frames:?}");
        assert_eq!(frames.len(), 9, "one nine-slice frame (slot 3), none fading on slot 1");
    }
    #[test] fn srgb_conversion_known_points() { assert_eq!(lin_to_srgb(0.), 0.); assert!((lin_to_srgb(1.) - 1.).abs() < 1e-6); assert!((lin_to_srgb(0.2140) - 0.5).abs() < 1e-3); }
    #[test] fn common_pickaxe_card_is_gray() { let c = rarity_srgb(0, 1); assert!(c[0] > 0.6 && c[0] < 0.85 && (c[2] - c[0]).abs() < 0.1); }
    #[test] fn original_textures_have_brush_sizes() {
        // Brush ImageSize values recovered from the widgets equal the exported texture sizes (so nine-slice borders map 1:1).
        let Some(mut c) = ctx() else { return };
        for (f, w) in [("Quickbar-Frame-Default.png", 80), ("Icon-BasicFrame-S.png", 24), ("Keybind-Frame-S.png", 24)] {
            let t = c.assets.texture(f); assert_eq!((t.w, t.h), (w, w), "{f}");
        }
    }
    /// QUICKBAR AMMO: FUN_140646e50 takes the first sub-brush larger than the wanted size on both axes, else the last one with a resource.
    #[test] fn multi_size_brush_pick_follows_the_native_rule() {
        let b = |s: f32, n: &str| Some(([s, s], n.to_owned()));
        // AthenaAmmoDataBulletsMedium AmmoIconBrush: XS 16, S 24, M (ctor default 32), L 40; no XXS / XL
        let ammo = MultiSizeBrush([None, b(16., "xs"), b(24., "s"), b(32., "m"), b(40., "l"), None]);
        assert_eq!(ammo.pick([ROW_S.first_trait_size; 2]), Some("s"), "S card, FirstTraitSize 18");
        assert_eq!(ammo.pick([ROW_XS.first_trait_size; 2]), Some("s"), "XS card, FirstTraitSize 16: Brush_XS (16) is not strictly larger");
        assert_eq!(ammo.pick([15.; 2]), Some("xs"));
        assert_eq!(ammo.pick([64.; 2]), Some("l"), "none larger: the last sub-brush with a resource");
        assert_eq!(MultiSizeBrush::default().pick([18.; 2]), None);
        let wide = MultiSizeBrush([None, Some(([40., 10.], "wide".to_owned())), b(24., "s"), None, None, None]);
        assert_eq!(wide.pick([18.; 2]), Some("s"), "both axes must be larger");
    }
    /// QUICKBAR AMMO: QuickbarSlot shows the S card from AnimMaximize's start and the XS card only when AnimMinimize has finished.
    #[test] fn card_row_is_s_until_the_minimise_finishes() {
        let mut q = Quickbar::default();
        assert_eq!(q.card_row(), ROW_S);
        q.primary_focused = false; q.tick(0, 0.05); assert_eq!(q.card_row(), ROW_S, "AnimMinimize running: still the S card");
        q.tick(0, 0.05); assert_eq!(q.card_row(), ROW_XS, "AnimMinimize finished: the XS card");
        q.primary_focused = true; q.tick(0, 0.); assert_eq!(q.card_row(), ROW_S, "AnimMaximize started: the S card at once");
    }
    /// QUICKBAR AMMO: the number and the icon sit in the card's bottom-right corner inside BackgroundPadding 3: [number][18 x 18 icon], centred on one row.
    #[test] fn count_row_sits_bottom_right_with_the_icon_after_the_number() {
        let Some(mut c) = ctx() else { return };
        let icon_tex = "Quickbar-Frame-Default.png";
        if c.assets.texture(icon_tex).id == TextureId::NONE { return; }
        let mut q = Quickbar::default(); q.snap(0);
        q.count[1] = Some(60); q.ammo[1] = Some(MultiSizeBrush([None, None, Some(([24., 24.], icon_tex.to_owned())), None, None, None]));
        let mut out = RenderList::new(); q.build(&mut c, 1920., 1080., 0, [true, true, false, false, false, false], &mut out);
        let card = layout(1920., 1080.)[1].card;
        let quads: Vec<_> = out.cmds.iter().filter_map(|c| match c { Cmd::Quad(q) => Some(q), _ => None }).collect();
        let bounds = |q: &&crate::draw::Quad| { let xs = q.pos.iter().map(|p| p[0]); let ys = q.pos.iter().map(|p| p[1]); (xs.clone().fold(f32::MAX, f32::min), ys.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max), ys.fold(f32::MIN, f32::max)) };
        let icon = quads.iter().map(bounds).find(|b| (b.2 - b.0 - 18.).abs() < 1e-3 && (b.3 - b.1 - 18.).abs() < 1e-3 && (b.2 - (card.x + card.w - 3.)).abs() < 1e-3).expect("an 18 x 18 icon at the card's right edge less BackgroundPadding 3");
        let glyphs: Vec<_> = quads.iter().map(bounds).filter(|b| b.0 >= card.x && b.2 <= icon.0 + 3. && b.1 >= card.y + card.h * 0.5 && b.3 <= card.y + card.h).collect();
        assert!(glyphs.len() >= 4, "two digits + their shadows left of the icon: {glyphs:?}");
        let (top, bottom) = (glyphs.iter().map(|g| g.1).fold(f32::MAX, f32::min), glyphs.iter().map(|g| g.3).fold(f32::MIN, f32::max));
        let row_mid = (icon.1 + icon.3) * 0.5;
        assert!(top < row_mid && bottom > row_mid && bottom <= card.y + card.h - 3. + 1e-3, "number centred on the icon row, inside the padding: {top} {bottom} {icon:?}");
        // no number and no icon: nothing is drawn for the slot (the pickaxe)
        q.count[1] = None; q.ammo[1] = None; let mut none = RenderList::new(); q.build(&mut c, 1920., 1080., 0, [true, true, false, false, false, false], &mut none);
        assert!(none.quad_count() < out.quad_count());
    }
    fn count_materials(l: &RenderList) -> usize { l.cmds.iter().filter(|c| matches!(c, Cmd::Material(_))).count() }
    fn all_finite(l: &RenderList) -> bool { l.cmds.iter().all(|c| match c { Cmd::Quad(q) => q.pos.iter().flatten().chain(q.uv.iter()).chain(q.tint.iter()).all(|v| v.is_finite()), Cmd::Material(m) => [m.rect.x, m.rect.y, m.rect.w, m.rect.h].iter().all(|v| v.is_finite()), _ => true }) }
    #[test] fn build_default_has_one_card_and_five_empty_frames() {
        let Some(mut c) = ctx() else { return };
        let mut q = Quickbar::default(); let mut out = RenderList::new();
        q.build(&mut c, 1920., 1080., 0, [true, false, false, false, false, false], &mut out);
        assert_eq!(count_materials(&out), 1);
        let Some(Cmd::Material(m)) = out.cmds.iter().find(|c| matches!(c, Cmd::Material(_))) else { panic!() };
        assert_eq!(m.rect, layout(1920., 1080.)[0].card.inflate(0.).deflate(0., -7., 0., 7.), "selected slot 1 is lifted by 7 units");
        assert!(matches!(m.kind, MaterialKind::ItemCard { icon, .. } if icon != TextureId::NONE), "slot 1 carries the pickaxe icon");
        assert!(all_finite(&out));
        // 5 empty frames * 9 + selection 9 + 6 keycaps * 9 = 108 nine-slice quads, + 6 key digits
        assert!(out.quad_count() >= 108 && out.quad_count() <= 108 + 6, "{}", out.quad_count());
    }
    #[test] fn build_full_bar_with_counts_has_six_cards_and_count_text() {
        let Some(mut c) = ctx() else { return };
        let mut q = Quickbar::default(); q.rarity = [0, 1, 2, 3, 4, 0]; q.count = [None, Some(3), Some(120), Some(7), Some(24), None]; q.snap(3);
        let mut a = RenderList::new(); q.build(&mut c, 1920., 1080., 3, [true; 6], &mut a);
        assert_eq!(count_materials(&a), 6); assert!(all_finite(&a));
        q.count = [None; 6]; let mut b = RenderList::new(); q.build(&mut c, 1920., 1080., 3, [true; 6], &mut b);
        assert!(a.quad_count() > b.quad_count(), "counts add glyph quads");
        // determinism
        let mut a2 = RenderList::new(); q.count = [None, Some(3), Some(120), Some(7), Some(24), None]; q.build(&mut c, 1920., 1080., 3, [true; 6], &mut a2);
        assert_eq!(a.cmds, a2.cmds);
    }
}
