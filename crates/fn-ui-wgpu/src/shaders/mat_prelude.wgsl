// Common prelude of every GPU UI material module (Settings uimat, HUD materials, procedural/panel).
//
// Per-draw uniform P (dynamic offset, 256 x vec4):
//   P[0]  rect (x, y, w, h) in target pixels            P[1]  widget tint (straight RGBA)
//   P[2]  Slate vertex colour v1 (linear RGBA)           P[3]  (flip_x, boxed, kind, 0)
//   P[4]  (native_w, native_h, size_w, size_h)           P[5]  nine-slice margin (L, T, R, B) as UV fractions
//   P[6..8)  free (procedural parameters)
//   P[8..11) cb0 (Slate GammaAndAlphaValues in P[10])    P[11] View.GameTime in .z (cb1[131])
//   P[12..16) texture slot meta (wrap_x, wrap_y, srgb, 0)
//   P[16..)  material constant buffer (cooked FMaterialUniformExpressionSet evaluation, built on the CPU)
// Procedural/panel shaders reuse P[6..16) for their own parameters (they have no cb0/textures).
struct Frame { vp: vec2<f32>, pad: vec2<f32> };
@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> P: array<vec4<f32>, 256>;
@group(2) @binding(0) var tx0: texture_2d<f32>;
@group(2) @binding(1) var tx1: texture_2d<f32>;
@group(2) @binding(2) var tx2: texture_2d<f32>;
@group(2) @binding(3) var tx3: texture_2d<f32>;

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) lin: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VOut {
    var corners = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    let q = corners[vi];
    let rect = P[0];
    let p = rect.xy + q * rect.zw;
    var o: VOut;
    o.pos = vec4<f32>(p.x / frame.vp.x * 2.0 - 1.0, 1.0 - p.y / frame.vp.y * 2.0, 0.0, 1.0);
    // lin = TexCoord0 / MaterialTexCoords: linear 0..1 over the element (mirrored for flip_x).
    o.lin = vec2<f32>(select(q.x, 1.0 - q.x, P[3].x > 0.5), q.y);
    return o;
}

fn srgb_to_lin(c: f32) -> f32 {
    if (c <= 0.04045) { return c / 12.92; }
    return pow((c + 0.055) / 1.055, 2.4);
}
fn tex_dims(k: u32) -> vec2<i32> {
    if (k == 0u) { return vec2<i32>(textureDimensions(tx0)); }
    if (k == 1u) { return vec2<i32>(textureDimensions(tx1)); }
    if (k == 2u) { return vec2<i32>(textureDimensions(tx2)); }
    return vec2<i32>(textureDimensions(tx3));
}
fn tex_load(k: u32, p: vec2<i32>) -> vec4<f32> {
    if (k == 0u) { return textureLoad(tx0, p, 0); }
    if (k == 1u) { return textureLoad(tx1, p, 0); }
    if (k == 2u) { return textureLoad(tx2, p, 0); }
    return textureLoad(tx3, p, 0);
}
// One bilinear tap set at the texel-centre convention (identical to the CPU `tex_sample` of fn-ui): sRGB RGB is decoded BEFORE filtering,
// wrap/clamp is applied per axis from the slot meta.
fn tex_texel(k: u32, x: i32, y: i32, dims: vec2<i32>, tmeta: vec4<f32>) -> vec4<f32> {
    var xi = x;
    var yi = y;
    if (tmeta.x > 0.5) { xi = ((x % dims.x) + dims.x) % dims.x; } else { xi = clamp(x, 0, dims.x - 1); }
    if (tmeta.y > 0.5) { yi = ((y % dims.y) + dims.y) % dims.y; } else { yi = clamp(y, 0, dims.y - 1); }
    var c = tex_load(k, vec2<i32>(xi, yi));
    if (tmeta.z > 0.5) { c = vec4<f32>(srgb_to_lin(c.x), srgb_to_lin(c.y), srgb_to_lin(c.z), c.w); }
    return c;
}
fn tex_sample_k(k: u32, u: f32, v: f32) -> vec4<f32> {
    let tmeta = P[12u + k];
    let dims = tex_dims(k);
    let x = u * f32(dims.x) - 0.5;
    let y = v * f32(dims.y) - 0.5;
    let x0 = floor(x);
    let y0 = floor(y);
    let fx = x - x0;
    let fy = y - y0;
    let xi = i32(x0);
    let yi = i32(y0);
    let a = tex_texel(k, xi, yi, dims, tmeta);
    let b = tex_texel(k, xi + 1, yi, dims, tmeta);
    let c = tex_texel(k, xi, yi + 1, dims, tmeta);
    let d = tex_texel(k, xi + 1, yi + 1, dims, tmeta);
    return (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy;
}
fn tex_sample_0(u: f32, v: f32) -> vec4<f32> { return tex_sample_k(0u, u, v); }
fn tex_sample_1(u: f32, v: f32) -> vec4<f32> { return tex_sample_k(1u, u, v); }
fn tex_sample_2(u: f32, v: f32) -> vec4<f32> { return tex_sample_k(2u, u, v); }
fn tex_sample_3(u: f32, v: f32) -> vec4<f32> { return tex_sample_k(3u, u, v); }

// Slate nine-patch (AddBoxElement) along one axis, identical to fn_ui::material::uimat::nine_slice_uv.
fn nine_axis(p: f32, size: f32, native: f32, m0: f32, m1: f32) -> f32 {
    var a = m0 * native;
    var b = m1 * native;
    if (a + b > size && a + b > 0.0) { let s = size / (a + b); a = a * s; b = b * s; }
    if (p < a) { if (a > 0.0) { return p / a * m0; } return 0.0; }
    if (p > size - b) { if (b > 0.0) { return (1.0 - m1) + (p - (size - b)) / b * m1; } return 1.0; }
    let w = size - a - b;
    if (w > 0.0) { return m0 + (p - a) / w * (1.0 - m0 - m1); }
    return m0;
}

fn finish(c0: vec4<f32>) -> vec4<f32> {
    let c = clamp(c0, vec4<f32>(0.0), vec4<f32>(1.0));
    let t = P[1];
    return vec4<f32>(c.rgb * c.a, c.a) * vec4<f32>(t.rgb * t.a, t.a);
}
