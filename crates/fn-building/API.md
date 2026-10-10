# fn-building core API (owner: core/placement agent)

Stable surface for the visuals, editing and structural/damage agents. Additive changes only; nothing listed here will be
renamed or removed without updating this file.

Space: UE axes, centimetres, X/Y horizontal, Z up, the same space as `fn_movement::scene`. FX space = this + (0, Z_OFFSET*100, 0)
(`building_hook::world_to_fx`). Yaw in degrees (UE `FRotator::Yaw`). The build grid is anchored on the Athena level origin
(`PlacementEnv::grid_origin`, from the storm/props level mapping), not on 0.

## `fn_building::pieces`
- `enum PieceKind { Wall, Floor, Stairs, Roof }` (Stairs = ramp, Roof = cone), `PieceKind::ALL`, `building_type()`, `item_definition()`.
- `enum Material { Wood, Stone, Metal }`, `Material::ALL`, `Material::next()`, `index()`.
- `PieceCatalog::load(&PakSet)`: per (kind, material) `PieceDef { class: ClassData, max_health, initial_health_pct, build_time, cost }`
  (`start_health()`), `ClassData { class_path, static_mesh, edit_pattern, convexes (local cm), cube (connectivity) }`;
  `class(path)`, `ensure_class(set, path)` for edit variants.
- `initial_health_pct_handles(set) -> [RowHandle; 3]` (added by the editing agent): the `BuildingInitialHealthPercentHandles`
  rows the catalog uses, so `EditDb` computes every edit class's build numbers the same way.

## `fn_building::grid` (1.8 grid: 512 x 512 x 384, cells centred on multiples of 512)
- `SNAP_GRID_SIZE`, `VERT_SNAP_GRID_SIZE`, `Rot`, `geom(kind)` (CentroidOffset, BaseLocToPivotOffset, BuildingPlacementDistance),
  `snap_transform(kind, target, view_rot, rotation_iterations)`, `centroid`, `grid_location`, `key_for(kind, loc, yaw) -> (GridKey, Slot)`.

## `fn_building::placement`
- `BuildPreview` (`set_piece`, `material`, `rotation_iterations`, `mirrored`, `update(world, cat, env, ViewInput) -> QueryResult`, `loc` / `rot`).
- `query(world, cat, def, loc, yaw, mirrored, env) -> QueryResult { CanAdd, ExistingActor(id), Obstructed, NoStructuralSupport, InvalidActor }`.
- `place`, `spawn_piece`, `piece_hulls(cat, piece)` (world-space collision convexes), `voxel` / `connected` (1.8 connectivity cubes).

## `fn_building::world`
- `type PieceId = u32` (never reused).
- `struct GridKey { kind, cell: IVec3, rot: u8 }` grid slot; `rot` = yaw quarter turns 0..=3.
- `struct PlacedPiece { id, kind, material, class_path: String, key, location: Vec3, yaw_deg, health, max_health, build: BuildProgress, edit: EditState, placed_time }`
  - `effective_class()` (edit class or original), `is_building()`, `health_fraction()`.
- `struct BuildProgress { elapsed, duration, start_health, health_to_build, accumulated }`, `fraction()` 0..=1, `done()` (= health_to_build <= 0).
- `struct EditState { class_path: Option<String>, rotation_iterations: i32, mirrored: bool }`.
- `struct PieceSpawn { kind, material, class_path, key, location, yaw_deg, max_health, start_health, build_time }`.
- `enum WorldEvent { Placed(id), Built(id), HealthChanged{id,old,new}, Edited(id), Removed(id) }`.
- `enum DamageOutcome { NoSuchPiece, Damaged{health}, Destroyed }`.

`BuildingWorld`:
| fn | use |
|---|---|
| `new()` | empty world |
| `revision() -> u64` | bumps on every change (cheap dirty check for visuals) |
| `time() -> f64` | accumulated tick time |
| `iter()`, `len()`, `get(id)`, `get_mut(id)` (bumps revision), `touch()` | read pieces |
| `at_key(&GridKey)`, `is_occupied(&GridKey)` | grid lookup |
| `spawn(PieceSpawn) -> PieceId` | add without validation (placement validates first) |
| `remove(id) -> Option<PlacedPiece>` | structural / destruction |
| `set_health(id, h)` | clamps to 0..=max, never removes |
| `apply_damage(id, amount) -> DamageOutcome` | removes at 0 |
| `set_edit(id, EditState)` | editing swaps the class |
| `rebuild_for_edit(id, EditRebuild { max_health, initial_health_pct, build_time, animates })` | (editing agent) an edit's new actor (`vfunc_247` InitReason 2): health = new max * pct * old fraction, back into the build-up path (`tick`) when BuildTime > 0 and the class animates; else full health, built. Called before `set_edit`, so `Edited` sees `is_building()` |
| `tick(dt)` | build-up as `ABuildingSMActor::Tick`: every 0.5 s heal ceil(rate * accumulated), rate = (max - start) / BuildTime |
| `events_since(cursor) -> (&[WorldEvent], next_cursor)` | per-consumer event log, start at cursor 0 |

## App side (`fn-app/src/building_hook.rs`)
The app owns one `BuildingWorld` (static in `building_hook`): `with_world(|w| ..)`, `with_catalog`, `preview()`,
`edit_visuals()`, `drain_edit_sound_cues()`, `world_to_fx` / `sim_to_world`. `frame(sim, build_input, install, dt)` runs once per
frame after the sim step: build intents, edit intents (`build_input::apply_edit_intents` on the same world), preview, placing,
`BuildingWorld::tick`, then `building_damage_hook::tick(&mut world)`, then the collision hulls (`GameSim::building_hulls`).
Request functions: `enter_build_mode`, `exit_build_mode`, `select_piece`, `set_material`, `request_place`, `rotate_preview`.

## `fn_building::edit` (owner: editing agent)
- `EditDb::load(&PakSet)`: player building classes (`PossibleEditClasses` source), their cooked `EMP_*` patterns, tile grids.
- `EditController` (bindable actions): `edit_begin(db, world, target_piece, pawn_loc)` / `edit_begin_checked(.., EditBlockers
  { has_attachment, structural_pending })` (+ `input_mode: EditInputMode` for the gamepad rule), `edit_press` / `edit_release`
  (edit-tool fire), `edit_hover(Option<EditTarget>)`, `edit_select_tile(i)`, `edit_reset`, `edit_confirm(&mut world)`,
  `edit_cancel`, `tick_frame(world, pawn, pawn_dead, camera, view_dir, dt)` (range / death cancel, tile picking, match flash,
  roof tile easing; `tick(..)` = dt 0, alive), `visuals(world)`, `drain_events()` (`EditEvent::PatternChanged` = play the edit montage).
- `EditVisuals` (render snapshot): tile mesh / material paths, per tile local position (roof tiles ease +32 Z while active), scale,
  active, highlighted, `interact: Option<f32>` (the tile MID's `InteractParameter`: hover 1/0 and the match flash; write it when
  `Some`), `BlueColor`, texture; stair indicators (`mode`, `texture`, `scalars` named by `data::material_params::AUX_SCALARS`
  = `sineOffset`, `isblinkingSign`, `isX`; `local_yaw_deg` already includes the +180 of a `reversed` arrow); the preview class
  (`PreviewMaterial`) with its transform.
- Confirm writes the world through `apply_edit`: `rebuild_for_edit` (the new actor rebuilds with the build animation),
  `set_edit(EditState { class_path, rotation_iterations, mirrored })` AND
  `PlacedPiece::location` / `yaw_deg` are updated to the replaced actor's transform (1.8 `ReplaceBuildingActor`
  rotates walls in place and other pieces about `BaseLocToPivotOffset` (0,-256,0)). Renderers should use
  `location` / `yaw_deg` as-is (do not add `rotation_iterations * 90` again) and `edit.mirrored` = X scale -1.
- App side: `fn-app/src/building_edit_hook.rs` (`EditHook`; `tick_frame(world, pawn, pawn_dead, camera, dir, dt)`). It asks
  `building_damage_hook::structural_pending(id)` (added by the editing agent) before starting an edit.

## `fn_building::visual` (owner: visuals agent)
- Pure logic of `ABuildingSMActor`'s look: `PieceVisualDef::from_cdo` (mesh, `AnimatingMaterial`, `DamagedButNotAnimatingMaterial`,
  sub-object counts, effects, level), `PieceAnim` (`EBuildingAnim` placement / building / breaking state machine, MID parameters,
  blueprint ghost, `Surface` = which material), `target_sub_objects`, cue tables, `piece_class(material, level, variant)`.
- App side: `fn-app/src/building_render.rs`. The world owner calls once `building_render::init(install, asset_roots)`, then every
  frame `building_render::frame(renderer.fx(), &world, preview, edit_visuals, &view, dt, &mut fx_frame)` before
  `set_fx_frame`, and forwards the returned `SoundEvent`s to the audio sink. It reads `WorldEvent`s with its own cursor
  (Placed -> placement anim + construction sound, Edited -> new class: placement anim + construction sound when `is_building()` (edits rebuild), else shown built, Removed -> death FX + sound).
  `Preview { class_path, location, yaw_deg, mirrored, valid }` = the placement marker (blue / red).
  `building_render::mesh_for(set, class_path)` = the static mesh of any (edit variant) class.
  `building_render::edit_cue_event(cue)` maps `EditHook::drain_sound_cues()` to `SoundEvent::BuildingCue`.
