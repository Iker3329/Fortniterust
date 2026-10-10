use crate::building_hook::{sim_to_world, world_to_fx, world_to_sim};
use crate::pickups_dxbc::Dxbc;
use fn_audio_sink::SoundEvent;
use fn_fx::view::ViewParams;
use fn_game::GameSim;
use fn_hitreg::explosive::{self as ex, BuildingCandidate, Cause, Event, Flying, Kind, Launch};
use fn_hitreg::shoot::{PawnTarget, Vitals};
use fn_hitreg::GunHit;
use fn_particles::assets::Catalog;
use fn_particles::{Env, Instance, Transform};
use fn_render::fx::{FxFrame, FxRenderer};
use fn_render::CameraParams;
use glam::{Mat3, Vec2, Vec3};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub const LOCAL_PAWN: u64 = 1;
pub const LOCAL_TEAM: u32 = 0;
pub const LOOP_KEY_BASE: u32 = 0x5200_0000;
const SUB_STEP: f32 = 1. / 120.;

#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    Spawned { id: u64, wid: String, instigator: u64, pos: Vec3, vel: Vec3 },
    Exploded { id: u64, pos: Vec3, normal: Vec3 },
}

struct Live {
    f: Flying,
    gun: Arc<GunHit>,
    wid: String,
    trail: Option<Instance>,
    loop_on: bool,
    spin: f32,
    remote: bool,
}

struct OneShot { inst: Instance }

#[derive(Default, Clone, Copy)]
struct Shake { remaining: f32, elapsed: f32, scale: f32, phase: [f32; 3], loc: [f32; 3] }

#[derive(Default)]
struct State {
    roots: Vec<PathBuf>,
    install: Option<PathBuf>,
    catalog: Option<Arc<Catalog>>,
    catalog_tried: bool,
    dxbc: Option<Result<Dxbc, String>>,
    live: Vec<Live>,
    fx: Vec<OneShot>,
    next_id: u64,
    sounds: Vec<SoundEvent>,
    net: Vec<NetEvent>,
    errors: HashSet<String>,
    log: Vec<String>,
    riding: Option<(u64, Vec3)>,
    shake: Shake,
    rnd: u64,
    stats: (u32, u32, u32, f32),
    pending_explode: Vec<u64>,
    hulls_added: (usize, usize),
}
static STATE: Mutex<Option<State>> = Mutex::new(None);
fn with<R>(f: impl FnOnce(&mut State) -> R) -> R { let mut g = STATE.lock().unwrap_or_else(|p| p.into_inner()); f(g.get_or_insert_with(|| State { next_id: 1, rnd: 0x524f_434b, ..Default::default() })) }

fn say(s: &mut State, line: String) { eprintln!("[rockets] {line}"); s.log.push(line); if s.log.len() > 400 { s.log.drain(..100); } }

pub fn init(install: Option<PathBuf>, roots: &[PathBuf]) { with(|s| { s.install = install; s.roots = roots.to_vec(); }); }

fn load_catalog(s: &mut State) {
    if s.catalog_tried || (s.roots.is_empty() && fn_live::var_os("FN_ROCKETS_FX_DIR").is_none()) { return; }
    s.catalog_tried = true;
    let dir = fn_live::var_os("FN_ROCKETS_FX_DIR").map(PathBuf::from).filter(|d| d.join("index.json").is_file()).or_else(|| s.roots.iter().map(|r| r.join("particles-rockets")).find(|d| d.join("index.json").is_file()));
    match dir.map(|d| (Catalog::load(&d), d)) {
        Some((Ok(c), d)) => { say(s, format!("particles loaded from {}", d.display())); s.catalog = Some(Arc::new(c)); }
        Some((Err(e), _)) => say(s, format!("{e}: no rocket particles")),
        None => say(s, "no `particles-rockets` folder in the asset roots: no rocket / grenade particles".into()),
    }
}

fn sim_dir_to_ue(d: Vec3) -> Vec3 { Vec3::new(d.x, d.z, d.y) }
fn fx_pos(p_sim: Vec3) -> Vec3 { world_to_fx(sim_to_world(p_sim)) }
fn game_pos(p_sim: Vec3) -> Vec3 { p_sim }

fn spawn_fx(s: &mut State, system: &Option<String>, p_sim: Vec3, rot: Mat3) -> Option<Instance> {
    let path = system.as_ref()?;
    let cat = s.catalog.clone()?;
    let name = path.split('.').next().unwrap_or(path);
    let Some(sys) = cat.system(name).or_else(|| cat.system(path)) else { if s.errors.insert(path.clone()) { say(s, format!("{path} is not in the particles-rockets catalog: not spawned")); } return None };
    s.rnd = s.rnd.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    Some(Instance::new(sys, Transform { pos: fx_pos(p_sim), rot, scale: 1. }, s.rnd, 0.))
}

fn sound(s: &mut State, path: &Option<String>, p_sim: Vec3) {
    let Some(p) = path else { return };
    match fn_audio_sink::rocket_cue(p) { Some(cue) => s.sounds.push(SoundEvent::BuildingCue { cue, pos: Some(game_pos(p_sim)) }), None => { if s.errors.insert(p.clone()) { say(s, format!("cue not in ROCKET_CUES: {p}")); } } }
}

fn rot_from_z(n_ue: Vec3) -> Mat3 {
    let z = n_ue.normalize_or_zero();
    if z == Vec3::ZERO { return Mat3::IDENTITY; }
    let pick = if z.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let x = pick.cross(z).cross(z).normalize() * -1.;
    let y = z.cross(x);
    Mat3::from_cols(x, y, z)
}
fn rot_from_x(x_ue: Vec3) -> Mat3 {
    let x = x_ue.normalize_or_zero();
    if x == Vec3::ZERO { return Mat3::IDENTITY; }
    let yaw = x.y.atan2(x.x);
    let pitch = x.z.atan2((x.x * x.x + x.y * x.y).sqrt());
    let [a, b, c] = crate::hurt_hook::ue_rotation_axes(pitch.to_degrees(), yaw.to_degrees(), 0.);
    Mat3::from_cols(a, b, c)
}

pub fn spawn(wid: &str, gun: Arc<GunHit>, instigator: u64, team: Option<u32>, l: Launch, remote: bool) -> u64 {
    with(|s| {
        load_catalog(s);
        let id = s.next_id; s.next_id += 1;
        let Some(def) = ex::def_of(&gun).cloned() else { return id };
        let trail_sys = if def.kind == Kind::Grenade { def.ribbon.clone() } else { def.trail.clone() };
        let mut trail = spawn_fx(s, &trail_sys, l.pos, rot_from_x(sim_dir_to_ue(l.vel)));
        if let (Some(t), true) = (trail.as_mut(), def.kind == Kind::Grenade) { t.set_float_parameter("TrailLifetime", def.trail_lifetime); }
        let mut loop_on = false;
        if let Some(cue) = def.in_air_sound.as_deref().and_then(fn_audio_sink::rocket_cue) { s.sounds.push(SoundEvent::ContainerLoop { cue, key: LOOP_KEY_BASE + (id as u32 & 0xffff), pos: Some(game_pos(l.pos)) }); loop_on = true; }
        s.rnd = s.rnd.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let spin = ((s.rnd >> 40) as f32 / (1u64 << 24) as f32 * 2. - 1.) * def.spin_rate_deg;
        say(s, format!("#{id} {} spawned at ({:.2}, {:.2}, {:.2}) m, {:.1} m/s, fuse {} s{}", wid.rsplit('/').next().unwrap_or(wid), l.pos.x, l.pos.y, l.pos.z, l.vel.length(), def.fuse_s, if remote { " (remote)" } else { "" }));
        s.live.push(Live { f: Flying::new(id, instigator, team, l), gun, wid: wid.to_owned(), trail, loop_on, spin, remote });
        s.stats.0 += 1;
        if !remote { s.net.push(NetEvent::Spawned { id, wid: wid.to_owned(), instigator, pos: l.pos, vel: l.vel }); if s.net.len() > 256 { s.net.drain(..128); } }
        id
    })
}

static GUNS: Mutex<Vec<Arc<GunHit>>> = Mutex::new(Vec::new());
pub fn gun_arc(gun: &GunHit) -> Arc<GunHit> {
    let mut g = GUNS.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(a) = g.iter().find(|a| a.wid == gun.wid) { return a.clone(); }
    let a = Arc::new(gun.clone()); g.push(a.clone()); a
}
pub fn fire_local(gun: &GunHit, l: Launch) -> u64 { spawn(&gun.wid, gun_arc(gun), LOCAL_PAWN, Some(LOCAL_TEAM), l, false) }
pub fn spawn_remote(wid: &str, instigator: u64, team: Option<u32>, pos: Vec3, vel: Vec3) -> Result<u64, String> {
    let gun = GUNS.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|a| a.wid == wid || a.wid.ends_with(wid)).cloned().ok_or_else(|| format!("{wid}: hit data not loaded here (register it with rocket_hook::gun_arc)"))?;
    Ok(spawn(&gun.wid.clone(), gun, instigator, team, Launch { pos, vel }, true))
}

static GIVE_AID: Mutex<Option<Vec<(u32, String)>>> = Mutex::new(None);
pub fn take_give_aid(frame: u32) -> Vec<String> {
    let mut g = GIVE_AID.lock().unwrap_or_else(|p| p.into_inner());
    let list = g.get_or_insert_with(|| std::env::var("FNRT_GIVE").ok().map(|v| v.split(';').filter(|p| !p.trim().is_empty()).map(|p| { let (w, f) = p.split_once('@').unwrap_or((p, "0")); (f.trim().parse().unwrap_or(0), w.trim().to_owned()) }).collect()).unwrap_or_default());
    let due: Vec<String> = list.iter().filter(|e| frame >= e.0).map(|e| e.1.clone()).collect();
    list.retain(|e| frame < e.0);
    due
}

pub fn explode_now(id: u64) { with(|s| s.pending_explode.push(id)); }
pub fn take_net_events() -> Vec<NetEvent> { with(|s| std::mem::take(&mut s.net)) }
pub fn live_count() -> usize { with(|s| s.live.len()) }

fn building_candidates(center_sim: Vec3, radius_m: f32) -> Vec<BuildingCandidate> {
    let c = sim_to_world(center_sim);
    let r = radius_m * 100.;
    let pieces: Vec<fn_building::world::PlacedPiece> = crate::building_hook::with_world(|w| w.iter().filter(|p| p.health > 0. && (p.location - c).length() < r + 800.).cloned().collect());
    if pieces.is_empty() { return vec![]; }
    crate::building_hook::with_catalog(|cat| {
        let Some(cat) = cat else { return vec![] };
        let mut out = vec![];
        for p in &pieces {
            let mut best: Option<(f32, Vec3)> = None;
            for h in fn_building::placement::piece_hulls(cat, p) {
                let d = h.sdf(c);
                let q = if d <= 0. { c } else { h.closest(c, Vec3::ZERO).point };
                let d = d.max(0.);
                if best.map_or(true, |b| d < b.0) { best = Some((d, q)); }
            }
            if let Some((d, q)) = best { if d <= r { out.push(BuildingCandidate { object: crate::building_damage_hook::BUILDING_OBJECT | p.id as u64, point: world_to_sim(q) }); } }
        }
        out
    })
}

fn rocket_hull(p: Vec3, r: f32) -> Option<fn_combat::Hull> {
    let mut v = vec![];
    for x in [-r, r] { for y in [-r, r] { for z in [-r, r] { v.push(p + Vec3::new(x, y, z)); } } }
    fn_combat::Hull::from_points(&v)
}

struct Blasted { id: u64, gun: Arc<GunHit>, pos: Vec3, normal: Vec3, instigator: u64, team: Option<u32>, cause: Cause, remote: bool }

fn ride_aid(sim: &mut GameSim) {
    static AT: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some(at) = *AT.get_or_init(|| std::env::var("FNRT_ROCKET_RIDE").ok().and_then(|v| v.trim().parse().ok())) else { return };
    if sim.frames < at || DONE.load(std::sync::atomic::Ordering::Relaxed) { return; }
    let top = with(|s| s.live.last().and_then(|r| ex::def_of(&r.gun).map(|d| r.f.pos + Vec3::Y * (d.capsule_radius_cm / 100.))));
    let Some(p) = top else { return };
    DONE.store(true, std::sync::atomic::Ordering::Relaxed);
    sim.controller.position = Vec2::new(p.x, p.z - fn_game::sim::Z_OFFSET);
    sim.controller.height = p.y;
    sim.controller.vertical_speed = 0.;
    sim.controller.velocity = Vec2::ZERO;
    sim.x = p.x; sim.z = p.z; sim.y = fn_game::EYE + p.y;
    with(|s| say(s, format!("ride aid: pawn placed on the rocket top at ({:.2}, {:.2}, {:.2}) m", p.x, p.y, p.z)));
}

fn ammo_aid(frame: u32, arms: Option<&mut crate::arms::Arms>) {
    static AT: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Some((n, at)) = *AT.get_or_init(|| std::env::var("FNRT_ROCKET_AMMO").ok().and_then(|v| { let (n, f) = v.split_once('@')?; Some((n.trim().parse().ok()?, f.trim().parse().ok()?)) })) else { return };
    if frame < at || DONE.load(std::sync::atomic::Ordering::Relaxed) { return; }
    let Some(a) = arms else { return };
    let Some(g) = a.gun_index() else { return };
    if let Some(r) = a.reserve.get_mut(g) { *r = n; DONE.store(true, std::sync::atomic::Ordering::Relaxed); with(|s| say(s, format!("ammo aid: reserve of firearm {g} set to {n}"))); }
}

pub fn frame(sim: &mut GameSim, arms: Option<&mut crate::arms::Arms>, dt: f32) -> Vec<SoundEvent> {
    ride_aid(sim);
    let mut arms = arms;
    ammo_aid(sim.frames, arms.as_deref_mut());
    let feet = sim.feet();
    let grounded = sim.controller.grounded();
    let (health, shield) = arms.as_deref().map_or((100., 0.), |a| (a.health, a.shield));
    let mut pawn = PawnTarget::new(LOCAL_PAWN, LOCAL_TEAM, feet, Vitals { health, max_health: crate::arms::MAX_HEALTH, shield, max_shield: crate::arms::MAX_SHIELD, resistance: 0., vulnerability: 0. });
    pawn.crouched = sim.controller.crouched;
    let grav = fn_hitreg::shoot::projectile_gravity_m_s2();
    let mut live = with(|s| std::mem::take(&mut s.live));
    let pending = with(|s| std::mem::take(&mut s.pending_explode));
    let mut blasts: Vec<Blasted> = vec![];
    let mut bounces: Vec<(Arc<GunHit>, Vec3, Vec3)> = vec![];
    let mut carry = Vec3::ZERO;
    let mut riding_now: Option<(u64, Vec3)> = None;
    let pawns = [pawn.clone()];
    let no_pawns: &[PawnTarget] = &[];
    for r in live.iter_mut() {
        let Some(def) = ex::def_of(&r.gun) else { continue };
        let rad = def.capsule_radius_cm / 100.;
        let top = r.f.pos.y + rad;
        let horiz = Vec2::new(feet.x - r.f.pos.x, feet.z - r.f.pos.z).length();
        let based = grounded && !r.f.exploded && horiz <= 0.30 && feet.y >= top - 0.15 && feet.y <= top + 0.06;
        let before = r.f.pos;
        if pending.contains(&r.f.id) && !r.f.exploded {
            r.f.exploded = true;
            blasts.push(Blasted { id: r.f.id, gun: r.gun.clone(), pos: r.f.pos, normal: Vec3::Y, instigator: r.f.instigator, team: r.f.team, cause: Cause::Impact, remote: true });
            continue;
        }
        let mut left = dt;
        let mut evs = vec![];
        while left > 1e-6 && !r.f.exploded { let h = left.min(SUB_STEP); left -= h; evs.extend(r.f.step(&r.gun, &sim.world, if r.remote { &pawns[..] } else { no_pawns }, h, grav)); }
        if based && !r.f.exploded { carry = r.f.pos - before; riding_now = Some((r.f.id, r.f.vel)); }
        for e in evs {
            match e {
                Event::Bounce { pos, normal, .. } => bounces.push((r.gun.clone(), pos, normal)),
                Event::Explode { pos, normal, cause, .. } => blasts.push(Blasted { id: r.f.id, gun: r.gun.clone(), pos, normal, instigator: r.f.instigator, team: r.f.team, cause, remote: r.remote }),
                Event::Stopped { .. } => {}
            }
        }
    }
    if carry != Vec3::ZERO {
        sim.controller.position += Vec2::new(carry.x, carry.z);
        sim.controller.height += carry.y;
        sim.x += carry.x; sim.z += carry.z; sim.y += carry.y;
    }
    let prev_riding = with(|s| s.riding);
    if let (Some((id, vel)), None) = (prev_riding, riding_now) {
        if !sim.controller.grounded() && live.iter().any(|r| r.f.id == id && !r.f.exploded) {
            sim.controller.velocity += Vec2::new(vel.x, vel.z);
            sim.controller.vertical_speed += vel.y;
            with(|s| say(s, format!("left rocket #{id}: base velocity ({:.1}, {:.1}, {:.1}) m/s imparted", vel.x, vel.y, vel.z)));
        }
    }
    if riding_now.is_some() && prev_riding.map(|p| p.0) != riding_now.map(|p| p.0) { with(|s| say(s, format!("riding rocket #{} (feet {:.2} m)", riding_now.unwrap().0, sim.feet().y))); }
    let mut hits_on_pawn: Vec<(f32, Vec3, bool)> = vec![];
    let mut reports: Vec<String> = vec![];
    for b in &blasts {
        let Some(def) = ex::def_of(&b.gun) else { continue };
        let radius = b.gun.ability.selection(fn_hitreg::data::APPLICATION_EXPLODE).map_or(5.12, |s| s.range / 100.);
        let cands = building_candidates(b.pos, radius);
        let mut ps = vec![pawn.clone()];
        let shield_before = ps[0].vitals.shield;
        let blast = ex::explode(&b.gun, &mut sim.world, &mut ps, b.instigator, b.team, &cands, b.pos, 1);
        if let Some(h) = blast.pawn_hits.iter().find(|h| h.id == LOCAL_PAWN) {
            hits_on_pawn.push((h.damage, b.pos, ps[0].vitals.shield < shield_before));
            pawn.vitals = ps[0].vitals;
        }
        let builds = blast.impacts.iter().filter(|i| i.object & crate::building_damage_hook::BUILDING_OBJECT != 0).count();
        let props = blast.impacts.iter().filter(|i| i.prop.is_some()).count();
        let destroyed = blast.impacts.iter().filter(|i| i.destroyed).count();
        reports.push(format!("#{} {} exploded ({:?}) at ({:.2}, {:.2}, {:.2}) m: radius {:.2} m, {} candidate(s), {} obstructed, {} pawn hit(s){}, {} building hit(s), {} prop hit(s), {} destroyed, env {} / pawn {}",
            b.id, def.template.rsplit('/').next().unwrap_or(""), b.cause, b.pos.x, b.pos.y, b.pos.z, blast.radius_m, blast.considered, blast.obstructed, blast.pawn_hits.len(),
            blast.pawn_hits.iter().map(|h| format!(" [id {} dmg {} shield {:.0} health {:.0}]", h.id, h.damage, h.applied.shield_damage, h.applied.health_damage)).collect::<String>(),
            builds, props, destroyed, b.gun.stats.env_dmg[0], b.gun.stats.dmg[0]));
    }
    if let Some(a) = arms {
        if !hits_on_pawn.is_empty() {
            a.health = pawn.vitals.health; a.shield = pawn.vitals.shield;
            for (_, from, shield_hit) in &hits_on_pawn { crate::hurt_hook::on_damage(*shield_hit, fx_pos(*from)); }
            if !crate::mp_hook::active() { crate::kill_feed_hook::local_damaged(fn_ui::screens::kill_feed::DeathCause::Heavy, fn_ui::screens::kill_feed::Killer::Themself, a.health); }
        }
    }
    let cam = sim.cam_view.eye;
    let sounds = with(|s| {
        load_catalog(s);
        for l in reports { say(s, l); }
        for (d, _, _) in &hits_on_pawn { s.stats.2 += 1; s.stats.3 += d; }
        for (gun, pos, normal) in bounces {
            let Some(def) = ex::def_of(&gun) else { continue };
            let n = sim_dir_to_ue(normal);
            if let Some(i) = spawn_fx(s, &def.bounce_fx.clone(), pos, rot_from_x(n)) { s.fx.push(OneShot { inst: i }); }
            sound(s, &def.bounce_sound.clone(), pos);
        }
        for b in &blasts {
            let Some(def) = ex::def_of(&b.gun) else { continue };
            s.stats.1 += 1;
            let rot = if def.kind == Kind::Rocket && b.cause == Cause::Impact { rot_from_z(sim_dir_to_ue(b.normal)) } else { Mat3::IDENTITY };
            if let Some(i) = spawn_fx(s, &def.explosion_fx.clone(), b.pos, rot) { s.fx.push(OneShot { inst: i }); }
            sound(s, &def.explosion_sound.clone(), b.pos);
            if def.in_air_sound.is_some() { s.sounds.push(SoundEvent::ContainerLoopStop { key: LOOP_KEY_BASE + (b.id as u32 & 0xffff), fade: def.in_air_fade_out }); }
            if let Some((_, inner, outer)) = &def.camera_shake {
                let d = (cam - b.pos).length() * 100.;
                let scale = if d <= *inner { 1. } else if d >= *outer { 0. } else { 1. - (d - inner) / (outer - inner) };
                if scale > 0. {
                    let mut r = || { s.rnd = s.rnd.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); ((s.rnd >> 40) as f32 / (1u64 << 24) as f32) * std::f32::consts::TAU };
                    let phase = [r(), r(), r()];
                    s.shake = Shake { remaining: SHAKE_DURATION, elapsed: 0., scale, phase, loc: [0.; 3] };
                }
            }
            if !b.remote { s.net.push(NetEvent::Exploded { id: b.id, pos: b.pos, normal: b.normal }); if s.net.len() > 256 { s.net.drain(..128); } }
        }
        for r in live.iter_mut() {
            if let Some(t) = r.trail.as_mut() {
                t.xf.pos = fx_pos(r.f.pos);
                if r.f.vel.length_squared() > 1e-6 { t.xf.rot = rot_from_x(sim_dir_to_ue(r.f.vel)); }
                if r.f.exploded { t.deactivate(); }
            }
            if r.loop_on && !r.f.exploded { s.sounds.push(SoundEvent::LoopMove { key: LOOP_KEY_BASE + (r.f.id as u32 & 0xffff), pos: game_pos(r.f.pos) }); }
        }
        for r in live.iter_mut().filter(|r| r.f.exploded) { if let Some(t) = r.trail.take() { s.fx.push(OneShot { inst: t }); } }
        live.retain(|r| !r.f.exploded);
        s.riding = riding_now;
        s.live = std::mem::take(&mut live);
        std::mem::take(&mut s.sounds)
    });
    let hulls: Vec<fn_combat::Hull> = with(|s| s.live.iter().filter_map(|r| ex::def_of(&r.gun).map(|d| (r.f.pos, d.capsule_radius_cm / 100.))).filter_map(|(p, r)| rocket_hull(p, r)).collect());
    let (base, count) = with(|s| s.hulls_added);
    if count > 0 && sim.door_hulls.len() == base + count { sim.door_hulls.truncate(base); }
    let base = sim.door_hulls.len();
    let count = hulls.len();
    sim.door_hulls.extend(hulls);
    with(|s| s.hulls_added = (base, count));
    sounds
}

const SHAKE_DURATION: f32 = 0.8;
const SHAKE_BLEND_IN: f32 = 0.05;
const SHAKE_BLEND_OUT: f32 = 0.23;
const SHAKE_LOC: [(f32, f32); 3] = [(20., 11.808002), (23.75, 18.432001), (27.5, 16.128)];

pub fn tick_shake(dt: f32) {
    with(|s| {
        let sh = &mut s.shake;
        sh.loc = [0.; 3];
        if sh.remaining <= 0. { return; }
        sh.remaining = (sh.remaining - dt).max(0.);
        sh.elapsed += dt;
        if sh.remaining <= 0. { return; }
        let w_in = if SHAKE_BLEND_IN > 0. { (sh.elapsed / SHAKE_BLEND_IN).min(1.) } else { 1. };
        let w_out = if sh.remaining < SHAKE_BLEND_OUT { sh.remaining / SHAKE_BLEND_OUT } else { 1. };
        let w = w_in.min(w_out) * sh.scale;
        for k in 0..3 { sh.phase[k] += dt * SHAKE_LOC[k].1; sh.loc[k] = SHAKE_LOC[k].0 * sh.phase[k].sin() * w; }
    });
}

pub fn apply_shake(c: &mut CameraParams) {
    let l = with(|s| s.shake.loc);
    if l == [0.; 3] { return; }
    let f = c.forward.normalize_or_zero(); let u = c.up.normalize_or_zero(); let right = f.cross(u).normalize_or_zero();
    c.eye += (f * l[0] + right * l[1] + u * l[2]) * 0.01;
}

fn mesh_rot(r: &Live) -> Option<(String, Mat3)> {
    let def = ex::def_of(&r.gun)?;
    let mesh = def.mesh.clone()?;
    let dir = if r.f.vel.length_squared() > 1e-6 { sim_dir_to_ue(r.f.vel) } else { Vec3::X };
    let base = rot_from_x(dir) * Mat3::from_rotation_x(90f32.to_radians());
    Some((mesh, base * Mat3::from_diagonal(def.mesh_scale)))
}

fn ensure_dxbc<'a>(dxbc: &'a mut Option<Result<Dxbc, String>>, install: &Option<PathBuf>, catalog: &Option<Arc<Catalog>>, fx: &mut FxRenderer) -> Option<&'a mut Dxbc> {
    if dxbc.is_none() {
        let mut r = match install { Some(d) => Dxbc::load(d, fx), None => Err("no install".into()) };
        if let (Ok(dx), Some(cat)) = (r.as_mut(), catalog.as_ref()) { for sys in cat.systems() { dx.skip_non_normal_emitters(&sys.package); } }
        *dxbc = Some(r);
    }
    dxbc.as_mut().and_then(|r| r.as_mut().ok())
}

const WARM_MESHES: [&str; 2] = ["/Game/Weapons/FORT_RocketLaunchers/Mesh/S_PumpkinLauncher_Missile.S_PumpkinLauncher_Missile", "/Game/Weapons/FORT_GrenadeLaunchers/Mesh/Sm_Frag_Temp.Sm_Frag_Temp"];
fn warm_system(package: &str) -> bool { package.starts_with("/Game/Weapons/FORT_RocketLaunchers/") || package.starts_with("/Game/Weapons/FORT_GrenadeLaunchers/") }

static PREFETCH: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static PREFETCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn start_prefetch() {
    if !cfg!(target_arch = "wasm32") || PREFETCHED.load(std::sync::atomic::Ordering::Relaxed) { return; }
    let Ok(mut g) = PREFETCH.lock() else { return };
    if g.is_some() { return; }
    let Some((install, roots)) = with(|s| { load_catalog(s); s.catalog.as_ref()?; Some((s.install.clone()?, s.catalog.clone()?)) }) else { if with(|s| s.catalog_tried) { PREFETCHED.store(true, std::sync::atomic::Ordering::Relaxed); } return };
    let h = std::thread::Builder::new().name("rocket prefetch".into()).spawn(move || {
        let t = std::time::Instant::now();
        let Ok(set) = fn_pak::PakSet::open_install(&install) else { PREFETCHED.store(true, std::sync::atomic::Ordering::Relaxed); return };
        let mut queue: std::collections::VecDeque<String> = roots.systems().filter(|s| warm_system(&s.package)).map(|s| fn_fx::data::long_package(&s.package).to_owned()).collect();
        queue.extend(WARM_MESHES.iter().map(|m| fn_fx::data::long_package(m).to_owned()));
        let mut seen: HashSet<String> = queue.iter().cloned().collect();
        let mut n = 0;
        fn_pak::retain_reads(|| {
            while let Some(long) = queue.pop_front() {
                if n >= 400 { break; }
                let Ok((a, x, b)) = fn_fx::data::package_files(&set, &long) else { continue };
                n += 1;
                let Ok(p) = fn_ueio::Package::parse(&a, &x, &b) else { continue };
                for i in &p.imports {
                    if i.class_name == "Package" && i.object_name.starts_with("/Game/") && seen.insert(i.object_name.clone()) { queue.push_back(i.object_name.clone()); }
                }
            }
        });
        eprintln!("[rockets] prefetch: {n} packages read on a worker in {:.0} ms", t.elapsed().as_secs_f32() * 1000.);
        PREFETCHED.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    if let Ok(h) = h { *g = Some(h); }
}

pub fn prebuild(fx: &mut FxRenderer) -> bool {
    if cfg!(target_arch = "wasm32") {
        start_prefetch();
        if !PREFETCHED.load(std::sync::atomic::Ordering::Relaxed) { return false; }
    }
    warm(fx);
    true
}

fn warm(fx: &mut FxRenderer) {
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.load(std::sync::atomic::Ordering::Relaxed) { return; }
    with(|s| {
        load_catalog(s);
        let Some(cat) = s.catalog.clone() else { if s.catalog_tried { DONE.store(true, std::sync::atomic::Ordering::Relaxed); } return };
        let t = std::time::Instant::now();
        let State { dxbc, install, errors, catalog, .. } = s;
        let Some(dx) = ensure_dxbc(dxbc, install, catalog, fx) else { return };
        let view = ViewParams { eye: Vec3::new(0., 2., 3.), forward: Vec3::new(0., -0.5, -0.85).normalize(), up: Vec3::Y, tan_half: (1., 0.6), near_m: 0.1, far_m: 1000., width: 64, height: 64, time: 0., real_time: 0., delta_world: 1. / 60. };
        dx.begin(&view);
        let mut n = 0;
        for sys in cat.systems().filter(|s| s.package.starts_with("/Game/Weapons/FORT_RocketLaunchers/") || s.package.starts_with("/Game/Weapons/FORT_GrenadeLaunchers/")).cloned().collect::<Vec<_>>() {
            let mut i = Instance::new(sys, Transform::identity(), 1, 100.);
            for _ in 0..24 {
                i.tick(1. / 16., &Env::none());
                let mut d = vec![]; i.draw([0., 2., 3.], &mut d);
                let bounds = crate::particles::instance_bounds(&i, &cat);
                for e in &d { if let Err(err) = dx.draw_emitter(fx, e, &i.xf, &bounds, 1. / 60.) { let k = format!("{}/{}", e.system, e.emitter); if errors.insert(k.clone()) { eprintln!("[rockets] {k}: NOT DRAWN: {err}"); } } }
                let _ = dx.take_frame();
                dx.begin(&view);
            }
            n += 1;
        }
        for m in ["/Game/Weapons/FORT_RocketLaunchers/Mesh/S_PumpkinLauncher_Missile.S_PumpkinLauncher_Missile", "/Game/Weapons/FORT_GrenadeLaunchers/Mesh/Sm_Frag_Temp.Sm_Frag_Temp"] {
            if let Err(e) = dx.static_ready(fx, m) { eprintln!("[rockets] {m}: {e}"); }
        }
        let _ = dx.take_frame();
        DONE.store(true, std::sync::atomic::Ordering::Relaxed);
        eprintln!("[rockets] warm-up: {n} particle systems + 2 projectile meshes built in {:.0} ms", t.elapsed().as_secs_f32() * 1000.);
    });
}

pub fn fx_frame(fx: &mut FxRenderer, view: &ViewParams, dt: f32, out: &mut FxFrame) {
    tick_shake(dt);
    if cfg!(target_arch = "wasm32") {
        static PLAYED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if PLAYED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 180 { warm(fx); }
    }
    let Ok(mut g) = STATE.lock() else { return };
    let Some(st) = g.as_mut() else { return };
    if st.live.is_empty() && st.fx.is_empty() { return; }
    let env = Env::none();
    for o in st.fx.iter_mut() { o.inst.tick(dt, &env); }
    st.fx.retain(|o| !o.inst.finished());
    for r in st.live.iter_mut() { if let Some(t) = r.trail.as_mut() { t.tick(dt, &env); } }
    let eye = [view.eye.x, view.eye.y, view.eye.z];
    let State { dxbc, install, live, fx: shots, errors, catalog, .. } = st;
    let Some(dx) = ensure_dxbc(dxbc, install, catalog, fx) else { return };
    dx.begin(view);
    for r in live.iter() {
        let Some((mesh, rot)) = mesh_rot(r) else { continue };
        if let Err(e) = dx.draw_static_item(fx, &mesh, fx_pos(r.f.pos), rot, [0.; 3]) { if errors.insert(format!("mesh {mesh}")) { eprintln!("[rockets] {mesh}: NOT DRAWN: {e}"); } }
    }
    if let Some(cat) = catalog.clone() {
        let insts: Vec<&Instance> = live.iter().filter_map(|r| r.trail.as_ref()).chain(shots.iter().map(|o| &o.inst)).collect();
        for i in insts {
            let mut d = vec![];
            i.draw(eye, &mut d);
            let bounds = crate::particles::instance_bounds(i, &cat);
            for e in &d { if let Err(err) = dx.draw_emitter(fx, e, &i.xf, &bounds, dt) { let k = format!("{}/{}", e.system, e.emitter); if errors.insert(k.clone()) { eprintln!("[rockets] {k}: NOT DRAWN: {err}"); } } }
        }
    }
    let fr = dx.take_frame();
    out.buffer_writes.extend(fr.buffer_writes);
    out.draws.extend(fr.draws);
    out.shadow_draws.extend(fr.shadow_draws);
    out.distance_field_objects.extend(fr.distance_field_objects);
}

pub fn report() -> String {
    with(|s| {
        if s.log.is_empty() { return String::new(); }
        let mut out = format!("rockets: {} spawned, {} exploded, {} hits on the local pawn ({} damage), {} live\n", s.stats.0, s.stats.1, s.stats.2, s.stats.3, s.live.len());
        for l in &s.log { out.push_str("  "); out.push_str(l); out.push('\n'); }
        out
    })
}
