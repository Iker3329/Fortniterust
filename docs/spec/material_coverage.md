# Material coverage: static props and terrain (matstat agent, 2026-10-05)

Status: **props ported, landscape not ported, work paused partway** (the coordinator asked everyone to wrap up). See "Half-done" and "Remaining plan" below.

## How the audit was made

`cargo run --release -p fn-fx --example material_coverage [props dir] [radius m]` (crates/fn-fx/examples/material_coverage.rs) builds the same layout
selection as `fn-app props::Library::from_catalog`: every placement in `layout/all.json` whose kind is in `tree/rock/bush/hedge`, within 150 m (XY) of
`layout/sample.json`'s centre. For each slot material it calls `fn_fx::material::Material::load` (instance chain + child-most static-permutation shader
map), checks the `FLocalVertexFactory` programs, and runs `fn_fx::program::assemble_scene_lit` (the actual DXBC->WGSL translation). `--sig <material>`
prints a material's base-pass output signature.

## Static props in the current layout

**309 placements, 20 kinds, 21 unique slot materials.** Every one of the 21 has a cooked shader map with `TBasePassVSFNoLightMapPolicy`,
`TBasePassPSFNoLightMapPolicy` (+`Skylight`), `FDepthOnlyPS` and `TShadowDepthVS` for FLocalVertexFactory, and every one assembles scene-lit.

| kind | placements | mesh | slots (material, source) |
|---|---|---|---|
| Athena_Prop_Rocks_StoneLarge01 | 2 | Athena_StoneLarge_01 | 0: MI_StoneLarge_01 (mesh) |
| Athena_Prop_Rocks_StoneLarge02 | 1 | Athena_StoneLarge_02 | 0: MI_StoneLarge_01 (mesh) |
| Athena_Tree_Large_1 | 57 | Athena_Tree_Large_01 | 0: MI_Tree_Large_01_Ambient; 1: MI_Tree_Large_01_Ambient_Leaves (mesh) |
| Athena_Tree_Medium_01_ | 13 | Athena_Tree_Medium_01 | 0: MI_Tree_Medium_01_Ambient; 1: MI_Tree_Medium_01_Ambient_Leaves (mesh) |
| Athena_Tree_Medium_03_ | 7 | Athena_Tree_Medium_03 | 0: M_Tree_Medium (component_override); 1: MI_Tree_Medium_03_Ambient_Leaves (mesh) |
| Athena_Tree_Medium_04_ | 12 | Athena_Tree_Medium_04 | 0: MI_Tree_Medium_04_Ambient; 1: MI_Tree_Medium_04_Ambient_Leaves (mesh) |
| Athena_Tree_Pine_01 | 32 | Athena_Tree_Pine_01_ | 0: M_Tree_Pine (component_override); 1: MI_Pine_01_Ambient_Leaves (mesh) |
| Athena_Tree_Pine_05 | 10 | Athena_Tree_Pine_6 | 0: MI_Pine_05_Ambient_Inst; 1: MI_Pine_05_Ambient_Leaves_Inst (mesh) |
| Athena_Tree_Pine_3 | 29 | Athena_Tree_Pine_02 | 0: M_Tree_Pine (component_override); 1: MI_Pine_02_Ambient_Leaves (mesh) |
| Plant_Shrub_01 | 26 | Shrub_20 | 0: MI_Shrub_20_Ambient |
| Plant_Shrub_06 | 28 | SM_Fort_Foliage_Shrubs_Shrub02a | 0: MI_Shrub_02_Ambient |
| Plant_Shrub_08 | 10 | Shrub_04 | 0: MI_Shrub_04_Ambient |
| Plant_Shrub_09 | 36 | Shrub_05 | 0: MI_Shrub_05_Ambient |
| Plant_Shrub_bush_sm_01 | 4 | shrub_bush_sm_01 | 0: MI_Bush_Solid_2_Ambient; 1: MI_Bush_Solid_2_Ambient_Leaves |
| Prop_Rocks_02 / _03 / Piles_1 / _3 / _4 / _5 | 10 / 1 / 5 / 7 / 6 / 13 | SM_Props_Rock02 / 03, SM_Props_RockPile01 / 03 / 04 / 05 | 0: MI_Props_Rocks |

| material | kinds / placements | root | shading | blend | 2-sided | shader map package | targets |
|---|---|---|---|---|---|---|---|
| MI_Props_Rocks | 6 / 42 | Fortress_BaseMaterial | DefaultLit | Opaque | no | Fortress_BaseMaterial_WithNormal | 5 |
| MI_StoneLarge_01 | 2 / 3 | Fortress_BaseMaterial | DefaultLit | Opaque | no | Fortress_BaseMaterial_WithNormal | 5 |
| MI_Shrub_02/04/05/20_Ambient | 1 each / 28, 10, 36, 26 | FortniteFoliage_MasterMaterial | Subsurface | Masked 0.333 | yes | MI_Shrub_xx_Storm | 6 |
| M_Tree_Medium | 1 / 7 | DEPRECATED_AnimatedTreeMaterial | Subsurface | Masked 0.5 | yes | M_Tree_Medium | 6 |
| M_Tree_Pine | 2 / 61 | DEPRECATED_AnimatedTreeMaterial | TwoSidedFoliage | Masked 0.5 | yes | M_Tree_Pine | 6 |
| MI_Bush_Solid_2_Ambient(_Leaves) | 1 / 4 | FortniteFoliage_MasterMaterial | Subsurface | Masked | yes | own | 6 |
| MI_Pine_01/02_Ambient_Leaves, MI_Pine_05_Ambient(_Leaves)_Inst | 1 each | FortniteFoliage_MasterMaterial | Subsurface | Masked | yes | MI_Pine_02_Ambient(_Leaves) | 6 |
| MI_Tree_Large_01_Ambient(_Leaves) | 1 / 57 | FortniteFoliage_MasterMaterial | Subsurface | Masked | yes | own | 6 |
| MI_Tree_Medium_01/04_Ambient, *_Leaves | 1 each | FortniteFoliage_MasterMaterial | Subsurface | Masked | yes | MI_Tree_Medium_02_Ambient, Tree_Medium_02_B_Calm / _L_Calm | 6 |

Targets: SV_Target0..3 = scene colour / GBufferA..C, SV_Target4 = velocity, SV_Target5 = GBufferD (custom data, subsurface models only).

**Before this work:** all 20 kinds were drawn by `fn_render::static_loader` with estimated values (texture-mean spec, roughness 0.5, invented wind and
backlight). The large oak's leaves came out grey-white (verification/materials/statics/before_h13).
**Now:** all 20 kinds / 21 materials run their real cooked shaders (`crates/fn-app/src/statics_render.rs`, see below).

Not in scope here: the ground-cover grass (`foliage.rs`, owned by the foliage agent).

## Terrain / backdrop (not ported)

| item | where it renders now | cooked shaders |
|---|---|---|
| gameplay baseplate ground | scene.wgsl kind 0 (ASSUMED landscape spec mapping) | n/a (it is not a UE mesh) |
| landscape `M_Athena_Terrain_Landscape_Inst` (root M_Athena_Terrain_Landscape, DefaultLit, Masked) | backdrop.wgsl reconstruction | the material package has 240 programs, **FLocalVertexFactory only**. The `FLandscapeVertexFactory` / `FLandscapeXYOffsetVertexFactory` base-pass VS/PS, FDepthOnlyPS and shadow-depth programs are inside the level package `Athena_Terrain.umap`: 2 LandscapeComponents, 3 LandscapeMaterialInstanceConstants, 486 programs (3x each base-pass type). The streamed sublevels hold the other components; the tool's scan of `Athena_Terrain_LS_00..05` used guessed paths that are not in the paks. It has been patched to follow the `LevelStreaming` `WorldAsset` paths, but that version has not been run. |
| skybox terraces `M_Skybox_Temperate_Terraces_01_moretiles(_Inst)` | backdrop.wgsl | FLocalVertexFactory VS/PS present, assembles OK (5 targets); no FDepthOnlyPS (opaque) |
| ocean `M_Athena_Water_Ocean` | backdrop.wgsl | FLocalVertexFactory VS/PS present, assembles OK; no shadow VS |

Porting the landscape needs an emulation of FLandscapeVertexFactory: packed vertex XY / subsection bytes, heightmap sampling in the VS,
`LandscapeParameters` and LOD globals, and per-component weightmaps and MICs. It also has to be drawn inside the backdrop pass, which uses its own projection
and depth instead of the scene pass. This is open work.

## What statics_render.rs does

- Pass: `TBasePassVSFNoLightMapPolicy` + `TBasePassPSFNoLightMapPolicy`, drawn scene-lit. The GBuffer it writes goes to fx_resolve / fx_resolve_sm (the
  foliage agent's shading-model terms handle Subsurface and TwoSidedFoliage).
- LODs: all LODs are loaded, and the LOD is picked with `ComputeStaticMeshLOD` (SceneManagement.cpp:406) from the cooked `ScreenSize[]`.
- Per-placement Primitive buffer (stable slot). Frustum culling, plus `CachedMaxDrawDistance`.
- Mirrored placements use reversed culling.
- Shadows: each material's own TShadowDepthVS/PS goes into the cascades, and the cooked distance field goes into the DF scene (df_hook rule).
- Velocity: the component is Static (ctor @14045b960 sets +0x13e = 0), so velocity is written only when the shader map's
  `FMaterialCompilationOutput::bModifiesMeshPosition` is set. That flag is parsed by the new `fn_fx::uniform::compilation_flags`.
- `scene.rs sync_props` removes the handled kinds from the old static draw. `FNRT_STATIC_MATERIALS=0` turns the new path off (this is the "before" state).

## Evidence so far

`verification/materials/statics/` (`run.sh <before|after> <hour>`, `statics.script`: same spawn camera, yaw 0 / 120 / 240):
- `before_h13/`, `after_h13/`: done. Report: 308 placements, 58 drawn at yaw 0 (LOD 13/24/21/0), 96 draws, 472 shadow draws.
- Evening (20.5) before/after: **not taken yet.**

## Half-done / known problems

1. **GPU cost (FIXED 2026-10-06, the pre-pass below is in: fn-fx program.rs `fs_depth` / `fs_main_eq` + `@invariant` position, fx.rs depth twins with the
   colour pass at CF_Equal, renderer.rs prepass node; verification/grass: scene pass 10.0 -> 6.05 ms at 12:30 with the real grass added).** The scene pass went
   from 5.0 to 15.8 ms (frame 18.8 to 30.2 ms) at 13h yaw0. The cause is masked foliage overdraw: FX scene-lit draws have no depth
   pre-pass, so every leaf layer runs the full cooked PS. UE4.16 does have one (cooked r.EarlyZPass=2, r.EarlyZPassOnlyMaterialMasking=True: FDepthOnlyPS clips
   in the pre-pass, then the base pass runs without clip under an equal depth test). The planned fix:
   - (a) program.rs scene tail emits an `fs_depth` entry (clip only);
   - (b) fx.rs `create_scene_lit_pipeline` builds a depth-only twin, and the colour pipeline switches to LessEqual with no depth write;
   - (c) a new `FxRenderer::draw_scene_lit_depth`, called with one line in renderer.rs's `prepass` node (the perf agent's file).
   Nothing for this has been edited yet.
2. The material `bModifiesMeshPosition` change, which replaced the earlier wrong "WPO false" detection, compiles but has not been run yet.
3. First in-game frame stalls about 5.9 s while the 21 materials are built (the same pattern as df_hook). Moving this into the loading phase or parallelising
   `Material::load` would remove it.
4. Named LIMITS: no prop hit shake (the old sway was invented, and the 1.8 BuildingProp damage feedback is not ported), no dithered LOD transition, no
   per-cascade caster culling.
5. The fn-render / fn-fx / fn-app tests were not run in this session.

## Remaining plan

Depth pre-pass (1) → rerun 13h + 20.5 before/after captures → run the `statics_render` tests (`FN_TEST_ROOT=... cargo test -p fn-app statics_render`) and the
fn-render / fn-fx suites → move the material build off the first frame → landscape (FLandscapeVertexFactory emulation in the backdrop pass), then the
skybox and ocean through FLocalVertexFactory.

---

## Foliage + character materials (foliage/character materials agent, 2026-10-05)

Source: `fxmataudit --md <paths>` (crates/fn-fx/src/bin/fxmataudit.rs) over every material slot of the placed tree / shrub / hedge / bush kinds (props export
`chain[0]`), the ground-cover grass, the default hero's sections (runtime log of `fn_pose::br_character`) and the starter weapons' mesh slots (`fxmataudit --wid`).
Flags = `fn_fx::material::Material::load` (bOverride-gated instance overrides). "VS material code" = the cooked base-pass VS binds `Material` /
`MaterialCollectionN` (vertex-stage material expressions, i.e. WPO / wind; the material inputs themselves are editor-only and stripped by the cooker).
oN = SV_Target registers the base-pass PS writes (o4 velocity, o5 GBufferD/CustomData under the cooked r.BasePassOutputsVelocity / r.SelectiveBasePassOutputs).
Every foliage material has a cooked SM5 shader map with base pass, FDepthOnlyPS and shadow-depth VS+PS for FLocalVertexFactory and FInstancedStaticMeshVertexFactory.

### Foliage

| Material (slot) | Root | Shader-map owner | Shading | Blend | Sides | WPO | MPC import | Programs |
|---|---|---|---|---|---|---|---|---|
| `MI_HeroTree_02` | `DEPRECATED_AnimatedTreeMaterial` | `MI_HeroTree_02` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Large_01_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Tree_Large_01_Ambient` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Large_01_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Tree_Large_01_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Medium_01_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Tree_Medium_02_Ambient` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Medium_01_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `Tree_Medium_02_L_Calm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Medium_03_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `Tree_Medium_02_L_Calm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `M_Tree_Medium` | `DEPRECATED_AnimatedTreeMaterial` | `M_Tree_Medium` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Medium_04_Ambient` | `FortniteFoliage_MasterMaterial` | `Tree_Medium_02_B_Calm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Medium_04_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `Tree_Medium_02_L_Calm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Pine_01_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Pine_02_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `M_Tree_Pine` | `DEPRECATED_AnimatedTreeMaterial` | `M_Tree_Pine` | TwoSidedFoliage | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Pine_05_Ambient_Inst` | `FortniteFoliage_MasterMaterial` | `MI_Pine_02_Ambient` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Pine_05_Ambient_Leaves_Inst` | `FortniteFoliage_MasterMaterial` | `MI_Pine_02_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Pine_02_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Pine_02_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_HeroTree_01_optimized` | `DEPRECATED_AnimatedTreeMaterial` | `MI_HeroTree_01_optimized` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `M_FieldGrassMatching` | `M_FieldGrassMatching` | `M_FieldGrassMatching` | DefaultLit | Masked | 2-sided | VS material code yes | FortniteMaterialParameters | Local: VS yes PS yes o[0, 1, 2, 3, 4] VS-UB [,View,Primitive,Material] depth-PS yes<br>ISM: same |
| `MI_Rectangle384_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_45Degree192_Storm` | Subsurface | Opaque | 1-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Rectangle192_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_45Degree192_Storm` | Subsurface | Opaque | 1-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Shrub_20_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Shrub_20_Storm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Shrub_22_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Shrub_22_Storm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Shrub_02_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Shrub_02_Storm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Shrub_04_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Shrub_04_Storm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Shrub_05_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Shrub_05_Storm` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Bush_Solid_2_Ambient` | `FortniteFoliage_MasterMaterial` | `MI_Bush_Solid_2_Ambient` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Bush_Solid_2_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Bush_Solid_2_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree02` | `DEPRECATED_AnimatedTreeMaterial` | `DEPRECATED_AnimatedTreeMaterial` | Subsurface | Masked | 2-sided | VS material code yes | FortniteMaterialParameters | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree04_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Tree04_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Bush_Solid_1_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Bush_Solid_1_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Bush_3_Inst` | `Fortress_BaseMaterial` | `MI_Tree_Bush_3_Inst` | DefaultLit | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Bush_Solid_3_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Bush_Solid_3_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `M_Tree_Log_1` | `DEPRECATED_AnimatedTreeMaterial` | `M_Tree_Log_1` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_Tree_Small_1a_Ambient_Leaves` | `FortniteFoliage_MasterMaterial` | `MI_Tree_Small_1a_Ambient_Leaves` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |
| `MI_StumpSet` | `DEPRECATED_AnimatedTreeMaterial` | `MI_StumpSet` | Subsurface | Masked | 2-sided | VS material code yes | - | Local: VS yes PS yes o[0, 1, 2, 3, 4, 5] VS-UB [,View,Primitive,PrimitiveFade,MaterialCollection0,Material] depth-PS yes<br>ISM: same |

Summary: 30 of 33 foliage slots are `MSM_Subsurface` (FortniteFoliage_MasterMaterial / DEPRECATED_AnimatedTreeMaterial), `M_Tree_Pine` is
`MSM_TwoSidedFoliage`, `MI_Tree_Bush_3_Inst` (Fortress_BaseMaterial) and the grass `M_FieldGrassMatching` are `MSM_DefaultLit`. All subsurface ones write o5.
Every foliage base-pass VS has vertex material code; the tree/shrub VS also binds `MaterialCollection0` = `/Game/Packages/Fortress_SharedMaterials/GlobalMaterialParameters/FortniteMaterialParameters`.

### Characters and held weapons

| Material | Root | Shader-map owner | Shading | Blend | Sides | WPO | Programs |
|---|---|---|---|---|---|---|---|
| `M_Med_Soldier_04_CAU_BR01` (body) | CharacterShader | CharacterShader_Gloss | SubsurfaceProfile | Masked | 2-sided | no | GPUSkin + Local base pass, o0-o5, depth-PS |
| `M_Med_Soldier_Head` (head) | CharacterShader | CharacterShader | SubsurfaceProfile | Masked | 1-sided | no | same |
| `M_SML_Survivor_Hair_02` (hair) | CharacterShader | CharacterShader_WhiteAO | SubsurfaceProfile | Masked | 1-sided | no | same |
| `Eyeshadow` | Eyeshadow | Eyeshadow | Unlit | Modulate | 2-sided | no | o0 only |
| `M_Athena_InvisibleGrenades` | itself | itself | DefaultLit | Masked | 1-sided | no | every coverage shader discards (not drawn) |
| `MI_FORT_Melee_Pick`, `MI_FORT_Rifles_MachineGun_1`, `MI_Sniper_Scar_Inst`, `MI_FORT_Shotguns_Crusader_Inst`, `MI_Sniper_01` | WeaponMat_Base | WeaponMat_Base | DefaultLit | Opaque | 1-sided | yes (Material UB) | GPUSkin + Local, o0-o4 |
| `MI_AmmoSet02_Inst` (shotgun slot 1) | BackPackShrinkingEffect | same | DefaultLit | Opaque | 1-sided | yes | same |

No audited material is PreintegratedSkin, Hair, Eye, Cloth or ClearCoat.

### What runs now

* **Shading models in the deferred resolve.** `fn-render/shaders/shading_models.wgsl` + `fx_resolve_sm.wgsl` (`sm_lit_world`, `fx_scene_lit_sm`), both in the
  scene-lit FX prelude (`fx.rs scene_lit_prelude_wgsl`). They follow UE4.16 GetDynamicLighting + SubsurfaceShading:
  * SUBSURFACE: SubsurfaceShadingSubsurface.
  * TWOSIDED_FOLIAGE: SubsurfaceShadingTwoSided.
  * The subsurface shadow term is the whole-scene CSM transmission of ShadowProjectionPixelShader.usf / CalculateOcclusion with bSubsurface: Density from
    CustomData.a, a per-cascade depth range taken from the cascade matrix, the same cascade blend / DF min / distance fade, and the light function.
  * Sky diffuse: DiffuseColor gets the subsurface colour added for SUBSURFACE, and TWOSIDED_FOLIAGE adds the back-face term Lookup(-N)·SSS·SkyVisibility
    (`g_sky_back` in lit_common.wgsl output_hdr).
  * fn-fx `program.rs` resolves every program that writes o5 with `fx_scene_lit_sm` (`GBUFFER_D_TARGET` = 5, `GBUFFER_VELOCITY_TARGET` = 4 exported for the velocity work).
  * The trees and shrubs drawn by `statics_render.rs` get this through `assemble_scene_lit`.
* **Characters.** `char_scene.wgsl` now resolves the translated head / hair / body GBuffer through the same `sm_lit_world` (o2.w ShadingModelID, o5 CustomData).
  * SubsurfaceProfile is lit as StandardShading on the stored BaseColor, which is UE's result with View.bSubsurfacePostprocessEnabled = 0.
  * The old local copy lacked the sun's MinRoughness 0.08 and F_Schlick's saturate(50·SpecularColor.g).
  * Weapons are DefaultLit opaque: no shading-model work is needed.
* **Character depth pre-pass (CHAR PREPASS, 2026-10-06).** The head, hair and body are `BLEND_Masked`. The cooked ini has `r.EarlyZPass=2`, `r.EarlyZPassMovable=True` and `r.EarlyZPassOnlyMaterialMasking=True`.
  * So UE draws the skinned character in the depth pre-pass with each material's own `FDepthOnlyPS` (the only place the opacity mask is clipped), then draws the base pass with depth Equal and no clip. BasePassRendering.cpp:308-361.
  * The port now does the same:
    * `fn-dxbc-cli --example br_character_materials` also emits `fn-render/shaders/char_depth_gen.wgsl`. That file holds the three `FDepthOnlyPS` programs of TGPUSkinVertexFactorytrue, which are identical; the head's is shader_70920.
    * `fs_char_depth` runs them in the pre-pass (`CharGpu::char_depth_pipeline`, LessEqual + write).
    * `CharGpu::char_pipeline` draws with Equal and no depth write.
    * `vs_char` and `vs_overlay` positions are `@invariant`.
  * The depth-only program is the CharacterResInEffect dissolve:
    * World Z against `DissolveGradientBaseLocation` / `DirectionalVector` / `Distance`, plus `T_NoisePack_01` x `GridDistortionAmount`.
    * `T_Slowfield_GridSquares_01` at UV x `texture pattern tiling` x `PatternAnimationOffset`.
    * A 1-D lookup into `ShieldPowerAnim`.a x `FirstSpawnPassComplete`, minus the 0.3333 clip value.
    * These are referenced textures 8, 9 and 10 of CharacterShader. Their sampler types are the eyeshadow's (both call `CharacterResInEffect`).
    * With the cooked defaults (`DissolveGradientPercentage` 1, `FirstSpawnPassComplete` 1) the mask is constant and nothing is clipped. The spawn-in / elimination dissolve parameters are runtime-driven and still at their defaults.
  * Image check (verification/mpc, 1600x900 at 6.5 h): against the same `@invariant` vertex shader without the pre-pass, the pre-pass is pixel-exact (0.0% of pixels > 2/255, max 2).
    * The `@invariant` attribute itself changes 0.6% of pixels (mean 0.09): float rounding in the screen-derivative tangent frame of `fs_char`.
    * The new baseline images are in `verification/mpc/charpre_h6.5`.
  * Speed: the scene pass at 2544x1353 dropped from about 9.0 to 6.7 ms. Before this, every overlapping layer of the hero was fully lit.
  * Still not like UE: the pipelines keep `cull_mode: None` for all three materials, but head and hair are 1-sided in UE.
* **Validation.** The fn-fx program tests pass (scene-lit prelude naga-valid) and so does fn-render `char_pipelines_create`, which includes the depth pipeline.

### Material parameter collections (DONE 2026-10-06, verification/mpc)

* **Binding.** A program's `MaterialCollectionN` = `FUniformExpressionSet::ParameterCollections[N]` (a collection StateId; `fn_fx::uniform::UniformSet::
  parameter_collections`), resolved through the root material's `MaterialParameterCollectionInfos` (fallback: the collections the shader-map package imports;
  `fn_fx::material::Material::collections`). Every program reading a collection binds the FX renderer's ONE buffer of it (`FxRenderer::collection_buffer`),
  created with the defaults by `fn_fx::gpu` (layout `fn_fx::mpc`: scalars 4 per float4 first, then one float4 per vector, = UE4.16 `GetParameterData` and the
  exe's @14227f350). The storm registers its `MPC_SafeZoneClouds` buffer the same way; an explicit `named` buffer still wins and is checked against the
  resolved path. Measured in a run: 59 program bindings to FortniteMaterialParameters, 3 to MPC_SafeZoneClouds, none unresolved (`[mpc]` report).
* **Readers among the drawn materials** (`mpc_probe`): props / buildings / trees VS + shadow VS float4 1.w `ImpactBounceStrength` (default 20; multiplied by
  the per-hit material value, 0 at rest: geometry pixel-identical); foliage PS 10.w / 11.x / 11.y (subsurface fade
  `saturate(saturate((SD - viewZ) / (SD * F) - 1) + AreDistanceFieldsEnabled?)`); building PS 12.w / 13.x / 59 (banner, x material cb[35].w = 0); the cloud
  dome PS float4 27 `SunAndMoonModelWorldPosition`; the sun PS float4 15 `FortniteActiveTimeOfDay`. Characters, weapons, pickups and the grass read none.
* **Writers** (`fn_fx::mpc`, per frame from `fn-app/src/mpc_hook.rs`):
  * TODM_A construction script: `World border cloud diffuse brightness` 0.5, `World Border Cloud Sub Surface Color and Opacity` (TODM_A CDO).
  * TODM_A `ReceiveBeginPlay` (once, when the world begins play): `AreDistanceFieldsEnabled?` = `GetShadowQuality() > 2` (sg.ShadowQuality, Epic = 1),
    `ShadowDistance` = the light's `DynamicShadowDistanceMovableLight` 4500, `ShadowDistanceFadeoutFraction` 0.1.
  * Manager tick: `FortniteActiveTimeOfDay` = phase weights (FUN_14064da00 / FUN_140635510; the smoothstep blend of FUN_140631d20), `RealGameTime`,
    `TimeOfDay_0-1`; `SkyLightColorAndIntensity` (FUN_14064f050); `SunlightDirectionalVector` = `SunAndMoonModelDirectionalVector` = the model direction and
    `SunAndMoonModelWorldPosition` = direction * `DistanceToSunOrMoon` (PositionTheSunAndMoonModels @0x140641e30).
  * NOT written (no drawn reader; listed with their 1.8 writer in `fn_fx::mpc`): Player*, SunLightColor / FogColor / FogDirectionalInscatteringColor,
    SunlightY/ZTransformVector, CloudsDirectSunLightColor / CloudsZenithColor, the weather-storm-only Foliage Directional Wind Intensity / StormStrength, the
    wind values. `MPC_Skybox_World` (written by the same construction script) is bound by no drawn program.
* **Found on the way:**
  * `View.DiffuseOverrideParameter` .. `RoughnessOverrideParameter` (float4 125..128.xy) were 0 in both View builders (`fn_fx::view`, `fn_matshader::view`).
    Every cooked SUBSURFACE / TWOSIDED_FOLIAGE base pass multiplies its SubsurfaceColor by 125.w (BasePassPixelShader.usf:632, unconditional), so the foliage
    GBufferD was always 0: they are now the FSceneView defaults (0,0,0,1) x3 / (0,1) a shipping build uploads (SceneView.cpp:592-595, :2161).
  * `DistanceToSunOrMoon` = 7.92e6 cm (constructor @0x14062cd04, not serialised): the hand-ported sky drew the disc at the SunMeshComponent template's
    3.6e6 (2.2x too large); `fn_render::sky::SUN_DISTANCE_CM` now uses it.
  * The hand-ported cloud dome (sky_clouds.wgsl) read `MaterialCollection0[27]` as the dome's position (sun tint and edge light at the dome centre); it is
    `SunAndMoonModelWorldPosition` (DXBC `cb2[27]`, both uses of r3).
  * `lobby_world.rs read_mpc_defaults` packed vectors first (right only because MPC_Pinata has no vector); it now uses `fn_fx::mpc`.
* **Verification aid:** `FN_MPC_SET="Name=value;..."` replaces FortniteMaterialParameters scalars in the uploaded buffer (forcing the fade to 0 changes 12 % of
  the forest pixels; it changed none before the View override fix, which is how that bug was found).
* **Regression check** (verification/mpc/regress, before_mpc.exe vs the new build): building pieces, the held gun + a rare pickup (hologram), the storm view,
  harvest debris and the start-up screens are unchanged; only foliage leaves (and their reflections) and the sky changed. `view_override_probe` agrees: of the
  storm, building, blueprint / preview, weapon, pickup, pickup FX, character and grass materials none reads View 125..128; the SUBSURFACE foliage base passes
  read 125 (the subsurface override). SubsurfaceProfile (characters) is not in BasePassPixelShader.usf:632's condition.
* **Limits:** `SunAndMoonModelWorldPosition` has no level -> FX `offset_cm` (fine while the TOD origin is the FX origin; a future cooked reader would need it);
  `RealGameTime` counts from app start in live play, not from the world's begin play.

### Not done (remaining plan)

1. **Wind inputs: NOT PRESENT IN 1.8 (verified 2026-10-06), nothing to port.** `AFortGameState::PreInitializeComponents` / `AFortGameStateZone::
   PreInitializeComponents` -> FUN_1405c3d80 spawns the wind manager from `UFortGameData::WindManager` (TAssetPtr<UClass> +0x1690). The only FortGameData is
   `/Game/Balance/DefaultGameData` (FortAssetManager GameDataName; `AthenaGameData` is a separate UAthenaGameData asset without that property); it does not
   serialize `WindManager` and the UFortGameData constructor (@1405751d0) sets it to an empty path, so 1.8 logs "NULL Wind Manager class specified by Game
   Data; cannot create Wind Manager; there will be no wind." and no `AFortWindManager` exists. Nothing else writes the tree MIDs' `Wind_*_CodeReferenced` /
   `Wind_Panner_*` / `WindVector_CodeControlled` (only the manager's ctor / FUN_1405213a0 and the editor preview `GetIntenseWindMaterialsForPreview` name
   them) nor the collection's wind scalars (only the manager's ctor; TODM_A's `Foliage Directional Wind Intensity` write is inside the weather-storm timeline).
   So the trees run their cooked material-instance wind values on View time and the collection keeps its wind defaults: what the port does. (The
   `/Game/VisualThreatManager/WindManager` Blueprint with its grass-wind scene captures is never spawned; the grass's wind textures stay DefaultTexture.)
   `Fort.Wind` (default 1; cooked scalability: 0 at Effects Low / Medium, 1 at High / Epic / Cine) only gates the manager.
   Superseded notes from the earlier research:
   * Native `AFortWindManager` (Fort.Wind=1 only) creates a MID per tree/shrub slot. It writes the speed/offset pairs `Wind_1..4_*_CodeReferenced` and
     `Wind_Panner_*_CodeReferenced` from `WindSpeedCurve` / `WindPannerSpeedCurve` evaluated at WindSpeed = clamp(|global wind + impulses|, 0, 1), and
     `WindVector_CodeControlled`.
   * It lerps the `TreeShader_ActiveVariables` scalars between the mild and `IntenseWindMaterials` MIDs.
   * The collection binding itself is DONE (see "Material parameter collections" below); the wind values of `FortniteMaterialParameters` keep their defaults
     until `AFortWindManager` is ported.
   * Open: whether idle BR wind is 0.1 at 45° (AFortWorldManager ctor) or about 0.0001, and the curve keys.
   * The grass gets no runtime writes; its cooked VS wind runs off View time.
2. **Grass (M_FieldGrassMatching, 35k blades): DONE 2026-10-06** (`crates/fn-app/src/grass_render.rs`, verification/grass/compare). The cooked
   `FInstancedStaticMeshVertexFactory` programs run on fn-fx `Vf::InstancedStatic` (FInstanceStream16 at ATTRIBUTE8-11 with UE's FFloat16 truncation, the UV
   repeat of InitRHI), the SetMesh values decoded from the serialized `FInstancedStaticMeshVertexFactoryShaderParameters` (fade cb0[11], dithered LOD cb0[6..10],
   `InstancingParams::hism`, `TemporalLod`), HISM culling by 32 m cells. Fixed on the way: unserialized texture-expression `Texture` = the class default
   (DefaultTexture), which shifted the grass's `ReferencedTextures` (and corrected the pickups' `EmissiveTexture` to T_Black).
3. **Screen-space subsurface pass** (SubsurfaceProfile SS_HeroSkin_01: setup / separable blur / recombine) for the character skin at Effects Medium+.
4. **Material quality / effects cvars.**
   * `fn_fx::cooked::Cooked::find` takes the first program in the package, with no shader-map quality selection. The grass package has a single SM5 map
     (quality Num). The tree / character maps could not be attributed by fn-matshader's label-anchored map scan (research tool removed, untested).
   * Not wired yet: r.EmitterSpawnRateScale (needs `bApplyGlobalSpawnRateScale` and `QualityLevelSpawnRateScale` in fn-particles' Spawn, UE4.16
     ParticleModules_Spawn.cpp:249 and ParticleEmitterInstances.cpp:1867), Fort.Wind, and r.SSS.*.
5. **Hit bounce.** `ABuildingSMActor::vfunc_313` / `vfunc_323` drive damage-MID globals (likely `Code_HitAnimation`, `WSDeformationCenter`,
   `Code_DeformationVector`, `WSDamageRadius`, `Code_DestructionAmount`; the name mapping is unproven).
