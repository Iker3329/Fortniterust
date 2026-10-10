// AO ORDER: UE 4.16's `ScreenSpaceAO` (ssao_ue.wgsl, binding 11) applied to the HDR scene colour right after the opaque
// scene, i.e. BEFORE fog / translucency / light shafts / SceneColorHalfRes, like UE 4.16 where SSAO only reaches the lighting (SkyLighting.usf /
// ReflectionEnvironment: the sky light's diffuse is multiplied by the AO before any post-process pass). Appended to common.wgsl + post.wgsl;
// `scene` (binding 1) is a copy of the HDR target here (a pass cannot read its own attachment).
//
// REFLECTION APPLY (docs/spec/reflection_gaps.md sections 1-2): the same pass then adds the specular indirect light the lit passes deferred (lit_common.wgsl
// LitOut, reflection.rs), exactly in ReflectionApplyPS's order (ReflectionEnvironmentShaders.usf:537-665, no reflection captures, APPLY_SKYLIGHT):
//   AO = GBufferAO * ScreenSpaceData.AmbientOcclusion; SpecularOcclusion = saturate(Square(NoV + AO) - 1 + AO)        (:561-563)
//   SpecularLighting = (0, 0, 0, 1); .rgb = SSR.rgb + .rgb * (1 - SSR.a); .a *= 1 - SSR.a                              (SSR composite)
//   .rgb *= SpecularColor (= EnvBRDF * SO); .rgb += .a * (Sky * SpecularColor * IndirectSpecularOcclusion + ExtraIndirectSpecular * SpecularColor)
// = SO * (SSR.rgb * EnvBRDF + (1 - SSR.a) * refl.rgb) with refl.rgb = the lit pass's sky term without SO. Both inputs carry the pixel's fog transmittance
// (UE fogs after ReflectionApplyPS). ScreenSpaceData.AmbientOcclusion = `ScreenSpaceAO` of the UE 4.16 SSAO passes (ssao_ue.rs / ssao_ue.wgsl), which
// already hold the distance fade, AmbientOcclusionPower and AmbientOcclusionIntensity (MainPS full-res tail).
struct ApplyU { flags: vec4<f32> }   // x SSR result valid, y SSAO on, z debug: show the SSR result alone (FN_SSR_DEBUG, reflection.rs)
@group(0) @binding(6) var refl_sky: texture_2d<f32>;    // rgb sky specular (no SO) x fog transmittance, a NoV
@group(0) @binding(7) var refl_brdf: texture_2d<f32>;   // rgb EnvBRDF x fog transmittance, a GBufferAO
@group(0) @binding(8) var ssr_tex: texture_2d<f32>;     // SSROutput (reflection.rs), rgb x RoughnessFade x Intensity, a coverage
@group(0) @binding(9) var<uniform> au: ApplyU;
@group(0) @binding(11) var screen_space_ao: texture_2d<f32>;   // UE SSAO: ScreenSpaceAO (PF_G8, full res)

fn reflections(px: vec2<i32>, screen_ao: f32) -> vec3<f32> {
    let r = textureLoad(refl_sky, px, 0);
    let b = textureLoad(refl_brdf, px, 0);
    let ao = b.a * screen_ao;
    let so = clamp((r.a + ao) * (r.a + ao) - 1.0 + ao, 0.0, 1.0);
    var ssr = vec4<f32>(0.0);
    if (au.flags.x > 0.5) { ssr = textureLoad(ssr_tex, px, 0); }
    return so * (ssr.rgb * b.rgb + (1.0 - ssr.a) * r.rgb);
}

@fragment
fn fs_ssao(in: VsOut) -> @location(0) vec4<f32> {
    let px = vec2<i32>(in.clip.xy);
    let s = textureLoad(scene, px, 0);
    // debug view: the dimmed scene + the raw SSR (the SSR reads the previous frame, so the scene must stay in the image)
    if (au.flags.z > 0.5) { let r = textureLoad(ssr_tex, px, 0); return vec4<f32>(s.rgb * 0.15 * (1.0 - r.a) + r.rgb, s.a); }
    let screen_ao = textureLoad(screen_space_ao, px, 0).r;
    if (i32(pp.view.w) == 2) { return vec4<f32>(vec3<f32>(screen_ao), s.a); }   // debug: AO only (shown raw by post.wgsl debug mode 2)
    // AO acts on the ambient (sky) light only: the HDR alpha holds the ambient share of each pixel
    let ambient_share = clamp(s.a, 0.0, 1.0);
    // FX SHADOW AO COMBINE (2026-10-05): SkyLightDiffusePS combines the sky visibility with `min(GBuffer.GBufferAO, ScreenSpaceData.AmbientOcclusion)` in BOTH
    // OcclusionCombineMode branches (UE4.16 SkyLighting.usf:83 / :88; the Athena sky light is OCM_Multiply: DFAO x min(...)). The lit passes already put
    // GBufferAO into the ambient share (lit_common.wgsl df_sky_occlusion; the AO-free OcclusionTint term is not in the share) and record it in refl_brdf.a, so the ambient share is
    // scaled by min(GBufferAO, SSAO) / GBufferAO, not by SSAO (that was GBufferAO x SSAO). No GBufferAO recorded (0: nothing lit / cleared) keeps SSAO.
    let gbuffer_ao = textureLoad(refl_brdf, px, 0).a;
    let sky_ao = select(screen_ao, min(gbuffer_ao, screen_ao) / gbuffer_ao, gbuffer_ao > 1e-4);
    return vec4<f32>(s.rgb * (1.0 - ambient_share * (1.0 - sky_ao)) + reflections(px, screen_ao), s.a);
}

// SSAO off (r.AmbientOcclusionLevels 0): ScreenSpaceData.AmbientOcclusion = 1; added with a One / One colour blend (alpha untouched), no scene copy
@fragment
fn fs_reflect(in: VsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(reflections(vec2<i32>(in.clip.xy), 1.0), 0.0);
}
