// UE 4.16 screen-space ambient occlusion: a line-by-line port of Engine/Shaders/PostProcessAmbientOcclusion.usf (MainSetupPS, MainPS with
// SSAO_NORMAL_MODE 1 and OPTIMIZATION_O1, ComputeUpsampleContribution, the quad message passing blur), checked instruction by instruction against the 1.8
// global shader cache: FPostProcessAmbientOcclusionSetupPS1 (GlobalShaderCache#496380), FPostProcessAmbientOcclusionPS 1 0 2 (#586389, half res, setup
// as input, no upsample, shader quality 2) and 0 1 2 (#600241, full res, upsample, shader quality 2). Units are UE's (cm, UE view space: x right,
// y up, z forward); `ssao_ue.rs` fills the uniform exactly like FScreenSpaceAOParameters::Set / FPostProcessAmbientOcclusionSetupPS::SetParameters.

struct AoU {
    // ScreenSpaceAOParams[6]:
    // [0] x AmbientOcclusionPower, y Bias / 1000, z 1 / AmbientOcclusionDistance, w AmbientOcclusionIntensity
    // [1] xy ViewportUVToRandomUV, z AORadiusInShader, w Ratio
    // [2] x ScaleToFullRes, y MipThreshold / ScaleToFullRes, z ScaleRadiusInWorldSpace, w MipBlend
    // [3] xy TemporalAA random offset, z StaticFraction, w InvTanHalfFov
    // [4] x fade multiplier, y fade additive, z HzbStepMipLevelFactor
    // [5] xy view rect size
    p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>, p4: vec4<f32>, p5: vec4<f32>,
    hzb_remap: vec4<f32>,     // HZBRemapping: xy mul, zw add (ScreenPos -> HZB UV)
    setup: vec4<f32>,         // AmbientOcclusionSetupParams: x ScaleToFullRes, y InvThreshold, zw view size
    in0_size: vec4<f32>,      // PostprocessInput0Size (w, h, 1 / w, 1 / h)
    in1_size: vec4<f32>,      // PostprocessInput1Size
    in2_size: vec4<f32>,      // PostprocessInput2Size
    out_size: vec4<f32>,      // this pass's render target (w, h, 1 / w, 1 / h) and the full-res buffer: x ScaleToFullRes of the target
    buffer_size: vec4<f32>,   // full-res buffer (w, h, 1 / w, 1 / h)
    to_view0: vec4<f32>,      // rows of the world -> UE view rotation (TranslatedWorldToView, renderer world axes in)
    to_view1: vec4<f32>,
    to_view2: vec4<f32>,
    depth: vec4<f32>,         // x port near (m), y port far (m), z UE near (cm) of the HZB device Z
}

@group(0) @binding(0) var<uniform> u: AoU;
@group(0) @binding(1) var depth_tex: texture_depth_2d;                // SceneDepth (port depth, full res)
@group(0) @binding(2) var gbuf_tex: texture_2d<f32>;                  // GBuffer world normal (reflection target 2: xyz normal, 0 = unlit)
@group(0) @binding(3) var in_setup: texture_2d<f32>;                  // PostprocessInput0 / 1: AO setup (rgb normal * 0.5 + 0.5, a depth / 131072)
@group(0) @binding(4) var in_lower: texture_2d<f32>;                  // PostprocessInput2: the lower resolution AO
@group(0) @binding(5) var hzb_tex: texture_2d<f32>;                   // PostprocessInput3: View.HZB (UE device Z, furthest of each block)
@group(0) @binding(6) var random_tex: texture_2d<f32>;                // RandomNormalTexture (GSystemTextures.SSAORandomization, 64x64 R8G8)
@group(0) @binding(7) var point_clamp: sampler;                       // TStaticSamplerState<SF_Point, AM_Clamp ...>
@group(0) @binding(8) var point_wrap: sampler;                        // TStaticSamplerState<SF_Point, AM_Wrap ...>
@group(0) @binding(9) var in_setup_up: texture_2d<f32>;               // PostprocessInput1 of the upsample (the setup of the lower level's input)

// permutation (FPostProcessAmbientOcclusionPSandCS<bTAOSetupAsInput, bDoUpsample, ShaderQuality> / FPostProcessAmbientOcclusionSetupPS<bInitialPass>)
override USE_AO_SETUP_AS_INPUT: bool = false;
override USE_UPSAMPLE: bool = false;
override SHADER_QUALITY: u32 = 2u;
override INITIAL_PASS: bool = true;

const CONSTANT_FLOAT16F_SCALE: f32 = 131072.0;   // 4096 * 32

struct VsOut { @builtin(position) pos: vec4<f32> }

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var o: VsOut;
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    return o;
}

// ---- permutation constants (#if SHADER_QUALITY ... / USE_AO_SETUP_AS_INPUT) ----
fn sample_steps() -> u32 {
    if (SHADER_QUALITY <= 1u) { return 1u; }
    if (SHADER_QUALITY == 2u) { return 2u; }
    return 3u;
}
// USE_SAMPLESET: 1 (3 points) at qualities 0..3, 3 (6 points) at quality 4 and for every AO-setup input pass
fn sampleset_size() -> u32 {
    if (USE_AO_SETUP_AS_INPUT || SHADER_QUALITY >= 4u) { return 6u; }
    return 3u;
}
fn sample_offset(i: u32) -> vec2<f32> {
    if (sampleset_size() == 3u) {
        // 3 points distributed on the unit disc, spiral order and distance
        if (i == 0u) { return vec2<f32>(0.0, -1.0) * 0.43; }
        if (i == 1u) { return vec2<f32>(0.58, 0.814) * 0.7; }
        return vec2<f32>(-0.58, 0.814);
    }
    // 6 points distributed on the unit disc, spiral order and distance
    switch i {
        case 0u: { return vec2<f32>(0.000, 0.200); }
        case 1u: { return vec2<f32>(0.325, 0.101); }
        case 2u: { return vec2<f32>(0.272, -0.396); }
        case 3u: { return vec2<f32>(-0.385, -0.488); }
        case 4u: { return vec2<f32>(-0.711, 0.274); }
        default: { return vec2<f32>(0.060, 0.900); }
    }
}
// QUAD_MESSAGE_PASSING_BLUR 2 (normal aware) at qualities 1 and 2, 0 otherwise
fn quad_blur() -> bool { return SHADER_QUALITY == 1u || SHADER_QUALITY == 2u; }

// ---- scene inputs ----
fn texel_of(uv: vec2<f32>, size: vec2<f32>) -> vec2<i32> {
    return clamp(vec2<i32>(floor(uv * size)), vec2<i32>(0), vec2<i32>(size) - vec2<i32>(1));
}
// CalcSceneDepth (point): the port's depth -> linear view depth in cm; the cleared depth is UE's infinite far plane
fn calc_scene_depth(uv: vec2<f32>) -> f32 {
    let d = textureLoad(depth_tex, texel_of(uv, u.buffer_size.xy), 0);
    if (d >= 1.0) { return 1.0e9; }
    let n = u.depth.x;
    let f = u.depth.y;
    return n * f / (f - d * (f - n)) * 100.0;
}
// GetGBufferData(UV, false).WorldNormal (point): DecodeNormal(GBufferA.xyz) = N * 2 - 1 (DeferredShadingCommon.usf:126 / :524). The port stores the unit world
// normal and 0 where nothing lit was drawn (sky, unlit); UE's GBufferA there is cleared to 0, which decodes to (-1, -1, -1) (symmetric, so the same in both axis
// conventions): the AO passes then normalize a finite vector exactly as UE does instead of normalize(0) = NaN.
fn gbuffer_normal(uv: vec2<f32>) -> vec3<f32> {
    let n = textureLoad(gbuf_tex, texel_of(uv, u.buffer_size.xy), 0).xyz;
    return select(n, vec3<f32>(-1.0), all(n == vec3<f32>(0.0)));
}
// ConvertFromDeviceZ of the HZB (UE reversed infinite projection: SceneDepth = Near / DeviceZ)
fn convert_from_device_z(device_z: f32) -> f32 { return u.depth.z / max(device_z, 1.0e-10); }
fn to_ue_view(n: vec3<f32>) -> vec3<f32> { return vec3<f32>(dot(u.to_view0.xyz, n), dot(u.to_view1.xyz, n), dot(u.to_view2.xyz, n)); }

// 0: not similar .. 1: very similar
fn compute_depth_similarity(a: f32, b: f32, tweak_scale: f32) -> f32 { return clamp(1.0 - abs(a - b) * tweak_scale, 0.0, 1.0); }

// ---- MainSetupPS: downsample depth + normal for the lower resolution passes ----
@fragment
fn fs_setup(in: VsOut) -> @location(0) vec4<f32> {
    let in_uv = in.pos.xy * u.out_size.zw;
    // INITIAL_PASS: PostprocessInput0Size (the scene), else PostprocessInput1Size (the previous setup)
    let pixel = select(u.in1_size.zw, u.in0_size.zw, INITIAL_PASS);
    var uvs = array<vec2<f32>, 4>(in_uv + vec2<f32>(-0.5, -0.5) * pixel, in_uv + vec2<f32>(0.5, -0.5) * pixel,
                                  in_uv + vec2<f32>(-0.5, 0.5) * pixel, in_uv + vec2<f32>(0.5, 0.5) * pixel);
    var samples: array<vec4<f32>, 4>;
    for (var i = 0u; i < 4u; i = i + 1u) {
        samples[i] = vec4<f32>(normalize(gbuffer_normal(uvs[i])) * 0.5 + 0.5, calc_scene_depth(uvs[i])); // GetGBufferData(UV[i], true): normalized (usf:239)
    }
    let max_z = max(max(samples[0].a, samples[1].a), max(samples[2].a, samples[3].a));
    var avg = vec4<f32>(0.0001);
    let inv_threshold = u.setup.y;
    for (var i = 0u; i < 4u; i = i + 1u) {
        avg = avg + vec4<f32>(samples[i].rgb, 1.0) * compute_depth_similarity(samples[i].a, max_z, inv_threshold);
    }
    let rgb = avg.rgb / avg.w;
    return vec4<f32>(rgb, max_z / CONSTANT_FLOAT16F_SCALE);
}

// ---- MainPS ----
fn get_depth_from_ao_input(uv: vec2<f32>) -> f32 {
    if (USE_AO_SETUP_AS_INPUT) {
        return textureSampleLevel(in_setup, point_clamp, uv, 0.0).a * CONSTANT_FLOAT16F_SCALE;
    }
    return calc_scene_depth(uv);
}
fn get_world_space_normal_from_ao_input(uv: vec2<f32>) -> vec3<f32> {
    if (USE_AO_SETUP_AS_INPUT) {
        return textureSampleLevel(in_setup, point_clamp, fract(uv), 0.0).xyz * 2.0 - 1.0;
    }
    return gbuffer_normal(uv);
}

fn compute_upsample_contribution(scene_depth: f32, in_uv: vec2<f32>, center_world_normal: vec3<f32>) -> vec4<f32> {
    let small = 0.0001;
    var weight_sum = small;
    var ret = vec4<f32>(small, 0.0, 0.0, 0.0);
    var min_iteration = 1.0;
    // AO_UPSAMPLE_QUALITY 1 (9 samples) for the AO-setup input passes, 0 (4 samples) at full resolution
    let nine = USE_AO_SETUP_AS_INPUT;
    let count = select(4u, 9u, nine);
    for (var i = 0u; i < count; i = i + 1u) {
        var o: vec2<f32>;
        if (nine) {
            o = vec2<f32>(f32(i % 3u) - 1.0, f32(i / 3u) - 1.0);
        } else {
            // UV[0] (-0.5, 0.5), UV[1] (0.5, 0.5), UV[2] (-0.5, -0.5), UV[3] (0.5, -0.5)
            o = vec2<f32>(select(-0.5, 0.5, (i & 1u) == 1u), select(0.5, -0.5, i >= 2u));
        }
        let uv = in_uv + o * u.in2_size.zw;
        let sample_value = textureSampleLevel(in_lower, point_clamp, uv, 0.0);
        min_iteration = min(min_iteration, sample_value.g);
        let normal_and_depth = textureSampleLevel(in_setup_up, point_clamp, uv, 0.0);
        let sample_depth = normal_and_depth.a * CONSTANT_FLOAT16F_SCALE;
        // when tweaking this constant look for crawling pattern at edges
        var weight = compute_depth_similarity(sample_depth, scene_depth, 0.003);
        let local_world_normal = normal_and_depth.xyz * 2.0 - 1.0;
        weight = weight * clamp(dot(local_world_normal, center_world_normal), 0.0, 1.0);
        ret = ret + vec4<f32>(sample_value.rgb, 1.0) * weight;
        weight_sum = weight_sum + weight;
    }
    ret = ret / weight_sum;
    ret.g = min_iteration;
    return ret;
}

// AO_SAMPLE_QUALITY is 3 in every permutation: blend with the upsampled lower level by MipBlend when there is one
fn compute_lerp_factor() -> f32 {
    if (!USE_UPSAMPLE) { return 0.0; }
    return u.p2.w;
}

fn screen_pos_to_hzb_uv(p: vec2<f32>) -> vec2<f32> { return u.hzb_remap.xy * p + u.hzb_remap.zw; }

fn compute_mip_level(sample_id: u32, step: u32) -> f32 {
    let sample_pos = (f32(sample_id) + 0.5) / f32(sampleset_size());
    let factor = u.p4.z;
    let scale = f32(step + 1u) / f32(sample_steps());
    return log2(factor * scale * sample_pos);
}

// WedgeWithNormal, OPTIMIZATION_O1: float3(InvNormAngleL, InvNormAngleR, Weight = 1)
fn wedge_with_normal(center: vec2<f32>, local_random: vec2<f32>, inv_fov_fix: vec3<f32>, view_pos: vec3<f32>, scaled_normal: vec3<f32>, mip: f32) -> vec2<f32> {
    let pos_l = center + local_random;
    let pos_r = center - local_random;
    let tex_l = textureSampleLevel(hzb_tex, point_clamp, screen_pos_to_hzb_uv(pos_l), mip).r;
    let tex_r = textureSampleLevel(hzb_tex, point_clamp, screen_pos_to_hzb_uv(pos_r), mip).r;
    let abs_l = convert_from_device_z(tex_l);
    let abs_r = convert_from_device_z(tex_r);
    let sample_l = vec3<f32>(pos_l * abs_l, abs_l);
    let sample_r = vec3<f32>(pos_r * abs_r, abs_r);
    let delta_l = (sample_l - view_pos) * inv_fov_fix;
    let delta_r = (sample_r - view_pos) * inv_fov_fix;
    let inv_l = clamp(dot(delta_l, scaled_normal) / dot(delta_l, delta_l), 0.0, 1.0);
    let inv_r = clamp(dot(delta_r, scaled_normal) / dot(delta_r, delta_r), 0.0, 1.0);
    return vec2<f32>(inv_l, inv_r);
}

fn reconstruct_normal(v: vec2<f32>) -> vec3<f32> { return vec3<f32>(v, sqrt(1.0 - dot(v, v))); }

@fragment
fn fs_ao(in: VsOut) -> @location(0) vec4<f32> {
    let power = u.p0.x;
    let ratio = u.p1.w;
    let ao_radius_in_shader = u.p1.z;
    let intensity = u.p0.w;
    let viewport_uv_to_random_uv = u.p1.xy;
    let bias = u.p0.y;
    let scale_factor = u.p2.x;
    let scale_radius_in_ws = u.p2.z;

    // UVAndScreenPos: the input texture UV of this pixel and its screen position (-1..1, y up)
    let uv = in.pos.xy * u.out_size.zw;
    let screen_pos = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);

    let inv_tan_half_fov = u.p3.w;
    let fov_fix = vec3<f32>(inv_tan_half_fov, ratio * inv_tan_half_fov, 1.0);
    let inv_fov_fix = 1.0 / fov_fix;

    let scene_depth = get_depth_from_ao_input(uv);
    let world_normal = get_world_space_normal_from_ao_input(uv);
    let view_space_normal = normalize(to_ue_view(world_normal));
    var view_pos = vec3<f32>(screen_pos * scene_depth, scene_depth);
    let actual_radius = ao_radius_in_shader * mix(scene_depth, 1.0, scale_radius_in_ws);
    // add bias after fixup (SSAO_NORMAL_MODE 1)
    view_pos = view_pos + bias * scene_depth * scale_factor * (view_space_normal * fov_fix);

    var weight_acc = vec2<f32>(0.0001);
    // AO_SAMPLE_QUALITY 3: one of 16 base vectors (rotation and scale) from a texture that repeats 4x4, jittered per frame with temporal AA
    let temporal_offset = u.p3.xy;
    let random_vec = (textureSampleLevel(random_tex, point_wrap, temporal_offset + uv * viewport_uv_to_random_uv, 0.0).rg * 2.0 - 1.0) * actual_radius;
    let fov_fix_xy = fov_fix.xy * (1.0 / view_pos.z);
    let random_base = vec4<f32>(random_vec, -random_vec.y, random_vec.x) * vec4<f32>(fov_fix_xy, fov_fix_xy);
    let center = view_pos.xy / view_pos.z;
    // OPTIMIZATION_O1
    let scaled_normal = view_space_normal * (0.08 * mix(scene_depth, 1000.0, scale_radius_in_ws));
    let n_samples = sampleset_size();
    let steps = sample_steps();
    for (var i = 0u; i < n_samples; i = i + 1u) {
        let unrotated = sample_offset(i);
        let local_random = unrotated.x * random_base.xy + unrotated.y * random_base.zw;
        var acc = vec3<f32>(0.0);
        for (var step = 0u; step < steps; step = step + 1u) {
            let scale = f32(step + 1u) / f32(steps);
            let mip = compute_mip_level(i, step);
            let s = wedge_with_normal(center, scale * local_random, inv_fov_fix, view_pos, scaled_normal, mip);
            // combine horizon samples (Weight is 1 with OPTIMIZATION_O1)
            acc = vec3<f32>(max(acc.xy, s), 1.0);
        }
        // Square(): the area scales quadratic with the angle - it gets a bit darker
        weight_acc = weight_acc + vec2<f32>((1.0 - acc.x) * (1.0 - acc.x) * acc.z, acc.z);
        weight_acc = weight_acc + vec2<f32>((1.0 - acc.y) * (1.0 - acc.y) * acc.z, acc.z);
    }

    var ao = weight_acc.x / weight_acc.y;
    if (USE_UPSAMPLE) {
        let filtered = compute_upsample_contribution(scene_depth, uv, world_normal);
        // recombined result from multiple resolutions
        ao = mix(ao, filtered.r, compute_lerp_factor());
    }
    if (!USE_AO_SETUP_AS_INPUT) {
        // full res: soft fade out AO in the distance, then the user AO adjust
        ao = mix(ao, 1.0, clamp(scene_depth * u.p4.x + u.p4.y, 0.0, 1.0));
        ao = 1.0 - (1.0 - pow(abs(ao), power)) * intensity;
    }

    // QUAD_MESSAGE_PASSING_BLUR (SM5): .x AO output, .y SceneDepth, .zw view space normal
    var out_ao = ao;
    if (quad_blur()) {
        let c = vec4<f32>(ao, scene_depth, view_space_normal.xy);
        let dx = dpdxFine(c);
        let dy = dpdyFine(c);
        let m = vec2<f32>(vec2<u32>(in.pos.xy) % vec2<u32>(2u));
        let pix_a = c;
        let pix_b = c - dx * (m.x * 2.0 - 1.0);
        let pix_c = c - dy * (m.y * 2.0 - 1.0);
        var wa = 1.0;
        var wb = 1.0;
        var wc = 1.0;
        // QUAD_MESSAGE_PASSING_NORMAL
        let na = reconstruct_normal(pix_a.zw);
        let nb = reconstruct_normal(pix_b.zw);
        let ncc = reconstruct_normal(pix_c.zw);
        wb = wb * clamp(pow(clamp(dot(na, nb), 0.0, 1.0), 4.0), 0.0, 1.0);
        wc = wc * clamp(pow(clamp(dot(na, ncc), 0.0, 1.0), 4.0), 0.0, 1.0);
        let inv = 1.0 / (wa + wb + wc);
        out_ao = wa * inv * pix_a.x + wb * inv * pix_b.x + wc * inv * pix_c.x;
    }
    if (!USE_AO_SETUP_AS_INPUT || quad_blur()) {
        // full res: "OutColor = OutColor.r" (single channel); with the blur every channel is the blurred value
        return vec4<f32>(out_ao);
    }
    return vec4<f32>(out_ao, 0.0, 0.0, 0.0);
}
