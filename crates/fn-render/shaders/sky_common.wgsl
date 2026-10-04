// Uniforms shared by the three sky passes (dome, sun, clouds). Prepended after common.wgsl.
struct SkyU {
    vp: mat4x4<f32>,        // wide-clip projection (near 0.1 m, far 4e6 m) * view rotation (camera at the origin)
    cam_world: vec4<f32>,   // xyz camera (metres, GL world), w = View.RealTime
    to_sun: vec4<f32>,      // xyz unit vector towards the sun, w = dome fog enabled
    horizon: vec4<f32>,     // "Sky - Horizon color"
    zenith: vec4<f32>,      // "Sky - Zenith Color"
    params: vec4<f32>,      // x Star Brightness, y Horiz-X, z Horiz-Add, w Horiz-Pow
    tod: vec4<f32>,         // time-of-day weights (morning, day, evening, night)
    sun_c: vec4<f32>,       // billboard centre (metres, camera-relative)
    sun_r: vec4<f32>,       // billboard right axis
    sun_u: vec4<f32>,       // billboard up axis, w = metres per mesh cm
    center_world: vec4<f32>,
    cam_local: vec4<f32>,
}
// Unit camera forward (world) for the height-fog volumetric exclusion: `vp` is perspective_rh * view rotation, so clip.w = -z_view and
// the w row of `vp` is the view axis.
fn sky_forward() -> vec3<f32> { return normalize(vec3<f32>(sky.vp[0][3], sky.vp[1][3], sky.vp[2][3])); }
