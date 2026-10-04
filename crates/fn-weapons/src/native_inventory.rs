//! Static control-flow port of FortInventoryContext.DropItemOnQuickBar.
//! Original registration 0x3131850 -> exec 0x3181150 -> body RVA 0x2E0EB60.
//! Evidence: inventory-native-implementations.json. Native callees remain explicit
//! dependencies, NOT guessed item mutations. This is not wired into fn-app.

/// The required native operations, labelled by original RVA. Implementations
/// must recover these callees before using this port as a playable inventory.
pub trait DropBackend {
    type Item: Copy;
    type Player: Copy;
    type Quickbars: Copy;
    fn resolve_player_05ac9c0(&mut self) -> Option<Self::Player>;
    fn player_quickbars_1a88(&mut self, player: Self::Player) -> Option<Self::Quickbars>;
    fn validate_2e56af0(&mut self, item: Self::Item, bar: u8, slot: i32) -> bool;
    /// Native 0x6839E0 fills bar and slot; slot starts at -1.
    fn find_06839e0(&mut self, bars: Self::Quickbars, item: Self::Item) -> (u8, i32);
    /// Original call 0x836130 uses FName QuickbarModified, args 1,0,0.
    fn quickbar_modified_0836130(&mut self, player: Self::Player);
    fn add_067bc90(&mut self, bars: Self::Quickbars, item: Self::Item, bar: u8, slot: i32);
    fn swap_0697cd0(&mut self, bars: Self::Quickbars, old_bar: u8, old_slot: i32, new_bar: u8, new_slot: i32) -> bool;
    /// Native 0x850520 receives the global at original VA 0x1468979B0.
    fn final_notify_0850520(&mut self, player: Self::Player);
}

/// Exact observed branch/call order of body RVA 0x2E0EB60. Not proof of the
/// unknown callbacks' semantics or full inventory behavior parity.
pub fn drop_item_on_quickbar<B: DropBackend>(backend: &mut B, item: Option<B::Item>, bar: u8, slot: i32) -> bool {
    let player = backend.resolve_player_05ac9c0();
    let Some(item) = item else { return false };
    let Some(player) = player else { return false };
    let Some(bars) = backend.player_quickbars_1a88(player) else { return false };
    if !backend.validate_2e56af0(item, bar, slot) { return false; }
    let (old_bar, old_slot) = backend.find_06839e0(bars, item);
    backend.quickbar_modified_0836130(player);
    if old_slot == -1 {
        backend.add_067bc90(bars, item, bar, slot);
    } else {
        // The original compares ONLY the slot integers here, not bar identity.
        // Do not add an intuitive same-bar check absent from this body.
        if old_slot == slot { return false; }
        if !backend.swap_0697cd0(bars, old_bar, old_slot, bar, slot) { return false; }
    }
    backend.final_notify_0850520(player);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Probe { player: bool, bars: bool, allowed: bool, previous: (u8,i32), swapped: bool, calls: Vec<&'static str> }
    impl Default for Probe { fn default() -> Self { Self { player:true,bars:true,allowed:true,previous:(0,-1),swapped:true,calls:vec![] } } }
    impl DropBackend for Probe {
        type Item = u32; type Player = u32; type Quickbars = u32;
        fn resolve_player_05ac9c0(&mut self)->Option<u32>{self.calls.push("player");self.player.then_some(1)}
        fn player_quickbars_1a88(&mut self,_:u32)->Option<u32>{self.calls.push("bars");self.bars.then_some(2)}
        fn validate_2e56af0(&mut self,_:u32,_:u8,_:i32)->bool{self.calls.push("validate");self.allowed}
        fn find_06839e0(&mut self,_:u32,_:u32)->(u8,i32){self.calls.push("find");self.previous}
        fn quickbar_modified_0836130(&mut self,_:u32){self.calls.push("modified")}
        fn add_067bc90(&mut self,_:u32,_:u32,_:u8,_:i32){self.calls.push("add")}
        fn swap_0697cd0(&mut self,_:u32,ob:u8,os:i32,nb:u8,ns:i32)->bool{assert_eq!((ob,os,nb,ns),(0,1,0,2));self.calls.push("swap");self.swapped}
        fn final_notify_0850520(&mut self,_:u32){self.calls.push("notify")}
    }
    #[test] fn fresh_item_adds_before_final_notification() {
        let mut p=Probe::default();assert!(drop_item_on_quickbar(&mut p,Some(3),0,2));
        assert_eq!(p.calls,["player","bars","validate","find","modified","add","notify"]);
    }
    #[test] fn same_slot_returns_false_after_modified_even_on_another_bar() {
        let mut p=Probe{previous:(1,2),..Probe::default()};assert!(!drop_item_on_quickbar(&mut p,Some(3),0,2));
        assert_eq!(p.calls,["player","bars","validate","find","modified"]);
    }
    #[test] fn swaps_require_success_before_final_notification() {
        for swapped in [false,true] {let mut p=Probe{previous:(0,1),swapped,..Probe::default()};
            assert_eq!(drop_item_on_quickbar(&mut p,Some(3),0,2),swapped);
            let mut expected=vec!["player","bars","validate","find","modified","swap"];
            if swapped {expected.push("notify")};assert_eq!(p.calls,expected);
        }
    }
    #[test] fn failed_validation_does_not_find_or_mutate() {
        let mut p=Probe{allowed:false,..Probe::default()};assert!(!drop_item_on_quickbar(&mut p,Some(3),0,2));
        assert_eq!(p.calls,["player","bars","validate"]);
    }
    #[test] fn null_item_still_resolves_player_first() {
        let mut p=Probe::default();assert!(!drop_item_on_quickbar(&mut p,None,0,2));assert_eq!(p.calls,["player"]);
    }
    #[test] fn absent_player_and_quickbars_stop_at_the_original_gates() {
        let mut p=Probe{player:false,..Probe::default()};assert!(!drop_item_on_quickbar(&mut p,Some(3),0,2));assert_eq!(p.calls,["player"]);
        let mut p=Probe{bars:false,..Probe::default()};assert!(!drop_item_on_quickbar(&mut p,Some(3),0,2));assert_eq!(p.calls,["player","bars"]);
    }
}
