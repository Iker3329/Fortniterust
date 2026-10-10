// LIGHT SHAFT HOOK: light-shaft BLOOM of the Athena sun, port of UE 4.16 LightShaftShader.usf (directional light, OCCLUSION_TERM 0):
//  fs_downsample = DownsampleLightShaftsPixelMain, fs_blur = BlurLightShaftsMain (NUM_SAMPLES = r.LightShaftNumSamples 12),
//  fs_apply = ApplyLightShaftsPixelMain (additive, colour channels only). Parameters and pass setup: light_shafts.rs (LightShaftRendering.cpp).
// Units as in UE: scene depth in cm (the port's depth buffer is converted), UVs over the (half-res) filter buffer.

struct LsParams {
    blur_origin: vec4<f32>,      // xy TextureSpaceBlurOrigin
    aspect: vec4<f32>,           // AspectRatioAndInvAspectRatio
    uv_min_max: vec4<f32>,       // UVMinMax
    params: vec4<f32>,           // LightShaftParameters: 1 / OcclusionDepthRange (1/cm), BloomScale, 1, OcclusionMaskDarkness
    tint_threshold: vec4<f32>,   // BloomTintAndThreshold
    depth: vec4<f32>,            // port only: camera near, far (m) of the depth buffer, full-res width, height
}
struct Radial { p: vec4<f32> }   // RadialBlurParameters: NumSamples, FirstPassDistance, PassIndex

@group(0) @binding(0) var<uniform> ls: LsParams;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var scene_depth: texture_depth_2d;
@group(0) @binding(3) var src_samp: sampler;          // TStaticSamplerState<SF_Bilinear, AM_Clamp ...>
@group(0) @binding(4) var<uniform> radial: Radial;

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}

// CalcSceneColor / CalcSceneDepth: SceneTextureParams.Set with the default SF_Point filter (SceneRenderTargets.cpp), i.e. the texel under the UV.
fn texel_of(uv: vec2<f32>, dims: vec2<u32>) -> vec2<i32> {
    return clamp(vec2<i32>(floor(uv * vec2<f32>(dims))), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
}
fn calc_scene_depth_cm(uv: vec2<f32>) -> f32 {
    let d = textureLoad(scene_depth, texel_of(uv, textureDimensions(scene_depth)), 0);
    // cleared depth = nothing drawn (sky): UE's infinite far plane gives an unbounded SceneDepth there
    if (d >= 1.0) { return 3.0e38; }
    let n = ls.depth.x;
    let f = ls.depth.y;
    return n * f / (f - d * (f - n)) * 100.0;
}

// DownsampleLightShaftsPixelMain, !OCCLUSION_TERM, !POINT_LIGHT_SHAFTS. The target is the half-res filter buffer; InUV = scene-buffer UV of the
// pixel centre (DrawRectangle of the downsampled rect over the full view rect).
@fragment
fn fs_downsample(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    // src = full-res scene colour; the downsampled rect is ViewSize / 2 (floor) and maps onto the whole view
    let uv = frag.xy / floor(vec2<f32>(textureDimensions(src)) / 2.0);
    let scene_color = textureLoad(src, texel_of(uv, textureDimensions(src)), 0).rgb;
    let scene_depth_cm = calc_scene_depth_cm(uv);
    let nc = (uv - ls.uv_min_max.xy) / ls.uv_min_max.zw;
    var edge = 1.0 - nc.x * (1.0 - nc.x) * nc.y * (1.0 - nc.y) * 8.0;
    edge = edge * edge * edge * edge;
    let luminance = max(dot(scene_color, vec3<f32>(0.3, 0.59, 0.11)), 6.10352e-5);
    let adjusted = max(luminance - ls.tint_threshold.a, 0.0);
    let bloom = ls.params.y * scene_color / luminance * adjusted * 2.0;
    let inv_range = ls.params.x;
    let bloom_distance_mask = clamp((scene_depth_cm - 0.5 / inv_range) * inv_range, 0.0, 1.0);
    let origin_mask = 1.0 - clamp(length(ls.blur_origin.xy - uv * ls.aspect.zw) * 2.0, 0.0, 1.0);
    return vec4<f32>(bloom * ls.tint_threshold.rgb * bloom_distance_mask * (1.0 - edge) * origin_mask * origin_mask, 1.0);
}

// BlurLightShaftsMain (NUM_SAMPLES 12), over the whole filter buffer (InUV = pixel centre / FilterBufferSize).
const NUM_SAMPLES: i32 = 12;
@fragment
fn fs_blur(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let in_uv = frag.xy / vec2<f32>(textureDimensions(src));
    let acuv = in_uv * ls.aspect.zw;
    let pass_scale = pow(0.4 * f32(NUM_SAMPLES), radial.p.z);
    let acbv = (ls.blur_origin.xy - acuv) * min(radial.p.y * pass_scale, 1.0);
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < NUM_SAMPLES; i = i + 1) {
        let suv = (acuv + acbv * f32(i) / f32(NUM_SAMPLES)) * ls.aspect.xy;
        let c = clamp(suv, ls.uv_min_max.xy, ls.uv_min_max.zw);
        sum = sum + textureSampleLevel(src, src_samp, c, 0.0).rgb;
    }
    return vec4<f32>(sum / f32(NUM_SAMPLES), 1.0);
}

// ApplyLightShaftsPixelMain: full-res, bilinear tap of the blurred filter buffer, blended One + One into the colour channels (CW_RGB).
@fragment
fn fs_apply(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let full = vec2<f32>(textureDimensions(scene_depth));
    let ds = floor(full / 2.0);
    let uv = frag.xy / full * ds / vec2<f32>(textureDimensions(src));
    return vec4<f32>(textureSampleLevel(src, src_samp, uv, 0.0).rgb, 1.0);
}
