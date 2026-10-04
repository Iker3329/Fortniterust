// Separable Gaussian with bilinear tap pairs (FRCPassPostProcessWeightedSampleSum, shader family TFilterPS<N><method>):
//   out = sum_i tap_i.weight * sample(uv + tap_i.offset * dir) * tint   [+ prev_tex(uv) when `flags.x != 0` = EFCM "add previous", TFilterPS<N>1]
// The compiled shader reads (offset, weight) pairs from constant buffers; the pairs are computed on the CPU (`cpu::gaussian_taps`).
// Prepended: common.wgsl.

struct BlurU {
    taps: array<vec4<f32>, 32>,  // x = offset in texels, y = weight (already normalised)
    dir_texel: vec4<f32>,        // xy = direction * texel size of the source; zw = 1 / target size
    tint: vec4<f32>,
    flags: vec4<u32>,            // x = add previous, y = tap count
}

@group(0) @binding(0) var<uniform> u: BlurU;
@group(0) @binding(1) var src_tex: texture_2d<f32>;
@group(0) @binding(2) var prev_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

@fragment
fn fs_blur(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.pos.xy * u.dir_texel.zw;
    var acc = vec4<f32>(0.0);
    for (var i = 0u; i < u.flags.y; i = i + 1u) {
        let t = u.taps[i];
        acc = acc + textureSampleLevel(src_tex, samp, uv + u.dir_texel.xy * t.x, 0.0) * t.y;
    }
    acc = acc * u.tint;
    if (u.flags.x != 0u) {
        acc = acc + textureSampleLevel(prev_tex, samp, uv, 0.0);
    }
    return acc;
}

