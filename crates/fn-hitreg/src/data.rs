//! Everything the 1.8 files serialize about how a firearm shoots, read at load time (no number below is typed in):
//!  * the stat row (`WeaponStatHandle`: `AthenaRangedWeapons.<row>`): spread, spread multipliers, heat / cool curve points, recoil, damage points + their ranges,
//!    damage zones (headshot multiplier), knockback, AI ranges;
//!  * the weapon actor Blueprint CDO (`WeaponActorClass`): the aim flags (`bShouldAimFromMuzzleAtCloseRange`, `bMaintainAimLocationDuringTargeting`,
//!    `bUseScopeTargeting`), muzzle / tracer / beam particle systems, impact effect + sound per physical surface, camera classes, projectile template;
//!  * the primary fire ability CDO (`PrimaryFireAbility`): the target selection list (shape, range, cone angles, filter flags) and the cue tags.
//! Units are the data's: centimetres, degrees, seconds.
use fn_pose::{loader, loadout_stats::data_table_row, ClipSourceDir};
use fn_ueio::{props::{read_props, Ctx, PropList, PropValue}, Package, Reader};

/// All floats / ints of the ranged stat row that matter to shooting. Field names follow the data columns.
#[derive(Clone, Debug, PartialEq)]
pub struct StatRow {
    pub spread: f32, pub spread_downsights: f32,
    pub standing_still_spread_mult: f32, pub crouching_spread_mult: f32, pub jumping_falling_spread_mult: f32, pub sprinting_spread_mult: f32,
    pub min_speed_for_spread_mult: f32, pub max_speed_for_spread_mult: f32, pub spread_downsights_additional_cooldown: f32,
    /// `HeatX1..3 / HeatY1..3` and `CoolX1..3 / CoolY1..3` (three points each; the native spread-heat model that consumes them is not recoverable).
    pub heat: [(f32, f32); 3], pub cool: [(f32, f32); 3],
    pub bullets_per_cartridge: u32, pub firing_rate: f32,
    // legacy recoil columns
    pub recoil_vert: f32, pub recoil_vert_scale: f32, pub recoil_vert_scale_gamepad: f32, pub vert_recoil_down_chance: f32,
    pub recoil_horiz: f32, pub recoil_horiz_scale: f32, pub recoil_horiz_scale_gamepad: f32,
    pub recoil_interp_speed: f32, pub recoil_recovery_interp_speed: f32, pub recoil_recovery_delay: f32, pub recoil_recovery_fraction: f32, pub recoil_downsights_mult: f32,
    // Athena recoil columns
    pub athena_recoil_magnitude: (f32, f32), pub athena_recoil_magnitude_scale: f32, pub athena_recoil_angle: (f32, f32), pub athena_recoil_roll: (f32, f32),
    pub athena_recoil_interp_speed: f32, pub athena_recoil_recovery_interp_speed: f32, pub athena_recoil_downsights_mult: f32,
    // damage: [point blank, mid, long, max range] and the ranges (cm) they belong to
    pub dmg: [f32; 4], pub env_dmg: [f32; 4], pub impact_dmg: [f32; 4], pub rng: [f32; 4],
    pub sweep_radius: f32, pub max_actual_range: f32,
    pub zone_light: f32, pub zone_normal: f32, pub zone_critical: f32, pub zone_vulnerability: f32,
    pub dice_crit_chance: f32, pub dice_crit_damage_mult: f32,
    pub knockback: [f32; 3], pub knockback_z_angle: f32, pub stun_time: f32,
    pub surface_ratio_row: String,
    pub bullets_per_tracer: u32,
    pub dmg_scale: f32, pub env_dmg_scale: f32,
    pub charge: (f32, f32, f32), pub charge_damage_mult: (f32, f32),
}

fn f(p: &PropList, k: &str) -> Result<f32, String> { p.get(k).and_then(PropValue::as_f64).map(|v| v as f32).ok_or_else(|| format!("stat row lacks {k}")) }
fn u(p: &PropList, k: &str) -> Result<u32, String> { f(p, k).map(|v| v.max(0.) as u32) }

impl StatRow {
    pub fn from_props(p: &PropList) -> Result<StatRow, String> {
        let pt = |a: &str, b: &str| -> Result<(f32, f32), String> { Ok((f(p, a)?, f(p, b)?)) };
        Ok(StatRow {
            spread: f(p, "Spread")?, spread_downsights: f(p, "SpreadDownsights")?,
            standing_still_spread_mult: f(p, "StandingStillSpreadMultiplier")?, crouching_spread_mult: f(p, "AthenaCrouchingSpreadMultiplier")?,
            jumping_falling_spread_mult: f(p, "AthenaJumpingFallingSpreadMultiplier")?, sprinting_spread_mult: f(p, "AthenaSprintingSpreadMultiplier")?,
            min_speed_for_spread_mult: f(p, "MinSpeedForSpreadMultiplier")?, max_speed_for_spread_mult: f(p, "MaxSpeedForSpreadMultiplier")?,
            spread_downsights_additional_cooldown: f(p, "SpreadDownsightsAdditionalCooldownTime")?,
            heat: [pt("HeatX1", "HeatY1")?, pt("HeatX2", "HeatY2")?, pt("HeatX3", "HeatY3")?],
            cool: [pt("CoolX1", "CoolY1")?, pt("CoolX2", "CoolY2")?, pt("CoolX3", "CoolY3")?],
            bullets_per_cartridge: u(p, "BulletsPerCartridge")?, firing_rate: f(p, "FiringRate")?,
            recoil_vert: f(p, "RecoilVert")?, recoil_vert_scale: f(p, "RecoilVertScale")?, recoil_vert_scale_gamepad: f(p, "RecoilVertScaleGamepad")?, vert_recoil_down_chance: f(p, "VertRecoilDownChance")?,
            recoil_horiz: f(p, "RecoilHoriz")?, recoil_horiz_scale: f(p, "RecoilHorizScale")?, recoil_horiz_scale_gamepad: f(p, "RecoilHorizScaleGamepad")?,
            recoil_interp_speed: f(p, "RecoilInterpSpeed")?, recoil_recovery_interp_speed: f(p, "RecoilRecoveryInterpSpeed")?, recoil_recovery_delay: f(p, "RecoilRecoveryDelay")?,
            recoil_recovery_fraction: f(p, "RecoilRecoveryFraction")?, recoil_downsights_mult: f(p, "RecoilDownsightsMultiplier")?,
            athena_recoil_magnitude: pt("AthenaRecoilMagnitudeMin", "AthenaRecoilMagnitudeMax")?, athena_recoil_magnitude_scale: f(p, "AthenaRecoilMagnitudeScale")?,
            athena_recoil_angle: pt("AthenaRecoilAngleMin", "AthenaRecoilAngleMax")?, athena_recoil_roll: pt("AthenaRecoilRollMagnitudeMin", "AthenaRecoilRollMagnitudeMax")?,
            athena_recoil_interp_speed: f(p, "AthenaRecoilInterpSpeed")?, athena_recoil_recovery_interp_speed: f(p, "AthenaRecoilRecoveryInterpSpeed")?,
            athena_recoil_downsights_mult: f(p, "AthenaRecoilDownsightsMultiplier")?,
            dmg: [f(p, "DmgPB")?, f(p, "DmgMid")?, f(p, "DmgLong")?, f(p, "DmgMaxRange")?],
            env_dmg: [f(p, "EnvDmgPB")?, f(p, "EnvDmgMid")?, f(p, "EnvDmgLong")?, f(p, "EnvDmgMaxRange")?],
            impact_dmg: [f(p, "ImpactDmgPB")?, f(p, "ImpactDmgMid")?, f(p, "ImpactDmgLong")?, f(p, "ImpactDmgMaxRange")?],
            rng: [f(p, "RngPB")?, f(p, "RngMid")?, f(p, "RngLong")?, f(p, "RngMax")?],
            sweep_radius: f(p, "SweepRadius")?, max_actual_range: f(p, "MaxActualRange")?,
            zone_light: f(p, "DamageZone_Light")?, zone_normal: f(p, "DamageZone_Normal")?, zone_critical: f(p, "DamageZone_Critical")?, zone_vulnerability: f(p, "DamageZone_Vulnerability")?,
            dice_crit_chance: f(p, "DiceCritChance")?, dice_crit_damage_mult: f(p, "DiceCritDamageMultiplier")?,
            knockback: [f(p, "KnockbackMagnitude")?, f(p, "MidRangeKnockbackMagnitude")?, f(p, "LongRangeKnockbackMagnitude")?], knockback_z_angle: f(p, "KnockbackZAngle")?, stun_time: f(p, "StunTime")?,
            surface_ratio_row: p.text("SurfaceRatioRowName").unwrap_or("").to_owned(),
            bullets_per_tracer: u(p, "BulletsPerTracer")?,
            dmg_scale: f(p, "DmgScale")?, env_dmg_scale: f(p, "EnvDmgScale")?,
            charge: (f(p, "MinChargeTime")?, f(p, "MaxChargeTime")?, f(p, "ChargeDownTime")?), charge_damage_mult: (f(p, "MinChargeDamageMultiplier")?, f(p, "MaxChargeDamageMultiplier")?),
        })
    }
}

/// `FortAbilityTargetSelection` of the primary fire ability (the data half of the hit selection: the native trace itself is C++).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetSelection {
    pub shape: String, pub test_type: String, pub primary_source: String, pub range: f32,
    pub cone_yaw: f32, pub cone_pitch: f32, pub cone_min_radius: f32,
    pub actor_type_filter: String, pub exclude_pawn_friends: bool, pub exclude_obstructed_by_world: bool, pub max_targets: i32,
}
#[derive(Clone, Debug, PartialEq, Default)]
pub struct FireAbility {
    pub package: String,
    pub selections: Vec<TargetSelection>,
    pub max_yaw_angle_to_target: Option<f32>,
    pub weapon_fire_cue: Vec<String>, pub impact_cues: Vec<String>, pub ability_tags: Vec<String>,
    /// `WeaponTestApplicationTag` / `ApplicationTag` of the containers.
    pub application_tags: Vec<String>,
    /// `TargetGameplayEffectClasses` of container 0 (GE_Ranged_GenericDamage -> template GET_DirectPhysicalDamage).
    pub effect_classes: Vec<String>,
}

/// The weapon actor's shooting-related presentation + aim flags.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Presentation {
    pub actor: String,
    pub aim_from_muzzle_at_close_range: bool, pub maintain_aim_during_targeting: bool, pub use_scope_targeting: bool,
    pub muzzle_particle: Option<String>, pub tracer_template: Option<String>, pub beam_particle: Option<String>,
    pub camera_base_3p: Option<String>, pub camera_targeting_3p: Option<String>,
    pub fire_sounds: Vec<(u32, String)>,
    /// `ImpactPhysicalSurfaceEffects[i]` / `ImpactPhysicalSurfaceSounds[i]`: index = EPhysicalSurface value (sounds: 1 wood, 2 stone, 3 metal, 4 player; the file names say so).
    pub impact_effects: Vec<(u32, String)>, pub impact_sounds: Vec<(u32, String)>,
    pub projectile_template: Option<String>,
}
impl Presentation {
    pub fn impact_effect(&self, surface: u32) -> Option<&str> { self.impact_effects.iter().find(|(i, _)| *i == surface).map(|x| x.1.as_str()) }
    pub fn impact_sound(&self, surface: u32) -> Option<&str> { self.impact_sounds.iter().find(|(i, _)| *i == surface).map(|x| x.1.as_str()) }
}

/// A fully loaded firearm for hit registration.
#[derive(Clone, Debug, PartialEq)]
pub struct GunHit { pub wid: String, pub row: String, pub stats: StatRow, pub pres: Presentation, pub ability: FireAbility, /** Some for projectile weapons (the bolt-action sniper) */ pub projectile: Option<ProjectileData> }

/// `InitialSpeed` / `MaxSpeed` (cm/s) of the projectile Blueprint, evaluated at curve time 0 from its `AthenaProjectiles` CurveTable rows. Gravity scale: the
/// `ProjectileMovementComponent` of the Blueprint serializes no override (engine default; the world gravity is the movement crate's).
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectileData { pub template: String, pub table: String, pub initial_speed_row: String, pub max_speed_row: String, pub initial_speed: f32, pub max_speed: f32 }

/// `FRichCurve::Eval` for the interpolation modes a table row uses (Linear 0 / Constant 1; cubic falls back to the key value), constant extrapolation.
pub fn eval_curve(keys: &[(f32, f32, i64)], t: f32) -> f32 {
    let Some(first) = keys.first() else { return 0. };
    if t <= first.0 { return first.1; }
    for w in keys.windows(2) { if t < w[1].0 { return if w[0].2 == 0 { w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) / (w[1].0 - w[0].0).max(1e-9) } else { w[0].1 }; } }
    keys.last().map_or(0., |k| k.1)
}
fn curve_row(set: &fn_pak::PakSet, table: &str, row: &str, t: f32) -> Result<f32, String> {
    let pd = fn_uasset::PackageData::load(set, table).map_err(|e| format!("{table}: {e}"))?;
    let pkg = pd.package().map_err(|e| format!("{table}: {e}"))?;
    let tb = fn_uasset::classes::read_table(&pkg, 0).map_err(|e| format!("{table}: {e}"))?;
    let (_, props) = tb.rows.iter().find(|(n, _)| n == row).ok_or_else(|| format!("{table}: no row {row}"))?;
    let keys: Vec<(f32, f32, i64)> = props.get("Keys").and_then(fn_uasset::Value::as_array).unwrap_or(&[]).iter()
        .filter_map(|k| Some((k.field("Time")?.as_f32()?, k.field("Value")?.as_f32()?, k.field("InterpMode").and_then(fn_uasset::Value::as_i64).unwrap_or(0)))).collect();
    if keys.is_empty() { return Err(format!("{table}: row {row} has no keys")); }
    Ok(eval_curve(&keys, t))
}
pub fn projectile(src: &ClipSourceDir, template: &str) -> Result<ProjectileData, String> {
    let ClipSourceDir::Pak(set) = src else { return Err("projectile curve tables are read from the pak".into()) };
    let pkg = loader::package(src, pkg_of(template))?;
    let p = cdo(&pkg)?;
    let handle = |k: &str| -> Result<(String, String), String> {
        let h = p.fields(k).ok_or_else(|| format!("projectile lacks {k}"))?;
        let table = h.object("CurveTable").filter(|i| *i < 0).map(|i| pkg.import_path(i)).ok_or("handle has no CurveTable")?;
        Ok((pkg_of(&table).to_owned(), h.text("RowName").ok_or("handle has no RowName")?.to_owned()))
    };
    let ((table, ir), (_, mr)) = (handle("InitialSpeed")?, handle("MaxSpeed")?);
    Ok(ProjectileData { template: pkg_of(template).to_owned(), initial_speed: curve_row(set, &table, &ir, 0.)?, max_speed: curve_row(set, &table, &mr, 0.)?, table, initial_speed_row: ir, max_speed_row: mr })
}

fn pkg_of(r: &str) -> &str { r.split('.').next().unwrap_or(r) }
fn path_of(pkg: &Package, p: &PropList, key: &str) -> Option<String> {
    if let Some(s) = p.text(key).filter(|s| s.starts_with("/Game/")) { return Some(s.to_owned()); }
    p.object(key).filter(|i| *i < 0).map(|i| pkg.import_path(i))
}
fn indexed_paths(pkg: &Package, p: &PropList, key: &str) -> Vec<(u32, String)> {
    p.0.iter().filter(|x| x.name == key).filter_map(|x| match &x.value {
        PropValue::Str(s) if s.starts_with("/Game/") => Some((x.array_index as u32, s.clone())),
        PropValue::Object(i) if *i < 0 => Some((x.array_index as u32, pkg.import_path(*i))),
        _ => None }).collect()
}
/// `FGameplayTagContainer` / `FGameplayTag` payload (`i32 n` + n x FName) of a raw property.
pub fn tag_names(pkg: &Package, v: &PropValue) -> Vec<String> {
    let PropValue::Raw(b) = v else { return vec![] };
    if b.len() < 4 || (b.len() - 4) % 8 != 0 { return vec![]; }
    let n = i32::from_le_bytes(b[0..4].try_into().unwrap()).max(0) as usize;
    if n * 8 != b.len() - 4 { return vec![]; }
    (0..n).filter_map(|k| { let i = i32::from_le_bytes(b[4 + k * 8..8 + k * 8].try_into().ok()?); pkg.names.get(i.max(0) as usize).cloned() }).collect()
}
fn cdo(pkg: &Package) -> Result<PropList, String> {
    let idx = pkg.exports.iter().position(|e| e.object_name.starts_with("Default__")).ok_or("package has no class default object")?;
    let (s, e) = pkg.export_range(idx);
    read_props(&mut Reader::at(&pkg.data[..e], s), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())
}

pub fn presentation(src: &ClipSourceDir, wid: &PropList, wid_pkg: &Package) -> Result<Presentation, String> {
    let actor_path = path_of(wid_pkg, wid, "WeaponActorClass").ok_or("WID lacks WeaponActorClass")?;
    let pkg = loader::package(src, pkg_of(&actor_path))?;
    let p = cdo(&pkg)?;
    let b = |k: &str| p.bool(k).unwrap_or(false);
    Ok(Presentation {
        actor: pkg_of(&actor_path).to_owned(),
        aim_from_muzzle_at_close_range: b("bShouldAimFromMuzzleAtCloseRange"), maintain_aim_during_targeting: b("bMaintainAimLocationDuringTargeting"), use_scope_targeting: b("bUseScopeTargeting"),
        muzzle_particle: path_of(&pkg, &p, "MuzzleParticleSystem"), tracer_template: path_of(&pkg, &p, "TracerTemplate"), beam_particle: path_of(&pkg, &p, "BeamParticleSystem"),
        camera_base_3p: path_of(&pkg, &p, "CameraBase3PClass"), camera_targeting_3p: path_of(&pkg, &p, "CameraTargeting3PClass"),
        fire_sounds: indexed_paths(&pkg, &p, "PrimaryFireSound"),
        impact_effects: indexed_paths(&pkg, &p, "ImpactPhysicalSurfaceEffects"), impact_sounds: indexed_paths(&pkg, &p, "ImpactPhysicalSurfaceSounds"),
        projectile_template: path_of(wid_pkg, wid, "ProjectileTemplate"),
    })
}

pub fn fire_ability(src: &ClipSourceDir, wid: &PropList, wid_pkg: &Package) -> Result<FireAbility, String> {
    let path = path_of(wid_pkg, wid, "PrimaryFireAbility").ok_or("WID lacks PrimaryFireAbility")?;
    let name = pkg_of(&path).to_owned();
    let pkg = loader::package(src, &name)?;
    let p = cdo(&pkg)?;
    let mut a = FireAbility { package: name, ..Default::default() };
    if let Some(v) = p.get("WeaponFireCue") { a.weapon_fire_cue = tag_names(&pkg, v); }
    if let Some(v) = p.get("AbilityTags") { a.ability_tags = tag_names(&pkg, v); }
    if let Some(t) = p.get("WeaponTestApplicationTag").and_then(PropValue::as_fields).and_then(|f| f.text("TagName")) { a.application_tags.push(t.to_owned()); }
    for (ci, c) in p.0.iter().filter(|x| x.name == "EffectContainers").enumerate() {
        let Some(c) = c.value.as_fields() else { continue };
        if let Some(t) = c.fields("ApplicationTag").and_then(|f| f.text("TagName")) { a.application_tags.push(t.to_owned()); }
        if let Some(v) = c.get("ImpactCues") { for t in tag_names(&pkg, v) { if !a.impact_cues.contains(&t) { a.impact_cues.push(t); } } }
        if ci == 0 { if let Some(arr) = c.array("TargetGameplayEffectClasses") { for e in arr { if let PropValue::Object(i) = e { if *i < 0 { a.effect_classes.push(pkg.import_path(*i)); } } } } }
        let Some(ts) = c.fields("TargetSelection") else { continue };
        if let Some(v) = ts.float("MaxYawAngleToTarget") { a.max_yaw_angle_to_target.get_or_insert(v as f32); }
        for s in ts.array("List").map(|v| v.as_slice()).unwrap_or(&[]) {
            let Some(s) = s.as_fields() else { continue };
            let flt = s.fields("TargetFilter");
            a.selections.push(TargetSelection {
                shape: s.text("Shape").unwrap_or("").to_owned(), test_type: s.text("TestType").unwrap_or("").to_owned(), primary_source: s.text("PrimarySource").unwrap_or("").to_owned(),
                range: s.float("Range").unwrap_or(0.) as f32, cone_yaw: s.float("ConeYawAngle").unwrap_or(0.) as f32, cone_pitch: s.float("ConePitchAngle").unwrap_or(0.) as f32, cone_min_radius: s.float("ConeMinRadius").unwrap_or(0.) as f32,
                actor_type_filter: flt.and_then(|f| f.text("ActorTypeFilter")).unwrap_or("").to_owned(), exclude_pawn_friends: flt.and_then(|f| f.bool("bExcludePawnFriends")).unwrap_or(false),
                exclude_obstructed_by_world: s.bool("bExcludeObstructedByWorld").unwrap_or(false), max_targets: s.int("MaxTargets").unwrap_or(0) as i32 });
        }
    }
    Ok(a)
}

/// Loads the stat row + weapon actor + fire ability of a WID package (`/Game/Athena/Items/Weapons/WID_...`).
pub fn load(src: &ClipSourceDir, wid_package: &str) -> Result<GunHit, String> {
    let pkg = loader::package(src, pkg_of(wid_package))?;
    let idx = pkg.exports.iter().position(|e| e.object_name == pkg.exports[0].object_name).unwrap_or(0);
    let (s, e) = pkg.export_range(idx);
    let wid = read_props(&mut Reader::at(&pkg.data[..e], s), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }).map_err(|e| e.to_string())?;
    let h = wid.fields("WeaponStatHandle").ok_or("WID lacks WeaponStatHandle")?;
    let table = h.object("DataTable").filter(|i| *i < 0).map(|i| pkg.import_path(i)).ok_or("stat handle has no DataTable")?;
    let row = h.text("RowName").ok_or("stat handle has no RowName")?.to_owned();
    let stats = StatRow::from_props(&data_table_row(src, &table, &row)?)?;
    let pres = presentation(src, &wid, &pkg)?;
    let projectile = match &pres.projectile_template { Some(t) => Some(self::projectile(src, t)?), None => None };
    Ok(GunHit { wid: wid_package.to_owned(), row, stats, ability: fire_ability(src, &wid, &pkg)?, pres, projectile })
}


