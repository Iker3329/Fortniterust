use crate::arms::{Arms, MontageData};
use crate::scene::Scene;
use fn_animgraph::{AnimState, Montage, PawnSample, CORE_ANIMS_MALE};
use fn_building::world::{GridKey, Material, PieceId, PieceKind, PieceSpawn, WorldEvent};
use fn_game::{GameSim, EYE};
use fn_hitreg::shoot::{BodySetup, HitBody, PawnTarget, Vitals};
use fn_net::proto::{self, build_op, flag, Build, Hit, State, Storm};
use fn_net::{parse_control, Control, Event, Session};
use fn_pose::CharacterPose;
use fn_render::ModelId;
use glam::{IVec3, Mat4, Quat, Vec2, Vec3, Vec4};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[cfg(target_arch = "wasm32")]
#[path = "../../../tools/web/fn-web/src/mp_web.rs"]
mod mp_web;

const PICKAXE_DAMAGE: f32 = 15.;
const PICKAXE_RANGE_M: f32 = 2.96;
const PICKAXE_NOTIFY_S: [f32; 2] = [0.1603, 0.2015];
const SPAWN_RING_M: f32 = 1.5;
const PHYSICS_ASSET: &str = "/Game/Characters/Player/Male/Male_Avg_Base/MPR/Player_Medium_Apose_PhysicsAsset";
const HIT_POS_TTL_MS: f64 = 1500.;
const NUMBER_PAWN: u8 = 1;
const NUMBER_SHIELD: u8 = 4;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static AUTOPLAY: AtomicBool = AtomicBool::new(false);
enum Inbound { Text(String), Bin(Vec<u8>) }
static INBOX: Mutex<VecDeque<Inbound>> = Mutex::new(VecDeque::new());
static NUMBERS: Mutex<Vec<(Vec3, u32, bool, u8)>> = Mutex::new(Vec::new());
static BODIES: OnceLock<Vec<BodySetup>> = OnceLock::new();
static BODIES_STARTED: AtomicBool = AtomicBool::new(false);

pub fn active() -> bool { ACTIVE.load(Ordering::Relaxed) }
pub fn players_alive() -> Option<u32> { MP.with(|m| m.borrow().as_ref().filter(|mp| mp.s.started).map(|mp| mp.s.names.keys().filter(|s| if **s == mp.s.slot { !mp.s.dead } else { !mp.s.remotes.get(s).map_or(false, |r| r.dead) }).count() as u32)) }
pub fn take_autoplay() -> bool { AUTOPLAY.load(Ordering::Relaxed) && AUTOPLAY.swap(false, Ordering::Relaxed) }
pub fn request_autoplay() { AUTOPLAY.store(true, Ordering::Relaxed); }
pub fn local_dead() -> bool { active() && MP.with(|m| m.borrow().as_ref().map_or(false, |mp| mp.s.started && mp.s.dead)) }
pub fn take_numbers() -> Vec<(Vec3, u32, bool, u8)> { NUMBERS.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default() }
fn push(m: Inbound) {
    ACTIVE.store(true, Ordering::Relaxed);
    if let Ok(mut q) = INBOX.lock() { if q.len() >= 4096 { q.pop_front(); } q.push_back(m); }
}
pub fn push_text(s: String) { push(Inbound::Text(s)); }
pub fn push_bin(b: Vec<u8>) { push(Inbound::Bin(b)); }

fn send(_b: &[u8]) {
    #[cfg(target_arch = "wasm32")]
    mp_web::send(_b);
}
fn feed(text: &str) {
    eprintln!("[mp] {text}");
    #[cfg(target_arch = "wasm32")]
    mp_web::feed(text);
}

fn ensure_bodies() {
    if BODIES.get().is_some() || BODIES_STARTED.load(Ordering::Relaxed) { return; }
    let Some(dir) = crate::hitreg_hook::install_dir() else { return };
    BODIES_STARTED.store(true, Ordering::Relaxed);
    std::thread::spawn(move || {
        let r = fn_pak::PakSet::open_install(&dir).map_err(|e| e.to_string()).and_then(|paks| fn_hitreg::shoot::load_body_setups(&fn_pose::ClipSourceDir::Pak(&paks), PHYSICS_ASSET));
        match r {
            Ok(b) => { eprintln!("[mp] hit bodies: {} physics-asset bodies from {PHYSICS_ASSET}", b.len()); let _ = BODIES.set(b); }
            Err(e) => { eprintln!("[mp] hit bodies unavailable ({e}): capsule stand-in bodies"); let _ = BODIES.set(vec![]); }
        }
    });
}

fn ue_bone_to_mesh() -> Mat4 { Mat4::from_cols(Vec4::new(0.01, 0., 0., 0.), Vec4::new(0., 0., 0.01, 0.), Vec4::new(0., 0.01, 0., 0.), Vec4::W) }

struct RemoteGfx { anim: AnimState, pose: CharacterPose, body: ModelId, head: ModelId, pick: ModelId, fire: Option<(usize, f32, bool)>, logged: bool }

#[derive(Clone)]
struct Target { slot: u8, feet: Vec3, dead: bool, crouched: bool, health: f32, shield: f32, bodies: Vec<HitBody> }

struct Mp {
    s: Session,
    t0: Instant,
    gfx: BTreeMap<u8, RemoteGfx>,
    pool: Vec<(ModelId, ModelId, ModelId)>,
    build_cursor: Option<u64>,
    keys: HashMap<PieceId, GridKey>,
    applying: HashSet<PieceId>,
    swing: (u8, f32, bool),
    spawned: bool,
    targets: Vec<Target>,
    last_fire: HashMap<u8, u32>,
    last_hit: HashMap<u8, (Vec3, f64)>,
    pieces_seen: usize,
}

thread_local! { static MP: RefCell<Option<Mp>> = const { RefCell::new(None) }; }

impl Mp {
    fn now_ms(&self) -> f64 { self.t0.elapsed().as_secs_f64() * 1000. }
    fn new(s: Session) -> Mp {
        Mp { s, t0: Instant::now(), gfx: BTreeMap::new(), pool: vec![], build_cursor: None, keys: HashMap::new(), applying: HashSet::new(), swing: (0, 0., false), spawned: false, targets: vec![], last_fire: HashMap::new(), last_hit: HashMap::new(), pieces_seen: 0 }
    }
}

fn held_index(core: Option<&str>) -> u8 { core.and_then(|c| CORE_ANIMS_MALE.iter().position(|(k, _)| *k == c)).map_or(0, |i| i as u8 + 1) }
fn held_core(i: u8) -> Option<&'static str> { if i == 0 { None } else { CORE_ANIMS_MALE.get(i as usize - 1).map(|(k, _)| *k) } }

fn teleport(sim: &mut GameSim, eye: Vec3) {
    let floor = sim.floor_below(Vec3::new(eye.x, eye.y + 1., eye.z));
    let h = (eye.y - EYE).max(floor);
    sim.controller.position = Vec2::new(eye.x, eye.z - fn_game::Z_OFFSET);
    sim.controller.velocity = Vec2::ZERO;
    sim.controller.vertical_speed = 0.;
    sim.controller.height = h;
    sim.x = eye.x; sim.z = eye.z; sim.y = EYE + h;
}

fn spawn_point(sim: &GameSim, slot: u8, players: usize) -> (Vec3, f32) {
    let (x, y, z, _, _) = fn_game::spawn();
    let n = players.max(2) as f32;
    let a = slot as f32 * std::f32::consts::TAU / n;
    let at = Vec3::new(x + a.cos() * SPAWN_RING_M, y, z + a.sin() * SPAWN_RING_M);
    let d = Vec3::new(x, y, z) - at;
    (Vec3::new(at.x, EYE + sim.ground_y(at.x, at.z), at.z), d.x.atan2(-d.z).to_degrees().rem_euclid(360.))
}

fn kind_u8(k: PieceKind) -> u8 { PieceKind::ALL.iter().position(|x| *x == k).unwrap_or(0) as u8 }
fn mat_u8(m: Material) -> u8 { match m { Material::Wood => 0, Material::Stone => 1, Material::Metal => 2 } }
fn kind_of(v: u8) -> Option<PieceKind> { PieceKind::ALL.get(v as usize).copied() }
fn mat_of(v: u8) -> Option<Material> { [Material::Wood, Material::Stone, Material::Metal].get(v as usize).copied() }

fn pawn_target(t: &Target) -> PawnTarget {
    let mut p = PawnTarget::new(t.slot as u64, t.slot as u32 + 1, t.feet, Vitals::new(t.health.max(0.01), t.shield));
    p.crouched = t.crouched;
    p.bodies = if t.bodies.is_empty() { p.coarse_bodies() } else { t.bodies.clone() };
    p
}

pub fn pawn_targets() -> Vec<PawnTarget> {
    if !active() { return vec![]; }
    MP.with(|m| m.borrow().as_ref().map(|mp| mp.targets.iter().filter(|t| !t.dead).map(pawn_target).collect()).unwrap_or_default())
}

fn remember_hits(mp: &mut Mp, hits: &[fn_hitreg::shoot::PawnHit]) {
    let now = mp.now_ms();
    for h in hits { mp.last_hit.insert(h.id as u8, (h.pos, now)); }
}

pub fn on_projectile_hits(row: &str, hits: &[fn_hitreg::shoot::PawnHit]) {
    if !active() { return; }
    MP.with(|m| {
        let mut b = m.borrow_mut();
        let Some(mp) = b.as_mut() else { return };
        if !mp.s.started || mp.s.dead { return; }
        let weapon = fn_net::weapon_id(row);
        let Some(seq) = mp.last_fire.get(&weapon).copied() else { return };
        remember_hits(mp, hits);
        for h in hits { if h.damage > 0. { mp.s.send_hit(Hit { fire_seq: seq, target: h.id as u8, weapon, flags: h.feedback.critical as u8, pellets: 1, damage: h.damage }); } }
    });
}

pub fn on_cartridge(row: &str, eye: Vec3, dir: Vec3, hits: &[fn_hitreg::shoot::PawnHit]) {
    if !active() { return; }
    MP.with(|m| {
        let mut b = m.borrow_mut();
        let Some(mp) = b.as_mut() else { return };
        if !mp.s.started || mp.s.dead { return; }
        let weapon = fn_net::weapon_id(row);
        let seq = mp.s.send_fire(weapon, eye, dir, 0);
        mp.last_fire.insert(weapon, seq);
        remember_hits(mp, hits);
        let mut per: BTreeMap<u8, (f32, u8, bool)> = BTreeMap::new();
        for h in hits {
            let e = per.entry(h.id as u8).or_insert((0., 0, false));
            e.0 += h.damage; e.1 += 1; e.2 |= h.feedback.critical;
        }
        for (target, (damage, pellets, head)) in per {
            if damage > 0. {
                eprintln!("[mp] hit claim fire {seq} -> {} for {damage:.1} ({pellets} pellet(s){})", mp.s.name_of(target), if head { ", head" } else { "" });
                mp.s.send_hit(Hit { fire_seq: seq, target, weapon, flags: head as u8, pellets, damage });
            }
        }
    });
}

fn melee(mp: &mut Mp, sim: &GameSim) {
    let (stage, time) = (sim.swing.stage, sim.swing.time);
    let (ps, pt, _) = mp.swing;
    if (stage == 1 || stage == 2) && (stage != ps || time < pt) { mp.swing.2 = true; }
    mp.swing.0 = stage; mp.swing.1 = time;
    let notify = PICKAXE_NOTIFY_S[if stage == 2 { 1 } else { 0 }];
    if !(mp.swing.2 && (stage == 1 || stage == 2) && time >= notify) { return; }
    mp.swing.2 = false;
    if mp.s.dead { return; }
    let eye = sim.cam_view.eye;
    let fwd = sim.cam_view.forward;
    let target = mp.targets.iter().filter(|t| !t.dead).filter_map(|t| {
        let (a, b, r) = pawn_target(t).capsule_segment();
        fn_hitreg::shoot::ray_capsule(eye, fwd, PICKAXE_RANGE_M, a, b, r + fn_combat::MELEE_RADIUS).map(|(d, _)| (t.slot, d, eye + fwd * d))
    }).min_by(|a, b| a.1.total_cmp(&b.1));
    eprintln!("[mp] pickaxe swing {stage} notify at {time:.3} s: {}", target.map_or("no pawn in reach".into(), |t| format!("hit {} at {:.2} m", mp.s.name_of(t.0), t.1)));
    if let Some((slot, _, pos)) = target {
        let seq = mp.s.send_fire(fn_net::PICKAXE, eye, fwd, 0);
        let now = mp.now_ms();
        mp.last_hit.insert(slot, (pos, now));
        mp.s.send_hit(Hit { fire_seq: seq, target: slot, weapon: fn_net::PICKAXE, flags: 0, pellets: 1, damage: PICKAXE_DAMAGE });
    }
}

fn local_state(sim: &GameSim, arms: Option<&Arms>, show_pick: bool) -> State {
    let c = &sim.controller;
    let mut f = 0u16;
    if c.crouched { f |= flag::CROUCHED; }
    if c.running { f |= flag::RUNNING; }
    if c.airborne() { f |= flag::AIRBORNE; }
    if sim.ads.active && sim.ads.weight > 0.5 { f |= flag::TARGETING; }
    if crate::building_hook::is_building() { f |= flag::BUILDING; }
    if sim.swing.stage > 0 { f |= flag::SWINGING; }
    if show_pick { f |= flag::PICKAXE; }
    if sim.cam_view.hide_pawn { f |= flag::HIDDEN; }
    let f = flag::with_gun(f, arms.and_then(|a| a.gun_index()).filter(|_| !show_pick));
    let held = held_index(arms.and_then(|a| a.held_core()).map(|h| h.0));
    State { seq: 0, t_ms: 0, pos: Vec3::new(sim.x, sim.y, sim.z), vel: Vec3::new(c.velocity.x, c.vertical_speed, c.velocity.y), yaw: sim.yaw, pitch: sim.pitch, flags: f, held, air_dm: (c.air_height() * 10.).clamp(0., 255.) as u8 }
}

fn sync_builds(mp: &mut Mp) {
    let n = crate::building_hook::with_world(|w| w.len());
    if n != mp.pieces_seen { mp.pieces_seen = n; }
    let (events, next) = crate::building_hook::with_world(|w| {
        let start = mp.build_cursor.unwrap_or(u64::MAX);
        let (ev, next) = w.events_since(if start == u64::MAX { u64::MAX } else { start });
        let out: Vec<(WorldEvent, Option<fn_building::world::PlacedPiece>)> = if start == u64::MAX { vec![] } else { ev.iter().map(|e| (e.clone(), match e { WorldEvent::Placed(id) | WorldEvent::Edited(id) => w.get(*id).cloned(), _ => None })).collect() };
        (out, next)
    });
    mp.build_cursor = Some(next);
    for (e, piece) in events {
        match (e, piece) {
            (WorldEvent::Placed(id), Some(p)) => {
                mp.keys.insert(id, p.key);
                if mp.applying.remove(&id) { continue; }
                mp.s.send_build(&Build { op: build_op::PLACE, kind: kind_u8(p.kind), material: mat_u8(p.material), rot: p.key.rot, cell: p.key.cell.to_array(), loc: p.location, yaw: p.yaw_deg, rot_iter: 0, mirrored: false, class_path: String::new() });
            }
            (WorldEvent::Edited(id), Some(p)) => {
                if mp.applying.remove(&id) { continue; }
                let cp = p.edit.class_path.clone().unwrap_or_default();
                mp.s.send_build(&Build { op: build_op::EDIT, kind: kind_u8(p.kind), material: mat_u8(p.material), rot: p.key.rot, cell: p.key.cell.to_array(), loc: p.location, yaw: p.yaw_deg, rot_iter: p.edit.rotation_iterations.clamp(-128, 127) as i8, mirrored: p.edit.mirrored, class_path: cp });
            }
            (WorldEvent::Removed(id), _) => {
                let key = mp.keys.remove(&id);
                if mp.applying.remove(&id) { continue; }
                if let Some(k) = key { mp.s.send_build(&Build { op: build_op::REMOVE, kind: kind_u8(k.kind), material: 0, rot: k.rot, cell: k.cell.to_array(), loc: Vec3::ZERO, yaw: 0., rot_iter: 0, mirrored: false, class_path: String::new() }); }
            }
            _ => {}
        }
    }
}

fn apply_build(mp: &mut Mp, b: &Build) {
    let (Some(kind), Some(material)) = (kind_of(b.kind), mat_of(b.material)) else { return };
    let key = GridKey { kind, cell: IVec3::from_array(b.cell), rot: b.rot & 3 };
    match b.op {
        build_op::PLACE => {
            let Some(def) = crate::building_hook::with_catalog(|c| c.and_then(|c| c.def(kind, material).cloned())) else { return };
            let id = crate::building_hook::with_world(|w| {
                if let Some(old) = w.at_key(&key) { mp.applying.insert(old); }
                w.spawn(PieceSpawn { kind, material, class_path: def.class.class_path.clone(), key, location: b.loc, yaw_deg: b.yaw, max_health: def.max_health, start_health: def.start_health(), build_time: def.build_time })
            });
            mp.applying.insert(id);
            mp.keys.insert(id, key);
        }
        build_op::EDIT => {
            crate::building_hook::with_world(|w| {
                let Some(id) = w.at_key(&key) else { return };
                mp.applying.insert(id);
                w.set_edit(id, fn_building::world::EditState { class_path: (!b.class_path.is_empty()).then(|| b.class_path.clone()), rotation_iterations: b.rot_iter as i32, mirrored: b.mirrored });
            });
        }
        build_op::REMOVE => {
            crate::building_hook::with_world(|w| { if let Some(id) = w.at_key(&key) { mp.applying.insert(id); w.remove(id); } });
        }
        _ => {}
    }
}

fn storm_of(sim: &GameSim) -> Storm { let s = &sim.storm; Storm { phase: 0, state: s.enabled as u8, center: s.center_cm, next: [s.radius_cm, 0., 0.], time_left: 0. } }

fn montage_at(m: &MontageData, t: f32) -> Option<Montage> { let (clip, time) = m.sample(t)?; let w = m.weight(t); (w > 0.).then_some(Montage { slot: m.slot, clip, time, weight: w, blend: m.blend }) }

fn draw_remotes(mp: &mut Mp, scene: &mut Scene, arms: Option<&Arms>, dt: f32) {
    let now = mp.now_ms();
    let live: Vec<u8> = mp.s.remotes.keys().copied().collect();
    let gone: Vec<u8> = mp.gfx.keys().copied().filter(|s| !live.contains(s)).collect();
    for s in gone { if let Some(g) = mp.gfx.remove(&s) { mp.pool.push((g.body, g.head, g.pick)); } }
    mp.targets.clear();
    let (b0, h0, p0) = scene.pawn_models();
    let setups = BODIES.get();
    let to_mesh = ue_bone_to_mesh();
    for (slot, r) in mp.s.remotes.iter() {
        let Some(smp) = r.sample(now) else { continue };
        let feet = smp.pos - Vec3::Y * EYE;
        let crouched = smp.flags & flag::CROUCHED != 0;
        let mut target = Target { slot: *slot, feet, dead: r.dead, crouched, health: r.health, shield: r.shield, bodies: vec![] };
        if r.dead || smp.flags & flag::HIDDEN != 0 { mp.targets.push(target); continue; }
        if !mp.gfx.contains_key(slot) {
            let (body, head, pick) = mp.pool.pop().unwrap_or_else(|| (scene.renderer.instance_model(b0), scene.renderer.instance_model(h0), scene.renderer.instance_model(p0)));
            let mut anim = AnimState::new();
            let still = PawnSample { velocity: (0., 0.), input: (0., 0.), facing_deg: smp.yaw, camera_yaw_deg: smp.yaw, camera_pitch_deg: smp.pitch, height: 0., vertical_speed: 0., fall_time_left: 0., crouched: false, running: false, melee_swinging: false };
            for _ in 0..60 { anim.step_sample(&still, 1. / 60.); }
            mp.gfx.insert(*slot, RemoteGfx { anim, pose: CharacterPose::default(), body, head, pick, fire: None, logged: false });
            eprintln!("[mp] remote pawn {} ({}) drawn at {:?}", slot, r.name, smp.pos);
        }
        let Some(g) = mp.gfx.get_mut(slot) else { continue };
        g.anim.set_held(held_core(smp.held).map(|k| (k, smp.flags & flag::TARGETING != 0)));
        let v2 = Vec2::new(smp.vel.x, smp.vel.z);
        let input = if v2.length() > 0.3 { v2.normalize() } else { Vec2::ZERO };
        let airborne = smp.flags & flag::AIRBORNE != 0;
        let height = if airborne { smp.air_m.max(1e-4) } else { 0. };
        let sample = PawnSample { velocity: (v2.x, v2.y), input: (input.x, input.y), facing_deg: smp.yaw, camera_yaw_deg: smp.yaw, camera_pitch_deg: smp.pitch,
            height, vertical_speed: smp.vel.y, fall_time_left: if airborne { fn_movement::controller::fall_time_left(height, smp.vel.y) } else { 0. },
            crouched, running: smp.flags & flag::RUNNING != 0, melee_swinging: smp.flags & flag::SWINGING != 0 };
        g.anim.step_sample(&sample, dt.max(1e-4));
        let mut montages = vec![];
        if let Some((gun, t, ads)) = g.fire.as_mut() {
            *t += dt;
            let data = arms.and_then(|a| a.data.guns.get(*gun));
            match data.and_then(|d| montage_at(if *ads { d.fire_ads.as_ref().unwrap_or(&d.fire) } else { &d.fire }, *t)) { Some(m) => montages.push(m), None => g.fire = None }
        }
        let tree = g.anim.plan(&montages);
        scene.character.rig.evaluate_into(&tree, &*scene.character.clips, &mut g.pose);
        let facing = g.anim.visual_yaw(smp.yaw);
        let world = Mat4::from_rotation_translation(Quat::from_rotation_y((180. - facing).to_radians()), feet);
        if let Some(p) = g.pose.parts.first() { scene.renderer.update_model(g.body, world, &p.skin_matrices); }
        if let Some(p) = g.pose.parts.get(1) { scene.renderer.update_model(g.head, world, &p.skin_matrices); }
        scene.models.push(g.body);
        scene.models.push(g.head);
        let socket = g.pose.socket("RightHand").unwrap_or(Mat4::IDENTITY);
        if smp.flags & flag::PICKAXE != 0 {
            scene.renderer.update_model(g.pick, world, &[socket]);
            scene.models.push(g.pick);
        }
        if let Some(gun) = flag::gun(smp.flags) { crate::gun_orig::remote(scene, *slot as usize, gun, world * socket); }
        if let (Some(setups), Some(part), Some(pose)) = (setups, scene.character.rig.parts.first(), g.pose.parts.first()) {
            if !setups.is_empty() {
                target.bodies = fn_hitreg::shoot::place_bodies(setups, |bone| part.tree.bone_index(bone).and_then(|i| pose.bone_mesh.get(i)).map(|m| world * *m * to_mesh));
                if !g.logged {
                    g.logged = true;
                    let head = target.bodies.iter().find(|b| b.bone.eq_ignore_ascii_case("head")).map(|b| (b.a + b.b) * 0.5);
                    let bone = part.tree.bone_index("head").and_then(|i| pose.bone_mesh.get(i)).map(|m| (world * *m).w_axis.truncate());
                    eprintln!("[mp] remote {} hit bodies: {} placed, head body at {:?}, head bone at {:?}, feet {:?}", slot, target.bodies.len(), head, bone, feet);
                }
            }
        }
        mp.targets.push(target);
    }
}

fn number_pos(mp: &Mp, victim: u8) -> Vec3 {
    let now = mp.now_ms();
    if let Some((p, t)) = mp.last_hit.get(&victim) { if now - t < HIT_POS_TTL_MS { return *p; } }
    mp.targets.iter().find(|t| t.slot == victim).map_or(Vec3::ZERO, |t| t.feet + Vec3::Y * 1.2)
}

fn on_events(mp: &mut Mp, sim: &mut GameSim, mut arms: Option<&mut Arms>) {
    for e in mp.s.take_events() {
        match e {
            Event::Started { .. } => { eprintln!("[mp] match started"); crate::kill_feed_hook::reset_eliminations(); }
            Event::Ended(reason) => feed(&format!("Match ended ({reason})")),
            Event::Damaged { attacker, amount, shield_hit, head } => {
                let from = mp.s.remotes.get(&attacker).and_then(|r| r.interp.latest().map(|s| s.pos)).unwrap_or(Vec3::new(sim.x, sim.y, sim.z));
                crate::hurt_hook::on_damage(shield_hit, fn_particles::sim::game_to_ue([from.x, from.y, from.z]));
                eprintln!("[mp] took {amount:.1} {} damage from {}{}", if shield_hit { "shield" } else { "health" }, mp.s.name_of(attacker), if head { " (head)" } else { "" });
            }
            Event::DamageShown { victim, amount, shield_hit, head } => {
                let pos = number_pos(mp, victim);
                let n = amount.round().max(0.) as u32;
                if n > 0 { if let Ok(mut q) = NUMBERS.lock() { q.push((pos, n, head, if shield_hit { NUMBER_SHIELD } else { NUMBER_PAWN })); } }
                eprintln!("[mp] hit {} for {amount:.1} {}{}", mp.s.name_of(victim), if shield_hit { "shield" } else { "health" }, if head { " (head)" } else { "" });
            }
            Event::OwnHealth { health, shield, dead } => {
                if let Some(a) = arms.as_deref_mut() { a.health = health; a.shield = shield; }
                eprintln!("[mp] own vitals {health:.0} / {shield:.0}{}", if dead { " (dead)" } else { "" });
            }
            Event::Elim { victim, killer, weapon, head } => {
                use fn_ui::screens::kill_feed::{Combatant, DeathCause, Elimination, Killer};
                if victim == mp.s.slot { if let Some(a) = arms.as_deref_mut() { a.health = 0.; a.shield = 0.; } }
                let who = |slot: u8| if slot == mp.s.slot { crate::kill_feed_hook::local() } else { Combatant::other(&mp.s.name_of(slot)) };
                let row = fn_net::caps::WEAPONS.get(weapon as usize).map_or("", |w| w.row);
                let (k, cause) = if killer == proto::NO_KILLER { (Killer::Themself, DeathCause::FallDamage) } else if killer == victim { (Killer::Themself, DeathCause::from_weapon_row(row)) } else { (Killer::Player(who(killer)), DeathCause::from_weapon_row(row)) };
                eprintln!("[mp] elimination: {} by {} with {row}{}", mp.s.name_of(victim), mp.s.name_of(killer), if head { " (head)" } else { "" });
                crate::kill_feed_hook::push(Elimination { killer: k, victim: who(victim), cause, dbno: false });
            }
            Event::Respawn(pos) => {
                let n = mp.s.names.len();
                let (at, yaw) = if pos == Vec3::ZERO { spawn_point(sim, mp.s.slot, n) } else { (pos, sim.yaw) };
                teleport(sim, at);
                sim.yaw = yaw;
                if let Some(a) = arms.as_deref_mut() { a.health = 100.; a.shield = 0.; }
                eprintln!("[mp] respawned at {at:?}");
            }
            Event::Correct(pos) => teleport(sim, pos),
            Event::Build { build, .. } => apply_build(mp, &build),
            Event::Storm(s) => { sim.storm.center_cm = s.center; sim.storm.radius_cm = s.next[0].max(1.); sim.storm.enabled = s.state != 0; }
            Event::Joined { name, .. } => eprintln!("[mp] {name} joined"),
            Event::Left { name, .. } => eprintln!("[mp] {name} left"),
            Event::RemoteFire { slot, fire } => {
                let gun = mp.s.remotes.get(&slot).and_then(|r| r.interp.latest()).and_then(|s| flag::gun(s.flags).map(|g| (g, s.flags & flag::TARGETING != 0)));
                if let (Some((g, ads)), Some(gfx)) = (gun, mp.gfx.get_mut(&slot)) { if fire.weapon != fn_net::PICKAXE { gfx.fire = Some((g, 0., ads)); } }
            }
            Event::Interact { .. } => {}
        }
    }
}

pub fn frame(sim: &mut GameSim, scene: &mut Scene, mut arms: Option<&mut Arms>, dt: f32) {
    if !active() { return; }
    MP.with(|cell| {
        let mut slot = cell.borrow_mut();
        let inbound: Vec<Inbound> = INBOX.lock().map(|mut q| q.drain(..).collect()).unwrap_or_default();
        for m in inbound {
            match m {
                Inbound::Text(t) => {
                    let Some(c) = parse_control(&t) else { continue };
                    if slot.as_ref().map_or(true, |m| m.s.ended.is_some()) {
                        if let Control::Welcome { slot: s, host, code } = &c { let pool = slot.take().map(|m| m.pool).unwrap_or_default(); let mut m = Mp::new(Session::new(*s, *host, code.clone())); m.pool = pool; *slot = Some(m); }
                    }
                    if let Some(mp) = slot.as_mut() { mp.s.control(c); }
                }
                Inbound::Bin(b) => if let Some(mp) = slot.as_mut() { let now = mp.now_ms(); mp.s.recv(&b, now); },
            }
        }
        let Some(mp) = slot.as_mut() else { return };
        ensure_bodies();
        on_events(mp, sim, arms.as_deref_mut());
        if mp.s.ended.is_some() {
            for (_, g) in std::mem::take(&mut mp.gfx) { mp.pool.push((g.body, g.head, g.pick)); }
            mp.s.remotes.clear();
            mp.targets.clear();
            ACTIVE.store(false, Ordering::Relaxed);
            for b in mp.s.take_outbox() { send(&b); }
            return;
        }
        if !mp.s.started { for b in mp.s.take_outbox() { send(&b); } return; }
        if !mp.spawned {
            mp.spawned = true;
            let (at, yaw) = spawn_point(sim, mp.s.slot, mp.s.names.keys().copied().max().map_or(2, |m| m as usize + 1));
            teleport(sim, at);
            sim.yaw = yaw;
            sim.pitch = 0.;
        }
        let now = mp.now_ms();
        if let Some(a) = arms.as_deref() { if !mp.s.dead { mp.s.report_local_vitals(a.health, a.shield); } }
        if let (true, Some(a)) = (mp.s.dead, arms.as_deref_mut()) { a.health = 0.; a.shield = 0.; }
        if mp.s.tick_send(dt) { let st = local_state(sim, arms.as_deref(), scene.show_pickaxe); mp.s.send_state(st, now); }
        if mp.s.host { let storm = storm_of(sim); mp.s.host_tick(dt, Some(storm)); }
        if crate::building_hook::with_catalog(|c| c.is_some()) { sync_builds(mp); }
        melee(mp, sim);
        draw_remotes(mp, scene, arms.as_deref(), dt);
        for b in mp.s.take_outbox() { send(&b); }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn held_index_roundtrips() {
        let k = CORE_ANIMS_MALE[0].0;
        assert_eq!(held_core(held_index(Some(k))), Some(k));
        assert_eq!(held_index(None), 0);
        assert_eq!(held_core(0), None);
    }
    #[test] fn inactive_costs_nothing() {
        assert!(!active());
        assert!(pawn_targets().is_empty());
        on_cartridge("x", Vec3::ZERO, Vec3::X, &[]);
    }
    #[test] fn gun_bits_roundtrip() {
        let f = flag::with_gun(flag::CROUCHED | flag::RUNNING, Some(3));
        assert_eq!(flag::gun(f), Some(3));
        assert_eq!(f & 3, 3);
        assert_eq!(flag::gun(flag::with_gun(f, None)), None);
    }
    #[test] fn ue_bone_space_maps_like_the_mesh_import() {
        let p = ue_bone_to_mesh().transform_point3(Vec3::new(100., 200., 300.));
        assert_eq!(p.to_array(), fn_mesh::gltf::ue_to_gltf_position([100., 200., 300.]));
    }
}
