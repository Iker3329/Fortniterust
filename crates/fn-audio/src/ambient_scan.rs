//! Level scan for the ambient system (build-time tool: needs the 1.8 paks, never used by the game at run time).
//!
//! Reads `/Game/Athena/Maps/Athena_Terrain` (persistent level + every always-loaded sublevel, each sublevel's `LevelTransform`
//! applied exactly like fn-export's props layout: `child * parent`, FTransform order) and writes `data/ambient_layout.json`:
//!
//! * `controller`  - `AmbientControllerComponent_Athena` (the biome / time-of-day bed owned by `Athena_PlayerController`): the five
//!   loop cues (class default object), the LPF frequencies / check frequency, and the numbers its Kismet bytecode uses (fade times,
//!   trace length, SoundMix, ...), which are read from the disassembly by [`controller_constants`].
//! * `time_of_day` - `WorldTimeOfDayManager` (`TODM_A_Child_Athena` over `TODM_A`): the day phases and the start time.
//! * `lakes`       - `BP_AmbientLakeAudioActor` instances: a looped 2D cue under a Box attenuation.
//! * `rivers`      - `BP_AmbientSplineActor*` instances: the baked `SamplePoints` and the three cues that follow the listener.
//! * `emitters`    - every other `AudioComponent` placed in the persistent level / sublevels with its world position, sound and overrides.
//! * `volumes`     - audio / reverb volume actors found in the level (none exist in 1.8 Athena; the key records the scan).

use fn_pak::PakSet;
use fn_uasset::{ObjectRef, Package, PackageData, PackageIndex, Value};
use serde_json::{json, Map, Value as J};
use std::collections::{BTreeMap, HashMap};

pub const MAP: &str = "/Game/Athena/Maps/Athena_Terrain";
pub const CONTROLLER: &str = "/Game/Athena/Sounds/Ambient/AmbientControllerComponent_Athena";
pub const TODM: &str = "/Game/TimeOfDay/TODM/TODM_A_Child_Athena";
pub const TODM_PARENT: &str = "/Game/TimeOfDay/TODM/TODM_A";

#[derive(Clone, Copy, Debug)]
pub struct Xf {
    pub q: [f64; 4],
    pub t: [f64; 3],
    pub s: [f64; 3],
}
pub const IDENT: Xf = Xf { q: [0.0, 0.0, 0.0, 1.0], t: [0.0; 3], s: [1.0; 3] };

fn qmul(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}
pub fn qrot(q: [f64; 4], v: [f64; 3]) -> [f64; 3] {
    let u = [q[0], q[1], q[2]];
    let c = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let t = c(u, v);
    let t = [2.0 * t[0], 2.0 * t[1], 2.0 * t[2]];
    let u2 = c(u, t);
    [v[0] + q[3] * t[0] + u2[0], v[1] + q[3] * t[1] + u2[1], v[2] + q[3] * t[2] + u2[2]]
}
/// `FRotator(Pitch, Yaw, Roll)` degrees -> `FQuat`.
pub fn rot_to_quat(r: [f64; 3]) -> [f64; 4] {
    let h = |d: f64| (d.to_radians() * 0.5).sin_cos();
    let (sp, cp) = h(r[0]);
    let (sy, cy) = h(r[1]);
    let (sr, cr) = h(r[2]);
    [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]
}
/// `FQuat::Rotator`.
pub fn quat_to_rot(q: [f64; 4]) -> [f64; 3] {
    let [x, y, z, w] = q;
    let st = z * x - w * y;
    let yaw_y = 2.0 * (w * z + x * y);
    let yaw_x = 1.0 - 2.0 * (y * y + z * z);
    let norm = |a: f64| {
        let mut a = a % 360.0;
        if a > 180.0 {
            a -= 360.0
        } else if a < -180.0 {
            a += 360.0
        }
        a
    };
    let t = 0.4999995;
    if st < -t {
        let yaw = yaw_y.atan2(yaw_x).to_degrees();
        [-90.0, yaw, norm(-yaw - 2.0 * x.atan2(w).to_degrees())]
    } else if st > t {
        let yaw = yaw_y.atan2(yaw_x).to_degrees();
        [90.0, yaw, norm(yaw - 2.0 * x.atan2(w).to_degrees())]
    } else {
        [(2.0 * st).asin().to_degrees(), yaw_y.atan2(yaw_x).to_degrees(), (-2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y)).to_degrees()]
    }
}
/// `child * parent` (apply child first, then parent), UE `FTransform` convention.
pub fn compose(child: Xf, parent: Xf) -> Xf {
    let sc = [parent.s[0] * child.t[0], parent.s[1] * child.t[1], parent.s[2] * child.t[2]];
    let r = qrot(parent.q, sc);
    Xf { q: qmul(parent.q, child.q), t: [r[0] + parent.t[0], r[1] + parent.t[1], r[2] + parent.t[2]], s: [child.s[0] * parent.s[0], child.s[1] * parent.s[1], child.s[2] * parent.s[2]] }
}
pub fn apply(x: Xf, p: [f64; 3]) -> [f64; 3] {
    let r = qrot(x.q, [p[0] * x.s[0], p[1] * x.s[1], p[2] * x.s[2]]);
    [r[0] + x.t[0], r[1] + x.t[1], r[2] + x.t[2]]
}

fn arr(v: Option<&J>, n: usize) -> Option<Vec<f64>> {
    let a = v?.as_array()?;
    if a.len() < n {
        return None;
    }
    a.iter().take(n).map(|x| x.as_f64()).collect()
}
fn xf_from_json(v: &J) -> Xf {
    let mut x = IDENT;
    if let Some(r) = arr(v.get("Rotation"), 4) {
        x.q = [r[0], r[1], r[2], r[3]];
    }
    if let Some(t) = arr(v.get("Translation"), 3) {
        x.t = [t[0], t[1], t[2]];
    }
    if let Some(s) = arr(v.get("Scale3D"), 3) {
        x.s = [s[0], s[1], s[2]];
    }
    x
}
fn short(p: &str) -> &str {
    p.rsplit(['/', '.', ':']).next().unwrap_or(p)
}
fn pkg_of(p: &str) -> &str {
    p.split('.').next().unwrap_or(p)
}

/// Persistent map + always-loaded sublevels with their transforms (same walk as fn-export `props layout`).
pub fn levels(set: &PakSet) -> Result<Vec<(String, Xf)>, String> {
    let pd = PackageData::load(set, MAP).map_err(|e| e.to_string())?;
    let pkg = pd.package().map_err(|e| e.to_string())?;
    let mut out = vec![(MAP.to_string(), IDENT)];
    for i in 0..pkg.exports.len() {
        if !pkg.export_class_name(i).starts_with("LevelStreaming") {
            continue;
        }
        let o = pkg.read_export(i).map_err(|e| e.to_string())?;
        let j = o.props.to_json(&pkg);
        let Some(wa) = j.get("WorldAsset").and_then(|v| v.as_str()) else { continue };
        out.push((pkg_of(wa).to_string(), j.get("LevelTransform").map(xf_from_json).unwrap_or(IDENT)));
    }
    Ok(out)
}

struct Cache<'a> {
    set: &'a PakSet,
    /// `package#export name` -> json of that export's tagged properties (templates / class default objects)
    tpl: HashMap<String, Option<J>>,
}

impl<'a> Cache<'a> {
    fn template(&mut self, package: &str, name: &str) -> Option<J> {
        let key = format!("{package}#{name}");
        if let Some(v) = self.tpl.get(&key) {
            return v.clone();
        }
        let v = (|| {
            let d = PackageData::load(self.set, package).ok()?;
            let p = d.package().ok()?;
            let i = p.find_export(name)?;
            Some(p.read_export(i).ok()?.props.to_json(&p))
        })();
        self.tpl.insert(key, v.clone());
        v
    }
    /// Property json of the archetype of export `i` (component template in a Blueprint package, or a level export).
    fn archetype(&mut self, pkg: &Package, i: usize) -> Option<J> {
        let t = pkg.exports[i].template_index;
        match pkg.resolve(t) {
            ObjectRef::Import { package, name, .. } => self.template(&package, &name),
            ObjectRef::Export { index, .. } => pkg.read_export(index).ok().map(|o| o.props.to_json(pkg)),
            ObjectRef::Null => None,
        }
    }
    /// Class default object json of the Blueprint class `export i` is an instance of.
    fn class_defaults(&mut self, pkg: &Package, i: usize) -> Option<J> {
        match pkg.resolve(pkg.exports[i].class_index) {
            ObjectRef::Import { package, name, .. } => {
                let cdo = format!("Default__{name}");
                self.template(&package, &cdo)
            }
            _ => None,
        }
    }
}

fn get<'j>(inst: &'j J, tpl: &'j Option<J>, key: &str) -> Option<&'j J> {
    inst.get(key).or_else(|| tpl.as_ref().and_then(|t| t.get(key)))
}

struct LevelCtx<'a, 'p> {
    pkg: &'a Package<'p>,
    props: HashMap<usize, J>,
    tpls: HashMap<usize, Option<J>>,
    xf: HashMap<usize, Xf>,
    level_xf: Xf,
}

impl<'a, 'p> LevelCtx<'a, 'p> {
    fn props(&mut self, i: usize) -> J {
        if let Some(j) = self.props.get(&i) {
            return j.clone();
        }
        let j = self.pkg.read_export(i).map(|o| o.props.to_json(self.pkg)).unwrap_or(J::Null);
        self.props.insert(i, j.clone());
        j
    }
    fn parent(&self, i: usize) -> Option<usize> {
        let o = self.pkg.read_export(i).ok()?;
        o.props.get("AttachParent").and_then(Value::as_object).and_then(|p: PackageIndex| p.export_index())
    }
    /// World transform (level transform included) of component export `i`: relative transforms along `AttachParent`, each merged with its template.
    fn world(&mut self, i: usize, cache: &mut Cache, depth: usize) -> Xf {
        if let Some(x) = self.xf.get(&i) {
            return *x;
        }
        let inst = self.props(i);
        let tpl = if let Some(t) = self.tpls.get(&i) { t.clone() } else {
            let t = cache.archetype(self.pkg, i);
            self.tpls.insert(i, t.clone());
            t
        };
        let loc = arr(get(&inst, &tpl, "RelativeLocation"), 3).unwrap_or(vec![0.0; 3]);
        let rot = arr(get(&inst, &tpl, "RelativeRotation"), 3).unwrap_or(vec![0.0; 3]);
        let scl = arr(get(&inst, &tpl, "RelativeScale3D"), 3).unwrap_or(vec![1.0; 3]);
        let rel = Xf { q: rot_to_quat([rot[0], rot[1], rot[2]]), t: [loc[0], loc[1], loc[2]], s: [scl[0], scl[1], scl[2]] };
        let parent = if depth < 16 { self.parent(i) } else { None };
        let x = match parent {
            Some(p) => compose(rel, self.world(p, cache, depth + 1)),
            None => compose(rel, self.level_xf),
        };
        self.xf.insert(i, x);
        x
    }
}

fn round(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn r3(v: [f64; 3]) -> J {
    json!([round(v[0]), round(v[1]), round(v[2])])
}

/// The `Sound` of a component: instance, then archetype, as a package path (no object suffix).
fn sound_of(inst: &J, tpl: &Option<J>) -> Option<String> {
    get(inst, tpl, "Sound").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| pkg_of(s).to_string())
}

/// Everything the layout file holds.
pub fn scan(set: &PakSet) -> Result<J, String> {
    let lvls = levels(set)?;
    let mut cache = Cache { set, tpl: HashMap::new() };
    let mut emitters: Vec<J> = vec![];
    let mut lakes: Vec<J> = vec![];
    let mut rivers: Vec<J> = vec![];
    let mut volumes: Vec<J> = vec![];
    let mut class_hist: BTreeMap<String, usize> = BTreeMap::new();
    let mut ambient_classes: BTreeMap<String, usize> = BTreeMap::new();
    for (lp, lxf) in &lvls {
        let pd = PackageData::load(set, lp).map_err(|e| format!("{lp}: {e}"))?;
        let pkg = pd.package().map_err(|e| format!("{lp}: {e}"))?;
        let mut cx = LevelCtx { pkg: &pkg, props: HashMap::new(), tpls: HashMap::new(), xf: HashMap::new(), level_xf: *lxf };
        let level_name = short(lp).to_string();
        for i in 0..pkg.exports.len() {
            let cn = pkg.export_class_name(i);
            let lower = cn.to_ascii_lowercase();
            if pkg.exports[i].name.starts_with("Default__") {
                continue;
            }
            if lower.contains("audiovolume") || lower.contains("reverb") || lower.contains("ambientzone") {
                volumes.push(json!({"level": level_name, "class": cn, "name": pkg.exports[i].name}));
            }
            if lower.contains("ambient") || lower.contains("audio") && !cn.ends_with("Component") {
                *ambient_classes.entry(cn.clone()).or_default() += 1;
            }
            match cn.as_str() {
                "AudioComponent" => {
                    let inst = cx.props(i);
                    let tpl = cache.archetype(&pkg, i);
                    let owner_i = pkg.exports[i].outer_index.export_index();
                    let (owner_class, owner_name) = owner_i.map(|o| (pkg.export_class_name(o), pkg.exports[o].name.clone())).unwrap_or_default();
                    *class_hist.entry(owner_class.clone()).or_default() += 1;
                    let sound = sound_of(&inst, &tpl);
                    let w = cx.world(i, &mut cache, 0);
                    let mut e = Map::new();
                    e.insert("level".into(), level_name.clone().into());
                    e.insert("owner".into(), owner_name.into());
                    e.insert("owner_class".into(), owner_class.into());
                    e.insert("name".into(), pkg.exports[i].name.clone().into());
                    e.insert("sound".into(), sound.map(J::from).unwrap_or(J::Null));
                    e.insert("pos_cm".into(), r3(w.t));
                    e.insert("rot_deg".into(), r3(quat_to_rot(w.q)));
                    e.insert("volume".into(), get(&inst, &tpl, "VolumeMultiplier").cloned().unwrap_or(json!(1.0)));
                    e.insert("pitch".into(), get(&inst, &tpl, "PitchMultiplier").cloned().unwrap_or(json!(1.0)));
                    e.insert("pitch_min".into(), get(&inst, &tpl, "PitchModulationMin").cloned().unwrap_or(json!(1.0)));
                    e.insert("pitch_max".into(), get(&inst, &tpl, "PitchModulationMax").cloned().unwrap_or(json!(1.0)));
                    e.insert("auto_activate".into(), get(&inst, &tpl, "bAutoActivate").cloned().unwrap_or(json!(true)));
                    e.insert("override_attenuation".into(), get(&inst, &tpl, "bOverrideAttenuation").cloned().unwrap_or(json!(false)));
                    if let Some(a) = get(&inst, &tpl, "AttenuationOverrides") {
                        e.insert("attenuation".into(), a.clone());
                    }
                    emitters.push(J::Object(e));
                }
                c if c.starts_with("BP_AmbientLakeAudioActor") => {
                    let inst = cx.props(i);
                    let cdo = cache.class_defaults(&pkg, i);
                    let root = pkg.read_export(i).ok().and_then(|o| o.props.get("RootComponent").and_then(Value::as_object)).and_then(|p| p.export_index());
                    let w = root.map(|r| cx.world(r, &mut cache, 0)).unwrap_or(*lxf);
                    // the WaterLoopAudio component carries the attenuation the construction script applied (and the Sound set by SetSound)
                    let comp = inst.get("WaterLoopAudio").and_then(|v| v.as_str()).and_then(|p| (0..pkg.exports.len()).find(|&k| pkg.object_path(PackageIndex(k as i32 + 1)) == p));
                    let (cinst, ctpl) = match comp {
                        Some(k) => (cx.props(k), cache.archetype(&pkg, k)),
                        None => (J::Null, None),
                    };
                    let sound = sound_of(&cinst, &ctpl).or_else(|| get(&inst, &cdo, "Sound").and_then(|v| v.as_str()).map(|s| pkg_of(s).to_string()));
                    lakes.push(json!({
                        "level": level_name, "name": pkg.exports[i].name, "class": c,
                        "center_cm": r3(w.t), "rot_deg": r3(quat_to_rot(w.q)), "scale": r3(w.s),
                        "sound": sound,
                        "attenuation": cinst.get("AttenuationOverrides").cloned().or_else(|| get(&inst, &cdo, "AttenuationOverride").cloned()),
                        "attenuation_extents": get(&inst, &cdo, "AttenuationExtents"),
                        "falloff_distance": get(&inst, &cdo, "FalloffDistance"),
                        "volume": cinst.get("VolumeMultiplier"),
                    }));
                }
                c if c.starts_with("BP_AmbientSplineActor") => {
                    let inst = cx.props(i);
                    let cdo = cache.class_defaults(&pkg, i);
                    // base-class CDO values come from the parent Blueprint; the River subclass only overrides the three sounds
                    let base = cache.template("/Game/Athena/Sounds/Ambient/BP_AmbientSplineActor", "Default__BP_AmbientSplineActor_C");
                    let pick = |k: &str| inst.get(k).or_else(|| cdo.as_ref().and_then(|c| c.get(k))).or_else(|| base.as_ref().and_then(|c| c.get(k))).cloned();
                    let find = |name: &str| -> Option<usize> { inst.get(name).and_then(|v| v.as_str()).and_then(|p| (0..pkg.exports.len()).find(|&k| pkg.object_path(PackageIndex(k as i32 + 1)) == p)) };
                    let mut sounds = Map::new();
                    for (field, key) in [("AudioClosest", "SoundClosest"), ("AudioNextClosest", "SoundNextClosest"), ("AudioMidpoint", "SoundMidpoint")] {
                        let s = find(field).and_then(|k| {
                            let ci = cx.props(k);
                            let ct = cache.archetype(&pkg, k);
                            sound_of(&ci, &ct)
                        });
                        let s = s.or_else(|| pick(key).and_then(|v| v.as_str().map(|s| pkg_of(s).to_string())));
                        sounds.insert(key.to_string(), s.map(J::from).unwrap_or(J::Null));
                    }
                    let samples: Vec<J> = inst
                        .get("SamplePoints")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|p| arr(Some(p), 3)).map(|p| r3(apply(*lxf, [p[0], p[1], p[2]]))).collect())
                        .unwrap_or_default();
                    rivers.push(json!({
                        "level": level_name, "name": pkg.exports[i].name, "class": c,
                        "sounds": sounds, "num_sample_points": pick("NumSamplePoints"), "samples": samples,
                        "check_frequency_close": pick("CheckFrequencyClose"), "check_frequency_far": pick("CheckFrequencyFar"),
                        "close_far_distance_threshold": pick("CloseFarDistanceThreshold"),
                    }));
                }
                _ => {}
            }
        }
    }
    Ok(json!({
        "schema": 1,
        "map": MAP,
        "units": "UE world space, cm, left handed, Z up (level transforms applied)",
        "levels": lvls.iter().map(|(p, x)| json!({"package": p, "translation": x.t, "rotation": x.q})).collect::<Vec<_>>(),
        "controller": controller_json(set)?,
        "time_of_day": tod_json(set)?,
        "lakes": lakes, "rivers": rivers, "emitters": emitters, "volumes": volumes,
        "scan": {"audio_component_owner_classes": class_hist, "ambient_or_audio_actor_classes": ambient_classes},
    }))
}

/// Every cue (package path, no object suffix) the scanned layout can play: bed loops, indoor loop, lakes, rivers, emitters.
pub fn referenced_cues(layout: &J) -> Vec<String> {
    let mut v: Vec<String> = vec![];
    let mut add = |s: Option<&J>| {
        if let Some(s) = s.and_then(|x| x.as_str()) {
            let s = pkg_of(s).to_string();
            if !s.is_empty() && !v.contains(&s) {
                v.push(s);
            }
        }
    };
    if let Some(l) = layout["controller"]["loops"].as_object() {
        for x in l.values() {
            add(Some(x));
        }
    }
    for e in layout["emitters"].as_array().into_iter().flatten() {
        add(e.get("sound"));
    }
    for e in layout["lakes"].as_array().into_iter().flatten() {
        add(e.get("sound"));
    }
    for r in layout["rivers"].as_array().into_iter().flatten() {
        for x in r["sounds"].as_object().into_iter().flat_map(|o| o.values()) {
            add(Some(x));
        }
    }
    v
}

fn num(v: Option<&J>) -> Option<f64> {
    v.and_then(|x| x.as_f64())
}

/// `AmbientControllerComponent_Athena` class defaults plus the constants of its bytecode.
fn controller_json(set: &PakSet) -> Result<J, String> {
    let d = PackageData::load(set, CONTROLLER).map_err(|e| e.to_string())?;
    let p = d.package().map_err(|e| e.to_string())?;
    let cdo = p.read_default_object().ok_or("no CDO")?.map_err(|e| e.to_string())?.props.to_json(&p);
    let mut consts = Map::new();
    let mut lines: Vec<String> = vec![];
    for i in 0..p.exports.len() {
        if p.export_class_name(i) != "Function" {
            continue;
        }
        if let Ok(f) = fn_uasset::classes::read_function(&p, i) {
            if f.script.is_empty() {
                continue;
            }
            let dis = fn_uasset::kismet::disassemble(&p, f.script);
            for (_, l) in dis.lines {
                lines.push(format!("{}: {l}", p.exports[i].name));
            }
        }
    }
    let find = |pat: &str| lines.iter().filter(|l| l.contains(pat)).cloned().collect::<Vec<_>>();
    // FadeIn(duration, volume, start) / FadeOut(duration, volume) literals, exactly as disassembled
    consts.insert("crossfade_lines".into(), json!(lines.iter().filter(|l| l.starts_with("Crossfade:") && (l.contains("FadeIn") || l.contains("FadeOut") || l.contains("SpawnSound2D"))).collect::<Vec<_>>()));
    consts.insert("indoor_lines".into(), json!(lines.iter().filter(|l| l.starts_with("SetIndoorMixEnabled:")).collect::<Vec<_>>()));
    consts.insert("trace_lines".into(), json!(find("LineTraceSingle")));
    consts.insert("mix_lines".into(), json!(find("SoundMixModifier")));
    consts.insert("fadeout_lines".into(), json!(find("FadeOut")));
    consts.insert("interp_lines".into(), json!(find("FInterpTo")));
    consts.insert("timer_lines".into(), json!(find("K2_SetTimerDelegate")));
    Ok(json!({
        "loops": {"morning": cdo.get("Loop_Morning"), "day": cdo.get("Loop_Day"), "evening": cdo.get("Loop_Evening"), "night": cdo.get("Loop_Night"), "indoors": cdo.get("Loop_Indoors")},
        "check_frequency_s": num(cdo.get("CheckFrequency")),
        "outside_lpf_hz": num(cdo.get("OutsideLPFFreq")),
        "inside_lpf_hz": num(cdo.get("InsideLPFFreq")),
        "initial_lpf_hz": num(cdo.get("CurrentLPFFreq")),
        "target_lpf_hz": num(cdo.get("TargetLPFFreq")),
        "bytecode": consts,
    }))
}

/// `TODM_A_Child_Athena` phases (child over parent `TODM_A`) and the start time.
fn tod_json(set: &PakSet) -> Result<J, String> {
    let mut out = Map::new();
    for (key, path, cdo) in [("child", TODM, "Default__TODM_A_Child_Athena_C"), ("parent", TODM_PARENT, "Default__TODM_A_C")] {
        let d = PackageData::load(set, path).map_err(|e| e.to_string())?;
        let p = d.package().map_err(|e| e.to_string())?;
        let i = p.find_export(cdo).ok_or("no CDO")?;
        let j = p.read_export(i).map_err(|e| e.to_string())?.props.to_json(&p);
        let mut phases = vec![];
        let mut idx = 0;
        loop {
            let k = if idx == 0 { "LightAndFogPhaseSettings".to_string() } else { format!("LightAndFogPhaseSettings[{idx}]") };
            let Some(ph) = j.get(&k) else { break };
            phases.push(json!({"time_phase_begins_h": ph.get("TimePhaseBegins"), "phase_length_h": ph.get("PhaseLengthInHours"), "transition_in_min": ph.get("TransitionInTimeInMinutes"), "transition_out_min": ph.get("TransitionOutTimeInMinutes")}));
            idx += 1;
        }
        out.insert(key.into(), json!({"phases": phases, "start_time_of_day_in_game": j.get("StartTimeOfDayInGame"), "default_time_of_day_speed": j.get("DefaultTimeOfDaySpeed"), "max_time_of_day_accumulation_factor": j.get("MaxTimeOfDayAccumulationFactor")}));
    }
    Ok(J::Object(out))
}
