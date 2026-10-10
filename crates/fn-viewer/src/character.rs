//! Skinned glTF models, the prototype's md5anim clips (with the retarget metadata it exports) and a simple pose evaluator.
//!
//! The pose here is intentionally simple (rest pose, or one looping base clip + an optional additive noise clip). The prototype
//! drives the same skeleton through its AnimBlueprint port (state machines, blend spaces, IK); none of that is ported here.
use crate::assets::{parse_mat, AssetIndex};
use fn_render::{MaterialImages, MaterialKind, SkinnedPrimitive, SkinnedVertex};
use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------------------------------------------------------
// md5anim clips (port of assets::Clip of the prototype)
// ---------------------------------------------------------------------------------------------------------------------------
#[derive(Clone, Copy)]
struct Retarget { mode: u8, source: Vec3 }
pub struct Clip {
    pub names: HashMap<String, usize>,
    pub frames: Vec<Vec<(Vec3, Quat)>>,
    pub rate: f32,
    retarget: HashMap<String, Retarget>,
    channels: HashMap<String, (bool, bool)>,
    pub additive: bool,
    retarget_base: bool,
}
impl Clip {
    /// Parses an exported MD5 animation (positions in cm -> metres, UE -> Y-up axes).
    pub fn parse(text: &str, label: &str) -> Result<Clip, String> {
        let mut names = HashMap::new();
        let mut frames = vec![];
        let mut current = vec![];
        let (mut frame, mut hierarchy) = (false, false);
        let mut rate = 30.;
        let mut retarget = HashMap::new();
        let mut channels = HashMap::new();
        let (mut additive, mut retarget_base) = (false, false);
        let bad = |l: &str| format!("malformed animation {label}: {l}");
        for l in text.lines().map(str::trim) {
            if l == "// additive 1" { additive = true; }
            if l == "// retarget-base 1" { retarget_base = true; }
            if l.starts_with("// track \"") {
                let parts: Vec<_> = l.split('"').collect();
                let v: Vec<_> = parts.get(2).map(|s| s.split_whitespace().collect()).unwrap_or_default();
                if parts.len() < 3 || v.len() < 2 { return Err(bad(l)); }
                channels.insert(parts[1].to_lowercase(), (v[0] == "1", v[1] == "1"));
            }
            if l.starts_with("// retarget \"") {
                let parts: Vec<_> = l.split('"').collect();
                let v: Vec<_> = parts.get(2).map(|s| s.split_whitespace().collect()).unwrap_or_default();
                let f = |i: usize| v.get(i).and_then(|s| s.parse::<f32>().ok());
                let (Some(mode), Some(x), Some(y), Some(z)) = (v.first().and_then(|s| s.parse::<u8>().ok()), f(1), f(2), f(3)) else { return Err(bad(l)) };
                retarget.insert(parts[1].to_lowercase(), Retarget { mode, source: Vec3::new(x, y, z) });
            }
            if l.starts_with("frameRate ") { rate = l.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or_else(|| bad(l))?; }
            if l == "hierarchy {" { hierarchy = true; continue; }
            if hierarchy && l.starts_with('"') {
                let n = l.split('"').nth(1).ok_or_else(|| bad(l))?.to_lowercase();
                let idx = names.len();
                names.insert(n, idx);
            }
            if l.starts_with("frame ") { frame = true; current = vec![]; continue; }
            if l == "}" {
                hierarchy = false;
                if frame { frames.push(std::mem::take(&mut current)); frame = false; }
                continue;
            }
            if frame {
                let v: Vec<f32> = l.split_whitespace().map(|x| x.parse().map_err(|_| bad(l))).collect::<Result<_, _>>()?;
                if v.len() == 6 {
                    let w = (1. - v[3] * v[3] - v[4] * v[4] - v[5] * v[5]).max(0.).sqrt();
                    current.push((Vec3::new(v[0], v[2], -v[1]) * 0.01, Quat::from_xyzw(v[3], v[5], -v[4], -w).normalize()));
                }
            }
        }
        if frames.is_empty() { return Err(format!("animation {label} has no frames")); }
        Ok(Clip { names, frames, rate, retarget, channels, additive, retarget_base })
    }
    pub fn load(path: &Path) -> Result<Clip, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read animation {}: {e}", path.display()))?;
        Clip::parse(&text, &path.display().to_string())
    }
    pub fn duration(&self) -> f32 { self.frames.len() as f32 / self.rate }
    pub fn sample(&self, name: &str, t: f32) -> Option<(Vec3, Quat)> {
        let i = *self.names.get(name)?;
        let f = t * self.rate;
        let a = f.floor() as usize % self.frames.len();
        let b = (a + 1) % self.frames.len();
        let (p, q) = self.frames[a][i];
        let (p2, q2) = self.frames[b][i];
        Some((p.lerp(p2, f.fract()), q.slerp(q2, f.fract())))
    }
    /// Applies the clip's track mask and retarget mode to a sampled pose (`bind` = the mesh skeleton's local bind pose).
    pub fn fit(&self, name: &str, pose: (Vec3, Quat), bind: (Vec3, Quat)) -> (Vec3, Quat) {
        let fallback = if self.additive { (Vec3::ZERO, Quat::IDENTITY) } else { bind };
        let (has_p, has_q) = self.channels.get(name).copied().unwrap_or((true, true));
        if !has_p && !has_q { return fallback; }
        let pose = (if has_p { pose.0 } else { fallback.0 }, if has_q { pose.1 } else { fallback.1 });
        match self.retarget.get(name) {
            Some(r) => retarget_pose(*r, pose, bind, self.additive, self.retarget_base),
            None => if name == "pelvis" { pose } else { (if self.additive { Vec3::ZERO } else { bind.0 }, pose.1) },
        }
    }
}
fn retarget_pose(r: Retarget, (p, q): (Vec3, Quat), bind: (Vec3, Quat), additive: bool, source_override: bool) -> (Vec3, Quat) {
    match r.mode {
        1 => (if additive { Vec3::ZERO } else if source_override { r.source } else { bind.0 }, q),
        2 => {
            let length = r.source.length();
            if length > 0.00001 { (p * (bind.0.length() / length), q) } else if additive { (Vec3::ZERO, Quat::IDENTITY) } else { bind }
        }
        _ => (p, q),
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// skeleton + glTF
// ---------------------------------------------------------------------------------------------------------------------------

pub struct SkelModel {
    pub names: Vec<String>,
    pub parents: Vec<Option<usize>>,
    pub local: Vec<(Vec3, Quat, Vec3)>,
    pub joints: Vec<usize>,
    pub inverse: Vec<Mat4>,
}
impl SkelModel {
    pub fn bone(&self, name: &str) -> Option<usize> { self.names.iter().position(|n| n == name) }
    /// Model-space matrix of every node. `clip`/`noise` are sampled at `t` (the clip replaces the local pose of every bone it
    /// animates, the additive noise clip is added on top, as in the prototype's `Model::draw`).
    pub fn globals(&self, clip: Option<&Clip>, noise: Option<&Clip>, t: f32) -> Vec<Mat4> {
        let mut globals = vec![Mat4::IDENTITY; self.names.len()];
        for i in 0..globals.len() {
            let (mut p, mut q, s) = self.local[i];
            let bind = (p, q);
            if let Some(clip) = clip {
                if let Some(pose) = clip.sample(&self.names[i], t) { (p, q) = clip.fit(&self.names[i], pose, bind); }
            }
            if let Some(n) = noise {
                if let Some((dp, dq)) = n.sample(&self.names[i], t).map(|pose| n.fit(&self.names[i], pose, bind)) { p += dp; q = (dq * q).normalize(); }
            }
            let local = Mat4::from_scale_rotation_translation(s, q, p);
            globals[i] = self.parents[i].map(|j| globals[j] * local).unwrap_or(local);
        }
        globals
    }
    /// Skin matrices `globals[joint] * inverse_bind` in joint order (what `vs_skinned` indexes with JOINTS_0).
    pub fn skin(&self, globals: &[Mat4]) -> Vec<Mat4> { self.joints.iter().enumerate().map(|(i, j)| globals[*j] * self.inverse[i]).collect() }
}

pub struct LoadedModel {
    pub skel: SkelModel,
    pub primitives: Vec<SkinnedPrimitive>,
}

/// Loads a skinned glTF exported by UEViewer with its materials resolved through the `.mat` files of the asset folder.
pub fn load_gltf(index: &AssetIndex, name: &str, kind: MaterialKind) -> Result<LoadedModel, String> {
    let path = index.find(name)?;
    let bad = |what: &str| format!("cannot load model {}: {what}", path.display());
    let (doc, buffers, _) = gltf::import(path).map_err(|e| bad(&e.to_string()))?;
    let count = doc.nodes().count();
    let mut parents = vec![None; count];
    let mut names = vec![String::new(); count];
    let mut local = vec![(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE); count];
    for node in doc.nodes() {
        let i = node.index();
        names[i] = node.name().unwrap_or("").to_lowercase();
        let (t, r, s) = node.transform().decomposed();
        local[i] = (Vec3::from(t), Quat::from_array(r), Vec3::from(s));
        for child in node.children() { parents[child.index()] = Some(i); }
    }
    let skin = doc.skins().next().ok_or_else(|| bad("no skin"))?;
    let joints: Vec<usize> = skin.joints().map(|n| n.index()).collect();
    let inverse: Vec<Mat4> = skin.reader(|b| Some(&buffers[b.index()].0)).read_inverse_bind_matrices().ok_or_else(|| bad("no inverse bind matrices"))?.map(|m| Mat4::from_cols_array_2d(&m)).collect();
    let mut primitives = vec![];
    for mesh in doc.meshes() {
        for primitive in mesh.primitives() {
            let r = primitive.reader(|b| Some(&buffers[b.index()].0));
            let mat = primitive.material().name().unwrap_or("").to_owned();
            let slots = index.find(&format!("{mat}.mat")).ok().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_mat(&t)).unwrap_or_default();
            let load = |n: &Option<String>| n.as_ref().and_then(|n| index.load_rgba(&format!("{n}.png")).ok());
            let material = MaterialImages { diffuse: load(&slots.diffuse), normal: load(&slots.normal), spec: load(&slots.spec), mask: load(&slots.mask), ue: None, two_sided: false, cooked: Default::default() };
            let pos: Vec<[f32; 3]> = r.read_positions().ok_or_else(|| bad("no positions"))?.collect();
            let nrm: Vec<[f32; 3]> = r.read_normals().ok_or_else(|| bad("no normals"))?.collect();
            let uv: Vec<[f32; 2]> = r.read_tex_coords(0).ok_or_else(|| bad("no uvs"))?.into_f32().collect();
            let jn: Vec<[u16; 4]> = r.read_joints(0).ok_or_else(|| bad("no joints"))?.into_u16().collect();
            let wt: Vec<[f32; 4]> = r.read_weights(0).ok_or_else(|| bad("no weights"))?.into_f32().collect();
            let indices: Vec<u32> = r.read_indices().ok_or_else(|| bad("no indices"))?.into_u32().collect();
            let vertices = (0..pos.len()).map(|i| SkinnedVertex { pos: pos[i], normal: nrm[i], uv: uv[i], joints: jn[i], weights: wt[i] }).collect();
            primitives.push(SkinnedPrimitive { vertices, indices, material, kind });
        }
    }
    Ok(LoadedModel { skel: SkelModel { names, parents, local, joints, inverse }, primitives })
}

/// SK_Pick is attached rigidly (its skin is ignored, like the prototype's `draw_attached`): joints -> 0, weight 1.
pub fn make_rigid(prims: &mut [SkinnedPrimitive]) {
    for p in prims { for v in &mut p.vertices { v.joints = [0; 4]; v.weights = [1., 0., 0., 0.]; } }
}
/// Pickaxe socket yaw on weapon_r (UE Yaw=-90 mirrored into the handedness-flipped glTF export).
pub const PICK_SOCKET_YAW: f32 = std::f32::consts::FRAC_PI_2;

#[cfg(test)]
mod tests {
    use super::*;
    const CLIP: &str = "MD5Version 10\nnumFrames 2\nnumJoints 2\nframeRate 60\n// additive 0\n// retarget-base 1\n// track \"pelvis\" 1 1\n// retarget \"pelvis\" 2 0 1 0\n// track \"spine\" 1 1\n// retarget \"spine\" 1 0.5 0 0\nhierarchy {\n\t\"pelvis\" -1 63 0\n\t\"spine\" 0 63 6\n}\nframe 0 {\n\t0 0 100 0 0 0\n\t5 0 0 0 0 0\n}\nframe 1 {\n\t0 0 200 0 0 0\n\t5 0 0 0 0 0\n}\n";
    #[test]
    fn parses_hierarchy_frames_and_axis_conversion() {
        let c = Clip::parse(CLIP, "t").unwrap();
        assert_eq!(c.frames.len(), 2);
        assert!((c.rate - 60.).abs() < 1e-6);
        // UE (x, y, z) cm -> (x, z, -y) m
        let (p, q) = c.frames[0][c.names["pelvis"]];
        assert!((p - Vec3::new(0., 1., 0.)).length() < 1e-5, "{p:?}");
        assert!(q.is_normalized());
        assert!(!c.additive);
    }
    #[test]
    fn scaled_translation_uses_bone_length_ratio_and_mesh_mode_keeps_bind_translation() {
        let r = Retarget { mode: 2, source: Vec3::Y };
        let p = retarget_pose(r, (Vec3::Y * 0.4, Quat::IDENTITY), (Vec3::Y * 2., Quat::IDENTITY), false, false);
        assert!((p.0.y - 0.8).abs() < 1e-6);
        let r1 = Retarget { mode: 1, source: Vec3::Y };
        let bind = (Vec3::Y * 2., Quat::IDENTITY);
        assert_eq!(retarget_pose(r1, (Vec3::X, Quat::IDENTITY), bind, false, false).0, bind.0);
        assert_eq!(retarget_pose(r1, (Vec3::X, Quat::IDENTITY), bind, true, false).0, Vec3::ZERO);
    }
    #[test]
    fn clip_sampling_interpolates_and_loops() {
        let c = Clip::parse(CLIP, "t").unwrap();
        let (a, _) = c.sample("pelvis", 0.).unwrap();
        let (b, _) = c.sample("pelvis", 0.5 / 60.).unwrap();
        let (l, _) = c.sample("pelvis", 2. / 60.).unwrap();
        assert!((b.y - 1.5).abs() < 1e-4, "{a:?} {b:?}");
        assert!((l - a).length() < 1e-5, "loops after 2 frames");
        assert!(c.sample("missing", 0.).is_none());
    }
    #[test]
    fn rest_pose_globals_chain_the_local_transforms() {
        let skel = SkelModel {
            names: vec!["a".into(), "b".into()], parents: vec![None, Some(0)],
            local: vec![(Vec3::new(1., 0., 0.), Quat::IDENTITY, Vec3::ONE), (Vec3::new(0., 2., 0.), Quat::IDENTITY, Vec3::ONE)],
            joints: vec![1], inverse: vec![Mat4::from_translation(Vec3::new(-1., -2., 0.))],
        };
        let g = skel.globals(None, None, 0.);
        assert!((g[1].transform_point3(Vec3::ZERO) - Vec3::new(1., 2., 0.)).length() < 1e-6);
        // skin matrix of a rest pose is the identity
        assert!(skel.skin(&g)[0].abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }
    #[test]
    fn make_rigid_binds_everything_to_joint_zero() {
        let mut p = vec![SkinnedPrimitive { vertices: vec![SkinnedVertex { joints: [3, 4, 5, 6], weights: [0.4; 4], ..Default::default() }], indices: vec![0, 0, 0], material: MaterialImages::default(), kind: MaterialKind::Prop }];
        make_rigid(&mut p);
        assert_eq!(p[0].vertices[0].joints, [0; 4]);
        assert_eq!(p[0].vertices[0].weights, [1., 0., 0., 0.]);
    }
}
