//! Adapter for the OLD prototype's data formats, kept so the parity tests and the headless harness can keep running on the exported
//! assets while the real pipeline (fn-pak -> fn-uasset -> fn-anim / fn-mesh) takes over:
//!
//! * [`Md5Clip`]  the old `assets::Clip`: parser for the exported `.md5anim` text files (with the `// track` / `// retarget` metadata
//!                comments) and the retargeting rules. Implements `fn_animgraph::ClipSource`.
//! * [`load_body_skeleton`] the old `assets::Model::load` skeleton part: the part mesh's glTF nodes plus the master skeleton's seven IK bones.
//!
//! Nothing here touches OpenGL or the OS. glTF is parsed with the pure-Rust `gltf` crate.
use fn_animgraph::{BoneXform, ClipSource, SkinPrimitive, SkinnedMesh, Skeleton};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
mod tests;

/// All files below `root`, recursively. An unreadable/missing directory yields no files (`find` then names the missing asset).
pub fn files(root: &Path) -> Vec<PathBuf> {
    let mut result = vec![];
    let Ok(dir) = std::fs::read_dir(root) else { return result };
    for e in dir.flatten() { let p = e.path(); if p.is_dir() { result.extend(files(&p)); } else { result.push(p); } }
    result
}
/// Case-insensitive lookup by file name. `Err` carries a user-presentable message naming the missing file.
pub fn find(files: &[PathBuf], name: &str) -> Result<PathBuf, String> {
    files.iter().find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))).cloned().ok_or_else(|| format!("Missing asset: {name}"))
}
/// Required clip by file stem + extension (e.g. "Idle_Noise.md5anim"); `Err` names the file.
pub fn load_clip(all: &[PathBuf], name: &str) -> Result<Md5Clip, String> { Md5Clip::load(&find(all, name)?) }
/// Loads every named clip (`<name>.md5anim`) into a library usable by `Skeleton::eval_pose`.
pub fn load_clip_set(all: &[PathBuf], names: &[String]) -> Result<HashMap<String, Md5Clip>, String> {
    let mut m = HashMap::new();
    for n in names { if !m.contains_key(n) { m.insert(n.clone(), load_clip(all, &format!("{n}.md5anim"))?); } }
    Ok(m)
}
/// The clips the equipped-pickaxe prototype loads (old main.rs `names`): the graph's `required_clips()` plus the swing montage clips.
pub fn prototype_clip_names() -> Vec<String> {
    let mut names: Vec<String> = ["M_NoWep_Relaxed", "JumpUp_NoWep_CMM_Additive", "JumpApex_NoWep__Loop_CMM_Additive", "JumpFall_NoWep_CMM_Additive", "JumpLand_NoWep_CMM_Additive", "Crouch_TwoHanded_Melee_CMM", "Idle_Noise",
        "TwoHanded_Loop_Swing1", "TwoHanded_Loop_Swing1_2Idle", "TwoHanded_Loop_Swing2", "TwoHanded_Loop_Swing2_2Idle", "TwoHanded_Loop_Swing1_InMotion", "TwoHanded_Loop_Swing2_InMotion", "TwoHanded_Loop_Swing1_2Idle_InMotion",
        "TwoHanded_Loop_Swing2_2Idle_InMotion", "TwoHanded_Loop_Swing1_Bounce", "TwoHanded_Loop_Swing2_Bounce", "Sprint_TwoHanded",
        "M_JumpUp_ARDS_Additive", "M_JumpApex_ARDS_Loop_Additive", "M_JumpFall_ARDS_Additive", "M_JumpLand_ARDS_Additive", "Melee_WideGrip_Equip_CMM"].iter().map(|s| s.to_string()).collect();
    for n in fn_animgraph::required_clips() { if !names.contains(&n) { names.push(n); } }
    names
}

#[derive(Clone, Copy)]
struct Retarget { mode: u8, source: Vec3 }
/// The old `assets::Clip`: frames of (translation m, rotation) per bone plus the retargeting metadata of the export.
pub struct Md5Clip { pub names: HashMap<String, usize>, pub frames: Vec<Vec<(Vec3, Quat)>>, pub rate: f32, retarget: HashMap<String, Retarget>, channels: HashMap<String, (bool, bool)>, additive: bool, retarget_base: bool }

impl Md5Clip {
    /// Parses an exported MD5 animation. `Err` names the file (missing, unreadable, malformed or without frames).
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("Cannot read animation {}: {e}", path.display()))?;
        Self::parse(&text, &path.display().to_string())
    }
    /// Parses MD5 animation text (`label` names the source in error messages).
    pub fn parse(text: &str, label: &str) -> Result<Self, String> {
        let mut names = HashMap::new(); let mut frames = vec![]; let mut current = vec![]; let mut frame = false; let mut hierarchy = false; let mut rate = 30.;
        let mut retarget = HashMap::new(); let mut channels = HashMap::new(); let mut additive = false; let mut retarget_base = false;
        let bad = |l: &str| format!("Malformed animation {label}: {l}");
        for l in text.lines().map(str::trim) {
            if l == "// additive 1" { additive = true; }
            if l == "// retarget-base 1" { retarget_base = true; }
            if l.starts_with("// track \"") {
                let parts: Vec<_> = l.split('"').collect(); let v: Vec<_> = parts.get(2).map(|s| s.split_whitespace().collect()).unwrap_or_default();
                if parts.len() < 3 || v.len() < 2 { return Err(bad(l)); }
                channels.insert(parts[1].to_lowercase(), (v[0] == "1", v[1] == "1"));
            }
            if l.starts_with("// retarget \"") {
                let parts: Vec<_> = l.split('"').collect(); let v: Vec<_> = parts.get(2).map(|s| s.split_whitespace().collect()).unwrap_or_default();
                let f = |i: usize| v.get(i).and_then(|s| s.parse::<f32>().ok());
                let (Some(mode), Some(x), Some(y), Some(z)) = (v.first().and_then(|s| s.parse::<u8>().ok()), f(1), f(2), f(3)) else { return Err(bad(l)) };
                retarget.insert(parts[1].to_lowercase(), Retarget { mode, source: Vec3::new(x, y, z) });
            }
            if l.starts_with("frameRate ") { rate = l.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| bad(l))?; }
            if l == "hierarchy {" { hierarchy = true; continue; }
            if hierarchy && l.starts_with('"') { let n = l.split('"').nth(1).ok_or_else(|| bad(l))?.to_lowercase(); let idx = names.len(); names.insert(n, idx); }
            if l.starts_with("frame ") { frame = true; current = vec![]; continue; }
            if l == "}" { hierarchy = false; if frame { frames.push(std::mem::take(&mut current)); frame = false; } continue; }
            if frame { let v: Vec<f32> = l.split_whitespace().map(|x| x.parse().map_err(|_| bad(l))).collect::<Result<_, _>>()?; if v.len() == 6 {
                let w = (1. - v[3] * v[3] - v[4] * v[4] - v[5] * v[5]).max(0.).sqrt();
                current.push((Vec3::new(v[0], v[2], -v[1]) * 0.01, Quat::from_xyzw(v[3], v[5], -v[4], -w).normalize()));
            } }
        }
        if frames.is_empty() { return Err(format!("Animation {label} has no frames")); }
        Ok(Self { names, frames, rate, retarget, channels, additive, retarget_base })
    }
    pub fn sample(&self, name: &str, t: f32) -> Option<(Vec3, Quat)> { self.sample_at(name, t, true) }
    /// Engine-exact key position (UE 4.16 `UAnimSequence`): the N keys span the sequence LENGTH with N-1 intervals (key spacing = length / (N-1), position =
    /// time / length * (N-1)), where length = N / frameRate of the export (the md5anim frameRate is NumFrames / Length). Looping clips wrap the time at the
    /// length (the last key equals the first, so the seam holds no duplicate frame); non-looping clips hold the last key.
    pub fn key_position(&self, t: f32, looping: bool) -> f32 {
        let n = self.frames.len(); if n < 2 { return 0.; }
        let len = n as f32 / self.rate; let t = if looping { t.rem_euclid(len) } else { t.clamp(0., len) };
        (t / len * (n - 1) as f32).clamp(0., (n - 1) as f32)
    }
    pub fn sample_at(&self, name: &str, t: f32, looping: bool) -> Option<(Vec3, Quat)> {
        let i = *self.names.get(name)?; let n = self.frames.len();
        let pos = self.key_position(t, looping); let a = (pos.floor() as usize).min(n - 1); let b = (a + 1).min(n - 1); let fr = pos - a as f32;
        let (p, q) = self.frames[a][i]; let (p2, q2) = self.frames[b][i];
        // UE `FQuat::FastLerp` + normalize (AEFLinearKeyLerp): shortest-path nlerp, not slerp.
        let q2 = if q.dot(q2) < 0. { -q2 } else { q2 };
        Some((p.lerp(p2, fr), (q * (1. - fr) + q2 * fr).normalize()))
    }
    fn fit(&self, name: &str, pose: (Vec3, Quat), bind: (Vec3, Quat)) -> (Vec3, Quat) {
        let fallback = if self.additive { (Vec3::ZERO, Quat::IDENTITY) } else { bind };
        let (has_p, has_q) = self.channels.get(name).copied().unwrap_or((true, true));
        if !has_p && !has_q { return fallback; }
        let pose = (if has_p { pose.0 } else { fallback.0 }, if has_q { pose.1 } else { fallback.1 });
        match self.retarget.get(name) {
            Some(r) => retarget_pose(*r, pose, bind, self.additive, self.retarget_base),
            // Compatibility for old exports only. Runtime clips are re-exported with metadata.
            None => if name == "pelvis" { pose } else { (if self.additive { Vec3::ZERO } else { bind.0 }, pose.1) },
        }
    }
    /// UE sequence length of these exports = frames / frame rate (JumpUp 13 / 32.5 = 0.4 s, JumpLand_NoWep 26 / 31.2 = 0.8333 s).
    pub fn duration(&self) -> f32 { self.frames.len() as f32 / self.rate }
    pub fn is_additive(&self) -> bool { self.additive }
}
fn retarget_pose(r: Retarget, (p, q): (Vec3, Quat), bind: (Vec3, Quat), additive: bool, source_override: bool) -> (Vec3, Quat) {
    match r.mode {
        1 => (if additive { Vec3::ZERO } else if source_override { r.source } else { bind.0 }, q),
        2 => { let length = r.source.length(); if length > 0.00001 { (p * (bind.0.length() / length), q) } else if additive { (Vec3::ZERO, Quat::IDENTITY) } else { bind } },
        // Animation (0) retains the sampled translation. Relative (3) is still
        // unimplemented, as in this UEViewer version; do not claim full retarget parity.
        _ => (p, q),
    }
}

impl ClipSource for Md5Clip {
    fn sample_bone(&self, bone: &str, t: f32, looping: bool, bind: BoneXform) -> Option<BoneXform> { self.sample_at(bone, t, looping).map(|p| self.fit(bone, p, bind)) }
    fn is_additive(&self) -> bool { self.additive }
    fn duration(&self) -> f32 { Md5Clip::duration(self) }
    fn has_bone(&self, bone: &str) -> bool { self.names.contains_key(bone) }
}

/// Appends every bone of the master skeleton mesh that `names` lacks (parents first; a bone whose parent is unknown is skipped).
fn add_missing_master_bones(doc: &gltf::Document, names: &mut Vec<String>, parents: &mut Vec<Option<usize>>, local: &mut Vec<(Vec3, Quat, Vec3)>) {
    let mut parent_of = HashMap::new(); for n in doc.nodes() { for c in n.children() { parent_of.insert(c.index(), n.index()); } }
    let name_of = |i: usize| doc.nodes().nth(i).and_then(|n| n.name().map(str::to_lowercase)).unwrap_or_default();
    loop {
        let mut added = false;
        for node in doc.nodes() {
            let name = node.name().unwrap_or("").to_lowercase();
            if name.is_empty() || names.contains(&name) { continue; }
            let Some(parent) = parent_of.get(&node.index()).map(|p| name_of(*p)).and_then(|p| names.iter().position(|n| *n == p)) else { continue };
            let (t, r, s) = node.transform().decomposed();
            names.push(name); parents.push(Some(parent)); local.push((Vec3::from(t), Quat::from_array(r), Vec3::from(s))); added = true;
        }
        if !added { break; }
    }
}

/// The body skeleton exactly as the old `Model::load` builds it for a part mesh: glTF node hierarchy (bone names lower-cased) plus, when the
/// part lacks them, the bones of `Fortnite_M_Avg_Player.gltf` (the seven `ik_*` bones). `joints` are the skin joint node indices.
pub struct BodySkeleton { pub skeleton: Skeleton, pub joints: Vec<usize>, pub inverse_bind: Vec<Mat4> }
pub fn load_body_skeleton(all: &[PathBuf], part: &str) -> Result<BodySkeleton, String> {
    let path = find(all, part)?;
    let bad = |what: &str| format!("Cannot load model {}: {what}", path.display());
    let (doc, buffers, _) = gltf::import(&path).map_err(|e| bad(&e.to_string()))?; let count = doc.nodes().count();
    let (mut parents, mut names, mut local) = (vec![None; count], vec![String::new(); count], vec![(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE); count]);
    for node in doc.nodes() { let i = node.index(); names[i] = node.name().unwrap_or("").to_lowercase(); let (t, r, s) = node.transform().decomposed(); local[i] = (Vec3::from(t), Quat::from_array(r), Vec3::from(s)); for c in node.children() { parents[c.index()] = Some(i); } }
    if names.iter().any(|n| n == "pelvis") && !names.iter().any(|n| n == "ik_hand_l") {
        if let Some((mdoc, _, _)) = find(all, "Fortnite_M_Avg_Player.gltf").ok().and_then(|p| gltf::import(p).ok()) { add_missing_master_bones(&mdoc, &mut names, &mut parents, &mut local); }
    }
    let skin = doc.skins().next().ok_or_else(|| bad("no skin"))?;
    let joints = skin.joints().map(|n| n.index()).collect();
    let inverse_bind = skin.reader(|b| Some(&buffers[b.index()].0)).read_inverse_bind_matrices().ok_or_else(|| bad("no inverse bind matrices"))?.map(|m| Mat4::from_cols_array_2d(&m)).collect();
    Ok(BodySkeleton { skeleton: Skeleton { names, parents, local }, joints, inverse_bind })
}
/// A skinned part mesh as the old `Model::load` reads it (no textures, no GL): the part's skeleton plus the glTF skin data, ready for
/// `SkinnedMesh::pose` (the renderer-free `apply_pose`). `--face-test` and the face-rigidity test use the head part.
pub struct SkinnedPart { pub body: BodySkeleton, pub mesh: SkinnedMesh }
/// Loads `part` (e.g. `SK_M_MED_Head_01_ATH.gltf`): skeleton like [`load_body_skeleton`] plus positions / normals / joints / weights of every primitive.
pub fn load_skinned_part(all: &[PathBuf], part: &str) -> Result<SkinnedPart, String> {
    let body = load_body_skeleton(all, part)?;
    let path = find(all, part)?; let bad = |what: &str| format!("Cannot load model {}: {what}", path.display());
    let (doc, buffers, _) = gltf::import(&path).map_err(|e| bad(&e.to_string()))?;
    let mut primitives = vec![];
    for mesh in doc.meshes() { for p in mesh.primitives() {
        let r = p.reader(|b| Some(&buffers[b.index()].0));
        let prim = (|| Some(SkinPrimitive { p: r.read_positions()?.collect(), n: r.read_normals()?.collect(), j: r.read_joints(0)?.into_u16().collect(), w: r.read_weights(0)?.into_f32().collect() }))();
        primitives.push(prim.ok_or_else(|| bad("a mesh primitive lacks positions/normals/joints/weights"))?);
    } }
    let mesh = SkinnedMesh { joints: body.joints.clone(), inverse_bind: body.inverse_bind.clone(), primitives };
    Ok(SkinnedPart { body, mesh })
}
/// The head part of the character (`SK_M_MED_Head_01_ATH`): face, eyes, lids, brows, lips, jaw, teeth, hair, skinned to the head bone and below.
pub fn load_head_model(all: &[PathBuf]) -> Result<SkinnedPart, String> { load_skinned_part(all, "SK_M_MED_Head_01_ATH.gltf") }
/// The skeleton the prototype poses and logs (`SK_M_Med_Soldier_04.gltf` + the master's IK bones).
pub fn load_soldier_skeleton(all: &[PathBuf]) -> Result<Skeleton, String> { Ok(load_body_skeleton(all, "SK_M_Med_Soldier_04.gltf")?.skeleton) }
