//! Original-item hotbar preview. Not firearm/effect execution or inventory parity.
use fn_hud_adapter::HudInputs;
use fn_ui::screens::hud::HudItem;

pub fn inputs() -> HudInputs {
    let mut items: [Option<HudItem>; 6] = Default::default();
    items[0] = Some(HudItem::pickaxe());
    // Icons and rarities verified against the requested original 1.8 WIDs.
    // Gun stacks do not display fabricated magazine ammunition as item counts.
    items[1] = Some(HudItem::new(0, "T-Icon-Weapons-SK-MachineGun-01.png", None));
    items[2] = Some(HudItem::new(0, "T-Icon-Weapons-SK-Crusader.png", None));
    items[3] = Some(HudItem::new(2, "T-Icon-Weapons-Sniper-01.png", None));
    items[4] = Some(HudItem::new(0, "T-Icon-MedBandage.png", Some(5)));
    HudInputs { items: Some(items), selected: 1, backpack: Some((4,5)), ..HudInputs::default() }
}
