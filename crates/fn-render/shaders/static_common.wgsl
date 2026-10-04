// Textured static meshes (real 1.8 trees / rocks): vertex path shared by the scene pass and the shadow cascades.
// Group 1 = material (the same layout in both modules), vertex slot 0 = StaticVertex, slot 1 = per-instance record (instance step mode).
//
// WIND / SHAKE: the UE wind (SpeedTree-like WPO nodes) is NOT recovered. ASSUMED model: a lateral gust along WIND_DIR whose top-of-mesh
// displacement is `sway * height` metres, weighted by a quadratic height ramp (or the vertex colour .r when the material says the exported
// colour is a real bend weight), plus a high-frequency per-vertex flutter on masked foliage. The hit shake is the old runtime's
// prop_wobble (x offset growing linearly with height). The shadow pass evaluates the same function so shadows follow the sway.

struct StaticMaterial {
    tint: vec4<f32>,   // rgb linear multiplier, a translucency (two-sided light transmission)
    p0: vec4<f32>,     // x alpha cutoff, y masked (0/1), z two-sided (0/1), w has normal map (0/1)
    p1: vec4<f32>,     // x roughness, y sun specular scale, z sway (fraction of height), w flutter (metres)
    p2: vec4<f32>,     // x wind weight from vertex colour .r, y albedo from vertex colour rgb, zw unused
}
@group(1) @binding(0) var<uniform> smat: StaticMaterial;
@group(1) @binding(1) var t_base: texture_2d<f32>;
@group(1) @binding(2) var t_nrm: texture_2d<f32>;
@group(1) @binding(3) var s_static: sampler;

struct InstIn {
    @location(4) pos_scale: vec4<f32>,
    @location(5) rot_fade: vec4<f32>,   // cos yaw, sin yaw, fade, shake amplitude
    @location(6) wind: vec4<f32>,       // phase, time, sway multiplier, 1 / mesh height
}

const WIND_DIR: vec2<f32> = vec2<f32>(0.8, 0.6);

struct StaticPos { world: vec3<f32>, normal: vec3<f32> }

fn static_transform(pos: vec3<f32>, normal: vec3<f32>, color: vec4<f32>, inst: InstIn) -> StaticPos {
    let c = inst.rot_fade.x;
    let s = inst.rot_fade.y;
    let scale = inst.pos_scale.w;
    // yaw about +Y (glam Quat::from_rotation_y): x' = c x + s z, z' = -s x + c z
    let rot = mat3x3<f32>(vec3<f32>(c, 0.0, -s), vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(s, 0.0, c));
    var world = rot * (pos * scale) + inst.pos_scale.xyz;
    let n = rot * normal;

    let h01 = clamp(pos.y * inst.wind.w, 0.0, 1.0);
    var weight = h01 * h01;
    if (smat.p2.x > 0.5) { weight = color.r; }
    let t = inst.wind.y;
    let ph = inst.wind.x;
    let gust = sin(t * 1.1 + ph) * 0.6 + sin(t * 2.3 + ph * 1.7) * 0.4;
    let side = sin(t * 0.8 + ph * 0.5 + 1.3);
    let amp = smat.p1.z * inst.wind.z * scale / max(inst.wind.w, 1e-4);   // sway * height (m)
    world.x = world.x + (WIND_DIR.x * gust - WIND_DIR.y * side * 0.35) * amp * weight;
    world.z = world.z + (WIND_DIR.y * gust + WIND_DIR.x * side * 0.35) * amp * weight;
    if (smat.p1.w > 0.0) {
        let k = dot(pos, vec3<f32>(3.1, 5.3, 4.7)) + ph;
        world = world + vec3<f32>(sin(t * 7.0 + k), sin(t * 6.1 + k * 1.3) * 0.5, sin(t * 8.3 + k * 0.7)) * smat.p1.w * scale * weight;
    }
    // hit shake: lateral x offset growing linearly with height (fn_combat::prop_wobble)
    world.x = world.x + h01 * inst.rot_fade.w;
    return StaticPos(world, n);
}
