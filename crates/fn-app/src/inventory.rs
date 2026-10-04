//! WIDGETS3: the live inventory of the app as the inventory screen's view (`fn_ui::screens::inventory_panel`) and the host side of the panel's actions.
//!
//! The state shown is the live HUD / loadout state: the quickbar items (`HudState::items`: rarity, icon, stack counts that `Arms::slot_counts` keeps current), the three build
//! resources (`HudState::resources`, the sim's harvest counters) and the backpack counter; the texts come from the original item definitions (`fn_pose::loadout_inventory`).
//! Item ids: 0 pickaxe, 1..=3 the firearms, 4 the consumable of slot 5, 5..=7 wood / stone / metal (catalog order: starter items 0..4, then pickaxe, wood, stone, metal).
//!
//! The panel's decisions are recovered from the original bytecode (`fn_weapons::recovered_panel`, `recovered_inventory`).
//! `DropItemOnQuickBar` now reaches the live starter quickbar, preserving item identity and doing the native-shaped
//! source/destination swap.  World drops and unequipping into a backpack are still not claimed as implemented because
//! this standalone has no pickup actors or separate backpack storage.
use fn_ui::screens::hud::HudState;
use fn_pose::loadout_inventory::CatalogItem;
use fn_ui::screens::inventory_panel::{InvItem, InventoryPanel, InventoryView, PanelAction};
use fn_weapons::{recovered_inventory, recovered_panel};
use crate::arms::QuickbarItem;

const CATALOG_OF_ID: [usize; 8] = [4, 0, 1, 2, 3, 5, 6, 7];
/// `EFortItemType` of the build resources when no catalog is loaded (WorldResource).
const RESOURCE_TYPE: u8 = 19;

pub struct InventoryUi { pub panel: InventoryPanel, catalog: Vec<CatalogItem>, quickbar: [Option<u32>; 6], pub log: Vec<String> }

impl Default for InventoryUi {
    fn default() -> Self { Self { panel: InventoryPanel::default(), catalog: vec![], quickbar: [Some(0), Some(1), Some(2), Some(3), Some(4), None], log: vec![] } }
}

/// What applying the panel's actions asks of the app.
#[derive(Default, Debug, PartialEq)]
pub struct Applied { pub close: bool, pub lines: Vec<String>, pub quickbar_moves: Vec<(u32, usize)> }

impl InventoryUi {
    pub fn set_catalog(&mut self, c: Vec<CatalogItem>) { self.catalog = c; }
    pub fn sync_quickbar(&mut self, quickbar: &[Option<QuickbarItem>; 6]) {
        self.quickbar = std::array::from_fn(|slot| quickbar[slot].map(QuickbarItem::inventory_id));
    }
    pub fn has_catalog(&self) -> bool { self.catalog.len() >= 8 }

    /// The inventory of this frame from the HUD state.
    pub fn view(&self, st: &HudState) -> InventoryView {
        let cat = |id: usize| self.catalog.get(CATALOG_OF_ID[id]);
        let mut items = vec![];
        for slot in 0..6usize {
            let Some(id) = self.quickbar[slot] else { continue };
            let Some(h) = st.items[slot].as_ref() else { continue };
            let c = cat(id as usize);
            items.push(InvItem {
                id, name: c.map(|c| c.name.trim().to_owned()).unwrap_or_default(), description: c.map(|c| c.description.clone()).unwrap_or_default(),
                rarity: h.rarity, icon: h.icon.clone(), count: h.count.unwrap_or(1), item_type: c.map_or(17, |c| c.item_type),
                equipable: true, is_resource: false, can_be_dropped: true, slot: (slot >= 1).then_some(slot as u8),
            });
        }
        for (k, n) in st.resources.iter().enumerate() {
            if *n == 0 { continue; }
            let id = 5 + k; let c = cat(id);
            items.push(InvItem {
                id: id as u32, name: c.map(|c| c.name.clone()).unwrap_or_default(), description: c.map(|c| c.description.clone()).unwrap_or_default(), rarity: c.map_or(0, |c| c.rarity),
                icon: c.map(|c| c.icon.clone()).unwrap_or_default(), count: *n, item_type: c.map_or(RESOURCE_TYPE, |c| c.item_type), equipable: false, is_resource: true, can_be_dropped: true, slot: None,
            });
        }
        InventoryView { items, backpack: st.backpack }
    }

    /// Resolve the panel's pending actions. Returns whether the screen asked to close plus the log lines (also kept in `self.log`).
    pub fn apply(&mut self) -> Applied {
        let mut a = Applied::default();
        for act in self.panel.take_actions() {
            let find = |id: u32| self.panel.view().items.iter().find(|i| i.id == id).cloned();
            match act {
                PanelAction::Select(id) => a.lines.push(format!("[inventory] select item {id} (AthenaHUDContext.SetSelectedInventoryItem, ItemDetails.SetItemToDetail)")),
                PanelAction::Close => { a.close = true; a.lines.push("[inventory] HandleClose (FortHUDContext.EnterCameraMode)".into()); }
                PanelAction::Equip(id) => {
                    let Some(it) = find(id) else { continue };
                    let d = recovered_panel::perform_equip(it.item_type, it.slot.is_some() || id == 0, false);
                    match d {
                        recovered_panel::EquipAction::CloseThenEquip => { a.close = true; a.lines.push(format!("[inventory] Equip item {id}: HandleClose, then FortInventoryContext.EquipItem (native body not recovered: not applied)")); }
                        recovered_panel::EquipAction::RemoveFromQuickbar => a.lines.push(format!("[inventory] Unequip item {id}: FortInventoryContext.RemoveItemFromQuickBar (native body not recovered, Arms binds items to slots: not applied)")),
                        recovered_panel::EquipAction::ProcessSlotAction => a.lines.push(format!("[inventory] Equip item {id}: ProcessSlotAction (gamepad path, not applied)")),
                        recovered_panel::EquipAction::Equip => a.lines.push(format!("[inventory] Equip item {id}: FortInventoryContext.EquipItem (native body not recovered: not applied)")),
                    }
                }
                PanelAction::DropOnSlot { item, slot } => {
                    let slot = slot as usize;
                    if let Some(from) = self.quickbar.iter().position(|id| *id == Some(item)) {
                        if (1..6).contains(&slot) && item != 0 {
                            self.quickbar.swap(from, slot);
                            a.quickbar_moves.push((item, slot));
                            a.lines.push(format!("[inventory] DropItemOnQuickBar(item {item}, bar 0, slot {slot}): live primary-bar swap {} -> {}", from + 1, slot + 1));
                        }
                    }
                }
                PanelAction::Drop { item, split } => {
                    let Some(it) = find(item) else { continue };
                    let q = recovered_inventory::drag_drop_quantity(true, true, it.can_be_dropped, it.count as i32, split, false);
                    a.lines.push(match q { Some(q) => format!("[inventory] DropItem(item {item}, quantity {q}): native body not recovered (no pickup actors): not applied"), None => format!("[inventory] drop of item {item} refused (CanBeDropped false)") });
                }
            }
        }
        self.log.extend(a.lines.iter().cloned());
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_ui::screens::hud::HudItem;
    fn cat(n: &str, ty: u8) -> CatalogItem { CatalogItem { path: format!("/{n}"), name: n.into(), description: format!("{n} description"), rarity: 0, item_type: ty, max_stack: None, icon: format!("{n}.png"), icon_path: String::new() } }
    fn ui() -> InventoryUi { let mut u = InventoryUi::default(); u.set_catalog(vec![cat("AR", 17), cat("Shotgun", 17), cat("Sniper", 17), cat("Bandage", 17), cat("Pickaxe", 18), cat("Wood", 19), cat("Stone", 19), cat("Metal", 19)]); u }
    fn state() -> HudState { let mut st = HudState::default(); st.items[1] = Some(HudItem::new(0, "ar.png", None)); st.items[4] = Some(HudItem::new(0, "bandage.png", Some(5))); st.resources = [120, 0, 7]; st.backpack = (4, 5); st }

    #[test] fn the_view_is_the_live_quickbar_plus_resources() {
        let u = ui(); let v = u.view(&state());
        let ids: Vec<u32> = v.items.iter().map(|i| i.id).collect(); assert_eq!(ids, [0, 1, 4, 5, 7], "pickaxe, the filled slots, wood and metal (stone is 0)");
        assert_eq!(v.backpack, (4, 5));
        let bandage = v.items.iter().find(|i| i.id == 4).unwrap(); assert_eq!((bandage.count, bandage.slot, bandage.name.as_str()), (5, Some(4), "Bandage"));
        let pick = &v.items[0]; assert_eq!((pick.slot, pick.item_type, pick.name.as_str()), (None, 18, "Pickaxe"), "the pickaxe is not an equip slot");
        let wood = v.items.iter().find(|i| i.id == 5).unwrap(); assert!(wood.is_resource && !wood.equipable && wood.count == 120 && wood.description == "Wood description");
        assert_eq!(v.items.iter().find(|i| i.id == 1).unwrap().slot, Some(1));
    }
    #[test] fn without_a_catalog_the_view_still_shows_the_state() { let v = InventoryUi::default().view(&state()); assert_eq!(v.items.len(), 5); assert!(v.items.iter().all(|i| i.name.is_empty())); }
    #[test] fn actions_resolve_to_the_recovered_decisions_and_never_pretend_native_effects() {
        let mut u = ui(); let v = u.view(&state()); u.panel.set_view(v); u.panel.set_open(true);
        u.panel.select(1); u.panel.key_equip(); let a = u.apply();
        assert!(!a.close && a.lines.iter().any(|l| l.contains("Unequip item 1") && l.contains("not applied")), "a slotted item's Equip action is RemoveItemFromQuickBar: {:?}", a.lines);
        u.panel.select(5); u.panel.key_equip(); assert!(u.apply().lines.is_empty(), "resources have no Equip action");
        u.panel.key_close(); assert!(u.apply().close);
    }
}
