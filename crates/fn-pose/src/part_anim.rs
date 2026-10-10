//! PART ANIM: the Battle Royale character parts' own AnimBlueprints (`CustomCharacterPart` -> `AdditionalData.AnimClass`), decoded from the 1.8
//! packages and evaluated on the part skeletons.
//!
//! What the data says (verified on the default hero, `HID_005_Athena_Commando_M`):
//! * BODY `SK_M_Med_Soldier_04_Skeleton_AnimBP`: `CopyPoseFromMesh` (the master) -> `ConvertLocalToComponentSpace` -> nine chained
//!   `AnimDynamics` nodes (`dyn_tags`, `dyn_scarf_*` x4 (two nodes bind `dyn_scarf_r`: authored twice in the data), `dyn_pouch_r`, `dyn_grenade02`,
//!   `dyn_grenade01`, `dyn_armband_knot`) -> `ConvertComponentToLocalSpace` -> Root. The four scarf nodes' `Alpha` pin is
//!   `1 * MapRangeClamped(PawnSpeed2D, 0, 100, 0.1, 0.33)` (the AnimBP's own bytecode, run by [`ExposedInputs`]).
//! * HEAD `SK_M_MED_Head_01_Skeleton_AnimBP`: `BlendListByBool(bCanPlayCustomAnimations)` between two `LayeredBoneBlend`s that both take the master's
//!   pose (`CopyPoseFromMesh`) and replace everything under `faceAttach` (the 27-bone face rig) with either the idle sequence
//!   `Commando_Male_Head_01_Idle` (looping) or the `DefaultSlot` montage slot (no source pin: face rig at its reference pose without a montage).
//!   `BlendListByBool` picks pose 0 for TRUE, pose 1 for FALSE (engine rule).
//!
//! `CopyPoseFromMesh` (engine): every part bone that exists in the master gets the master's LOCAL transform (`ChildCS.GetRelativeTransform(ParentCS)`
//! of the master component-space pose), every other bone its reference pose. [`PartAnimRig`] therefore evaluates the pose tree ONCE on the master
//! skeleton (TwoBoneIK / HeadTrack / retarget on the master's own bind, like the original) and copies it by bone name, instead of evaluating the tree on each
//! part's own bind pose.
use crate::br_character::PartKind;
use crate::character::{CharacterPose, CharacterRig, PoseResult};
use crate::clip::{ClipSourceDir, FnClip, SampleMode};
use crate::eval::PoseEvaluator;
use crate::loader;
use crate::part::Part;
use crate::skel::{BoneTree, BoneXform, Pose};
use fn_animgraph::anim_dynamics::{from_ue, to_ue, AngularConstraintType, AnimDynamicsParams, AnimDynamicsSim, ConstraintSetup, LinearLimitType, SolverAssumptions};
use fn_animgraph::{ClipLibrary, ClipSource, Node};
use fn_uasset::kismet::{self, Expr};
use fn_uasset::{Package, Value};
use glam::{Quat, Vec3};
use std::collections::HashMap;

/// Values the part AnimInstance (`CustomCharacterPartAnimInstance`, native) feeds the graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartAnimInputs {
    /// `PawnSpeed2D` (cm/s): the player's 2D speed (`AnimState.vars.speed2d`).
    pub pawn_speed_2d: f32,
    /// `bCanPlayCustomAnimations`: a native bool whose default is not in the packages (C++ ctor / pawn code). `false` = zero-initialised object.
    pub can_play_custom_animations: bool,
}
impl Default for PartAnimInputs { fn default() -> Self { PartAnimInputs { pawn_speed_2d: 0., can_play_custom_animations: false } } }
impl PartAnimInputs {
    fn lookup(&self, name: &str) -> Option<f32> {
        match name.to_ascii_lowercase().as_str() { "pawnspeed2d" => Some(self.pawn_speed_2d), "bcanplaycustomanimations" => Some(self.can_play_custom_animations as u8 as f32), _ => None }
    }
}

/// One decoded AnimGraph node (indices are the AnimBP's node link ids).
#[derive(Clone, Debug)]
pub enum PNode {
    Root { result: i32 },
    CopyPoseFromMesh,
    ComponentToLocal { input: i32 },
    LocalToComponent { input: i32 },
    Dynamics { params: AnimDynamicsParams, input: i32 },
    BlendListByBool { poses: Vec<i32>, blend_times: Vec<f32>, active_default: bool },
    LayeredBoneBlend { base: i32, blends: Vec<i32>, weights: Vec<f32>, filters: Vec<Vec<(String, i32)>>, mesh_space_rotation: bool },
    Slot { slot: String, source: i32 },
    SequencePlayer { sequence: String, play_rate: f32, start: f32, looping: bool },
    Other(String),
}

/// One exposed-input program: the bytecode entry offset in the ubergraph, run before the node updates.
#[derive(Clone, Debug)]
struct Pin { #[allow(dead_code)] node: String, offset: u32 }

/// Pointer operands of an expression (4 bytes serialized, 8 in memory): the ubergraph entry points `ExecuteUbergraph(N)` are IN-MEMORY script offsets.
fn ref_count(e: &Expr) -> u32 {
    use Expr::*;
    let own = match e {
        LocalVariable(_) | InstanceVariable(_) | DefaultVariable(_) | LocalOutVariable(_) | Let { .. } | LetValueOnPersistentFrame { .. } | ClassContext { .. } | Context { .. } | ContextFailSilent { .. }
        | MetaCast { .. } | DynamicCast { .. } | ObjToInterfaceCast { .. } | CrossInterfaceCast { .. } | InterfaceToObjCast { .. } | FinalFunction { .. } | LocalFinalFunction { .. } | CallMath { .. }
        | CallMulticastDelegate { .. } | ObjectConst(_) | PropertyConst(_) | StructConst { .. } | ArrayConst { .. } | StructMemberContext { .. } => 1,
        _ => 0,
    };
    own + e.children().iter().map(|c| ref_count(c)).sum::<u32>()
}

/// Evaluator of the AnimBP's exposed-input bytecode (`EvaluateGraphExposedInputs_*` -> `ExecuteUbergraph(offset)`): straight-line `let` statements over
/// float / bool math (`MapRangeClamped`, `Multiply_FloatFloat`, ...), the native inputs by name, and writes into node struct members (`Alpha`,
/// `bActiveValue`). Anything else is reported in `ExposedInputs::unsupported`, never guessed.
#[derive(Clone, Debug, Default)]
pub struct ExposedInputs {
    statements: Vec<kismet::Statement>,
    /// In-memory script offset of every statement (`serialized offset + 4 * pointer operands before it`).
    mem_offsets: Vec<u32>,
    pins: Vec<Pin>,
    pub unsupported: Vec<String>,
}

fn name_of(e: &Expr) -> Option<&str> { match e { Expr::LocalVariable(r) | Expr::InstanceVariable(r) | Expr::DefaultVariable(r) | Expr::LocalOutVariable(r) => Some(&r.name), _ => None } }

impl ExposedInputs {
    fn eval(&self, e: &Expr, locals: &HashMap<String, f32>, inputs: &PartAnimInputs, bad: &mut Vec<String>) -> Option<f32> {
        Some(match e {
            Expr::FloatConst(f) => *f,
            Expr::IntConst(i) => *i as f32,
            Expr::IntConstByte(b) | Expr::ByteConst(b) => *b as f32,
            Expr::IntZero => 0., Expr::IntOne => 1., Expr::True => 1., Expr::False => 0.,
            Expr::LocalVariable(r) | Expr::LocalOutVariable(r) => *locals.get(&r.name)?,
            Expr::InstanceVariable(r) | Expr::DefaultVariable(r) => match inputs.lookup(&r.name) { Some(v) => v, None => { bad.push(format!("native input {}", r.name)); return None } },
            Expr::PrimitiveCast { expr, .. } => self.eval(expr, locals, inputs, bad)?,
            Expr::CallMath { func, args } => {
                let a: Option<Vec<f32>> = args.iter().map(|x| self.eval(x, locals, inputs, bad)).collect();
                let a = a?;
                match (func.name.as_str(), a.as_slice()) {
                    ("Multiply_FloatFloat", [x, y]) => x * y,
                    ("Add_FloatFloat", [x, y]) => x + y,
                    ("Subtract_FloatFloat", [x, y]) => x - y,
                    ("Divide_FloatFloat", [x, y]) => if *y != 0. { x / y } else { 0. },
                    ("MapRangeClamped", [v, ia, ib, oa, ob]) => fn_animgraph::map_range_clamped(*v, *ia, *ib, *oa, *ob),
                    ("FClamp", [v, lo, hi]) => v.clamp(*lo, *hi),
                    ("Not_PreBool", [x]) => (*x == 0.) as u8 as f32,
                    (n, _) => { bad.push(format!("math function {n}")); return None }
                }
            }
            other => { bad.push(format!("expression {}", other.render())); return None }
        })
    }

    /// Runs every pin program: `(node name, member, value)` triples.
    pub fn run(&self, inputs: &PartAnimInputs) -> Vec<(String, String, f32)> {
        let mut out = vec![];
        let mut bad = vec![];
        for pin in &self.pins {
            let mut locals = HashMap::new();
            let Some(start) = self.mem_offsets.iter().position(|o| *o == pin.offset) else { continue };
            for s in &self.statements[start..] {
                match &s.expr {
                    Expr::Let { var, value, .. } | Expr::LetBool { var, value } => {
                        let target = match &**var { Expr::StructMemberContext { prop, expr } => name_of(expr).map(|n| (n.to_owned(), prop.name.clone())), Expr::LocalVariable(r) => { if let Some(v) = self.eval(value, &locals, inputs, &mut bad) { locals.insert(r.name.clone(), v); } continue } _ => None };
                        if let Some((node, member)) = target { if let Some(v) = self.eval(value, &locals, inputs, &mut bad) { out.push((node, member, v)); } }
                    }
                    Expr::Jump(_) | Expr::Return(_) | Expr::ComputedJump(_) | Expr::EndOfScript => break,
                    _ => {}
                }
            }
        }
        out
    }
}

/// The decoded part AnimBP.
#[derive(Clone, Debug)]
pub struct PartAnimGraph {
    pub package: String,
    pub target_skeleton: Option<String>,
    /// Link id -> node. `nodes[root]` is `AnimNode_Root`.
    pub nodes: Vec<PNode>,
    pub node_names: Vec<String>,
    pub root: usize,
    exposed: ExposedInputs,
    /// Properties of the part AnimInstance the bytecode reads / the data lists that this port cannot drive.
    pub notes: Vec<String>,
}

fn f32s<const N: usize>(v: Option<&Value>) -> Option<[f32; N]> {
    let a = v?.as_array()?;
    if a.len() != N { return None }
    let mut o = [0f32; N];
    for i in 0..N { o[i] = a[i].as_f32()?; }
    Some(o)
}
fn vec3_of(v: Option<&Value>) -> Option<[f32; 3]> { match v? { Value::Vector(x) => Some(*x), other => f32s::<3>(Some(other)) } }
fn axis_of(s: &str) -> u8 { if s.ends_with("AxisX") { 0 } else if s.ends_with("AxisY") { 1 } else { 2 } }
fn link_of(v: Option<&Value>) -> i32 { v.and_then(|v| v.field("LinkID")).and_then(|v| v.as_i64()).map_or(-1, |i| i as i32) }
fn bone_ref(v: Option<&Value>) -> Option<String> { let n = v?.field("BoneName")?.as_str()?; if n == "None" { None } else { Some(n.to_lowercase()) } }

fn decode_dynamics(s: &fn_uasset::PropertyList) -> AnimDynamicsParams {
    let mut p = AnimDynamicsParams::default();
    let f = |n: &str, d: f32| s.get(n).and_then(|v| v.as_f32()).unwrap_or(d);
    let b = |n: &str, d: bool| s.get(n).and_then(|v| v.as_bool()).unwrap_or(d);
    p.bound_bone = bone_ref(s.get("BoundBone")).unwrap_or_default();
    p.chain = b("bChain", p.chain);
    p.chain_end = bone_ref(s.get("ChainEnd"));
    p.box_extents = vec3_of(s.get("BoxExtents")).unwrap_or(p.box_extents);
    p.local_joint_offset = vec3_of(s.get("LocalJointOffset")).unwrap_or(p.local_joint_offset);
    p.gravity_scale = f("GravityScale", p.gravity_scale);
    p.linear_spring = b("bLinearSpring", false);
    p.linear_spring_constant = f("LinearSpringConstant", 0.);
    p.angular_spring = b("bAngularSpring", false);
    p.angular_spring_constant = f("AngularSpringConstant", 0.);
    p.pre_iterations = s.get("NumSolverIterationsPreUpdate").and_then(|v| v.as_i64()).map_or(p.pre_iterations, |v| v.max(0) as u32);
    p.post_iterations = s.get("NumSolverIterationsPostUpdate").and_then(|v| v.as_i64()).map_or(p.post_iterations, |v| v.max(0) as u32);
    p.use_planar_limit = b("bUsePlanarLimit", p.use_planar_limit);
    p.planar_limit_count = s.get("PlanarLimits").and_then(|v| v.as_array()).map_or(0, |a| a.len());
    p.use_spherical_limits = b("bUseSphericalLimits", false);
    p.spherical_limit_count = s.get("SphericalLimits").and_then(|v| v.as_array()).map_or(0, |a| a.len());
    p.enable_wind = b("bEnableWind", false);
    p.wind_scale = f("WindScale", 1.);
    p.override_linear_damping = b("bOverrideLinearDamping", false);
    p.linear_damping_override = f("LinearDampingOverride", 0.);
    p.override_angular_damping = b("bOverrideAngularDamping", false);
    p.angular_damping_override = f("AngularDampingOverride", 0.);
    p.external_force = vec3_of(s.get("ExternalForce")).unwrap_or([0.; 3]);
    p.simulation_space_component = s.get("SimulationSpace").and_then(|v| v.as_str()).map_or(true, |v| v.ends_with("Component"));
    p.relative_space_bone = bone_ref(s.get("RelativeSpaceBone"));
    p.lod_threshold = s.get("LODThreshold").and_then(|v| v.as_i64()).map_or(-1, |v| v as i32);
    p.alpha = f("Alpha", 1.);
    if let Some(sb) = s.get("AlphaScaleBias") { p.alpha_scale = sb.field("Scale").and_then(|v| v.as_f32()).unwrap_or(1.); p.alpha_bias = sb.field("Bias").and_then(|v| v.as_f32()).unwrap_or(0.); }
    p.do_update = b("bDoUpdate", true);
    p.do_eval = b("bDoEval", true);
    if let Some(c) = s.get("ConstraintSetup").and_then(|v| v.as_struct()) {
        let mut k = ConstraintSetup::default();
        k.angular_type = if c.get("AngularConstraintType").and_then(|v| v.as_str()).is_some_and(|s| s.ends_with("Cone")) { AngularConstraintType::Cone } else { AngularConstraintType::Angular };
        k.twist_axis = c.get("TwistAxis").and_then(|v| v.as_str()).map_or(k.twist_axis, axis_of);
        k.angular_min = vec3_of(c.get("AngularLimitsMin")).unwrap_or([0.; 3]);
        k.angular_max = vec3_of(c.get("AngularLimitsMax")).unwrap_or([0.; 3]);
        k.angular_target = vec3_of(c.get("AngularTarget")).unwrap_or([0.; 3]);
        k.angular_target_axis = c.get("AngularTargetAxis").and_then(|v| v.as_str()).map_or(k.angular_target_axis, axis_of);
        k.cone_angle = c.get("ConeAngle").and_then(|v| v.as_f32()).unwrap_or(0.);
        k.linear_min = vec3_of(c.get("LinearAxesMin")).unwrap_or([0.; 3]);
        k.linear_max = vec3_of(c.get("LinearAxesMax")).unwrap_or([0.; 3]);
        k.linear_fully_locked = c.get("bLinearFullyLocked").and_then(|v| v.as_bool()).unwrap_or(false);
        for (i, n) in ["LinearXLimitType", "LinearYLimitType", "LinearZLimitType"].iter().enumerate() {
            k.linear_types[i] = match c.get(n).and_then(|v| v.as_str()) { Some(t) if t.ends_with("Free") => LinearLimitType::Free, Some(t) if t.ends_with("Locked") => LinearLimitType::Locked, _ => LinearLimitType::Limited };
        }
        p.constraint = k;
    }
    p
}

/// Raw `.uasset` + `.uexp` of a package.
fn raw_package(src: &ClipSourceDir, long: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    match src {
        ClipSourceDir::Pak(set) => {
            let a = set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned();
            let x = set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default();
            Ok((a, x))
        }
        ClipSourceDir::RawPackages(root) => {
            let base = root.join("FortniteGame/Content").join(long.trim_start_matches("/Game/"));
            let a = std::fs::read(format!("{}.uasset", base.display())).map_err(|e| format!("{}: {e}", base.display()))?;
            Ok((a, std::fs::read(format!("{}.uexp", base.display())).unwrap_or_default()))
        }
        ClipSourceDir::FnAnimDir(_) => Err("a .fnanim directory has no AnimBlueprints".into()),
    }
}
fn long_of(path: &str) -> &str { path.split_once('.').map_or(path, |s| s.0) }

impl PartAnimGraph {
    /// Decodes the AnimBP package of a part's `AnimClass` (`/Game/..Pkg.Name_C`).
    pub fn load(src: &ClipSourceDir, anim_class: &str) -> Result<PartAnimGraph, String> {
        let long = long_of(anim_class);
        let (a, x) = raw_package(src, long)?;
        let pkg = Package::parse(&a, &x).map_err(|e| format!("{long}: {e}"))?;
        let cls = pkg.exports_of_class("AnimBlueprintGeneratedClass").next().ok_or_else(|| format!("{long}: no AnimBlueprintGeneratedClass"))?;
        let cls_obj = pkg.read_export(cls).map_err(|e| format!("{long}: {e}"))?;
        let target_skeleton = cls_obj.get("TargetSkeleton").and_then(|v| v.as_object()).map(|i| pkg.object_path(i));
        let cdo = pkg.read_default_object().ok_or_else(|| format!("{long}: no class default object"))?.map_err(|e| format!("{long}: {e}"))?;
        // node link ids = reversed declaration order of the AnimGraphNode_* struct properties (verified against the link topology of both BR part AnimBPs)
        let mut names: Vec<String> = (0..pkg.exports.len()).filter(|&i| pkg.export_class_name(i) == "StructProperty" && pkg.exports[i].name.starts_with("AnimGraphNode_")).map(|i| pkg.exports[i].name.clone()).collect();
        names.reverse();
        let mut nodes = vec![];
        let mut notes = vec![];
        for n in &names {
            let Some(Value::Struct { name: st, props }) = cdo.get(n) else { nodes.push(PNode::Other(format!("{n}: no serialized defaults"))); continue };
            let int_list = |k: &str| props.get(k).and_then(|v| v.as_array()).map(|a| a.iter().map(|x| link_of(Some(x))).collect::<Vec<_>>()).unwrap_or_default();
            nodes.push(match st.as_str() {
                "AnimNode_Root" => PNode::Root { result: link_of(props.get("Result")) },
                "AnimNode_CopyPoseFromMesh" => PNode::CopyPoseFromMesh,
                "AnimNode_ConvertComponentToLocalSpace" => PNode::ComponentToLocal { input: link_of(props.get("ComponentPose")) },
                "AnimNode_ConvertLocalToComponentSpace" => PNode::LocalToComponent { input: link_of(props.get("LocalPose")) },
                "AnimNode_AnimDynamics" => PNode::Dynamics { params: decode_dynamics(props), input: link_of(props.get("ComponentPose")) },
                "AnimNode_BlendListByBool" => PNode::BlendListByBool {
                    poses: int_list("BlendPose"),
                    blend_times: props.get("BlendTime").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_f32()).collect()).unwrap_or_default(),
                    active_default: props.get("bActiveValue").and_then(|v| v.as_bool()).unwrap_or(false),
                },
                "AnimNode_LayeredBoneBlend" => PNode::LayeredBoneBlend {
                    base: link_of(props.get("BasePose")),
                    blends: int_list("BlendPoses"),
                    weights: props.get("BlendWeights").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_f32()).collect()).unwrap_or_default(),
                    filters: props.get("LayerSetup").and_then(|v| v.as_array()).map(|a| a.iter().map(|l| l.field("BranchFilters").and_then(|v| v.as_array()).map(|f| f.iter().filter_map(|b| Some((b.field("BoneName")?.as_str()?.to_lowercase(), b.field("BlendDepth").and_then(|d| d.as_i64()).unwrap_or(0) as i32))).collect()).unwrap_or_default()).collect()).unwrap_or_default(),
                    mesh_space_rotation: props.get("bMeshSpaceRotationBlend").and_then(|v| v.as_bool()).unwrap_or(false),
                },
                "AnimNode_Slot" => PNode::Slot { slot: props.get("SlotName").and_then(|v| v.as_str()).unwrap_or("DefaultSlot").to_owned(), source: link_of(props.get("Source")) },
                "AnimNode_SequencePlayer" => PNode::SequencePlayer {
                    sequence: props.get("Sequence").and_then(|v| v.as_object()).map(|i| pkg.object_path(i)).unwrap_or_default(),
                    play_rate: props.get("PlayRate").and_then(|v| v.as_f32()).unwrap_or(1.),
                    start: props.get("StartPosition").and_then(|v| v.as_f32()).unwrap_or(0.),
                    looping: props.get("bLoopAnimation").and_then(|v| v.as_bool()).unwrap_or(true),
                },
                other => { notes.push(format!("{n}: node type {other} not decoded")); PNode::Other(other.to_owned()) }
            });
        }
        let root = nodes.iter().position(|n| matches!(n, PNode::Root { .. })).ok_or_else(|| format!("{long}: no Root node"))?;
        // exposed inputs: EvaluateGraphExposedInputs_ExecuteUbergraph_<Bp>_<Node> -> ExecuteUbergraph(offset)
        let mut ex = ExposedInputs::default();
        if let Some(ug) = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "Function" && pkg.exports[i].name.starts_with("ExecuteUbergraph_")) {
            let f = fn_uasset::classes::read_function(&pkg, ug).map_err(|e| format!("{long}: ubergraph: {e}"))?;
            ex.statements = kismet::parse(&pkg, f.script).map_err(|e| format!("{long}: ubergraph bytecode: {e}"))?;
            let mut shift = 0u32;
            for s in &ex.statements { ex.mem_offsets.push(s.offset as u32 + shift); shift += 4 * ref_count(&s.expr); }
            for i in 0..pkg.exports.len() {
                if pkg.export_class_name(i) != "Function" { continue }
                let fname = &pkg.exports[i].name;
                let Some(node) = fname.strip_prefix("EvaluateGraphExposedInputs_").and_then(|r| r.find("AnimGraphNode_").map(|k| r[k..].to_owned())) else { continue };
                let fd = fn_uasset::classes::read_function(&pkg, i).map_err(|e| format!("{long}: {fname}: {e}"))?;
                let st = kismet::parse(&pkg, fd.script).map_err(|e| format!("{long}: {fname}: {e}"))?;
                let off = st.iter().find_map(|s| match &s.expr {
                    Expr::LocalVirtualFunction { args, .. } | Expr::VirtualFunction { args, .. } | Expr::LocalFinalFunction { args, .. } | Expr::FinalFunction { args, .. } => match args.first() { Some(Expr::IntConst(v)) => Some(*v as u32), Some(Expr::IntConstByte(v)) => Some(*v as u32), Some(Expr::IntZero) => Some(0), Some(Expr::IntOne) => Some(1), _ => None },
                    _ => None,
                });
                match off {
                    Some(o) if ex.mem_offsets.contains(&o) => ex.pins.push(Pin { node, offset: o }),
                    Some(o) => notes.push(format!("{fname}: entry offset {o} is not a statement start (memory-offset model)")),
                    None => notes.push(format!("{fname}: entry offset not decoded")),
                }
            }
        }
        Ok(PartAnimGraph { package: long.to_owned(), target_skeleton, nodes, node_names: names, root, exposed: ex, notes })
    }

    /// Dynamics nodes in evaluation (pose-flow) order with their bone.
    pub fn dynamics_chain(&self) -> Vec<(usize, &AnimDynamicsParams)> {
        let mut order = vec![];
        let mut cur = match &self.nodes[self.root] { PNode::Root { result } => *result, _ => -1 };
        let mut guard = 0;
        while cur >= 0 && (cur as usize) < self.nodes.len() && guard < 64 {
            guard += 1;
            match &self.nodes[cur as usize] {
                PNode::Dynamics { params, input } => { order.push((cur as usize, params)); cur = *input }
                PNode::ComponentToLocal { input } | PNode::LocalToComponent { input } => cur = *input,
                _ => break,
            }
        }
        order.reverse();
        order
    }
    pub fn unsupported(&self) -> Vec<String> {
        let mut u = self.exposed.unsupported.clone();
        for n in &self.nodes { if let PNode::Dynamics { params, .. } = n { for x in params.unsupported() { u.push(format!("{}: {x}", params.bound_bone)); } } }
        u
    }
}

/// Runtime of one part's graph.
pub struct PartAnim {
    pub graph: PartAnimGraph,
    pub kind: PartKind,
    pub assumptions: SolverAssumptions,
    sims: HashMap<usize, AnimDynamicsSim>,
    /// Link id -> (time seconds) of every SequencePlayer.
    seq_time: HashMap<usize, f32>,
    seq_clip: HashMap<usize, FnClip>,
    /// BlendListByBool weights per node (active child 1.0).
    blend_w: HashMap<usize, Vec<f32>>,
    mask: HashMap<usize, Vec<Vec<f32>>>,
    /// Last exposed-input values `(node name, member) -> value`.
    pub pins: Vec<(String, String, f32)>,
    pub missing: Vec<String>,
}

pub struct EvalCtx<'a> { pub tree: &'a BoneTree, pub master_local: &'a [BoneXform], pub master_map: &'a [Option<usize>], pub dt: f32, pub inputs: &'a PartAnimInputs }

impl PartAnim {
    /// Loads the graph and the animation assets it references (sequence players) for the part skeleton `part`.
    pub fn load(src: &ClipSourceDir, anim_class: &str, kind: PartKind) -> Result<PartAnim, String> {
        let graph = PartAnimGraph::load(src, anim_class)?;
        let mut seq_clip = HashMap::new();
        let mut missing = vec![];
        if let Some(skel_path) = graph.target_skeleton.as_deref() {
            let skel = loader::package(src, long_of(skel_path)).and_then(|p| fn_anim::SkeletonInfo::from_package(&p).map_err(|e| e.to_string()));
            for (i, n) in graph.nodes.iter().enumerate() {
                if let PNode::SequencePlayer { sequence, .. } = n {
                    let r = skel.as_ref().map_err(|e| e.clone()).and_then(|skel| {
                        let p = loader::package(src, long_of(sequence))?;
                        let seq = fn_anim::AnimSequence::parse(&p).map_err(|e| format!("{sequence}: {e}"))?;
                        Ok(FnClip::from_sequence(&seq, skel, SampleMode::Ue))
                    });
                    match r { Ok(c) => { seq_clip.insert(i, c); } Err(e) => missing.push(format!("{sequence}: {e}")) }
                }
            }
        }
        Ok(PartAnim { graph, kind, assumptions: SolverAssumptions::default(), sims: HashMap::new(), seq_time: HashMap::new(), seq_clip, blend_w: HashMap::new(), mask: HashMap::new(), pins: vec![], missing })
    }

    /// The exposed-input pins (node, member, value) for given native inputs.
    pub fn pin_values(&self, inputs: &PartAnimInputs) -> Vec<(String, String, f32)> { self.graph.exposed.run(inputs) }

    fn pin(&self, node: usize, member: &str) -> Option<f32> {
        let name = self.graph.node_names.get(node)?;
        self.pins.iter().rev().find(|(n, m, _)| n == name && m == member).map(|p| p.2)
    }

    /// Evaluates the part graph for one tick: returns the part's local pose.
    pub fn evaluate(&mut self, cx: &EvalCtx) -> Pose {
        self.pins = self.graph.exposed.run(cx.inputs);
        let root = match &self.graph.nodes[self.graph.root] { PNode::Root { result } => *result, _ => -1 };
        self.eval(root, cx, 0)
    }

    fn bind(&self, tree: &BoneTree) -> Pose { tree.bind_pose() }

    fn eval(&mut self, id: i32, cx: &EvalCtx, depth: u32) -> Pose {
        let tree = cx.tree;
        if id < 0 || id as usize >= self.graph.nodes.len() || depth > 32 { return self.bind(tree); }
        let idx = id as usize;
        match self.graph.nodes[idx].clone() {
            PNode::Root { result } => self.eval(result, cx, depth + 1),
            PNode::ComponentToLocal { input } | PNode::LocalToComponent { input } => self.eval(input, cx, depth + 1),
            PNode::CopyPoseFromMesh => {
                let mut p = self.bind(tree);
                for (i, m) in cx.master_map.iter().enumerate() { if let Some(m) = m { if i < p.len() && *m < cx.master_local.len() { p[i] = cx.master_local[*m]; } } }
                p
            }
            PNode::Slot { source, .. } => if source >= 0 { self.eval(source, cx, depth + 1) } else { self.bind(tree) /* no source pin: ResetToRefPose; montage slots are not driven in the BR path */ },
            PNode::SequencePlayer { play_rate, start, looping, .. } => {
                let Some(clip) = self.seq_clip.get(&idx) else { return self.bind(tree) };
                let len = clip.sequence_length().max(1e-4);
                let dur = ClipSource::duration(clip).max(1e-4);
                let rate_scale = len / dur;
                let t = self.seq_time.entry(idx).or_insert(start);
                *t += cx.dt * play_rate * rate_scale;
                *t = if looping { t.rem_euclid(len) } else { t.min(len) };
                let t = *t;
                (0..tree.len()).map(|i| { let bind = (tree.local[i].0, tree.local[i].1); clip.sample_bone(&tree.names[i], t, looping, bind).unwrap_or(bind) }).collect()
            }
            PNode::BlendListByBool { poses, blend_times, active_default } => {
                let active = self.pin(idx, "bActiveValue").map(|v| v != 0.).unwrap_or(active_default);
                let target = if active { 0 } else { 1 }.min(poses.len().saturating_sub(1)); // engine: TRUE selects pose 0
                let n = poses.len();
                let w = self.blend_w.entry(idx).or_insert_with(|| { let mut w = vec![0.; n]; if n > 0 { w[target] = 1.; } w });
                // linear ramp to the active child over its BlendTime
                let t = blend_times.get(target).copied().unwrap_or(0.).max(0.);
                for (i, wi) in w.iter_mut().enumerate() {
                    let goal = if i == target { 1. } else { 0. };
                    let step = if t <= 1e-6 { 1. } else { cx.dt / t };
                    *wi = if *wi < goal { (*wi + step).min(goal) } else { (*wi - step).max(goal) };
                }
                let w = w.clone();
                let total: f32 = w.iter().sum::<f32>().max(1e-6);
                let mut acc: Option<Pose> = None;
                let mut acc_w = 0.;
                for (i, c) in poses.iter().enumerate() {
                    if w[i] <= 1e-6 { continue }
                    let p = self.eval(*c, cx, depth + 1);
                    match acc.as_mut() {
                        None => { acc = Some(p); acc_w = w[i] }
                        Some(a) => { let f = w[i] / (acc_w + w[i]); for (x, y) in a.iter_mut().zip(&p) { *x = (x.0.lerp(y.0, f), x.1.slerp(y.1, f)); } acc_w += w[i]; }
                    }
                }
                let _ = total;
                acc.unwrap_or_else(|| self.bind(tree))
            }
            PNode::LayeredBoneBlend { base, blends, weights, filters, .. } => {
                let mut out = self.eval(base, cx, depth + 1);
                if !self.mask.contains_key(&idx) {
                    let m: Vec<Vec<f32>> = filters.iter().map(|f| { let fl: Vec<(&str, i32)> = f.iter().map(|(b, d)| (b.as_str(), *d)).collect(); tree.mask_weights(&fl) }).collect();
                    self.mask.insert(idx, m);
                }
                for (l, b) in blends.iter().enumerate() {
                    let w = weights.get(l).copied().unwrap_or(1.);
                    if w <= 0. { continue }
                    let layer = self.eval(*b, cx, depth + 1);
                    let Some(mask) = self.mask.get(&idx).and_then(|m| m.get(l)) else { continue };
                    for i in 0..out.len() {
                        let f = (mask[i] * w).clamp(0., 1.);
                        if f > 0. { out[i] = (out[i].0.lerp(layer[i].0, f), out[i].1.slerp(layer[i].1, f).normalize()); }
                    }
                }
                out
            }
            PNode::Dynamics { params, input } => {
                let mut local = self.eval(input, cx, depth + 1);
                let Some(bi) = tree.bone_index(&params.bound_bone) else { self.missing.push(format!("AnimDynamics bone {} is not in the part skeleton", params.bound_bone)); return local };
                if !params.do_eval { return local }
                let alpha_pin = self.pin(idx, "Alpha").unwrap_or(params.alpha);
                let alpha = params.actual_alpha(alpha_pin);
                if alpha < 1e-5 { self.sims.entry(idx).or_default().reset(); return local }
                let comp = tree.component(&local);
                let (bp, bq) = comp[bi];
                let input_ue = to_ue(bp, bq);
                let sim = self.sims.entry(idx).or_default();
                let out_ue = sim.step(&params, &self.assumptions, input_ue, cx.dt);
                let blended = AnimDynamicsSim::blend(input_ue, out_ue, alpha);
                let (np, nq) = from_ue(blended.0, blended.1);
                local[bi] = tree.local_of(bi, &comp, (np, nq));
                local
            }
            PNode::Other(_) => self.bind(tree),
        }
    }
}

/// A character whose parts run their own AnimBlueprints on top of the master pose (the BR path of `Scene`).
pub struct PartAnimRig {
    /// `Fortnite_M_Avg_Player` (the master / IK skeleton the pose tree is evaluated on).
    pub master: Part,
    /// One per rig part (`None` = the part has no AnimClass: it follows the master pose only).
    pub anims: Vec<Option<PartAnim>>,
    master_map: Vec<Vec<Option<usize>>>,
    /// `true`: `CopyPoseFromMesh` exactly (pose tree evaluated on the master, shared bones copy its local transforms).
    /// `false`: the pose tree is evaluated on each part's own bind pose (the retarget-mode path of [`CharacterRig::evaluate_into`]) and the part AnimBP's
    /// `CopyPoseFromMesh` takes that pose; only the part graph's own nodes (face layer, AnimDynamics) are added.
    pub copy_master_pose: bool,
    pub notes: Vec<String>,
    pub female: bool,
}

impl PartAnimRig {
    pub fn new(rig: &CharacterRig, master: Part, anims: Vec<Option<PartAnim>>) -> PartAnimRig {
        let master_map = rig.parts.iter().map(|p| (0..p.tree.len()).map(|i| if i == 0 { None } else { master.tree.bone_index(&p.tree.names[i]) }).collect()).collect();
        let copy_master_pose = true;
        let mut notes = vec![];
        for (i, a) in anims.iter().enumerate() {
            let Some(a) = a else { continue };
            for u in a.graph.unsupported() { notes.push(format!("part {i} ({}): NOT simulated: {u}", a.graph.package)); }
            for m in &a.missing { notes.push(format!("part {i}: MISSING {m}")); }
            for n in &a.graph.notes { notes.push(format!("part {i}: {n}")); }
        }
        PartAnimRig { master, anims, master_map, copy_master_pose, notes, female: false }
    }

    /// Like `CharacterRig::evaluate_into`, but the pose tree is evaluated once on the master skeleton and each part copies it (`CopyPoseFromMesh`)
    /// and then runs its AnimBP.
    pub fn evaluate_into(&mut self, rig: &CharacterRig, tree: &Node, clips: &dyn ClipLibrary, dt: f32, inputs: &PartAnimInputs, out: &mut CharacterPose) {
        out.parts.resize_with(rig.parts.len(), PoseResult::default);
        out.missing_clips.clear();
        let ev = PoseEvaluator::new(&self.master.tree, clips, rig.opts);
        let master_local = if self.copy_master_pose { ev.eval_pose(tree) } else { vec![] };
        let ik = ev.ik_stats.get();
        for m in ev.missing.borrow().iter() { if !out.missing_clips.contains(m) { out.missing_clips.push(m.clone()); } }
        for (pi, (part, res)) in rig.parts.iter().zip(out.parts.iter_mut()).enumerate() {
            let (src_pose, src_map): (Pose, Vec<Option<usize>>) = if self.copy_master_pose { (master_local.clone(), self.master_map[pi].clone()) } else {
                let pe = PoseEvaluator::new(&part.tree, clips, rig.opts);
                let p = pe.eval_pose(tree);
                res.ik_stats = pe.ik_stats.get();
                (p, (0..part.tree.len()).map(Some).collect())
            };
            res.bone_local = match self.anims.get_mut(pi).and_then(|a| a.as_mut()) {
                Some(a) => a.evaluate(&EvalCtx { tree: &part.tree, master_local: &src_pose, master_map: &src_map, dt, inputs }),
                None => copy_pose(&part.tree, &src_pose, &src_map),
            };
            part.tree.globals_into(&res.bone_local, &mut res.bone_mesh);
            part.skin_matrices_into(&res.bone_mesh, &mut res.skin_matrices);
            if self.copy_master_pose { res.ik_stats = ik; }
            out.ik_stats = res.ik_stats;
        }
        out.sockets.clear();
        for s in &rig.sockets {
            let (Some(part), Some(res)) = (rig.parts.get(s.part), out.parts.get(s.part)) else { continue };
            if let Some(i) = part.tree.bone_index(&s.bone) { out.sockets.push((s.name.clone(), res.bone_mesh[i] * s.local)); }
        }
    }
}

/// `CopyPoseFromMesh` alone: master local transform for bones the master has, reference pose for the rest.
pub fn copy_pose(tree: &BoneTree, master_local: &[BoneXform], map: &[Option<usize>]) -> Pose {
    let mut p = tree.bind_pose();
    for (i, m) in map.iter().enumerate() { if let Some(m) = m { if i < p.len() && *m < master_local.len() { p[i] = master_local[*m]; } } }
    p
}

#[allow(dead_code)]
fn unused(_: Vec3, _: Quat) {}
