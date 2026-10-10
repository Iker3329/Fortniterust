//! PART ANIM: scene-side glue for the BR character parts' own AnimBlueprints (`fn_pose::part_anim`): face rig (head) and the AnimDynamics cloth /
//! physics bones (body). Loaded next to the BR character parts (same paks), stashed here, and taken by the loader into `Character::part_anim`.
//! `FN_PART_ANIM=0` disables it (bind pose for those bones, as before). The original path is literal engine
//! `CopyPoseFromMesh` (tree evaluated on the master, local transforms copied). `FN_PART_COPY_MASTER=0`
//! is retained only as a diagnostic escape hatch.
use fn_pose::br_character::BrCharacter;
use fn_pose::part_anim::{PartAnim, PartAnimInputs, PartAnimRig};
use fn_pose::{CharacterRig, ClipSourceDir, EvalOptions, SocketDef};
use std::sync::Mutex;

static STASH: Mutex<Option<PartAnimRig>> = Mutex::new(None);

/// Decodes the parts' AnimBPs and stashes the runtime. Failures are reported and leave the parts at the master pose.
pub fn install(ch: &BrCharacter, src: &ClipSourceDir) {
    if let Some(par) = build(ch, src) { if let Ok(mut s) = STASH.lock() { *s = Some(par); } }
}

/// The parts' AnimBP runtime of `ch` (what [`install`] stashes), returned instead of stashed (COSMETIC SWAP: the runtime skin swap builds it on a worker thread).
pub fn build(ch: &BrCharacter, src: &ClipSourceDir) -> Option<PartAnimRig> {
    if fn_live::var("FN_PART_ANIM").is_ok_and(|v| v == "0") { eprintln!("[part-anim] disabled by FN_PART_ANIM=0"); return None; }
    let master = ch.master.clone()?;
    let rig = CharacterRig { parts: ch.rig_parts(), sockets: vec![SocketDef::right_hand(0)], opts: EvalOptions::default() };
    let mut anims = vec![];
    for p in ch.parts.iter().take(2) {
        anims.push(match &p.def.anim_class {
            None => None,
            Some(a) => match PartAnim::load(src, a, p.def.kind) { Ok(x) => { eprintln!("[part-anim] {:?}: {} ({} nodes, {} AnimDynamics)", p.def.kind, x.graph.package, x.graph.nodes.len(), x.graph.dynamics_chain().len()); Some(x) } Err(e) => { eprintln!("[part-anim] {:?}: cannot decode {a}: {e}", p.def.kind); None } },
        });
    }
    let mut par = PartAnimRig::new(&rig, master, anims);
    // The body/head AnimBPs take their shared bones from Fortnite_M_Avg_Player.
    // Evaluating the player graph separately on each cosmetic skeleton shifts
    // `ik_hand_gun` and leaves the visible support hand away from the firearm.
    par.copy_master_pose = !fn_live::var("FN_PART_COPY_MASTER").is_ok_and(|v| v == "0");
    par.female = ch.female;
    for n in &par.notes { eprintln!("[part-anim] {n}"); }
    Some(par)
}

pub fn take() -> Option<PartAnimRig> { STASH.lock().ok().and_then(|mut s| s.take()) }

/// Native inputs of the part AnimInstance from the sim: `PawnSpeed2D` (cm/s) from the graph's own variable. `bCanPlayCustomAnimations` default false.
pub fn inputs(speed2d_cm_s: f32) -> PartAnimInputs { PartAnimInputs { pawn_speed_2d: speed2d_cm_s, can_play_custom_animations: false } }
