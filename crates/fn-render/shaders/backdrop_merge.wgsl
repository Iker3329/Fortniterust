// SSR BACKDROP (2026-10-05): writes the backdrop pass's depth into the scene depth (backdrop.rs `encode_depth_merge`). UE4.16 renders the landscape, the ocean
// and the far terraces into the ONE scene depth buffer (reversed-Z, infinite far plane), so they are geometry for the HZB / SSR, the AO, the fog, the TAA
// reprojection, the light-shaft occlusion and every later depth test. The port draws them with their own projection (near BD_NEAR, far BD_FAR) and converts:
// view depth z of the backdrop depth, then the main pass's perspective_rh depth f / (f - n) * (1 - n / z); a z beyond the main far plane (a finite plane the port
// uses instead of UE's infinite one) is clamped just below 1 so it stays geometry, not sky (1 = cleared).
struct Bd {
    view_rot_proj: mat4x4<f32>,
    eye: vec4<f32>,
    misc: vec4<f32>,     // x unused, y / z main pass near / far (m)
}
@group(0) @binding(0) var bd_depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> bd: Bd;

const BD_NEAR: f32 = 20.0;      // backdrop.rs BD_NEAR
const BD_FAR: f32 = 2.0e6;      // backdrop.rs BD_FAR
const BELOW_ONE: f32 = 0.99999994;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(i32(i & 1u) * 4 - 1), f32(i32((i >> 1u) & 1u) * 4 - 1), 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) p: vec4<f32>) -> @builtin(frag_depth) f32 {
    let d = textureLoad(bd_depth, vec2<i32>(p.xy), 0);
    if (d >= 1.0) { discard; }
    let z = BD_NEAR * BD_FAR / (BD_FAR - d * (BD_FAR - BD_NEAR));
    let n = bd.misc.y;
    let f = bd.misc.z;
    return clamp(f / (f - n) * (1.0 - n / z), 0.0, BELOW_ONE);
}
