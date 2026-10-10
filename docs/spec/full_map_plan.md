# Full map port: plan after terrain (research workflow 2026-10-07)

**Plan after terrain (ordered, 8 steps)**

**1. Make the statics path cheap before adding content**
- **Goal:** cut CPU per frame. Use one pooled Primitive buffer with 512 B dynamic offsets and one bind group per material instead of per placement. Cull shadow casters per cascade (prism test plus small-caster test). Only push distance-field objects for resident tiles. Give POI kinds the real cull distance; today they use `cull_cm=0`.
- **Data:** cooked `ScreenSize[]`, the FortGlobals cull curve and ViewDistanceScale (CookedIni L3338, L5079-5094).
- **Touch:** `fn-app/src/statics_render.rs` (:226, :295, :380, :408-419), `fn-render/src/statics.rs`.
- **Risk:** low to medium. Shadows may differ.
- **Verify:** Salty report shows shadow draws ≤400 (now 2675) and frame time improves. Benchmark with `perf_walk1369.script` at 2544x1353, plus a real-vs-port screenshot of the shadows.

**2. Landscape material from the cooked DXBC**
- **Goal:** parse the level shader maps and decode `TerrainLayerWeightParameters` to map shader maps to components. Derive `LayerMask_*` from `WeightmapLayerAllocations`. Upload the raw weightmaps and heightmaps from `.ubulk`. Feed the interpolants, using UE absolute cm. Feed sky SH slots 155-162. Resolve through `gun_resolve`. Deduplicate by `program_hash`. Remove the fixed RGBA mapping and the `PLATE_BLEND_M` hack.
- **Data:** LS_00..05 and Athena_Terrain uexp/ubulk; `LandscapeRender.cpp:1036-1070`.
- **Touch:** `fn-matshader/src/maps.rs`, `material.rs`, `fn-render/shaders/backdrop.wgsl`, `docs/spec/material_coverage.md`.
- **Risk:** medium to high. The MIC-to-map mapping is UNVERIFIED (12 maps vs 10 MICs). About 32 permutations put pressure on the wasm heap.
- **Verify:** real-vs-port screenshots at 3 spots (grass/road, forest/mud, rock cliff), plus a unit test on the map-to-component table.

**3. Spatial tile bake for the whole map**
- **Goal:** bake 256 m tiles (about 144) of 32 B placements for LS_01-05 (5628 static mesh components) and all POIs, including recursive `Athena_SUB_*` worlds. Cull by tile, then per placement.
- **Touch:** `fn-export/src/props/{poi,layout,backdrop_actors}.rs`, a new tile writer, `statics_render.rs`.
- **Risk:** medium. The whole-map placement count is UNVERIFIED (30-100k).
- **Verify:** placement counts per level match `fn-inspect exports`; real-vs-port overview shots at 3 POIs; FPS walk.

**4. HLOD proxies**
- **Goal:** add the 74 landscape `LODActor`s plus about 11 building HLODs. Show a proxy when distance ≥ `MinDrawDistance` and hide its children.
- **Data:** the `Athena_Terrain_LS_0N_HLOD` packages; `SceneVisibility.cpp:3070-3100`.
- **Risk:** medium. How `r.HLOD.DistanceOverride=35000` is consumed is UNVERIFIED, and the switch will pop with no dither.
- **Verify:** far screenshots of the real client vs the port from high ground; about 85 draws beyond 560 m.

**5. Asynchronous kind streaming and memory budget**
- **Goal:** load kinds ordered by nearest placement within a per-frame budget, with LRU eviction. Bake WGSL offline. Deduplicate pipelines and create them asynchronously in the browser. Use f16 UVs and u16 indices.
- **Risk:** high on wasm. Per-kind pipeline cost (42 ms native) dominates, and memory may not fit.
- **Verify:** load time per `docs/spec/load_time.md`, the browser build runs, heap stays under budget.

**6. LandscapeGrassType procedural spawn**
- **Goal:** export the 4 grass types and their varieties, then spawn from the step 2 weightmaps with the real grid, jitter and cull distances on the GPU.
- **Data:** `MaterialExpressionLandscapeGrassOutput_4`, `Athena_Terrain_Grass*`.
- **Touch:** `fn-export/src/props/foliage.rs`, `fn-app/src/grass_render.rs`.
- **Risk:** medium. Instance counts are a big FPS risk.
- **Verify:** close-up grass and bush screenshots against the real client; FPS walk.

**7. Survey sublevels for InstancedFoliageActors**
- **Goal:** survey the LS and POI sublevels for painted foliage and export any found through the existing foliage path.
- **Risk:** low. This is UNVERIFIED whether any exists.
- **Verify:** `fn-inspect` counts.

**8. Optional: real landscape VS and LOD morph**
- **Goal:** replace the invented 1/4/16 LODs and 12 m skirts with the cooked landscape VS (`#970225`).
- **Risk:** medium. Collision must stay on the walkable mesh.
- **Verify:** wireframe and silhouette screenshots at distance.

Audio sublevel: leave it skipped for now.

**Contradictions between reports**
- **POI count:** map-inventory says 28 POI sublevels; streaming-perf says 29. Streaming-perf's breakdown (5 landscape + 29 POI + audio + LS_00) adds up to 36, but both reports say 35 sublevels.
- **"Terrain exported":** map-inventory marks it done, but landscape-material shows the shading is invented (fixed RGBA mapping, 450 m fade hack). Only the heightfield is truly exported.
- **Grass layers vs grass types:** the grass types are Grass, Grass_Standard, Gravel and Forest, but the material's layers are Grass, GrassNoFoliage, Forest, Mud and Road. "Gravel" has no matching layer, so the layer-to-type wiring is UNVERIFIED.
- **Grass units:** the grass report gives "Scale 1–6 m" and bare density numbers. In UE, scale is a multiplier and density is per 10 m², so the units are probably mislabelled.
- **Cull distances:** HLOD members have `CachedMaxDrawDistance` 800-940 m, but the fort curve gives 60-240 m. It is unresolved which one wins.
- **Painted foliage:** the grass report says "no trees" in the persistent level, while streaming-perf counts 5628 StaticMeshComponents in the landscape levels. These do not conflict (actors vs foliage), but foliage in the sublevels is unsurveyed.

**Still UNVERIFIED**
- MIC-to-shader-map mapping.
- Whether `LayerMask_*` and `Weightmap*` values are serialised.
- Whole-map placement total and geometry size.
- Whether the DXBC-to-WGSL path can take instanced transforms.
- How `r.HLOD.DistanceOverride` is used.
- Sublevel foliage.

## Status: all POIs in the game (2026-10-07)

**Export.** `fn-export props poi --all --out <assets>/props` writes `poi/index.json` plus one file per streamed level of `Athena_Terrain`. The level list is read from the data: the 35 `LevelStreaming*` exports of the persistent map (`layout::levels`).
- Skipped: `Athena_Terrain_Audio`.
- 29 `Athena_POI_*` files, each holding the whole level plus the building worlds its foundations stream:
  - bridges `bridge_001`..`005`
  - `communitypark_001`, `containertown`, `durrrburger_001`
  - `farmstead`, `farmstead_002`, `hedgemaze`
  - `lake_001`, `lakeisland_001`, `lobby_001` (the spawn island)
  - `mesa_002`, `mesa_005`, `prison_001`
  - `rangeroutpost` (Lonely Lodge), `roadside_001`, `rvpark_001`
  - `stripmall`, `suburban_006` (Salty Springs), `suburban_015`
  - `swamp_001`, `toiletfactory_001`, `tunnel_001`
  - `warehouses_001`, `warehouses_002`
- `ls_00`..`ls_05` and `terrain`: everything except the trees, rocks, bushes and hedges that the props layout places (catalog classes of those categories). That covers terraces, cliffs, shacks, cars and fences.
- Totals: 35573 mesh placements, 1439 mesh kinds, 35k actors, 78 MB of JSON.
- The map's ground-cover foliage is only counted.
- The building worlds' `S_WavyGrass_01` instances (50942) go to `ground_cover` for the grass path.
- Curve keys are stored once per health row in `health_rows`.
- `lodge.json` and `salty.json`, the old radius exports, are no longer loaded once `index.json` exists.

**Game (`fn-app/src/poi.rs`, `statics_render.rs`).**
- **Loading.** Every file is parsed on the loader workers in about 250 ms.
- **Collision.**
  - Hulls are built once per mesh and then mapped per placement.
  - Collision is always present: 35035 colliding actors (28125 damageable, 6910 invulnerable), 367k convex pieces.
- **Deduplication.** A layout placement that is a POI actor (same class within 5 cm) is left to the POI path.
- **Streaming.** Placements are grouped by file and 256 m cell (174 groups). A group is resident while the view is within 350 m of its bounds, with 10% hysteresis.
  - 350 m is `r.HLOD.DistanceOverride`. The `LODActor` proxies the real client shows beyond that are NOT PORTED.
  - A resident group's kinds build nearest first, 6 ms per frame live and all at once in scripted runs.
  - The groups around the spawn build under the loading screen: 647 kinds in about 20 s, compared with the old first-frame build of 257 kinds in 15 s.
- **Culling.**
  - Placements beyond 350 m are not drawn.
  - Off-screen placements beyond 140 m are dropped. That is the CSM range of 45 m × 2 plus 50 m.
  - Both keep their distance-field object.
- **Terrace edges.** At the lodge the landscape lies 6-10 m under the POI terraces. The probe ran along UE x 53652, y 66000..80000. The edges are covered by the cooked cliff meshes (`S_Cave_*`); see the edge shot from the landscape side, verification/all_pois/edge_lodge.script. The old gap came from two things: the 150 m radius export left out the `Athena_Terrain_LS_*` terraces and cliffs, and the pawn stood under the terraces.
- **Spawn.** `GameSim::stand_on_floor` puts the spawn, a respawn and the script `tp=X:Y[:UP]` on the highest collision surface. At the default spawn the pawn now stands on the lodge terrace at 17.1 m, not under it at 3.2 m.
- **Grass.** Each blade stands at its real UE z (the floating fix), and the whole map's grass is loaded.

**Dev switches.**
- `FN_POI=0`: no POIs.
- `FN_POI_STREAM_M=<m>`: residency distance.
- `FN_POI_COLLIDE=0`: POIs are drawn only, for the perf split.

**Perf.** `perf_walk1369`, 2544x1353, RTX 2060.
- Before (lodge.json only): 51 fps (19.5 ms).
- After: 46-48 fps (21.0-21.9 ms) on the old path. The pawn now walks on the terrace, which shows more of the town; that run measured 42 fps.
- The extra CPU is the O(props) scans of the sim and HUD with 35k collision props, about 3 ms, plus about 1.8 ms of distance-field prep. Each of these costs 0.2-0.3 ms: `trace_cylinders` for focus, reticle and building HUD; `fn_props::trace`; `World::update`; and `refresh_blockers`.
- The camera traces now only see props within 40 m (`GameSim::camera_props`).

**Not 1:1 / open.**
- HLOD proxies: a POI farther than 350 m is not drawn.
- Layout trees and rocks still exist only within 150 m of the layout centre. Only the trees inside POI levels exist everywhere.
- Kinds that build while playing cause frame hitches of about 40 ms per kind. The shader translation is synchronous.
- The wasm build was not run. It parses the 78 MB of JSON on up to 8 threads in the browser.
- POI lights, particles and loot are not drawn.

---

# Research reports

## landscape-material
**Verdict:** feasible. The cooked data is complete, so the landscape can be rendered from the original shaders, not reconstructed. It needs one new vertex-factory emulation and some new parsing. I only read data and ran existing exes; I did not build or run any Rust. Two steps depend on that and are marked UNVERIFIED.

**Assets verified**
- Root material: `/Game/Athena/Prototype/Terrain/M_Athena_Terrain_Landscape` (class Material, blend `BLEND_Masked`, DefaultLit). Its package has only `FLocalVertexFactory` programs, which landscape never uses.
- Instance parent: `M_Athena_Terrain_Landscape_Inst`, which overrides `GrassMudEdge_Amount` -3.5, `RoadContrast` 768 and `RefractionDepthBias` 0.
- Each `LandscapeComponent` has one `LandscapeMaterialInstanceConstant` (MIC) in its level package. The cooked shader maps are in those level packages: `/Game/Athena/Maps/Athena_Terrain` and `/Game/Athena/Maps/Landscape/Athena_Terrain_LS_00..05`. The sublevel umaps exist in `pakchunk0_s1` (`fn-inspect ls`).
- There are 94 components: 2 + 6 + 16 + 12 + 31 + 8 + 19. All have `NumSubsections` 1, `ComponentSizeQuads` 127, and no `XYOffsetmapTexture` (checked all 7 packages). So only `FLandscapeVertexFactory` is needed, not the XY-offset variant.
- The LS_00 names table lists `FLandscapeVertexFactory`. LS_00 holds 12 SM5 plus 12 SM4 shader maps, all labelled "M_Athena_Terrain_Landscape".
- The MIC props show only `Parent` and `bHasStaticPermutationResource`. The `LayerMask_*`, `Weightmap0/1` and `Heightmap` parameter names are in the names table. Whether their values are serialised is UNVERIFIED; they can be derived anyway (step 3).
- The graph itself is not recoverable. Only the parameter, texture and function-call nodes are exported (112 exports). The Multiply, Power and Lerp nodes referenced by name are missing. This is why the DXBC route is the right one.

**Layers and parameters (root props)**
- Layer names: Grass, GrassNoFoliage, Forest, Mud, Road. Road_2 is the layer-info asset. `DataLayer` appears on 3 components.
- Layer blend types: height blend on some layers, weight blend on others.
- Textures: T_Grasslands_D/N, T_FORT_ForestFloor01, Mud, T_Street_Road_D, T_Cave_Exterior_Wall_D/N, WhiteRockCustom_D.
- Scalars include `TexSize` 1534.97, `TexSize_1` 768, `TexSize_2` 3740.7, `RoadSize` 1024, `GrassSpecPower` 2, `GrassSpecAmount` 0.3, and the Rock_Tiling_X/Y values.
- Vectors: Road_Tint, Mud_Tint, RockTint_1, RockTint_2. Static switches: UseTopoMasks and UseCheapProjection.
- Per-component weightmap layouts: about 32 distinct layer-to-channel combinations. Examples: Grass,GrassNoFoliage,Mud,Forest in texture 0; Road_2 in texture 1; Forest in texture 1.
- The fixed r/g/b/a layer mapping in `backdrop.wgsl` `shade_terrain` therefore cannot match every component.

**Cooked shaders (LS_00 via `fn-dxbc scan`)**
- Each map has VS, PS, depth-only and shadow programs.
- Example 5-render-target base-pass PS: `LS_00#1020146` (10.4 KB, t0..t8, s0..s6). It writes GBuffer outputs o0..o4. `mov o2.xzw (0,0,0.8,0.192157)` confirms roughness 0.8 and the DefaultLit shading ID. `fn-dxbc translate wgsl` succeeds (1096 lines) and it has no UAV or atomic ops.
- Some PS variants (for example `#976825`) are 2-output shadow variants with UAV stores. Do not pick those.
- The PS samples weightmap t0 at v1.xy and the heightmap-as-normalmap at v1.zw. That is the `NormalmapTexture`, with the normal in the B and A channels.
- It uses triplanar rock sampling from world position (`v3 - cb1[62]`, in UE centimetres) and layer masks from `dp4 weightmap, cb4[1..4]`.
- Material CB constants include the `LayerMask_*` vectors.
- VS `#970225` samples the heightmap with `sample_l` (t0, plus t1 for an XY-offset map that is unused here). It reads CB0[19] for the landscape and LOD parameters and takes one float4 vertex attribute.
- Source: `LandscapeVertexFactory.usf`, and `LandscapeRender.cpp:1036-1070` for the uniform values.

**Existing tooling**
- `fn-matshader` has `MaterialMaps::parse` (shader maps, programs, vertex-factory name, bindings), the View/Primitive buffer builders (`view.rs`, `VIEW_FLOAT4S` 178), and the DXBC-to-WGSL assembly into `gun_resolve.wgsl`.
- It is gun-only, hard-wired to `TGPUSkinVertexFactoryfalse`, and has never been run on a level umap.
- `docs/spec/material_coverage.md:61-66` already records the landscape as open work.

**Steps**
1. Parse the maps from each level's uexp with `MaterialMaps::parse(label "M_Athena_Terrain_Landscape")`. Select programs by `find(map, "FLandscapeVertexFactory", "TBasePassVS…NoLightMapPolicy" / "TBasePassPSFNoLightMapPolicySkylight")`. Do not pick programs by size.
2. Decode the `StaticParameters.TerrainLayerWeightParameters` (name, `WeightmapIndex`) in each `FMaterialShaderMapId`. This ties each map to the components that use it. `maps.rs` only finds the id head today.
3. Build the Material CB per component with `uniform::evaluate`. The `LayerMask_<layer>` value is the unit vector for `WeightmapTextureChannel` in `WeightmapLayerAllocations`. The scalar and vector parameters come from the MIC chain.
4. Upload per component the raw 128×128 BGRA8 weightmap and heightmap textures. The mip 0 data is in the `.ubulk` (LS_00 is 917,504 bytes, 14 × 65,536). The layer textures come from the packages above.
5. Feed the interpolants from the existing CPU mesh. Layer coords are local XY plus `SectionBase` in quads. The weightmap and heightmap UVs use `WeightmapScaleBias` and `HeightmapScaleBias`. v3 is the translated world position.
6. Optional: emulate the VS and its LOD morph. The current 1/4/16-stride LODs with 12 m skirts are not the real LODs.
7. Resolve the GBuffer outputs through `gun_resolve`.

**Blockers and risks**
- The mapping from MICs to shader maps is UNVERIFIED (LS_00 has 12 SM5 maps for 10 MICs and 6 components). Step 2 resolves it.
- Whether the MIC `LayerMask_*` and `Weightmap*` values are serialised is UNVERIFIED. Step 3 covers it by deriving them.
- The View buffer must be built for the backdrop camera (near 20 m, far 2e6 m) and the extra landscape buffers added.
- The pass needs UE absolute centimetre world coordinates, including the Z offset the port subtracts. Otherwise the triplanar rock phase is wrong. The 450 m `PLATE_BLEND_M` height fade is an invented hack that the real normal map will not follow.
- There are about 32 permutations. Deduplicate by `program_hash` to limit WGSL modules, which matters for the browser build.
- The base-pass PS needs its sky-light SH inputs (View slots 155-162) fed.
- The `backdrop.rs:18` claim that nodes are "not serialised" is only partly true. It does not matter once the DXBC is used.

Files: `crates/fn-render/shaders/backdrop.wgsl`, `crates/fn-matshader/src/maps.rs`, `crates/fn-matshader/src/material.rs`, `docs/spec/material_coverage.md`.

## landscape-grass
Based on my investigation of Fortnite 1.8 assets and the current port implementation, here's the landscape grass/foliage spawn system and what the port currently exports:

## 1.8 Landscape Grass System (Data Sources)

**Material-Driven Grass Output**
- **Landscape Material**: `/Game/Athena/Prototype/Terrain/M_Athena_Terrain_Landscape` (fn-inspect exports, M_Athena_Terrain_Landscape)
- **Expression**: `MaterialExpressionLandscapeGrassOutput_4` (export index 1; fn-inspect props) connects 4 terrain layer outputs to 4 distinct LandscapeGrassType assets
- **Layer**: `/Game/Athena/Prototype/Terrain/Grass_LayerInfo` (LandscapeLayerInfoObject, LayerName="Grass"; used by LandscapeComponent_1 WeightmapLayerAllocations, fn-inspect props Athena_Terrain)

**Grass Types & Varieties** (fn-inspect props `/Game/Athena/Prototype/Terrain/Athena_Terrain_Grass`):
- **Athena_Terrain_Grass** (input: MaterialExpressionSubtract_2): 8 GrassVarieties
  - Mesh 0-1: `Athena_SM_WavyGrass_01` (Density 50.0 / 10.0, Scale 1–6 m, Cull 6000–9000 cm, AlignToSurface, Grid)
  - Mesh 2: `Foliage_Grassblades_Clump01_Animated` (Density 0.045, Scale 3–4.5 m, Cull 5000–8000 cm, AlignToSurface)
  - Mesh 3: `SM_Athena_Bush_01` (Density 0.08, Scale 0.5–0.8 m, Cull 7000–25000 cm, Grid)
  - Mesh 4: `SM_Athena_Plant_03` (Density 0.075, Scale 1.0–1.25 m, Cull 10000 cm)
  - Mesh 5: `SM_Fort_Foliage_Shrubs_Shrub02a` (Density 0.1, Scale 0.65–1.0 m, Cull 2000–8000 cm, Grid, PlacementJitter 0.5)
  - Mesh 6: `Shrub_05` (Density 0.05, Scale 0.65–1.0 m, Cull 2000–6000 cm, Grid)
  - Mesh 7: `Mushroom01` (Density 0.035, Scale 0.5–0.65 m, Cull 8000 cm, Grid)
- **Athena_Terrain_Grass_Standard**, **Athena_Terrain_Gravel**, **Athena_Terrain_Forest** (3 more types, not yet detailed)

**Painted Foliage**
- **InstancedFoliageActor_0** (Athena_Terrain.umap, export index 35; fn-inspect props): contains only `FoliageInstancedStaticMeshComponent_0`
- **Component 0**: `S_WavyGrass_01` (555,419 instances; StaticMesh `/Game/Environments/Foliage/Meshes/S_WavyGrass_01`; Cull 4096–5120 cm; CastShadow false; Mobility Static; fn-inspect props)

## Current Port Export (crates/fn-export/src/props/foliage.rs + grass_render.rs)

**Exported**: Only `FoliageInstancedStaticMeshComponent_0` (S_WavyGrass_01 ambient ground cover)
- glTF mesh + cooked material (`M_FieldGrassMatching`)
- 555k instance transforms (position, yaw, scale) + PerInstanceRandom
- Cull distances (4096–5120 cm) respected in GPU cull
- Output: `<props>/foliage/foliage.json` + `instances.bin`

**Rendered via**: `grass_render.rs` (FX vertex factory InstancedStatic, wind WPO via T_FieldGrass_01_M/T_SwirlNoise_N, blade normal blend, terrain grass colour lookup)

## Gap List

1. **LandscapeGrassType procedural spawn**: Port does NOT export the 4 LandscapeGrassType assets or their GrassVarieties; no procedural grass layer generation per terrain weight map.
2. **Material LandscapeGrassOutput**: No exporter for `MaterialExpressionLandscapeGrassOutput` and its GrassTypes array.
3. **Painted foliage variants**: InstancedFoliageActor holds only S_WavyGrass_01 in 1.8 (verified); no trees/rocks/other actor-placed foliage in Athena PersistentLevel (UNVERIFIED: may exist in sublevels).
4. **Sublevels**: Port may have InstancedFoliageActor in sublevel packages (e.g., POI blueprints); not surveyed.
5. **Density/scale distribution**: 1.8 uses procedural jitter + grid placement per GrassVariety; port bakes all instances at export time.

---

**Sources cited**: `/Game/Athena/Prototype/Terrain/M_Athena_Terrain_Landscape` (MaterialExpressionLandscapeGrassOutput_4, export 1); `/Game/Athena/Prototype/Terrain/Athena_Terrain_Grass` (LandscapeGrassType); `/Game/Athena/Maps/Athena_Terrain` (InstancedFoliageActor_0, export 35; LandscapeComponent_1 props; Landscape1 props); `crates/fn-export/src/props/foliage.rs:1–185`; `crates/fn-app/src/grass_render.rs:1–100`.

## map-inventory
Based on my analysis of the Athena_Terrain persistent level, here's the comprehensive inventory:

## Athena_Terrain Streaming Inventory

| **Kind** | **Count** | **Export Handler** | **Content** | **Gap/Status** |
|----------|-----------|-------------------|------------|----------------|
| **Terrain Landscape (LS_00–LS_05)** | 6 | `backdrop.rs` | LandscapeComponent (heightfield, weights, materials) | ✓ EXPORTED |
| **POI (Points of Interest)** | 28 | `layout.rs`, `poi.rs`, `backdrop_actors.rs` | Static mesh actors, harvestable props, building foundations | ✓ EXPORTED (partial: harvestables via layout; full mesh via poi --center/--radius) |
| **Building Sublevels (Athena_SUB_*)** | 23* | `poi.rs` (recursive via AdditionalWorlds) | Interior structures (houses, stores, etc.) | ✓ EXPORTED (when foundation present) |
| **Audio (Athena_Terrain_Audio)** | 1 | None | Sound actors, emitters, ambience | **SKIPPED** |

**Persistent Level Content:**
- Backdrop actors (ocean planes, far skybox rocks): `backdrop_actors.rs`
- Ambient foliage (S_WavyGrass_01): `foliage.rs`
- Safe/IslandZone volumes: geometry (BodySetup)

**Key Export Paths** (crates/fn-export/src/props/):
- `layout.rs` — harvestable placements (persistent + LS only, not POIs by default)
- `backdrop.rs` — terrain heightfield from persistent + LS_xx
- `backdrop_actors.rs` — FortStaticMeshActor backdrop (persistent only)
- `poi.rs` — all static meshes + foundations in POIs + recursive building worlds
- `foliage.rs` — FoliageInstancedStaticMeshComponent ambient cover

**Verified Facts:**
- All 35 sublevels use `LevelStreamingAlwaysLoaded` class (/Game/Athena/Maps/Athena_Terrain.Athena_Terrain:PersistentLevel)
- POI sublevels load via WorldAsset + LevelTransform with actor transforms [fn-inspect props: /Game/Athena/Maps/Athena_Terrain]
- Building sublevels not in LevelStreaming — loaded on-demand by poi.rs when scanning BuildingFoundation.AdditionalWorlds
- 23 unique Athena_SUB_* variants (3x3 and 5x5 tile houses/buildings)

**Gap: Audio sublevel — no exporter processes Athena_Terrain_Audio actors or soundscapes.**

*Building sublevels are not LevelStreaming entries but recursively loaded by poi.rs; count from ls "*Athena_SUB*.umap" with FN_ROOT set.

## streaming-perf
**Plan: streaming the whole 3 km map's statics (Athena only). Read-only. Numbers marked UNVERIFIED are my estimates.**

**Today**
- **Draws:** the real-shader FX path (`crates/fn-app/src/statics_render.rs:371-440`) makes one non-instanced `FxDraw` per placement per section. Each placement gets its own 512 B Primitive buffer (`:295`) and its own bind group and VS-globals buffer per (material, placement) (`:408-419`), and none are ever freed.
- **Culling:** the per-frame loop walks every live prop and every POI placement (`:349`, `:360`). Frustum culling, `ComputeStaticMeshLOD` and the cooked `ScreenSize[]` are all per placement. There is no spatial grid.
- **Distance field:** distance-field objects are pushed for every casting placement before the cull test (`:380`).
- **POI kinds:** they are built with `cull_cm = 0` (`:226`), so they are never distance-culled.
- **Old textured path:** `crates/fn-render/src/statics.rs:301-345` is instanced (48 B per instance, one draw per mesh/LOD/section, per-cascade buckets) but linear over all instances and uses estimated materials.
- **Vertices and indices:** vertices are 40 B for 1-2 UV sets (`gpu.rs:59`, f32 UVs). Indices are u32 (`fx.rs:545`).
- **Measured at Salty** (`verification/salty/shots/salty-report.txt:318`, 1080p): 2390 placements, 245 drawn, 314 main draws, **2675 shadow draws**, 166k triangles. Frame 51 fps. Scene-draw CPU 13.4 ms. `prep finish` 7.6 ms. Distance-field prep 2.2 ms plus tiles 1.8 ms. GPU only 7.2 ms, so it is CPU-bound.
- **Load:** `shots.log:1203` shows POI build of 406 kinds in 17.1 s, about 42 ms per kind, on the first frame. Per `docs/spec/load_time.md`, translation and pipelines cost about 6.5 s on the browser main thread, and each sync pipeline costs 0.5-1.1 s there.
- **Texture memory** (`performance.md` §6): BC blocks keep forest-walk FX textures at 77 MB.

**What 1.8 does**
- **Streaming:** `Athena_Terrain` has 35 `LevelStreamingAlwaysLoaded` sublevels: 5 landscape (LS_01-05), 29 POI, audio and LS_00. Everything is resident and culling does the work.
- **Static-mesh counts** (fn-inspect `exports`): the landscape levels hold 5628 `StaticMeshComponent`s (LS_01 1736, LS_02 418, LS_03 1585, LS_04 119, LS_05 1770). The 29 POI levels hold 16,281.
- **Building sublevels:** Salty's POI file lists 332 components but the baked set has 2369 placements, about 7x (`poi/salty.json`).
- **Whole-map total:** UNVERIFIED, roughly 30-100k placements.
- **Cull distance, no HLOD parent:** the FortGlobals curve gives 60-240 m, default 180 m (`quality.rs:149-175`, exe `@140498440`, CookedIni L3338).
- **ViewDistanceScale:** 0.5 / 0.67 / 0.83 / 1.0 (CookedIni L5079-5094).
- **HLOD:** 74 landscape `LODActor`s (16+9+27+5+17 in LS_01-05) plus about 11 building HLOD packages.
  - **Clusters:** 6-103 sub-actors each. `LODDrawDistance` is about 240-560 m, typically about 490 m, with `LODLevel` 1 (LS_03 `LODActor_*` props).
  - **Members:** in LS_03, 1361 of 1585 components have `LODParentPrimitive`. Their `CachedMaxDrawDistance` is about 800-940 m.
  - **Switch rule:** a proxy shows when distance >= `MinDrawDistance`, and its children hide (UE4.16 `SceneVisibility.cpp:3070-3100`; `LODActor.cpp:406,684`).
  - **Override:** `r.HLOD.DistanceOverride=35000` (CookedIni L994) is UNVERIFIED as to how it is consumed.
  - **Proxy data:** `/Game/Athena/Maps/Landscape/HLOD/Athena_Terrain_LS_0N_HLOD` holds one StaticMesh, MIC and Texture2D per cluster. LS_01-05 total about 26.7 MB cooked.
- **LOD scale:** `r.StaticMeshLODDistanceScale` is absent from the cooked ini, so scale 1 (`statics_render.rs:22`).

**Design**
1. **Bake** (fn-export) per-tile binary placement files. Use a 256 m grid, about 144 tiles. Each placement is 32 B (kind u16, pos, yaw, scale, flags, HLOD id). That is about 2-3 MB for 60-100k placements, fetchable over HTTP on wasm.
2. **Cull by tile first.** Test tile AABBs, then iterate only surviving tiles. Expect about 10 tiles, about 4k placements per frame, instead of every placement.
3. **Distance bands:**
   - Non-HLOD statics use the fort curve times ViewDistanceScale. Apply it to POI kinds too (they are currently uncapped).
   - HLOD'd statics show below `LODDrawDistance` and their proxy shows beyond it.
   - Beyond about 560 m the whole map is the 74 proxies plus about 11 building HLODs, roughly 85 draws.
   - Keep tile instance data resident only within about 600 m.
4. **Draw budget** (targets): main view <= 800 draws, with 85 of them HLOD. Shadows <= 400, with CPU per-cascade prism and small-caster culling before pushing, as the old path does (`statics.rs:340`). Today's 2675 is the problem.
5. **Primitive buffers:** use one pooled buffer with 512 B dynamic offsets and one bind group per material entry. GPU objects drop from O(placements) to O(materials).
   - Phase 2: instancing via an instance-indexed transform in the DXBC-to-WGSL translation. Feasibility UNVERIFIED.
   - Memory: 32 B per placement on the CPU, about 3 MB total, versus about 1 KB or more of GPU objects per placement today (50k placements is about 50 MB plus about 100k wgpu handles).
6. **Distance-field objects:** only for resident tiles within the global distance field clipmap extent, not all placements. Prep is 4 ms at 2.4k placements and scales linearly.
7. **Kind streaming:**
   - Ordered by nearest placement, with a per-frame budget like the 25 ms of `prebuild_step`.
   - LRU eviction around 600-800 MB for statics.
   - Pre-translated WGSL baked offline.
   - Pipelines deduplicated by shader map and render state, created asynchronously on the browser. A placement waits for its pipeline.
   - Halve vertex UVs (f16) and use u16 indices where possible: about 35% less geometry.
   - Geometry estimate, UNVERIFIED: Salty's 406 kinds are about 40 MB, the whole map 100-200 MB.

**Top risks**
1. Per-kind material and pipeline cost (about 42 ms native, seconds on the browser) dominates, not geometry.
2. The hundreds of unique materials may exceed the wasm heap and texture budget (the parsed shader maps are already most of the browser heap).
3. If the DXBC translation can't take instanced transforms, the draw count caps the browser frame rate.
4. Catalog `cull_distance_cm` may not match the cooked `CachedMaxDrawDistance`. The 800-940 m of HLOD members versus 60-240 m is a real gap.
5. The bus drop sweeps 3 km fast, so the landing area must be prefetched.
6. No dithered HLOD fade, so pop-in at the switch distance.
