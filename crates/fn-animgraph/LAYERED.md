# LayeredBoneBlend `bMeshSpaceRotationBlend` audit

Source: `fn-inspect --root <install> props /Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint`
(CDO export 2227, 9 `AnimGraphNode_LayeredBoneBlend_*` structs). Node ids are from the old `animation-graph-nodes.json`.

## Key finding
The old report (`animation-graph-nodes.json`) lists NO `bMeshSpaceRotationBlend` field for any of the 9 nodes. That is a
limitation of the old parser (it dropped the bool tags), not of the data: reading the package directly, EVERY node carries
an explicit `bMeshSpaceRotationBlend` (and `bHasRelevantPoses`, `bBlendRootMotionBasedOnRootBone`) tag. So the note in
`JUMP_AIR.md` ("345 / 484 / 503 carry no serialized value, engine default false") is wrong. The serialized values for
345 / 484 / 503 are all `true`. The exe constructor default is therefore irrelevant (not looked up).

All nodes: BlendWeights=[1.0] (one blend pose, overridden at runtime by the exposed-input copy record where listed),
bBlendRootMotionBasedOnRootBone=true, bHasRelevantPoses=false, no other serialized flags.

| node | tag | BasePose / BlendPose | mesh_space (package) | CurveBlendOption | branch filters (bone:depth) | weight pin | port |
|---|---|---|---|---|---|---|---|
| 135 | FEA20 | 138 / 133 | true | MaxWeight | Clavicle_L:2, IK_Hand_L:0 | LeftArmMaskBlendWeight | not ported |
| 147 | 0249E | 150 / 145 | true | MaxWeight | Clavicle_L:2, IK_Hand_L:0 | LeftArmMaskBlendWeight | not ported |
| 161 | 06311 | 163 / 155 | true | MaxWeight | Clavicle_L:2, IK_Hand_L:0 | LeftArmMaskBlendWeight | not ported |
| 175 | E8D5D | 177 / 168 | true | MaxWeight | Clavicle_L:2, IK_Hand_L:0 | LeftArmMaskBlendWeight | not ported |
| 345 | C0F5C | 346 / 471 | true | BlendByWeight | Root:0, Ik_hand_root:-1, Spine_01:-2 | none (1.0) | graph.rs:1229 `mesh_space:true`, filters match |
| 404 | 315FB | 405 / 400 | true | MaxWeight | ik_foot_root:0, thigh_r:0, thigh_l:0 | none | not ported |
| 484 | E2124 | 330 / 485 | true | BlendByWeight | Spine_01:2, Ik_hand_root:0 | JumpUpperBodyBlendWeight | graph.rs:1251 `mesh_space:true`, filters match |
| 500 | B2DAF | 10 / 502 | **false** | MaxWeight | clavicle_r:3, ik_hand_root:0, weapon_r:0 | RightArmMaskWeight | graph.rs:1116 `mesh_space:false`, filters match |
| 503 | C82F1 | 500 / 504 | true | MaxWeight | root:0, ik_hand_root:-1, Spine_01:-1 | CrouchingLowerBodyMaskWeight | graph.rs:1117 `mesh_space:true`, filters match |

## Port before / after
No mismatches: every Layered construction in graph.rs (1116, 1117, 1229, 1251) already equals the package data in
mesh_space, filters and weight source. No code was changed (before == after). Nodes 135/147/161/175/404 (left-arm and
foot layers) have no `Node::Layered` in the port at all; they are unported, not mismatched.
CurveBlendOption is not modelled by `Node::Layered` (skeleton.rs notes curve blending is scalar); noted only.

## Tests (FN_OLD_ROOT / FN_TEST_ROOT set, no source change)
- `cargo test --release -p fn-animgraph`: 44 + 19 + 2 passed, 0 failed.
- `cargo test --release -p fn-pose`: same failures as before the audit (unchanged by definition): 1 in `jump_air`
  (state transitions vs old log) and 5 in `vs_old_logs` (pickaxe/dir old-log comparisons); the rest pass.

Suggested follow-up: correct the `JUMP_AIR.md` note about 345 / 484 / 503.

## Left-arm and JogStop layers (nodes 135 / 147 / 161 / 175 / 404)
Sources: animation-graph-nodes.json links + animation-state-machines.json `machines[].states[].rootNodeSerialized`, `fn-inspect props` (bools),
bytecode dump `animbp_disasm.txt` (offsets below are ubergraph offsets).

### Positions (each node is the `Result` of a state's Root node)
| node | Root | machine / state | base pose | blend pose |
|---|---|---|---|---|
| 175 | 181 | LocomotionAdditive / JogStartAdditive | 177 BlendListByEnum<LocomotionCardinalDirection> (0x23eb) of M_Jog_{E,W,S,N}_Start_Additive | 168 TwoWayBlend: A = 169 Idle_Noise (loop), B = 173 jog tree, Alpha = FClamp(Speed2D / 300, 0, 1) (0x21b7..0x21f0) |
| 161 | 167 | LocomotionAdditive / JogStopAdditive | 163 BlendListByEnum<StopCardinalDirection> (0x20a4) of M_Jog_*_Stop_Additive | 155 BlendListByBool<IsMoving2D> (0x1ec2), BlendTime [0.1 true, 0.6 false]: true = 159 jog tree, false = 154 Idle_Noise (loop) |
| 147 | 153 | LocomotionAdditive / JogPrePivotAdditive | 150 BlendListByEnum<PrePivotCardinalDirection> (0x1d0d) of M_Jog_*_Pivot_Additive | 145 jog tree |
| 135 | 141 | LocomotionAdditive / JogPostPivotAdditive | 138 BlendListByEnum<PostPivotCardinalDirection> (0x16ab) of M_Jog_*_Pivot_Additive | 133 jog tree |
| 404 | 410 | LocomotionLowerBody / JogStop | 405 RotateRootBone (Yaw = StopAnimDeltaAngle, 0x5519) over 406 BlendListByEnum<StopCardinalDirection> (0x552e) of M_Jog_*_Stop_Core | 400 BlendListByEnum<StopCardinalDirection> (0x5504), same M_Jog_*_Stop_Core clips, unrotated |

Jog tree (blend pose of 135 / 147 / 161 / 175): BlendListByBool<bIsRelaxedLevel1> (nodes 133 / 145 / 159 / 173; BlendTime [0.4, 0]):
true -> BlendListByBool<bIsRelaxedLevel2> (131 / 143 / 157 / 171) of [JogAdditiveRelaxedLvl2BS, JogAdditiveRelaxedBS]; false -> JogAdditiveBS.
Each BlendSpacePlayer: X = LocalVelocityYawAngle - GetCurveValue(JogBlendspaceRotation), Y = Speed2D, PlayRate = JogPlayRate (135 / 147: 0x14e3..) or
SpeedAdjustedPlayrate (161 / 175: 0x1f2e, 0x2260). Pin 0 of a BlendListByBool is the `true` pin. The four Layered nodes read mesh space = true, CurveBlendOption MaxWeight,
BlendWeights[0] = LeftArmMaskBlendWeight through the exposed-input copy (Array_Set at 0x162a / 0x1cb4 / 0x2075 / 0x23a7). Node 404: BlendWeights = [1.0], no weight pin.

### LeftArmMaskBlendWeight (BlueprintUpdateAnimation, 0x7b23..0x7c31)
```
t = (WC==5 || WC==13) || (WC==1 && bIsRelaxedLevel2 && !bIsTargeting) || WC==15       (0x7b23..0x7bd8)
LeftArmMaskBlendWeight = FInterpTo(LeftArmMaskBlendWeight, t ? 1 : 0, DeltaSeconds, 10)   (SelectFloat 0x7bee, FInterpTo 0x7c0d, store 0x7c31)
```
No curve is involved (the only curves in that block are BothHandFK / LeftHandFK for the hand-IK alphas, 0x7954..0x7b14).
Enum values: Melee 0, Pistol 1 and GoingCommando 9 are confirmed by the report (`weaponCoreAnim`, list 127 / 463). 5 / 13 / 15 come from the declaration order of the
`EFortWeaponCoreAnimation::` name table in FortniteClient-Win64-Shipping.exe (Melee, Pistol, Shotgun, PaperBlueprint, Rifle, MeleeOneHand, MachinePistol, RocketLauncher,
GrenadeLauncher, GoingCommando, AssaultRifle, TacticalShotgun, SniperRifle, TrapPlacement, ShoulderLauncher, AbilityDecoTool, _MAX = 17 entries, matching the 17-entry
enumToPoseIndex arrays; the three confirmed values sit at the right indices) so 5 = MeleeOneHand, 13 = TrapPlacement, 15 = AbilityDecoTool.

### What was added (crates/fn-animgraph/src/graph.rs only)
- `Vars::update`: the existing `left_arm_mask_blend_weight` line targeted Shotgun / RocketLauncher / ShoulderLauncher; corrected to MeleeOneHand / TrapPlacement /
  AbilityDecoTool (+ the unchanged Pistol && bIsRelaxedLevel2 && !bIsTargeting term). Same FInterpTo speed 10.
- `AnimState::left_arm_layer` / `jog_additive_tree` + the four `additive_state_node` arms (JogStart / JogStop / JogPrePivot / JogPostPivot) now build
  `Node::Layered{filters [Clavicle_L:2, IK_Hand_L:0], weight = left_arm_mask_blend_weight, mesh_space: true, curve_blend: MaxWeight}`; the Layered is only built while the
  weight is > 0 (weight 0 = base unchanged, as for the right-arm layer; always the case for the pickaxe).
- `AnimState.add_stop_moving` (BlendListByBool<IsMoving2D> of node 155, BlendTime [0.1, 0.6], re-initialised when JogStopAdditive is entered).
- `lower_state_node(JOG_STOP)`: now `Node::Layered{base: RotateRoot(stop clip, StopAnimDeltaAngle), blend: stop clip, filters [ik_foot_root:0, thigh_r:0, thigh_l:0], weight 1.0, mesh_space true, MaxWeight}`.
- No change to `slot()`, `upper_pre_slot()`, the CopyBone / hand-IK chain, `SlotBlend`, `CurveBlend`, the jump machine, `sample_blended_curve`, recoil or movement; scene.rs untouched
  (the weight needs no curve read).

### ASSUMED / not modelled
- ASSUMED: JogAdditiveRelaxedBS / JogAdditiveRelaxedLvl2BS / per-weapon JogAdditiveBS resolve to the one jog additive space the port keeps (`sp.jog_add`,
  M_Jog_Melee_TH_Additive), exactly as `additive::MOVEMENT` already does; the relaxed BlendListByBool cross-fades (0.4 s) therefore have no visible effect and are not built.
  With the corrected weapons the left-arm blend of MeleeOneHand / TrapPlacement / AbilityDecoTool / relaxed Pistol would use that weapon's own CoreAnims additive space in the game.
- bIsRelaxedLevel2 is never set by the port (stays false), so the Pistol term is dead at runtime today (pre-existing).
- Pre-existing, not changed: the port's JogStartAdditive base uses `start_transition_initial_direction`, node 177 reads LocomotionCardinalDirection (0x23eb).
- CurveBlendOption MaxWeight is carried as the flag; the curve behaviour is `CurveBlend`'s (unchanged).

### Tests (graph.rs `tests`)
`left_arm_mask_blend_weight_follows_the_bytecode_formula` (enum set, FInterpTo 1/6 step at 60 Hz, convergence), `left_arm_layers_sit_in_the_four_additive_jog_states_with_the_serialized_settings`
(one position per node 175 / 161 / 147 / 135: filters, mesh_space, MaxWeight, weight, base clip, blend structure), `jog_stop_additive_moving_list_uses_the_serialized_blend_times`,
`jog_stop_lower_body_layer_has_the_serialized_filters_and_rotates_only_the_base` (node 404).
Results: `cargo test --release -p fn-animgraph` 53 passed, 0 failed; `cargo build --release -p fn-app` finished OK (existing warnings only).
Files changed: crates/fn-animgraph/src/graph.rs, crates/fn-animgraph/LAYERED.md.
