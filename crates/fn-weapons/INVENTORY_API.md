# Inventory API (materials, ammo)

Owner: inventory agent. Other agents should call these functions. Do not write to the raw fields directly.

## Build materials (wood / stone / metal)

The materials are stored on `fn_combat::World` (`sim.world`), next to where harvesting counts them. The functions are in `crates/fn-combat/src/mats.rs`:

```rust
use fn_combat::Resource;                          // Wood, Stone, Metal; Resource::tier() = 0 / 1 / 2, Resource::from_tier(t)
sim.world.mats(Resource::Wood) -> u32             // held amount of one tier
sim.world.all_mats() -> [u32; 3]                  // wood, stone, metal (the HUD / inventory order)
sim.world.can_afford(Resource::Stone, 10) -> bool
sim.world.spend_mats(Resource::Stone, 10) -> bool // all-or-nothing: false = not enough, and nothing changes
sim.world.add_mats(Resource::Wood, n) -> u32      // harvest / pickup: capped at MAX_MATS (999, the cooked MaxStackSize); returns the amount actually added
sim.world.take_mats(Resource::Metal, n) -> u32    // inventory drop: removes up to n
sim.world.set_mats(r, n)                          // dev / console only
fn_combat::mats::MAX_MATS                         // 999
```

- A fresh BR player has 0 / 0 / 0. That comes from the `Athena_GameMode` `StartingItems`, which grant only `WoodItemData x0`.
- The HUD resource counters and the inventory screen both read `sim.world` every frame, so a change shows up on the next frame. You don't need to notify anything.
- Harvesting in `fn-combat/src/lib.rs` (`World::strike`) adds through `add_mats`, so it is capped at 999 like pickups. (Fixed 2026-10-05; this line used to say it still added with `+=`.)

## Ammo

Ammo is stored on `crate::arms::Arms` (fn-app). There is one stack per ammo item, shared by every firearm that uses that item. The functions are in `crates/fn-app/src/arms/inventory_ops.rs`:

```rust
arms.ammo_for_gun(g) -> u32                 // the stack of firearm g's ammo type
arms.ammo_stacks() -> Vec<(String, u32)>    // (ammo item path, count) for every ammo type the loaded firearms use
arms.pickup_ammo(g, n) -> Result<(added, leftover), _>  // capped at the ammo item's MaxStackSize (999)
arms.spend_ammo(g, n) -> u32                // removes up to n, returns what was removed
arms.drop_ammo(g, n)                        // inventory drop (queues a world pickup)
```

The reload already consumes the stack (`reload_ammo::Run` transfers between the mag and the stack).

## Script steps (for verification runs)

`inv=open|close|equip|drop`, `invclick=T`, `invdbl=T`, `invdrag=T>T`, `invsplit=T>T` (Shift-drag), `mats=W:S:M`, with targets `slot1`..`slot5`, `tileK`, `drop`. See `verification/inventory/*.txt`.

## Starting loadout

- By default, interactive play starts with the 1.8 BR loadout: the pickaxe only, no ammo, and 0 materials (`Arms::new_br`).
- The old dev loadout (AR, pump, sniper, and 5 bandages) needs `--dev-loadout` or `FN_DEV_LOADOUT=1`.
- Scripted runs (`--script`, `--screenshot`) keep the dev loadout so that the existing weapon / heal scripts keep working. A script can ask for the BR start with the line `loadout br`.
