// M_AridSky dome (FortniteSkyDome_Optimized, StarMapMeshComponent scaled (37500,37500,18750) cm).
// Line-by-line port of the pixel shader UE 4.16 (Fortnite 1.8) compiled into M_AridSky (shader_6890, ps_5_0). The editor graph is NOT
// in the cooked package, so this is the only recovered form of the material maths. D3D exp/log are base 2.
//
// Texture registers: t0 Galaxy_Background (sRGB), t1 Galaxy_Background_Tiling (sRGB), t2 T_StormCloudDetail01_M (linear),
// t3 T_Sky_Stars_Sharp (linear, mip bias -1), t4 Noise32x32 (linear). Output is linear HDR emissive (Unlit material).
@group(0) @binding(0) var<uniform> sky: SkyU;
@group(0) @binding(1) var t_galaxy: texture_2d<f32>;
@group(0) @binding(2) var t_galaxy_tile: texture_2d<f32>;
@group(0) @binding(3) var t_storm: texture_2d<f32>;
@group(0) @binding(4) var t_stars: texture_2d<f32>;
@group(0) @binding(5) var t_noise: texture_2d<f32>;
@group(0) @binding(6) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv0: vec2<f32>,
    @location(1) uv2y: f32,
    @location(2) world_pos: vec3<f32>,
}

// pos = true dome-local position in metres (~1.5e6 m radius, GL world); texcoord = (uv0.x, uv0.y, uv1.y, uv2.y).
@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) tc: vec4<f32>) -> VsOut {
    var o: VsOut;
    o.uv0 = tc.xy;
    o.uv2y = tc.w;
    o.world_pos = pos;
    o.clip = sky.vp * vec4<f32>(pos - sky.cam_world.xyz, 1.0);
    // PERF (2026-10-06): on the far plane (depth 1), so the sky pass's LessEqual test against the scene depth (drawn after the pre-pass) shades only the pixels
    // nothing opaque covers, like UE's base-pass sky behind the early-Z (the image is unchanged: the sky used to be overdrawn there)
    o.clip.z = o.clip.w;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let time = sky.cam_world.w;
    let uv = in.uv0;
    let params = sky.params;
    // star twinkle noise: noise texture panned slowly across a 25x tiled uv
    let pn = uv * 25.0 + time * vec2<f32>(0.0245, 0.000306);
    let n = textureSample(t_noise, samp, vec2<f32>(1.5) - pn).xyz;
    let n2 = n.z * n.z;
    let base = uv * vec2<f32>(6.0, 5.0);
    let uvA = (n.xy - 0.5) * 0.0025 + base;
    let uvB = (n.xy - 0.5) * 0.005 + base;
    let sA = textureSampleBias(t_stars, samp, uvA, -1.0).xyz;
    let sBase = textureSampleBias(t_stars, samp, base, -1.0).xyz;
    let sC = textureSampleBias(t_stars, samp, uvB, -1.0).xyz;
    let stormSmall = textureSample(t_storm, samp, uv * 0.4).x;
    var s = vec3<f32>(sBase.x, sA.y, sC.z) * n2;
    s = s * 1.5;
    s = s * s;
    let stormBig = textureSample(t_storm, samp, uv).x;
    let m = (sBase.x + stormBig) * stormSmall;
    s = m * s + s;
    let g2 = textureSampleBias(t_stars, samp, vec2<f32>(1.25) - uvB * 2.0, -1.0).xyz;
    let g1 = textureSampleBias(t_stars, samp, uvB * 2.0, -1.0).xyz;
    var layer = g2 * 0.5 + g1;
    layer = s * 2.0 + layer;
    layer = layer * params.x;                       // Sky - Star Brightness (0 in the Day instance)
    var col = s * 0.025 + layer;
    col = col + sky.zenith.xyz - sky.horizon.xyz;
    // horizon band: uv2.y runs 0.5 (horizon) .. 1 (zenith). 1.8 compiled form: max(|x|, 1e-6) before the log.
    var f = 1.0 - exp2(log2(max(abs(1.0 - (in.uv2y * params.y + params.z)), 1e-6)) * params.w);
    f = max(f, 0.0);
    col = f * col + sky.horizon.xyz;
    // milky-way: tiling layer + one non-tiling band masked to a strip in v
    let tile = textureSample(t_galaxy_tile, samp, uv * vec2<f32>(4.258048, 6.387072)).xyz;
    let q = vec3<f32>(uv.x * 2.98166, uv.y * 6.249084 - 2.185346, uv.y * 6.249084 - 2.685346);
    let mask = clamp((1.0 - 2.0 * abs(q.z)) * 100000.0, 0.0, 1.0);
    let band = textureSample(t_galaxy, samp, q.xy).xyz;
    col = col + band * mask + tile;
    col = max(col, vec3<f32>(0.0));
    if (sky.to_sun.w > 0.5) {
        let fog = height_fog(sky.fog, in.world_pos, sky.cam_world.xyz, sky.to_sun.xyz, sky_forward());
        col = col * fog.a + fog.rgb;
    }
    return vec4<f32>(col, 1.0);
}
