

// Shared WGSL helpers, prepended to every scene/sky shader by the Rust side (WGSL has no #include).
// Ported from FortniteRuntime/src/shaders/common.glsl + fog.glsl.
// All colour maths in the scene passes is linear; encoding to sRGB happens once, in post.wgsl.

fn decode_srgb(x: f32) -> f32 {
    if (x <= 0.04045) { return x / 12.92; }
    return pow((x + 0.055) / 1.055, 2.4);
}
fn encode_srgb(x: f32) -> f32 {
    if (x <= 0.0031308) { return x * 12.92; }
    return 1.055 * pow(x, 1.0 / 2.4) - 0.055;
}
fn to_linear(v: vec3<f32>) -> vec3<f32> { return vec3<f32>(decode_srgb(v.x), decode_srgb(v.y), decode_srgb(v.z)); }
fn to_srgb(v: vec3<f32>) -> vec3<f32> { return vec3<f32>(encode_srgb(v.x), encode_srgb(v.y), encode_srgb(v.z)); }
fn luma709(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)); }

// Unreal-style ExponentialHeightFog: the compiled TExponentialHeightFogPS<DirectionalLightInscatteringAndVolumetricFog> of the 1.8 global shader cache
// (ported in src/fog.rs `HeightFogPs::eval`, which this mirrors line for line), evaluated per fragment. World units here are metres (GL, Y up);
// the maths runs in Unreal units (cm, Z up = renderer Y).
// TOD LIVE: the values are PER-FRAME uniform data (src/fog.rs HeightFogUniform, the live clock's ExpHeightFogValues phase blend), not baked constants: the
// including shader passes its block (lit_common.wgsl `light.fog`, sky_common.wgsl `sky.fog`). Density / falloff carry the engine /1000 remap.
struct HeightFogU {
    p0: vec4<f32>,         // x FogDensity, y FogHeightFalloff (both /1000), z FogHeight (cm), w FogMaxOpacity
    p1: vec4<f32>,         // x StartDistance (cm), y DirectionalInscatteringExponent, z DirectionalInscatteringStartDistance (cm), w View.VolumetricFogMaxDistance (cm, 0 = no volume)
    dir_color: vec4<f32>,  // rgb View.DirectionalInscatteringColor (x sun luminance), w FogCutoffDistance (cm)
    color: vec4<f32>,      // rgb FogInscatteringColor
}

// Returns (inscattered radiance rgb, transmittance a). final = scene * a + rgb. `fwd` = unit camera forward (world), used by the volumetric exclusion
// (the height fog is excluded from the first VolumetricFogMaxDistance of view depth, the volume covers it: the excluded distance along the ray is
// VolumetricFogMaxDistance / cos(angle to the view axis)).
fn height_fog(f: HeightFogU, world_m: vec3<f32>, camera_m: vec3<f32>, to_sun: vec3<f32>, fwd: vec3<f32>) -> vec4<f32> {
    let FOG_DENSITY = f.p0.x;
    let FOG_FALLOFF = f.p0.y;
    let FOG_HEIGHT_CM = f.p0.z;
    let FOG_MAX_OPACITY = f.p0.w;
    let FOG_START_DISTANCE_CM = f.p1.x;
    let FOG_DIR_EXPONENT = f.p1.y;
    let FOG_DIR_START_CM = f.p1.z;
    let FOG_VOLUME_DISTANCE_CM = f.p1.w;
    let FOG_DIR_COLOR = f.dir_color.xyz;
    let FOG_CUTOFF_DISTANCE_CM = f.dir_color.w;
    let FOG_COLOR = f.color.xyz;
    let c = (world_m - camera_m) * 100.0;
    let cam_z = camera_m.y * 100.0;
    let len = max(length(c), 1e-6);
    let along = dot(c, fwd);
    // (the select keeps 0 * inf at along == 0 from becoming NaN when there is no volume: D3D `max` returns the non-NaN operand, WGSL leaves it undefined)
    let vol_exclude = select(0.0, max(len / along * FOG_VOLUME_DISTANCE_CM, 0.0), FOG_VOLUME_DISTANCE_CM > 0.0);
    let exclude = max(vol_exclude, FOG_START_DISTANCE_CM);
    let inv_len = 1.0 / len;
    var origin_density: f32;
    var ray_len: f32;
    var dz: f32;
    if (exclude > 0.0) {
        let t = min(exclude * inv_len, 1.0);   // ExcludeIntersectionTime (saturated: a point inside the volume has no height fog left)
        let to_excl_z = c.y * t;
        origin_density = exp2(-max(FOG_FALLOFF * (cam_z + to_excl_z - FOG_HEIGHT_CM), -127.0)) * FOG_DENSITY;
        ray_len = len * (1.0 - t);
        dz = c.y - to_excl_z;
    } else {
        origin_density = FOG_DENSITY * exp2(-max(FOG_FALLOFF * (cam_z - FOG_HEIGHT_CM), -127.0));
        ray_len = len;
        dz = c.y;
    }
    // RECOVERED line integral (1 - 2^-x) / x with x = max(FogHeightFalloff * (|dz| > 0.01 ? dz : 0.01), -127)
    let x = max(FOG_FALLOFF * select(0.01, dz, abs(dz) > 0.01), -127.0);
    let line = (1.0 - exp2(-x)) * origin_density / x;
    let optical = ray_len * line;
    let dir = c * inv_len;
    let dir_gain = pow(clamp(dot(dir, to_sun), 0.0, 1.0), FOG_DIR_EXPONENT);
    let dir_factor = 1.0 - min(exp2(-(max(ray_len - FOG_DIR_START_CM, 0.0) * line)), 1.0);
    var dir_in = FOG_DIR_COLOR * (dir_gain * dir_factor);
    var trans = max(min(exp2(-optical), 1.0), 1.0 - FOG_MAX_OPACITY);
    if (FOG_CUTOFF_DISTANCE_CM > 0.0 && FOG_CUTOFF_DISTANCE_CM < len) { dir_in = vec3<f32>(0.0); trans = 1.0; }
    return vec4<f32>(FOG_COLOR * (1.0 - trans) + dir_in, trans);
}
