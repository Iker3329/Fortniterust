//! DISTANCE FIELD HOOK: distance-field ambient occlusion of the movable Athena sky light (UE4.16 screen-grid DFAO, `RenderDistanceFieldLighting`:
//! global distance field clipmaps, tile-culled object cone tracing, geometry-aware upsample, temporal history + stability filter) and the sky-light
//! parameters the scene shaders apply it with (`SkyLighting.usf` `SkyLightDiffusePS`). Shaders: `shaders/distance_field_ao.wgsl`.
//!
//! # Why the sky light is shadowed at all
//! `AFortTimeOfDayManager`'s native constructor (1.8 exe 0x14062c240) creates the `SkyLightComponent` and stores `Mobility = 2` (Movable) at +0x13e
//! (USceneComponent::Mobility, SDK offset 0x13E) over the class default Stationary. A movable sky light without static lighting is rendered by
//! `RenderDynamicSkyLighting`, which runs DFAO when `CastShadows` (default true), `r.DistanceFieldAO` (1) and the AO show flag are on.
//!
//! # Values
//! * `OcclusionMaxDistance` 600, `OcclusionCombineMode` OCM_Multiply: SERIALIZED on TODM_A's `SkyLightComponent`; `OcclusionExponent` 0.7:
//!   TODM_A_Child_Athena's override (TODM_A has 0.8). `Contrast` is not serialized: 0 (clamped to 0.01 by `FDistanceFieldAOParameters`).
//! * `OcclusionTint` / `MinOcclusion` come from the day phases (`FUN_14064f050`: `SetOcclusionTint(ToFColor(SkyLightOcclusionTint, sRGB))`,
//!   `SetMinOcclusion(param[8])`, the phase blend lerps all ten floats, `FUN_14064f190`).
//! * cvars: the 1.8 exe registration defaults (crates/fn-exe-defaults/reports/exe-defaults.json); the 1.8 ini files are AES-encrypted and are not
//!   read, so an ini override of these would be missed.
//!
//! # Adaptations (see also the README)
//! * No G-buffer: the AO normal buffer is rebuilt from the depth buffer (geometric normals; UE uses the G-buffer normal, i.e. normal maps included).
//! * Forward shading: the sky lighting is applied inside the lit shaders. DFAO LAG (2026-10-07): the full depth pre-pass gives this frame's depth before them,
//!   so the DFAO runs between the pre-pass and the scene pass and the lit shaders read THIS frame's AO (`AOUpsamplePS` bilateral at the pixel's UV), as UE
//!   applies it after its base pass. Was: the previous frame's AO reprojected to the current world position, which left the disoccluded pixels around
//!   anything moving unoccluded for a frame (a light silhouette trailing the character).
//! * GDF PARTIAL UPDATES (2026-10-06): the global distance field clipmaps update like UE4.16 (`r.AOGlobalDistanceFieldPartialUpdates` /
//!   `StaggeredUpdates` 1 in 1.8, global_distance_field.rs): staggered, and only the slabs the camera movement exposed plus the modified objects' influence
//!   boxes are recomposited, at scrolled (toroidal, `FullUpdateOrigin`-anchored) texels. Was: the whole clipmap recomposited when its origin moved.
//! * Landscape heightfields in the global distance field: TERRAIN MAIN PASS (2026-10-07): the Athena landscape is composited like UE composites it (one heightfield
//!   lattice of the whole map, `DfAo::set_heightfield`; the main pass now draws the near landscape, so its depth is in the AO buffer). Only without a landscape
//!   the old flat 512 m plate plane stands in (it cut straight through the hills).
use crate::camera::CameraParams;
use crate::distance_field::{dir_to_df, to_df, DfObject, DfUniform};
use crate::global_distance_field::{ClipmapState, UpdateRegion};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

pub mod ue {
    /// `OcclusionMaxDistance` (cm): serialized 600 on TODM_A `SkyLightComponent`.
    pub const OCCLUSION_MAX_DISTANCE: f32 = 600.;
    /// `OcclusionExponent`: TODM_A_Child_Athena `SkyLightComponent` override (0.7; TODM_A 0.8, class default 1).
    pub const OCCLUSION_EXPONENT: f32 = 0.7;
    /// `Contrast`: not serialized, class default 0 -> `FMath::Clamp(Contrast, .01f, 2.0f)` (DistanceFieldAmbientOcclusion.cpp:113).
    pub const CONTRAST: f32 = 0.01;
    /// `OcclusionCombineMode`: serialized OCM_Multiply (SkyLighting.usf: `SkyVisibility * min(GBufferAO, SSAO)`).
    pub const COMBINE_MULTIPLY: bool = true;
    /// `r.AOGlobalDFStartDistance` (exe 100): ObjectMaxOcclusionDistance = min(OcclusionMaxDistance, 100).
    pub const GLOBAL_DF_START_DISTANCE: f32 = 100.;
    /// `r.AOStepExponentScale` (exe 0.5).
    pub const STEP_EXPONENT_SCALE: f32 = 0.5;
    /// `r.AOMaxViewDistance` (exe 20000), `GetMaxAOViewDistance` = min(., 65000).
    pub const MAX_VIEW_DISTANCE: f32 = 20000.;
    /// `r.AOViewFadeDistanceScale` (exe 0.7).
    pub const VIEW_FADE_DISTANCE_SCALE: f32 = 0.7;
    /// `r.AOHistoryWeight` .85, `r.AOHistoryMinConfidenceScale` .8, `r.AOHistoryDistanceThreshold` 30, `r.AOHistoryStabilityPass` 1 (exe defaults).
    pub const HISTORY_WEIGHT: f32 = 0.85;
    pub const HISTORY_MIN_CONFIDENCE_SCALE: f32 = 0.8;
    pub const HISTORY_DISTANCE_THRESHOLD: f32 = 30.;
    pub const HISTORY_STABILITY_PASS: bool = true;
    /// `r.AOGlobalDFResolution` (exe 128), `r.AOGlobalDFClipmapDistanceExponent` (exe 2), `GMaxGlobalDistanceFieldClipmaps` 4.
    pub const GLOBAL_DF_RESOLUTION: u32 = 128;
    pub const CLIPMAP_DISTANCE_EXPONENT: f32 = 2.;
    pub const NUM_CLIPMAPS: usize = 4;
    /// `AWorldSettings::GlobalDistanceFieldViewDistance` (cm): not serialized by Athena_Terrain's FortWorldSettings; UE4.16 ctor 20000 (WorldSettings.cpp:82).
    pub const GLOBAL_DISTANCE_FIELD_VIEW_DISTANCE: f32 = 20000.;
    /// `GCullGridTileSize` 16, `GMaxGridCulledObjects` 2047 (GlobalDistanceField.cpp:241 / 276).
    pub const CULL_GRID_TILE_SIZE: u32 = 16;
    pub const MAX_GRID_CULLED_OBJECTS: u32 = 2047;
    /// `GAODownsampleFactor` 2, `GConeTraceDownsampleFactor` 4, `GDistanceFieldAOTileSizeX` 16.
    pub const DOWNSAMPLE_FACTOR: u32 = 2;
    pub const CONE_TRACE_DOWNSAMPLE_FACTOR: u32 = 4;
    pub const TILE_SIZE: u32 = 16;
    /// `NUM_CONE_STEPS` 10, `NUM_CONE_DIRECTIONS` 9.
    pub const NUM_CONE_STEPS: u32 = 10;
    /// `RelaxedSpacedVectors9` (r.AOSampleSet exe 1), DistanceFieldAmbientOcclusion.cpp:150.
    pub const RELAXED_SPACED_VECTORS9: [[f32; 3]; 9] = [
        [-0.467612, 0.739424, 0.484347], [0.517459, -0.705440, 0.484346], [-0.419848, -0.767551, 0.484347],
        [0.343077, 0.804802, 0.484347], [0.364239, 0.244290, 0.898695], [-0.381547, 0.185815, 0.905481],
        [-0.870176, -0.090559, 0.484347], [0.874448, 0.027390, 0.484346], [0.032967, -0.435625, 0.899524],
    ];
    /// `JitterOffsets` (DistanceFieldScreenGridLighting.cpp:45), scaled by GConeTraceDownsampleFactor (`r.AOUseJitter` 1, `r.AOUseHistory` 1).
    pub const JITTER_OFFSETS: [[f32; 2]; 4] = [[0.25, 0.], [0.75, 0.25], [0.5, 0.75], [0., 0.5]];
    /// `r.AOMaxObjectsPerCullTile` (exe 256).
    pub const MAX_OBJECTS_PER_CULL_TILE: u32 = 256;
}

/// `GAOConeHalfAngle = acos(1 - 1 / 9)`.
pub fn cone_half_angle() -> f32 { (1. - 1. / 9f32).acos() }
/// `FDistanceFieldAOParameters`: (ObjectMaxOcclusionDistance, GlobalMaxOcclusionDistance) with the global distance field on.
pub fn occlusion_distances() -> (f32, f32) {
    let d = ue::OCCLUSION_MAX_DISTANCE.clamp(2., 3000.);
    (d.min(ue::GLOBAL_DF_START_DISTANCE), if d >= ue::GLOBAL_DF_START_DISTANCE { d } else { 0. })
}
/// `FAOParameters::Set`: AOStepScale = (ObjectMaxOcclusionDistance / (1 + tan(cone))) / 2^(AOStepExponentScale * (NumConeSteps - 1)).
pub fn step_scale() -> f32 {
    let largest = occlusion_distances().0 / (1. + cone_half_angle().tan());
    largest / 2f32.powf(ue::STEP_EXPONENT_SCALE * (ue::NUM_CONE_STEPS - 1) as f32)
}
/// `BentNormalNormalizeFactor = 1 / |mean(SampleDirections)|`.
pub fn bent_normal_normalize_factor() -> f32 {
    let s: Vec3 = ue::RELAXED_SPACED_VECTORS9.iter().map(|v| Vec3::from(*v)).sum();
    1. / (s / 9.).length()
}
/// `ComputeClipmapExtent`: GlobalDistanceFieldViewDistance / exponent^3 * exponent^i.
pub fn clipmap_extent(i: usize) -> f32 { ue::GLOBAL_DISTANCE_FIELD_VIEW_DISTANCE / ue::CLIPMAP_DISTANCE_EXPONENT.powi(3) * ue::CLIPMAP_DISTANCE_EXPONENT.powi(i as i32) }
/// `GlobalMaxSphereQueryRadius = MaxOcclusionDistance / (1 + tan(GAOConeHalfAngle))` with the sky light's (unclamped) OcclusionMaxDistance.
pub fn global_max_sphere_query_radius() -> f32 { ue::OCCLUSION_MAX_DISTANCE / (1. + cone_half_angle().tan()) }
/// `ContrastAndNormalizeMulAdd` (TDynamicSkyLightDiffusePS::SetParameters).
pub fn contrast_mul_add() -> [f32; 3] {
    let c = ue::CONTRAST;
    let min = 1. / (1. + (-c * (0. * 10. - 5.)).exp());
    let max = 1. / (1. + (-c * (1. * 10. - 5.)).exp());
    [c, 1. / (max - min), -min / (max - min)]
}

// ---- time-of-day sky-light occlusion values (TODM_A merged with TODM_A_Child_Athena, phase order as time_of_day::PHASES) ----
/// `SkyLightValues.SkyLightOcclusionTint` rgb per phase: the child's Morning / Day / Evening, the parent's Night (the child does not serialize it).
pub const PHASE_OCCLUSION_TINT: [[f32; 3]; 4] = [[0.015, 0.015, 0.015], [0.02, 0.02, 0.02], [0.005, 0.005, 0.005], [0.001131, 0.001648, 0.01]];
/// `SkyLightValues.SkyLightMinOcclusion` per phase: only TODM_A's Night serializes it (0.05); the others keep the struct default, ASSUMED 0 (the
/// native `FFortSkyLightValues` constructor was not traced).
pub const PHASE_MIN_OCCLUSION: [f32; 4] = [0., 0., 0., 0.05];
fn srgb_byte_to_linear(v: u8) -> f32 { let c = v as f32 / 255.; if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) } }
fn linear_to_srgb_byte(c: f32) -> u8 {
    let c = c.clamp(0., 1.);
    let e = if c <= 0.003_130_8 { c * 12.92 } else { c.powf(1. / 2.4) * 1.055 - 0.055 };
    (e * 255.999).floor() as u8
}
/// (OcclusionTint linear rgb as the sky-light proxy holds it, MinOcclusion) at `hour`.
pub fn sky_occlusion_at(hour: f32) -> ([f32; 3], f32) {
    let (a, b, t) = crate::time_of_day::blend_at(hour);
    let tint: [f32; 3] = std::array::from_fn(|i| PHASE_OCCLUSION_TINT[a][i] + (PHASE_OCCLUSION_TINT[b][i] - PHASE_OCCLUSION_TINT[a][i]) * t);
    let min_occ = PHASE_MIN_OCCLUSION[a] + (PHASE_MIN_OCCLUSION[b] - PHASE_MIN_OCCLUSION[a]) * t;
    // FUN_1412ca330 = FLinearColor::ToFColor(sRGB) -> FColor OcclusionTint -> FLinearColor(FColor) in the proxy; MinOcclusion clamped 0..1
    (tint.map(|c| srgb_byte_to_linear(linear_to_srgb_byte(c))), min_occ.clamp(0., 1.))
}

/// `struct AoU` of distance_field_ao.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct AoUniform {
    eye: [f32; 4], right: [f32; 4], up: [f32; 4], fwd: [f32; 4],
    prev_eye: [f32; 4], prev_right: [f32; 4], prev_up: [f32; 4], prev_fwd: [f32; 4],
    size: [f32; 4], half_size: [f32; 4], grid: [f32; 4], tiles: [f32; 4], jitter: [f32; 4],
    ao_a: [f32; 4], ao_b: [f32; 4], hist: [f32; 4], depth: [f32; 4],
    clip: [[f32; 4]; 4], clip_misc: [f32; 4],
    cones: [[f32; 4]; 9],
    /// GDF PARTIAL UPDATES: each clipmap's toroidal origin `FullUpdateOrigin * CellSize` (ComputeGlobalUV with the scroll offset)
    clip_full: [[f32; 4]; 4],
}
/// `struct GdfClip`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GdfClipUniform { center_extent: [f32; 4], misc: [f32; 4], hf: [f32; 4], /** GDF PARTIAL UPDATES */ region: [f32; 4], grid: [f32; 4], full: [f32; 4] }
/// GDF PARTIAL UPDATES: one [`GdfClipUniform`] per update region in one buffer, bound with a dynamic offset (WebGPU / wgpu minUniformBufferOffsetAlignment 256).
const REGION_STRIDE: usize = 256;

/// The camera in DF space (what the AO buffer of a frame was computed from).
#[derive(Clone, Copy, Debug)]
struct CamBasis { eye: Vec3, right: Vec3, up: Vec3, fwd: Vec3, tan_x: f32, tan_y: f32, near_cm: f32, view_proj: Mat4, eye_m: Vec3, fwd_m: Vec3 }
impl CamBasis {
    fn of(cam: &CameraParams) -> CamBasis {
        let (tx, ty) = cam.tan_half();
        let right = cam.forward.cross(cam.up).normalize();
        CamBasis { eye: to_df(cam.eye), right: dir_to_df(right), up: dir_to_df(cam.up.normalize()), fwd: dir_to_df(cam.forward.normalize()), tan_x: tx, tan_y: ty, near_cm: cam.near * 100.,
            view_proj: cam.proj_matrix() * cam.view_matrix(), eye_m: cam.eye, fwd_m: cam.forward.normalize() }
    }
}

/// HISTORY RESET (2026-10-05): `IsLargeCameraMovement` (UE4.16 SceneVisibility.cpp:2024) with the thresholds of the motion-blur / prev-transform reset
/// (SceneVisibility.cpp:2294: 45 deg, 10000 cm): the view's right, up or forward axis turned by more than `rot_deg`, or the eye moved more than `dist_cm`.
/// The up axis is the true view up (`right x forward`; `CamBasis::up` may be the world up), compared with the same formula in both frames.
fn is_large_camera_movement(cur: &CamBasis, prev: &CamBasis, rot_deg: f32, dist_cm: f32) -> bool {
    let k = rot_deg.to_radians().cos();
    let up = |c: &CamBasis| c.right.cross(c.fwd).normalize_or_zero();
    cur.right.dot(prev.right) < k || up(cur).dot(up(prev)) < k || cur.fwd.dot(prev.fwd) < k || (cur.eye - prev.eye).length_squared() > dist_cm * dist_cm
}
/// The literal thresholds of the `bResetCamera` / `bPrevTransformsReset` test (SceneVisibility.cpp:2294; not the occlusion-query `GEngine` thresholds).
const RESET_ROTATION_DEG: f32 = 45.;
const RESET_TRANSLATION_CM: f32 = 10000.;

const HDR16: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

struct Sized {
    w: u32, h: u32, half: [u32; 2], grid: [u32; 2], tiles: [u32; 2],
    normal: wgpu::Texture, bent_low: wgpu::Texture, bent_half: wgpu::Texture, conf: wgpu::Texture,
    hist_new: wgpu::Texture, conf_new: wgpu::Texture, state: [wgpu::Texture; 2], state_conf: [wgpu::Texture; 2],
    tile_cones: wgpu::Buffer, tile_lists: wgpu::Buffer,
}

struct Pipes {
    layouts: Vec<wgpu::BindGroupLayout>,
    normal: wgpu::ComputePipeline, tile_cones: wgpu::ComputePipeline, tile_cull: wgpu::ComputePipeline, cone_trace: wgpu::ComputePipeline,
    upsample: wgpu::ComputePipeline, history: wgpu::ComputePipeline, filter: wgpu::ComputePipeline, gdf: wgpu::ComputePipeline, gdf_cull: wgpu::ComputePipeline,
    /// PERF (2026-10-06): the cone trace's global-distance-field sampler (binding 33): linear + repeat when the device filters R32Float (hardware trilinear, UE's
    /// Texture3DSampleLevel SF_Bilinear + AM_Wrap), else nearest (the shader then keeps its manual 8-tap filter).
    gdf_samp: wgpu::Sampler,
}
const P_NORMAL: usize = 0;
const P_TILE_CONES: usize = 1;
const P_TILE_CULL: usize = 2;
const P_CONE: usize = 3;
const P_UPSAMPLE: usize = 4;
const P_HISTORY: usize = 5;
const P_FILTER: usize = 6;
const P_GDF: usize = 7;
const P_GDF_CULL: usize = 8;

fn bgl_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry { wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None } }
fn uni() -> wgpu::BindingType { wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None } }
/// GDF PARTIAL UPDATES: the per-region [`GdfClipUniform`] (dynamic offset).
fn uni_dyn() -> wgpu::BindingType { wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<GdfClipUniform>() as u64) } }
fn sto(ro: bool) -> wgpu::BindingType { wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: ro }, has_dynamic_offset: false, min_binding_size: None } }
fn tex2(filterable: bool) -> wgpu::BindingType { wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false } }
fn tex3(filterable: bool) -> wgpu::BindingType { wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false } }
fn st2(f: wgpu::TextureFormat) -> wgpu::BindingType { wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: f, view_dimension: wgpu::TextureViewDimension::D2 } }
fn samp() -> wgpu::BindingType { wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering) }

impl Pipes {
    fn new(device: &wgpu::Device) -> Pipes {
        // PERF (2026-10-06): hardware-filtered global distance field when R32Float is filterable on this device (Features::FLOAT32_FILTERABLE)
        let hw = device.features().contains(wgpu::Features::FLOAT32_FILTERABLE);
        let src = fn_live::wgsl!("../shaders/distance_field_ao.wgsl").replace("const GDF_HW: bool = false;", if hw { "const GDF_HW: bool = true;" } else { "const GDF_HW: bool = false;" });
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("distance field ao"), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let gdf_samp_ty = wgpu::BindingType::Sampler(if hw { wgpu::SamplerBindingType::Filtering } else { wgpu::SamplerBindingType::NonFiltering });
        let gdf_filter = if hw { wgpu::FilterMode::Linear } else { wgpu::FilterMode::Nearest };
        let gdf_samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("dfao global distance field"), mag_filter: gdf_filter, min_filter: gdf_filter, address_mode_u: wgpu::AddressMode::Repeat, address_mode_v: wgpu::AddressMode::Repeat, address_mode_w: wgpu::AddressMode::Repeat, ..Default::default() });
        let depth = wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let gdf_out = wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: wgpu::TextureFormat::R32Float, view_dimension: wgpu::TextureViewDimension::D3 };
        let sets: Vec<Vec<wgpu::BindGroupLayoutEntry>> = vec![
            vec![bgl_entry(0, uni()), bgl_entry(1, depth), bgl_entry(2, st2(HDR16))],
            vec![bgl_entry(0, uni()), bgl_entry(3, tex2(false)), bgl_entry(4, sto(false))],
            vec![bgl_entry(0, uni()), bgl_entry(5, sto(true)), bgl_entry(6, sto(true)), bgl_entry(7, sto(true)), bgl_entry(8, tex3(true)), bgl_entry(9, samp()), bgl_entry(10, sto(false))],
            vec![bgl_entry(0, uni()), bgl_entry(3, tex2(false)), bgl_entry(7, sto(true)), bgl_entry(8, tex3(true)), bgl_entry(9, samp()), bgl_entry(11, sto(true)), bgl_entry(12, tex3(hw)), bgl_entry(13, st2(HDR16)), bgl_entry(33, gdf_samp_ty)],
            vec![bgl_entry(0, uni()), bgl_entry(3, tex2(false)), bgl_entry(14, tex2(false)), bgl_entry(15, st2(HDR16)), bgl_entry(16, st2(HDR16))],
            vec![bgl_entry(0, uni()), bgl_entry(17, tex2(false)), bgl_entry(18, tex2(false)), bgl_entry(19, tex2(true)), bgl_entry(20, tex2(true)), bgl_entry(21, st2(HDR16)), bgl_entry(22, st2(HDR16)), bgl_entry(30, samp())],
            vec![bgl_entry(0, uni()), bgl_entry(23, tex2(false)), bgl_entry(24, tex2(false)), bgl_entry(25, st2(HDR16)), bgl_entry(26, st2(HDR16))],
            vec![bgl_entry(0, uni()), bgl_entry(7, sto(true)), bgl_entry(8, tex3(true)), bgl_entry(9, samp()), bgl_entry(27, gdf_out), bgl_entry(28, sto(true)), bgl_entry(29, uni_dyn()), bgl_entry(34, tex2(false)) /* TERRAIN MAIN PASS: the landscape heightfield */],
            vec![bgl_entry(7, sto(true)), bgl_entry(29, uni_dyn()), bgl_entry(31, sto(true)), bgl_entry(32, sto(false))], // PERF: CullObjectsToGridCS
        ];
        let layouts: Vec<wgpu::BindGroupLayout> = sets.iter().map(|e| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("dfao bgl"), entries: e })).collect();
        let mk = |i: usize, entry: &str| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(entry), bind_group_layouts: &[Some(&layouts[i])], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(entry), layout: Some(&pl), module: &sm, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        Pipes {
            normal: mk(P_NORMAL, "cs_normal"), tile_cones: mk(P_TILE_CONES, "cs_tile_cones"), tile_cull: mk(P_TILE_CULL, "cs_tile_cull"), cone_trace: mk(P_CONE, "cs_cone_trace"),
            upsample: mk(P_UPSAMPLE, "cs_upsample"), history: mk(P_HISTORY, "cs_history"), filter: mk(P_FILTER, "cs_history_filter"), gdf: mk(P_GDF, "cs_gdf_composite"), gdf_cull: mk(P_GDF_CULL, "cs_gdf_cull"), gdf_samp,
            layouts,
        }
    }
}

struct BindGroups { normal: wgpu::BindGroup, tile_cones: wgpu::BindGroup, tile_cull: wgpu::BindGroup, cone: wgpu::BindGroup, upsample: wgpu::BindGroup, history: [wgpu::BindGroup; 2], filter: [wgpu::BindGroup; 2], /** GDF PARTIAL UPDATES: one each, the region's uniform at a dynamic offset */ gdf: wgpu::BindGroup, gdf_cull: wgpu::BindGroup }

/// What the frame's AO dispatches need from the renderer.
pub(crate) struct AoFrameInput<'a> {
    pub cam: &'a CameraParams,
    pub objects: &'a [DfObject],
    pub objects_buf: &'a wgpu::Buffer,
    pub atlas_view: &'a wgpu::TextureView,
    pub atlas_sampler: &'a wgpu::Sampler,
    pub depth_view: &'a wgpu::TextureView,
    /// Changes whenever the depth texture / objects buffer / atlas were re-created (bind groups are rebuilt).
    pub resource_gen: u64,
    /// Half edge of the ground plate (m) represented as the global distance field heightfield.
    pub plate_half_m: f32,
}

pub(crate) struct DfAo {
    pipes: Option<Pipes>,
    uniform: wgpu::Buffer,
    /// GDF PARTIAL UPDATES (2026-10-06): this frame's update regions' [`GdfClipUniform`]s at [`REGION_STRIDE`] (dynamic offsets), capacity in regions.
    region_buf: wgpu::Buffer,
    region_cap: usize,
    /// The cull grid of the region being composited (`GObjectGridBuffers`, one for all regions: each region's cull + composite run back to back).
    grid_buf: wgpu::Buffer,
    /// PERF: the four clipmaps' volume-culled object lists (count, indices) at offset `clip * (near_cap + 1)`, read by `cs_gdf_cull`, and the capacity in objects.
    near_buf: wgpu::Buffer,
    near_cap: usize,
    view_objects: wgpu::Buffer,
    view_objects_cap: usize,
    gdf: wgpu::Texture,
    lin_samp: wgpu::Sampler,
    scene_tex: wgpu::Texture,
    scene_view: wgpu::TextureView,
    sized: Option<Sized>,
    bgs: Option<BindGroups>,
    bg_key: (u64, u64),
    sized_gen: u64,
    parity: usize,
    history_valid: bool,
    temporal_index: u32,
    gdf_update_index: u32,
    /// GDF PARTIAL UPDATES (2026-10-06): `FGlobalDistanceFieldClipmapState` of each clipmap (global_distance_field.rs) and this frame's update regions
    /// (clipmap, region, the clipmap's FullUpdateOrigin * CellSize), composited in this order.
    clip_state: [ClipmapState; ue::NUM_CLIPMAPS],
    regions: Vec<(usize, UpdateRegion, Vec3)>,
    /// `bForceFullUpdate` of the next prepared frame (the clipmaps were never composited, or their textures were re-created).
    gdf_force: bool,
    /// TERRAIN MAIN PASS (2026-10-07): the landscape heightfield of the global distance field (R32Float game heights in DF cm, [`DfAo::set_heightfield`]) and its
    /// `GdfClip.hf` (2, cell cm, lattice origin x / y in DF cm); a 1x1 stand-in is bound without one.
    hf: Option<(wgpu::Texture, [f32; 4])>,
    hf_dummy: wgpu::Texture,
    /// Camera of the frame whose AO is in `scene_tex` (None = no AO yet).
    scene_cam: Option<CamBasis>,
    pending_cam: Option<CamBasis>,
    /// Active this frame (dispatches recorded).
    active: bool,
    pub view_object_count: usize,
    /// HISTORY RESET (2026-10-05): `View.bCameraCut` of the next prepared frame (see [`DfAo::camera_cut`]).
    camera_cut: bool,
}

impl DfAo {
    pub fn new(device: &wgpu::Device) -> DfAo {
        let ub = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let storage = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let r = ue::GLOBAL_DF_RESOLUTION;
        let gdf = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("global distance field clipmaps (4 x 128^3, R16F values)"), size: wgpu::Extent3d { width: r, height: r, depth_or_array_layers: r * ue::NUM_CLIPMAPS as u32 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3, format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[],
        });
        let grid_cells = (r / ue::CULL_GRID_TILE_SIZE).pow(3) as usize;
        let scene_tex = Self::tex(device, "dfao scene (unused)", 1, 1);
        let scene_view = scene_tex.create_view(&Default::default());
        DfAo {
            pipes: None,
            uniform: ub("dfao uniform", std::mem::size_of::<AoUniform>()),
            region_buf: ub("gdf update regions", 16 * REGION_STRIDE),
            region_cap: 16,
            grid_buf: storage("gdf cull grid", grid_cells * (ue::MAX_GRID_CULLED_OBJECTS as usize + 1) * 4),
            near_buf: storage("gdf volume-culled objects", ue::NUM_CLIPMAPS * (256 + 1) * 4),
            near_cap: 256,
            view_objects: storage("dfao view objects", 256 * 4),
            view_objects_cap: 256,
            gdf,
            lin_samp: device.create_sampler(&wgpu::SamplerDescriptor { label: Some("dfao history sampler"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
            scene_tex, scene_view, sized: None, bgs: None, bg_key: (u64::MAX, u64::MAX), sized_gen: 0, parity: 0, history_valid: false, temporal_index: 0, gdf_update_index: 0,
            clip_state: Default::default(), regions: vec![], gdf_force: true, scene_cam: None, pending_cam: None, active: false, view_object_count: 0,
            camera_cut: false,
            hf: None, hf_dummy: Self::hf_texture(device, 1, 1),
        }
    }
    fn tex(device: &wgpu::Device, label: &str, w: u32, h: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label), size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: HDR16,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        })
    }
    fn hf_texture(device: &wgpu::Device, w: u32, h: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gdf landscape heightfield"), size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::R32Float, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        })
    }
    /// TERRAIN MAIN PASS (2026-10-07): the landscape heightfield composited into the global distance field (UE4.16 CompositeHeightfieldsIntoGlobalDistanceFieldCS,
    /// GlobalDistanceField.usf:314: distance = dot(N, P - surface point under P) of the heightfield, clamped to +- AOGlobalMaxSphereQueryRadius, only inside its
    /// footprint). Replaces the old flat plate plane, which cut straight through the hills. One static R32Float lattice of the whole map (textureLoad + manual
    /// bilinear: WebGPU has no filterable 32-bit floats by default); every clipmap is recomposited on its next update. `None` = back to the plate (renderer).
    pub fn set_heightfield(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, g: Option<&crate::backdrop::HeightGrid>) {
        self.hf = g.filter(|g| g.w >= 2 && g.h >= 2 && g.y.len() == (g.w * g.h) as usize).map(|g| {
            let t = Self::hf_texture(device, g.w, g.h);
            let cm: Vec<f32> = g.y.iter().map(|&y| if y <= crate::backdrop::HEIGHT_HOLE { -1.0e9 } else { y * 100. }).collect();
            queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, bytemuck::cast_slice(&cm),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * g.w), rows_per_image: Some(g.h) }, wgpu::Extent3d { width: g.w, height: g.h, depth_or_array_layers: 1 });
            // DF space = renderer (x, z, y) * 100 (distance_field.rs to_df): lattice i runs along game x = DF x, j along game z = DF y
            (t, [2., g.cell * 100., g.origin[0] * 100., g.origin[1] * 100.])
        });
        self.gdf_force = true;
        self.bg_key = (u64::MAX, u64::MAX);
    }
    pub fn scene_view(&self) -> &wgpu::TextureView { &self.scene_view }
    /// The atlas was re-packed (new volumes): UE4.16 re-queues every primitive when `GDistanceFieldVolumeTextureAtlas.GetGeneration()` changes
    /// (DistanceFieldObjectManagement.cpp:976-990), i.e. at least 100 modified bounds per clipmap = a full update of each clipmap on its next update turn.
    pub fn atlas_changed(&mut self) {
        for st in self.clip_state.iter_mut() { st.modified.resize(crate::global_distance_field::MAX_PARTIAL_PRIMITIVE_MODIFICATIONS, Vec4::ZERO); }
        self.bg_key = (u64::MAX, u64::MAX);
    }
    /// HISTORY RESET (2026-10-05): the game's camera cut (`PlayerCameraManager::bGameCameraCutThisFrame` -> `FSceneView::bCameraCut`, LocalPlayer.cpp:690)
    /// for the next [`DfAo::prepare`]. Large camera moves (teleports > 100 m, turns > 45 deg) are detected by `prepare` itself, like UE's `bPrevTransformsReset`.
    pub fn camera_cut(&mut self) { self.camera_cut = true; }
    /// GDF PARTIAL UPDATES: the bounding spheres (DF space: centre, radius) of the objects added, removed or moved since the last object rebuild
    /// (`PrimitiveModifiedBounds`, DistanceFieldObjectManagement.cpp:590 / :923 / :928), accumulated by every clipmap until its next update
    /// (GlobalDistanceField.cpp:900). Only `< 100` of them matter (more = a full update), so the lists stop growing there.
    pub fn objects_changed(&mut self, modified: &[Vec4]) {
        let cap = crate::global_distance_field::MAX_PARTIAL_PRIMITIVE_MODIFICATIONS;
        for st in self.clip_state.iter_mut() { for b in modified { if st.modified.len() >= cap { break; } st.modified.push(*b); } }
    }

    /// The sky-occlusion half of the scene uniform. The sky light's `OcclusionExponent` / `MinOcclusion` / `OcclusionTint` / `OcclusionCombineMode` are set
    /// every frame, DFAO or not (UE4.16 DistanceFieldAmbientOcclusion.cpp:932-940 `TDynamicSkyLightDiffusePS::SetParameters`, both `APPLY_SHADOWING`
    /// permutations; SkyLighting.usf:76-88 apply them outside `#if APPLY_SHADOWING`); `ao0.x` = 1 (the AO buffer / bent normal) when the DFAO runs this frame.
    /// DFAO LAG (2026-10-07): the buffer is THIS frame's (renderer node "dfao" before the scene pass), so the lookup matrices (`prev_*` in the uniform, names kept)
    /// are this frame's camera (`pending_cam`); the lit shaders' reprojection is then the identity. Was: the previous frame's buffer + camera (`scene_cam`).
    pub fn fill_uniform(&self, u: &mut DfUniform) {
        let (tint, min_occ) = sky_occlusion_at(crate::time_of_day::hour());
        u.ao0 = [0., ue::OCCLUSION_EXPONENT.clamp(0.1, 10.), min_occ, if ue::COMBINE_MULTIPLY { 1. } else { 0. }];
        u.ao1 = [tint[0], tint[1], tint[2], 0.];
        let Some(c) = self.pending_cam.filter(|_| self.active) else { return };
        let cm = contrast_mul_add();
        let s = self.sized.as_ref().map_or([1, 1], |s| s.half);
        u.ao0[0] = 1.;
        u.ao2 = [cm[0], cm[1], cm[2], 0.];
        u.ao3 = [s[0] as f32, s[1] as f32, 1. / s[0] as f32, 1. / s[1] as f32];
        u.prev_view_proj = c.view_proj.to_cols_array_2d();
        u.prev_eye = c.eye_m.extend(0.).to_array();
        u.prev_fwd = c.fwd_m.extend(0.).to_array();
    }

    fn ensure_sized(&mut self, device: &wgpu::Device, w: u32, h: u32) -> bool {
        if matches!(&self.sized, Some(s) if s.w == w && s.h == h) { return false; }
        let half = [w / ue::DOWNSAMPLE_FACTOR, h / ue::DOWNSAMPLE_FACTOR];
        let grid = [half[0] / ue::CONE_TRACE_DOWNSAMPLE_FACTOR, half[1] / ue::CONE_TRACE_DOWNSAMPLE_FACTOR];
        let tiles = [half[0].div_ceil(ue::TILE_SIZE), half[1].div_ceil(ue::TILE_SIZE)];
        let buf = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16) as u64, usage: wgpu::BufferUsages::STORAGE, mapped_at_creation: false });
        let t = |l: &str, w: u32, h: u32| Self::tex(device, l, w, h);
        self.sized = Some(Sized {
            w, h, half, grid, tiles,
            normal: t("dfao normal", half[0], half[1]), bent_low: t("dfao bent normal (screen grid)", grid[0].max(1), grid[1].max(1)),
            bent_half: t("dfao bent normal (upsampled)", half[0], half[1]), conf: t("dfao confidence", half[0], half[1]),
            hist_new: t("dfao history (reprojected)", half[0], half[1]), conf_new: t("dfao confidence (reprojected)", half[0], half[1]),
            state: [t("dfao history A", half[0], half[1]), t("dfao history B", half[0], half[1])],
            state_conf: [t("dfao confidence history A", half[0], half[1]), t("dfao confidence history B", half[0], half[1])],
            tile_cones: buf("dfao tile cones", (tiles[0] * tiles[1]) as usize * 32),
            tile_lists: buf("dfao tile object lists", (tiles[0] * tiles[1]) as usize * (ue::MAX_OBJECTS_PER_CULL_TILE as usize + 1) * 4),
        });
        self.scene_tex = Self::tex(device, "dfao scene bent normal", half[0], half[1]);
        self.scene_view = self.scene_tex.create_view(&Default::default());
        self.history_valid = false;
        self.scene_cam = None;
        self.sized_gen += 1;
        true
    }

    fn build_bind_groups(&mut self, device: &wgpu::Device, inp: &AoFrameInput) {
        let pipes = self.pipes.as_ref().unwrap();
        let s = self.sized.as_ref().unwrap();
        let v = |t: &wgpu::Texture| t.create_view(&Default::default());
        let gdf_sampled = self.gdf.create_view(&Default::default());
        let gdf_storage = self.gdf.create_view(&Default::default());
        let hf_view = self.hf.as_ref().map_or(&self.hf_dummy, |h| &h.0).create_view(&Default::default()); // TERRAIN MAIN PASS
        let (normal, bent_low, bent_half, conf, hist_new, conf_new) = (v(&s.normal), v(&s.bent_low), v(&s.bent_half), v(&s.conf), v(&s.hist_new), v(&s.conf_new));
        let state = [v(&s.state[0]), v(&s.state[1])];
        let state_conf = [v(&s.state_conf[0]), v(&s.state_conf[1])];
        use wgpu::BindingResource as R;
        fn e(b: u32, r: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding: b, resource: r } }
        let bg = |i: usize, entries: &[wgpu::BindGroupEntry]| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("dfao bg"), layout: &pipes.layouts[i], entries });
        let u = || e(0, self.uniform.as_entire_binding());
        let region_window = || wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &self.region_buf, offset: 0, size: wgpu::BufferSize::new(std::mem::size_of::<GdfClipUniform>() as u64) });
        let (region, region_c) = (region_window(), region_window());
        let history = std::array::from_fn(|p| bg(P_HISTORY, &[u(), e(17, R::TextureView(&bent_half)), e(18, R::TextureView(&conf)), e(19, R::TextureView(&state[p])), e(20, R::TextureView(&state_conf[p])),
            e(21, R::TextureView(&hist_new)), e(22, R::TextureView(&conf_new)), e(30, R::Sampler(&self.lin_samp))]));
        let filter = std::array::from_fn(|p| bg(P_FILTER, &[u(), e(23, R::TextureView(&hist_new)), e(24, R::TextureView(&conf_new)), e(25, R::TextureView(&state[1 - p])), e(26, R::TextureView(&state_conf[1 - p]))]));
        self.bgs = Some(BindGroups {
            normal: bg(P_NORMAL, &[u(), e(1, R::TextureView(inp.depth_view)), e(2, R::TextureView(&normal))]),
            tile_cones: bg(P_TILE_CONES, &[u(), e(3, R::TextureView(&normal)), e(4, s.tile_cones.as_entire_binding())]),
            tile_cull: bg(P_TILE_CULL, &[u(), e(5, s.tile_cones.as_entire_binding()), e(6, self.view_objects.as_entire_binding()), e(7, inp.objects_buf.as_entire_binding()),
                e(8, R::TextureView(inp.atlas_view)), e(9, R::Sampler(inp.atlas_sampler)), e(10, s.tile_lists.as_entire_binding())]),
            cone: bg(P_CONE, &[u(), e(3, R::TextureView(&normal)), e(7, inp.objects_buf.as_entire_binding()), e(8, R::TextureView(inp.atlas_view)), e(9, R::Sampler(inp.atlas_sampler)),
                e(11, s.tile_lists.as_entire_binding()), e(12, R::TextureView(&gdf_sampled)), e(13, R::TextureView(&bent_low)), e(33, R::Sampler(&pipes.gdf_samp))]),
            upsample: bg(P_UPSAMPLE, &[u(), e(3, R::TextureView(&normal)), e(14, R::TextureView(&bent_low)), e(15, R::TextureView(&bent_half)), e(16, R::TextureView(&conf))]),
            history, filter,
            // GDF PARTIAL UPDATES: the region uniform is a GdfClipUniform-sized window of `region_buf`, moved per dispatch with a dynamic offset
            gdf: bg(P_GDF, &[u(), e(7, inp.objects_buf.as_entire_binding()), e(8, R::TextureView(inp.atlas_view)), e(9, R::Sampler(inp.atlas_sampler)),
                e(27, R::TextureView(&gdf_storage)), e(28, self.grid_buf.as_entire_binding()), e(29, region), e(34, R::TextureView(&hf_view))]),
            gdf_cull: bg(P_GDF_CULL, &[e(7, inp.objects_buf.as_entire_binding()), e(29, region_c), e(31, self.near_buf.as_entire_binding()), e(32, self.grid_buf.as_entire_binding())]),
        });
    }

    /// CPU half of a frame: sizes, culling (view objects, global distance field grids), uniforms, bind groups. Returns true when the scene AO view was
    /// re-created (the renderer's globals bind group must be rebuilt). `enabled` = the AO show flag and a non-empty distance-field scene.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, inp: &AoFrameInput, enabled: bool) -> bool {
        self.active = enabled && !inp.objects.is_empty();
        if !self.active { self.history_valid = false; self.scene_cam = None; return false; }
        if self.pipes.is_none() { self.pipes = Some(Pipes::new(device)); }
        let cam = inp.cam;
        let resized = self.ensure_sized(device, cam.width, cam.height);
        let cb = CamBasis::of(cam);
        // HISTORY RESET (2026-10-05): UE4.16 `UpdateHistory` (DistanceFieldLightingPost.cpp:429) reuses the AO history only when `!View.bCameraCut &&
        // !View.bPrevTransformsReset`; `bPrevTransformsReset` = first frame / time reset || bCameraCut || IsLargeCameraMovement(45 deg, 10000 cm) against the
        // previous view (SceneVisibility.cpp:2292). On a reset the history is the current frame's result ("jitter.z = 0" in cs_history) and the previous frame's
        // AO (reprojected by the lit pass, this port's one-frame lag) is dropped instead of being smeared across the cut.
        let cut = std::mem::take(&mut self.camera_cut);
        if cut || self.scene_cam.as_ref().is_some_and(|p| is_large_camera_movement(&cb, p, RESET_ROTATION_DEG, RESET_TRANSLATION_CM)) {
            self.history_valid = false;
            self.scene_cam = None;
        }
        // ---- view culling (CullObjectsForViewCS): within AOMaxViewDistance + r of the camera, sphere (r + AOObjectMaxDistance) vs the frustum ----
        let (obj_max, global_max) = occlusion_distances();
        let max_view = ue::MAX_VIEW_DISTANCE.min(65000.);
        let mut view_list: Vec<u32> = vec![];
        let side = |t: f32| (1. + t * t).sqrt();
        for (i, o) in inp.objects.iter().enumerate() {
            let d = o.center - cb.eye;
            if d.length_squared() >= (max_view + o.radius).powi(2) { continue; }
            let r = o.radius + obj_max;
            let (x, y, z) = (d.dot(cb.right), d.dot(cb.up), d.dot(cb.fwd));
            if z < cb.near_cm - r { continue; }
            if (x - z * cb.tan_x) / side(cb.tan_x) > r || (-x - z * cb.tan_x) / side(cb.tan_x) > r { continue; }
            if (y - z * cb.tan_y) / side(cb.tan_y) > r || (-y - z * cb.tan_y) / side(cb.tan_y) > r { continue; }
            view_list.push(i as u32);
        }
        self.view_object_count = view_list.len();
        let mut rebind = false;
        if view_list.len() > self.view_objects_cap {
            self.view_objects_cap = view_list.len().next_power_of_two();
            self.view_objects = device.create_buffer(&wgpu::BufferDescriptor { label: Some("dfao view objects"), size: (self.view_objects_cap * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            rebind = true;
        }
        if !view_list.is_empty() { queue.write_buffer(&self.view_objects, 0, bytemuck::cast_slice(&view_list)); }

        // ---- global distance field: ComputeUpdateRegionsAndUpdateViewState (GlobalDistanceField.cpp:865) ----
        // GDF PARTIAL UPDATES (2026-10-06, global_distance_field.rs): staggered (clipmap 0 every frame, 1 every 2nd, 2 / 3 every 4th) AND partial: an updating
        // clipmap composites only the slabs the camera movement exposed and the influence boxes of the objects modified since its last update, written at
        // their scrolled (toroidal) texels; a full update only when forced (first use) or when the partial regions would cost more. Was: the whole clipmap
        // recomposited whenever its snapped origin moved (about 1 ms on a third of the frames of the forest walk) and on every object change.
        self.gdf_update_index += 1;
        if self.gdf_update_index > 4 { self.gdf_update_index = 0; }
        let radius = global_max_sphere_query_radius();
        let res = ue::GLOBAL_DF_RESOLUTION;
        let force = std::mem::take(&mut self.gdf_force);
        // PERF: the volume-culled lists hold at most every object (grown before any clipmap writes this frame's list)
        if inp.objects.len() > self.near_cap {
            self.near_cap = inp.objects.len().next_power_of_two();
            self.near_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gdf volume-culled objects"), size: (ue::NUM_CLIPMAPS * (self.near_cap + 1) * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            rebind = true;
        }
        self.regions.clear();
        let mut clip = [[0f32; 4]; 4];
        let mut clip_full = [[0f32; 4]; 4];
        for c in 0..ue::NUM_CLIPMAPS {
            let extent = clipmap_extent(c);
            let cell = extent * 2. / res as f32;
            if let Some(regions) = crate::global_distance_field::update_clipmap(&mut self.clip_state[c], c, self.gdf_update_index, force, cb.eye, extent, res as i32, radius) {
                let st = &self.clip_state[c];
                let full = st.full_update_origin.as_vec3() * cell;
                if !regions.is_empty() {
                    // CullObjectsForVolumeCS for the clipmap's bounds (a superset of each region's list; CullObjectsToGridCS then tests every grid cell exactly)
                    let center = st.last_partial_update_origin.as_vec3() * cell;
                    let vb_w = Vec3::splat(extent).length();
                    let mut list: Vec<u32> = Vec::with_capacity(inp.objects.len() + 1);
                    list.push(0);
                    list.extend(inp.objects.iter().enumerate().filter(|(_, o)| o.center.distance(center) < vb_w + o.radius + radius).map(|(oi, _)| oi as u32));
                    list[0] = (list.len() - 1) as u32;
                    queue.write_buffer(&self.near_buf, (c * (self.near_cap + 1) * 4) as u64, bytemuck::cast_slice(&list));
                }
                for r in regions { self.regions.push((c, r, full)); }
            }
            let st = &self.clip_state[c];
            let center = st.last_partial_update_origin.as_vec3() * cell;
            clip[c] = [center.x, center.y, center.z, extent];
            let full = st.full_update_origin.as_vec3() * cell;
            clip_full[c] = [full.x, full.y, full.z, 0.];
        }
        // one GdfClipUniform per region (dynamic offsets): UpdateRegionVolumeMin, UpdateRegionSize, CullGridDimension = DivideAndRoundUp(CellsSize, 16)
        if self.regions.len() > self.region_cap {
            self.region_cap = self.regions.len().next_power_of_two();
            self.region_buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gdf update regions"), size: (self.region_cap * REGION_STRIDE) as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            rebind = true;
        }
        if !self.regions.is_empty() {
            // TERRAIN MAIN PASS (2026-10-07): the landscape heightfield (mode 2) when installed; the flat plate plane (mode 1) only without a landscape (plate_half_m > 0)
            let hf_half = inp.plate_half_m * 100.;
            let hf = self.hf.as_ref().map(|h| h.1).unwrap_or(if hf_half > 0. { [1., 0., hf_half, hf_half] } else { [0.; 4] });
            let mut bytes = vec![0u8; self.regions.len() * REGION_STRIDE];
            for (i, (c, r, full)) in self.regions.iter().enumerate() {
                let gd = (r.cells + 15) / ue::CULL_GRID_TILE_SIZE as i32;
                let cu = GdfClipUniform {
                    center_extent: [r.bounds.min.x, r.bounds.min.y, r.bounds.min.z, clipmap_extent(*c)],
                    misc: [*c as f32, radius, res as f32, (*c * (self.near_cap + 1)) as f32],
                    hf,
                    region: [r.cells.x as f32, r.cells.y as f32, r.cells.z as f32, 0.],
                    grid: [gd.x as f32, gd.y as f32, gd.z as f32, 0.],
                    full: [full.x, full.y, full.z, 0.],
                };
                bytes[i * REGION_STRIDE..i * REGION_STRIDE + std::mem::size_of::<GdfClipUniform>()].copy_from_slice(bytemuck::bytes_of(&cu));
            }
            queue.write_buffer(&self.region_buf, 0, &bytes);
        }

        // ---- the AO uniform ----
        let prev = self.scene_cam.unwrap_or(cb);
        let s = self.sized.as_ref().unwrap();
        let jit = ue::JITTER_OFFSETS[self.temporal_index as usize % 4];
        let cones: [[f32; 4]; 9] = std::array::from_fn(|i| { let v = ue::RELAXED_SPACED_VECTORS9[i]; [v[0], v[1], v[2], 0.] });
        let f4 = |v: Vec3, w: f32| [v.x, v.y, v.z, w];
        let au = AoUniform {
            eye: f4(cb.eye, 0.), right: f4(cb.right, cb.tan_x), up: f4(cb.up, cb.tan_y), fwd: f4(cb.fwd, cb.near_cm),
            prev_eye: f4(prev.eye, 0.), prev_right: f4(prev.right, prev.tan_x), prev_up: f4(prev.up, prev.tan_y), prev_fwd: f4(prev.fwd, prev.near_cm),
            size: [s.w as f32, s.h as f32, 1. / s.w as f32, 1. / s.h as f32],
            half_size: [s.half[0] as f32, s.half[1] as f32, 1. / s.half[0].max(1) as f32, 1. / s.half[1].max(1) as f32],
            grid: [s.grid[0] as f32, s.grid[1] as f32, 1. / s.grid[0].max(1) as f32, 1. / s.grid[1].max(1) as f32],
            tiles: [s.tiles[0] as f32, s.tiles[1] as f32, view_list.len() as f32, ue::MAX_OBJECTS_PER_CULL_TILE as f32],
            jitter: [jit[0] * ue::CONE_TRACE_DOWNSAMPLE_FACTOR as f32, jit[1] * ue::CONE_TRACE_DOWNSAMPLE_FACTOR as f32, if self.history_valid { 1. } else { 0. }, if ue::HISTORY_STABILITY_PASS { 1. } else { 0. }],
            ao_a: [obj_max, step_scale(), ue::STEP_EXPONENT_SCALE, max_view],
            ao_b: [global_max, cone_half_angle().tan(), bent_normal_normalize_factor(), 1. / ((1. - ue::VIEW_FADE_DISTANCE_SCALE) * max_view)],
            hist: [ue::HISTORY_WEIGHT, ue::HISTORY_MIN_CONFIDENCE_SCALE, ue::HISTORY_DISTANCE_THRESHOLD, 0.],
            depth: [cam.near, cam.far, 0., 0.],
            clip,
            clip_misc: [res as f32, 1. / res as f32, radius, if self.clip_state.iter().all(|c| c.initialized) && global_max > 0. && fn_live::var_os("FN_DEBUG_DFAO_NOGLOBAL").is_none() /* PERF: debug-only A/B */ { 1. } else { 0. }],
            cones,
            clip_full,
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&au));
        let key = (inp.resource_gen, self.sized_gen);
        if rebind || key != self.bg_key || self.bgs.is_none() { self.build_bind_groups(device, inp); self.bg_key = key; }
        self.pending_cam = Some(cb);
        resized
    }

    /// GDF PARTIAL UPDATES (2026-10-06): this frame's update regions, each `CullObjectsToGridCS` over DivideAndRoundUp(CellsSize, 16) grid cells, then
    /// `CompositeObjectDistanceFieldsCS` over DivideAndRoundUp(CellsSize, 4) groups (GlobalDistanceField.cpp:1288-1350: per region, cull then composite).
    fn encode_gdf_regions(&self, pass: &mut wgpu::ComputePass<'_>, p: &Pipes, b: &BindGroups) {
        for (i, (_, r, _)) in self.regions.iter().enumerate() {
            let off = (i * REGION_STRIDE) as u32;
            let gd = (r.cells + 15) / ue::CULL_GRID_TILE_SIZE as i32;
            pass.set_pipeline(&p.gdf_cull);
            pass.set_bind_group(0, &b.gdf_cull, &[off]);
            pass.dispatch_workgroups(gd.x as u32, gd.y as u32, gd.z as u32);
            let g = (r.cells + 3) / 4;
            pass.set_pipeline(&p.gdf);
            pass.set_bind_group(0, &b.gdf, &[off]);
            pass.dispatch_workgroups(g.x as u32, g.y as u32, g.z as u32);
        }
    }
    /// Records the frame's DFAO dispatches (after the scene pass wrote the depth) and copies the new history into the scene AO texture.
    /// PERF (FPS investigation 2026-10-06, debug only: `FN_DFAO_SPLIT=1`): the same dispatches as [`Self::encode`], each stage in its own compute pass with the
    /// timestamps `ts(stage name)` gives, to see which stage costs what.
    pub fn encode_split<'a>(&self, enc: &mut wgpu::CommandEncoder, ts: &dyn Fn(&str) -> Option<wgpu::ComputePassTimestampWrites<'a>>) {
        if !self.active { return; }
        let (Some(p), Some(b), Some(s)) = (self.pipes.as_ref(), self.bgs.as_ref(), self.sized.as_ref()) else { return };
        if !self.regions.is_empty() {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("dfao gdf"), timestamp_writes: ts("dfao gdf update") });
            self.encode_gdf_regions(&mut pass, p, b);
        }
        let d8 = |x: u32| x.div_ceil(8).max(1);
        let stages: [(&str, &wgpu::ComputePipeline, &wgpu::BindGroup, [u32; 2]); 7] = [
            ("dfao normal", &p.normal, &b.normal, [d8(s.half[0]), d8(s.half[1])]),
            ("dfao tile cones", &p.tile_cones, &b.tile_cones, [s.tiles[0], s.tiles[1]]),
            ("dfao tile cull", &p.tile_cull, &b.tile_cull, [s.tiles[0], s.tiles[1]]),
            ("dfao cone trace", &p.cone_trace, &b.cone, [d8(s.grid[0]), d8(s.grid[1])]),
            ("dfao upsample", &p.upsample, &b.upsample, [d8(s.half[0]), d8(s.half[1])]),
            ("dfao history", &p.history, &b.history[self.parity], [d8(s.half[0]), d8(s.half[1])]),
            ("dfao filter", &p.filter, &b.filter[self.parity], [d8(s.half[0]), d8(s.half[1])]),
        ];
        for (name, pipe, bg, [x, y]) in stages {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(name), timestamp_writes: ts(name) });
            pass.set_pipeline(pipe); pass.set_bind_group(0, bg, &[]); pass.dispatch_workgroups(x, y, 1);
        }
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &s.state[1 - self.parity], mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &self.scene_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: s.half[0], height: s.half[1], depth_or_array_layers: 1 },
        );
    }
    pub fn encode(&self, enc: &mut wgpu::CommandEncoder, timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>) {
        if !self.active { return; }
        let (Some(p), Some(b), Some(s)) = (self.pipes.as_ref(), self.bgs.as_ref(), self.sized.as_ref()) else { return };
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("dfao"), timestamp_writes: timestamps });
            self.encode_gdf_regions(&mut pass, p, b);
            let d8 = |x: u32| x.div_ceil(8).max(1);
            pass.set_pipeline(&p.normal);
            pass.set_bind_group(0, &b.normal, &[]);
            pass.dispatch_workgroups(d8(s.half[0]), d8(s.half[1]), 1);
            pass.set_pipeline(&p.tile_cones);
            pass.set_bind_group(0, &b.tile_cones, &[]);
            pass.dispatch_workgroups(s.tiles[0], s.tiles[1], 1);
            pass.set_pipeline(&p.tile_cull);
            pass.set_bind_group(0, &b.tile_cull, &[]);
            pass.dispatch_workgroups(s.tiles[0], s.tiles[1], 1);
            pass.set_pipeline(&p.cone_trace);
            pass.set_bind_group(0, &b.cone, &[]);
            pass.dispatch_workgroups(d8(s.grid[0]), d8(s.grid[1]), 1);
            pass.set_pipeline(&p.upsample);
            pass.set_bind_group(0, &b.upsample, &[]);
            pass.dispatch_workgroups(d8(s.half[0]), d8(s.half[1]), 1);
            pass.set_pipeline(&p.history);
            pass.set_bind_group(0, &b.history[self.parity], &[]);
            pass.dispatch_workgroups(d8(s.half[0]), d8(s.half[1]), 1);
            pass.set_pipeline(&p.filter);
            pass.set_bind_group(0, &b.filter[self.parity], &[]);
            pass.dispatch_workgroups(d8(s.half[0]), d8(s.half[1]), 1);
        }
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: &s.state[1 - self.parity], mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &self.scene_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: s.half[0], height: s.half[1], depth_or_array_layers: 1 },
        );
    }

    /// After the frame was submitted: the history written this frame becomes next frame's input.
    pub fn end_frame(&mut self) {
        if !self.active { return; }
        self.parity = 1 - self.parity;
        self.history_valid = true;
        self.temporal_index = (self.temporal_index + 1) % 4;
        self.scene_cam = self.pending_cam.take();
    }
    pub fn active(&self) -> bool { self.active }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_resets_on_large_camera_movement_like_prev_transforms_reset() {
        // HISTORY RESET (2026-10-05): IsLargeCameraMovement(45 deg, 10000 cm), SceneVisibility.cpp:2024 / 2294
        let cam = |eye: Vec3, yaw_deg: f32| {
            let (s, c) = yaw_deg.to_radians().sin_cos();
            let fwd = Vec3::new(s, 0., -c);
            CamBasis { eye, right: fwd.cross(Vec3::Y).normalize(), up: Vec3::Y, fwd, tan_x: 1., tan_y: 1., near_cm: 10., view_proj: Mat4::IDENTITY, eye_m: eye * 0.01, fwd_m: fwd }
        };
        let a = cam(Vec3::ZERO, 0.);
        let large = |b: &CamBasis| is_large_camera_movement(b, &a, RESET_ROTATION_DEG, RESET_TRANSLATION_CM);
        assert!(!large(&cam(Vec3::new(500., 0., 300.), 30.)), "walking + a 30 deg turn keeps the history");
        assert!(large(&cam(Vec3::ZERO, 50.)), "a > 45 deg turn in one frame resets it");
        assert!(large(&cam(Vec3::new(10001., 0., 0.), 0.)), "a > 100 m teleport resets it");
        assert!(!large(&cam(Vec3::new(9999., 0., 0.), 0.)));
    }
    #[test]
    fn derived_constants_follow_the_ue_formulas() {
        // ObjectMaxOcclusionDistance = min(600, r.AOGlobalDFStartDistance 100); the global trace covers up to 600 cm
        assert_eq!(occlusion_distances(), (100., 600.));
        // the object trace ends where the global trace starts: GetStepOffset(NUM_CONE_STEPS) = AOStepScale * ((0.5 * 10)^2 + 1)
        let end = step_scale() * (25. + 1.);
        assert!((end - 100. / (1. + cone_half_angle().tan()) * 26. / 2f32.powf(4.5)).abs() < 1e-3);
        // cone half angle: each of 9 cones covers 1/9 of the hemisphere
        assert!((cone_half_angle() - (8f32 / 9.).acos()).abs() < 1e-6);
        // clipmaps: 25 m, 50 m, 100 m, 200 m half extents
        assert_eq!((0..4).map(clipmap_extent).collect::<Vec<_>>(), vec![2500., 5000., 10000., 20000.]);
        // the bent normal of an unoccluded hemisphere has unit length
        let s: Vec3 = ue::RELAXED_SPACED_VECTORS9.iter().map(|v| Vec3::from(*v)).sum();
        assert!(((s / 9.).length() * bent_normal_normalize_factor() - 1.).abs() < 1e-5);
        // the contrast curve with Contrast 0.01 is (almost) the identity on [0, 1], and exactly maps 0 -> 0, 1 -> 1
        let [c, m, a] = contrast_mul_add();
        let f = |x: f32| (1. / (1. + (-c * (x * 10. - 5.)).exp())) * m + a;
        assert!(f(0.).abs() < 1e-5 && (f(1.) - 1.).abs() < 1e-5 && (f(0.5) - 0.5).abs() < 1e-3);
    }
    #[test]
    fn occlusion_tint_is_quantised_like_an_fcolor() {
        // Evening plateau (20.5 h): 0.005 linear -> sRGB byte 15 (floor(0.0611 * 255.999)) -> 0.00478 linear
        let (t, m) = sky_occlusion_at(20.5);
        assert!((t[0] - srgb_byte_to_linear(15)).abs() < 1e-6, "{t:?}");
        assert_eq!(m, 0.);
    }
}
