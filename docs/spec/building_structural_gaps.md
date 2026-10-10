# Building structural support / damage: verification and open gaps

Written by the independent verifier of `crates/fn-building/src/structural.rs` and `crates/fn-building/src/damage.rs`, 2026-10-05.
Sources: the 1.8 exe decompile (`C:\FortniteRef\decomp\named`, image VAs of `FortniteClient-Win64-Shipping.exe`), the 1.8 SDK dump
(`C:\FortniteRef\Fortnite_1.8.0_SDK`), and 1.8 cooked data, read with `fn-inspect props` and with `fn_uasset::classes::read_table` for curve and data tables.
Lines marked **UNCONFIRMED** are interpretations that were not fully traced.

---

## 0. Verdicts on the existing claims

| # | Claim (`damage.rs` / `structural.rs` module docs) | Verdict | Evidence |
|---|---|---|---|
| 1 | Health table `MAX_HEALTH_L0` = `AttributesBuildingSection` `<PlayerBuilding{Mat}{Tier}>.<Size9 wall / Size7 other>.FortHealthSet.MaxHealth` at level 0 | **CONFIRMED** | See below. |
| 2 | Support rule: a piece is supported iff `IsSupportedByWorld` (+0xE90 & 0x18), else its group has a world-supported member | **CONFIRMED** | See below. |
| 3 | Chain-collapse timer = 0.25 s + 0.1 s per Manhattan cell, no randomness | **CONFIRMED** | See below. |
| 4 | Building damage = the weapon's `EnvDmg*` columns; pickaxe `EnvDmgPB` 50 | **CONFIRMED** | See below. One step is open. |
| 5 | Pickaxe makes weak spots on player builds (`bAllowWeakSpots` = 1, no PB exclusion) | **CONFIRMED** | See below. Weak-spot damage 100 is still user-specified. |
| 6 | Hitting a player build yields 0 materials | **CONFIRMED** | See below. |

**Claim 1 evidence:**
- All 18 W/S/M rows have their first key at `Time 0.0`, and the values equal the table. `damage::tests::health_table_matches_the_paks` passes against the paks.
- The level is 0 in Athena; see section 7.
- `FFortAttributeSetInitter` (vtable 143ff2058) value lookup `FUN_1404361b0` evaluates the row's curve AT the level (`FUN_14211ea90(curve, level)`). It does not use UE's `Level-1` discrete table.

**Claim 2 evidence:**
- `IsBuildingActorStructurallySupported` @1404f3dc0: authority only (`Role` +0x118 == 3).
  - Supported if vtable +0x930 is true.
  - Otherwise, if group record `[sys+0x78] + group*0xA8` is valid (+0xA0 != 0), supported iff `Num(+0x58) - NumFree(+0x84) > 0`.
- `ABuildingSMActor::IsStructurallySupported` @14047fde0 calls it only when `bRegisterWithStructuralGrid` (+0xE90 bit 0x20) is set. Without that bit it returns false.
- +0x930 = slot 294 @14047fe40 = `(+0xE90 & 0x18) != 0`. SDK bits at +0xE90: 0x08 `bSupportedDirectly`, 0x10 `bForciblyStructurallySupported`, 0x20 `bRegisterWithStructuralGrid`.

**Claim 3 evidence:**
- `FUN_1404f4be0` reads `UFortGameData` +0x4B0..+0x4BC via `FUN_1405861d0/e0/f0`, `FUN_140586200` and clamps the base range to [0.1, FLT_MAX].
- `FUN_1404cecc0` sets the timer to `rand*(1/32767)*(hi-lo)+lo + (rand*(1/32767)*(hi2-lo2)+lo2) * (|dx|+|dy|+|dz|)`.
- The `UFortGameData` ctor @1405751d0 does 8-byte stores: `param_1[0x96] = 0x3e800000` sets +0x4B0 = 0.25 and +0x4B4 = 0; `param_1[0x97] = 0x3dcccccd` sets +0x4B8 = 0.1 and +0x4BC = 0.
- `/Game/Balance/DefaultGameData` (class `FortGameData`) serialises no collapse field. `/Game/Balance/AthenaGameData` is a different class (`UAthenaGameData`: currency, XP table, dances, random characters) and holds none of these fields.
- Nuance not in `structural.rs`: `FUN_1404cecc0` also runs the cube connectivity test (`FUN_1404daa90`, line ~160 of its decompile) for each candidate. A neighbour gets a timer only if it is in the neighbour list (section 4) AND cube-connected (section 2).

**Claim 4 evidence:**
- The damage GE `GE_Melee_GenericDamage` executes `FortDamageFormulaExecutionCalculation` @14065f1c0.
- Its target category comes from `FUN_14092c310`, which calls slot +0x18 of the target's `UFortDamageableActorInterface`. If the target does not implement the interface, the category defaults to 1 = environmental.
- When environmental, the execution captures `FortDamageSet.OutgoingBaseEnvironmentalDamage` (`FUN_1406653d0`) and evaluates the weapon stats' falloff with selector 1. In `FUN_1409223e0` that reads stats +0x1C..+0x28 = `EnvDmgPB/Mid/Long/MaxRange`.
- Data: `AthenaMeleeWeapons.Harvest_Pickaxe_Athena_C_T01` has `EnvDmgPB/Mid/Long/MaxRange` = 50/50/50/50, `DmgPB` = 15, `EnvDmgScaleTable` null.
- **UNCONFIRMED:** the value `ABuildingSMActor`'s interface slot +0x18 returns. It is assumed environmental, which matches the default.

**Claim 5 evidence:**
- `ABuildingSMActor` ctor @14045b960 writes +0x810 = 0x0101 (`bAllowWeakSpots`, `bUseComplexForWeakSpots`). No `PBWA_*` CDO overrides it.
- `OnDamageServer` (vfunc 238 @140478e70) calls `AFortPlayerController::ServerSetupWeakSpotsOnBuildingActor` when all of these hold:
  - not `bDestroyed` (+0x554 bit 0);
  - vfunc 414 @140491c80 (vtable +0xCF0) finds the `Gameplay.Effect.GenerateWeakSpots` tag (globals +0x198);
  - vfunc 297 (+0x948, the `WeakSpotTrace`) succeeds.
- The Athena pickaxe ability `GAB_Melee_ImpactCombo_Athena` (super `GAB_Melee_GenericCombo`) applies `GE_Melee_GenericDamageMakeWeakspot` on every target list, the `WeakSpots` filter list included. That GE adds `Gameplay.Effect.GenerateWeakSpots`.

**Claim 6 evidence:**
- `ABuildingSMActor::DetermineMaxResourcesToSpawn` @1404687d0 tries three sources in order:
  1. `AFortWorldSettings` +0x6A0 table with the actor's row name (+0x898);
  2. the actor's `BuildingResourceAmountOverride` (+0x890);
  3. otherwise `GetBuildingCost * UFortGameData::GetBuildingDeconCostMult(.., bIsPlayerBuildable = +0x564 bit 3)`.
- `GetBuildingDeconCostMult` @1405867e0 uses `PlayerBuildingDeconCostMultHandles` (+0x558) for player-buildable actors.
- The PBWA CDOs set `bIsPlayerBuildable` and no override row. Checked: W1/S2/M3 × Solid/Floor/StairW/RoofC.
- `DefaultGameData` `PlayerBuildingDeconCostMult_{Wood,Stone,Metal}` has 9 keys in total, all 0.0.

---

## 1. EMP connectivity cube (`UBuildingEditModeMetadata.ConnectivityCubeData`)

- `UBuildingEditModeMetadata` +0x58 holds `FConnectivityCube`, size 0xC0.
- It is registered with native struct ops (`FScriptStruct_..._StaticRegisterNativesFConnectivityCube` @1409b5ec0, `Z_Construct_UScriptStruct_FConnectivityCube` @140d57dd0), so it is NOT tagged.
- `fn-inspect props` currently fails on it with `Malformed: tag name index 25 out of range` and dumps the raw 48 bytes. **Gap: fn-uasset needs a native reader.**

**Serialized format (48 bytes):**
- 6 `TBitArray`s in `EFortConnectivityCubeFace` order: Front 0, Left 1, Back 2, Right 3, Upper 4, Lower 5.
- Each one is `int32 NumBits` (always 25) followed by `ceil(NumBits/32)` dwords (1 dword).

**In memory:**
- 6 × 0x20 bytes: `TBitArray` with inline storage at +0x00, heap pointer +0x10, `NumBits` +0x18, `MaxBits` +0x1C.
- Copy: `FUN_1404a4960`. Clear: `FUN_1404a1b10` / `FUN_1404a1b60`.

**Bit layout:**
- Each face is a 5×5 grid: `bit(face, row, col) = (dword >> (row*5 + col)) & 1` (`FUN_1404b9a20`).
- Rows are vertical for side faces (only rows 0..3 are used) and the second horizontal axis for Upper/Lower (rows 0..4).

Example, `EMP_Wall_Solid`: Front `0x000fc63f`, Left `0x00084210`, Back `0`, Right `0x00008421`, Upper `0x01f00000`, Lower `0x01f00000`.
- The 57 EMP assets in 1.8 are: Floor ×8, Pillar, Roof ×5, Stair ×4, Wall ×39.
- Their raw hex was dumped during verification. Regenerate with `fn-inspect props <EMP> | grep -A3 ConnectivityCubeData`.

**Cube transforms:**
- Rotate 90° about Z: `FUN_1404c3140`. It rotates Upper and Lower in place (`FUN_1404c34c0` on faces 4 and 5) and cycles the side faces 0←1←2←3←0 (Front takes Left, and so on).
- Mirror: `FUN_1404bb3d0`. It flips faces 0, 2, 4, 5 left/right (`FUN_1404bb670`), swaps faces 1↔3, then flips 1 and 3.

---

## 2. Connectivity test `FUN_1404daa90(sys, A, locA, rotA, mirA, B, locB, rotB, mirB) -> bool`

Called by `FUN_1404da8a0`, which feeds each actor's `vfunc +0x8C8` location, its root-component rotation, and `bMirrored` (+0x780 bit 1).

**Cache:** results are cached at `sys + 0x28`: lookup `FUN_1404e7b40`, store `FUN_1404dd030`.

**Cell lookup** (`FUN_1404ea060`):
- `cell = floor((loc - sys.origin) / (512, 512, 384) + 0.5)`. The origin is at sys+0x3C4.
- Constants: 0x144040c68 = 1/512, 0x144040c6c = 1/384, 0x143feb1d8 = 0.5. The exe computes `ROUND(2u + 0.5) >> 1`.

**Cube construction** (`FUN_1404f75f0`):
- It dispatches on `BuildingType` (+0x3F0) through three type sets: walls at sys+0x278, floors at +0x2C8, centre-cell at +0x318.
- Common prologue in every handler:
  1. Copy `EditModePatternData` (+0xC00) cube data (`FUN_1422c52d0` = +0x58).
  2. If `bMirrored`, mirror.
  3. Compute quarter-turns `q = round(normalize_0_360(yaw) / 90)`. Constants: 0x14404e210 = 1/90; 0x143ff9044 = 360 and 0x143ff9014 = 1/360 for the fmod.
- Wall (`FUN_1404f7b80`):
  - If `q - 1 < 2` (q = 1 or 2), mirror again.
  - Side `s = FUN_1404e1e80(loc, cell)` is 0..3: the dominant axis of `loc - cell corner` (corner = origin + cell*512); 0/1 = ∓X, 2/3 = ±Y.
  - If s == 3, cell.y -= 1. If s < 2, rotate once; if s == 0, also cell.x -= 1.
- Floor / stairs / roof set (`FUN_1404f79d0`):
  - `FUN_1404e1de0` returns `round((z - (origin.z + cell.z*384)) / 384) * 384 < 1e-4`, i.e. true when the piece sits at the bottom of its cell. When it returns false (the piece is at the cell top), cell.z += 1.
  - Then rotate n times: n = q, except an odd q becomes `(q+2) & 3`.
- Centre-cell (`FUN_1404f7840`): the same as the floor set, without the z adjust.

**Lattice comparison** (`FUN_1404f8bb0(lattice, cube, d, insert)`):
- `d` = cell difference. It requires `|d.x|, |d.y|, |d.z| < 3`.
- The lattice spacing is 4 per cell in X/Y and 3 per cell in Z.
- `FUN_1404f92d0` walks each face. For bit (row, col) it emits `start + col*colStep + row*rowStep`, with cols 0..4, and rows 0..3 for side faces or 0..4 for faces 4/5.

| face | plane | start | col step | row step |
|---|---|---|---|---|
| 1 Left | x = 4dx | (4dx, 4dy, 3dz+3) | (0,+1,0) | (0,0,-1) |
| 0 Front | y = 4dy+4 | (4dx, 4dy+4, 3dz+3) | (+1,0,0) | (0,0,-1) |
| 3 Right | x = 4dx+4 | (4dx+4, 4dy+4, 3dz+3) | (0,-1,0) | (0,0,-1) |
| 2 Back | y = 4dy | (4dx+4, 4dy, 3dz+3) | (-1,0,0) | (0,0,-1) |
| 5 Lower | z = 3dz | (4dx, 4dy, 3dz) | (+1,0,0) | (0,+1,0) |
| 4 Upper | z = 3dz+3 | (4dx, 4dy, 3dz+3) | (+1,0,0) | (0,+1,0) |

**The test:**
1. A is inserted with d = (0,0,0).
2. B is queried with d = (cellB - cellA): each set point found in A's set is collected.
3. The return value is the collected count. **Connected iff count > 1**, i.e. at least 2 shared lattice points: a shared edge, not a single corner.

**What `structural.rs` reconstructed:** its edge-sharing heuristic stands in for this table. The exact rule is now available and should replace it.

---

## 3. Pivot offset (cell position of a piece)

- `ABuildingSMActor` vfunc 281 @140477f30 returns `location - rotation.RotateVector(BaseLocToPivotOffset)`, with `BaseLocToPivotOffset` at +0x6F8.
- When +0x554 bit 0x10 is set, it returns the plain location instead. That bit is set by the wall ctor @14049fe20.
- The Wall @14049fe20, Floor @140459d10 and Roof @14045b660 ctors set +0x6FC = -256, i.e. `BaseLocToPivotOffset` = (0,-256,0). Stairs @14049f370 gets it through the floor ctor.
- The W1 Wall/Floor/StairW/RoofC CDOs do not override it.
- The structural code uses the actor location (`vfunc +0x8C8`) for cells.

---

## 4. Groups (`UndermineGroup`) and neighbours

**Group storage:**
- Group id per actor: `UndermineGroup` +0xCAC. It is written by `FUN_14048d780`, authority only. Getter: vtable +0xA38; setter: +0xB68.
- Group records: `sys+0x78`, stride 0xA8, valid flag +0xA0. Each record holds the member set (record+0x10..) and the world-supported member set (record +0x50, counted by +0x58 / +0x84).

**Neighbour gather** (`FUN_1404ee8c0(sys, type, loc, cell, out)`): it dispatches by type set.
- **Wall** (`FUN_1404edcf0`), with side s from `FUN_1404e1e80`:
  - The two wall slots of its own cell. Slot keys come from `FUN_1404f0160`, sides 0/1 from map sys+0x128 and 2/3 from map sys+0x178 (`FUN_1404f0040`).
  - The floors above and below (`FUN_1404e96c0`, map sys+0x1C8).
  - The centre cell (`FUN_1404e8800`, map sys+0xD8).
  - The same three for the cell across the wall: x±1 or y±1 by side.
  - The perpendicular walls at z+1 and z-1.
  - The in-line walls to either side.
  - The centre cells at z±1 of both cells.
- **Floor** (`FUN_1404ecf40`):
  - The 4 walls of its cell and of the cell at z±1, picked by `FUN_1404e1de0`.
  - The 4 horizontally adjacent floors.
  - The centre cells of its own cell and the neighbouring cells.
- **Centre-cell** (`FUN_1404ebeb0`):
  - The 4 walls of its cell, z+1 and z-1.
  - The floors above/below of its cell and of the 4 horizontal neighbours.
  - The centre cells of the 4 horizontal neighbours and at z±1.

**Group join / dissolve:**
- Join on register: `FUN_1404db8e0`. For every gathered neighbour that passes `FUN_1404da8a0` (the connectivity test in section 2), it collects the neighbour's group.
  - 0 groups: create a group (`FUN_1404e1840`).
  - 1 group: join it (`FUN_1404d8dc0`).
  - Several groups: merge them (`FUN_1404e0b20`).
  - World-supported members (vtable +0x930) are also added to the record's +0x50 set.
- Dissolve on death: `FUN_1404fc9a0`, called from `ABuildingSMActor` vfunc 240 @14047aa10 after `FUN_1404f4be0` (schedule) and `FUN_1404fc7a0` (unregister from the cell maps).
  1. Clear the dead actor's group.
  2. Reset every member to -1.
  3. Re-run `FUN_1404db8e0` for each former member, rebuilding the groups.
  4. Notify `sys+0x3F0` (`FUN_1415471e0`).

---

## 5. World support: `bSupportedDirectly` writer and the `WorldOverlapCheck` sweep

**The writer** that `structural.rs` says was not found: vtable +0x928 = slot 293 @140480660, `+0xE90 |= 8` (`bSupportedDirectly`). Its only building callers are the three structural register functions:

| register fn | piece set | slot check | sweep fn |
|---|---|---|---|
| `FUN_1404d9250` | floor (map sys+0x1C8) | `FUN_1404de610` | `FUN_1404df7e0` |
| `FUN_1404d9d80` | wall (maps sys+0x128 / +0x178) | `FUN_1404de8c0` | `FUN_1404df7e0` |
| `FUN_1404d90f0` | centre-cell (map sys+0xD8) | `FUN_1404de3e0` | `FUN_1404df7e0` |

**Slot-check result codes:**
- 0 = OK.
- 1 = the slot is occupied by an actor without `bDestroyOnPlayerBuildingPlacement`.
- 2 = world-blocked.
- 4 = outside the grid.

Registration proceeds on 0, or on 1 with the force flag. `bSupportedDirectly` is set when the sweep reported a world contact (out-param `param_8`).

**Sweep `FUN_1404df7e0(sys, start, end, halfExtent, .., threshZ, allow, &touchedWorld, &list)`:**
- Query name `"WorldOverlapCheck"`. Box sweep from `start` down to `end` (`FUN_14209d330`).
- Collision object/response params come from `FUN_140297ab0` and `DAT_145198f30`. Which engine query `FUN_14209d330` is remains **UNCONFIRMED**; it is probably `SweepMultiByObjectType`.

| piece | half extent (x,y,z) | start z | end z | threshZ |
|---|---|---|---|---|
| floor | (248, 248, 0) | z+15 | z-18 | z+13 |
| wall, sides 0/1 (wall plane normal to X) | (6, 248, 0) | z+375 | z-18 | z+10 |
| wall, sides 2/3 (wall plane normal to Y) | (248, 6, 0) | z+375 | z-18 | z+10 |
| centre-cell | (248, 248, 0) | z+375 | z-18 | z+10 |

Constants: 0x1440c5e38 = 248, 0x144078d7c = 6, 0x1440c5e30 = 18, 0x144040c98 = 15, 0x1440c5e40 = 375, 0x1440c5e2c = 13, 0x143fe3714 = 10.

In `FUN_1404de8c0` the extents are `(x, y) = (6, 248)` when side < 2, else `(248, 6)`.

**Per hit:**
- **`ABuildingActor` whose component blocks channel 0x18:**
  - Ignored if `bDoNotBlockBuildings` (+0x564 bit 0) is set or `allow` is true.
  - With `bUseCentroidForBlockBuildingsCheck` (bit 2), the hit counts only if the actor's centroid (vtable +0x7F0) lies strictly inside the swept box and its z is at least threshZ.
  - It is appended to `list`. If it lacks `bDestroyOnPlayerBuildingPlacement` (bit 1), the sweep fails (returns 0 = blocked). Otherwise it is only listed, to be destroyed by placement.
- **`AStaticMeshActor` with a `UStaticMeshComponent`:**
  - If the hit `Location.z` (`FHitResult` +0x14) > threshZ and not `allow`, the sweep fails.
  - Otherwise it sets `touchedWorld = 1`.
- **`ULandscapeHeightfieldCollisionComponent`:** sets `touchedWorld = 1`.
- **Anything else:** ignored.

Consequence (**UNCONFIRMED** interpretation): only landscape and plain `AStaticMeshActor` geometry at or below the piece base make a piece world-supported. Placed `ABuildingActor`s (map houses, props) never set `bSupportedDirectly`; they can only support through the group/connectivity path.

---

## 6. Collapse scheduling (for reference)

- `FUN_1404f4be0`, called from the dead actor's vfunc 240:
  1. Gather neighbours (section 4).
  2. Read the four `UFortGameData` values (claim 3).
  3. Call `FUN_1404cecc0` on the wall list (a_8[0..1]) and the floor list (a_8[2..3]), and `FUN_1404ce6b0` on the centre list (a_8[4..5]).
- Each candidate that passes `FUN_1404daa90` gets `FTimerManager::SetTimer` (`FUN_1424d6af0`) on its handle at actor +0xFB0 (`plVar9 + 0x1F6`). The delegate is `LAB_14046041c` → vtable +0xB70 (vfunc 366 @1404665c0).

---

## 7. Building health level: `AFortGameStateZone::PlayerBuildingSkillLevel` (+0x85C)

**Who reads it:**
- `ABuildingActor` vfunc 225 @14046bc40: when `AttributeInitLevelSource` (+0x4C8) == 1 (`PlayerBuildingSkill`), it returns `GameState->vtable[+0x880]()`.
- That slot (272) is @1405d53a0 in both the Athena (14406f3b0) and Zone (14411e8d0) vtables: `movss xmm0, [rcx+0x85C]; ret`.

**SDK:** `float PlayerBuildingSkillLevel // 0x085C (Net, Transient)` on `AFortGameStateZone`. `AFortGameStateAthena` derives from `AFortGameStatePvP`.

**Writers:**
- `AFortGameStateZone` ctor @1405aeee0: `+0x85C = 0`.
- `AFortGameStateZone` vfunc 271 @1405e0760 (slot 271 is shared by the Athena vtable) does the following, on authority only:
  1. Call vtable +0x8C0, i.e. `AFortGameStateZone` vfunc 280 @1405c0e80, with the attribute `UFortConstructionSet.BuildingSkill` (`FUN_1405cb810`).
  2. Vfunc 280 iterates the world's player controllers. For every `AFortPlayerController` with `MyPlayerInfo` (+0x2128) != null, it reads `BuildingSkill` from the ability system of the actor behind `MyPlayerInfo`+0x158 (`FUN_1406071c0` → `FUN_143e3b950` GetNumericAttribute).
  3. It takes the maximum, starting from -FLT_MAX (0x144031dac).
  4. If any PC qualified and `|value - old| > 1e-8`, it stores the value and calls vtable +0x8C8.
- Vfunc 271 is called from `AFortGameStateAthena::OnRep_GamePhase` @1404369d0, among other callers.
- `AFortGameStateOutpost` vfunc 271 @1405e0640 is the only other writer, and it is Save the World only.

**Athena value: 0.**
- `BuildingSkill` is only initialised from Save-the-World hero data. The only rows, in `AttributesHeroNonScaling`, are 0, per `damage.rs` (not re-verified here).
- An ASC without a `FortConstructionSet` returns 0 from GetNumericAttribute. So the maximum is 0, and if no PC has `MyPlayerInfo` the ctor value 0 stays.
- Because the property is `Net`, clients receive the server's 0.

---

## 8. Remaining gaps (for a fixer)

1. **fn-uasset:** add a native reader for `FConnectivityCube` (format in section 1).
2. **fn-building/structural.rs:** replace the edge-sharing heuristic with the exact algorithm:
   - the cube rasterisation and lattice table (section 2), including the wall side / mirror / rotation rules and the floor z adjust;
   - the `> 1` shared-point test;
   - the neighbour lists (section 4);
   - the cube-connectivity filter on collapse candidates (section 6).
3. **fn-building:** implement world support with the `WorldOverlapCheck` boxes and thresholds (section 5) instead of the caller-provided `world_supported` closure. `bSupportedDirectly` is set only at registration.
4. **Weak-spot damage:**
   - `DefaultGameData.BuildingWeakPointDamageMultiplier` = 4.5 at t = 0, 1, 2 (constant).
   - The exe contains no `BuildingWeakPointDamageMultiplier` string (ASCII or UTF-16).
   - The pickaxe chain `GAB_Melee_ImpactCombo_Athena` → `GE_Melee_GenericDamageMakeWeakspot` → `GE_Melee_GenericDamage` (only `OutgoingBaseAbilityDamageMultiplier` captured) does not reference the row.
   - So `PICKAXE_WEAK_SPOT_DAMAGE` = 100 remains user-specified. **UNRESOLVED:** how `FortDamageFormulaExecutionCalculation` @14065f1c0 scales a weak-spot (critical) hit.
5. **UNCONFIRMED:** the damage category `ABuildingSMActor` returns from `UFortDamageableActorInterface` slot +0x18 (claim 4).
