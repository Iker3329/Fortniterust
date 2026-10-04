//! Narrow original quickbar operations, read from 1.8 AMD64 instructions.
//! Evidence privately retained in equip-native{,-details}.json and
//! quickbar-validation-native.json. Not the complete native inventory backend.

pub type Guid = [u8; 16];

/// Original slots are groups of GUIDs, not a single item per slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub items: Vec<Guid>,
    /// Original 24-byte slot's byte at +0x10.
    pub flag_10: bool,
}
impl Slot {
    /// Native 0x67BB00: reject disabled slots and zero GUIDs. Empty slots
    /// grow to one entry; populated slots overwrite ONLY entry zero,
    /// preserving any additional group members. Returns 0 or -1, not bool.
    pub fn set_first_guid(&mut self, guid: Guid) -> i32 {
        if !self.flag_10 || guid == [0; 16] { return -1; }
        if let Some(first) = self.items.first_mut() { *first = guid; }
        else { self.items.push(guid); }
        0
    }
    /// Native 0x677AC0: stable removal of every matching GUID, returning
    /// the number removed. Unlike lookup/insertion, it accepts a zero GUID
    /// and ignores the slot flag. Capacity is retained by the native array.
    pub fn remove_guid(&mut self, guid: Guid) -> usize {
        let before = self.items.len();
        self.items.retain(|entry| *entry != guid);
        before - self.items.len()
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotDefinition {
    /// Original 56-byte definition's byte-enum array at +0x00.
    pub allowed_item_types: Vec<u8>,
    /// Original definition's +0x10 flag. Its wider meaning remains unresolved.
    pub flag_10: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bar {
    pub active_slot: i32,
    /// Native bar +0x04: previous active index.
    pub previous_slot: i32,
    /// Native bar +0x08. Its complete role is not recovered yet.
    pub index_08: i32,
    pub slots: Vec<Slot>,
    pub definitions: Vec<SlotDefinition>,
}
impl Default for Bar {
    // Native AFortQuickBars constructor 0x6782F0 writes ZERO at both
    // +0x388 and +0x418; subsequent initialization/replication is separate.
    fn default() -> Self { Self { active_slot: 0, previous_slot: -1, index_08: -1, slots: vec![], definitions: vec![] } }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Quickbars { pub primary: Bar, pub secondary: Bar }

/// ServerAddItemInternal_Validate (vtable +0x610 -> RVA 0x694DF0):
/// accepts enum < 3 AND signed slot >= -3. Do not reject special negative
/// selectors or substitute the stricter placement bounds from can_place.
pub fn server_add_item_validate(bar: u8, slot: i32) -> bool { bar < 3 && slot >= -3 }
/// ServerActivateSlotInternal_Validate (+0x5E0 -> 0x693DE0): only enum < 2.
/// Slot, blend duration and force flag are not checked by this validator.
pub fn server_activate_slot_validate(bar: u8) -> bool { bar < 2 }

impl Quickbars {
    /// Data mutation fragment of 0x698B20 (0x698C08..0x698C63), ONLY
    /// after native owner-class validation, first-GUID inventory resolution
    /// and item virtual +0x2F8 permission have succeeded. Not an equip API:
    /// callers must still run the original notifications/secondary routing.
    /// None means invalid bounds; Some(false) means selected index unchanged,
    /// but other active bars have still been deactivated by the native loop.
    pub fn apply_validated_active_slot_state(&mut self, bar: u8, slot: i32, force: bool) -> Option<bool> {
        let index=usize::try_from(slot).ok()?;
        if index>=self.validation_bar(bar).slots.len() { return None; }
        // Compare raw enum to each loop index: >=2 selects primary above,
        // yet clears BOTH bars here. Do not normalize this enum to zero.
        for (which,other) in [(0u8,&mut self.primary),(1u8,&mut self.secondary)] {
            if other.active_slot!=-1 && bar!=which {
                other.previous_slot=other.active_slot;
                other.active_slot=-1;
            }
        }
        let selected=if bar==1 { &mut self.secondary } else { &mut self.primary };
        let old=selected.active_slot;
        if old==slot { return Some(false); }
        if force || selected.previous_slot==slot { selected.previous_slot=old; }
        selected.active_slot=slot;
        Some(true)
    }
    /// The final data-selection portion of native 0x68CA30, AFTER owner
    /// class/interface validation. Caller must still perform those native
    /// gates and resolve this GUID through the inventory interface.
    /// Unlike can_place, enum >= 2 is rejected here. No invalid-GUID filter:
    /// the original passes even a zero GUID to the lookup if stored first.
    pub fn first_slotted_guid(&self, bar: u8, slot: i32) -> Option<Guid> {
        let bar = match bar { 0 => &self.primary, 1 => &self.secondary, _ => return None };
        let index = usize::try_from(slot).ok()?;
        bar.slots.get(index)?.items.first().copied()
    }
    /// Native 0x6865D0: first bar whose active index is NOT -1; returns 2
    /// if neither qualifies. No implicit primary default or validity clamp.
    pub fn active_bar(&self) -> u8 {
        if self.primary.active_slot != -1 { 0 }
        else if self.secondary.active_slot != -1 { 1 }
        else { 2 }
    }
    /// Native 0x69A5E0 selects secondary only for byte enum value 1.
    /// Other values select primary in this function, even though other APIs
    /// (such as GetSlottedItem) reject an enum >= 2.
    fn validation_bar(&self, bar: u8) -> &Bar {
        if bar == 1 { &self.secondary } else { &self.primary }
    }
    /// Native 0x69A5E0 plus 0x69A660. `item_type` is the original item's
    /// virtual +0x2C0 result; None represents the native null item pointer.
    /// No invented item-category conversions or default slot definitions.
    pub fn can_place(&self, item_type: Option<u8>, bar: u8, slot: i32, check_flag: bool) -> bool {
        let Some(item_type) = item_type else { return false };
        let Ok(index) = usize::try_from(slot) else { return false };
        let bar = self.validation_bar(bar);
        let Some(slot) = bar.slots.get(index) else { return false };
        if !slot.flag_10 { return false; }
        let Some(definition) = bar.definitions.get(index) else { return false };
        if check_flag && definition.flag_10 { return false; }
        definition.allowed_item_types.contains(&item_type)
    }
}

/// Native 0x683AD0: reject all-zero GUID; scan slots in increasing order,
/// scan ALL GUIDs in each slot, and return the first matching SLOT index.
/// Flags, active index, definitions and inventory existence aren't read here.
pub fn find_slot(bar: &Bar, guid: Guid) -> i32 {
    if guid == [0; 16] { return -1; }
    bar.slots.iter().position(|slot| slot.items.contains(&guid))
        .and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

/// Native player-level EquipItem body 0x824CB0: known bar routing, with
/// unrecovered toggle/equip consequences kept explicit rather than simulated.
pub trait EquipBackend {
    type Item: Copy;
    fn has_quickbars(&mut self) -> bool;
    fn item_type_virtual_2c0(&mut self, item: Self::Item) -> u8;
    fn active_bar_6865d0(&mut self) -> u8;
    fn toggle_bar_83d040(&mut self);
    fn equip_item_67bb80(&mut self, item: Self::Item);
}
pub fn equip_item<B: EquipBackend>(backend: &mut B, item: Option<B::Item>) {
    let Some(item) = item else { return };
    if !backend.has_quickbars() { return; }
    // Original compares the item type to literal 0x0E and converts equality
    // to a boolean byte: requested primary=0 or secondary=1.
    let requested_bar = u8::from(backend.item_type_virtual_2c0(item) == 0x0E);
    if backend.active_bar_6865d0() != requested_bar { backend.toggle_bar_83d040(); }
    // The original re-reads the active bar after toggling. Toggle can fail.
    if backend.active_bar_6865d0() == requested_bar { backend.equip_item_67bb80(item); }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn guid(n: u8) -> Guid { let mut g = [0;16]; g[15] = n; g }
    #[test] fn insertion_overwrites_first_without_discarding_group_members() {
        let mut slot=Slot {items:vec![],flag_10:true};
        assert_eq!(slot.set_first_guid(guid(1)),0);
        assert_eq!(slot.items,[guid(1)]);
        slot.items.extend([guid(2),guid(3)]);
        assert_eq!(slot.set_first_guid(guid(4)),0);
        assert_eq!(slot.items,[guid(4),guid(2),guid(3)]);
        assert_eq!(slot.set_first_guid([0;16]),-1);
        assert_eq!(slot.items,[guid(4),guid(2),guid(3)]);
        slot.flag_10=false;
        assert_eq!(slot.set_first_guid(guid(5)),-1);
        assert_eq!(slot.items,[guid(4),guid(2),guid(3)]);
    }
    #[test] fn removal_is_stable_removes_duplicates_and_accepts_zero() {
        let mut slot=Slot {items:vec![guid(1),guid(2),guid(1),[0;16],guid(3),guid(1)],flag_10:false};
        let capacity=slot.items.capacity();
        assert_eq!(slot.remove_guid(guid(1)),3);
        assert_eq!(slot.items,[guid(2),[0;16],guid(3)]);
        assert_eq!(slot.items.capacity(),capacity);
        assert_eq!(slot.remove_guid([0;16]),1);
        assert_eq!(slot.items,[guid(2),guid(3)]);
        assert_eq!(slot.remove_guid(guid(4)),0);
        slot.items.clear();assert_eq!(slot.remove_guid(guid(1)),0);
    }
    fn populated() -> Quickbars {
        let bar = Bar { active_slot: -1, slots: vec![Slot { items: vec![guid(1), guid(2)], flag_10: true }], definitions: vec![SlotDefinition { allowed_item_types: vec![4, 14], flag_10: false }], ..Bar::default() };
        Quickbars { primary: bar.clone(), secondary: bar }
    }
    #[test] fn active_bar_preserves_original_sentinel_and_priority() {
        let mut b = populated(); assert_eq!(b.active_bar(), 2);
        b.secondary.active_slot=0; assert_eq!(b.active_bar(),1);
        b.primary.active_slot=99; assert_eq!(b.active_bar(),0);
        b.primary.active_slot=-2; assert_eq!(b.active_bar(),0);
    }
    #[test] fn constructor_indices_are_zero_even_with_no_slots() {
        let q=Quickbars::default();
        assert_eq!(q.primary.active_slot,0);assert_eq!(q.secondary.active_slot,0);
        assert_eq!(q.primary.previous_slot,-1);assert_eq!(q.primary.index_08,-1);
        assert_eq!(q.secondary.previous_slot,-1);assert_eq!(q.secondary.index_08,-1);
        assert_eq!(q.active_bar(),0);assert_eq!(q.first_slotted_guid(0,0),None);
    }
    #[test] fn activation_state_switches_bar_before_unchanged_slot_check() {
        let mut q=populated();q.primary.active_slot=0;q.secondary.active_slot=0;
        assert_eq!(q.apply_validated_active_slot_state(0,0,false),Some(false));
        assert_eq!(q.primary.previous_slot,-1);
        assert_eq!(q.secondary.active_slot,-1);assert_eq!(q.secondary.previous_slot,0);
        assert_eq!(q.apply_validated_active_slot_state(1,0,false),Some(true));
        assert_eq!(q.primary.active_slot,-1);assert_eq!(q.primary.previous_slot,0);
        assert_eq!(q.secondary.active_slot,0);assert_eq!(q.secondary.previous_slot,-1);
    }
    #[test] fn activation_previous_index_changes_only_on_force_or_return_to_previous() {
        let mut q=populated();q.primary.slots.resize(3,Slot::default());
        q.primary.active_slot=0;q.primary.previous_slot=2;
        assert_eq!(q.apply_validated_active_slot_state(0,1,false),Some(true));
        assert_eq!(q.primary.previous_slot,2);
        assert_eq!(q.apply_validated_active_slot_state(0,2,false),Some(true));
        assert_eq!(q.primary.previous_slot,1);
        assert_eq!(q.apply_validated_active_slot_state(0,0,true),Some(true));
        assert_eq!(q.primary.previous_slot,2);
        assert_eq!(q.apply_validated_active_slot_state(0,0,true),Some(false));
        assert_eq!(q.primary.previous_slot,2);
    }
    #[test] fn activation_bounds_fail_before_mutation_and_raw_enum_is_not_normalized() {
        let q=populated();
        for slot in [-3,-2,-1,1,i32::MAX] { let mut copy=q.clone();assert_eq!(copy.apply_validated_active_slot_state(0,slot,true),None);assert_eq!(copy,q); }
        for bar in 2..=255 {
            let mut copy=q.clone();copy.primary.active_slot=0;copy.secondary.active_slot=0;
            assert_eq!(copy.apply_validated_active_slot_state(bar,0,false),Some(true));
            assert_eq!(copy.primary.active_slot,0);assert_eq!(copy.primary.previous_slot,-1);
            assert_eq!(copy.secondary.active_slot,-1);assert_eq!(copy.secondary.previous_slot,0);
        }
    }
    #[test] fn original_rpc_validation_keeps_special_signed_selectors() {
        for bar in 0..=255 {
            assert_eq!(server_activate_slot_validate(bar),bar<2);
            for slot in [i32::MIN,-4,-3,-2,-1,0,1,i32::MAX] {
                assert_eq!(server_add_item_validate(bar,slot),bar<3 && slot>=-3);
            }
        }
    }
    #[test] fn find_scans_group_members_and_returns_first_slot_not_guid_index() {
        let mut b=populated().primary;
        b.slots.insert(0,Slot::default());
        b.slots.push(Slot {items:vec![guid(2)],flag_10:false});
        assert_eq!(find_slot(&b,guid(2)),1);
        assert_eq!(find_slot(&b,guid(1)),1);
        assert_eq!(find_slot(&b,guid(3)),-1);
        b.slots[0].items.push([0;16]); assert_eq!(find_slot(&b,[0;16]),-1);
    }
    #[test] fn placement_preserves_bounds_flags_and_type_membership() {
        let mut b=populated();
        assert!(b.can_place(Some(4),0,0,true));
        assert!(!b.can_place(None,0,0,true));
        assert!(!b.can_place(Some(4),0,-1,true));
        assert!(!b.can_place(Some(4),0,1,true));
        assert!(!b.can_place(Some(3),0,0,true));
        b.primary.definitions[0].flag_10=true;
        assert!(!b.can_place(Some(4),0,0,true));
        assert!(b.can_place(Some(4),0,0,false));
        b.primary.slots[0].flag_10=false;
        assert!(!b.can_place(Some(4),0,0,false));
        b.primary.slots[0].flag_10=true; b.primary.definitions.clear();
        assert!(!b.can_place(Some(4),0,0,false));
    }
    #[test] fn placement_enum_handling_matches_this_native_function_not_other_apis() {
        let mut b=populated();b.secondary.definitions[0].allowed_item_types.clear();
        for byte in 0..=255 { assert_eq!(b.can_place(Some(4),byte,0,true),byte!=1); }
    }
    #[test] fn slotted_lookup_uses_first_guid_even_when_zero_and_ignores_slot_flags() {
        let mut b=populated();
        b.primary.slots[0].flag_10=false;
        b.primary.definitions.clear();
        assert_eq!(b.first_slotted_guid(0,0),Some(guid(1)));
        b.primary.slots[0].items.insert(0,[0;16]);
        assert_eq!(b.first_slotted_guid(0,0),Some([0;16]));
        b.primary.slots[0].items.clear();
        assert_eq!(b.first_slotted_guid(0,0),None);
        assert_eq!(b.first_slotted_guid(1,0),Some(guid(1)));
        assert_eq!(b.first_slotted_guid(1,-1),None);
        assert_eq!(b.first_slotted_guid(1,1),None);
        for byte in 2..=255 {assert_eq!(b.first_slotted_guid(byte,0),None);}
    }
    struct Probe { bars: bool, kind:u8, active:u8, after_toggle:u8, calls:Vec<&'static str> }
    impl EquipBackend for Probe {
        type Item = u32;
        fn has_quickbars(&mut self)->bool { self.calls.push("bars");self.bars }
        fn item_type_virtual_2c0(&mut self,_:u32)->u8 { self.calls.push("type");self.kind }
        fn active_bar_6865d0(&mut self)->u8 { self.calls.push("active");self.active }
        fn toggle_bar_83d040(&mut self) {self.calls.push("toggle");self.active=self.after_toggle;}
        fn equip_item_67bb80(&mut self,_:u32) {self.calls.push("equip");}
    }
    fn probe(kind:u8,active:u8,after:u8)->Probe { Probe {bars:true,kind,active,after_toggle:after,calls:vec![]} }
    #[test] fn equip_rechecks_bar_and_does_not_equip_after_failed_toggle() {
        let mut p=probe(4,1,0);equip_item(&mut p,Some(7));
        assert_eq!(p.calls,["bars","type","active","toggle","active","equip"]);
        let mut p=probe(4,1,1);equip_item(&mut p,Some(7));
        assert_eq!(p.calls,["bars","type","active","toggle","active"]);
    }
    #[test] fn equip_native_item_type_routing_covers_every_byte() {
        for kind in 0..=255 {let desired=u8::from(kind==14);let mut p=probe(kind,desired,desired);equip_item(&mut p,Some(7));assert_eq!(p.calls,["bars","type","active","active","equip"]);}
    }
    #[test] fn equip_null_and_missing_quickbars_stop_before_type_lookup() {
        let mut p=probe(4,0,0);equip_item(&mut p,None);assert!(p.calls.is_empty());
        p.bars=false;equip_item(&mut p,Some(7));assert_eq!(p.calls,["bars"]);
    }
}
