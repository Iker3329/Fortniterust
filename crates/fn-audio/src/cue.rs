//! `USoundCue` node graphs (UE 4.16) parsed from cooked exports into an owned tree.
//!
//! Every node is an export of the cue's package whose tagged properties hold everything (`ChildNodes` = array of
//! export references, `FirstNode` on the cue). Properties missing from an export take the engine defaults noted at
//! each [`NodeKind`] variant (UE 4.16 constructors; cooked data only stores non-default values).

use crate::attenuation::Attenuation;
use crate::curve::Curve;
use crate::error::{malformed, Result};
use crate::soundclass::{object_name, ClassInfo};
use fn_uasset::{Package, PackageIndex, PropertyList, Value};
use std::sync::Arc;

/// Resolves object references that live in other packages while a cue is parsed.
pub trait CueResolver {
    /// `SoundAttenuation` asset -> its settings.
    fn attenuation(&mut self, object_path: &str) -> Option<Attenuation>;
    /// `SoundConcurrency` asset -> its settings.
    fn concurrency(&mut self, object_path: &str) -> Option<Concurrency>;
}

/// Resolver that knows nothing (assets referenced from other packages stay unresolved). Used by tests.
pub struct NoResolver;
impl CueResolver for NoResolver {
    fn attenuation(&mut self, _: &str) -> Option<Attenuation> {
        None
    }
    fn concurrency(&mut self, _: &str) -> Option<Concurrency> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolutionRule {
    PreventNew,
    StopOldest,
    StopFarthestThenPreventNew,
    StopFarthestThenOldest,
    StopLowestPriority,
    StopQuietest,
    StopLowestPriorityThenPreventNew,
}

/// `FSoundConcurrencySettings` (defaults per UE 4.16: MaxCount 16, StopFarthestThenPreventNew, VolumeScale 1).
#[derive(Clone, Debug, PartialEq)]
pub struct Concurrency {
    /// Name of the shared `SoundConcurrency` asset, or the cue's own name for inline overrides.
    pub group: String,
    pub max_count: u32,
    pub rule: ResolutionRule,
    pub limit_to_owner: bool,
    pub volume_scale: f32,
}

impl Concurrency {
    pub fn from_props(group: &str, p: &PropertyList) -> Concurrency {
        let rule = match p.get("ResolutionRule").and_then(Value::as_str).map(|s| s.rsplit("::").next().unwrap_or(s)) {
            Some("PreventNew") => ResolutionRule::PreventNew,
            Some("StopOldest") => ResolutionRule::StopOldest,
            Some("StopFarthestThenOldest") => ResolutionRule::StopFarthestThenOldest,
            Some("StopLowestPriority") => ResolutionRule::StopLowestPriority,
            Some("StopQuietest") => ResolutionRule::StopQuietest,
            Some("StopLowestPriorityThenPreventNew") => ResolutionRule::StopLowestPriorityThenPreventNew,
            _ => ResolutionRule::StopFarthestThenPreventNew,
        };
        Concurrency {
            group: group.to_string(),
            max_count: p.get("MaxCount").and_then(Value::as_i64).map(|v| v.max(1) as u32).unwrap_or(16),
            rule,
            limit_to_owner: p.get("bLimitToOwner").and_then(Value::as_bool).unwrap_or(false),
            volume_scale: p.get("VolumeScale").and_then(Value::as_f32).unwrap_or(1.0),
        }
    }
}

/// `FDistanceDatum` of the cross-fade nodes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FadeRange {
    pub fade_in_start: f32,
    pub fade_in_end: f32,
    pub fade_out_start: f32,
    pub fade_out_end: f32,
    pub volume: f32,
}

impl FadeRange {
    /// Gain for input value `x` (distance in cm or the parameter value): ramps 0->volume over the fade-in range, holds,
    /// then ramps back to 0 over the fade-out range.
    pub fn gain(&self, x: f32) -> f32 {
        let up = if x >= self.fade_in_end {
            1.0
        } else if x <= self.fade_in_start {
            0.0
        } else {
            (x - self.fade_in_start) / (self.fade_in_end - self.fade_in_start)
        };
        let down = if x <= self.fade_out_start {
            1.0
        } else if x >= self.fade_out_end {
            0.0
        } else {
            1.0 - (x - self.fade_out_start) / (self.fade_out_end - self.fade_out_start)
        };
        self.volume * up * down
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// `SoundNodeWavePlayer`: `SoundWaveAssetPtr` (long package path) + `bLooping`. `None` wave = null pointer.
    WavePlayer { wave: Option<String>, looping: bool },
    /// `SoundNodeDialoguePlayer`: plays the SoundWave of the dialogue wave (resolved lazily by the library).
    Dialogue { dialogue_wave: Option<String> },
    /// `SoundNodeRandom`. Default weights 1.0; `bRandomizeWithoutReplacement` default false.
    Random { weights: Vec<f32>, without_replacement: bool },
    /// `SoundNodeMixer`: plays all children; `InputVolume` default 1.0 each.
    Mixer { input_volume: Vec<f32> },
    /// `SoundNodeConcatenator`: children back to back.
    Concatenator { input_volume: Vec<f32> },
    /// `SoundNodeModulator`; engine defaults Pitch 0.95..1.05, Volume 0.95..1.05 (applied when a property is absent).
    Modulator { pitch: (f32, f32), volume: (f32, f32) },
    /// `SoundNodeModulatorContinuous`: parameter-driven; this crate plays it at the parameter-less value (1.0).
    ModulatorContinuous,
    /// `SoundNodeAttenuation`: distance attenuation for the subtree.
    Attenuation(Option<Arc<Attenuation>>),
    /// `SoundNodeLooping`. Engine defaults `bLoopIndefinitely = true` (LoopCount 1,000,000): all 151 nodes in 1.8.
    Looping { count: u32, indefinite: bool },
    /// `SoundNodeDelay`: random start delay in seconds, `DelayMin/Max` default 0.
    Delay { min: f32, max: f32 },
    /// `SoundNodeSwitch`: child chosen by an integer parameter (`IntParameterName`, e.g. `FS_Surfaces`).
    Switch { param: String },
    /// `SoundNodeBranch`: child 0/1/2 by a bool parameter (not present in 1.8).
    Branch { param: String },
    /// `SoundNodeParamCrossFade`: child gains from a float parameter.
    ParamCrossFade { param: String, inputs: Vec<FadeRange> },
    /// `SoundNodeDistanceCrossFade`: child gains from the listener distance.
    DistanceCrossFade { inputs: Vec<FadeRange> },
    /// `SoundNodeDoppler`: intensity only (no pitch shift is applied).
    Doppler { intensity: f32 },
    /// `SoundNodeEnveloper`: volume/pitch envelopes over time since the node started.
    Enveloper { volume: Curve, pitch: Curve, looping: bool, loop_start: f32, loop_end: f32 },
    /// `SoundNodeGroupControl`: played as a Mixer here (documented simplification).
    GroupControl { sizes: Vec<i32> },
    /// `SoundNodeQualityLevel`: one child per audio quality level.
    QualityLevel,
    /// `SoundNodeSoundClass`: overrides the SoundClass of the subtree.
    SoundClass { class: String, info: Option<ClassInfo> },
    /// Anything else (class name kept); played like a Mixer so no sound is silently lost.
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct CueNode {
    pub name: String,
    pub kind: NodeKind,
    pub children: Vec<CueNode>,
}

impl CueNode {
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a CueNode)) {
        f(self);
        for c in &self.children {
            c.walk(f);
        }
    }
    pub fn class_label(&self) -> &str {
        match &self.kind {
            NodeKind::WavePlayer { .. } => "WavePlayer",
            NodeKind::Dialogue { .. } => "DialoguePlayer",
            NodeKind::Random { .. } => "Random",
            NodeKind::Mixer { .. } => "Mixer",
            NodeKind::Concatenator { .. } => "Concatenator",
            NodeKind::Modulator { .. } => "Modulator",
            NodeKind::ModulatorContinuous => "ModulatorContinuous",
            NodeKind::Attenuation(_) => "Attenuation",
            NodeKind::Looping { .. } => "Looping",
            NodeKind::Delay { .. } => "Delay",
            NodeKind::Switch { .. } => "Switch",
            NodeKind::Branch { .. } => "Branch",
            NodeKind::ParamCrossFade { .. } => "ParamCrossFade",
            NodeKind::DistanceCrossFade { .. } => "DistanceCrossFade",
            NodeKind::Doppler { .. } => "Doppler",
            NodeKind::Enveloper { .. } => "Enveloper",
            NodeKind::GroupControl { .. } => "GroupControl",
            NodeKind::QualityLevel => "QualityLevel",
            NodeKind::SoundClass { .. } => "SoundClass",
            NodeKind::Unknown(c) => c,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SoundCue {
    /// Export name (`PickAxe_Swing_Athena_Cue`).
    pub name: String,
    /// Long package path (`/Game/Athena/Sounds/Weapons/PickAxe_Swing_Athena_Cue`).
    pub path: String,
    pub root: Option<CueNode>,
    pub volume_multiplier: f32,
    pub pitch_multiplier: f32,
    pub priority: f32,
    /// Serialized `Duration` of the cue (seconds).
    pub duration: f32,
    /// `SoundClassObject` object name (resolved to [`ClassInfo`] by the library).
    pub class_name: Option<String>,
    pub class: ClassInfo,
    /// Cue-level attenuation (applies when the graph has no `SoundNodeAttenuation` on the path).
    pub attenuation: Option<Arc<Attenuation>>,
    pub concurrency: Option<Concurrency>,
}

fn f32s(v: Option<&Value>) -> Vec<f32> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f32).collect()).unwrap_or_default()
}

fn fade_ranges(v: Option<&Value>) -> Vec<FadeRange> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|d| {
                    let g = |n: &str, def: f32| d.field(n).and_then(Value::as_f32).unwrap_or(def);
                    FadeRange { fade_in_start: g("FadeInDistanceStart", 0.0), fade_in_end: g("FadeInDistanceEnd", 0.0), fade_out_start: g("FadeOutDistanceStart", 0.0), fade_out_end: g("FadeOutDistanceEnd", 0.0), volume: g("Volume", 1.0) }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn soft_or_object_path(pkg: &Package, v: Option<&Value>) -> Option<String> {
    match v? {
        Value::SoftObject(s) if !s.is_empty() => Some(s.clone()),
        Value::Object(i) if !i.is_null() => Some(pkg.object_path(*i)),
        _ => None,
    }
}

/// Strip the `.ObjectName` suffix: `/Game/A/B.B` -> `/Game/A/B`.
pub fn long_package(object_path: &str) -> &str {
    object_path.split('.').next().unwrap_or(object_path)
}

struct Builder<'a, 'p> {
    pkg: &'a Package<'p>,
    resolver: &'a mut dyn CueResolver,
    active: Vec<usize>,
    nodes_built: usize,
}

impl Builder<'_, '_> {
    fn attenuation_of(&mut self, props: &PropertyList) -> Option<Arc<Attenuation>> {
        let over = props.get("bOverrideAttenuation").and_then(Value::as_bool).unwrap_or(false);
        if over {
            if let Some(Value::Struct { props: p, .. }) = props.get("AttenuationOverrides") {
                return Some(Arc::new(Attenuation::from_props(p)));
            }
        }
        let obj = props.get("AttenuationSettings").and_then(Value::as_object).filter(|i| !i.is_null())?;
        let path = self.pkg.object_path(obj);
        self.resolver.attenuation(&path).map(Arc::new)
    }

    fn build(&mut self, idx: usize, depth: usize) -> Result<Option<CueNode>> {
        if depth > 64 || self.active.contains(&idx) || self.nodes_built > 20_000 {
            return Ok(None); // cycle / runaway guard
        }
        if idx >= self.pkg.exports.len() {
            return malformed("SoundCue node", format!("export {idx} out of range"));
        }
        self.nodes_built += 1;
        self.active.push(idx);
        let class = self.pkg.export_class_name(idx);
        let o = self.pkg.read_export(idx)?;
        let p = &o.props;
        let f = |n: &str, d: f32| p.get(n).and_then(Value::as_f32).unwrap_or(d);
        let kind = match class.as_str() {
            "SoundNodeWavePlayer" => NodeKind::WavePlayer { wave: soft_or_object_path(self.pkg, p.get("SoundWaveAssetPtr")).map(|s| long_package(&s).to_string()), looping: p.get("bLooping").and_then(Value::as_bool).unwrap_or(false) },
            "SoundNodeDialoguePlayer" => NodeKind::Dialogue {
                dialogue_wave: p.get("DialogueWaveParameter").and_then(|d| d.field("DialogueWave")).and_then(|v| soft_or_object_path(self.pkg, Some(v))).map(|s| long_package(&s).to_string()),
            },
            "SoundNodeRandom" => NodeKind::Random { weights: f32s(p.get("Weights")), without_replacement: p.get("bRandomizeWithoutReplacement").and_then(Value::as_bool).unwrap_or(false) },
            "SoundNodeMixer" => NodeKind::Mixer { input_volume: f32s(p.get("InputVolume")) },
            "SoundNodeConcatenator" => NodeKind::Concatenator { input_volume: f32s(p.get("InputVolume")) },
            "SoundNodeModulator" => NodeKind::Modulator { pitch: (f("PitchMin", 0.95), f("PitchMax", 1.05)), volume: (f("VolumeMin", 0.95), f("VolumeMax", 1.05)) },
            "SoundNodeModulatorContinuous" => NodeKind::ModulatorContinuous,
            "SoundNodeAttenuation" => NodeKind::Attenuation(self.attenuation_of(p)),
            "SoundNodeLooping" => NodeKind::Looping { count: p.get("LoopCount").and_then(Value::as_i64).map(|v| v.max(0) as u32).unwrap_or(1_000_000), indefinite: p.get("bLoopIndefinitely").and_then(Value::as_bool).unwrap_or(true) },
            "SoundNodeDelay" => NodeKind::Delay { min: f("DelayMin", 0.0), max: f("DelayMax", 0.0) },
            "SoundNodeSwitch" => NodeKind::Switch { param: p.get("IntParameterName").and_then(Value::as_str).unwrap_or("").to_string() },
            "SoundNodeBranch" => NodeKind::Branch { param: p.get("BoolParameterName").and_then(Value::as_str).unwrap_or("").to_string() },
            "SoundNodeParamCrossFade" => NodeKind::ParamCrossFade { param: p.get("ParamName").and_then(Value::as_str).unwrap_or("").to_string(), inputs: fade_ranges(p.get("CrossFadeInput")) },
            "SoundNodeDistanceCrossFade" => NodeKind::DistanceCrossFade { inputs: fade_ranges(p.get("CrossFadeInput")) },
            "SoundNodeDoppler" => NodeKind::Doppler { intensity: f("DopplerIntensity", 1.0) },
            "SoundNodeEnveloper" => NodeKind::Enveloper {
                volume: Curve::from_runtime_curve(p.get("VolumeCurve")),
                pitch: Curve::from_runtime_curve(p.get("PitchCurve")),
                looping: p.get("bLoop").and_then(Value::as_bool).unwrap_or(false),
                loop_start: f("LoopStart", 0.0),
                loop_end: f("LoopEnd", 0.0),
            },
            "SoundNodeGroupControl" => NodeKind::GroupControl { sizes: p.get("GroupSizes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).map(|v| v as i32).collect()).unwrap_or_default() },
            "SoundNodeQualityLevel" => NodeKind::QualityLevel,
            "SoundNodeSoundClass" => NodeKind::SoundClass { class: p.get("SoundClassOverride").and_then(Value::as_object).filter(|i| !i.is_null()).map(|i| object_name(&self.pkg.object_path(i)).to_string()).unwrap_or_default(), info: None },
            other => NodeKind::Unknown(other.to_string()),
        };
        let mut children = Vec::new();
        if let Some(arr) = p.get("ChildNodes").and_then(Value::as_array) {
            for c in arr {
                let Some(pi) = c.as_object() else { continue };
                if let Some(ci) = export_of(pi) {
                    if let Some(n) = self.build(ci, depth + 1)? {
                        children.push(n);
                    }
                }
            }
        }
        self.active.pop();
        Ok(Some(CueNode { name: o.name.clone(), kind, children }))
    }
}

fn export_of(pi: PackageIndex) -> Option<usize> {
    pi.export_index()
}

/// Parse the (first) `SoundCue` export of `pkg` with its node graph.
pub fn parse_cue(pkg: &Package, resolver: &mut dyn CueResolver) -> Result<SoundCue> {
    let ci = match pkg.exports_of_class("SoundCue").next() {
        Some(i) => i,
        None => return malformed("SoundCue", "package has no SoundCue export"),
    };
    let obj = pkg.read_export(ci)?;
    if let Some(e) = &obj.error {
        return malformed("SoundCue properties", e.to_string());
    }
    let p = &obj.props;
    let mut b = Builder { pkg, resolver, active: Vec::new(), nodes_built: 0 };
    let root = match p.get("FirstNode").and_then(Value::as_object).and_then(export_of) {
        Some(i) => b.build(i, 0)?,
        None => None,
    };
    let attenuation = b.attenuation_of(p);
    // Concurrency: inline override wins, otherwise the shared asset.
    let concurrency = if p.get("bOverrideConcurrency").and_then(Value::as_bool).unwrap_or(false) {
        match p.get("ConcurrencyOverrides") {
            Some(Value::Struct { props, .. }) => Some(Concurrency::from_props(&obj.name, props)),
            _ => None,
        }
    } else {
        p.get("SoundConcurrencySettings").and_then(Value::as_object).filter(|i| !i.is_null()).and_then(|i| b.resolver.concurrency(&pkg.object_path(i)))
    };
    let f = |n: &str, d: f32| p.get(n).and_then(Value::as_f32).unwrap_or(d);
    Ok(SoundCue {
        name: obj.name.clone(),
        path: pkg.name.clone(),
        root,
        volume_multiplier: f("VolumeMultiplier", 1.0),
        pitch_multiplier: f("PitchMultiplier", 1.0),
        priority: f("Priority", 1.0),
        duration: f("Duration", 0.0),
        class_name: p.get("SoundClassObject").and_then(Value::as_object).filter(|i| !i.is_null()).map(|i| object_name(&pkg.object_path(i)).to_string()),
        class: ClassInfo::default(),
        attenuation,
        concurrency,
    })
}

impl SoundCue {
    /// All wave paths referenced by the graph.
    pub fn wave_paths(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(r) = &self.root {
            r.walk(&mut |n| {
                if let NodeKind::WavePlayer { wave: Some(w), .. } = &n.kind {
                    if !v.contains(w) {
                        v.push(w.clone());
                    }
                }
            });
        }
        v
    }

    /// Indented human-readable dump of the graph (CLI `cue`).
    pub fn dump(&self) -> String {
        fn rec(n: &CueNode, d: usize, out: &mut String) {
            let pad = "  ".repeat(d);
            let detail = match &n.kind {
                NodeKind::WavePlayer { wave, looping } => format!(" {}{}", wave.as_deref().unwrap_or("<null>"), if *looping { " (looping)" } else { "" }),
                NodeKind::Random { weights, without_replacement } => format!(" weights={weights:?}{}", if *without_replacement { " without-replacement" } else { "" }),
                NodeKind::Mixer { input_volume } | NodeKind::Concatenator { input_volume } => format!(" input_volume={input_volume:?}"),
                NodeKind::Modulator { pitch, volume } => format!(" pitch={:.2}..{:.2} volume={:.2}..{:.2}", pitch.0, pitch.1, volume.0, volume.1),
                NodeKind::Attenuation(a) => match a {
                    Some(a) => format!(" {:?} inner={} falloff={} omni={} spread={} lpf={} spatialize={}", a.algorithm, a.inner_radius(), a.falloff_distance, a.omni_radius, a.stereo_spread, a.lpf_enabled, a.spatialize),
                    None => " <unresolved>".into(),
                },
                NodeKind::Looping { count, indefinite } => format!(" count={count} indefinite={indefinite}"),
                NodeKind::Delay { min, max } => format!(" {min}..{max}s"),
                NodeKind::Switch { param } | NodeKind::Branch { param } | NodeKind::ParamCrossFade { param, .. } => format!(" param={param}"),
                NodeKind::SoundClass { class, .. } => format!(" class={class}"),
                NodeKind::Dialogue { dialogue_wave } => format!(" {}", dialogue_wave.as_deref().unwrap_or("<null>")),
                _ => String::new(),
            };
            out.push_str(&format!("{pad}{} [{}]{detail}\n", n.class_label(), n.name));
            for c in &n.children {
                rec(c, d + 1, out);
            }
        }
        let mut s = format!(
            "{} ({})\n  class {:?} (bus {:?}, base volume {:.3}), VolumeMultiplier {}, PitchMultiplier {}, Duration {:.3}s, Priority {}\n",
            self.name, self.path, self.class_name, self.class.bus, self.class.base_volume, self.volume_multiplier, self.pitch_multiplier, self.duration, self.priority
        );
        if let Some(a) = &self.attenuation {
            s.push_str(&format!("  cue attenuation: {:?} inner={} falloff={}\n", a.algorithm, a.inner_radius(), a.falloff_distance));
        }
        if let Some(c) = &self.concurrency {
            s.push_str(&format!("  concurrency: {c:?}\n"));
        }
        if let Some(r) = &self.root {
            rec(r, 1, &mut s);
        }
        s
    }
}

