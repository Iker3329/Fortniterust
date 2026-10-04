//! Static ports of the remaining Athena inventory SCREEN decisions, read from the Kismet bytecode of the cooked widgets (`fn-inspect disasm`):
//! `AthenaInventoryPanel` (`PerformEquip`, `RegisterEquip`, `HandleDrop`), `AthenaInventoryEquipSlot` (`TryAndShowDropTarget`, `RefreshItem`), `AthenaInventoryDropSlot`
//! (`Tick`, `OnDrop`) and `AthenaInventoryFortItemTileButton` (`OnDragDetected`, `SetEquipSlotOverlay`). Together with [`crate::recovered_inventory`] (OnDrop quantity,
//! `GetSelectedEquipSlot`) and [`crate::native_inventory`] (`DropItemOnQuickBar`) this is every decision of the screen that lives in Blueprint bytecode.
//! The native context calls (`EquipItem`, `RemoveItemFromQuickBar`, `DropItem`, the quickbar add / swap) are NOT recovered: callers get the decision, not the mutation.
//!
//! `EFortItemType` numbers are the declaration order of the exe's enum name table (`WeaponMelee` 16, `WeaponRanged` 17, `WeaponHarvest` 18, ... `Trap` 14).

pub const ITEM_TYPE_14: u8 = 14;
pub const ITEM_TYPE_WEAPON_MELEE: u8 = 16;
pub const ITEM_TYPE_WEAPON_RANGED: u8 = 17;
pub const ITEM_TYPE_WEAPON_HARVEST: u8 = 18;

/// What `AthenaInventoryPanel.PerformEquip(Item)` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipAction {
    /// `GetType() == 14`: `HandleClose` (leave the inventory) then `FortInventoryContext.EquipItem(Item)`.
    CloseThenEquip,
    /// Slotted item (type != 14): `FortInventoryContext.RemoveItemFromQuickBar(Item)`.
    RemoveFromQuickbar,
    /// Not slotted, using a gamepad: `ProcessSlotAction` (drop the selected item on the focused equip slot via `DropItemOnQuickBar(item, 0, slot)`).
    ProcessSlotAction,
    /// Not slotted, keyboard / mouse: `FortInventoryContext.EquipItem(Item)` (the inventory stays open).
    Equip,
}
/// `PerformEquip` branch order: `NotEqual(GetType, 14)` gate, then `IsSlotted`, then `IsUsingGamepad`.
pub fn perform_equip(item_type: u8, slotted: bool, gamepad: bool) -> EquipAction {
    if item_type == ITEM_TYPE_14 { EquipAction::CloseThenEquip }
    else if slotted { EquipAction::RemoveFromQuickbar }
    else if gamepad { EquipAction::ProcessSlotAction }
    else { EquipAction::Equip }
}
/// `RegisterEquip`: the input action bound to the selected item is `Unequip` while it is slotted (and not type 14), else `Equip`.
pub fn equip_action_name(item_type: u8, slotted: bool) -> &'static str { if item_type != ITEM_TYPE_14 && slotted { "Unequip" } else { "Equip" } }
/// `RegisterEquip` / `UpdateInteractionWidgetVisibility`: the Equip action is only live for an item that `IsEquipable(controller)` and does NOT match the search string "Resource".
pub fn equip_action_enabled(is_equipable: bool, matches_resource: bool) -> bool { is_equipable && !matches_resource }
/// `AthenaInventoryEquipSlot.TryAndShowDropTarget`: while a drag with a `FortItem` payload is running, the slot's `DropTarget` highlight is visible only for item types 16 or 17.
pub fn equip_slot_drop_target_visible(dragging_payload_type: Option<u8>) -> bool { matches!(dragging_payload_type, Some(ITEM_TYPE_WEAPON_MELEE | ITEM_TYPE_WEAPON_RANGED)) }
/// `AthenaInventoryDropSlot.Tick`: the `DropTargetOverlay` is visible for any drag whose payload casts to `FortItem`.
pub fn drop_slot_target_visible(dragging_a_fort_item: bool) -> bool { dragging_a_fort_item }
/// `AthenaInventoryFortItemTileButton.SetEquipSlotOverlay(EquipSlot)`: 1..=5 shows the `EquipSlotBorder` with texture `quickbar_s<EquipSlot>`; anything else collapses it.
pub fn tile_equip_overlay(equip_slot: i32) -> Option<&'static str> {
    match equip_slot { 1 => Some("quickbar_s1"), 2 => Some("quickbar_s2"), 3 => Some("quickbar_s3"), 4 => Some("quickbar_s4"), 5 => Some("quickbar_s5"), _ => None }
}
/// `AthenaInventoryFortItemTileButton.OnDragDetected`: the drag operation carries `SplitTheStack = IsShiftDown(PointerEvent)`.
pub fn tile_drag_split(shift_down: bool) -> bool { shift_down }
/// `AthenaInventoryEquipSlot.RefreshItem`: a valid slotted item shows the card and hides the empty frame, otherwise the opposite.
pub fn equip_slot_shows_card(slotted_item_valid: bool) -> bool { slotted_item_valid }
/// `AthenaInventoryEquipSlot.OnDrop` always returns true once the payload casts to `FortItem` (it calls `DropItemOnQuickBar(item, 0, SlotIndex)` first); any other payload also returns true.
pub fn equip_slot_on_drop_handled() -> bool { true }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn perform_equip_follows_the_bytecode_branch_order() {
        assert_eq!(perform_equip(14, false, false), EquipAction::CloseThenEquip);
        assert_eq!(perform_equip(14, true, true), EquipAction::CloseThenEquip, "the type gate comes first");
        assert_eq!(perform_equip(17, true, false), EquipAction::RemoveFromQuickbar);
        assert_eq!(perform_equip(17, true, true), EquipAction::RemoveFromQuickbar, "slotted wins over the gamepad");
        assert_eq!(perform_equip(17, false, true), EquipAction::ProcessSlotAction);
        assert_eq!(perform_equip(17, false, false), EquipAction::Equip);
    }
    #[test] fn the_bound_action_is_unequip_only_for_slotted_non_14_items() {
        assert_eq!(equip_action_name(17, true), "Unequip"); assert_eq!(equip_action_name(17, false), "Equip"); assert_eq!(equip_action_name(14, true), "Equip");
        assert!(equip_action_enabled(true, false) && !equip_action_enabled(true, true) && !equip_action_enabled(false, false));
    }
    #[test] fn drop_target_highlights() {
        assert!(equip_slot_drop_target_visible(Some(16)) && equip_slot_drop_target_visible(Some(17)));
        assert!(!equip_slot_drop_target_visible(Some(18)) && !equip_slot_drop_target_visible(Some(1)) && !equip_slot_drop_target_visible(None));
        assert!(drop_slot_target_visible(true) && !drop_slot_target_visible(false));
    }
    #[test] fn tile_overlay_is_one_to_five() {
        assert_eq!(tile_equip_overlay(1), Some("quickbar_s1")); assert_eq!(tile_equip_overlay(5), Some("quickbar_s5"));
        assert_eq!(tile_equip_overlay(0), None); assert_eq!(tile_equip_overlay(6), None); assert_eq!(tile_equip_overlay(-1), None);
        assert!(tile_drag_split(true) && !tile_drag_split(false));
    }
}
