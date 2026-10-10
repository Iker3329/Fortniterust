# Building visuals: gaps and mismatches (fixer spec)

Verifier pass on 2026-10-05 over `crates/fn-building/src/visual.rs`, `crates/fn-app/src/building_render.rs`, the fn-audio-sink building events, and the
`app.rs` hooks. All addresses are in the 1.8 exe (`C:\FortniteRef\decomp\named`, image base 0x140000000). Float constants were read from the exe file.
Evidence screenshots are in `verification/building/visuals/verify/`:
- `v_*` come from the `FNRT_BUILDING_DEMO` run.
- `p_*` come from the `FNRT_BUILD_SCRIPT` run.
- `l_*` come from a live run driven by real keyboard and mouse input.

Field names come from the 1.8 SDK dump (`ABuildingSMActor`, `ABuildingActor`, `UFortGameData`).

## Priority order
1. **Edit-mode visuals are never drawn (high).** See section A.
2. **The surface flips to the damaged material during construction (medium-high).** See section B.
3. **The construction loop does not fade out when construction ends (medium).** See section C.
4. **The per-frame MID drive is missing (medium):** hit bounce, finished-highlight flash, blueprint `Code_Emissive`, the damage overlay, and the destruction animation. See section D.
5. **Death FX are placed at the wrong location (low-medium).** See section E.
6. **Particle gaps:** sparkle triangle emitters, and lit debris materials. See sections F and G.
7. **Lighting and shadows of pieces.** See section H.

---

## A. Edit tiles, stair indicators and the edit preview fail shader validation (HIGH)
**Observed in game.** Pressing G on a placed wall (`l_20` to `l_24`) puts the player in the edit pose, but no tiles and no `MPreviewPulse` preview appear. The run log (`verify/run_live.log`) shows these errors:
```
[building] edit tile material NOT DRAWN: fx MGreen FLocalVertexFactory: Validation Error
  ... fs_main: let n_ue = normalize(o[1].xyz * 2.0 - 1.0);  Accessing index 1 is out of [21] bounds
[building] edit preview material NOT DRAWN: fx MPreviewPulse FLocalVertexFactory: Validation Error
```
**Cause.** `fn_fx::gpu::build_material_layout` sets `lit = mat.flags.shading != "MSM_Unlit"` (gpu.rs:289).

`MGreen` and `MPreviewPulse` (`/Game/Packages/Fortnite_UniqueMaterials_1/Effects/`) are `BLEND_Translucent` and use the default DefaultLit shading. Their base-pass pixel shader writes only `SV_Target0`, not the deferred GBuffer. `fn_fx::program::assemble` (program.rs:~208) still appends the GBuffer lighting tail, which reads `o[1]` and `o[3]`.

`M_Blueprint_StairsBaseShader` (the stair indicator) has `MGreen` as its parent, so it fails in the same way.

**Fix.**
- Set `lit` only when the blend mode is Opaque or Masked: `lit = shading != Unlit && blend in {Opaque, Masked}`. A translucent pixel shader's `o[0]` is already its final colour.
- UE 4.16 forward-lit translucency would also sample the translucency lighting volume. The FX pass has no such volume, so name that as a limit.
- Then re-verify edit mode in game: tile colours, `isX`, `InteractParameter`, `BlueColor`, the stair indicators, and the pulse.

## B. DamageVisualsState is wrong during construction (MEDIUM-HIGH)
**Exe.** `vfunc_320` @1404989e0 chooses one of three states:
- state 1 (AnimatingMaterial) when `vfunc_272` @140491e30 returns true;
- otherwise state 2 (DamagedButNotAnimatingMaterial) when health < max;
- otherwise state 0.

`vfunc_272` returns true when any of these holds:
- `vfunc_211` @14047ed90 is true. That is: an animation is running, or the time since the animation start is within `vfunc_0xc58()`, or `vfunc_0xa20`.
- `vfunc_0xac0` @14047fe50 is true. It reads `(*(u32*)(this+0xE90) >> 6) & 1`: the under-construction flag, set while `HealthToAutoBuild` (+0xC70) > 0 by `vfunc_389` @14048d2d0.
- `vfunc_0x668` @14047eab0 is true (`*(this+0xCA0) != 0`).

So the piece keeps its AnimatingMaterial MIDs for the whole construction period, while auto-build health is still being added. The damaged material only appears once construction is over and health is below max.

**Port.** `PieceAnim::update_surface` returns `DamagedStatic` whenever `kind == None && health < max`. Between build steps (target reached, health still rising) the piece swaps to `*_Damaged`. The live log shows `Metal_lvl1_Wall_Damaged` being loaded while the metal wall was still building.

**Fix.** Pass "under construction" (the core's remaining auto-build health > 0) into `tick`. Treat it as animating for the surface. Also count the hit and highlight windows in section D, as `vfunc_211` and `vfunc_324` @14047f9c0 do:
- a hit is animating while `now - LastHitTime(+0x78C) < 5.0` (`DAT_143fe3710` = 5.0);
- the highlight is animating while `now - HighlightStart(+0xF88) < duration(+0xF8C)`.

## C. The construction sound fades out when construction ends (MEDIUM)
**Exe.** In `ABuildingSMActor::Tick` @1404967f0, when the under-construction bit (0xE90 & 0x40) is clear and `ConstructionAudioComponent` (+0x970) is playing, the tick calls `FadeOut(0.5, 0.0)` (vtable +0x4E0, `DAT_143feb1d8` = 0.5). `EndPlay` @140469ac0 also stops the component at removal.

**Port.** It stops the sound only at removal (`BuildingLoopStop`). The loop keeps playing after the piece is finished.

**Fix.**
- Add `SoundEvent::BuildingLoopFade { key, fade: 0.5 }`, or reuse the `WeaponLoopStop` fade path.
- Emit it when the core's auto-build health reaches 0 (the moment `vfunc_389(0)` would clear the bit), not when the animation completes.

## D. The per-frame MID drive is missing (`vfunc_323` @140499690 and the triggers that feed it) (MEDIUM)
Parameter FNames were read from the exe's static initialiser (1400996fd to 140099b4d):

| Address | Name |
|---|---|
| f4f0 | `Code_HitAnimation` |
| f4f8 | `Code_DestructionAmount` |
| f4e8 | `Code_CustomDamagePercentage` |
| f4e0 | `InvertDamageFalloff` |
| f500 | `WSDeformationCenter` |
| f508 | `Code_DeformationVector` |
| f510 | `WSDamageRadius` |
| f4c8 | `Emissive_Color_Add` |
| f488 | `Code_Emissive` |
| f4d0 | `CustomExplosionVectorForExplosion` |
| f4b8 | `outwardMotion(WS)` |
| f480 | `code_BreakAnimation` |

Game-data assets come from the `UFortGameData` ctor string table @140573b50. The mapping from asset to field is inferred from the constructor's order and the SDK field order:

| Field | Offset | Asset |
|---|---|---|
| `BounceCurve` | +0x1A80 | `/Game/Packages/Fortress_SharedMaterials/Base_Material_Functions/HitEffectBounceAnimation` |
| `BlueprintDamageCurve` (UCurveVector) | +0x1AE0 | `/Game/GameplayCurves/RedImpactCurve` |
| `FullHealthAnimCurve` | +0x1B00 | `/Game/Building/CurveEffects/FinishedHighlight` (keys 0 to 1 s, peak 1.999 at t = 0.3218) |
| `DamageMaterial` | +0x1A60 | `/Game/Packages/jl_tests/Translucent_hitEffects` |
| `BuildingFullHealthFlashColor` | +0x1B8 | (1.67, 0.36, 0.06, 1.0), from `/Game/Balance/DefaultGameData` |

`vfunc_325` @14046c010 returns `EditCurve` (+0x1AC0) instead of `BounceCurve` while editing.

### D1. Hit bounce (`vfunc_313` @140493170, called on a damaging hit and by `vfunc_393` @1404859a0 on completion)
1. Store the hit time in +0x78C, the radius in +0x794, the location and deformation vectors in +0x798/+0x7A8, and the mode in +0x7B8. If +0x998 (`DamageAmountStart`) is 0, store the current health there.
2. Create the MIDs (`vfunc_318`).
3. On every animating MID, the overlay MID (+0x990) and the blueprint MIDs, set:
   - `WSDeformationCenter` = hit location;
   - `Code_DeformationVector` = direction;
   - `Code_HitAnimation` = 0;
   - `WSDamageRadius` = 1 / radius (0 if radius <= 0).

   On the animating MIDs only, also set `InvertDamageFalloff` = 1 if mode == 2, else 0.

Completion bounce (`vfunc_393`): location = the bounds origin's XY with the root Z, direction = (0, 0, 1), radius = `Bounds.SphereRadius * 5.0`, mode 2. `ABuildingRoof` overrides this as `vfunc_393` @140485900.

### D2. Tick (`vfunc_323`)
Let `tb = clamp(now - HitTime, curve range)`. Each step below writes to the MID sets named in its own line.
- **Hit animation.** `Code_HitAnimation = BounceCurve(tb)`. Write it to the animating MIDs, the overlay and the blueprint MIDs. Also call the BP event `OnBounceAnimationUpdate`.
- **Bounce fraction.** `f = tb / curve length`.
- **Damage fraction.** `dmg = clamp((DamageAmountStart + (health - DamageAmountStart) * f) / max)`. Write `Code_CustomDamagePercentage = 1 - dmg` to the animating MIDs.
- **Overlay damage.** `Code_DestructionAmount = 1 - clamp((((health + DamageAmountStart) * 0.5) + (health - (health + DamageAmountStart) * 0.5) * f) / max)`. Write it to the overlay MID.
- **Finished highlight.** Let `th = now - HighlightStart(+0xF88)`, clamped to `[0, duration(+0xF8C)]`. Write `Emissive_Color_Add = BuildingFullHealthFlashColor * FullHealthAnimCurve(th)` to the animating MIDs.
- **Constructed effect.** When the flag at +0xF90 is set and the curve value this frame is below its value at `th - dt`, clear the flag and call `vfunc_317`, which spawns the ConstructedEffect. The sparkle therefore spawns at the curve's peak, about 0.32 s after completion, not at completion as the port does now.
- **Blueprint emissive.** While blueprint MIDs exist: `Code_Emissive = (2 - dmg) * BlueprintDamageCurve(t / len).xyz`, where `t` is measured from +0xF94. Write it to each blueprint MID through `FUN_14129c290` (vector to linear colour), together with `Code_HitAnimation`.
- **Highlight start** (`vfunc_391` @1404937c0, at completion). Create the MIDs, set `HighlightStart = now` and `flag = 1`, and reset `Emissive_Color_Add` to (0, 0, 0, 1).

### D3. Damage overlay (`vfunc_322` @14048ef60)
- **Condition.** It runs when `MIDDamageOverlay` is null and `vfunc_0xa08` is true. The latter probably reads `bNeedsDamageOverlay` (+0x95C).
- **Component.** A second `UStaticMeshComponent` with the same mesh, with shadows disabled. Every material slot gets one MID of `DamageMaterial` (`Translucent_hitEffects`).
- **Parameters.** It receives `Code_HitAnimation` and `Code_DestructionAmount` from D2. Check each class's `bNeedsDamageOverlay` before porting.

### D4. Destruction animation (`EBA_Destruction`, `vfunc_402` @14047c840)
- **Scale.** `s = clamp(1 + (LastDamageAmount(+0x99C) / max) - GetBuildingDmgPctForCritDestructionAnim(level, res), 1, GetBuildingCritDmgAnimScaleCap(level, res))`.
- **Direction.** The normalised hit direction: `LastDamageHit` (+0x9A0) TraceEnd minus TraceStart.
- **On each MID.** Set `Code_totalDestructionAmount` = 0 and `Code_FloorBeingDamaged` = 1. Read `outwardMotion(WS)` and write it back multiplied by `s`. If `s > 1`, set `CustomExplosionVectorForExplosion` = direction.
- **Blueprint.** Show it, and set `code_BreakAnimation` = 0 on the blueprint MIDs.
- **Duration.** It comes from the curve rows `BuildingDestroyAnimDuration_{Wood,Stone,Metal}` = 1.5 s for every level (`/Game/Balance/DataTables/DefaultGameData`).
- **Port.** The world removes a piece at 0 health, so this never runs. Porting it needs a "dying" state in the core before removal.

## E. Death FX and death sound location (LOW-MEDIUM)
**Exe.** In `vfunc_316` @140485ab0, the location is `vfunc_0x7F0` = `ABuildingActor::GetCentroid` @14046ced0. That is `actor transform * CentroidOffset` (+0x6EC). If `DeathParticleSocketName` exists on the mesh, the location and rotation come from the socket instead (`FUN_1420d92e0`).

`CentroidOffset` is a class-ctor default. No player CDO overrides it.

| Class | Ctor | CentroidOffset |
|---|---|---|
| `ABuildingWall` | @14049fe20 | (0, 0, 192) |
| `ABuildingFloor` | @140459d10 | (0, 256, 0) |
| `ABuildingStairs` | @14049f370 | (0, 256, 192) |
| `ABuildingRoof` | @14045b660 | (0, 256, 0) |

The values come from `DAT_14401ee38` = 192 and `DAT_143ff903c` = 256. Check the X/Y order against the decompile.

The particle component's scale is then set from the root component (vtable +0x3A0).

**Port.** It uses the actor origin for all 296 classes that have no socket.

**Fix.** Use the centroid. `PBWA_W1_Solid` is the only class with `DeathFX`, at (0, 0, 194) with zero rotation, so it is already correct.

## F. Sparkle systems using `ParticleModuleLocationPrimitiveTriangle` (LOW)
The exporter marks this module `Unsupported` in `FortniteRuntime/assets/particles-building/systems/`:

| System | Emitters | Height | Angle | Thickness |
|---|---|---|---|---|
| `P_PBW_W_Sparkle_Brace` | Burst, Fall | 340 | 90 | 0 |
| `P_PBW_W_Sparkle_RoofI` | — | 400 | 90 | 1 |
| `P_PBW_W_Sparkle_RoofO` | — | 400 | 90 | 0 |
| `P_PBW_W_Sparkle_RoofWall` | — | 350 | 90 | 0 |

`StartOffset` is per emitter. Read it from `fn-inspect props /Game/Effects/Fort_Effects/PBW/Build_Sparkle/P_PBW_W_Sparkle_<X>`.

Algorithm: UE 4.16 `UParticleModuleLocationPrimitiveTriangle::SpawnEx`, in `Engine/Source/Runtime/Engine/Private/Particles/ParticleModules_Location.cpp:851`.
- Corners are `Offset + (+H/2, 0, 0)`, `Offset + (-H/2, +B, 0)` and `Offset + (-H/2, -B, 0)`, with `B = H * tan(Angle/2 deg)`.
- Three random fractions, normalised by their sum (minimum `KINDA_SMALL_NUMBER`), are the barycentric weights.
- `Z = rand * Thickness - Thickness/2`.
- The offset is transformed by `EmitterToSimulation` and added to the particle location.
- All four values are evaluated from the distributions at `EmitterTime`.

Implement the module in fn-particles and re-export.

## G. Lit particle materials in destruction effects (LOW)
These emitters are not drawn because the FX sprite and mesh pass has no lit path (logged as `shading model MSM_DefaultLit is not ported`):
- `P_Destruction_Wall_Solid_stone`, unnamed emitter: `/Game/Effects/Fort_Effects/Materials/Rocks/M_Chunks_Base`.
- `P_Destruction_Wall_Solid_stone/ChipsLight`, mesh emitter on `SM_PropRocks_part4`: `/Game/Effects/Fort_Effects/World/Destruction/Rocks/M_PropRocks_shards_01`.
- `P_Destruction_Wall_Solid_wood/ChunksLazy`: also `M_Chunks_Base`. Not seen in a run (no wood piece was destroyed), but it is the same material.
- The metal emitters use `M_SparkRound_SpeedLerp`, `M_HitImpactDefault` and `M_Smoke_Destruction_01_Inst`. Check their shading before calling metal complete.

**Fix.** Particle-sprite and mesh vertex factories with the opaque or masked DefaultLit base pass need the same GBuffer-to-sun-and-sky tail as `fn_fx::program::assemble` uses for meshes (`TBasePassPSFNoLightMapPolicy` on `FParticleSpriteVertexFactory` / `FMeshParticleVertexFactory`). This is the same blend gate as in section A, inverted.

## H. Pieces lighting and shadows (LOW, already named in building_render.rs)
- **Lighting.** Pieces are lit by the FX pass's simple term in `fn_fx::program::assemble`: GBuffer normal and base colour, times sun N·L plus sky SH. Specular, roughness, reflections, AO and the deferred light accumulation are not run.
- **Shadows.** In the build the verifier tested, pieces cast no shadows: every `v_*`, `p_*` and `l_*` shot shows walls with no ground shadow, while the character casts one. Since then, `FX SHADOW` (building_render.rs, 2026-10-05) has landed: placed pieces now cast into the CSM cascades with their own cooked `TShadowDepthVS/PS`, and previews and edit actors do not cast, matching the exe. A fixer is adding shadow receiving. Neither change was verified in game in this pass.
- **Still to do.** Re-run the demo after the receiving fix, and check:
  - the build-up's WPO in the shadow (sub-objects flying in);
  - that the blueprint ghost casts no shadow (`vfunc_387` clears the flags at +0x2A9/+0x2AA);
  - that the damage overlay casts no shadow (`vfunc_322` clears the same flags);
  - that the pieces receive the sun shadow.
- **Fix, lighting.** Route the opaque and masked piece draws through the main lit path instead of the FX pass.

## I. Smaller notes
- **Blueprint ghost slots.** The exe's blueprint component (`vfunc_387` @14048ec40) is a full mesh. Only slot 0 gets the blueprint MID; the other slots keep the mesh's own materials. The port draws slot 0 only. This matters only for meshes with more than one slot.
- **Log text.** The `building_render.rs` log line `(shader map of X)` prints `Material::base_path`, which is the root material. It does not name the package whose programs were used, which is misleading. The instances do carry their own maps (about 1 MB `.uexp` each), and `load_material` picks them correctly.
- **Wrong path in a doc comment.** `visual.rs` `CODE_RED_DEFAULT` cites `/Game/Packages/Fortnite_UniqueMaterials_1/Walls/PlayerBuiltWall_BluePrintShader`. The asset actually used is `/Game/Packages/PBW/Blueprint/PlayerBuiltWall_BluePrintShader`, a MIC whose parent is that path.
- **Level source.** The level is `ABuildingActor::CurrentBuildingLevel` (+0x498), read by vtable +0x610, which is `mov eax,[rcx+0x498]` @14046d760. For all 297 tiered classes, `CurrentBuildingLevel + 1` equals the `L<n>` folder: it is absent (0) in L1 and set to 1 and 2 in L2 and L3. Read the CDO field instead of the folder.
- **Death-particle fallback.** `vfunc_326` @14046efc0 returns `FallbackDeathParticles` (+0x19C0); `ABuildingWall::vfunc_326` @1404afa10 returns `WallFallbackDeathParticles` (+0x19E0). The port's Wall/other split is correct.
- **Health-change re-dispatch.** `vfunc_210` @140482370 calls `vfunc_398` when the piece may animate and is not animating. It is reached from `ABuildingActor::vfunc_239` @140479820. The port's idle re-dispatch is equivalent.
