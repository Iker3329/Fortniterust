#[path = "storm_volumes.rs"]
mod volumes;
pub use volumes::{SAFE_ZONE_VOLUME_0, SAFE_ZONE_VOLUME_1, SAFE_ZONE_VOLUME_2};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SafeZoneDef { pub radius: f32, pub reject_radius: f32, pub wait: f32, pub shrink: f32 }

pub const PHASE_COUNT: usize = 10;
pub const SAFE_ZONE_DEFS: [SafeZoneDef; PHASE_COUNT] = [
    SafeZoneDef { radius: 150_000., reject_radius: 0., wait: 0., shrink: 0. },
    SafeZoneDef { radius: 80_000., reject_radius: 0., wait: 200., shrink: 180. },
    SafeZoneDef { radius: 40_000., reject_radius: 0., wait: 150., shrink: 90. },
    SafeZoneDef { radius: 20_000., reject_radius: 0., wait: 120., shrink: 90. },
    SafeZoneDef { radius: 10_000., reject_radius: 0., wait: 120., shrink: 60. },
    SafeZoneDef { radius: 5_000., reject_radius: 0., wait: 90., shrink: 40. },
    SafeZoneDef { radius: 2_500., reject_radius: 0., wait: 90., shrink: 30. },
    SafeZoneDef { radius: 1_250., reject_radius: 0., wait: 60., shrink: 25. },
    SafeZoneDef { radius: 625., reject_radius: 0., wait: 60., shrink: 25. },
    SafeZoneDef { radius: 0., reject_radius: 0., wait: 45., shrink: 25. },
];
pub const STARTING_RADIUS_CM: f32 = 185_000.;
pub const START_DELAY_S: f32 = 60.;
pub const SAFE_ZONE_DAMAGE: [f32; PHASE_COUNT] = [0.006, 0.008, 0.01, 0.02, 0.05, 0.075, 0.1, 0.1, 0.1, 0.1];
pub const DAMAGE_PERIOD_S: f32 = 1.0;
pub const INSIDE_CHECK_PERIOD_S: f32 = 1.0;
pub const INITIAL_GE_LEVEL: usize = 1;
pub const VOLUME_REJECTION_CHANCE: [f32; 3] = [0.9, 0.7, 0.0];
pub const MAP_INFO_ROOT_CM: [f32; 3] = [32224., -25744., 1312.];
pub const PICK_TRIES: u32 = 10_000;
pub const RAND_MAX: i32 = 0x7fff;

pub fn volumes() -> [&'static [&'static [[f32; 2]]]; 3] { [&SAFE_ZONE_VOLUME_0, &SAFE_ZONE_VOLUME_1, &SAFE_ZONE_VOLUME_2] }

#[derive(Clone, Copy, Debug)]
pub struct CrtRand(pub u32);
impl CrtRand {
    pub fn rand(&mut self) -> i32 { self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011); ((self.0 >> 16) & 0x7fff) as i32 }
    pub fn frand(&mut self) -> f32 { self.rand() as f32 * (1.0 / RAND_MAX as f32) }
}

pub fn rand_point_in_circle(rng: &mut CrtRand, r: f32) -> [f32; 2] {
    loop {
        let x = rng.frand() * 2. - 1.;
        let y = rng.frand() * 2. - 1.;
        if x * x + y * y <= 1. { return [x * r, y * r]; }
    }
}

fn inside_hull(h: &[[f32; 2]], p: [f32; 2]) -> bool {
    if h.len() < 3 { return false; }
    let n = h.len();
    (0..n).all(|i| {
        let (a, b) = (h[i], h[(i + 1) % n]);
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-3 * ((b[0] - a[0]).abs() + (b[1] - a[1]).abs()).max(1.)
    })
}
pub fn volume_encompasses(vol: &[&[[f32; 2]]], p: [f32; 2]) -> bool { vol.iter().any(|h| inside_hull(h, p)) }

pub fn rejects(rng: &mut CrtRand, vols: &[&[&[[f32; 2]]]], chances: &[f32], p: [f32; 2], final_zone: bool) -> bool {
    if vols.is_empty() { return false; }
    if !final_zone { return !volume_encompasses(vols[0], p); }
    for i in (0..vols.len()).rev() {
        if volume_encompasses(vols[i], p) {
            let r = rng.frand();
            let c = chances.get(i).copied().unwrap_or(0.).clamp(0., 1.);
            return r < c;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
pub fn pick_safe_zone_location(rng: &mut CrtRand, vols: &[&[&[[f32; 2]]]], chances: &[f32], prev: [f32; 3], max_r: f32, min_r: f32, root: [f32; 3], enclose_r: f32, final_zone: bool) -> [f32; 3] {
    if max_r <= 0. { return prev; }
    for _ in 0..PICK_TRIES {
        let o = rand_point_in_circle(rng, max_r);
        if min_r * min_r <= o[0] * o[0] + o[1] * o[1] {
            let c = [o[0] + prev[0], o[1] + prev[1], prev[2]];
            let (dx, dy) = (c[0] - root[0], c[1] - root[1]);
            if dx * dx + dy * dy <= enclose_r * enclose_r && !rejects(rng, vols, chances, [c[0], c[1]], final_zone) { return c; }
        }
    }
    let (x, y) = (root[0] - prev[0], root[1] - prev[1]);
    let l2 = x * x + y * y;
    let d = if l2 == 1. { [x, y] } else if l2 >= 1e-8 { let k = 1. / l2.sqrt(); [x * k, y * k] } else { [0., 0.] };
    [prev[0] + d[0] * max_r, prev[1] + d[1] * max_r, prev[2]]
}

pub fn build_locations(rng: &mut CrtRand, defs: &[SafeZoneDef], root: [f32; 3], vols: &[&[&[[f32; 2]]]], chances: &[f32]) -> Vec<[f32; 3]> {
    let n = defs.len();
    let mut locs = vec![[0.; 3]; n];
    if n == 0 { return locs; }
    locs[0] = root;
    let r0 = defs[0].radius;
    for i in (1..n).rev() {
        let enclose = r0 - defs[i].radius;
        let (prev, max_r, min_r, last) = if i == n - 1 { (locs[0], enclose, defs[0].reject_radius, true) } else { (locs[i + 1], defs[i].radius - defs[i + 1].radius, defs[i].reject_radius, false) };
        locs[i] = pick_safe_zone_location(rng, vols, chances, prev, max_r, min_r, root, enclose, last);
    }
    locs
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneState { Holding, Shrinking }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Indicator {
    pub radius: f32, pub last_radius: f32, pub next_radius: f32,
    pub last_center: [f32; 3], pub next_center: [f32; 3], pub center: [f32; 3],
    pub start_shrink: f32, pub finish_shrink: f32, pub state: Option<ZoneState>,
}
impl Indicator {
    pub fn tick(&mut self, now: f32) -> Option<ZoneState> {
        let s = if now >= self.start_shrink {
            if now >= self.finish_shrink { self.center = self.next_center; self.radius = self.next_radius; ZoneState::Holding }
            else {
                let a = (now - self.start_shrink) / (self.finish_shrink - self.start_shrink);
                self.radius = (self.last_radius - self.next_radius) * (1. - a) + self.next_radius;
                self.center = std::array::from_fn(|k| (self.next_center[k] - self.last_center[k]) * a + self.last_center[k]);
                ZoneState::Shrinking
            }
        } else { self.center = self.last_center; self.radius = self.last_radius; ZoneState::Holding };
        if self.state != Some(s) { self.state = Some(s); Some(s) } else { None }
    }
    pub fn contains(&self, p: [f32; 2]) -> bool { let (dx, dy) = (self.center[0] - p[0], self.center[1] - p[1]); dx * dx + dy * dy <= self.radius * self.radius }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneEvent { Spawned, Advanced { phase: usize }, StateChanged(ZoneState), Damage { fraction: f32 } }

#[derive(Clone, Debug)]
pub struct SafeZones {
    pub start_time: f32,
    pub check_origin: f32,
    pub active: bool,
    pub phase: usize,
    pub locations: Vec<[f32; 3]>,
    pub ind: Option<Indicator>,
    pub ge_level: usize,
    pub inside_tag: bool,
    pub next_check: f32,
    pub next_damage: f32,
    pub rng: CrtRand,
}

impl SafeZones {
    pub fn new(bus_exit_time: f32, seed: u32) -> SafeZones {
        SafeZones { start_time: bus_exit_time + START_DELAY_S, check_origin: bus_exit_time, active: false, phase: 0, locations: vec![], ind: None, ge_level: INITIAL_GE_LEVEL, inside_tag: false,
            next_check: bus_exit_time + INSIDE_CHECK_PERIOD_S, next_damage: f32::INFINITY, rng: CrtRand(seed) }
    }
    pub fn damage_fraction(&self) -> f32 { SAFE_ZONE_DAMAGE[self.ge_level.min(PHASE_COUNT - 1)] }
    fn inside_check(&mut self, pawn: Option<[f32; 2]>) {
        if let (Some(ind), Some(p)) = (self.ind.as_ref(), pawn) { self.inside_tag = ind.contains(p); }
    }
    fn activate(&mut self, now: f32, pawn: Option<[f32; 2]>) {
        self.active = true;
        self.phase = 0;
        let vols = volumes();
        self.locations = build_locations(&mut self.rng, &SAFE_ZONE_DEFS, MAP_INFO_ROOT_CM, &vols, &VOLUME_REJECTION_CHANCE);
        let d = SAFE_ZONE_DEFS[0];
        let start = now + d.wait;
        let r = d.radius.max(STARTING_RADIUS_CM);
        let c = self.locations[0];
        self.ind = Some(Indicator { radius: r, last_radius: r, next_radius: r, last_center: c, next_center: c, center: c, start_shrink: start, finish_shrink: start + d.shrink, state: None });
        self.inside_check(pawn);
        self.ge_level = INITIAL_GE_LEVEL;
        self.next_damage = now;
    }
    fn advance(&mut self, now: f32) {
        self.phase += 1;
        let d = SAFE_ZONE_DEFS[self.phase];
        let next = self.locations[self.phase];
        if let Some(ind) = self.ind.as_mut() {
            ind.last_center = ind.next_center;
            ind.next_center = next;
            ind.last_radius = ind.radius;
            ind.next_radius = d.radius;
            ind.start_shrink = now + d.wait;
            ind.finish_shrink = ind.start_shrink + d.shrink;
        }
        self.ge_level = self.phase;
    }
    pub fn tick(&mut self, now: f32, pawn: Option<[f32; 2]>) -> Vec<ZoneEvent> {
        let mut ev = vec![];
        if !self.active && now > self.start_time { self.activate(now, pawn); ev.push(ZoneEvent::Spawned); }
        if self.active && self.phase < PHASE_COUNT - 1 && self.ind.map_or(false, |i| now > i.finish_shrink) { self.advance(now); ev.push(ZoneEvent::Advanced { phase: self.phase }); }
        if let Some(ind) = self.ind.as_mut() { if let Some(s) = ind.tick(now) { ev.push(ZoneEvent::StateChanged(s)); } }
        loop {
            let t = self.next_check.min(self.next_damage);
            if t > now { break; }
            if self.next_check <= self.next_damage {
                self.inside_check(pawn);
                self.next_check += INSIDE_CHECK_PERIOD_S;
            } else {
                if self.active && pawn.is_some() && !self.inside_tag { ev.push(ZoneEvent::Damage { fraction: self.damage_fraction() }); }
                self.next_damage += DAMAGE_PERIOD_S;
            }
        }
        ev
    }
    pub fn phase_step(&self, now: f32) -> (u8, f32) {
        match self.ind {
            None => if now < self.start_time { (6, self.start_time - now) } else { (0, 0.) },
            Some(i) => {
                if i.start_shrink - now > 0. { (7, i.start_shrink - now) }
                else if i.finish_shrink - now > 0. { (8, i.finish_shrink - now) }
                else { (0, 0.) }
            }
        }
    }
}

pub fn health_after_storm_tick(health: f32, max_health: f32, fraction: f32) -> f32 { (health - max_health * fraction).max(0.) }

#[cfg(test)]
mod tests {
    use super::*;
    fn run_until(z: &mut SafeZones, from: f32, to: f32, dt: f32, pawn: Option<[f32; 2]>) -> Vec<(f32, ZoneEvent)> {
        let mut out = vec![];
        let mut t = from;
        while t < to { t += dt; for e in z.tick(t, pawn) { out.push((t, e)); } }
        out
    }
    #[test] fn the_table_matches_athena_game_data() {
        let total: f32 = SAFE_ZONE_DEFS[1..].iter().map(|d| d.wait + d.shrink).sum();
        assert_eq!(total, 1500.);
        assert_eq!(SAFE_ZONE_DEFS.map(|d| d.radius)[..9], crate::storm_state::PHASE_RADII_CM);
    }
    #[test] fn crt_rand_matches_msvc() {
        let mut r = CrtRand(1);
        assert_eq!([r.rand(), r.rand(), r.rand()], [41, 18467, 6334]);
    }
    #[test] fn the_chain_nests_every_zone_in_the_previous_one() {
        let mut in_volume = 0;
        for seed in 1..40 {
            let mut rng = CrtRand(seed);
            let locs = build_locations(&mut rng, &SAFE_ZONE_DEFS, MAP_INFO_ROOT_CM, &volumes(), &VOLUME_REJECTION_CHANCE);
            assert_eq!(locs[0], MAP_INFO_ROOT_CM);
            for i in 1..PHASE_COUNT {
                let (a, b) = (locs[i - 1], locs[i]);
                let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
                assert!(d <= SAFE_ZONE_DEFS[i - 1].radius - SAFE_ZONE_DEFS[i].radius + 1., "seed {seed} phase {i}: {d}");
                assert_eq!(b[2], MAP_INFO_ROOT_CM[2]);
            }
            if (1..PHASE_COUNT - 1).all(|i| volume_encompasses(&SAFE_ZONE_VOLUME_0, [locs[i][0], locs[i][1]])) { in_volume += 1; }
        }
        assert!(in_volume >= 35, "{in_volume}");
    }
    #[test] fn the_island_volume_contains_the_map_centre_and_not_the_spawn_island() {
        assert!(volume_encompasses(&SAFE_ZONE_VOLUME_0, [MAP_INFO_ROOT_CM[0], MAP_INFO_ROOT_CM[1]]));
        assert!(!volume_encompasses(&SAFE_ZONE_VOLUME_0, [-125440., -113664.]));
    }
    #[test] fn the_storm_forms_60_s_after_the_last_stop_at_185000_cm_then_runs_1500_s() {
        let mut z = SafeZones::new(10., 7);
        assert_eq!(z.phase_step(10.), (6, 60.));
        let ev = run_until(&mut z, 10., 71., 0.1, Some([MAP_INFO_ROOT_CM[0], MAP_INFO_ROOT_CM[1]]));
        let spawn = ev.iter().find(|e| e.1 == ZoneEvent::Spawned).unwrap().0;
        assert!(spawn > 70. && spawn < 70.2);
        let adv = ev.iter().find(|e| e.1 == ZoneEvent::Advanced { phase: 1 }).unwrap().0;
        assert!(adv > spawn);
        let i = z.ind.unwrap();
        assert_eq!((i.radius, i.next_radius), (185_000., 80_000.));
        assert!((i.start_shrink - adv - 200.).abs() < 1e-3 && (i.finish_shrink - i.start_shrink - 180.).abs() < 1e-3);
        assert_eq!(z.phase_step(adv + 50.).0, 7);
        assert_eq!(z.phase_step(adv + 250.).0, 8);
        run_until(&mut z, 71., adv + 290., 0.25, None);
        let i = z.ind.unwrap();
        assert!((i.radius - (80_000. + (185_000. - 80_000.) * 0.5)).abs() < 300., "{}", i.radius);
        run_until(&mut z, adv + 290., adv + 1510., 0.25, None);
        assert_eq!(z.phase, 9);
        assert_eq!(z.ind.unwrap().radius, 0.);
        assert_eq!(z.phase_step(adv + 1510.), (0, 0.));
    }
    #[test] fn damage_ticks_once_a_second_outside_only_at_the_ge_level() {
        let far = [-180000., -150000.];
        let mut z = SafeZones::new(0., 3);
        let ev = run_until(&mut z, 0., 69.9, 0.05, Some(far));
        let dmg: Vec<f32> = ev.iter().filter_map(|e| if let ZoneEvent::Damage { fraction } = e.1 { Some(e.0) } else { None }).collect();
        assert_eq!(dmg.len(), 10, "{dmg:?}");
        assert!(dmg.windows(2).all(|w| (w[1] - w[0] - 1.).abs() < 0.06));
        assert_eq!(z.damage_fraction(), 0.008);
        let mut z = SafeZones::new(0., 3);
        let ev = run_until(&mut z, 0., 80., 0.05, Some([MAP_INFO_ROOT_CM[0], MAP_INFO_ROOT_CM[1]]));
        assert!(!ev.iter().any(|e| matches!(e.1, ZoneEvent::Damage { .. })));
        assert_eq!(health_after_storm_tick(100., 100., 0.075), 92.5);
        assert_eq!(health_after_storm_tick(5., 100., 0.1), 0.);
    }
    #[test] fn the_ge_level_follows_the_phase_after_the_first() {
        let mut z = SafeZones::new(0., 3);
        let mut levels = vec![];
        let mut t = 0.;
        while t < 1700. { t += 0.5; for e in z.tick(t, None) { if let ZoneEvent::Advanced { phase } = e { levels.push((phase, z.damage_fraction())); } } }
        assert_eq!(levels.len(), 9);
        for (p, f) in levels { assert_eq!(f, SAFE_ZONE_DAMAGE[p]); }
    }
}
