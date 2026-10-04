//! Movement / gravity constants for the Athena player pawn (Fortnite 1.8, UE 4.16 CL 3724489), in UE units
//! (centimetres, cm/s, cm/s^2, seconds). Checked against `reports/movement-full.json`
//! (regenerate: `node tools/movement-full-evidence.cjs`, which reads the 1.8 paks and the 1.8 exe read-only).
//!
//! Status words (same as the report):
//!  * RECOVERED  - serialized for the Athena pawn in the 1.8 paks (PlayerPawn_Athena CharMoveComp, CharacterAttributesPlayer row).
//!  * INHERITED  - serialized on a parent Blueprint (PlayerPawn_Athena_Generic) and not overridden below it.
//!  * EXE        - not serialized in any pak; read from the immediate operands of the native UCharacterMovementComponent
//!                 constructor in FortniteClient-Win64-Shipping.exe (static read of .text, the exe is never run). The member name
//!                 is attached by UE 4.16 declaration order, the value is exact. No Fortnite subclass constructor was found that
//!                 changes it (see movement-full.json `exeCtor`).
//!  * ASSUMED    - nothing in the pak or the exe pins it; UE 4.16 source default or a labelled reconstruction.
#![allow(dead_code)]
// ---- speeds (RECOVERED: CharacterAttributesPlayer row PlayerLoadout_AthenaPlayer; which stat feeds GetMaxSpeed is ASSUMED)
pub const SPEED_WALK: f32 = 200.;
/// Normal jog ("Run" stat); Shift sprint is `SPEED_SPRINT`.
pub const SPEED_RUN: f32 = 410.;
pub const SPEED_SPRINT: f32 = 550.;
pub const SPEED_CROUCHED_RUN: f32 = 290.;
pub const SPEED_CROUCHED_SPRINT: f32 = 420.;
/// Same value as the last key of the directional curve (0.65 at >= 110 degrees).
pub const SPEED_BACKWARDS_MULTIPLIER: f32 = 0.65;
pub const SPEED_DBNO: f32 = 105.;
/// RECOVERED value, ASSUMED meaning (acceleration scale while strafing in sprint): NOT applied by the port.
pub const ACCELERATION_STRAFE_MULTIPLIER_SPRINT: f32 = 0.5;
/// ASSUMED = UCharacterMovementComponent default 0: MinAnalogWalkSpeed is not serialized anywhere in 1.8 (it was in the 1.10 stat row).
pub const MIN_ANALOG_WALK_SPEED: f32 = 0.;
/// RECOVERED DirectionalSpeedMultiplierCurve keys (angle between movement and facing in degrees, multiplier); all keys are RCIM_Constant (a step curve,
/// see `directional_multiplier`) and the last value holds. Use is ASSUMED.
pub const DIRECTIONAL_SPEED_CURVE: [(f32, f32); 3] = [(0., 1.), (70., 0.8), (110., 0.65)];
// ---- ground
/// RECOVERED (PlayerPawn_Athena CharMoveComp; the Generic pawn's 1000 is overridden).
pub const MAX_ACCELERATION: f32 = 750.;
/// RECOVERED (PlayerPawn_Athena CharMoveComp).
pub const GROUND_FRICTION: f32 = 6.;
/// INHERITED (PlayerPawn_Athena_Generic CharMoveComp).
pub const BRAKING_DECELERATION_WALKING: f32 = 1000.;
/// EXE (engine ctor writes 2.0 at +0x1f0; the 1.10 stat row also said 2).
pub const BRAKING_FRICTION_FACTOR: f32 = 2.;
/// ASSUMED engine default (bit field): ApplyVelocityBraking uses GroundFriction * BrakingFrictionFactor.
pub const USE_SEPARATE_BRAKING_FRICTION: bool = false;
/// ASSUMED engine default 0 (separate braking friction is unused).
pub const BRAKING_FRICTION: f32 = 0.;
/// ASSUMED engine default 1/33 s (a computed constant, not a stored immediate, so the exe scan cannot confirm it).
pub const BRAKING_SUB_STEP_TIME: f32 = 1. / 33.;
/// EXE: the CMC ctor writes the qword 0x00000000_45000000 at +0x1fc (RVA 0x2065604): BrakingDecelerationWalking 2048 and BrakingDecelerationFalling (+0x200) = 0.0 explicitly.
/// `PhysFalling` reads it through `GetMaxBrakingDeceleration` (vtable 0x670, not overridden by Fort).
pub const BRAKING_DECELERATION_FALLING: f32 = 0.;
// ---- jump / air
/// INHERITED (PlayerPawn_Athena_Generic CharMoveComp, fn-inspect props /Game/Athena/PlayerPawn_Athena_Generic). CMC ctor default 420 (RVA 0x2065454, +0x198).
pub const JUMP_Z_VELOCITY: f32 = 600.;
/// INHERITED (PlayerPawn_Athena_Generic CharMoveComp). CMC ctor default 0.05 (RVA 0x20655d1, +0x20c).
pub const AIR_CONTROL: f32 = 0.95;
/// EXE: ctor writes 2.0 at +0x210 (RVA 0x20655db) and 25.0 at +0x214 (qword store at RVA 0x20655e5, right after AirControl 0.05 at +0x20c).
pub const AIR_CONTROL_BOOST_MULTIPLIER: f32 = 2.;
pub const AIR_CONTROL_BOOST_VELOCITY_THRESHOLD: f32 = 25.;
/// EXE: the same qword store at RVA 0x20655e5 writes 0.0 to +0x218 (FallingLateralFriction; `PhysFalling` passes it to CalcVelocity at RVA 0x2088c95 `movss xmm2,[rsi+0x218]`).
pub const FALLING_LATERAL_FRICTION: f32 = 0.;
/// EXE-verified 0: `ACharacter` ctor (RVA 0x2038fa0) stores 0 at +0x494 (JumpMaxHoldTime) and 1 at +0x498 (JumpMaxCount); the AFortPawn / AFortPlayerPawn / AFortPlayerPawnAthena ctors
/// do not touch them and no pawn Blueprint serializes either (UE4.16 Character.cpp:64-66 has the same defaults). The stat `MaxJumpTime` (0.25 s, RECOVERED) is NOT mapped to it; Fortnite's jump height is not hold dependent.
/// The 1.8 `PhysFalling` (0x2088a20) has no JumpForceTimeRemaining / bApplyGravityWhileJumping branch (that is UE >= 4.17), so the hold time cannot change gravity in this port either.
pub const JUMP_MAX_HOLD_TIME: f32 = 0.;
pub const MAX_JUMP_TIME_STAT: f32 = 0.25;
/// EXE-verified 1 (see JUMP_MAX_HOLD_TIME): CanJump therefore refuses any jump while falling.
pub const JUMP_MAX_COUNT: u32 = 1;
/// NOT USED by the step: the 1.8 `PhysFalling` always applies gravity for the whole sub-step (RVA 0x2088d92 passes timeTick straight to NewFallVelocity). Kept for API compatibility.
pub const APPLY_GRAVITY_WHILE_JUMPING: bool = true;
/// EXE (UFortMovementComp_CharacterAthena ctor 0x4167b0, TArray at +0x758 filled at RVAs 0x416a9e..0x416bd5): `{JumpZ scale, speed scale}` per consecutive-jump index
/// (a jump within [`JUMP_CHAIN_WINDOW`] seconds of the previous one escalates the index, clamped to the last entry). Used by `UFortMovementComp_CharacterAthena::DoJump`
/// (0x421d20: JumpZVelocity x `.0`, then horizontal speed clamp to the movement-style speed x `.1`) and `GetMaxSpeed` (0x4343e0: speed x `.1` while falling inside the window).
pub const JUMP_CHAIN_TABLE: [(f32, f32); 5] = [(1., 1.), (1., 1.), (0.75, 0.75), (0.5, 0.5), (0.25, 0.25)];
/// EXE: Athena ctor `mov qword [rdi+0x768], 0x3f800000` (RVA 0x416be5): the window length, +0x768 = 1.0 s (+0x76c index 0). Decays by the pawn tick delta (0x41f920).
pub const JUMP_CHAIN_WINDOW: f32 = 1.;
/// EXE `UFortMovementComp_Character::DoJump` (0x6d4f20): JumpZVelocity is scaled while the pawn tick delta recorded this frame (`AFortPawn::Tick` -> setter 0x6d24a0, stored at
/// +0x750 only while `bPressedJump`) is above [`JUMP_FRAME_DT_THRESHOLD`] (0.03 s, RVA 0x14410a678): scale = min(2.5, [`JUMP_FRAME_DT_OFFSET`] + [`JUMP_FRAME_DT_SLOPE`] * dt).
/// With `bit 8 of +0x754` set the delta is first clamped to [`JUMP_FRAME_DT_CLAMP`]; nothing in the exe sets that bit (default clear).
pub const JUMP_FRAME_DT_THRESHOLD: f32 = 0.03;
pub const JUMP_FRAME_DT_CLAMP: f32 = 0.05;
pub const JUMP_FRAME_DT_SLOPE: f32 = f32::from_bits(0x40a26299); // 5.074536  (.rdata 0x41b0e44)
pub const JUMP_FRAME_DT_OFFSET: f32 = f32::from_bits(0x3f63c92b); // 0.8897883 (.rdata 0x41b0e40)
pub const JUMP_FRAME_DT_MAX_SCALE: f32 = 2.5; // .rdata 0x4078d78
/// The JumpZ multiplier `UFortMovementComp_Character::DoJump` applies for a pawn tick of `dt` seconds (1.0 at or below 0.03 s, i.e. at 60 Hz and faster).
pub fn jump_frame_scale(dt: f32) -> f32 {
    if dt > JUMP_FRAME_DT_THRESHOLD { (dt * JUMP_FRAME_DT_SLOPE + JUMP_FRAME_DT_OFFSET).min(JUMP_FRAME_DT_MAX_SCALE) } else { 1. }
}
// ---- gravity (full decode with RVAs: crates/fn-movement/FALLING.md)
//  Runtime path (all RVAs of FortniteClient-Win64-Shipping.exe, UE 4.16):
//    UFortMovementComp_CharacterAthena::GetGravityZ  0x428f00  = FortMovementComp_Character::GetGravityZ (0x6d8350) x (skydive/parachute GravityScalar, custom modes 2/3 only)
//    FortMovementComp_Character::GetGravityZ         0x6d8350  = UCharacterMovementComponent::GetGravityZ (0x207ea90) x FortMovementSet.GravityZScale (0x7de520; 1.0 without a set)
//    UCharacterMovementComponent::GetGravityZ        0x207ea90 = UMovementComponent::GetGravityZ (0x20b1ab0) x GravityScale (+0x190)
//    UMovementComponent::GetGravityZ                 0x20b1ab0 = PhysicsVolume->GetGravityZ (0x2396870) -> UWorld::GetGravityZ (0x2516d20) -> AWorldSettings::GetGravityZ (0x2516ca0)
//    AWorldSettings::GetGravityZ                     0x2516ca0 = bGlobalGravitySet ? GlobalGravityZ : UPhysicsSettings CDO.DefaultGravityZ (+0x38)
//  Athena_Terrain's FortWorldSettings serializes neither bGlobalGravitySet nor GlobalGravityZ (fn-inspect props /Game/Athena/Maps/Athena_Terrain), AFortWorldSettings does not
//  override the virtual, so the game uses `UPhysicsSettings.DefaultGravityZ`. Its exe constructor default is -980 (UPhysicsSettings ctor 0x235e770 stores -980.0f at +0x38,
//  RVA 0x235e77e); the project `Config/DefaultEngine.ini` can override it (config object) and that file is AES-encrypted, so the value the game really ran with
//  cannot be recovered from the exe or the paks.
/// UPhysicsSettings constructor default (exe RVA 0x235e77e, `mov dword [rbx+0x38], 0xc4750000`). EXE-verified; this is what the code path yields when no ini overrides it.
pub const WORLD_GRAVITY_Z_EXE_DEFAULT: f32 = -980.;
/// PROVENANCE: decrypted config, not independently verified. `[/Script/Engine.PhysicsSettings] DefaultGravityZ=-2800.000000` as read by an earlier agent from the
/// encrypted 1.8 `FortniteGame/Config/DefaultEngine.ini` (a route the project rules forbid; the value is kept, not re-derived, until allowed data replaces it). The allowed
/// sources only say: no map override (Athena_Terrain WorldSettings), engine default -980 ([`WORLD_GRAVITY_Z_EXE_DEFAULT`]). Everything downstream (GravityScale, the
/// integration, terminal velocity) is exe/pak-verified and independent of this one number.
pub const WORLD_GRAVITY_Z: f32 = -2800.;
/// EXE: the CMC ctor (0x2065290) stores 1.0 at +0x190 (GravityScale, RVA 0x2065441); no Fort/Athena constructor (0x6ceee0, 0x4167b0) and no pawn Blueprint changes it.
pub const GRAVITY_SCALE: f32 = 1.;
/// EXE: `FortMovementComp_Character::GetGravityZ` multiplies by `UFortMovementSet.GravityZScale` (getter 0x7de520 reads the attribute at set+0xd0; 1.0 when the pawn has no set).
/// The UFortMovementSet constructor (0x416dc0) initialises that attribute's base and current value to 1.0 (vcalls +0x10/+0x8 with xmm1 = 1.0 on set+0xd0, RVAs 0x416f0a / 0x416f16).
/// No CharacterAttributesPlayer row has a gravity column; the GameplayEffects only multiply it (x0.333 low gravity, x0.19 gravity sphere).
pub const GRAVITY_Z_SCALE_ATTRIBUTE: f32 = 1.;
/// EXE: `UPhysicsSettings.DefaultTerminalVelocity` constructor default 4000.0 (RVA 0x235e798, +0x3c); `ADefaultPhysicsVolume` ctor (0x213d7a0, RVA 0x213d7e1) copies it
/// into `APhysicsVolume::TerminalVelocity` (+0x3c0), which `NewFallVelocity` reads. Like gravity it is a config object, so an ini override cannot be excluded; the earlier
/// decrypted-ini value is the same 4000 (PROVENANCE for any difference: decrypted config, not independently verified).
pub const TERMINAL_VELOCITY: f32 = 4000.;
/// Effective gravity magnitude (cm/s^2).
pub const GRAVITY: f32 = -WORLD_GRAVITY_Z * GRAVITY_SCALE * GRAVITY_Z_SCALE_ATTRIBUTE;
pub const KILL_Z: f32 = -5000.; // RECOVERED Athena_Terrain FortWorldSettings KillZ
// ---- capsule / floor
pub const CAPSULE_HALF_HEIGHT: f32 = 75.; // RECOVERED (PlayerPawn_Athena_Generic CollisionCylinder)
/// ASSUMED = ACharacter ctor default (34); CollisionCylinder does not serialize a radius in any pawn Blueprint (the engine UCapsuleComponent default would be 22).
pub const CAPSULE_RADIUS: f32 = 34.;
/// EXE: engine ctor writes 40.0 at +0x21c (right before Buoyancy 1.0 at +0x220).
pub const CROUCHED_HALF_HEIGHT: f32 = 40.;
/// EXE: +0x194 = 45.0.
pub const MAX_STEP_HEIGHT: f32 = 45.;
/// ASSUMED engine value: SetWalkableFloorZ(0.71f) is a call argument, not a stored immediate (44.765 degrees).
pub const WALKABLE_FLOOR_Z: f32 = 0.71;
/// INHERITED (PlayerPawn_Athena_Generic CharMoveComp; engine default 0).
pub const PERCH_RADIUS_THRESHOLD: f32 = 20.;
/// EXE: +0x228 = 40.0.
pub const PERCH_ADDITIONAL_HEIGHT: f32 = 40.;
/// EXE: +0x330 = 4.0.
pub const LEDGE_CHECK_THRESHOLD: f32 = 4.;
pub const CAN_WALK_OFF_LEDGES: bool = true;
/// ASSUMED engine default false: a crouched pawn does not walk off a ledge.
pub const CAN_WALK_OFF_LEDGES_WHEN_CROUCHING: bool = false;
/// EXE: +0x2ec = 0.05 and +0x2f0 = 8.
pub const MAX_SIMULATION_TIME_STEP: f32 = 0.05;
pub const MAX_SIMULATION_ITERATIONS: u32 = 8;
/// EXE: +0x2f4 = 500.
pub const MAX_DEPENETRATION_WITH_GEOMETRY: f32 = 500.;

/// Apex height (cm) and airtime (s) of a first jump from the ground with the constants above at a pawn tick of 0.03 s or shorter (no frame-delta scale).
pub fn jump_apex_cm() -> f32 { JUMP_Z_VELOCITY * JUMP_Z_VELOCITY / (2. * GRAVITY) }
pub fn jump_airtime() -> f32 { 2. * JUMP_Z_VELOCITY / GRAVITY }
/// Take-off vertical speed of a first jump (chain index 0) with the pawn tick delta `dt`: JumpZVelocity x chain scale x [`jump_frame_scale`].
pub fn jump_launch_z(dt: f32) -> f32 { JUMP_Z_VELOCITY * JUMP_CHAIN_TABLE[0].0 * jump_frame_scale(dt) }
/// Apex height (cm) / airtime (s) of a first jump with the pawn tick delta `dt` (continuous closed form, no sampling).
pub fn jump_apex_cm_at(dt: f32) -> f32 { let v = jump_launch_z(dt); v * v / (2. * GRAVITY) }
pub fn jump_airtime_at(dt: f32) -> f32 { 2. * jump_launch_z(dt) / GRAVITY }
/// Directional multiplier: a STEP function. Every key of the 1.8 `DirectionalSpeedMultiplierCurve` has `InterpMode` 1 = RCIM_Constant (UE 4.16
/// `ERichCurveInterpMode`: Linear 0, Constant 1, Cubic 2; re-read by tools/verify-1.8.cjs), so `FRichCurve::Eval` returns the value of the last key
/// at or before the angle: 1.0 below 70 degrees, 0.8 from 70 to under 110, 0.65 from 110 on. (Earlier versions interpolated linearly, which gave 0.875
/// at a 45 degree diagonal instead of the recovered full speed.)
pub fn directional_multiplier(deg: f32) -> f32 {
    let a = deg.abs(); let k = DIRECTIONAL_SPEED_CURVE; // RCIM_Constant
    k.iter().rev().find(|key| a >= key.0).map_or(k[0].1, |key| key.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report() -> serde_json::Value { serde_json::from_str(&fn_testdata::report_text("movement-full.json")).unwrap() }
    fn entry<'a>(r: &'a serde_json::Value, name: &str) -> &'a serde_json::Value { r["values"].as_array().unwrap().iter().find(|v| v["name"] == name).unwrap_or_else(|| panic!("{name} missing from movement-full.json")) }
    fn get(r: &serde_json::Value, name: &str) -> f64 { entry(r, name)["value"].as_f64().unwrap_or_else(|| panic!("{name} is not numeric")) }
    fn status(r: &serde_json::Value, name: &str) -> String { entry(r, name)["status"].as_str().unwrap().to_owned() }
    /// 1.8: the pawn Blueprint carries JumpZ/MaxAcceleration/Braking/AirControl/GroundFriction/PerchRadiusThreshold, the stat row only the speeds.
    #[test] fn constants_match_movement_full_json() {
        let r = report(); let row = |n: &str| get(&r, &format!("PlayerLoadout_AthenaPlayer.{n}"));
        for (n, c) in [("SpeedWalk", SPEED_WALK), ("SpeedRun", SPEED_RUN), ("SpeedSprint", SPEED_SPRINT), ("SpeedCrouchedRun", SPEED_CROUCHED_RUN), ("SpeedCrouchedSprint", SPEED_CROUCHED_SPRINT), ("SpeedDBNO", SPEED_DBNO)] { assert_eq!(row(n) as f32, c, "{n}"); }
        for (n, c) in [("SpeedBackwardsMultiplier", SPEED_BACKWARDS_MULTIPLIER), ("AccelerationStrafeMultiplierSprint", ACCELERATION_STRAFE_MULTIPLIER_SPRINT), ("MaxJumpTime", MAX_JUMP_TIME_STAT)] { assert!((row(n) as f32 - c).abs() < 1e-6, "{n}"); }
        for (n, c) in [("JumpZVelocity", JUMP_Z_VELOCITY), ("MaxAcceleration", MAX_ACCELERATION), ("BrakingDecelerationWalking", BRAKING_DECELERATION_WALKING), ("GroundFriction", GROUND_FRICTION), ("PerchRadiusThreshold", PERCH_RADIUS_THRESHOLD)] { assert_eq!(get(&r, n) as f32, c, "{n}"); }
        assert!((get(&r, "AirControl") as f32 - AIR_CONTROL).abs() < 1e-6);
        assert_eq!(get(&r, "WorldSettings.KillZ") as f32, KILL_Z);
        assert_eq!(get(&r, "CollisionCylinder.CapsuleHalfHeight") as f32, CAPSULE_HALF_HEIGHT);
        // The statuses must say where each value comes from.
        assert_eq!(status(&r, "MaxAcceleration"), "recovered"); assert_eq!(status(&r, "GroundFriction"), "recovered");
        assert_eq!(status(&r, "JumpZVelocity"), "inherited"); assert_eq!(status(&r, "AirControl"), "inherited");
    }
    /// Native defaults read from the 1.8 exe's constructor immediates (movement-full.json `exeCtor`).
    #[test] fn exe_constructor_defaults_match_the_report() {
        let r = report();
        for (n, c) in [("CMC.GravityScale", GRAVITY_SCALE), ("CMC.MaxStepHeight", MAX_STEP_HEIGHT), ("CMC.CrouchedHalfHeight", CROUCHED_HALF_HEIGHT), ("CMC.PerchAdditionalHeight", PERCH_ADDITIONAL_HEIGHT),
            ("CMC.BrakingFrictionFactor", BRAKING_FRICTION_FACTOR), ("CMC.AirControlBoostMultiplier", AIR_CONTROL_BOOST_MULTIPLIER), ("CMC.AirControlBoostVelocityThreshold", AIR_CONTROL_BOOST_VELOCITY_THRESHOLD),
            ("CMC.LedgeCheckThreshold", LEDGE_CHECK_THRESHOLD), ("CMC.MaxSimulationTimeStep", MAX_SIMULATION_TIME_STEP), ("CMC.MaxDepenetrationWithGeometry", MAX_DEPENETRATION_WITH_GEOMETRY)] {
            assert!((get(&r, n) as f32 - c).abs() < 1e-6, "{n}"); assert_eq!(status(&r, n), "exe-verified", "{n}");
        }
        assert_eq!(get(&r, "CMC.MaxSimulationIterations") as u32, MAX_SIMULATION_ITERATIONS);
        // The engine JumpZVelocity default (420) is NOT used: the Athena pawn overrides it with 600.
        assert_eq!(get(&r, "CMC.JumpZVelocity(engine default, overridden)") as f32, 420.);
    }
    /// Gravity is the 1.8 project setting DefaultGravityZ -2800 (DefaultEngine.ini); the old report only knew the engine
    /// constructor default (-980) but it agrees that no WorldSettings override exists.
    #[test] fn gravity_is_the_recovered_project_setting() {
        let r = report();
        assert!((GRAVITY - 2800.).abs() < 1e-3);
        // v^2 / 2g = 64.29 cm, 2v / g = 0.4286 s
        assert!((jump_apex_cm() - 64.2857).abs() < 0.01); assert!((jump_airtime() - 0.428571).abs() < 1e-3);
        assert_eq!(JUMP_MAX_COUNT, 1);
        assert_eq!(status(&r, "WorldSettings.WorldGravityZ"), "engine-default-assumed"); // WorldSettings has no override: the project setting applies
    }
    #[test] fn directional_curve_is_a_step_function() { assert_eq!(directional_multiplier(0.), 1.); assert_eq!(directional_multiplier(35.), 1.); assert_eq!(directional_multiplier(45.), 1.); assert_eq!(directional_multiplier(69.9), 1.); assert_eq!(directional_multiplier(70.), 0.8); assert_eq!(directional_multiplier(109.9), 0.8); assert_eq!(directional_multiplier(110.), 0.65); assert_eq!(directional_multiplier(180.), 0.65); assert_eq!(directional_multiplier(-70.), 0.8); assert_eq!(directional_multiplier(-45.), 1.); }
}
