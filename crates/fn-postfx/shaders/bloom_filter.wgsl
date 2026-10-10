// One FRCPassPostProcessWeightedSampleSum pass of the bloom (FilterPixelShader.usf, COMBINE_METHOD 0 = weighted, 1 = weighted + additive):
//   Dest = sum_i Sample(FilterTexture, InUV + Offset_i) * (Tint * Weight_i);   Dest += Sample(AdditiveTexture, InUV)   (combine 1)
// Both textures use TStaticSamplerState<SF_Bilinear, AM_Border, AM_Border, AM_Clamp> (border colour 0): the bilinear fetch is done by hand
// with textureLoad so texels outside the image contribute 0, exactly like the border sampler. Prepended: common.wgsl.

struct FilterU {
    taps: array<vec4<f32>, 32>,  // x = offset in texels of the source, y = normalised weight
    uv_scale: vec4<f32>,         // xy: pixel position -> source uv (fast blur: the X pass covers 2 source texels per output texel)
    step: vec4<f32>,             // xy: uv per texel of offset (axis), zw: constant uv shift (fast blur: +-0.5 texel in y)
    tint: vec4<f32>,
    flags: vec4<u32>,            // x = add the additive texture, y = tap count
}

@group(0) @binding(0) var<uniform> u: FilterU;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var add_tex: texture_2d<f32>;

fn fetch(t: texture_2d<f32>, p: vec2<i32>, size: vec2<i32>) -> vec4<f32> {
    if (p.x < 0 || p.y < 0 || p.x >= size.x || p.y >= size.y) { return vec4<f32>(0.0); }
    return textureLoad(t, p, 0);
}

fn sample_border(t: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(t));
    let p = uv * vec2<f32>(size) - 0.5;
    let i = vec2<i32>(floor(p));
    let f = p - floor(p);
    let a = fetch(t, i, size);
    let b = fetch(t, i + vec2<i32>(1, 0), size);
    let c = fetch(t, i + vec2<i32>(0, 1), size);
    let d = fetch(t, i + vec2<i32>(1, 1), size);
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

@fragment
fn fs_filter(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.pos.xy * u.uv_scale.xy;
    var acc = vec4<f32>(0.0);
    for (var i = 0u; i < u.flags.y; i = i + 1u) {
        let t = u.taps[i];
        acc = acc + sample_border(src_tex, uv + u.step.xy * t.x + u.step.zw) * (u.tint * t.y);
    }
    if (u.flags.x != 0u) {
        acc = acc + sample_border(add_tex, uv);
    }
    return acc;
}
