//! INVENTORY MODEL: the rules of the 1.8 Battle Royale primary quickbar / backpack as pure decisions (no state of its own beyond what the caller passes in).
//!
//! SOURCES (each rule says whether it is data, exe, or an inference):
//! * DATA, `/Game/Athena/Athena_GameMode` CDO (`bOverrideQuickBars = true`) `QuickBarDefinitions[0]` (primary bar, 9 slot definitions): slot 0 accepts `WeaponHarvest` only; slots 1..=5
//!   accept `WeaponMelee`, `WeaponRanged`, `Gadget`; slot 6 `HeroAbility`, slots 7 / 8 `HomebaseGadget` (nothing in BR ever has those types, the HUD shows the first six).
//!   Nothing is a static slot. Ammo (`Ammo`) and build resources (`WorldResource`) are accepted by NO slot: they only ever live in the backpack side of the inventory.
//! * DATA, item definitions: `MaxStackSize` (wood / stone / metal 999, every ammo type 999, Bandage 15, Medkit 3, Shield Potion 2, Slurp-like `PurpleStuff` 2), pickaxe `bCanBeDropped = false`,
//!   `bAllowMultipleStacks = false`.
//! * EXE, `AFortPlayerController` input binding table (`SlotUp` handler 0x14083ce70, `SlotDown` handler 0x14083ce10 -> 0x140684240 / 0x140684390): the mouse wheel moves the focused slot to the
//!   next / previous slot that holds an item, wrapping around the slot list, stopping when it comes back to where it started ([`cycle_slot`]).

/// `EFortItemType` numbers (declaration order of the exe's enum name table).
pub mod item_type {
    pub const AMMO: u8 = 1;
    pub const GADGET: u8 = 10;
    pub const HERO_ABILITY: u8 = 12;
    pub const HOMEBASE_GADGET: u8 = 11;
    pub const WEAPON_MELEE: u8 = 16;
    pub const WEAPON_RANGED: u8 = 17;
    pub const WEAPON_HARVEST: u8 = 18;
    pub const WORLD_RESOURCE: u8 = 19;
}

/// Slots of the primary quickbar the BR HUD shows (slot 0 = pickaxe, 1..=5 = the weapon slots).
pub const PRIMARY_SLOTS: usize = 6;
/// Slot definitions of the Athena primary bar (`QuickBarDefinitions[0].QuickbarSlots`), including the three BR never fills.
pub const PRIMARY_DEFINITIONS: usize = 9;

/// `AcceptedItemTypes` of primary slot `slot` (Athena game mode data).
pub fn accepted_types(slot: usize) -> &'static [u8] {
    use item_type::*;
    match slot {
        0 => &[WEAPON_HARVEST],
        1..=5 => &[WEAPON_MELEE, WEAPON_RANGED, GADGET],
        6 => &[HERO_ABILITY],
        7 | 8 => &[HOMEBASE_GADGET],
        _ => &[],
    }
}
/// Can an item of `item_type` be put into primary slot `slot`?
pub fn slot_accepts(slot: usize, item_type: u8) -> bool { accepted_types(slot).contains(&item_type) }

/// Mouse-wheel slot cycling (`SlotUp` = `dir` +1, `SlotDown` = -1): the first slot after `focus` in `dir` order, wrapping over the bar's slot list, that holds an item. `None` when no
/// other slot holds one (the native loop returns when it is back at the focused slot).
pub fn cycle_slot(occupied: &[bool], focus: usize, dir: i32) -> Option<usize> {
    let n = occupied.len() as i32;
    if n == 0 || dir == 0 { return None; }
    let step = dir.signum();
    let mut i = (focus as i32 + step).rem_euclid(n);
    while i != focus as i32 {
        if occupied[i as usize] { return Some(i as usize); }
        i = (i + step).rem_euclid(n);
    }
    None
}

// ---------------------------------------------------------------------------------------------------------------------------------------------
// Item definition flags
// ---------------------------------------------------------------------------------------------------------------------------------------------

/// The `UFortItemDefinition` fields the inventory rules read. Values come from the item's cooked properties, falling back to the class constructor defaults recovered from the exe
/// ([`ItemRules::weapon`], [`ItemRules::ammo`], [`ItemRules::resource`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemRules {
    /// `EFortItemType` of the definition.
    pub item_type: u8,
    /// `MaxStackSize` (+0x13C).
    pub max_stack: u32,
    /// `bAllowMultipleStacks` (+0x138 bit 0): more than one stack of the item may exist.
    pub allow_multiple_stacks: bool,
    /// `bInventorySizeLimited` (+0x141): the item's stacks count against the backpack size (`Backpack n / 5`).
    pub size_limited: bool,
    /// `bCanBeDropped` (`UFortWorldItem::CanBeDropped`, definition +0x284 bit 3).
    pub can_be_dropped: bool,
}
impl ItemRules {
    /// `UFortWeaponItemDefinition` / `UFortWeaponRangedItemDefinition` constructors (RVA 0x915f10 sets `bInventorySizeLimited = 1`; the base `UFortItemDefinition` ctor 0x679400 sets
    /// `bAllowMultipleStacks = 1`, `MaxStackSize = 1`; `bCanBeDropped` is on from the `UFortWorldItemDefinition` ctor 0x6a5bc0). Firearms and every healing consumable are ranged weapon
    /// definitions; consumables serialize their own `MaxStackSize` (Bandage 15, Medkit 3, Shield Potion 2).
    pub const fn weapon(max_stack: u32) -> ItemRules { ItemRules { item_type: item_type::WEAPON_RANGED, max_stack, allow_multiple_stacks: true, size_limited: true, can_be_dropped: true } }
    /// `WID_Harvest_Pickaxe_Athena_*`: serialized `bCanBeDropped = false`, `bAllowMultipleStacks = false`, `bInventorySizeLimited = false`.
    pub const fn pickaxe() -> ItemRules { ItemRules { item_type: item_type::WEAPON_HARVEST, max_stack: 1, allow_multiple_stacks: false, size_limited: false, can_be_dropped: false } }
    /// `UFortAmmoItemDefinition` ctor (0x915d60): `bAllowMultipleStacks = 0`, `bInventorySizeLimited = 0`, `MaxStackSize` 0x7FFFFFFF until the data sets it (999 for every BR ammo type).
    pub const fn ammo(max_stack: u32) -> ItemRules { ItemRules { item_type: item_type::AMMO, max_stack, allow_multiple_stacks: false, size_limited: false, can_be_dropped: true } }
    /// `UFortResourceItemDefinition` ctor (0x679d10): `bAllowMultipleStacks = 0`, `bInventorySizeLimited = 0`, item type `WorldResource` (0x13), `MaxStackSize` 999 from the data.
    pub const fn resource(max_stack: u32) -> ItemRules { ItemRules { item_type: item_type::WORLD_RESOURCE, max_stack, allow_multiple_stacks: false, size_limited: false, can_be_dropped: true } }
}

/// Backpack size of a fresh BR player. DATA: `/Game/Athena/MappedEffects/GE_Map_Athena_Inventory_StartingSize` adds a ScalableFloat magnitude of 5.0 (Additive) to
/// `FortHomebaseSet.WorldInventorySizeBonus`; the exe's `GetBackpackItemCounts` reads max / count / overflow (0x686ed0 / 0x687480 / 0x687230).
pub const BACKPACK_START_SIZE: u32 = 5;

/// `Backpack n / max` numerator: every stack whose definition is size limited counts one, slotted or not (exe 0x687480, via `UFortWorldItem::vfunc_137`), plus the overflow count
/// (`AthenaInventoryLimitStatusIndicator.RefreshWorldInventoryStatus` shows `Items + Overflow`).
pub fn backpack_numerator(size_limited_stacks: u32, overflow: u32) -> u32 { size_limited_stacks + overflow }

// ---------------------------------------------------------------------------------------------------------------------------------------------
// Pickup: stacking and slotting
// ---------------------------------------------------------------------------------------------------------------------------------------------

/// Where `AFortQuickBars::ServerAddItemInternal` (slot argument -1, exe 0x693df0) puts an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoSlot {
    /// Pass 1: the first empty slot that accepts the item's type, scanning from slot 0 (or `PreferredQuickbarSlot` when it is in 1..N-1) and wrapping.
    Empty(usize),
    /// Pass 2 (no empty slot): scanning from the focused slot (wrapping), the first occupied slot that accepts the type; its item is replaced. With the pickaxe focused, slot 0 rejects
    /// every weapon, so the scan moves on to slot 1.
    ReplaceFocusedScan(usize),
}
/// `slot_types[i]` = the item type in slot `i` (None = empty). Returns where an item of `item_type` goes, or None when no slot accepts it ("no acceptable slot on any quickbar": the item
/// stays in the backpack unslotted).
pub fn auto_slot(slot_types: &[Option<u8>], focus: usize, item_type: u8, preferred: Option<usize>) -> Option<AutoSlot> {
    let n = slot_types.len();
    if n == 0 { return None; }
    let start = match preferred { Some(p) if p >= 1 && p < n => p, _ => 0 };
    for k in 0..n { let i = (start + k) % n; if slot_types[i].is_none() && slot_accepts(i, item_type) { return Some(AutoSlot::Empty(i)); } }
    for k in 0..n { let i = (focus.min(n - 1) + k) % n; if slot_types[i].is_some() && slot_accepts(i, item_type) { return Some(AutoSlot::ReplaceFocusedScan(i)); } }
    None
}

/// Why a pickup (or part of it) was refused. The texts are the 1.8 strings (`AFortPickup::vfunc_194` -> 0x82c220).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// `EInventoryOverflowReason::ItemStackLimitReached`: "Can't hold more of this item!".
    StackLimit,
    /// `BackpackStackLimitReached`: "Backpack full!".
    BackpackFull,
}
impl Refusal { pub fn text(self) -> &'static str { match self { Refusal::StackLimit => "Can't hold more of this item!", Refusal::BackpackFull => "Backpack full!" } } }

/// What one pickup does, computed from the stacks the player already holds.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PickupPlan {
    /// Amount added to existing stacks of the same definition: (index into the `existing` slice, amount).
    pub merged: Vec<(usize, u32)>,
    /// Sizes of the NEW stacks created for the rest (each at most `MaxStackSize`).
    pub new_stacks: Vec<u32>,
    /// What does not fit and stays on the ground with the pickup.
    pub leftover: u32,
    pub refusal: Option<Refusal>,
}
/// Merge-then-create stacking of `count` of an item (overflow math 0x68a320 / 0x68a7a0): fill the existing stacks of the definition to `MaxStackSize`, then create new stacks while
/// multiple stacks are allowed and (for a size-limited item) the backpack has room (`backpack_free` = free backpack entries). Anything left over is refused.
/// `existing` = the counts of the stacks of this definition already held.
pub fn plan_pickup(rules: &ItemRules, existing: &[u32], count: u32, backpack_free: u32) -> PickupPlan {
    let mut plan = PickupPlan::default();
    let mut left = count;
    let max = rules.max_stack.max(1);
    for (i, c) in existing.iter().enumerate() {
        if left == 0 { break; }
        let take = max.saturating_sub(*c).min(left);
        if take > 0 { plan.merged.push((i, take)); left -= take; }
    }
    let mut free = backpack_free;
    let mut stacks = existing.len();
    while left > 0 {
        if stacks >= 1 && !rules.allow_multiple_stacks { plan.refusal = Some(Refusal::StackLimit); break; }
        if rules.size_limited { if free == 0 { plan.refusal = Some(Refusal::BackpackFull); break; } free -= 1; }
        let take = max.min(left); plan.new_stacks.push(take); left -= take; stacks += 1;
    }
    plan.leftover = left;
    plan
}

// ---------------------------------------------------------------------------------------------------------------------------------------------
// Swap, drop, focus
// ---------------------------------------------------------------------------------------------------------------------------------------------

/// `AFortQuickBars::SwapItems` (exe 0x697cd0): both slots must be in range, and each item must be accepted by its DESTINATION slot (an empty source skips its check). Returns the new
/// focus on success: if the focused slot was `a` it becomes `b`, else if it was `b` it becomes `a`, else unchanged.
pub fn swap_slots(slot_types: &[Option<u8>], focus: usize, a: usize, b: usize) -> Option<usize> {
    if a >= slot_types.len() || b >= slot_types.len() { return None; }
    if let Some(t) = slot_types[a] { if !slot_accepts(b, t) { return None; } }
    if let Some(t) = slot_types[b] { if !slot_accepts(a, t) { return None; } }
    Some(if focus == a { b } else if focus == b { a } else { focus })
}

/// Which slot is focused after the focused slot's item left the bar (dropped, used up, unequipped). NOT RECOVERED: the removal body (`ServerRemoveItemInternal`) is a stripped no-op in
/// the 1.8 client exe. INFERENCE (observed BR behaviour): the player falls back to the previously focused slot when it still holds an item, otherwise to the pickaxe.
pub fn focus_after_removal(occupied: &[bool], previous_focus: Option<usize>, removed: usize) -> usize {
    match previous_focus { Some(p) if p != removed && occupied.get(p).copied().unwrap_or(false) => p, _ => 0 }
}

/// Amount dropped / left behind by `DropItem(item, quantity)`: refused for an item that cannot be dropped, otherwise at most the stack. Returns (dropped, remaining).
pub fn drop_amount(rules: &ItemRules, stack: u32, requested: u32) -> Option<(u32, u32)> {
    if !rules.can_be_dropped || stack == 0 || requested == 0 { return None; }
    let d = requested.min(stack); Some((d, stack - d))
}

#[cfg(test)]
mod tests {
    use super::*;
    const R: Option<u8> = Some(item_type::WEAPON_RANGED);
    const H: Option<u8> = Some(item_type::WEAPON_HARVEST);
    #[test] fn pass_one_takes_the_first_empty_accepting_slot() {
        // pickaxe + AR, rest empty: a weapon goes to slot 2 (slot 0 rejects it, slot 1 is taken)
        assert_eq!(auto_slot(&[H, R, None, None, None, None], 1, item_type::WEAPON_RANGED, None), Some(AutoSlot::Empty(2)));
        assert_eq!(auto_slot(&[H, None, R, None, None, None], 2, item_type::WEAPON_RANGED, None), Some(AutoSlot::Empty(1)), "holes are filled first");
        assert_eq!(auto_slot(&[H, R, R, None, None, None], 0, item_type::WEAPON_RANGED, Some(4)), Some(AutoSlot::Empty(4)), "PreferredQuickbarSlot moves the scan start");
        assert_eq!(auto_slot(&[H, R, R, None, None, None], 0, item_type::WEAPON_RANGED, Some(9)), Some(AutoSlot::Empty(3)), "an out-of-range preference is ignored");
    }
    #[test] fn pass_two_replaces_from_the_focused_slot() {
        let full = [H, R, R, R, R, R];
        assert_eq!(auto_slot(&full, 3, item_type::WEAPON_RANGED, None), Some(AutoSlot::ReplaceFocusedScan(3)));
        assert_eq!(auto_slot(&full, 0, item_type::WEAPON_RANGED, None), Some(AutoSlot::ReplaceFocusedScan(1)), "pickaxe focused: slot 0 rejects the weapon, slot 1 is replaced");
        assert_eq!(auto_slot(&full, 5, item_type::WEAPON_RANGED, None), Some(AutoSlot::ReplaceFocusedScan(5)));
        assert_eq!(auto_slot(&full, 2, item_type::WORLD_RESOURCE, None), None, "resources are never slotted");
        assert_eq!(auto_slot(&[None; 6], 0, item_type::WEAPON_HARVEST, None), Some(AutoSlot::Empty(0)));
    }
    #[test] fn stacks_merge_then_create_up_to_the_backpack() {
        let bandage = ItemRules::weapon(15);
        // 14 held, pick up 5: 1 tops the stack up, 4 become a second stack (multiple stacks are allowed)
        let p = plan_pickup(&bandage, &[14], 5, 3); assert_eq!((p.merged.as_slice(), p.new_stacks.as_slice(), p.leftover, p.refusal), (&[(0, 1)][..], &[4][..], 0, None));
        // backpack full: the remainder stays on the ground
        let p = plan_pickup(&bandage, &[15], 5, 0); assert_eq!((p.leftover, p.refusal), (5, Some(Refusal::BackpackFull)));
        let gun = ItemRules::weapon(1); let p = plan_pickup(&gun, &[], 1, 2); assert_eq!((p.new_stacks.as_slice(), p.leftover), (&[1][..], 0));
        let p = plan_pickup(&gun, &[], 1, 0); assert_eq!(p.refusal, Some(Refusal::BackpackFull));
    }
    #[test] fn ammo_and_resources_are_one_capped_stack_outside_the_backpack() {
        let wood = ItemRules::resource(999);
        let p = plan_pickup(&wood, &[], 50, 0); assert_eq!(p.new_stacks, [50], "no backpack room needed");
        let p = plan_pickup(&wood, &[990], 50, 0); assert_eq!((p.merged.as_slice(), p.leftover, p.refusal), (&[(0, 9)][..], 41, Some(Refusal::StackLimit)), "single stack: the rest is refused");
        let p = plan_pickup(&wood, &[999], 1, 5); assert_eq!((p.leftover, p.refusal), (1, Some(Refusal::StackLimit)));
        assert_eq!(plan_pickup(&ItemRules::ammo(999), &[], 1200, 0).new_stacks, [999], "a fresh ammo stack caps at MaxStackSize");
    }
    #[test] fn swap_checks_the_destination_types_and_moves_the_focus() {
        let bar = [H, R, R, None, R, None];
        assert_eq!(swap_slots(&bar, 1, 1, 3), Some(3), "focus follows the focused item");
        assert_eq!(swap_slots(&bar, 3, 1, 3), Some(1));
        assert_eq!(swap_slots(&bar, 2, 1, 4), Some(2), "unrelated focus stays");
        assert_eq!(swap_slots(&bar, 1, 0, 1), None, "the pickaxe cannot leave slot 0, a weapon cannot enter it");
        assert_eq!(swap_slots(&bar, 1, 1, 7), None);
    }
    #[test] fn drops_respect_can_be_dropped_and_clamp_to_the_stack() {
        assert_eq!(drop_amount(&ItemRules::pickaxe(), 1, 1), None);
        assert_eq!(drop_amount(&ItemRules::resource(999), 120, 60), Some((60, 60)));
        assert_eq!(drop_amount(&ItemRules::weapon(15), 5, 99), Some((5, 0)));
        assert_eq!(drop_amount(&ItemRules::weapon(1), 1, 0), None);
    }
    #[test] fn focus_returns_to_the_previous_slot_or_the_pickaxe() {
        let occ = [true, true, true, false, true, false];
        assert_eq!(focus_after_removal(&occ, Some(4), 2), 4); assert_eq!(focus_after_removal(&occ, Some(3), 2), 0); assert_eq!(focus_after_removal(&occ, None, 2), 0); assert_eq!(focus_after_removal(&occ, Some(2), 2), 0);
    }
    #[test] fn backpack_numerator_adds_overflow() { assert_eq!(backpack_numerator(4, 1), 5); assert_eq!(BACKPACK_START_SIZE, 5); }
    #[test] fn slot_rules_match_the_athena_game_mode_data() {
        assert!(slot_accepts(0, item_type::WEAPON_HARVEST) && !slot_accepts(0, item_type::WEAPON_RANGED));
        for s in 1..=5 { assert!(slot_accepts(s, item_type::WEAPON_RANGED) && slot_accepts(s, item_type::WEAPON_MELEE) && slot_accepts(s, item_type::GADGET)); assert!(!slot_accepts(s, item_type::WEAPON_HARVEST), "the pickaxe only goes to slot 0"); }
        for s in 0..PRIMARY_DEFINITIONS { assert!(!slot_accepts(s, item_type::AMMO) && !slot_accepts(s, item_type::WORLD_RESOURCE), "ammo / resources never take a slot"); }
    }
    #[test] fn wheel_cycles_occupied_slots_with_wrap() {
        let occ = [true, true, false, true, false, false];
        assert_eq!(cycle_slot(&occ, 0, 1), Some(1)); assert_eq!(cycle_slot(&occ, 1, 1), Some(3)); assert_eq!(cycle_slot(&occ, 3, 1), Some(0), "wraps over the empty slots");
        assert_eq!(cycle_slot(&occ, 0, -1), Some(3)); assert_eq!(cycle_slot(&occ, 3, -1), Some(1)); assert_eq!(cycle_slot(&occ, 1, -1), Some(0));
        assert_eq!(cycle_slot(&[true, false, false], 0, 1), None, "nothing else to select");
        assert_eq!(cycle_slot(&occ, 0, 0), None);
    }
}
