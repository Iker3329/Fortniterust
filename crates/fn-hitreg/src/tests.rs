use super::*;
use fn_combat::{harvest_fx::{FxEvent, Rng}, Harvestable, Kind, World};
use fn_pose::ClipSourceDir;
use glam::Vec3;

const WID: [&str; 3] = [
    "/Game/Athena/Items/Weapons/WID_Assault_Auto_Athena_C_Ore_T02",
    "/Game/Athena/Items/Weapons/WID_Shotgun_Standard_Athena_C_Ore_T03",
    "/Game/Athena/Items/Weapons/WID_Sniper_BoltAction_Scope_Athena_R_Ore_T03",
];
fn guns() -> Option<Vec<GunHit>> {
    let root = std::env::var_os("FN_TEST_ROOT")?;
    let paks = fn_pak::PakSet::open_install(&root).unwrap();
    // the PakSet must outlive the borrow: leak it for the test process
    let paks: &'static fn_pak::PakSet = Box::leak(Box::new(paks));
    Some(WID.iter().map(|w| load(&ClipSourceDir::Pak(paks), w).unwrap()).collect())
}
fn wor(props: Vec<Harvestable>) -> World { World { props, ..Default::default() } }

#[test]
fn stat_rows_wire_blueprints_and_projectile_are_read_from_the_install() {
    let Some(g) = guns() else { eprintln!("FN_TEST_ROOT not set, skipping"); return };
    let ar = &g[0];
    assert_eq!(ar.row, "Assault_Auto_Athena_C_Ore_T03");
    assert_eq!((ar.stats.spread, ar.stats.spread_downsights, ar.stats.standing_still_spread_mult, ar.stats.crouching_spread_mult, ar.stats.jumping_falling_spread_mult, ar.stats.sprinting_spread_mult), (0.15, 0.7, 0.55, 0.8, 1.25, 1.5));
    assert_eq!((ar.stats.min_speed_for_spread_mult, ar.stats.max_speed_for_spread_mult), (90., 100.));
    assert_eq!(ar.stats.heat, [(2.85, 8.75), (4.1, 8.75), (5.5, 10.)]);
    assert_eq!(ar.stats.athena_recoil_magnitude, (4., 4.));
    assert_eq!((ar.stats.athena_recoil_angle, ar.stats.athena_recoil_interp_speed, ar.stats.athena_recoil_recovery_interp_speed, ar.stats.athena_recoil_downsights_mult), ((-25., 25.), 12., 6., 0.5));
    assert_eq!((ar.stats.dmg[0], ar.stats.env_dmg[0], ar.stats.rng, ar.stats.zone_critical, ar.stats.trace_range_m()), (32., 35., [2500., 3750., 25000., 25000.], 2.5, 250.));
    assert!((ar.stats.env_dmg[1] - 24.99).abs() < 1e-3);
    assert!(ar.pres.aim_from_muzzle_at_close_range && ar.pres.maintain_aim_during_targeting && !ar.pres.use_scope_targeting);
    assert!(ar.pres.tracer_template.as_deref().unwrap().ends_with("TracerGeneric_Athena_C") || ar.pres.tracer_template.as_deref().unwrap().contains("TracerGeneric_Athena"));
    assert!(ar.pres.impact_sound(1).unwrap().contains("WOOD_Bullet_Impact") && ar.pres.impact_sound(2).unwrap().contains("STONE_Bullet_Impact"));
    assert!(ar.pres.impact_effect(1).unwrap().contains("P_Impact_Wood") && ar.pres.impact_effect(2).unwrap().contains("P_Impact_Stone") && ar.pres.impact_effect(0).unwrap().contains("P_Impact_Default"));
    assert_eq!(ar.ability.package, "/Game/Abilities/Weapons/Ranged/GA_Ranged_GenericDamage");
    assert_eq!((ar.ability.selections[0].shape.as_str(), ar.ability.selections[0].range, ar.ability.selections[0].exclude_obstructed_by_world, ar.ability.selections[0].max_targets), ("EFortTargetSelectionShape::Cone", 5000., true, -1));
    assert_eq!(ar.ability.impact_cues, ["GameplayCue.Impact.Physical"]);
    assert_eq!(ar.ability.weapon_fire_cue, ["GameplayCue.Weapons.Activation"]);
    assert!(ar.projectile.is_none());
    let sg = &g[1];
    assert_eq!((sg.stats.bullets_per_cartridge, sg.stats.dmg[0], sg.stats.env_dmg[0], sg.stats.rng[3]), (10, 9., 10., 1500.));
    let sn = &g[2];
    let pr = sn.projectile.as_ref().expect("sniper is a projectile weapon");
    assert_eq!((pr.initial_speed, pr.max_speed, pr.initial_speed_row.as_str()), (30000., 30000., "Sniper_BoltAction_InitialSpeed"));
    assert_eq!((sn.stats.spread_downsights, sn.stats.dmg[0], sn.stats.env_dmg[0]), (0., 105., 105.));
    assert!(sn.pres.use_scope_targeting);
}

#[test]
fn bullets_damage_props_through_their_health_with_surface_events() {
    let Some(g) = guns() else { return };
    let mut world = wor(vec![Harvestable::new(Kind::Tree, Vec3::new(0., 0., 6.))]);
    let shot = Shot { eye: Vec3::new(0., 1.75, 12.), forward: Vec3::new(0., 0., -1.) };
    let mut rng = Rng(7);
    let rep = fire(&g[0], &mut world, &shot, &AimState::default(), Policy::default(), &mut rng);
    assert_eq!((rep.pellets, rep.misses, rep.impacts.len()), (1, 0, 1));
    let i = &rep.impacts[0];
    assert_eq!((i.prop, i.damage, i.surface, i.destroyed), (Some(0), 35., 1, false));
    assert!((i.distance_m - 5.6).abs() < 1e-3, "tree trunk radius 0.4 at z=6 -> hit at z=6.4");
    assert!(i.sound.as_deref().unwrap().contains("WOOD_Bullet_Impact") && i.effect.as_deref().unwrap().contains("P_Impact_Wood"));
    assert_eq!(world.props[0].health, 565.);
    for _ in 0..17 { fire(&g[0], &mut world, &shot, &AimState::default(), Policy::default(), &mut rng); }
    assert!(!world.props[0].alive(), "18 x 35 = 630 >= 600");
    assert!(world.fx.events.iter().any(|e| matches!(e, FxEvent::Broke { object: _, .. })));
    // ground
    let down = Shot { eye: Vec3::new(20., 1.75, 12.), forward: Vec3::new(0., -1., -1.).normalize() };
    let r = fire(&g[0], &mut world, &down, &AimState::default(), Policy::default(), &mut rng);
    assert!(r.impacts[0].prop.is_none() && (r.impacts[0].pos.y).abs() < 1e-4 && r.impacts[0].surface == 0);
    // shotgun: 10 pellets x EnvDmg 10 at point blank with no spread logic
    let mut w2 = wor(vec![Harvestable::new(Kind::Rock, Vec3::new(0., 0., 6.))]);
    let r = fire(&g[1], &mut w2, &Shot { eye: Vec3::new(0., 0.7, 9.), forward: Vec3::new(0., 0., -1.) }, &AimState::default(), Policy::default(), &mut rng);
    assert_eq!((r.pellets, r.impacts.len(), r.impacts[0].surface), (10, 10, 2));
    assert_eq!(w2.props[0].health, 450. - 100.);
}

#[test]
fn stand_in_spread_falloff_and_recoil_follow_the_data_columns() {
    let Some(g) = guns() else { return };
    let a = Policy { assume_unrecovered: true };
    let s = &g[0].stats;
    let moving = AimState { speed_cm_s: 300., ..Default::default() };
    assert_eq!(s.spread_multiplier(&AimState::default(), Policy::default()), 1.);
    assert!((s.spread_multiplier(&moving, a) - 1.).abs() < 1e-6);
    assert!((s.spread_multiplier(&AimState { ads: true, speed_cm_s: 300., ..Default::default() }, a) - 0.7).abs() < 1e-6);
    assert!((s.spread_multiplier(&AimState::default(), a) - 0.55).abs() < 1e-6, "standing still");
    assert!((s.spread_multiplier(&AimState { sprinting: true, speed_cm_s: 300., ..Default::default() }, a) - 1.5).abs() < 1e-6);
    assert!((s.spread_deg(&AimState { crouched: true, speed_cm_s: 300., ..Default::default() }, a) - 0.12).abs() < 1e-6);
    assert_eq!(s.damage_at(&s.env_dmg, 2000., Policy::default()), 35.);
    assert_eq!(s.damage_at(&s.env_dmg, 20000., Policy::default()), 35., "default keeps the point-blank value (native interpolation left out)");
    assert_eq!(s.damage_at(&s.env_dmg, 2500., a), 35.);
    assert!((s.damage_at(&s.env_dmg, 3750., a) - 24.99).abs() < 1e-3);
    assert!((s.damage_at(&s.env_dmg, 3125., a) - (35. + 24.99) / 2.).abs() < 1e-3);
    assert_eq!(s.dmg[0] * s.zone_multiplier(Zone::Critical), 80.);
    // recoil: default policy is inert; stand-in kicks and recovers
    let mut rng = Rng(3);
    let mut r = Recoil::default(); r.kick(s, false, &mut rng, Policy::default()); assert_eq!(r.target, (0., 0.));
    r.kick(s, false, &mut rng, a);
    let kicked = r.target; assert!(kicked.0 > 3.6 && kicked.0 <= 4.0, "4 deg at <=25 deg off vertical: {kicked:?}");
    for _ in 0..30 { r.update(s, 1. / 60., true, a); }
    assert!((r.current.0 - kicked.0).abs() < 0.2);
    for _ in 0..180 { r.update(s, 1. / 60., false, a); }
    assert!(r.current.0.abs() < 0.05);
    // spread directions stay inside the cone
    let d = Vec3::new(0.3, 0.1, -1.).normalize();
    for _ in 0..200 { let v = shoot::spread_direction(d, 2., &mut rng); assert!(v.dot(d).clamp(-1., 1.).acos().to_degrees() <= 2.001); }
}

#[test]
fn sniper_projectile_flies_at_the_recovered_speed_and_damages_a_rock() {
    let Some(g) = guns() else { return };
    let sn = &g[2];
    let mut world = wor(vec![Harvestable::new(Kind::Rock, Vec3::new(0., 0., -90.))]);
    let shot = Shot { eye: Vec3::new(0., 1.0, 12.), forward: Vec3::new(0., 0., -1.) };
    let mut rng = Rng(1);
    let mut p = Projectile::launch(sn, &shot, &AimState::default(), Policy::default(), &mut rng).unwrap();
    assert!((p.vel.length() - 300.).abs() < 1e-3);
    let mut hit = None; let mut t = 0.0f32;
    while hit.is_none() && !p.dead() && t < 5. { hit = p.step(sn, &mut world, 1. / 120., 0., 400., Policy::default()); t += 1. / 120.; }
    let h = hit.expect("rock hit");
    assert_eq!((h.prop, h.damage, h.surface), (Some(0), 105., 2));
    assert!((t - (12. + 90. - 1.0) / 300.).abs() < 0.02, "flight time {t}");
    assert_eq!(world.props[0].health, 450. - 105.);
}

