//! Firearm camera recoil (view kick), recovered from the 1.8 client exe (`FortniteClient-Win64-Shipping.exe`, image base 0x140000000) and the 1.8 stat rows.
//! Pure functions plus one small state struct; no file access (the front-end reads the `AthenaRangedWeapons` row and fills [`RecoilStats`]).
//! Evidence (instruction dumps with RVAs): `parity/recoil-evidence/recoil-natives-disasm.json`; write-up: `crates/fn-weapons/RECOIL.md`.
//!
//! RECOVERED NATIVE CODE (all `AFortWeaponRanged` unless noted; RVAs are image offsets):
//! * `0x925660` horizontal recoil value, `0x92bab0` vertical recoil value ([`horizontal_recoil`], [`vertical_recoil`]);
//! * `0x917930` AddRecoil, called once at the end of the fire function (vtable slot +0x940, RVA 0x933630) ([`RecoilState::add_recoil`]);
//! * `0x936200` vtable slot +0x760 UpdateRecoil, called every frame by `AFortPlayerController::UpdateRotation` (RVA 0x853440) with the frame's
//!   look-input delta rotator (`DeltaRot`) ([`RecoilState::update`]);
//! * `0x12db910` `FMath::FInterpTo` ([`finterp_to`]); `0x921cf0` stores `LastFireTime = World->TimeSeconds` when the weapon fires ([`RecoilState::on_fire`]).
//!
//! Where it is applied: the per-frame recoil step is ADDED to the controller's rotation-input delta (`DeltaRot`, 12-byte FRotator: pitch, yaw, roll in
//! degrees), which `UpdateRotation` then hands to `PlayerCameraManager::ProcessViewRotation` (vtable +0x6D0) together with the control rotation. So recoil is a
//! look delta, not a camera offset.
//!
//! NOT PORTED (reported, not approximated): see `RECOIL.md` "Missing": the targeting aim-adjustment block that precedes recoil inside `0x936200`
//! (`TargetingRotAdjustmentTotal`, not recoil), gameplay-effect modifiers of the `WeaponVerticalRecoil` / `WeaponHorizontalRecoil` attributes (no gameplay effects
//! exist in this runtime, the unmodified aggregate is the stat value), `ProcessViewRotation` itself, the value the encrypted engine config gives `Weapon.UseAthenaRecoil`,
//! and the seed of the CRT `rand()` stream.

/// (pitch, yaw, roll) in degrees: the FRotator layout at `+0x00 / +0x04 / +0x08`.
pub type Rot = [f32; 3];
const ZERO: Rot = [0.; 3];

/// `Weapon.UseAthenaRecoil` (RVA 0xafd10 registers it, help text "Let's us turn on/off new athena recoil code"): its storage `0x14662C1A0` is `.bss` and nothing in the
/// exe writes it, so the exe default is 0 = the legacy `Recoil*` columns are used. The encrypted engine config could still set it (unreadable here): callers pass the
/// effective value to [`RecoilState::add_recoil`] / [`RecoilState::update`] as `athena`.
pub const USE_ATHENA_RECOIL_EXE_DEFAULT: bool = false;
/// `AFortWeapon` ctor (RVA 0x914b60): `mov dword [rdi+0x954], 0xC1200000` -> `LastFireTime` starts at -10.0.
pub const LAST_FIRE_TIME_DEFAULT: f32 = -10.0;

/// The `FFortRangedWeaponStats` fields the recoil code reads (struct offsets from the reflection registration in `Z_Construct_UScriptStruct` RVA 0x11dff80; each
/// field is read in the functions named in the comments). Values come from the weapon's `AthenaRangedWeapons` row.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RecoilStats {
    /// `+0x194` RecoilVert / `+0x198` RecoilVertScale / `+0x19C` RecoilVertScaleGamepad (0x92bab0).
    pub vert: f32, pub vert_scale: f32, pub vert_scale_gamepad: f32,
    /// `+0x1A0` VertRecoilDownChance (0x917d26).
    pub vert_down_chance: f32,
    /// `+0x1A4` RecoilHoriz / `+0x1A8` RecoilHorizScale / `+0x1AC` RecoilHorizScaleGamepad (0x925660).
    pub horiz: f32, pub horiz_scale: f32, pub horiz_scale_gamepad: f32,
    /// `+0x1B0` RecoilInterpSpeed, `+0x1B4` RecoilRecoveryInterpSpeed, `+0x1B8` RecoilRecoveryDelay, `+0x1BC` RecoilRecoveryFraction (0x936200).
    pub interp_speed: f32, pub recovery_interp_speed: f32, pub recovery_delay: f32, pub recovery_fraction: f32,
    /// `+0x1C0` RecoilDownsightsMultiplier (0x917d58).
    pub downsights_mult: f32,
    /// `+0x1C4/+0x1C8` AthenaRecoilMagnitudeMin/Max, `+0x1CC` AthenaRecoilMagnitudeScale, `+0x1D0/+0x1D4` AthenaRecoilAngleMin/Max (degrees),
    /// `+0x1D8/+0x1DC` AthenaRecoilRollMagnitudeMin/Max (0x917a44).
    pub athena_mag_min: f32, pub athena_mag_max: f32, pub athena_mag_scale: f32,
    pub athena_angle_min: f32, pub athena_angle_max: f32, pub athena_roll_min: f32, pub athena_roll_max: f32,
    /// `+0x1E0` AthenaRecoilInterpSpeed, `+0x1E4` AthenaRecoilRecoveryInterpSpeed (0x936200), `+0x1E8` AthenaRecoilDownsightsMultiplier (0x917c3b).
    pub athena_interp_speed: f32, pub athena_recovery_interp_speed: f32, pub athena_downsights_mult: f32,
}

/// `maxss a, b` (returns `b` unless `a > b`).
fn maxss(a: f32, b: f32) -> f32 { if a > b { a } else { b } }
/// `minss a, b` (returns `b` unless `a < b`).
fn minss(a: f32, b: f32) -> f32 { if a < b { a } else { b } }
/// The compiled `ucomiss` / `jne` pair used for "!= ZeroRotator" checks: unordered (NaN) compares as equal.
fn ne_ordered(a: f32, b: f32) -> bool { a < b || a > b }

/// Horizontal recoil value `(gamepad ? RecoilHorizScaleGamepad : RecoilHorizScale) * RecoilHoriz`, clamped at 0 (`maxss`): RVA 0x925660 (+0x925708 gamepad test via
/// `IsUsingGamepad` 0x5c9540 -> 0x22c4030). The gameplay-effect aggregate on the `WeaponHorizontalRecoil` attribute (0x925806 .. 0x9258f3) is the identity without effects.
pub fn horizontal_recoil(s: &RecoilStats, gamepad: bool) -> f32 { maxss((if gamepad { s.horiz_scale_gamepad } else { s.horiz_scale }) * s.horiz, 0.) }
/// Vertical recoil value `(gamepad ? RecoilVertScaleGamepad : RecoilVertScale) * RecoilVert`, clamped at 0 (`maxss`): RVA 0x92bab0 (same shape as 0x925660).
pub fn vertical_recoil(s: &RecoilStats, gamepad: bool) -> f32 { maxss((if gamepad { s.vert_scale_gamepad } else { s.vert_scale }) * s.vert, 0.) }

/// `FMath::FInterpTo` (RVA 0x12db910): `speed <= 0` -> target; `dist^2 < 1e-8` (const 0x143ff9008) -> target; else `cur + dist * clamp(dt * speed, 0, 1)`.
pub fn finterp_to(cur: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if !(speed > 0.) { return target; }
    let dist = target - cur;
    if dist * dist < f32::from_bits(0x322b_cc77) { return target; }
    let a = dt * speed;
    let alpha = if a < 0. { 0. } else { minss(a, 1.) };
    alpha * dist + cur
}

/// One lane of the vectorised `FRotator::NormalizeAxis` inside the compiled `IsNearlyZero` (RVAs 0x9366f8.. and 0x936dd3..): `x - trunc(x / 360) * 360`, `+360` when
/// negative, `-360` when above 180 (constants 360 / 180 are stored by RVA 0xafe40 / 0xafe30 into `0x14662C500` / `0x14662C4F0`).
fn norm_lane(x: f32) -> f32 {
    let q = ((x / 360.) as i32) as f32;
    let mut x = x - q * 360.;
    if !(0. <= x) { x += 360.; }
    if 180. < x { x - 360. } else { x }
}
/// `FRotator::IsNearlyZero(tolerance)` as compiled: every normalised axis within `tolerance` (NaN lanes compare as "within").
pub fn is_nearly_zero(r: Rot, tolerance: f32) -> bool { !r.iter().any(|&v| tolerance < norm_lane(v).abs()) }

/// The CRT `rand()` the exe imports (`rand`, IAT slot 0x143FA67B8): the MSVC / UCRT linear congruential generator. THIS ROUTINE IS NOT IN THE EXE (it is imported):
/// it is the documented UCRT algorithm; the exe's seed (`srand`) is a runtime value that is not recovered, so the seed is a parameter.
#[derive(Clone, Debug)]
pub struct CrtRand { state: u32 }
impl CrtRand {
    pub fn new(seed: u32) -> CrtRand { CrtRand { state: seed } }
    /// 0 ..= 32767.
    pub fn next(&mut self) -> i32 { self.state = self.state.wrapping_mul(214013).wrapping_add(2531011); ((self.state >> 16) & 0x7fff) as i32 }
}
/// `FMath::FRand`-style draw as compiled at every call site: `(float)rand() * 0x38000100` (the constant at 0x143fe36ec, 1/32767).
fn frand(rand: &mut dyn FnMut() -> i32) -> f32 { (rand() as f32) * f32::from_bits(0x3800_0100) }

/// `FMath::SinCos` as compiled inline in AddRecoil (RVA 0x917aa3 .. 0x917c06): returns (sin, cos) of `radians`. Polynomial constants from 0x143feb1ac..0x143feb1f4.
pub fn sin_cos(radians: f32) -> (f32, f32) {
    let c = |o: u32| f32::from_bits(o);
    let mut y = radians;
    let mut q = y * c(0x3e22_f983);
    q = if y >= 0. { q + 0.5 } else { q - 0.5 };
    let qi = (q as i32) as f32;
    y -= qi * c(0x40c9_0fdb);
    let sign;
    if y > c(0x3fc9_0fdb) { y = c(0x4049_0fdb) - y; sign = -1.; }
    else if y >= c(0xbfc9_0fdb) { sign = 1.; }
    else { y = c(0xc049_0fdb) - y; sign = -1.; }
    let y2 = y * y;
    let mut sn = c(0x3638_b88e) - y2 * c(0x32cd_365b);
    let mut cs = c(0x37cf_b4c2) - y2 * c(0x348b_dd11);
    sn *= y2; sn -= c(0x3950_0bf1);
    cs *= y2; cs -= c(0x3ab6_09aa);
    sn *= y2; sn += c(0x3c08_8886);
    cs *= y2; cs += c(0x3d2a_aaa3);
    sn *= y2; sn -= c(0x3e2a_aaab);
    cs *= y2; cs -= 0.5;
    sn *= y2; sn += 1.;
    cs *= y2; cs += 1.;
    (sn * y, cs * sign)
}

/// Per-weapon recoil state: native members of `AFortWeaponRanged` (not UPROPERTYs): `+0xB9C` pending kick (`RecoilToApply`), `+0xBA8` `RecoilOffsetToRecover`,
/// `+0xBB4` `RecoilManuallyRecovered` (names from the engine's own log strings at 0x1442917a0), plus the reflected `LastFireTime` (`+0x954`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecoilState { pub pending: Rot, pub to_recover: Rot, pub manual: Rot, pub last_fire_time: f32 }
impl Default for RecoilState { fn default() -> Self { RecoilState { pending: ZERO, to_recover: ZERO, manual: ZERO, last_fire_time: LAST_FIRE_TIME_DEFAULT } } }

impl RecoilState {
    /// `AFortWeapon` fire notification (RVA 0x921cf0): `LastFireTime = World->TimeSeconds` (`[world + 0x900]`).
    pub fn on_fire(&mut self, now: f32) { self.last_fire_time = now; }

    /// AddRecoil (RVA 0x917930), once per executed fire call (end of 0x933630). `targeting` = `bIsTargeting` (`[weapon + 0x965]`), `gamepad` = `IsUsingGamepad`,
    /// `athena` = `Weapon.UseAthenaRecoil != 0` (the owner-is-Athena-pawn check 0x92de40 is true for every player here). `rand` = the CRT `rand()` stream.
    /// Draw order (each is one `rand()` call): Athena path: magnitude, angle, roll; legacy path: vertical, horizontal, vertical-down test, horizontal-sign test.
    pub fn add_recoil(&mut self, s: &RecoilStats, targeting: bool, gamepad: bool, athena: bool, rand: &mut dyn FnMut() -> i32) {
        if athena {
            // 0x917a44..0x917c4f
            let mag = (frand(rand) * (s.athena_mag_max - s.athena_mag_min) + s.athena_mag_min) * s.athena_mag_scale;
            let deg = frand(rand) * (s.athena_angle_max - s.athena_angle_min) + s.athena_angle_min;
            let (sin, cos) = sin_cos(deg * f32::from_bits(0x3c8e_fa35));
            let mut yaw = sin * mag; let mut pitch = cos * mag;
            let mut roll = (frand(rand) * (s.athena_roll_max - s.athena_roll_min) + s.athena_roll_min) * s.athena_mag_scale;
            if targeting { let m = s.athena_downsights_mult; pitch *= m; yaw *= m; roll *= m; }
            self.pending[0] = pitch + self.pending[0]; self.pending[1] = yaw + self.pending[1]; self.pending[2] = roll + self.pending[2];
        } else {
            // 0x917c84..0x917de3
            let vert = vertical_recoil(s, gamepad); let horiz = horizontal_recoil(s, gamepad);
            let (hv, hh) = (vert * 0.5, horiz * 0.5);
            let mut pitch = frand(rand) * (vert - hv) + hv;
            let mut yaw = frand(rand) * (horiz - hh) + hh;
            let mut roll = 0.0f32;
            if !(frand(rand) >= s.vert_down_chance) { pitch = -pitch; }
            if !(frand(rand) >= 0.5) { yaw = -yaw; }
            if targeting { let m = s.downsights_mult; pitch *= m; yaw *= m; roll = m * roll; }
            let p = pitch + self.pending[0]; let y = yaw + self.pending[1]; let r = roll + self.pending[2];
            // the accumulated pending kick is clamped to +-vert / +-horiz (0x917d8d..0x917ddb); roll is not clamped
            let clamp = |v: f32, lim: f32| if !(v >= -lim) { -lim } else { minss(v, lim) };
            self.pending = [clamp(p, vert), clamp(y, horiz), r];
        }
    }

    /// UpdateRecoil (vtable +0x760, RVA 0x936200, the recoil half starting at 0x9366be): `delta_rot` = the frame's look-input delta rotator (what
    /// `AFortPlayerController::UpdateRotation` passes in `r9`), which this ADDS the recoil step to. `dt` = frame DeltaTime, `now` = `World->TimeSeconds`,
    /// `custom_time_dilation` = `AActor::CustomTimeDilation` (`[weapon + 0x80]`, 1.0 normally).
    pub fn update(&mut self, s: &RecoilStats, dt: f32, delta_rot: &mut Rot, now: f32, custom_time_dilation: f32, athena: bool) {
        let input = *delta_rot; // [rsp+0x50]: the look input before any recoil
        // 0x9366d2..0x93684b: the player's own counter-input accumulates while any recoil is outstanding, else it is cleared
        if !is_nearly_zero(self.pending, 0.5) || !is_nearly_zero(self.to_recover, 0.5) { for i in 0..3 { self.manual[i] -= input[i]; } } else { self.manual = ZERO; }

        // 0x936870..0x936a3c: ease the pending kick into the view
        let any_ne = |r: Rot| ne_ordered(r[0], 0.) || ne_ordered(r[1], 0.) || ne_ordered(r[2], 0.);
        if any_ne(self.pending) {
            let speed = if athena { s.athena_interp_speed } else { s.interp_speed };
            let step = [self.pending[0] - finterp_to(self.pending[0], 0., dt, speed), self.pending[1] - finterp_to(self.pending[1], 0., dt, speed), self.pending[2] - finterp_to(self.pending[2], 0., dt, speed)];
            for i in 0..3 { self.pending[i] -= step[i]; delta_rot[i] = step[i] + delta_rot[i]; }
            let frac = if athena { 1. } else { s.recovery_fraction };
            for i in 0..3 { self.to_recover[i] = frac * step[i] + self.to_recover[i]; }
            if !ne_ordered(step[0], 0.) && !ne_ordered(step[1], 0.) && !ne_ordered(step[2], 0.) { self.pending = ZERO; }
        }

        // 0x936a44..0x936c40: reconcile the player's counter-input with the outstanding recovery (pitch, then yaw; roll has no branch)
        if any_ne(self.to_recover) {
            let (m, o) = pitch_block(self.manual[0], self.to_recover[0]); self.manual[0] = m; self.to_recover[0] = o;
            let (m, o) = yaw_block(self.manual[1], self.to_recover[1]); self.manual[1] = m; self.to_recover[1] = o;
        }

        // 0x936c48..0x936d9c: after RecoilRecoveryDelay since the last shot, ease the recoverable offset back out of the view
        let since = (now - self.last_fire_time) * custom_time_dilation;
        if any_ne(self.to_recover) && since > s.recovery_delay {
            let speed = if athena { s.athena_recovery_interp_speed } else { s.recovery_interp_speed };
            let step = [self.to_recover[0] - finterp_to(self.to_recover[0], 0., dt, speed), self.to_recover[1] - finterp_to(self.to_recover[1], 0., dt, speed), self.to_recover[2] - finterp_to(self.to_recover[2], 0., dt, speed)];
            for i in 0..3 { self.to_recover[i] -= step[i]; }
            for i in 0..3 { delta_rot[i] -= step[i]; }
        }
        // 0x936da1..0x936e55
        if is_nearly_zero(self.to_recover, f32::from_bits(0x38d1_b717)) { self.to_recover = ZERO; }
    }
}

/// The pitch half of the counter-input reconciliation (RVA 0x936a73..0x936bc4): returns (manual, to_recover). Reproduced branch for branch, quirks included.
fn pitch_block(m: f32, o: f32) -> (f32, f32) {
    let tiny = f32::from_bits(0x322b_cc77);
    if m > 0. {
        if o > 0. {
            let o2 = maxss(o - m, 0.);
            return (if o2 >= tiny { maxss(m - o2, 0.) } else { 0. }, o2);
        }
    } else {
        if m >= 0. { /* m == 0: falls to 0x936b25 */ }
        else if !(o >= 0.) {
            let o2 = minss(o - m, 0.);
            return (if o2 <= tiny { minss(m - o2, 0.) } else { 0. }, o2);
        }
    }
    // 0x936b25
    if m >= 0. {
        // 0x936b61: m <= 0 -> nothing; m > 0 with o < 0 only logs ("PITCH: GREATER THAN AND LESS THAN"); no state change
        return (m, o);
    }
    // m < 0 (0x936b2b)
    if o > 0. {
        let o2 = maxss(o - m, 0.);
        return (maxss(m + o2, 0.), o2);
    }
    (m, o)
}
/// The yaw half (RVA 0x936bc4..0x936c40): no `m < 0, o > 0` branch.
fn yaw_block(m: f32, o: f32) -> (f32, f32) {
    let tiny = f32::from_bits(0x322b_cc77);
    if m > 0. {
        if o > 0. {
            let o2 = maxss(o - m, 0.);
            return (if o2 >= tiny { maxss(m - o2, 0.) } else { 0. }, o2);
        }
        return (m, o);
    }
    if m >= 0. { return (m, o); }
    if o >= 0. { return (m, o); }
    let o2 = minss(o - m, 0.);
    (if o2 <= tiny { minss(m - o2, 0.) } else { 0. }, o2)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn near(a: f32, b: f32, e: f32) -> bool { (a - b).abs() <= e }

    /// `AthenaRangedWeapons` rows read from the 1.8 install (hitreg-dump row output, see RECOIL.md).
    fn ar() -> RecoilStats { RecoilStats { vert: 5., vert_scale: 1., vert_scale_gamepad: 0.5, vert_down_chance: 0., horiz: 0.25, horiz_scale: 1., horiz_scale_gamepad: 1., interp_speed: 8., recovery_interp_speed: 6., recovery_delay: 0., recovery_fraction: 1., downsights_mult: 0.5,
        athena_mag_min: 4., athena_mag_max: 4., athena_mag_scale: 1., athena_angle_min: -25., athena_angle_max: 25., athena_roll_min: 0., athena_roll_max: 0., athena_interp_speed: 12., athena_recovery_interp_speed: 6., athena_downsights_mult: 0.5 } }
    fn shotgun() -> RecoilStats { RecoilStats { vert: 9.9, vert_scale_gamepad: 0.75, horiz: 2., interp_speed: 12., recovery_interp_speed: 3.6, downsights_mult: 0.85, athena_mag_min: 9., athena_mag_max: 9., athena_recovery_interp_speed: 4., athena_downsights_mult: 0.85, ..ar() } }
    fn sniper() -> RecoilStats { RecoilStats { vert: 5., horiz: 0., interp_speed: 12., recovery_interp_speed: 6., downsights_mult: 0.4, athena_mag_min: 5., athena_mag_max: 5., athena_downsights_mult: 0.4, ..ar() } }
    fn seq(v: Vec<i32>) -> impl FnMut() -> i32 { let mut i = 0; move || { let r = v[i % v.len()]; i += 1; r } }

    #[test] fn recoil_values_follow_the_scale_columns_and_the_gamepad_flag() {
        assert_eq!((vertical_recoil(&ar(), false), vertical_recoil(&ar(), true)), (5., 2.5));
        assert_eq!((horizontal_recoil(&ar(), false), horizontal_recoil(&ar(), true)), (0.25, 0.25));
        assert_eq!(vertical_recoil(&shotgun(), true), 9.9f32 * 0.75);
        assert_eq!(horizontal_recoil(&sniper(), false), 0., "bolt sniper RecoilHoriz is 0");
        let neg = RecoilStats { vert: -3., vert_scale: 1., ..ar() };
        assert_eq!(vertical_recoil(&neg, false), 0., "maxss clamps at zero");
    }
    #[test] fn finterp_to_is_the_engine_function() {
        assert_eq!(finterp_to(10., 0., 0.1, 0.), 0., "speed <= 0 snaps to the target");
        assert_eq!(finterp_to(1e-5, 0., 0.1, 5.), 0., "dist^2 < 1e-8 snaps");
        assert!(near(finterp_to(10., 0., 1. / 60., 6.), 10. - 10. * 0.1, 1e-5), "alpha = dt * speed");
        assert!(near(finterp_to(10., 0., 1., 6.), 0., 1e-6), "alpha clamps at 1");
        assert_eq!(finterp_to(10., 0., -1., 6.), 10., "negative alpha clamps to 0");
    }
    #[test] fn is_nearly_zero_normalises_each_axis() {
        assert!(is_nearly_zero([0., 0., 0.], 1e-4));
        assert!(is_nearly_zero([360., -360., 720.], 1e-4), "full turns are zero");
        assert!(!is_nearly_zero([0.6, 0., 0.], 0.5) && is_nearly_zero([0.5, -0.5, 0.], 0.5));
        assert!(!is_nearly_zero([359.4, 0., 0.], 0.5), "359.4 normalises to -0.6");
        assert!(is_nearly_zero([359.7, 0., 0.], 0.5));
    }
    #[test] fn crt_rand_is_the_ucrt_lcg() {
        let mut r = CrtRand::new(1);
        assert_eq!((0..5).map(|_| r.next()).collect::<Vec<_>>(), vec![41, 18467, 6334, 26500, 19169]);
    }
    #[test] fn sin_cos_matches_the_compiled_polynomial_over_the_recoil_range_and_beyond() {
        for d in [-725., -180., -90., -25., -1., 0., 0.5, 25., 89., 90., 91., 180., 270., 725.] {
            let r = f32::to_radians(d); let (s, c) = sin_cos(r);
            assert!(near(s, r.sin(), 3e-5) && near(c, r.cos(), 3e-5), "{d}: {s} {c} vs {} {}", r.sin(), r.cos());
        }
        assert_eq!(sin_cos(0.), (0., 1.));
    }
    #[test] fn legacy_add_recoil_uses_the_vertical_and_horizontal_columns() {
        // draws: pitch fraction, yaw fraction, vertical-down test, horizontal-sign test
        let mut st = RecoilState::default();
        st.add_recoil(&ar(), false, false, false, &mut seq(vec![32767, 32767, 32767, 32767]));
        let k = f32::from_bits(0x3800_0100); let f = 32767. * k;
        assert_eq!(st.pending[0], f * (5. - 2.5) + 2.5, "pitch = frand * (v - v/2) + v/2");
        assert_eq!(st.pending[1], f * (0.25 - 0.125) + 0.125, "frand 1.0 >= 0.5: positive yaw");
        assert_eq!(st.pending[2], 0.);
        let mut st = RecoilState::default();
        st.add_recoil(&ar(), false, false, false, &mut seq(vec![0, 0, 0, 0]));
        assert_eq!((st.pending[0], st.pending[1]), (2.5, -0.125), "frand 0 < 0.5: yaw flips; 0 >= VertRecoilDownChance 0: pitch stays up");
    }
    #[test] fn legacy_add_recoil_downsights_multiplier_and_the_pending_clamp() {
        let mut st = RecoilState::default();
        st.add_recoil(&ar(), true, false, false, &mut seq(vec![0, 0, 0, 32767]));
        assert_eq!((st.pending[0], st.pending[1]), (1.25, 0.0625), "x RecoilDownsightsMultiplier 0.5");
        let mut st = RecoilState::default();
        for _ in 0..6 { st.add_recoil(&ar(), false, false, false, &mut seq(vec![32767, 32767, 32767, 32767])); }
        assert_eq!((st.pending[0], st.pending[1]), (5., 0.25), "the accumulated pending kick clamps at +-RecoilVert / +-RecoilHoriz");
        let mut st = RecoilState::default();
        st.add_recoil(&RecoilStats { vert_down_chance: 1., ..ar() }, false, false, false, &mut seq(vec![0, 0, 0, 32767]));
        assert_eq!(st.pending[0], -2.5, "frand < VertRecoilDownChance flips the pitch kick down");
    }
    #[test] fn athena_add_recoil_is_magnitude_at_an_angle_from_vertical() {
        // draws: magnitude, angle, roll
        let mut st = RecoilState::default();
        st.add_recoil(&ar(), false, false, true, &mut seq(vec![0, 0, 0]));
        let (s, c) = sin_cos((-25f32) * f32::from_bits(0x3c8e_fa35));
        assert_eq!(st.pending, [c * 4., s * 4., 0.], "angle = AthenaRecoilAngleMin");
        assert!(near(st.pending[0], 4. * 25f32.to_radians().cos(), 1e-4) && near(st.pending[1], -4. * 25f32.to_radians().sin(), 1e-4));
        let mut st = RecoilState::default();
        st.add_recoil(&shotgun(), true, false, true, &mut seq(vec![0, 16383, 0]));
        assert!(st.pending[0] > 0. && st.pending[0] < 9. * 0.85 + 1e-4, "downsights multiplier applies after the magnitude: {:?}", st.pending);
    }
    #[test] fn update_eases_the_pending_kick_into_the_look_delta_and_records_it_for_recovery() {
        let mut st = RecoilState { pending: [5., 0.2, 0.], ..Default::default() };
        let mut d = [0., 0., 0.];
        st.last_fire_time = 100.;
        st.update(&ar(), 1. / 60., &mut d, 100., 1., false); // same instant as the shot: since = 0, not > RecoilRecoveryDelay 0 -> no recovery yet
        let fi = finterp_to(5., 0., 1. / 60., 8.); let step = 5. - fi;
        assert_eq!(d[0], step + 0.);
        assert_eq!(st.pending[0], 5. - step);
        assert_eq!(st.to_recover[0], step, "RecoilRecoveryFraction 1.0");
        assert!(near(step, 5. * 8. / 60., 1e-5));
    }
    #[test] fn a_shot_kicks_the_view_up_and_the_recovery_returns_it_exactly_without_input() {
        let (mut st, mut net) = (RecoilState::default(), [0f32; 3]);
        let mut r = CrtRand::new(7);
        st.on_fire(1.0);
        st.add_recoil(&ar(), false, false, false, &mut || r.next());
        let (mut peak, mut now) = (0f32, 1.0f32);
        for _ in 0..600 { now += 1. / 60.; let mut d = [0.; 3]; st.update(&ar(), 1. / 60., &mut d, now, 1., false); for i in 0..3 { net[i] += d[i]; } peak = peak.max(net[0]); }
        assert!(peak > 0.5, "the view kicks up: {peak}");
        assert!(net.iter().all(|v| v.abs() < 3e-4), "all recoil is recovered when the player does not counter it: {net:?}");
        assert_eq!((st.pending, st.to_recover), ([0.; 3], [0.; 3]), "state snaps to ZeroRotator");
    }
    #[test] fn recovery_waits_for_the_recovery_delay_after_the_last_shot() {
        let s = RecoilStats { recovery_delay: 0.5, ..ar() };
        let mut st = RecoilState { to_recover: [2., 0., 0.], last_fire_time: 10., ..Default::default() };
        let mut d = [0.; 3];
        st.update(&s, 0.1, &mut d, 10.4, 1., false);
        assert_eq!((d, st.to_recover), ([0.; 3], [2., 0., 0.]), "0.4 s since the shot <= delay 0.5");
        st.update(&s, 0.1, &mut d, 10.51, 1., false);
        let step = 2. - finterp_to(2., 0., 0.1, 6.);
        assert_eq!(d[0], -step, "recovery subtracts from the look delta");
        let mut st = RecoilState { to_recover: [2., 0., 0.], last_fire_time: 10., ..Default::default() };
        let mut d = [0.; 3];
        st.update(&s, 0.1, &mut d, 10.3, 2., false);
        assert!(d[0] < 0., "CustomTimeDilation scales the elapsed time (0.3 * 2 > 0.5)");
    }
    #[test] fn countering_the_recoil_with_input_reduces_what_recovery_takes_back() {
        // pending kick applied, then the player pulls down 1 degree: manual = -input accumulates and the pitch block removes it from the recoverable offset
        let mut st = RecoilState { to_recover: [3., 0., 0.], last_fire_time: 0., ..Default::default() };
        let mut d = [-1., 0., 0.];
        st.update(&ar(), 1. / 60., &mut d, 0., 1., false);
        assert_eq!((st.manual[0], st.to_recover[0]), (0., 2.), "manual = -input = 1, then the pitch block: o' = max(o - m, 0) = 2 and (o' >= 1e-8) manual = max(m - o', 0) = 0");
        // the quirky second assignment of the original: o' >= 1e-8 -> manual = max(m - o', 0)
        assert_eq!(pitch_block(1., 3.), (0., 2.));
        assert_eq!(pitch_block(3., 4.), (2., 1.), "m - o' (not zero): reproduced as compiled");
        assert_eq!(pitch_block(5., 4.), (0., 0.), "offset fully consumed: manual reset");
        assert_eq!(pitch_block(-1., -3.), (0., -2.));
        assert_eq!(pitch_block(-1., 3.), (3., 4.), "m < 0, o > 0: o' = max(o - m, 0), m' = max(m + o', 0)");
        assert_eq!(pitch_block(1., -2.), (1., -2.), "m > 0, o < 0 only logs");
        assert_eq!(yaw_block(1., 3.), (0., 2.));
        assert_eq!(yaw_block(-1., 3.), (-1., 3.), "no m < 0, o > 0 branch for yaw");
    }
    #[test] fn manual_counter_input_is_cleared_once_nothing_is_outstanding() {
        let mut st = RecoilState { manual: [1., 2., 3.], ..Default::default() };
        st.update(&ar(), 1. / 60., &mut [0.5, 0., 0.], 0., 1., false);
        assert_eq!(st.manual, [0.; 3]);
    }
    #[test] fn athena_path_uses_the_athena_speeds_and_a_full_recovery_fraction() {
        let s = RecoilStats { recovery_fraction: 0.25, ..ar() };
        let mut st = RecoilState { pending: [4., 0., 0.], last_fire_time: 5., ..Default::default() };
        st.update(&s, 1. / 60., &mut [0.; 3], 5., 1., true);
        let step = 4. - finterp_to(4., 0., 1. / 60., 12.);
        assert_eq!(st.to_recover[0], step, "athena: fraction fixed at 1.0, AthenaRecoilInterpSpeed 12");
        let mut st = RecoilState { pending: [4., 0., 0.], last_fire_time: 5., ..Default::default() };
        st.update(&s, 1. / 60., &mut [0.; 3], 5., 1., false);
        let step = 4. - finterp_to(4., 0., 1. / 60., 8.);
        assert_eq!(st.to_recover[0], 0.25 * step, "legacy: RecoilRecoveryFraction, RecoilInterpSpeed 8");
    }
}

