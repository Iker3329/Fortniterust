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
//!  - AthenaQuickBarSlot: 80x80 item box; "Empty" frame (Quickbar-Frame-Default, box 0.25) inset 6 at 20% alpha; item card inset 2;
//!    "Selection" (Icon-BasicFrame-S, box 0.5) fills the slot; keybind widget box 45x45 BELOW the item (VerticalBox order top/item/bottom);
//!    keycap border (Keybind-Frame-L) brush alpha 0.6; "Selected" animation lifts the slot 7 units over 0.1 s.
//!  - Keybind text style TextStyle-Base-XS(-B-S): size 16, shadow offset 1.6 / colour (0,0,0,0.75).
//!  - Rarity colours: RarityData FortRarityItemData[N].Color1..5 (linear).
//! APPROXIMATE (not recovered): DPI curve (UE4 default, config is encrypted), keycap size (square 32), which RarityData stop paints which
//! part of the item card (FortMultiSizeItemCard is native C++), selection tint (white), animation easing (linear), font face (Segoe UI Bold,
//! original is NotoSans-Bold which is not exported), item-count position (bottom-right of card), whether the top keybind row is hidden.
//!
//! PORT NOTES: the old per-label GL text textures are replaced by the shared glyph atlas; the label box geometry (`ceil` advance + 2, `ceil` ascent/descent + 2,
//! glyphs offset by 1) is reproduced and snapped to whole pixels. The old non-material gradient fallback (used only when the card bake failed) is gone: the
//! card is always a `MaterialKind::ItemCard` command. NotoSans-Bold is now the real face (the old runtime used Segoe UI Bold unless NotoSans was found).
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

/// Pixel rectangles (selection animation NOT applied) for the six slots at a client size.
pub fn layout(w: f32, h: f32) -> [SlotLayout; 6] {
    let s = dpi_scale(w, h);
    let right = w - HUD_PAD_RIGHT * s;
    let bottom = h - (HUD_PAD_BOTTOM + PRIMARY_PAD_BOTTOM) * s;
    let left = right - ROW_WIDTH * s;
    let item_y = bottom - (KEYBIND_BOX + SLOT) * s;
    let mut out = [SlotLayout { item: Rect::ZERO, empty: Rect::ZERO, card: Rect::ZERO, keybind: Rect::ZERO, keycap: Rect::ZERO }; 6];
    for (i, o) in out.iter_mut().enumerate() {
        let x = left + if i == 0 { 0. } else { GROUP_GAP + SLOT * i as f32 } * s;
        let item = Rect::new(x, item_y, SLOT * s, SLOT * s);
        let keybind = Rect::new(x + (SLOT - KEYBIND_BOX) * 0.5 * s, item_y + SLOT * s, KEYBIND_BOX * s, KEYBIND_BOX * s);
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

#[derive(Clone)]
pub struct Quickbar {
    /// Rarity per slot (0 Common .. 4 Legendary). Slot 1 is the Common pickaxe (WID_Harvest_Pickaxe_Athena_C_T01).
    pub rarity: [usize; 6],
    /// Stack count per slot; None draws no count (the pickaxe has none; no ammo is invented).
    pub count: [Option<u32>; 6],
    /// Exported icon texture file name per slot (the item card material's "Icon" parameter); empty = no icon. Slot 1 defaults to the pickaxe.
    pub icon: [String; 6],
    /// WIDGETS3: `QuickbarSlotCooldown` state per slot (`screens::slot_cooldown`).
    pub cooldown: [Option<crate::screens::slot_cooldown::SlotCooldown>; 6],
    anim: [f32; 6],
    tex: Option<Tex>,
}
impl Default for Quickbar {
    fn default() -> Self {
        let mut icon: [String; 6] = Default::default();
        icon[0] = PICKAXE_ICON.to_owned();
        Self { rarity: [0; 6], count: [None; 6], icon, cooldown: [None; 6], anim: [1., 0., 0., 0., 0., 0.], tex: None }
    }
}

impl Quickbar {
    pub fn new() -> Self { Self::default() }
    /// Current Selected-animation weight per slot (0..1, linear; the cubic ease is applied when drawing).
    pub fn anim(&self) -> [f32; 6] { self.anim }
    /// Advance the Selected animation (0.1 s linear) toward the selected slot.
    pub fn tick(&mut self, selected: usize, dt: f32) { for (i, a) in self.anim.iter_mut().enumerate() { let target = if i == selected { 1. } else { 0. }; let step = dt / SELECTED_TIME; *a = if *a < target { (*a + step).min(target) } else { (*a - step).max(target) }; } }
    /// Test hook: jump the animation to its end state.
    pub fn snap(&mut self, selected: usize) { for (i, a) in self.anim.iter_mut().enumerate() { *a = if i == selected { 1. } else { 0. }; } }

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
        let lay = layout(w, h);
        for i in 0..6 {
            // Selected animation: Translation Y 0 -> -7 over 0.1 s, RichCurve interp = Cubic with zero tangents (reports/quickbar-evidence.json) = smoothstep.
            let dy = -lift_px(cubic_ease(self.anim[i]), s);
            let mv = |r: Rect| Rect::new(r.x, r.y + dy, r.w, r.h);
            let l = lay[i];
            if occupied[i] {
                let c = mv(l.card);
                let r = RARITY[self.rarity[i].min(4)];
                out.material(MaterialKind::ItemCard { colors: [r[0], r[1], r[2], r[3]], icon: icons[i], hero_tri: tex.hero_tri }, c, WHITE, false);
                if let Some(n) = self.count[i] { self.text_centered(ctx, out, &n.to_string(), c.x + c.w - 4. * s, c.y + c.h - 2. * s, s, true); }
            } else {
                // "Empty" image: Quickbar-Frame-Default, box margin 0.25, ColorAndOpacity alpha 0.2.
                out.nine_slice(tex.frame, mv(l.empty), [0.25; 4], s, [1., 1., 1., 0.2]);
            }
            if i == selected || self.anim[i] > 0. { out.nine_slice(tex.selection, mv(l.item), [0.5; 4], s, [1., 1., 1., self.anim[i]]); }
            // QuickbarSlotCooldown (WIDGETS3): the Item overlay's child above the cards and the selection frame, filling the 80 x 80 item box
            if let Some(cd) = self.cooldown[i] { crate::screens::slot_cooldown::build(ctx, out, mv(l.item), s, i, &cd); }
            // Keybind widget: keycap border (alpha 0.6) + slot number.
            out.nine_slice(tex.keycap, mv(l.keycap), [0.25; 4], s, [1., 1., 1., 0.6]);
            let k = mv(l.keycap);
            self.text_centered(ctx, out, &(i + 1).to_string(), k.x + k.w * 0.5, k.y + k.h * 0.5, s, false);
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
        // Right edge: 1920 - 20. Row width 496. Slot 6 ends at right-8 (trailing group gap).
        assert_eq!(l[0].item.x, 1900. - 496.);
        assert_eq!(l[5].item.x + l[5].item.w, 1900. - 8.);
        // Bottom: 10 + 8 padding, keybind row 45 under the 80 item.
        assert_eq!(l[0].keybind.y + l[0].keybind.h, 1080. - 18.);
        assert_eq!(l[0].item.y, 1080. - 18. - 45. - 80.);
    }
    #[test] fn group_gap_only_after_first_slot() {
        let l = layout(1920., 1080.);
        assert_eq!(l[1].item.x - (l[0].item.x + l[0].item.w), 8.);
        for i in 2..6 { assert_eq!(l[i].item.x - (l[i - 1].item.x + l[i - 1].item.w), 0.); }
    }
    #[test] fn paddings_and_keybind_geometry() {
        let l = layout(1920., 1080.)[2];
        assert_eq!(l.empty, Rect::new(l.item.x + 6., l.item.y + 6., 68., 68.));
        assert_eq!(l.card, Rect::new(l.item.x + 2., l.item.y + 2., 76., 76.));
        assert_eq!(l.keybind.w, 45.); assert_eq!(l.keybind.x + 22.5, l.item.x + 40.);
        assert_eq!(l.keybind.y, l.item.y + l.item.h);
    }
    #[test] fn layout_scales_with_dpi_and_stays_on_screen() {
        for (w, h) in [(1920., 1080.), (1280., 720.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.), (3840., 2160.)] {
            let l = layout(w, h); let s = dpi_scale(w, h);
            assert!((l[0].item.w - 80. * s).abs() < 1e-3);
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
