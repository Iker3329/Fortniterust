use crate::interp::{Interp, Sample};
use crate::proto::{self, Build, Fire, HealthEntry, Hit, Msg, State, Storm};
use glam::Vec3;
use std::collections::BTreeMap;

pub const SEND_HZ: f32 = 20.;
pub const MAX_HEALTH: f32 = 100.;
pub const MAX_SHIELD: f32 = 100.;

#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    Welcome { slot: u8, host: bool, code: String },
    Lobby(Vec<(u8, String, bool)>),
    Start { seed: u32 },
    Left { slot: u8 },
    End { reason: String },
    Kicked { reason: String },
    Error { code: String },
}

pub fn parse_control(text: &str) -> Option<Control> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_owned();
    Some(match v.get("t")?.as_str()? {
        "welcome" => Control::Welcome { slot: v.get("slot")?.as_u64()? as u8, host: v.get("host")?.as_bool()?, code: s("code") },
        "lobby" => Control::Lobby(v.get("players")?.as_array()?.iter().filter_map(|p| Some((p.get("slot")?.as_u64()? as u8, p.get("name")?.as_str()?.to_owned(), p.get("host")?.as_bool()?))).collect()),
        "start" => Control::Start { seed: v.get("seed").and_then(|x| x.as_u64()).unwrap_or(0) as u32 },
        "left" => Control::Left { slot: v.get("slot")?.as_u64()? as u8 },
        "end" => Control::End { reason: s("reason") },
        "kicked" => Control::Kicked { reason: s("reason") },
        "error" => Control::Error { code: s("code") },
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Started { seed: u32 },
    Ended(String),
    Damaged { attacker: u8, amount: f32, shield_hit: bool, head: bool },
    DamageShown { victim: u8, amount: f32, shield_hit: bool, head: bool },
    OwnHealth { health: f32, shield: f32, dead: bool },
    Elim { victim: u8, killer: u8, weapon: u8, head: bool },
    Respawn(Vec3),
    Correct(Vec3),
    RemoteFire { slot: u8, fire: Fire },
    Build { slot: u8, build: Build },
    Interact { slot: u8, kind: u8, id: u32 },
    Storm(Storm),
    Joined { slot: u8, name: String },
    Left { slot: u8, name: String },
}

#[derive(Default)]
pub struct Remote { pub interp: Interp, pub name: String, pub health: f32, pub shield: f32, pub dead: bool, pub seen: bool }
impl Remote {
    pub fn sample(&self, now_ms: f64) -> Option<Sample> { self.interp.sample(now_ms) }
}

pub struct Session {
    pub slot: u8,
    pub host: bool,
    pub code: String,
    pub started: bool,
    pub ended: Option<String>,
    pub names: BTreeMap<u8, String>,
    pub remotes: BTreeMap<u8, Remote>,
    pub health: f32,
    pub shield: f32,
    pub dead: bool,
    outbox: Vec<Vec<u8>>,
    events: Vec<Event>,
    seq: u32,
    fire_seq: u32,
    send_accum: f32,
    reported: (u8, u8),
    storm_in: f32,
}

fn to_u8(v: f32) -> u8 { v.round().clamp(0., 100.) as u8 }

impl Session {
    pub fn new(slot: u8, host: bool, code: String) -> Session {
        Session { slot, host, code, started: false, ended: None, names: BTreeMap::new(), remotes: BTreeMap::new(), health: MAX_HEALTH, shield: 0., dead: false,
            outbox: vec![], events: vec![], seq: 0, fire_seq: 0, send_accum: 1., reported: (100, 0), storm_in: 0. }
    }
    pub fn take_outbox(&mut self) -> Vec<Vec<u8>> { std::mem::take(&mut self.outbox) }
    pub fn take_events(&mut self) -> Vec<Event> { std::mem::take(&mut self.events) }
    pub fn name_of(&self, slot: u8) -> String { if slot == proto::NO_KILLER { "The Storm".into() } else { self.names.get(&slot).cloned().unwrap_or_else(|| format!("Player {slot}")) } }

    pub fn control(&mut self, c: Control) {
        match c {
            Control::Welcome { slot, host, code } => { self.slot = slot; self.host = host; self.code = code; }
            Control::Lobby(list) => {
                for (slot, name, _) in &list {
                    if !self.names.contains_key(slot) && *slot != self.slot { self.events.push(Event::Joined { slot: *slot, name: name.clone() }); }
                    self.names.insert(*slot, name.clone());
                }
                let live: Vec<u8> = list.iter().map(|p| p.0).collect();
                self.names.retain(|s, _| live.contains(s));
                self.remotes.retain(|s, _| live.contains(s));
            }
            Control::Start { seed } => { if !self.started { self.started = true; self.dead = false; self.health = MAX_HEALTH; self.shield = 0.; self.reported = (100, 0); self.events.push(Event::Started { seed }); } }
            Control::Left { slot } => {
                let name = self.name_of(slot);
                self.remotes.remove(&slot);
                self.names.remove(&slot);
                self.events.push(Event::Left { slot, name });
            }
            Control::End { reason } | Control::Kicked { reason } | Control::Error { code: reason } => { if self.ended.is_none() { self.ended = Some(reason.clone()); self.events.push(Event::Ended(reason)); } }
        }
    }

    pub fn tick_send(&mut self, dt: f32) -> bool {
        self.send_accum += dt;
        if self.send_accum >= 1. / SEND_HZ { self.send_accum -= 1. / SEND_HZ; if self.send_accum >= 1. / SEND_HZ { self.send_accum = 0.; } true } else { false }
    }
    pub fn send_state(&mut self, mut s: State, now_ms: f64) {
        self.seq = self.seq.wrapping_add(1);
        s.seq = self.seq;
        s.t_ms = now_ms as u32;
        self.outbox.push(proto::encode(&Msg::State(s)));
    }
    pub fn send_fire(&mut self, weapon: u8, origin: Vec3, dir: Vec3, flags: u8) -> u32 {
        self.fire_seq = self.fire_seq.wrapping_add(1);
        self.outbox.push(proto::encode(&Msg::Fire(Fire { seq: self.fire_seq, weapon, flags, origin, dir: dir.normalize_or_zero() })));
        self.fire_seq
    }
    pub fn send_hit(&mut self, h: Hit) { self.outbox.push(proto::encode(&Msg::Hit(h))); }
    pub fn send_build(&mut self, b: &Build) { self.outbox.push(proto::encode(&Msg::Build(b.clone()))); }
    pub fn send_interact(&mut self, kind: u8, id: u32) { self.outbox.push(proto::encode(&Msg::Interact { kind, id })); }
    pub fn send_storm(&mut self, s: Storm) { if self.host { self.outbox.push(proto::encode(&Msg::Storm(s))); } }

    pub fn report_local_vitals(&mut self, health: f32, shield: f32) {
        if self.dead { return; }
        let (h, s) = (to_u8(health), to_u8(shield));
        let auth = (to_u8(self.health), to_u8(self.shield));
        if (h, s) == auth || (h, s) == self.reported { return; }
        self.reported = (h, s);
        let m = if h as u16 + s as u16 >= auth.0 as u16 + auth.1 as u16 { Msg::Heal { health: h, shield: s } } else { Msg::SelfDamage { health: h, shield: s } };
        self.outbox.push(proto::encode(&m));
    }

    pub fn recv(&mut self, bytes: &[u8], now_ms: f64) {
        let Some((from, m)) = proto::decode_relayed(bytes) else { return };
        self.handle(from, m, now_ms);
    }

    fn handle(&mut self, from: u8, m: Msg, now_ms: f64) {
        match m {
            Msg::State(s) => {
                if from == self.slot { return; }
                let r = self.remotes.entry(from).or_insert_with(|| Remote { health: MAX_HEALTH, ..Default::default() });
                r.name = self.names.get(&from).cloned().unwrap_or_default();
                r.seen = true;
                r.interp.push(s, now_ms);
            }
            Msg::Fire(f) => if from != self.slot { self.events.push(Event::RemoteFire { slot: from, fire: f }); },
            Msg::Build(b) => if from != self.slot { self.events.push(Event::Build { slot: from, build: b }); },
            Msg::Interact { kind, id } => if from != self.slot { self.events.push(Event::Interact { slot: from, kind, id }); },
            Msg::Hit(_) | Msg::Heal { .. } | Msg::SelfDamage { .. } => {}
            Msg::Health(list) => if from == proto::SERVER_SLOT { self.apply_health(&list) },
            Msg::Elim { victim, killer, weapon, head } => {
                if from != proto::SERVER_SLOT { return; }
                if victim == self.slot { self.dead = true; self.health = 0.; self.shield = 0.; }
                if let Some(r) = self.remotes.get_mut(&victim) { r.dead = true; }
                self.events.push(Event::Elim { victim, killer, weapon, head });
            }
            Msg::Damage { victim, attacker, flags, amount } => {
                if from != proto::SERVER_SLOT { return; }
                if victim == self.slot { self.events.push(Event::Damaged { attacker, amount, shield_hit: flags & 1 != 0, head: flags & 2 != 0 }); }
                else if attacker == self.slot { self.events.push(Event::DamageShown { victim, amount, shield_hit: flags & 1 != 0, head: flags & 2 != 0 }); }
            }
            Msg::Spawn { slot, pos } => {
                if from != proto::SERVER_SLOT { return; }
                if slot == self.slot { self.dead = false; self.health = MAX_HEALTH; self.shield = 0.; self.reported = (100, 0); self.events.push(Event::Respawn(pos)); }
                else if let Some(r) = self.remotes.get_mut(&slot) { r.dead = false; r.health = MAX_HEALTH; r.shield = 0.; r.interp.reset(); }
            }
            Msg::Storm(s) => if from != self.slot { self.events.push(Event::Storm(s)); },
            Msg::Correct(p) => self.events.push(Event::Correct(p)),
            Msg::Loot(_) | Msg::Pong { .. } => {}
        }
    }

    fn apply_health(&mut self, list: &[HealthEntry]) {
        for e in list {
            if e.slot == self.slot {
                self.health = e.health as f32; self.shield = e.shield as f32; self.dead = e.dead;
                self.reported = (e.health, e.shield);
                self.events.push(Event::OwnHealth { health: self.health, shield: self.shield, dead: e.dead });
            } else if let Some(r) = self.remotes.get_mut(&e.slot) { r.health = e.health as f32; r.shield = e.shield as f32; r.dead = e.dead; }
        }
    }

    pub fn host_tick(&mut self, dt: f32, storm: Option<Storm>) {
        if !self.host { return; }
        self.storm_in -= dt;
        if self.storm_in <= 0. { self.storm_in = 2.; if let Some(s) = storm { self.send_storm(s); } }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::relayed;

    fn server(to: &mut Session, m: Msg) { to.recv(&relayed(proto::SERVER_SLOT, &proto::encode(&m)), 0.); }

    #[test] fn control_messages_parse() {
        assert_eq!(parse_control(r#"{"t":"welcome","code":"ABC234","slot":3,"host":false,"token":"x"}"#), Some(Control::Welcome { slot: 3, host: false, code: "ABC234".into() }));
        assert_eq!(parse_control(r#"{"t":"lobby","players":[{"slot":0,"name":"Host","host":true}]}"#), Some(Control::Lobby(vec![(0, "Host".into(), true)])));
        assert_eq!(parse_control(r#"{"t":"end","reason":"host_left"}"#), Some(Control::End { reason: "host_left".into() }));
    }

    #[test] fn server_damage_health_and_elim_reach_the_victim_and_the_shooter() {
        let mut v = Session::new(1, false, "ABCDEF".into());
        let mut k = Session::new(0, true, "ABCDEF".into());
        for s in [&mut v, &mut k] { s.control(Control::Lobby(vec![(0, "K".into(), true), (1, "V".into(), false)])); s.control(Control::Start { seed: 1 }); s.take_events(); }
        for s in [&mut v, &mut k] {
            server(s, Msg::Damage { victim: 1, attacker: 0, flags: 1 | 2, amount: 20. });
            server(s, Msg::Damage { victim: 1, attacker: 0, flags: 2, amount: 60. });
            server(s, Msg::Health(vec![HealthEntry { slot: 1, health: 40, shield: 0, dead: false }]));
        }
        let ev = v.take_events();
        assert!(ev.contains(&Event::Damaged { attacker: 0, amount: 20., shield_hit: true, head: true }));
        assert!(ev.contains(&Event::OwnHealth { health: 40., shield: 0., dead: false }));
        let ek = k.take_events();
        assert!(ek.contains(&Event::DamageShown { victim: 1, amount: 60., shield_hit: false, head: true }));
        for s in [&mut v, &mut k] { server(s, Msg::Elim { victim: 1, killer: 0, weapon: 1, head: false }); }
        assert!(v.dead && v.take_events().contains(&Event::Elim { victim: 1, killer: 0, weapon: 1, head: false }));
        server(&mut v, Msg::Spawn { slot: 1, pos: Vec3::ZERO });
        assert!(!v.dead && v.take_events().contains(&Event::Respawn(Vec3::ZERO)));
    }

    #[test] fn vitals_from_a_player_are_ignored() {
        let mut c = Session::new(1, false, "ABCDEF".into());
        c.recv(&relayed(0, &proto::encode(&Msg::Health(vec![HealthEntry { slot: 1, health: 1, shield: 0, dead: false }]))), 0.);
        c.recv(&relayed(0, &proto::encode(&Msg::Elim { victim: 1, killer: 0, weapon: 1, head: false })), 0.);
        c.recv(&relayed(0, &proto::encode(&Msg::Damage { victim: 1, attacker: 0, flags: 0, amount: 50. })), 0.);
        assert_eq!((c.health, c.dead), (MAX_HEALTH, false));
        assert!(c.take_events().is_empty());
    }

    #[test] fn local_vitals_claims_follow_the_authority() {
        let mut c = Session::new(1, false, "ABCDEF".into());
        server(&mut c, Msg::Health(vec![HealthEntry { slot: 1, health: 70, shield: 0, dead: false }]));
        c.take_events();
        c.report_local_vitals(70., 0.);
        assert!(c.take_outbox().is_empty(), "matching the authority sends nothing");
        c.report_local_vitals(85., 0.);
        assert_eq!(c.take_outbox()[0][0], proto::HEAL);
        c.report_local_vitals(60., 0.);
        assert_eq!(c.take_outbox()[0][0], proto::SELFDMG);
    }

    #[test] fn only_the_host_sends_storm() {
        let mut h = Session::new(0, true, "X".into());
        let mut c = Session::new(1, false, "X".into());
        h.host_tick(0.1, Some(Storm::default()));
        c.host_tick(0.1, Some(Storm::default()));
        assert_eq!(h.take_outbox()[0][0], proto::STORM);
        assert!(c.take_outbox().is_empty());
    }

    #[test] fn states_feed_the_remote_interpolator() {
        let mut c = Session::new(1, false, "X".into());
        c.control(Control::Lobby(vec![(0, "Host".into(), true), (1, "Me".into(), false)]));
        for k in 0..4 { c.recv(&relayed(0, &proto::encode(&Msg::State(State { seq: k + 1, t_ms: k * 50, pos: Vec3::new(k as f32, 2., 0.), ..Default::default() }))), k as f64 * 50.); }
        let r = &c.remotes[&0];
        assert_eq!(r.name, "Host");
        assert!(r.sample(500.).is_some());
        c.recv(&relayed(1, &proto::encode(&Msg::State(State::default()))), 0.);
        assert!(!c.remotes.contains_key(&1), "own echoes are ignored");
    }

    #[test] fn send_rate_is_twenty_hertz() {
        let mut s = Session::new(0, false, "X".into());
        let n = (0..120).filter(|_| s.tick_send(1. / 60.)).count();
        assert!((39..=41).contains(&n), "{n}");
    }
}
