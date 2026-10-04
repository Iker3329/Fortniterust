// Gaussian depth of field, FAR layer only (Athena Day: DepthOfFieldNearBlurSize = 0 => near layer disabled).
// fs_setup_far      = FPostProcessDOFSetupPS10      (shader_953491-family, reports/postfx-shaders/FPostProcessDOFSetupPS10__956196.asm)
// fs_recombine_far  = FPostProcessDOFRecombinePS100 (reports/postfx-shaders/FPostProcessDOFRecombinePS100__762094.asm)
// The Gaussian blur between the two is `blur.wgsl` (FRCPassPostProcessWeightedSampleSum, TFilterPS).
// Prepended: common.wgsl.

struct DofU {
    depth_xf: vec4<f32>,   // InvDeviceZToWorldZTransform (renderer units)
    dist: vec4<f32>,       // x = focal_distance + focal_region (cm), y = far_transition (cm), z = sky distance (cm, huge = off), w = cm per renderer unit
    rows0: vec4<f32>,      // cb1[44]  ScreenToWorld rows (only xyz used)
    rows1: vec4<f32>,      // cb1[45]
    rows2: vec4<f32>,      // cb1[46]
    clamp_rect: vec4<f32>, // cb0[20]  uv clamp of the blurred buffer (min.xy, max.zw)
    inv_size: vec4<f32>,   // xy = 1 / full-res size (setup input / recombine target), zw = 1 / setup (half-res) size
}

@group(0) @binding(0) var<uniform> u: DofU;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var far_tex: texture_2d<f32>;

fn load_depth(p: vec2<i32>) -> f32 {
    let dims = vec2<i32>(textureDimensions(depth_tex));
    return textureLoad(depth_tex, clamp(p, vec2<i32>(0), dims - vec2<i32>(1)), 0);
}

// D3D gather4 of the 2x2 texel block around uv: x = (i0, j1), y = (i1, j1), z = (i1, j0), w = (i0, j0).
fn gather_depth(uv: vec2<f32>) -> vec4<f32> {
    let dims = vec2<f32>(textureDimensions(depth_tex));
    let b = vec2<i32>(floor(uv * dims - vec2<f32>(0.5)));
    return vec4<f32>(
        load_depth(b + vec2<i32>(0, 1)),
        load_depth(b + vec2<i32>(1, 1)),
        load_depth(b + vec2<i32>(1, 0)),
        load_depth(b),
    );
}

fn far_coc(lin_cm: f32) -> f32 {
    // div_sat (depth - (focal + region)) / FarTransitionRegion
    return sat((lin_cm - u.dist.x) / u.dist.y);
}

@fragment
fn fs_setup_far(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.pos.xy * u.inv_size.zw;                       // v0.xy: centre of the half-res texel in source uv
    let scr = vec2<f32>(uv.x * 2.0 - 1.0, -(uv.y * 2.0 - 1.0)); // v0.zw
    // r0.x = saturate(3 * normalize(ray).z)   (sky rule: blur fades with elevation)
    let ray = scr.x * u.rows0.xyz + scr.y * u.rows1.xyz + u.rows2.xyz;
    let s = sat(3.0 * ray.z * inverseSqrt(dot(ray, ray)));
    let d = gather_depth(uv);
    let t = u.inv_size.xy;                                    // cb0[0].zw
    // taps: (-.5,+.5) (+.5,+.5) (+.5,-.5) (-.5,-.5) texels, weight index = gather component
    var offs = array<vec2<f32>, 4>(vec2<f32>(-0.5, 0.5), vec2<f32>(0.5, 0.5), vec2<f32>(0.5, -0.5), vec2<f32>(-0.5, -0.5));
    var acc = vec4<f32>(0.0);
    for (var i = 0; i < 4; i = i + 1) {
        let lin = convert_from_device_z(d[i], u.depth_xf) * u.dist.w;
        let far = far_coc(lin);
        let w = select(far, far * (1.0 - s), u.dist.z < lin);  // lt cb0[18].x, depth
        let c = textureSampleLevel(color_tex, samp, uv + offs[i] * t, 0.0).rgb;
        acc = acc + vec4<f32>(c, 1.0) * w;
    }
    return acc * 0.25;
}

@fragment
fn fs_recombine_far(in: VsOut) -> @location(0) vec4<f32> {
    let uv = in.pos.xy * u.inv_size.xy;
    let d = load_depth(vec2<i32>(in.pos.xy));
    let lin = convert_from_device_z(d, u.depth_xf) * u.dist.w;
    let col = textureSampleLevel(color_tex, samp, uv, 0.0).rgb;
    let far = far_coc(lin);
    let sharp = pow(max((0.25 - far) * 4.0, 0.0), 2.0);       // weight of the un-blurred image
    let uvb = clamp(uv, u.clamp_rect.xy, u.clamp_rect.zw);
    let fb = textureSampleLevel(far_tex, samp, uvb, 0.0);
    let norm = (fb.rgb + col * 0.0001) / (fb.a + 0.0001);
    let sa = sqrt(fb.a);
    let a = sa * (3.0 - 2.0 * sqrt(sa));
    let far_mix = col + a * (norm - col);
    return vec4<f32>(far_mix + sharp * (col - far_mix), 0.0);
}
