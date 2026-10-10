# Airborne animation (jump / fall): what 1.8 does and how the port matches it

Scope: the player AnimBP `/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint` while the pawn is airborne.
Sources: `fn-inspect disasm` bytecode (offsets below are ubergraph offsets from `animbp_disasm.txt`, hex), `reports/animation-graph-nodes.json`
(node index = array position), `reports/animation-state-machines.json` (rules / durations / blend profiles; its `boundInputs` are NOT used),
`fn-inspect props` of the jump clips. Gravity -2800 cm/s^2 and JumpZ 600 cm/s are the recovered values (flight 0.429 s) and were not changed.

## 1. What 1.8 does while airborne

### Variables (BlueprintUpdateAnimation, ubergraph 0x6ba5 ..)
* `IsOnGround = (MovementMode == 2) || (MovementMode == 1)` (0x6c3b..0x6caf: Walking / NavWalking). A jump or a fall sets MOVE_Falling, so it is false for the
  whole flight, from the takeoff tick.
* `bIsJumping`, `bIsFalling`, `bLandingPredicted`, `PredictedFallTimeLeft` are never assigned by the Blueprint (native `FortAnimInstance`, not in any package):
  **ASSUMED** (graph.rs:337): bIsJumping = airborne and vz > 0, bIsFalling = airborne and vz <= 0, bLandingPredicted = bIsFalling and time-to-touchdown <= 0.25 s x 3.
  Evidence that this is the only split consistent with the machine: AdditiveJumps/IdleAdditive tries `bIsFalling` (node 219) BEFORE `bIsJumping` (node 218), so
  bIsFalling must be false while the jump clip is supposed to play.
* `IsMoving2D = VSize2D(GetVelocity()) > 0` (0x748b), `IsAccelerating2D = VSize2D(GetCurrentAcceleration()) > 0` (0x2e76): nothing airborne-specific, they stay true
  in the air while input is held. `Speed2D`, `LocalVelocity`, `LocalVelocityYawAngle`, `SpeedAdjustedPlayrate` (0x2e9c..0x3303) are only refreshed while IsMoving2D.
  `VelocityZ` (0x73ed..0x7452 = IsFalling ? Z : 0) is read only by the FullBody `Stunned` rule.
* The jog start / stop / pivot triggers are forced to false when `!IsOnGround` (0xa9db..0xaa72, also `ShouldDisableJogTransitions`), so no JogStart / JogStop /
  JogPivot state can be entered in the air. The DoOnce gates are NOT re-armed by that branch.
* `JumpUpperBodyBlendWeight = FInterpTo(.., IsPlayingMeleeAnim ? 1 : 0, dt, 12)` (0x6e42): 0 unless a swing montage plays.
* `FallPlayRate` / `FallAnimPlayTime` (0xa6bd..0xa7ce): `bLandingPredicted ? FMin(FMax(0.4 - FallAnimPlayTime, 0) / FMax(PredictedFallTimeLeft, 0.01), 1.2) : 1` (21 Fall players, node 280 ..).
* `JumpLandStarted` (0x7c41..0x7c7d): `if GetCurveValue(MeleeAnim) > 0.5 { JumpLandStarted = true }` with NO else branch; `AnimNotify_JumpLandFinished` (0xb44e) sets it false.

### State machines (transition rules followed through each TransitionResult.bCanEnterTransition assignment)
LocomotionLowerBody (machine node 471, Layered node 345 blend pose; baked state roots in `poseGraph.stateRoots`):
* Movement -> Idles, TransitionResult_068AA4.. (0x4e54..0x4ec9): `!IsOnGround || !IsAboveMinimumLocomotionSpeed || bPlayingRootMotion || !IsMoving2D`, 0.4 s HermiteCubic, blend profile **FastFeet**
  (legs x2, ik feet x3: `anim_data.rs` FAST_FEET). It fires on the first airborne tick, so a jog jump fades the Movement state out over 0.4 s (legs are fully idle after ~0.2 s, pelvis / spine after 0.4 s).
* Idles -> Movement, TransitionResult_CCE490.. (0x4f9a..0x4fed): `IsMoving2D && !bPlayingRootMotion && IsOnGround && IsAboveMinimumLocomotionSpeed`; Idles -> JogStart/Stop/PrePivot need the triggers above: nothing re-enters a locomotion state in the air.
* Turns -> Idles (0x4d1b..0x4de0) also fires on `bIsFalling || bIsJumping`. ShuffleInPlace -> Idles on `!IsOnGround`.
* Idles pose = BlendListByBool node 438 (pin0 `M_Melee_TwoHanded`, pin1 cached UpperBody) selected by `(bIsFalling || bIsJumping) && (IsPlayingMeleeAnim || IsCrouching)` (0x57c2..0x5804), blend [0.2, 0.1].

FullBody (SplitBody contains the lower/upper blend + LocomotionAdditive):
* SplitBody -> Sprinting needs `!bIsJumping && !bIsFalling && !bRecentlyFired && Speed2D >= SprintSpeedThreshold` (0x2bcb.. region); Sprinting -> SplitBody `(!IsCrouchSprinting && !bIsSprinting) || !IsOnGround || !IsAccelerating2D` 0.3 s HermiteCubic (no profile).
  => a sprint jump fades the whole-body sprint pose out over 0.3 s and it cannot return in the air.

LocomotionAdditive:
* MovementAdditive -> IdleAdditive `bIsJumping && !bPlayingRootMotion && !IsCrouching` (TransitionResult_200BE7.., 0x2c73..) 0.08 s HermiteCubic, **SlowTorso**; (crouching: 0.35 s no profile);
  and `!IsAccelerating2D || !IsOnGround || !IsAboveMinimumLocomotionSpeed || bPlayingRootMotion` 0.4 s FastFeet. IdleAdditive -> MovementAdditive needs IsOnGround.

AdditiveJumps (machine node 330, `GetStateWeight(184, n)` index 184; states IdleAdditive 0, NewJump 1, NewFall 2, NewFallLand 3, NewJumpLoop 4):
* Idle -> Fall `bIsFalling && !bPlayingRootMotion` 0.4 s Linear (node 219, first); Idle -> Jump `bIsJumping && !bPlayingRootMotion && !IsCrouching` 0.08 s Linear (218).
* Jump -> Fall `!bIsJumping && (bLandingPredicted || !bIsFalling)` 0.2 s (217, 0x2b6c..0x2ba4); Jump -> Loop automatic remaining time 0.2 s (216); Loop -> Fall same rule as Jump (212).
* Fall -> Land `!bIsFalling` 0.04 s (215, 0x2bf2); Land -> Idle automatic remaining time 0.2 s HermiteCubic (214), or `bIsJumping || bIsFalling` 0.08 s (213).
* Sequence players: JumpUp 1.0, Apex 0.5 (node 236), Fall `FallPlayRate`, Land 1.0, StartPosition 0 (nodes 321 / 236 / 280 / 244). Melee WeaponCoreAnim = 0 -> ARDS clips (enumToPoseIndex of nodes 240 / 275).
* Land state root 277: BlendListByBool<JumpLandStarted> node 259: pin0 (true) `JumpLand_NoWep_CMM_Additive_Core` (0.8333 s, node 258), pin1 the enum list (melee: `M_JumpLand_ARDS_Additive`, 0.6 s).
  `fn-inspect props JumpLand_NoWep_CMM_Additive_Core`: Notifies[0] = `JumpLandFinished` at 0.7680283 s (TriggerWeightThreshold 1e-5).

Jump additive application (pose chain `poseChainFromJumpBlend`):
* LayeredBoneBlend 484 (base = AdditiveJumps, blend = IdentityPose 485, branches Spine_01 depth 2 + Ik_hand_root 0, weight `JumpUpperBodyBlendWeight`, 0x6695) -> ApplyAdditive 333 with Alpha
  `(1 - FMin(StateWeight(AdditiveJumps, NewFallLand) + StateWeight(AdditiveJumps, IdleAdditive), 1)) * FMin(Speed2D, 1)` (0x4052..0x4108) -> ApplyMeshSpaceAdditive 331 with Alpha
  `SelectFloat(0, 1 - TargetingWeight*0.33, NearlyEqual(GetCurveValue(RemoveJumpAdditive), 1, 0.01))`, times 0.2 when `!bIsFalling && IsCrouching` (0x3a2d..0x3b14).
* LeanAdditiveAlpha (node 486 ApplyAdditive): 1 while `IsMoving2D && !bIsTargeting` (0x7103, 0x710d), else FInterpTo 0 at 8. **No airborne term**: the jog lean additive (`JogAR_LeanC_CMM`, BlendSpace1D `JogAdditiveLean`, X = LeanAngle, PlayRate = SpeedAdjustedPlayrate, 0x6a72..0x6a9a) stays at alpha 1 in the air.

### Net effect for a jog / sprint jump
No locomotion state is entered while airborne. The jog (Movement) blend space and the sprint clip are only faded out, with the state weights `1 - Hermite(t / 0.4)` (Movement; legs reach idle at ~0.2 s through FastFeet),
`1 - Hermite(t / 0.3)` (Sprinting), `Hermite(t / 0.08)` (IdleAdditive). They are still blending during the first ~0.2-0.4 s of the 0.43 s flight; that is the 1.8 behaviour, not a port artefact.

## 2. Comparison with the port: differences found

| # | 1.8 | Port before | Where (now) |
|---|-----|-------------|-------------|
| 1 | `JumpLandStarted` latches true on MeleeAnim > 0.5 and is cleared only by the JumpLandFinished notify (0.768 s into `JumpLand_NoWep_CMM_Additive_Core`) | assigned every tick from the swing flag (`jump_land_started = p.melee_land`), so it dropped one tick after the swing and the NoWep Core land never played after a swing ended | graph.rs:453 (latch), 1040 (notify), 926 (`land_core_t`) |
| 2 | Land state's automatic Land -> Idle rule uses the relevant player's remaining time: 0.8333 s Core land when pin0 is selected, else 0.6 s ARDS | constant 0.6 s | graph.rs:775 `set_land_len`, 979 |
| 3 | Core land player (node 258) runs on its own clock (restarts when relevant again) | sampled with the Land state time | graph.rs:1205 `jump_source` uses `land_core_t` |

Everything else on the airborne path was compared and is identical: IsOnGround source, all transition rules / durations / ease / blend profiles (already asserted against the report by
`transitions_match_the_baked_machines` and `rule_text_from_the_bytecode_equals_the_rust_rules`), the FullBody/LocomotionAdditive/LowerBody air transitions, the idle-pose BlendList (graph.rs:1030),
FallPlayRate (graph.rs JumpMachine::update_fall_rate), ApplyAdditive 333 alpha (graph.rs:1254), ApplyMeshSpaceAdditive 331 alpha (graph.rs:1257), JumpUpperBodyBlendWeight (graph.rs:346), lean alpha (graph.rs:380), Movement BlendSpace
play rates (SpeedAdjustedPlayrate, graph.rs:376) and state-entry restarts (take_entered in AnimState::step).

## 3. ASSUMED / not provable from the data (left as is, labelled in code)
* bIsJumping / bIsFalling / bLandingPredicted / PredictedFallTimeLeft (native, graph.rs:334-337) and `bIsSprinting` = `Controller::running` (native `IsSprinting()`).
* `RemoveJumpAdditive` curve = 0 and `AdditiveLayer` curve = 0 in every player clip (graph.rs:1256 comment; not re-verified here).
* LayeredBoneBlend nodes 345 / 484 / 503 carry no serialized `bMeshSpaceRotationBlend`, i.e. the engine default (false), while the port evaluates them with `mesh_space:true` (graph.rs:1229, 1251, 1097).
  Left unchanged: the choice predates this work, 484's weight is 0 outside a swing, and 345 shapes the whole body pose (not an airborne-only item). Needs a pose-oracle check before changing.
* FastFeet / SlowTorso semantics (per-bone blend weight = clamp(w * scale)) as implemented in skeleton.rs / fn-pose eval.
* Notify firing: relevance of the Core land player = Land state weight > 1e-5 and BlendList pin0 weight > 1e-5, clock restarts when not relevant; "relevant player of the Land state = the higher BlendList weight" (graph.rs:978-980, 1037-1043).
* `p.melee_land` = "swing montage playing" (the real MeleeAnim curve > 0.5 is only approximated by the swing flag); unchanged. `sim_tick.rs` (headless twin) passes `melee_swinging:false`; the game path `GameSim::step_anim` passes the swing flag.

## 4. Fix
graph.rs only (`crates/fn-animgraph/src/graph.rs`): latch + notify (`Vars::update`, `AnimState::step`, new `land_core_t`, `JUMP_LAND_CORE_LENGTH`, `JUMP_LAND_FINISHED_NOTIFY`),
`JumpMachine::set_land_len`, `jump_source` Land uses `land_core_t`. No gun / hand / recoil code, `slot()`, `upper_pre_slot()`, CopyBone / hand-IK chain or `SlotBlend` touched.
The jog / sprint fade-out in the air needed no change: the machines already equal the 1.8 rules.

## 5. Tests (crates/fn-animgraph, `graph::tests`)
* `airborne_jog_locomotion_weights_follow_the_1_8_state_machines`: jog (365 cm/s) takeoff; for each of the 25 airborne ticks Movement weight = 1 - Hermite(t/0.4), IdleAdditive = Hermite(t/0.08), FullBody stays SplitBody,
  pose-tree leaf weight of `M_Jog_AssaultRifle_*` = the Movement state weight (0 after 0.4 s), jog additive leaves = MovementAdditive weight, no walk / sprint / crouch clip, lean additive stays 1, Movement never re-entered.
* `airborne_sprint_pose_fades_out_over_0_3s_and_never_returns`: Sprinting weight = 1 - Hermite(t/0.3), `Sprint_TwoHanded` leaf = that weight, jog leaves = (1 - sprint) * Movement.
* `standing_jump_has_no_locomotion_clip_in_the_pose_tree`: no locomotion clip at any airborne tick; lower body stays Idles = 1; jump additive clips present from tick 2.
* `jump_land_started_latches_until_the_land_core_clip_fires_its_notify`: flag latches across the flight, is cleared at 0.768 s of the Core land, Land lasts to 0.633 s with the 0.833 s clip, and a landing with no latched flag uses the 0.6 s ARDS land (Land ends ~0.4 s).

Results (`$env:FN_OLD_ROOT='...\FortniteRuntime'; $env:FN_TEST_ROOT='...\FortniteGame'; $env:CARGO_TARGET_DIR='D:\target-jumpair'`):
* `cargo build --release -p fn-app`: finished OK.
* `cargo test --release -p fn-animgraph`: 44 passed, 0 failed (40 existing + 4 new).
* Not run: `jump_air` in fn-pose / fn-legacy tests that compare against the OLD prototype logs (recorded before the gravity correction); e.g. fn-legacy
  `the_legs_leave_the_walk_cycle_when_the_jump_starts...` asserts a flight of >= 0.5 s and may legitimately differ with the 0.43 s flight.

## 6. Files changed
* `crates/fn-animgraph/src/graph.rs` (fix + 4 tests + helpers `leaf_weights`, `tree_weight`, `air_run`)
* `crates/fn-animgraph/JUMP_AIR.md` (this file)


## 7. Re-verification (second pass) - exe-confirmed items and the BlendProfile fix
Confirmed from the 1.8 exe (decompile `FUN_14040e8a0` = FortAnimInstance native update, size 1480): `IsFalling = CharacterMovement->IsFalling()` (vtable +0x508);
`bIsJumping (+0x398) = IsFalling && !(PawnVelocityZ(+0x374) < 0)`; `bIsFalling (+0x399) = IsFalling && !bIsJumping`; landing prediction only when `bShouldPredictLanding (+0x39a) && bIsFalling`,
else `bLandingPredicted (+0x39b) = false`, `PredictedFallTimeLeft (+0x39c) = 0`; the predictor sweeps up to `min(FallLookAheadMaxIterations (+0x3a4), 5)` sub-steps of `FallLookAheadSubStepping (+0x3a0)` s
along the current velocity (gravity-scaled, clamped to terminal velocity) with a collision trace (`Fort_PredictLanding`); on a hit `PredictedFallTimeLeft = (sqrt(vz^2 - g*2*h) + vz)/g`-style
(0x40ee..). So section 3's bIsJumping/bIsFalling/bLandingPredicted/PredictedFallTimeLeft are no longer ASSUMED (the port approximates the sweep by `time-to-touchdown <= 0.75 s`).
Transition rules / durations / profiles were re-read from `fn-inspect disasm` and the baked report: Movement -> Idles (FastFeet 0.4 s), Sprinting -> SplitBody (0.3 s), MovementAdditive -> IdleAdditive
(SlowTorso 0.08 s), AdditiveJumps rules, the DBNO rule at 0xb650 belongs to the DBNO machine (not the player locomotion).
FOUND AND FIXED: the transition BlendProfile math. UE 4.16 / exe `FAnimationActiveTransitionEntry::Update` (RVA 0x1fe9e20) + `NormalizeDataWeight` (0x2004740): next-state per-bone data `a*s`, previous
`(1-a)/s`, normalised per bone => next weight `a s^2/(a s^2 + 1 - a)` (`blend_profile_weight`). The port used `clamp(a*s)`. For FastFeet legs (s = 2) in a 0.4 s jog->idle fade: old 0.31 / 1.0 / 1.0 at
t = 0.1 / 0.2 / 0.3 s, exe 0.43 / 0.80 / 0.96 (the legs leave the jog slightly faster at first but finish later). `fn-pose/src/eval.rs` (the game's evaluator, `Node::MixProfile`) still has the old
`(w * s).clamp(0, 1)`; it must call `fn_animgraph::blend_profile_weight(w, s)`. Probe: `cargo test --release -p fn-animgraph --test jump_probe -- --nocapture`.
