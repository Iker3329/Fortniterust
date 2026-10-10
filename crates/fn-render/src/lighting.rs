//! Sun / sky-light values, terrain colour variation and cascaded shadow map fitting.
//! Pure maths (no GPU), with the shadow matrices adapted to wgpu clip space (depth 0..1, texture v pointing down).
//!
//! Every number is READ from the time-of-day manager data (`time_of_day`, i.e. TODM_A / TODM_A_Child_Athena), the UE4.16 sources in
//! `C:\UE4.16` or the 1.8 exe; nothing here is tuned by eye. Unreal units are cm, rotators are (Pitch, Yaw, Roll) degrees; this crate works
//! in metres in the "GL world" (X right, Y up, -Z forward).
//!
//! # What the 1.8 exe does (AFortTimeOfDayManager, ported here)
//! * Directional light ROTATION (`FUN_140631bd0`, called by `UpdateDirectionalLightForTimeOfDay` `FUN_14064d5d0`): `Q = SlerpFullPath_NotNormalized(Quat(SunriseDirectionalLightRotation),
//!   Quat(SunsetDirectionalLightRotation), p)` with `p` = [`time_of_day::sun_arc_progress`]. The light does NOT use the sun-object divergence
//!   (`FUN_140641e30` only positions the sun/moon MODEL with that); `time_of_day::state_at(h).to_sun` is the model direction, [`sun_direction`] the light's.
//! * Directional light COLOUR/INTENSITY (`FUN_14064d5d0` + `FUN_140631570`): the two phases' `FColor` bytes are lerped per channel in sRGB-byte
//!   space (truncating), converted to linear, multiplied by the lerped Brightness, by `DirectionalLightColorMultiplier` (ctor 1.0) and mixed with
//!   `StormLightColor` by `StormStrength` (ctor 0), then split into FColor colour + Intensity = max component. Net radiance = linear colour x Brightness.
//! * SKY light (`FUN_14064f050`): `SetLightColor(ToFColor(SkyLightColor, sRGB))` (components CLAMPED to 0..1 by `FLinearColor::ToFColor`, exe `FUN_1412ca330`),
//!   `SetIntensity(SkyLightColor.A)`. So the day colour (3.575, 4.176, 5.0) is white in the light component; the same raw float4 also feeds the
//!   material parameter `SkyLightColorAndIntensity` (not the sky light). The sky light is a SPECIFIED-CUBEMAP light: the TODM ctor (0x14062c240)
//!   stores `SourceType` (+0x2b8) = 1 = SLS_SpecifiedCubemap and `bLowerHemisphereIsBlack` (+0x2d5) = 0 over the class defaults, and TODM_A serializes
//!   `Cubemap` = T_hdrsky_test03_low; its SH irradiance (full sphere) is in `sky_light.rs`.
use crate::time_of_day as tod;
use glam::{Mat4, Vec3, Vec4};

/// DirectionalLightComponent.DynamicShadowDistanceMovableLight = 4500 cm and CascadeDistributionExponent = 2: both SERIALIZED in TODM_A
/// (`DayNightDirectionalLightComponent`, verified with fn-inspect). The light is Movable, so UE uses `DynamicShadowDistanceMovableLight` as the CSM radius
/// (`FDirectionalLightSceneProxy` ctor) and, because a movable light has no static shadowing, `IsPrecomputedLightingValid()` is true and the serialized exponent applies.
pub const SHADOW_DISTANCE_M: f32 = 45.0;
pub const CASCADE_EXPONENT: f32 = 2.0;
/// DynamicShadowCascades is not serialized: the 1.8 exe class constructor stores 3 (reports/exe-defaults.json, store RVA 0x20a482d). The cvar
/// `r.Shadow.CSM.MaxCascades` default 10 does not limit it.
pub const CASCADE_COUNT: usize = 3;
/// CascadeTransitionFraction (exe ctor 0.1, RVA 0x20a4841; `r.Shadow.CSM.TransitionScale` default 1): each non-last cascade is extended by this fraction of its length.
pub const CASCADE_TRANSITION_FRACTION: f32 = 0.1;
/// ShadowDistanceFadeoutFraction (exe ctor 0.1, RVA 0x20a484b): shadows fade out linearly over the last 10 % of the distance (shaders/lit_common.wgsl `sun_shadow`).
pub const SHADOW_DISTANCE_FADEOUT_FRACTION: f32 = 0.1;
/// ShadowSharpen: SERIALIZED 0.05 in TODM_A (class default 0). UE4.16 uploads `ShadowSharpen * 7 + 1` and does `saturate((Shadow - 0.5) * k + 0.5)`
/// (Engine/Shaders/ShadowProjectionPixelShader.usf:175, ShadowRendering.h:1495).
pub const SHADOW_SHARPEN: f32 = 0.05;
pub fn shadow_sharpen_factor() -> f32 { SHADOW_SHARPEN * 7. + 1. }
/// UDirectionalLightComponent::LightSourceAngle = 1.0 deg: RECOVERED from the 1.8 exe class ctor (reports exe-defaults.json, store at
/// RVA 0x20a4823); TODM_A does not serialize it. UE4.16 does not use it for CSM filtering, only for the ray-traced distance-field cascade
/// (`bUseRayTracedDistanceFieldShadows` is serialized true in TODM_A): see `distance_field.rs`.
pub const LIGHT_SOURCE_ANGLE_DEG: f32 = 1.0;
/// The sun's LIGHT FUNCTION. `DayNightDirectionalLightComponent.LightFunctionMaterial` of TODM_A_Child_Athena is
/// `/Game/Athena/Prototype/Terrain/M_Athena_Fortress_Skybox_LF_Spinning_2` (a MaterialInstanceConstant with its own cooked shader map). Its SM5 `FLightFunctionPS`
/// (stream 33143, `fn-dxbc disasm "<pkg>#33143"`) never samples a texture other than the depth buffer: the emissive colour is the material uniform vector slot 2 =
/// `FoldedMath(Constant 0.6 + SelectionColor.rgb)` (SelectionColor is the editor selection colour, black at runtime) = (0.6, 0.6, 0.6), so there is NO spatial cloud
/// pattern in this instance; the result `GreyScale = dot(max(emissive, 0), 0.3333)` = 0.59994 multiplies the sun (`OpaqueShadowTerm *= LightAttenuation.z`).
/// The shader fades it to `DisabledBrightness` over the last 20 % of `LightFunctionFadeDistance`: `DisabledBrightness` = 1.0 (child override, parent 0.3) and
/// `LightFunctionFadeDistance` = 25000 cm (TODM_A), both read with `fn-inspect props`. (8-bit quantisation of the attenuation buffer is not reproduced.)
/// LIGHT FUNCTION (2026-10-07): this is the LOW material-quality program only; the Epic client runs the High one (r.MaterialQualityLevel 1), ported in
/// `light_function.rs` and used by the renderer. Kept for reference / tests.
pub const LIGHT_FUNCTION_GREY: f32 = 0.6 * 0.3333 * 3.;
pub const LIGHT_FUNCTION_DISABLED_BRIGHTNESS: f32 = 1.0;
pub const LIGHT_FUNCTION_FADE_DISTANCE_M: f32 = 250.;
/// `SHADOW_BORDER` (ShadowRendering.h): every CSM is rendered into `MaxCSMShadowResolution - 2 * SHADOW_BORDER` texels with a cleared border
/// (`AddViewDependentWholeSceneShadowsForView`, ShadowSetup.cpp:2649; `NeedsUnatlasedCSMDepthsWorkaround` is false on PC).
pub const SHADOW_BORDER: u32 = 4;
/// `r.Shadow.CSMDepthBias` default (ShadowRendering.cpp:17), not overridden by the Epic scalability group (BaseScalability.ini ShadowQuality@3).
pub const CSM_DEPTH_BIAS_CVAR: f32 = 20.0;
/// `ULightComponent::ShadowBias` = `FLightSceneProxy::GetUserShadowBias()`: UE ctor default 0.5 (LightComponent.cpp:337); the 1.8 exe ctor stores 0.5 too (RVA 0x20a4db3).
pub const LIGHT_SHADOW_BIAS: f32 = 0.5;
/// `DepthRangeClamp` (ShadowSetup.cpp, SetupWholeSceneProjection): a directional cascade's depth range is at least +-5000 cm around the sphere centre.
pub const DEPTH_RANGE_CLAMP_M: f32 = 50.;
/// `MaxDownsampleFactor` of the cascade origin snap (ShadowSetup.cpp): the centre moves in steps of 4 shadow texels.
pub const SNAP_TEXELS: f32 = 4.;

fn srgb_to_linear(v: u8) -> f32 {
    let c = v as f32 / 255.;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
/// `FLinearColor::ToFColor(true)` for one channel: clamp to 0..1, sRGB encode, `floor(x * 255.999)` (exe `FUN_1412ca330`, UE4.16 Color.cpp).
fn linear_to_srgb_byte(c: f32) -> u8 {
    let c = c.clamp(0., 1.);
    let e = if c <= 0.003_130_8 { c * 12.92 } else { c.powf(1. / 2.4) * 1.055 - 0.055 };
    (e * 255.999).floor() as u8
}
/// Unreal world direction (X, Y, Z up) to the renderer world (Y up, right handed): `(x, y, z) -> (x, z, y)`.
/// SUN-AXIS FIX (2026-10-05): this was `(y, z, -x)`, a 90 deg yaw away from every placed thing. The CANONICAL UE -> renderer map of the port is the
/// UEViewer glTF swap `(x, z, y)` (* 0.01 for positions): props (`fn_propdata::ue_to_game_position`), terrain backdrop (`backdrop::to_game`), foliage, storm,
/// ambient audio, FX / matshader views (`fn_fx::view::to_ue`), particles (`fn_particles::sim::ue_to_game`), the character shaders and the DF space all use it.
/// With it UE +X (= NORTH in 1.8: T_Compass has N at pan yaw 0, and Loot Lake / Dusty Depot / Salty Springs / Flush Factory / Moisty Mire POI sublevel
/// offsets of Athena_Terrain fit MiniMapAthena.png with +X up, +Y right) is renderer +X and UE +Y (EAST) is renderer +Z. The sun disc, the sky dome
/// (`sky::ue_to_gl` delegates here), the CSM / distance-field shadows and the fog now share that frame with the props. Inverse: `sky_light::world_to_ue_dir`.
pub fn ue_to_world(v: Vec3) -> Vec3 { Vec3::new(v.x, v.z, v.y) }
/// Direction the light travels, from an Unreal rotator (FRotator::Vector()).
pub fn light_travel_direction(pitch_deg: f32, yaw_deg: f32) -> Vec3 {
    let (p, y) = (pitch_deg.to_radians(), yaw_deg.to_radians());
    ue_to_world(Vec3::new(p.cos() * y.cos(), p.cos() * y.sin(), p.sin())).normalize()
}
/// Unreal-space direction the DIRECTIONAL LIGHT travels at `hour` (`FUN_140631bd0`, called by `UpdateDirectionalLightForTimeOfDay` `FUN_14064d5d0`):
/// `GetNormalized(FQuat::SlerpFullPath_NotNormalized(Quat(SunriseDirectionalLightRotation), Quat(SunsetDirectionalLightRotation), p))` with `p` =
/// `FUN_140631920` = [`tod::sun_arc_progress`], then `FRotator(Q)` -> `SetWorldRotation`; the light travels along the forward axis.
/// SUN SLERP (2026-10-07): the full-path slerp (no hemisphere flip, [`tod::slerp_full_path_not_normalized`]); was glam's shortest-path `slerp`.
pub fn light_travel_direction_ue(hour: f32) -> Vec3 {
    let p = tod::sun_arc_progress(hour);
    tod::quat_forward(tod::quat_get_normalized(tod::slerp_full_path_not_normalized(tod::ue_quat(tod::SUNRISE_ROTATION_PYR_DEG), tod::ue_quat(tod::SUNSET_ROTATION_PYR_DEG), p)))
}
/// Unit vector from the scene towards the sun LIGHT at `hour` (renderer world). Shared by diffuse lighting, shadow maps and the fog's directional inscattering.
pub fn sun_direction_at(hour: f32) -> Vec3 { -ue_to_world(light_travel_direction_ue(hour)) }
/// Unit vector from the scene towards the sun light at the current time-of-day clock.
pub fn sun_direction() -> Vec3 { sun_direction_at(tod::hour()) }

/// Linear sun radiance of a blended time-of-day state: lerped sRGB bytes -> linear x Brightness / pi (UE Lambert divides albedo by pi, so Brightness 3
/// means 3/pi for white albedo). The exe's FColor/Intensity renormalisation (max component) changes the value by < 0.3 % (byte quantisation) and is not reproduced.
pub fn sun_radiance_of(st: &tod::TodState) -> [f32; 3] {
    let (a, b) = (&tod::PHASES[st.from].sun_color_srgb, &tod::PHASES[st.to].sun_color_srgb);
    // exe FUN_14064d5d0: (char)(long)((float)(int)(B - A) * t + A), i.e. truncation of the lerp of the FColor bytes
    let byte = |i: usize| (((b[i] as i32 - a[i] as i32) as f32 * st.blend + a[i] as f32) as i64).clamp(0, 255) as u8;
    let k = st.sun_brightness / std::f32::consts::PI;
    std::array::from_fn(|i| srgb_to_linear(byte(i)) * k)
}
/// Linear sun radiance at the current clock.
pub fn sun_radiance() -> [f32; 3] { sun_radiance_of(&tod::current()) }

/// Sky-light colour x intensity in linear light for a blended state: `ToFColor(SkyLightColor)` (clamped to 0..1, sRGB-quantised) back to linear, times the
/// intensity (= SkyLightColor alpha). See the module docs: the Day colour (3.575, 4.176, 5.0) clamps to white.
pub fn sky_light_tint_of(st: &tod::TodState) -> [f32; 3] {
    std::array::from_fn(|i| srgb_to_linear(linear_to_srgb_byte(st.sky_light_color[i])) * st.sky_light_alpha)
}
/// The sky light's diffuse term as the lit shaders get it: `View.SkyIrradianceEnvironmentMap` (the SH of the SPECIFIED cubemap
/// `T_hdrsky_test03_low`, full sphere: `bLowerHemisphereIsBlack` = 0, see `sky_light.rs` for the exe/asset evidence) with `View.SkyLightColor`
/// = [`sky_light_tint_of`] folded in. `GetSkySHDiffuse(N)` of this (lit_common.wgsl `sky_lookup`) is the lit shaders' sky term.
pub fn sky_sh_of(st: &tod::TodState) -> [[f32; 4]; 7] { crate::sky_light::pack_sky_irradiance(&crate::sky_light::IRRADIANCE_SH, sky_light_tint_of(st)) }
/// [`sky_sh_of`] at the current clock.
pub fn sky_sh() -> [[f32; 4]; 7] { sky_sh_of(&tod::state_at(tod::hour())) }
/// One colour for consumers that still model the sky as `sky_radiance * (0.5 + 0.5 n.y)` (fx, pickups, gun / storm DXBC view buffers, volumetric
/// fog, backdrop fresnel): `GetSkySHDiffuse(+Z) * SkyLightColor`, i.e. the irradiance / pi an up-facing surface gets.
pub fn sky_radiance_of(st: &tod::TodState) -> [f32; 3] { crate::sky_light::eval_sky_sh(&sky_sh_of(st), Vec3::Z) }
/// [`sky_radiance_of`] at the current clock.
pub fn sky_radiance() -> [f32; 3] { sky_radiance_of(&tod::state_at(tod::hour())) }

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


// ---- Cascaded shadow map fitting (UE4.16 DirectionalLightComponent.cpp + ShadowSetup.cpp + ShadowRendering.cpp) ----
#[derive(Clone, Copy, Debug)]
pub struct Cascade {
    /// world -> light clip space: orthographic, XY +-1 over the `resolution`-texel viewport, Z = the NORMALISED shadow depth `d` of UE (0..1, see
    /// [`fit_cascades`]), WITHOUT [`Cascade::depth_bias`] (the depth pass adds it, like `ShadowDepthVertexShader.usf` `SetShadowDepthOutputs`)
    pub light_view_proj: Mat4,
    /// view-space depth (metres) at which this cascade ends (`SplitFar` before the transition extension)
    pub far: f32,
    /// `SplitFarFadeRegion` (metres): the cascade is extended past `far` by this much and blended into the next one (0 for the last cascade)
    pub fade_extension: f32,
    /// world metres covered by one shadow-map texel
    pub texel: f32,
    /// bounding-sphere radius (`ShadowBounds.W`, metres)
    pub radius: f32,
    /// `ShaderDepthBias` == `ComputeTransitionSize()` of a whole-scene directional shadow, in normalised depth units
    pub depth_bias: f32,
    /// `ResolutionX`: texels of the viewport inside the `resolution + 2 * SHADOW_BORDER` map
    pub resolution: u32,
    /// PERF (2026-10-07, caster culling): `ShadowBounds.Center` of a CSM cascade = the UNSNAPPED split sphere (`GetShadowSplitBounds`, ShadowSetup.cpp:686-692), metres
    pub bounds_center: Vec3,
    /// PERF: unit vector towards the sun (`-LightProxy->GetDirection()`)
    pub to_sun: Vec3,
    /// PERF: `CascadeSettings.ShadowBoundsAccurate` (`ComputeShadowCullingVolume`, DirectionalLightComponent.cpp:399, built from the cascade's camera
    /// sub-frustum SplitNear .. SplitFar + fade, :541-608) in UE axes (renderer (x, y, z) -> (x, z, y)), metres: (normal, W), outside when `n.p - W > r`
    pub cull_planes: [Vec4; 18],
    pub cull_count: u8,
}
impl Cascade {
    /// PERF (2026-10-07): UE4.16 `FSceneRenderer::GatherShadowPrimitivesPrimitive` caster test of a view-dependent whole-scene shadow (ShadowSetup.cpp:2416-2438):
    /// the caster's bounding sphere must meet the cylinder of radius `ShadowBounds.W` around the light axis through `ShadowBounds.Center` (not beyond the
    /// spherical cap on the far side of the light), and `ShadowBoundsAccurate` (the camera sub-frustum extruded towards the light). The sphere test is
    /// conservative against UE's `IntersectBox` of the AABB: a culled caster has every vertex outside, so it cannot shadow any receiver in the slice.
    pub fn caster_in_bounds(&self, c: Vec3, r: f32) -> bool {
        let light_direction = -self.to_sun; // GetDirection(): the light's travel direction
        let p2c = self.bounds_center - c;
        let along = p2c.dot(light_direction);
        let axis_sq = (p2c - light_direction * along).length_squared();
        let combined_sq = (self.radius + r) * (self.radius + r);
        if !(axis_sq < combined_sq && !(along < 0. && p2c.length_squared() > combined_sq)) { return false; }
        let cu = Vec3::new(c.x, c.z, c.y);
        self.cull_planes[..self.cull_count as usize].iter().all(|p| p.truncate().dot(cu) - p.w <= r)
    }
}
/// `ComputeAccumulatedScale`: sizes of the cascades grow by `exponent` each (1, e, e^2, ...); fraction of the total covered by the first `idx` cascades.
fn accumulated_scale(exponent: f32, idx: usize, count: usize) -> f32 {
    if idx == 0 { return 0.; }
    let (mut cur, mut total, mut ret) = (1f32, 0f32, 0f32);
    for i in 0..count {
        if i < idx { ret += cur; }
        total += cur;
        cur *= exponent;
    }
    ret / total
}
/// Split ends for `count` cascades over [near, distance]: UE's `GetSplitDistance` (`near + ComputeAccumulatedScale(exponent, i, count) * (distance - near)`).
pub fn cascade_splits(near: f32, distance: f32, count: usize, exponent: f32) -> Vec<f32> {
    (1..=count).map(|i| near + accumulated_scale(exponent, i, count) * (distance - near)).collect()
}
/// Bounding sphere (centre distance along the view axis, radius) of the camera sub-frustum [split_near, split_far]: `GetShadowSplitBoundsDepthRange`
/// (the radius is clamped to >= 1 cm like UE, NOT rounded: it only depends on the FOV / aspect / split, so it is already rotation invariant).
pub(crate) fn split_sphere(tan_half_h: f32, aspect: f32, split_near: f32, split_far: f32) -> (f32, f32) {
    let tan_y = tan_half_h / aspect;
    let (far_x, far_y, near_x, near_y) = (tan_half_h * split_far, tan_y * split_far, tan_half_h * split_near, tan_y * split_near);
    let len = split_far - split_near;
    let optimal = ((near_x * near_x + near_y * near_y) - (far_x * far_x + far_y * far_y)) / (2. * len) + len * 0.5;
    let centre_z = (split_far - optimal).clamp(split_near, split_far);
    let r2 = [(split_near, near_x, near_y), (split_far, far_x, far_y)]
        .into_iter().map(|(d, hx, hy)| hx * hx + hy * hy + (d - centre_z) * (d - centre_z)).fold(0f32, f32::max);
    (centre_z, r2.sqrt().max(0.01))
}
/// `FProjectedShadowInfo::UpdateShaderDepthBias` / `ComputeTransitionSize` for a whole-scene directional shadow (ShadowRendering.cpp:1173 / 1235), in the
/// normalised depth units of the shadow map: `r.Shadow.CSMDepthBias / (MaxSubjectZ - MinSubjectZ) * (ShadowBounds.W / ResolutionX) * UserShadowBias`.
/// Lengths in cm (the cvar is in cm); `radius_m` = `ShadowBounds.W`, `depth_range_m` = `MaxSubjectZ - MinSubjectZ`.
pub fn csm_depth_bias(radius_m: f32, depth_range_m: f32, resolution: u32) -> f32 {
    CSM_DEPTH_BIAS_CVAR / (depth_range_m * 100.) * (radius_m * 100. / resolution as f32) * LIGHT_SHADOW_BIAS
}
/// Fit the cascades exactly like UE4.16 (`GetShadowSplitBounds` + `SetupWholeSceneProjection`):
/// * splits from [`cascade_splits`], every non-last cascade extended by `CascadeTransitionFraction * TransitionScale(1)` of its length;
/// * the sphere of the extended slice (UE's optimal-centre formula) is the orthographic footprint (`Scales = 1 / ShadowBounds.W`), unrounded;
/// * depth range: `MaxSubjectZ = max(W, 5000 cm)`, `MinSubjectZ = min(-W, -5000 cm)` around the sphere centre; the stored depth is
///   `d = (z - MinSubjectZ) / (W - MinSubjectZ)` (z along the light direction from the centre), i.e. `InvMaxSubjectDepth`-normalised, so the far plane is
///   the sphere's far side;
/// * the sphere centre is snapped in light space (grid anchored at the world origin) to a multiple of `MaxDownsampleFactor` (4) shadow texels of the
///   `MaxCSMShadowResolution - 2 * SHADOW_BORDER` viewport (`fmod`, i.e. truncating).
/// `view_inverse` maps eye space to world; `tan_half_h` is tan(horizontal fov/2); `aspect` is w/h; `size` is the full shadow map edge (`MaxCSMShadowResolution`).
pub fn fit_cascades(view_inverse: Mat4, tan_half_h: f32, aspect: f32, near: f32, to_sun: Vec3, size: u32) -> [Cascade; CASCADE_COUNT] {
    fit_cascades_ex(view_inverse, tan_half_h, aspect, near, to_sun, size, false, &CsmSetup::EPIC)
}
/// The scalability-dependent inputs of the CSM split (`quality::ShadowScalability::csm`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CsmSetup {
    /// `GetNumShadowMappedCascades(View.MaxShadowCascades)` = min(DynamicShadowCascades, r.Shadow.CSM.MaxCascades) (DirectionalLightComponent.cpp:357).
    pub num_cascades: usize,
    /// `GetCSMMaxDistance` (metres) = DynamicShadowDistanceMovableLight * clamp(r.Shadow.DistanceScale, 0, 2) (DirectionalLightComponent.cpp:364).
    pub distance_m: f32,
    /// `CascadeTransitionFraction * clamp(r.Shadow.CSM.TransitionScale, 0, 2)` (DirectionalLightComponent.cpp:635).
    pub transition_fraction: f32,
}
impl CsmSetup {
    /// `[ShadowQuality@3]` (BaseScalability.ini:118): MaxCascades 10, DistanceScale 1, TransitionScale 1.
    pub const EPIC: CsmSetup = CsmSetup { num_cascades: CASCADE_COUNT, distance_m: SHADOW_DISTANCE_M, transition_fraction: CASCADE_TRANSITION_FRACTION };
}
/// [`fit_cascades`] with the DISTANCE FIELD HOOK: when the light also creates the ray-traced distance-field cascade
/// (`ShouldCreateRayTracedCascade`, DirectionalLightComponent.cpp:340), `NumTotalCascades` = CSM + 1, so the LAST shadow-mapped cascade is no longer
/// the last one and also gets the `SplitFar += FadeExtension` of `GetShadowSplitBounds` (DirectionalLightComponent.cpp:640) and a fade plane.
/// `csm` = the scalability inputs: only the first `csm.num_cascades` entries are cascades (the rest repeat the last one and are never drawn or sampled).
#[allow(clippy::too_many_arguments)]
pub fn fit_cascades_ex(view_inverse: Mat4, tan_half_h: f32, aspect: f32, near: f32, to_sun: Vec3, size: u32, ray_traced_cascade: bool, csm: &CsmSetup) -> [Cascade; CASCADE_COUNT] {
    let num = csm.num_cascades.clamp(1, CASCADE_COUNT);
    let splits = cascade_splits(near, csm.distance_m.max(near + 0.01), num, CASCADE_EXPONENT);
    let resolution = size.saturating_sub(2 * SHADOW_BORDER).max(1);
    let up = if to_sun.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    // light-space axes of look_at_rh (right = f x up, up' = right x f): the snapping grid and the shadow map axes
    let fwd = -to_sun;
    let right = fwd.cross(up).normalize();
    let light_up = right.cross(fwd);
    let mut start = near;
    std::array::from_fn(|c| {
        let c = c.min(num - 1);
        if c + 1 == num { start = if num > 1 { splits[num - 2] } else { near }; }
        let end = splits[c];
        let fade_extension = if c + 1 < num || ray_traced_cascade { (end - start) * csm.transition_fraction } else { 0. };
        let (centre_z, radius) = split_sphere(tan_half_h, aspect, start, end + fade_extension);
        let split_near = start;
        start = end;
        let center = view_inverse.transform_point3(Vec3::new(0., 0., -centre_z));
        let texel = 2. * radius / resolution as f32;
        // origin snap (ShadowSetup.cpp SetupWholeSceneProjection): light-space xy / W, fmod by 2 * MaxDownsampleFactor / ResolutionX
        let step = 2. * SNAP_TEXELS / resolution as f32;
        let (sx, sy) = ((center.dot(right) / radius) % step, (center.dot(light_up) / radius) % step);
        let snapped = center - right * (sx * radius) - light_up * (sy * radius);
        let e = radius.max(DEPTH_RANGE_CLAMP_M); // MaxSubjectZ = e, MinSubjectZ = -e
        let view = Mat4::look_at_rh(snapped + to_sun * e, snapped, up);
        let proj = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0., radius + e);
        // PERF: ShadowBoundsAccurate of this cascade's camera sub-frustum (GetShadowSplitBoundsDepthRange corner order: near TR, BR, TL, BL, then far), in UE axes
        let ue = |v: Vec3| Vec3::new(v.x, v.z, v.y);
        let tan_y = tan_half_h / aspect;
        let corner = |d: f32, sx: f32, sy: f32| ue(view_inverse.transform_point3(Vec3::new(sx * d * tan_half_h, sy * d * tan_y, -d)));
        let (sn, sf) = (split_near, end + fade_extension);
        let verts = [corner(sn, 1., 1.), corner(sn, 1., -1.), corner(sn, -1., 1.), corner(sn, -1., -1.), corner(sf, 1., 1.), corner(sf, 1., -1.), corner(sf, -1., 1.), corner(sf, -1., -1.)];
        let hull = crate::distance_field::shadow_culling_volume(&verts, ue(to_sun));
        let mut cull_planes = [Vec4::ZERO; 18];
        for (d, s) in cull_planes.iter_mut().zip(&hull) { *d = *s; }
        Cascade { light_view_proj: proj * view, far: end, fade_extension, texel, radius, depth_bias: csm_depth_bias(radius, 2. * e, resolution), resolution,
            bounds_center: center, to_sun, cull_planes, cull_count: hull.len().min(18) as u8 }
    })
}
/// world -> shadow-map texture coordinates of the full `resolution + 2 * SHADOW_BORDER` map (xy in 0..1 with v pointing down as in wgpu, z = the
/// normalised depth `d`). The viewport occupies the map centre (`GetWorldToShadowMatrix`: `0.5 * ResolutionX / BufferResolution`).
pub fn texture_matrix(c: &Cascade) -> Mat4 {
    let k = 0.5 * c.resolution as f32 / (c.resolution + 2 * SHADOW_BORDER) as f32;
    Mat4::from_cols(Vec4::new(k, 0., 0., 0.), Vec4::new(0., -k, 0., 0.), Vec4::new(0., 0., 1., 0.), Vec4::new(0.5, 0.5, 0., 1.)) * c.light_view_proj
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
    fn sun_direction_is_unit_and_low_at_the_default_sunset() {
        let d = sun_direction_at(tod::SUNSET_HOUR);
        assert!((d.length() - 1.).abs() < 1e-5);
        let elev = d.y.asin().to_degrees();
        assert!(elev > 3. && elev < 30., "sunset light elevation {elev}");
    }
    /// SUN SLERP (2026-10-07): an f64 reference of the decompiled chain, written from the exe functions independently of the production helpers:
    /// FUN_140631bd0 = FRotator::Quaternion (UE4.16 UnrealMath.cpp:374 scalar form) of +0x3e8 / +0x3f4 -> FQuat__SlerpFullPath_NotNormalized (0x1412f1c60: dot,
    /// clamp to [-1, 1] by DAT_143feb1ec / DAT_143fe36fc, acosf, `|angle| >= DAT_143ff9010 (1e-4)` else quat1, sin weights) -> rsqrt normalisation when the square
    /// sum >= DAT_143ff9008 (1e-8) -> FRotator(Q) -> SetWorldRotation; the light's direction = the rotation's X axis. Compared at every quarter hour.
    #[test]
    fn light_direction_matches_the_decompiled_full_path_slerp() {
        fn q64(r: [f32; 3]) -> [f64; 4] {
            let h = |d: f32| (d as f64).to_radians() * 0.5;
            let ((sp, cp), (sy, cy), (sr, cr)) = (h(r[0]).sin_cos(), h(r[1]).sin_cos(), h(r[2]).sin_cos());
            [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]
        }
        fn slerp_full(a: [f64; 4], b: [f64; 4], t: f64) -> [f64; 4] {
            let c = (a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]).clamp(-1., 1.);
            let ang = c.acos();
            if ang.abs() < 1e-4 { return a; }
            let (s0, s1) = (((1. - t) * ang).sin() / ang.sin(), (t * ang).sin() / ang.sin());
            std::array::from_fn(|i| a[i] * s0 + b[i] * s1)
        }
        fn fwd(q: [f64; 4]) -> [f64; 3] {
            let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            let [x, y, z, w] = q.map(|v| v / n);
            [1. - 2. * (y * y + z * z), 2. * (x * y + w * z), 2. * (x * z - w * y)]
        }
        let (a, b) = (q64(tod::SUNRISE_ROTATION_PYR_DEG), q64(tod::SUNSET_ROTATION_PYR_DEG));
        let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
        let mut max_shortest = 0f64;
        for i in 0..96 {
            let h = i as f32 * 0.25;
            let p = tod::sun_arc_progress(h) as f64;
            let want = fwd(slerp_full(a, b, p));
            let got = light_travel_direction_ue(h);
            let err = ((got.x as f64 - want[0]).powi(2) + (got.y as f64 - want[1]).powi(2) + (got.z as f64 - want[2]).powi(2)).sqrt();
            assert!(err < 2e-5, "h {h}: port {got:?} vs decompile {want:?}");
            // what the old shortest-path slerp (hemisphere flip when the dot is negative) would have produced
            let bs = if dot < 0. { b.map(|v| -v) } else { b };
            let old = fwd(slerp_full(a, bs, p));
            max_shortest = max_shortest.max(((old[0] - want[0]).powi(2) + (old[1] - want[1]).powi(2) + (old[2] - want[2]).powi(2)).sqrt());
        }
        eprintln!("SUN SLERP: Quat(Sunrise) . Quat(Sunset) = {dot:.6}; max |shortest-path - full-path| direction difference = {max_shortest:.6}");
    }
    /// LIVE DUMP (2026-10-07, MATCH track): the real 1.8 client in Athena_Terrain (C:\FortniteRef\lighting_ref\match\lightdump_00{0,1}_user_tod*.json,
    /// dump\lightdump_005.json). The DayNightDirectionalLightComponent's RelativeRotation forward is the FUN_140631bd0 full-path slerp (no divergence);
    /// its LightShaftOverrideDirection is the sun MODEL direction (FUN_140641e30, divergence power 5) = `tod::light_travel_direction_ue` (light_shafts.rs).
    /// 17.5 h discriminates the two (d = 0.02): the light is 0.6 deg off the model there.
    #[test]
    fn sun_light_and_shaft_directions_match_the_live_dump() {
        let rot = |p: f32, y: f32| Vec3::new(p.to_radians().cos() * y.to_radians().cos(), p.to_radians().cos() * y.to_radians().sin(), p.to_radians().sin());
        // (hour, DL RelativeRotation pitch/yaw, LightShaftOverrideDirection)
        for (h, pitch, yaw, shaft) in [(12.0, -69.951675, -85.96449, Vec3::new(0.024125433, -0.34196213, -0.939404)),
                                       (12.686272, -69.826065, -98.857704, Vec3::new(-0.05310353, -0.34075797, -0.93865013)),
                                       (17.5, -48.52932, -146.2161, Vec3::new(-0.5575304, -0.37448463, -0.74089205))] {
            let (light, model) = (light_travel_direction_ue(h), tod::light_travel_direction_ue(h));
            assert!((light - rot(pitch, yaw)).length() < 2e-4, "h {h}: light {light:?} vs dump {:?}", rot(pitch, yaw));
            assert!((model - shaft).length() < 2e-4, "h {h}: model {model:?} vs dump shaft {shaft:?}");
        }
        assert!((light_travel_direction_ue(17.5) - tod::light_travel_direction_ue(17.5)).length() > 5e-3);
    }
    #[test]
    fn light_follows_the_slerp_of_the_serialized_sunrise_and_sunset_rotations() {
        // p = 0 at 3 h = SunriseDirectionalLightRotation (337, -30), p = 1 at 22 h = SunsetDirectionalLightRotation (200, 28); pitch 200 = light travels 20 deg down.
        let (a, b) = (light_travel_direction_ue(3.), light_travel_direction_ue(22.));
        let rot = |p: f32, y: f32| Vec3::new(p.to_radians().cos() * y.to_radians().cos(), p.to_radians().cos() * y.to_radians().sin(), p.to_radians().sin());
        assert!((a - rot(337., -30.)).length() < 1e-4, "{a:?}");
        assert!((b - rot(200., 28.)).length() < 1e-4, "{b:?}");
        // the light has no sun-object divergence: at p = 1 it is still 20 deg above the horizon while the sun MODEL has set
        let elev22 = sun_direction_at(22.).y.asin().to_degrees();
        assert!((elev22 - 20.).abs() < 0.01, "{elev22}");
        assert!(tod::state_at(22.).to_sun.y.asin().to_degrees() < elev22);
        // noon-ish: high sun
        assert!(sun_direction_at(12.5).y.asin().to_degrees() > 60.);
    }
    #[test]
    fn ue_axes_map_to_renderer_world() {
        // SUN-AXIS FIX: the canonical (x, z, y) swap, identical to the props / backdrop / FX maps (UE north +X -> +X, east +Y -> +Z, up -> +Y)
        assert_eq!(ue_to_world(Vec3::X), Vec3::X);
        assert_eq!(ue_to_world(Vec3::Y), Vec3::Z);
        assert_eq!(ue_to_world(Vec3::Z), Vec3::Y);
        let ue = Vec3::new(123., -45., 67.);
        let p = fn_propdata::ue_to_game_position(ue.to_array());
        assert!((Vec3::from(p) - ue_to_world(ue) * 0.01).length() < 1e-6, "sun frame == prop frame");
        assert!((crate::sky_light::world_to_ue_dir(ue_to_world(ue)) - ue).length() < 1e-4, "sky-light lookup is the inverse");
        assert!((crate::distance_field::dir_to_df(ue_to_world(ue)) - ue).length() < 1e-4, "DF space is UE space");
    }
    #[test]
    fn splits_follow_ue_accumulated_scale_and_end_at_shadow_distance() {
        // exponent 2, 3 cascades: sizes 1:2:4 of the span -> ends at 1/7, 3/7, 7/7 (ComputeAccumulatedScale)
        let s = cascade_splits(0.1, 45., 3, 2.);
        assert_eq!(s.len(), 3);
        assert!((s[2] - 45.).abs() < 1e-4);
        for (i, f) in [1. / 7., 3. / 7., 1.].into_iter().enumerate() { assert!((s[i] - (0.1 + 44.9 * f)).abs() < 1e-3, "{i}: {}", s[i]); }
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
    fn caster_bounds_keep_the_slice_and_its_upwind_casters_and_drop_the_rest() {
        // PERF (2026-10-07): Cascade::caster_in_bounds (UE4.16 ShadowSetup.cpp:2416-2438 + ShadowBoundsAccurate)
        for yaw in [0., 37., 123., 250.] {
            let vi = camera(yaw);
            let th = (40f32).to_radians().tan();
            let aspect = 16. / 9.;
            let sun = sun_direction();
            let cs = fit_cascades(vi, th, aspect, 0.1, sun, 2048);
            let mut start = 0.1;
            for c in cs {
                for d in [start, 0.5 * (start + c.far), c.far] {
                    for (sx, sy) in [(-1., -1.), (1., 1.), (-1., 1.), (1., -1.), (0., 0.)] {
                        let p = vi.transform_point3(Vec3::new(0.999 * sx * th * d, 0.999 * sy * th / aspect * d, -d));
                        assert!(c.caster_in_bounds(p, 0.01), "yaw {yaw} depth {d}: slice point dropped");
                        // anything between the slice and the sun can shadow it
                        assert!(c.caster_in_bounds(p + sun * 0.5 * c.radius, 0.01), "yaw {yaw} depth {d}: upwind caster dropped");
                    }
                }
                // behind the camera, down-wind and far off to the side: no receiver of the slice can see it
                let behind = vi.transform_point3(Vec3::new(0., 0., 3. * c.far + 50.)) - sun * 2. * c.radius;
                assert!(!c.caster_in_bounds(behind, 0.5), "yaw {yaw}: far caster kept");
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
    /// Texel coordinate (in the `resolution`-texel viewport, origin at the viewport corner) of world point `p` in a cascade.
    fn viewport_texel(c: &Cascade, p: Vec3) -> (f32, f32) {
        let q = c.light_view_proj.project_point3(p);
        ((q.x * 0.5 + 0.5) * c.resolution as f32, (q.y * 0.5 + 0.5) * c.resolution as f32)
    }
    #[test]
    fn shadow_grid_is_world_anchored_so_it_does_not_crawl_with_camera_motion() {
        // UE4.16 SetupWholeSceneProjection snaps the sphere centre to multiples of 4 viewport texels in light space (grid anchored at the world origin):
        // a world point keeps the same FRACTIONAL texel position whatever the camera does (the camera moving by less than 4 texels changes nothing;
        // by more, the whole grid steps by a whole number of texels).
        let o = Vec3::new(2., 0., -3.);
        let a = fit_cascades(camera(10.), 0.84, 1.5, 0.1, sun_direction(), 2048)[1];
        let ta = viewport_texel(&a, o);
        for dx in [0.3f32, 1.7, 5.9, -12.3] {
            let b = fit_cascades(camera(10.) * Mat4::from_translation(Vec3::new(dx, 0., 0.)), 0.84, 1.5, 0.1, sun_direction(), 2048)[1];
            let tb = viewport_texel(&b, o);
            let frac = |v: f32| (v - v.round()).abs();
            // fractional texel position of a fixed world point is identical (up to fp noise) in both fits
            let d = ((ta.0 - tb.0).rem_euclid(1.0)).min(1.0 - (ta.0 - tb.0).rem_euclid(1.0));
            let e = ((ta.1 - tb.1).rem_euclid(1.0)).min(1.0 - (ta.1 - tb.1).rem_euclid(1.0));
            assert!(d < 0.05 && e < 0.05, "dx {dx}: {ta:?} vs {tb:?} (frac {} {})", frac(ta.0), frac(tb.0));
        }
    }
    #[test]
    fn cascade_centre_snaps_to_multiples_of_four_texels_in_light_space() {
        // the origin of the cascade (light-space coordinates of the centre / W) is a multiple of 2 * 4 / ResolutionX after the snap
        let to_sun = sun_direction();
        for yaw in [0., 33., 211.] {
            let cs = fit_cascades(camera(yaw), 0.84, 1.5, 0.1, to_sun, 2048);
            let fwd = -to_sun;
            let right = fwd.cross(Vec3::Y).normalize();
            let up = right.cross(fwd);
            for c in cs {
                // the view matrix looks at the snapped centre: recover it from the centre of the clip volume (xy = 0 -> snapped centre axis)
                let inv = c.light_view_proj.inverse();
                let ctr = inv.project_point3(Vec3::new(0., 0., 0.5));
                let step = 8. / c.resolution as f32;
                for v in [ctr.dot(right) / c.radius, ctr.dot(up) / c.radius] {
                    let m = (v / step).round() * step;
                    assert!((v - m).abs() < 2e-4, "yaw {yaw}: {v} not on the {step} grid");
                }
            }
        }
    }
    #[test]
    fn cascade_sphere_is_not_rounded_and_resolution_excludes_the_border() {
        let cs = fit_cascades(camera(0.), 0.84, 16. / 9., 0.1, sun_direction(), 2048);
        for c in cs {
            assert_eq!(c.resolution, 2048 - 2 * SHADOW_BORDER);
            assert!((c.texel - 2. * c.radius / 2040.).abs() < 1e-7);
        }
        let (_, r) = split_sphere(0.84, 16. / 9., 0.1, 45.);
        assert_eq!(cs[2].radius, split_sphere(0.84, 16. / 9., 0.1 + 44.9 * 3. / 7., 45.).1);
        assert!(r > 0.);
        assert_eq!(cs[2].fade_extension, 0.);
        assert!((cs[0].fade_extension - (cs[0].far - 0.1) * 0.1).abs() < 1e-6);
    }
    #[test]
    fn csm_depth_bias_follows_update_shader_depth_bias() {
        // radius 15 m, depth range 2 * 50 m, 2040 texels: 20 / 10000 cm * (1500 / 2040) * 0.5 = 7.35e-4
        let b = csm_depth_bias(15., 100., 2040);
        assert!((b - 20. / 10000. * (1500. / 2040.) * 0.5).abs() < 1e-9, "{b}");
        let c = fit_cascades(camera(0.), 0.84, 16. / 9., 0.1, sun_direction(), 2048);
        for k in c { assert!((k.depth_bias - csm_depth_bias(k.radius, 2. * k.radius.max(50.), 2040)).abs() < 1e-9); }
    }
    #[test]
    fn texture_matrix_maps_the_viewport_into_the_bordered_map_with_flipped_v() {
        let c = fit_cascades(camera(0.), 0.84, 1.5, 0.1, sun_direction(), 2048)[0];
        let m = texture_matrix(&c);
        let p = Vec3::new(1., 0.5, -2.);
        let clip = c.light_view_proj.project_point3(p);
        let uv = m.project_point3(p);
        let k = 0.5 * 2040. / 2048.;
        assert!((uv.x - (clip.x * k + 0.5)).abs() < 1e-5);
        assert!((uv.y - (0.5 - clip.y * k)).abs() < 1e-5);
        assert!((uv.z - clip.z).abs() < 1e-5);
        // clip-space +-1 lands exactly SHADOW_BORDER texels inside the map edge
        let edge = m * c.light_view_proj.inverse() * Vec4::new(-1., 1., 0.5, 1.);
        assert!((edge.x / edge.w * 2048. - SHADOW_BORDER as f32).abs() < 1e-2 && (edge.y / edge.w * 2048. - SHADOW_BORDER as f32).abs() < 1e-2, "{edge:?}");
    }
    #[test]
    fn sun_radiance_at_pure_evening_is_the_serialized_evening_light() {
        let st = tod::state_at(tod::SUNSET_HOUR);
        assert_eq!((st.from, st.to), (tod::EVENING, tod::EVENING));
        let s = sun_radiance_of(&st);
        // Evening: LightColor FColor (255,174,114), Brightness 1.0, Lambert 1/pi
        assert!((s[0] - 1. / std::f32::consts::PI).abs() < 1e-5);
        assert!((s[1] / s[0] - srgb_to_linear(174)).abs() < 1e-5 && (s[2] / s[0] - srgb_to_linear(114)).abs() < 1e-5);
        assert!(s[0] > s[1] && s[1] > s[2], "warm sun {s:?}");
    }
    #[test]
    fn day_sun_irradiance_on_white_matches_lambert_pi_convention() {
        let s = sun_radiance_of(&tod::state_at(12.));
        assert!((s[0] - 3. / std::f32::consts::PI).abs() < 1e-5);
        assert!((s[1] / s[0] - srgb_to_linear(225)).abs() < 1e-5 && (s[2] / s[0] - srgb_to_linear(161)).abs() < 1e-5);
    }
    #[test]
    fn sky_light_colour_is_clamped_like_to_fcolor() {
        // Day SkyLightColor (3.575, 4.176, 5.0, 1.0) clamps to white x intensity 1; Evening (0.735, 0.575, 0.320, 0.8) stays below 1 and is scaled by 0.8
        let day = sky_light_tint_of(&tod::state_at(12.));
        assert!(day.iter().all(|v| (v - 1.).abs() < 1e-6), "{day:?}");
        let eve = sky_light_tint_of(&tod::state_at(tod::SUNSET_HOUR));
        for (i, c) in [0.735f32, 0.574_911, 0.319_725].into_iter().enumerate() { assert!((eve[i] / 0.8 - c).abs() < 0.01, "{i}: {}", eve[i]); }
        assert!(eve[0] > eve[1] && eve[1] > eve[2]);
    }
    #[test]
    fn sky_light_sh_is_finite_and_lights_down_facing_surfaces() {
        // SLS_SpecifiedCubemap with bLowerHemisphereIsBlack = 0 (sky_light.rs): down-facing normals get the cubemap's lower half, less than the top
        for h in [tod::SUNSET_HOUR, 12.] {
            let st = tod::state_at(h);
            let k = sky_radiance_of(&st);
            assert!(k.iter().all(|v| v.is_finite() && *v >= 0.), "{h}: {k:?}");
            let down = crate::sky_light::eval_sky_sh(&sky_sh_of(&st), -Vec3::Z);
            assert!(luma(down) > 0. && luma(down) < luma(k), "hour {h}: down {down:?} up {k:?}");
        }
    }
    #[test]
    fn dump_lighting_values_for_the_report() {
        for h in [19.0f32, 20.0, tod::SUNSET_HOUR, 21.0, 12.0] {
            let st = tod::state_at(h);
            let d = sun_direction_at(h);
            let sh = sky_sh_of(&st);
            eprintln!("hour {h}: phases {}->{} t {:.3} light elev {:.2} deg azimuth(UE yaw of light travel) {:.1}  sun model elev {:.2}  sun_rad {:?} tint {:?} sky SH up {:?} side {:?} down {:?}",
                st.from, st.to, st.blend, d.y.asin().to_degrees(), light_travel_direction_ue(h).y.atan2(light_travel_direction_ue(h).x).to_degrees(), st.to_sun.y.asin().to_degrees(),
                sun_radiance_of(&st), sky_light_tint_of(&st), crate::sky_light::eval_sky_sh(&sh, Vec3::Z), crate::sky_light::eval_sky_sh(&sh, Vec3::X), crate::sky_light::eval_sky_sh(&sh, -Vec3::Z));
        }
    }
    #[test]
    fn terrain_colour_multiplier_follows_recovered_parameters() {
        assert!((cheap_contrast(0.3, 0.) - 0.3).abs() < 1e-7 && cheap_contrast(0., -2.).abs() < 1e-7 && (cheap_contrast(1., -2.) - 1.).abs() < 1e-7);
        let m = terrain_color_multiplier();
        for i in 0..3 { assert!(m[i] >= TERRAIN_EDGE_DARK[i] && m[i] <= TERRAIN_HIGHLIGHT[i], "{i}: {}", m[i]); }
        assert!(m[0] > m[1] && m[1] > m[2] && luma(m) > 1.2 && luma(m) < 2.0, "{m:?}");
        // The WGSL retains the recovered values but must not fabricate a global terrain-mask tint on the baseplate.
        let g = fn_live::wgsl!("../shaders/scene.wgsl");
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
