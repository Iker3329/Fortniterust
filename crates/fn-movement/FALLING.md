# Falling / jumping physics of the 1.8 player pawn (UE 4.16)

Sources used: the 1.8 paks through `fn-inspect` (`props`, read-only), and the 1.8 exe `FortniteClient-Win64-Shipping.exe` through the static tools
`fn-exe-defaults` (`classdump`, `dx dis`) and `fn-exe-slate` (`scan_disp`, `scan_calls` examples), a vtable reader and a pointer finder (scratch scripts). RVAs are image RVAs
(VA = 0x140000000 + RVA). The exe was never run, the client was never run, no key was recovered and no encrypted `Config/*.ini` was opened by this work.

## 1. Parameters and sources

| Parameter | Value | Source |
|---|---|---|
| World gravity (`DefaultGravityZ`) | **-980 by code path; the game's real value is not recoverable from allowed data** (port keeps -2800, see section 5) | exe `UPhysicsSettings` ctor 0x235e770 stores `-980.0f` at +0x38 (RVA 0x235e77e). `AWorldSettings::GetGravityZ` 0x2516ca0: `bGlobalGravitySet ? GlobalGravityZ : PhysicsSettings CDO +0x38` |
| Map override | none | `fn-inspect props /Game/Athena/Maps/Athena_Terrain` export 34 `FortWorldSettings`: only `KillZ -5000`, no `bGlobalGravitySet` / `GlobalGravityZ` / `WorldGravityZ`; the AWorldSettings ctor (0x250e490) leaves both bits clear; `AFortWorldSettings` (ctor 0x6535b0) does not override vtable slot 0x5e8 |
| `GravityScale` (CMC) | 1.0 | exe CMC ctor 0x2065290, `mov [rdi+0x190],0x3f800000` (RVA 0x2065441); not touched by the Fort ctor 0x6ceee0 or Athena ctor 0x4167b0; not in any pawn CDO |
| `GravityZScale` (FortMovementSet attribute) | 1.0 | `UFortMovementSet` ctor 0x416dc0 sets the attribute at set+0xd0 to base 1.0 and current 1.0 (vcalls at 0x416f0a / 0x416f16); getter 0x7de520 returns that value, or 1.0 when the pawn has no set. GEs only multiply it (x0.333, x0.19), no CharacterAttributesPlayer gravity column |
| Skydive / parachute `GravityScalar` | not used on the ground / in a normal jump | `UFortMovementComp_CharacterAthena::GetGravityZ` 0x428f00 multiplies only for `MovementMode == MOVE_Custom` with custom mode 2 or 3 (`PlayerPawn_Athena` CDO `CharMoveComp` AirControlParams rows) |
| `TerminalVelocity` | 4000 | exe `UPhysicsSettings` ctor +0x3c = 4000.0 (RVA 0x235e798); `ADefaultPhysicsVolume` ctor 0x213d7a0 copies CDO+0x3c into `APhysicsVolume+0x3c0` (RVA 0x213d7e1). No PhysicsVolume actor in Athena_Terrain |
| `JumpZVelocity` | 600 | `PlayerPawn_Athena_Generic` `CharMoveComp` (`fn-inspect props /Game/Athena/PlayerPawn_Athena_Generic`); CMC ctor default 420 (RVA 0x2065454, +0x198) |
| `AirControl` | 0.95 | `PlayerPawn_Athena_Generic` `CharMoveComp`; CMC ctor default 0.05 (RVA 0x20655d1, +0x20c) |
| `AirControlBoostMultiplier` / `...VelocityThreshold` | 2.0 / 25.0 | CMC ctor, RVAs 0x20655db / 0x20655e5 (+0x210 / +0x214) |
| `FallingLateralFriction` | 0.0 | the qword store at RVA 0x20655e5 writes 25.0 to +0x214 and 0.0 to +0x218; `PhysFalling` reads +0x218 (RVA 0x2088c95) |
| `BrakingDecelerationFalling` | 0.0 | the qword store at RVA 0x2065604 writes 2048.0 (walking) to +0x1fc and 0.0 to +0x200; `GetMaxBrakingDeceleration` (vtable 0x670) is not overridden by Fort |
| `MaxSimulationTimeStep` / `MaxSimulationIterations` | 0.05 / 8 | CMC ctor RVAs 0x20654ee / 0x20654f8 (+0x2ec / +0x2f0); read by `PhysFalling` at 0x2088b20..0x2088b2f |
| `MaxAcceleration`, `GroundFriction`, speeds | 750, 6, 410/550 | `PlayerPawn_Athena` `CharMoveComp`; speed stats from CharacterAttributesPlayer (unchanged by this work) |
| Fall-speed clamp | `Z >= -|TerminalVelocity|` only | Athena `NewFallVelocity` 0x43a380 (see section 2); there is no `MaxFallSpeed` member in this CMC |
| Consecutive-jump table | `{1,1}, {1,1}, {.75,.75}, {.5,.5}, {.25,.25}` and window 1.0 s | Athena ctor 0x4167b0: `TArray` at +0x758 filled at RVAs 0x416a9e..0x416bd5; `+0x768 = 1.0f`, `+0x76c = 0`, `+0x770 = 0` (RVA 0x416be5). Not serialized in any pak |
| DoJump frame-delta scale | threshold 0.03 s, `0.8897883 + 5.074536*dt`, max 2.5, optional 0.05 s clamp (flag bit 8 of +0x754, never set) | `UFortMovementComp_Character::DoJump` 0x6d4f20; constants in .rdata 0x410a678, 0x41b0e40, 0x41b0e44, 0x4078d78, 0x4040c78 |

## 2. The decoded algorithm

Class chain and constructors: `UCharacterMovementComponent` ctor 0x2065290 (vtable 0x47bfed8) <- `UFortMovementComp_Character` ctor 0x6ceee0 (vtable 0x41abd48) <-
`UFortMovementComp_CharacterAthena` ctor 0x4167b0 (vtable 0x4074c88). Overrides were found by diffing the three vtables slot by slot.

**PhysFalling** = CMC vtable slot 0x6b8, RVA 0x2088a20 (not overridden). Per iteration (`while remaining >= 1e-6 && Iterations < MaxSimulationIterations(8)`):

1. `timeTick = GetSimulationTimeStep(remaining, Iterations)` inlined at 0x2088b20: `if remaining > 0.05 && Iterations < 8: remaining = min(0.05, remaining*0.5)`, `max(1e-6, .)`. At 30 Hz (0.0333 s) it is one iteration.
2. `FallAcceleration = GetFallingLateralAcceleration(dt)` (vtable 0x6c0, computed once before the loop, Z zeroed): `AirControl` (0.95), boosted to `min(1, 2*0.95)` while `|V.xy|^2 < 25^2` (`BoostAirControl`), clamped to `MaxAcceleration`.
3. `MaxDecel = GetMaxBrakingDeceleration()` (0.0 while falling); if there is air control, `VelocityNoAirControl` comes from a `CalcVelocity` with zero acceleration and `Velocity.Z = 0`; the real `CalcVelocity(timeTick, FallingLateralFriction 0, false, MaxDecel)` runs with `Acceleration = FallAcceleration`, `Velocity.Z = 0`, then `Velocity.Z = OldVelocity.Z` (vtable 0x640, not overridden; speed cap from `GetMaxSpeed`).
4. Gravity: `Gravity = (0, 0, GetGravityZ())` (vtable 0x3a0) and `Velocity = NewFallVelocity(Velocity, Gravity, timeTick)`, same for `VelocityNoAirControl` (vtable 0x630, RVAs 0x2088d76..0x2088dd8). **timeTick is passed unchanged**: there is no `JumpForceTimeRemaining` / `bApplyGravityWhileJumping` branch (that is UE >= 4.17).
5. `AirControlAccel = (Velocity - VelocityNoAirControl)/timeTick`; `NotifyJumpApex` when `bNotifyApex` and `Velocity.Z <= 0` (0x2088e41..0x2088e78, animation hook only).
6. Move by `Adjusted = 0.5 * (OldVelocity + Velocity) * timeTick` (midpoint rule, exact for constant gravity) via `SafeMoveUpdatedComponent` (0x1420bb9b0); landing / slide / `LimitAirControl` (vtable 0x6e0) / two-wall logic is the stock 4.16 code.

**GetGravityZ** (all ahead of the integration):
`Athena 0x428f00 -> Fort 0x6d8350 -> CMC 0x207ea90 -> UMovementComponent 0x20b1ab0 -> APhysicsVolume 0x2396870 -> UWorld 0x2516d20 -> AWorldSettings 0x2516ca0`, i.e.
`g = ((WorldGravityZ * GravityScale) * GravityZScale)` (Athena factor only in the skydive / parachute custom modes).

**NewFallVelocity** - the Athena override (0x43a380, outside custom modes 2/3 at 0x43a5a9..0x43a5db) is *not* the stock vector clamp of 0x2084ed0:
`Result = Initial; Result.Z = max(Initial.Z + Gravity.Z * dt, -|PhysicsVolume->TerminalVelocity|)`. X / Y pass through. For a pure -Z gravity this is numerically the same as the stock code.

**Jump** (`ACharacter::CheckJumpInput` -> `DoJump`):
- Athena `DoJump` 0x421d20: `idx = window > 0 ? min(idx+1, 4) : 0`; `JumpZVelocity *= table[idx].first`; call Fort DoJump; on success `window = 1.0`, store `idx`, and clamp horizontal speed to `styleSpeed * table[idx].second` (limit < 1e-4 zeroes it).
- Fort `DoJump` 0x6d4f20: if the recorded pawn tick delta `dt` (+0x750; written by `AFortPawn::Tick` 0x7d0b00 via setter 0x6d24a0 only while `bPressedJump`) is `> 0.03`: `JumpZVelocity *= min(2.5, 0.8897883 + 5.074536*dt)`; then CMC `DoJump` 0x2078570: `Velocity.Z = JumpZVelocity; SetMovementMode(MOVE_Falling)`.
- `AFortPlayerPawnAthena::Tick` 0x41f900 decays `window = max(0, window - dt)`.
- Athena `GetMaxSpeed` 0x4343e0: while `MOVE_Falling` and the window is open the speed is multiplied by `table[idx].second`.
At 60 Hz and faster the frame-delta scale is 1.0; at the port's 30 Hz tick it is 1.0589 (see differences).

## 3. Differences found in the port, and fixes

| # | Port before | 1.8 exe | Fix |
|---|---|---|---|
| 1 | `cmc.rs` (old ~945-950) applied a `JumpForceTimeRemaining` / `bApplyGravityWhileJumping` gravity-time branch | `PhysFalling` 0x2088d92 passes `timeTick` unchanged | removed; `phys_falling` now calls `new_fall_velocity(v, g, time_tick)` |
| 2 | `new_fall_velocity` (old ~859) was the stock vector terminal clamp with `if dt > 0` | Athena override: `Z = max(Z + g dt, -|T|)` | replaced by the exe form (same numbers for -Z gravity) |
| 3 | `DoJump` set `Velocity.Z = JumpZVelocity` only | Athena consecutive-jump table + horizontal speed clamp, Fort frame-delta scale | `do_jump` / `fort_do_jump` / `base_do_jump`, `pawn_tick` (window decay + recorded delta), new `Params` and `Cmc` fields |
| 4 | `get_max_speed` ignored the jump window | falling speed x `table[idx].second` inside the window | `get_max_speed` + `style_max_speed` split |
| 5 | `GetGravityZ` multiplied in another order and labelled the GravityZScale "ASSUMED 1" | `((world * GravityScale) * GravityZScale)`, GravityZScale 1.0 from the exe | order fixed, provenance corrected |
| 6 | `FALLING_LATERAL_FRICTION`, `BRAKING_DECELERATION_FALLING`, `TERMINAL_VELOCITY`, `GRAVITY_SCALE` labelled ASSUMED / ini | all are explicit exe constructor stores (table above) | provenance corrected in `params.rs`, values unchanged |
| 7 | tests assumed frame-rate independent jumps and a hold-time gravity feature | decoded frame-delta scale; no hold-time gravity | `tests.rs`, `fn-game` `sim_tick.rs` / `jump_feel.rs`, `fn-sim` `series.rs` / `time_test.rs` updated to the decoded closed forms |

Not ported (animation / gameplay hooks, not physics): `NotifyJumpApex`, the skydive / parachute custom modes, `ProcessLanded` Fort gameplay events (0x6e2ca0), the gameplay-cue block in Fort `DoJump`.
The style speed used by the horizontal clamp (vtable 0x888 at 0x428410: movement-style stat x directional curve) is mapped to the existing `style_max_speed` (the stat -> style mapping was already marked ASSUMED).

## 4. The frame-delta scale and the 30 Hz tick

The exe scales JumpZ when the *pawn tick delta* is above 0.03 s: factor 1.0 at 60 Hz and faster, 1.0589 at 30 fps, 1.2281 at 15 fps. `Cmc::tick(dt)` feeds its own `dt` (the sim tick),
so at the port's fixed 30 Hz lock a first jump launches at `600 * 1.0589 = 635.36 cm/s` (apex +12 %), while a 60 fps `Controller::step` loop gets exactly 600. That is the decoded behavior of the
exe at that delta; the lock itself is the user's direction (README "Fixed 30 Hz tick"), not something the exe pins. `Cmc::frame_dt_override` (default `None`) and `Params::jump_frame_time_boost`
(default `true`) exist for a front-end that prefers to feed the display frame time or switch the term off; neither is used by default.

## 5. Provenance of gravity

- Allowed data: no map override; the game reads `UPhysicsSettings.DefaultGravityZ`; its exe constructor default is **-980** (`WORLD_GRAVITY_Z_EXE_DEFAULT`).
- `UPhysicsSettings` is a config object, so the shipped `Config/DefaultEngine.ini` (AES-encrypted in the pak) can override that default, and the exe and the paks cannot say whether it does.
  The runtime value is therefore **either -980 (no override) or whatever the ini sets**; the allowed sources cannot distinguish them.
- The -2800 in `WORLD_GRAVITY_Z` came from an earlier agent that decrypted `DefaultEngine.ini`, a route the project rules forbid. It is left in place unchanged and labelled in code and here:
  **PROVENANCE: decrypted config, not independently verified** (same for the 4000 terminal velocity only in the sense that the exe default also says 4000).
  `-980` (apex 183.7 cm, airtime 1.22 s for JumpZ 600) is the only value the allowed sources give; if the project wants the strictly allowed value, change `WORLD_GRAVITY_Z` to `WORLD_GRAVITY_Z_EXE_DEFAULT`.
- Everything else in the falling path (GravityScale, GravityZScale, integration, clamp, air control, jump chain) is exe / pak derived and independent of that one number.

## 6. Tests (real results)

Proof test: `fn-movement` `tests::standing_and_running_jump_equal_the_decoded_1_8_algorithm_at_30_hz` re-implements the decoded step independently (`v1 = max(v0 + g dt, -T)`, `z += 0.5 (v0+v1) dt`)
with the recovered parameters (g = -2800 composed as `(world*GravityScale)*GravityZScale`, launch = `600 * 1.0 * (0.8897883 + 5.074536/30)`) and compares it with the port's `Cmc::tick`:

```
30 Hz jump (g -2800, launch 635.3638 cm/s): standing apex 72.0293 cm (decoded 72.0293), 14 ticks = 0.4667 s (decoded 14);
running jump 191.333 cm (decoded 191.333 = 410 cm/s * 14 ticks * 1/30 s)
```

Companions in the same file: `jump_frame_scale_and_the_consecutive_jump_table_follow_the_exe` (launch speeds 600, 600, 450, 300, 150, 150 inside the window, restart after it),
`the_fort_new_fall_velocity_only_clamps_downward_z_at_the_physics_volume_terminal_velocity`, `apply_gravity_while_jumping_is_not_a_1_8_behaviour`.

`cargo test --release --no-fail-fast -p fn-movement -p fn-sim -p fn-game` (`CARGO_TARGET_DIR=D:\target-gravity2`):

| Suite | Result |
|---|---|
| fn-game lib | 101 passed, 0 failed |
| fn-movement lib | 61 passed, 0 failed |
| fn-sim lib | 18 passed, 0 failed |
| fn-sim `tests/parity.rs` | 4 passed, **10 failed** |

The 10 parity failures compare the port's logs with golden logs written by the old exe (`testdata/old-logs*`). They are not caused by this change: the movement-related ones
(`sim_tick_test`, `jump_feel_test`, `jump_air_test`, `movement_test`, `camera_video`, `camera_prop_collision`) fail because the goldens were recorded with gravity -980 (golden line `Jump (JumpZ 600, g 9.80 m/s^2)`),
and the port ran -2800 before this work (so they failed before it too, and the 30 Hz ones additionally differ by the decoded frame-delta scale now);
`face_test`, `pickaxe_test` (both rates) and `dir_test` do not involve jumps (max diff 0.0001 - 0.16 in animation columns) and come from the animation graph, which is outside this task.
No baseline run existed before the edits (no git), so this attribution is by log content, not by a before/after run.

`cargo build --release -p fn-app`: finished successfully (warnings only in fn-fx / fn-render, untouched).

## 7. Files changed

- `crates/fn-movement/src/params.rs` - gravity / terminal / falling constants with RVAs and provenance labels, `WORLD_GRAVITY_Z_EXE_DEFAULT`, jump chain table and window, frame-delta constants, `jump_frame_scale`, `jump_launch_z`, `jump_apex_cm_at`, `jump_airtime_at`.
- `crates/fn-movement/src/cmc.rs` - exe-form `new_fall_velocity`, removed jump-force gravity branch, `do_jump` chain (`base_do_jump`, `fort_do_jump`), `pawn_tick`, `style_max_speed`, falling speed scale, `GetGravityZ` order, new `Params` / `Cmc` fields (`frame_dt_override`).
- `crates/fn-movement/src/tests.rs` - jump tests re-based on the decoded closed forms, hold-time test replaced, new proof / table / clamp tests.
- `crates/fn-game/src/sim_tick.rs`, `crates/fn-game/src/jump_feel.rs`, `crates/fn-game/src/jump_air.rs` (comment) - 30 Hz jump expectations use the decoded scale; 30 fps per-frame jump test added.
- `crates/fn-sim/src/series.rs`, `crates/fn-sim/src/time_test.rs` - jump sweep / airtime expectations; stale `flight: simulated 1.2` assertion (a -980 leftover) replaced.
- `crates/fn-hitreg/src/shoot.rs` - comment only (provenance label).
- `crates/fn-movement/FALLING.md` - this file.


## 8. Gravity audit: runtime modifiers, cvars, readable data (exact vs inferred)

Scope: everything about gravity that sections 1, 2 and 5 did not cover. Tools: `decomp-grep` over the named decompile, byte scans of the exe for ASCII and UTF-16 strings, `fn-inspect props / ls / exports`,
and a scratch curve-table reader built on `fn-pak` + `fn-uasset` (`read_table`, kept outside the repo). No ini file was opened, the exe was not run, nothing was decrypted.

### 8.1 `UpdateGameStateModifiedGravity` and game-state / GameMode gravity (EXACT: absent in 1.8)

- The names `UpdateGameStateModifiedGravity`, `ModifiedGravity`, `GameStateModified`, `GravityModif` occur nowhere: not in the exe (decomp 0 functions; ASCII + UTF-16 string scan 0 hits) and not in the 1.8 SDK dump
  (`C:\FortniteRef\Fortnite_1.8.0_SDK\extracted\SDK`). That function belongs to a later build; 1.8 has no game-state gravity modifier.
- Complete list of gravity-named strings in the 1.8 exe (ASCII + UTF-16): `DefaultGravityZ`, `GlobalGravityZ`, `bGlobalGravitySet`, `WorldGravityZ`, `bWorldGravitySet`, `OnRep_WorldGravityZ`, `GetGravityZ`, `GravityScale`, `GetGravityScale`, `SetGravityScale`,
  `OnRep_GravityScale`, `InitialGravityScale`, `ProjectileGravityScale`, `outProjectileGravity`, `GravityZScale` (FortMovementSet attribute), `GravityScalar` (AirControlParams), `GravityDir*` (input motion state), `OverrideGravityZ` (UGameplayStatics
  predict-path parameter), `bEnableGravity` / `bGravityEnabled` / `SetEnableGravity*` / `bSimGravityDisabled` (physics bodies), `AbbreviatedGravity` (a keyboard key name), the vehicle `*StiffnessPerUnitGravity` PhysX members, and the UE debug format
  `"%s In physicsvolume %s on base %s component %s gravity %f"` (CMC DisplayDebug, RVA 0x2077a50). None is a game-state, GameMode or Athena-pawn global gravity setter.
- The SDK has no gravity member on the game state, GameMode, player pawn or Athena controller classes; the only gravity members are `AFortProjectileBase::GravityScale` (+0x6bc) and `UFortMovementSet::GravityZScale` (+0xd0).
- `AWorldSettings` gravity state: `bWorldGravitySet` = bit 0x40 and `bGlobalGravitySet` = bit 0x80 of +0x390, `WorldGravityZ` +0x3a0 (replicated, transient), `GlobalGravityZ` +0x3a4. `GetGravityZ` is vtable slot 189, RVA 0x2516ca0
  (lazy fill of +0x3a0 from `GlobalGravityZ` or `UPhysicsSettings` CDO +0x38). The only other store to those fields in the decomp is the old-package-version fixup in vfunc 20 (RVA 0x25214b0: package version < 0x14d and
  `GlobalGravityZ != 0` sets bit 0x80). A pattern search for stores to `+0x3a0` / `+0x3a4` / `+0x390 |= 0x40|0x80` in `Fort*` / `Athena*` functions found only unrelated members of other classes.
  Caveat (INFERRED): a pattern search cannot exclude an indirect write through a computed offset. The Athena_Terrain WorldSettings serializes no gravity (section 1), so readable data sets nothing either.

### 8.2 Console variables (EXACT: none)

No console-variable name containing "Gravity" exists in the exe (the complete gravity-string list is in 8.1). UE 4.16 itself has no gravity cvar for the CMC or the world, so there is no cvar default to trace.
Effective gravity is only `bGlobalGravitySet ? GlobalGravityZ : UPhysicsSettings.DefaultGravityZ`. The one exe default is -980 (ctor 0x235e770, `mov dword [rbx+0x38], 0xc4750000`, RVA 0x235e77e) and it is a `Config` property (SDK: `DefaultGravityZ` +0x38, "Edit, Config").

### 8.3 Pawn, skydive / parachute, GameplayEffects, other gravity paths

- Other consumers share `GetGravityZ` (vtable slot 0x3a0): the CMC chain of section 2, `UProjectileMovementComponent` (vfunc at RVA 0x20cefa0 calls it; the projectile scale then multiplies), and PhysX scene gravity via `UWorld::GetGravityZ`.
  The Fort/Athena movement code contains no hard-coded gravity constant: `PhysFalling` reads `GetGravityZ()` (RVAs 0x2088d76..0x2088dd8) and `NewFallVelocity` takes it as an argument. Jump uses no gravity (it writes `Velocity.Z = JumpZVelocity`, 0x2078570); only the fall afterwards integrates it.
- Skydive / parachute custom modes (`MOVE_Custom` 2/3): `UFortMovementComp_CharacterAthena::GetGravityZ` 0x428f00 multiplies by the active `AirControlParams.GravityScalar`. The rows are readable in `/Game/Athena/Balance/DataTables/AthenaGameData` (CurveTable, value at t=0, read with the scratch reader):

  | Row | GravityScalar | TerminalVelocity | MaxLateralSpeed | LateralFriction | MaxAcceleration |
  |---|---|---|---|---|---|
  | `Default.SkydivingControlPassive` | 1.0 | 1200 | 1200 | 0.8 | 4096 |
  | `Default.SkydivingControlActive` | 1.0 | 6000 | 2400 | 1.5 | 4096 |
  | `Default.ParachuteControlSkydive` | 1.0 | 500 | 2000 | 1.2 | 2048 |
  | `Default.ParachuteControlJump` | 1.0 | 500 | 800 | 1.3 | 2048 |

  All GravityScalars are 1.0 (EXACT), so skydive / parachute flight uses the world gravity unscaled; they neither confirm nor change its value.
- EXACT but not ported: in custom modes 2/3 the Athena `NewFallVelocity` (RVA 0x43a380..0x43a5df) is not the plain clamp: while `Vz < -T` (T from the AirControlParams TerminalVelocity) it sub-steps `Vz += (g / T) * Vz * dt` (step cap constant `DAT_144078d58`),
  so speed relaxes toward `-T` at a rate proportional to `g / T`; otherwise `Vz = max(Vz + g dt, -T)`. In skydive / parachute the approach to terminal velocity therefore depends on world gravity too. The normal ground / jump path is unaffected.
- GameplayEffects: `GravityZScale` is modified only by `GE_GM_SpeedUp_LowGravity` (Infinite; mutator `GM_Player_LowGravity`, delivery team `HumanCampaign` = Save the World, not BR) and the Outlander `GE_Outlander_GravitySphere*` / `GravityWell` abilities (STW class).
  The `PlayerPawn_Athena` / `PlayerPawn_Athena_Generic` CDOs serialize only health / shield regen, revive and `AthenaGameData` movement handles, no startup GE touching gravity; the Generic pawn has a `GameplayCue.GravitySphere` handler (visual of the STW ability).
  INFERRED: nothing on spawn or on the lobby island changes effective gravity in BR (no evidence in pawn CDOs or exe; GEs granted by unread native GameMode code cannot be fully excluded).
- Readable map data: `Athena_Terrain.umap` exports 4 `TriggerVolume`, 1 `PhysicsSerializer`, 1 `FortWorldSettings`, no `PhysicsVolume` / gravity volume, and `DefaultPhysicsVolume: null`. No readable uasset / umap serializes a base gravity value; the only `UPhysicsSettings` source is the exe constructor.
  Not opened by design: all `*.ini` (including the 68 encrypted configs and the engine `Base*.ini`).

### 8.4 What points to -980 vs -2800

| Evidence | Points to | Strength |
|---|---|---|
| exe `UPhysicsSettings` ctor default -980 (RVA 0x235e77e); no map override; no runtime setter; no cvar | -980 | EXACT as "the code default"; says nothing about the shipped ini |
| `DefaultGravityZ` is a `Config` property and DefaultEngine.ini is encrypted in the pak | open | not resolvable with allowed data |
| an earlier agent's read of the decrypted ini: `DefaultGravityZ=-2800` | -2800 | not admissible (forbidden route), not re-verified |
| Projectile designer scales (readable, `parity/guns/weapons.md` section 4): sniper bullet `ProjectileGravityScale` 0.18 at 30000 cm/s, grenade launcher 0.3 at 2750 cm/s, rocket / zapper 0 | weak, see below | INFERRED |
| Skydive rows (all scalar 1.0): reaching TerminalVelocity 1200 / 6000 / 500 takes `T/|g|` = 1.22 / 6.12 / 0.51 s at -980 versus 0.43 / 2.14 / 0.18 s at -2800 | weak, favours the heavier value if terminal speed is meant to be reached in a few seconds | INFERRED |
| Jump clips `JumpUp_NoWep_CMM_Additive` 0.40 s, `JumpFall_NoWep_CMM_Additive` 0.40 s, `JumpLand_NoWep_CMM_Additive` 0.83 s versus rise time at JumpZ 600: 0.61 s (-980), 0.21 s (-2800; 0.23 s with the 30 Hz scaled launch) | none, both fit a 0.4 s blend clip | inconclusive (AnimBP transition timings not decoded) |

Projectile arithmetic (INFERRED; effective gravity = world x scale): grenade launcher 0.3 gives 294 cm/s^2 at -980 (45 degree range v^2/g = 257 m) or 840 cm/s^2 at -2800 (90 m); sniper 0.18 gives 176 vs 504 cm/s^2 (drop over 100 m: 9.8 cm vs 28 cm).
A 0.3 launcher reads as a "short lobbed arc" more naturally at -2800, but this is a plausibility argument, not a measurement, and no 1.8 artifact states the intended range.

### 8.5 Verdict

- EXACT: the exe code path and every multiplier (CMC GravityScale 1.0, FortMovementSet GravityZScale 1.0, skydive / parachute GravityScalar 1.0 in `AthenaGameData`) give effective Z acceleration = `DefaultGravityZ`. 1.8 has no `UpdateGameStateModifiedGravity`,
  no game-state gravity setter found, no gravity cvar, no BR GameplayEffect on the pawn, no map / volume override, and no readable asset that serializes the base gravity.
- EXACT: the only obtainable value is the constructor default -980; the shipped value of that `Config` property lives only in the encrypted ini.
- INFERRED: nothing readable contradicts -980 and nothing readable proves -2800; the skydive-time and grenade-arc arguments lean slightly towards a heavier world but are soft.
- Decision: `WORLD_GRAVITY_Z` stays -2800 (its only source is the encrypted config; label unchanged) and `params.rs` was not edited, because no allowed exe/pak-verified value changed the answer. `WORLD_GRAVITY_Z_EXE_DEFAULT = -980` is the strictly allowed value.
  To settle "gravity is wrong" without the ini, use a reference from the real 1.8 game (jump height / airtime): -980 gives apex 183.7 cm and airtime 1.22 s at JumpZ 600; -2800 gives apex 64.3 cm and airtime 0.43 s (72.0 cm apex with the 30 Hz scaled launch 635.4).

## 9. Re-audit 2026-10-05 ("I'm falling too fast"): pak data bounds gravity from below

Tools: `fn-inspect exports / props / disasm` over every Athena / Building / Blueprints uasset (bytecode grep for `Gravity`, `LaunchCharacter`, `SuggestProjectile`, `PredictProjectile`, `TerminalVelocity`),
a scratch curve-table reader on `fn-pak` + `fn_uasset::classes::read_table` (outside the repo), `decomp-grep`. No ini opened, nothing decrypted, the client was not run.

### 9.1 Re-checked, unchanged (EXACT)
- `Athena_Terrain` export 34 `FortWorldSettings`: `KillZ -5000`, `MapWorldScale 290`, `MapZOffset 42000`, ...; no `bGlobalGravitySet` / `GlobalGravityZ` / `WorldGravityZ`. No PhysicsVolume actor.
- Pawn class chain `FortPlayerPawnAthena -> PlayerPawn_Athena_Generic_Parent_C -> PlayerPawn_Athena_Generic_C -> PlayerPawn_Athena_C`. `CharMoveComp` template overrides:
  Generic_Parent none; Generic `JumpZVelocity 600, AirControl 0.95, MaxAcceleration 1000, BrakingDecelerationWalking 1000, PerchRadiusThreshold 20, NavAgentProps.AgentHeight 150`;
  PlayerPawn_Athena `MaxAcceleration 750, GroundFriction 6` + the four AirControlParams (curve handles). No `GravityScale`, `FallingLateralFriction`, `BrakingDecelerationFalling` or terminal-velocity override anywhere.
- New detail: the native `AFortPawn` ctor (0x7c9ff0) writes CMC `JumpZVelocity` = 980.0 (+0x198), `AirControl` 0.9 (+0x20c), +0x1d4 = 300; the Generic Blueprint overrides JumpZ to 600 and AirControl to 0.95. Gravity is not touched.
- `UFortMovementSet` ctor (0x416dc0) initialises `BackwardSpeedMultiplier`, `GravityZScale`, `SpeedMultiplier` to 1.0; `JumpHeight` (+0xb0) is left at 0 and is only read through a cached attribute handle in `AFortPlayerPawn` vfunc 0x7e02d0 (not part of the jump / gravity path).
- `GE_GM_SpeedUp_LowGravity` multiplies `GravityZScale` by exactly 0.333 (STW mutator, not on the BR pawn).
- No cooked asset carries `DefaultGravityZ` (it is a `Config` property of `UPhysicsSettings`); the exe ctor default stays the only readable base value (-980).

### 9.2 New: falling damage is keyed by fall height (EXACT)
- `UFortMovementComp_Character::TickComponent` (0x6ea350): while falling, `FallingStartedZ` (+0x74c) = max(FallingStartedZ, current Z), i.e. the apex; not falling resets it to -3.4e38. `OnMovementModeChanged` (0x6dff00) seeds it.
- `AFortPawn` Landed (vfunc 244, 0x7eb460): `height = max(0, FallingStartedZ - Z)` -> `FFortPawnStats` falling-damage lookup (0x7dd7f0, "FallingDamageTable Lookup") -> damage fraction x max health (0x7cdf50). `bIgnoreNextFallingDamage` (+0x848) skips it.
- Curves (keys height cm -> fraction of max health):
  - `/Game/Athena/Balance/DataTables/AthenaFallingDamage` row `Player`: 0 up to 1299, 0.1 at 1300, 0.2 at 1536, 0.5 at 1920, 1.0 at 2304 (= 6 walls of 384), 5.0 at 7200.
  - `/Game/Characters/Enemies/DataTables/FallingDamage` row `Player` (STW): 0 up to 760, 0.05 at 850, 0.15 at 1000, 0.25 at 1500, ..., 1.0 at 3000.
  Because the key is height, these tables do not depend on gravity by themselves.

### 9.3 New: launch props bound the gravity (INFERRED, designer intent)
The game sets launch *velocities*; the resulting apex is v^2 / (2|g|) and is then charged against the height-keyed damage curve, so props that are clearly meant to be harmless constrain |g|.
`LaunchCharacter(V, false, false)` adds V, so `V.z = -Vz + c` sets the vertical speed to exactly `c`.

| Prop (bytecode) | Launch Z | sets bIgnoreNextFallingDamage | apex at -980 | apex at -2800 | placed in 1.8 BR maps |
|---|---|---|---|---|---|
| `Recreation_Pool_DivingBoard01` | 1600 | **no** | 1306 cm (STW curve: 0.15..0.25 damage; Athena curve: 0.1) | 457 cm (no damage) | no (STW prop, same project gravity) |
| `FireHydrant__2` | 2000 | yes | 2041 cm | 714 cm | 4 (`Athena_POI_StripMall`) |
| `Prop_TirePile_01` | `-Vz + 2000` (ceiling within ~4.4 m: `-Vz + 600`), Z override | yes | 2041 cm walking on | 714 cm | 86 (19 sublevels, 26 on the lobby island) |

The diving board carries no fall-damage protection, so it is only harmless if `1600^2 / (2|g|) <= 760` (STW curve start; the drop into the pool only raises the bound): **|g| >= 1684 cm/s^2**.
At the engine default -980 the board would cost 15-25 % health per use. -2800 satisfies the bound; -980 does not. No asset gives an upper bound (no BP queries `GetGravityZ`, predicts a path, or sets a jump height that must be reached).

### 9.4 Verdict and what changed
- The exact base value is still not readable from allowed data. The best data-backed value is unchanged: **-2800** (consistent with every bound; -980 contradicts the diving-board design; nothing points between them).
- Effective chain (normal fall / jump): `DefaultGravityZ (-2800) x CMC GravityScale (1.0, exe 0x2065441) x FortMovementSet.GravityZScale (1.0, exe 0x416dc0)` = -2800 cm/s^2;
  `NewFallVelocity` clamps Z at `-TerminalVelocity` = -4000 cm/s (exe 0x235e798); `FallingLateralFriction` 0, `BrakingDecelerationFalling` 0, `AirControl` 0.95.
- Timings (unchanged): first jump 600 cm/s -> apex 64.3 cm, rise 0.214 s, airtime 0.429 s (30 fps pawn tick: launch 635.4, apex 72.0 cm, airtime 0.454 s).
  Fall of 13 m (Athena damage threshold) 0.96 s, impact 2698 cm/s; terminal 4000 cm/s after 1.43 s / 28.6 m. At -980 these would be 183.7 cm / 1.22 s, 1.63 s, 4.08 s / 81.6 m.
- The port already implements the terminal clamp, zero falling friction and air control exactly; skydive / glider modes are entered only from the bus / launch pads (custom modes 2/3), never on a normal fall, and all their GravityScalars are 1.0.
  So the "falling too fast" feel is the 1.8 value as far as the data can tell, not a missing clamp or friction.
- Code: `params.rs` doc comment on `WORLD_GRAVITY_Z` (evidence pointer, value unchanged); stale comments fixed in `controller.rs` (`gravity()` said 980) and `fn-console` `registry.rs` (`gravity` help said "1 = the recovered -980").
  `cargo test --release -p fn-movement -p fn-console`: 65 + 30 + 1 passed, 0 failed.
