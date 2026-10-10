//! The native firing model of `AFortWeaponRanged` (1.8 `FortniteClient-Win64-Shipping.exe`, static disassembly, image base 0x140000000), ported 1:1.
//!
//! Everything here is f32 in the compiled operation order. Offsets named `stat+0x..` are `FFortRangedWeaponStats` members (reflection registration RVA 0x11dff80,
//! `props.exe 11e2965`): Spread 0x120, SpreadDownsights 0x124, StandingStillSpreadMultiplier 0x128, AthenaCrouching 0x12C, AthenaJumpingFalling 0x130, AthenaSprinting 0x134,
//! MinSpeedForSpreadMultiplier 0x138, MaxSpeedForSpreadMultiplier 0x13C, SpreadDownsightsAdditionalCooldownTime 0x140, HeatX1..3/Y1..3 0x144..0x158, HeatXScale/YScale
//! 0x15C/0x160, CoolX1..3/Y1..3 0x164..0x178, CoolXScale/YScale 0x17C/0x180, BulletsPerCartridge 0x184, FiringRate 0x188, ROFScale 0x18C, BurstFiringRate 0x190,
//! RecoilRecoveryDelay 0x1B8; base struct: BaseLevel +0x08 (int), DmgPB.. +0x0C, EnvDmgPB.. +0x1C, RngPB/Mid/Long/Max +0x40..0x4C, DmgScaleTable +0x50, DmgScale +0x60.
//! Weapon native members (not UPROPERTYs, `AFortWeaponRanged`): live spread state at +0xC00 / +0xC0B / +0xC10 (heat) / +0xC24 (multiplier) / +0xC38 (movement) / +0xC4C
//! (crouch) / +0xC60 (airborne). The exe keeps FIVE copies of that state and moves the live one to slot (3*i+1) mod 5 at the start of every tick (RVA 0x93e240) while
//! perturbing the other four with constants: an anti-tamper decoy, no gameplay effect, so one live slot is modelled here.
//!
//! RVAs: `AFortWeaponRanged::Tick` 0x93f8b0 (vtable 0x429fb10 = slot +0x408; the per-frame spread update), heat add 0x917e40 (called once at the end of the fire function),
//! heat curve 0x92a310, cool curve 0x92a0c0, fire 0x933630 (vtable +0x940), perfect-aim test 0x93b0c0, aim source selection 0x942c80 (vtable +0x858), weapon trace 0x945cc0
//! (vtable +0x868), damage curve 0x9223e0, fire rate 0x924f50 (vtable +0x920), burst rate 0x923100 (vtable +0x928), fire timer callback 0x9212d0.
use crate::data::StatRow;
use glam::Vec3;

/// `SMALL_NUMBER` (exe constant 0x322bcc77).
pub const SMALL_NUMBER: f32 = 1.0e-8;
const PI: f32 = std::f32::consts::PI;

/// `FMath::FInterpTo` (RVA 0x12db910; UE4.16 UnrealMath.cpp:2372).
pub fn f_interp_to(current: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0. { return target; }
    let dist = target - current;
    if dist * dist < SMALL_NUMBER { return target; }
    current + dist * (dt * speed).clamp(0., 1.)
}

/// `FMath::Fmod` as compiled at 0x933a3x (UE4.16 UnrealMathUtility.h): `Y <= 1e-8` returns 0.
pub fn fmod(x: f32, y: f32) -> f32 {
    if y.abs() <= SMALL_NUMBER { return 0.; }
    let quotient = (x / y).trunc();
    let mut int_portion = y * quotient;
    if int_portion.abs() > x.abs() { int_portion = x; }
    x - int_portion
}

/// `FRichCurve` with linear keys and constant extrapolation (pre/post infinity = 4, the `0x404` the exe stores), built with `AddKey` (RichCurve.cpp:248, new keys are
/// `RCIM_Linear`, inserted before the first key with a time >= the new one) and read with `Eval` (RichCurve.cpp:1022) / `GetTimeRange` (RichCurve.cpp:546).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve { pub keys: Vec<(f32, f32)> }
impl Curve {
    pub fn add_key(&mut self, time: f32, value: f32) {
        let mut i = 0;
        while i < self.keys.len() && self.keys[i].0 < time { i += 1; }
        self.keys.insert(i, (time, value));
    }
    pub fn time_range(&self) -> (f32, f32) {
        match (self.keys.first(), self.keys.last()) { (Some(a), Some(b)) => (a.0, b.0), _ => (0., 0.) }
    }
    pub fn eval(&self, t: f32, default: f32) -> f32 {
        let k = &self.keys; let n = k.len();
        if n == 0 { return default; }
        if n < 2 || t <= k[0].0 { return k[0].1; }
        if t < k[n - 1].0 {
            let (mut first, last) = (1usize, n - 1);
            let mut count = last - first;
            while count > 0 {
                let step = count / 2; let middle = first + step;
                if t >= k[middle].0 { first = middle + 1; count -= step + 1; } else { count = step; }
            }
            let (a, b) = (k[first - 1], k[first]);
            let diff = b.0 - a.0;
            if diff > 0. { let alpha = (t - a.0) / diff; return a.1 + alpha * (b.1 - a.1); }
            return a.1;
        }
        k[n - 1].1
    }
}

/// `max(Level - BaseLevel, 0)` as the exe computes it from `stat+0x08` and the weapon level (`vtable+0x878`, RVA 0x92c550). Every 1.8 BR row has `BaseLevel` 1 and the
/// `*Scale` columns 0, so the level never changes a value; the weapon level of a BR pickup is its item level (ASSUMED equal to `BaseLevel` here).
pub fn level_delta(s: &StatRow, level: i32) -> f32 { ((level - s.base_level) as f32).max(0.) }

/// Heat curve `FRichCurve` of RVA 0x92a310: keys `(HeatX_i + d*HeatXScale, HeatY_i + d*HeatYScale)`, `i = 1..3` in that order.
pub fn heat_curve(s: &StatRow, level: i32) -> Curve { three_key(s.heat, s.heat_scale, level_delta(s, level)) }
/// Cool curve of RVA 0x92a0c0 (same construction with the `Cool*` columns).
pub fn cool_curve(s: &StatRow, level: i32) -> Curve { three_key(s.cool, s.cool_scale, level_delta(s, level)) }
fn three_key(p: [(f32, f32); 3], scale: (f32, f32), d: f32) -> Curve {
    let (dx, dy) = (d * scale.0, d * scale.1);
    let mut c = Curve::default();
    for (x, y) in p { c.add_key(dx + x, dy + y); }
    c
}

/// What the tick reads from the pawn / camera (`AFortWeaponRanged` +0x150 owner, +0x965 `bIsTargeting`, +0x968 `LastTargetingTransitionTime`, +0x80 `CustomTimeDilation`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpreadInputs {
    /// `World->TimeSeconds`.
    pub now: f32,
    /// `|GetVelocity()|` of the owner, cm/s, all three axes (the exe takes `sqrt(x*x + y*y + z*z)`).
    pub speed_cm_s: f32,
    /// `ACharacter::bIsCrouched` (`[owner + 0x48C] & 1`).
    pub crouched: bool,
    /// `CharacterMovement->IsFalling()` (owner vtable +0x5E0 -> component vtable +0x508).
    pub falling: bool,
    /// `EFortMovementStyle == Sprinting (3)` (owner vtable +0x890, RVA 0x7eb250).
    pub sprinting: bool,
    /// `bIsTargeting`.
    pub targeting: bool,
    /// The blend weight of the targeting camera mode (RVA 0x4e7e90 on the weapon's `CameraTargeting1PClass` / `3PClass`): 0 at the hip, 1 fully aimed down sights.
    pub ads_alpha: f32,
    /// `LastTargetingTransitionTime` and the active camera instance's blend time (`[instance + 0x30]`), used only when `SpreadDownsightsAdditionalCooldownTime > 0`.
    pub targeting_since: f32, pub camera_blend_time: f32,
    pub time_dilation: f32,
    /// `Weapon.UseAthena...`: the Athena-only multipliers (crouch / jumping / sprinting) apply when the owner is an Athena pawn (RVA 0x92de40, true for every BR player).
    pub athena: bool,
}
impl Default for SpreadInputs {
    fn default() -> Self { SpreadInputs { now: 0., speed_cm_s: 0., crouched: false, falling: false, sprinting: false, targeting: false, ads_alpha: 0., targeting_since: 0., camera_blend_time: 0., time_dilation: 1., athena: true } }
}

/// The live spread state of one weapon actor. A weapon actor is created when the weapon is equipped, so the state starts at zero (the exe zero-initialises the members) and
/// `last_fire_time` at -10 (constructor 0x914b60).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpreadState {
    /// `heat[state]` (+0xC10): the bloom, kept inside `[HeatX1, HeatX3]` once the first tick / shot ran.
    pub heat: f32,
    /// `mult[state]` (+0xC24): ADS blend * movement * crouch * airborne, recomputed every tick.
    pub mult: f32,
    /// The three smoothed factors (+0xC38 movement, +0xC4C crouch, +0xC60 airborne).
    pub movement: f32, pub crouch: f32, pub air: f32,
    /// Flag byte +0xC0B: heat within 0.1 of the cool-curve minimum AND the movement factor within 0.1 of `StandingStillSpreadMultiplier`.
    pub at_min: bool,
    /// `LastFireTime` (+0x954).
    pub last_fire_time: f32,
}
impl Default for SpreadState {
    fn default() -> Self { SpreadState { heat: 0., mult: 0., movement: 0., crouch: 0., air: 0., at_min: false, last_fire_time: -10. } }
}
impl SpreadState {
    /// The per-frame update, RVA 0x93f8b0 (called with `dt` = the actor tick delta).
    pub fn tick(&mut self, s: &StatRow, i: &SpreadInputs, dt: f32, level: i32) {
        // cool curve at the current heat: the cooling rate AND the interpolation speed of the three smoothed factors (xmm11)
        let cool = cool_curve(s, level);
        let (cmin, cmax) = cool.time_range();
        let rate = cool.eval(self.heat, 0.);
        // cooling starts once (now - LastFireTime) * CustomTimeDilation exceeds RecoilRecoveryDelay (stat+0x1B8)
        if (i.now - self.last_fire_time) * i.time_dilation > s.recoil_recovery_delay {
            self.heat -= rate * dt;
            self.heat = if self.heat < cmin { cmin } else { self.heat.min(cmax) };
        }
        // ADS blend: Lerp(1, SpreadDownsights, alpha). With SpreadDownsightsAdditionalCooldownTime > 0 and bIsTargeting the alpha is the elapsed fraction of
        // (camera blend time + that cooldown) instead of the camera weight (0x93fbc4..0x93fc3c).
        let mut alpha = i.ads_alpha;
        if i.targeting && s.spread_downsights_additional_cooldown > 0. {
            let a = (i.now - i.targeting_since) / (i.camera_blend_time + s.spread_downsights_additional_cooldown);
            alpha = if a >= 0. { a.min(1.) } else { 0. };
        }
        let mut mult = (s.spread_downsights - 1.) * alpha + 1.;
        // movement factor target (0x93fc77..0x93fd73)
        let still = s.standing_still_spread_mult;
        let min_speed = s.min_speed_for_spread_mult.max(0.);
        let target = if i.speed_cm_s < min_speed { still } else {
            let max_speed = s.max_speed_for_spread_mult.max(cmin).max(0.1);
            let clamped = max_speed.min(i.speed_cm_s);
            let range = max_speed - min_speed;
            let frac = if range > SMALL_NUMBER { (clamped - min_speed) / range } else { 1. };
            let mut t = (1. - still) * frac + still;
            if i.athena && i.sprinting { t *= s.sprinting_spread_mult; }
            t
        };
        self.movement = f_interp_to(self.movement, target, dt, rate);
        // 0x93fdcf..0x93fe18: "at minimum" flag
        self.at_min = (self.heat - cmin).abs() <= 0.1 && (self.movement - still).abs() <= 0.1;
        mult *= self.movement;
        if i.athena {
            let crouch_t = if i.crouched { s.crouching_spread_mult } else { 1. };
            self.crouch = f_interp_to(self.crouch, crouch_t, dt, rate);
            mult *= self.crouch;
            let air_t = if i.falling { s.jumping_falling_spread_mult } else { 1. };
            self.air = f_interp_to(self.air, air_t, dt, rate);
            mult *= self.air;
        }
        self.mult = mult;
    }

    /// The bloom the next pellet uses, degrees (full cone angle): `mult[state] * heat[state] + bonus` (0x933919..0x933922). `bonus` = RVA 0x7929f0, the AI target-moving-laterally
    /// term (`AIAdditionalSpreadForTargetMovingLaterally`); 0 for a human player.
    pub fn spread_deg(&self, bonus: f32) -> f32 { self.mult * self.heat + bonus }

    /// `LastFireTime = World->TimeSeconds` (RVA 0x944900, once per pellet in the fire loop, vtable +0x900).
    pub fn mark_fired(&mut self, now: f32) { self.last_fire_time = now; }

    /// The heat added once per fire call (RVA 0x917e40, end of 0x933630): `heat += HeatCurve.Eval(heat) * Spread`, clamped to the curve's time range `[HeatX1, HeatX3]`.
    pub fn add_heat(&mut self, s: &StatRow, level: i32) {
        let c = heat_curve(s, level);
        let (min, max) = c.time_range();
        let y = c.eval(self.heat, 0.);
        let h = y * s.spread + self.heat;
        self.heat = if h < min { min } else { h.min(max) };
    }

    /// RVA 0x93b0c0 (`ShouldUsePerfectAim(pellet, bullets)`): with `bullets != 1` the FIRST pellet of a cartridge is exact; with a single bullet the shot is exact only while
    /// the spread sits at its minimum, the item definition allows it (`bShouldUsePerfectAimWhenTargetingMinSpread`), the owner's velocity is exactly zero and `bIsTargeting`.
    pub fn perfect_aim(&self, pellet: u32, bullets: u32, wid_allows: bool, velocity_exactly_zero: bool, targeting: bool) -> bool {
        if bullets != 1 { return pellet == 0; }
        self.at_min && wid_allows && velocity_exactly_zero && targeting
    }
}

/// `FRandomStream` (UE4.16 RandomStream.h): `Seed = Seed * 196314165 + 907633515`, fraction from the low 23 mantissa bits of 1.0f, `Fractional`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RandomStream { pub seed: i32 }
impl RandomStream {
    pub fn new(seed: i32) -> Self { RandomStream { seed } }
    /// `GetFraction` / `FRand` in [0, 1). Compiled at 0x93396b..0x9339d9 (constants 0xBB38435 = 196314165, 0x3619636B = 907633515).
    pub fn fraction(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(196314165).wrapping_add(907633515);
        let r = f32::from_bits((0x3f80_0000u32 & 0xff80_0000) | (self.seed as u32 & 0x007f_ffff));
        r - r.trunc()
    }
}

/// UE world (X north, Y east, Z up, left handed) <-> the port's world (Y up, right handed): the canonical `(x, y, z) <-> (x, z, y)` swap
/// (`fn_render::lighting::ue_to_world`, `fn_propdata::ue_to_game_position`).
/// SUN-AXIS FIX (2026-10-05): was `ue = (-z, x, y)`, 90 deg of yaw off the props / terrain. Every use here is a round trip of a direction through a
/// yaw-equivariant UE formula (`VRandCone`, `FRotator::Vector`), so shot directions are unchanged; only the meaning of "UE" now matches the map.
pub fn to_ue(v: Vec3) -> Vec3 { Vec3::new(v.x, v.z, v.y) }
pub fn from_ue(v: Vec3) -> Vec3 { Vec3::new(v.x, v.z, v.y) }

/// `FVector::GetSafeNormal()`: exactly-unit stays, below `SMALL_NUMBER` becomes zero.
pub fn safe_normal(v: Vec3) -> Vec3 {
    let sq = v.length_squared();
    if sq == 1. { return v; }
    if sq < SMALL_NUMBER { return Vec3::ZERO; }
    v * (1. / sq.sqrt())
}

/// `FVector::RotateAngleAxis` (UE4.16 UnrealMath.cpp): rotation of `v` by `deg` about the unit `axis`.
pub fn rotate_angle_axis(v: Vec3, deg: f32, axis: Vec3) -> Vec3 {
    let (s, c) = (deg.to_radians()).sin_cos();
    let (xx, yy, zz, xy, yz, zx) = (axis.x * axis.x, axis.y * axis.y, axis.z * axis.z, axis.x * axis.y, axis.y * axis.z, axis.z * axis.x);
    let (xs, ys, zs) = (axis.x * s, axis.y * s, axis.z * s);
    let omc = 1. - c;
    Vec3::new(
        (omc * xx + c) * v.x + (omc * xy - zs) * v.y + (omc * zx + ys) * v.z,
        (omc * xy + zs) * v.x + (omc * yy + c) * v.y + (omc * yz - xs) * v.z,
        (omc * zx - ys) * v.x + (omc * yz + xs) * v.y + (omc * zz + c) * v.z)
}

/// `FRandomStream::VRandCone(Dir, HorizontalHalf, VerticalHalf)` (UE4.16 RandomStream.h:248), the inlined copy at RVA 0x93396b..0x933d78, with both half angles the same
/// number (the exe passes one value). `dir` and the result are in this port's axes; the maths run in UE axes. Two stream draws, in this order: `RandU`, `RandV`.
///
/// `Theta = 2*PI*RandU`; `Phi = Acos(2*RandV - 1)`; the half angle for `Theta` is `Sqrt(1 / ((Cos(Theta)/V)^2 + (Sin(Theta)/H)^2))`; `Phi = Fmod(Phi, that)`;
/// the result is `Dir` rotated by `Phi` about `DirY` (the Y axis of `FRotationMatrix(Dir.Rotation())`, i.e. `(-sin yaw, cos yaw, 0)`), then by `Theta` about `Dir`, normalised.
/// NOTE the density: `Phi` is the sphere-uniform polar angle wrapped by `Fmod`, so points concentrate toward the cone axis (not area-uniform).
pub fn vrand_cone(dir: Vec3, half_h_rad: f32, half_v_rad: f32, rs: &mut RandomStream) -> Vec3 {
    let d = to_ue(dir);
    if !(half_v_rad > 0. && half_h_rad > 0.) { return from_ue(safe_normal(d)); }
    let rand_u = rs.fraction();
    let rand_v = rs.fraction();
    let theta = 2. * PI * rand_u;
    let phi = (2. * rand_v - 1.).clamp(-1., 1.).acos();
    let (inv_v, inv_h) = (1. / half_v_rad, 1. / half_h_rad);
    let (st, ct) = theta.sin_cos();
    let mut cone = (ct * inv_v) * (ct * inv_v) + (st * inv_h) * (st * inv_h);
    cone = (1. / cone).sqrt();
    let phi = fmod(phi, cone);
    // FVector::Rotation(): yaw = atan2(Y, X), pitch = atan2(Z, sqrt(X^2 + Y^2)); FRotationMatrix with roll 0: X = Dir (unit), Y = (-sin yaw, cos yaw, 0)
    let yaw = d.y.atan2(d.x);
    let axis_x = safe_normal(d); // GetScaledAxis(X) of the rotation matrix = the unit direction
    let axis_y = { let (sy, cy) = yaw.sin_cos(); Vec3::new(-sy, cy, 0.) };
    let mut r = rotate_angle_axis(d, phi * 180. * (1. / PI), axis_y);
    r = rotate_angle_axis(r, theta * 180. * (1. / PI), axis_x);
    from_ue(safe_normal(r))
}

/// Which damage table (`EFortBaseWeaponDamage`, byte at the 4th argument of RVA 0x9223e0): Combat reads `Dmg*` (+0x0C..), Environmental reads `EnvDmg*` (+0x1C..).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageKind { Combat, Environmental }

impl StatRow {
    /// The damage-over-distance curve of RVA 0x9223e0 (Athena path, cvar `Weapon.UseAthenaFalloff` irrelevant when `[GameInstance + 0x508] != 1`, which is the case outside the
    /// Save-the-World game type; INFERENCE: the byte is the game-type enum, constructor default 2 at RVA 0x538f4c, 1 = campaign): keys
    /// `(0, PB) (RngPB, PB) (RngMid, Mid) (RngLong, Long) (RngMax, Max)`, linear, constant outside.
    pub fn falloff_curve(&self, kind: DamageKind) -> Curve {
        let v = match kind { DamageKind::Combat => self.dmg, DamageKind::Environmental => self.env_dmg };
        let mut c = Curve::default();
        c.add_key(0., v[0]); c.add_key(self.rng[0], v[0]); c.add_key(self.rng[1], v[1]); c.add_key(self.rng[2], v[2]); c.add_key(self.rng[3], v[3]);
        c
    }
    /// Damage of one bullet at `range_cm` before the zone multiplier: `Eval(range)` then the level scale of RVA 0x9242a0 / 0x924440. With no scale table (every 1.8 BR row) the
    /// scale is `DmgScale` (+0x60; the Environmental version reads +0x60 too, not `EnvDmgScale`) and the mode is multiplicative: `(max(level - BaseLevel, 0) * scale + 1) * value`.
    pub fn damage_at(&self, kind: DamageKind, range_cm: f32, level: i32) -> f32 {
        let value = self.falloff_curve(kind).eval(range_cm, 0.);
        ((level - self.base_level).max(0) as f32 * self.dmg_scale + 1.) * value
    }
    /// `DamageZone_*` column of the zone a hit lands in.
    pub fn zone_damage(&self, kind: DamageKind, range_cm: f32, level: i32, zone: crate::shoot::Zone) -> f32 { self.damage_at(kind, range_cm, level) * self.zone_multiplier(zone) }

    /// `GetFireRate` RVA 0x924f50: `(max(level - BaseLevel, 0) * ROFScale + 1) * FiringRate` shots per second (the gameplay-effect attribute adjustment is identity without effects).
    pub fn fire_rate(&self, level: i32) -> f32 { (level_delta(self, level) * self.rof_scale + 1.) * self.firing_rate }
    /// Seconds between trigger pulls / between the first cartridges of consecutive bursts, `1 / fire_rate`.
    pub fn fire_interval(&self, level: i32) -> f32 { 1. / self.fire_rate(level) }
    /// RVA 0x923100 (vtable +0x928): `BurstFiringRate` when positive, else `fire_rate * (CartridgePerFire + 1)`. The fire timer callback (0x9212d0, `divss 1.0, result`) waits
    /// `1 / burst_rate` between the cartridges of one trigger pull while fewer than `CartridgePerFire` have been fired.
    pub fn burst_rate(&self, level: i32) -> f32 { if self.burst_firing_rate > 0. { self.burst_firing_rate } else { self.fire_rate(level) * (self.cartridge_per_fire as f32 + 1.) } }
    pub fn burst_interval(&self, level: i32) -> f32 { 1. / self.burst_rate(level) }
    /// Auto reload delay after the clip runs dry (RVA 0x922310): `AutoReloadDelayOverride` when >= 0, else the constant 0.3001 s (0x1442a0b74).
    pub fn auto_reload_delay(&self) -> f32 { if self.auto_reload_delay_override >= 0. { self.auto_reload_delay_override } else { f32::from_bits(0x3e99_a6b5) } }
    /// Distance the aim trace reaches, cm: `max(RngMax, RngLong)` (vtable +0x808, RVA 0x929630: `maxss [stat+0x4C], [stat+0x48]`), floored at 100 cm in the fire function.
    pub fn trace_distance_cm(&self) -> f32 { self.rng[3].max(self.rng[2]) }
}
