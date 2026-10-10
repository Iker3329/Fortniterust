// TEMPORAL AA: the scene velocity target of UE 4.16 for the
// primitives that move: the skinned models. Static primitives write nothing (cleared to 0 = "no velocity", the TAA shader then uses the camera
// reprojection from depth), exactly like UE where only `DrawsVelocity()` primitives output it. This is the separate velocity pass of VelocityRendering.cpp
// (depth-tested against the scene depth); CookedIniVersion.txt has RendererSettings r.BasePassOutputsVelocity True (L75) but the cooked shader
// set says 0 ([AdditionalSettings] L5899), i.e. the separate pass; the encoded values are the same either way.
// VelocityShader.usf:155-198 / VelocityCommon.usf:9-19: ScreenPos = TranslatedWorldToClip * pos, PrevScreenPos = PrevTranslatedWorldToClip * prev
// pos (previous bones + previous model matrix), Velocity = (ScreenPos.xy / w - jitter) - (PrevScreenPos.xy / w - prev jitter), i.e. the unjittered
// screen positions; EncodeVelocityToTexture (Common.usf:1736-1741) `v * (0.499 * 0.5) + 32767 / 65535`, stored in a PF_G16R16 (16-bit unorm)
// target: the value is quantised to 1/65535 here and kept in an Rg32Float texture.
struct VelU {
    view_proj: mat4x4<f32>,        // jittered (rasterisation, must match the scene pass depth)
    view_proj_no_aa: mat4x4<f32>,  // this frame, no jitter
    prev_view_proj_no_aa: mat4x4<f32>,
}
struct Object { model: mat4x4<f32> }
@group(0) @binding(0) var<uniform> vu: VelU;
@group(1) @binding(0) var<uniform> object: Object;
@group(1) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;
@group(2) @binding(0) var<uniform> prev_object: Object;
@group(2) @binding(1) var<storage, read> prev_bones: array<mat4x4<f32>>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) cur: vec4<f32>,
    @location(1) prev: vec4<f32>,
}

fn skin_velocity(pos: vec3<f32>, joints: vec4<u32>, weights: vec4<f32>) -> VsOut {
    var o: VsOut;
    // same expression as scene.wgsl vs_skinned (the depth test against the scene depth needs the same position)
    let m = bones[joints.x] * weights.x + bones[joints.y] * weights.y + bones[joints.z] * weights.z + bones[joints.w] * weights.w;
    let p_local = (m * vec4<f32>(pos, 1.0)).xyz;
    let wp = object.model * vec4<f32>(p_local, 1.0);
    o.clip = vu.view_proj * wp;
    o.cur = vu.view_proj_no_aa * wp;
    let pm = prev_bones[joints.x] * weights.x + prev_bones[joints.y] * weights.y + prev_bones[joints.z] * weights.z + prev_bones[joints.w] * weights.w;
    let pp = prev_object.model * vec4<f32>((pm * vec4<f32>(pos, 1.0)).xyz, 1.0);
    o.prev = vu.prev_view_proj_no_aa * pp;
    return o;
}

@vertex
fn vs_skinned(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) joints: vec4<u32>, @location(4) weights: vec4<f32>) -> VsOut {
    return skin_velocity(pos, joints, weights);
}

// VELOCITY (2026-10-07): the BR character parts (char_scene.wgsl). Their pre-pass runs the material's FDepthOnlyPS coverage clip with an `@invariant` position
// (CHAR PREPASS), so this twin is `@invariant` too and depth-tested EQUAL against the scene depth: it writes exactly the pixels the clipped pre-pass kept
// (UE: the velocity pass of a masked material runs its clip, VelocityShader.usf:248 `GetMaterialClippingVelocity`). Was: the plain `vs_skinned` + LessEqual with a
// -2 depth bias, which wrote the clipped-away texels (velocity on the background next to the hair / cloth edges) and lost covered ones whose depth
// rounded differently (holes), so those pixels reprojected with the camera only: the lagging outline around the moving character.
struct VsOutChar {
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) cur: vec4<f32>,
    @location(1) prev: vec4<f32>,
}
@vertex
fn vs_skinned_char(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) joints: vec4<u32>, @location(4) weights: vec4<f32>) -> VsOutChar {
    let s = skin_velocity(pos, joints, weights);
    var o: VsOutChar;
    o.clip = s.clip; o.cur = s.cur; o.prev = s.prev;
    return o;
}
@fragment
fn fs_velocity_char(in: VsOutChar) -> @location(0) vec4<f32> { return velocity_out(in.cur, in.prev); }

@fragment
fn fs_velocity(in: VsOut) -> @location(0) vec4<f32> { return velocity_out(in.cur, in.prev); }
fn velocity_out(cur: vec4<f32>, prev: vec4<f32>) -> vec4<f32> {
    let v = cur.xy / cur.w - prev.xy / prev.w;
    let enc = v * (0.499 * 0.5) + 32767.0 / 65535.0;
    // PF_G16R16 unorm storage
    let q = round(clamp(enc, vec2<f32>(0.0), vec2<f32>(1.0)) * 65535.0) / 65535.0;
    return vec4<f32>(q, 0.0, 0.0);
}
