// INVENTORY FIX3: SBackgroundBlur post-process (UE 4.16 SlatePostProcessPixelShader.usf GaussianBlurMain / DownsampleMain, FSlatePostProcessor::BlurRect).
// A full-viewport triangle; the fragment's position inside the destination viewport (`dst`) is the uv of the source texture.
// mode 0: separable Gaussian along `dir` with the discrete taps of fn_ui::draw::BlurCmd::taps (offset, weight) packed two per vec4 (texel-centre
//         samples, equal to the shader's bilinear pairs), sample uvs clamped to [uv_min, uv_max]; mode 1: 4-tap box downsample (offsets +-`dir` texels);
// mode 2: one bilinear sample (upsample). The output alpha is 1 (GaussianBlurMain returns float4(rgb, 1)).
struct P { dst: vec4<f32>, uv_bounds: vec4<f32>, texel_dir: vec4<f32>, n: vec4<u32>, taps: array<vec4<f32>, 128> };
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var t: texture_2d<f32>;
@group(0) @binding(2) var s: sampler;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vi << 1u) & 2u) * 2.0 - 1.0;
    let y = f32(vi & 2u) * 2.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn tap(uv: vec2<f32>) -> vec4<f32> { return textureSampleLevel(t, s, clamp(uv, p.uv_bounds.xy, p.uv_bounds.zw), 0.0); }

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = (pos.xy - p.dst.xy) / p.dst.zw;
    let texel = p.texel_dir.xy;
    let dir = p.texel_dir.zw;
    var c = vec4<f32>(0.0);
    if (p.n.y == 0u) {
        for (var i = 0u; i < p.n.x; i = i + 1u) {
            let v = p.taps[i / 2u];
            var o = v.x; var w = v.y;
            if ((i & 1u) == 1u) { o = v.z; w = v.w; }
            if (o == 0.0) { c += tap(uv) * w; }
            else { c += (tap(uv + dir * texel * o) + tap(uv - dir * texel * o)) * w; }
        }
    } else if (p.n.y == 1u) {
        let d = dir * texel;
        c = (tap(uv + d * vec2<f32>(-1.0, -1.0)) + tap(uv + d * vec2<f32>(1.0, -1.0)) + tap(uv + d * vec2<f32>(-1.0, 1.0)) + tap(uv + d * vec2<f32>(1.0, 1.0))) * 0.25;
    } else {
        c = tap(uv);
    }
    return vec4<f32>(c.rgb, 1.0);
}
