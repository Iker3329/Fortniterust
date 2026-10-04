// BR character material pass (head / hair / body): the ORIGINAL compiled base-pass pixel shader of the material (char_material_gen.wgsl, translated from the cooked
// DXBC by fn-dxbc) evaluated per pixel, then lit like a deferred G-buffer: sun (cascaded shadows) + sky ambient + the shader's own emissive output.
//
// Prepended: common.wgsl, lit_common.wgsl, char_material_gen.wgsl (translated PS functions `head_ps` / `hair_ps` / `body_ps`).
//
// What the translated shader consumes (UE 4.16 base pass, TBasePassPSFNoLightMapPolicy of TGPUSkinVertexFactory):
//   v[0] = TangentToWorld[0] (tangent X), v[1] = TangentToWorld[2] (normal) with the bitangent sign in w, v[2] = vertex colour, v[3].xy = UV0,
//   v[4] = VELOCITY_PREV_POS, v[6] = SV_Position, v[7].x = SV_IsFrontFace (body only); View uniform buffer (cb1), primitive buffer (cb2), pass buffer (cb0), material buffer (cb3, evaluated on the CPU
//   from the cooked uniform expression set + the material instance parameters).
// Everything the shader reads is in Unreal world space (cm, X forward, Y right, Z up). The renderer's world is metres, Y up: u = (r.x, r.z, r.y) * 100 (the axis map
// of the prop / mesh export, see fn-app props.rs). The shader reconstructs the pixel's world position from SV_Position with View.SvPositionToTranslatedWorld (cb1[40..43]);
// here that matrix is replaced by the constant mapping (anything) -> the interpolated translated world position, which is the same point.
//
// Not evaluated (reported): the screen-space subsurface profile (SS_HeroSkin_01) pass, the velocity / selective-output targets o4 / o5 (the G-buffer of a renderer that does
// not have them), the checkerboard subsurface path (cb1[144].x = 0).

struct Material { kind: vec4<u32> }
@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var t_diffuse: texture_2d<f32>;
@group(1) @binding(2) var t_normal: texture_2d<f32>;
@group(1) @binding(3) var t_spec: texture_2d<f32>;
@group(1) @binding(4) var t_mask: texture_2d<f32>;

struct Object { model: mat4x4<f32> }
@group(2) @binding(0) var<uniform> object: Object;
@group(2) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;

// group 3: the material's constant buffer (cb3 registers, evaluated from the cooked uniform expression set) + its fixed texture slot (index 7 = UI_Faces/Noise)
struct CharMat { cb: array<vec4<f32>, 32>, perm_info: vec4<u32> }   // perm_info.x: permutation (0 head, 1 hair, 2 body); was `meta`, a reserved word in wgpu 30's naga
@group(3) @binding(0) var<uniform> cmat: CharMat;
@group(3) @binding(1) var fx_tex7: texture_2d<f32>;
@group(3) @binding(2) var fx_samp7: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

@vertex
fn vs_char(
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
) -> VsOut {
    var o: VsOut;
    let m = bones[joints.x] * weights.x + bones[joints.y] * weights.y + bones[joints.z] * weights.z + bones[joints.w] * weights.w;
    let p_local = (m * vec4<f32>(pos, 1.0)).xyz;
    var n_local = (m * vec4<f32>(normal, 0.0)).xyz;
    n_local = n_local / max(length(n_local), 1e-6);
    let wp = object.model * vec4<f32>(p_local, 1.0);
    o.clip = camera.view_proj * wp;
    o.world_pos = wp.xyz;
    o.world_normal = (object.model * vec4<f32>(n_local, 0.0)).xyz;
    o.uv = uv;
    return o;
}

// renderer world (m, Y up) <-> Unreal world (cm, Z up)
fn to_ue(r: vec3<f32>) -> vec3<f32> { return vec3<f32>(r.x, r.z, r.y); }
fn from_ue(u: vec3<f32>) -> vec3<f32> { return vec3<f32>(u.x, u.z, u.y); }

struct GBuf { emissive: vec3<f32>, normal_ue: vec3<f32>, metallic: f32, specular: f32, roughness: f32, shading_model: f32, base: vec3<f32>, ao: f32 }

fn char_run(perm: u32, v: array<vec4<f32>, 8>, tw: vec3<f32>, cam_ue: vec3<f32>, time: f32) -> GBuf {
    var o: array<vec4<f32>, 6>;
    // head / hair declare 7 inputs (no front-face flag read), body 8
    var v7: array<vec4<f32>, 7>;
    for (var i = 0; i < 7; i = i + 1) { v7[i] = v[i]; }
    if (perm == 0u) {
        head_cb0[3] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        head_cb1[43] = vec4<f32>(tw, 1.0);
        head_cb1[62] = vec4<f32>(-cam_ue, 0.0);
        head_cb1[131] = vec4<f32>(0.0, 0.0, time, 0.0);
        for (var i = 0; i < 27; i = i + 1) { head_cb3[i] = cmat.cb[i]; }
        o = head_ps(v7);
    } else if (perm == 1u) {
        hair_cb0[3] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        hair_cb1[43] = vec4<f32>(tw, 1.0);
        hair_cb1[62] = vec4<f32>(-cam_ue, 0.0);
        hair_cb1[131] = vec4<f32>(0.0, 0.0, time, 0.0);
        for (var i = 0; i < 27; i = i + 1) { hair_cb3[i] = cmat.cb[i]; }
        o = hair_ps(v7);
    } else {
        body_cb0[3] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        body_cb1[43] = vec4<f32>(tw, 1.0);
        body_cb1[62] = vec4<f32>(-cam_ue, 0.0);
        body_cb1[130] = vec4<f32>(0.0, 0.0, 0.0, 1.0);   // View.CullingSign (1 for the main view)
        body_cb1[131] = vec4<f32>(0.0, 0.0, time, 0.0);
        body_cb2[9] = vec4<f32>(0.0, 0.0, 0.0, 1.0);     // Primitive.LocalToWorldDeterminantSign (the UE mesh is not mirrored)
        for (var i = 0; i < 27; i = i + 1) { body_cb3[i] = cmat.cb[i]; }
        o = body_ps(v);
    }
    var g: GBuf;
    g.emissive = o[0].xyz;
    g.normal_ue = normalize(o[1].xyz * 2.0 - 1.0);
    g.metallic = o[2].x;
    g.specular = o[2].y;
    g.roughness = o[2].z;
    g.shading_model = o[2].w;
    g.base = o[3].xyz;
    g.ao = o[3].w;
    return g;
}

@fragment
fn fs_char(in: VsOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let cam = camera.position.xyz;
    let p = in.world_pos;
    let n0 = normalize(in.world_normal);
    // tangent frame of the renderer (screen-space derivatives, the exported meshes carry no tangents): same maths as perturb_normal
    let dp1 = dpdx(p);
    let dp2 = gl_dfdy3(p);
    let duv1 = dpdx(in.uv);
    let duv2 = gl_dfdy2(in.uv);
    let dp2perp = cross(dp2, n0);
    let dp1perp = cross(n0, dp1);
    let t_r = dp2perp * duv1.x + dp1perp * duv2.x;
    let b_r = dp2perp * duv1.y + dp1perp * duv2.y;
    let n_u = normalize(to_ue(n0));
    let t_u = normalize(to_ue(t_r) - n_u * dot(n_u, to_ue(t_r)));
    // the shader builds the bitangent as cross(Z, X) * sign: pick the sign that reproduces the renderer's bitangent direction
    let w_sign = select(-1.0, 1.0, dot(cross(n_u, t_u), to_ue(b_r)) >= 0.0);
    let tw = (to_ue(p) - to_ue(cam)) * 100.0;
    var v: array<vec4<f32>, 8>;
    v[0] = vec4<f32>(t_u, 0.0);
    v[1] = vec4<f32>(n_u, w_sign);
    v[2] = vec4<f32>(1.0);
    v[3] = vec4<f32>(in.uv, 0.0, 0.0);
    v[4] = vec4<f32>(0.0, 0.0, 0.0, 1.0);   // VELOCITY_PREV_POS (feeds only the velocity target o4, which is not used)
    v[6] = in.clip;   // SV_Position: the VsOut @builtin(position) is the fragment coordinate (must appear only once)
    v[7] = vec4<f32>(bitcast<f32>(select(0u, 1u, front)), 0.0, 0.0, 0.0);
    let g = char_run(cmat.perm_info.x, v, tw, to_ue(cam) * 100.0, camera.position.w);

    let n = normalize(from_ue(g.normal_ue));
    let view = normalize(cam - p);
    let lit_enabled = light.misc.x > 0.5;
    if (!lit_enabled) { return output_hdr(g.base, vec3<f32>(0.0), p, cam); }
    // UE DefaultLit on the G-buffer values: DiffuseColor = BaseColor (1 - Metallic), SpecularColor = lerp(0.08 Specular, BaseColor, Metallic)
    let diffuse_color = g.base * (1.0 - g.metallic);
    let f0 = mix(vec3<f32>(0.08 * g.specular), g.base, g.metallic);
    let sun_dir = light.sun_dir.xyz;
    let ndl = clamp(dot(n, sun_dir), 0.0, 1.0);
    let vis = sun_visibility(p, n, ndl);
    let nl = ndl * vis;
    let ambient = ambient_light(n) * g.ao;
    // GGX (D_GGX * Vis_SmithJointApprox * F_Schlick) like lit_common::ggx_spec but with a coloured F0
    let h = normalize(view + sun_dir);
    let nv = max(dot(n, view), 1e-3);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let vh = clamp(dot(view, h), 0.0, 1.0);
    let a = g.roughness * g.roughness;
    let a2 = a * a;
    let dd = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (3.14159265 * dd * dd);
    let vis_term = 0.5 / (ndl * (nv * (1.0 - a) + a) + nv * (ndl * (1.0 - a) + a) + 1e-4);
    let fres = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
    // sun_radiance already carries the Lambert 1/pi; the GGX BRDF is not divided by pi here, so its light is Intensity = x pi (as the ground path does)
    let spec = light.sun_radiance.xyz * 3.14159265 * d * vis_term * fres * ndl * vis;
    let amb_color = diffuse_color * ambient;
    let lit = diffuse_color * light.sun_radiance.xyz * nl + amb_color + spec + g.emissive;
    return output_hdr(lit, amb_color, p, cam);
}
