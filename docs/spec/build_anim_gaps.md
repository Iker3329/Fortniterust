# Build-mode animation / blueprint: open gaps (spec for a fixer)

Written by the independent verifier of the build-anim port (`crates/fn-pose/src/loadout_build.rs`, `crates/fn-app/src/build_anim.rs`), 2026-10-05.
Every value below was read from the 1.8 cooked data (fn-inspect `props`) or the 1.8 exe decompile (RVAs of `FortniteClient-Win64-Shipping.exe`), or taken
from UE 4.16 source. Nothing here was guessed, except where a line says **UNCONFIRMED**.

---

## Gap 1: paper spring-bone wobble (`Blueprint_Paper_VIM`)

The paper wobbles through **4 `AnimNode_SpringBone` nodes**. It does not use AnimDynamics or RigidBody.

`SK_Blueprint_Paper.WeaponMesh0.AnimClass` = `/Game/Weapons/FORT_BuildingTools/Meshes/Blueprint_Paper_VIM.Blueprint_Paper_VIM_C`. This is the same for both
tools. The AnimBP has 7 nodes. LinkID resolves as `AnimNodeProperties[N-1-LinkID]`, with nodes in declaration order: MeshRefPose, Root, C2L, SB_D9C2, SB_6213,
SB_88B6, SB_9858. The resolved chain is:

```
AnimNode_MeshSpaceRefPose
  -> SpringBone "Spring_1"   (AnimGraphNode_SpringBone_9858..)
  -> SpringBone "Spring_2"   (.._88B6..)
  -> SpringBone "Spring_3"   (.._6213..)
  -> SpringBone "Spring_4"   (.._D9C2..)
  -> ConvertComponentToLocalSpace -> Root
```

The input is the **mesh reference pose**: the paper has no animation of its own, only the springs.

All four nodes have identical values:

| property | value |
|---|---|
| Alpha | **0.5** (AlphaScaleBias Scale 1, Bias 0; the SkeletalControlBase blends input to output by 0.5) |
| SpringStiffness | 100.0 |
| SpringDamping | 10.0 |
| bLimitDisplacement / MaxDisplacement | true / 40.0 cm |
| ErrorResetThresh | 256.0 cm |
| bTranslateX/Y/Z | true / true / true |
| bRotateX/Y/Z | false / false / false (the bone rotation is unchanged) |
| LODThreshold | -1 |

The algorithm is UE 4.16 `AnimNode_SpringBone.cpp`, which you should port verbatim:
- Simulate in world space at a fixed 120 Hz step: `FixedTimeStep = 1/120 * TimeDilation`, and accumulate `RemainingTime += dt`.
- `TargetPos` is the bone's world position (component-space ref pose times the paper mesh's world transform, which follows the hand socket).
- On the first evaluation (`RemainingTime == 0`): `BoneLocation = TargetPos` and velocity is 0.
- Each step:
  - Reset if the error is larger than 256.
  - `a = 100*(Target-Loc) - 10*Vel`.
  - The damping safety scale only applies when `Damping > 1/step`, which is never true here (10 < 120).
  - Integrate: `Vel += a*step`.
  - Clamp `|Vel|*step <= 256`, then `Loc += Vel*step`.
  - Project `Loc` onto the 40 cm sphere around the target.
  - Recompute `Vel = (Loc-Old)/step`.
- Output: the bone location is `BoneLocation` back in component space, and the rotation is unchanged.
- `OwnerVelocity` is **0** here. `PreUpdate` only uses the owner velocity when the skel component has no owner, but the weapon actor owns the paper mesh.
- Each spring node feeds the next (Spring_1 is evaluated first), and each applies `Alpha` = 0.5.

Bone names come from `SK_Blueprint_Paper_Skeleton`: `Spring_1`..`Spring_4`. The renderer currently draws the paper as a rigid ref-pose mesh
(`build_anim.rs::mesh_prims`, a single joint). The fixer has to skin `SK_Blueprint_Paper` to its own skeleton and run the four springs every frame.

## Gap 2: paper glow and pulse (emissive)

**The material chain:**
- Both `SK_Blueprint_Paper` and `Pencil` import `/Game/Weapons/FORT_BuildingTools/Materials/ArchitectTools` (a MaterialInstanceConstant) as their only material.
- **ArchitectTools (MIC):** parent `/Game/Packages/Fortnite_UniqueMaterials_1/Character/M_BluePrintArchitect`.
  - Scalars: `EmissiveMultiply` = **1.0**, `SpecMultiply` = 2.0, `SpecPower` = 0.6.
  - Textures: `Diffuse` = `T_ArchitectTools_D`, `EmissiveMask` = `T_ArchitectTools_E`, `Normal` = `T_ArchitectTools_N`.
- **M_BluePrintArchitect (Material):** TwoSided, opaque.
  - BaseColor = `Diffuse`.rgb; Normal = `Normal`.
  - Specular / Metallic / Roughness = the R / G / B channels of the non-parameter sample `T_ArchitectTools_S` (SAMPLERTYPE_Masks).
  - EmissiveColor = `MaterialExpressionMultiply_1`. That expression is not in the cooked exports (editor-only). Its inputs are most likely
    `EmissiveMask` x `EmissiveMultiply`, because those are the only remaining parameters. This is **UNCONFIRMED**: derive it from the compiled
    shader in the material's shader map (`nativeBytes` 251729) with the fn-dxbc tools before you implement it.
- The port draws `MaterialKind::Prop` with Diffuse and Normal only, so the emissive term and the S-mask spec are both missing.

**Runtime (exe):**
- `AFortWeap_BuildingTool::PostInitializeComponents` (0x1409359b0):
  - Builds the MID with `UMaterialInstanceDynamic::Create(BlueprintPaperMaterial = ArchitectTools)` and stores it in `BlueprintPaperMID` (+0xB00).
  - Sets it on the WeaponMesh (+0x910) at element 0 through vfunc +0x4F0.
  - When `BlueprintPaperPulseCurve` (+0xAF0) is set:
    - It sets `BlueprintPaperPulseTimeline` (+0xB10) `LengthMode` to 1 (TL_LastKeyFrame) and does **not** loop.
    - It calls `AddInterpFloat(curve, OnBlueprintPaperPulseUpdate)`.
- `OnBlueprintPaperPulseUpdate` (0x14092f190) runs `MID->SetScalarParameterValue("EmissiveMultiply", value)`.
- `AFortWeap_BuildingTool::Tick` (0x14093f670) runs:
  - `AActor::Tick`
  - the resource swap (Diffuse and EmissiveMask = `Blueprint{Diffuse,EmissiveMask}Textures[CurrentResourceType]`, only when that differs from
    `LastResourceType` and is not 4, and only for a local controller)
  - `BlueprintPaperPulseTimeline.TickTimeline(dt)`.
- **When the pulse plays:** `AFortWeap_BuildingTool::PlayWeaponFireFX` (0x140935590) calls `Timeline.PlayFromStart()`
  (`SetPlaybackPosition(0)`, `bReversePlayback=0`, `bPlaying=1`) and then force feedback.
  - `ServerCreateBuildingActor_Implementation` (0x140845d70, right after `FUN_1407f6a70` plays the place montage) calls vfunc +0x648(0) on the pawn's
    current weapon when it is an `AFortWeap_BuildingTool`.
  - That is most likely this PlayWeaponFireFX (vtable slot match **UNCONFIRMED**). Either way, the pulse plays **once per successful placement**.
- **Pulse curve:** `/Game/GameplayCurves/HandheldCharacterBlueprintGlowAnimation` (CurveFloat). All keys are linear (InterpMode 0):
  `(0.00153, 1.0)`, `(0.15, 7.0)`, `(0.30, 1.0)`. The timeline length is 0.30 s (last key).
  - At rest, nothing calls the update, so EmissiveMultiply stays at the MIC value **1.0**: the mask glows steadily at 1x.
  - On each placement it ramps to **7x** at 0.15 s and back to 1x at 0.30 s.
- **The edit tool:** `AFortWeap_EditingTool` is not a `FortWeap_BuildingTool`, so it has no MID, no swap and no pulse. Its paper shows the plain
  ArchitectTools MIC (`T_ArchitectTools_D` / `_E`, EmissiveMultiply 1). The pencil always shows the plain MIC.

## Gap 3: equip sound `Player_Equip_Blueprint_Cue`

- **Notify:** the `Blueprint_Equip` montage (`/Game/Animation/Game/MainPlayer/Montages/Blueprint/Blueprint_Equip`, used by both tools' `EquipAnimation`) has
  one `AnimNotifyEvent`:
  - `AnimNotify_PlaySound_C_0`, `NotifyName` "Player_Equip_Blueprint_Cue".
  - Link time **0.035156 s** (LinkMethod Absolute, segment 0), MontageTickType Queued, TriggerChance 1, TriggerWeightThreshold 0.
  - Notify props: `Sound` = `/Game/Sounds/Fort_GamePlay_Sounds/Player/Player_Equip_Blueprint_Cue`, **`bFollow` = true**, AttachName None,
    and Volume/Pitch multipliers 1 (defaults, not serialized).
- **Cue:**
  - VolumeMultiplier **0.3**.
  - AttenuationSettings `/Game/Sounds/Attenuation/Fort_Prop_Close_Att`.
  - SoundClass `Fort_Player_Foley`.
  - It is a `SoundNodeRandom` over the waves `player_blueprint_takeout_01` / `_02`.
- **Wiring:** `build_anim.rs` already loads `MontageData::from_anim_in`, so `fx` holds the notify, but it never dispatches it.
  - Fire `Fx::Sound` when an instance crosses 0.035 s, the same way `arms.rs` does for gun equips (`ArmsSound::Notify` -> `audio::arms_to_sounds`,
    played at the pawn mesh, attached).
  - `fn_audio::weapon_events::by_cue_path` has no row for this cue yet, so add one (event id such as `build.equip`).
  - It plays on build enter, on edit begin, and on the re-equip of the build tool after an edit (all three are `Blueprint_Equip`).

## Gap 4: firearm reload / fire actions while a blueprint is held

- **Exe facts:**
  - Equipping the building or edit tool makes it the pawn's current weapon, and the previous firearm runs `AFortWeapon::OnUnEquip`.
  - `AFortWeap_BuildingTool` / `_EditingTool` derive from `AFortWeapon`, not `AFortWeaponRanged`. They have no ammo and no reload.
  - Their ctor leaves `bAllowTargeting` (+0x964) = 0 (`AFortWeapon::AFortWeapon` 0x140914b60), and `AFortWeapon::SetTargeting` (0x14093ade0) checks it.
  - So while a blueprint is held there is no reload, no firearm fire and no ADS.
- **Port today** (`app.rs::drive_arms`):
  - `arms.update(.., ArmsInput{ reload_press, .. })` still runs for the selected firearm while building.
  - `inp.attack` is cleared in build mode (app.rs line 802), but `ai.reload_press` is not, so a reload can start and finish invisibly.
  - Entering build mode does not cancel an in-progress reload or the fire timers.
  - `Arms::reequip` cancels the action but does not reset `fire_t / burst / auto_reload / refire / interrupt_retry / queued` the way `Arms::select` does.
- **Fix:**
  - While `build_anim::is_active()`, pass `ArmsInput { reload_press: false, fire_held: false, .. }`.
  - On `Event::Enter` / `EditBegin` from a firearm, run the same teardown as `Arms::select` (`cancel_action` and clear the weapon timers).
  - Keep `reequip` on exit, and reset the same timers there.

## Smaller mismatches found while verifying (not in the original gap list)

1. **The `FN_BUILDANIM` `wood/stone/metal` events are overwritten the next frame.** `build_anim::poll` sets `s.resource` from `BuildInput::material()` every frame,
   so a timeline resource event only shows for its own frame.
   - Screenshots: `verification/building/anim/verify/v00_material_strip_*.png`. Metal shows on the event frame (145) and is back to wood at frame 170.
   - In-game, the build input drives it, so this is dev-only. To fix it, let the timeline set a sticky override, or set the build input's material.
2. **Gun montages are cut instantly when entering build mode.** `drive` calls `sim.arms.clear()` every frame. UE 4.16 `Montage_Play(Blueprint_Equip)` stops
   the DefaultGroup montages with the new montage's BlendIn (0.667 s HermiteCubic) instead of cutting them. Low severity.
3. **No equip montage after leaving build mode with a consumable selected.** `put_away` plays nothing when the Primary bar holds a consumable; the
   consumable's own equip montage should play. Low severity.
4. **Edit-exit montage on the authority path.** In `FUN_140837140` (the body of LocalExitEditMode), `Blueprint_Edit` plays only when
   pawn `Role < ROLE_Authority`, pawn `bIsDBNO` (+0x920) == 0 and the edit was not cancelled. 1.8 BR clients are autonomous proxies, so
   the client-view port is right to play it on confirm. Add the `!bIsDBNO` condition if DBNO is ported. Info only.
