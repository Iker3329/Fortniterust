//! VISUALS (owner: visuals agent): what a placed Athena piece looks like and sounds like over its life, as `ABuildingSMActor` (1.8 exe) drives it.
//! Pure logic, no GPU: the renderer (`fn-app/src/building_render.rs`) turns [`PieceAnim`] into material-parameter values of the ORIGINAL cooked materials.
//!
//! RECOVERED (1.8 exe, `C:\FortniteRef\decomp\named`; addresses are the exe's):
//! * The build-up is NOT a mesh scale: it is the world-position-offset animation of the piece's `AnimatingMaterial` (e.g. `Wood_lvl1_Wall`, parent
//!   `Builder_2`), driven per actor through `UMaterialInstanceDynamic` scalar parameters (the exe's static FNames, initialised @1400997a0..140099ae0):
//!   `BuildAnimPhase` (DAT_14521f4a0), `TotalNumberOfSubObjects` (f498), `BreakAnim` (f4a8), `TargetBreakNumber` (f4b0), `Code_totalDestructionAmount` (f4c0),
//!   `Code_FloorBeingDamaged` (f4d8). The mesh's sub-objects (planks / bricks / panels) fly in one by one as `BuildAnimPhase` grows.
//! * `EBuildingAnim` state machine (`CurBuildingAnimType` +0xc7b, `CurAnimSubObjectNum` +0xc7c, `CurBuildingAnimStartTime` +0xc80, `CurAnimSubObjectTargetNum` +0xc84):
//!   - start `vfunc_397` @140463170: records the start time, target = [`target_sub_objects`] (`vfunc_411` @140464530), creates the MIDs (`vfunc_318` @14048e730:
//!     `TotalNumberOfSubObjects` = `BuildingAnimatingSubObjects`, `Code_FloorBeingDamaged` = 0, `BreakAnim` = 0, `BuildAnimPhase` = `AnimatingSubObjects`), then
//!     `EBA_Building` `vfunc_400` @14047c790, `EBA_Breaking` `vfunc_401` @14047c6a0, `EBA_Destruction` `vfunc_402` @14047c840, `EBA_Placement` `vfunc_403` @14047d000.
//!   - ticks: building `vfunc_407` @140497270, breaking `vfunc_409` @140497080, placement `vfunc_408` @140497860; when one ends `vfunc_398` @140468ac0 re-dispatches:
//!     target > current -> Building, target < current -> Breaking (target >= `NumFrameSubObjects`) or Destruction.
//! * Durations: `UFortGameData::GetBuildingPlacementAnimDuration` / `GetBuildingPieceBreakAnimDuration` (curve rows `BuildingPlaceAnimDuration_*` 0.35 s,
//!   `BuildingBreakAnimDuration_*` 1.0 s of `/Game/Balance/DataTables/DefaultGameData`, the handles of `DefaultGameData`); build speed `vfunc_368` @14046c050 =
//!   (`AnimatingSubObjects` - `NumFrameSubObjects`) / `BuildTime` (`vfunc_371`: the `FortBuildingActorSet.BuildTime` attribute; `FLT_MAX` when it is 0).
//! * Material in use (`vfunc_320` @1404989e0, `FUN_14046d7a0`): animating -> `AnimatingMaterial` MIDs; health < max and not animating -> the MIDs are rebuilt
//!   from `DamagedButNotAnimatingMaterial` (parameters copied); back at full health -> the MIDs are cleared (`bClearMIDWhenReturningToUndamagedState`, set in
//!   the ctor @14045b960: `|= 0x8000` of +0x780, no player piece overrides it) and the mesh shows its own material again.
//! * Blueprint ghost (`vfunc_387` @14048ec40, `vfunc_390` @14048cb40, `vfunc_388` @140466a80): while animating a second static-mesh component with the same
//!   mesh shows a MID of `FortGameData.BlueprintParentMaterial` (`PlayerBuiltWall_BluePrintShader`, the `UFortGameData` ctor table @140573b50) on slot 0, its
//!   `Blueprint_*` parameters copied from the `AnimatingMaterial` (`ExtractBlueprintParamValuesFromAnimMat` @14046a0d0); it is destroyed when the build completes.
//!   During placement `code_JonsDanceTime` = `Interact` = (1 - |t - 0.5| * 2) * 2.5 (`vfunc_408`, `DAT_144078d78` = 2.5).
//! * Sounds: placement starts `FortGameData.BuildingConstructionSounds[resource].Sounds[level]` attached to the actor (`vfunc_403` + `FUN_140586210`), faded out over
//!   0.5 s by the Tick once the under-construction bit is clear (BVISFIX), stopped at the latest by `EndPlay` @140469ac0; completion plays `ConstructionCompleteSound` = `Fort_Build_Done_Cue` at the actor (`vfunc_407`, game data +0x1a40);
//!   death plays `DeathSound`, else `PlayerBuiltBuildingDeathSounds` (player built), else `GenericBuildingDeathSounds` at the death location (`vfunc_316` @140485ab0).
//! * Effects: death = `DeathParticles` (fallback `vfunc_0xa30`) at the `DeathParticleSocketName` socket, rotation + scale of the mesh component (`vfunc_316`);
//!   construction finished = `ConstructedEffect` (fallback `FortGameData.FallbackConstructedEffect` = `P_PBW_W_Sparkle_Solid`) at the actor (`vfunc_317` @1404863e0,
//!   called from the tick `vfunc_323` at the finished-highlight peak, ~0.32 s after completion). Death effects / sound at `GetCentroid` @14046ced0 (actor
//!   rotation * `CentroidOffset` + location, [`centroid_offset`]) unless the mesh has the `DeathParticleSocketName` socket (`vfunc_316` @140485ab0).
//! * Placement preview (`AFortPlayerController::BuildPreviewMarker`, `BuildPreviewMarkerParentMaterial` = `PlayerBuiltWall_BluePrintShader`, controller ctor
//!   FUN_140810f30): `Code_Color` = the MID's own `code_Blue` when the placement is valid, `Code_Red` otherwise (FUN_140831f80 / FUN_140831b60).
//!
//! BVISFIX (2026-10-05, `docs/spec/building_visual_gaps.md`, every address re-checked in the decompile):
//! * Level: `ABuildingActor::CurrentBuildingLevel` (+0x498, vtable +0x610 = `mov eax,[rcx+0x498]` @14046d760) read from the class CDO (absent = 0), not the folder.
//! * Damage-visuals state (`vfunc_320` @1404989e0): state 1 (AnimatingMaterial) while `vfunc_272` @140491e30 holds = an animation runs, the piece is UNDER
//!   CONSTRUCTION (bit 0x40 of +0xE90, set by `vfunc_389` @14048d2d0 while `HealthToAutoBuild` > 0), or `vfunc_324` @14047f9c0 (hit within 5 s: `DAT_143fe3710`
//!   = 5.0; finished highlight running) via `vfunc_211` @14047ed90. The damaged material only shows once construction is over and health < max.
//! * Per-frame MID drive `vfunc_323` @140499690 (run by `ABuildingSMActor::Tick` @1404967f0 while `vfunc_324`): `Code_HitAnimation` from the bounce curve,
//!   `Code_CustomDamagePercentage`, the overlay's `Code_DestructionAmount`, `Emissive_Color_Add` = `BuildingFullHealthFlashColor` * `FullHealthAnimCurve`, the
//!   `ConstructedEffect` at the flash peak (flag +0xF90), the blueprint's `Code_Emissive` from `BlueprintDamageCurve`. Hit bounce `vfunc_313` @140493170 (from the
//!   damage path `vfunc_276` @140486ae0 -> `vfunc_314` @14048fb20, and at completion `vfunc_393` @1404859a0 / roof @140485900), highlight start `vfunc_391`
//!   @1404937c0, damage overlay `vfunc_322` @14048ef60 (only when the class has NO AnimatingMaterial and `bNeedsDamageOverlay`, `vfunc_321` @14048b5a0: never for
//!   the player pieces), destruction `vfunc_402` @14047c840 + tick `vfunc_410` @140497610 (started by OnDeath `vfunc_277` @140486330; ends hidden).
//! * Curves (`UFortGameData` ctor @1405751d0 copies the path table @1466222e0 built by FUN_140573b50): BounceCurve +0x1A80 = `HitEffectBounceAnimation`,
//!   EditCurve +0x1AC0 = `/Game/GameplayCurves/NewlyFinishedWallBounceMagnitudeCurve` (used by `vfunc_325` @14046c010 when the bounce mode is 2 = the completion
//!   bounce, NOT while editing as the spec said), BlueprintDamageCurve +0x1AE0 = `RedImpactCurve`, FullHealthAnimCurve +0x1B00 = `FinishedHighlight`,
//!   DamageMaterial +0x1A60 = `Translucent_hitEffects`.
//! ASSUMED (named): a health increase while a piece is not animating re-dispatches through `vfunc_398` like the end of an animation does (the caller of
//! `vfunc_398` on health change is not named in the decompile); the anim-start window of `vfunc_211` (vtable +0xc58, not resolved) is not counted; the hit
//! location is the damage HitResult's ImpactPoint (the decompile of `vfunc_276` reuses that stack slot for the attacker direction, see `PieceAnim::on_damage`).
//! NBOUNCE (2026-10-05, neighbour bounce + destruction scale, every address re-checked in the decompile):
//! * `vfunc_314` @14048fb20 = (Damage, Location, Direction, Radius, bBounce, bPropagate); `vfunc_276` passes (Damage, the hit location, the attacker
//!   direction, clamp(Bounds.SphereRadius, 0, 300), 1, 1). With bBounce: `DamageAmountStart` = health + damage, the direction is projected for the actor's
//!   own class (IsA `ABuildingFloor` -> Z = 0, which includes `ABuildingStairs`: its ctor @14049f370 runs the floor ctor; IsA `ABuildingWall` -> the
//!   components where |RightVector_i| > 1e-4 (`DAT_143ff9010`) zeroed, then normalised). Then, when bPropagate AND `bPropagatesBounceEffects` (+0x920,
//!   SDK `ABuildingSMActor`; `ABuildingSMActor` ctor @14045b960 stores 1, no wall / floor / stairs / roof / pillar ctor and none of the 329 cooked `PBWA_*`
//!   CDOs overrides it, so TRUE for every player piece): `UWorld::OverlapMultiByChannel` (trace tag `WallBounceOverlap`, ignore self, sphere of the same
//!   radius at the hit location, identity rotation, channel 0x18 = ECC_GameTraceChannel11 = `FCC_Trace_World`, CookedIniVersion.txt L1383, which the
//!   pieces' `FortBuildingMesh` profile blocks, L1353). Every overlapped `ABuildingSMActor` is collected (one entry per overlapping component, no dedup) and
//!   edits the ONE shared direction: IsA floor -> Z = 0; IsA wall -> the components where |RightVector_i| of the DAMAGED actor's root (not the
//!   neighbour's: both root loads @14048fc87 / @14049002d read `[rdi+0x168]`, rdi = this from the prologue @14048fb73) exceeds 1e-8 (`DAT_143ff9008`)
//!   zeroed, NOT renormalised. Then a zero direction becomes (Direction.x, Direction.y, 0), it is normalised, and `vfunc_313` runs on the damaged actor and
//!   then on every collected actor with the same (time, radius, direction, location, mode 0) (the instigator argument is the damaged actor). No recursion
//!   through `vfunc_314`; `vfunc_313` itself forwards to `AttachedBuildingActors` (+0xD60, SDK) that are not pending kill (trap / attachment actors: the
//!   port has none, so that forward is empty). See [`PieceAnim::on_damage`], [`PieceAnim::on_neighbour_bounce`], [`sphere_overlaps_hulls`].
//! * Destruction scale (`vfunc_402` @14047c840): `LastDamageAmount` (+0x99C) is written ONLY by `vfunc_276` (= its Damage argument), which
//!   `ABuildingActor::GameplayCue_Damage` @14046b570 calls with the cue's `RawMagnitude` for a non-fatal hit. The damage GEs (`GE_Melee_GenericDamage` /
//!   `GE_Ranged_GenericDamage` -> `GET_DirectPhysicalDamage`) carry `GameplayCue.Damage.Physical` with `MagnitudeAttribute` = `FortHealthSet.Damage`, so
//!   RawMagnitude = that modifier's TotalMagnitude (UE4.16 GameplayEffect.cpp:2461, AbilitySystemComponent.cpp:992): the damage dealt, before the health
//!   clamp (`UFortHealthSet::PostGameplayEffectExecute` @140445130 subtracts it from Health afterwards). When Health ends <= 0 that function sets the context's
//!   `bIsFatalHit` (+0x68 of `FFortGameplayEffectContext`, SDK) and `GameplayCue_Damage` then calls OnDeath `vfunc_277` INSTEAD of `vfunc_276`: the killing
//!   hit never reaches `LastDamageAmount` / `LastDamageHit`, the bounce or the blueprint. So the scale is `LastDamageAmount / MaxHealth` of the last
//!   NON-fatal hit (0 when there was none), and for a non-fatal hit damage dealt == health lost; [`PieceAnim::on_damage`] ignores a hit that leaves 0 health.
//! NOT PORTED: the BP event `OnBounceAnimationUpdate`; the neighbour bounce of non-player `ABuildingSMActor`s (trees / rocks are `fn-props`, not in the
//! building world) and of pieces already playing their destruction (ASSUMED: excluded, the dying actor's collision after OnDeath is not checked).
use crate::pieces::{Material, PieceKind};
use serde_json::Value;

/// Material parameter names the exe sets (its static FNames, see the module doc).
pub mod param {
    pub const BUILD_ANIM_PHASE: &str = "BuildAnimPhase";
    pub const TOTAL_SUB_OBJECTS: &str = "TotalNumberOfSubObjects";
    pub const BREAK_ANIM: &str = "BreakAnim";
    pub const TARGET_BREAK_NUMBER: &str = "TargetBreakNumber";
    pub const TOTAL_DESTRUCTION: &str = "Code_totalDestructionAmount";
    pub const FLOOR_BEING_DAMAGED: &str = "Code_FloorBeingDamaged";
    /// Blueprint ghost MID (DAT_14521f470 / f478).
    pub const JONS_DANCE_TIME: &str = "code_JonsDanceTime";
    pub const INTERACT: &str = "Interact";
    /// Copied from the animating material to the blueprint MID (DAT_14521f448 .. f468, f490).
    pub const BLUEPRINT_TEXTURE: &str = "Blueprint_Outline_Texture";
    pub const BLUEPRINT_CHANNELS: [&str; 4] = ["Blueprint_Outline_Texture_Channel", "Blueprint_Outline_Texture_Channel2", "Blueprint_Outline_Texture_Channel3", "Blueprint_Outline_Texture_Channel4"];
    pub const BLUEPRINT_GRID_MULTIPLIER: &str = "Blueprint_GridMultiplier";
    /// Preview marker colour (DAT_14521f430 / f438 / f440).
    pub const CODE_COLOR: &str = "Code_Color";
    pub const CODE_RED: &str = "Code_Red";
    pub const CODE_BLUE: &str = "code_Blue";
    /// BVISFIX: the per-frame MID drive (exe static FNames, initialiser @1400996fd..140099b4d; `code_opacity` = DAT_14521f3a8, read from the
    /// `lea rdx,L"code_opacity"; lea rcx,DAT_14521f3a8` pair @0x140099a2d of the exe).
    pub const HIT_ANIMATION: &str = "Code_HitAnimation";
    pub const DESTRUCTION_AMOUNT: &str = "Code_DestructionAmount";
    pub const CUSTOM_DAMAGE_PCT: &str = "Code_CustomDamagePercentage";
    pub const INVERT_DAMAGE_FALLOFF: &str = "InvertDamageFalloff";
    pub const DEFORMATION_CENTER: &str = "WSDeformationCenter";
    pub const DEFORMATION_VECTOR: &str = "Code_DeformationVector";
    pub const DAMAGE_RADIUS: &str = "WSDamageRadius";
    pub const EMISSIVE_COLOR_ADD: &str = "Emissive_Color_Add";
    pub const CODE_EMISSIVE: &str = "Code_Emissive";
    pub const EXPLOSION_VECTOR: &str = "CustomExplosionVectorForExplosion";
    pub const OUTWARD_MOTION: &str = "outwardMotion(WS)";
    pub const BREAK_ANIMATION: &str = "code_BreakAnimation";
    pub const CODE_OPACITY: &str = "code_opacity";
}

/// BVISFIX: `UFortGameData` assets of the per-frame drive (see the module doc for the field <- path mapping).
pub const BOUNCE_CURVE: &str = "/Game/Packages/Fortress_SharedMaterials/Base_Material_Functions/HitEffectBounceAnimation";
pub const EDIT_CURVE: &str = "/Game/GameplayCurves/NewlyFinishedWallBounceMagnitudeCurve";
pub const BLUEPRINT_DAMAGE_CURVE: &str = "/Game/GameplayCurves/RedImpactCurve";
pub const FULL_HEALTH_ANIM_CURVE: &str = "/Game/Building/CurveEffects/FinishedHighlight";
pub const DAMAGE_MATERIAL: &str = "/Game/Packages/jl_tests/Translucent_hitEffects.Translucent_hitEffects";
/// `DefaultGameData.BuildingFullHealthFlashColor` (/Game/Balance/DefaultGameData CDO).
pub const FULL_HEALTH_FLASH_COLOR: [f32; 4] = [1.67, 0.36, 0.06, 1.0];
/// Rows `BuildingDestroyAnimDuration_{Wood,Stone,Metal}` (all keys 1.5).
pub const DESTROY_ANIM_DURATION: [f32; 3] = [1.5, 1.5, 1.5];
/// `DAT_143fe3710`: the hit window of `vfunc_324` and the completion-bounce radius factor of `vfunc_393`.
pub const HIT_WINDOW: f32 = 5.0;
/// `DAT_143ff9040`: cap of the damage-bounce radius (`vfunc_276`: clamp(Bounds.SphereRadius, 0, 300)).
pub const HIT_RADIUS_CAP: f32 = 300.0;
/// `DAT_143ff9010`: the "axis component is zero" tolerance of `vfunc_314`'s wall direction projection.
const AXIS_EPS: f32 = 1e-4;

/// `ABuildingActor::CentroidOffset` (+0x6EC) of the class ctors (BVISFIX, section E): wall @14049fe20 (0, 0, 192), floor @140459d10 (0, 256, 0), stairs
/// @14049f370 (0, 256, 192), roof @14045b660 (0, 256, 0) (`DAT_14401ee38` = 192, `DAT_143ff903c` = 256); no player CDO overrides it.
pub fn centroid_offset(kind: PieceKind) -> glam::Vec3 {
    match kind { PieceKind::Wall => glam::Vec3::new(0., 0., 192.), PieceKind::Floor | PieceKind::Roof => glam::Vec3::new(0., 256., 0.), PieceKind::Stairs => glam::Vec3::new(0., 256., 192.) }
}

/// A UE4.16 `FRichCurve` (`UCurveFloat::FloatCurve`) with `UCurveBase::GetTimeRange` (CurveBase.cpp:16, RichCurve.cpp:546) and `FRichCurve::Eval`
/// (the evaluator of `edit::data::FlashCurve`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RichCurve(pub crate::edit::data::FlashCurve);
impl RichCurve {
    pub fn eval(&self, t: f32) -> f32 { self.0.eval(t) }
    /// (min, max) key time; (0, 0) without keys.
    pub fn time_range(&self) -> (f32, f32) { match (self.0.keys.first(), self.0.keys.last()) { (Some(a), Some(b)) => (a.0, b.0), _ => (0., 0.) } }
    pub fn from_keys(keys: Vec<(f32, f32, u8, f32, f32)>) -> RichCurve { RichCurve(crate::edit::data::FlashCurve { keys }) }
}
/// `UCurveVector` (three `FRichCurve`s; `GetTimeRange` = min / max over the three).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VectorCurve(pub [RichCurve; 3]);
impl VectorCurve {
    pub fn eval(&self, t: f32) -> [f32; 3] { [self.0[0].eval(t), self.0[1].eval(t), self.0[2].eval(t)] }
    pub fn time_range(&self) -> (f32, f32) {
        let (mut lo, mut hi) = self.0[0].time_range();
        for c in &self.0[1..] { let (a, b) = c.time_range(); lo = lo.min(a); hi = hi.max(b); }
        (lo, hi)
    }
}

fn read_rich_curve(v: &fn_uasset::Value) -> Option<RichCurve> {
    let keys = v.field("Keys")?.as_array()?;
    let interp = |v: Option<&fn_uasset::Value>| -> u8 {
        match v {
            Some(fn_uasset::Value::Byte(b)) => *b,
            Some(fn_uasset::Value::Enum { value, .. }) | Some(fn_uasset::Value::Name(value)) => match value.rsplit("::").next().unwrap_or(value) { "RCIM_Constant" => 1, "RCIM_Cubic" => 2, "RCIM_None" => 3, _ => 0 },
            Some(v) => v.as_i64().unwrap_or(0) as u8,
            None => 0,
        }
    };
    let mut out: Vec<(f32, f32, u8, f32, f32)> = keys.iter().filter_map(|k| Some((k.field("Time")?.as_f32()?, k.field("Value")?.as_f32()?, interp(k.field("InterpMode")),
        k.field("ArriveTangent").and_then(|v| v.as_f32()).unwrap_or(0.0), k.field("LeaveTangent").and_then(|v| v.as_f32()).unwrap_or(0.0)))).collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(RichCurve::from_keys(out))
}
/// The tagged properties (name, static-array index, value) of the first `class` export of a curve package.
fn curve_props(set: &fn_pak::PakSet, path: &str, class: &str) -> Result<Vec<(String, u32, fn_uasset::Value)>, String> {
    let data = fn_uasset::PackageData::load(set, path).map_err(|e| format!("{path}: {e}"))?;
    let pkg = data.package().map_err(|e| format!("{path}: {e}"))?;
    let i = pkg.exports_of_class(class).next().ok_or_else(|| format!("{path}: no {class} export"))?;
    let obj = pkg.read_export(i).map_err(|e| format!("{path}: {e}"))?;
    Ok(obj.props.iter().map(|p| (p.name.clone(), p.index, p.value.clone())).collect())
}
/// A `CurveFloat` package's `FloatCurve`.
pub fn load_float_curve(set: &fn_pak::PakSet, path: &str) -> Result<RichCurve, String> {
    let props = curve_props(set, path, "CurveFloat")?;
    props.iter().find(|p| p.0 == "FloatCurve" && p.1 == 0).and_then(|p| read_rich_curve(&p.2)).ok_or_else(|| format!("{path}: no FloatCurve.Keys"))
}
/// A `CurveVector` package's `FloatCurves[0..3]` (static array; a missing element = no keys).
pub fn load_vector_curve(set: &fn_pak::PakSet, path: &str) -> Result<VectorCurve, String> {
    let props = curve_props(set, path, "CurveVector")?;
    let c = |k: u32| props.iter().find(|p| p.0 == "FloatCurves" && p.1 == k).and_then(|p| read_rich_curve(&p.2)).unwrap_or_default();
    Ok(VectorCurve([c(0), c(1), c(2)]))
}

/// BVISFIX: the game data the per-frame MID drive reads (`UFortGameData`; a missing curve behaves as in the exe: no curve = that step is skipped).
#[derive(Clone, Debug, PartialEq)]
pub struct GameDataFx {
    pub bounce: Option<RichCurve>,
    pub edit_bounce: Option<RichCurve>,
    pub full_health: Option<RichCurve>,
    pub blueprint_damage: Option<VectorCurve>,
    pub flash_color: [f32; 4],
    /// `GetBuildingDmgPctForCritDestructionAnim` / `GetBuildingCritDmgAnimScaleCap` per [resource][level] (rows `BuildingCritDamagePct_*`,
    /// `BuildingCritDestroyAnimScaleCap_*` of `/Game/Balance/DataTables/DefaultGameData`, handles at game data +0x6E0 / +0x720).
    pub crit_pct: [[f32; 3]; 3],
    pub crit_cap: [[f32; 3]; 3],
    pub destroy_duration: [f32; 3],
}
impl Default for GameDataFx {
    fn default() -> Self { GameDataFx { bounce: None, edit_bounce: None, full_health: None, blueprint_damage: None, flash_color: FULL_HEALTH_FLASH_COLOR, crit_pct: [[1.0; 3]; 3], crit_cap: [[1.0; 3]; 3], destroy_duration: DESTROY_ANIM_DURATION } }
}
impl GameDataFx {
    /// Loads the curves (errors are returned per asset so the caller can log them; the value then stays `None`).
    pub fn load(set: &fn_pak::PakSet) -> (GameDataFx, Vec<String>) {
        let mut errs = vec![];
        let mut f = GameDataFx::default();
        let mut fl = |p: &str| match load_float_curve(set, p) { Ok(c) => Some(c), Err(e) => { errs.push(e); None } };
        f.bounce = fl(BOUNCE_CURVE);
        f.edit_bounce = fl(EDIT_CURVE);
        f.full_health = fl(FULL_HEALTH_ANIM_CURVE);
        match load_vector_curve(set, BLUEPRINT_DAMAGE_CURVE) { Ok(c) => f.blueprint_damage = Some(c), Err(e) => errs.push(e) }
        match crate::pieces::CurveTable::load(set, "/Game/Balance/DataTables/DefaultGameData") {
            Ok(t) => for (r, m) in ["Wood", "Stone", "Metal"].iter().enumerate() {
                for lvl in 0..3 {
                    // FCurveTableRowHandle::Eval at the building level (vtable +0x610, 0-based) as `vfunc_402` passes it
                    match t.eval(&format!("BuildingCritDamagePct_{m}"), lvl as f32) { Ok(v) => f.crit_pct[r][lvl] = v, Err(e) => errs.push(e) }
                    match t.eval(&format!("BuildingCritDestroyAnimScaleCap_{m}"), lvl as f32) { Ok(v) => f.crit_cap[r][lvl] = v, Err(e) => errs.push(e) }
                    if lvl == 0 { match t.eval(&format!("BuildingDestroyAnimDuration_{m}"), 0.) { Ok(v) => f.destroy_duration[r] = v, Err(e) => errs.push(e) } }
                }
            },
            Err(e) => errs.push(e),
        }
        (f, errs)
    }
}

/// `FortGameData.BlueprintParentMaterial` / `AFortPlayerController.BuildPreviewMarkerParentMaterial` (exe ctor string tables @140573b50, FUN_140810f30).
pub const BLUEPRINT_MATERIAL: &str = "/Game/Packages/PBW/Blueprint/PlayerBuiltWall_BluePrintShader.PlayerBuiltWall_BluePrintShader";
/// `FortGameData.ConstructionCompleteSound`.
pub const CONSTRUCTION_COMPLETE_CUE: &str = "/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_Done_Cue";
/// `FortGameData.FallbackConstructedEffect` / `WallFallbackDeathParticles` / `FallbackDeathParticles`.
pub const FALLBACK_CONSTRUCTED_EFFECT: &str = "/Game/Effects/Fort_Effects/PBW/Build_Sparkle/P_PBW_W_Sparkle_Solid";
pub const WALL_FALLBACK_DEATH_PARTICLES: &str = "/Game/Effects/Fort_Effects/PBW/Destruction/General/P_General_Wall_Destruction";
pub const FALLBACK_DEATH_PARTICLES: &str = "/Game/Effects/Fort_Effects/World/Destruction/Base_Effects/P_General_Deco_Destruction";

/// `DefaultGameData.BuildingConstructionSounds[resource].Sounds[level]` (/Game/Balance/DefaultGameData, Wood / Stone / Metal x level 0..2).
pub const CONSTRUCTION_CUES: [[&str; 3]; 3] = [
    ["/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartWood_Lev1_Cue", "/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartWood_Lev2_Cue", "/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartWood_Lev3_Cue"],
    ["/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartStone_Lev1_Cue", "/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartStone_Lev2_Cue", "/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartStone_Lev3_Cue"],
    ["/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartMetal_Lev1_Cue", "/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartMetal_Lev2_Cue", "/Game/Sounds/Fort_UI_Sounds/Gameplay/Fort_Build_StartMetal_Lev3_Cue"],
];
/// `DefaultGameData.PlayerBuiltBuildingDeathSounds[resource]` (the same cue for the three levels).
pub const PLAYER_BUILT_DEATH_CUES: [&str; 3] = [
    "/Game/Sounds/Fort_Impact_Sounds/Demolition/Defaults/Demo_WoodPlayer_ImpactFinal_Cue",
    "/Game/Sounds/Fort_Impact_Sounds/Demolition/Defaults/Demo_RockPlayer_ImpactFinal_Cue",
    "/Game/Sounds/Fort_Impact_Sounds/Demolition/Defaults/Demo_Metal_ImpactFinal_Cue",
];
/// `/Game/Balance/DataTables/DefaultGameData` rows `BuildingPlaceAnimDuration_{Wood,Stone,Metal}` (all keys 0.35).
pub const PLACEMENT_ANIM_DURATION: [f32; 3] = [0.35, 0.35, 0.35];
/// Rows `BuildingBreakAnimDuration_{Wood,Stone,Metal}` (all keys 1.0).
pub const BREAK_ANIM_DURATION: [f32; 3] = [1.0, 1.0, 1.0];
/// `DAT_144078d78` (the placement pulse amplitude of the blueprint MID, `vfunc_408`).
pub const BLUEPRINT_PULSE: f32 = 2.5;
/// `DAT_143ff9008`, the engine's `SMALL_NUMBER` (the "anim finished" tests).
const SMALL: f32 = 1e-8;

/// `Code_Red` / `code_Blue` effective values of [`BLUEPRINT_MATERIAL`] (`/Game/Packages/PBW/Blueprint/PlayerBuiltWall_BluePrintShader`, a MIC whose parent is
/// `/Game/Packages/Fortnite_UniqueMaterials_1/Walls/PlayerBuiltWall_BluePrintShader`; BVISFIX: the doc named the parent); the renderer reads them from the package, these are for tests and tools).
pub const CODE_RED_DEFAULT: [f32; 4] = [0.646_385, 0.072_977_65, 0.033_549_33, 0.237_56];
pub const CODE_BLUE_DEFAULT: [f32; 4] = [0.000_014, 0.059_048, 0.392_251, 1.0];

/// Index of a tier in the per-resource tables (`EFortResourceType`: Wood 0, Stone 1, Metal 2).
pub fn resource_index(m: Material) -> usize { match m { Material::Wood => 0, Material::Stone => 1, Material::Metal => 2 } }

/// The visual data of a building class, read from its CDO (`fn-inspect props <class package> cdo`; the JSON of `fn_uasset` `to_json`).
#[derive(Clone, Debug, PartialEq)]
pub struct PieceVisualDef {
    pub class_path: String,
    pub static_mesh: String,
    pub animating_material: Option<String>,
    pub damaged_material: Option<String>,
    /// `AnimatingSubObjects` (float in the class).
    pub animating_sub_objects: f32,
    /// `NumFrameSubObjects`.
    pub num_frame_sub_objects: i32,
    pub constructed_effect: Option<String>,
    pub death_particles: Option<String>,
    pub death_particle_socket: Option<String>,
    pub death_sound: Option<String>,
    pub resource: Material,
    /// 0-based building level: the CDO's `CurrentBuildingLevel` (+0x498; absent = 0), see the module doc.
    pub level: usize,
    pub edit_mode_pattern: Option<String>,
    /// `bNeedsDamageOverlay` (+0x95C, read by `vfunc_321` @14048b5a0).
    pub needs_damage_overlay: bool,
    /// NBOUNCE: `bPropagatesBounceEffects` (+0x920, the `WallBounceOverlap` gate of `vfunc_314`; ctor @14045b960 = 1).
    pub propagates_bounce: bool,
}

fn s(v: &Value, k: &str) -> Option<String> { v.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty()).map(|x| x.to_owned()) }

/// 0-based level from a player class path (`.../Player/Wood/L2/PBWA_W2_Floor.PBWA_W2_Floor_C` -> 1). BVISFIX: no longer the level source (the CDO's
/// `CurrentBuildingLevel` is, see [`PieceVisualDef::from_cdo`]); kept for the test that the two agree on the player folders.
pub fn level_of_class(class_path: &str) -> usize {
    class_path.split('/').find_map(|seg| seg.strip_prefix('L').and_then(|n| n.parse::<usize>().ok())).map(|n| n.saturating_sub(1).min(2)).unwrap_or(0)
}

impl PieceVisualDef {
    /// `props` = the CDO's `properties` object. Fields not on the CDO keep the native defaults of `ABuildingSMActor` (0 / null).
    pub fn from_cdo(class_path: &str, props: &Value) -> Result<PieceVisualDef, String> {
        let static_mesh = s(props, "StaticMesh").ok_or_else(|| format!("{class_path}: CDO has no StaticMesh"))?;
        let resource = match props.get("ResourceType").and_then(|v| v.as_str()).map(|x| x.rsplit("::").next().unwrap_or(x)) {
            Some("Stone") => Material::Stone, Some("Metal") => Material::Metal, _ => Material::Wood,
        };
        Ok(PieceVisualDef {
            class_path: class_path.to_owned(), static_mesh,
            animating_material: s(props, "AnimatingMaterial"), damaged_material: s(props, "DamagedButNotAnimatingMaterial"),
            animating_sub_objects: props.get("AnimatingSubObjects").and_then(|v| v.as_f64()).unwrap_or(0.) as f32,
            num_frame_sub_objects: props.get("NumFrameSubObjects").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
            constructed_effect: s(props, "ConstructedEffect"), death_particles: s(props, "DeathParticles"), death_particle_socket: s(props, "DeathParticleSocketName"),
            death_sound: s(props, "DeathSound"), resource,
            // BVISFIX: `ABuildingActor::CurrentBuildingLevel` (+0x498, read by vtable +0x610 @14046d760); unserialized on the L1 CDOs = the native 0
            level: props.get("CurrentBuildingLevel").and_then(|v| v.as_i64()).unwrap_or(0).clamp(0, 2) as usize,
            edit_mode_pattern: s(props, "EditModePatternData"),
            // native default 1 (ABuildingSMActor ctor @14045b960: `+0x95c = 1`)
            needs_damage_overlay: props.get("bNeedsDamageOverlay").and_then(|v| v.as_bool()).unwrap_or(true),
            // NBOUNCE: native default 1 (ABuildingSMActor ctor @14045b960: `+0x920 = 1`); no PBWA CDO serializes it
            propagates_bounce: props.get("bPropagatesBounceEffects").and_then(|v| v.as_bool()).unwrap_or(true),
        })
    }
    /// The cue started at placement (`BuildingConstructionSounds[resource].Sounds[level]`).
    pub fn construction_cue(&self) -> &'static str { CONSTRUCTION_CUES[resource_index(self.resource)][self.level.min(2)] }
    /// The death cue (`vfunc_316`: own `DeathSound`, else the player-built table; every player piece is player built).
    pub fn death_cue(&self) -> String { self.death_sound.clone().map(|c| c.split('.').next().unwrap_or(&c).to_owned()).unwrap_or_else(|| PLAYER_BUILT_DEATH_CUES[resource_index(self.resource)].to_owned()) }
    /// `ConstructedEffect` or the game-data fallback.
    pub fn constructed_effect_or_fallback(&self) -> String { self.constructed_effect.as_deref().map(|p| p.split('.').next().unwrap_or(p).to_owned()).unwrap_or_else(|| FALLBACK_CONSTRUCTED_EFFECT.to_owned()) }
    /// `DeathParticles` or the game-data fallback (walls use the wall fallback; ASSUMED from the field name, `vfunc_0xa30` not read).
    pub fn death_particles_or_fallback(&self, kind: PieceKind) -> String {
        self.death_particles.as_deref().map(|p| p.split('.').next().unwrap_or(p).to_owned()).unwrap_or_else(|| (if kind == PieceKind::Wall { WALL_FALLBACK_DEATH_PARTICLES } else { FALLBACK_DEATH_PARTICLES }).to_owned())
    }
}

/// `vfunc_411` @140464530: `CeilToInt(NumFrame + (Animating - NumFrame) * clamp(health / max, 0, 1))`, 0 when health <= 0.
pub fn target_sub_objects(def: &PieceVisualDef, health: f32, max_health: f32) -> i32 {
    if health <= 0. { return 0; }
    let frac = if max_health == 0. { 0. } else { (health / max_health).clamp(0., 1.) };
    let nf = def.num_frame_sub_objects as f32;
    ((def.animating_sub_objects - nf) * frac + nf).ceil() as i32
}

/// `FMath::FInterpConstantTo` (FUN_1412db8b0).
pub fn finterp_constant_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    let dist = target - current;
    if dist * dist < SMALL { return target; }
    let step = speed * dt;
    current + dist.clamp(-step, step)
}

/// `EBuildingAnim`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimKind { None, Building, Breaking, Destruction, Placement }

/// `EFortDamageVisualsState` (`vfunc_320`): which material the piece's mesh shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    /// The static mesh's own material (no MID).
    Base,
    /// `AnimatingMaterial` MID with [`MidParams`].
    Animating,
    /// `DamagedButNotAnimatingMaterial` MID with [`MidParams`].
    DamagedStatic,
}

/// The parameters the actor has written to its animating MIDs (`None` = never written: the material default applies).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MidParams {
    pub build_anim_phase: Option<f32>,
    pub total_sub_objects: Option<f32>,
    pub break_anim: Option<f32>,
    pub target_break_number: Option<f32>,
    pub total_destruction: Option<f32>,
    pub floor_being_damaged: Option<f32>,
    /// BVISFIX: the per-frame drive / hit bounce / destruction parameters (`vfunc_313`, `vfunc_323`, `vfunc_391`, `vfunc_402`).
    pub hit_animation: Option<f32>,
    pub custom_damage_pct: Option<f32>,
    pub damage_radius: Option<f32>,
    pub invert_damage_falloff: Option<f32>,
    pub deformation_center: Option<[f32; 4]>,
    pub deformation_vector: Option<[f32; 4]>,
    pub emissive_color_add: Option<[f32; 4]>,
    pub explosion_vector: Option<[f32; 4]>,
    /// `vfunc_402` reads the MID's `outwardMotion(WS)` and writes it back times `s`: the renderer multiplies the material's value by this.
    pub outward_motion_scale: Option<f32>,
}
impl MidParams {
    pub fn scalars(&self) -> Vec<(&'static str, f32)> {
        [(param::BUILD_ANIM_PHASE, self.build_anim_phase), (param::TOTAL_SUB_OBJECTS, self.total_sub_objects), (param::BREAK_ANIM, self.break_anim),
         (param::TARGET_BREAK_NUMBER, self.target_break_number), (param::TOTAL_DESTRUCTION, self.total_destruction), (param::FLOOR_BEING_DAMAGED, self.floor_being_damaged),
         (param::HIT_ANIMATION, self.hit_animation), (param::CUSTOM_DAMAGE_PCT, self.custom_damage_pct), (param::DAMAGE_RADIUS, self.damage_radius),
         (param::INVERT_DAMAGE_FALLOFF, self.invert_damage_falloff)]
            .into_iter().filter_map(|(n, v)| v.map(|v| (n, v))).collect()
    }
    pub fn vectors(&self) -> Vec<(&'static str, [f32; 4])> {
        [(param::DEFORMATION_CENTER, self.deformation_center), (param::DEFORMATION_VECTOR, self.deformation_vector), (param::EMISSIVE_COLOR_ADD, self.emissive_color_add),
         (param::EXPLOSION_VECTOR, self.explosion_vector)]
            .into_iter().filter_map(|(n, v)| v.map(|v| (n, v))).collect()
    }
}

/// BVISFIX: what the actor has written to its blueprint MIDs (`vfunc_313`, `vfunc_323`, `vfunc_402`, `vfunc_410`); reset when the ghost is destroyed (`vfunc_388`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BlueprintMid {
    pub hit_animation: Option<f32>,
    pub damage_radius: Option<f32>,
    pub deformation_center: Option<[f32; 4]>,
    pub deformation_vector: Option<[f32; 4]>,
    pub code_emissive: Option<[f32; 4]>,
    pub break_animation: Option<f32>,
    pub opacity: Option<f32>,
}
impl BlueprintMid {
    pub fn scalars(&self) -> Vec<(&'static str, f32)> {
        [(param::HIT_ANIMATION, self.hit_animation), (param::DAMAGE_RADIUS, self.damage_radius), (param::BREAK_ANIMATION, self.break_animation), (param::CODE_OPACITY, self.opacity)]
            .into_iter().filter_map(|(n, v)| v.map(|v| (n, v))).collect()
    }
    pub fn vectors(&self) -> Vec<(&'static str, [f32; 4])> {
        [(param::DEFORMATION_CENTER, self.deformation_center), (param::DEFORMATION_VECTOR, self.deformation_vector), (param::CODE_EMISSIVE, self.code_emissive)]
            .into_iter().filter_map(|(n, v)| v.map(|v| (n, v))).collect()
    }
}

/// BVISFIX: the damage overlay component (`MIDDamageOverlay` +0x990, `vfunc_322`): one MID of `DamageMaterial` on every slot of a second mesh component.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OverlayMid {
    pub hit_animation: Option<f32>,
    pub destruction_amount: Option<f32>,
    pub damage_radius: Option<f32>,
    pub deformation_center: Option<[f32; 4]>,
    pub deformation_vector: Option<[f32; 4]>,
}
impl OverlayMid {
    pub fn scalars(&self) -> Vec<(&'static str, f32)> {
        [(param::HIT_ANIMATION, self.hit_animation), (param::DESTRUCTION_AMOUNT, self.destruction_amount), (param::DAMAGE_RADIUS, self.damage_radius)]
            .into_iter().filter_map(|(n, v)| v.map(|v| (n, v))).collect()
    }
    pub fn vectors(&self) -> Vec<(&'static str, [f32; 4])> {
        [(param::DEFORMATION_CENTER, self.deformation_center), (param::DEFORMATION_VECTOR, self.deformation_vector)].into_iter().filter_map(|(n, v)| v.map(|v| (n, v))).collect()
    }
}

/// The last bounce (`vfunc_313`): `LastHitTime` +0x78C, radius +0x794, deformation vector +0x798, location +0x7A8, mode +0x7B8 (2 = the completion bounce).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounce { pub time: f64, pub radius: f32, pub location: glam::Vec3, pub direction: glam::Vec3, pub mode: u8 }

/// NBOUNCE: the `vfunc_313` arguments `vfunc_314` @14048fb20 hands the damaged actor and every `WallBounceOverlap` neighbour (mode 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageBounce { pub time: f64, pub radius: f32, pub direction: glam::Vec3, pub location: glam::Vec3 }

/// NBOUNCE: the `WallBounceOverlap` query of `vfunc_314`: centre = the hit location, radius = clamp(Bounds.SphereRadius, 0, 300) of the damaged piece
/// (`vfunc_276`, `DAT_143ff9040`), i.e. [`damage_bounce_radius`].
pub fn damage_bounce_radius(frame: &PieceFrame) -> f32 { frame.sphere_radius.clamp(0., HIT_RADIUS_CAP) }

/// NBOUNCE: does the sphere (`center`, `radius`) overlap a piece's collision (its cooked convexes at the actor transform)? `OverlapMultiByChannel` with
/// simple collision (`FCollisionQueryParams` bTraceComplex = 0): a convex overlaps when its distance to the centre is <= radius (ASSUMED: touching counts).
pub fn sphere_overlaps_hulls(hulls: &[fn_hull::Hull], center: glam::Vec3, radius: f32) -> bool {
    hulls.iter().any(|h| h.aabb_dist(center) <= radius && h.sdf(center) <= radius)
}

/// BVISFIX: what the actor's components report this frame (renderer space = the space of the piece's LocalToWorld, UE cm, Z up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PieceFrame {
    pub kind: PieceKind,
    /// Actor (root component) location.
    pub location: glam::Vec3,
    pub yaw_deg: f32,
    /// Root component `Bounds` (the mesh bounds moved by the component): origin and sphere radius.
    pub bounds_origin: glam::Vec3,
    pub sphere_radius: f32,
}
impl PieceFrame {
    /// `USceneComponent::GetRightVector` of the root (rotation only).
    pub fn right(&self) -> glam::Vec3 { let (s, c) = self.yaw_deg.to_radians().sin_cos(); glam::Vec3::new(-s, c, 0.) }
}

/// Something the renderer / audio should do because of an animation step.
#[derive(Clone, Debug, PartialEq)]
pub enum AnimEvent {
    /// Placement began: start the construction cue attached to the actor.
    ConstructionSoundStart(&'static str),
    /// BVISFIX: construction is over (under-construction bit clear): `ConstructionAudioComponent->FadeOut(0.5, 0.0)` (`ABuildingSMActor::Tick` @1404967f0,
    /// `DAT_143feb1d8` = 0.5).
    ConstructionSoundFadeOut(f32),
    /// The build reached all sub-objects: `Fort_Build_Done_Cue` at the actor (`vfunc_407`).
    Completed,
    /// BVISFIX: `vfunc_317` (the `ConstructedEffect`), spawned by `vfunc_323` once the finished-highlight curve passes its peak (~0.32 s after completion).
    ConstructedEffect,
}

/// Per-piece animation state (the `ABuildingSMActor` members listed in the module doc).
#[derive(Clone, Debug, PartialEq)]
pub struct PieceAnim {
    pub kind: AnimKind,
    pub cur_sub: i32,
    pub target: i32,
    pub start_time: f64,
    /// MIDs exist (`AnimatingMIDs` non-empty).
    pub has_mids: bool,
    pub mid: MidParams,
    /// The blueprint ghost component exists / is visible.
    pub blueprint: bool,
    pub blueprint_visible: bool,
    /// `code_JonsDanceTime` = `Interact` of the blueprint MIDs.
    pub blueprint_pulse: f32,
    pub surface: Surface,
    /// The build finished at least once (the actor's highlight / constructed effect were triggered).
    pub completed: bool,
    /// BVISFIX: blueprint MID parameters, the damage overlay (`None` = no overlay component), the last bounce, `DamageAmountStart` (+0x998),
    /// `LastDamageAmount` (+0x99C), the last damage's shot direction (`LastDamageHit` TraceEnd - TraceStart), `HighlightStart` (+0xF88, 0 = never),
    /// its duration (+0xF8C, ctor 1.0), the constructed-effect flag (+0xF90, ctor 1), the blueprint damage time (+0xF94), the under-construction bit
    /// (0x40 of +0xE90), the construction sound playing, and the actor hidden by the end of the destruction animation.
    pub bp: BlueprintMid,
    pub overlay: Option<OverlayMid>,
    pub hit: Option<Bounce>,
    pub damage_amount_start: f32,
    pub last_damage: f32,
    pub last_shot_dir: glam::Vec3,
    pub highlight_start: f64,
    pub highlight_duration: f32,
    pub constructed_pending: bool,
    pub bp_damage_time: f64,
    pub under_construction: bool,
    pub construction_sound: bool,
    pub hidden: bool,
}

impl PieceAnim {
    fn new(def: &PieceVisualDef, now: f64, target: i32, completed: bool) -> PieceAnim {
        PieceAnim { kind: AnimKind::None, cur_sub: def.animating_sub_objects as i32, target, start_time: now, has_mids: false, mid: MidParams::default(), blueprint: false, blueprint_visible: false, blueprint_pulse: 0., surface: Surface::Base, completed,
            bp: BlueprintMid::default(), overlay: None, hit: None, damage_amount_start: 0., last_damage: 0., last_shot_dir: glam::Vec3::ZERO, highlight_start: 0., highlight_duration: 1.0,
            constructed_pending: true, bp_damage_time: 0., under_construction: false, construction_sound: false, hidden: false }
    }

    /// A piece that was just spawned: `MulticastRequestBuildingAnimation(EBA_Placement)` (the player-built path; `vfunc_397(4)` -> `vfunc_403`).
    /// `under_construction` = the core's `HealthToAutoBuild` > 0 at spawn.
    pub fn placed(def: &PieceVisualDef, now: f64, health: f32, max_health: f32, under_construction: bool) -> (PieceAnim, Vec<AnimEvent>) {
        let mut a = PieceAnim::new(def, now, 0, false);
        a.under_construction = under_construction;
        let mut ev = vec![];
        if def.animating_material.is_some() { a.start(def, AnimKind::Placement, now, health, max_health, &[], &mut ev); } else { a.completed = true; }
        a.update_surface(now, health, max_health);
        (a, ev)
    }

    /// A piece that exists fully built (editing replaced it, or a world loaded): no animation, base material.
    pub fn built(def: &PieceVisualDef) -> PieceAnim { PieceAnim::new(def, 0., def.animating_sub_objects as i32, true) }

    fn create_mids(&mut self, def: &PieceVisualDef) {
        // vfunc_318: MIDs of the AnimatingMaterial, only when there are none yet
        if self.has_mids || def.animating_material.is_none() { return; }
        self.has_mids = true;
        self.mid = MidParams { total_sub_objects: Some(def.animating_sub_objects), floor_being_damaged: Some(0.), break_anim: Some(0.), build_anim_phase: Some(def.animating_sub_objects), ..Default::default() };
    }

    /// `vfunc_397(kind)` + the per-kind start function. `fx` = (game data, frame) for the destruction start.
    fn start(&mut self, def: &PieceVisualDef, kind: AnimKind, now: f64, health: f32, max_health: f32, fx: &[(&GameDataFx, &PieceFrame)], ev: &mut Vec<AnimEvent>) {
        self.kind = kind;
        self.create_mids(def);
        self.start_time = now;
        self.target = target_sub_objects(def, health, max_health);
        match kind {
            AnimKind::Placement => {
                // vfunc_403
                self.cur_sub = def.num_frame_sub_objects;
                self.mid.floor_being_damaged = Some(0.);
                self.mid.build_anim_phase = Some(self.cur_sub as f32);
                self.mid.total_destruction = Some(1.0);
                self.show_blueprint();
                self.blueprint_pulse = 0.;
                self.construction_sound = true;
                ev.push(AnimEvent::ConstructionSoundStart(def.construction_cue()));
            }
            AnimKind::Building => {
                // vfunc_400
                self.mid.floor_being_damaged = Some(0.);
                self.mid.break_anim = Some(0.);
                self.mid.build_anim_phase = Some(self.cur_sub as f32);
                self.show_blueprint();
            }
            AnimKind::Breaking => {
                // vfunc_401
                self.mid.floor_being_damaged = Some(1.0);
                self.mid.break_anim = Some(0.);
                self.mid.build_anim_phase = Some(self.cur_sub as f32);
                self.mid.target_break_number = Some(self.target as f32);
                self.show_blueprint();
            }
            AnimKind::Destruction => {
                // BVISFIX: vfunc_402 @14047c840: s = clamp(1 + LastDamageAmount / MaxHealth - DmgPctForCritDestructionAnim, 1, CritDmgAnimScaleCap), on every MID
                // Code_totalDestructionAmount 0, Code_FloorBeingDamaged 1, outwardMotion(WS) *= s, and when s > 1 CustomExplosionVectorForExplosion = the
                // normalised LastDamageHit TraceEnd - TraceStart; the blueprint is shown with code_BreakAnimation 0
                let (r, l) = (resource_index(def.resource), def.level.min(2));
                let (pct, cap) = fx.first().map(|(g, _)| (g.crit_pct[r][l], g.crit_cap[r][l])).unwrap_or((1.0, 1.0));
                let raw = (1.0 / max_health) * self.last_damage - pct + 1.0;
                let s = if raw >= 1.0 { if cap <= raw { cap } else { raw } } else { 1.0 };
                let d = self.last_shot_dir;
                let l2 = d.length_squared();
                let dir = if l2 == 1.0 { d } else if l2 < SMALL { glam::Vec3::ZERO } else { d / l2.sqrt() };
                self.mid.total_destruction = Some(0.);
                self.mid.floor_being_damaged = Some(1.0);
                self.mid.outward_motion_scale = Some(self.mid.outward_motion_scale.unwrap_or(1.0) * s);
                if 1.0 < s { self.mid.explosion_vector = Some([dir.x, dir.y, dir.z, 1.0]); }
                self.show_blueprint();
                self.bp.break_animation = Some(0.);
            }
            AnimKind::None => {}
        }
    }

    /// `vfunc_390(true)`: create (`vfunc_387`, only with an animating material) and show the blueprint ghost.
    fn show_blueprint(&mut self) { self.blueprint = true; self.blueprint_visible = true; }
    /// `vfunc_388`: the blueprint ghost (and its MIDs) destroyed.
    fn destroy_blueprint(&mut self) { self.blueprint = false; self.blueprint_visible = false; self.bp = BlueprintMid::default(); }

    /// `vfunc_398`: after an animation (or a health change while idle) start the next one if the target moved.
    fn redispatch(&mut self, def: &PieceVisualDef, now: f64, health: f32, max_health: f32, fx: &[(&GameDataFx, &PieceFrame)], ev: &mut Vec<AnimEvent>) {
        if def.animating_material.is_none() { return; }
        let target = target_sub_objects(def, health, max_health);
        if self.cur_sub < target { self.start(def, AnimKind::Building, now, health, max_health, fx, ev); }
        else if target < self.cur_sub { self.start(def, if def.num_frame_sub_objects <= target { AnimKind::Breaking } else { AnimKind::Destruction }, now, health, max_health, fx, ev); }
    }

    /// `vfunc_397(EBA_None)` + `vfunc_398`.
    fn end(&mut self, def: &PieceVisualDef, now: f64, health: f32, max_health: f32, fx: &[(&GameDataFx, &PieceFrame)], ev: &mut Vec<AnimEvent>) {
        self.kind = AnimKind::None;
        self.redispatch(def, now, health, max_health, fx, ev);
    }

    /// `vfunc_324` @14047f9c0: a hit within 5 s, or the finished highlight running.
    pub fn mid_fx_active(&self, now: f64) -> bool {
        if let Some(h) = self.hit { if (h.time as f32).abs() > SMALL && ((now - h.time) as f32) < HIT_WINDOW { return true; } }
        self.highlight_start != 0. && ((now - self.highlight_start) as f32) < self.highlight_duration
    }

    /// `vfunc_320` (+ `vfunc_272` / `vfunc_211`): the damage-visuals state picks the MID material, or clears the MIDs back at full health.
    fn update_surface(&mut self, now: f64, health: f32, max_health: f32) {
        let animating = self.kind != AnimKind::None || self.under_construction || self.mid_fx_active(now);
        self.surface = if animating { Surface::Animating } else if health < max_health { Surface::DamagedStatic } else { Surface::Base };
        if self.surface == Surface::Base && self.has_mids {
            // bClearMIDWhenReturningToUndamagedState (vfunc_319)
            self.has_mids = false;
            self.mid = MidParams::default();
        }
        if !self.has_mids { self.surface = Surface::Base; }
    }

    /// `vfunc_313` @140493170: record the bounce and write its parameters on the animating MIDs, the overlay MID and the blueprint MIDs.
    fn bounce(&mut self, def: &PieceVisualDef, now: f64, radius: f32, direction: glam::Vec3, location: glam::Vec3, mode: u8, health: f32) {
        self.hit = Some(Bounce { time: now, radius, location, direction, mode });
        if self.damage_amount_start == 0. { self.damage_amount_start = health; }
        self.create_mids(def);
        let inv = if radius <= 0. { 0. } else { 1.0 / radius };
        // UKismetMathLibrary::Conv_VectorToLinearColor: (x, y, z, 1)
        let (c, d) = ([location.x, location.y, location.z, 1.0], [direction.x, direction.y, direction.z, 1.0]);
        if self.has_mids {
            let m = &mut self.mid;
            m.deformation_center = Some(c); m.deformation_vector = Some(d); m.hit_animation = Some(0.); m.damage_radius = Some(inv);
            m.invert_damage_falloff = Some(if mode == 2 { 1.0 } else { 0.0 });
        }
        if let Some(o) = self.overlay.as_mut() { o.deformation_center = Some(c); o.deformation_vector = Some(d); o.hit_animation = Some(0.); o.damage_radius = Some(inv); }
        if self.blueprint { let b = &mut self.bp; b.deformation_center = Some(c); b.deformation_vector = Some(d); b.hit_animation = Some(0.); b.damage_radius = Some(inv); }
    }

    /// A damaging hit: `vfunc_276` @140486ae0 (`LastDamageAmount`, `LastDamageHit`, radius = clamp(Bounds.SphereRadius, 0, 300)) -> `vfunc_314` @14048fb20 (show the
    /// blueprint, blueprint damage time, damage overlay, `DamageAmountStart` = health + damage, the bounce direction projected per class, then `vfunc_313` with
    /// mode 0). `impact` = the HitResult ImpactPoint, `to_attacker` = unit vector from the impact towards the damage causer (`vfunc_276`: normalize(causer
    /// location - ImpactPoint), else -ImpulseDir). `health` = after the damage. `damage` = the cue's RawMagnitude (the `FortHealthSet.Damage` magnitude; for
    /// a non-fatal hit = the health lost).
    /// NBOUNCE: `neighbours` = the kinds of the pieces the `WallBounceOverlap` sphere ([`damage_bounce_radius`] at `impact`, [`sphere_overlaps_hulls`])
    /// overlaps, self excluded; only used when `def.propagates_bounce`. Returns the bounce to hand those pieces ([`PieceAnim::on_neighbour_bounce`]), `None`
    /// for a fatal hit (health <= 0: `bIsFatalHit`, `GameplayCue_Damage` @14046b570 calls OnDeath `vfunc_277` instead of `vfunc_276`, so nothing here runs and
    /// `LastDamageAmount` keeps the previous hit's value for the destruction scale of `vfunc_402`).
    #[allow(clippy::too_many_arguments)]
    pub fn on_damage(&mut self, def: &PieceVisualDef, frame: &PieceFrame, now: f64, damage: f32, health: f32, impact: glam::Vec3, to_attacker: glam::Vec3, neighbours: &[PieceKind]) -> Option<DamageBounce> {
        if health <= 0. { return None; }
        self.last_damage = damage;
        self.last_shot_dir = -to_attacker;
        let radius = damage_bounce_radius(frame);
        // vfunc_314: vfunc_390(true) (the ghost exists only with an animating material, vfunc_387), +0xF94 = now, vfunc_322
        if def.animating_material.is_some() { self.show_blueprint(); }
        self.bp_damage_time = now;
        if self.overlay.is_none() && def.animating_material.is_none() && def.needs_damage_overlay { self.overlay = Some(OverlayMid::default()); }
        self.damage_amount_start = health + damage;
        let d = damage_bounce_direction(frame, to_attacker, if def.propagates_bounce { neighbours } else { &[] });
        self.bounce(def, now, radius, d, impact, 0, health);
        Some(DamageBounce { time: now, radius, direction: d, location: impact })
    }

    /// NBOUNCE: `vfunc_313` on a `WallBounceOverlap` neighbour of a damaged piece (`vfunc_314` @14048fb20: the same time / radius / direction / location,
    /// mode 0). Only the bounce: no blueprint, no `LastDamageAmount`, no `DamageAmountStart` = health + damage (`vfunc_313` sets it to the neighbour's own
    /// health when it is 0). `health` = the neighbour's current health.
    pub fn on_neighbour_bounce(&mut self, def: &PieceVisualDef, b: &DamageBounce, health: f32) {
        self.bounce(def, b.time, b.radius, b.direction, b.location, 0, health);
    }

    /// OnDeath (`ABuildingSMActor::vfunc_277` @140486330): with an animating material the destruction animation starts (`vfunc_397(3)`).
    pub fn on_death(&mut self, def: &PieceVisualDef, fx: &GameDataFx, frame: &PieceFrame, now: f64, max_health: f32) {
        if def.animating_material.is_none() { self.hidden = true; return; }
        let mut ev = vec![];
        self.start(def, AnimKind::Destruction, now, 0., max_health, &[(fx, frame)], &mut ev);
    }

    /// `vfunc_323` @140499690: the per-frame MID drive (run while `vfunc_324`).
    fn drive_mids(&mut self, fx: &GameDataFx, now: f64, dt: f32, health: f32, max_health: f32) -> Option<AnimEvent> {
        let mut out = None;
        let Some(hit) = self.hit else { return None };
        let curve = if hit.mode == 2 { fx.edit_bounce.as_ref() } else { fx.bounce.as_ref() }; // vfunc_325
        let since = (now - hit.time) as f32;
        let (mut t0, mut t1, mut hit_anim) = (0.0f32, 1.0f32, 0.0f32);
        if let Some(c) = curve {
            (t0, t1) = c.time_range();
            hit_anim = c.eval(since.clamp(t0, t1.max(t0)));
        }
        let f = since.clamp(t0, t1.max(t0)) / (t1 - t0);
        let inv_max = 1.0 / max_health;
        let das = self.damage_amount_start;
        let dmg = (((health - das) * f + das) * inv_max).clamp(0., 1.);
        let mid_ov = (health + das) * 0.5;
        let ov = (inv_max * ((health - mid_ov) * f + mid_ov)).clamp(0., 1.);
        if let Some(o) = self.overlay.as_mut() { o.hit_animation = Some(hit_anim); o.destruction_amount = Some(1.0 - ov); }
        if self.has_mids {
            self.mid.hit_animation = Some(hit_anim);
            self.mid.custom_damage_pct = Some(1.0 - dmg);
            if let Some(c) = fx.full_health.as_ref() {
                let th = (now - self.highlight_start) as f32;
                let dur = self.highlight_duration;
                let prev_t = { let p = th - dt; if 0.0 <= p { p.min(th).max(0.).min(dur) } else { 0.0f32.min(dur) } };
                let cur_t = if 0.0 <= th { th.min(dur) } else { 0.0 };
                let (prev, cur) = (c.eval(prev_t), c.eval(cur_t));
                let k = fx.flash_color;
                self.mid.emissive_color_add = Some([k[0] * cur, k[1] * cur, k[2] * cur, k[3] * cur]);
                if self.constructed_pending && cur < prev { self.constructed_pending = false; out = Some(AnimEvent::ConstructedEffect); }
            }
        }
        if self.blueprint {
            if let Some(v) = fx.blueprint_damage.as_ref() {
                let (b0, b1) = v.time_range();
                let t = ((now - self.bp_damage_time) as f32).clamp(b0, b1.max(b0));
                let c = v.eval(t / (b1 - b0));
                let k = (1.0 - dmg) + 1.0;
                self.bp.hit_animation = Some(hit_anim);
                self.bp.code_emissive = Some([k * c[0], k * c[1], k * c[2], 1.0]);
            }
        }
        out
    }

    /// Advances one frame (`ABuildingSMActor::Tick` @1404967f0: animation tick `vfunc_406`, the MID drive while `vfunc_324`, the construction sound fade, the
    /// damage-visuals state `vfunc_320`). `now` = world time (s), `build_time` = the piece's `BuildTime` attribute (s), `under_construction` = the core's
    /// `HealthToAutoBuild` > 0.
    pub fn tick(&mut self, def: &PieceVisualDef, fx: &GameDataFx, frame: &PieceFrame, now: f64, dt: f32, health: f32, max_health: f32, build_time: f32, under_construction: bool) -> Vec<AnimEvent> {
        let mut ev = vec![];
        let r = resource_index(def.resource);
        self.under_construction = under_construction;
        let fxs = [(fx, frame)];
        match self.kind {
            AnimKind::Placement => {
                // vfunc_408
                let t = (((now - self.start_time) as f32) / PLACEMENT_ANIM_DURATION[r]).clamp(0., 1.);
                let rest = 1.0 - t;
                self.mid.total_destruction = Some(rest);
                if self.blueprint { self.blueprint_pulse = (1.0 - (t - 0.5).abs() * 2.0) * BLUEPRINT_PULSE; }
                if rest.abs() <= SMALL {
                    self.mid.total_destruction = Some(0.);
                    self.end(def, now, health, max_health, &fxs, &mut ev);
                }
            }
            AnimKind::Building => {
                // vfunc_407 (+ vfunc_368 speed)
                let speed = if build_time > 0. { (def.animating_sub_objects - def.num_frame_sub_objects as f32) / build_time } else { f32::MAX };
                let old = self.mid.build_anim_phase.unwrap_or(def.animating_sub_objects);
                let phase = finterp_constant_to(old, self.target as f32, dt, speed);
                self.mid.build_anim_phase = Some(phase);
                let (nc, oc) = (phase.floor() as i32, old.floor() as i32);
                if oc < nc { self.cur_sub = nc; }
                if self.target <= self.cur_sub {
                    self.kind = AnimKind::None;
                    if self.cur_sub as f32 == def.animating_sub_objects {
                        // vfunc_388 (blueprint ghost destroyed), vfunc_391 @1404937c0 (highlight: MIDs, HighlightStart = now, flag +0xF90 = 1,
                        // Emissive_Color_Add = (0, 0, 0, 1) = DAT_145198810), vfunc_393 @1404859a0 (completion bounce), ConstructionCompleteSound
                        self.destroy_blueprint();
                        self.create_mids(def);
                        self.highlight_start = now;
                        self.constructed_pending = true;
                        if self.has_mids { self.mid.emissive_color_add = Some([0., 0., 0., 1.]); }
                        // vfunc_393: location (Bounds.Origin.X, Bounds.Origin.Y, root Z) (the roof override @140485900 keeps Bounds.Origin.Z), direction
                        // (0, 0, 1), radius Bounds.SphereRadius * 5.0 (DAT_143fe3710), mode 2
                        let loc = if frame.kind == PieceKind::Roof { frame.bounds_origin } else { glam::Vec3::new(frame.bounds_origin.x, frame.bounds_origin.y, frame.location.z) };
                        self.bounce(def, now, frame.sphere_radius * HIT_WINDOW, glam::Vec3::Z, loc, 2, health);
                        self.completed = true;
                        ev.push(AnimEvent::Completed);
                    } else {
                        self.redispatch(def, now, health, max_health, &fxs, &mut ev);
                    }
                }
            }
            AnimKind::Breaking => {
                // vfunc_409
                let t = (((now - self.start_time) as f32) / BREAK_ANIM_DURATION[r]).clamp(0., 1.);
                self.mid.break_anim = Some(t);
                if (t - 1.0).abs() <= SMALL {
                    self.cur_sub = self.target;
                    self.mid.break_anim = Some(0.);
                    self.mid.floor_being_damaged = Some(0.);
                    self.mid.build_anim_phase = Some(self.target as f32);
                    self.end(def, now, health, max_health, &fxs, &mut ev);
                }
            }
            AnimKind::Destruction => {
                // BVISFIX: vfunc_410 @140497610: t = (now - start) / BuildingDestructionAnimDuration; MIDs Code_totalDestructionAmount = t; blueprint
                // code_BreakAnimation = t, code_opacity = 1 - t; at t = 1: CurAnimSubObjectNum = 0, BuildAnimPhase = 0, SetActorHiddenInGame(true), vfunc_397(0)
                let t = (((now - self.start_time) as f32) / fx.destroy_duration[r]).clamp(0., 1.);
                self.mid.total_destruction = Some(t);
                if self.blueprint { self.bp.break_animation = Some(t); self.bp.opacity = Some(1.0 - t); }
                if (t - 1.0).abs() <= SMALL {
                    self.cur_sub = 0;
                    self.mid.build_anim_phase = Some(0.);
                    self.hidden = true;
                    self.kind = AnimKind::None;
                }
            }
            AnimKind::None => {
                if def.animating_material.is_some() { self.redispatch(def, now, health, max_health, &fxs, &mut ev); }
            }
        }
        // Tick: vfunc_323 while vfunc_324
        if self.mid_fx_active(now) { if let Some(e) = self.drive_mids(fx, now, dt, health, max_health) { ev.push(e); } }
        // Tick: the under-construction bit is clear and the construction audio component plays -> FadeOut(0.5, 0)
        if !under_construction && self.construction_sound { self.construction_sound = false; ev.push(AnimEvent::ConstructionSoundFadeOut(0.5)); }
        // Tick: vfunc_320
        self.update_surface(now, health, max_health);
        ev
    }
}

/// `ABuildingFloor` IsA: floors and stairs (`ABuildingStairs` ctor @14049f370 runs the `ABuildingFloor` ctor; roofs derive from `ABuildingSMActor`).
fn is_floor(kind: PieceKind) -> bool { matches!(kind, PieceKind::Floor | PieceKind::Stairs) }

/// NBOUNCE: the bounce direction of `vfunc_314` @14048fb20 for a hit on `frame` from `to_attacker`, with the kinds of the `WallBounceOverlap` neighbours
/// (empty when the class does not propagate). Steps (decompile order):
/// 1. the damaged actor: IsA floor -> Z = 0; IsA wall -> zero the components where |RightVector_i| > 1e-4 (`DAT_143ff9010`), normalise (`FVector::GetSafeNormal`:
///    exact unit length kept, < 1e-8 -> 0);
/// 2. per overlapped neighbour, in order, on the same vector: floor -> Z = 0; wall -> zero the components where |RightVector_i| of the DAMAGED actor > 1e-8
///    (`DAT_143ff9008`), no renormalisation;
/// 3. a zero vector becomes (to_attacker.x, to_attacker.y, 0); `GetSafeNormal`.
pub fn damage_bounce_direction(frame: &PieceFrame, to_attacker: glam::Vec3, neighbours: &[PieceKind]) -> glam::Vec3 {
    let norm = |v: glam::Vec3| { let l2 = v.length_squared(); if l2 == 1.0 { v } else if l2 >= SMALL { v / l2.sqrt() } else { glam::Vec3::ZERO } };
    let zero_along = |d: glam::Vec3, r: glam::Vec3, eps: f32| glam::Vec3::new(if eps < r.x.abs() { 0. } else { d.x }, if eps < r.y.abs() { 0. } else { d.y }, if eps < r.z.abs() { 0. } else { d.z });
    let r = frame.right();
    let mut d = to_attacker;
    if is_floor(frame.kind) { d.z = 0.; }
    if frame.kind == PieceKind::Wall { d = norm(zero_along(d, r, AXIS_EPS)); }
    for &n in neighbours {
        if is_floor(n) { d.z = 0.; }
        if n == PieceKind::Wall { d = zero_along(d, r, SMALL); }
    }
    if d == glam::Vec3::ZERO { d = glam::Vec3::new(to_attacker.x, to_attacker.y, 0.); }
    norm(d)
}

/// The preview marker colour parameter to copy into `Code_Color` (FUN_140831f80: `code_Blue` only for a valid placement).
pub fn preview_color_source(valid: bool) -> &'static str { if valid { param::CODE_BLUE } else { param::CODE_RED } }

/// Class path of a player piece variant (the folder layout of `/Game/Building/ActorBlueprints/Player`): `variant` is the suffix after `PBWA_<M><n>_`
/// (`Solid`, `Floor`, `StairW`, `RoofC`, `Windows`, ...). The cooked names are irregular for a few variants (`PBWA_M3_Floor_2`, `PBWA_S2_Pillar2`,
/// `PBWA_S1_WindowsC`): the editing data (`EditDb`, from `PossibleEditClasses`) is the authority; this is the plain pattern.
pub fn piece_class(material: Material, level: usize, variant: &str) -> String {
    let (folder, letter) = match material { Material::Wood => ("Wood", 'W'), Material::Stone => ("Stone", 'S'), Material::Metal => ("Metal", 'M') };
    let n = level.min(2) + 1;
    format!("/Game/Building/ActorBlueprints/Player/{folder}/L{n}/PBWA_{letter}{n}_{variant}.PBWA_{letter}{n}_{variant}_C")
}

/// World position (UE cm) -> the port's game space (metres, Y up) used by the renderer and the audio sink: `(x, z, y) * 0.01`.
pub fn ue_to_game(p: glam::Vec3) -> glam::Vec3 { glam::Vec3::new(p.x, p.z, p.y) * 0.01 }

#[cfg(test)]
mod tests {
    use super::*;
    fn def() -> PieceVisualDef {
        PieceVisualDef { class_path: "/Game/Building/ActorBlueprints/Player/Wood/L1/PBWA_W1_Solid.PBWA_W1_Solid_C".into(), static_mesh: "m".into(), animating_material: Some("a".into()), damaged_material: Some("d".into()),
            animating_sub_objects: 27., num_frame_sub_objects: 4, constructed_effect: None, death_particles: None, death_particle_socket: None, death_sound: None, resource: Material::Wood, level: 0, edit_mode_pattern: None, needs_damage_overlay: true, propagates_bounce: true }
    }
    fn lin(k: &[(f32, f32)]) -> RichCurve { RichCurve::from_keys(k.iter().map(|&(t, v)| (t, v, 0, 0., 0.)).collect()) }
    /// The cooked curves' keys (fn-inspect props of FinishedHighlight / RedImpactCurve; a short bounce stand-in).
    fn gfx() -> GameDataFx {
        GameDataFx {
            bounce: Some(lin(&[(0., 0.), (0.0437, -1.4), (0.132, 1.3), (0.7, 0.)])), edit_bounce: Some(lin(&[(0., 0.), (0.1, 1.0), (0.5, 0.)])),
            full_health: Some(lin(&[(0., 0.), (0.17289, 0.08795), (0.23777, 0.31456), (0.32183, 1.99932), (0.45551, 0.21357), (0.55441, 0.08028), (1.0, 0.)])),
            blueprint_damage: Some(VectorCurve([lin(&[(0., 0.0001), (0.2319, 1.668), (1.0, 0.)]), lin(&[(0., 0.), (1., 0.)]), lin(&[(0., 0.), (1., 0.)])])),
            crit_pct: [[0.5; 3]; 3], crit_cap: [[2.0; 3]; 3], ..GameDataFx::default()
        }
    }
    fn frame(kind: PieceKind) -> PieceFrame { PieceFrame { kind, location: glam::Vec3::new(100., 200., 0.), yaw_deg: 0., bounds_origin: glam::Vec3::new(100., 200., 194.), sphere_radius: 290. } }
    #[test] fn target_is_ceil_of_the_health_lerp() {
        let d = def();
        assert_eq!(target_sub_objects(&d, 0., 200.), 0);
        assert_eq!(target_sub_objects(&d, 200., 200.), 27);
        assert_eq!(target_sub_objects(&d, 100., 200.), 16); // 4 + 23 * 0.5 = 15.5 -> 16
        assert_eq!(target_sub_objects(&d, 1., 200.), 5);
    }
    #[test] fn finterp_matches_the_engine() {
        assert_eq!(finterp_constant_to(0., 10., 0.5, 4.), 2.);
        assert_eq!(finterp_constant_to(9.9, 10., 0.5, 4.), 10.);
        assert_eq!(finterp_constant_to(10., 0., 0.5, 4.), 8.);
    }
    /// The build-up with the core's 0.5 s auto-build steps: the AnimatingMaterial stays for the whole construction (never the damaged one between steps),
    /// the loop fades when construction ends, the completion flash peaks with the sparkle ~0.32 s later, the MIDs clear after the 5 s hit window.
    #[test] fn placement_then_building_to_completion() {
        let (d, g, fr) = (def(), gfx(), frame(PieceKind::Wall));
        let (mut a, ev) = PieceAnim::placed(&d, 0., 20., 200., true);
        assert_eq!(ev, vec![AnimEvent::ConstructionSoundStart(CONSTRUCTION_CUES[0][0])]);
        assert_eq!((a.kind, a.cur_sub, a.mid.total_destruction, a.surface), (AnimKind::Placement, 4, Some(1.0), Surface::Animating));
        assert!(a.blueprint_visible);
        a.tick(&d, &g, &fr, 0.175, 0.175, 20., 200., 4.0, true);
        assert!((a.blueprint_pulse - 2.5).abs() < 1e-5 && (a.mid.total_destruction.unwrap() - 0.5).abs() < 1e-5);
        a.tick(&d, &g, &fr, 0.35, 0.175, 20., 200., 4.0, true);
        assert_eq!((a.kind, a.target), (AnimKind::Building, 7));
        let (mut t, mut h, mut acc) = (0.35f64, 20f32, 0.35f32);
        let (mut done, mut fade, mut sparkle) = (None, None, None);
        for _ in 0..800 {
            t += 1. / 60.; acc += 1. / 60.;
            // BuildingWorld::tick: CeilToInt(rate * accumulated) every 0.5 s
            if acc >= 0.5 { h = (h + (180. / 4.0 * acc).ceil()).min(200.); acc = 0.; }
            let building = h < 200.;
            let ev = a.tick(&d, &g, &fr, t, 1. / 60., h, 200., 4.0, building);
            if building { assert_eq!(a.surface, Surface::Animating, "t {t}: damaged material during construction"); }
            if ev.contains(&AnimEvent::Completed) { done = Some(t); assert!(!a.blueprint); assert_eq!(a.mid.emissive_color_add, Some([0., 0., 0., 0.])); }
            if ev.iter().any(|e| matches!(e, AnimEvent::ConstructionSoundFadeOut(f) if *f == 0.5)) { fade = Some(t); }
            if ev.contains(&AnimEvent::ConstructedEffect) { sparkle = Some(t); }
        }
        let (done, fade, sparkle) = (done.expect("completed"), fade.expect("fade"), sparkle.expect("sparkle"));
        assert!(fade <= done, "the loop fades when the auto-build ends ({fade}), not later ({done})");
        assert!((sparkle - done - 0.322).abs() < 0.02, "sparkle {} s after completion", sparkle - done);
        assert_eq!(a.cur_sub, 27);
        assert!(t - done > 5.0 && a.surface == Surface::Base, "MIDs cleared after the hit window");
    }
    #[test] fn damage_bounces_shows_the_blueprint_and_breaks_down() {
        let (d, g, fr) = (def(), gfx(), frame(PieceKind::Wall));
        let mut a = PieceAnim::built(&d);
        a.on_damage(&d, &fr, 10., 100., 100., glam::Vec3::new(150., 200., 100.), glam::Vec3::new(0.6, 0.8, 0.), &[]);
        assert!(a.blueprint_visible && a.has_mids);
        // wall at yaw 0: right = +Y, the Y component is dropped (vfunc_314)
        assert_eq!(a.hit.unwrap().direction, glam::Vec3::X);
        assert_eq!(a.mid.damage_radius, Some(1.0 / 290.));
        assert_eq!(a.damage_amount_start, 200.);
        a.tick(&d, &g, &fr, 10.0, 0.016, 100., 200., 4.0, false);
        assert_eq!((a.kind, a.target, a.mid.target_break_number), (AnimKind::Breaking, 16, Some(16.)));
        a.tick(&d, &g, &fr, 10.23, 0.016, 100., 200., 4.0, false);
        let e = a.bp.code_emissive.unwrap();
        assert!(e[0] > 1.0 && a.mid.custom_damage_pct.unwrap() > 0.0, "{a:?}");
        a.tick(&d, &g, &fr, 11.0, 0.016, 100., 200., 4.0, false);
        assert_eq!((a.kind, a.cur_sub, a.surface), (AnimKind::None, 16, Surface::Animating));
        a.tick(&d, &g, &fr, 15.1, 0.016, 100., 200., 4.0, false);
        assert_eq!((a.surface, a.mid.build_anim_phase), (Surface::DamagedStatic, Some(16.)));
    }
    #[test] fn death_plays_the_destruction_then_hides() {
        let (d, g, fr) = (def(), gfx(), frame(PieceKind::Wall));
        let mut a = PieceAnim::built(&d);
        // NBOUNCE: a non-fatal 180 hit records LastDamageAmount = 180 and LastDamageHit (vfunc_276); the killing 50 hit (20 health left -> 0) is a
        // bIsFatalHit cue: GameplayCue_Damage @14046b570 calls vfunc_277, not vfunc_276, so it changes nothing here (not 20 = health lost, not 50 = dealt)
        assert!(a.on_damage(&d, &fr, 1., 180., 20., fr.bounds_origin, glam::Vec3::NEG_Y, &[]).is_some());
        assert_eq!(a.on_damage(&d, &fr, 1., 20., 0., fr.bounds_origin, glam::Vec3::X, &[]), None);
        assert_eq!((a.last_damage, a.last_shot_dir), (180., glam::Vec3::Y));
        a.on_death(&d, &g, &fr, 1., 200.);
        // vfunc_402 @14047c840: s = clamp(1 + LastDamageAmount 180 / MaxHealth 200 - DmgPctForCritDestructionAnim 0.5, 1, cap 2) = 1.4
        assert_eq!(a.kind, AnimKind::Destruction);
        assert!((a.mid.outward_motion_scale.unwrap() - 1.4).abs() < 1e-5);
        assert_eq!(a.mid.explosion_vector, Some([0., 1., 0., 1.]));
        a.tick(&d, &g, &fr, 1.75, 0.016, 0., 200., 4.0, false);
        assert!((a.mid.total_destruction.unwrap() - 0.5).abs() < 1e-4 && (a.bp.opacity.unwrap() - 0.5).abs() < 1e-4 && !a.hidden);
        a.tick(&d, &g, &fr, 2.5, 0.016, 0., 200., 4.0, false);
        assert!(a.hidden && a.kind == AnimKind::None);
    }
    /// NBOUNCE: a piece killed by its first hit: `LastDamageAmount` is still the ctor's 0, so s = clamp(0 / 200 - 0.5 + 1, 1, 2) = 1 (no scale, no
    /// `CustomExplosionVectorForExplosion`), whatever the killing damage was.
    #[test] fn one_shot_destruction_has_no_crit_scale() {
        let (d, g, fr) = (def(), gfx(), frame(PieceKind::Wall));
        let mut a = PieceAnim::built(&d);
        assert_eq!(a.on_damage(&d, &fr, 1., 200., 0., fr.bounds_origin, glam::Vec3::NEG_Y, &[]), None);
        assert!(!a.blueprint_visible && a.hit.is_none(), "the fatal cue never bounces (vfunc_314 is not reached)");
        a.on_death(&d, &g, &fr, 1., 200.);
        assert_eq!((a.mid.outward_motion_scale, a.mid.explosion_vector), (Some(1.0), None));
    }
    /// NBOUNCE: `vfunc_314` @14048fb20 direction rules (self floor / stairs / wall, then the neighbours on the shared vector with the damaged actor's
    /// RightVector and the 1e-8 tolerance, then the zero fallback).
    #[test] fn damage_bounce_direction_rules() {
        let v = glam::Vec3::new(0.6, 0.0, 0.8);
        // stairs IsA ABuildingFloor (ctor @14049f370): Z dropped like a floor
        assert!((damage_bounce_direction(&frame(PieceKind::Stairs), v, &[]) - glam::Vec3::X).length() < 1e-6);
        assert!((damage_bounce_direction(&frame(PieceKind::Roof), v, &[]) - v).length() < 1e-6);
        // a roof hit next to a floor: the floor neighbour drops Z
        assert!((damage_bounce_direction(&frame(PieceKind::Roof), v, &[PieceKind::Floor]) - glam::Vec3::X).length() < 1e-6);
        // a roof (yaw 0: right = +Y) next to a wall: the Y component is dropped, not renormalised before the final GetSafeNormal
        let w = glam::Vec3::new(0.48, 0.6, 0.64);
        let got = damage_bounce_direction(&frame(PieceKind::Roof), w, &[PieceKind::Wall]);
        assert!((got - glam::Vec3::new(0.6, 0., 0.8)).length() < 1e-6, "{got:?}");
        // a wall hit straight along its right vector with a floor neighbour: (0, 1, 0) -> wall zeroes Y -> zero -> fallback (x, y, 0) = (0, 1, 0)
        assert_eq!(damage_bounce_direction(&frame(PieceKind::Wall), glam::Vec3::Y, &[PieceKind::Floor]), glam::Vec3::Y);
        // yaw 45: right = (-0.707, 0.707, 0): a wall neighbour zeroes X and Y (|r| > 1e-8): Z alone is left
        let f45 = PieceFrame { yaw_deg: 45., ..frame(PieceKind::Roof) };
        assert!((damage_bounce_direction(&f45, glam::Vec3::new(0.6, 0., 0.8), &[PieceKind::Wall]) - glam::Vec3::Z).length() < 1e-6);
    }
    /// NBOUNCE: the neighbours get the damaged piece's bounce (vfunc_313, mode 0) but nothing else of the hit; `bPropagatesBounceEffects` = 0 ignores them.
    #[test] fn neighbour_bounce_propagates() {
        let (d, g, fr) = (def(), gfx(), frame(PieceKind::Roof));
        let mut hit = PieceAnim::built(&d);
        let b = hit.on_damage(&d, &fr, 3., 50., 150., glam::Vec3::new(100., 200., 50.), glam::Vec3::new(0.6, 0., 0.8), &[PieceKind::Floor]).unwrap();
        assert!((b.time, b.radius, b.location) == (3., 290., glam::Vec3::new(100., 200., 50.)) && (b.direction - glam::Vec3::X).length() < 1e-6, "{b:?}");
        assert_eq!(hit.hit.unwrap().direction, b.direction, "the damaged piece bounces with the neighbour-edited direction too");
        let mut n = PieceAnim::built(&d);
        n.on_neighbour_bounce(&d, &b, 190.);
        assert_eq!(n.hit, Some(Bounce { time: 3., radius: 290., location: b.location, direction: b.direction, mode: 0 }));
        assert_eq!((n.mid.damage_radius, n.mid.hit_animation, n.mid.invert_damage_falloff), (Some(1. / 290.), Some(0.), Some(0.)));
        assert_eq!((n.damage_amount_start, n.last_damage, n.blueprint_visible), (190., 0., false));
        n.tick(&d, &g, &fr, 3.1, 0.016, 190., 200., 4.0, false);
        assert_eq!(n.surface, Surface::Animating, "the hit window of vfunc_324 runs on the neighbour");
        let no = PieceVisualDef { propagates_bounce: false, ..def() };
        let mut a = PieceAnim::built(&no);
        a.on_damage(&no, &fr, 3., 50., 150., glam::Vec3::ZERO, glam::Vec3::new(0.6, 0., 0.8), &[PieceKind::Floor]);
        assert!((a.hit.unwrap().direction - glam::Vec3::new(0.6, 0., 0.8)).length() < 1e-6);
        assert!(PieceVisualDef::from_cdo("/Game/X.X_C", &serde_json::json!({ "StaticMesh": "m" })).unwrap().propagates_bounce, "ctor @14045b960 default 1");
    }
    #[test] fn sphere_overlap_of_hulls() {
        let cube = |c: glam::Vec3| fn_hull::Hull::from_points(&[-1., 1.].iter().flat_map(|&x| [-1., 1.].iter().flat_map(move |&y| [-1., 1.].iter().map(move |&z| c + glam::Vec3::new(x, y, z) * 10.))).collect::<Vec<_>>()).unwrap();
        let h = [cube(glam::Vec3::new(100., 0., 0.))];
        assert!(sphere_overlaps_hulls(&h, glam::Vec3::ZERO, 90.));
        assert!(!sphere_overlaps_hulls(&h, glam::Vec3::ZERO, 89.));
        assert!(sphere_overlaps_hulls(&h, glam::Vec3::new(100., 0., 0.), 0.), "a centre inside overlaps");
        assert_eq!(damage_bounce_radius(&PieceFrame { sphere_radius: 400., ..frame(PieceKind::Wall) }), 300.);
    }
    #[test] fn class_and_level_helpers() {
        assert_eq!(level_of_class("/Game/Building/ActorBlueprints/Player/Metal/L3/PBWA_M3_Solid.PBWA_M3_Solid_C"), 2);
        assert_eq!(piece_class(Material::Stone, 0, "Floor"), "/Game/Building/ActorBlueprints/Player/Stone/L1/PBWA_S1_Floor.PBWA_S1_Floor_C");
        assert_eq!(preview_color_source(false), "Code_Red");
        assert_eq!(centroid_offset(PieceKind::Wall), glam::Vec3::new(0., 0., 192.));
        assert_eq!(centroid_offset(PieceKind::Stairs), glam::Vec3::new(0., 256., 192.));
        let props = serde_json::json!({ "StaticMesh": "m", "CurrentBuildingLevel": 2 });
        assert_eq!(PieceVisualDef::from_cdo("/Game/X/L1/PBWA_M3_Solid.PBWA_M3_Solid_C", &props).unwrap().level, 2, "the CDO field, not the folder");
    }
}
