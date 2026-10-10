//! FALL DAMAGE (fn-game): the 1.8 falling damage of the BR player pawn, ported from the exe and the cooked data. Renderer-free; `GameSim::tick` feeds the
//! [`FallTracker`] after every movement step and the front-end drains [`FallTracker::take_landings`] and applies [`fall_damage`] to the pawn's health / shield.
//!
//! RECOVERED (exe = FortniteClient-Win64-Shipping.exe 1.8, addresses as in `D:\FortniteRef\decomp\named`):
//! * apex: `UFortMovementComp_Character::TickComponent` 0x1406ea350, after `UCharacterMovementComponent::TickComponent`: while falling `FallingStartedZ` (+0x74c) =
//!   max(FallingStartedZ, actor Z); while not falling it is reset to -3.4e38 (0xff7fc99e). `OnMovementModeChanged` 0x1406dff00 raises it to the Z at the mode change.
//! * landing: `AFortPawn::Landed` (vfunc 244) 0x1407eb460: `height = max(0, FallingStartedZ - Z)` (cm), then (authority only) 0x1407cdf50.
//! * damage 0x1407cdf50: unless `bIgnoreNextFallingDamage` (+0x848): `frac = FallingDamageTable Lookup(height)` (0x1407dd7f0, `UCurveTable::FindCurve` + `FRichCurve::Eval`),
//!   `dmg = GetMaxHealth() (vfunc 0x960) * frac * 1.0` (the stack argument is DAT_143fe36fc = 1.0f, read from the exe). If `dmg > 0`: when `Health <= dmg` and
//!   `frac < 1.0` (and an interface check at +0x7a0 vfunc 0x28 is not set, see NOT PORTED) the hit is made non-lethal: `dmg = Health - 1.0`. It is applied with
//!   `UFortKismetLibrary::ApplyGlobalEnvironmentDamageToActor` 0x1405fb7d0 with the tag `Gameplay.Damage.Environment.Falling` (global tag slot 0x26, FUN_1405af6c0) and the
//!   class `FallingDamageTemplateGE` of the game data (`UFortGameData` +0x1740; `/Game/Balance/DefaultGameData` = `GET_FallingDamage_C`), then the event
//!   `Event.Damage.Received.Falling` is sent. `bIgnoreNextFallingDamage` is cleared after every Landed, damaged or not.
//! * shields: `GET_FallingDamage` executes `UFortDamageFormulaExecutionCalculation` 0x14065f1c0. Its shield split (decomp lines ~1040-1095) skips the shield only for
//!   the tags `Gameplay.Damage.OutsideSafeZone` (slot 0x27, +0x138) and `Gameplay.Damage.DBNOBleedOut` (slot 0x28, +0x140). Falling (slot 0x26) is not one of them, so
//!   that split alone would put fall damage on the shield first, but real 1.8 BR never takes fall damage from the shield (user-confirmed against the real
//!   game), so the port applies all of it to health and leaves the shield untouched.
//! * curve: the pawn stats row is `PlayerPawn_Athena` `PawnStatHandle` = `CharacterAttributesPlayer` / `PlayerLoadout_AthenaPlayer` (`AFortPawn::GetPawnStats` 0x140413090
//!   reads +0x808): `FallingDamageTable = /Game/Athena/Balance/DataTables/AthenaFallingDamage`, `FallingDamageTableRow = Player`. Keys (height cm -> fraction of max
//!   health) are [`ATHENA_FALLING_DAMAGE_PLAYER`]; every key is `RCIM_Linear`, Pre/PostInfinityExtrap `RCCE_Constant`. The test `cooked_curve_matches` re-reads them.
//!
//! NOT PORTED: the +0x7a0 interface check before the non-lethal clamp is not identified (treated as "not set", so the clamp always applies when `frac < 1`); the
//! `FallingStartedZ` feedback event (0x754 bit, a callout above DAT_143ff9048) has no feedback system here; there is no death / DBNO on 0 health in this port.

/// `AthenaFallingDamage` row `Player` (cooked CurveTable `/Game/Athena/Balance/DataTables/AthenaFallingDamage`): (fall height cm, fraction of max health).
pub const ATHENA_FALLING_DAMAGE_PLAYER: [(f32, f32); 10] = [
    (0., 0.), (384., 0.), (768., 0.), (1299., 0.), (1300., 0.1), (1536., 0.2), (1920., 0.5), (2304., 1.0), (2704., 1.0), (7200., 5.0),
];
/// The scale argument of 0x1407cdf50 (`DAT_143fe36fc`, 1.0f).
pub const FALL_DAMAGE_SCALE: f32 = 1.0;
/// `FallingStartedZ` reset value (0xff7fc99e = -3.4e38).
const NOT_FALLING: f32 = -3.4e38;

/// `FRichCurve::Eval` of a curve whose keys are all `RCIM_Linear` with constant extrapolation.
pub fn eval_linear(keys: &[(f32, f32)], t: f32) -> f32 {
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return 0. };
    if t <= first.0 { return first.1; }
    if t >= last.0 { return last.1; }
    for w in keys.windows(2) {
        if t < w[1].0 { let a = (t - w[0].0) / (w[1].0 - w[0].0); return w[0].1 + (w[1].1 - w[0].1) * a; }
    }
    last.1
}
/// Fraction of max health a fall of `height_cm` costs (`FallingDamageTable Lookup`).
pub fn fall_damage_fraction(height_cm: f32) -> f32 { eval_linear(&ATHENA_FALLING_DAMAGE_PLAYER, height_cm) }

/// What one landing does to the pawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FallHit {
    /// Fall height, cm (apex minus landing Z).
    pub height_cm: f32,
    /// Curve value.
    pub fraction: f32,
    /// Damage 0x1407cdf50 hands to the effect (after the non-lethal clamp).
    pub damage: f32,
    /// What the damage execution took from the shield and from health.
    pub shield_damage: f32,
    pub health_damage: f32,
}
impl FallHit { pub fn any(&self) -> bool { self.damage > 0. } }

/// 0x1407cdf50 + the shield split of the damage execution for a landing after a fall of `height_cm`, on a pawn with `health` / `shield` and `max_health`.
pub fn fall_damage(height_cm: f32, health: f32, shield: f32, max_health: f32) -> FallHit {
    let height_cm = height_cm.max(0.);
    let fraction = fall_damage_fraction(height_cm);
    let mut damage = max_health * fraction * FALL_DAMAGE_SCALE;
    if damage <= 0. { return FallHit { height_cm, fraction, damage: 0., shield_damage: 0., health_damage: 0. }; }
    if health <= damage && fraction < 1.0 { damage = health - 1.0; }
    // Fall damage never touches the shield in 1.8 BR: all of it goes to health.
    let _ = shield;
    FallHit { height_cm, fraction, damage, shield_damage: 0., health_damage: damage.max(0.) }
}

/// `FallingStartedZ` bookkeeping around the movement component's tick, plus the landings it produced.
#[derive(Clone, Debug)]
pub struct FallTracker {
    /// `FallingStartedZ` in metres (feet height; the capsule offset cancels in the difference).
    pub apex_m: f32,
    /// `bIgnoreNextFallingDamage` (+0x848): launch props set it; cleared after every landing.
    pub ignore_next: bool,
    landings: Vec<f32>,
    was_airborne: bool,
    seen_landings: u32,
}
impl Default for FallTracker { fn default() -> Self { FallTracker { apex_m: NOT_FALLING, ignore_next: false, landings: vec![], was_airborne: false, seen_landings: 0 } } }
impl FallTracker {
    /// Call before the movement step with the feet height (m) and the falling state at that moment.
    pub fn before_step(&mut self, height_m: f32, airborne: bool) {
        if !airborne { self.apex_m = NOT_FALLING; }
        self.was_airborne = airborne;
        if !airborne { self.apex_m = self.apex_m.max(height_m); } // seed: OnMovementModeChanged raises it to the Z where falling starts
    }
    /// Call after the movement step: `landings` = the component's landing counter (`Controller::landings`), `height_m` the feet height, `airborne` the new state.
    pub fn after_step(&mut self, height_m: f32, airborne: bool, landings: u32) {
        let landed = landings != self.seen_landings;
        self.seen_landings = landings;
        if landed && (self.was_airborne || self.apex_m > NOT_FALLING) {
            let h_cm = ((self.apex_m - height_m) * 100.).max(0.);
            if !self.ignore_next { self.landings.push(h_cm); }
            self.ignore_next = false;
        }
        if airborne { self.apex_m = self.apex_m.max(height_m); } else { self.apex_m = NOT_FALLING; }
        self.was_airborne = airborne;
    }
    /// The fall heights (cm) of the landings since the last call.
    pub fn take_landings(&mut self) -> Vec<f32> { std::mem::take(&mut self.landings) }
    /// `AFortPawn::ResetFallingHeight` 0x1407fde60 (teleports / launch props).
    pub fn reset(&mut self) { self.apex_m = NOT_FALLING; }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAX: f32 = 100.;
    #[test] fn curve_values_follow_the_cooked_keys() {
        assert_eq!(fall_damage_fraction(0.), 0.);
        assert_eq!(fall_damage_fraction(1299.), 0.);
        assert!((fall_damage_fraction(1300.) - 0.1).abs() < 1e-6);
        assert!((fall_damage_fraction(1500.) - (0.1 + 0.1 * 200. / 236.)).abs() < 1e-5);
        assert!((fall_damage_fraction(2304.) - 1.0).abs() < 1e-6);
        assert!((fall_damage_fraction(2500.) - 1.0).abs() < 1e-6);
        assert!((fall_damage_fraction(10_000.) - 5.0).abs() < 1e-6, "constant post-infinity");
    }
    #[test] fn damage_at_full_health_no_shield() {
        let h = fall_damage(1299., 100., 0., MAX); assert!(!h.any());
        let h = fall_damage(1300., 100., 0., MAX); assert!((h.damage - 10.).abs() < 1e-4 && (h.health_damage - 10.).abs() < 1e-4);
        let h = fall_damage(1500., 100., 0., MAX); assert!((h.damage - 18.474577).abs() < 1e-3, "{h:?}");
        let h = fall_damage(1920., 100., 0., MAX); assert!((h.damage - 50.).abs() < 1e-4);
        let h = fall_damage(2304., 100., 0., MAX); assert!((h.damage - 100.).abs() < 1e-4, "frac 1.0 is not < 1: lethal, no clamp");
    }
    #[test] fn a_fall_below_the_lethal_fraction_leaves_one_health() {
        // 2200 cm -> frac 0.5 + 0.5 * 280/384 = 0.8646: 86.5 damage on 50 health is clamped to health - 1
        let h = fall_damage(2200., 50., 0., MAX); assert!((h.damage - 49.).abs() < 1e-4 && (h.health_damage - 49.).abs() < 1e-4, "{h:?}");
        let h = fall_damage(2400., 50., 0., MAX); assert!(h.damage >= 100., "frac >= 1: no clamp");
    }
    #[test] fn falling_damage_skips_the_shield() {
        let h = fall_damage(1920., 100., 30., MAX); assert!(h.shield_damage == 0. && (h.health_damage - 50.).abs() < 1e-4, "{h:?}");
        let h = fall_damage(1300., 100., 50., MAX); assert!(h.shield_damage == 0. && (h.health_damage - 10.).abs() < 1e-4);
    }
    #[test] fn tracker_measures_apex_to_landing() {
        let mut t = FallTracker::default();
        // grounded at 15 m (teleport), then a fall to 0 m in steps, landing on the 4th step
        t.before_step(15., false); t.after_step(14.9, true, 0);
        t.before_step(14.9, true); t.after_step(10., true, 0);
        t.before_step(10., true); t.after_step(3., true, 0);
        t.before_step(3., true); t.after_step(0., false, 1);
        let l = t.take_landings(); assert_eq!(l.len(), 1); assert!((l[0] - 1500.).abs() < 1e-2, "{l:?}");
        // a jump: apex is the highest post-tick Z
        t.before_step(0., false); t.after_step(0.3, true, 1); t.before_step(0.3, true); t.after_step(0.64, true, 1); t.before_step(0.64, true); t.after_step(0., false, 2);
        let l = t.take_landings(); assert!((l[0] - 64.).abs() < 1e-2);
        t.ignore_next = true; t.before_step(5., false); t.after_step(4., true, 2); t.before_step(4., true); t.after_step(0., false, 3);
        assert!(t.take_landings().is_empty() && !t.ignore_next, "bIgnoreNextFallingDamage skips one landing and is cleared");
    }
    /// The cooked curve, read from the paks (needs FN_TEST_ROOT = the 1.8 install).
    #[test] fn cooked_curve_matches() {
        let Some(root) = fn_live::var_os("FN_TEST_ROOT").map(std::path::PathBuf::from) else { eprintln!("FN_TEST_ROOT not set: skipped"); return };
        let Some(dir) = fn_pak::find_paks_dir(&root) else { eprintln!("no paks: skipped"); return };
        let set = fn_pak::PakSet::open_install(&dir).expect("paks");
        let pd = fn_uasset::PackageData::load(&set, "/Game/Athena/Balance/DataTables/AthenaFallingDamage").expect("AthenaFallingDamage");
        let pkg = pd.package().expect("package");
        let tb = fn_uasset::classes::read_table(&pkg, 0).expect("curve table");
        let (_, row) = tb.rows.iter().find(|(n, _)| n == "Player").expect("row Player");
        let keys: Vec<(f32, f32, i64)> = row.get("Keys").and_then(fn_uasset::Value::as_array).expect("keys").iter()
            .map(|k| (k.field("Time").and_then(fn_uasset::Value::as_f32).unwrap(), k.field("Value").and_then(fn_uasset::Value::as_f32).unwrap(), k.field("InterpMode").and_then(|v| v.as_f32()).map_or(0, |v| v as i64))).collect();
        assert_eq!(keys.len(), ATHENA_FALLING_DAMAGE_PLAYER.len());
        for (k, c) in keys.iter().zip(ATHENA_FALLING_DAMAGE_PLAYER) { assert_eq!((k.0, k.1), c); assert_eq!(k.2, 0, "RCIM_Linear"); }
    }
}
