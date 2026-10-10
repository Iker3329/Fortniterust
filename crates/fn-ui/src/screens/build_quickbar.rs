//! BUILD HUD: the building quickbar (`/Game/Athena/HUD/AthenaQuickbarSecondary`, five `AthenaQuickBarSlot` children of the `QuickbarSlot` base) as the 1.8
//! Athena HUD lays it out, plus its minimise / maximise behaviour. Every number below was read from the cooked 1.8 packages with `fn-inspect props / disasm`
//! (the original widget data and Kismet bytecode) or from the 1.8 exe; nothing is redrawn from screenshots.
//!
//! PLACEMENT (`AthenaHUD` WidgetTree): `QuickbarSecondary` is VerticalBoxSlot_0 (HAlign_Right, VAlign_Bottom) of "[Vertical Box] Bottom Left Content", whose overlay slot
//! is HAlign_Right / VAlign_Bottom with Padding Right 20 / Bottom 10. Its sibling below is "[Size Box] Quickbars" (MinDesiredHeight 160, the only enabled override)
//! that holds the primary bar, so the building bar's bottom edge sits 10 + 160 units above the screen bottom (the primary bar's content, 45 + item box + 8, is
//! always below 160) and its right edge 20 units in from the right. HUD element tags `HUD.Quickbar.Build` (both bars).
//!
//! TREE (`AthenaQuickbarSecondary`): `[Bang Wrapper] QuickBarTop` > HorizontalBox_140 [
//!   `[Horizontal Box] Camera Mode Content` (HorizontalBoxSlot_1689, Fill) [
//!     HorizontalBox_1 (VAlign_Center) [ `KeybindSwitchBinding` (KeybindWidget, Brush Size Medium, BoundAction SwitchQuickbar via `HandleKeyBindsChange`),
//!                                       `[Image] Switch Action` (40x40, Icon-QuickbarMode-Build-M; Fight-M while maximised) ],
//!     Overlay_0 (Padding L8 R8) [ HorizontalBox_0 [ QuickbarSlotF1..F4 (VAlign_Center) ], SizeBox_Building rail (Hidden on mouse/keyboard: `HandleShowHideRail`) ],
//!     `[Image] Separator` (Rule-Dotted-HUD 8x74 tiled vertically, ColorAndOpacity alpha 0.8, Padding L-5 B2, VAlign_Bottom) ],
//!   Overlay_1 (HAlign_Center, VAlign_Center, Padding R8) [ QuickbarSlotF5 (the trap slot) ] ].
//!
//! SLOT (`QuickbarSlot` / `AthenaQuickBarSlot`): VerticalBox_0 [ SwitcherTopComboSwitcher (Padding Bottom 16; index 0 = `SizeBoxTopKeybind` 32x32 > `KeybindTop`,
//! default Brush Size = Small: 24x24 Keybind-Frame-S, NotoSans-Bold 12, frame alpha 0.6), `[SizeBox] Item`, SwitcherBottomComboSwitcher (Collapsed for the secondary bar) ].
//! `GetKeyBindingAction_Keyboard`: secondary slot N binds `HotbarKeyF{N+1}`. `UpdateKeyBindingVisibility`: on mouse/keyboard the secondary bar shows the TOP keybind and
//! collapses the bottom one; `AthenaQuickbarSecondary.HandleMinimizeHiding` hides the top keybind only for the gamepad presets, so F1..F5 stay visible while minimised.
//! Item overlay: `Selection` (Icon-BasicFrame-S, margin 0.5, fill), `Empty` (Quickbar-Frame-Default, margin 0.25, alpha 0.2, Padding 6), item cards (Padding 2).
//!
//! SIZE (`QuickbarSlot.UpdateItemBoxSize`): `[SizeBox] Item` width/height = Lerp(XS card + padding, S card + padding, AnimatedBoxScaleFactor). Card dimensions are
//! `UFortItemCardUtilities::GetCardDimensions` @142fa68a0; 1.8 uses the new item cards (`UFortMultiSizeItemCard::ShouldUseNewItemCards` @14305a780: true unless
//! `-UseOldItemCards`) so the size comes from `DefaultUIDataConfiguration.OtherItemCardSizes` (building pieces are neither schematic nor personnel): XS = 60, S = 88.
//! With the 2+2 card padding the item box is 64 (minimised) .. 92 (maximised).
//! `AnimMaximize` / `AnimMinimize`: one float track 0 -> 1 / 1 -> 0 over 0.1 s, RichCurve interp Cubic with zero tangents (= smoothstep).
//! `AthenaHUD.SetQuickbarSizes`: FocusedQuickbar 1 (secondary) -> QuickbarSecondary.Maximize, QuickbarPrimary.Minimize, Separator Visible; FocusedQuickbar 0 ->
//! the reverse with the Separator Collapsed. `AthenaQuickbarSecondary` minimise sets the Switch Action icon to Icon-QuickbarMode-Build-M, maximise to -Fight-M.
//! Selection: `QuickbarBase.OnQuickbarSlotFocusChanged` selects the focused slot of the focused bar only (the other bar deselects, FocusedSlot = -1); `Selected` lifts
//! Overlay_Content 7 units over 0.1 s (cubic, the same animation as the primary bar).
//!
//! NOT 1:1 (documented gaps): the FortMultiSizeItemCard is native C++; the card is drawn with the port's translated `M-Item-Inst-S` material at the item box
//! minus padding (the native card swaps between its XS and S sizes at the animation ends; the port scales one card with the box). The item card's own decorations
//! for building pieces (if any) are not decoded. Building piece rarity is the item definition default (no `Rarity` property cooked -> RarityData row 0).
use crate::draw::{MaterialKind, RenderList, TextureId};
use crate::layout::{dpi_scale, Rect};
use crate::material::hud::cubic_ease;
use crate::scene::UiContext;
use crate::screens::quickbar::RARITY;
use crate::screens::{art, ascent, atlas_px, line_height, mat_tex, put_text, text_width, Snap, WHITE};
use crate::text::FontId;

// ---- AthenaHUD placement ----
pub const HUD_PAD_RIGHT: f32 = 20.; // "[Vertical Box] Bottom Left Content" overlay slot Padding.Right
pub const HUD_PAD_BOTTOM: f32 = 10.; // same slot Padding.Bottom
pub const QUICKBARS_MIN_HEIGHT: f32 = 160.; // "[Size Box] Quickbars" MinDesiredHeight (bOverride_MinDesiredHeight)
// ---- AthenaQuickbarSecondary ----
pub const SLOTS_PAD: f32 = 8.; // HorizontalBoxSlot_1697 Padding Left/Right 8 around HorizontalBox_0
pub const TRAP_PAD_RIGHT: f32 = 8.; // HorizontalBoxSlot_1692 Padding.Right
pub const SWITCH_ICON: f32 = 40.; // [Image] Switch Action Brush ImageSize 40x40
pub const SEPARATOR: (f32, f32) = (8., 74.); // [Image] Separator Brush ImageSize
pub const SEPARATOR_PAD_LEFT: f32 = -5.; // HorizontalBoxSlot_0 Padding.Left
pub const SEPARATOR_PAD_BOTTOM: f32 = 2.; // HorizontalBoxSlot_0 Padding.Bottom
pub const SEPARATOR_ALPHA: f32 = 0.8; // ColorAndOpacity (1,1,1,0.8)
// ---- QuickbarSlot ----
pub const TOP_KEYBIND_BOX: f32 = 32.; // SizeBoxTopKeybind 32x32
pub const TOP_KEYBIND_PAD: f32 = 16.; // VerticalBoxSlot_2 Padding.Bottom
pub const CARD_PAD: f32 = 2.; // OverlaySlot_2 / OverlaySlot_3 Padding 2
pub const EMPTY_PAD: f32 = 6.; // OverlaySlot_19518 Padding 6
pub const EMPTY_ALPHA: f32 = 0.2; // Empty ColorAndOpacity alpha
pub const CARD_XS: f32 = 60.; // DefaultUIDataConfiguration.OtherItemCardSizes[XS] = (60, 60)
pub const CARD_S: f32 = 88.; // DefaultUIDataConfiguration.OtherItemCardSizes[S] = (88, 88)
pub const BOX_MIN: f32 = CARD_XS + 2. * CARD_PAD;
pub const BOX_MAX: f32 = CARD_S + 2. * CARD_PAD;
pub const RESIZE_TIME: f32 = 0.1; // AnimMaximize / AnimMinimize PlaybackRange upper bound
pub const SELECTED_LIFT: f32 = 7.; // Selected: Translation[1] 0 -> -7
pub const SELECTED_TIME: f32 = 0.1;
// ---- KeybindWidget (UpdateFrameSize table [24, 24, 24, 32, 64, 64]; font Round(size * 1.5 * 96 / 300); frame padding size * 0.1; frame alpha 0.6) ----
pub const KEYCAP_SMALL: f32 = 24.;
pub const KEYCAP_SMALL_FONT: f32 = 12.;
pub const KEYCAP_MEDIUM: f32 = 32.;
pub const KEYCAP_MEDIUM_FONT: f32 = 15.;
pub const KEYCAP_ALPHA: f32 = 0.6;
/// `Brush_S` / `Brush_M` of KeybindWidget (Box margin 0.25).
pub const FRAME_S: &str = "Keybind-Frame-S.png";
pub const FRAME_M: &str = "Keybind-Frame-M.png";
/// HUD FIX: the mode icons and the separator are not in the exported UI folder; the game decodes them from the paks at load
/// (`fn_pose::loadout_stats::HUD_PAK_TEXTURES` -> the UI asset overlay under these names), also in scripted runs.
pub const ICON_BUILD: &str = "Icon-QuickbarMode-Build-M.png";
pub const ICON_FIGHT: &str = "Icon-QuickbarMode-Fight-M.png";
pub const SEPARATOR_TEX: &str = "Rule-Dotted-HUD.png";
/// `SmallPreviewImage` of BuildingItemData_Wall / _Floor / _Stair_W / _RoofS (the four building slots in their quickbar order).
pub const PIECE_ICONS: [&str; 4] = ["T_BuildMode_Prefab_Wall_Large-64.png", "T_BuildMode_Prefab_Floor_Large-64.png", "T_BuildMode_Prefab_Stair_Large-64.png", "T_BuildMode_Prefab_Ceiling_Large-64.png"];

/// What the building bar shows (plain data from the game side).
#[derive(Clone, Debug, PartialEq)]
pub struct BuildBarState {
    /// `FortInventoryContext.GetQuickbarFocus` OutQuickBar == Secondary (build mode). Drives maximise / minimise of both bars.
    pub focused_secondary: bool,
    /// `AthenaQuickbarSecondary.CurrentBuildingSlot` (0..=4; 4 = the trap slot F5).
    pub building_slot: usize,
    /// Icon texture per slot F1..F5 (None = empty slot: the "Empty" frame).
    pub icons: [Option<String>; 5],
    /// RarityData row per slot (0 = Common).
    pub rarity: [u8; 5],
    /// Key text of `HotbarKeyF1..F5` (empty = unbound: the keycap is not drawn).
    pub slot_keys: [String; 5],
    /// Key text of `SwitchQuickbar` for `KeybindSwitchBinding`.
    pub switch_key: String,
}
impl Default for BuildBarState {
    fn default() -> Self {
        BuildBarState { focused_secondary: false, building_slot: 0, icons: [Some(PIECE_ICONS[0].into()), Some(PIECE_ICONS[1].into()), Some(PIECE_ICONS[2].into()), Some(PIECE_ICONS[3].into()), None],
            rarity: [0; 5], slot_keys: ["F1".into(), "F2".into(), "F3".into(), "F4".into(), "F5".into()], switch_key: "Q".into() }
    }
}

/// Item box size (Slate units) for an `AnimatedBoxScaleFactor` (already eased).
pub fn item_box(scale_factor: f32) -> f32 { BOX_MIN + (BOX_MAX - BOX_MIN) * scale_factor.clamp(0., 1.) }

/// Pixel rectangles of the bar (selection lift not applied).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarLayout {
    pub scale: f32,
    pub row: Rect,
    pub switch_keycap_box: Rect,
    pub switch_icon: Rect,
    /// Item box of F1..F5 (index 4 = the trap slot).
    pub item: [Rect; 5],
    /// SizeBoxTopKeybind of F1..F5.
    pub top_keybind: [Rect; 5],
    /// None while the separator is Collapsed (secondary minimised).
    pub separator: Option<Rect>,
}

/// Layout at client size `w` x `h`. `scale_factor` = eased AnimatedBoxScaleFactor of the secondary slots, `separator_visible` = FocusedQuickbar == Secondary
/// (`SetQuickbarSizes`), `switch_keycap_w` = width of the Medium keycap in Slate units (min 32, grows with the key text).
pub fn layout(w: f32, h: f32, scale_factor: f32, separator_visible: bool, switch_keycap_w: f32) -> BarLayout {
    let s = dpi_scale(w, h);
    let bx = item_box(scale_factor);
    let col_h = TOP_KEYBIND_BOX + TOP_KEYBIND_PAD + bx;
    let hb1_w = switch_keycap_w.max(KEYCAP_MEDIUM) + SWITCH_ICON;
    let hb1_h = KEYCAP_MEDIUM.max(SWITCH_ICON);
    let sep_w = if separator_visible { SEPARATOR.0 + SEPARATOR_PAD_LEFT } else { 0. };
    let sep_h = if separator_visible { SEPARATOR.1 + SEPARATOR_PAD_BOTTOM } else { 0. };
    let total_w = hb1_w + SLOTS_PAD + 4. * bx + SLOTS_PAD + sep_w + bx + TRAP_PAD_RIGHT;
    let row_h = hb1_h.max(col_h).max(sep_h);
    let right = w - HUD_PAD_RIGHT * s;
    let bottom = h - (HUD_PAD_BOTTOM + QUICKBARS_MIN_HEIGHT) * s;
    let (x0, top) = (right - total_w * s, bottom - row_h * s);
    let row = Rect::new(x0, top, total_w * s, row_h * s);
    // HorizontalBox_1: VAlign_Center in the row; its children VAlign_Center in its 40-unit height.
    let hb1_top = top + (row_h - hb1_h) * 0.5 * s;
    let kw = switch_keycap_w.max(KEYCAP_MEDIUM);
    let switch_keycap_box = Rect::new(x0, hb1_top + (hb1_h - KEYCAP_MEDIUM) * 0.5 * s, kw * s, KEYCAP_MEDIUM * s);
    let switch_icon = Rect::new(x0 + kw * s, hb1_top + (hb1_h - SWITCH_ICON) * 0.5 * s, SWITCH_ICON * s, SWITCH_ICON * s);
    let col_top = top + (row_h - col_h) * 0.5 * s; // slots are VAlign_Center
    let column = |x: f32| (Rect::new(x + (bx - TOP_KEYBIND_BOX) * 0.5 * s, col_top, TOP_KEYBIND_BOX * s, TOP_KEYBIND_BOX * s), Rect::new(x, col_top + (TOP_KEYBIND_BOX + TOP_KEYBIND_PAD) * s, bx * s, bx * s));
    let mut item = [Rect::ZERO; 5];
    let mut top_keybind = [Rect::ZERO; 5];
    let slots_x = x0 + (hb1_w + SLOTS_PAD) * s;
    for i in 0..4 { let (k, it) = column(slots_x + i as f32 * bx * s); top_keybind[i] = k; item[i] = it; }
    let slots_end = slots_x + (4. * bx + SLOTS_PAD) * s;
    let separator = separator_visible.then(|| Rect::new(slots_end + SEPARATOR_PAD_LEFT * s, bottom - (SEPARATOR_PAD_BOTTOM + SEPARATOR.1) * s, SEPARATOR.0 * s, SEPARATOR.1 * s));
    let (k, it) = column(right - (TRAP_PAD_RIGHT + bx) * s);
    top_keybind[4] = k; item[4] = it;
    BarLayout { scale: s, row, switch_keycap_box, switch_icon, item, top_keybind, separator }
}

#[derive(Clone, Copy)]
struct Tex { frame_s: crate::assets::TexInfo, frame_m: crate::assets::TexInfo, empty: crate::assets::TexInfo, selection: crate::assets::TexInfo, build: TextureId, fight: TextureId, separator: crate::assets::TexInfo, hero_tri: TextureId }

/// HUD FIX: the Separator brush is `Tiling: ESlateBrushTileType::Vertical` (cooked `[Image] Separator`). UE4.16 ElementBatcher.cpp:367 tiles it
/// `LocalSize.Y / TextureHeight` times (the texture's real pixel height, `ResourceProxy->ActualSize`), from the top of the element; the last tile is cut.
/// The U axis is not tiled (the 8-unit width maps the whole texture). Returns (rect, uv) per tile.
pub fn tile_vertical(r: Rect, tex_h_px: u32, scale: f32) -> Vec<(Rect, [f32; 4])> {
    let th = tex_h_px.max(1) as f32 * scale;
    let mut out = vec![];
    let mut y = 0.;
    while y < r.h - 1e-3 && out.len() < 4096 {
        let hh = th.min(r.h - y);
        out.push((Rect::new(r.x, r.y + y, r.w, hh), [0., 0., 1., hh / th]));
        y += th;
    }
    out
}

/// The building quickbar widget: state + its two animations.
#[derive(Clone, Default)]
pub struct BuildQuickbar {
    pub state: BuildBarState,
    /// Drawn only once a producer drives it (the game sets this every frame). Off by default so the HUD capture scenes that reproduce the old runtime's
    /// screenshots (which never had the building bar) stay as recorded; the 1.8 HUD always shows the bar.
    pub shown: bool,
    /// AnimatedBoxScaleFactor timeline position 0..1 (linear; eased when drawn). Starts minimised (the combat bar is focused on spawn).
    resize_t: f32,
    /// Selected animation position per slot 0..1.
    sel: [f32; 5],
    /// Slot selected at the last `tick` (detects the deselect that restarts the lift reverse from its end).
    sel_on: [bool; 5],
    tex: Option<Tex>,
    /// Focus at the last `tick` (a change starts AnimMaximize / AnimMinimize from time 0, see [`resize_step`]).
    focus_last: bool,
}

/// HUD FIX2: one step of a slot's AnimatedBoxScaleFactor timeline (`t` linear 0..1, maximised = 1). `QuickbarSlot.Maximize` (cooked Kismet,
/// `fn-inspect disasm /Game/UI/InGame/HUD/QuickbarSlot`): `IsAnimationPlaying(AnimMinimize)` -> `StopAnimation(AnimMinimize)`, then
/// `PlayAnimation(AnimMaximize, StartAtTime 0, 1 loop, Forward, speed 1)`; `Minimize` is the mirror image (stops AnimMaximize, plays AnimMinimize from 0).
/// UE4.16 `UUMGSequencePlayer::PlayInternal` puts the cursor at StartAtTime, so a focus change mid-animation JUMPS to the new animation's first key
/// (AnimMaximize 0 -> 1, AnimMinimize 1 -> 0) instead of reversing from the current value. QuickbarBase only calls them on a real change (`IsMaximized`).
pub fn resize_step(t: f32, focused: bool, last: &mut bool, dt: f32) -> f32 {
    let mut t = t;
    if focused != *last { t = if focused { 0. } else { 1. }; *last = focused; }
    let step = dt / RESIZE_TIME;
    if focused { (t + step).min(1.) } else { (t - step).max(0.) }
}

impl BuildQuickbar {
    pub fn new() -> Self { Self::default() }
    /// AnimatedBoxScaleFactor as drawn (eased).
    pub fn scale_factor(&self) -> f32 { cubic_ease(self.resize_t) }
    pub fn selection_anim(&self) -> [f32; 5] { self.sel }
    /// Advance Maximize / Minimize (0.1 s) and the Selected animation toward the focused slot (only while the secondary bar is focused).
    pub fn tick(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.max(0.) } else { 0. };
        self.resize_t = resize_step(self.resize_t, self.state.focused_secondary, &mut self.focus_last, dt);
        for (i, a) in self.sel.iter_mut().enumerate() {
            let on = self.state.focused_secondary && i == self.state.building_slot;
            // deselect = PlayAnimation(Selected, StartAtTime = GetEndTime, Reverse): a slot that just lost focus reverses from the end (smallfix_followups.md section 2)
            if self.sel_on[i] && !on { *a = 1.; }
            self.sel_on[i] = on;
            let t = if on { 1. } else { 0. };
            let st = dt / SELECTED_TIME;
            *a = if *a < t { (*a + st).min(t) } else { (*a - st).max(t) };
        }
    }
    /// Jump both animations to their end state (captures, tests).
    pub fn snap(&mut self) {
        self.resize_t = if self.state.focused_secondary { 1. } else { 0. };
        self.focus_last = self.state.focused_secondary;
        for (i, a) in self.sel.iter_mut().enumerate() { let on = self.state.focused_secondary && i == self.state.building_slot; self.sel_on[i] = on; *a = if on { 1. } else { 0. }; }
    }

    fn ensure_tex(&mut self, ctx: &mut UiContext) -> Tex {
        if let Some(t) = self.tex { return t; }
        let t = Tex {
            frame_s: art(ctx, FRAME_S), frame_m: art(ctx, FRAME_M), empty: art(ctx, "Quickbar-Frame-Default.png"), selection: art(ctx, "Icon-BasicFrame-S.png"),
            build: art(ctx, ICON_BUILD).id, fight: art(ctx, ICON_FIGHT).id, separator: art(ctx, SEPARATOR_TEX), hero_tri: mat_tex(ctx, "T-Card-Hero-Anim-Triangle.png", true).id,
        };
        self.tex = Some(t); t
    }

    #[allow(clippy::too_many_arguments)]
    fn keycap(ctx: &mut UiContext, out: &mut RenderList, frame: crate::assets::TexInfo, rect: Rect, s: f32, font: f32, key: &str) {
        if let Some(icon) = crate::screens::target_info::key_icon(key) { crate::screens::target_info::draw_key_icon(ctx, out, icon, rect, s); return; }
        let text = &crate::screens::target_info::hud_key_text(key);
        out.nine_slice(frame, rect, [0.25; 4], s, [1., 1., 1., KEYCAP_ALPHA]);
        let px = atlas_px(font, s);
        let tw = text_width(ctx, FontId::Bold, text, px);
        let (lh, asc) = (line_height(ctx, FontId::Bold, px), ascent(ctx, FontId::Bold, px));
        put_text(ctx, out, FontId::Bold, px, (rect.x + (rect.w - tw) * 0.5).round(), (rect.y + (rect.h - lh) * 0.5 + asc).round(), text, WHITE, None, Snap::PerGlyph);
    }

    /// Append the bar. Draw order follows the widget tree (keybind, switch icon, slots F1..F4, separator, trap slot).
    pub fn build(&mut self, ctx: &mut UiContext, w: f32, h: f32, out: &mut RenderList) {
        if !self.shown { return; }
        let tex = self.ensure_tex(ctx);
        let s = dpi_scale(w, h);
        let st = self.state.clone();
        let mpx = atlas_px(KEYCAP_MEDIUM_FONT, s);
        let switch_w = if st.switch_key.is_empty() || crate::screens::target_info::key_icon(&st.switch_key).is_some() { KEYCAP_MEDIUM } else { (text_width(ctx, FontId::Bold, &crate::screens::target_info::hud_key_text(&st.switch_key), mpx) / s + 2. * KEYCAP_MEDIUM * 0.1).max(KEYCAP_MEDIUM) };
        let l = layout(w, h, self.scale_factor(), st.focused_secondary, switch_w);
        if !st.switch_key.is_empty() { Self::keycap(ctx, out, tex.frame_m, l.switch_keycap_box, s, KEYCAP_MEDIUM_FONT, &st.switch_key); }
        out.sprite(if st.focused_secondary { tex.fight } else { tex.build }, l.switch_icon, WHITE);
        let spx = atlas_px(KEYCAP_SMALL_FONT, s);
        for i in 0..5 {
            if i == 4 { if let Some(r) = l.separator { if tex.separator.id != TextureId::NONE { for (tr, uv) in tile_vertical(r, tex.separator.h, s) { out.quad(tex.separator.id, tr, uv, [1., 1., 1., SEPARATOR_ALPHA]); } } } }
            let dy = -cubic_ease(self.sel[i]) * SELECTED_LIFT * s;
            let it = Rect::new(l.item[i].x, l.item[i].y + dy, l.item[i].w, l.item[i].h);
            match &st.icons[i] {
                Some(icon) => {
                    let icon = mat_tex(ctx, icon, false).id;
                    let r = RARITY[(st.rarity[i] as usize).min(4)];
                    let c = Rect::new(it.x + CARD_PAD * s, it.y + CARD_PAD * s, it.w - 2. * CARD_PAD * s, it.h - 2. * CARD_PAD * s);
                    out.material(MaterialKind::ItemCard { colors: [r[0], r[1], r[2], r[3]], icon, hero_tri: tex.hero_tri }, c, WHITE, false);
                }
                None => out.nine_slice(tex.empty, Rect::new(it.x + EMPTY_PAD * s, it.y + EMPTY_PAD * s, it.w - 2. * EMPTY_PAD * s, it.h - 2. * EMPTY_PAD * s), [0.25; 4], s, [1., 1., 1., EMPTY_ALPHA]),
            }
            // `Selection` image Visible / Collapsed in the same frame as the focus change (HotbarKeyF1 FUN_14083a690 -> FUN_140698b20 -> AFortQuickBars+0x590
            // broadcast -> OnQuickbarSlotFocusChanged -> QuickbarSlot.SetSelected), full opacity; only the lift (`sel`) animates (smallfix_followups.md section 2)
            if st.focused_secondary && i == st.building_slot { out.nine_slice(tex.selection, it, [0.5; 4], s, WHITE); }
            // KeybindTop: Small keycap (24, grows with the text) centred in the 32x32 SizeBoxTopKeybind
            let key = &st.slot_keys[i];
            if !key.is_empty() {
                let kb = l.top_keybind[i];
                let kw = if crate::screens::target_info::key_icon(key).is_some() { KEYCAP_SMALL * s } else { (text_width(ctx, FontId::Bold, &crate::screens::target_info::hud_key_text(key), spx) / s + 2. * KEYCAP_SMALL * 0.1).max(KEYCAP_SMALL) * s };
                let kr = Rect::new(kb.x + (kb.w - kw) * 0.5, kb.y + (kb.h - KEYCAP_SMALL * s) * 0.5, kw, KEYCAP_SMALL * s);
                Self::keycap(ctx, out, tex.frame_s, kr, s, KEYCAP_SMALL_FONT, key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Cmd;
    use crate::screens::testutil::ctx;
    /// HUD FIX: Rule-Dotted-HUD (8x8, ImportedSize) tiled vertically over the 74-unit separator: 9 whole tiles + a quarter tile cut at the bottom.
    #[test] fn separator_tiles_vertically_from_the_top() {
        let t = tile_vertical(Rect::new(10., 20., 8., 74.), 8, 1.);
        assert_eq!(t.len(), 10);
        assert_eq!(t[0], (Rect::new(10., 20., 8., 8.), [0., 0., 1., 1.]));
        assert_eq!(t[9], (Rect::new(10., 92., 8., 2.), [0., 0., 1., 0.25]));
        assert_eq!(tile_vertical(Rect::new(0., 0., 16., 148.), 8, 2.).len(), 10, "DPI scale applies to the tile height too");
    }
    /// HUD FIX2: Minimize during a half-played AnimMaximize stops it and plays AnimMinimize from time 0: the box jumps to maximised, then shrinks.
    #[test] fn focus_change_restarts_the_other_animation_from_its_start() {
        let mut b = BuildQuickbar::new();
        b.state.focused_secondary = true; b.tick(0.05); assert!((b.resize_t - 0.5).abs() < 1e-5);
        b.state.focused_secondary = false; b.tick(0.); assert_eq!(b.resize_t, 1., "AnimMinimize's first key");
        b.tick(0.03); assert!((b.resize_t - 0.7).abs() < 1e-5);
        b.state.focused_secondary = true; b.tick(0.); assert_eq!(b.resize_t, 0., "AnimMaximize's first key");
        b.tick(0.); assert_eq!(b.resize_t, 0., "no restart without a change");
    }
    #[test] fn item_box_is_the_ui_config_card_plus_padding() {
        assert_eq!((BOX_MIN, BOX_MAX), (64., 92.));
        assert_eq!(item_box(0.), 64.); assert_eq!(item_box(1.), 92.); assert_eq!(item_box(0.5), 78.);
    }
    #[test] fn bar_sits_above_the_quickbars_sizebox_right_aligned() {
        let l = layout(1920., 1080., 1., true, 32.);
        assert_eq!(l.scale, 1.);
        assert_eq!(l.row.x + l.row.w, 1900.);
        assert_eq!(l.row.y + l.row.h, 1080. - 170.);
        // maximised: column = 32 + 16 + 92 = 140 tall (taller than the separator 76 and the switch row 40)
        assert_eq!(l.row.h, 140.);
        // width: (32 + 40) + 8 + 4*92 + 8 + (8 - 5) + 92 + 8
        assert_eq!(l.row.w, 72. + 8. + 368. + 8. + 3. + 92. + 8.);
        assert_eq!(l.item[4].x + l.item[4].w, 1900. - 8.);
        assert_eq!(l.item[1].x - l.item[0].x, 92.);
        assert_eq!(l.item[0].y - l.top_keybind[0].y, 48.);
        let sep = l.separator.unwrap(); assert_eq!((sep.w, sep.h, sep.y + sep.h), (8., 74., 1080. - 170. - 2.));
        assert_eq!(sep.x, l.item[3].x + 92. + 8. - 5.);
    }
    #[test] fn minimised_bar_collapses_the_separator_and_shrinks_the_boxes() {
        let l = layout(1920., 1080., 0., false, 32.);
        assert!(l.separator.is_none());
        assert_eq!(l.item[0].w, 64.); assert_eq!(l.row.h, 32. + 16. + 64.);
        assert_eq!(l.row.w, 72. + 8. + 256. + 8. + 64. + 8.);
    }
    #[test] fn maximise_and_minimise_take_a_tenth_of_a_second() {
        let mut b = BuildQuickbar::new();
        assert_eq!(b.scale_factor(), 0.);
        b.state.focused_secondary = true; b.tick(0.05); assert!((b.resize_t - 0.5).abs() < 1e-5); assert!((b.scale_factor() - 0.5).abs() < 1e-5, "smoothstep(0.5) = 0.5");
        b.tick(0.05); assert_eq!(b.scale_factor(), 1.);
        assert!((b.selection_anim()[0] - 1.).abs() < 1e-5, "slot F1 selected while focused");
        b.state.focused_secondary = false; b.tick(1.); assert_eq!(b.scale_factor(), 0.); assert_eq!(b.selection_anim(), [0.; 5], "the unfocused bar shows no selection");
    }
    /// F1 -> F2: the frame moves in the same frame (alpha 1, no fade); the lift of a cut-short select reverses from its end.
    #[test] fn selection_frame_switches_in_the_same_frame() {
        let mut b = BuildQuickbar::new(); b.state.focused_secondary = true; b.snap();
        b.state.building_slot = 1; b.tick(0.03);
        assert!((b.sel[0] - 0.7).abs() < 1e-5 && (b.sel[1] - 0.3).abs() < 1e-5);
        b.state.building_slot = 2; b.tick(0.02);
        assert!((b.sel[1] - 0.8).abs() < 1e-5, "reverse from the end, {}", b.sel[1]);
        let Some(mut c) = ctx() else { return };
        b.shown = true;
        let mut out = RenderList::new(); b.build(&mut c, 1920., 1080., &mut out);
        let sel = b.tex.unwrap().selection.id;
        let a: Vec<f32> = out.cmds.iter().filter_map(|c| match c { Cmd::Quad(q) if q.tex == sel => Some(q.tint[3]), _ => None }).collect();
        assert!(!a.is_empty() && a.len() <= 9 && a.iter().all(|v| *v == 1.), "one opaque frame only: {a:?}");
    }
    #[test] fn builds_four_piece_cards_and_an_empty_trap_slot() {
        let Some(mut c) = ctx() else { return };
        let mut b = BuildQuickbar::new(); b.shown = true; b.state.focused_secondary = true; b.snap();
        let mut out = RenderList::new(); b.build(&mut c, 1920., 1080., &mut out);
        assert_eq!(out.cmds.iter().filter(|c| matches!(c, Cmd::Material(_))).count(), 4);
        let mut out2 = RenderList::new(); b.build(&mut c, 1920., 1080., &mut out2);
        assert_eq!(out.cmds, out2.cmds, "deterministic");
    }
}
