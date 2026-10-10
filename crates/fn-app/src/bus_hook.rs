use crate::building_hook::{sim_to_world, world_to_fx};
use crate::pickups_dxbc::Dxbc;
use fn_audio_sink::{SoundEvent, DROP_CUES};
use fn_fx::view::ViewParams;
use fn_game::drop_in::{AerialPhase, DropEvent};
use fn_game::GameSim;
use fn_render::fx::{FxFrame, FxRenderer};
use fn_ui::screens::aerial_feedback::{AerialAction, AerialPrompt};
use glam::{Mat3, Mat4, Vec3};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

pub const BUS: &str = "/Game/Athena/Aircraft/Meshes/Vehicle_Athena_Bus.Vehicle_Athena_Bus";
pub const BALLOON: &str = "/Game/Athena/Aircraft/Meshes/Vehicle_Athena_Bus_Balloon_Full.Vehicle_Athena_Bus_Balloon_Full";
pub const APPARATUS: &str = "/Game/Athena/Aircraft/Meshes/Vehicle_Athena_Bus_Apparatus.Vehicle_Athena_Bus_Apparatus";
pub const FLAME: &str = "/Game/Missions/Primary/LaunchTheBalloon/Meshes_NoLOD/BalloonFlame.BalloonFlame";
pub const GENERATOR: &str = "/Game/Missions/Primary/LaunchTheBalloon/Meshes_NoLOD/LaunchTheBalloon_van_generator.LaunchTheBalloon_van_generator";
pub const BUS_YAW: f32 = -90.;
pub const FLAME_LOCATION: [f32; 3] = [0., -32., 520.];
pub const FLAME_SCALE: f32 = 1.6;
pub const GENERATOR_LOCATION: [f32; 3] = [76.5, 506., 360.];
pub const GENERATOR_YAW: f32 = -140.70;

const KEY_BUS_LOOP: u32 = 0x4255_0001;
const KEY_BUS_MUSIC: u32 = 0x4255_0002;
const KEY_SKYDIVE: u32 = 0x4255_0003;
const KEY_GLIDER: u32 = 0x4255_0004;
const SKYDIVE_FADE_OUT: f32 = 0.5;

#[derive(Default)]
struct State { install: Option<PathBuf>, dxbc: Option<Result<Dxbc, String>>, errors: HashSet<String>, bus_audio: bool, skydive_audio: bool, glider_audio: bool }
static STATE: Mutex<Option<State>> = Mutex::new(None);
fn with<R>(f: impl FnOnce(&mut State) -> R) -> R { let mut g = STATE.lock().unwrap_or_else(|p| p.into_inner()); f(g.get_or_insert_with(State::default)) }

pub fn init(install: Option<PathBuf>) { with(|s| s.install = install); }

fn crt_rand(seed: &mut u32) -> f32 { *seed = seed.wrapping_mul(214_013).wrapping_add(2_531_011); ((*seed >> 16) & 0x7fff) as f32 / 32767. }

pub fn enabled(deterministic: bool) -> bool {
    match fn_live::var("FN_BUS").ok().as_deref().map(str::trim) { Some("1") => true, Some("0") => false, _ => !deterministic }
}

pub fn start_bus_now(sim: &mut GameSim) {
    let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u32);
    let (a, b) = (crt_rand(&mut seed), crt_rand(&mut seed));
    sim.fly = fn_game::FlyMode::Walk;
    sim.controller = sim.new_controller();
    sim.start_match_on_bus(a, b);
    eprintln!("[bus] startbus: a fresh Battle Bus takes off");
}

pub fn begin_match(sim: &mut GameSim, deterministic: bool) {
    if !enabled(deterministic) { return; }
    let mut seed = match fn_live::var("FN_BUS_SEED").ok().and_then(|v| v.trim().parse::<u32>().ok()) { Some(v) => v, None => std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u32) };
    let (a, b) = (crt_rand(&mut seed), crt_rand(&mut seed));
    let skip_warmup = fn_live::var("FN_BUS_SKIP_WARMUP").map_or(false, |v| v.trim() == "1");
    if skip_warmup { sim.start_match_on_bus(a, b); } else {
        let s = crt_rand(&mut seed);
        sim.start_match_on_spawn_island(s, a, b, fn_game::drop_in::WARMUP_COUNTDOWN_DURATION);
        eprintln!("[bus] warmup on the spawn island: {:.0} s countdown at player start {:.0} of {}", fn_game::drop_in::WARMUP_COUNTDOWN_DURATION, s * fn_game::drop_in::WARMUP_STARTS.len() as f32, fn_game::drop_in::WARMUP_STARTS.len());
    }
    if let Some(d) = sim.drop.as_ref() { if let Some(ac) = d.aircraft.as_ref() {
        let p = &ac.path;
        eprintln!("[bus] match starts on the Battle Bus: yaw {:.1} deg, start ({:.0}, {:.0}, {:.0}) cm, flight {:.1} s, doors open at {:.1} s, last stop at {:.1} s", p.yaw_deg, p.start.x, p.start.y, p.start.z, p.duration, p.drop_start_offset, p.drop_end_offset);
    } }
}

fn cue(path: &str) -> &'static str { DROP_CUES.iter().copied().find(|c| *c == path).unwrap_or("") }

pub fn frame(sim: &mut GameSim) -> Vec<SoundEvent> {
    let mut out = vec![];
    crate::statics_render::set_stream_hint(landing_hint(sim));
    crate::statics_render::set_read_hint(bus_ground(sim));
    let Some(d) = sim.drop.as_mut() else { return out };
    let events = d.take_events();
    let bus = d.aircraft_view().map(|(p, _)| p);
    let pawn = sim.feet();
    with(|s| {
        for e in &events {
            match e {
                DropEvent::ExitedAircraft { auto } => { eprintln!("[bus] left the bus ({})", if *auto { "auto-kick at the last stop" } else { "jump" }); out.push(SoundEvent::BuildingCue { cue: cue(DROP_CUES[2]), pos: None }); }
                DropEvent::DoorsOpened => eprintln!("[bus] doors open"),
                DropEvent::AircraftExitedDropZone => eprintln!("[bus] last stop: the bus left the drop zone"),
                DropEvent::FlightEnded => eprintln!("[bus] flight ended"),
                DropEvent::ParachuteOpened => { eprintln!("[bus] glider open"); out.push(SoundEvent::BuildingCue { cue: cue(DROP_CUES[5]), pos: Some(pawn) }); }
                DropEvent::ParachuteClosed => { eprintln!("[bus] glider closed"); out.push(SoundEvent::BuildingCue { cue: cue(DROP_CUES[6]), pos: Some(pawn) }); }
                DropEvent::Landed => eprintln!("[bus] landed at ({:.1}, {:.1}, {:.1})", pawn.x, pawn.y, pawn.z),
                DropEvent::EnteredAircraft => eprintln!("[bus] entered the Battle Bus"),
            }
        }
        match bus {
            Some(p) if !s.bus_audio => { s.bus_audio = true; out.push(SoundEvent::ContainerLoop { cue: cue(DROP_CUES[0]), key: KEY_BUS_LOOP, pos: Some(p) }); out.push(SoundEvent::ContainerLoop { cue: cue(DROP_CUES[1]), key: KEY_BUS_MUSIC, pos: Some(p) }); }
            Some(p) => { out.push(SoundEvent::LoopMove { key: KEY_BUS_LOOP, pos: p }); out.push(SoundEvent::LoopMove { key: KEY_BUS_MUSIC, pos: p }); }
            None if s.bus_audio => { s.bus_audio = false; out.push(SoundEvent::ContainerLoopStop { key: KEY_BUS_LOOP, fade: 0. }); out.push(SoundEvent::ContainerLoopStop { key: KEY_BUS_MUSIC, fade: 0. }); }
            None => {}
        }
        let c = sim.controller.component();
        let sky = !sim.on_bus() && c.is_skydiving() && !c.is_parachute_open();
        let glide = !sim.on_bus() && c.is_parachute_open();
        if sky != s.skydive_audio { s.skydive_audio = sky; out.push(if sky { SoundEvent::ContainerLoop { cue: cue(DROP_CUES[3]), key: KEY_SKYDIVE, pos: None } } else { SoundEvent::ContainerLoopStop { key: KEY_SKYDIVE, fade: SKYDIVE_FADE_OUT } }); }
        if glide != s.glider_audio { s.glider_audio = glide; out.push(if glide { SoundEvent::ContainerLoop { cue: cue(DROP_CUES[4]), key: KEY_GLIDER, pos: None } } else { SoundEvent::ContainerLoopStop { key: KEY_GLIDER, fade: 0. } }); }
    });
    out.retain(|e| !matches!(e, SoundEvent::BuildingCue { cue: "", .. } | SoundEvent::ContainerLoop { cue: "", .. }));
    out
}

pub const HINT_MAX_DRIFT_M: f32 = 1000.;

fn landing_hint(sim: &GameSim) -> Option<Vec3> {
    if sim.drop.is_none() || sim.on_bus() { return None; }
    let c = sim.controller.component();
    if !c.is_skydiving() && !c.in_aerial_mode() { return None; }
    let p = sim.controller.render_position();
    let z = p.y + fn_game::Z_OFFSET;
    let g = sim.ground_y(p.x, z);
    let fall = (-sim.controller.vertical_speed).max(1.);
    let t = (sim.controller.height - g).max(0.) / fall;
    let d = (sim.controller.velocity * t).clamp_length_max(HINT_MAX_DRIFT_M);
    Some(Vec3::new(p.x + d.x, g, z + d.y))
}

fn bus_ground(sim: &GameSim) -> Option<Vec3> {
    if !sim.on_bus() { return None; }
    let (p, _) = sim.drop.as_ref()?.aircraft_view()?;
    Some(Vec3::new(p.x, sim.ground_y(p.x, p.z), p.z))
}

pub fn hud(sim: &GameSim, st: &mut fn_ui::screens::hud::HudState, jump_key: &str) {
    if sim.drop.is_none() { return; }
    crate::statics_render::set_on_bus(matches!(sim.aerial_phase(), AerialPhase::BusCantExit | AerialPhase::BusCanExit));
    let action = match sim.aerial_phase() { AerialPhase::BusCanExit => Some(AerialAction::Jump), AerialPhase::Skydiving => Some(AerialAction::DeployGlider), AerialPhase::Parachuting => Some(AerialAction::Skydive), _ => None };
    st.world.aerial = action.map(|action| AerialPrompt { key: jump_key.to_owned(), action });
    if sim.on_bus() {
        st.visible.reticle = false; st.visible.player_health = false; st.visible.pickup_info = false; st.visible.quick_bar = false;
        st.kill_feed.clear(); st.world.ammo = None; st.world.equip = None; st.world.interaction = None; st.world.target = None;
    }
}

fn ue_yaw(deg: f32) -> Mat3 { Mat3::from_rotation_z(deg.to_radians()) }

pub fn fx_frame(fx: &mut FxRenderer, view: &ViewParams, sim: &GameSim, out: &mut FxFrame) {
    let Some(d) = sim.drop.as_ref() else { return };
    let Some(ac) = d.aircraft.as_ref() else { return };
    let Some((pos, _)) = d.aircraft_view() else { return };
    let root = Mat4::from_translation(world_to_fx(sim_to_world(pos))) * Mat4::from_mat3(ue_yaw(ac.path.yaw_deg));
    let bus = root * Mat4::from_mat3(ue_yaw(BUS_YAW));
    let parts = [
        (BUS, bus), (BALLOON, bus), (APPARATUS, bus),
        (FLAME, bus * Mat4::from_translation(Vec3::from(FLAME_LOCATION)) * Mat4::from_scale(Vec3::splat(FLAME_SCALE))),
        (GENERATOR, bus * Mat4::from_translation(Vec3::from(GENERATOR_LOCATION)) * Mat4::from_mat3(ue_yaw(GENERATOR_YAW))),
    ];
    with(|s| {
        if s.dxbc.is_none() { s.dxbc = Some(match s.install.as_ref() { Some(i) => Dxbc::load(i, fx), None => Err("no install".into()) }); }
        let State { dxbc, errors, .. } = s;
        let dx = match dxbc.as_mut().unwrap() { Ok(d) => d, Err(e) => { if errors.insert("dxbc".into()) { eprintln!("[bus] bus meshes unavailable: {e}"); } return; } };
        dx.begin(view);
        for (mesh, m) in parts {
            if let Err(e) = dx.draw_static_item(fx, mesh, m.w_axis.truncate(), Mat3::from_mat4(m), [0.; 3]) { if errors.insert(mesh.to_owned()) { eprintln!("[bus] {mesh}: NOT DRAWN: {e}"); } }
        }
        let fr = dx.take_frame();
        out.buffer_writes.extend(fr.buffer_writes);
        out.draws.extend(fr.draws);
        out.shadow_draws.extend(fr.shadow_draws);
        out.distance_field_objects.extend(fr.distance_field_objects);
    });
}
