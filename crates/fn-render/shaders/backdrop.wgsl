// BACKDROP pass: the landscape components beyond the main pass far plane, ocean planes, far skybox terraces. Prepended: common.wgsl, lit_common.wgsl (group 0:
// camera, light, fog tail), terrain_common.wgsl. Own camera-relative projection (group 1 binding 0) so the 2e6 m far plane does not disturb the main pass. No
// cascade shadows (everything here is beyond the shadow distance; the near landscape is drawn by the main pass, terrain.wgsl).
// TERRAIN MAIN PASS (2026-10-07): Bd / Mat, the group 1 bindings, tex_lin and the landscape layer shading live in terrain_common.wgsl.

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) wp: vec3<f32>,
    @location(1) n: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) col: vec4<f32>,
}

@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) col: vec4<f32>) -> VsOut {
    var o: VsOut;
    o.clip = bd.view_rot_proj * vec4<f32>(pos - bd.eye.xyz, 1.0);
    o.wp = pos;
    o.n = n;
    o.uv = uv;
    o.col = col;
    return o;
}

// sun diffuse + sky ambient (no shadow); the ambient share goes to alpha like every other lit path.
// REFLECTION APPLY: DefaultLit with the material's GBufferB Specular / Roughness (Metallic 0): the sun's GGX lobe (UE F_Schlick, lit_common.wgsl ggx_spec_color)
// and the deferred sky specular / SSR inputs (reflection_inputs, GBufferAO 1), like every DefaultLit GBuffer pixel in UE (ReflectionApplyPS).
fn shade_lit(albedo: vec3<f32>, n: vec3<f32>, wp: vec3<f32>, specular: f32, roughness: f32) -> LitOut {
    let cam = bd.eye.xyz;
    let ndl = max(dot(n, light.sun_dir.xyz), 0.0);
    let amb = albedo * ambient_light(n);
    let v = normalize(cam - wp);
    let spec_color = vec3<f32>(0.08 * specular);
    let lit = albedo * light.sun_radiance.xyz * ndl + amb + light.sun_radiance.xyz * 3.14159265 * ggx_spec_color(n, v, light.sun_dir.xyz, roughness, spec_color);
    reflection_inputs(n, v, wp, spec_color, roughness, 1.0);
    return lit_out(output_hdr(lit, amb, wp, cam));
}

// ---- landscape: terrain_common.wgsl terrain_surface (the reconstruction shared with the main-pass terrain). TERRAIN MAIN PASS (2026-10-07): was discarded
// inside the 512 m plate (`abs(wp.xz) < misc.x`); each component is now drawn by ONE pass: the main pass when it lies wholly within the main far plane, else
// this one (backdrop.rs prepare), so the two never shade the same surface (the depth merge would let either win the lit main pass Equal test) ----
fn shade_terrain(in: VsOut) -> LitOut {
    let n = normalize(in.n);
    let s = terrain_surface(in.wp, n, in.col);
    return shade_lit(s.rgb, n, in.wp, s.a, LANDSCAPE_ROUGHNESS);
}
// M_Skybox_Temperate_Terraces_01_moretiles (the far terraces' material; its _Inst sets no Spec / Roughness): the compiled PS writes
// GBufferB = (0, saturate(Spec), saturate(Roughness), DefaultLit) with the defaults Spec 0.1, Roughness 0.85.
const SKYBOX_SPECULAR: f32 = 0.1;
const SKYBOX_ROUGHNESS: f32 = 0.85;
// M_Athena_Water_Ocean: the compiled PS writes the constant GBufferB (Metallic 0, Specular 0.5, Roughness 0, DefaultLit).
const OCEAN_SPECULAR: f32 = 0.5;
const OCEAN_ROUGHNESS: f32 = 0.0;
// OCEAN MIN ROUGHNESS: the sun's LightMinRoughness = max(ULightComponent::MinRoughness 0.08, 0.02) (see shade_ocean; the same constant as fx_resolve.wgsl FX_SUN_MIN_ROUGHNESS)
const SUN_MIN_ROUGHNESS: f32 = 0.08;

// ---- far skybox rocks / terraces: p0.x overlay tile, p1.rgb tint ----
fn shade_skybox(in: VsOut) -> LitOut {
    let n = normalize(in.n);
    let base = tex_lin(t0, in.uv * mat.p0.x);
    let detail = tex_lin(t1, in.uv * mat.p0.y);
    let ao = textureSample(t2, rsamp, in.uv).r;
    let albedo = base * mix(vec3<f32>(1.0), detail * 1.6, mat.p0.z) * mix(1.0, ao, mat.p0.w) * mat.p1.rgb;
    return shade_lit(albedo, n, in.wp, SKYBOX_SPECULAR, SKYBOX_ROUGHNESS);
}

// ---- ocean: M_Athena_Water_Ocean (opaque; OCEAN MIN ROUGHNESS (2026-10-05): the sun lobe uses the compiled PS's GBufferB, Specular 0.5 -> SpecularColor 0.04 with
// UE's F_Schlick, Roughness 0 raised by the deferred light's `LobeRoughness = max(LobeRoughness, LightData.MinRoughness)` (DeferredLightingCommon.usf:164
// AreaLightSpecular; the directional light has SourceRadius / SourceLength 0, DirectionalLightComponent.cpp:176-180) with `LightMinRoughness = max(MinRoughness,
// .02)` and ULightComponent::MinRoughness 0.08 (exe ctor store 0x3da3d70a, fn-exe-defaults; the Athena sun does not override it) -> 0.08. It was the
// placeholder roughness 0.5 / F0 0.04 before). p0 = Water_Color, p1 = Water_Color_1, p2.x normal tile (m), p2.y near flatness, p2.z far flatness, p2.w pan speed (m/s) ----
fn shade_ocean(in: VsOut) -> LitOut {
    let cam = bd.eye.xyz;
    let dist = length(in.wp - cam);
    let t = bd.eye.w;
    let uv1 = in.wp.xz / mat.p2.x + vec2<f32>(t * mat.p2.w / mat.p2.x, t * 0.6 * mat.p2.w / mat.p2.x);
    let uv2 = in.wp.xz / (mat.p2.x * 0.37) - vec2<f32>(t * 0.5 * mat.p2.w / mat.p2.x, 0.0);
    let n1 = unpack_normal(textureSample(t0, rsamp, uv1));
    let n2 = unpack_normal(textureSample(t0, rsamp, uv2));
    var tn = normalize(vec3<f32>(n1.xy + n2.xy, n1.z * n2.z));
    // FlattenNormal with NormalNear 0.9266 -> NormalFar 0.9753 over FadeOffset 80.7 m + FadeLength 1252.9 m
    let flat_amt = mix(mat.p2.y, mat.p2.z, clamp((dist - 80.7) / 1252.9, 0.0, 1.0));
    tn = normalize(mix(tn, vec3<f32>(0.0, 0.0, 1.0), flat_amt));
    let n = normalize(vec3<f32>(tn.x, tn.z, tn.y));   // tangent x -> world x, tangent y -> world z (water UVs are world aligned)
    let v = normalize(cam - in.wp);
    let ndv = clamp(dot(n, v), 0.0, 1.0);
    let base = mix(mat.p1.rgb, mat.p0.rgb, pow(1.0 - ndv, 2.0));
    let sun = light.sun_dir.xyz;
    let ndl = max(dot(n, sun), 0.0);
    let amb = base * ambient_light(n);
    var lit = base * light.sun_radiance.xyz * ndl + amb;
    lit = lit + light.sun_radiance.xyz * 3.14159265 * ggx_spec_color(n, v, sun, max(OCEAN_ROUGHNESS, SUN_MIN_ROUGHNESS), vec3<f32>(0.08 * OCEAN_SPECULAR));
    // SKY REFLECTION HOOK: the sky light's processed specular cube (was the SH stand-in `sky_lookup(reflect(-v, n)) * fres`), deferred like every DefaultLit pixel,
    // with the GBufferB the compiled M_Athena_Water_Ocean PS writes (Specular 0.5 -> SpecularColor 0.04, Roughness 0)
    reflection_inputs(n, v, in.wp, vec3<f32>(0.08 * OCEAN_SPECULAR), OCEAN_ROUGHNESS, 1.0);
    return lit_out(output_hdr(lit, amb, in.wp, cam));
}

// PERF (2026-10-06): the scene depth of the depth pre-pass, which now runs before this pass: a fragment behind a nearer scene surface is skipped before any
// shading. The depth merge keeps the nearer depth and the scene pass draws over those pixels, so they never reached the image (same image, less work).
@group(2) @binding(0) var scene_depth: texture_depth_2d;
const BD_NEAR_M: f32 = 20.0;        // backdrop.rs BD_NEAR (backdrop_merge.wgsl)
const BD_FAR_M: f32 = 2.0e6;        // backdrop.rs BD_FAR
const BD_BELOW_ONE: f32 = 0.99999994;
// This fragment's view depth (m) from its backdrop-projection depth.
fn bd_view_depth(d: f32) -> f32 { return BD_NEAR_M * BD_FAR_M / (BD_FAR_M - d * (BD_FAR_M - BD_NEAR_M)); }
// This fragment's depth converted to the main projection exactly as backdrop_merge.wgsl converts the stored backdrop depth.
fn covered_by_scene(p: vec4<f32>) -> bool {
    let z = bd_view_depth(p.z);
    let n = bd.misc.y;
    let f = bd.misc.z;
    let main_depth = clamp(f / (f - n) * (1.0 - n / z), 0.0, BD_BELOW_ONE);
    return textureLoad(scene_depth, vec2<i32>(p.xy), 0) < main_depth;
}

@fragment
fn fs_main(in: VsOut) -> LitOut {
    let k = mat.kind.x;
    if (covered_by_scene(in.clip)) { discard; }
    if (k == 0u) { return shade_terrain(in); }
    if (k == 1u) { return shade_skybox(in); }
    return shade_ocean(in);
}
