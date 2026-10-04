//! `QuickbarSlotCooldown_C` (`/Game/UI/InGame/HUD/QuickbarSlotCooldown`), the cooldown layer of every quickbar slot (`AthenaQuickBarSlot` -> `QuickbarSlot`).
//!
//! SOURCES (fn-inspect `props` / `disasm` of `QuickbarSlotCooldown`, `QuickbarSlot`, `AthenaQuickbarPrimary`, `TextStyle-Base-XS-B-S`; the cooked exe for the enum names):
//! * Placement: `AthenaQuickbarSlot` `Overlay_Content` (the 80 x 80 `Item` SizeBox) = [Selection, Empty, ItemCardMaximized, ItemCardMinimized, **QuickbarSlotCooldown**,
//!   SlotInteraction]; the widget's overlay slot has no overrides (Fill / Fill), so it covers the whole 80 x 80 item box, above the item cards and the selection frame.
//! * Tree: `Overlay_0` = [`SwitcherOpacityOverlay` (CommonWidgetSwitcher, Collapsed until a cooldown starts; page 0 `ImageRadialOpacityLayer` = material `M-HUD-Slot-Cooldown`,
//!   page 1 `ImageRectangularOpacityLayer` = a plain `Image` with ColorAndOpacity (0, 0, 0, 0.5)), `CooldownImage` (120 x 120 brush of `M_Cooldown_Radial`, Fill), `CooldownText`
//!   (CommonTextBlock `TextStyle-Base-XS-B-S` = NotoSans-Bold 16, drop shadow (1.6, 1.6) black 0.75, default text "0:00", Center / Center)].
//! * Bytecode: `Initialize(QuickbarItemDisplayStyle, DesiredCooldownTypesSupported)` stores the style and the supported cooldown types. The slot's style is `ItemDisplayStyle` enumerator 3
//!   of the `/Game/UI/Items/ItemDisplayStyle` user enum (= value 1 "World", the `QuickbarSlot` CDO default; value 3 is "Ability"). `OnCooldownStarted(type)`: style World -> for type 3 or 4
//!   `SwitcherOpacityOverlay.SetActiveWidgetIndex(1)` (the rectangular layer) + `SetVisibility(HitTestInvisible)`; style Ability -> for type 2 or 4 index 0 (the radial layer).
//!   `OnCooldownStopped(type)`: the same types -> `SetVisibility(Hidden)`. `Show Countdown(Show)`: `CooldownText.SetVisibility(Show ? SelfHitTestInvisible : Collapsed)`
//!   (`QuickbarSlot.Maximize` -> true, `Minimize` -> false). `EFortItemCooldownType` = 0 None, 1 AmmoRegeneration, 2 ItemActivation, 3 WeaponReloading, 4 Death (the exe's enum name
//!   table, declaration order).
//! * Which types a slot supports (`AthenaQuickbarPrimary` instance data, `CooldownTypesSupported`): slot 1 = [Death]; slots 2..6 = [Death, WeaponReloading]. The native class only
//!   starts / stops the widget for those types, so on the Athena quickbar the only visible cooldown is the dark rectangle over a RELOADING firearm slot (and Death); the consumable
//!   cooldown of PurpleStuff (a `GE_*Cooldown` effect, `ItemActivation`) is NOT a supported type of any Athena slot and therefore draws nothing there (this is the data, not an omission).
//!
//! NOT RECOVERED (reported, not substituted): the native `UFortQuickbarSlotCooldown` code that (a) writes the `Amount_0-1` scalar of the `CooldownImage` material instance (so the meaning
//! of the value, remaining or elapsed fraction, is unknown, and `Item-CooldownMeter_Material`'s shader was not translated) -> the `CooldownImage` ring is not drawn; (b) formats
//! `CooldownText` (the designer default "0:00" gives the `M:SS` pattern; the rounding of the remaining time is not in the data: whole seconds rounded UP are used here).
use crate::draw::RenderList;
use crate::layout::Rect;
use crate::scene::UiContext;
use crate::screens::{atlas_px, centred_baseline, put_text, text_width, Snap};
use crate::text::FontId;

/// `EFortItemCooldownType` (the exe's enum name table, in declaration order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CooldownKind { None = 0, AmmoRegeneration = 1, ItemActivation = 2, WeaponReloading = 3, Death = 4 }

/// `ItemDisplayStyle` user enum values (display names of NewEnumerator2..8).
pub const STYLE_WORLD: u8 = 1;
pub const STYLE_ABILITY: u8 = 3;
/// `ImageRectangularOpacityLayer` ColorAndOpacity.
pub const RECT_COLOR: [f32; 4] = [0., 0., 0., 0.5];
/// `CooldownText` style `TextStyle-Base-XS-B-S`.
pub const TEXT_SIZE: f32 = 16.;
pub const TEXT_SHADOW: ([f32; 2], [f32; 4]) = ([1.6, 1.6], [0., 0., 0., 0.75]);

/// A running cooldown of the item in one slot (what the native `OnCooldownStarted` .. `OnCooldownStopped` span carries).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotCooldown { pub kind: CooldownKind, pub remaining: f32, pub duration: f32 }

/// `CooldownTypesSupported` of the Athena primary quickbar slot `slot` (0..5).
pub fn supported(slot: usize) -> &'static [CooldownKind] {
    const SLOT1: [CooldownKind; 1] = [CooldownKind::Death];
    const OTHERS: [CooldownKind; 2] = [CooldownKind::Death, CooldownKind::WeaponReloading];
    if slot == 0 { &SLOT1 } else { &OTHERS }
}
/// The page / visibility switch of `OnCooldownStarted` for the slot style: `Some(true)` = the rectangular layer is shown, `Some(false)` = the radial layer, `None` = nothing.
pub fn layer_for(style: u8, kind: CooldownKind) -> Option<bool> {
    match (style, kind) {
        (STYLE_WORLD, CooldownKind::WeaponReloading | CooldownKind::Death) => Some(true),
        (STYLE_ABILITY, CooldownKind::ItemActivation | CooldownKind::Death) => Some(false),
        _ => None,
    }
}
/// Is this cooldown shown on the slot at all (supported by the slot AND handled by the style's switch)?
pub fn visible(slot: usize, style: u8, cd: &SlotCooldown) -> bool { supported(slot).contains(&cd.kind) && layer_for(style, cd.kind).is_some() && cd.remaining > 0. }
/// `CooldownText` content: `M:SS` of the remaining time (whole seconds, rounded up).
pub fn time_text(remaining: f32) -> String { let s = remaining.max(0.).ceil() as u32; format!("{}:{:02}", s / 60, s % 60) }

/// Draw the cooldown over the slot's 80 x 80 item box `item` (the slot's lift already applied by the caller).
pub fn build(ctx: &mut UiContext, out: &mut RenderList, item: Rect, scale: f32, slot: usize, cd: &SlotCooldown) {
    if !visible(slot, STYLE_WORLD, cd) { return; }
    out.fill(item, RECT_COLOR);
    let px = atlas_px(TEXT_SIZE, scale);
    let txt = time_text(cd.remaining);
    let tw = text_width(ctx, FontId::Bold, &txt, px);
    let base = centred_baseline(ctx, FontId::Bold, px, item.y, item.h);
    put_text(ctx, out, FontId::Bold, px, item.x + (item.w - tw) * 0.5, base, &txt, [1., 1., 1., 1.], Some(([TEXT_SHADOW.0[0] * scale, TEXT_SHADOW.0[1] * scale], TEXT_SHADOW.1)), Snap::Pen);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Cmd;
    fn cd(kind: CooldownKind, remaining: f32) -> SlotCooldown { SlotCooldown { kind, remaining, duration: 3. } }
    #[test] fn only_the_supported_types_of_the_athena_slots_show() {
        assert_eq!(supported(0), [CooldownKind::Death]);
        for s in 1..6 { assert_eq!(supported(s), [CooldownKind::Death, CooldownKind::WeaponReloading]); }
        assert!(visible(2, STYLE_WORLD, &cd(CooldownKind::WeaponReloading, 1.)), "a reloading firearm slot");
        assert!(!visible(0, STYLE_WORLD, &cd(CooldownKind::WeaponReloading, 1.)), "slot 1 supports Death only");
        assert!(!visible(4, STYLE_WORLD, &cd(CooldownKind::ItemActivation, 20.)), "the consumable (PurpleStuff) cooldown is not a supported Athena slot type");
        assert!(!visible(2, STYLE_WORLD, &cd(CooldownKind::WeaponReloading, 0.)));
    }
    #[test] fn the_style_switch_is_the_bytecode_chain() {
        assert_eq!(layer_for(STYLE_WORLD, CooldownKind::WeaponReloading), Some(true)); assert_eq!(layer_for(STYLE_WORLD, CooldownKind::Death), Some(true));
        assert_eq!(layer_for(STYLE_WORLD, CooldownKind::ItemActivation), None); assert_eq!(layer_for(STYLE_WORLD, CooldownKind::None), None); assert_eq!(layer_for(STYLE_WORLD, CooldownKind::AmmoRegeneration), None);
        assert_eq!(layer_for(STYLE_ABILITY, CooldownKind::ItemActivation), Some(false)); assert_eq!(layer_for(STYLE_ABILITY, CooldownKind::WeaponReloading), None);
    }
    #[test] fn countdown_text_is_minutes_colon_two_digit_seconds() {
        assert_eq!(time_text(0.), "0:00"); assert_eq!(time_text(2.1), "0:03"); assert_eq!(time_text(59.2), "1:00"); assert_eq!(time_text(65.), "1:05"); assert_eq!(time_text(-1.), "0:00");
    }
    #[test] fn build_draws_the_half_black_rectangle_over_the_item_box_and_the_text() {
        use crate::screens::testutil::ctx;
        let Some(mut c) = ctx() else { return };
        let item = Rect::new(100., 200., 80., 80.);
        let mut out = RenderList::new(); build(&mut c, &mut out, item, 1., 2, &cd(CooldownKind::WeaponReloading, 2.2));
        let Some(Cmd::Quad(q)) = out.cmds.first() else { panic!("rect first") }; assert_eq!(q.bounds(), item); assert_eq!(q.tint, RECT_COLOR);
        assert!(out.quad_count() > 1, "text glyph quads follow");
        let mut none = RenderList::new(); build(&mut c, &mut none, item, 1., 4, &cd(CooldownKind::ItemActivation, 20.)); assert!(none.is_empty());
    }
}
