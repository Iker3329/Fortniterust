// Bloom front end of the UE 4.16 tonemapper chain.
// fs_setup      = FPostProcessBloomSetupPS (reports/postfx-shaders/FPostProcessBloomSetupPS__968774.asm):
//                   c = min(scene, 65536); a = saturate((dot(c, (0.3, 0.59, 0.11)) * exposure - BloomThreshold) * 0.5); out = c * a
//                 (the exposure scale comes from the eye-adaptation texel through the VS; here it is a uniform)
// fs_downsample = FPostProcessDownsamplePS1 (4 bilinear taps at +-cb0[18].xy = 1 source texel => 4x4 box, max(rgb, 0))
// The Gaussian levels are `bloom_filter.wgsl` (bloom.rs) / `blur.wgsl` (DOF). Prepended: common.wgsl.
// Downsample uv: UE draws SrcRect = DestRect * 2 (FRCPassPostProcessDownsample::Process), i.e. uv = pixel * 2 / source size (bloom.rs passes
// that in `inv_dst`; for even sizes it equals pixel / target size).

struct BloomU {
    p: vec4<f32>,         // x = exposure, y = threshold
    inv_dst: vec4<f32>,   // xy = pixel position -> source uv
    src_texel: vec4<f32>, // xy = 1 / source size
}

@group(0) @binding(0) var<uniform> u: BloomU;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

@fragment
fn fs_setup(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.pos.xy * u.inv_dst.xy;
    let c = min(textureSampleLevel(src_tex, samp, uv, 0.0).rgb, vec3<f32>(65536.0));
    let luma = dot(c, vec3<f32>(0.3, 0.59, 0.11));
    let a = clamp((luma * u.p.x - u.p.y) * 0.5, 0.0, 1.0);
    return vec4<f32>(c * a, 0.0);
}

@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.pos.xy * u.inv_dst.xy;
    let o = u.src_texel.xy;
    var s = textureSampleLevel(src_tex, samp, uv - o, 0.0);
    s = s + textureSampleLevel(src_tex, samp, uv + vec2<f32>(o.x, -o.y), 0.0);
    s = s + textureSampleLevel(src_tex, samp, uv + vec2<f32>(-o.x, o.y), 0.0);
    s = s + textureSampleLevel(src_tex, samp, uv + o, 0.0);
    s = s * 0.25;
    return vec4<f32>(max(s.rgb, vec3<f32>(0.0)), s.a);
}
