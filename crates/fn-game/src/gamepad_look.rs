//! Gamepad (right stick) look of Fortnite 1.8 Battle Royale, ported from the 1.8 exe (decompile `C:\FortniteRef\decomp\named`) and the cooked
//! `/Game/Gamepad/AthenaGamepadSettings` data asset (pakchunk0, unencrypted). The numbers are loaded at run time ([`GamepadSettings::load`]);
//! only the native class defaults the asset does not override come from the exe constructor.
//!
//! The chain, per frame (UE convention: `AddYawInput` value x `InputYawScale` 2.5 = degrees right, `AddPitchInput` value x `InputPitchScale` -1.75
//! = degrees up, CookedIniVersion.txt L3167 / L3168):
//! 1. Stick dead zones: [`crate::inputdev::pad::stick`] (`ApplyGamepadStickDeadzone` FUN_1405fb290 + AxisConfig DeadZone 0.25).
//! 2. `UFortPlayerInputAthena::vfunc_71` @1404392c0 (MassageAxisInput) for Gamepad_RightX / Gamepad_RightY (keys DAT_14678f788 / DAT_14678f7a0, the
//!    pair `FUN_140615c60` runs through the right-stick dead zone): x `GamepadTargetingMultiplier` (+0x4fc) while the pawn targets (+0x104b) and the
//!    weapon is not using its scope, x `GamepadScopedMultiplier` (+0x500) while it is (`AFortWeaponRanged::IsUsingScope`), then x `GetFOVAngle() *
//!    GamepadFOVScale` (+0x504 = 0.015, ctor @140417450) because `bEnableFOVScaling` is on (ini L1972). See [`massage_axis`].
//!    UE negates Gamepad_RightY before the player input sees it (UE 4.16 SceneViewport.cpp:964), so stick up is negative from here on.
//! 3. Axis mappings `Turn_Gamepad` = Gamepad_RightX x1 and `LookUp_Gamepad` = Gamepad_RightY x1 (ini L2000 / L2002); Invert View inverts
//!    `LookUp_Gamepad` along with `Lookup` (`UPlayerInput::InvertAxis`, FUN_14053f560). The bound handlers (pawn FUN_1407db780 / FUN_1407db6d0,
//!    controller FUN_14041cf40 / FUN_14041cea0) clamp the value to [-1, 1] and store it in UFortPlayerInput +0x3dc / +0x3e0 (only when non-zero).
//!    See [`stick_axes`].
//! 4. `FUN_140615dd0` once per tick ([`GamepadLook::tick`]): with `gpad.GamepadProcessingEnabled` (DAT_145181b38, default 1, FUN_14009f1c0) and a
//!    settings asset, yaw input = `GamepadLookScale.X` x ADS / edit scale x `YawCurve(x)` x `SensitivityCurve(GamepadLookSensitivity.X)` x
//!    (1 - aim-assist slow) x acceleration x dt x 60 (DAT_143fe3728) + aim-assist pull; pitch the same with `.Y` / `PitchCurve`; then
//!    `AddYawInput` / `AddPitchInput` (vtable +0xb18 / +0xb10 = @1423895a0 / @142388900). Without the asset the stored axis goes straight to them.
//!    Curves: `FUN_1405cc960` (odd-symmetric, input clamped to [-1, 1]) on UE 4.16 `FRichCurve::Eval` ([`RichCurve::eval`]).
//!    Acceleration: `FUN_1406233c0` ([`GamepadLook::accel_step`]). Aim assist: `FUN_140620df0` ([`GamepadLook::aim_assist`]).
//!
//! Not ported (and why): the target-dependent half of aim assist (target selection FUN_1406214d0 with `AimAssistTargetSelection`, the pull
//! vectors FUN_14060df50 / FUN_14060ed20 / FUN_14060e970 and the slow limiter that uses them) needs other pawns, and the game has none (no pawn
//! list in fn-app); without a target the exe path gives slow 0 and pull 0, which is what runs here. The edit-mode assist FUN_140621820 (6.6 KB,
//! edit-tile snapping) and its exit hook FUN_1406184c0 are not ported either: in edit mode only the scale and acceleration-time switches apply.
use crate::inputdev::pad;

/// `KINDA_SMALL_NUMBER` (DAT_143ff9010 = 1e-4): the stick counts as moved above this.
pub const STICK_EPS: f32 = 1e-4;
/// `UFortPlayerInputAthena` +0x504 `GamepadFOVScale` (ctor @140417450: 0x3c75c28f).
pub const GAMEPAD_FOV_SCALE: f32 = 0.015;
/// `PlayerController:InputYawScale` / `InputPitchScale` (CookedIniVersion.txt L3167 / L3168).
pub const INPUT_YAW_SCALE: f32 = 2.5;
pub const INPUT_PITCH_SCALE: f32 = -1.75;
/// The `dt x 60` of FUN_140615dd0 (DAT_143fe3728 = 60.0).
pub const FRAME_SCALE: f32 = 60.;
pub const ASSET_PATH: &str = "/Game/Gamepad/AthenaGamepadSettings";

/// One `FRichCurveKey` (UE 4.16): `interp` 0 linear, 1 constant, 2 cubic, 3 none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveKey { pub time: f32, pub value: f32, pub arrive: f32, pub leave: f32, pub interp: u8 }
/// `ERichCurveExtrapolation`: 0 cycle, 1 cycle with offset, 2 oscillate, 3 linear, 4 constant (the default), 5 none.
#[derive(Clone, Debug, PartialEq)]
pub struct RichCurve { pub keys: Vec<CurveKey>, pub pre: u8, pub post: u8, pub default_value: Option<f32> }
const RCCE_CYCLE_WITH_OFFSET: u8 = 1;
const RCCE_OSCILLATE: u8 = 2;
const RCCE_LINEAR: u8 = 3;
const RCCE_CONSTANT: u8 = 4;
fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t }
/// `BezierInterp` (RichCurve.cpp:905).
fn bezier(p0: f32, p1: f32, p2: f32, p3: f32, a: f32) -> f32 {
    let (p01, p12, p23) = (lerp(p0, p1, a), lerp(p1, p2, a), lerp(p2, p3, a));
    lerp(lerp(p01, p12, a), lerp(p12, p23, a), a)
}
impl RichCurve {
    pub fn new(keys: Vec<CurveKey>) -> RichCurve { RichCurve { keys, pre: RCCE_CONSTANT, post: RCCE_CONSTANT, default_value: None } }
    /// `CycleTime` (RichCurve.cpp:932).
    fn cycle_time(min: f32, max: f32, t: &mut f32) -> i32 {
        let (init, dur) = (*t, max - min);
        let mut n = 0;
        if *t > max { n = ((max - *t) / dur).floor() as i32; *t += dur * n as f32; } else if *t < min { n = ((*t - min) / dur).floor() as i32; *t -= dur * n as f32; }
        if *t == max && init < min { *t = min; }
        if *t == min && init > max { *t = max; }
        n.abs()
    }
    /// `FRichCurve::RemapTimeValue` (RichCurve.cpp:962): the cycle / oscillate extrapolations.
    fn remap(&self, t: &mut f32) -> f32 {
        let k = &self.keys;
        if k.len() < 2 { return 0.; }
        let (first, last) = (k[0], k[k.len() - 1]);
        let pre_side = *t <= first.time;
        let mode = if pre_side { self.pre } else if *t >= last.time { self.post } else { return 0. };
        if mode == RCCE_LINEAR || mode == RCCE_CONSTANT { return 0.; }
        let n = Self::cycle_time(first.time, last.time, t);
        if mode == RCCE_CYCLE_WITH_OFFSET { return if pre_side { (first.value - last.value) * n as f32 } else { (last.value - first.value) * n as f32 }; }
        if mode == RCCE_OSCILLATE && n % 2 == 1 { *t = first.time + (last.time - *t); }
        0.
    }
    /// `FRichCurve::Eval(InTime, InDefaultValue = 0)` of UE 4.16 (RichCurve.cpp:1022), as `UCurveFloat::GetFloatValue` calls it.
    pub fn eval(&self, time: f32) -> f32 {
        let mut t = time;
        let offset = self.remap(&mut t);
        let k = &self.keys;
        let n = k.len();
        let mut v = self.default_value.unwrap_or(0.);
        if n == 0 {
        } else if n < 2 || t <= k[0].time {
            v = if self.pre == RCCE_LINEAR && n > 1 {
                let dt = k[1].time - k[0].time;
                if dt.abs() <= 1e-8 { k[0].value } else { (k[1].value - k[0].value) / dt * (t - k[0].time) + k[0].value }
            } else { k[0].value };
        } else if t < k[n - 1].time {
            // lower bound for the second key of the segment
            let (mut first, mut count) = (1usize, n - 2);
            while count > 0 { let step = count / 2; let mid = first + step; if t >= k[mid].time { first = mid + 1; count -= step + 1; } else { count = step; } }
            let (a, b) = (k[first - 1], k[first]);
            let diff = b.time - a.time;
            v = if diff > 0. && a.interp != 1 {
                let alpha = (t - a.time) / diff;
                if a.interp == 0 { lerp(a.value, b.value, alpha) } else { bezier(a.value, a.value + a.leave * diff * (1. / 3.), b.value - b.arrive * diff * (1. / 3.), b.value, alpha) }
            } else { a.value };
        } else {
            v = if self.post == RCCE_LINEAR {
                let dt = k[n - 2].time - k[n - 1].time;
                if dt.abs() <= 1e-8 { k[n - 1].value } else { (k[n - 2].value - k[n - 1].value) / dt * (t - k[n - 1].time) + k[n - 1].value }
            } else { k[n - 1].value };
        }
        v + offset
    }
}

/// `UFortGamepadSettings` (1.8 SDK layout, 0x130 bytes): the fields the look and aim-assist code reads.
#[derive(Clone, Debug, PartialEq)]
pub struct GamepadSettings {
    /// +0x28 / +0x30 / +0x38 (`UCurveFloat`s; None = null pointer).
    pub yaw_curve: Option<RichCurve>, pub pitch_curve: Option<RichCurve>, pub sensitivity_curve: Option<RichCurve>,
    /// +0x40 `GamepadLookScale`, +0x48 `GamepadLookScaleDownsights`, +0x4c `GamepadLookScaleScope`, +0x50 `GamepadLookScaleEditMode`, +0x54 `GamepadLookScaleDownsightsDecayTime`.
    pub look_scale: [f32; 2], pub scale_downsights: f32, pub scale_scope: f32, pub scale_edit_mode: f32, pub downsights_decay_time: f32,
    /// +0x58 `AimAssistStrength`, +0x5c `AimAssistStrengthDownsights`.
    pub aim_assist_strength: f32, pub aim_assist_strength_downsights: f32,
    /// +0x60 `GamepadLookAccelTime`, +0x68 `GamepadEditModeLookAccelTime`, +0x70 `GamepadLookDecelTime`.
    pub accel_time: [f32; 2], pub edit_accel_time: [f32; 2], pub decel_time: [f32; 2],
    /// +0x78 `AimAssistPullStrength`, +0x7c `AimAssistPullMaxRate`, +0x80 `AimAssistPullRampUpTime`, +0x84 `AimAssistPullDecayTime`.
    pub pull_strength: f32, pub pull_max_rate: f32, pub pull_ramp_up_time: f32, pub pull_decay_time: f32,
    /// +0x88 `AimAssistInitialDownsightStrength`, +0x8c `AimAssistInitialDownsightTime`.
    pub initial_downsight_strength: f32, pub initial_downsight_time: f32,
    /// +0x90..+0xa0 `SlowMinDistance`, `SlowMinStrength`, `SlowMaxDistance`, `SlowMaxStrength`, `SlowDecayTime`.
    pub slow_min_distance: f32, pub slow_min_strength: f32, pub slow_max_distance: f32, pub slow_max_strength: f32, pub slow_decay_time: f32,
}
impl GamepadSettings {
    /// The native class defaults: `UFortGamepadSettings::UFortGamepadSettings` @1405b49e0 (DAT_143fe36fc = 1.0, DAT_144008e28 = 0.2,
    /// DAT_1441261fc = 0.15, DAT_143fe36f0 = 0.1). The asset overrides some of them.
    pub fn native_defaults() -> GamepadSettings {
        GamepadSettings { yaw_curve: None, pitch_curve: None, sensitivity_curve: None,
            look_scale: [1., 1.], scale_downsights: 0.5, scale_scope: 0.3, scale_edit_mode: 1., downsights_decay_time: 0.5,
            aim_assist_strength: 0.5, aim_assist_strength_downsights: 1., accel_time: [0.2, 0.2], edit_accel_time: [0.15, 0.15], decel_time: [0.1, 0.1],
            pull_strength: 0.05, pull_max_rate: 60., pull_ramp_up_time: 0.3, pull_decay_time: 0.5, initial_downsight_strength: 0.1, initial_downsight_time: 0.2,
            slow_min_distance: 100., slow_min_strength: 0.4, slow_max_distance: 1500., slow_max_strength: 0.7, slow_decay_time: 1. }
    }
    /// The cooked `AthenaGamepadSettings` (and the curve assets it references) from the paks.
    pub fn load(set: &fn_pak::PakSet) -> Result<GamepadSettings, String> {
        let data = fn_uasset::PackageData::load(set, ASSET_PATH).map_err(|e| format!("{ASSET_PATH}: {e}"))?;
        let pkg = data.package().map_err(|e| format!("{ASSET_PATH}: {e}"))?;
        let i = pkg.exports_of_class("FortGamepadSettings").next().ok_or_else(|| format!("{ASSET_PATH}: no FortGamepadSettings export"))?;
        let obj = pkg.read_export(i).map_err(|e| format!("{ASSET_PATH}: {e}"))?;
        let mut s = GamepadSettings::native_defaults();
        let f = |name: &str, d: &mut f32| if let Some(v) = obj.get(name).and_then(|v| v.as_f32()) { *d = v; };
        let v2 = |name: &str, d: &mut [f32; 2]| match obj.get(name) {
            Some(fn_uasset::Value::Vector2D(v)) => *d = *v,
            Some(v) => if let (Some(x), Some(y)) = (v.field("X").and_then(|x| x.as_f32()), v.field("Y").and_then(|y| y.as_f32())) { *d = [x, y]; },
            None => {}
        };
        v2("GamepadLookScale", &mut s.look_scale); f("GamepadLookScaleDownsights", &mut s.scale_downsights); f("GamepadLookScaleScope", &mut s.scale_scope);
        f("GamepadLookScaleEditMode", &mut s.scale_edit_mode); f("GamepadLookScaleDownsightsDecayTime", &mut s.downsights_decay_time);
        f("AimAssistStrength", &mut s.aim_assist_strength); f("AimAssistStrengthDownsights", &mut s.aim_assist_strength_downsights);
        v2("GamepadLookAccelTime", &mut s.accel_time); v2("GamepadEditModeLookAccelTime", &mut s.edit_accel_time); v2("GamepadLookDecelTime", &mut s.decel_time);
        f("AimAssistPullStrength", &mut s.pull_strength); f("AimAssistPullMaxRate", &mut s.pull_max_rate); f("AimAssistPullRampUpTime", &mut s.pull_ramp_up_time); f("AimAssistPullDecayTime", &mut s.pull_decay_time);
        f("AimAssistInitialDownsightStrength", &mut s.initial_downsight_strength); f("AimAssistInitialDownsightTime", &mut s.initial_downsight_time);
        f("SlowMinDistance", &mut s.slow_min_distance); f("SlowMinStrength", &mut s.slow_min_strength); f("SlowMaxDistance", &mut s.slow_max_distance);
        f("SlowMaxStrength", &mut s.slow_max_strength); f("SlowDecayTime", &mut s.slow_decay_time);
        let curve = |name: &str| -> Result<Option<RichCurve>, String> {
            let Some(idx) = obj.get(name).and_then(|v| v.as_object()) else { return Ok(None) };
            if idx.is_null() { return Ok(None); }
            let path = pkg.object_path(idx);
            load_curve(set, path.split('.').next().unwrap_or(&path)).map(Some)
        };
        s.yaw_curve = curve("GamepadLookYawCurve")?; s.pitch_curve = curve("GamepadLookPitchCurve")?; s.sensitivity_curve = curve("GamepadLookSensitivityCurve")?;
        Ok(s)
    }
    /// `FUN_1405cc960`: (pitch, yaw) = (PitchCurve(y), YawCurve(x)), each input clamped to [-1, 1] and evaluated odd-symmetrically
    /// (a negative input evaluates the curve at its magnitude and negates the result); a null curve gives 0.
    pub fn look_curves(&self, x: f32, y: f32) -> (f32, f32) {
        let sym = |c: &Option<RichCurve>, v: f32| c.as_ref().map_or(0., |c| { let v = v.clamp(-1., 1.); if v >= 0. { c.eval(v) } else { -c.eval(-v) } });
        (sym(&self.pitch_curve, y), sym(&self.yaw_curve, x))
    }
    /// `FUN_1405cca50` (stored at UFortPlayerInput +0x404 by FUN_140619a20 when the settings are applied): the sensitivity curve at the
    /// `GamepadLookSensitivity` setting per axis; (1, 1) without the curve.
    pub fn sensitivity(&self, sens: [f32; 2]) -> [f32; 2] {
        match &self.sensitivity_curve { Some(c) => [c.eval(sens[0]), c.eval(sens[1])], None => [1., 1.] }
    }
    /// `FUN_1405d6db0`: aim-assist slow strength for a target at `distance` (cm): `MapRangeClamped(distance, SlowMinDistance, SlowMaxDistance,
    /// SlowMinStrength, SlowMaxStrength)`, 0 when the distance range is inverted.
    pub fn slow_strength(&self, distance: f32) -> f32 {
        let (a, b) = (self.slow_min_distance, self.slow_max_distance);
        if b < a { return 0.; }
        let t = if b == a { if distance >= b { 1. } else { 0. } } else { ((distance - a) / (b - a)).clamp(0., 1.) };
        lerp(self.slow_min_strength, self.slow_max_strength, t)
    }
}

/// One UE 4.16 `UCurveFloat` package (`/Game/...`): its `FloatCurve`.
pub fn load_curve(set: &fn_pak::PakSet, path: &str) -> Result<RichCurve, String> {
    let data = fn_uasset::PackageData::load(set, path).map_err(|e| format!("{path}: {e}"))?;
    let pkg = data.package().map_err(|e| format!("{path}: {e}"))?;
    let i = pkg.exports_of_class("CurveFloat").next().ok_or_else(|| format!("{path}: no CurveFloat export"))?;
    let obj = pkg.read_export(i).map_err(|e| format!("{path}: {e}"))?;
    let fc = obj.get("FloatCurve").ok_or_else(|| format!("{path}: no FloatCurve"))?;
    let byte = |v: Option<&fn_uasset::Value>, names: &[&str], default: u8| -> u8 {
        match v {
            Some(fn_uasset::Value::Enum { value, .. }) | Some(fn_uasset::Value::Name(value)) => { let n = value.rsplit("::").next().unwrap_or(value); names.iter().position(|x| *x == n).map_or(default, |p| p as u8) }
            Some(v) => v.as_i64().map_or(default, |b| b as u8),
            None => default,
        }
    };
    const INTERP: &[&str] = &["RCIM_Linear", "RCIM_Constant", "RCIM_Cubic", "RCIM_None"];
    const EXTRAP: &[&str] = &["RCCE_Cycle", "RCCE_CycleWithOffset", "RCCE_Oscillate", "RCCE_Linear", "RCCE_Constant", "RCCE_None"];
    let keys = fc.field("Keys").and_then(|k| k.as_array()).ok_or_else(|| format!("{path}: no FloatCurve.Keys"))?;
    let keys = keys.iter().map(|k| Some(CurveKey { time: k.field("Time")?.as_f32()?, value: k.field("Value")?.as_f32()?,
        arrive: k.field("ArriveTangent").and_then(|v| v.as_f32()).unwrap_or(0.), leave: k.field("LeaveTangent").and_then(|v| v.as_f32()).unwrap_or(0.),
        interp: byte(k.field("InterpMode"), INTERP, 0) })).collect::<Option<Vec<_>>>().ok_or_else(|| format!("{path}: unreadable key"))?;
    Ok(RichCurve { keys, pre: byte(fc.field("PreInfinityExtrap"), EXTRAP, RCCE_CONSTANT), post: byte(fc.field("PostInfinityExtrap"), EXTRAP, RCCE_CONSTANT),
        default_value: fc.field("DefaultValue").and_then(|v| v.as_f32()).filter(|v| *v != f32::MAX) })
}

/// `vfunc_71` for Gamepad_RightX / RightY after the AxisConfig dead zone: x the gamepad targeting (+0x4fc) or scoped (+0x500) multiplier while the
/// pawn targets / the weapon uses its scope, then x `fov * GamepadFOVScale` (no FOV term when `fov` <= 0, like the exe without a camera manager).
pub fn massage_axis(v: f32, targeting: bool, using_scope: bool, targeting_mult: f32, scoped_mult: f32, fov: f32) -> f32 {
    let v = if using_scope { v * scoped_mult } else if targeting { v * targeting_mult } else { v };
    if fov > 0. { v * fov * GAMEPAD_FOV_SCALE } else { v }
}

/// What the look tick needs from the rest of the game this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LookContext {
    /// Camera FOV (`PlayerCameraManager::GetFOVAngle`), degrees.
    pub fov: f32,
    /// `bIsTargeting` (+0x104b) and the scope state of the held weapon (`IsUsingScope` for vfunc_71; flag bit 3 of +0xabc in FUN_140615dd0, the
    /// scope-targeting weapon: the port uses `bUseScopeTargeting` while targeting for both, as `AdsMachine::look_scale` does).
    pub targeting: bool, pub scope_weapon: bool,
    /// `EditModeInputComponent` is on the input stack (FUN_140834ae0): building edit mode.
    pub editing: bool,
    /// `APawn::GetLastMovementInputVector` is non-zero.
    pub moving: bool,
    /// `GamepadTargetingMultiplier` / `GamepadScopedMultiplier` (`UFortClientSettingsRecord` GamepadTargetingSlow / GamepadScopedSlow, stored unscaled).
    pub targeting_mult: f32, pub scoped_mult: f32,
    /// `GamepadLookSensitivity` (1..10; `UFortClientSettingsRecord` +0xd4, default 5 from the ctor @140539c00).
    pub sensitivity: [f32; 2],
    /// Invert View (`InvertAxis("LookUp_Gamepad")`).
    pub invert: bool,
}

/// Steps 1-3: the values the `Turn_Gamepad` / `LookUp_Gamepad` handlers store (+0x3dc, +0x3e0) for a raw XInput right stick. `y` is UE's
/// LookUp_Gamepad value: negative for stick up (SceneViewport flips Gamepad_RightY), positive again with Invert View.
pub fn stick_axes(rx: i16, ry: i16, c: &LookContext) -> [f32; 2] {
    let (x, y) = pad::stick(rx, ry, pad::RIGHT_DEADZONE);
    let using_scope = c.targeting && c.scope_weapon;
    let x = massage_axis(x, c.targeting, using_scope, c.targeting_mult, c.scoped_mult, c.fov);
    let y = massage_axis(-y, c.targeting, using_scope, c.targeting_mult, c.scoped_mult, c.fov);
    let y = if c.invert { -y } else { y };
    [x.clamp(-1., 1.), y.clamp(-1., 1.)]
}

/// The per-player state of `UFortPlayerInput` that the look tick keeps (offsets of the 1.8 object).
#[derive(Clone, Debug, PartialEq)]
pub struct GamepadLook {
    /// +0x3c8 `bApplyGamepadDownsightsLookScale`: 1 in the UFortPlayerInput ctor @1405f7a60, cleared by the UFortPlayerInputAthena ctor @140417450
    /// (`param_1 + 0x79 = 0`), and nothing sets it again: Athena never applies `GamepadLookScaleDownsights` / `Scope`.
    pub apply_downsights_scale: bool,
    /// +0x414 aim assist on (`UFortClientSettingsRecord` +0x160, 1 in its ctor @140539c00).
    pub aim_assist_enabled: bool,
    /// +0x40c / +0x410 acceleration per axis (0..1).
    pub accel: [f32; 2],
    /// +0x43c targeting start time / +0x440 targeting end time (-1 = unset, ctor).
    pub targeting_start: f32, pub targeting_end: f32,
    /// +0x434 slow, +0x438 previous slow, +0x430 pull strength, +0x424 / +0x428 pull (pitch, yaw), +0x420 has target.
    pub slow: f32, pub prev_slow: f32, pub pull_strength: f32, pub pull: [f32; 2], pub has_target: bool,
    /// +0x49f was in edit mode last tick.
    pub was_editing: bool,
    /// World time (seconds) for the start / end stamps.
    pub now: f32,
}
impl Default for GamepadLook {
    fn default() -> Self {
        GamepadLook { apply_downsights_scale: false, aim_assist_enabled: true, accel: [0., 0.], targeting_start: -1., targeting_end: -1., slow: 0., prev_slow: 0., pull_strength: 0., pull: [0., 0.], has_target: false, was_editing: false, now: 0. }
    }
}
/// Result of one tick: the `AddYawInput` / `AddPitchInput` values and what they turn the view by.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LookOut { pub yaw_input: f32, pub pitch_input: f32, pub yaw_deg: f32, pub pitch_up_deg: f32 }

impl GamepadLook {
    /// `FUN_1406233c0`: per axis, ramp up by dt / AccelTime (edit mode: EditModeLookAccelTime) while the stick is moved, down by dt / DecelTime when not.
    pub fn accel_step(&mut self, s: &GamepadSettings, axes: [f32; 2], editing: bool, dt: f32) -> [f32; 2] {
        let t = if editing { s.edit_accel_time } else { s.accel_time };
        let [mut ax, mut ay] = self.accel;
        if axes[0].abs() > STICK_EPS || axes[1].abs() > STICK_EPS {
            ax = if t[0] <= 0. { 1. } else { let v = ax + dt / t[0]; if v > 1. { 1. } else { v } };
            ay = if t[1] > 0. { let v = ay + dt / t[1]; if v <= 1. { v } else { 1. } } else { 1. };
        } else {
            let d = s.decel_time;
            ax = if d[0] <= 0. { 0. } else { let v = ax - dt / d[0]; if v < 0. { 0. } else { v } };
            ay = if d[1] > 0. { let v = ay - dt / d[1]; if v >= 0. { v } else { 0. } } else { 0. };
        }
        self.accel = [ax, ay];
        self.accel
    }
    /// `FUN_140620df0` without a target (see the module docs): the slow is reset and only decays from its previous value, the pull is zero and the pull
    /// strength decays. `moved` = stick or movement input this frame.
    pub fn aim_assist(&mut self, s: &GamepadSettings, c: &LookContext, axes: [f32; 2], dt: f32) {
        self.prev_slow = self.slow;
        self.slow = 0.;
        if !self.aim_assist_enabled { return; }
        self.has_target = false; // FUN_1406214d0 (target selection): no other pawns in this game
        self.pull = [0., 0.];
        if self.has_target {
            self.pull_strength = if s.pull_ramp_up_time <= 0. { s.pull_strength } else { (self.pull_strength + dt / s.pull_ramp_up_time).min(s.pull_strength) };
        } else {
            self.pull_strength = if s.pull_decay_time <= 0. { 0. } else { (self.pull_strength - dt / s.pull_decay_time).max(0.) };
        }
        let _moved = axes[0].abs() > STICK_EPS || axes[1].abs() > STICK_EPS || c.moving;
        let _initial = c.targeting && s.initial_downsight_time > self.now - self.targeting_start;
        // (moved || initial) && target valid -> strength x pull vectors (FUN_14060df50 / ed20 / e970), slow_strength(distance), slow limiter: needs a target
        if s.slow_decay_time > 0. && self.slow < self.prev_slow { self.slow = (self.prev_slow - dt / s.slow_decay_time).max(self.slow); }
    }
    /// `FUN_140615dd0`. `axes` = the stored +0x3dc / +0x3e0 values ([`stick_axes`]); `settings` None = no settings asset (or
    /// `gpad.GamepadProcessingEnabled` 0): the axes go to AddYawInput / AddPitchInput unchanged.
    pub fn tick(&mut self, settings: Option<&GamepadSettings>, c: &LookContext, axes: [f32; 2], dt: f32) -> LookOut {
        self.now += dt;
        let Some(s) = settings else { return out(axes[0], axes[1]) };
        let (pitch_c, yaw_c) = s.look_curves(axes[0], axes[1]);
        let mut k = 1.;
        if self.apply_downsights_scale { k = if c.scope_weapon { s.scale_scope } else { s.scale_downsights }; }
        if !c.targeting {
            self.targeting_start = -1.;
            if self.targeting_end <= 0. { self.targeting_end = self.now; }
            let t = (self.now - self.targeting_end) / s.downsights_decay_time;
            let t = if t >= 0. { if t >= 1. { 1. } else { t } } else { 0. };
            k = (1. - k) * t + k;
        } else {
            self.targeting_end = -1.;
            if self.targeting_start <= 0. { self.targeting_start = self.now; }
        }
        let (mut sx, mut sy) = (s.look_scale[0] * k, s.look_scale[1] * k);
        let accel = self.accel_step(s, axes, c.editing, dt);
        if c.editing {
            self.was_editing = true;
            sx *= s.scale_edit_mode; sy *= s.scale_edit_mode; // + FUN_140621820 (edit-mode assist, not ported)
        } else {
            self.was_editing = false; // (FUN_1406184c0 on leaving edit mode, not ported)
            self.aim_assist(s, c, axes, dt);
        }
        let sens = s.sensitivity(c.sensitivity);
        let keep = 1. - self.slow;
        let yaw = sx * yaw_c * sens[0] * keep * accel[0] * dt * FRAME_SCALE + self.pull[1];
        let pitch = sy * pitch_c * sens[1] * keep * accel[1] * dt * FRAME_SCALE + self.pull[0];
        out(yaw, pitch)
    }
}
fn out(yaw: f32, pitch: f32) -> LookOut { LookOut { yaw_input: yaw, pitch_input: pitch, yaw_deg: yaw * INPUT_YAW_SCALE, pitch_up_deg: pitch * INPUT_PITCH_SCALE } }

#[cfg(test)]
mod tests {
    use super::*;
    fn key(time: f32, value: f32, arrive: f32, leave: f32) -> CurveKey { CurveKey { time, value, arrive, leave, interp: 2 } }
    /// The keys of `/Game/Gamepad/GamepadLookCurve` and `GamepadLookSensitivitySettingCurve` as cooked (dump_exports).
    fn look_curve() -> RichCurve { RichCurve::new(vec![key(0., 0., 0.9043032, 0.), key(0.55, 0.3, 1., 1.), key(1., 1., 2.5275137, 13.313529)]) }
    fn sens_curve() -> RichCurve { RichCurve::new(vec![key(0., 0.05, 0.9043032, 0.), key(1., 0.25, 0.45934895, 0.17257269), key(5., 1., 0.21762529, 0.21762376), key(10., 3.5, 1.0252255, 1.0252204)]) }
    fn settings() -> GamepadSettings {
        GamepadSettings { yaw_curve: Some(look_curve()), pitch_curve: Some(look_curve()), sensitivity_curve: Some(sens_curve()), look_scale: [1.85, 1.35], scale_downsights: 0.38, scale_scope: 0.25,
            aim_assist_strength: 0.75, accel_time: [0.15, 0.15], pull_strength: 0.25, pull_max_rate: 15., initial_downsight_strength: 0.06, slow_min_distance: 1., slow_min_strength: 0.5, slow_max_strength: 0.8, slow_decay_time: 0.5,
            ..GamepadSettings::native_defaults() }
    }
    fn ctx(fov: f32) -> LookContext { LookContext { fov, targeting_mult: 1., scoped_mult: 1., sensitivity: [5., 5.], ..Default::default() } }
    fn near(a: f32, b: f32, e: f32) -> bool { (a - b).abs() <= e }

    /// Hand-computed UE 4.16 FRichCurve values: P1 = P0 + Leave x Diff / 3, P2 = P3 - Arrive x Diff / 3, Bezier at alpha.
    /// GamepadLookCurve at 0.275: segment 0..0.55, alpha 0.5, P = (0, 0, 0.3 - 0.55/3, 0.3) -> (0 + 0 + 3 x 0.116667 + 0.3) / 8 = 0.08125.
    /// At 0.775: segment 0.55..1, alpha 0.5, P = (0.3, 0.45, 1 - 2.5275137 x 0.15, 1) -> (0.3 + 1.35 + 1.862619 + 1) / 8 = 0.564077.
    /// Sensitivity curve at 2: segment 1..5, alpha 0.25, P = (0.25, 0.25 + 0.17257269 x 4/3, 1 - 0.21762529 x 4/3, 1) -> 0.423455.
    #[test] fn rich_curve_matches_hand_computed_ue_values() {
        let c = look_curve();
        assert!(near(c.eval(0.275), 0.08125, 1e-6), "{}", c.eval(0.275));
        assert!(near(c.eval(0.775), 0.564077, 2e-6), "{}", c.eval(0.775));
        assert!(near(c.eval(0.5), 0.251690, 2e-6) && near(c.eval(0.9), 0.776045, 2e-6) && near(c.eval(0.225), 0.055151, 2e-6));
        assert_eq!((c.eval(0.), c.eval(0.55), c.eval(1.)), (0., 0.3, 1.), "keys are hit exactly");
        assert_eq!((c.eval(-0.5), c.eval(1.5)), (0., 1.), "constant extrapolation (RCCE_Constant default)");
        let s = sens_curve();
        assert!(near(s.eval(2.), 0.423455, 2e-6) && near(s.eval(7.5), 1.745249, 2e-6) && near(s.eval(0.5), 0.092581, 2e-6), "{} {} {}", s.eval(2.), s.eval(7.5), s.eval(0.5));
        assert_eq!((s.eval(1.), s.eval(5.), s.eval(10.)), (0.25, 1., 3.5));
        // linear and constant keys, linear extrapolation
        let mut l = RichCurve::new(vec![CurveKey { interp: 0, ..key(0., 0., 0., 0.) }, CurveKey { interp: 1, ..key(2., 4., 0., 0.) }, key(3., 10., 0., 0.)]);
        assert_eq!((l.eval(1.), l.eval(2.5)), (2., 4.));
        l.pre = RCCE_LINEAR; l.post = RCCE_LINEAR; assert_eq!((l.eval(-1.), l.eval(4.)), (-2., 16.));
    }
    /// The cooked asset, read the way the game reads it (needs FN_TEST_ROOT = the 1.8 install).
    #[test] fn cooked_athena_gamepad_settings_load() {
        let root = fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install");
        let set = fn_pak::PakSet::open_install(root).unwrap();
        let s = GamepadSettings::load(&set).unwrap();
        assert_eq!((s.look_scale, s.scale_downsights, s.scale_scope, s.accel_time, s.aim_assist_strength), ([1.85, 1.35], 0.38, 0.25, [0.15, 0.15], 0.75));
        assert_eq!((s.pull_strength, s.pull_max_rate, s.slow_min_strength, s.slow_max_strength, s.slow_decay_time), (0.25, 15., 0.5, 0.8, 0.5));
        assert_eq!((s.decel_time, s.scale_edit_mode, s.downsights_decay_time), ([0.1, 0.1], 1., 0.5), "native defaults the asset leaves alone");
        let (y, p, k) = (s.yaw_curve.as_ref().unwrap(), s.pitch_curve.as_ref().unwrap(), s.sensitivity_curve.as_ref().unwrap());
        assert_eq!(y, p, "yaw and pitch both reference GamepadLookCurve");
        assert_eq!(y.keys.len(), 3); assert!(near(y.eval(0.775), 0.564077, 2e-6) && near(y.keys[2].arrive, 2.5275137, 1e-6));
        assert_eq!(k.keys.iter().map(|k| (k.time, k.value)).collect::<Vec<_>>(), vec![(0., 0.05), (1., 0.25), (5., 1.), (10., 3.5)]);
        assert!(y.keys.iter().chain(&k.keys).all(|k| k.interp == 2) && y.pre == RCCE_CONSTANT && y.post == RCCE_CONSTANT && y.default_value.is_none());
    }
    #[test] fn curves_are_odd_symmetric_and_clamped() {
        let s = settings();
        assert_eq!(s.look_curves(1., -1.), (-1., 1.));
        assert_eq!(s.look_curves(2., -3.), (-1., 1.), "inputs clamp to [-1, 1]");
        let (p, y) = s.look_curves(-0.775, 0.275); assert!(near(y, -0.564077, 2e-6) && near(p, 0.08125, 1e-6));
        assert_eq!(GamepadSettings::native_defaults().look_curves(0.5, 0.5), (0., 0.), "null curves give 0");
        assert_eq!(s.sensitivity([5., 10.]), [1., 3.5]);
    }
    #[test] fn fov_term_and_multipliers_before_the_clamp() {
        // hip fire at FOV 80: 80 x 0.015 = 1.2, so full deflection saturates and 0.5 deflection becomes 0.6
        assert!(near(massage_axis(0.5, false, false, 0.3, 0.6, 80.), 0.6, 1e-6));
        assert!(near(massage_axis(0.5, true, false, 0.3, 0.6, 80.), 0.18, 1e-6), "targeting multiplier");
        assert!(near(massage_axis(0.5, true, true, 0.3, 0.6, 15.), 0.5 * 0.6 * 0.225, 1e-6), "scoped multiplier, sniper FOV 15");
        assert_eq!(stick_axes(32767, 0, &ctx(80.)), [1., 0.]);
        let a = stick_axes(0, 32767, &ctx(80.)); assert_eq!(a, [0., -1.], "stick up = negative LookUp_Gamepad (SceneViewport flip)");
        assert_eq!(stick_axes(0, 32767, &LookContext { invert: true, ..ctx(80.) }), [0., 1.]);
        assert_eq!(stick_axes(5000, 0, &ctx(80.)), [0., 0.], "inside the dead zone");
    }
    #[test] fn acceleration_ramps_over_accel_time_and_decays_over_decel_time() {
        let s = settings(); let mut g = GamepadLook::default();
        let dt = 1. / 60.;
        for i in 1..=9 { let a = g.accel_step(&s, [1., 0.], false, dt); assert!(near(a[0], (i as f32 * dt / 0.15).min(1.), 1e-5), "frame {i}: {a:?}"); }
        assert_eq!(g.accel, [1., 1.], "0.15 s = 9 frames");
        g.accel_step(&s, [0., 0.], false, dt); assert!(near(g.accel[0], 1. - dt / 0.1, 1e-5), "decel time 0.1 (native default)");
        for _ in 0..10 { g.accel_step(&s, [0., 0.], false, dt); } assert_eq!(g.accel, [0., 0.]);
    }
    /// Full deflection after the 0.15 s ramp, hip fire at FOV 80, default sensitivity 5 (curve 1.0), no target:
    /// yaw = 1.85 x 1 x 1 x 60 x 2.5 = 277.5 deg/s, pitch = 1.35 x 60 x 1.75 = 141.75 deg/s (stick up = look up).
    #[test] fn full_deflection_turn_rates() {
        let s = settings(); let c = ctx(80.); let dt = 1. / 60.;
        let mut g = GamepadLook::default();
        let axes = stick_axes(32767, 32767, &c);
        let mut first = LookOut::default();
        for i in 0..60 { let o = g.tick(Some(&s), &c, axes, dt); if i == 0 { first = o; } }
        let o = g.tick(Some(&s), &c, axes, dt);
        // diagonal: radial dead zone -> 0.7071 per axis -> AxisConfig dead zone (0.7071 - 0.25) / 0.75 = 0.6095 -> x 1.2 = 0.7314 on the curve
        let v = ((0.5f32.sqrt() - 0.25) / 0.75 * 1.2).min(1.); let cv = look_curve().eval(v);
        assert!(near(o.yaw_deg / dt, 277.5 * cv, 0.01) && near(o.pitch_up_deg / dt, 141.75 * cv, 0.01), "{o:?} {cv}");
        assert!(near(first.yaw_deg, o.yaw_deg / 9., 1e-4), "first frame at 1/9 acceleration");
        let mut g = GamepadLook::default();
        let axes = stick_axes(32767, 0, &c); for _ in 0..30 { g.tick(Some(&s), &c, axes, dt); }
        let o = g.tick(Some(&s), &c, axes, dt); assert!(near(o.yaw_deg / dt, 277.5, 0.01) && o.pitch_up_deg == 0., "{o:?}");
        let mut g = GamepadLook::default();
        let axes = stick_axes(0, 32767, &c); for _ in 0..30 { g.tick(Some(&s), &c, axes, dt); }
        let o = g.tick(Some(&s), &c, axes, dt); assert!(near(o.pitch_up_deg / dt, 141.75, 0.01) && o.yaw_deg == 0., "{o:?}");
        // max sensitivity 10 -> x 3.5; Athena never applies GamepadLookScaleDownsights: targeting keeps the hip rate at slider 1.0
        let mut g = GamepadLook::default(); let c10 = LookContext { sensitivity: [10., 10.], targeting: true, ..c };
        let axes = stick_axes(32767, 0, &c10); for _ in 0..30 { g.tick(Some(&s), &c10, axes, dt); }
        assert!(near(g.tick(Some(&s), &c10, axes, dt).yaw_deg / dt, 277.5 * 3.5, 0.05));
        // no settings asset: the stored axis goes to AddYawInput directly (one call per frame)
        let o = GamepadLook::default().tick(None, &c, [1., 0.], dt); assert_eq!(o.yaw_deg, 2.5);
    }
    #[test] fn aim_assist_without_a_target_has_no_slow_or_pull_and_slow_strength_maps_by_distance() {
        let s = settings(); let mut g = GamepadLook { slow: 0.6, pull_strength: 0.2, ..Default::default() };
        g.aim_assist(&s, &ctx(80.), [1., 0.], 0.1);
        assert!(near(g.slow, 0.6 - 0.1 / 0.5, 1e-6) && near(g.pull_strength, 0.2 - 0.1 / 0.5, 1e-6) && g.pull == [0., 0.]);
        assert_eq!(s.slow_strength(0.), 0.5); assert_eq!(s.slow_strength(1500.), 0.8); assert!(near(s.slow_strength(750.5), 0.65, 1e-6));
    }
}
