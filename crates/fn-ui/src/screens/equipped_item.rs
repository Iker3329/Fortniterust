//! The equipped-item ammo counter: `AthenaEquippedItemWidget_C` (tag `HUD.Quickbar.Build.EquippedItem`) of the 1.8 `AthenaHUD`.
//!
//! SOURCES (fn-inspect `props` / `disasm` of `/Game/Athena/HUD/AthenaEquippedItemWidget` and `AthenaHUD`, `TextStyle-Base-L-S`, `StyleLibrary`, `FortUIStylesheet`):
//! * Placement: AthenaHUD `[Overlay] Persistent HUD Content` slot `OverlaySlot_10` (Padding Bottom 160, HAlign Center, VAlign Bottom) holds `Overlay_3`, whose `OverlaySlot_0` holds this
//!   widget (HAlign Center, VAlign Bottom) next to `AthenaHealProgress` (see `equip_progress.rs`).
//! * Tree: `Overlay_28` = [`SwitcherContent` (VAlign Bottom; index 1 = `HorizontalBoxAmmo` for firearms), `TextAmmoOutOverlay` (padding Bottom 40 / Left 4, VAlign Bottom,
//!   RenderTransform angle -5 / translation (0, 24), `TextStyle-Base-S-B-S`, "Out of Ammo!", Collapsed unless the player has no ammo)].
//!   `HorizontalBoxAmmo` = [`SizeBox_6` (Height 24, min 16; padding Top 18, HAlign Right, VAlign Top; empty), `AmmoDisplayVBox` -> `AmmoDisplayHBox` = [`TextMagazineAmmoCount`
//!   (HAlign Right, padding Left 8, Fill 0.75), `CommonBorderDivider` (Border_HUD_Line_Vert, `SizeBox_3` 4 x 36, padding 8 / 8), `TextTotalAmmoCount`],
//!   `HorizontalBox_0` (padding Left 4 / Top 8, HAlign Center, VAlign Top) -> `Overlay_529` -> `AmmoIcon` 40 x 40].
//! * Style: both counts `TextStyle-Base-L-S` = size 32, drop shadow offset (1, 2) black 0.75 (the default NotoSans-Regular face).
//! * Bytecode: `UpdateVisibility` shows the ammo page for a `FortWeaponRanged` whose native `ShouldDisplayAmmoCounter()` is true (the pickaxe is neither ranged nor a building tool: the
//!   whole switcher is Collapsed). `UpdateCurrentAmmo(LocalCount, LocalRemaining)`: `InfiniteAmmo = Remaining > 200000000`; `UpdateWidgetColor(Remaining > 0)`; magazine text =
//!   `GetCurrentAmmoText(Count)` (Count < 10: "  {0}", < 100: " {0}", else `Conv_IntToText(Count, grouping, 1 digit)`); magazine opacity 1.0 when Count > 0 else 0.5; total text =
//!   `Max(Remaining - Count, 0)` as `Conv_IntToText` ("∞" when InfiniteAmmo), opacity 1.0 when that is > 0 else 0.5; the icon is `GetItemDefinitionSmallPreviewImageBrush(CurrentAmmoItemDefinition, size 3)`
//!   (size index 3 = 40). `UpdateWidgetColor(HasAmmo)`: "Out of Ammo!" is Visible when !HasAmmo; icon and the three texts are white when HasAmmo, else the stylesheet's `Color-Debuff`
//!   (linear 0.75, 0.09387, 0.07125 = the second colour of the `FortUIStylesheet` literal, member order Buff, Debuff, Unique, Fire, Ice, Bolt, HarvestWeakPoint).
//!
//! NOT RECOVERED / NOT DONE (reported, not substituted): native `ShouldDisplayAmmoCounter`, `GetLocalMagazineAmmoCount` / `GetLocalRemainingAmmo` internals (the caller supplies the numbers),
//! (the -5 degree rotation of "Out of Ammo!" IS applied now: `xform::rotate_quads_since` about the text widget's default 0.5 / 0.5 pivot), the `M_Border_HUD_Line_Vert` material
//! (drawn by the existing `PanelLine` path like the other HUD dividers), and the exact `GetItemDefinitionSmallPreviewImageBrush` implementation (taken to be the definition's `SmallPreviewImage`).
use crate::draw::{MaterialKind, RenderList, TextureId};
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::{art, atlas_px, ascent, line_height, mat_tex, put_text, text_width, Snap, WHITE};
use crate::text::FontId;

pub const BOTTOM_PAD: f32 = 160.; // OverlaySlot_10 Padding.Bottom (shared with equip_progress)
pub const FONT_SIZE: f32 = 32.; // TextStyle-Base-L-S Size
pub const SHADOW: ([f32; 2], [f32; 4]) = ([1., 2.], [0., 0., 0., 0.75]); // ShadowOffset, ShadowColor
pub const ICON: f32 = 40.; // AmmoIcon brush ImageSize
pub const ICON_PAD: (f32, f32) = (4., 8.); // HorizontalBoxSlot_3 padding Left / Top
pub const MAG_PAD_LEFT: f32 = 8.; // HorizontalBoxSlot_329
pub const DIVIDER_PAD: f32 = 8.; // HorizontalBoxSlot_1 padding Left = Right
pub const DIVIDER_W: f32 = 4.; // SizeBox_3 WidthOverride
pub const DIVIDER_H: f32 = 36.; // SizeBox_3 HeightOverride
pub const SPACER: (f32, f32) = (24., 18.); // SizeBox_6 HeightOverride, slot Padding.Top (zero width: no WidthOverride)
pub const LINE_VERT_ALPHA: f32 = 0.2; // M_Border_HUD_Line_Vert Global Alpha (as the other HUD dividers)
pub const OUT_OF_AMMO: &str = "Out of Ammo!";
pub const OUT_PAD: (f32, f32) = (4., 40.); // OverlaySlot_111 padding Left / Bottom
pub const OUT_TRANSLATE_Y: f32 = 24.; // TextAmmoOutOverlay RenderTransform translation
pub const OUT_ANGLE_DEG: f32 = -5.; // TextAmmoOutOverlay RenderTransform Angle (degrees, UMG: positive = clockwise)
pub const OUT_FONT: f32 = 18.; // TextStyle-Base-S size (the -B-S variant only adds bold + shadow)
pub const INFINITE: i32 = 200_000_000; // UpdateCurrentAmmo: Greater_IntInt(LocalRemaining, 200000000)
/// `FortUIStylesheet.Color-Debuff` (linear).
pub const DEBUFF: [f32; 3] = [0.75, 0.09387, 0.07125];
pub const NO_AMMO_OPACITY: f32 = 0.5;

/// What `UpdateCurrentAmmo` receives, plus the ammo definition's icon file.
#[derive(Clone, Debug, PartialEq)]
pub struct EquippedAmmo {
    /// `LocalCount`: rounds in the magazine (`GetLocalMagazineAmmoCount`).
    pub magazine: i32,
    /// `LocalRemaining`: magazine + reserve (`GetLocalRemainingAmmo`).
    pub remaining: i32,
    /// PNG name of the ammo item definition's `SmallPreviewImage` (e.g. `T-Items-AmmoMedium.png`).
    pub icon: String,
}

/// `GetCurrentAmmoText`.
pub fn magazine_text(count: i32) -> String {
    if count < 10 { format!("  {count}") } else if count < 100 { format!(" {count}") } else { int_text(count) }
}
/// `Conv_IntToText(v, bAlwaysSign = false?, grouping = true)`: "1,234".
pub fn int_text(v: i32) -> String {
    let n = v.max(0) as u64; let s = n.to_string(); let mut out = String::new();
    for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); } out.push(c); }
    out
}
/// `TextTotalAmmoCount` text: `Max(Remaining - Count, 0)`, or "∞" while `InfiniteAmmo`.
pub fn total_text(a: &EquippedAmmo) -> String { if a.remaining > INFINITE { "\u{221E}".to_owned() } else { int_text((a.remaining - a.magazine).max(0)) } }
/// `HasAmmo` of `UpdateWidgetColor`.
pub fn has_ammo(a: &EquippedAmmo) -> bool { a.remaining > 0 }

/// Resolved geometry in pixels (exposed for tests): (widget rect, mag text x, divider rect, total text x, icon rect, row rect).
#[derive(Clone, Copy, Debug)]
pub struct Geometry { pub widget: Rect, pub mag_x: f32, pub divider: Rect, pub total_x: f32, pub icon: Rect, pub row: Rect }
pub fn geometry(w: f32, h: f32, mag_w: f32, total_w: f32, line_h: f32) -> Geometry {
    let s = dpi_scale(w, h);
    let row_h = line_h.max(DIVIDER_H * s);
    let b_w = (MAG_PAD_LEFT * s + mag_w) + (2. * DIVIDER_PAD + DIVIDER_W) * s + total_w;
    let c_w = (ICON_PAD.0 + ICON) * s;
    let total_w_all = b_w + c_w;
    let total_h = (SPACER.0 + SPACER.1).max(0.) * s;
    let total_h = total_h.max(row_h).max((ICON_PAD.1 + ICON) * s);
    let x0 = ((w - total_w_all) * 0.5).round();
    let top = (h - BOTTOM_PAD * s - total_h).round();
    let row = Rect::new(x0, top, b_w, row_h);
    let mag_x = x0 + MAG_PAD_LEFT * s;
    let div_x = mag_x + mag_w + DIVIDER_PAD * s;
    let divider = Rect::new(div_x, top + (row_h - DIVIDER_H * s) * 0.5, DIVIDER_W * s, DIVIDER_H * s);
    let total_x = div_x + (DIVIDER_W + DIVIDER_PAD) * s;
    let icon = Rect::new(x0 + b_w + ICON_PAD.0 * s, top + ICON_PAD.1 * s, ICON * s, ICON * s);
    Geometry { widget: Rect::new(x0, top, total_w_all, total_h), mag_x, divider, total_x, icon, row }
}

/// Draw the counter for a held firearm. Returns the names of art the widget needed but the asset source lacked.
pub fn build(ctx: &mut UiContext, w: f32, h: f32, a: &EquippedAmmo, out: &mut RenderList) -> Vec<String> {
    let s = dpi_scale(w, h);
    let px = atlas_px(FONT_SIZE, s);
    let (mag, total) = (magazine_text(a.magazine), total_text(a));
    let (mag_w, total_w) = (text_width(ctx, FontId::Regular, &mag, px), text_width(ctx, FontId::Regular, &total, px));
    let lh = line_height(ctx, FontId::Regular, px);
    let g = geometry(w, h, mag_w, total_w, lh);
    let ok = has_ammo(a);
    let tint = if ok { WHITE } else { [crate::screens::srgb(DEBUFF[0]), crate::screens::srgb(DEBUFF[1]), crate::screens::srgb(DEBUFF[2]), 1.] };
    let mut missing = vec![];
    let base = (g.row.y + (g.row.h - lh) * 0.5 + ascent(ctx, FontId::Regular, px)).round();
    let shadow = Some(([SHADOW.0[0] * s, SHADOW.0[1] * s], SHADOW.1));
    let with_alpha = |c: [f32; 4], o: f32| [c[0], c[1], c[2], c[3] * o];
    put_text(ctx, out, FontId::Regular, px, g.mag_x, base, &mag, with_alpha(tint, if a.magazine > 0 { 1. } else { NO_AMMO_OPACITY }), shadow.map(|(o, c)| (o, with_alpha(c, if a.magazine > 0 { 1. } else { NO_AMMO_OPACITY }))), Snap::Pen);
    let line = mat_tex(ctx, "T-Border-HUD-Line-Vert.png", false);
    if line.id != TextureId::NONE { out.material(MaterialKind::PanelLine { mask: line.id, global_alpha: LINE_VERT_ALPHA }, g.divider, WHITE, false); } else { missing.push("T-Border-HUD-Line-Vert.png".to_owned()); }
    let total_o = if (a.remaining - a.magazine).max(0) > 0 || a.remaining > INFINITE { 1. } else { NO_AMMO_OPACITY };
    put_text(ctx, out, FontId::Regular, px, g.total_x, base, &total, with_alpha(tint, total_o), shadow.map(|(o, c)| (o, with_alpha(c, total_o))), Snap::Pen);
    let icon = art(ctx, &a.icon);
    if icon.id != TextureId::NONE { out.sprite(icon.id, g.icon, tint); } else { missing.push(a.icon.clone()); }
    if !ok {
        // TextAmmoOutOverlay (TextStyle-Base-S-B-S): Overlay_28 slot padding Left 4 / Bottom 40, VAlign Bottom, translated down by 24 (the -5 degree rotation is not drawn).
        let opx = atlas_px(OUT_FONT, s);
        let (ow, olh) = (text_width(ctx, FontId::Bold, OUT_OF_AMMO, opx), line_height(ctx, FontId::Bold, opx));
        let bottom = g.widget.y + g.widget.h - OUT_PAD.1 * s + OUT_TRANSLATE_Y * s;
        let ob = (bottom - olh + ascent(ctx, FontId::Bold, opx)).round();
        let (ox, start) = (g.widget.x + OUT_PAD.0 * s, out.cmds.len());
        put_text(ctx, out, FontId::Bold, opx, ox, ob, OUT_OF_AMMO, tint, shadow, Snap::Pen);
        // RenderTransform Angle -5 about the default pivot (0.5, 0.5) of the text widget (its own box ow x olh, already translated by (0, 24)); WIDGETS3
        crate::screens::xform::rotate_quads_since(out, start, (ox + ow * 0.5, bottom - olh * 0.5), OUT_ANGLE_DEG);
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ammo(magazine: i32, remaining: i32) -> EquippedAmmo { EquippedAmmo { magazine, remaining, icon: "T-Items-AmmoMedium.png".into() } }
    #[test] fn magazine_text_pads_like_getcurrentammotext() {
        assert_eq!(magazine_text(7), "  7"); assert_eq!(magazine_text(30), " 30"); assert_eq!(magazine_text(100), "100"); assert_eq!(magazine_text(1234), "1,234");
    }
    #[test] fn total_is_remaining_minus_magazine_clamped_and_infinite_above_200_million() {
        assert_eq!(total_text(&ammo(24, 144)), "120"); assert_eq!(total_text(&ammo(30, 30)), "0"); assert_eq!(total_text(&ammo(5, 3)), "0"); assert_eq!(total_text(&ammo(30, 200_000_001)), "\u{221E}");
        assert_eq!(int_text(1_000_000), "1,000,000");
    }
    #[test] fn build_draws_the_counts_divider_and_out_of_ammo_only_when_dry() {
        use crate::screens::testutil::ctx;
        let Some(mut c) = ctx() else { return };
        let mut out = RenderList::new(); build(&mut c, 1280., 720., &ammo(24, 144), &mut out); let n = out.cmds.len(); assert!(n > 0, "text quads were emitted");
        let mut dry = RenderList::new(); build(&mut c, 1280., 720., &ammo(0, 0), &mut dry); assert!(dry.cmds.len() > n, "the dry state adds the Out of Ammo! text");
    }    #[test] fn has_ammo_is_remaining_above_zero() { assert!(has_ammo(&ammo(0, 1))); assert!(!has_ammo(&ammo(0, 0))); }
    #[test] fn geometry_is_centred_and_sits_160_units_above_the_bottom() {
        let g = geometry(1920., 1080., 60., 90., 43.); let s = dpi_scale(1920., 1080.);
        assert!((g.widget.x + g.widget.w * 0.5 - 960.).abs() <= 1.0, "{:?}", g.widget);
        assert!((g.widget.y + g.widget.h - (1080. - 160. * s)).abs() <= 1.0);
        assert!(g.mag_x < g.divider.x && g.divider.x + g.divider.w < g.total_x && g.total_x < g.icon.x, "mag | divider | total | icon left to right");
        assert!((g.divider.w - 4. * s).abs() < 1e-3 && (g.divider.h - 36. * s).abs() < 1e-3 && (g.icon.w - 40. * s).abs() < 1e-3);
        assert!((g.icon.y - (g.widget.y + 8. * s)).abs() < 1e-3, "HorizontalBoxSlot_3 padding Top 8");
    }
}
