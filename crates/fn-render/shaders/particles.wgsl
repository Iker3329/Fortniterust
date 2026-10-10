// Particle pass: camera facing / velocity aligned sprites with sub-UV sheets, and instanced mesh particles. Appended after common.wgsl + lit_common.wgsl
// (camera / light blocks, group 0, height fog, sky ambient). Group 1 = the material. Blend state (translucent / additive / modulate / opaque+masked) is in the pipeline.
// Everything the cooked material graph cannot tell us is expressed through the material uniform (see particles.rs): texture, which channel is opacity, which channel
// drives a 2 / 3 colour ramp, opacity exponent, tint.

struct Mat {
    tint: vec4<f32>,
    ramp0: vec4<f32>,
    ramp1: vec4<f32>,
    ramp2: vec4<f32>,
    p0: vec4<f32>,   // x clip, y opacity exponent, z flags (1 unlit, 2 masked, 4 has texture, 8 additive, 16 modulate, 32 opaque/masked base pass), w ramp mode (0 none, 2, 3)
    p1: vec4<f32>,   // x opacity channel (-1 none), y value channel (-1 none)
    p2: vec4<f32>,   // PARTICLE REFLECTIONS: x GBufferB known (1), y Metallic, z Specular, w Roughness (particles.rs ParticleMaterialDesc::gbuffer)
}
@group(1) @binding(0) var<uniform> pmat: Mat;
@group(1) @binding(1) var ptex: texture_2d<f32>;
@group(1) @binding(2) var psamp: sampler;

fn flag(bit: f32) -> bool { return ((u32(pmat.p0.z) / u32(bit)) % 2u) >= 1u; }

struct VOut {
    // FULL PREPASS (2026-10-06): the opaque / masked batches' depth pre-pass twin runs this same vertex entry (same module) as their base-pass pipeline (depth Equal)
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv0: vec2<f32>,
    @location(2) uv1: vec2<f32>,
    @location(3) blend: f32,
    @location(4) color: vec4<f32>,
    @location(5) normal: vec3<f32>,
    @location(6) dynp: vec4<f32>,
}

// ---- sprites ---------------------------------------------------------------------------------------------------------------------------
struct SpriteIn {
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec4<f32>,   // pos.xyz, rotation (rad)
    @location(1) b: vec4<f32>,   // size.xy (m), sub image index, dynamic 0
    @location(2) color: vec4<f32>,
    @location(3) c: vec4<f32>,   // velocity.xyz (m/s), dynamic 1
    @location(4) d: vec4<f32>,   // columns, rows, flags (1 blend frames, 2 velocity aligned), dynamic 2
    @location(5) e: vec4<f32>,   // pivot offset xy, dynamic 3, unused
}

fn corner(vi: u32) -> vec2<f32> {
    var c = array<vec2<f32>, 6>(vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0));
    return c[vi];
}
fn cell_uv(c: vec2<f32>, index: f32, cols: f32, rows: f32) -> vec2<f32> {
    let n = max(cols * rows, 1.0);
    let i = clamp(floor(index), 0.0, n - 1.0);
    let col = i - floor(i / cols) * cols;
    let row = floor(i / cols);
    return (c + vec2<f32>(col, row)) / vec2<f32>(cols, rows);
}

@vertex
fn vs_sprite(in: SpriteIn) -> VOut {
    let c = corner(in.vi);
    let pos = in.a.xyz;
    let to_cam = normalize(camera.position.xyz - pos);
    var right = camera.view_inverse[0].xyz;
    var up = camera.view_inverse[1].xyz;
    let aligned = ((u32(in.d.z) / 2u) % 2u) >= 1u;
    let speed = length(in.c.xyz);
    if (aligned && speed > 1e-3) {
        up = in.c.xyz / speed;
        right = normalize(cross(up, to_cam));
    } else {
        let s = sin(in.a.w); let co = cos(in.a.w);
        let r2 = right * co + up * s; let u2 = up * co - right * s;
        right = r2; up = u2;
    }
    let local = vec2<f32>(c.x - 0.5 + in.e.x, 0.5 - c.y + in.e.y);
    var o: VOut;
    o.world = pos + right * (local.x * in.b.x) + up * (local.y * in.b.y);
    o.clip = camera.view_proj * vec4<f32>(o.world, 1.0);
    let cols = max(in.d.x, 1.0); let rows = max(in.d.y, 1.0);
    o.uv0 = cell_uv(c, in.b.z, cols, rows);
    o.uv1 = cell_uv(c, in.b.z + 1.0, cols, rows);
    o.blend = select(0.0, fract(in.b.z), ((u32(in.d.z)) % 2u) >= 1u);
    o.color = in.color;
    o.normal = to_cam;
    o.dynp = vec4<f32>(in.b.w, in.c.w, in.d.w, in.e.z);
    return o;
}

// ---- meshes ----------------------------------------------------------------------------------------------------------------------------
struct MeshIn {
    @location(0) pos: vec3<f32>,
    @location(1) nrm: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) r0: vec4<f32>,   // rows of orient * scale, translation in w
    @location(4) r1: vec4<f32>,
    @location(5) r2: vec4<f32>,
    @location(6) color: vec4<f32>,
    @location(7) dynp: vec4<f32>,
}
@vertex
fn vs_mesh(in: MeshIn) -> VOut {
    let p = vec4<f32>(in.pos, 1.0);
    let world = vec3<f32>(dot(in.r0, p), dot(in.r1, p), dot(in.r2, p));
    // normal: rotate with the matrix, undo the scale (rows are scaled by the same factors)
    let n = vec3<f32>(dot(in.r0.xyz, in.nrm) / max(dot(in.r0.xyz, in.r0.xyz), 1e-8), dot(in.r1.xyz, in.nrm) / max(dot(in.r1.xyz, in.r1.xyz), 1e-8), dot(in.r2.xyz, in.nrm) / max(dot(in.r2.xyz, in.r2.xyz), 1e-8));
    var o: VOut;
    o.world = world;
    o.clip = camera.view_proj * vec4<f32>(world, 1.0);
    o.uv0 = in.uv; o.uv1 = in.uv; o.blend = 0.0;
    o.color = in.color;
    o.normal = normalize(n);
    o.dynp = in.dynp;
    return o;
}

// ---- shading -------------------------------------------------------------------------------------------------------------------------
fn chan(t: vec4<f32>, ch: f32) -> f32 {
    if (ch < 0.5) { return t.r; }
    if (ch < 1.5) { return t.g; }
    if (ch < 2.5) { return t.b; }
    return t.a;
}

// FULL PREPASS (2026-10-06): the depth pre-pass entry of a MASKED opaque batch (particles.rs `depth_pipeline`): UE4.16 runs every opaque / masked primitive in
// the pre-pass (cooked r.SelectiveBasePassOutputs 1 / r.StencilForLODDither 1 -> ShouldForceFullDepthPass -> DDM_AllOpaque + movable, DeferredShadingRenderer.cpp:146-182)
// with the material's coverage clip (FDepthOnlyPS, r.EarlyZPassOnlyMaterialMasking 1); the base pass then tests CF_Equal. The coverage below is fs_main's, expression
// for expression (same texture fetches, same opacity), so both entries keep the same pixels. Opaque (non-masked) batches have no fragment stage in the pre-pass.
@fragment
fn fs_depth(in: VOut) {
    var t = vec4<f32>(1.0);
    if (flag(4.0)) {
        t = textureSample(ptex, psamp, in.uv0);
        if (in.blend > 0.0) { t = mix(t, textureSample(ptex, psamp, in.uv1), in.blend); }
    }
    var opacity = 1.0;
    if (pmat.p1.x > -0.5) { opacity = pow(max(chan(t, pmat.p1.x), 0.0), pmat.p0.y); }
    let alpha = opacity * in.color.a * pmat.tint.a;
    if (flag(2.0) && alpha < pmat.p0.x) { discard; }
}

@fragment
// REFLECTION APPLY: LitOut (lit_common.wgsl) because the opaque / masked batches draw inside the scene pass, whose pipelines all carry the reflection targets;
// opaque lit batches record reflection inputs only when their material's GBufferB is known (PARTICLE REFLECTIONS below), else zeros = no sky specular / SSR.
// Blended pipelines have one target.
fn fs_main(in: VOut, @builtin(front_facing) front: bool) -> LitOut {
    var t = vec4<f32>(1.0);
    if (flag(4.0)) {
        t = textureSample(ptex, psamp, in.uv0);
        if (in.blend > 0.0) { t = mix(t, textureSample(ptex, psamp, in.uv1), in.blend); }
    }
    var opacity = 1.0;
    if (pmat.p1.x > -0.5) { opacity = pow(max(chan(t, pmat.p1.x), 0.0), pmat.p0.y); }
    var base = t.rgb;
    if (!flag(4.0)) { base = vec3<f32>(1.0); }
    let mode = u32(pmat.p0.w);
    if (mode == 2u) {
        let v = clamp(chan(t, pmat.p1.y), 0.0, 1.0);
        base = mix(pmat.ramp0.rgb, pmat.ramp1.rgb, v);
    } else if (mode == 3u) {
        let v = clamp(chan(t, pmat.p1.y), 0.0, 1.0);
        base = select(mix(pmat.ramp0.rgb, pmat.ramp1.rgb, v * 2.0), mix(pmat.ramp1.rgb, pmat.ramp2.rgb, (v - 0.5) * 2.0), v >= 0.5);
    }
    base = base * pmat.tint.rgb * in.color.rgb;
    let alpha = opacity * in.color.a * pmat.tint.a;
    if (flag(2.0) && alpha < pmat.p0.x) { discard; }
    if (flag(32.0)) {
        // PARTICLE BASE PASS (particles.rs module docs): opaque / masked particles are base-pass primitives, lit like the static meshes (static_scene.wgsl): the sun
        // through the cascade / distance-field shadows (sun_visibility) and the sky ambient, then output_hdr (DFAO sky occlusion, height fog, the ambient share in alpha
        // for the AO pass). Unlit (MSM_Unlit): emissive only, fog applied by the same tail.
        let cam = camera.position.xyz;
        if (flag(1.0)) { return lit_out(output_hdr(base, vec3<f32>(0.0), in.world, cam)); }
        var n = normalize(in.normal);
        if (dot(n, normalize(cam - in.world)) < 0.0) { n = -n; }
        // PARTICLE REFLECTIONS (2026-10-05): with a KNOWN GBufferB (pmat.p2) the pixel is a full DefaultLit GBuffer pixel: DiffuseColor / SpecularColor of
        // DecodeGBuffer (BaseColor - BaseColor * Metallic, lerp(0.08 * Specular, BaseColor, Metallic)), the sun's GGX lobe with the directional light's
        // LobeRoughness = max(Roughness, MinRoughness 0.08) (DeferredLightingCommon.usf:164) and the reflection inputs (sky specular + SSR, ReflectionApplyPS, GBufferAO 1:
        // the AmbientOcclusion input default). Without it (graph-driven inputs) the diffuse-only shading stays and no reflection inputs are written.
        let known = pmat.p2.x > 0.5;
        let metallic = select(0.0, pmat.p2.y, known);
        let diffuse_color = base * (1.0 - metallic);
        let spec_color = mix(vec3<f32>(0.08 * pmat.p2.z), base, metallic);
        var lit = base;
        var amb = vec3<f32>(0.0);
        let v = normalize(cam - in.world);
        if (light.misc.x > 0.5) {
            let sun = light.sun_dir.xyz;
            let ndl = dot(n, sun);
            let n_sun = select(-n, n, ndl >= 0.0);
            let vis = sun_visibility(in.world, n_sun, abs(ndl));
            amb = diffuse_color * ambient_light(n);
            lit = diffuse_color * light.sun_radiance.xyz * max(ndl, 0.0) * vis + amb;
            if (known && ndl > 0.0) { lit = lit + light.sun_radiance.xyz * 3.14159265 * ggx_spec_color(n, v, sun, max(pmat.p2.w, 0.08), spec_color) * vis; }
        }
        if (known) { reflection_inputs(n, v, in.world, spec_color, pmat.p2.w, 1.0); }
        return lit_out(output_hdr(lit + base * pmat.p1.z, amb, in.world, cam));
    }
    var rgb = base;
    if (!flag(1.0)) {
        // default lit: sun + sky ambient on a camera facing normal (particles do not receive cascade shadows)
        var n = normalize(in.normal);
        let v = normalize(camera.position.xyz - in.world);
        if (dot(n, v) < 0.0) { n = -n; }
        let ndl = max(dot(n, light.sun_dir.xyz), 0.0);
        rgb = base * (light.sun_radiance.xyz * ndl + ambient_light(n) + vec3<f32>(pmat.p1.z));
    }
    if (light.misc.w > 0.5) {
        let fog = height_fog(light.fog, in.world, camera.position.xyz, light.sun_dir.xyz, -camera.view_inverse[2].xyz);
        rgb = rgb * fog.a + select(fog.rgb, vec3<f32>(0.0), flag(8.0));
    }
    if (flag(8.0)) { return lit_out(vec4<f32>(rgb * alpha, alpha)); }
    if (flag(16.0)) { return lit_out(vec4<f32>(mix(vec3<f32>(1.0), rgb, alpha), alpha)); }
    return lit_out(vec4<f32>(rgb, alpha));
}
