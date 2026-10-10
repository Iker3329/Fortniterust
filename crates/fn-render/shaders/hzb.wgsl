// REFLECTION APPLY (SSR): the hierarchical Z buffer of UE 4.16 (`BuildHZB`, SceneOcclusion.cpp:1076; `HZBBuildPS`, HZBOcclusion.usf:12), PF_R16F, the
// FURTHEST depth of every 2x2 block. UE stores its device Z (reversed, infinite far: Z = near / ViewDepth, 0 = sky) and keeps the MIN; this renderer's depth
// is 0 = near / 1 = far, so mip 0 converts every pixel to UE's device Z (near 10 cm, metres here: 0.1 / w) and keeps the min the same way.
// Mip 0 (STAGE 0): texel (i, j) of the HZBSize = 2^NumMips chain covers buffer pixels 2i..2i+1, 2j..2j+1 (InputUvFactorAndOffset = 2 HZBSize / BufferSize,
// four point taps at +-0.25 buffer texel), clamped to the viewport (InputViewportMaxBound); mips 1..: the 2x2 texels of the previous mip, clamped to it.
struct HzbU { depth: vec4<f32> }   // x port near (m), y port far (m), z UE near (m) of the device Z
@group(0) @binding(0) var<uniform> hu: HzbU;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var src_tex: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// UE device Z of a port depth sample: near / ViewDepth; the cleared depth (1 = nothing drawn) is UE's far plane at infinity, 0
fn ue_device_z(d: f32) -> f32 {
    if (d >= 1.0) { return 0.0; }
    let n = hu.depth.x;
    let f = hu.depth.y;
    let w = n * f / (f - d * (f - n));
    return hu.depth.z / w;
}

@fragment
fn fs_mip0(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(depth_tex));
    let p = vec2<i32>(floor(frag.xy)) * 2;
    let m = size - vec2<i32>(1);
    let a = ue_device_z(textureLoad(depth_tex, min(p, m), 0));
    let b = ue_device_z(textureLoad(depth_tex, min(p + vec2<i32>(1, 0), m), 0));
    let c = ue_device_z(textureLoad(depth_tex, min(p + vec2<i32>(0, 1), m), 0));
    let d = ue_device_z(textureLoad(depth_tex, min(p + vec2<i32>(1, 1), m), 0));
    return vec4<f32>(min(min(a, b), min(c, d)));
}

@fragment
fn fs_down(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let m = vec2<i32>(textureDimensions(src_tex)) - vec2<i32>(1);
    let p = vec2<i32>(floor(frag.xy)) * 2;
    let a = textureLoad(src_tex, min(p, m), 0).r;
    let b = textureLoad(src_tex, min(p + vec2<i32>(1, 0), m), 0).r;
    let c = textureLoad(src_tex, min(p + vec2<i32>(0, 1), m), 0).r;
    let d = textureLoad(src_tex, min(p + vec2<i32>(1, 1), m), 0).r;
    return vec4<f32>(min(min(a, b), min(c, d)));
}
