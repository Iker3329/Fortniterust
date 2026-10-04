// Instanced premultiplied quad pipeline of fn-ui-wgpu.
// One instance = one fn_ui::draw::Quad: four arbitrary corner positions (pixels, y down), a uv rect and a PREMULTIPLIED tint.
// Output = premultiplied texel * premultiplied tint, blended with ONE / ONE_MINUS_SRC_ALPHA on a non-sRGB target (gamma-space compositing).
struct Frame { vp: vec2<f32>, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var t: texture_2d<f32>;
@group(1) @binding(1) var s: sampler;

struct VIn {
    @builtin(vertex_index) vi: u32,
    @location(0) p01: vec4<f32>,
    @location(1) p23: vec4<f32>,
    @location(2) uv: vec4<f32>,
    @location(3) col: vec4<f32>,
};
struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) col: vec4<f32>,
};

@vertex
fn vs_main(in: VIn) -> VOut {
    var order = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    let corner = order[in.vi];
    var p: vec2<f32>;
    var uv: vec2<f32>;
    if (corner == 0u) { p = in.p01.xy; uv = vec2<f32>(in.uv.x, in.uv.y); }
    else if (corner == 1u) { p = in.p01.zw; uv = vec2<f32>(in.uv.z, in.uv.y); }
    else if (corner == 2u) { p = in.p23.xy; uv = vec2<f32>(in.uv.z, in.uv.w); }
    else { p = in.p23.zw; uv = vec2<f32>(in.uv.x, in.uv.w); }
    var o: VOut;
    o.pos = vec4<f32>(p.x / frame.vp.x * 2.0 - 1.0, 1.0 - p.y / frame.vp.y * 2.0, 0.0, 1.0);
    o.uv = uv;
    o.col = in.col;
    return o;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    return textureSample(t, s, in.uv) * in.col;
}
