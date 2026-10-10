# Spec: match start to landing (warmup, Battle Bus, skydive, glider, landing), Fortnite 1.8 Battle Royale

Status: research spec, read-only. Nothing here is implemented yet. Athena (BR) only, never Save the World.
Written 2026-10-05 from:
- the named Ghidra decompile (`C:\FortniteRef\decomp\named`, addresses are VAs 0x14xxxxxxx; RVA = VA - 0x140000000),
- exe constants read statically from `FortniteClient-Win64-Shipping.exe` (never run),
- cooked 1.8 assets through `fn-inspect` (`props`, `exports`, `disasm`),
- the 1.8 SDK dump (`C:\FortniteRef\Fortnite_1.8.0_SDK\extracted\SDK`),
- UE 4.16.3 source (`C:\UE4.16`).
No config was decrypted, so `Config` fields carry their code defaults (they are flagged).

Conventions: "AGD" = CurveTable `/Game/Athena/Balance/DataTables/AthenaGameData`. Every `FScalableFloat` in this area is evaluated with
`FScalableFloat::GetValueAtLevel` (0x143d81440) at level 0, so the effective value is `Value (1.0) x AGD row key at Time 0`. Kismet offsets are the
serialized offsets printed by `fn-inspect disasm`; its jump targets are in-memory offsets and are not translated, so BP branch wiring is
reconstructed from block contents where noted.

The port's existing pieces this builds on: `crates/fn-movement/FALLING.md` (PhysFalling, NewFallVelocity for the normal fall, gravity,
fall damage), `crates/fn-animgraph` (player AnimBP: locomotion, jump, land, `YawDeltaSmoothed`, `AimPitch`, BlendSpace evaluator).

---

## 0. Quick reference (the numbers a porter needs first)

| What | Value | Section |
|---|---|---|
| Phases | Setup (wait for `WarmupRequiredPlayerCount` = 80 loaded players) -> Warmup (60 s countdown, cut to 15 s at >= 95 % of MaxPlayerCount 100) -> Aircraft -> SafeZones | A 1 |
| Warmup island | `/Game/Athena/Maps/POI/Athena_POI_Lobby_001` streamed at (-125440, -113664, 3840), 110 `FortPlayerStartWarmup`; warmup pawns cannot be damaged | A 1.7 |
| Bus phase start | all pawns destroyed, PCs go to `Spectating`, player-built buildings destroyed, every PC enters the aircraft (no pawn on the bus) | A 1.7, B 2.1 |
| Phase steps | GetReady when warmup time left <= 10 s; BusLocked until `DropStartTime` (shown time = left + 1); BusFlying until `DropEndTime` | A 1.6 |
| Bus path | random angle a1 on the drop circle (centre/radius from `IslandZoneVolume` bounds: C (32512, -24192), R 112256), exit at a1 + 180 +/- 45 deg (`AircraftDeviationAngle` 45), line clipped to the spawn box (MapInfo +/- 160000), height 80000 + MapInfo z 1312 = 81312, speed 6000 cm/s | A 2.4 |
| Doors open / auto-kick | doors open when the bus enters the island volume (`DropStartTime`); at `DropEndTime` every PC still on the bus is force-restarted (auto-jump); SafeZones phase starts with a 60 s delay | A 2.6, 2.7 |
| Bus camera | no spring arm: `CameraSpaceViewOffset` X -2200 / -1800 / -1200 at view pitch -90 / 0 / +90, view-target offset (-45, 0, 300), FOV 90, free look = control rotation, initial pitch -30 | B 2.4 |
| Bus visuals | aircraft actor has no mesh; client spawns cosmetic `B_StockBattleBus_C` (static meshes bus + balloon + apparatus + flame + generator), loop sound + party music, fade in 2 s | B 1.5 |
| Bus HUD | AthenaAerialFeedback "Press [Jump] To Jump" (BusCanExit only); GamePhaseAlert "BATTLE BUS LAUNCHING IN" / "DOORS WILL OPEN IN" / "EVERYBODY OFF, LAST STOP IN"; no bus route line and no "thank the bus driver" in 1.8 | B 3, D 6 |
| Jump | Space / gamepad bottom, `ServerAttemptAircraftJump(ControlRotation)`; new pawn at the aircraft, yaw only, velocity = bus forward x 6000 | A 3, C 2.3 |
| Skydive | `MOVE_Custom` 3; glider open = `MOVE_Custom` 2; stock PhysFalling with overridden params | C 2 |
| Skydive params | passive: accel 4096, friction 0.8, lateral 1200, terminal 1200; active (dive): 4096 / 1.5 / 2400 / 6000 (blended by dive steepness) | C 1.1, 2.5 |
| Glider params | from skydive: 2048 / 1.2 / 2000 / 500; from a jump: 2048 / 1.3 / 800 / 500 | C 1.1 |
| Dive input | hold forward while looking down: input dir z < -0.1 and dot(look, input) > 0.4 | C 2.5 |
| Deploy | Jump toggles glider (open <-> close, any number of times); auto force-open below Z 20000 when a 84 cm sphere sweep 10000 cm along 0.3-blended velocity hits, or below Z 10000; forced-open glider cannot be closed | C 3 |
| Landing | `Landed` ends skydive -> Walking; first landing after a skydive takes no fall damage; glider keeps fall height ~0 | C 4 |
| Cameras in the air | Glide FOV 90, Dive FOV 100, Glider 80 (Athena_PlayerCameraModeSkydive*) | C 5 |
| Anim | FullBody SM states SkyDive_Glide / SkyDive_Dive / ParaThing_Into / ParaThing, 0.2 s blends; glider mesh AnimBP Closed / Open_Into / Idle_Open | D 2, D 3 |
| Default glider | `/Game/Athena/Items/Cosmetics/Gliders/DefaultGlider` -> `SK_M_VinderTech_GliderChute`, attached to socket `weapon_r` | D 1, C 1.2 |

## 0.1 Cross-section reconciliation (read this before the sections)

The four sections were researched separately; where they overlap, this is the resolved reading:
1. Aircraft fields +0x40C / +0x410 / +0x414 (B lists them as unnamed) are `FlightDuration`, `DropStartOffset`, `DropEndOffset` written by InitializeFlightPath FUN_1403f92f0 and consumed by StartFlightPath FUN_140408ef0 (A 2.4, 2.5).
2. FUN_14041dd60 (B calls it unresolved) is **StartSkydiving** (C 2.4). FUN_1403d1850 (A calls it unnamed) is **SpawnCosmeticActor** (B 1.5).
3. The global DAT_14521d994 that picks custom mode 2 or 3 at skydive start (A UNRESOLVED 7) is cvar `Athena.AutoDeployParachuteOnBeginSkydive`, default 0, so mode 3 (C 1.5).
4. `FlightSpeed` (C UNRESOLVED 6) is `MapInfo.AircraftSpeed` = 6000, set in InitializeFlightPath (A 2.4 step 7).
5. **Camera after the jump:** ExitAircraft calls `SetViewTarget(pawn)` with FViewTargetTransitionParams {BlendTime **0**, BlendFunction 1 (VTBlend_Cubic), BlendExp **2.0** (0x143ff9020)}. A and B read it this way; C 2.3 step 2 calls the 2.0 a blend time, which is wrong: 2.0 is the exponent, and the blend time is 0. The same params are used in EnterAircraft.
6. Movement stepping: the Athena pawn ctor sets `MaxSimulationTimeStep` 0.2 and `MaxSimulationIterations` 2 (C 1.2), which overrides the 0.05 / 8 in FALLING.md. The port should take the Athena values.
7. `GetAerialPhase` returns None while the glider is force-opened (A 2.8, B 3, C 3.2), so the aerial prompt (D 6) disappears after an auto-deploy; with a manual deploy it reads "To Skydive" (glider can be closed).
8. `AthenaAerialFeedback` is described in both B 3.2 and D 6; they agree.
9. The 1.8 answer to "no redeploy after landing": correct. After `EndSkydiving` a normal fall cannot open the glider (`Athena.EnableParachuteEverywhere` = 0), and the only new skydive source is a launch pad (C 4.3). In the air before landing, the glider CAN be closed and reopened freely (C 3.1).

---

---

## A. Match phases, timers, and bus flight path (Fortnite 1.8 Athena)

How to read the sources:
- Addresses are VAs in `FortniteClient-Win64-Shipping.exe` (1.8). Functions come from the Ghidra decompile (`C:\FortniteRef\decomp\named`). Where no name existed, I give the role I inferred in *italics*.
- Float constants were read from the exe image with a small PE reader (`scratchpad/bus/rf.js`). I read the file only and never ran the exe.
- Field names and offsets come from the SDK (`FN_FortniteGame_classes.hpp`, `FN_Engine_classes.hpp`) unless the text says "unnamed +0xNNN".
- Cooked values were read with `fn-inspect`. "AGD" means the CurveTable `/Game/Athena/Balance/DataTables/AthenaGameData`. Every Aircraft/SafeZone FScalableFloat is evaluated as `Value(1.0) * Curve.Eval(level 0)` (`FScalableFloat::GetValueAtLevel` 143d81440, called with level 0 everywhere below). So the effective value is the AGD key at Time=0.

---

### 1. Match state machine

#### 1.1 Enums (SDK)
- `EAthenaGamePhase`: 0 None, 1 Setup, 2 Warmup, 3 Aircraft, 4 SafeZones, 5 EndGame.
- `EAthenaGamePhaseStep`: 0 None, 1 Setup, 2 Warmup, 3 GetReady, 4 BusLocked, 5 BusFlying, 6 StormForming, 7 StormHolding, 8 StormShrinking, 9 EndGame.
- `EAthenaAerialPhase`: 0 None, 1 BusCantExit, 2 BusCanExit, 3 Skydiving, 4 Parachuting.

#### 1.2 Initial state
`AFortGameStateAthena` ctor (140413710):
- `GamePhase = Setup (1)` and `SafeZonePhase = 0`: a 16-bit store of 1 at +0x14D8.
- `WarmupCountdownStartTime`, `WarmupCountdownEndTime`, `AircraftStartTime`, `SafeZonesStartTime`, `EndGameStartTime` and `EndGameKickPlayerTime` are all set to **-1.0** (+0x13B0..+0x13C4, `0xbf800000`).

#### 1.3 Who changes GamePhase
*SetGamePhase* (FUN_14044baf0) only acts with authority (`Role == 3`) and only when the value changes. It writes `GamePhase`, calls `AFortGameStateAthena::OnRep_GamePhase(old)` (1404369d0) and then GameState vfunc +0x5A0.

| Transition | Caller | Condition |
|---|---|---|
| Setup -> Warmup | *StartWarmupPhase* FUN_1404097c0, called from `AFortGameModeAthena::HandleMatchHasStarted` (1403f78a0) | GameState valid and `bSkipWarmup` (GM +0x8C0 bit0) clear |
| Setup -> Aircraft (warmup skipped) | *StartAircraftPhase* FUN_140407e40, called from HandleMatchHasStarted when StartWarmupPhase returns 0 | `bSkipAircraft` (bit2) clear, valid MapInfo, aircraft spawns |
| Setup -> None | HandleMatchHasStarted | both of the above fail (log "Set AthenaGamePhase to [None]") |
| Warmup -> Aircraft | *UpdateWarmupPhase* FUN_14040f650 (from `AFortGameModeAthena::Tick` 14040a6b0 while GamePhase==2) -> StartAircraftPhase | countdown over and at least 1 player |
| Aircraft -> SafeZones | *StartSafeZonePhase* FUN_140409430(gm, force=0), called at the end of `OnAircraftExitedDropZone` (1403fbfc0) | `MapInfo != null`, `MapInfo.SafeZoneDefinitions.Num() > 1` (+0x648), and phase != 4 unless forced |
| -> EndGame | `AFortGameModeAthena::HandleMatchHasEnded` 1403f7590 | (outside this area) |

The match starts when `AFortGameModeAthena::ReadyToStartMatch_Implementation` (1404021d0) returns true. That requires:
- `bWorldIsReady` (AFortGameMode +0x570 bit0),
- `MatchState == WaitingToStart` (+0x428 compared against a global FName; the FName identity is inferred), and
- *NumLoadedPlayers* (FUN_1403d31f0) `>= WarmupRequiredPlayerCount`.

*NumLoadedPlayers* counts FortPlayerControllers that have a PlayerState, whose PlayerState +0x3B4 bit0 is clear (probably `bIsSpectator`), and whose PC +0x6DC bit 0x40 is set (probably a loaded flag).

So **Setup = waiting for `WarmupRequiredPlayerCount` loaded players**. During Setup, `PlayerCanRestart` returns false, so no pawns exist.

`OnRep_GamePhase` (1404369d0) does the following:
- When the new phase is in 1..3 it adjusts the net-tick cvars (server), calls PlayerPawnAthena vfunc +0x878 on every pawn and BuildingActor vfunc +0x7E8 on every building actor.
- When the new phase is **Aircraft** it calls FUN_140443ee0. That function runs `EnterAircraft(PC, Aircraft)` for every PlayerControllerAthena with `!bInAircraft && !bHasExitedAircraft`.
- It always broadcasts `GamePhaseChanged` (+0x1390).
- `AFortPlayerControllerAthena::vfunc_611` (14043c260) also calls EnterAircraft for a controller that arrives late while the phase is Aircraft.

#### 1.4 GameMode option flags and defaults
`AFortGameModeAthena::InitGame` (1403f7c00) parses each option from the URL options or the command line into a bitfield at GM +0x8C0:
- `SkipWarmup` -> bit0
- `PauseWarmup` -> bit1
- `SkipAircraft` -> bit2
- `PauseAircraft` -> bit3
- `ForceAthenaStreaming` -> bit4

It also reads `SquadPlay` (+0x8A2), `ForceDBNO` (+0x8A0), `MaxPlayers=` (+0x8E8), `MaxTeamSize=` (+0x8C4) and `TeamSpread=` (+0x8C8).
- If the world is PIE (`WorldType == 3`, FUN_142519780), `WarmupRequiredPlayerCount` is forced to 1.
- `RequiredPlayers=` (URL or command line) overrides `WarmupRequiredPlayerCount`.

`AFortGameModeAthena::vfunc_310` (1403d7eb0, map-load setup):
- Finds the placed `AFortAthenaMapInfo` and stores it in `GameState.MapInfo`. If there is none it logs "Skipping warmup and aircraft phases" and sets bits 0 and 2.
- If no `AFortPlayerStartWarmup` exists, it sets bSkipWarmup.

Constructor defaults (`AFortGameModeAthena` ctor 1403cd560). None of these are overridden in the `Athena_GameMode` BP CDO (`fn-inspect props /Game/Athena/Athena_GameMode cdo` shows only GameStateClass, PlayerControllerClass, QuickBars, StartingItems, GE_OutsideSafeZone, SafeZoneIndicatorClass and DeathPenaltyData).

| Field (offset) | Default | Source |
|---|---|---|
| WarmupRequiredPlayerCount (+0xA94) | **80** (0x50) | ctor 1403cd560; 1 in PIE; `RequiredPlayers=` override (InitGame) |
| WarmupCountdownDuration (+0xA98, Config) | **60.0** (0x42700000) | ctor |
| unnamed +0xA9C (early-countdown player fraction) | **0.95** (0x3f733333) | ctor; used in UpdateWarmupPhase |
| WarmupEarlyCountdownDuration (+0xAA0, Config) | **15.0** (0x41700000) | ctor |
| MaxPlayerCount (+0x8E8) | **100** | ctor; `MaxPlayers=` override |
| OverloadedInitialConnectTimeout (+0x8A4, Config) | 90.0 | ctor |
| FailedMatchAssignmentTimeout (+0x8A8, Config) | 120.0 | ctor |
| EndGameKickPlayersDelay (+0xAE0) | 60.0 | ctor |
| bAllowSpectateAfterDeath / bUseRandomTimeOfDay (+0xB08/+0xB09, Config) | true / true | ctor (`0x101` store) |
| unnamed +0x488 -> GameState.CurrentPlaylistId | 2 | ctor; copied in InitGame |

Config caveat: three of these fields (WarmupCountdownDuration, WarmupEarlyCountdownDuration, the time-of-day flag) are `Config`. A `[/Script/FortniteGame.FortGameModeAthena]` section in DefaultGame.ini could override them, and I could not read the ini (see UNRESOLVED).

`HandleMatchHasStarted` with `bUseRandomTimeOfDay` sets `TimeOfDayManager.SetTimeOfDayFloat(rand() * (1/32767) * 23.99)`. The constants are 143fe36ec = 3.05185e-5 and 144062898 = 23.99.

#### 1.5 Warmup countdown (`UpdateWarmupPhase` FUN_14040f650, every GM tick while GamePhase == Warmup)
Let `now = GameState.GetServerWorldTimeSeconds()` (GameState vfunc +0x5E0 = `AGameStateBase::GetServerWorldTimeSeconds` 14219a1e0) and `End = WarmupCountdownEndTime`.
1. **End < 0** (not started): if `PauseWarmup` is clear, set `WarmupCountdownStartTime = now` and `WarmupCountdownEndTime = now + WarmupCountdownDuration` (60). The countdown therefore starts on the first warmup tick.
2. **now < End**:
   - If `(End - now) > WarmupEarlyCountdownDuration` (15, strict) and `NumLoadedPlayers >= MaxPlayerCount * 0.95` (>= 95 with defaults), set `End = now + 15` (log "Player percent reached to start early countdown").
   - After that first application `End - now == 15`, so the strict `>` stops it from firing again.
3. **now > End** (strict): log "Warmup countdown is over".
   - If `NumPlayers` (GM vfunc +0x658) < 1, call GameSession vfunc +0x668 (restart session).
   - Otherwise call `StartAircraftPhase`.

Console commands (*GameMode exec* FUN_1403d6f30): `UNPAUSEWARMUP` clears bit1, `STARTAIRCRAFT`, `UNPAUSEAIRCRAFT`, `AIRCRAFTUNLOCK`, `STARTSAFEZONE`, `PAUSESAFEZONE`, `SKIPSHRINKSAFEZONE`, `STARTSHRINKSAFEZONE`, `SPAWNSUPPLYDROP`.

#### 1.6 GamePhaseStep and GamePhaseStepTimeRemaining
These are computed every `AFortGameStateAthena::Tick` (14044e6c0 -> FUN_1404506d0) on any machine where gate FUN_14041eef0 passes. Both fields lack the `Net` flag, so they are recomputed locally from replicated times. Let `now` = server world time.

| GamePhase | Condition | Step | TimeRemaining |
|---|---|---|---|
| Warmup | `WarmupCountdownStartTime < 0` | 1 Setup | 0 |
| Warmup | start >= 0 and `now < WarmupCountdownEndTime` | `r <= 10` ? 3 **GetReady** : 2 **Warmup** | `r = FloorToInt(End - now)`. Code is `ROUND(2x - 0.5) >> 1`, which is UE `FMath::FloorToInt` (UE4.16 `Windows/WindowsPlatformMath.h:44`); 10.0 = 143fe3714 |
| Warmup | otherwise | 0 | 0 |
| Aircraft | `now < Aircraft.DropStartTime` | 4 **BusLocked** | `DropStartTime - now + 1.0` (1.0 = 143fe36fc) |
| Aircraft | `DropStartTime <= now < DropEndTime` | 5 **BusFlying** | `DropEndTime - now` |
| Aircraft | otherwise / no aircraft | 0 | 0 |
| SafeZones | no SafeZoneIndicator, `now < SafeZonesStartTime` | 6 StormForming | `SafeZonesStartTime - now` |
| SafeZones | indicator: `SafeZoneStartShrinkTime - now > 0` | 7 StormHolding | that value |
| SafeZones | indicator: `SafeZoneFinishShrinkTime - now > 0` | 8 StormShrinking | that value |
| EndGame | `now < EndGameKickPlayerTime` | 9 EndGame | `EndGameKickPlayerTime - now` |

When the step changes, the function writes `GamePhaseStep` and broadcasts `GamePhaseStepChanged` (+0x13A0).

#### 1.7 Warmup spawn island (exists in 1.8)
- `/Game/Athena/Maps/Athena_Terrain` streams `/Game/Athena/Maps/POI/Athena_POI_Lobby_001` through `LevelStreamingAlwaysLoaded_7` (export 109) with `LevelTransform.Translation = (-125440, -113664, 3840)`.
- Lobby_001 contains **110 `FortPlayerStartWarmup`** actors. Their CollisionCapsule RelativeLocation spans local x [-18762.7, 9177.3], y [-14769.2, 10558.8], z [-223.3, 1494.6]. That is world x [-144202.7, -116262.7], y [-128433.2, -103105.2], z [3616.7, 5334.6], outside `IslandZoneVolume`.
- Yaws are multiples of 45 degrees and also -67.5. RelativeRotation is pitch 0, roll 0.
- The level also holds 72 `Tiered_Athena_FloorLoot_01_C` and 1 `Tiered_Chest_6_Parent_C`. These are warmup loot actors; their contents are not in this area.
- `Athena_Faceoff.umap` has 6 warmup starts; it is not the BR map.
- No asset named `*SpawnIsland*` or `*Playlist*` exists in the 1.8 paks (`fn-inspect ls`).

Spawning during Setup/Warmup:
- `PlayerCanRestart_Implementation` (140400380) returns false in Setup (1), SafeZones (4) and EndGame (5). It returns true in None (0) and Warmup (2) when the parent allows. In Aircraft (3) see section 3.
- `ChoosePlayerStart_Implementation` (1403d2d40): in phase 1 or 2 it calls *ChooseWarmupStart* (FUN_1404364a0).
  - It collects all `AFortPlayerStartWarmup` actors and picks a random starting index (`FRand * N`).
  - Walking cyclically through all N, it keeps the start with the smallest `PlayerClaimedCount` (unnamed +0x3B8; strict `<`, so ties go to the first one seen from the random start).
  - It caches the start in `PC.WarmupPlayerStart` (+0x2FC0) and increments the claimed count. Later warmup respawns reuse the cached start.
  - In phase 3 it returns the Aircraft actor (FUN_1403d2ce0 -> GameState.Aircraft). Otherwise it uses the base chooser in PIE only.
- `FinishRestartPlayer` (1403d7bc0): while GamePhase == Warmup the new pawn gets `bCanBeDamaged = false` (AActor +0x148 bit1 cleared; SDK `FN_Engine_classes.hpp:65`). Warmup is invulnerable.

What happens to warmup players when the bus phase starts (`StartAircraftPhase` FUN_140407e40):
1. The first time only, it runs *RemoveMissingPlayersFromSession* (FUN_1404035c0).
2. It spawns and initializes the aircraft (section 2) and starts the flight at `now`, unless `PauseAircraft` is set.
3. It binds the GM delegates `OnAircraftEnteredDropZone`, `OnAircraftExitedDropZone` and `OnAircraftFlightEnded` to the aircraft multicast delegates at +0x388, +0x398 and +0x3A8.
4. *SetAircraft* (FUN_14044c0c0): `GameState.Aircraft = ac`, `bAircraftIsLocked = 1`, unnamed +0x13E8 = 0. Then `GameState.AircraftStartTime = now`.
5. For every PlayerControllerAthena with a PlayerPawnAthena: call FUN_1407d94d0(pawn, 0) (unnamed), `Destroy(pawn)`, `APlayerController::Reset`, then `ClientGotoState(NAME_Spectating)`. FName index 0x142 = 322 = `Spectating` (UE4.16 `UObject/UnrealNames.inl:159`).
6. It destroys every `ABuildingActor` with `IsPlayerBuilt()` (vfunc +0x7A0), then calls GameSession vfunc +0x780.
7. It calls `SetGamePhase(Aircraft)`. OnRep then puts everyone into the aircraft through EnterAircraft (section 1.3).

---

### 2. Bus flight path

#### 2.1 Cooked inputs
MapInfo instance: `/Game/Athena/Maps/Athena_Terrain` export 24 `DefaultMapInfo_4` (class `DefaultMapInfo_C`). It inherits from the CDO `/Game/Athena/Balance/MapInfos/DefaultMapInfo` `Default__DefaultMapInfo_C`.

| Property | Effective value | Source |
|---|---|---|
| AircraftClass | `/Game/Athena/Aircraft/AthenaAircraft.AthenaAircraft_C` | DefaultMapInfo CDO |
| MapInfo actor location | (32224, -25744, 1312) | Athena_Terrain export 120 `RootComponent0.RelativeLocation` |
| AircraftSpawnZone | Min (-160000,-160000), Max (160000,160000), *relative to the MapInfo location* | instance override (CDO had +-150000) |
| AircraftDropZone | Min (-110000,-110000), Max (110000,110000) | instance (CDO +-80000). **Not used for the path** because a drop volume is set |
| AircraftDropVolume | `IslandZoneVolume` (TriggerVolume, export 164) | instance |
| IslandZoneVolume | BrushComponent0 location (32512, -24192, 56320), no rotation or scale. `BodySetup_50` (export 5) has 8 convex elements. The union of the local hull vertices is x [-105472, 105472], y [-112256, 112256], z [-75000, 75000] | exports 18, 5 |
| AircraftHeight | AGD `Default.Aircraft.Height` = **80000** | CDO + AGD |
| AircraftSpeed | AGD `Default.Aircraft.Speed` = **6000** cm/s | CDO + AGD |
| AircraftDeviationAngle | AGD `Default.Aircraft.DeviationAngle` = **45** deg | CDO + AGD |
| SafeZoneStartDelay | AGD `Default.SafeZone.StartDelay` = **60** s | CDO + AGD |
| SkydivingScanForImpactWorldZ / ForceDeployParachuteWorldZ | 20000 / 0 | CDO + AGD |
| AthenaAircraft_C CDO | `ExitLocation = (-580, 0, 200)`, `CameraModeClass = AthenaAircraft_CameraMode_C`, `DefaultBusSkin = BBID_DefaultBus` | `/Game/Athena/Aircraft/AthenaAircraft` cdo |

All AGD rows listed have keys at Time 0..20. Only the Time 0 key is non-zero, and level 0 is the only level evaluated.

`AFortAthenaAircraft` native ctor defaults (1403cc7a0):
- `CameraModeClass = UFortCameraMode_AthenaAircraft`, `CameraInitialRotation = (Pitch -30, 0, 0)` (1440628ac = -30.0), `ExitLocation = 0`.
- FlightStart/End/DropStart/DropEnd and the three relative offsets (+0x40C/+0x410/+0x414) = -1.
- `MiniMapIconScale = 0.5`, collision disabled, flags at +0x418/+0x419 = 0.

#### 2.2 Validation (*IsMapInfoValid* FUN_1403fa200)
It fails when:
- there is no MapInfo,
- AircraftClass is not an AFortAthenaAircraft,
- `AircraftSpeed <= 0`, or
- the SpawnZone has `Min >= Max` on any axis.

If `AircraftDropVolume` is set it returns true at this point. Otherwise it also requires the DropZone to have `Min < Max` and to lie strictly inside the SpawnZone.

#### 2.3 Spawn (*SpawnAircraft* FUN_140406dc0)
`UWorld::SpawnActor(MapInfo.AircraftClass, ...)`, then *InitializeFlightPath*(aircraft, MapInfo) (FUN_1403f92f0), then `SetActorTransform(Rotation = FlightStartRotation, Location = FlightStartLocation, Scale = 1)` (FUN_141f34ac0).

#### 2.4 *InitializeFlightPath* (FUN_1403f92f0)
Let `M` = MapInfo root location (or 0 if no root). All math is 2D except Z.
1. Spawn box `S = [M.xy + SpawnZone.Min, M.xy + SpawnZone.Max]`. Drop box `D = [M.xy + DropZone.Min, M.xy + DropZone.Max]`. Flight height `Z = AircraftHeight + M.z`.
2. Drop circle:
   - With a DropVolume: `(C, E) = AActor::GetActorBounds(volume)` (origin, extent).
   - Otherwise: `C = center(D)` and `E = half-size(D)` (0.5 = 143feb1d8).
   - `R = max(E.x, E.y)`.
3. `a1 = rand() * (360/32767)` degrees (144062858 = 0.0109867).
   `P1 = (C.x + R*sin(a1), C.y + R*cos(a1))`. SinCos is UE `FMath::SinCos` (UE4.16 `Math/UnrealMathUtility.h:315`); deg->rad 0.0174533 = 143feb1c8. Note X uses **sin** and Y uses **cos**.
4. `dev = AircraftDeviationAngle` and `a2 = a1 + 180 + Lerp(-dev, +dev, FRand())` degrees (180 = 143ff9038; FRand = `rand()/32767`).
   `P2 = (C.x + R*sin(a2), C.y + R*cos(a2))`.
5. *ProjectPathOntoZone* (FUN_140401410) intersects the **infinite** line P1->P2 with box `S` using slab clipping with parameter t along `P2-P1`, unclamped. It outputs `Start = P1 + tmin*(P2-P1)` and `End = P1 + tmax*(P2-P1)`. It fails with a log if |dx| and |dy| are both <= 1e-8, or if the line misses the box.
6. Drop entry and exit:
   - **With a DropVolume** (*ProjectPathOnZoneVolume* FUN_140401730): `dir = normalize(P2-P1)` and `L = 2*sqrt(E.x^2+E.y^2) + 1000` (2.0 = 143ff9020, 1000 = 143fe3734).
     - Trace 1: from `(P2 - dir*L, Z)` to `(P2, Z)` against the volume's BrushComponent (`LineTraceComponent`, vfunc +0x798). `Enter = hit.Location.xy`.
     - Trace 2: from `(P1 + dir*L, Z)` to `(P1, Z)`. `Exit = hit.Location.xy`.
     - On a miss, Enter and Exit stay P1 and P2 and a warning is logged.
     - The hit is against the 8-hull island polygon (BodySetup_50 vertex data), not the AABB.
   - Without a volume: run *ProjectPathOntoZone* on box `D`.
7. Outputs. `u = normalize(End - Start)` (zero vector if length <= 1e-8 at 143ff9008), `len = |End - Start|`, `v = AircraftSpeed`.
   - Flight/Drop absolute times reset to -1.
   - `FlightDuration` (unnamed +0x40C) = `len / v`.
   - `FlightStartLocation = (Start.x, Start.y, Z)`.
   - `FlightStartRotation = (Pitch 0, Yaw = Atan2(u.y, u.x) * 57.29578, Roll 0)`. `FGenericPlatformMath::Atan2` is 141254aa0; 14406289c = 57.2958. The decompiler dropped the argument order; Atan2(Y, X) is inferred from yaw semantics.
   - `FlightSpeed = v`.
   - `DropStartOffset` (+0x410) = `max(0, dot(Enter - Start, u) / v)`.
   - `DropEndOffset` (+0x414) = `max(DropStartOffset, dot(Exit - Start, u) / v)`.

Values these inputs give (derived, not measured in game):
- Height Z = 81312.
- Drop circle center (32512, -24192), R = 112256.
- Spawn box world x [-127776, 192224], y [-185744, 134256].
- The chord's central angle is 135..225 degrees, so the line passes within `R*cos(67.5 deg) ~ 42960` of C. Flight length is therefore about one box width to one box diagonal (~320000 to 452548 cm), or about **53 to 75 s** at 6000 cm/s.

#### 2.5 *StartFlightPath*(aircraft, t0) (FUN_140408ef0)
This runs only if `FlightStartTime < 0` (otherwise it logs "Flight was already started"):
- `FlightStartTime = t0`
- `FlightEndTime = t0 + FlightDuration`
- `DropStartTime = t0 + DropStartOffset`
- `DropEndTime = t0 + DropEndOffset`

It is called from StartAircraftPhase with `t0 = now` unless `PauseAircraft` is set. In that case the `UNPAUSEAIRCRAFT` exec starts it.

#### 2.6 `AFortAthenaAircraft::Tick` (14040a4d0)
- Returns early if `bFlightEnded` (+0x419), if `FlightStartTime < 0`, or if there is no GameStateAthena.
- `now = GameState.GetServerWorldTimeSeconds()`.
- **Every machine** sets the location: `Location = FlightStartLocation + FlightStartRotation.Vector() * FlightSpeed * (now - FlightStartTime)`. FRotator::Vector is FUN_1412f4470; SetActorLocation is FUN_141f48200 with no sweep and no teleport.
- **Authority only** (`Role == 3`):
  - `bInDropZone` (+0x418) = `DropStartTime < now && now < DropEndTime`.
  - A rising edge broadcasts the delegate at +0x388 (EnteredDropZone). A falling edge broadcasts +0x398 (ExitedDropZone).
  - If `now > FlightEndTime`: set `bFlightEnded = 1`. If still in the drop zone, clear it and broadcast ExitedDropZone. Then broadcast +0x3A8 (FlightEnded).

`AFortAthenaAircraft::GetVelocity` (1403f6da0) = root component world rotation applied to `(FlightSpeed * scale.x, 0, 0)`, i.e. `forward * FlightSpeed`.

#### 2.7 GameMode delegate handlers
- `OnAircraftEnteredDropZone` (1403fbf70) calls *SetAircraftLocked*(GameState, false) (FUN_14044b670): clears `bAircraftIsLocked` and copies `PlayersLeft` (+0x13C8) into unnamed +0x13E8 on the unlock edge. **The bus door opens at DropStartTime.** BusLocked therefore lasts `DropStartOffset` seconds; it is not a fixed constant.
- `OnAircraftExitedDropZone` (1403fbfc0), the **auto-kick**. For every PlayerControllerAthena:
  - If `IsInAircraft` (+0x31B5, 1404389a0): call GM vfunc +0x728. The vtable 144061988 slot holds 1404044c0 = `AFortGameModeAthena::RestartPlayer`, so the player is force-restarted at the aircraft. The decompiler dropped the PC argument.
  - Otherwise, if `!bHasExitedAircraft` (+0x31B6) or PC +0x6DC bit 0x40 is clear: log "failed to load before aircraft phase ended" and kick through GameSession vfunc +0x648 with "Failed to load before game started."
  - Then `StartSafeZonePhase(gm, false)`, which sets `SafeZonesStartTime = now + SafeZoneStartDelay (60)` and `SetGamePhase(SafeZones)`, and starts a 1.0 s looping timer on handle GM +0x8D0 calling FUN_140404530 (safe-zone area).
- `OnAircraftFlightEnded` (1403fc1e0) unbinds the three delegates from `GameState.Aircraft` and `Destroy`s the aircraft.

#### 2.8 `AFortGameStateAthena::GetAerialPhase(PC)` (140427820)
- If the PC has a PlayerPawnAthena:
  - `!bIsSkydiving` (+0x18E9) -> None.
  - `bIsParachuteForcedOpen` (+0x18EB) -> None.
  - Otherwise `3 + bIsParachuteOpen` (+0x18EA): Skydiving or Parachuting.
- Otherwise, if `GamePhase == Aircraft && Aircraft`: `now < DropStartTime` -> BusCantExit; `now < DropEndTime` -> BusCanExit; otherwise None.

---

### 3. Entering, jumping and exiting

#### 3.1 EnterAircraft (`AFortPlayerControllerAthena::EnterAircraft` 1404251d0)
It warns if `bHasExitedAircraft`. If not already `bInAircraft`:
1. `PushInputComponent(AircraftInputComponent)` (+0x2FC8).
2. Camera `SetViewTarget(aircraft)` with transition params BlendTime 0, BlendFunction 1, BlendExp 2.0 (143ff9020).
3. Set the camera mode to `aircraft.CameraModeClass` if it is a `UFortCameraMode_AthenaAircraft`.
4. `SetControlRotation(aircraftWorldRotation + CameraInitialRotation)` via FUN_140427b10, i.e. (pitch -30, flight yaw, 0).
5. Zero PC +0x784..+0x78C.
6. `bInAircraft = 1`, `bHasExitedAircraft = 0` (16-bit store 0x0001 at +0x31B5).
7. Broadcast `OnAircraftStateChange` (+0x2840).
8. If local, run FUN_1403d1850 (cosmetic, unnamed).

#### 3.2 Input binding
`AFortPlayerControllerAthena::SetupInputComponent` (14044c0e0) creates `AircraftInputComponent0` with these bindings:
- Axes `Turn` (string at 144062bc4), `Turn_Gamepad`, `LookUp`, `LookUp_Gamepad`.
- Action **`Jump`** (string at 144063634), `IE_Pressed` -> *OnAircraftJumpPressed* FUN_14041ce10.
- Action `EscapeKey` -> FUN_140836fa0.

FUN_14041ce10 checks that GameStateAthena exists, `bInAircraft`, `Aircraft != null` and `!bAircraftIsLocked`. If all hold it calls `ServerAttemptAircraftJump(GetControlRotation())` (vfunc +0x5F0).

#### 3.3 Server RPC
- `ServerAttemptAircraftJump_Validate` (14044ac00) requires GameStateAthena, `Aircraft != null` and `!bAircraftIsLocked`. Note that it does **not** check bInAircraft or the time window.
- `ServerAttemptAircraftJump_Implementation` (14044ab90):
  1. Set unnamed +0x31B7 (*bAttemptingAircraftJump*) to 1.
  2. If `bInAircraft`: call vfunc +0x5F8 (PC vtable 144070898 + 0x5F8 = 140849a00 `AFortPlayerController::SetControlRotation`; the decompiler dropped the argument, presumably ClientRotation), store `ClientRotation` at PC +0x31A0, and call vfunc +0x7D8. PC vtable 144070898 + 0x7D8 holds 1423a4e40 = `APlayerController::ServerRestartPlayer_Implementation`.
  3. Set +0x31B7 back to 0.

The restart path is:
1. `PlayerCanRestart` in Aircraft phase requires `bInAircraft && bAttemptingAircraftJump && GameState.Aircraft && !bAircraftIsLocked`.
2. `RestartPlayer` (1404044c0) clears PC +0x6DC bit 0x20 and calls FUN_140813db0. That function records a location from a weak actor at PC +0x3CC; this looks like the respawn/spectate target.
3. `AFortGameMode::RestartPlayer`.
4. `ChoosePlayerStart` returns the Aircraft actor.
5. `FinishRestartPlayer` (1403d7bc0), when `bInAircraft && bAttempting`:
   - uses the stored ClientRotation as the restart rotation;
   - after the pawn spawns, sets the pawn actor rotation to `(0, ClientYaw, 0)` (FUN_141f48dd0) and calls `SetControlRotation(ClientRotation)` and `ClientSetRotation`.

Bots: *BotAircraftJump* (FUN_14040af20) picks a jump delay. If its configured delay is negative it uses `FRand * 30` s (143fe3724 = 30). The timer starts when the bus is unlocked.

#### 3.4 ExitAircraft
`AFortPlayerControllerAthena::Possess` (140444480) and `PawnClientRestart` (140443df0) call FUN_14043c050, which calls `ExitAircraft` (1404260a0) if the controller is in the aircraft. ExitAircraft does the following:
1. `bInAircraft = 0`, `bHasExitedAircraft = 1` (16-bit 0x0100 at +0x31B5).
2. `PopInputComponent(AircraftInputComponent)` (1423a12b0).
3. Camera SetViewTarget(pawn) with the same blend params, and clear the camera mode override.
4. Pawn *BeginSkydiving* (FUN_14041dd60):
   - `bIsSkydiving = 1`, unnamed +0x1729 = 0, `bIsBelowAutoDeployTestHeight = 0`.
   - CMC `SetMovementMode(MOVE_Custom = 6, custom 2 or 3)`; the choice depends on global DAT_14521d994.
   - On the server, store the start location at +0x1BCC and `now - AircraftStartTime` at +0x1BD8.
5. **Launch velocity: `CMC.Velocity` (+0x104) = `Aircraft.GetVelocity()`**, i.e. bus forward * 6000, then CMC vfunc +0x420.
6. If local: `PlaySound2D(AudioOnExitAircraft)` (+0x28C8).
7. Broadcast `OnAircraftStateChange`, then `FixupInteractionWidgetsOnUnzoom`.
8. On authority, increment `Aircraft.JumpFlashCount` (+0x3B8). Its OnRep plays the jump effect `PlayEffectsForPlayerJumped`.

---

---

## B. Battle Bus actor, bus camera, bus-phase HUD and audio (Fortnite 1.8, Athena only)

Citation conventions:
- `fn:<Name>@<VA>` is a function in the decompile (`C:\FortniteRef\decomp\named`).
- `rdata:<VA>` is a constant read from `FortniteClient-Win64-Shipping.exe` .rdata (read with a PE reader, file not run).
- `asset:<pkg>[:<export>].<prop>` is a value from `fn-inspect props/exports/disasm`.
- `SDK:<file>:<line>` is a line in `C:\FortniteRef\Fortnite_1.8.0_SDK\extracted\SDK`.
- `UE:<path>:<line>` is a line in `C:\UE4.16`.

Kismet offsets quoted from `fn-inspect disasm` are the tool's listing offsets. They do not always match the in-memory `EntryPoint` numbers, so control flow below is reconstructed from node order and event names, not from jump numbers.

---

### B1. The bus actor

#### B1.1 Class chain and native layout

| Class | Parent | Source |
|---|---|---|
| `AthenaAircraft_C` (`/Game/Athena/Aircraft/AthenaAircraft`) | `/Script/FortniteGame.FortAthenaAircraft` | asset:AthenaAircraft exports[5].super |
| `AFortAthenaAircraft` (size 0x4C0) | `AActor` | SDK:FN_FortniteGame_classes.hpp:11432-11454 |
| `B_StockBattleBus_C` (`/Game/Athena/Aircraft/Skins/B_StockBattleBus`) | `/Script/FortniteGame.FortClientOnlyActor` | asset:B_StockBattleBus exports[16].super |
| `UAthenaBattleBusItemDefinition` | `UAthenaCosmeticItemDefinition`; field `TAssetPtr<UClass> BusPrefabClass` @0x2B0 | SDK:FN_FortniteGame_classes.hpp:19619-19622 |

AFortAthenaAircraft fields. Offsets come from the SDK. The three delegate slots were identified from code.

| Off | Field | Meaning / evidence |
|---|---|---|
| 0x388 | (delegate, unnamed in SDK) | `OnAircraftEnteredDropZone`. StartAircraftPhase binds `AFortGameModeAthena::OnAircraftEnteredDropZone` to aircraft+0x388 (fn:FUN_140407e40@140407e40, lines 84-93) |
| 0x398 | (delegate) | `OnAircraftExitedDropZone` (same fn, lines 94-103) |
| 0x3A8 | (delegate) | `OnAircraftFlightEnded` (same fn, lines 104-115) |
| 0x3B8 | `JumpFlashCount` (Net) | incremented by ExitAircraft on authority (fn:AFortPlayerControllerAthena__ExitAircraft@1404260a0, last block) |
| 0x3C0 | `CameraModeClass` | |
| 0x3C8 | `CameraInitialRotation` | |
| 0x3D4 | `ExitLocation` | |
| 0x3E0 | `FlightStartLocation` (Net, NetQuantize100) | |
| 0x3EC | `FlightStartRotation` (Net) | |
| 0x3F8 | `FlightSpeed` (Net) | |
| 0x3FC/0x400/0x404/0x408 | `FlightStartTime/FlightEndTime/DropStartTime/DropEndTime` (Net) | |
| 0x40C..0x414 | 3 floats, unnamed | ctor sets them to -1.0 (fn:AFortAthenaAircraft__AFortAthenaAircraft@1403cc7a0); no use found |
| 0x418 | bool, unnamed | "doors open" flag (Tick, below) |
| 0x419 | bool, unnamed | "flight ended" flag (Tick) |
| 0x41C | `MiniMapIconScale` | |
| 0x420 | `MiniMapIconBrush` (FSlateBrush 0x90) | |
| 0x4B0 | `DefaultBusSkin` | |
| 0x4B8 | `SpawnedCosmeticActor` | |

#### B1.2 Native constructor defaults (fn:AFortAthenaAircraft__AFortAthenaAircraft@1403cc7a0)

| Field | Native default | Evidence |
|---|---|---|
| RootComponent | `USceneComponent` default subobject named `RootComponent0` | ctor `CreateDefaultSubobject(..., "RootComponent0", USceneComponent)` |
| Collision | `SetActorEnableCollision(false)` | ctor |
| CameraModeClass | `UFortCameraMode_AthenaAircraft::StaticClass()` | ctor |
| CameraInitialRotation | Pitch = rdata:1440628ac = **-30.0**, Yaw 0, Roll 0 | ctor writes +0x3C8 from 1440628ac, then zeros +0x3CC/+0x3D0 |
| ExitLocation | (0,0,0) (copied from zero-init global 146683fe0) | ctor |
| FlightStartLocation/Rotation | zero | ctor |
| FlightSpeed | 0 | ctor |
| FlightStartTime, FlightEndTime, DropStartTime, DropEndTime, +0x40C,+0x410,+0x414 | **-1.0** (0xBF800000) | ctor |
| MiniMapIconScale | **0.5** (0x3F000000) | ctor |
| MiniMapIconBrush | ImageSize 32x32 (0x42000000 x2), DrawAs byte = 3, tint from global 1451987f0 = (1,1,1,1) | ctor; rdata:1451987f0 |
| bools 0x418/0x419 | 0 | ctor |

#### B1.3 BP CDO overrides: `Default__AthenaAircraft_C`

| Property | Value | Source |
|---|---|---|
| CameraModeClass | `/Game/Athena/Aircraft/AthenaAircraft_CameraMode.AthenaAircraft_CameraMode_C` | asset:AthenaAircraft cdo |
| DefaultBusSkin | `/Game/Athena/Items/Cosmetics/BattleBuses/BBID_DefaultBus.BBID_DefaultBus` | asset:AthenaAircraft cdo |
| ExitLocation | (-580, 0, 200) | asset:AthenaAircraft cdo |
| MiniMapIconBrush | ImageSize (256,256), ResourceObject `/Game/Athena/Aircraft/T_AthenaBus_Top.T_AthenaBus_Top` | asset:AthenaAircraft cdo |
| RootComponent | `Default__AthenaAircraft_C:RootComponent0` (plain SceneComponent, no props) | asset:AthenaAircraft exports[8] |
| SCS | one node `DefaultSceneRoot` (SceneComponent) as DefaultSceneRootNode only, no other components | asset:AthenaAircraft SCS_Node_0 / SimpleConstructionScript_0 |

**The aircraft actor has no mesh, audio, or particle components.** All visuals and sounds come from the client-side cosmetic actor (B1.5).

BP functions of AthenaAircraft_C (asset:AthenaAircraft disasm):
- `UserConstructionScript` is empty.
- `PlayEffectsForPlayerJumped` (overrides the native BlueprintImplementableEvent) only calls `ExecuteUbergraph_AthenaAircraft`. That ubergraph is 9 bytes long and contains only `computedjump` and `return`, so **the jump effect is a no-op in 1.8**.

#### B1.4 Native runtime behaviour

**BeginPlay** (fn:AFortAthenaAircraft__BeginPlay@1403d2020):
- Calls AActor::BeginPlay.
- If not a dedicated server (`FUN_141f3fea0(this) != 1`), creates a `UFortMiniMapIndicator` through FUN_1408ecfb0@1408ecfb0. That helper gets the world's minimap manager, allocates `UFortMiniMapIndicator`, attaches it to the actor with a zero offset, and calls FUN_1408d9440(ind, 4). The meaning of the 4 is unresolved.
- Binds three delegates on the indicator:
  1. Brush getter at LAB_1403f5c90. Bytes `lea rax,[rcx+0x420]; ret` return `&MiniMapIconBrush`.
  2. Rotation getter at `AStrategicBuildingActor::GetDeathEffectsRotation`@1404e92a0 (code-folded). It returns the root component's world rotation.
  3. Scale getter at LAB_1403f5c70. Bytes `movss xmm0,[rcx+0x41C]`, stored to both x and y, return FVector2D(MiniMapIconScale, MiniMapIconScale).
- The bus therefore appears on the minimap and full map as `T_AthenaBus_Top`, rotated by the actor's world rotation and scaled by 0.5.

**Tick** (fn:AFortAthenaAircraft__Tick@14040a4d0):
1. Return early if the "flight ended" flag (0x419) is set, or if FlightStartTime < 0.
2. Require the world GameState to be an `AFortGameStateAthena`. `t = GameState->GetServerWorldTimeSeconds()` (vfunc +0x5E0).
3. Compute `Location = FlightStartLocation + FlightStartRotation.Vector() * FlightSpeed * (t - FlightStartTime)` and call SetActorLocation (FUN_141f48200). Rotation is never changed here.
4. Authority only (Role == 3):
   - `doorsOpen = (DropStartTime < t) && (t < DropEndTime)`, stored at 0x418.
   - On a false-to-true transition, broadcast delegate 0x388 (EnteredDropZone).
   - On a true-to-false transition, broadcast delegate 0x398 (ExitedDropZone).
   - If `t > FlightEndTime` (strict), set 0x419. If the doors were still open, clear the flag and broadcast 0x398. Then broadcast 0x3A8 (FlightEnded).

**GetVelocity** (fn:AFortAthenaAircraft__GetVelocity@1403f6da0): root component rotation applied to `(FlightSpeed * RootScale.X, 0, 0)`. This is the bus forward velocity. ExitAircraft uses it (B2.3).

**OnRep_JumpFlashCount**: the exec thunk at 140b215e0 jumps to 1403fcf50, which jumps to fn:AFortAthenaAircraft__PlayEffectsForPlayerJumped@140a40050. That function does `FindFunctionChecked` and `ProcessEvent` on the BP event, and the BP event is empty (B1.3).

#### B1.5 Cosmetic bus actor (what you actually see and hear)

**Spawn path:** FUN_1403d1850@1403d1850, which this document calls `AFortAthenaAircraft::SpawnCosmeticActor(BusItemDef)`.
- Its only caller is fn:AFortPlayerControllerAthena__EnterAircraft@1404251d0, and only when `IsLocalController()` (PC vfunc +0x660) is true.
- It runs client-side, once per local player, on entering the bus.

Steps:
1. If SpawnedCosmeticActor (0x4B8) exists, Destroy it and null it.
2. `ItemDef = arg (PC.CustomizationLoadout.BattleBus, PC+0x2928) ?: DefaultBusSkin (0x4B0)`. The loadout offset is from SDK:FN_FortniteGame_structs.hpp:7016-7026 (BattleBus at loadout+0x50) and SDK:FN_FortniteGame_classes.hpp:17059 (CustomizationLoadout @0x28D8).
   - `Athena_PlayerController_C` CDO's CustomizationLoadout sets only Glider and Pickaxe (asset:/Game/Athena/Athena_PlayerController cdo), so BattleBus is null and **DefaultBusSkin = BBID_DefaultBus is used**.
3. `UAthenaBattleBusItemDefinition::GetBusPrefabClass` loads `BusPrefabClass` synchronously. It must be an AActor subclass.
4. `SpawnActor(class)` at the default transform, then `SetActorEnableCollision(false)`.
5. `AttachToComponent(aircraft RootComponent, rules rdata/data:1451bb798 = {2,2,2,0})`. That is SnapToTarget for location, rotation and scale, with no weld.
6. FUN_14065bc60@14065bc60 applies the item's `MaterialOverrides`:
   - It iterates every component of the spawned actor.
   - Per component, FUN_14065bd60@14065bd60: for each override (ItemDef+0x2A0, stride 0x30), if `ComponentName` is None or equals the component's FName, call `SetMaterial(MaterialOverrideIndex, OverrideMaterial.LoadSynchronous())`.
   - The struct layout is from SDK:FN_FortniteGame_structs.hpp:9649-9655.

**Battle Bus item definitions:**

| Asset | BusPrefabClass | DisplayName (key / ns) | Other |
|---|---|---|---|
| `/Game/Athena/Items/Cosmetics/BattleBuses/BBID_DefaultBus` | `/Game/Athena/Aircraft/Skins/B_StockBattleBus.B_StockBattleBus_C` | "Battle Bus" (ADFFD42F42D1A29609E8C7AD95B07E4D / [7BDE3422404EB11638470588E79A28D8]) | Small/LargePreviewImage `/Game/Athena/Textures/Icons/BattleBuses/Icon-DefaultBus` |
| `/Game/Athena/Items/Cosmetics/BattleBuses/BBID_PurpleBus` | same prefab | "Purple Battle Bus" (F3DFF673450DC68A639549BC84D9318B) | MaterialOverrides: `{ComponentName "Bus", index 0, /Game/Athena/Environments/Materials/MI_Athena_Bus_Purple}`; icon `Icon-PurpleBus` |

**B_StockBattleBus_C components.** All are StaticMeshComponents except the scenes. There is **no SkeletalMesh, no AnimBP, no ParticleSystemComponent, and no AudioComponent template**: audio is spawned at runtime (below).

Hierarchy: `Scene` (root, SCS_Node_4) > `Bus` (SCS_Node_1) > {`Balloon`, `Apparatus` > `BalloonFlame`, `LaunchTheBalloon_van_generator`}. Source: asset:B_StockBattleBus SCS_Node_0..6 ChildNodes/RootNodes.

| Component | Mesh | Relative transform | Collision / flags | Material (import table) |
|---|---|---|---|---|
| Bus | `/Game/Athena/Aircraft/Meshes/Vehicle_Athena_Bus` | Rot (P0, **Y-90**, R0) | BodyInstance WalkableSlope_Unwalkable; bAffectDistanceFieldLighting=false; bLightAttachmentsAsGroup=true | `/Game/Athena/Aircraft/Materials/MI_Athena_Bus_Blue`. That MI's parent is `/Game/Vehicles/FORT_Vehicles_01/Materials/MI_CopCar`; it uses T_Athena_Bus_D/N/S and Vehicle_SchoolBus_01_E1/e2 (package import strings) |
| Balloon | `/Game/Athena/Aircraft/Meshes/Vehicle_Athena_Bus_Balloon_Full` | identity | NoCollision; Visibility+Camera ignore; no DF lighting | `/Game/Missions/Primary/LaunchTheBalloon/Materials/M_LauchTheBalloon_Apparatus` |
| Apparatus | `/Game/Athena/Aircraft/Meshes/Vehicle_Athena_Bus_Apparatus` | identity | NoCollision; OverrideMaterials [null] | `M_LauchTheBalloon_Apparatus` |
| BalloonFlame (child of Apparatus) | `/Game/Missions/Primary/LaunchTheBalloon/Meshes_NoLOD/BalloonFlame` | Loc (0, -32, 520), Rot ~0, Scale 1.6 | NoCollision | `/Game/Missions/Primary/LaunchTheBalloon/Materials/M_BalloonBurner_Flame` |
| LaunchTheBalloon_van_generator | `/Game/Missions/Primary/LaunchTheBalloon/Meshes_NoLOD/LaunchTheBalloon_van_generator` | Loc (76.5, 506.0, 360), Rot (P0, Y-140.70, R0) | NoCollision | `.../Materials/M_LauchTheBalloon_Generator` |

Mesh bounds, from each mesh export's ExtendedBounds:
- Bus: origin (19.8, 22.4, 170.9), extent (180.7, 641.6, 171.3).
- Balloon: origin (0.9, -24.3, 1068.7), extent (374.5, 404.4, 687.1).
- Apparatus: origin (12.9, 143.6, 286.4), extent (204.9, 607.8, 290.5).

The bus mesh is modelled along +Y, which is why `Bus` has Yaw -90. The "hanging" look is purely static: the balloon mesh sits above the bus at Z≈1070, and nothing animates it. No movement or anim component was found in any export.

Mesh material slot names (StaticMaterials.MaterialSlotName): Bus "M_Vehicle_SchoolBus_01"; Apparatus and Balloon "mat_app_01". fn-inspect returns `MaterialInterface: null`. The material paths above come from the package import strings (each mesh package imports exactly one material).

**B_StockBattleBus_C CDO** (asset:B_StockBattleBus Default__B_StockBattleBus_C):
- `LoopingSound = /Game/Athena/Sounds/Vehicles/Athena_Bus_Flying_Loop_Cue`
- `LoopingMusic = /Game/Athena/Sounds/Vehicles/Athena_Bus_Music_Loop_Cue`

**Graph** (asset:B_StockBattleBus disasm): `ReceiveBeginPlay` calls the custom event `Start Audio`, which runs:
1. `SpawnSoundAttached(LoopingSound, Bus component, socket None, loc 0, rot 0, KeepRelativeOffset, bStopWhenAttachedToDestroyed=false, Vol 1, Pitch 1, Start 0, no attenuation override, no concurrency)`, then `FadeIn(2.0, 1.0, 0.0)`.
2. The same for LoopingMusic, also `FadeIn(2.0, 1.0, 0.0)`.

`UserConstructionScript` is empty.

---

### B2. Players on the bus

#### B2.1 How players get onto the bus

- **Server, StartAircraftPhase** (fn:FUN_140407e40@140407e40, log text "AFortGameModeAthena::StartAircraftPhase: Initiating aircraft phase"):
  1. Spawns the aircraft and binds the three delegates (B1.1).
  2. FUN_14044c0c0 is called with (GameState, aircraft). It presumably sets GameState.Aircraft; this is not verified in detail.
  3. For every `AFortPlayerControllerAthena`:
     - If its Pawn is an `AFortPlayerPawnAthena`, call FUN_1407d94d0(pawn, 0). Purpose unresolved.
     - **Destroy the pawn** (`AActor::Destroy`).
     - Call PC vfunc +0x588 (unnamed).
     - `ClientGotoState(PC, FName index 0x142)`. 0x142 = 322 = `NAME_Spectating` (UE:Engine/Source/Runtime/Core/Public/UObject/UnrealNames.inl:159).
  4. Destroys all player-built `ABuildingActor`s (`IsPlayerBuilt` → vfunc +0x7A0).
  5. Sets GamePhase = Aircraft (3) through FUN_14044baf0.
- **Result: while on the bus the player has no pawn.** The PC is in the Spectating state and the camera views the aircraft.
- **EnterAircraft is triggered by:**
  - FUN_140443ee0@140443ee0 (GameState side, uses GameState+0x14E0 = Aircraft): for each local PC that is neither in nor already exited, call EnterAircraft(Aircraft).
  - fn:AFortPlayerControllerAthena__vfunc_611@14043c260: if GamePhase == 3, not in/exited, and Aircraft != null, call EnterAircraft.

#### B2.2 EnterAircraft (fn:AFortPlayerControllerAthena__EnterAircraft@1404251d0)

1. If `bHasExitedAircraft` (PC+0x31B6), log a warning; the function continues anyway.
2. Proceed only if `bIsInAircraft` (PC+0x31B5) is false. Field names are inferred: `IsInAircraft`@1404389a0 reads 0x31B5.
3. `PushInputComponent(AircraftInputComponent)` (FUN_1423a2cc0 with PC+0x2FC8; SDK:FN_FortniteGame_classes.hpp:17078).
4. If the PlayerCameraManager (PC+0x420) is an `AFortPlayerCameraBase`:
   - `bAutoManageActiveCameraTarget` (PC+0x430, SDK:FN_Engine_classes.hpp:1792) = false.
   - `SetViewTarget(Aircraft, {BlendTime 0, BlendFunc 1 (VTBlend_Cubic), BlendExp 2.0, bLockOutgoing false})`. Camera vfunc +0x6C8 is APlayerCameraManager::SetViewTarget@1423a7810 (vtable 1440c1268+0x6C8 → 1423a7810). The 2.0 comes from rdata:143ff9020.
   - Camera `OverrideCameraClass` (+0x19F8, SDK:FN_FortniteGame_classes.hpp:23155) = Aircraft.CameraModeClass if it is a subclass of UFortCameraMode_AthenaAircraft, else null.
   - `SetControlRotation(FUN_140427b10(Aircraft))`. FUN_140427b10@140427b10 returns **Aircraft root rotation + CameraInitialRotation, component-wise** (so the initial pitch is -30 relative to the bus).
5. PC+0x784 (`PendingSpectatorLocation`, SDK:FN_FortniteGame_classes.hpp:13196) = 0.
6. bIsInAircraft = 1, bHasExitedAircraft = 0.
7. Broadcast `OnAircraftStateChange` (PC+0x2840).
8. Clear PC+0xF08 (InteractResponse).
9. FUN_1405fe1a0(PC+0xD20, 0) (unnamed).
10. If local: `Aircraft.SpawnCosmeticActor(CustomizationLoadout.BattleBus)` (B1.5).

**View target is locked to the aircraft while in it:**
- fn:AFortPlayerControllerAthena__SetViewTarget@14044c030: while bIsInAircraft and GameState.Aircraft != null, any SetViewTarget whose target is not the aircraft is ignored.
- fn:AFortPlayerControllerAthena__AutoManageActiveCameraTarget@14041d370: while in the aircraft, re-targets the aircraft with the same {0, Cubic, 2.0} params.

#### B2.3 Aircraft input and jumping

**AircraftInputComponent bindings** (fn:AFortPlayerControllerAthena__SetupInputComponent@14044c0e0; name strings read from rdata 144062bc4 = "Turn", 144063634 = "Jump"):

| Binding | Type | Handler |
|---|---|---|
| "Turn" | axis | LAB_14041cf20 |
| "Turn_Gamepad" | axis | FUN_14041cf40 |
| "LookUp" | axis | LAB_14041ce80 |
| "LookUp_Gamepad" | axis | FUN_14041cea0 |
| "Jump" | action, IE_Pressed (0) | FUN_14041ce10 |
| "EscapeKey" | action | FUN_140836fa0 |

Jump bindings come from ConfigD: `ActionMappings` Jump = SpaceBar / Gamepad_FaceButton_Bottom (C:\FortniteRef\scratch\CookedIniVersion.txt).

**Jump handler** (FUN_14041ce10@14041ce10): if GameState exists, bIsInAircraft, GameState.Aircraft != null, and `bAircraftIsLocked` (GameState+0x14E8 bit0, SDK:FN_FortniteGame_classes.hpp:18308) is clear, call `ServerAttemptAircraftJump(GetControlRotation())`.

**ServerAttemptAircraftJump_Implementation** (fn@14044ab90):
- Sets a guard byte (0x31B7).
- If in the aircraft, SetControlRotation(arg), stores the rotation at PC+0x31A0, and calls PC vfunc +0x7D8. That is presumably the pawn restart / spawn; the pawn spawn is not in this area.

**ExitAircraft** (fn:AFortPlayerControllerAthena__ExitAircraft@1404260a0). Called from FUN_14043c050@14043c050 on the pawn side when the possessing Controller is an Athena PC that is in the aircraft, i.e. **when the new pawn is possessed**.
1. bIsInAircraft = 0, bHasExitedAircraft = 1.
2. PopInputComponent(AircraftInputComponent).
3. Camera: bAutoManageActiveCameraTarget = true; `SetViewTarget(Pawn, {BlendTime 0, VTBlend_Cubic, 2.0, false})`; OverrideCameraClass = null.
4. If the pawn is an AFortPlayerPawnAthena:
   - FUN_14041dd60(pawn). Purpose unresolved; probably starts skydiving.
   - **`CharacterMovement.Velocity` (pawn+0x3F0 → +0x104) = Aircraft.GetVelocity()**, then movement vfunc +0x420. The pawn inherits the bus forward velocity.
5. If local and `AudioOnExitAircraft` (PC+0x28C8) is set, play it as a 2D sound (FUN_1421a3640(world, sound, 1.0, 1.0, 0, ...)).
6. Broadcast OnAircraftStateChange; `FixupInteractionWidgetsOnUnzoom`.
7. On authority: `Aircraft.JumpFlashCount++`.

`Athena_PlayerController_C` CDO: `AudioOnExitAircraft = /Game/Athena/Sounds/Bus/FlyingBus_Drop_Cue`, `AthenaSoundMix = /Game/Sounds/FortSoundClassesAndModes/Fort_Default_Athena_Mix` (asset:/Game/Athena/Athena_PlayerController cdo).

#### B2.4 Camera mode `AthenaAircraft_CameraMode_C`

Chain: `AthenaAircraft_CameraMode_C` → `UFortCameraMode_AthenaAircraft` (0x120, SDK:FN_FortniteGame_classes.hpp:11417) → `UFortCameraMode_ThirdPerson` (0x110, :11374) → `UFortCameraMode` (0x50, :11312).

Final property values come from three layers:
1. Base ctors: fn:UFortCameraMode__UFortCameraMode@1404d4370 and fn:UFortCameraMode_ThirdPerson__UFortCameraMode_ThirdPerson@1404d43b0.
2. Aircraft ctor: fn:UFortCameraMode_AthenaAircraft__UFortCameraMode_AthenaAircraft@1403ce1c0.
3. BP CDO (asset:AthenaAircraft_CameraMode Default__...). The CDO overrides only the two InterpOffsets.

| Off | Property | Value | Set by |
|---|---|---|---|
| 0x30 | TransitionTime | 0 (not written) | none |
| 0x34 | TransitionOutTime | 0 (not written) | none |
| 0x38 | TransitionParams | BlendTime 0, BlendFunction 0 (written 1 then 0), BlendExp 2.0, bLockOutgoing 0 | base ctor |
| 0x48 | bResetInterpolation | 1 | base ctor |
| 0x50 | **FOV** | **90.0** (0x42B40000) | ThirdPerson ctor |
| 0x54 | CameraSpaceForwardDistance | 0 (not written) | none |
| 0x58 | CameraOrigin | 0 | ThirdPerson ctor |
| 0x60 | CameraOriginSocketName | None | ThirdPerson ctor |
| 0x68 | **CameraOriginInterpSpeed** | ThirdPerson sets (12,12,12) (rdata:144008e48 = 12.0; Z 0x41400000); **Aircraft ctor overrides to (0,0,0)** | Aircraft ctor |
| 0x78 | **ViewTargetSpaceViewOffset** | SamplePoints, all `(-45, 0, 300)` at pitch -90 / 0 / +90; LargeBodyTypeAddtnlOffset 0 | BP CDO |
| 0x88 | ViewTargetSpaceViewOffset_Crouch | empty | ThirdPerson ctor |
| 0x98 | ViewTargetSpaceViewOffseInterpSpeed | (12,12,12) | ThirdPerson ctor |
| 0xA8 | **CameraSpaceViewOffset** | SamplePoints: pitch -90 → (-2200,0,0); pitch 0 → (-1800,0,0); pitch +90 → (-1200,0,0) | BP CDO |
| 0xB8 | bSupportsShoulderSwap | true | ThirdPerson ctor |
| 0xD4 | PenetrationBlendInTime | 0.1 | ThirdPerson ctor |
| 0xD8 | PenetrationBlendOutTime | 0.15 | ThirdPerson ctor |
| 0xDC | **bPreventPenetration** | ThirdPerson 1, **Aircraft 0** | Aircraft ctor |
| 0xDD | **bDoPredictiveAvoidance** | ThirdPerson 1, **Aircraft 0** | Aircraft ctor |
| 0xE0 | CollisionPushOutDistance | 2.0 | ThirdPerson ctor |
| 0xE4 | HidePawnPenetrationPercent | 0.0 | ThirdPerson ctor |
| 0xE8 | PenetrationAvoidanceFeelers | 7 entries (table below); inert because penetration is off | ThirdPerson ctor |
| 0x100 | LastDrawDebugTime | -FLT_MAX | ThirdPerson ctor |
| 0x110 | **CameraOriginInterpSpeedToPawn** | **(2.0, 2.0, 1.0)** (rdata:143ff9020 = 2.0 for X/Y, rdata:143fe36fc = 1.0 for Z) | Aircraft ctor |

Feelers, as (AdjustmentRot P/Y/R; WorldWeight; PawnWeight; Extent; TraceInterval):
- (0,0,0; 1.0; 1.0; 14; 0)
- (0,+16,0; .75; .75; 0; 3)
- (0,-16,0; .75; .75; 0; 3)
- (0,+32,0; .5; .5; 0; 5)
- (0,-32,0; .5; .5; 0; 5)
- (+20,0,0; 1.0; 1.0; 0; 4)
- (-20,0,0; .5; .5; 0; 4)

The constants are rdata 1440b2898 = 14, 144040c9c = 16, 1440c5e48 = -16, 1440b289c = 32, 1440c5e4c = -32, 143fe3720 = 20, 144099398 = -20.

There is no spring arm or boom. The "arm length" is the CameraSpaceViewOffset X, interpolated by view pitch: 2200 looking straight down, 1800 at level, 1200 looking up.

**Overridden vfuncs.** Vtable 14405f4a0; slots 69..78 were read from .rdata.

| Slot | Function | Behaviour |
|---|---|---|
| 69 (+0x228) UpdateCamera(ViewTarget, DeltaTime, OutVT) | fn:UFortCameraMode_AthenaAircraft__vfunc_69@14040e3d0 | (a) `POV.FOV = FOV` (90). (b) `Rot = slot77(ViewTarget)`, normalized, converted to a quat. (c) `Origin = slot76(ViewTarget)`. (d) `LaggedOrigin = slot78(Origin, Quat, DeltaTime)`. (e) Evaluate CameraSpaceViewOffset at the view pitch (FUN_1403f5f40, mode 2). (f) Update the ViewTargetSpace offset (FUN_1404f2a80). (g) `POV.Location = LaggedOrigin + Quat.Rotate(CamSpaceOffset with X += CameraSpaceForwardDistance) + slot75(ViewTarget)`. slot75 is the base (fn:UFortCameraMode_ThirdPerson__vfunc_75@1404e8170): `LastViewTargetSpaceViewOffset` rotated by the view-target actor transform. (h) PlayerCamera vfunc +0x640(DeltaTime). (i) Clear bResetInterpolation. **There are no collision or penetration traces.** |
| 75 | base ThirdPerson vfunc_75 | see above |
| 76 GetCameraOrigin | fn:UFortCameraMode_AthenaAircraft__vfunc_76@1403f2750 | Calls the base (fn:UFortCameraMode_ThirdPerson__vfunc_76@1404e7f80): for a non-pawn, non-PC view target, this returns the root component world location (the aircraft). Then, if `PlayerCamera.PCOwner.Pawn` (camera+0x388 → PC+0x390) is an AFortPlayerPawn, the origin is that pawn's actor location instead. |
| 77 GetViewRotation | FUN_1403f2820@1403f2820 (unnamed override) | If PlayerCamera.PCOwner exists, return `PCOwner->GetControlRotation()` (vfunc +0x5F0); else return the view target root rotation. **Free look = control rotation.** |
| 78 InterpolateOrigin | fn:UFortCameraMode_AthenaAircraft__vfunc_78@1403f9d20 | If PCOwner.Pawn is an AFortPlayerPawn, set `CameraOriginInterpSpeed := CameraOriginInterpSpeedToPawn` (2,2,1). Then call the base fn:UFortCameraMode_ThirdPerson__vfunc_78@1404f2780: keep `CameraOriginLocalOffset` (0xC8) in view-target-local space, add the origin delta since `LastCameraOrigin` (0xBC), FInterpTo each axis toward 0 with speed CameraOriginInterpSpeed, and return `Origin - Rotate(offset)`. If bResetInterpolation is set, snap. |

Base vfunc_78 uses FMath::FInterpTo (FUN_1412db910@1412db910; UE:Engine/Source/Runtime/Core/Private/Math/UnrealMath.cpp:2372). If speed <= 0 it returns the target. So:
- **While on the bus:** speed is 0, the origin rigidly follows the aircraft root.
- **After the jump:** the pawn exists, the speed becomes 2/2/1 per second, and the camera origin eases from the bus to the pawn. This only applies while the aircraft mode is still active.

Note that ExitAircraft clears OverrideCameraClass, so how long this blend is visible depends on the camera-instance blend. That is unresolved.

**Camera mode selection:** FUN_1404f6340@1404f6340 returns OverrideCameraClass first, if set; it is called by FUN_1404e6d40@1404e6d40, which caches one mode instance per (class, view target). The FortCameraMode receives `OutVT.Target` (= the aircraft) as its ViewTarget argument (fn:AFortPlayerCameraBase__UpdateViewTarget@140502ff0, call through +0x228).

**Pitch limits:**
- Not set by AFortPlayerCameraBase's ctor (fn:AFortPlayerCameraBase__AFortPlayerCameraBase@1404d0d20 does not write +0x19AC/+0x19B0).
- `MainPlayerCamera_C` CDO has no ViewPitchMin/Max.
- So the engine default applies: ViewPitchMin = -89.9, ViewPitchMax = 89.9 (UE:Engine/Source/Runtime/Engine/Private/PlayerCameraManager.cpp:44-45, offsets SDK:FN_Engine_classes.hpp:3315-3316).
- Whether the Athena PC's PlayerCameraManagerClass is MainPlayerCamera_C was not verified (see UNRESOLVED).
- Yaw is unlimited.

---

### B3. HUD during the bus phase

GameState enums (SDK:FN_FortniteGame_structs.hpp):
- EAthenaGamePhase: 0 None, 1 Setup, 2 Warmup, **3 Aircraft**, 4 SafeZones, 5 EndGame (:323).
- EAthenaGamePhaseStep: 0 None, 1 Setup, 2 Warmup, **3 GetReady, 4 BusLocked, 5 BusFlying**, 6 StormForming, 7 StormHolding, 8 StormShrinking, 9 EndGame (:336).
- EAthenaAerialPhase: 0 None, **1 BusCantExit, 2 BusCanExit**, 3 Skydiving, 4 Parachuting (:3833).

**`AFortGameStateAthena::GetAerialPhase(PC)`** (fn@140427820):
- If the PC has an AFortPlayerPawnAthena pawn:
  - `!bIsSkydiving` (pawn+0x18E9) → None.
  - `bIsParachuteForcedOpen` (0x18EB) → None.
  - Otherwise 3 + `bIsParachuteOpen` (0x18EA). Field names are from SDK:FN_FortniteGame_classes.hpp:10760-10762.
- Else, if GamePhase == 3 and Aircraft is set, using ServerWorldTime `t`:
  - `t < DropStartTime` → **BusCantExit**.
  - `t < DropEndTime` → **BusCanExit**.
- Otherwise None.

#### B3.1 Widget instances in `/Game/Athena/HUD/AthenaHUD` (AthenaHUD_C : FortniteUI.AthenaHUDBase)

| Instance | Class / asset | Slot | Notes |
|---|---|---|---|
| AthenaAerialFeedback | `/Game/Athena/HUD/AthenaAerialFeedback` (AthenaAerialFeedback_C : FortHUDElementWidget) | `[Overlay] Persistent HUD Content:OverlaySlot_6` | jump prompt |
| GamePhaseAlert | `/Game/Athena/HUD/Phase/AthenaGamePhaseChangeWidget` (: FortniteUI.AthenaHUDGamePhaseChangingBase) | `[Overlay] Persistent HUD Content:OverlaySlot_11`, initial Visibility Collapsed | centre "BATTLE BUS LAUNCHING IN ..." banner |
| AthenaGamePhaseWidget | `/Game/Athena/HUD/Phase/AthenaGamePhaseWidget` (: FortHUDElementWidget) | `HorizontalBox_2:HorizontalBoxSlot_0` | small icon + mm:ss timer next to players-left/kills |
| MinimapContainer | `/Game/Athena/HUD/AthenaMinimapContainer`; HUDElementTag `HUD.TopRight.Minimap` | `Overlay_2:OverlaySlot_0` | contains `FortLegacySlateBridgeWidget` "FortSlateWidgetMinimap", `ContainedWidgetType = EFortLegacySlateWidget::AthenaMiniMap` (native Slate minimap) |

Source: asset:AthenaHUD exports 348, 358, 360, 376/377, 1324.

**Hide on bus** (asset:AthenaHUD disasm `HandleAircraftModeChanged`):
- On Construct, the HUD binds `HandleAircraftModeChanged` to `AFortPlayerControllerAthena.OnAircraftStateChange` and calls it once. Destruct unbinds it.
- The handler calls `PC.SetHUDElementVisibility(TagContainer{HUD.Athena.HideOnBus}, PC.IsInAircraft())`.
- Widgets tagged `HUD.Athena.HideOnBus` (asset:AthenaHUD HUDElementTags):
  - AthenaEquippedItemWidget
  - AthenaHealProgress (AthenaEquipProgress_C)
  - AthenaKillFeedWidget
  - LocalPlayerHitPointInfo
  - AthenaPickupManager
  - QuickbarPrimary
  - QuickbarSecondary
  - Reticle
- `CheckHUDElementVisibility`: if the element's tag is in the hidden set and the element is visible, set it Hidden (2); else set it SelfHitTestInvisible (4).
- Not hidden on the bus: minimap, compass, game-phase widgets, players-left, kills, aerial feedback.

#### B3.2 Jump prompt: AthenaAerialFeedback

Tree (asset:AthenaAerialFeedback): HorizontalBox `Feedback` = [`Press` CommonTextBlock "Press" (key DC3163344C79CDF76EBA74BD94D005E2, ns [7542B0684376D8F3BF6D9B886221F029], style `/Game/UI/Foundation/Text/TextStyle-Base-S`)] [`KeybindWidget_0` (`/Game/UI/InGame/KeybindWidget/KeybindWidget`, **BoundAction = "Jump"**, Brush Size Medium, frame `/Game/UI/InGame/KeybindWidget/Textures/Keybind-Frame-L`, tint alpha 0.6)] [`FeedbackText` CommonTextBlock, default "{Action}"].

Logic: the widget's Tick event runs the ubergraph every frame (asset:AthenaAerialFeedback disasm).
1. Get GameState (cast to Athena_GameState_C) and owning PC (cast to Athena_PlayerController_C).
2. Switch on `GameState.GetAerialPhase(PC)`:
   - None or BusCantExit → `Feedback.SetVisibility(Collapsed)`.
   - BusCanExit → FeedbackText = "To Jump" (key 377864294736257A6FC15C93BFB21574).
   - Skydiving → "To Deploy {Glider}" (key 69B4CD994A4389965BA22D98DD59B1F5), with `{Glider}` = `PC.CustomizationLoadout.Glider.DisplayName`.
   - Parachuting → "To Skydive" (key 06FB95D54EA2E03D88BFE1AF883A8B81).
3. For the three visible cases, `Feedback.SetVisibility(HitTestInvisible)`.

**On the bus, the prompt is hidden until DropStartTime and reads "Press [Space] To Jump" until DropEndTime.**

#### B3.3 GamePhaseAlert banner (AthenaGamePhaseChangeWidget)

**Native NativeTick** (FUN_142dd1f90@142dd1f90; it calls UpdateMessaging every tick):
1. Get GameState; read `GamePhaseStep` (+0x14DA) and `GamePhaseStepTimeRemaining` (+0x14DC).
2. `secs = FloorToInt(TimeRemaining)`, computed as `round(2x - 0.5) >> 1` with rdata:143feb1d8 = 0.5.
3. If `secs < 60`: TimeText = Format(LOCTEXT("GamePhaseChanging","SecondsRemainingFormat","{0} Seconds"), AsNumber(fmod(secs, 60))).
4. Else: TimeText = Format(LOCTEXT("GamePhaseChanging","MinutesSecondsRemainingFormat","{0} Minutes {1} Seconds"), secs/60, fmod(secs, 60)). The 60 is rdata:143fe3728.
5. Pick the message per step and call `UpdateMessaging(Step, Message, TimeText)`:

| Step | Message (LOCTEXT ns "GamePhaseChanging", key) | TimeText |
|---|---|---|
| 1 Setup | "WAITING FOR PLAYERS..." (WaitingForContendersWarmup) | FUN_14032bb00() (presumably empty text) |
| 2 Warmup | "WAITING FOR PLAYERS" (WaitingForPlayers) | time |
| **3 GetReady** | **"BATTLE BUS LAUNCHING IN"** (AircraftStartingIn) | time |
| **4 BusLocked** | **"DOORS WILL OPEN IN"** (AircraftUnlockingIn) | time |
| **5 BusFlying** | **"EVERYBODY OFF, LAST STOP IN"** (AircraftEndingTime) | time |
| 6 | "STORM FORMING IN" (StormFormingIn) | time |
| 7 | "STORM EYE SHRINKS IN" (StormShrinkingIn) | time |
| 8 | "STORM EYE SHRINKING" (StormEyeShrinking) | time |
| 9 | "GAME OVER, RETURNING TO LOBBY IN" (GameOverReturning) | time |

"BATTLE BUS LAUNCHING IN" also appears in `Localization/Game/en/Game.locres` (pakchunk0).

**Native construct** (FUN_142dcb210@142dcb210): binds `HandleGamePhaseStepChanged` to `GameState.GamePhaseStepChanged` (+0x13A0, SDK:FN_FortniteGame_classes.hpp:18279) and calls it immediately with the current step. HandleGamePhaseStepChanged (fn@142dc7920) forwards to the BP event `GamePhaseStepChanged`.

**BP side** (asset:Phase/AthenaGamePhaseChangeWidget disasm):
- `UpdateMessaging`: `Message.SetText(Message)`, `TimeText.SetText(TimeText)`, and `Icon.SetBrushFromMaterial(sel[Step])`. The table is: 0 none, 1 M_GamePhase_Setup, 2 M_GamePhase_Warmup, **3 M_GamePhase_Bus, 4 M_GamePhase_Bus, 5 M_GamePhase_Jump**, 6/7 M_GamePhase_StormHolding, 8 M_GamePhase_StormMoving, 9 M_GamePhase_End. All are under `/Game/Athena/HUD/Phase/`.
- `GamePhaseStepChanged(Step)`: a SwitchEnum over every value. For a real step:
  - `PlayGamePhaseAlertSound(Step)` plays `PlaySound2D(Fort_GamePhaseChange_Message_Athena_Cue)` for steps 1-8 and nothing for 0 and 9.
  - `SetVisibility(HitTestInvisible)`.
  - `PlayAnimation(ModeChanged, 0, 1 loop, Forward, 1.0)`.
  - `PlayAnimation(Step == 8 ? IconSlide : IconPulse, ...)`.
- `OnAnimationFinished(ModeChanged)` → `SetVisibility(Collapsed)`.
- The exact branch layout is reconstructed from node order; jump targets in the listing are not reliable.
- **Result:** at each step change (GetReady, BusLocked, BusFlying) the banner shows with a chime for the length of the `ModeChanged` animation, then collapses. The animation length is unresolved.

#### B3.4 Small phase widget (AthenaGamePhaseWidget)

Tick (asset:Phase/AthenaGamePhaseWidget disasm):
- `TimeText = Format("{0}:{1}" (key 246238F74CE3A73CC1C608AE97D9BD1F), {0} = trunc(T)/60 with min 1 integral digit, {1} = trunc(T)%60 with min 2 integral digits)`, where T = GameState.GamePhaseStepTimeRemaining. Conv_IntToText uses grouping=true and max digits 324.
- `Icon` material uses the same step table as B3.3: Bus icon for GetReady/BusLocked, Jump icon for BusFlying.
- Construct caches the GameState.

#### B3.5 Minimap and full map during the bus

- The bus icon is the native `UFortMiniMapIndicator` from B1.4 (`T_AthenaBus_Top`, 256x256 brush, scale 0.5, rotates with the actor). It is drawn by the native Slate minimap (`EFortLegacySlateWidget::AthenaMiniMap`). The map texture is `/Game/Athena/HUD/MiniMap/MiniMapAthena` (Texture2D 2048x2048, TEXTUREGROUP_UI).
- **Bus route line:**
  - No code drawing the flight path was found.
  - No UI-module function references `GameState.Aircraft` (+0x14E0) except GetAerialPhase. The users of +0x14E0 are listed by `decomp-grep -F "0x14e0)"`, and none are in the FortniteUI range 0x142D-0x143x.
  - The only 0x14xx minimap hits (FUN_141981670, FUN_14199daf0) are unrelated struct copies.
  - Treat the bus-path line as **not present in 1.8** (see UNRESOLVED).

#### B3.6 "Thank the bus driver"

**Does not exist in 1.8.** Searches:
1. Decompile, case-insensitive `thank|bus driver`: the only hits are the Feedback form text and the EndUserSettings text.
2. `Localization/Game/en/Game.locres` (pakchunk0, offset 4133949440, 5,625,346 bytes, uncompressed and unencrypted): no string containing "bus driver". "thank" hits are all Save the World dialogue or Founder text. The "driver" hits are "Driver" and "Masters Driver".
3. CookedIniVersion.txt: no "Thank" action mapping.
4. No `*Thank*` asset other than StW VO (`cons_f_ThankCommander_*`).
5. The aircraft input component binds only Turn, LookUp, Jump and EscapeKey (B2.3).

---

### B4. Bus-phase sounds and music

| Sound | Asset | Contents | Played by |
|---|---|---|---|
| Bus flying ambience | `/Game/Athena/Sounds/Vehicles/Athena_Bus_Flying_Loop_Cue` | SoundClass `Fort_Gadgets`, Vol 1, attenuation `/Game/Sounds/Attenuation/Fort_Bus_Flying_Athena_Att` (Logarithmic, Omni 1500, Falloff 7000, LPF 7000-10000, StereoSpread 2000, custom curve). Mixer of five branches: (1) Looping(Modulator P0.8-1.2 V0.7-1.2 (Delay 1-7 s (Random[Creak_03, Creak_01, Creak_02])) ); (2) `SFX_Bus_Flying_BalloonRelease_Loop` looping; (3) Looping(Modulator P1 V0.8-1.2 (Delay 5-20 s (Random[BeepBeep_01, _02, _03])) ); (4) Enveloper(`SFX_FlyingBus_Crowd_Loop` looping, volume curve 1→0 over 0-90 s); (5) `SFX_Bus_Flying_Wind_Loop` looping | B_StockBattleBus `Start Audio`, attached to `Bus`, FadeIn 2 s |
| Bus music | `/Game/Athena/Sounds/Vehicles/Athena_Bus_Music_Loop_Cue` | Vol **3.0**, Fort_Gadgets, attenuation `Fort_Bus_Music_Athena_Att` (Logarithmic, Omni 1500, Falloff 20000, LPF 5000-16000, LPFFrequencyAtMax 3000, bAttenuateWithLPF); one looping wave `SFX_Bus_Flying_PartyMusic_Loop` | same, FadeIn 2 s |
| Jump-out sound | `/Game/Athena/Sounds/Bus/FlyingBus_Drop_Cue` | Vol 1.4, Fort_Gadgets, no attenuation, wave `SFX_FlyingBus_Drop_03_16`, duration 12.92 s | ExitAircraft, 2D, local player only (B2.3) |
| Phase-change chime | `/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_GamePhaseChange_Message_Athena_Cue` | Vol 0.2, SoundClass Fort_UI_InGame, Delay 0.75 s then one wave (SoundNodeWavePlayer_24), override attenuation Falloff 1600 | GamePhaseAlert, PlaySound2D, steps 1-8 |
| Spawn-island bus (pre-launch) | `/Game/Athena/Sounds/Vehicles/BP_Bus_Lobby_SoundController` | CDO: LoopingSound `Athena_Bus_Engine_Idle_Cue` (BeepBeep_01/02/03 + Wind_Loop, Flying attenuation); LoopingMusic `Athena_Bus_Music_Lobby_Loop_Cue` (Vol 3, PartyMusic_Distant_Loop + PartyMusic_Loop); IgnitionSound `Athena_Bus_Ignition_Cue` (Vol 3, Ignition, Ignition_Distant, BeepBeep_03, BeepBeep_03_Distant); IgnitionTime 4; GetReadyFadeCurve `Curve_BusGetReadyFadeValue` (keys t0=1, t0.7=1, t1=0) | Its ubergraph (summarized below the table) |

**BP_Bus_Lobby_SoundController ubergraph.** It is an actor placed in the level; where it is placed is unresolved.
1. On BeginPlay: spawn LoopingSound and LoopingMusic attached to DefaultSceneRoot, then bind `On Game Phase Step Changed` to GameState.GamePhaseStepChanged.
2. On step GetReady: record GetReadyGameTime and set bGettingReady.
3. Each Tick while getting ready:
   - `alpha = MapRangeClamped(GameTime, GetReadyGameTime, GameState.WarmupCountdownEndTime, 0, 1)`.
   - Both loops' VolumeMultiplier = curve(alpha).
   - When alpha > 0.8, play `Athena_Bus_Flying_Transition_Cue` (wave `/Game/Athena/Sounds/Wind_Falling/sfx_parachute_skydive_loop_enter_02`) once as 2D.
4. On another step (BusFlying is the most likely): SetActorTickEnabled(false), spawn IgnitionSound, and Delay(IgnitionTime 4 s).

The step-to-branch mapping is approximate because the listing jump targets are unreliable.

Other related assets found but not referenced by anything above:
- `/Game/Sounds/Attenuation/Fort_Bus_Lobby_Close_Athena_Att`
- `Fort_Bus_Lobby_Distant_Athena_Att`
- `Curves/Fort_Bus_Lobby_Crossfade_Close_Athena`
- `Curves/Fort_Bus_Lobby_Crossfade_Far_Athena`
- `/Game/Sounds/Fort_Music/Menu/fortnite_lobby_music_athena_Cue` (the front-end lobby music, not bus-phase)

---

---

## C. Skydive, glider (parachute) and landing: movement and logic (Fortnite 1.8, Athena)

Scope: Battle Royale only. Sources: named Ghidra decompile (`C:\FortniteRef\decomp\named`, addresses are VA), the 1.8 exe read statically
(constants by VA through a scratch PE reader, `dx dis` from `fn-exe-defaults`), cooked assets via `fn-inspect` and the existing
`dump_table` example binary (`C:\target-fn-verify-edit\release\examples\dump_table.exe`, read-only use), the 1.8 SDK dump and UE 4.16 source.
Nothing was run, no config was decrypted, the repo was not modified. This builds on `crates/fn-movement/FALLING.md` (gravity, PhysFalling,
NewFallVelocity non-custom path, fall-damage curve) and does not repeat it.

Offsets below: CMC = `UFortMovementComp_CharacterAthena` instance, Pawn = `AFortPlayerPawnAthena` instance (SDK `FN_FortniteGame_classes.hpp`
lines 21992 and 10721). `FAirControlParams` = 5 x `FScalableFloat` (0x28 each): MaxAcceleration +0x00, LateralFriction +0x28, MaxLateralSpeed +0x50,
TerminalVelocity +0x78, GravityScalar +0xA0 (SDK `FN_FortniteGame_structs.hpp:10103`). `FScalableFloat::GetValueAtLevel` = 0x143d81440:
`Value * Curve.Eval(level)` when a curve row is set, else `Value`; all callers below pass level 0.

---------------------------------------------------------------------------------------------------------------------------------------

### 1. Values

#### 1.1 The four FAirControlParams (CMC)

Effective value = `Value` x `AthenaGameData` row at t=0. The `CharMoveComp` template of `/Game/Athena/PlayerPawn_Athena` (export 346) sets every
handle to `CurveTable=/Game/Athena/Balance/DataTables/AthenaGameData`, `RowName=Default.<Block>.<Field>`, and `Value=1.0` for MaxAcceleration,
LateralFriction, MaxLateralSpeed and TerminalVelocity. GravityScalar serializes only the curve handle, so its `Value` stays at the native
constructor value 1.0 (Athena CMC ctor 0x1404167b0 calls the ScalableFloat setter 0x143d92950 for all 20 fields). Row keys read with `dump_table`
(every row is `(0,v) (1,0) ... (20,0)`; level 0 gives v).

| Block (CMC offset) | MaxAcceleration | LateralFriction | MaxLateralSpeed | TerminalVelocity | GravityScalar |
|---|---|---|---|---|---|
| SkydivingControlParamsPassive (+0x788) | 4096 | 0.8 | 1200 | 1200 | 1.0 |
| SkydivingControlParamsActive (+0x850) | 4096 | 1.5 | 2400 | 6000 | 1.0 |
| ParachuteControlParamsSkydive (+0x918) | 2048 | 1.2 | 2000 | 500 | 1.0 |
| ParachuteControlParamsJump (+0x9E0) | 2048 | 1.3 | 800 | 500 | 1.0 |

The native ctor 0x1404167b0 defaults are identical numbers (constants 0x144078d94=4096, 0x144008e34=0.8, 0x144078d88=1200, 0x144078d90=2400,
0x144008e40=1.5, 0x144078d98=6000, 0x144031d78=2048, 0x14406288c=1.2, 0x144078d8c=2000, 0x144031d70=500, 0x144078d64=1.3, 0x144040cc4=800,
0x143fe36fc=1.0), so the result does not depend on whether the curve or the native value wins.

Other Athena CMC ctor stores (0x1404167b0, tail): FallingSlopeSafeSlideAngle (+0x774) = 60, FallingSlopeSafeSlideMaxSpeed (+0x778) = 1200,
FallingSlopeDamageScalarMin (+0x77C) = 0.33 (slope fall damage, `HandleImpact` 0x140436ce0, MOVE_Falling only, not skydive).

#### 1.2 Pawn (AFortPlayerPawnAthena) skydive / glider fields

| Field (offset) | Value | Source |
|---|---|---|
| ParachuteDeployTraceForGroundDistance (+0x1940) | **10000** (Value 1.0 x row `Default.Parachute.DeployTraceForGroundDistance` = 10000) | `PlayerPawn_Athena` CDO (`Default__PlayerPawn_Athena_C`, export 341); native ctor default 8000 (ctor 0x140413f00, setter call with 0x144078d9c = 8000) is overridden |
| ParachuteDeployVelocityBlend (+0x19B8) | **0.3** (Value 1.0 x row `Default.Parachute.DeployVelocityBlend` = 0.3) | same CDO; native default 0.3 (0x144008e2c) |
| LaunchPadParachuteDeployTraceForGroundDistance (+0x1968) | 2000 | native ctor 0x140413f00 (0x144078d8c); not serialized in any of the three pawn BPs |
| LaunchPadParachuteDeployTraceForDownwardSpeed (+0x1990) | 0 | native ctor 0x140413f00 (setter with 0) |
| ParachuteCooldownToOpen (+0x19E0) / ToClose (+0x19E4) | 0.5 s / 0.5 s | ctor stores at RVA 0x414487 / 0x414491 (`0x3F000000`); only used when cvar `Athena.LimitParachuteDeployRate` != 0 (section 3.4) |
| bParachuteDeployFixedVerticalDistance (+0x19E8) | true | ctor store at RVA 0x414480 |
| ParachuteAttachSocket (+0x1938) | `weapon_r` | ctor 0x140413f00 (FName literal) |
| MeshParachuteOpen (+0x1930) | subobject `MeshParachuteOpen0` (SkeletalMeshComponent), attached to `CharacterMesh0`, created hidden | ctor 0x140413f00; CDO export 355 |
| OnSkydivingLoop (+0x1708) | `/Game/Athena/Sounds/Wind_Falling/Wind_UmbrellaClosed_Loop_01_Cue` | `PlayerPawn_Athena` CDO |
| DirectionalSpeedMultiplierCurve (+0x1730) | `/Game/Athena/Balance/Movement/DirectionalSpeedMultiplierCurve` | `PlayerPawn_Athena_Generic_Parent` CDO |
| ParachuteDirectionalSpeedMultiplierCurve (+0x1738) | `/Game/Athena/Balance/Movement/ParachuteDirectionalSpeedMultiplierCurve` | `PlayerPawn_Athena` CDO |
| SkydivingDirectionalSpeedMultiplierCurve (+0x1740) | `/Game/Athena/Balance/Movement/SkydivingDirectionalSpeedMultiplierCurve` | `PlayerPawn_Athena` CDO |
| Capsule radius used for the deploy sweep | 42 (AFortPawn ctor 0x1407c9ff0 stores 42.0 at Capsule+0x694; no override in the Athena ctor or BPs; BP only sets HalfHeight 75) | |

Also written by the Athena pawn ctor to its CharacterMovement subobject (relevant to PhysFalling stepping, **differs from FALLING.md**):
`MaxSimulationTimeStep` (+0x2EC) = **0.2** (RVA 0x414511) and `MaxSimulationIterations` (+0x2F0) = **2** (RVA 0x4144F9); also CrouchedHalfHeight (+0x21C) = 50
(RVA 0x4144DE). No pawn BP overrides them. FALLING.md lists 0.05 / 8 from the engine CMC ctor; the Athena pawn ctor runs later and overwrites them.

#### 1.3 Directional speed multiplier curves (UCurveFloat, keys read with `fn-inspect props`)

All three assets have identical keys, `InterpMode` raw byte 1 = `RCIM_Constant` (UE4.16 `RichCurve.h:14-20` enum order Linear, Constant, Cubic), tangents 0:

| Time (deg) | 0 | 70 | 110 |
|---|---|---|---|
| Value | 1.0 | 0.8 | 0.65 |

So it is a step function: 1.0 for 0..70 deg, 0.8 for 70..110 deg, 0.65 beyond 110 deg (constant extrapolation). The angle is
`acos(dot(ActorForward, Velocity.GetSafeNormal2D()))` in degrees (pawn vfunc_273 0x140428410, constant 57.29578 at 0x14406289c).

#### 1.4 Map info

`Athena_Terrain` export 24 `DefaultMapInfo_4` (class `DefaultMapInfo_C`) serializes neither field, so the class CDO `/Game/Athena/Balance/MapInfos/DefaultMapInfo` applies:

| Field (AFortAthenaMapInfo offset) | Value | Row |
|---|---|---|
| SkydivingScanForImpactWorldZ (+0x518) | **20000** | `Default.Skydiving.ScanForImpactWorldZ` (Value 1.0) |
| SkydivingForceDeployParachuteWorldZ (+0x540) | **0** | `Default.Skydiving.ForceDeployParachuteWorldZ` (Value 1.0) |
| (context) AircraftHeight / AircraftSpeed | 80000 / 6000 | `Default.Aircraft.Height` / `.Speed` (aircraft is not this section's area) |

The pawn reaches the MapInfo through GameState+0x14D0 (0x140428e80; GameState via 0x140428e10).

#### 1.5 Console variables (exe defaults, `fn-exe-defaults/reports/exe-defaults.json`)

| Cvar | Default | Backing global | Effect found |
|---|---|---|---|
| Athena.AutoDeployParachuteOnBeginSkydive | 0 | 0x14521d994 | StartSkydiving picks custom mode 2 instead of 3 when set |
| Athena.EnableParachuteEverywhere | 0 | 0x14521d990 | allows the glider from a normal fall (section 3.1) |
| Athena.LimitParachuteDeployRate | 0 | 0x14521d998 | enables the open/close cooldowns |
| Athena.EnableSimulationForSkydiving / ParachutingFromSkydiving / ParachutingFromJump | 0 / 0 / (see json) | 0x14521d9a4 / 0x14521d9a8 / ... | proxy simulation switches in CMC vfunc_323 0x14044ce90 (network smoothing only) |

---------------------------------------------------------------------------------------------------------------------------------------

### 2. Movement modes and the move

#### 2.1 Mode numbers (EXACT)

No `EFortCustomMovement` enum exists in the SDK dump (searched `FN_FortniteGame_structs.hpp`). The numbers come from code:
`MOVE_Custom (6)` with CustomMovementMode **3 = skydiving**, **2 = parachute / glider open**, 1 = another Fort mode (not skydive).
Evidence: `GetCurrentAirControlParams` 0x1404280d0 (3 -> Skydiving params, 2 -> Parachute params), `PhysCustom` 0x140443e60,
`ConstrainInputAcceleration` 0x140421660 (mode 3 only), `StartSkydiving` (below) sets 6/3, `OpenParachute` sets 6/2.
`EAthenaAerialPhase` (SDK structs:3833, None/BusCantExit/BusCanExit/Skydiving/Parachuting) is a UI/state query only:
`AFortGameStateAthena::GetAerialPhase` 0x140427820 returns Skydiving (3) / Parachuting (4) from `bIsSkydiving` and `bIsParachuteOpen`, and None while `bIsParachuteForcedOpen`.

#### 2.2 Function map (names used below)

| Role | Address | Notes |
|---|---|---|
| StartSkydiving (unnamed) | 0x14041dd60 | also the "became true" body of `OnRep_IsSkydiving` 0x1404400a0 |
| EndSkydiving (unnamed) | 0x140424fe0 | also the "became false" path of `OnRep_IsSkydiving` |
| OpenParachute (unnamed) | 0x140443d80 | `SetMovementMode(Custom, 2)` |
| CloseParachute (unnamed) | 0x140421480 | back to Custom 3 or Falling |
| Enter parachute mode hook | 0x14043a930 | sets bIsParachuteOpen, shows/attaches mesh, sounds |
| Leave parachute mode hook | 0x14043b110 -> 0x14043afb0 | restores friction, clears bIsParachuteOpen, hides mesh |
| Enter / leave skydive mode hooks | 0x14043ab30 / 0x14043b180 | skydive loop audio start / stop, friction restore |
| `ACharacter::CheckJumpInput(dt)` Athena override | 0x14041f900 (pawn vfunc_256) | parachute toggle; base is 0x142042240 |
| `ClearJumpInput` Athena override | 0x14041fae0 (vfunc_257) | |
| Jump-pressed pawn hook | 0x14043b290 (pawn vfunc_388; AFortPlayerPawn slot is empty 0x141a68a80) | called from AFortPlayerController vfunc_646 0x14083ad60 |
| `Landed` Athena override | 0x140438fa0 (vfunc_244) | |
| Pawn Tick | 0x14044ec10 | auto-deploy |
| Style/speed (GetMaxSpeed source) | 0x140428410 (vfunc_273, vtable +0x888) | called by Fort `GetMaxSpeed` 0x1406d8fe0 |

#### 2.3 Leaving the bus: ExitAircraft (EXACT except where marked)

Server: `ServerAttemptAircraftJump_Implementation` 0x14044ab90 stores the client `FRotator` at PC+0x31A0 and calls PC vtable +0x7D8 =
`APlayerController::ServerRestartPlayer_Implementation` 0x1423a4e40 -> GameMode RestartPlayer.
- `AFortGameModeAthena::ChoosePlayerStart_Implementation` 0x1403d2d40: in GamePhase 3 (Aircraft, `EAthenaGamePhase` SDK structs:323) the start spot is the **aircraft actor** (0x1403d2ce0 -> GameState+0x14E0). So the pawn spawns at the aircraft's actor location.
- `AFortGameModeAthena::FinishRestartPlayer` 0x1403d7bc0: when the PC is in the aircraft (PC+0x31B5) and the jump flag (+0x31B7) is set, the spawn rotation is the client rotation; the pawn's actor rotation is then set to **yaw only** (pitch and roll zeroed) and `ClientSetRotation` sends the full client rotation as control rotation.
`AFortPlayerControllerAthena::ExitAircraft` 0x1404260a0 (reached from 0x14043c050 once the PC still flagged in-aircraft has a pawn):
1. PC flags: in-aircraft (+0x31B5) = 0, +0x31B6 = 1.
2. Camera manager `SetViewTarget(pawn)` with blend time **2.0 s** (constant 0x143ff9020) when the camera manager is an `AFortPlayerCameraBase`.
3. `StartSkydiving(pawn)` (below).
4. **Initial velocity**: `CMC.Velocity = Aircraft->GetVelocity()`; `AFortAthenaAircraft::GetVelocity` 0x1403f6da0 = actor rotation applied to `(FlightSpeed(+0x3F8) * scale.X, 0, 0)`, i.e. aircraft forward x FlightSpeed (Z = 0 for a level bus). Then CMC vtable +0x420 (component velocity update).
5. Local player: plays `AudioOnExitAircraft` (PC+0x28C8). Server: increments a jumped-player counter on the aircraft (+0x3B8).
INFERRED: FlightSpeed is filled from MapInfo `AircraftSpeed` (6000); the aircraft code was not traced here.

#### 2.4 StartSkydiving 0x14041dd60 (EXACT)

`bIsSkydiving`=1, `bPendingSkydiveLaunch`=0, `bIsBelowAutoDeployTestHeight`=0; adds a global gameplay tag (tag object at `FFortGlobalTags`+0x2E8,
via ASC vtable +0xA88 has / +0xA98 add; tag name UNRESOLVED); refreshes the parachute mesh state (0x14044ba30); then, unless the role is SimulatedProxy:
`SetMovementMode(MOVE_Custom, AutoDeployParachuteOnBeginSkydive ? 2 : 3)` -> **Custom 3** by default. On the authority it also sets
**`bIgnoreNextFallingDamage` (Pawn+0x848) = 1**, records the jump location (Pawn+0x1BCC) and a match time (Pawn+0x1BD8) for analytics (used in EndSkydiving).

#### 2.5 The skydive / glider move (EXACT)

`PhysCustom` 0x140443e60: for Custom 2 or 3 it snapshots `Acceleration` (CMC+0x288) into CMC+0xAAC, sets a "use snapshot" flag (CMC+0xAA8),
runs the stock **`UCharacterMovementComponent::PhysFalling`** and restores the flag. There is no custom integrator; everything comes from virtual overrides that
check `MovementMode==6 && CustomMode in {2,3}`:

| Override | Address | Behaviour in Custom 2/3 |
|---|---|---|
| `IsFalling` | 0x140438950 | true (so CMC/Fort code treats both modes as falling: FallingStartedZ tracking, ConstrainInput, ProcessLanded) |
| `GetCurrentAirControlParams` | 0x1404280d0 | mode 3: Active if `IsActivelySkydiving` else Passive; mode 2: `ParachuteControlParamsSkydive` if Pawn.bIsSkydiving else `ParachuteControlParamsJump` |
| `GetGravityZ` | 0x140428f00 | Fort gravity x `GravityScalar` (1.0) |
| `GetMaxAcceleration` | 0x140434230 | `MaxAcceleration` (4096 skydive, 2048 glider) |
| `UpdateCharacterStateBeforeMovement` | 0x140450450 | every move: `FallingLateralFriction` (CMC+0x218) = `LateralFriction` |
| `GetMaxSpeed` | 0x1404343e0 -> Fort 0x1406d8fe0 -> pawn vfunc_273 0x140428410 | `MaxLateralSpeed x DirCurve(angle)` (glider curve when bIsParachuteOpen, else skydiving curve; 1.0 if the curve is null); then, if actively skydiving, `x |AccelDir.xy|` (horizontal length of the normalised input acceleration) |
| `NewFallVelocity` | 0x14043a380 | terminal-velocity relaxation, below |
| `ConstrainInputAcceleration` | 0x140421660 | mode 3: keep X,Y and `Z = min(Z, 0)` (downward input kept); mode 2: stock (Z removed because IsFalling is true, UE4.16 `CharacterMovementComponent.cpp:6617`) |

PhysFalling (UE 4.16 `CharacterMovementComponent.cpp:3853`) then gives, per sub-step:
- Lateral acceleration `= (Acceleration.X, Acceleration.Y, 0) * AirControl` (AirControl 0.95 from `PlayerPawn_Athena_Generic` CharMoveComp; boosted to 1.0 while `|V.xy| < 25`), clamped to `GetMaxAcceleration()` (`GetFallingLateralAcceleration` :3813, `BoostAirControl` :3841). Input acceleration = `MaxAcceleration x input` (`ScaleInputAcceleration` :6629).
- `CalcVelocity(dt, FallingLateralFriction, ...)` (:2894) with Velocity.Z = 0: over max speed or no input -> `ApplyVelocityBraking` (:3388) with friction x `BrakingFrictionFactor` 2.0 (:297) and BrakingDecelerationFalling 0, never dropping below MaxSpeed while still pushing forward; with input -> `V -= (V - AccelDir*|V|) * min(dt*Friction, 1)` then accelerate and clamp to MaxSpeed. Velocity.Z restored afterwards.
- Gravity -> `NewFallVelocity`.
- Stepping: MaxSimulationTimeStep 0.2 / MaxSimulationIterations 2 (section 1.2).

**NewFallVelocity in Custom 2/3** (0x14043a380; constants 0x144078d50 = 1e-6, 0x144078d58 = 1/33 s, 0x143feb1d8 = 0.5):
- `T` = current params `TerminalVelocity`; if actively skydiving: `T = Passive.T + |min(AccelDir.z, 0)| x (Active.T - Passive.T)` = 1200 + 4800 x dive steepness (read straight from CMC+0x800 / +0x8C8).
- If `Vz < -T` (and T > 0): sub-step the remaining dt (step = dt if dt <= 1/33, else min(dt/2, 1/33); loop while >= 1e-6): `Vz += (g / T) * Vz * step` (exponential decay toward -T at rate |g|/T per second), result `min(Vz, -T)`.
- Else: `Vz = max(Vz + g*dt, -T)`. X/Y pass through.
Derived (port gravity -2800, INFERRED arithmetic): decay rate 5.6 /s under the glider (T 500), 2.33 /s passive (T 1200), 0.47 /s full dive (T 6000).

**Active vs passive** (`UFortMovementComp_CharacterAthena::IsActivelySkydiving` 0x140438420; constants 0x143ff9010 = 1e-4, 0x144078dac = -0.1, 0x144040c7c = 0.4):
only in Custom 3 with a pawn; `a` = input acceleration (snapshot during PhysCustom, live otherwise); requires some component |a| > 1e-4;
`d = a / |a|`, `f = ForwardVector(ControlRotation)` (full pitch, 0x1412f4470); **active iff `d.z < -0.1` and `dot(f, d) > 0.4`**.
`IsPassivelySkydiving` 0x140438c60 = not active. `IsActivelyStrafingInAir` 0x140438670 (exposed to BP): Custom 3 and `|dot(f, d)| <= 0.8` (0x144008e34).
Input source: "MoveForward"/"MoveRight" axes are bound in `AFortPlayerPawn::SetupPlayerInputComponent` 0x140803a00 to pawn slots 391/392;
AFortPlayerPawn's MoveForward 0x1407f6220 adds input along the forward axis of the controller's control rotation (rotation-to-axis helper 0x14045d500).
INFERRED: that axis keeps the control pitch (no pitch zeroing is visible in 0x1407f6220), which is what gives the input a negative Z when looking down while holding forward;
this is the only way `d.z < -0.1` can occur, and `ConstrainInputAcceleration` explicitly keeps that Z in mode 3. So: **dive = hold forward while looking down more than ~5.7 deg
(asin 0.1), with the move direction within ~66 deg (acos 0.4) of the look direction.** Glide (passive) = anything else.

Dive vs glide numbers (from the table): passive 1200 cm/s fall, lateral cap 1200 (x direction curve); active lateral cap 2400 x curve x cos(dive), terminal blended up to 6000;
glider from skydive: lateral cap 2000, fall 500; glider from a jump/launch (bIsSkydiving false): lateral cap 800, fall 500.

Rotation: no Athena override of `PhysicsRotation` / `ComputeOrientToMovementRotation` exists (Athena CMC override list in names.tsv); Fort `ComputeOrientToMovementRotation`
0x1406d3300 is the generic path. Body pitch during the dive is not set by movement code (animation, out of scope).

---------------------------------------------------------------------------------------------------------------------------------------

### 3. Glider deploy, close, auto-deploy

#### 3.1 Manual deploy / close input (EXACT)

Jump press -> AFortPlayerController vfunc_646 0x14083ad60 -> pawn vfunc_388 0x14043b290: if the pawn is skydiving or the glider is open
(or `EnableParachuteEverywhere` && MOVE_Falling && !CanJump) and **not forced open**, it writes the request int Pawn+0x18F0 = `bIsParachuteOpen ? 2 (close) : 1 (open)`.
The request travels to the server in the saved move (`SetMoveFor` 0x14044bf20 copies it; `GetCompressedFlags` 0x140427df0: 2 -> FLAG_Custom_0 0x10, 1 -> FLAG_Custom_1 0x20;
`UpdateFromCompressedFlags` 0x140450670 restores it).
`CheckJumpInput` override 0x14041f900 (runs each tick with dt): decays the consecutive-jump window; if jump is pressed while crouching it returns; if not skydiving, glider closed
(and not the parachute-everywhere case) it runs the normal jump (super 0x1407d0b00). Otherwise it never jumps and instead:
- request 1, glider closed -> **OpenParachute** 0x140443d80 (Custom 2), unless (not forced && LimitParachuteDeployRate && now - lastToggle < CooldownToOpen);
- request 2, glider open, not forced -> **CloseParachute** 0x140421480, unless (LimitParachuteDeployRate && now - lastToggle < CooldownToClose);
- lastToggle (Pawn+0x1BC8) = world time on either action.
The input action name feeding vfunc_646 was not found (UNRESOLVED; the aircraft "Jump" binding in `SetupInputComponent` 0x14044c0e0 goes to 0x14041ce10 -> ServerAttemptAircraftJump).

**Re-close / redeploy exists in 1.8**: CloseParachute returns to **Custom 3 (skydive)** while bIsSkydiving is true, else to MOVE_Falling if not on ground.
The glider can be toggled any number of times before landing, except once forced open. With the default cvar the 0.5 s cooldowns are not applied.

#### 3.2 Auto-deploy (Pawn Tick 0x14044ec10, every tick, non-SimulatedProxy) (EXACT)

While bIsSkydiving:
1. Test-height gate (`bIsBelowAutoDeployTestHeight`, Pawn+0x18EC): for the bus skydive, set when `ActorZ <= SkydivingScanForImpactWorldZ` (20000). For later
   (launch-pad) skydives (Pawn+0x18ED set, see 4.1) the same Z test runs only while `Velocity.Z < -LaunchPadParachuteDeployTraceForDownwardSpeed` (0 -> any descent).
2. Once below and not yet forced:
   - `b = clamp(ParachuteDeployVelocityBlend, 0, 1)` = 0.3; `dir = normalize(lerp((0,0,-1), Velocity.GetSafeNormal(), b))`.
   - `dist` = ParachuteDeployTraceForGroundDistance (10000) for the bus skydive, LaunchPad... (2000) otherwise; if `bParachuteDeployFixedVerticalDistance` and `b < 0.99` (0x144078d60) and `dir.z < -1e-4`: `dist /= |dir.z|` (constant vertical reach).
   - Sphere sweep 0x14044fdf0 "TraceForLandingGeometry": channel ECC_Pawn (2) with response params ignore-all except WorldStatic and WorldDynamic = Block; radius = 2 x scaled capsule radius (84 cm);
     from actor location to `loc + dir*dist`.
   - Forced if the sweep hits, **or** if `ActorZ <= SkydivingForceDeployParachuteWorldZ + ParachuteDeployTraceForGroundDistance(unscaled)` = 0 + 10000 (bus case; uses the base distance, not the stretched one).
   - On force: `bIsParachuteForcedOpen` (Pawn+0x18EB) = 1, request = 1, and on the authority with the glider closed: OpenParachute immediately.
3. While skydiving the weapon is holstered every tick (`SetHolsterWeapon(1)` when the holster counter Pawn+0xB7C is 0).
While bIsParachuteOpen (every tick): `FallingStartedZ` (CMC+0x74C) = current Z (0x1406e8130).

Forced-open semantics: the jump hook refuses to post a close request, CheckJumpInput's close branch needs not-forced, `GetAerialPhase` reports None. Cleared only by EndSkydiving.

#### 3.3 Mode-change hooks (`OnMovementModeChanged` 0x14043b400, only with valid data)
- Entering Custom 2 (0x14043a930): bIsParachuteOpen = 1, MeshParachuteOpen visible and attached to the mesh at `weapon_r`, FallingStartedZ reset, deploy sound attached, glider loop audio (2D) started.
- Entering Custom 3 (0x14043ab30): parachute mesh refresh, skydive loop audio `OnSkydivingLoop` started (2D).
- Leaving Custom 2 (0x14043b110): FallingLateralFriction restored from the class CDO's CMC (0.0), then 0x14043afb0: bIsParachuteOpen = 0, mesh hidden and detached, FallingStartedZ reset, close sound, glider loop stopped.
- Leaving Custom 3 (0x14043b180): friction restored, skydive loop audio faded out (0.5 s).

#### 3.4 Launch pads (note only)
`HandlePendingLaunch` 0x1404370a0: applies PendingLaunchVelocity, then StartSkydiving if `bPendingSkydiveLaunch`, else EndSkydiving if skydiving, then MOVE_Falling.
`IsSkydivingFromLaunchPad` 0x140438d20 = bIsSkydiving && Pawn+0x18ED.

---------------------------------------------------------------------------------------------------------------------------------------

### 4. Landing

#### 4.1 What ends the skydive (EXACT)
- In Custom 2/3 a floor hit from PhysFalling goes to Athena `ProcessLanded` 0x140449040 / `IsValidLandingSpot` 0x140438e50: landing is accepted only if the hit component's
  primitive vtable +0x538 check passes for the pawn (INFERRED: `CanCharacterStepUp`); else the pawn keeps falling/sliding.
- Accepted: Fort `ProcessLanded` 0x1406e2ca0 -> UE `ProcessLanded` (:5109): `Landed(Hit)` first, then `SetPostLandedPhysics` (:5144) -> MOVE_Walking.
- Athena `Landed` 0x140438fa0: after super (AFortPawn fall-damage logic), if the hit allows it and not SimulatedProxy: clears bPressedJump and the request int, `ResetJumpState` 0x142059a50, and if bIsSkydiving -> **EndSkydiving** 0x140424fe0.
- EndSkydiving: bIsSkydiving = 0, bIsParachuteForcedOpen = 0, removes the skydive tag, hides/detaches the glider mesh, unholsters the weapon (if holster counter > 0), local PC UI hook (0x14083cfb0) and
  authority analytics (0x140388310 with jump location and time) on the first one, stores the landing location (Pawn+0x1BDC), then sets **Pawn+0x18ED = 1** (all later skydives use the launch-pad branch), stops skydive audio.
- Then the switch to Walking triggers the "leaving Custom 2" hook (glider closed). Replicated clients get the same through `OnRep_IsSkydiving` / `OnRep_IsParachuteOpen` 0x140440070.

#### 4.2 Fall damage during the drop (EXACT)
- `FallingStartedZ` (CMC+0x74C) = max over the fall while IsFalling (Fort TickComponent 0x1406ea350; IsFalling includes Custom 2/3); seeded in Fort `OnMovementModeChanged` 0x1406dff00.
- The glider resets it to the current Z on open (0x14043a930), every tick while open (Tick 0x14044ec10) and on close (0x14043afb0), so a glider landing has ~0 fall height.
- StartSkydiving sets `bIgnoreNextFallingDamage` on the authority, so the first landing after a skydive takes no fall damage whatever the height (AFortPawn Landed 0x1407eb460 honours that flag per FALLING.md 9.2).
- Falling feedback: Fort TickComponent fires a falling feedback event once (`bTriggeredFallingFeedbackSinceLanded`, CMC+0x754 bit 0) when the authority's fall exceeds 384 cm (0x143ff9048); bit 1 set on entering a falling mode.
- Slope fall damage (`HandleImpact` 0x140436ce0) runs only in MOVE_Falling (3), not in the custom modes, and also respects bIgnoreNextFallingDamage.
- No skydive-specific fall-damage GameplayEffect was found; damage uses the `AthenaFallingDamage` curve (FALLING.md 9.2).

#### 4.3 After landing
- Jumping works normally again (CheckJumpInput falls through to the super path once bIsSkydiving and bIsParachuteOpen are false).
- A new skydive only comes from a launch pad (`bPendingSkydiveLaunch` + HandlePendingLaunch) or StartSkydiving from another bus exit; with `EnableParachuteEverywhere` = 0 (default) a normal fall cannot open the glider.
- Respawn is not allowed after the bus phase: `PlayerCanRestart_Implementation` 0x140400380 returns false in phases 4/5 and in phase 3 only allows the aircraft jump.
- `LandingMontagePair` (Pawn+0x1880) is not a skydive landing animation: Athena `PreReplication` (vfunc_80 0x1404487c0) copies it from the ability system component (+0x1758) for replication and `OnRep_ReplicatedAnimMontage` 0x140441c70 copies it back. It carries any GAS montage.

---------------------------------------------------------------------------------------------------------------------------------------

### 5. Camera mode switching

Native selection (unnamed `AFortPlayerCameraBase` function 0x1404e1ad0; offsets SDK classes:23148):
DBNO pawn -> `DBNOCameraClass`; Athena pawn with bIsParachuteOpen -> **SkydiveParachuteCameraClass** (+0x1A28); else actively skydiving -> **SkydiveDiveCameraClass** (+0x1A20);
else bIsSkydiving -> **SkydiveGlideCameraClass** (+0x1A18); else the normal 3P/targeting class. The class can therefore flip between Dive and Glide every frame with the input.

The only asset that sets these: `/Game/Blueprints/Camera/Original/MainPlayerCamera` CDO (`Default__MainPlayerCamera_C`): SkydiveDive/Glide/Parachute ->
`/Game/Blueprints/Camera/Athena/Athena_PlayerCameraModeSkydive{Dive,Glide,Parachute}_C`. Each mode BP's super is `Athena_PlayerCameraModeBase_C` (super `FortCameraMode_ThirdPerson`).

| Property (CDO) | Base | SkydiveGlide | SkydiveDive | SkydiveParachute |
|---|---|---|---|---|
| FOV | 80 | 90 | 100 | (inherits 80) |
| TransitionTime | 0.45 | 1.0 | 2.0 | 1.0 |
| CameraSpaceForwardDistance | -280 | inherited | inherited | inherited |
| CameraOriginInterpSpeed | (12, 1e11, 60) | (30, 1e11, 60) | (30, 1e11, 60) | (30, 1e11, 60) |
| CameraSpaceViewOffset by pitch (0 / +90 / -90) | (0,0,0) / (-10,0,25) / (10,0,25) | (-30,0,0) / (-75,0,25) / (100,0,50) | (0,0,0) / (0,0,0) / (150,0,25) | (0,0,0) / (-75,0,0) / (-145,0,-25) |
| ViewTargetSpaceViewOffset (pitch 0) | (0,45,75) | (0,0,75) | (0,0,75) | (-75,0,145) |
| ViewTargetSpaceViewOffseInterpSpeed | (10,10,10) | (8,8,8) | (8,8,8) | (8,8,8) |
| TransitionOutTime / TransitionParams / PenetrationAvoidanceFeelers / CollisionPushOutDistance | 0.45 / EaseInOut / 5 feelers / 0.2 | inherited | inherited | inherited |

The math of `FortCameraMode_ThirdPerson` (how offsets, interp speeds and transitions combine) is the camera section's job and was not traced here.

---------------------------------------------------------------------------------------------------------------------------------------

---

## D. Skydive / glider animation, glider item, audio, FX, aerial HUD prompt

Tool used throughout: `fn-inspect` (release build in the repo), read-only. "ABP" = `/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint` (the only player AnimBP; male and female share it, gender is a `BlendListByEnum`). Kismet offsets are the serialized offsets printed by `fn-inspect disasm`.

### D.1 Default glider item (1.8)

`fn-inspect props /Game/Athena/Items/Cosmetics/Gliders/DefaultGlider` (export 0, class `AthenaGliderItemDefinition`):

| Property | Value |
|---|---|
| `GliderType` | not serialized -> enum default `EFortGliderType::Glider` (0) (SDK `FN_FortniteGame_structs.hpp` enum EFortGliderType) |
| `SkeletalMesh` | `/Game/Gadgets/Assets/VinderTech_GliderChute/Meshes/SK_M_VinderTech_GliderChute` (skeleton `SK_M_VinderTech_GliderChute_Skeleton`) |
| `AnimClass` | `/Game/Gadgets/Assets/VinderTech_GliderChute/Blueprints/VinderTech_GliderChute_AnimBP.VinderTech_GliderChute_AnimBP_C` |
| `OpenSound` | `/Game/Sounds/Fort_Gadgets/Athena/Athena_Glider_Open_Cue` |
| `OpenSoundLoop` | `/Game/Athena/Sounds/Wind_Falling/Wind_UmbrellaOpen_Loop_01_Cue` |
| `CloseSound` | `/Game/Sounds/Fort_Gadgets/Athena/Athena_Glider_Close_Cue` |
| `DisplayName` | "Glider" (key 8DB5C2C742C711FC3033C09785F42CA2); Description "Standard Battle Glider. "; Rarity Handmade |
| Icons | `/Game/UI/Foundation/Textures/Icons/Skydiving/T-Icon_Glider-S-VinderTech-Glider-SKIN-Default(-L)` |

The material is the mesh's own (skins swap `MI_VinderTech_GliderChute_*` under `/Game/Gadgets/Assets/VinderTech_GliderChute/Materials/`; the default glider sets no override). Which code path picks `DefaultGlider` when the loadout has none is NOT confirmed here (see UNRESOLVED).

### D.2 Glider mesh AnimBP (`VinderTech_GliderChute_AnimBP`)

`fn-inspect props <pkg>` class export `VinderTech_GliderChute_AnimBP_C` -> `BakedStateMachines[0]` "Locomotion", `InitialState 0`; node indices resolved through the class property order (`RootAnimNodeIndex 12` = `AnimGraphNode_Root`, verified).

| State | Player node | Asset (all under `/Game/Animation/Game/MainPlayer/Parachute/Medium/Female/`) |
|---|---|---|
| 0 `Closed` | SequencePlayer A984..., `bLoopAnimation true`, PlayRate 1 | `Skydive_HangGlider_Glide_Closed_GLIDER` (0.0333 s, 1 frame) |
| 2 `Open_Into` | SequencePlayer 2133..., `bLoopAnimation false`, PlayRate 1 | `Skydive_HangGlider_Glide_Into_GLIDER` (4.6667 s) |
| 1 `Idle_Open` | BlendSpacePlayer 7E66..., X = `YawDeltaSmoothed`, Y = `AimPitch` | `BS_Skydive_HangGlider_GLIDER` (Yaw -7..7, Pitch -85..0, interp 0.25 s; samples Glide_GLIDER (0,0)/(0,-85), Glide_Left_GLIDER (-7,*), Glide_Right_GLIDER (7,*); clips 2.6667 s) |

Transitions (all `CrossfadeDuration 0.2`, `HermiteCubic`, `TLT_StandardBlend`):
- Closed -> Open_Into: `IsParachuteOpen` (TransitionResult 6AAE... copy record).
- Open_Into -> Idle_Open: `GetRelevantAnimTimeRemaining(0, 2) == 0` (bytecode 0x187..0x1b7), i.e. after the 4.667 s Into clip ends.
- Open_Into -> Closed and Idle_Open -> Closed: `!IsParachuteOpen` (copy records with `LogicalNegateBool`).
EventGraph (`ExecuteUbergraph`, 0x6..0x12a): owner pawn cast to `FortPlayerPawnAthena`; `IsParachuteOpen = pawn.IsParachuteOpen()`; copies `Parachute_LeftRight`, `YawDeltaSmoothed`, `AimPitch` from the pawn's player AnimBP instance (cast to `Fortnite_M_Avg_Player_AnimBlueprint_C`).

### D.3 Player AnimBP: skydive / glide states

Source: ABP `BakedStateMachines` (decoded earlier into `C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\reports\animation-state-machines.json`, machine 2 "FullBody"), cross-checked against the 1.8 pak bytecode (`fn-inspect disasm <ABP>`) and the CDO node structs (`fn-inspect props <ABP> cdo`). LinkID = 509 - (index of the node property in the class export list), verified on 12 nodes.

**FullBody state machine (machine 2, initial `SplitBody`)** - skydive part; every transition 0.2 s HermiteCubic StandardBlend unless noted:

| From | To | Rule |
|---|---|---|
| SplitBody | SkyDive_Glide | `IsSkydiving && !IsParachuteOpen && !IsSkydivingLaunchPad` |
| SplitBody | ParaThing_Into | `IsParachuteOpen && IsSkydivingLaunchPad` (launch pad only) |
| SkyDive_Glide | SkyDive_Dive | `(ForwardBack > 100 \|\| UpDown < -20) && IsSkydiving` (bytecode 0x23e..0x296) |
| SkyDive_Dive | SkyDive_Glide | `(ForwardBack < 100 && UpDown > -20) \|\| !IsSkydiving` (0x17ca..) |
| SkyDive_Glide / SkyDive_Dive | ParaThing_Into | `IsParachuteOpen` |
| SkyDive_Glide | SplitBody | `!IsSkydiving` |
| ParaThing_Into | ParaThing | automatic remaining-time rule, **0.3 s** |
| ParaThing_Into / ParaThing | SkyDive_Glide | `IsSkydiving && !IsParachuteOpen && !IsSkydivingLaunchPad` (glider closed in the air) |
| ParaThing_Into | SplitBody | `IsSkydivingLaunchPad && !IsParachuteOpen`, or `!IsSkydiving && !IsSkydivingLaunchPad` |
| ParaThing | SplitBody | `IsSkydivingLaunchPad && !IsParachuteOpen`, or `!IsSkydiving` |

Note: `ForwardBack`/`UpDown` here are **LocalAcceleration** X/Z (bytecode 0x3c33..0x3c9f), not velocity.

**State pose trees** (BlendListByBool: pose 0 = true, pose 1 = false):

| State | Tree |
|---|---|
| SkyDive_Glide (root 95) | BlendListByBool `IsSkydiveLeaning` (C00BFF..., BlendTime 0.5/0.5 Linear): true -> `BS_Skydive_Glide_Lean` (X `SkyDive_LeftRight`, Y `AimPitch`); false -> `BS_Skydive_Glide_2D` (X `YawDeltaSmoothed`, Y `AimPitch`) |
| SkyDive_Dive (root 91) | BlendListByBool `IsSkydiveLeaning` (3CF345..., 0.15/0.15 Linear): true -> `BS_Skydive_Dive_Lean`; false -> `BS_Skydive_Dive_2D` (same inputs) |
| ParaThing_Into (root 50) | BlendListByBool `IsUsingUmbrella` (3DDB56..., 0.1/0.1): true -> `BS_Skydive_Parachute_Into`; false -> `BS_Skydive_HangGlider_Into` (X `YawDeltaSmoothed`, Y `AimPitch`) |
| ParaThing (root 87) | BlendListByBool `IsUsingUmbrella` (745166..., 0.1/0.1): true -> ApplyAdditive(alpha 1){ base `BS_Skydive_Parachute`, additive = state machine 5 `Lean_Umbrella` }; false -> ApplyAdditive(alpha 1){ base = BlendListByEnum `Gender` (616CB4..., EnumToPoseIndex [0,0,1,0,0], times 0) -> pose 0 `Male/BS_Skydive_HangGlider`, pose 1 (Female) `Female/BS_Skydive_HangGlider`; additive = state machine 4 `Lean_Glider` } |
| Lean_Glider/Lean (machine 4, single state, root 81) | BlendListByBool `ParachuteLeanTransition` (E92FEA..., BlendTime **0.35 / 0.75** Linear): true -> BlendListByEnum `Gender` (6DB04C..., [0,0,1,0,0]) -> male: BlendListByEnum `Direction` (6FF6E4..., 0.25 each, EnumToPoseIndex [0,1,2,3,4,0]) {Center: `Male/Skydive_HangGlider_Lean_Center_Pose` rate 1; Forward: `Male/..._Lean_Forward_Into` rate 2; Back: `Male/..._Lean_Back_Into` rate 2; Left: `Male/..._Lean_Left_Into` rate 2; Right: `Male/..._Lean_Right_Into` rate 2}; female: Direction (AB1415...) {`Female/Skydive_HangGlider_Lean_Center` rate 1, Female `..._Forward_Into`/`_Back_Into`/`_Left_Into`/`_Right_Into` rate 2}; false -> Gender (5583B3...) -> `Male/BS_Skydive_HangGlider_Lean` / `Female/BS_Skydive_HangGlider_Lean` (X `LeftRight`, Y `ForwardBack`) |
| Lean_Umbrella/Lean (machine 5, root 60) | BlendListByBool `ParachuteLeanTransition` (BF9410..., 0.35/0.75): true -> Direction (2AA234..., 0.25) {`Skydive_Parachute_Lean_Center_Pose` rate 1, `_Forward_Into`/`_Back_Into`/`_Left_Into`/`_Right_Into` rate 1.5}; false -> `BS_Skydive_Parachute_Lean` (X `LeftRight`, Y `ForwardBack`) |

All lean SequencePlayers are `bLoopAnimation false`, StartPosition 0 (CDO). All state machines: `MaxTransitionsPerFrame 3`, `bSkipFirstUpdateTransition true`.

**EventGraph variables** (ABP `ExecuteUbergraph`, bytecode 0x3b29..0x3dbd and 0x3e7e..0x4028):
- `IsSkydivingLaunchPad = pawn.IsSkydivingFromLaunchPad()`, `IsUsingUmbrella = pawn.IsUsingUmbrella()`, `IsSkydiveStrafing = pawn.IsActivelyStrafingInAir()`, `IsGliding = pawn.IsPassivelySkydiving()`, `IsDiving = pawn.IsActivelySkydiving()`, `IsParachuteOpen = pawn.IsParachuteOpen()`, `IsSkydiving = pawn.IsSkydiving()` (native, see section C).
- `ForwardBack, LeftRight, UpDown = LocalAcceleration.{X,Y,Z}`; `LocalAcceleration = InverseTransformDirection(ActorTransform, WorldAcceleration)` while `IsAccelerating2D` (0x2d52..0x2da3), else (0,0,0) (0x2e34).
- `Parachute_ForwardBack, Parachute_LeftRight, SkyDive_LeftRight = LocalVelocity.{X,Y,Y}` (0x3cae..0x3d1a).
- `IsParachuteLeaning = Parachute_LeftRight != 0 || Parachute_ForwardBack != 0` (0x3d29..0x3d6b).
- `IsSkydiveLeaning = NearlyEqual(YawDeltaSmoothed, 0, 0.01) && SkyDive_LeftRight != 0` (0x3d76..0x3dbd).
- `FootIKAlpha = IsSkydiving ? 0 : 1` (0x3dcd); `UpperBodyAimOffsetAlpha = (IsPlayingMeleeAnim || ShouldApplyAimOffsetToFullBody || IsSkydiving || isDBNO) ? 0 : 1` (0x772c..0x778d); `FullBodyAimOffsetAlpha = ShouldApplyAimOffsetToFullBody && !IsSkydiving && !isDBNO` (0x779c..0x77ff).
- `Direction` (user enum `/Game/Animation/Game/MainPlayer/Parachute/Medium/Female/Direction`: 0 Center, 1 Forward, 2 Back, 3 Left, 4 Right), from LocalAcceleration (0x3eb5..0x3fc7), evaluated in this order, later wins: `LeftRight < -500 -> Left`; `LeftRight > 500 -> Right`; `ForwardBack < -500 -> Back`; `ForwardBack > 500 -> Forward`; if both NearlyEqual(0, 0.01) -> `Center` and `IsParachuteLeaning = false`.
- `ParachuteLeanTransition`: a DoOnce gate (0x3fce..0x4028): while `IsParachuteLeaning` the gate fires once and sets `ParachuteLeanTransition = true`; when not leaning the gate is reset and `ParachuteLeanTransition = false`. The `AnimNotify_SkyDiveLeanOut` event (function 1891) sets `ParachuteLeanTransition = false` (0xb566). The `*_Lean_*_Into` clips carry notifies `SkyDiveLeanInto` at 0.0637 s and `SkyDiveLeanOut` at ~2.40 s (glider clips) / ~3.80 s (umbrella clips) (sequence `Notifies`). So: a new lean plays the directional "Into" clip (rate 2 for glider, ~1.2 s real time), then at the LeanOut notify blends (0.75 s) back to the lean blendspace.
- Exact Kismet jump targets of the DoOnce use in-memory offsets that the disassembler does not translate; the true/false wiring above is from the block contents (UNRESOLVED: confirm the reset branch).

**BlendSpaces** (`fn-inspect props`; GridNum default 4 = UE4.16 `BlendSpaceBase.h:65` when not serialized; InterpolationType default 0 = Average):

| BlendSpace (Parachute/Medium/...) | X axis | Y axis | Interp (s) | Samples (anim @ x,y) |
|---|---|---|---|---|
| Female/BS_Skydive_Glide_2D | Yaw -7..7 | Pitch -85..0 | 0.25/0.25 | Skydive_Glide @0,0 and 0,-85; _Left @-7,0/-7,-85; _Right @7,0/7,-85 |
| Female/BS_Skydive_Glide_Lean | LeftRight -1200..1200 | Pitch -85..0 | 0.25 | Glide_Lean_Left @-1200,{0,-85}; Glide_Lean_Right @1200,{0,-85}; Glide @0,{0,-85} |
| Female/BS_Skydive_Dive_2D | Yaw -7..7 | Pitch -85..0 | 0.25 | Dive @0,0; Dive_Left @-7,0; Dive_Right @7,0; Dive_Down @0,-85; Dive_Down_Left @-7,-85; Dive_Down_Right @7,-85 |
| Female/BS_Skydive_Dive_Lean | LeftRight -1600..1600 | Pitch -85..0 | 0.25 | Dive_Lean_Left @-1600,0; Dive_Lean_Right @1600,0; Dive_Lean_Down_Left @-1600,-85; Dive_Lean_Down_Right @1600,-85; Dive @0,0; Dive_Down @0,-85 |
| Male/ and Female/BS_Skydive_HangGlider | Yaw -14..14 (GridNum 6) | Pitch -85..0 | 0.35 | HangGlider_Glide @0; _Left @-4.6667; _Right @4.6667; _Left_Max @-14; _Right_Max @14 (each at Pitch 0 and -85) |
| Female/BS_Skydive_HangGlider_Into | Yaw -7..7 | Pitch -85..0 | 0.35 | Glide_Into @0; Glide_Into_Left @-7; Glide_Into_Right @7 (Pitch 0 and -85) |
| Male/BS_Skydive_HangGlider_Lean | LeftRight -1950..1950 | ForwardBack -1950..1950 | 0.35 | Lean_Center_Pose @0,0; Lean_Back_Pose @0,-1950; Lean_Forward_Pose @0,1950; Lean_Left_Pose @-1950,0; Lean_Right_Pose @1950,0 |
| Female/BS_Skydive_HangGlider_Lean | same | same | 0.35 | Lean_Center / Lean_Back / Lean_Forward / Lean_Left / Lean_Right at the same points |
| Female/BS_Skydive_Parachute (umbrella) | Yaw -14..14 (GridNum 6) | "PItch" -85..0 | 0.35 | Parachute @0; _Left @-4.6667; _Right @4.6667; _Left_Max @-14; _Right_Max @14 |
| Female/BS_Skydive_Parachute_Into | Yaw -7..7 | Pitch -85..0 | 0.35 | Parachute_Into / _Into_Left / _Into_Right |
| Female/BS_Skydive_Parachute_Lean | LeftRight -2000..2000 | ForwardBack -2000..2000 | 0.35 | Lean_*_Pose at 0 / +-2000 |
| Female/BS_Skydive_HangGlider_GLIDER (glider mesh) | Yaw -7..7 | Pitch -85..0 | 0.25 | see D.2 |

**Clip lengths** (`SequenceLength`; `AdditiveAnimType` when set; all RateScale 1): Skydive_Dive*, Skydive_Dive_Lean_{Left,Right,Down_Left,Down_Right} 4.0 s; Skydive_Glide*, Skydive_Glide_Lean_{Left,Right} 8.0 s; *_Lean_*_Pose and *_Lean_Center 0.0333 s (`AAT_LocalSpaceBase`, RefPoseSeq = the Center pose); Skydive_HangGlider_Glide* (incl. Left/Right/_Max, Male and Female, and *_GLIDER) 2.6667 s; Skydive_HangGlider_Glide_Into* 4.6667 s; Skydive_HangGlider_Lean_*_Into 2.6667 s (`AAT_LocalSpaceBase`); Skydive_Parachute* 4.0 s; Skydive_Parachute_Into* 3.6 s; Skydive_Parachute_Lean_*_Into 4.0 s (`AAT_LocalSpaceBase`). The Dive_Lean / Glide_Lean full clips have a RefPoseSeq but no AdditiveAnimType (non-additive).

**Landing**: there is no skydive-specific landing state in the ABP; on landing `IsSkydiving` goes false, FullBody returns to `SplitBody` (0.2 s) and the normal AdditiveJumps land states (already ported in `fn-animgraph`) play. No Athena landing montage asset was found (`LandingMontagePair` exists on the pawn, SDK `FN_FortniteGame_classes.hpp` 0x1880, but no 1.8 BR asset feeds it; UNRESOLVED).

### D.4 Audio

| Use | Asset | Cue contents (`fn-inspect props`) |
|---|---|---|
| Skydive wind loop (`PlayerPawn_Athena` CDO `OnSkydivingLoop`) | `/Game/Athena/Sounds/Wind_Falling/Wind_UmbrellaClosed_Loop_01_Cue` | VolumeMultiplier 1.5, Attenuation `/Game/Sounds/Attenuation/Fort_VO_Player_Att`, SoundClass `Fort_Gadgets`; Mixer(InputVolume [1, 1.5]) of { WavePlayer `Wind_UmbrellaClosed_Loop_01` looping, Delay(0.2..0.5 s) -> Random(weights 1,1,1) of `sfx_parachute_skydive_loop_enter_01/02/03` } |
| Glider open loop (`DefaultGlider.OpenSoundLoop`) | `/Game/Athena/Sounds/Wind_Falling/Wind_UmbrellaOpen_Loop_01_Cue` | VolumeMultiplier 0.3, Fort_VO_Player_Att, Fort_Gadgets; WavePlayer `Wind_UmbrellaOpen_Loop_01` looping |
| Glider open (`OpenSound`) | `/Game/Sounds/Fort_Gadgets/Athena/Athena_Glider_Open_Cue` | VolumeMultiplier 3, Duration 4.377 s, Attenuation `Fort_Player_Umbrella_Athena_Att`; wave `/Game/Athena/Sounds/Glider/SFX_Glider_Open_Athena` |
| Glider close (`CloseSound`) | `/Game/Sounds/Fort_Gadgets/Athena/Athena_Glider_Close_Cue` | VolumeMultiplier 2, Duration 1.154 s, `Fort_Player_Umbrella_Athena_Att`; wave `SFX_Glider_Close_Athena` |
| Landing | `PlayerPawn_Athena` CDO `DefaultLandingSound` `/Game/Athena/Sounds/Character/Athena_Character_Land_Cue`, `DefaultHardLandingSound` `.../Athena_Character_LandHard_Cue` | (used by the generic landing code) |

Parachute loop volume by height (PlayerPawn_Athena `ExecuteUbergraph` 0x1217..0x1426, tick path, not on a dedicated server): only when `IsParachuteForcedOpen() && IsLocallyControlled()` and `ParachuteAudioLoop` valid: LineTraceSingle from actor location to the absolute point (0, 0, -10000) (TraceTypeQuery byte 9, trace-complex false), `d = |ImpactPoint - TraceStart|`, target = MapRangeClamped(d, 0, 6000, 0, 1), `ParachuteAudioLoop.VolumeMultiplier = FInterpTo(current, target, dt, 1)`. Note the end point is world (0,0,-10000), not location - 10000 (as written in the bytecode). Who creates `SkydiveAudioLoop` / `ParachuteAudioLoop` (native) is in section C.

### D.5 FX

From `/Game/Athena/PlayerPawn_Athena` (CDO + `ExecuteUbergraph`):
- `Contrails` array (CDO): [0] `/Game/Athena/Effects/Particles/PFX_FlyingContrail_Base`, [1] same, [2] `_Chrome`, [3] `_Fireball`, [4] `_Bubbles`, [5] `_Sparkles`. Spawn (0xf06..0x112a): `ActiveContrail = Contrails[RandomInteger(2)]` (so always `PFX_FlyingContrail_Base` in 1.8), SpawnEmitterAttached to `Mesh` at sockets `ball_l` (Contrail_A, UVOffset 0), `ball_r` (Contrail_B, 0.25), `pinky_01_l` (Contrail_C, 0.5), `pinky_01_r` (Contrail_D, 0.75); `bContrailReady = true`. A looping timer `ContrailCheck` every 0.1 s is set at 0xddf; a block at 0xc80..0xd98 sets all four `TrailAlpha = 0` and `TrailWidth = (0,0,0)`.
- Tick while `IsSkydiving()` (0x193e..): DoOnce -> if locally controlled SpawnEmitterAttached `PFX_FallingVapor` (`/Game/Athena/Effects/Particles/PFX_FallingVapor`) on `Mesh` (stored `Vapor Effect`). If not parachute-open and `bContrailReady`: `TrailAlpha = MapRangeClamped(|Velocity|, 1200, 3300, 0.1, 0.7)` and `TrailWidth = (MapRangeClamped(|Velocity|, 1200, 3300, 0.75, 1.25), 0, 0)` on all four. When not skydiving: reset the DoOnce, deactivate `Vapor Effect` (locally controlled), and if `bContrailReady`: clear timer `ContrailCheck`, Deactivate all four contrails, `bContrailReady = false`.
- Exact branch targets of the parachute-open path (0x19ff jumps to in-memory offset 10158) not translated; UNRESOLVED whether contrails are zeroed or left alone while the glider is open.

### D.6 Aerial HUD prompt (`/Game/Athena/HUD/AthenaAerialFeedback`)

Widget tree: HorizontalBox `Feedback` = [CommonTextBlock `Press` "Press"] [KeybindWidget_C `KeybindWidget_0`, `BoundAction "Jump"`] [CommonTextBlock `FeedbackText` "{Action}"]. Tick (`ExecuteUbergraph` 0x6..0x458): GameState cast `Athena_GameState_C`, owning player cast `Athena_PlayerController_C`, `phase = GameState.GetAerialPhase(PC)`:

| `EAthenaAerialPhase` | `Feedback` visibility | `FeedbackText` |
|---|---|---|
| 0 None, 1 BusCantExit | Collapsed (SetVisibility 1) | - |
| 2 BusCanExit | HitTestInvisible (3) | "To Jump" (key 377864294736257A6FC15C93BFB21574) |
| 3 Skydiving | HitTestInvisible | Format("To Deploy {Glider}", Glider = PC.CustomizationLoadout.Glider.DisplayName) (key 69B4CD994A4389965BA22D98DD59B1F5) -> "Press [Space] To Deploy Glider" |
| 4 Parachuting | HitTestInvisible | "To Skydive" (key 06FB95D54EA2E03D88BFE1AF883A8B81) |

All three texts are in namespace `[7542B0684376D8F3BF6D9B886221F029]`. The "To Skydive" prompt while the glider is open is the cooked-UI evidence that 1.8 lets the player close the glider in the air with Jump (the native toggle and its limits are in section C).

No altitude readout exists in 1.8: no `*Altitude*` asset (`fn-inspect ls '*Altitude*'` -> nothing) and the Athena HUD folder has no skydive/height widget (`fn-inspect ls 'FortniteGame/Content/Athena/HUD/**/*.uasset'`).

---


---

## UNRESOLVED

What could not be established, and what was tried. Items closed by another section are listed in 0.1 and not repeated here.

### Spawn and match flow
1. **Where the jumping pawn spawns, and who uses `ExitLocation` (-580, 0, 200).** The start spot is the aircraft actor (`ChoosePlayerStart` 0x1403d2d40), and rotation is yaw-only (`FinishRestartPlayer` 0x1403d7bc0). No native read of aircraft +0x3D4 was found.
   - Tried: decomp-grep `+ 0x3d4)`, callers of GetAircraft 0x140427930, and AthenaAircraft BP disasm.
   - `AFortGameModeAthena::SpawnDefaultPawnFor_Implementation` 0x1404071a0 returns null (disasm 0x140407278 -> 0x1404072e6).
   - Next step: fully disassemble `AFortGameMode::SpawnDefaultPawnFor_Implementation` 0x1405a0dd0 and `AGameModeBase::RestartPlayerAtPlayerStart` 0x1421a7ec0.
2. **`Config` values** `WarmupCountdownDuration` (60), `WarmupEarlyCountdownDuration` (15) and `bUseRandomTimeOfDay` (true) are code defaults only. The DefaultGame.ini overrides are encrypted and were not read.
3. **Unnamed gates and fields:**
   - The GamePhaseStep gate FUN_14041eef0 (GameInstance +0x530/+0x538 flag); GameState vfuncs +0x950 and +0x5A0.
   - PC +0x6DC bit 0x40 ("loaded"); PlayerState +0x3B4 bit0; GameState +0x13E8.
   - PC +0x31B5 / +0x31B6 / +0x31B7, inferred to be bInAircraft / bHasExitedAircraft / jump guard (the SDK marks this region MISSED OFFSET).
4. FUN_1407d94d0(pawn, 0), called on each warmup pawn before it is destroyed at bus start, is not decoded.
5. The `MatchState` FName compared in `ReadyToStartMatch` (DAT_14685ce58) is inferred to be `WaitingToStart`.
6. Warmup loadout and items granted on the warmup spawn were not investigated beyond the 72 floor-loot and 1 chest actors in Lobby_001.

### Bus actor, camera, HUD
7. Minimap indicator FUN_1408d9440(indicator, 4): the meaning of `4` and the minimap vs full-map behaviour were not traced.
8. **Bus route line on the map:** not found. No UI-module code reads `GameState.Aircraft`, and no "FlightPath" string appears in UI code. Treated as absent, but not proven absent.
9. **Athena `PlayerCameraManagerClass`:** not confirmed.
   - `Athena_PlayerController` CDO does not set it, and the string `MainPlayerCamera` is not in the exe, so it probably comes from an encrypted ini.
   - `/Game/Blueprints/Camera/Original/MainPlayerCamera` is the only asset that fills the Skydive camera classes.
   - Pitch limits +/-89.9 are the engine default (UE4.16 `PlayerCameraManager.cpp:44-45`).
10. **Camera instance blending** after ExitAircraft clears `OverrideCameraClass`: does the aircraft mode keep blending, using the 2/2/1 origin interp toward the pawn?
    - Depends on `AFortPlayerCameraBase::UpdateViewTarget` 0x140502ff0 (vfunc +0x240/+0x250), which was not fully decoded.
    - The `FortCameraMode_ThirdPerson` math for the skydive modes was not traced either.
11. **Not dumped:**
    - The lengths and curves of the widget animations `ModeChanged`, `IconPulse` and `IconSlide` (AthenaGamePhaseChangeWidget).
    - The parameters of the materials `M_GamePhase_Bus` and `M_GamePhase_Jump`.
    - The wave of `Fort_GamePhaseChange_Message_Athena_Cue` (SoundNodeWavePlayer_24 has no SoundWaveAssetPtr in the dump).
12. Where `BP_Bus_Lobby_SoundController` is placed, and its exact step-to-branch wiring.
13. Static-mesh material slots resolve to null in `fn-inspect`. The bus materials come from package import strings (one material per package).

### Movement
14. **On-foot glider toggle:** the input action name that reaches `AFortPlayerController` vfunc_646 0x14083ad60 was not found. Tried the "Jump" FName literals and xrefs to 0x14083ad60.
15. **Control pitch in MoveForward input** (0x1407f6220 / 0x14045d500): that it keeps the pitch is inferred, because `IsActivelySkydiving` needs a negative input Z.
16. The gameplay tag that StartSkydiving adds (`FFortGlobalTags` +0x2E8) is not mapped.
17. Primitive vtable +0x538 in Athena `ProcessLanded` / `IsValidLandingSpot` / `Landed` is assumed to be `CanCharacterStepUp`.
18. FUN_14080d0a0 (called on every skydive/glider transition) and 0x14044ba30 (parachute mesh refresh) are not decoded.
19. **World gravity** is still open (FALLING.md: exe default -980 vs the port's -2800). Every g-dependent skydive figure (decay rates, time to terminal velocity) inherits this.

### Animation, audio, FX
20. The default glider is set by data: `/Game/Athena/Athena_PlayerController` CDO `CustomizationLoadout.Glider = /Game/Athena/Items/Cosmetics/Gliders/DefaultGlider` (and `Pickaxe = DefaultPickaxe`). Not traced: the native path that copies the PC loadout to the pawn (`OnRep_CustomizationLoadout`) and selects the glider mesh / AnimClass.
21. **Kismet branch wiring** reconstructed from block contents, with in-memory jump targets not translated:
    - the ABP `ParachuteLeanTransition` DoOnce reset branch;
    - the PlayerPawn_Athena parachute-open contrail branch (whether contrails are zeroed while the glider is open) and the `ContrailCheck` entry;
    - the branches in AthenaGamePhaseChangeWidget and AthenaAerialFeedback.
22. The emitter internals of `PFX_FlyingContrail_Base` and `PFX_FallingVapor` were not dumped: modules, and how the `TrailAlpha` / `TrailWidth` / `UVOffset` parameters are used.
23. No BR landing montage exists. `LandingMontagePair` is a generic GAS montage replication field (C 4.3).
24. The native creation of `SkydiveAudioLoop` / `ParachuteAudioLoop` is traced in C 3.3 (2D loops). Their fade parameters are covered only for the 0.5 s skydive fade-out.

## Coordinator addendum (2026-10-05)
- **Gravity is RESOLVED.** `PhysicsSettings:DefaultGravityZ=-2800` and `DefaultTerminalVelocity=4000`, from `C:\FortniteRef\scratch\CookedIniVersion.txt`, extracted from the unencrypted pakchunk0 file. Measured in game at 28.0 m/s². Remove the gravity item from UNRESOLVED.
- Before porting, check `CookedIniVersion.txt` for the other "encrypted config" UNRESOLVED items, such as the camera manager class (`PlayerCameraManagerClass` under `Game` or `Engine` sections) and the input action names (`ConfigD_Athena` ActionMappings).
