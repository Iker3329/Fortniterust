//! The ambient system of Athena, rebuilt from the 1.8 data (`data/ambient_layout.json`, written by `fn-audio-ambient` from `Athena_Terrain` and its sublevels).
//!
//! **What exists in the data** (and therefore here):
//! * the **bed** (`AmbientControllerComponent_Athena`, a component of `Athena_PlayerController`): one 2D loop per day phase
//!   (`Temperate_Grasslands_Hills_{Morning,Day,Evening,Night}_Athena_Cue`) chosen by the time-of-day manager's `CurrentDayNightPhase` and re-chosen on every
//!   phase change with a 5 s cross-fade (`Crossfade`: `SpawnSound2D` + `FadeIn(5, 1, 0)`, the previous one `FadeOut(5, 0)`); every 1.5 s (`CheckFrequency`) five
//!   2000 cm line traces from the pawn (forward, back, left, right, up) decide "surrounded": when all hit, `Fort_InsideAmbient_Athena_MixMod` is pushed
//!   (`Fort_Ambient_Outdoors_Athena` x0.6, 1 s fades), the bed's low-pass target goes 20000 -> 5000 Hz (`FInterpTo(cur, target, dt, 1)` every tick) and
//!   `Amb_RoomTone_Athena_Cue` fades in over 1 s; leaving reverses it. There is NO per-biome / per-zone selection in the 1.8 map data: the island is one
//!   biome, the zone is the whole map, and there are no AudioVolume / ReverbVolume / AmbientZone actors in any Athena level (the scan found none).
//! * **lakes** (`BP_AmbientLakeAudioActor`): a looped 2D cue under a Box attenuation (half extents + 1000 cm linear falloff, not spatialised).
//! * **rivers** (`BP_AmbientSplineActor[_River]`): baked `SamplePoints`; three looped cues follow the listener along the spline (closest point, next closest,
//!   a projected "midpoint" that eases with `VInterpTo(speed 5)`), faded in / out over 5 s when the listener is within 5000 cm of the spline (checked every
//!   1 s near, 3 s far).
//! * **emitters**: every other `AudioComponent` placed in the level with `bAutoActivate` (`AmbientSound` stream loops with Box attenuations, waterfalls, chest
//!   hum, wood creaks, a TV), at its world position with the cue's attenuation or the component's override.
//!
//! The [`Director`] is pure logic: it consumes the listener / pawn state and emits [`Act`]s that `fn-audio-sink` executes on the engine. Positions are
//! "audio space" centimetres (the sink's `x, y-up, z`); level data is UE world cm and is mapped by [`WorldMap`].

use crate::ambient_scan::{qrot, rot_to_quat};
use crate::attenuation::{Attenuation, DistanceModel, Shape};
use crate::mixer::Vec3;
use crate::playadj::Adjust;
use serde_json::Value as J;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

pub const LAYOUT_JSON: &str = include_str!("../data/ambient_layout.json");
pub type Key = u32;

/// `Crossfade`: `FadeIn(5, 1, 0)` of the new loop / `FadeOut(5, 0)` of the old (bytecode literals).
pub const CROSSFADE_S: f32 = 5.0;
/// `SetIndoorMixEnabled`: `FadeIn(1, 1, 0)` / `FadeOut(1, 0)` of `Loop_Indoors`.
pub const INDOOR_FADE_S: f32 = 1.0;
/// `ReceiveEndPlay`: `FadeOut(3, 0)` of both bed slots and the indoor loop.
pub const END_PLAY_FADE_S: f32 = 3.0;
/// `FInterpTo(CurrentLPFFreq, TargetLPFFreq, DeltaSeconds, 1)`.
pub const LPF_INTERP_SPEED: f32 = 1.0;
/// `Trace`: `Start + Direction * 2000`.
pub const TRACE_LEN_CM: f32 = 2000.0;
pub const INSIDE_MIX: &str = "/Game/Sounds/FortSoundClassesAndModes/Fort_InsideAmbient_Athena_MixMod";
/// `SetFade`: `FadeIn(5, 1, 0)` / `FadeOut(5, 0)` of the three river components.
pub const RIVER_FADE_S: f32 = 5.0;
/// `VInterpTo(FollowLoc, ClosestPoint + projection, DeltaSeconds, 5)`.
pub const RIVER_FOLLOW_SPEED: f32 = 5.0;
/// Most emitter plays alive at once (nearest first); the mixer holds 48 voices in all. ASSUMED engineering cap (UE virtualises by audibility).
pub const MAX_EMITTER_PLAYS: usize = 24;

fn pkg_of(p: &str) -> String {
    p.split('.').next().unwrap_or(p).to_string()
}
fn f(v: &J, k: &str) -> Option<f32> {
    v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32)
}
fn v3(v: Option<&J>) -> Option<[f64; 3]> {
    let a = v?.as_array()?;
    Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?, a.get(2)?.as_f64()?])
}
fn tail(s: &str) -> &str {
    s.rsplit("::").next().unwrap_or(s)
}

/// `FSoundAttenuationSettings` from the json the scan wrote (UE 4.16 defaults for what the cooked data omits, as in `Attenuation::default`).
pub fn atten_from_json(j: &J) -> Attenuation {
    let mut a = Attenuation::default();
    if let Some(b) = j.get("bAttenuate").and_then(|x| x.as_bool()) {
        a.attenuate = b;
    }
    if let Some(b) = j.get("bSpatialize").and_then(|x| x.as_bool()) {
        a.spatialize = b;
    }
    if let Some(s) = j.get("AttenuationShape").and_then(|x| x.as_str()) {
        a.shape = match tail(s) {
            "Capsule" => Shape::Capsule,
            "Box" => Shape::Box,
            "Cone" => Shape::Cone,
            _ => Shape::Sphere,
        };
    }
    if let Some(e) = v3(j.get("AttenuationShapeExtents")) {
        a.extents = [e[0] as f32, e[1] as f32, e[2] as f32];
    }
    if let Some(x) = f(j, "FalloffDistance") {
        a.falloff_distance = x;
    }
    if let Some(s) = j.get("DistanceAlgorithm").and_then(|x| x.as_str()) {
        a.algorithm = match tail(s) {
            "Logarithmic" => DistanceModel::Logarithmic,
            "Inverse" => DistanceModel::Inverse,
            "LogReverse" => DistanceModel::LogReverse,
            "NaturalSound" => DistanceModel::NaturalSound,
            "Custom" => DistanceModel::Custom,
            _ => DistanceModel::Linear,
        };
    }
    if let Some(x) = f(j, "dBAttenuationAtMax") {
        a.db_attenuation_at_max = x;
    }
    if let Some(x) = f(j, "OmniRadius") {
        a.omni_radius = x;
    }
    if let Some(x) = f(j, "StereoSpread") {
        a.stereo_spread = x;
    }
    a
}

#[derive(Clone, Debug)]
pub struct CueInfo {
    pub attenuate: bool,
    pub spatialize: bool,
    /// Cue-level attenuation range (cm); `None` when the cue has no attenuation or it is off.
    pub max_distance: Option<f32>,
}

/// Day phases of `TODM_A_Child_Athena` (`WorldTimeOfDayManager` of the map).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Morning = 0,
    Day = 1,
    Evening = 2,
    Night = 3,
}

#[derive(Clone, Debug)]
pub struct Tod {
    /// `TimePhaseBegins` (hours) of Morning, Day, Evening, Night. Morning (3.0) and Day (7.0) are serialised in the child; Evening and Night are NOT
    /// (struct defaults), so they are derived: Evening = Day begin + Day length (7 + 12 = 19, = the parent `TODM_A` Evening begin), Night = Morning begin - Night
    /// length (3 - 5 = 22 h).
    pub begins: [f32; 4],
    pub derived: [bool; 4],
    /// `StartTimeOfDayInGame` (the child inherits the parent's 7.0).
    pub start_hour: f32,
    /// `DefaultTimeOfDaySpeed` of the parent (80); whether a 1.8 match advances the clock is native code and not recoverable.
    pub default_speed: f32,
}

impl Tod {
    pub fn phase_at(&self, hour: f32) -> Phase {
        let h = hour.rem_euclid(24.0);
        let b = self.begins;
        if h >= b[3] || h < b[0] {
            Phase::Night
        } else if h >= b[2] {
            Phase::Evening
        } else if h >= b[1] {
            Phase::Day
        } else {
            Phase::Morning
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lake {
    pub name: String,
    pub center_ue: [f64; 3],
    pub rot_deg: [f64; 3],
    pub cue: Option<String>,
    pub half_extents: [f32; 3],
    /// Sphere shaped copy (extents 0) of the override: the curve is evaluated on the distance to the box.
    pub atten: Arc<Attenuation>,
}

#[derive(Clone, Debug)]
pub struct River {
    pub name: String,
    pub samples_ue: Vec<[f64; 3]>,
    pub cue_closest: Option<String>,
    pub cue_next: Option<String>,
    pub cue_mid: Option<String>,
    pub check_close_s: f32,
    pub check_far_s: f32,
    pub threshold_cm: f32,
}

#[derive(Clone, Debug)]
pub struct Emitter {
    pub level: String,
    pub owner: String,
    pub owner_class: String,
    pub name: String,
    pub cue: String,
    pub pos_ue: [f64; 3],
    pub rot_deg: [f64; 3],
    pub volume: f32,
    pub pitch_min: f32,
    pub pitch_max: f32,
    /// `bOverrideAttenuation` + `AttenuationOverrides` (shape as authored).
    pub over: Option<Attenuation>,
}

impl Emitter {
    pub fn id(&self) -> String {
        format!("{}/{}", self.level, self.owner)
    }
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub loops: [String; 5],
    pub check_frequency: f32,
    pub outside_lpf: f32,
    pub inside_lpf: f32,
    pub initial_lpf: f32,
    pub tod: Tod,
    pub lakes: Vec<Lake>,
    pub rivers: Vec<River>,
    pub emitters: Vec<Emitter>,
    pub cues: HashMap<String, CueInfo>,
}

impl Layout {
    pub fn parse(text: &str) -> Result<Layout, String> {
        let j: J = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let c = &j["controller"];
        let lp = |k: &str| c["loops"][k].as_str().map(pkg_of).ok_or_else(|| format!("controller loop {k}"));
        let num = |k: &str| c[k].as_f64().map(|x| x as f32).ok_or_else(|| format!("controller {k}"));
        let ch = &j["time_of_day"]["child"];
        let ph = |i: usize, k: &str| ch["phases"][i][k].as_f64().map(|x| x as f32);
        let b0 = ph(0, "time_phase_begins_h").ok_or("morning begin")?;
        let b1 = ph(1, "time_phase_begins_h").ok_or("day begin")?;
        let l1 = ph(1, "phase_length_h").ok_or("day length")?;
        let l3 = ph(3, "phase_length_h").ok_or("night length")?;
        let tod = Tod {
            begins: [b0, b1, ph(2, "time_phase_begins_h").unwrap_or(b1 + l1), ph(3, "time_phase_begins_h").unwrap_or((b0 - l3).rem_euclid(24.0))],
            derived: [false, false, ph(2, "time_phase_begins_h").is_none(), ph(3, "time_phase_begins_h").is_none()],
            start_hour: j["time_of_day"]["parent"]["start_time_of_day_in_game"].as_f64().ok_or("start time")? as f32,
            default_speed: j["time_of_day"]["parent"]["default_time_of_day_speed"].as_f64().ok_or("tod speed")? as f32,
        };
        let mut cues = HashMap::new();
        for (k, v) in j["cues"].as_object().into_iter().flatten() {
            let a = &v["attenuation"];
            cues.insert(
                pkg_of(k).to_ascii_lowercase(),
                CueInfo { attenuate: a["attenuate"].as_bool().unwrap_or(false), spatialize: a["spatialize"].as_bool().unwrap_or(false), max_distance: a["max_distance_cm"].as_f64().map(|x| x as f32) },
            );
        }
        let sound = |v: &J| v.as_str().filter(|s| !s.is_empty()).map(pkg_of);
        let mut lakes = vec![];
        for l in j["lakes"].as_array().into_iter().flatten() {
            let att = l.get("attenuation").filter(|a| a.is_object()).map(atten_from_json).unwrap_or_default();
            let mut sph = att.clone();
            let half = sph.extents;
            sph.shape = Shape::Sphere;
            sph.extents = [0.0; 3];
            lakes.push(Lake {
                name: l["name"].as_str().unwrap_or("").into(),
                center_ue: v3(l.get("center_cm")).ok_or("lake center")?,
                rot_deg: v3(l.get("rot_deg")).unwrap_or([0.0; 3]),
                cue: sound(&l["sound"]),
                half_extents: half,
                atten: Arc::new(sph),
            });
        }
        let mut rivers = vec![];
        for r in j["rivers"].as_array().into_iter().flatten() {
            rivers.push(River {
                name: r["name"].as_str().unwrap_or("").into(),
                samples_ue: r["samples"].as_array().into_iter().flatten().filter_map(|p| v3(Some(p))).collect(),
                cue_closest: sound(&r["sounds"]["SoundClosest"]),
                cue_next: sound(&r["sounds"]["SoundNextClosest"]),
                cue_mid: sound(&r["sounds"]["SoundMidpoint"]),
                check_close_s: f(r, "check_frequency_close").ok_or("river close freq")?,
                check_far_s: f(r, "check_frequency_far").ok_or("river far freq")?,
                threshold_cm: f(r, "close_far_distance_threshold").ok_or("river threshold")?,
            });
        }
        let mut emitters = vec![];
        for e in j["emitters"].as_array().into_iter().flatten() {
            let oc = e["owner_class"].as_str().unwrap_or("");
            // lake / river components are handled by their actors above; components that are not auto-activated are started by gameplay (hydrants, toilets, ...)
            if oc.starts_with("BP_AmbientLake") || oc.starts_with("BP_AmbientSpline") || e["auto_activate"].as_bool() != Some(true) {
                continue;
            }
            let Some(cue) = sound(&e["sound"]) else { continue };
            let over = (e["override_attenuation"].as_bool() == Some(true)).then(|| e.get("attenuation").filter(|a| a.is_object()).map(atten_from_json)).flatten();
            emitters.push(Emitter {
                level: e["level"].as_str().unwrap_or("").into(),
                owner: e["owner"].as_str().unwrap_or("").into(),
                owner_class: oc.into(),
                name: e["name"].as_str().unwrap_or("").into(),
                cue,
                pos_ue: v3(e.get("pos_cm")).ok_or("emitter pos")?,
                rot_deg: v3(e.get("rot_deg")).unwrap_or([0.0; 3]),
                volume: f(e, "volume").unwrap_or(1.0),
                pitch_min: f(e, "pitch_min").unwrap_or(1.0),
                pitch_max: f(e, "pitch_max").unwrap_or(1.0),
                over,
            });
        }
        Ok(Layout {
            loops: [lp("morning")?, lp("day")?, lp("evening")?, lp("night")?, lp("indoors")?],
            check_frequency: num("check_frequency_s")?,
            outside_lpf: num("outside_lpf_hz")?,
            inside_lpf: num("inside_lpf_hz")?,
            initial_lpf: num("initial_lpf_hz")?,
            tod,
            lakes,
            rivers,
            emitters,
            cues,
        })
    }

    /// The layout compiled into the crate.
    pub fn embedded() -> &'static Layout {
        static L: OnceLock<Layout> = OnceLock::new();
        L.get_or_init(|| Layout::parse(LAYOUT_JSON).expect("data/ambient_layout.json"))
    }

    /// Every cue the director can play (package paths), bed loops first.
    pub fn cue_list(&self) -> Vec<String> {
        let mut v: Vec<String> = self.loops.to_vec();
        let mut add = |c: &Option<String>| {
            if let Some(c) = c {
                if !v.contains(c) {
                    v.push(c.clone());
                }
            }
        };
        for l in &self.lakes {
            add(&l.cue);
        }
        for r in &self.rivers {
            add(&r.cue_closest);
            add(&r.cue_next);
            add(&r.cue_mid);
        }
        for e in &self.emitters {
            add(&Some(e.cue.clone()));
        }
        v
    }
}

/// The ambient cues as `&'static str` (for the audio pack's cue list).
pub fn cue_paths() -> &'static [&'static str] {
    static P: OnceLock<Vec<&'static str>> = OnceLock::new();
    P.get_or_init(|| Layout::embedded().cue_list().into_iter().map(|s| &*Box::leak(s.into_boxed_str())).collect())
}

/// UE world cm (left handed, Z up) <-> audio space cm (`x, y up, z`): `audio = anchor + (ue.x - c.x, ue.z - c.z, ue.y - c.y)`, the same placement as the game's real props
/// (`props::spawn_layout`: relative to the layout centre, moved to `ANCHOR`, flat baseplate).
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldMap {
    pub center_ue_cm: [f32; 3],
    pub anchor_cm: [f32; 3],
}

impl WorldMap {
    pub fn to_audio(&self, ue: [f64; 3]) -> Vec3 {
        let (c, a) = (self.center_ue_cm, self.anchor_cm);
        Vec3::new(a[0] + (ue[0] as f32 - c[0]), a[1] + (ue[2] as f32 - c[2]), a[2] + (ue[1] as f32 - c[1]))
    }
    pub fn to_ue(&self, p: Vec3) -> [f64; 3] {
        let (c, a) = (self.center_ue_cm, self.anchor_cm);
        [(p.x - a[0] + c[0]) as f64, (p.z - a[2] + c[1]) as f64, (p.y - a[1] + c[2]) as f64]
    }
}

/// Distance (cm) from `p` to an oriented box (centre, FRotator degrees, half extents), 0 inside. All UE space.
pub fn box_distance(p: [f64; 3], center: [f64; 3], rot_deg: [f64; 3], half: [f32; 3]) -> f32 {
    let q = rot_to_quat(rot_deg);
    let l = qrot([-q[0], -q[1], -q[2], q[3]], [p[0] - center[0], p[1] - center[1], p[2] - center[2]]);
    let d = |i: usize| ((l[i].abs() - half[i] as f64).max(0.0)) as f32;
    (d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt()
}

/// What the director wants the engine to do (executed by `fn-audio-sink`).
#[derive(Clone, Debug)]
pub enum Act {
    /// Start `cue` (`position` None = 2D) at `volume` / `pitch`, then apply `adjust` (initial gain, attenuation override, ...).
    Play { key: Key, cue: String, position: Option<Vec3>, volume: f32, pitch: f32, adjust: Adjust },
    Adjust { key: Key, adjust: Adjust },
    Stop { key: Key },
    PushMix(&'static str),
    PopMix(&'static str),
}

/// Per-frame input.
pub struct Frame<'a> {
    pub dt: f32,
    /// Listener (camera) position, audio-space cm.
    pub listener: Vec3,
    /// The pawn (`GetViewTarget` actor) location, audio-space cm, and its horizontal forward direction.
    pub pawn: Vec3,
    pub forward: Vec3,
    /// `LineTraceSingle` against world geometry (ignoring pawns): `(origin cm, direction, length cm) -> hit`. Called only while a "surrounded" probe runs.
    pub trace: &'a mut dyn FnMut(Vec3, Vec3, f32) -> bool,
}

#[derive(Default)]
struct Bed {
    started: bool,
    slot_a: Option<Key>,
    slot_b: Option<Key>,
    active_is_a: bool,
    indoor: Option<Key>,
    inside_cur: bool,
    inside_prev: bool,
    lpf_cur: f32,
    lpf_target: f32,
    lpf_sent: f32,
    phase: Option<Phase>,
    timer: f32,
    probe: Option<usize>,
}

struct RiverState {
    active: bool,
    timer: f32,
    period: f32,
    closest: Vec3,
    next: Vec3,
    follow: Vec3,
    closest_index: usize,
    keys: [Option<Key>; 3],
}

#[derive(Default)]
struct LakeState {
    key: Option<Key>,
    last_dist: f32,
}

#[derive(Default)]
struct EmitterState {
    key: Option<Key>,
    last_dist: f32,
}

pub struct Director {
    layout: &'static Layout,
    pub map: WorldMap,
    enabled: bool,
    pub hour: f32,
    /// Game hours per real second (0 = the clock holds `hour`).
    pub speed_hours_per_s: f32,
    bed: Bed,
    lakes: Vec<LakeState>,
    rivers: Vec<RiverState>,
    emitters: Vec<EmitterState>,
    disabled_owners: Vec<String>,
    next_key: Key,
    rng: u64,
    out: Vec<Act>,
}

fn finterp_to(cur: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    cur + d * (dt * speed).clamp(0.0, 1.0)
}

fn dist2(a: Vec3, b: Vec3) -> f32 {
    let d = a.sub(b);
    d.dot(d)
}

impl Director {
    pub fn new(layout: &'static Layout) -> Director {
        Director {
            layout,
            map: WorldMap::default(),
            enabled: false,
            hour: layout.tod.start_hour,
            speed_hours_per_s: 0.0,
            bed: Bed { lpf_cur: layout.initial_lpf, lpf_target: layout.outside_lpf, lpf_sent: layout.initial_lpf, active_is_a: false, ..Default::default() },
            lakes: layout.lakes.iter().map(|_| LakeState::default()).collect(),
            rivers: layout
                .rivers
                .iter()
                .map(|r| RiverState { active: false, timer: r.check_far_s, period: r.check_far_s, closest: Vec3::default(), next: Vec3::default(), follow: Vec3::default(), closest_index: 0, keys: [None; 3] })
                .collect(),
            emitters: layout.emitters.iter().map(|_| EmitterState::default()).collect(),
            disabled_owners: vec![],
            next_key: 1,
            rng: 0x2545F4914F6CDD1D,
            out: vec![],
        }
    }

    pub fn layout(&self) -> &'static Layout {
        self.layout
    }
    pub fn phase(&self) -> Phase {
        self.layout.tod.phase_at(self.hour)
    }
    pub fn is_inside(&self) -> bool {
        self.bed.inside_cur
    }
    /// The bed cue currently fading in / playing (None until started).
    pub fn bed_cue(&self) -> Option<&'static str> {
        self.bed.phase.map(|p| self.layout.loops[p as usize].as_str())
    }
    /// Emitter plays alive right now.
    pub fn active_emitters(&self) -> usize {
        self.emitters.iter().filter(|e| e.key.is_some()).count()
    }
    pub fn active_lakes(&self) -> usize {
        self.lakes.iter().filter(|e| e.key.is_some()).count()
    }
    pub fn active_rivers(&self) -> usize {
        self.rivers.iter().filter(|r| r.active).count()
    }
    /// Silence / restore the hum of an owner (`level/owner`, e.g. an opened chest).
    pub fn set_owner_enabled(&mut self, id: &str, on: bool) {
        self.disabled_owners.retain(|o| o != id);
        if !on {
            self.disabled_owners.push(id.to_string());
        }
    }

    /// Enable (`BeginPlay`) / disable (`EndPlay`) the whole system. Returns the acts of the transition (also available from the next `update`).
    pub fn set_enabled(&mut self, on: bool) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        if !on {
            // ReceiveEndPlay: FadeOut(3, 0) of A, B and the indoor loop, pop the inside mix
            for k in [self.bed.slot_a.take(), self.bed.slot_b.take(), self.bed.indoor.take()].into_iter().flatten() {
                self.out.push(Act::Adjust { key: k, adjust: Adjust { gain: Some((None, 0.0, END_PLAY_FADE_S, true)), ..Default::default() } });
            }
            if self.bed.inside_cur {
                self.out.push(Act::PopMix(INSIDE_MIX));
            }
            let b = &mut self.bed;
            b.started = false;
            b.phase = None;
            b.inside_cur = false;
            b.inside_prev = false;
            b.probe = None;
            b.lpf_target = self.layout.outside_lpf;
            for l in 0..self.lakes.len() {
                if let Some(k) = self.lakes[l].key.take() {
                    self.out.push(Act::Stop { key: k });
                }
            }
            for e in 0..self.emitters.len() {
                if let Some(k) = self.emitters[e].key.take() {
                    self.out.push(Act::Stop { key: k });
                }
            }
            for r in &mut self.rivers {
                r.active = false;
                for k in r.keys.iter_mut().filter_map(|k| k.take()) {
                    self.out.push(Act::Stop { key: k });
                }
            }
        }
    }

    fn key(&mut self) -> Key {
        let k = self.next_key;
        self.next_key += 1;
        k
    }
    fn rand01(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        ((x.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32) / (1u64 << 24) as f32
    }

    /// `Crossfade(NewSound)`: the previous loops fade out over 5 s, the new one is spawned 2D and fades in over 5 s (low-pass enabled at the current frequency).
    fn crossfade(&mut self, phase: Phase) {
        for k in [self.bed.slot_a.take(), self.bed.slot_b.take()].into_iter().flatten() {
            self.out.push(Act::Adjust { key: k, adjust: Adjust { gain: Some((None, 0.0, CROSSFADE_S, true)), ..Default::default() } });
        }
        let key = self.key();
        let lpf = (self.bed.lpf_cur < 19_999.0).then_some(self.bed.lpf_cur);
        self.out.push(Act::Play {
            key,
            cue: self.layout.loops[phase as usize].clone(),
            position: None,
            volume: 1.0,
            pitch: 1.0,
            adjust: Adjust { gain: Some((Some(0.0), 1.0, CROSSFADE_S, false)), lpf_hz: Some(lpf), ..Default::default() },
        });
        if self.bed.active_is_a {
            self.bed.slot_b = Some(key);
        } else {
            self.bed.slot_a = Some(key);
        }
        self.bed.active_is_a = !self.bed.active_is_a;
        self.bed.phase = Some(phase);
    }

    /// `SetIndoorMixEnabled`.
    fn indoor(&mut self, on: bool) {
        if on {
            let key = self.key();
            self.out.push(Act::Play { key, cue: self.layout.loops[4].clone(), position: Some(Vec3::default()), volume: 1.0, pitch: 1.0, adjust: Adjust { gain: Some((Some(0.0), 1.0, INDOOR_FADE_S, false)), ..Default::default() } });
            self.bed.indoor = Some(key);
        } else if let Some(k) = self.bed.indoor.take() {
            self.out.push(Act::Adjust { key: k, adjust: Adjust { gain: Some((None, 0.0, INDOOR_FADE_S, true)), ..Default::default() } });
        }
    }

    /// `Evaluate Inside State`.
    fn evaluate_inside(&mut self) {
        if self.bed.inside_cur == self.bed.inside_prev {
            return;
        }
        if self.bed.inside_cur {
            self.out.push(Act::PushMix(INSIDE_MIX));
            self.bed.lpf_target = self.layout.inside_lpf;
            self.indoor(true);
        } else {
            self.out.push(Act::PopMix(INSIDE_MIX));
            self.bed.lpf_target = self.layout.outside_lpf;
            self.indoor(false);
        }
        self.bed.inside_prev = self.bed.inside_cur;
    }

    fn update_bed(&mut self, fr: &mut Frame) {
        if !self.bed.started {
            // ReceiveBeginPlay: Crossfade(loop of CurrentDayNightPhase), bind the day-phase callbacks, start the looping CheckFrequency timer
            self.bed.started = true;
            let p = self.phase();
            self.crossfade(p);
        }
        // OnDayPhaseChange
        let p = self.phase();
        if self.bed.phase != Some(p) {
            self.crossfade(p);
        }
        // OnCheckIfSurrounded: every CheckFrequency; the five traces run one per frame (a `Delay(0)` between them)
        self.bed.timer += fr.dt;
        if self.bed.probe.is_none() && self.bed.timer >= self.layout.check_frequency {
            self.bed.timer -= self.layout.check_frequency;
            self.bed.inside_prev = self.bed.inside_cur;
            self.bed.inside_cur = false;
            self.bed.probe = Some(0);
        }
        if let Some(step) = self.bed.probe {
            let up = Vec3::new(0.0, 1.0, 0.0);
            let fwd = fr.forward;
            let right = fwd.cross(up).normalized();
            let dirs = [fwd, Vec3::new(-fwd.x, -fwd.y, -fwd.z), Vec3::new(-right.x, -right.y, -right.z), right, up];
            let hit = (fr.trace)(fr.pawn, dirs[step], TRACE_LEN_CM);
            if !hit {
                self.bed.probe = None;
                self.evaluate_inside();
            } else if step + 1 == dirs.len() {
                self.bed.probe = None;
                self.bed.inside_cur = true;
                self.evaluate_inside();
            } else {
                self.bed.probe = Some(step + 1);
            }
        }
        // ReceiveTick: CurrentLPFFreq = FInterpTo(Current, Target, dt, 1); SetLowPassFilterFrequency on A and B
        self.bed.lpf_cur = finterp_to(self.bed.lpf_cur, self.bed.lpf_target, fr.dt, LPF_INTERP_SPEED);
        if (self.bed.lpf_cur - self.bed.lpf_sent).abs() > 1.0 {
            self.bed.lpf_sent = self.bed.lpf_cur;
            let hz = (self.bed.lpf_cur < 19_999.0).then_some(self.bed.lpf_cur);
            for k in [self.bed.slot_a, self.bed.slot_b].into_iter().flatten() {
                self.out.push(Act::Adjust { key: k, adjust: Adjust { lpf_hz: Some(hz), ..Default::default() } });
            }
        }
    }

    fn update_lakes(&mut self, fr: &Frame) {
        let lis_ue = self.map.to_ue(fr.listener);
        for i in 0..self.layout.lakes.len() {
            let lake = &self.layout.lakes[i];
            let Some(cue) = lake.cue.clone() else { continue };
            let d = box_distance(lis_ue, lake.center_ue, lake.rot_deg, lake.half_extents);
            let reach = lake.atten.falloff_distance;
            let st = &mut self.lakes[i];
            match st.key {
                None if d < reach => {
                    let key = self.next_key;
                    self.next_key += 1;
                    st.key = Some(key);
                    st.last_dist = d;
                    let pos = self.map.to_audio(lake.center_ue);
                    self.out.push(Act::Play {
                        key,
                        cue,
                        position: Some(pos),
                        volume: 1.0,
                        pitch: 1.0,
                        adjust: Adjust { position: Some(Some(pos)), atten: Some(Some(lake.atten.clone())), dist: Some(Some(d)), ..Default::default() },
                    });
                }
                Some(k) if d > reach * 1.1 + 100.0 => {
                    st.key = None;
                    self.out.push(Act::Stop { key: k });
                }
                Some(k) if (d - st.last_dist).abs() > 1.0 => {
                    st.last_dist = d;
                    self.out.push(Act::Adjust { key: k, adjust: Adjust { dist: Some(Some(d)), ..Default::default() } });
                }
                _ => {}
            }
        }
    }

    fn update_emitters(&mut self, fr: &Frame) {
        let lis_ue = self.map.to_ue(fr.listener);
        // (index, effective distance, reach)
        let mut near: Vec<(usize, f32, f32)> = vec![];
        for i in 0..self.layout.emitters.len() {
            let e = &self.layout.emitters[i];
            let info = self.layout.cues.get(&e.cue.to_ascii_lowercase());
            let (d, reach) = match &e.over {
                Some(a) if a.shape == Shape::Box => (box_distance(lis_ue, e.pos_ue, e.rot_deg, a.extents), a.falloff_distance),
                Some(a) => {
                    let c = self.map.to_ue(self.map.to_audio(e.pos_ue));
                    let dd = ((lis_ue[0] - c[0]).powi(2) + (lis_ue[1] - c[1]).powi(2) + (lis_ue[2] - c[2]).powi(2)).sqrt() as f32;
                    (dd, a.max_distance())
                }
                None => {
                    let dd = ((lis_ue[0] - e.pos_ue[0]).powi(2) + (lis_ue[1] - e.pos_ue[1]).powi(2) + (lis_ue[2] - e.pos_ue[2]).powi(2)).sqrt() as f32;
                    // no attenuation (or none that culls): audible everywhere; the cue's own range otherwise
                    (dd, info.and_then(|c| c.max_distance).unwrap_or(f32::INFINITY))
                }
            };
            let off = self.disabled_owners.iter().any(|o| *o == e.id());
            if reach.is_finite() && (d < reach || self.emitters[i].key.is_some()) && !off {
                near.push((i, d, reach));
            } else if let Some(k) = self.emitters[i].key.take() {
                self.out.push(Act::Stop { key: k });
            }
        }
        near.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let mut alive = 0;
        for (i, d, reach) in near {
            let started = self.emitters[i].key.is_some();
            if started && d > reach * 1.1 + 100.0 {
                let k = self.emitters[i].key.take().unwrap();
                self.out.push(Act::Stop { key: k });
                continue;
            }
            if !started && (d >= reach || alive >= MAX_EMITTER_PLAYS) {
                continue;
            }
            alive += 1;
            let e = self.layout.emitters[i].clone();
            if let Some(k) = self.emitters[i].key {
                if (d - self.emitters[i].last_dist).abs() > 1.0 && matches!(&e.over, Some(a) if a.shape == Shape::Box) {
                    self.emitters[i].last_dist = d;
                    self.out.push(Act::Adjust { key: k, adjust: Adjust { dist: Some(Some(d)), ..Default::default() } });
                }
                continue;
            }
            let key = self.key();
            self.emitters[i].key = Some(key);
            self.emitters[i].last_dist = d;
            let mut adjust = Adjust { position: Some(Some(self.map.to_audio(e.pos_ue))), ..Default::default() };
            if let Some(a) = &e.over {
                let mut a2 = a.clone();
                if a.shape == Shape::Box {
                    a2.shape = Shape::Sphere;
                    a2.extents = [0.0; 3];
                    adjust.dist = Some(Some(d));
                }
                adjust.atten = Some(Some(Arc::new(a2)));
            }
            let pitch = if e.pitch_max > e.pitch_min { e.pitch_min + (e.pitch_max - e.pitch_min) * self.rand01() } else { e.pitch_min };
            self.out.push(Act::Play { key, cue: e.cue.clone(), position: Some(self.map.to_audio(e.pos_ue)), volume: e.volume, pitch, adjust });
        }
    }

    fn update_rivers(&mut self, fr: &Frame) {
        let target = fr.pawn;
        for i in 0..self.layout.rivers.len() {
            let r = &self.layout.rivers[i];
            if r.samples_ue.len() < 2 {
                continue;
            }
            let pts: Vec<Vec3> = r.samples_ue.iter().map(|p| self.map.to_audio(*p)).collect();
            let n = pts.len() - 1; // NumSamplePoints
            let st = &mut self.rivers[i];
            st.timer -= fr.dt;
            if st.timer <= 0.0 {
                st.timer += st.period.max(0.05);
                // Check Closest Point: nearest sample, then the nearer of its two neighbours (an out of range Array_Get yields the zero vector, never nearer)
                let (mut ci, mut best) = (0usize, f32::INFINITY);
                for (k, p) in pts.iter().enumerate() {
                    let d = dist2(*p, target);
                    if d < best {
                        best = d;
                        ci = k;
                    }
                }
                let nb = |k: isize| (k >= 0 && (k as usize) < pts.len()).then(|| pts[k as usize]);
                let (a, b) = (nb(ci as isize + 1), nb(ci as isize - 1));
                st.next = match (a, b) {
                    (Some(a), Some(b)) => if dist2(a, target) < dist2(b, target) { a } else { b },
                    (Some(a), None) => a,
                    (None, Some(b)) => b,
                    _ => pts[ci],
                };
                st.closest = pts[ci];
                st.closest_index = ci;
                let near = best < r.threshold_cm * r.threshold_cm;
                if near && !st.active {
                    st.active = true;
                    st.period = r.check_close_s;
                    st.follow = st.closest;
                    for (slot, cue, pos) in [(0, &r.cue_closest, st.closest), (1, &r.cue_next, st.next), (2, &r.cue_mid, st.follow)] {
                        if let Some(c) = cue {
                            let key = self.next_key;
                            self.next_key += 1;
                            st.keys[slot] = Some(key);
                            self.out.push(Act::Play { key, cue: c.clone(), position: Some(pos), volume: 1.0, pitch: 1.0, adjust: Adjust { gain: Some((Some(0.0), 1.0, RIVER_FADE_S, false)), ..Default::default() } });
                        }
                    }
                } else if !near && st.active {
                    st.active = false;
                    st.period = r.check_far_s;
                    for k in st.keys.iter_mut().filter_map(|k| k.take()) {
                        self.out.push(Act::Adjust { key: k, adjust: Adjust { gain: Some((None, 0.0, RIVER_FADE_S, true)), ..Default::default() } });
                    }
                } else if st.active {
                    for (slot, pos) in [(0, st.closest), (1, st.next)] {
                        if let Some(k) = st.keys[slot] {
                            self.out.push(Act::Adjust { key: k, adjust: Adjust { position: Some(Some(pos)), ..Default::default() } });
                        }
                    }
                }
            }
            if st.active {
                // ReceiveTick: FollowLoc = VInterpTo(FollowLoc, ClosestPoint + Project(Target - ClosestPoint onto Next - ClosestPoint), dt, 5); at either end of the spline it sits on the closest point
                let seg = st.next.sub(st.closest);
                let l2 = seg.dot(seg);
                let goal = if l2 > 1e-6 {
                    let t = target.sub(st.closest).dot(seg) / l2;
                    Vec3::new(st.closest.x + seg.x * t, st.closest.y + seg.y * t, st.closest.z + seg.z * t)
                } else {
                    st.closest
                };
                if st.closest_index == 0 || st.closest_index == n {
                    st.follow = st.closest;
                } else {
                    let k = (fr.dt * RIVER_FOLLOW_SPEED).clamp(0.0, 1.0);
                    st.follow = Vec3::new(st.follow.x + (goal.x - st.follow.x) * k, st.follow.y + (goal.y - st.follow.y) * k, st.follow.z + (goal.z - st.follow.z) * k);
                }
                if let Some(key) = st.keys[2] {
                    self.out.push(Act::Adjust { key, adjust: Adjust { position: Some(Some(st.follow)), ..Default::default() } });
                }
            }
        }
    }

    /// Advance everything by one frame and return what the engine must do.
    pub fn update(&mut self, fr: &mut Frame) -> Vec<Act> {
        if self.enabled {
            self.hour = (self.hour + self.speed_hours_per_s * fr.dt).rem_euclid(24.0);
            self.update_bed(fr);
            self.update_lakes(fr);
            self.update_emitters(fr);
            self.update_rivers(fr);
        }
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> Director {
        let mut d = Director::new(Layout::embedded());
        d.set_enabled(true);
        d
    }
    fn frame<'a>(dt: f32, at: Vec3, trace: &'a mut dyn FnMut(Vec3, Vec3, f32) -> bool) -> Frame<'a> {
        Frame { dt, listener: at, pawn: at, forward: Vec3::new(0.0, 0.0, -1.0), trace }
    }

    #[test]
    fn the_embedded_layout_parses_and_matches_the_scan() {
        let l = Layout::embedded();
        assert!(l.loops[1].ends_with("Temperate_Grasslands_Hills_Day_Athena_Cue"), "{:?}", l.loops);
        assert!(l.loops[4].ends_with("Amb_RoomTone_Athena_Cue"));
        assert_eq!((l.check_frequency, l.outside_lpf, l.inside_lpf), (1.5, 20_000.0, 5_000.0));
        assert_eq!(l.lakes.len(), 4);
        assert_eq!(l.lakes.iter().filter(|x| x.cue.is_some()).count(), 3, "BP_AmbientLakeAudioActor3 has no Sound set");
        assert_eq!(l.rivers.len(), 5);
        assert_eq!(l.rivers.iter().map(|r| r.samples_ue.len() - 1).collect::<Vec<_>>().iter().filter(|n| **n == 60 || **n == 15 || **n == 10 || **n == 80 || **n == 40).count(), 5);
        assert!(l.emitters.iter().all(|e| !e.cue.is_empty()));
        assert!(l.cues.contains_key(&l.loops[1].to_ascii_lowercase()));
    }

    #[test]
    fn time_of_day_starts_in_the_day_phase_with_the_recovered_boundaries() {
        let t = &Layout::embedded().tod;
        assert_eq!(t.start_hour, 7.0);
        assert_eq!(t.begins[0], 3.0);
        assert_eq!(t.begins[1], 7.0);
        assert_eq!((t.begins[2], t.derived[2]), (19.0, true));
        assert_eq!((t.begins[3], t.derived[3]), (22.0, true));
        for (h, p) in [(7.0, Phase::Day), (6.99, Phase::Morning), (2.99, Phase::Night), (3.0, Phase::Morning), (18.9, Phase::Day), (19.0, Phase::Evening), (22.0, Phase::Night), (23.9, Phase::Night)] {
            assert_eq!(t.phase_at(h), p, "{h}");
        }
    }

    #[test]
    fn the_bed_starts_with_the_day_loop_fading_in_over_five_seconds_and_crossfades_on_a_phase_change() {
        let mut d = dir();
        let mut miss = |_: Vec3, _: Vec3, _: f32| false;
        let acts = d.update(&mut frame(0.016, Vec3::default(), &mut miss));
        let Some(Act::Play { cue, adjust, position, .. }) = acts.first() else { panic!("{acts:?}") };
        assert!(cue.ends_with("Temperate_Grasslands_Hills_Day_Athena_Cue"));
        assert_eq!(*position, None, "SpawnSound2D");
        assert_eq!(adjust.gain, Some((Some(0.0), 1.0, 5.0, false)));
        // the clock reaches the evening: the day loop fades out (5 s, stops) and the evening loop fades in
        d.hour = 19.0;
        let acts = d.update(&mut frame(0.016, Vec3::default(), &mut miss));
        assert!(acts.iter().any(|a| matches!(a, Act::Adjust { adjust, .. } if adjust.gain == Some((None, 0.0, 5.0, true)))));
        assert!(acts.iter().any(|a| matches!(a, Act::Play { cue, .. } if cue.ends_with("Evening_Athena_Cue"))));
    }

    #[test]
    fn five_hits_every_1_5_s_make_it_indoors_and_a_miss_makes_it_outdoors_again() {
        let mut d = dir();
        let mut all = |_: Vec3, _: Vec3, _: f32| true;
        let mut inside = false;
        let mut pushed = 0;
        for _ in 0..200 {
            let acts = d.update(&mut frame(0.016, Vec3::default(), &mut all));
            pushed += acts.iter().filter(|a| matches!(a, Act::PushMix(_))).count();
            inside |= d.is_inside();
        }
        assert!(inside && pushed == 1, "pushed {pushed}");
        assert!(d.bed.lpf_target == 5_000.0 && d.bed.lpf_cur < 20_000.0, "the low-pass eases toward 5 kHz");
        let mut none = |_: Vec3, _: Vec3, _: f32| false;
        let mut popped = 0;
        for _ in 0..200 {
            let acts = d.update(&mut frame(0.016, Vec3::default(), &mut none));
            popped += acts.iter().filter(|a| matches!(a, Act::PopMix(_))).count();
        }
        assert!(!d.is_inside() && popped == 1);
        assert_eq!(d.bed.lpf_target, 20_000.0);
    }

    #[test]
    fn a_box_attenuation_is_one_inside_and_falls_off_linearly_over_the_falloff_distance() {
        assert_eq!(box_distance([0.0, 0.0, 0.0], [0.0; 3], [0.0; 3], [100.0, 200.0, 50.0]), 0.0);
        assert!((box_distance([100.0 + 300.0, 0.0, 0.0], [0.0; 3], [0.0; 3], [100.0, 200.0, 50.0]) - 300.0).abs() < 1e-3);
        // yawed 90 degrees: the long axis now points along world X
        assert!(box_distance([150.0, 0.0, 0.0], [0.0; 3], [0.0, 90.0, 0.0], [100.0, 200.0, 50.0]) < 1e-3);
        let l = &Layout::embedded().lakes[0];
        assert_eq!(l.atten.extents, [0.0; 3]);
        assert!((l.atten.gain(500.0) - 0.5).abs() < 1e-5, "Linear, falloff 1000");
    }

    #[test]
    fn a_lake_plays_inside_its_box_and_stops_far_away_and_a_river_follows_the_listener() {
        let mut d = dir();
        let lake = Layout::embedded().lakes.iter().find(|l| l.cue.is_some()).unwrap().clone();
        let inside = d.map.to_audio(lake.center_ue);
        let mut miss = |_: Vec3, _: Vec3, _: f32| false;
        let acts = d.update(&mut frame(0.016, inside, &mut miss));
        assert!(acts.iter().any(|a| matches!(a, Act::Play { cue, adjust, .. } if cue.ends_with("Amb_Water_LakeLoop_Cue") && adjust.dist == Some(Some(0.0)))));
        assert_eq!(d.active_lakes(), 1);
        let far = Vec3::new(inside.x + 500_000.0, inside.y, inside.z);
        let acts = d.update(&mut frame(0.016, far, &mut miss));
        assert!(acts.iter().any(|a| matches!(a, Act::Stop { .. })));
        assert_eq!(d.active_lakes(), 0);
        // river: first check after 3 s (far period), activates within 5000 cm of a sample
        let r = &Layout::embedded().rivers[0];
        let at = d.map.to_audio(r.samples_ue[3]);
        let mut played = 0;
        for _ in 0..400 {
            played += d.update(&mut frame(0.016, at, &mut miss)).iter().filter(|a| matches!(a, Act::Play { adjust, .. } if adjust.gain == Some((Some(0.0), 1.0, 5.0, false)) )).count();
        }
        assert!(played >= 3 || d.active_rivers() >= 1, "river sounds started ({played})");
    }
}
