//! INVENTORY API: the player's build materials (wood / stone / metal) as the 1.8 inventory holds them, on [`World`] (where harvesting already counts them).
//! Readers / writers: the build HUD and cost checks (`mats`, `spend_mats`), harvesting (`add_mats`), pickups, the inventory screen. See `crates/fn-weapons/INVENTORY_API.md`.
//!
//! SOURCES
//! * DATA, `/Game/Items/ResourcePickups/{Wood,Stone,Metal}ItemData` (`FortResourceItemDefinition`): `MaxStackSize` 999, `bAllowMultipleStacks = false` (one stack per material; the
//!   rule is `fn_weapons::inventory_model::plan_pickup` with `ItemRules::resource(999)`: what does not fit the stack is refused).
//! * DATA, `/Game/Athena/Athena_GameMode` CDO `StartingItems`: the only material granted at spawn is `WoodItemData` with `Count 0`; a fresh player has 0 / 0 / 0.
//! * Spending: INFERENCE (the build-cost check is server code, stripped from the 1.8 client exe): a build goes through only when the stack holds the full cost, the counter never goes
//!   negative, so [`World::spend_mats`] is all-or-nothing. The cost numbers themselves belong to the building code.
use crate::{Resource, World};

/// `MaxStackSize` of `WoodItemData` / `StoneItemData` / `MetalItemData` (the cooked definitions; `fn_pose::loadout_quickbar` test `item_rules_of_the_real_definitions` checks wood).
pub const MAX_MATS: u32 = 999;

impl Resource {
    /// Tier index 0 wood, 1 stone, 2 metal (the order of the HUD resource counters and of `[u32; 3]` mats arrays across the app).
    pub fn tier(self) -> usize { match self { Resource::Wood => 0, Resource::Stone => 1, Resource::Metal => 2 } }
    pub fn from_tier(t: usize) -> Option<Resource> { match t { 0 => Some(Resource::Wood), 1 => Some(Resource::Stone), 2 => Some(Resource::Metal), _ => None } }
}

impl World {
    fn mats_mut(&mut self, r: Resource) -> &mut u32 { match r { Resource::Wood => &mut self.wood, Resource::Stone => &mut self.stone, Resource::Metal => &mut self.metal } }
    /// Materials held of one tier.
    pub fn mats(&self, r: Resource) -> u32 { match r { Resource::Wood => self.wood, Resource::Stone => self.stone, Resource::Metal => self.metal } }
    /// Wood, stone, metal.
    pub fn all_mats(&self) -> [u32; 3] { [self.wood, self.stone, self.metal] }
    /// Adds up to `n` (harvest, pickup); the stack is capped at [`MAX_MATS`]. Returns what was actually added (the rest is refused / stays on the ground).
    pub fn add_mats(&mut self, r: Resource, n: u32) -> u32 {
        let v = self.mats_mut(r);
        let add = MAX_MATS.saturating_sub(*v).min(n);
        *v += add;
        add
    }
    /// Pays `n` of one tier if the stack holds at least `n` (true); otherwise nothing changes (false).
    pub fn spend_mats(&mut self, r: Resource, n: u32) -> bool {
        let v = self.mats_mut(r);
        if *v < n { return false; }
        *v -= n;
        true
    }
    /// Can `n` of one tier be paid?
    pub fn can_afford(&self, r: Resource, n: u32) -> bool { self.mats(r) >= n }
    /// Removes up to `n` (an inventory drop); returns what was removed.
    pub fn take_mats(&mut self, r: Resource, n: u32) -> u32 { let v = self.mats_mut(r); let d = n.min(*v); *v -= d; d }
    /// Sets one tier (development / console only), capped at [`MAX_MATS`].
    pub fn set_mats(&mut self, r: Resource, n: u32) { *self.mats_mut(r) = n.min(MAX_MATS); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn add_caps_at_the_stack_size_and_spend_is_all_or_nothing() {
        let mut w = World::default();
        assert_eq!(w.all_mats(), [0, 0, 0], "a fresh player has no materials (StartingItems: Wood x0)");
        assert_eq!(w.add_mats(Resource::Wood, 30), 30); assert_eq!(w.add_mats(Resource::Wood, 980), 969); assert_eq!(w.mats(Resource::Wood), 999);
        assert_eq!(w.add_mats(Resource::Wood, 1), 0);
        assert!(!w.spend_mats(Resource::Stone, 10)); assert_eq!(w.mats(Resource::Stone), 0);
        assert!(w.spend_mats(Resource::Wood, 10)); assert_eq!(w.mats(Resource::Wood), 989);
        assert_eq!(w.take_mats(Resource::Metal, 5), 0); w.set_mats(Resource::Metal, 5000); assert_eq!(w.mats(Resource::Metal), 999);
        for t in 0..3 { assert_eq!(Resource::from_tier(t).unwrap().tier(), t); }
    }
}
