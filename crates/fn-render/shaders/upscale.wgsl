// SCREEN PERCENTAGE: UE 4.16 `FRCPassPostProcessUpscale` (PostProcessUpscale.usf MainPS, METHOD = r.Upscale.Quality, PostProcessUpscale.cpp:337-359)
// from the view-size LDR image to the window. Fortnite's PostProcessQuality@0..3 set r.Upscale.Quality 1 / 2 / 2 / 3 (CookedIniVersion.txt
// L5186 / 5203 / 5220 / 5237): 1 = bilinear, 2 = directional blur with unsharp mask, 3 = Catmull-Rom bicubic in five taps. Panini off
// (r.Upscale.Panini.D 0). Input sampler bilinear clamp.
struct UpscaleU { src: vec4<f32>, method: vec4<f32> }   // src: w, h, 1/w, 1/h of the input; method.x
@group(0) @binding(0) var<uniform> uu: UpscaleU;
@group(0) @binding(1) var tex: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    o.clip = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return o;
}
fn s(p: vec2<f32>) -> vec4<f32> { return textureSampleLevel(tex, samp, p, 0.0); }
fn luma709(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let m = i32(uu.method.x);
    let px = uu.src.zw;
    if (m == 2) {
        let x = 0.5;
        let nw = s(uv + vec2<f32>(-x, -x) * px).rgb;
        let ne = s(uv + vec2<f32>(x, -x) * px).rgb;
        let sw = s(uv + vec2<f32>(-x, x) * px).rgb;
        let se = s(uv + vec2<f32>(x, x) * px).rgb;
        let avg = nw * 0.25 + ne * 0.25 + sw * 0.25 + se * 0.25;
        let dsw_ne = luma709(sw) - luma709(ne);
        let dse_nw = luma709(se) - luma709(nw);
        var dir = vec2<f32>(dsw_ne + dse_nw, dsw_ne - dse_nw);
        dir = dir * (0.125 * inverseSqrt(dot(dir, dir) + 6e-8));
        let cn = s(uv - dir * px).rgb;
        let cp = s(uv + dir * px).rgb;
        let unsharp = 0.25;
        return vec4<f32>((cn + cp) * ((unsharp + 1.0) * 0.5) - avg * unsharp, 1.0);
    }
    if (m == 3) {
        // Texture2DSampleBicubic (Common.usf:405-471)
        let size = uu.src.xy;
        let p = uv * size;
        let tc = floor(p - 0.5) + 0.5;
        let f = p - tc; let f2 = f * f; let f3 = f2 * f;
        let w0 = f2 - 0.5 * (f3 + f);
        let w1 = 1.5 * f3 - 2.5 * f2 + 1.0;
        let w3 = 0.5 * (f3 - f2);
        let w2 = 1.0 - w0 - w1 - w3;
        let wt0 = w0; let wt1 = w1 + w2; let wt2 = w3;
        let s0 = (tc - 1.0) * px; let s1 = (tc + w2 / wt1) * px; let s2 = (tc + 2.0) * px;
        var c = s(vec2<f32>(s1.x, s0.y)) * wt1.x * wt0.y;
        c = c + s(vec2<f32>(s0.x, s1.y)) * wt0.x * wt1.y;
        c = c + s(vec2<f32>(s1.x, s1.y)) * wt1.x * wt1.y;
        c = c + s(vec2<f32>(s2.x, s1.y)) * wt2.x * wt1.y;
        c = c + s(vec2<f32>(s1.x, s2.y)) * wt1.x * wt2.y;
        let corner = wt1.x * wt0.y + wt0.x * wt1.y + wt1.x * wt1.y + wt2.x * wt1.y + wt1.x * wt2.y;
        return vec4<f32>((c / corner).rgb, 1.0);
    }
    return vec4<f32>(s(uv).rgb, 1.0);
}
