// TERRAIN MAIN PASS (2026-10-07): the landscape layer shading shared by the backdrop pass (backdrop.wgsl, components beyond the main pass far plane) and the
// main-pass terrain (terrain.wgsl, depth pre-pass + lit base pass + CSM casters). Prepended after common.wgsl / lit_common.wgsl, before either of them, so the
// reconstruction below is defined ONCE. NOT exact (labelled): M_Athena_Terrain_Landscape's arithmetic nodes are not serialised in the cooked packages, so the layer
// blend here is a reconstruction from its parameters + textures (layer tiling, rock slope window ASSUMED, backdrop_load.rs TILES_M / ROCK_SLOPE).

struct Bd {
    view_rot_proj: mat4x4<f32>,
    eye: vec4<f32>,      // xyz camera position (world), w time
    misc: vec4<f32>,     // x unused (was the plate half edge), y / z main pass near / far (m)
}
struct Mat {
    kind: vec4<u32>,     // x: 0 terrain, 1 skybox, 2 ocean
    p0: vec4<f32>, p1: vec4<f32>, p2: vec4<f32>, p3: vec4<f32>,
}
@group(1) @binding(0) var<uniform> bd: Bd;
@group(1) @binding(1) var<uniform> mat: Mat;
@group(1) @binding(2) var t0: texture_2d<f32>;
@group(1) @binding(3) var t1: texture_2d<f32>;
@group(1) @binding(4) var t2: texture_2d<f32>;
@group(1) @binding(5) var t3: texture_2d<f32>;
@group(1) @binding(6) var t4: texture_2d<f32>;
@group(1) @binding(7) var t5: texture_2d<f32>;
@group(1) @binding(8) var rsamp: sampler;

// SRGB BEFORE FILTER (2026-10-07): every group-1 texture is uploaded with its cooked SRGB flag (backdrop.rs BdMaterial::srgb): an SRGB=true slot is an
// Rgba8UnormSrgb view the sampler decodes BEFORE filtering, as D3D11 does for UE's _SRGB views, so the samples are already linear (was to_linear after the filter).
fn tex_lin(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> { return textureSample(t, rsamp, uv).rgb; }

// REFLECTION APPLY: the GBufferB of the Athena landscape component materials (compiled base-pass PS of the LandscapeMaterialInstanceConstants in
// Athena_Terrain_LS_00, all 48 permutations): Metallic 0, Roughness 0.8, Specular = saturate(pow(max(|GrassD.a|, 1e-6), GrassSpecPower) * GrassSpecAmount *
// GrassWeight), M_Athena_Terrain_Landscape defaults GrassSpecPower 2 / GrassSpecAmount 0.3 (cb slot -> name by parameter name: ASSUMED). Same as scene.wgsl.
const LANDSCAPE_ROUGHNESS: f32 = 0.8;
fn landscape_specular(grass_alpha: f32, grass_weight: f32) -> f32 {
    return clamp(pow(max(abs(grass_alpha), 1e-6), 2.0) * 0.3 * grass_weight, 0.0, 1.0);
}

// ---- landscape: layer weights in the vertex colour (r grass, g forest, b road, a mud), steep slopes show the rock layers ----
// p0.x forest tile (m), p0.y mud tile (m), p0.z road tile (m), p0.w rock tile (m); p1.x grass tile (m); p1.y rock slope start, p1.z rock slope end
// Returns the albedo (rgb) and the GBuffer Specular (a). `n` = the unit vertex normal.
// PERF (2026-10-07, main-pass terrain): only the layers with weight are sampled (most pixels carry one or two of the four, and no rock below the slope window):
// the UV derivatives are taken once in uniform control flow and the samples use textureSampleGrad with them (legal in non-uniform flow, the same mip / anisotropy
// as the implicit derivatives of the unbranched samples it replaces: same image).
fn lin_grad(t: texture_2d<f32>, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> vec3<f32> { return textureSampleGrad(t, rsamp, uv, gx, gy).rgb; }
fn rock_triplanar(t: texture_2d<f32>, wp: vec3<f32>, dx: vec3<f32>, dy: vec3<f32>, w: vec3<f32>, s: f32) -> vec3<f32> {
    var r = vec3<f32>(0.0);
    if (w.x > 0.0) { r = r + lin_grad(t, wp.zy * s, dx.zy * s, dy.zy * s) * w.x; }
    if (w.y > 0.0) { r = r + lin_grad(t, wp.xz * s, dx.xz * s, dy.xz * s) * w.y; }
    if (w.z > 0.0) { r = r + lin_grad(t, wp.xy * s, dx.xy * s, dy.xy * s) * w.z; }
    return r;
}
fn terrain_surface(wp: vec3<f32>, n: vec3<f32>, col: vec4<f32>) -> vec4<f32> {
    let xz = wp.xz;
    let dx = dpdx(wp);
    let dy = dpdy(wp);
    var base = vec3<f32>(0.0);
    var spec = 0.0;
    if (col.r > 0.0) {
        let s = 1.0 / mat.p1.x;
        let grass_tex = textureSampleGrad(t0, rsamp, xz * s, dx.xz * s, dy.xz * s);
        base = grass_tex.rgb * col.r;
        // the landscape GBufferB of its compiled component PS (LANDSCAPE_ROUGHNESS above): Specular from the grass layer's T_Grasslands_D alpha x the grass weight
        // (vertex colour r), Roughness 0.8; the rock / mud / road / forest layers add no Specular term in that PS
        spec = landscape_specular(grass_tex.a, col.r);
    }
    if (col.g > 0.0) { let s = 1.0 / mat.p0.x; base = base + lin_grad(t1, xz * s, dx.xz * s, dy.xz * s) * col.g; }
    if (col.b > 0.0) { let s = 1.0 / mat.p0.z; base = base + lin_grad(t3, xz * s, dx.xz * s, dy.xz * s) * col.b; }
    if (col.a > 0.0) { let s = 1.0 / mat.p0.y; base = base + lin_grad(t2, xz * s, dx.xz * s, dy.xz * s) * col.a; }
    var albedo = base;
    let rock_w = smoothstep(mat.p1.y, mat.p1.z, 1.0 - n.y);
    if (rock_w > 0.0) {
        // the rock layers are projected on all three axes in the material (TCMT_XZ / TCMT_YZ coords): blend by |n|
        let a = abs(n);
        let w = a / max(a.x + a.y + a.z, 1e-4);
        let s = 1.0 / mat.p0.w;
        let rock = mix(rock_triplanar(t4, wp, dx, dy, w, s), rock_triplanar(t5, wp, dx, dy, w, s), 0.5);
        albedo = mix(base, rock, rock_w);
    }
    return vec4<f32>(albedo * mat.p2.rgb, spec);
}
