//! Instance parameters (`SetFloatParameter` / `SetVectorParameter` / `SetColorParameter`), `ParticleModuleSizeScale`, `ParticleModuleSpawnPerUnit`, `SetEmitterEnable`, axis lock, on
//! synthetic systems (no game data needed).
use fn_particles::dist::{Dist, ParamDist, ParamMode};
use fn_particles::model::*;
use fn_particles::{Env, Instance, Params, Transform};
use glam::Vec3;
use std::sync::Arc;

fn module(kind: ModuleKind) -> Module { Module { kind, enabled: true, lod_validity: 15, seed: None, name: String::new() } }
fn lod(modules: Vec<ModuleKind>, spawn: Spawn, req: Required) -> Lod { Lod { level: 0, enabled: true, required: req, spawn, type_data: TypeData::Sprite, modules: modules.into_iter().map(module).collect(), peak_active: 0 } }
fn sys(emitters: Vec<(&str, Lod)>) -> Arc<ParticleSystem> {
    Arc::new(ParticleSystem { name: "T".into(), emitters: emitters.into_iter().map(|(n, l)| Emitter { name: n.into(), lods: vec![l] }).collect(), lod_distances: vec![0.0], ..Default::default() })
}
fn burst(n: i32) -> Spawn { Spawn { rate: Dist::constant(0.0), rate_scale: Dist::constant(1.0), burst_scale: Dist::constant(1.0), bursts: vec![Burst { count: n, count_low: -1, time: 0.0 }] } }
fn rate(r: f32) -> Spawn { Spawn { rate: Dist::constant(r), rate_scale: Dist::constant(1.0), burst_scale: Dist::constant(1.0), bursts: vec![] } }
fn req(duration: f32, loops: i32, local: bool) -> Required { Required { emitter_duration: duration, emitter_loops: loops, use_local_space: local, ..Default::default() } }
fn life(l: f32) -> ModuleKind { ModuleKind::Lifetime { lifetime: Dist::constant(l) } }

/// `Constant` c, inputs 0..1 -> outputs lo..hi, `DPM_Normal`.
fn float_param(name: &str, c: f32, lo: f32, hi: f32) -> Dist {
    Dist::Param(ParamDist { name: name.into(), vector: false, constant: [c; 3], min_input: [0.0; 3], max_input: [1.0; 3], min_output: [lo; 3], max_output: [hi; 3], modes: [ParamMode::Normal; 3], set: None })
}
/// Vector parameter used as is (`DPM_Direct`), `Constant` c.
fn vec_param_direct(name: &str, c: [f32; 3]) -> Dist {
    Dist::Param(ParamDist { name: name.into(), vector: true, constant: c, min_input: [0.0; 3], max_input: [1.0; 3], min_output: [0.0; 3], max_output: [1.0; 3], modes: [ParamMode::Direct; 3], set: None })
}
fn rng() -> fn_particles::Rng { fn_particles::Rng::new(1) }
fn size_start(m: &Module) -> &Dist { match &m.kind { ModuleKind::Size { start } => start, _ => unreachable!() } }

#[test] fn an_unset_parameter_evaluates_to_its_constant_through_the_map_exactly_like_the_collapsed_constant() {
    let mut r = rng();
    // DPM_Normal: Constant 0.7 through 0..1 -> 0..10; clamps; gradient 0 when MaxInput <= MinInput
    assert!((float_param("X", 0.7, 0.0, 10.0).eval1(0.0, &mut r) - 7.0).abs() < 1e-5);
    assert_eq!(float_param("X", 5.0, 0.0, 10.0).eval1(0.0, &mut r), 10.0, "clamped to MaxInput");
    let flat = Dist::Param(ParamDist { name: "X".into(), vector: false, constant: [3.0; 3], min_input: [1.0; 3], max_input: [1.0; 3], min_output: [4.0; 3], max_output: [9.0; 3], modes: [ParamMode::Normal; 3], set: None });
    assert_eq!(flat.eval1(0.0, &mut r), 4.0, "MaxInput <= MinInput: gradient 0, output MinOutput");
    // DPM_Direct returns Constant unchanged, DPM_Abs takes |Constant| first
    assert_eq!(vec_param_direct("C", [2.0, -3.0, 4.0]).eval3(0.0, &mut r), [2.0, -3.0, 4.0]);
    let abs = Dist::Param(ParamDist { name: "A".into(), vector: false, constant: [-0.25; 3], min_input: [0.0; 3], max_input: [1.0; 3], min_output: [0.0; 3], max_output: [1.0; 3], modes: [ParamMode::Abs; 3], set: None });
    assert_eq!(abs.eval1(0.0, &mut r), 0.25);
    // the system level: nothing set -> param_names lists it, evaluation untouched by an empty Params
    let s = sys(vec![("e", lod(vec![life(1.0), ModuleKind::Size { start: float_param("SizeX", 0.5, 0.0, 100.0) }], burst(1), req(1.0, 1, false)))]);
    assert_eq!(s.param_names(), vec![("SizeX".to_string(), false)]);
    let mut i = Instance::new(s.clone(), Transform::identity(), 1, 0.0); i.set_params(&Params::new()); i.tick(1.0 / 60.0, &Env::none());
    assert!((i.emitters[0].particles[0].size.x - 50.0).abs() < 1e-3);
}

#[test] fn the_parameter_lookup_follows_the_component_float_scalars_vector_any_and_case_insensitive_names() {
    let mut p = Params::new();
    p.set_float("SparkleRateScale", 2.0); p.set_vector("BGSphereRadius", [1.0, 2.0, 3.0]);
    assert_eq!(p.get_float("sparklerATEscale"), Some(2.0));
    assert_eq!(p.get_any_vector("SPARKLERATESCALE"), Some([2.0; 3]), "GetAnyVectorParameter reads a scalar replicated");
    assert_eq!(p.get_float("BGSphereRadius"), None, "GetFloatParameter reads scalars only");
    assert_eq!(p.get_any_vector("bgsphereradius"), Some([1.0, 2.0, 3.0]));
    p.set_color("Color", [0.2, 0.4, 1.0, 1.0]);
    let c = p.get_any_vector("color").unwrap();
    assert!((c[0] - 0.2).abs() < 0.01 && (c[1] - 0.4).abs() < 0.01 && c[2] == 1.0, "8-bit sRGB quantised: {c:?}");
}

#[test] fn setting_color_changes_the_color_scale_over_life_of_live_particles_and_new_ones() {
    // ColorScaleOverLife = the `Color` parameter (DPM_Direct, constant white), AlphaScale 1; start colour white
    let m = vec![life(5.0), ModuleKind::Size { start: Dist::constant3([10.0; 3]) }, ModuleKind::Color { color: Dist::constant3([1.0; 3]), alpha: Dist::constant(1.0) },
        ModuleKind::ColorScaleOverLife { color: vec_param_direct("Color", [1.0; 3]), alpha: Dist::constant(1.0) }];
    let s = sys(vec![("e", lod(m, rate(60.0), req(1.0, 0, false)))]);
    let mut i = Instance::new(s, Transform::identity(), 1, 0.0);
    i.run(0.2, 1.0 / 60.0, &Env::none());
    assert_eq!(&i.emitters[0].particles[0].color[..3], &[1.0; 3], "unset: the white Constant");
    i.set_vector_parameter("color", [0.2, 0.4, 1.0]);       // FName lookup is case-insensitive
    i.tick(1.0 / 60.0, &Env::none());
    for p in &i.emitters[0].particles { assert_eq!(&p.color[..3], &[0.2, 0.4, 1.0], "live particles follow the parameter on the next tick"); }
    i.set_color_parameter("Color", [1.0, 0.0, 0.0, 1.0]);
    i.tick(1.0 / 60.0, &Env::none());
    assert_eq!(&i.emitters[0].particles.last().unwrap().color[..3], &[1.0, 0.0, 0.0]);
    // bake: a copy whose set parameters became constants
    let mut p = Params::new(); p.set_vector("Color", [0.5, 0.25, 0.125]);
    let baked = i.system.bake(&p);
    assert!(matches!(&baked.emitters[0].lods[0].modules[3].kind, ModuleKind::ColorScaleOverLife { color: Dist::Const(c), .. } if *c == [0.5, 0.25, 0.125]));
    // new_with_params: set before the first tick
    let one = sys(vec![("e", lod(vec![life(5.0), ModuleKind::Size { start: Dist::constant3([10.0; 3]) }, ModuleKind::ColorScaleOverLife { color: vec_param_direct("Color", [1.0; 3]), alpha: Dist::constant(1.0) }], burst(1), req(1.0, 1, false)))]);
    let mut j = Instance::new_with_params(one, Transform::identity(), 1, 0.0, &p);
    j.tick(1.0 / 60.0, &Env::none()); assert_eq!(&j.emitters[0].particles[0].color[..3], &[0.5, 0.25, 0.125]);
}

#[test] fn a_float_parameter_drives_spawn_time_values_and_the_rate_scale() {
    let mut spawn = rate(60.0); spawn.rate_scale = float_param("SparkleRateScale", 1.0, 0.0, 1.0);
    // RateScale = parameter through 0..1 -> 0..1: 0.5 halves the 60/s rate
    let s = sys(vec![("Sparkle", lod(vec![life(10.0), ModuleKind::Size { start: float_param("SizeSparkle", 0.1, 0.0, 100.0) }], spawn, req(1.0, 0, false)))]);
    let mut i = Instance::new(s.clone(), Transform::identity(), 1, 0.0);
    i.run(1.0, 1.0 / 60.0, &Env::none()); let full = i.spawned_total();
    let mut h = Instance::new(s, Transform::identity(), 1, 0.0); h.set_float_parameter("SparkleRateScale", 0.5); h.set_float_parameter("SizeSparkle", 0.25);
    h.run(1.0, 1.0 / 60.0, &Env::none());
    assert!((58..=60).contains(&full), "{full}"); assert!((28..=31).contains(&h.spawned_total()), "{}", h.spawned_total());
    assert!((h.emitters[0].particles[0].size.x - 25.0).abs() < 1e-3);
}

#[test] fn size_scale_assigns_base_size_times_the_scale_at_spawn_and_on_every_update() {
    // SizeMultiplyLife (0.5) BEFORE SizeScale is overridden by it, AFTER it multiplies on top
    let sm = |m: f32| ModuleKind::SizeMultiplyLife { mult: Dist::constant3([m; 3]), x: true, y: true, z: true };
    let before = vec![life(5.0), ModuleKind::Size { start: Dist::constant3([10.0, 20.0, 30.0]) }, sm(0.5), ModuleKind::SizeScale { scale: Dist::constant3([3.0, 2.0, 1.0]) }];
    let after = vec![life(5.0), ModuleKind::Size { start: Dist::constant3([10.0, 20.0, 30.0]) }, ModuleKind::SizeScale { scale: Dist::constant3([3.0, 2.0, 1.0]) }, sm(0.5)];
    for (m, want) in [(before, Vec3::new(30.0, 40.0, 30.0)), (after, Vec3::new(15.0, 20.0, 15.0))] {
        let mut i = Instance::new(sys(vec![("e", lod(m, burst(1), req(1.0, 1, false)))]), Transform::identity(), 1, 0.0);
        i.tick(1.0 / 60.0, &Env::none()); assert_eq!(i.emitters[0].particles[0].size, want);
        assert_eq!(i.emitters[0].particles[0].base_size, Vec3::new(10.0, 20.0, 30.0), "BaseSize untouched");
        i.run(0.5, 1.0 / 60.0, &Env::none()); assert_eq!(i.emitters[0].particles[0].size, want, "stays on every update");
    }
    // a parameter changing every tick (SizeSphere) moves the live particle
    let m = vec![life(0.0), ModuleKind::Size { start: Dist::constant3([100.0; 3]) }, ModuleKind::SizeScale { scale: vec_param_direct("SizeSphere", [1.0; 3]) }];
    let mut i = Instance::new(sys(vec![("Sphere", lod(m, burst(1), req(0.0, 0, true)))]), Transform::identity(), 1, 0.0);
    for k in 1..=10 { let s = k as f32 * 0.1; i.set_vector_parameter("SizeSphere", [s; 3]); i.tick(1.0 / 60.0, &Env::none()); assert!((i.emitters[0].particles[0].size.x - 100.0 * s).abs() < 1e-3); }
    assert_eq!(i.emitters[0].particles.len(), 1, "duration 0 never loops: one burst; lifetime 0: immortal");
}

#[test] fn set_emitter_enabled_stops_spawning_but_lets_particles_finish_and_resumes() {
    let s = sys(vec![("GodRays", lod(vec![life(0.5)], rate(60.0), req(1.0, 0, false))), ("Other", lod(vec![life(0.5)], rate(60.0), req(1.0, 0, false)))]);
    let mut i = Instance::new(s, Transform::identity(), 1, 0.0); let env = Env::none();
    i.run(0.2, 1.0 / 60.0, &env); assert!(i.emitters[0].spawned_total >= 10);
    assert!(i.set_emitter_enabled("godrays", false), "case-insensitive FName"); assert!(!i.set_emitter_enabled("nope", false));
    let n = i.emitters[0].spawned_total; let alive = i.emitters[0].particles.len(); assert!(alive > 0);
    i.run(0.1, 1.0 / 60.0, &env); assert_eq!(i.emitters[0].spawned_total, n, "nothing new while disabled"); assert!(i.emitters[1].spawned_total > n, "the other emitter keeps spawning");
    i.run(0.6, 1.0 / 60.0, &env); assert_eq!(i.emitters[0].particles.len(), 0, "the live particles finished their 0.5 s life"); assert_eq!(i.emitter_enabled("GodRays"), Some(false));
    i.set_emitter_enabled("GodRays", true); i.run(0.2, 1.0 / 60.0, &env); assert!(i.emitters[0].spawned_total > n && !i.emitters[0].particles.is_empty());
}

#[test] fn spawn_per_unit_spawns_by_distance_moved_and_nothing_when_standing_still() {
    let spu = |ignore_rate: bool, ign: [bool; 3], max: f32| ModuleKind::SpawnPerUnit { unit_scalar: 10.0, spawn_per_unit: Dist::constant(1.0), max_frame_distance: max, ignore_spawn_rate_when_moving: ignore_rate, movement_tolerance: 0.1, ignore_movement: ign };
    let mk = |m: ModuleKind, base_rate: f32| sys(vec![("Trail", lod(vec![life(10.0), m], rate(base_rate), req(1.0, 0, false)))]);
    let env = Env::none();
    // 600 cm/s along X for 1 s at 60 Hz: 10 cm per tick, UnitScalar 10, SpawnPerUnit 1 -> about one particle per tick = 60
    let mut i = Instance::new(mk(spu(false, [false; 3], 0.0), 0.0), Transform::identity(), 1, 0.0);
    i.tick(1.0 / 60.0, &env); assert_eq!(i.spawned_total(), 0, "first tick: Location == OldLocation");
    for k in 1..=60 { i.xf.pos = Vec3::new(10.0 * k as f32, 0.0, 0.0); i.tick(1.0 / 60.0, &env); }
    assert!((55..=60).contains(&i.spawned_total()), "{}", i.spawned_total());
    // world space: the particles of a frame are spread over the path (the last ones lie near the end of it)
    let xs: Vec<f32> = i.emitters[0].particles.iter().map(|p| p.loc.x).collect(); assert!(xs.windows(2).all(|w| w[1] >= w[0] - 1e-3) && xs[xs.len() - 1] > 590.0, "{xs:?}");
    let n = i.spawned_total(); i.run(0.5, 1.0 / 60.0, &env); assert_eq!(i.spawned_total(), n, "standing still: no distance, no particles");
    // ignored axis: moving along X with X ignored spawns nothing
    let mut a = Instance::new(mk(spu(false, [true, false, false], 0.0), 0.0), Transform::identity(), 1, 0.0);
    a.tick(1.0 / 60.0, &env); for k in 1..=30 { a.xf.pos = Vec3::new(10.0 * k as f32, 0.0, 0.0); a.tick(1.0 / 60.0, &env); } assert_eq!(a.spawned_total(), 0);
    // MaxFrameDistance: a teleport spawns nothing
    let mut t = Instance::new(mk(spu(false, [false; 3], 50.0), 0.0), Transform::identity(), 1, 0.0);
    t.tick(1.0 / 60.0, &env); t.xf.pos = Vec3::new(5000.0, 0.0, 0.0); t.tick(1.0 / 60.0, &env); assert_eq!(t.spawned_total(), 0);
    // bIgnoreSpawnRateWhenMoving: the base rate (30/s) only runs while standing still
    let mut m = Instance::new(mk(spu(true, [false; 3], 0.0), 30.0), Transform::identity(), 1, 0.0);
    m.tick(1.0 / 60.0, &env); m.run(0.5, 1.0 / 60.0, &env); let still = m.spawned_total(); assert!(still >= 14, "{still}");
    for k in 1..=30 { m.xf.pos = Vec3::new(10.0 * k as f32, 0.0, 0.0); m.tick(1.0 / 60.0, &env); }
    let moving = m.spawned_total() - still; assert!((25..=31).contains(&moving), "only the per-unit particles while moving: {moving}");
}

#[test] fn the_axis_lock_flag_reaches_the_draw_data() {
    let mut r = req(1.0, 1, true); r.screen_alignment = Alignment::Rectangle;
    let m = vec![life(5.0), ModuleKind::Size { start: Dist::constant3([10.0; 3]) }, ModuleKind::OrientationAxisLock { axis: AxisLock::RotateZ }];
    let mut i = Instance::new(sys(vec![("GodRays", lod(m, burst(1), r))]), Transform::identity(), 1, 0.0); i.tick(1.0 / 60.0, &Env::none());
    let mut out = vec![]; i.draw([0.0; 3], &mut out);
    assert_eq!(out[0].axis_lock, AxisLock::RotateZ);
    let (up, right) = out[0].axis_lock_axes.unwrap(); assert_eq!(up, [0.0; 3]); assert_eq!(right, [0.0, 1.0, 0.0], "UE Z is game Y");
    assert_eq!(AxisLock::from_ue("EPAL_NEGATIVE_Y"), AxisLock::NegY);
    let mut plain = vec![]; let mut j = Instance::new(sys(vec![("e", lod(vec![life(5.0)], burst(1), req(1.0, 1, true)))]), Transform::identity(), 1, 0.0); j.tick(1.0 / 60.0, &Env::none()); j.draw([0.0; 3], &mut plain);
    assert_eq!(plain[0].axis_lock, AxisLock::None); assert!(plain[0].axis_lock_axes.is_none());
}

#[test] fn json_of_the_old_format_loads_and_a_param_round_trips() {
    let old: Dist = serde_json::from_str(r#"{"Const":[0.5,0.5,0.5]}"#).unwrap(); assert_eq!(old, Dist::Const([0.5; 3]));
    let p = float_param("X", 0.3, 0.0, 2.0);
    let j = serde_json::to_string(&p).unwrap(); assert!(!j.contains("\"set\""), "an unset parameter keeps the export format small: {j}");
    assert_eq!(serde_json::from_str::<Dist>(&j).unwrap(), p);
    let mut s = (*sys(vec![("e", lod(vec![ModuleKind::Size { start: p }], burst(1), req(1.0, 1, false)))])).clone();
    let mut pr = Params::new(); pr.set_float("x", 1.0); s.apply_params(&pr);
    let Dist::Param(d) = size_start(&s.emitters[0].lods[0].modules[0]) else { panic!() }; assert_eq!(d.set, Some([1.0; 3])); assert_eq!(d.value(), [2.0; 3]);
    s.apply_params(&Params::new());
    let Dist::Param(d) = size_start(&s.emitters[0].lods[0].modules[0]) else { panic!() }; assert_eq!(d.set, None);
}

#[test] fn lifetime_zero_is_immortal_and_a_zero_duration_emitter_fires_its_burst_once() {
    let mut i = Instance::new(sys(vec![("Rays", lod(vec![life(0.0)], burst(1), req(0.0, 0, true)))]), Transform::identity(), 1, 0.0);
    i.run(5.0, 1.0 / 60.0, &Env::none()); assert_eq!(i.spawned_total(), 1); assert_eq!(i.live_count(), 1); assert!(!i.finished());
}
