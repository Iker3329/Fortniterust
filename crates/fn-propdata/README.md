# fn-propdata

Pure-Rust loader for the **real Fortnite 1.8 harvestable props** (trees, rocks, bushes, cars, pallets, fences, ...) exported by
`fn-export props export` into `FortniteRuntime/assets/props/`. Dependencies: `serde_json`, `image` (PNG). No GPU / windowing /
UE dependency, `unsafe_code = "forbid"`. The game reads health / resource / collision / placements, the renderer reads meshes,
materials and textures.

```rust
let cat = fn_propdata::PropCatalog::load("assets/props")?;
let oak = cat.get("Athena_Tree_Large_1").unwrap();      // PropKind { id, display, lods, materials, collision, health, resource, .. }
assert_eq!((oak.health, oak.resource.kind), (600.0, ResourceKind::Wood));
let lods = oak.load_lods()?;                            // Vec<LodMesh>, metres, Y up
let mat  = oak.load_material(1)?;                       // .mat.json of slot 1 (alpha mode, two-sided, textures, wind params)
let rgba = mat.base_color_texture().unwrap().load_image()?;
let sample = cat.load_sample_layout()?;                 // 40 PropInstance around a dense tree + rock cluster
let all    = cat.load_full_layout()?;                   // 9609 harvestable placements of Athena_Terrain
```

`PropInstance::game_position()` / `game_yaw()` convert a placement to the runtime convention (see "Coordinates").

## Exporting

```text
fn-export props export --paks <Paks dir> --out <assets>/props --report reports/props-1.8.json
    [--min-count 25] [--only substr] [--classes A,B] [--max-tex 2048] [--no-sheet]
fn-export props survey|meshcheck|layout|refs|sheet ...      (investigation tools, see src/props/mod.rs)
```
Kinds exported: every harvestable Blueprint class (`AttributeInitKey` category `Resource*` / `Prop*` with a wood / stone / metal
`ResourceType`) placed in `Athena_Terrain` (persistent level + 35 always-loaded sublevels, level transforms applied) that is a
tree or rock, or placed at least `--min-count` times. 85 kinds in the 1.8 export.

## On-disk schema (all `schema: 1`)

```text
props/catalog.json                       { schema, game, source_map, units, textures_dir, kinds: [{id, dir, display, category, resource, health, placed_in_map, lod0_vertices}] }
props/textures/<Texture>.png             shared RGBA8 PNG (<= 2048 px; opaque textures are stored RGB), textures/index.json = sRGB flag, compression, sizes, has_alpha, grey
props/<Id>/<Mesh>.gltf + .bin            all LODs (see "glTF")
props/<Id>/materials/<Material>.mat.json one per material used by the kind's slots
props/<Id>/collision.json                BodySetup simple collision
props/<Id>/props.json                    everything else (below)
props/layout/all.json | sample.json | stats.json
props/sheet/contact-sheet*.png           CPU-rendered check images
```
`<Id>` is the Blueprint class name without `_C` (`Athena_Tree_Pine_3`), the key used by the layouts.

### props.json
```text
id, display, category (tree|rock|bush|hedge|log|vehicle|container|fence|pallet|crate|furniture|other), blueprint, class_chain[]
mesh      { name, ue_path, gltf, bounds_cm{origin,extent,radius,min,max}, lod_for_collision, light_map_coordinate_index, sockets[],
            lods[{vertices, triangles, screen_size, uv_channels, vertex_colors, index_buffer_32bit, sections[{material_slot, triangles, first_index, min_vertex, max_vertex}]}] }
materials [{slot, slot_name, name, ue_path, mat_json, source: mesh|component_override}]       slot i = glTF material i = StaticMaterials[i]
collision "collision.json"
health    { max, attribute_row ("ResourceWood.4.FortHealthSet.MaxHealth"), found, attribute_key }     AttributesBuildingProps key at curve time 0
resource  { type: Wood|Stone|Metal, amount_row, amount_table, rate_t0, rate_t1, athena_rate_t0, athena_rate_t1, semantic }
destruction { death_particles, death_sound, hit_sound, loot_tier_group, replacement_class (null), related_properties{}, note }
render    { cull_distance_cm, cull_distance_m, random_scale, random_scale_range, random_yaw, cast_shadow, wind{ class-level wind / leaf emitter properties } }
placement { athena_terrain_count, scale_min, scale_max, scale_mean }
verification { per-LOD counts, section vertex ranges, bounds vs ExtendedBounds, StaticMaterials count, glTF round trip through this crate }
ue_properties { class_default_object (merged CDO of the Blueprint chain), static_mesh_component }
```

### glTF (`fn_mesh::gltf_lods`)
One node + mesh per LOD (`LOD0..LODn`, the scene lists LOD0 only); a **shared vertex set per LOD** (`POSITION`, `NORMAL`, `TANGENT`,
`COLOR_0` when present, `TEXCOORD_0..n`), one index accessor per section (u16, u32 above 65535 vertices), `primitive.material` = material
slot. Materials carry `baseColorTexture`, `normalTexture`, `alphaMode` (`MASK` + `alphaCutoff` for `BLEND_Masked`), `doubleSided`, and
`extras` (`slot`, `slot_name`, `ue_material`, `mat_json`). `extras.lods` on the document lists node / mesh / screen size / counts.
Tangent `w` is **-sign_UE** (the axis swap is a reflection; this is the glTF-correct bitangent sign), unlike the legacy exporter.

### *.mat.json
`blend_mode`, `alpha_mode` (`OPAQUE|MASK|BLEND`), `opacity_mask_clip_value`, `two_sided`, `shading_model`, `chain` (instance -> ... ->
root material), `textures{param: {texture, ue_path, png (relative), srgb, compression, format, width/height (PNG), source_width/height
(cooked top mip), address_x/y, has_alpha, grey}}`, `slots{base_color, normal, mask, opacity_from, source}`, `scalars`, `vectors`,
`static_switches`, `wind{uses_world_position_offset, parameters}`, `inputs_connected`, `tint_guess`.

* **Texture roles are a heuristic** unless `slots.source` starts with `graph-direct`: the cooked material keeps only parameter nodes
  (the arithmetic between them is stripped), so `base_color` / `normal` / `mask` are picked from parameter names, compression
  settings (`TC_Normalmap`, `TC_Masks`) and texture-name families. `opacity_from: "mask"` means the **R channel** of the mask texture
  (verified on the pine / oak atlases: R is 1 on leaves and trunk, 0 outside); `"base_color"` means the colour texture's alpha.
* Static switch values of **instances** live in the cooked native tail and are not decoded; `static_switches` are the root material's defaults.
* `tint_guess` (grey canopy textures only): normalised `Gradient_TopColor` / `SimpleSSColor`; an approximation, not the graph.

### collision.json
UE local space of the mesh, **cm, left handed, Z up**, rotations are quaternions `(x,y,z,w)`: `shapes[]` of `box{center,rotation,half_extent}`,
`sphere{center,radius}`, `capsule{center,rotation,radius,half_length}`, `convex{vertices[],transform{translation,rotation,scale},bounds_min/max}`;
plus `trace_flag`, `profile` (`BlockAll`), `collision_enabled`, `responses[]`, `bounds_min/max`. Convex hulls are **points only** (faces live in
the cooked PhysX blob, not decoded): rebuild the hull from the vertices. All 1.8 tree / rock bodies are convex hulls.

### layout/
`all.json`: `{classes[], fields: [class,x,y,z,pitch,yaw,roll,sx,sy,sz], instances[[...]]}` (every harvestable placement, UE world cm, `FRotator` degrees).
`sample.json`: 40 nearest placements to a centre chosen as the harvestable maximising `min(#trees, #rocks)` within 40 m; `pos` is relative to
`center_abs`, `abs` absolute. `stats.json`: class histogram, per-level counts, bbox, density, foliage notes.

## Coordinates
* **Meshes** (glTF): metres, right-handed, Y up: `(x, y, z)_UE cm -> (x, z, y) * 0.01` (a reflection: indices keep their facing).
* **Collision / bounds / layout**: UE cm, left-handed, Z up. `ue_to_game_position` applies the same swap + 0.01.
* **Yaw**: the swap flips handedness, so a UE yaw of `a` degrees is a rotation of `-a` radians about +Y in the game frame (`PropInstance::game_yaw`). Pitch / roll of
  placements are kept in `rot_deg` but not converted (tree placements are yaw-only except for a few tilted rocks).
* Mesh origin = placement origin (trees / rocks are pivoted at their base); `scale` is uniform in the level.

## Verification
`cargo test -p fn-propdata` (synthetic catalog + the real export when present: 85 kinds, every LOD / material / texture decodes, the
LOD vertex / triangle counts equal `props.json`, 600 / 450 health, collision hulls vs. hand-inspected `BodySetup`, layout 9609 placements).
`cargo test -p fn-mesh -p fn-export` cover the glTF writer (u16 / u32 indices, multi-LOD, vertex colours, UV sets, tangent sign), the
section / index / bounds validator, the material slot chooser, the Athena placement maths and the contact-sheet rasteriser.
