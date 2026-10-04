//! Per-frame constant data of the original gun programs: the `Primitive` and `PrimitiveFade` uniform buffers, the GPU-skin vertex factory's loose parameters and the bone
//! matrices, all in Unreal conventions (cm, Z up, row-vector matrices) derived from the renderer's matrices (metres, Y up).
//!
//! Spaces: `C` maps UE mesh space (cm) to the port's glTF mesh space (the renderer's model space): `(x, y, z) -> (x, z, y) * 0.01`. The renderer draws the gun with a
//! matrix `G` (glTF mesh space -> port world), so the original programs get
//! * `LocalToWorld = C^-1 * G * C ... ` expressed from UE mesh space to UE world: `W * G * C` with `W = C^-1` (port world -> UE world, `(x, y, z) -> (x, z, y) * 100`),
//! * bone matrices `C^-1 * M_gltf * C` (the renderer's skin matrices act on glTF-space positions).
use glam::{Mat4, Vec3, Vec4};

/// UE mesh space (cm, Z up) -> port glTF space (m, Y up): `(x, y, z) -> (x, z, y) * 0.01`.
pub fn ue_to_port() -> Mat4 { Mat4::from_cols(Vec4::new(0.01, 0.0, 0.0, 0.0), Vec4::new(0.0, 0.0, 0.01, 0.0), Vec4::new(0.0, 0.01, 0.0, 0.0), Vec4::W) }
/// Port (m, Y up) -> UE (cm, Z up): `(x, y, z) -> (x, z, y) * 100`.
pub fn port_to_ue() -> Mat4 { Mat4::from_cols(Vec4::new(100.0, 0.0, 0.0, 0.0), Vec4::new(0.0, 0.0, 100.0, 0.0), Vec4::new(0.0, 100.0, 0.0, 0.0), Vec4::W) }

/// `LocalToWorld` of the original programs (UE mesh space -> UE world) from the renderer's model matrix `g` (port glTF mesh space -> port world).
pub fn local_to_world_ue(g: Mat4) -> Mat4 { port_to_ue() * g * ue_to_port() }

fn rows3(m: &Mat4) -> [[f32; 4]; 3] { std::array::from_fn(|i| { let c = [m.x_axis, m.y_axis, m.z_axis, m.w_axis]; [c[0][i], c[1][i], c[2][i], c[3][i]] }) }

pub const PRIMITIVE_FLOAT4S: usize = 16;
/// The `Primitive` uniform buffer (slots read by the recovered programs: 0..3 `LocalToWorld` rows, 8 `ObjectWorldPositionAndRadius`, 9 `ObjectOrientation` +
/// `LocalToWorldDeterminantSign` in `.w`, 10 `ActorWorldPosition`, 14 the per-axis inverse scale of the rows; 4..7 `WorldToLocal` filled for completeness).
/// `sphere` = bounding sphere of the mesh (UE local cm: centre, radius).
pub fn primitive_buffer(l2w: &Mat4, sphere: ([f32; 3], f32)) -> Vec<[f32; 4]> {
    let mut v = vec![[0f32; 4]; PRIMITIVE_FLOAT4S];
    // row-vector convention: row i is the image of local basis vector i, row 3 the translation
    let cols = [l2w.x_axis, l2w.y_axis, l2w.z_axis, l2w.w_axis];
    for i in 0..3 { v[i] = [cols[i].x, cols[i].y, cols[i].z, 0.0]; }
    v[3] = [cols[3].x, cols[3].y, cols[3].z, 1.0];
    let inv = l2w.inverse();
    let icols = [inv.x_axis, inv.y_axis, inv.z_axis, inv.w_axis];
    for i in 0..3 { v[4 + i] = [icols[i].x, icols[i].y, icols[i].z, 0.0]; }
    v[7] = [icols[3].x, icols[3].y, icols[3].z, 1.0];
    let scale = [cols[0].truncate().length(), cols[1].truncate().length(), cols[2].truncate().length()];
    let centre = l2w.transform_point3(Vec3::from(sphere.0));
    v[8] = [centre.x, centre.y, centre.z, sphere.1 * scale.iter().cloned().fold(0.0, f32::max)];
    let det = l2w.determinant();
    let orient = cols[2].truncate().normalize_or_zero();
    v[9] = [orient.x, orient.y, orient.z, if det < 0.0 { -1.0 } else { 1.0 }];
    v[10] = [cols[3].x, cols[3].y, cols[3].z, 0.0];
    v[14] = [1.0 / scale[0].max(1e-12), 1.0 / scale[1].max(1e-12), 1.0 / scale[2].max(1e-12), 0.0];
    v
}

/// `PrimitiveFade`: `FadeTimeScaleBias` = (0, 1): the primitive is not fading (the program computes `saturate(time * x + y)` = 1 = fully present).
pub fn fade_buffer() -> Vec<[f32; 4]> { vec![[0.0, 1.0, 0.0, 0.0]] }

pub const VF_FLOAT4S: usize = 9;
/// The GPU-skin vertex factory's loose parameters (the unnamed buffer 0 of the vertex program): 2..5 `PreviousLocalToWorld` (rows + translation, set to the current
/// transform: only the VELOCITY output reads them), 6 `MeshOrigin`, 7 `MeshExtension` (xyz) + the previous-bone-matrices flag (`.w` = 0: no motion blur data), 8.x motion blur scale 0.
pub fn vf_buffer(l2w: &Mat4, mesh_origin: [f32; 3], mesh_extension: [f32; 3]) -> Vec<[f32; 4]> {
    let mut v = vec![[0f32; 4]; VF_FLOAT4S];
    let cols = [l2w.x_axis, l2w.y_axis, l2w.z_axis, l2w.w_axis];
    for i in 0..4 { v[2 + i] = [cols[i].x, cols[i].y, cols[i].z, if i == 3 { 1.0 } else { 0.0 }]; }
    v[6] = [mesh_origin[0], mesh_origin[1], mesh_origin[2], 0.0];
    v[7] = [mesh_extension[0], mesh_extension[1], mesh_extension[2], 0.0];
    v
}

/// Bone matrices of one section in the vertex factory's `float3x4` rows (`BoneMatrices[3 * b + r]`, column-vector convention `p' = M p`), indexed by the SECTION-LOCAL
/// bone index. `skin` = the renderer's skin matrices (glTF space, joint-slot order), `slots[b]` the slot of section bone `b`; missing slots stay identity.
pub fn bone_buffer(skin: &[Mat4], slots: &[Option<usize>], vec4s: usize) -> Vec<[f32; 4]> {
    let (to_port, to_ue) = (ue_to_port(), port_to_ue());
    let mut out = vec![[0f32; 4]; vec4s];
    for b in 0..vec4s / 3 {
        let m = match slots.get(b).copied().flatten().and_then(|s| skin.get(s)) { Some(m) => to_ue * *m * to_port, None => Mat4::IDENTITY };
        let r = rows3(&m);
        for k in 0..3 { out[3 * b + k] = r[k]; }
    }
    out
}

pub fn bytes(v: &[[f32; 4]]) -> Vec<u8> { v.iter().flat_map(|r| r.iter().flat_map(|f| f.to_le_bytes())).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    /// The original program's skinned, transformed position equals what the renderer's `vs_skinned` computes: `G * M_gltf * p_gltf`.
    #[test] fn original_program_math_equals_the_renderer_transform() {
        let g = Mat4::from_rotation_translation(Quat::from_rotation_y(0.7) * Quat::from_rotation_x(0.2), Vec3::new(3., 1., -2.));
        let m = Mat4::from_rotation_translation(Quat::from_rotation_z(0.4), Vec3::new(0.01, 0.02, -0.03));
        let p_ue = Vec3::new(12.0, -7.5, 30.0);
        let p_gltf = ue_to_port().transform_point3(p_ue);
        let want = g.transform_point3(m.transform_point3(p_gltf));
        // program: bone rows (p' = M p), then row-vector LocalToWorld, then back to port metres
        let rows = bone_buffer(&[m], &[Some(0)], 3);
        let skinned = Vec3::new(
            rows[0][0] * p_ue.x + rows[0][1] * p_ue.y + rows[0][2] * p_ue.z + rows[0][3],
            rows[1][0] * p_ue.x + rows[1][1] * p_ue.y + rows[1][2] * p_ue.z + rows[1][3],
            rows[2][0] * p_ue.x + rows[2][1] * p_ue.y + rows[2][2] * p_ue.z + rows[2][3]);
        let prim = primitive_buffer(&local_to_world_ue(g), ([0.; 3], 1.));
        let w = Vec3::new(skinned.x, skinned.y, skinned.z);
        let world_ue = w.x * Vec3::new(prim[0][0], prim[0][1], prim[0][2]) + w.y * Vec3::new(prim[1][0], prim[1][1], prim[1][2]) + w.z * Vec3::new(prim[2][0], prim[2][1], prim[2][2]) + Vec3::new(prim[3][0], prim[3][1], prim[3][2]);
        let got = Vec3::new(world_ue.x, world_ue.z, world_ue.y) * 0.01;
        assert!((got - want).length() < 1e-4, "{got:?} vs {want:?}");
        assert_eq!(prim[9][3], 1.0, "S G S keeps the handedness");
        for k in 0..3 { assert!((prim[14][k] - 1.0).abs() < 1e-4); }
    }
}
