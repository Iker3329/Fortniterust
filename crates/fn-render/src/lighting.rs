//! Sun / sky-light constants recovered from the original packages, terrain colour variation and cascaded shadow map fitting.
//! Pure maths (no GPU): ported from FortniteRuntime/src/lighting.rs, with the shadow matrices adapted to wgpu clip space
//! (depth 0..1, texture v pointing down).
//!
//! Values come from reports/sun-shadow-lighting.json of the prototype (tools/sun-shadow-evidence.cjs). Unreal units are cm,
//! rotators are (Pitch, Yaw, Roll) degrees; this crate works in metres in the "GL world" (X right, Y up, -Z forward).
use glam::{Mat4, Vec3, Vec4};

/// TODM_A DayNightDirectionalLightComponent.RelativeRotation (the native TOD tick that animates it is C++ and NOT recovered).
pub const SUN_ROTATION_PYR_DEG: [f32; 3] = [-61.238_25, -90.036_71, 88.621_44];
/// TODM_A directional light: DynamicShadowDistanceMovableLight 4500 cm, CascadeDistributionExponent 2.
pub const SHADOW_DISTANCE_M: f32 = 45.0;
pub const CASCADE_EXPONENT: f32 = 2.0;
/// DynamicShadowCascades is not serialized; 3 is the assumed default (unproven).
pub const CASCADE_COUNT: usize = 3;
/// TODM_A_Child_Athena day phase: LightColor FColor and Brightness.
pub const SUN_COLOR_SRGB: [u8; 3] = [255, 225, 161];
pub const SUN_BRIGHTNESS: f32 = 3.0;
/// Day-phase SkyLightValues.SkyLightColor (linear RGB; alpha 1.0).
pub const SKY_LIGHT_COLOR: [f32; 3] = [3.575, 4.175_99, 5.0];
/// Camera height used when integrating the captured sky (eye of the default third-person target, metres).
const SKY_CAPTURE_EYE_M: f32 = 1.75;
/// UDirectionalLightComponent::LightSourceAngle = 1.0 deg: RECOVERED from the 1.8 exe class ctor (reports exe-defaults.json, store at
/// RVA 0x20a4823). The TODM_A sun light does not serialize it (reports/sun-shadow-lighting.json), so the ctor default applies (the old
/// 0.5357 was a UE 4.18 value recalled from memory). Used only to size the PCF penumbra.
pub const LIGHT_SOURCE_ANGLE_DEG: f32 = 1.0;
/// Assumed occluder-to-receiver distance for the penumbra estimate (APPROXIMATION; original uses distance-field soft shadows).
pub const PENUMBRA_OCCLUDER_M: f32 = 3.0;
/// Receiver offsets, in shadow texels (tuned in the prototype against acne / peter-panning).
pub const NORMAL_OFFSET_TEXELS: f32 = 1.0;
pub const CONSTANT_BIAS_TEXELS: f32 = 1.5;

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
/// Unreal world (X forward, Y right, Z up, cm) to the renderer world (X right, Y up, -Z forward).
pub fn ue_to_world(v: Vec3) -> Vec3 { Vec3::new(v.y, v.z, -v.x) }
/// Direction the light travels, from an Unreal rotator (FRotator::Vector()).
pub fn light_travel_direction(pitch_deg: f32, yaw_deg: f32) -> Vec3 {
    let (p, y) = (pitch_deg.to_radians(), yaw_deg.to_radians());
    ue_to_world(Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())).normalize()
}
/// Unit vector from the scene towards the sun. Shared by sky disc, diffuse lighting and shadow maps.
pub fn sun_direction() -> Vec3 { -light_travel_direction(SUN_ROTATION_PYR_DEG[0], SUN_ROTATION_PYR_DEG[1]) }
/// Linear sun radiance. UE's Lambert BRDF divides albedo by pi, so Brightness 3 means 3/pi for white albedo.
pub fn sun_radiance() -> [f32; 3] {
    let k = SUN_BRIGHTNESS / std::f32::consts::PI;
    std::array::from_fn(|i| srgb_to_linear(SUN_COLOR_SRGB[i]) * k)
}
/// Cosine-weighted mean radiance of the upper hemisphere as the sky dome is rendered (recovered M_AridSky_Inst_Day colours and
/// horizon band, fogged by the recovered ExponentialHeightFog), i.e. what a UE sky-light capture of this sky would hold.
/// APPROXIMATIONS: dome treated as 1.5e6 m away in every direction, clouds/sun/stars ignored, SH projection replaced by the
/// shader's `0.5 + 0.5 n.y` hemisphere weight.
pub fn sky_capture_radiance() -> [f32; 3] {
    use std::sync::OnceLock;
    static CAPTURE: OnceLock<[f32; 3]> = OnceLock::new();
    *CAPTURE.get_or_init(|| {
        let eye = Vec3::new(0., SKY_CAPTURE_EYE_M, 0.);
        let sun = sun_direction();
        let (ne, na) = (96usize, 24usize);
        let (mut sum, mut wsum) = ([0f64; 3], 0f64);
        for ie in 0..ne {
            let elevation = (ie as f32 + 0.5) / ne as f32 * std::f32::consts::FRAC_PI_2;
            let lat = crate::sky::mesh_latitude_for_elevation(elevation, crate::sky::STAR_DOME_SCALE[2] / crate::sky::STAR_DOME_SCALE[0]);
            let base = crate::sky::dome_base_color(crate::sky::uv2y_for_latitude(lat));
            let weight = (elevation.sin() * elevation.cos()) as f64; // cos(theta to zenith) * solid-angle element
            for ia in 0..na {
                let az = (ia as f32 + 0.5) / na as f32 * std::f32::consts::TAU;
                let dir = Vec3::new(elevation.cos() * az.cos(), elevation.sin(), elevation.cos() * az.sin());
                let (ins, t) = crate::sky::fog_cpu(eye, eye + dir * 1.5e6, sun);
                for c in 0..3 { sum[c] += weight * (base[c] * t + ins[c]) as f64; }
                wsum += weight;
            }
        }
        std::array::from_fn(|c| (sum[c] / wsum) as f32)
    })
}
/// Linear sky-light radiance fed to the lit shader: captured sky x SkyLightColor x Intensity 1.
pub fn sky_radiance() -> [f32; 3] {
    let c = sky_capture_radiance();
    std::array::from_fn(|i| c[i] * SKY_LIGHT_COLOR[i])
}

// ---- Terrain grass colour (M_Athena_Terrain_Landscape / MF_TerrainColorVariation), mirrored by shaders/scene.wgsl ----
pub const TERRAIN_HIGHLIGHT: [f32; 3] = [3., 2.337_033, 1.38];
pub const TERRAIN_EDGE_DARK: [f32; 3] = [0.9, 0.9, 0.9];
pub const TERRAIN_MASK_CONTRAST: f32 = -2.;
/// Measured mean G of `T_Athena_Terrain_Mask`; retained for the future map-backed landscape material.
pub const TERRAIN_MASK_MEAN: f32 = 0.208_6;
pub fn cheap_contrast(x: f32, contrast: f32) -> f32 { let s = x * x * (3. - 2. * x); x + (s - x) * contrast }
/// Albedo multiplier for a map-backed terrain texel (linear). The temporary flat baseplate uses neutral
/// albedo until it has the landscape's real mask UV mapping.
pub fn terrain_color_multiplier() -> [f32; 3] {
    let a = cheap_contrast(TERRAIN_MASK_MEAN, TERRAIN_MASK_CONTRAST).clamp(0., 1.);
    std::array::from_fn(|i| TERRAIN_EDGE_DARK[i] + (TERRAIN_HIGHLIGHT[i] - TERRAIN_EDGE_DARK[i]) * a)
}

// ---- Ground extent (visual only) ----
/// Four rectangles (x0,z0),(x0,z1),(x1,z1),(x1,z0) that surround the square [-half,half]^2 out to +-outer, wound like the
/// baseplate quad. The playable plate is 512 m; the original terrain runs for kilometres, so the ground is continued out to the
/// far plane where the recovered fog (max opacity 0.8, reached after ~1.8 km) blends it into the sky horizon colour.
pub fn ground_ring(half: f32, outer: f32) -> [[(f32, f32); 4]; 4] {
    let q = |x0: f32, z0: f32, x1: f32, z1: f32| [(x0, z0), (x0, z1), (x1, z1), (x1, z0)];
    [q(-outer, -outer, outer, -half), q(-outer, half, outer, outer), q(-outer, -half, -half, half), q(half, -half, outer, half)]
}

// ---- Cascaded shadow map fitting ----
#[derive(Clone, Copy, Debug)]
pub struct Cascade {
    /// world -> light clip space (orthographic, wgpu depth 0..1)
    pub light_view_proj: Mat4,
    /// view-space depth (metres) at which this cascade ends
    pub far: f32,
    /// world metres covered by one shadow-map texel
    pub texel: f32,
    pub radius: f32,
}
/// Split ends for `count` cascades over [near, distance]; UE-style exponent distribution (approximation of UE's split).
pub fn cascade_splits(near: f32, distance: f32, count: usize, exponent: f32) -> Vec<f32> {
    (1..=count).map(|i| near + (distance - near) * (i as f32 / count as f32).powf(exponent)).collect()
}
/// Fit stable (texel-snapped) orthographic cascades to the camera frustum slices.
/// `view_inverse` maps eye space to world; `tan_half_h` is tan(horizontal fov/2); `aspect` is w/h.
pub fn fit_cascades(view_inverse: Mat4, tan_half_h: f32, aspect: f32, near: f32, to_sun: Vec3, size: u32) -> [Cascade; CASCADE_COUNT] {
    let splits = cascade_splits(near, SHADOW_DISTANCE_M, CASCADE_COUNT, CASCADE_EXPONENT);
    let up = if to_sun.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let mut start = near;
    std::array::from_fn(|c| {
        let end = splits[c];
        let mut corners = [Vec3::ZERO; 8];
        for (k, d) in [start, end].into_iter().enumerate() {
            let (hx, hy) = (tan_half_h * d, tan_half_h / aspect * d);
            for (j, (sx, sy)) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)].into_iter().enumerate() {
                corners[k * 4 + j] = view_inverse.transform_point3(Vec3::new(sx * hx, sy * hy, -d));
            }
        }
        start = end;
        let center = corners.iter().copied().sum::<Vec3>() / 8.;
        // Sphere radius is rotation invariant, so the map does not shimmer when the camera turns.
        let radius = (corners.iter().map(|p| p.distance(center)).fold(0., f32::max) * 16.).ceil() / 16.;
        let texel = 2. * radius / size as f32;
        let pad = 60.; // casters between the sun and the slice
        let view = Mat4::look_at_rh(center + to_sun * (radius + pad), center, up);
        // Snap the centre to the texel grid in light space.
        let ls = view.transform_point3(center);
        let snapped = Vec3::new((ls.x / texel).round() * texel, (ls.y / texel).round() * texel, ls.z);
        let shift = Mat4::from_translation(Vec3::new(snapped.x - ls.x, snapped.y - ls.y, 0.));
        let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0., 2. * radius + pad);
        Cascade { light_view_proj: proj * shift * view, far: end, texel, radius }
    })
}
/// world -> shadow-map texture coordinates (xy in 0..1 with v pointing down as in wgpu, z = depth 0..1).
pub fn texture_matrix(c: &Cascade) -> Mat4 {
    Mat4::from_cols(Vec4::new(0.5, 0., 0., 0.), Vec4::new(0., -0.5, 0., 0.), Vec4::new(0., 0., 1., 0.), Vec4::new(0.5, 0.5, 0., 1.)) * c.light_view_proj
}

#[cfg(test)]
mod tests {
    use super::*;
    fn camera(yaw_deg: f32) -> Mat4 {
        // eye -> world for a camera at (1,1.5,2) looking down yaw/pitch -10.
        (Mat4::from_rotation_x(-10f32.to_radians()) * Mat4::from_rotation_y(yaw_deg.to_radians()) * Mat4::from_translation(Vec3::new(1., 1.5, 2.))).inverse()
    }
    fn luma(c: [f32; 3]) -> f32 { 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2] }
    #[test]
    fn sun_direction_is_unit_and_above_horizon() {
        let d = sun_direction();
        assert!((d.length() - 1.).abs() < 1e-6);
        assert!(d.y > 0.85 && d.y < 0.89);
    }
    #[test]
    fn sun_direction_matches_serialized_rotator_math() {
        // Pitch -61.24 => 61.24 deg elevation; yaw -90.04 => sun on Unreal +Y => +X.
        let d = sun_direction();
        assert!((d.y - 61.238_25f32.to_radians().sin()).abs() < 1e-5);
        assert!(d.x > 0.47 && d.x < 0.49);
        assert!(d.z.abs() < 1e-3);
    }
    #[test]
    fn ue_axes_map_to_renderer_world() {
        assert_eq!(ue_to_world(Vec3::X), -Vec3::Z);
        assert_eq!(ue_to_world(Vec3::Y), Vec3::X);
        assert_eq!(ue_to_world(Vec3::Z), Vec3::Y);
    }
    #[test]
    fn splits_follow_exponent_and_end_at_shadow_distance() {
        let s = cascade_splits(0.1, 45., 3, 2.);
        assert_eq!(s.len(), 3);
        assert!((s[2] - 45.).abs() < 1e-4);
        assert!(s[0] < s[1] && s[1] < s[2]);
        assert!((s[0] - (0.1 + 44.9 / 9.)).abs() < 1e-4);
    }
    #[test]
    fn cascades_contain_their_frustum_slice() {
        for yaw in [0., 37., 123., 250.] {
            let vi = camera(yaw);
            let th = (40f32).to_radians().tan();
            let aspect = 16. / 9.;
            let cs = fit_cascades(vi, th, aspect, 0.1, sun_direction(), 2048);
            let mut start = 0.1;
            for c in cs {
                for d in [start, c.far] {
                    for (sx, sy) in [(-1., -1.), (1., 1.), (-1., 1.), (1., -1.)] {
                        let p = vi.transform_point3(Vec3::new(sx * th * d, sy * th / aspect * d, -d));
                        let q = c.light_view_proj.project_point3(p);
                        assert!(q.x.abs() <= 1.001 && q.y.abs() <= 1.001 && q.z >= -0.001 && q.z <= 1.001, "yaw {yaw} depth {d}: {q:?}");
                    }
                }
                start = c.far;
            }
        }
    }
    #[test]
    fn caster_above_player_projects_inside_near_cascade_depth_range() {
        let vi = camera(0.);
        let cs = fit_cascades(vi, (40f32).to_radians().tan(), 1.5, 0.1, sun_direction(), 2048);
        // a point 30 m towards the sun from the ground in view is a possible caster and must not be clipped.
        let p = vi.transform_point3(Vec3::new(0., 0., -3.)) + sun_direction() * 30.;
        let q = cs[0].light_view_proj.project_point3(p);
        assert!(q.z > 0. && q.z < 1., "{q:?}");
    }
    #[test]
    fn texel_snapping_is_stable_under_subtexel_camera_motion() {
        let a = fit_cascades(camera(10.), 0.84, 1.5, 0.1, sun_direction(), 2048)[1];
        let vi = camera(10.) * Mat4::from_translation(Vec3::new(0.0004, 0., 0.));
        let b = fit_cascades(vi, 0.84, 1.5, 0.1, sun_direction(), 2048)[1];
        let o = Vec3::new(2., 0., -3.);
        let (pa, pb) = (a.light_view_proj.project_point3(o), b.light_view_proj.project_point3(o));
        // positions in texel units differ by <1 texel (no continuous crawling)
        assert!(((pa.x - pb.x) * 1024.).abs() < 1.5 && ((pa.y - pb.y) * 1024.).abs() < 1.5);
    }
    #[test]
    fn texture_matrix_maps_light_clip_to_unit_uv_with_flipped_v() {
        let c = fit_cascades(camera(0.), 0.84, 1.5, 0.1, sun_direction(), 2048)[0];
        let m = texture_matrix(&c);
        let p = Vec3::new(1., 0.5, -2.);
        let clip = c.light_view_proj.project_point3(p);
        let uv = m.project_point3(p);
        assert!((uv.x - (clip.x * 0.5 + 0.5)).abs() < 1e-5);
        assert!((uv.y - (0.5 - clip.y * 0.5)).abs() < 1e-5);
        assert!((uv.z - clip.z).abs() < 1e-5);
    }
    #[test]
    fn sun_and_sky_radiance_are_finite_and_warm_sun() {
        let s = sun_radiance();
        assert!(s.iter().all(|v| v.is_finite() && *v > 0.));
        assert!(s[0] > s[1] && s[1] > s[2]);
        let k = sky_radiance();
        assert!(k[2] > k[0]);
    }
    #[test]
    fn captured_sky_ambient_is_blue_bounded() {
        let k = sky_capture_radiance();
        assert!(k.iter().all(|v| v.is_finite() && *v > 0.), "{k:?}");
        assert!(k[2] > k[1] && k[1] > k[0], "captured sky is blue: {k:?}");
        assert!(luma(k) < 0.15, "capture luminance {}", luma(k));
        let a = sky_radiance();
        let l = luma(a);
        let sun = luma(sun_radiance()) * sun_direction().y;
        assert!(l > 0.15 && l < 0.40 && l < 0.7 * sun && l > 0.25 * sun, "ambient {l} vs sun horizontal {sun}");
    }
    #[test]
    fn sun_irradiance_on_white_matches_lambert_pi_convention() {
        let s = sun_radiance();
        assert!((s[0] - SUN_BRIGHTNESS / std::f32::consts::PI).abs() < 1e-5);
        assert!((s[1] / s[0] - srgb_to_linear(225)).abs() < 1e-5 && (s[2] / s[0] - srgb_to_linear(161)).abs() < 1e-5);
    }
    #[test]
    fn terrain_colour_multiplier_follows_recovered_parameters() {
        assert!((cheap_contrast(0.3, 0.) - 0.3).abs() < 1e-7 && cheap_contrast(0., -2.).abs() < 1e-7 && (cheap_contrast(1., -2.) - 1.).abs() < 1e-7);
        let m = terrain_color_multiplier();
        for i in 0..3 { assert!(m[i] >= TERRAIN_EDGE_DARK[i] && m[i] <= TERRAIN_HIGHLIGHT[i], "{i}: {}", m[i]); }
        assert!(m[0] > m[1] && m[1] > m[2] && luma(m) > 1.2 && luma(m) < 2.0, "{m:?}");
        // The WGSL retains the recovered values but must not fabricate a global terrain-mask tint on the baseplate.
        let g = include_str!("../shaders/scene.wgsl");
        for needle in ["vec3<f32>(3.0, 2.337033, 1.38)", "vec3<f32>(0.9, 0.9, 0.9)", "TERRAIN_MASK_CONTRAST: f32 = -2.0", "TERRAIN_MASK_MEAN: f32 = 0.2086", "return vec3<f32>(1.0, 1.0, 1.0);"] {
            assert!(g.contains(needle), "scene.wgsl lacks {needle}");
        }
    }
    #[test]
    fn ground_ring_tiles_the_outside_of_the_plate_without_overlap_and_faces_up() {
        let (h, o) = (256., 6000.);
        let mut area = 0.;
        for q in ground_ring(h, o) {
            let (x0, z0, x1, z1) = (q[0].0, q[0].1, q[2].0, q[2].1);
            assert!(x1 > x0 && z1 > z0);
            let (e1, e2) = ((q[1].0 - q[0].0, q[1].1 - q[0].1), (q[2].0 - q[0].0, q[2].1 - q[0].1));
            assert!(e1.1 * e2.0 - e1.0 * e2.1 > 0.);
            assert!(x0 >= h || x1 <= -h || z0 >= h || z1 <= -h, "{q:?} overlaps the plate");
            area += (x1 - x0) * (z1 - z0);
        }
        assert!((area - ((2. * o) * (2. * o) - (2. * h) * (2. * h))).abs() < 1.0, "area {area}");
    }
}
