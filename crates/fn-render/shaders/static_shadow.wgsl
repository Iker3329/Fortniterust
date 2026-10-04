// Shadow cascades for static meshes. Prepended: static_common.wgsl (group 1 = material). Group 0 = light view-projection.
// The opaque variant has no fragment stage; the masked variant samples the opacity (base colour alpha, the same coverage-preserving mips
// as the colour pass) and discards below the cutoff, so leaves cast leaf-shaped shadows in all three cascades.
struct LightViewProj { m: mat4x4<f32> }
@group(0) @binding(0) var<uniform> light_vp: LightViewProj;

struct ShOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_static_shadow(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) color: vec4<f32>, inst: InstIn) -> ShOut {
    let sp = static_transform(pos, normal, color, inst);
    var o: ShOut;
    o.clip = light_vp.m * vec4<f32>(sp.world, 1.0);
    o.uv = uv;
    return o;
}

@fragment
fn fs_static_shadow(in: ShOut) {
    if (textureSample(t_base, s_static, in.uv).a < smat.p0.x) { discard; }
}
