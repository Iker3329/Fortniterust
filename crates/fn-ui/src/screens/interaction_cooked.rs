//! PICKUP CARD: the interaction prompt of a pickup drawn from the COOKED widget trees, nothing recreated.
//!
//! `InteractionIndicator_C` (`/Game/UI/InGame/HUD/InteractionIndicator`) is composed with the generic UMG runtime of `athena_lobby` (`umg::Ui`): root
//! `CommonBorder_0` (`Border-KeyBase-Center-M`) > `Overlay_0` > `HorizontalBox_0` (padding 5) [VerticalBox_0 { `KeybindWidget` (Use, Medium),
//! HorizontalBox_1 { Spacer 8x1, `TextUnderKeybindText` (TextBlock, Roboto Bold 18) } } (Left / Top)] [Spacer_0 8x1] [`HUD-PickupItemWidget` (Left / Center)]
//! [`BasicInteractionWidget`] [`Interaction_DefenderBeacon`] [Spacer_1 8x1]. `HUD-PickupItemWidget_C`: VerticalBox_0 [`SizeBoxInteractionFailure` (pad top 8,
//! width 320, Collapsed) > CommonBorder_0 > { `ImageBackpack` (T-Icon-Backpack-32, red), `TextInteractionFailure` (TextStyle-Base-XS-B-S-Red) }]
//! [SizeBox_0 (MinDesiredWidth 400) > `ItemDetailsHeader` (`Show Ammo Header` true)] — the header is filled by `item_header`.
//!
//! The Blueprint behaviour (Kismet, `fn-inspect disasm`):
//! * `InteractionIndicator.HandleInteractionChanged`: self Visible / Collapsed by the interaction; the Pickup / Basic / DefenderBeacon children Collapsed; a
//!   `FortPickup` with `bUsePickupWidget` -> `ShowPickupWidget(pickup)`: `HUD-PickupItemWidget` SelfHitTestInvisible + `SetPickup`, `KeybindWidget` Collapsed when
//!   `GetInteractErrorText` returns true else SelfHitTestInvisible (`SetForcedHoldKeybindStatus(0)`), `TextUnderKeybindText.SetText(GetInteractText)`,
//!   SelfHitTestInvisible when `GetInteractText` returns false (it returns false when it wrote a text) else Collapsed.
//! * `AFortPickup::GetInteractText` (exe 0x140686d60): "Pick up" (`Fort.Pickup.Text` / `PickUpText`) or "Swap\nequipped" (`SwapEquippedText`), returns 0
//!   with the text when the pickup can be interacted with, else 1 (no text). `GetInteractErrorText` (0x140686b00): "Can't pickup this item!",
//!   "Can't hold more of this item!", "Backpack full!", "Pack belongs to another player!", returns 1 with the text when refused.
//! * `HUD-PickupItemWidget.InitializeItemData`: `PickupItem = Pickup.PrimaryPickupDummyItem`; `ItemDetailsHeader` SelfHitTestInvisible when valid else Collapsed,
//!   `SetItemToDetail(PickupItem)`. `InitializeInteractionFailureText`: error -> `SizeBoxInteractionFailure` SelfHitTestInvisible + the text, `ImageBackpack`
//!   SelfHitTestInvisible when `GetBackpackItemCounts` items >= max else Collapsed; no error -> both Collapsed.
//!
//! The native `KeybindWidget` keycap (frame + key text, `UpdateFrameSize` Medium) is laid out as a box of its size and painted by `interaction` (the same
//! numbers as the basic prompt). NOT RECOVERED: the placement pivot (as `interaction`: the panel centre on anchor + `ScreenSpaceRelativeOffset` (65, -65)).
use super::athena_lobby::layout::{self as ul, Ctx};
use super::athena_lobby::paint::{Painter, Tf};
use super::athena_lobby::slate_mat::SlateMat;
use super::athena_lobby::store::Store;
use super::athena_lobby::umg::{Ui, Vis};
use super::interaction as basic;
use super::item_header::{loc, DetailsW, HeaderData};
use crate::draw::RenderList;
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::inventory_panel::InvItem;
use crate::screens::{art, ascent, atlas_px, line_height, put_text, text_width, Snap, WHITE};
use crate::text::FontId;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub const INDICATOR_CLASS: &str = "/Game/UI/InGame/HUD/InteractionIndicator.InteractionIndicator_C";

/// What `ShowPickupWidget` reads from the focused `FortPickup`.
#[derive(Clone, Debug, PartialEq)]
pub struct PickupCard {
    /// `PrimaryPickupDummyItem` (definition texts, rarity, type, stack count).
    pub item: InvItem,
    /// `GetInteractText`: "Pick up" / "Swap\nequipped"; None when the pickup cannot be interacted with.
    pub action: Option<String>,
    /// `GetInteractErrorText` when the pickup is refused.
    pub error: Option<String>,
    /// `FortInventoryContext.GetBackpackItemCounts`: items >= max (the red backpack icon of the failure row).
    pub backpack_full: bool,
}

pub struct CookedInteraction {
    ui: Ui,
    root: usize,
    keybind: Option<usize>,
    text_under: Option<usize>,
    pickup: Option<usize>,
    basic: Option<usize>,
    beacon: Option<usize>,
    header_root: Option<usize>,
    failure_box: Option<usize>,
    failure_text: Option<usize>,
    backpack: Option<usize>,
    details: DetailsW,
    hdr: HeaderData,
    mats: HashMap<String, Option<Rc<SlateMat>>>,
    /// assets the painter could not resolve (reported once each)
    pub missing: Vec<String>,
}

impl CookedInteraction {
    pub fn load(install: &Path) -> Result<CookedInteraction, String> {
        let store = Rc::new(Store::open(install)?);
        let mut ui = Ui::new(store.clone());
        let root = ui.add_class_instance(INDICATOR_CLASS, None, "", json!({}), None).ok_or("cannot compose InteractionIndicator_C")?;
        let v = |ui: &Ui, owner: usize, n: &str| ui.var(owner, n).or_else(|| ui.find_in(owner, n));
        let pickup = v(&ui, root, "HUD-PickupItemWidget");
        let pv = |ui: &Ui, n: &str| pickup.and_then(|p| v(ui, p, n));
        let (header_root, failure_box, failure_text, backpack) = (pv(&ui, "ItemDetailsHeader"), pv(&ui, "SizeBoxInteractionFailure"), pv(&ui, "TextInteractionFailure"), pv(&ui, "ImageBackpack"));
        let details = DetailsW::find(&mut ui, pickup, "ItemDetailsHeader");
        let keybind = v(&ui, root, "KeybindWidget");
        // the native keycap is a box of its size (set per frame in `apply`), painted over its geometry
        if let Some(k) = keybind { ui.w[k].class = "Spacer".into(); ui.w[k].props["Size"] = json!([basic::KEYCAP_MIN, basic::KEYCAP_MIN]); }
        // its composed children (frame, `{K}` text, hold ring) are replaced by that painting
        if let Some(k) = keybind { for i in 0..ui.w.len() { if i != k && ui.is_descendant(i, k) { ui.w[i].vis = Vis::Collapsed; } } }
        let hdr = HeaderData::load(&store);
        let (text_under, basic, beacon) = (v(&ui, root, "TextUnderKeybindText"), v(&ui, root, "BasicInteractionWidget"), v(&ui, root, "Interaction_DefenderBeacon"));
        Ok(CookedInteraction { ui, root, keybind, text_under, pickup, basic, beacon,
            header_root, failure_box, failure_text, backpack, details, hdr, mats: HashMap::new(), missing: vec![] })
    }

    fn set_vis(&mut self, id: Option<usize>, v: Vis) { if let Some(i) = id { self.ui.w[i].vis = v; } }
    fn set_text(&mut self, id: Option<usize>, t: &str) { if let Some(i) = id { self.ui.w[i].text = Some(t.to_owned()); } }

    /// `HandleInteractionChanged` -> `ShowPickupWidget` -> `SetPickup` for this frame's pickup.
    fn apply(&mut self, uic: &mut UiContext, key: &str, card: &PickupCard, s: f32) {
        let store = self.ui.store.clone();
        self.ui.w[self.root].vis = Vis::SelfHitTestInvisible;
        let (b, d) = (self.basic, self.beacon);
        self.set_vis(b, Vis::Collapsed); self.set_vis(d, Vis::Collapsed);
        let p = self.pickup;
        self.set_vis(p, Vis::SelfHitTestInvisible);
        // KeybindWidget: Collapsed on an error; the keycap box = max(text + 2 x 3.2, 32) square-ish (Medium), Slate units
        let k = self.keybind;
        self.set_vis(k, if card.error.is_some() { Vis::Collapsed } else { Vis::SelfHitTestInvisible });
        if let Some(k) = k {
            let kpx = atlas_px(basic::KEYCAP_FONT, s);
            let kw = if crate::screens::target_info::key_icon(key).is_some() { 0. } else { text_width(uic, FontId::Bold, &crate::screens::target_info::hud_key_text(key), kpx) / s };
            let kh = line_height(uic, FontId::Bold, kpx) / s;
            self.ui.w[k].props["Size"] = json!([(kw + 2. * basic::KEYCAP_PAD).max(basic::KEYCAP_MIN), (kh + 2. * basic::KEYCAP_PAD).max(basic::KEYCAP_MIN)]);
        }
        // TextUnderKeybindText = GetInteractText (localized `Fort.Pickup.Text`)
        let tu = self.text_under;
        match &card.action {
            Some(a) => {
                let t = if a.starts_with("Swap") { loc(&store, "Fort.Pickup.Text", "SwapEquippedText", "Swap\nequipped") } else { loc(&store, "Fort.Pickup.Text", "PickUpText", "Pick up") };
                self.set_text(tu, &t); self.set_vis(tu, Vis::SelfHitTestInvisible);
            }
            None => self.set_vis(tu, Vis::Collapsed),
        }
        // HUD-PickupItemWidget.InitializeInteractionFailureText / InitializeItemData
        let (fb, ft, bp) = (self.failure_box, self.failure_text, self.backpack);
        match &card.error {
            Some(e) => { self.set_vis(fb, Vis::SelfHitTestInvisible); self.set_text(ft, e); self.set_vis(bp, if card.backpack_full { Vis::SelfHitTestInvisible } else { Vis::Collapsed }); }
            None => { self.set_vis(bp, Vis::Collapsed); self.set_vis(fb, Vis::Collapsed); }
        }
        let hr = self.header_root;
        self.set_vis(hr, Vis::SelfHitTestInvisible);
        self.details.apply(&mut self.ui, &mut self.hdr, &card.item);
    }

    /// Lay out and paint the indicator for `card`, centred on `anchor` (pixels; None = the screen centre) + (65, -65) units.
    pub fn draw(&mut self, uic: &mut UiContext, w: f32, h: f32, key: &str, card: &PickupCard, anchor: Option<(f32, f32)>) -> RenderList {
        let s = dpi_scale(w, h);
        self.apply(uic, key, card, s);
        let a = anchor.unwrap_or((w * 0.5, h * 0.5));
        let (cx, cy) = (a.0 / s + basic::OFFSET.0, a.1 / s + basic::OFFSET.1);
        // two passes (auto-wrapping text uses the width of the previous pass), as `layout::run`, with the root at its desired size
        let n = self.ui.w.len();
        let mut lay = ul::new_layout(n, s);
        for _ in 0..2 {
            let wrap = lay.wrap_w.clone();
            lay = ul::new_layout(n, s);
            lay.wrap_w = wrap;
            let mut c = Ctx { ui: &mut self.ui, uic: &mut *uic, lay };
            let d = c.desired(self.root);
            // whole pixels, as the basic prompt
            let x = ((cx - d[0] * 0.5) * s).round() / s;
            let y = ((cy - d[1] * 0.5) * s).round() / s;
            c.arrange(self.root, [x, y, d[0], d[1]]);
            lay = c.lay;
        }
        let key_rect = self.keybind.filter(|&k| self.ui.w[k].vis.draws()).map(|k| lay.rect[k]);
        let mut p = Painter { c: Ctx { ui: &mut self.ui, uic, lay }, out: RenderList::new(), mats: &mut self.mats, blur: vec![], missing: vec![], scale: s, disabled: false };
        p.paint(self.root, Tf::ID, [1.; 4]);
        self.details.paint_tier(&mut p, &self.hdr);
        // the KeybindWidget keycap: `Keybind-Frame-M` (alpha 0.6) + the key text (NotoSans Bold 15) centred
        if let Some(r) = key_rect {
            // `KeybindWidget_C`: the `Switcher` holding the keycap is HAlign / VAlign Center in its root overlay (`SizeBoxBackplate` Collapsed: not a hold
            // keybind, `ShowHoldBackPlate`), so the keycap keeps its size, centred in the column the VerticalBox slot stretches it to
            let ks = p.c.ui.w[self.keybind.unwrap()].props["Size"].as_array().map(|a| [a[0].as_f64().unwrap_or(0.) as f32, a[1].as_f64().unwrap_or(0.) as f32]).unwrap_or([r[2], r[3]]);
            let kc = Rect::new(((r[0] + (r[2] - ks[0]) * 0.5) * s).round(), ((r[1] + (r[3] - ks[1]) * 0.5) * s).round(), ks[0] * s, ks[1] * s);
            if let Some(icon) = crate::screens::target_info::key_icon(key) { crate::screens::target_info::draw_key_icon(p.c.uic, &mut p.out, icon, kc, s); } else {
            let key = &crate::screens::target_info::hud_key_text(key);
            let frame = art(p.c.uic, basic::FRAME_TEXTURE);
            p.out.nine_slice(frame, kc, [basic::FRAME_MARGIN; 4], s, [1., 1., 1., basic::FRAME_ALPHA]);
            let kpx = atlas_px(basic::KEYCAP_FONT, s);
            let kw = text_width(p.c.uic, FontId::Bold, key, kpx);
            let (lh, asc) = (line_height(p.c.uic, FontId::Bold, kpx), ascent(p.c.uic, FontId::Bold, kpx));
            put_text(p.c.uic, &mut p.out, FontId::Bold, kpx, kc.x + (kc.w - kw) * 0.5, (kc.y + (kc.h - lh) * 0.5 + asc).round(), key, WHITE, None, Snap::PerGlyph);
            }
        }
        let missing = std::mem::take(&mut p.missing);
        let out = std::mem::take(&mut p.out);
        for m in missing { if !self.missing.contains(&m) { eprintln!("[interaction] not drawn: {m}"); self.missing.push(m); } }
        out
    }

    /// Pixel rectangle of a widget by name after the last layout (test helper).
    #[cfg(test)]
    fn rect_of(&mut self, uic: &mut UiContext, w: f32, h: f32, id: usize, anchor: (f32, f32)) -> [f32; 4] {
        let s = dpi_scale(w, h);
        let mut c = Ctx { ui: &mut self.ui, uic, lay: ul::new_layout(0, s) };
        c.lay = ul::new_layout(c.ui.w.len(), s);
        let d = c.desired(self.root);
        let (cx, cy) = (anchor.0 / s + basic::OFFSET.0, anchor.1 / s + basic::OFFSET.1);
        c.arrange(self.root, [cx - d[0] * 0.5, cy - d[1] * 0.5, d[0], d[1]]);
        c.lay.rect[id]
    }
}

/// The HUD's holder: the install to compose from and the composed indicator (loaded on first use; a failed load falls back to the basic prompt).
#[derive(Default)]
pub struct PickupPrompt { install: Option<PathBuf>, cooked: Option<Option<CookedInteraction>> }
/// A clone keeps the install and composes its own trees on first use.
impl Clone for PickupPrompt { fn clone(&self) -> Self { PickupPrompt { install: self.install.clone(), cooked: None } } }
impl PickupPrompt {
    pub fn set_install(&mut self, install: PathBuf) { if self.install.as_ref() != Some(&install) { self.install = Some(install); self.cooked = None; } }
    /// CONTAINERS: the install set with [`Self::set_install`] (the hold ring reads its material from the same paks).
    pub fn install(&self) -> Option<PathBuf> { self.install.clone() }
    /// Draw the cooked indicator; false when it is unavailable (no install / the trees could not be composed).
    pub fn build(&mut self, ctx: &mut UiContext, w: f32, h: f32, key: &str, card: &PickupCard, anchor: Option<(f32, f32)>, out: &mut RenderList) -> bool {
        if self.cooked.is_none() {
            let c = match self.install.as_ref() { Some(i) => CookedInteraction::load(i), None => Err("no install set".into()) };
            // the trees' textures come from the paks when the UI's art source lacks them (as the inventory)
            #[cfg(feature = "pak")]
            if c.is_ok() { if let Some(i) = self.install.as_ref() { if let Err(e) = super::inventory_cooked::CookedInventory::attach_paks(ctx, i) { eprintln!("[interaction] pak textures unavailable: {e}"); } } }
            self.cooked = Some(match c { Ok(c) => Some(c), Err(e) => { eprintln!("[interaction] InteractionIndicator not composed (basic prompt drawn): {e}"); None } });
        }
        let Some(Some(ci)) = self.cooked.as_mut() else { return false };
        out.extend(ci.draw(ctx, w, h, key, card, anchor));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn pickup_card_composes_from_the_cooked_trees() {
        let p = crate::pak_assets::default_install();
        if !p.exists() { return; }
        let mut uic = UiContext::from_install(&p, None).expect("ui");
        let mut ci = CookedInteraction::load(&p).expect("compose");
        let wood = InvItem { id: 5, name: "Wood".into(), description: String::new(), rarity: 0, icon: String::new(), count: 660, item_type: 19, equipable: false, is_resource: true, can_be_dropped: true, slot: None, def_path: String::new() };
        let card = PickupCard { item: wood, action: Some("Pick up".into()), error: None, backpack_full: false };
        let out = ci.draw(&mut uic, 1920., 1080., "E", &card, Some((960., 540.)));
        assert!(!out.is_empty());
        let t = |ci: &CookedInteraction, id: Option<usize>| id.and_then(|i| ci.ui.w[i].text.clone()).unwrap_or_default();
        assert_eq!(t(&ci, ci.details.name), "WOOD");
        assert_eq!(t(&ci, ci.details.stack_text), "660");
        assert_eq!(t(&ci, ci.text_under), "Pick up");
        assert!(ci.ui.w[ci.details.stack.unwrap()].vis.draws(), "count > 1 shows the stack counter");
        assert!(!ci.ui.w[ci.details.level_bar.unwrap()].vis.takes_space(), "a world resource has no level bar");
        assert!(!ci.ui.w[ci.failure_box.unwrap()].vis.takes_space());
        // the card is at least SizeBox_0's MinDesiredWidth (400) wide, right of the keycap column
        let (hdr, kb) = (ci.header_root.unwrap(), ci.keybind.unwrap());
        let (hr, kr) = (ci.rect_of(&mut uic, 1920., 1080., hdr, (960., 540.)), ci.rect_of(&mut uic, 1920., 1080., kb, (960., 540.)));
        eprintln!("header {hr:?} keycap {kr:?} missing {:?}", ci.missing);
        assert!(hr[2] >= 400. - 0.01 && hr[0] > kr[0] + kr[2]);
        // refused: the keycap and the text under it go, the failure row shows
        let mut bad = card.clone(); bad.action = None; bad.error = Some("Can't hold more of this item!".into());
        ci.draw(&mut uic, 1920., 1080., "E", &bad, None);
        assert!(!ci.ui.w[kb].vis.takes_space() && !ci.ui.w[ci.text_under.unwrap()].vis.takes_space());
        assert!(ci.ui.w[ci.failure_box.unwrap()].vis.draws());
        assert_eq!(t(&ci, ci.failure_text), "Can't hold more of this item!");
    }
}
