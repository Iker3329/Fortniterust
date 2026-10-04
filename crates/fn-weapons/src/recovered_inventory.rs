//! Narrow static ports of original Athena inventory Blueprint decisions.
//! Evidence: inventory-evidence/AthenaInventoryDropSlot.disasm.txt, OnDrop [41],
//! and AthenaInventoryPanel.disasm.txt, GetSelectedEquipSlot [79].
//! Native FortInventoryContext mutation/equip/drop behavior remains unresolved.
//! These helpers do not implement a replacement inventory or weapon system.

/// Original OnDrop: only the expected drag operation with a FortWorldItem
/// payload reaches CanBeDropped. Invalid casts return false. A valid but
/// undroppable item returns the Blueprint's default false output.
pub fn drag_drop_quantity(
    is_athena_drag: bool,
    is_world_item: bool,
    can_be_dropped: bool,
    count: i32,
    split_the_stack: bool,
    shift_down: bool,
) -> Option<i32> {
    if !is_athena_drag || !is_world_item || !can_be_dropped { return None; }
    // UE Divide_IntInt truncates toward zero. Do not substitute ceil-half,
    // add fabricated count validation, or reuse the modal's different rule.
    Some(if split_the_stack || shift_down { (count / 2).max(1) } else { count })
}

/// Original GetSelectedEquipSlot loops EquipSlot1..5, breaks at the first
/// selected widget, and returns array index + 1; none selected returns -1.
/// Slot 0 is not represented among these five inventory equip widgets.
pub fn selected_equip_slot(selected: [bool; 5]) -> i32 {
    selected.iter().position(|s| *s).map_or(-1, |i| i as i32 + 1)
}

/// Original CreateQuantitySelectorWidget: initial selection = count / 2.
/// Unlike drag-to-drop, this Blueprint does not call Max(..., 1).
pub fn drop_modal_initial_quantity(count: i32) -> i32 { count / 2 }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drag_drop_respects_both_casts_and_drop_permission() {
        for a in [false, true] { for w in [false, true] { for d in [false, true] {
            assert_eq!(drag_drop_quantity(a, w, d, 7, false, false),
                if a && w && d { Some(7) } else { None });
        } } }
    }
    #[test]
    fn split_flag_and_shift_each_request_floor_half_with_minimum_one() {
        for (split, shift) in [(true, false), (false, true), (true, true)] {
            for (count, expected) in [(1, 1), (2, 1), (3, 1), (5, 2), (999, 499)] {
                assert_eq!(drag_drop_quantity(true, true, true, count, split, shift), Some(expected));
            }
        }
    }
    #[test]
    fn modal_does_not_inherit_drag_drop_minimum() {
        assert_eq!(drop_modal_initial_quantity(1), 0);
        assert_eq!(drop_modal_initial_quantity(5), 2);
        assert_eq!(drop_modal_initial_quantity(-5), -2);
    }
    #[test]
    fn equip_widget_indices_are_one_based_first_selected_wins() {
        assert_eq!(selected_equip_slot([false; 5]), -1);
        for i in 0..5 { let mut slots = [false; 5]; slots[i] = true;
            assert_eq!(selected_equip_slot(slots), i as i32 + 1); }
        assert_eq!(selected_equip_slot([false, true, true, false, false]), 2);
    }
}
