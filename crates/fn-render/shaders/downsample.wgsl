// HDR helpers: mip downsample (bloom taps + 1x1 mean for exposure metering) and the exposure-adaptation compute shader.

struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> }   // uv: GL convention (y up)

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    o.clip = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x, y) * 0.5 + 0.5;
    return o;
}

// Each pass box-filters the previous mip into the next one (a bilinear tap at the centre of a 2x2 block).
@group(0) @binding(0) var ds_src: texture_2d<f32>;
@group(0) @binding(1) var ds_samp: sampler;
@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(ds_src, ds_samp, vec2<f32>(in.uv.x, 1.0 - in.uv.y), 0.0);
}
