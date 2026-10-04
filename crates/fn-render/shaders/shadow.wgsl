// Cascaded shadow map depth pass (one render pass per cascade, light view-projection in group 0).
// Casters are the skinned character parts and the pickaxe (same skinning as scene.wgsl).
struct LightViewProj { m: mat4x4<f32> }
@group(0) @binding(0) var<uniform> light_vp: LightViewProj;

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
    return light_vp.m * (object.model * vec4<f32>(p_local, 1.0));
}

// Props: world-space positions (stride 36: pos, normal, colour; only pos is read).
@vertex
fn vs_prop(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return light_vp.m * vec4<f32>(pos, 1.0);
}
