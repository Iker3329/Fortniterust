// Cascaded shadow map depth pass (one render pass per cascade, light view-projection in group 0).
// Casters are the skinned character parts and the pickaxe (same skinning as scene.wgsl).
// Port of ShadowDepthVertexShader.usf `SetShadowDepthOutputs` for a whole-scene directional shadow: the cascade matrix already yields the normalised depth
// (`z * InvMaxSubjectDepth`); vertices in front of the near plane are clamped to it (`bClampToNearPlane`, true for whole-scene directional shadows) and
// `ShaderDepthBias` is ADDED to the stored depth. The depth pass has no hardware slope / constant bias in UE4.16.
struct LightViewProj { m: mat4x4<f32>, depth_bias: vec4<f32> }
@group(0) @binding(0) var<uniform> light_vp: LightViewProj;

fn shadow_depth_outputs(world: vec4<f32>) -> vec4<f32> {
    var p = light_vp.m * world;
    if (p.z < 0.0) { p.z = 0.000001; p.w = 1.0; }
    p.z = (p.z + light_vp.depth_bias.x) * p.w;
    return p;
}

struct Object { model: mat4x4<f32> }
@group(1) @binding(0) var<uniform> object: Object;
@group(1) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;

@vertex
fn vs_skinned(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
) -> @builtin(position) vec4<f32> {
    let m = bones[joints.x] * weights.x + bones[joints.y] * weights.y + bones[joints.z] * weights.z + bones[joints.w] * weights.w;
    let p_local = (m * vec4<f32>(pos, 1.0)).xyz;
    return shadow_depth_outputs(object.model * vec4<f32>(p_local, 1.0));
}

// Props: world-space positions (stride 36: pos, normal, colour; only pos is read).
@vertex
fn vs_prop(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return shadow_depth_outputs(vec4<f32>(pos, 1.0));
}
