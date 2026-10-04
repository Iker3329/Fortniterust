// Eyeshadow: the plain BLEND_Modulate / MSM_Unlit material of the eye region. `eyeshadow_ps` (eyeshadow_gen.wgsl) is the original compiled base-pass pixel shader; its output
// colour is MULTIPLIED into the scene (blend DestColor, Zero), so it is drawn after the scene pass with the scene depth read-only.
//
// Prepended: common.wgsl, lit_common.wgsl, eyeshadow_gen.wgsl.
// Inputs of the translated shader: v[2].x = vertex colour R (the painted eyeshadow mask), v[3].xy = UV0, v[4].w = TEXCOORD7.w (atmospheric-fog fade written by the vertex
// shader, 1 inside the fog start distance: see fn_pose::br_material), v[5].xyz = translated world position (world - camera, cm, Unreal axes); View.PreViewTranslation
// (cb0[62]) = -camera.

struct Object { model: mat4x4<f32> }
@group(1) @binding(0) var<uniform> object: Object;
@group(1) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;

struct EyeMat { cb1: array<vec4<f32>, 10> }
@group(2) @binding(0) var<uniform> emat: EyeMat;
@group(2) @binding(1) var fx_tex1: texture_2d<f32>;
@group(2) @binding(2) var fx_samp1: sampler;
@group(2) @binding(3) var fx_tex0: texture_2d<f32>;
@group(2) @binding(4) var fx_samp0: sampler;
@group(2) @binding(5) var fx_tex2: texture_2d<f32>;
@group(2) @binding(6) var fx_samp2: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vs_overlay(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
    @location(5) color: vec4<f32>,
) -> VsOut {
    var o: VsOut;
    let m = bones[joints.x] * weights.x + bones[joints.y] * weights.y + bones[joints.z] * weights.z + bones[joints.w] * weights.w;
    let p_local = (m * vec4<f32>(pos, 1.0)).xyz;
    let wp = object.model * vec4<f32>(p_local, 1.0);
    o.clip = camera.view_proj * wp;
    o.world_pos = wp.xyz;
    o.uv = uv;
    o.color = color;
    return o;
}

fn to_ue(r: vec3<f32>) -> vec3<f32> { return vec3<f32>(r.x, r.z, r.y); }

@fragment
fn fs_overlay(in: VsOut) -> @location(0) vec4<f32> {
    let cam_ue = to_ue(camera.position.xyz) * 100.0;
    let tw = to_ue(in.world_pos) * 100.0 - cam_ue;
    eyeshadow_cb0[62] = vec4<f32>(-cam_ue, 0.0);
    for (var i = 0; i < 10; i = i + 1) { eyeshadow_cb1[i] = emat.cb1[i]; }
    var v: array<vec4<f32>, 6>;
    v[2] = in.color;
    v[3] = vec4<f32>(in.uv, 0.0, 0.0);
    v[4] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    v[5] = vec4<f32>(tw, 0.0);
    let o = eyeshadow_ps(v);
    return vec4<f32>(o[0].xyz, 1.0);
}
