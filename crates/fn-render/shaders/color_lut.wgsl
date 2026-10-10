// COLOR LUT (GLOBAL LOOK 2026-10-05): FCombineLUTs, the volume colour-grading LUT the UE 4.16 tonemapper samples. Appended after common.wgsl + post.wgsl
// (`lut_device_color` and the PostParams block `pp` live in post.wgsl; this module only uses `pp`, binding 0 of group 0).
// 1.8 DXBC of the global shader cache (GlobalShaderCache#1182809, the USE_VOLUME_LUT pixel shader writing one slice per SV_RenderTargetArrayIndex):
//   Neutral = (UV - 0.5/32) * 32/31 (layer / 31)          `add v0.xy, -0.015625` + `mul 1.032258`, `mul r1.z, layer, 0.032258`
//   LinearColor = LogToLin(Neutral) = 0.18 * 2^((Neutral - 0.434018) * 14)   (NO `- LogToLin(0)` in the 1.8 build, unlike the 4.16 source)
//   ... white balance, colour grade, film tone map, AP1 -> sRGB, MappingPolynomial, ColorScale / Overlay, pow(InverseGamma.y), LinearToSrgb ...
//   OutColor.rgb = OutDeviceColor / 1.05                    `mul o0.xyz, r0.xyzx, l(0.952381)`
// into a 32^3 PF_A2B10G10R10 (PostProcessCombineLUTs.cpp:1022) render target: the unorm write saturates and rounds to 10 bits. The tonemapper samples it
// bilinearly (clamp) at saturate(log2(c) / 14 + 0.610727) * 31/32 + 0.5/32 and multiplies by 1.05 (post.wgsl).
// The texture here is rgba16float (storage-writable in core WebGPU); the 10-bit unorm rounding is applied explicitly before the store.
@group(1) @binding(0) var lut_out: texture_storage_3d<rgba16float, write>;

@compute @workgroup_size(4, 4, 4)
fn combine_luts(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id >= vec3<u32>(32u))) { return; }
    let neutral = vec3<f32>(id) / 31.0;
    let linear = exp2((neutral - vec3<f32>(0.434018)) * 14.0) * 0.18;
    let stored = clamp(lut_device_color(linear) * (1.0 / 1.05), vec3<f32>(0.0), vec3<f32>(1.0));
    textureStore(lut_out, vec3<i32>(id), vec4<f32>(round(stored * 1023.0) / 1023.0, 0.0));
}
