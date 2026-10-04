//! glTF 2.0 exporter producing the same files as the UEViewer (umodel, `-gltf`) exports the old runtime consumed.
//!
//! The text of the `.gltf` and the layout of the `.bin` follow `ExportGLTF.cpp` of the UEViewer fork exactly
//! (including `printf("%g")` number formatting), so an export of a mesh is byte-identical to the old one except for
//! the `generator` string. Conventions (all taken from that exporter):
//!
//! * **Axes / units**: glTF is right handed Y-up in metres, UE is left handed Z-up in centimetres. Positions are
//!   converted with `(x, y, z) -> (x, z, y) * 0.01`, directions with `(x, y, z) -> (x, z, y)` and bone rotations by
//!   swapping the quaternion's `y` and `z` components (no sign change).
//! * **Node layout** (skeletal): node 0 is the mesh node (`mesh 0`, `skin 0`, `children [1]`); nodes `1..=N` are the
//!   bones in *UEViewer sorted order* (parents before children, depth first, siblings by original index; see
//!   [`sort_bones`]); node `i + 1` is bone `i`, and `skins[0].joints` is `[1, 2, ..., N]`. Bone transforms are the
//!   local reference pose after UEViewer's `BAKE_BONE_SCALES` (translations multiplied by the accumulated scale of
//!   all ancestors) with every non-root bone's rotation conjugated (UE stores the inverse of the usual
//!   orientation) and then swapped to glTF axes. `inverseBindMatrices` are the inverted world bone coords.
//! * **Static mesh**: node 0 only (`mesh 0`).
//! * **Primitives**: one primitive per mesh section, material index = section index (`materials[i]` is named after
//!   the section's material, `dummy_material_<i>` when absent; the base colour is a debug colour derived from `i`).
//!   Each primitive has its own vertex set: the vertices referenced by the section in order of first use.
//!   Attributes: `POSITION`, `NORMAL`, `TANGENT` (xyz + sign in w, exactly +-1), optional `COLOR_0` (normalised u8
//!   VEC4, bytes as stored), skeletal `JOINTS_0` (u16 VEC4, mesh bone indices after bone sorting, unused slots 0) and
//!   `WEIGHTS_0` (normalised u8 VEC4, 4 strongest influences renormalised to sum 255), `TEXCOORD_n` (UV as stored, no
//!   V flip).
//! * **Buffer order** in the `.bin` (each buffer padded to 4 bytes): inverse bind matrices (skeletal), then per
//!   section: indices (u16, or u32 above 65536 vertices), positions, normals, tangents, [colours], [joints, weights],
//!   UV sets.
//!
//! Bone and skin processing mirrors `CSkeletalMesh::FinalizeMesh` (`SortBones`, weight clean-up/renormalisation) of
//! the UEViewer fork: only the first four influences of the engine's up-to-eight are exported.

use crate::skeletal::{SkeletalLod, SkeletalMesh};
use crate::static_mesh::{StaticLod, StaticMesh};
use fn_ueio::{Error, Result};
use std::fmt::Write as _;
use std::path::Path;

/// Convert a UE position (cm, Z-up, left handed) to the exporter's glTF convention (m, Y-up).
pub fn ue_to_gltf_position(p: [f32; 3]) -> [f32; 3] {
    [p[0] * 0.01f32, p[2] * 0.01f32, p[1] * 0.01f32]
}

/// Convert a UE direction to the glTF convention (swap y/z).
pub fn ue_to_gltf_direction(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[2], v[1]]
}

/// Convert a UE quaternion (x, y, z, w) to the glTF convention (swap y/z, as the UEViewer exporter does).
pub fn ue_to_gltf_rotation(q: [f32; 4]) -> [f32; 4] {
    [q[0], q[2], q[1], q[3]]
}

// ---------------------------------------------------------------------------------------------------------------
// C `printf("%g")` emulation
// ---------------------------------------------------------------------------------------------------------------

/// Format like C's `%g` (6 significant digits, trailing zeros removed, exponent form below 1e-4 / from 1e6).
pub fn fmt_g(v: f64) -> String {
    if v.is_nan() {
        return if v.is_sign_negative() { "-nan".into() } else { "nan".into() };
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf".into() } else { "inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    const P: i32 = 6;
    let e_form = format!("{:.*e}", (P - 1) as usize, v); // d.ddddde[-]x, correctly rounded
    let (mant, exp) = e_form.split_once('e').unwrap();
    let x: i32 = exp.parse().unwrap();
    if x < -4 || x >= P {
        let mut m = mant.to_string();
        if m.contains('.') {
            while m.ends_with('0') {
                m.pop();
            }
            if m.ends_with('.') {
                m.pop();
            }
        }
        format!("{}e{}{:02}", m, if x < 0 { '-' } else { '+' }, x.abs())
    } else {
        let decimals = (P - 1 - x).max(0) as usize;
        let mut s = format!("{:.*}", decimals, v);
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        s
    }
}

fn g(v: f32) -> String {
    fmt_g(v as f64)
}

fn json_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

// ---------------------------------------------------------------------------------------------------------------
// Little vector helpers replicating the C++ float arithmetic order (so results match bit for bit)
// ---------------------------------------------------------------------------------------------------------------

type V3 = [f32; 3];

fn dot(a: &V3, b: &V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize3(v: &mut V3) {
    let length = dot(v, v).sqrt();
    if length != 0.0 {
        let s = 1.0f32 / length;
        v[0] *= s;
        v[1] *= s;
        v[2] *= s;
    }
}

#[derive(Clone, Copy, Default)]
struct Coords {
    origin: V3,
    axis: [V3; 3],
}

fn vec_ma(a: &V3, scale: f32, b: &V3) -> V3 {
    [a[0] + scale * b[0], a[1] + scale * b[1], a[2] + scale * b[2]]
}

fn axis_untransform_vector(axis: &[V3; 3], src: &V3) -> V3 {
    let tmp = [axis[0][0] * src[0], axis[0][1] * src[0], axis[0][2] * src[0]]; // VectorScale: scale * a
    let tmp = vec_ma(&tmp, src[1], &axis[1]);
    vec_ma(&tmp, src[2], &axis[2])
}

fn coords_untransform(parent: &Coords, src: &Coords) -> Coords {
    let t = vec_ma(&parent.origin, src.origin[0], &parent.axis[0]);
    let t = vec_ma(&t, src.origin[1], &parent.axis[1]);
    let origin = vec_ma(&t, src.origin[2], &parent.axis[2]);
    let axis = [
        axis_untransform_vector(&parent.axis, &src.axis[0]),
        axis_untransform_vector(&parent.axis, &src.axis[1]),
        axis_untransform_vector(&parent.axis, &src.axis[2]),
    ];
    Coords { origin, axis }
}

fn invert_coords(s: &Coords) -> Coords {
    let o = [dot(&s.origin, &s.axis[0]), dot(&s.origin, &s.axis[1]), dot(&s.origin, &s.axis[2])];
    Coords {
        origin: [-o[0], -o[1], -o[2]],
        axis: [
            [s.axis[0][0], s.axis[1][0], s.axis[2][0]],
            [s.axis[0][1], s.axis[1][1], s.axis[2][1]],
            [s.axis[0][2], s.axis[1][2], s.axis[2][2]],
        ],
    }
}

/// `CQuat::ToAxis` for a quaternion (x, y, z, w).
fn quat_to_axis(q: [f32; 4]) -> [V3; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let x2 = x * 2.0;
    let y2 = y * 2.0;
    let z2 = z * 2.0;
    let xx = x * x2;
    let xy = x * y2;
    let xz = x * z2;
    let yy = y * y2;
    let yz = y * z2;
    let zz = z * z2;
    let wx = w * x2;
    let wy = w * y2;
    let wz = w * z2;
    [
        [1.0 - (yy + zz), xy - wz, xz + wy],
        [xy + wz, 1.0 - (xx + zz), yz - wx],
        [xz - wy, yz + wx, 1.0 - (xx + yy)],
    ]
}

// ---------------------------------------------------------------------------------------------------------------
// Bone processing (UEViewer: BAKE_BONE_SCALES + conjugation + SortBones)
// ---------------------------------------------------------------------------------------------------------------

/// Reorder a bone hierarchy the way UEViewer's `CSkeletalMesh::SortBones` does: depth first from the root, children
/// in original index order. Returns `(sorted_to_old, old_to_sorted)`.
pub fn sort_bones(parents: &[i32]) -> (Vec<usize>, Vec<usize>) {
    let n = parents.len();
    let mut order = Vec::with_capacity(n);
    if n == 0 {
        return (order, Vec::new());
    }
    order.push(0usize);
    fn rec(parent: usize, parents: &[i32], order: &mut Vec<usize>) {
        for i in 0..parents.len() {
            if i > 0 && parents[i] == parent as i32 && order.len() < parents.len() {
                order.push(i);
                rec(i, parents, order);
            }
        }
    }
    rec(0, parents, &mut order);
    // Bones not reachable from the root keep their relative order at the end (UEViewer would abort).
    if order.len() < n {
        for i in 0..n {
            if !order.contains(&i) {
                order.push(i);
            }
        }
    }
    let mut remap = vec![0usize; n];
    for (new, &old) in order.iter().enumerate() {
        remap[old] = new;
    }
    (order, remap)
}

/// Bone data in exporter (UEViewer-sorted) order, in UE units with UEViewer's bone orientation convention.
struct ExportBone {
    name: String,
    parent: usize,
    pos: V3,
    /// UEViewer `CSkelMeshBone::Orientation` (conjugated for non-root bones).
    rot: [f32; 4],
}

fn export_bones(mesh: &SkeletalMesh) -> (Vec<ExportBone>, Vec<usize>) {
    let n = mesh.ref_skeleton.len();
    // BAKE_BONE_SCALES + conjugate (bones >= 1)
    let mut pos = Vec::with_capacity(n);
    let mut rot = Vec::with_capacity(n);
    for (i, b) in mesh.ref_skeleton.iter().enumerate() {
        let mut scale = [1.0f32; 3];
        let mut p = b.parent;
        let mut guard = 0;
        while p >= 0 && (p as usize) < n && guard < n {
            let s = mesh.ref_skeleton[p as usize].ref_scale;
            scale[0] *= s[0];
            scale[1] *= s[1];
            scale[2] *= s[2];
            p = mesh.ref_skeleton[p as usize].parent;
            guard += 1;
        }
        pos.push([b.ref_pos[0] * scale[0], b.ref_pos[1] * scale[1], b.ref_pos[2] * scale[2]]);
        let mut q = b.ref_rot;
        if i >= 1 {
            q = [-q[0], -q[1], -q[2], q[3]];
        }
        rot.push(q);
    }
    let parents: Vec<i32> = mesh.ref_skeleton.iter().map(|b| b.parent).collect();
    let (order, remap) = sort_bones(&parents);
    let bones = order
        .iter()
        .map(|&old| {
            let b = &mesh.ref_skeleton[old];
            let parent = if b.parent > 0 { remap[b.parent as usize] } else { 0 };
            ExportBone { name: b.name.clone(), parent, pos: pos[old], rot: rot[old] }
        })
        .collect();
    (bones, remap)
}

// ---------------------------------------------------------------------------------------------------------------
// Export vertex model shared by skeletal and static meshes
// ---------------------------------------------------------------------------------------------------------------

struct ExpSection {
    first_index: usize,
    num_faces: usize,
    material: Option<String>,
}

struct Skin {
    /// Sorted-bone indices, `-1` terminates the list.
    bones: Vec<[i16; 4]>,
    /// Packed 4 x u8 weights.
    weights: Vec<u32>,
    bone_nodes: Vec<(String, V3, [f32; 4], usize)>,
    inverse_bind: Vec<[f32; 16]>,
}

/// Export options.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Reproduce UEViewer's `half2float` bug for half precision UVs: it ignores the denormal/zero encoding, so a
    /// half `0` becomes `2^-15` (3.0517578e-5) and denormals are offset the same way. The default (`false`) converts
    /// correctly; the difference is at most 3.1e-5 in UV space. Only used to prove bit-exactness against old exports.
    pub legacy_uev_half_uv: bool,
}

/// UEViewer's naive half->float on an already (correctly) widened half value.
pub(crate) fn legacy_half(v: f32) -> f32 {
    let bits = v.to_bits();
    let sign = bits & 0x8000_0000;
    let e = (bits >> 23) & 0xff;
    // correct halves: zero/denormal -> f32 exponent field <= 112 (< 2^-14) ; normal halves have e >= 113
    if e >= 113 || v.is_nan() || v.is_infinite() {
        return v;
    }
    let mant = (v.abs() * 16777216.0f32) as u32; // |v| = mant * 2^-24 for denormal halves (mant < 1024)
    f32::from_bits(sign | (112u32 << 23) | (mant << 13))
}

struct ExpLod<'a> {
    legacy_half: bool,
    positions: &'a [[f32; 3]],
    tangent_x: &'a [[u8; 4]],
    tangent_z: &'a [[u8; 4]],
    uvs: &'a [Vec<[f32; 2]>],
    colors: &'a [[u8; 4]],
    indices: &'a [u32],
    sections: Vec<ExpSection>,
    skin: Option<Skin>,
}

/// `CPackedNormal` -> float4 as UEViewer's `Unpack(CVec4&, CPackedNormal)`.
fn unpack4(p: [u8; 4]) -> [f32; 4] {
    [crate::unpack_byte_uev(p[0]), crate::unpack_byte_uev(p[1]), crate::unpack_byte_uev(p[2]), crate::unpack_byte_uev(p[3])]
}

struct Buffer {
    data: Vec<u8>,
    count: usize,
    ty: &'static str,
    component: u32,
    normalized: bool,
    min: Option<String>,
    max: Option<String>,
}

impl Buffer {
    fn new(count: usize, ty: &'static str, component: u32, item_size: usize, normalized: bool) -> Buffer {
        Buffer { data: Vec::with_capacity(count * item_size), count, ty, component, normalized, min: None, max: None }
    }
    fn f32s(&mut self, v: &[f32]) {
        for x in v {
            self.data.extend_from_slice(&x.to_le_bytes());
        }
    }
    fn finish(&mut self) {
        while self.data.len() % 4 != 0 {
            self.data.push(0);
        }
    }
}

fn material_debug_color(index: usize) -> [f32; 3] {
    const TABLE: [f32; 4] = [0.9, 0.3, 0.6, 1.0];
    const TABLE2: [usize; 8] = [0, 1, 2, 4, 7, 3, 5, 6];
    let index = (index & 0xFFF8) | (TABLE2[index & 7] ^ 7);
    let c = |x: usize| (x & 1) | ((x >> 2) & 2);
    [TABLE[c(index)], TABLE[c(index >> 1)], TABLE[c(index >> 2)]]
}

fn write_material(out: &mut String, name: &Option<String>, index: usize, last: bool) {
    let col = material_debug_color(index);
    let name = match name {
        Some(n) => n.clone(),
        None => format!("dummy_material_{index}"),
    };
    let _ = write!(
        out,
        "    {{\n      \"name\" : \"{}\",\n      \"pbrMetallicRoughness\" : {{\n        \"baseColorFactor\" : [ {}, {}, {}, 1.0 ],\n        \"metallicFactor\" : 0.1,\n        \"roughnessFactor\" : 0.5\n      }}\n    }}{}\n",
        json_str(&name),
        g(col[0]),
        g(col[1]),
        g(col[2]),
        if last { "" } else { "," }
    );
}

/// Section export: builds the buffers, returns the primitive JSON.
fn export_section(lod: &ExpLod, section_index: usize, buffers: &mut Vec<Buffer>, out: &mut String) {
    let s = &lod.sections[section_index];
    let n_idx = s.num_faces * 3;
    let get = |i: usize| lod.indices.get(s.first_index + i).copied().unwrap_or(0) as usize;

    // Remap section indices to local indices
    let mut remap = vec![-1i64; lod.positions.len()];
    let mut local_verts = 0usize;
    for i in 0..n_idx {
        let v = get(i);
        if remap[v] == -1 {
            remap[v] = local_verts as i64;
            local_verts += 1;
        }
    }
    let mut rev = vec![0usize; local_verts];
    for (old, &new) in remap.iter().enumerate() {
        if new != -1 {
            rev[new as usize] = old;
        }
    }

    let index_buf = buffers.len();
    let mut ib = if local_verts <= 65536 { Buffer::new(n_idx, "SCALAR", 5123, 2, false) } else { Buffer::new(n_idx, "SCALAR", 5125, 4, false) };
    for i in 0..n_idx {
        let nv = remap[get(i)] as u32;
        if local_verts <= 65536 {
            ib.data.extend_from_slice(&(nv as u16).to_le_bytes());
        } else {
            ib.data.extend_from_slice(&nv.to_le_bytes());
        }
    }
    ib.finish();
    buffers.push(ib);

    let mut pb = Buffer::new(local_verts, "VEC3", 5126, 12, false);
    let mut nb = Buffer::new(local_verts, "VEC3", 5126, 12, false);
    let mut tb = Buffer::new(local_verts, "VEC4", 5126, 16, false);
    let mut mins = [0.0f32; 3];
    let mut maxs = [0.0f32; 3];
    for (k, &old) in rev.iter().enumerate() {
        let p = lod.positions[old];
        let mut pos = [p[0], p[2], p[1]];
        pos[0] *= 0.01f32;
        pos[1] *= 0.01f32;
        pos[2] *= 0.01f32;

        let n4 = unpack4(lod.tangent_z[old]);
        let mut t4 = unpack4(lod.tangent_x[old]);
        // glTF keeps the binormal sign in tangent.w
        t4[3] = if n4[3] < 0.0 { -1.0 } else { 1.0 };
        let mut nrm = [n4[0], n4[2], n4[1]];
        let mut tan = [t4[0], t4[2], t4[1]];
        normalize3(&mut nrm);
        normalize3(&mut tan);

        pb.f32s(&pos);
        nb.f32s(&nrm);
        tb.f32s(&[tan[0], tan[1], tan[2], t4[3]]);
        if k == 0 {
            mins = pos;
            maxs = pos;
        } else {
            for c in 0..3 {
                mins[c] = mins[c].min(pos[c]);
                maxs[c] = maxs[c].max(pos[c]);
            }
        }
    }
    pb.min = Some(format!("[ {}, {}, {} ]", g(mins[0]), g(mins[1]), g(mins[2])));
    pb.max = Some(format!("[ {}, {}, {} ]", g(maxs[0]), g(maxs[1]), g(maxs[2])));
    let pos_buf = buffers.len();
    let nrm_buf = pos_buf + 1;
    let tan_buf = pos_buf + 2;
    for mut b in [pb, nb, tb] {
        b.finish();
        buffers.push(b);
    }

    let mut next = pos_buf + 3;
    let mut color_buf = None;
    if !lod.colors.is_empty() {
        let mut cb = Buffer::new(local_verts, "VEC4", 5121, 4, true);
        for &old in &rev {
            // file order is FColor's memory layout (B, G, R, A); the exporter writes R, G, B, A
            let c = lod.colors[old];
            cb.data.extend_from_slice(&[c[2], c[1], c[0], c[3]]);
        }
        cb.finish();
        buffers.push(cb);
        color_buf = Some(next);
        next += 1;
    }
    let mut skin_bufs = None;
    if let Some(skin) = &lod.skin {
        let mut jb = Buffer::new(local_verts, "VEC4", 5123, 8, false);
        let mut wb = Buffer::new(local_verts, "VEC4", 5121, 4, true);
        for &old in &rev {
            for &b in &skin.bones[old] {
                let b = if b < 0 { 0u16 } else { b as u16 };
                jb.data.extend_from_slice(&b.to_le_bytes());
            }
            wb.data.extend_from_slice(&skin.weights[old].to_le_bytes());
        }
        jb.finish();
        wb.finish();
        buffers.push(jb);
        buffers.push(wb);
        skin_bufs = Some((next, next + 1));
        next += 2;
    }
    let mut uv_bufs = Vec::new();
    for uvset in lod.uvs {
        let mut ub = Buffer::new(local_verts, "VEC2", 5126, 8, false);
        for &old in &rev {
            let uv = uvset[old];
            if lod.legacy_half {
                ub.f32s(&[legacy_half(uv[0]), legacy_half(uv[1])]);
            } else {
                ub.f32s(&uv);
            }
        }
        ub.finish();
        buffers.push(ub);
        uv_bufs.push(next);
        next += 1;
    }

    let _ = write!(
        out,
        "        {{\n          \"attributes\" : {{\n            \"POSITION\" : {pos_buf},\n            \"NORMAL\" : {nrm_buf},\n            \"TANGENT\" : {tan_buf},\n"
    );
    if let Some(c) = color_buf {
        let _ = write!(out, "            \"COLOR_0\" : {c},\n");
    }
    if let Some((j, w)) = skin_bufs {
        let _ = write!(out, "            \"JOINTS_0\" : {j},\n            \"WEIGHTS_0\" : {w},\n");
    }
    for (i, &u) in uv_bufs.iter().enumerate() {
        let _ = write!(out, "            \"TEXCOORD_{i}\" : {u}{}\n", if i + 1 < uv_bufs.len() { "," } else { "" });
    }
    let _ = write!(
        out,
        "          }},\n          \"indices\" : {index_buf},\n          \"material\" : {section_index}\n        }}{}\n",
        if section_index + 1 < lod.sections.len() { "," } else { "" }
    );
}

fn write_skin(out: &mut String, skin: &Skin, mesh_name: &str, matrix_buf: usize) {
    let n = skin.bone_nodes.len();
    let _ = write!(out, "  \"nodes\" : [\n    {{\n      \"name\" : \"{}\",\n      \"mesh\" : 0,\n      \"skin\" : 0,\n      \"children\" : [ 1 ]\n    }},\n", json_str(mesh_name));
    for (i, (name, pos, rot, _parent)) in skin.bone_nodes.iter().enumerate() {
        let _ = write!(out, "    {{\n      \"name\" : \"{}\",\n", json_str(name));
        let children: Vec<usize> = (0..n).filter(|&j| j != i && skin.bone_nodes[j].3 == i).collect();
        if !children.is_empty() {
            let list: Vec<String> = children.iter().map(|c| (c + 1).to_string()).collect();
            let _ = write!(out, "      \"children\" : [ {} ],\n", list.join(", "));
        }
        let _ = write!(
            out,
            "      \"translation\" : [ {}, {}, {} ],\n      \"rotation\" : [ {}, {}, {}, {} ]\n",
            g(pos[0]),
            g(pos[1]),
            g(pos[2]),
            g(rot[0]),
            g(rot[1]),
            g(rot[2]),
            g(rot[3])
        );
        let _ = write!(out, "    }}{}\n", if i + 1 == n { "" } else { "," });
    }
    out.push_str("  ],\n");
    let _ = write!(out, "  \"skins\" : [\n    {{\n      \"inverseBindMatrices\" : {matrix_buf},\n      \"skeleton\" : 1,\n      \"joints\" : [");
    for i in 0..n {
        if i & 31 == 0 {
            out.push_str("\n        ");
        }
        let _ = write!(out, "{}{}", i + 1, if i == n - 1 { "" } else { "," });
    }
    out.push_str("\n      ]\n    }\n  ],\n");
}

/// Check that every section's index range exists and refers to existing vertices (the exporter indexes unchecked).
fn validate(lod: &ExpLod) -> Result<()> {
    let n = lod.positions.len();
    if lod.tangent_x.len() != n || lod.tangent_z.len() != n || lod.uvs.iter().any(|u| u.len() != n) || (!lod.colors.is_empty() && lod.colors.len() != n) {
        return Err(Error::Invalid("vertex streams have different lengths".into()));
    }
    if let Some(s) = &lod.skin {
        if s.bones.len() != n || s.weights.len() != n {
            return Err(Error::Invalid("skin weight stream length differs from the vertex count".into()));
        }
    }
    for (i, s) in lod.sections.iter().enumerate() {
        let end = s.first_index + s.num_faces * 3;
        if end > lod.indices.len() {
            return Err(Error::Invalid(format!("section {i}: index range {}..{end} exceeds the index buffer ({})", s.first_index, lod.indices.len())));
        }
        if lod.indices[s.first_index..end].iter().any(|&v| v as usize >= n) {
            return Err(Error::Invalid(format!("section {i}: index outside the {n} vertices")));
        }
    }
    Ok(())
}

/// Write the whole document for one LOD.
fn write_document(mesh_name: &str, lod: &ExpLod) -> (String, Vec<u8>) {
    let mut buffers: Vec<Buffer> = Vec::new();
    let mut out = String::new();
    out.push_str("{\n  \"asset\" : {\n    \"generator\" : \"fn-mesh (UE4.16 decoder, UEViewer-compatible glTF)\",\n    \"version\" : \"2.0\"\n  },\n");
    out.push_str("  \"scene\" : 0,\n  \"scenes\" : [\n    {\n      \"nodes\" : [ 0 ]\n    }\n  ],\n");

    if let Some(skin) = &lod.skin {
        let mut mb = Buffer::new(skin.inverse_bind.len(), "MAT4", 5126, 64, false);
        for m in &skin.inverse_bind {
            mb.f32s(m);
        }
        mb.finish();
        buffers.push(mb);
        write_skin(&mut out, skin, mesh_name, 0);
    } else {
        let _ = write!(out, "  \"nodes\" : [\n    {{\n      \"name\" : \"{}\",\n      \"mesh\" : 0\n    }}\n  ],\n", json_str(mesh_name));
    }

    out.push_str("  \"materials\" : [\n");
    for (i, s) in lod.sections.iter().enumerate() {
        write_material(&mut out, &s.material, i, i + 1 == lod.sections.len());
    }
    out.push_str("  ],\n");

    out.push_str("  \"meshes\" : [\n    {\n      \"primitives\" : [\n");
    for i in 0..lod.sections.len() {
        export_section(lod, i, &mut buffers, &mut out);
    }
    let _ = write!(out, "      ],\n      \"name\" : \"{}\"\n    }}\n  ],\n", json_str(mesh_name));

    let total: usize = buffers.iter().map(|b| b.data.len()).sum();
    let _ = write!(out, "  \"buffers\" : [\n    {{\n      \"uri\" : \"{}.bin\",\n      \"byteLength\" : {total}\n    }}\n  ],\n", json_str(mesh_name));
    out.push_str("  \"bufferViews\" : [\n");
    let mut off = 0usize;
    for (i, b) in buffers.iter().enumerate() {
        let _ = write!(out, "    {{\n      \"buffer\" : 0,\n      \"byteOffset\" : {off},\n      \"byteLength\" : {}\n    }}{}\n", b.data.len(), if i + 1 == buffers.len() { "" } else { "," });
        off += b.data.len();
    }
    out.push_str("  ],\n  \"accessors\" : [\n");
    for (i, b) in buffers.iter().enumerate() {
        let _ = write!(out, "    {{\n      \"bufferView\" : {i},\n");
        if b.normalized {
            out.push_str("      \"normalized\" : true,\n");
        }
        if let (Some(mn), Some(mx)) = (&b.min, &b.max) {
            let _ = write!(out, "      \"min\" : {mn},\n      \"max\" : {mx},\n");
        }
        let _ = write!(out, "      \"componentType\" : {},\n      \"count\" : {},\n      \"type\" : \"{}\"\n    }}{}\n", b.component, b.count, b.ty, if i + 1 == buffers.len() { "" } else { "," });
    }
    out.push_str("  ]\n}\n");

    let mut bin = Vec::with_capacity(total);
    for b in &buffers {
        bin.extend_from_slice(&b.data);
    }
    (out, bin)
}

// ---------------------------------------------------------------------------------------------------------------
// Skeletal mesh
// ---------------------------------------------------------------------------------------------------------------

fn build_skin(mesh: &SkeletalMesh, lod: &SkeletalLod) -> Result<Skin> {
    if lod.influences.len() != lod.num_vertices || lod.sections.is_empty() {
        return Err(Error::Invalid("LOD has no skin weights / sections".into()));
    }
    if mesh.ref_skeleton.is_empty() {
        return Err(Error::Invalid("mesh has no reference skeleton".into()));
    }
    let (bones, remap) = export_bones(mesh);

    // Skeleton nodes + inverse bind matrices
    let mut coords: Vec<Coords> = Vec::with_capacity(bones.len());
    let mut inverse_bind = Vec::with_capacity(bones.len());
    let mut bone_nodes = Vec::with_capacity(bones.len());
    for (i, b) in bones.iter().enumerate() {
        let mut rot = b.rot;
        if i == 0 {
            rot = [-rot[0], -rot[1], -rot[2], rot[3]];
        }
        let bone_pos = [b.pos[0], b.pos[2], b.pos[1]];
        let bone_pos = [bone_pos[0] * 0.01f32, bone_pos[1] * 0.01f32, bone_pos[2] * 0.01f32];
        let rot = [rot[0], rot[2], rot[1], rot[3]]; // swap y/z
        bone_nodes.push((b.name.clone(), bone_pos, rot, b.parent));
        let rot_w = [rot[0], rot[1], rot[2], rot[3] * -1.0];
        let mut bc = Coords { origin: bone_pos, axis: quat_to_axis(rot_w) };
        if i != 0 {
            bc = coords_untransform(&coords[b.parent], &bc);
        }
        coords.push(bc);
        let inv = invert_coords(&bc);
        let a = inv.axis;
        let o = inv.origin;
        inverse_bind.push([a[0][0], a[0][1], a[0][2], 0.0, a[1][0], a[1][1], a[1][2], 0.0, a[2][0], a[2][1], a[2][2], 0.0, o[0], o[1], o[2], 1.0]);
    }

    // Vertex influences: CSkelMeshLod conversion + SortBones remap + FinalizeMesh weight fix
    let sections = lod.vertex_sections();
    let mut out_bones = Vec::with_capacity(lod.num_vertices);
    let mut out_weights = Vec::with_capacity(lod.num_vertices);
    for v in 0..lod.num_vertices {
        let sec = &lod.sections[sections[v]];
        let inf = &lod.influences[v];
        let mut bone = [-1i16; 4];
        let mut w = [0u8; 4];
        let mut i2 = 0;
        for i in 0..4 {
            if inf.weights[i] == 0 {
                continue;
            }
            w[i2] = inf.weights[i];
            bone[i2] = sec.bone_map.get(inf.bones[i] as usize).copied().unwrap_or(0) as i16;
            i2 += 1;
        }
        // SortBones remap
        for b in bone.iter_mut() {
            if *b < 0 {
                break;
            }
            *b = remap.get(*b as usize).copied().unwrap_or(0) as i16;
        }
        // FinalizeMesh: drop zero / duplicate influences
        let mut should_fix = false;
        for i in 0..4 {
            let b = bone[i];
            if b < 0 {
                break;
            }
            if w[i] == 0 {
                should_fix = true;
                continue;
            }
            for k in 0..i {
                if bone[k] == b {
                    let nw = (w[i] as i32 + w[k] as i32).min(255);
                    w[i] = (nw & 0xFF) as u8;
                    w[k] = 0;
                    should_fix = true;
                }
            }
        }
        if should_fix {
            for i in (0..4).rev() {
                if w[i] == 0 {
                    if i < 3 {
                        for j in i..3 {
                            w[j] = w[j + 1];
                            bone[j] = bone[j + 1];
                        }
                    }
                    w[3] = 0;
                    bone[3] = -1;
                }
            }
        }
        // renormalise to 255
        let mut total = 0i32;
        let mut num = 0;
        while num < 4 {
            if bone[num] < 0 {
                break;
            }
            total += w[num] as i32;
            num += 1;
        }
        if total != 255 && total > 0 {
            let scale = 255.0f32 / total as f32;
            total = 0;
            for i in 0..num {
                let x = w[i] as f32 * scale;
                let r = (if x >= 0.0 { x + 0.5f32 } else { x - 0.5f32 }) as i32;
                w[i] = r as u8;
                total += w[i] as i32;
            }
            let delta = 255 - total;
            w[0] = (w[0] as i32 + delta) as u8;
        }
        out_bones.push(bone);
        out_weights.push(u32::from_le_bytes(w));
    }
    Ok(Skin { bones: out_bones, weights: out_weights, bone_nodes, inverse_bind })
}

fn skeletal_lod_doc(mesh: &SkeletalMesh, lod_index: usize, name: &str, opts: &Options) -> Result<(String, Vec<u8>)> {
    let lod = mesh.lods.get(lod_index).ok_or_else(|| Error::Invalid(format!("mesh has no LOD {lod_index}")))?;
    if lod.indices.is_empty() || lod.positions.is_empty() {
        return Err(Error::Invalid(format!("LOD {lod_index} has no geometry")));
    }
    let sections = lod
        .sections
        .iter()
        .enumerate()
        .map(|(i, s)| ExpSection {
            first_index: s.base_index as usize,
            num_faces: s.num_triangles as usize,
            material: mesh.materials.get(mesh.section_material_index(lod_index, i)).and_then(|m| m.material_name.clone()),
        })
        .collect();
    let exp = ExpLod {
        legacy_half: opts.legacy_uev_half_uv && !lod.uv_full_precision,
        positions: &lod.positions,
        tangent_x: &lod.tangent_x,
        tangent_z: &lod.tangent_z,
        uvs: &lod.uvs,
        colors: &lod.colors,
        indices: &lod.indices,
        sections,
        skin: Some(build_skin(mesh, lod)?),
    };
    validate(&exp)?;
    Ok(write_document(name, &exp))
}

/// Convert LOD 0 of a skeletal mesh to glTF text and binary (UEViewer-compatible layout, see the module docs).
///
/// A mesh without geometry yields an empty document; use [`skeletal_mesh_lod_to_gltf`] to get the error instead.
pub fn skeletal_mesh_to_gltf(mesh: &SkeletalMesh, name: &str) -> (String, Vec<u8>) {
    skeletal_lod_doc(mesh, 0, name, &Options::default()).unwrap_or_default()
}

/// Convert one LOD of a skeletal mesh.
pub fn skeletal_mesh_lod_to_gltf(mesh: &SkeletalMesh, lod: usize, name: &str) -> Result<(String, Vec<u8>)> {
    skeletal_lod_doc(mesh, lod, name, &Options::default())
}

/// Convert one LOD of a skeletal mesh with explicit [`Options`].
pub fn skeletal_mesh_lod_to_gltf_with(mesh: &SkeletalMesh, lod: usize, name: &str, opts: &Options) -> Result<(String, Vec<u8>)> {
    skeletal_lod_doc(mesh, lod, name, opts)
}

fn write_files(dir: &Path, name: &str, doc: (String, Vec<u8>)) -> Result<()> {
    std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(format!("create {}: {e}", dir.display())))?;
    let gltf = dir.join(format!("{name}.gltf"));
    let bin = dir.join(format!("{name}.bin"));
    std::fs::write(&gltf, doc.0.as_bytes()).map_err(|e| Error::Invalid(format!("write {}: {e}", gltf.display())))?;
    std::fs::write(&bin, &doc.1).map_err(|e| Error::Invalid(format!("write {}: {e}", bin.display())))?;
    Ok(())
}

/// Export LOD 0 of a skeletal mesh as `<dir>/<name>.gltf` + `<dir>/<name>.bin`.
pub fn export_skeletal_mesh(mesh: &SkeletalMesh, dir: &Path, name: &str) -> Result<()> {
    let doc = skeletal_lod_doc(mesh, 0, name, &Options::default())?;
    write_files(dir, name, doc)
}

// ---------------------------------------------------------------------------------------------------------------
// Static mesh
// ---------------------------------------------------------------------------------------------------------------

fn static_lod_doc(mesh: &StaticMesh, lod_index: usize, name: &str, opts: &Options) -> Result<(String, Vec<u8>)> {
    let lod: &StaticLod = mesh.lods.get(lod_index).ok_or_else(|| Error::Invalid(format!("mesh has no LOD {lod_index}")))?;
    if lod.indices.is_empty() || lod.positions.is_empty() {
        return Err(Error::Invalid(format!("LOD {lod_index} has no geometry")));
    }
    let sections = lod
        .sections
        .iter()
        .map(|s| ExpSection {
            first_index: s.first_index as usize,
            num_faces: s.num_triangles as usize,
            material: mesh.materials.get(s.material_index.max(0) as usize).and_then(|m| m.material_name.clone()),
        })
        .collect();
    let exp = ExpLod { legacy_half: opts.legacy_uev_half_uv && !lod.uv_full_precision, positions: &lod.positions, tangent_x: &lod.tangent_x, tangent_z: &lod.tangent_z, uvs: &lod.uvs, colors: &lod.colors, indices: &lod.indices, sections, skin: None };
    validate(&exp)?;
    Ok(write_document(name, &exp))
}

/// Convert LOD 0 of a static mesh to glTF text and binary.
pub fn static_mesh_to_gltf(mesh: &StaticMesh, name: &str) -> (String, Vec<u8>) {
    static_lod_doc(mesh, 0, name, &Options::default()).unwrap_or_default()
}

/// Convert one LOD of a static mesh.
pub fn static_mesh_lod_to_gltf(mesh: &StaticMesh, lod: usize, name: &str) -> Result<(String, Vec<u8>)> {
    static_lod_doc(mesh, lod, name, &Options::default())
}

/// Convert one LOD of a static mesh with explicit [`Options`].
pub fn static_mesh_lod_to_gltf_with(mesh: &StaticMesh, lod: usize, name: &str, opts: &Options) -> Result<(String, Vec<u8>)> {
    static_lod_doc(mesh, lod, name, opts)
}

/// Export LOD 0 of a static mesh as `<dir>/<name>.gltf` + `<dir>/<name>.bin`.
pub fn export_static_mesh(mesh: &StaticMesh, dir: &Path, name: &str) -> Result<()> {
    let doc = static_lod_doc(mesh, 0, name, &Options::default())?;
    write_files(dir, name, doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printf_g() {
        assert_eq!(fmt_g(0.0), "0");
        assert_eq!(fmt_g(-0.0), "-0");
        assert_eq!(fmt_g(1.0), "1");
        assert_eq!(fmt_g(0.3f32 as f64), "0.3");
        assert_eq!(fmt_g(0.931191f32 as f64), "0.931191");
        assert_eq!(fmt_g(1.24006e-16), "1.24006e-16");
        assert_eq!(fmt_g(-5.30405e-15), "-5.30405e-15");
        assert_eq!(fmt_g(100000.0), "100000");
        assert_eq!(fmt_g(1234567.0), "1.23457e+06");
        assert_eq!(fmt_g(0.0001), "0.0001");
        assert_eq!(fmt_g(0.00001234), "1.234e-05");
        assert_eq!(fmt_g(123456.5), "123456");
        assert_eq!(fmt_g(1.0e30), "1e+30");
    }

    #[test]
    fn debug_colors() {
        assert_eq!(material_debug_color(0), [0.3, 0.3, 0.3]);
        assert_eq!(material_debug_color(1), [0.9, 0.3, 0.3]);
        assert_eq!(material_debug_color(2), [0.3, 0.9, 0.3]);
    }

    #[test]
    fn bone_sort_is_depth_first() {
        // 0 root; 1,2 children of 0; 3 child of 1
        let (order, remap) = sort_bones(&[-1, 0, 0, 1]);
        assert_eq!(order, vec![0, 1, 3, 2]);
        assert_eq!(remap, vec![0, 1, 3, 2]);
    }
}

