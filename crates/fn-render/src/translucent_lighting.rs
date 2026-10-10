//! TRANSLUCENCY LIGHTING VOLUME (MIRRORFIX 2026-10-05): the two-cascade lighting volume UE4.16 builds every frame for translucent lit materials
//! (`TranslucencyLightingMode` `TLM_Volumetric*`). The cooked base-pass pixel programs of those materials (the edit tiles `MGreen`, `MPreviewPulse`,
//! `M_Blueprint_StairsBaseShader`, all `TLM_VolumetricNonDirectional`) sample it themselves (`GetTranslucencyVolumeLighting`, BasePassPixelShader.usf:117);
//! this module only produces the volume and the `View` constants that locate it:
//!
//! * bounds: `FViewInfo::CalcTranslucencyLightingVolumeBounds` (TranslucentLighting.cpp:109) -> `View.TranslucencyLightingVolumeMin / InvSize`
//!   (SceneRendering.cpp:763-769), [`volume_bounds`] / [`view_constants`], shared with `fn_fx::view` so both sides agree bit for bit;
//! * clear (`ClearTranslucentVolumeLighting`, :942), directional light injection (`InjectTranslucentVolumeLighting`, :1578 / `InjectMainPS`,
//!   TranslucentLightInjectionShaders.usf:109): for a whole-scene-shadowed sun once per CSM cascade with that cascade's clip planes, transition masking and
//!   shadow map (`RenderShadowProjections`, ShadowRendering.cpp:1411-1428, `FVolumeShadowingParameters::Set`, VolumeLighting.h:37, `ComputeVolumeShadowing`,
//!   VolumeLightingCommon.usf), otherwise once unshadowed (LightRendering.cpp:657-661); the light function (`GetLightFunctionShadowFactor`) multiplies it;
//! * `InjectAmbientCubemapTranslucentVolumeLighting` (:1134) only runs with post-process ambient cubemaps (`ContributingCubemaps`): Athena has none (no entry
//!   in the port's post-process data), so nothing is added; the SKY light is not injected in UE4.16 either: the programs evaluate `GetSkySHDiffuse` *
//!   `View.SkyLightColor` per pixel (BasePassPixelShader.usf:255-283, the View constants `fn_fx::view` already fills);
//! * `FilterTranslucentVolumeLighting` (:1770 / `FilterMainPS`, TranslucentLightingShaders.usf:93) with `r.TranslucencyVolumeBlur`.
//!
//! Resolution / blur: `r.TranslucencyLightingVolumeDim` / `r.TranslucencyVolumeBlur` of `[EffectsQuality@N]` in the 1.8 cooked Scalability
//! (`CookedIniVersion.txt`: 24 / 32 / 48 / 64, blur 0 / 0 / 1 / 1); `sg.EffectsQuality` defaults to 3 (exe, reports/exe-defaults.json) = 64^3, blurred.
//! Cascades: `r.TranslucencyLightingVolumeInnerDistance` 1500 cm / `OuterDistance` 5000 cm, `r.TranslucentVolumeMinFOV` 45, `FOVSnapFactor` 10 (exe defaults).
//! Format PF_FloatRGBA (`FSceneRenderTargets` volume targets) = Rgba16Float. Compute instead of the slice-rasterising VS/GS/PS: one thread per voxel at the
//! same voxel position / UV the `FWriteToSliceVS` quad gives (`Input.Vertex.UV` = texel centre, layer centre), additive cascade blending summed in-thread.
use crate::lighting::{Cascade, CASCADE_COUNT};
use glam::Vec3;

pub const TVC_MAX: usize = 2;
/// `r.TranslucencyLightingVolumeInnerDistance` / `OuterDistance` (TranslucentLighting.cpp:97-107; 1.8 exe defaults 1500 / 5000).
pub const INNER_DISTANCE_CM: f32 = 1500.0;
pub const OUTER_DISTANCE_CM: f32 = 5000.0;
/// `r.TranslucentVolumeMinFOV` (45) and `r.TranslucentVolumeFOVSnapFactor` (10), degrees (1.8 exe defaults).
pub const MIN_FOV_DEG: f32 = 45.0;
pub const FOV_SNAP_DEG: f32 = 10.0;
pub const VOLUME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// `r.TranslucencyLightingVolumeDim` + `r.TranslucencyVolumeBlur`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeSettings { pub dim: u32, pub blur: bool }
impl VolumeSettings {
    /// `[EffectsQuality@level]` of the 1.8 cooked Scalability (CookedIniVersion.txt:5284-5345).
    pub fn for_effects_quality(level: u32) -> VolumeSettings {
        match level { 0 => VolumeSettings { dim: 24, blur: false }, 1 => VolumeSettings { dim: 32, blur: false }, 2 => VolumeSettings { dim: 48, blur: true }, _ => VolumeSettings { dim: 64, blur: true } }
    }
    /// `sg.EffectsQuality` 3 (the exe default).
    pub const DEFAULT: VolumeSettings = VolumeSettings { dim: 64, blur: true };
    /// The settings in effect (one process-wide value, like the cvars: the renderer builds the volume with it and `fn_fx::view` places it with it).
    pub fn current() -> VolumeSettings { let v = CURRENT.load(std::sync::atomic::Ordering::Relaxed); VolumeSettings { dim: v & 0xffff, blur: v >> 16 != 0 } }
    /// Sets `r.TranslucencyLightingVolumeDim` / `r.TranslucencyVolumeBlur` (e.g. from [`Self::for_effects_quality`] of the scalability level).
    pub fn set_current(s: VolumeSettings) { CURRENT.store(s.dim.min(0xffff) | ((s.blur as u32) << 16), std::sync::atomic::Ordering::Relaxed); }
}
static CURRENT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(64 | (1 << 16));

/// One cascade's box (UE world cm): `FBox(Center - W, Center + W)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CascadeBox { pub min: [f32; 3], pub max: [f32; 3] }

/// The port camera (metres, Y up) in UE axes (cm, Z up): origin, view direction, right, up, as `fn_fx::view::ViewParams::clip` builds them.
fn ue_basis(eye_m: Vec3, forward: Vec3, up: Vec3) -> (Vec3, Vec3, Vec3, Vec3) {
    let to = |v: Vec3| Vec3::new(v.x, v.z, v.y);
    let dir = to(forward).normalize();
    let up0 = to(up).normalize();
    let right = up0.cross(dir).normalize();
    let up = dir.cross(right).normalize();
    (to(eye_m) * 100.0, dir, right, up)
}

/// `FViewInfo::CalcTranslucencyLightingVolumeBounds` (TranslucentLighting.cpp:109-183) for the camera `eye_m` / `forward` / `up` (port metres / axes) with the
/// projection `tan_half` (= 1 / ProjectionMatrix.M[0][0], 1 / M[1][1]); float maths like the engine.
pub fn volume_bounds(eye_m: Vec3, forward: Vec3, up: Vec3, tan_half: (f32, f32), dim: u32) -> [CascadeBox; TVC_MAX] {
    let (origin, dir, right, upv) = ue_basis(eye_m, forward, up);
    // FOV = atan(1 / M00), clamped to the min FOV and rounded UP to the snap factor; AspectRatio = M11 / M00
    let snap = FOV_SNAP_DEG.to_radians();
    let fov = tan_half.0.atan().max(MIN_FOV_DEG.to_radians());
    let fov = fov + snap - fov % snap;
    let aspect = tan_half.0 / tan_half.1;
    std::array::from_fn(|c| {
        let (start, end) = if c == 0 { (0.0, INNER_DISTANCE_CM) } else { (INNER_DISTANCE_CM, OUTER_DISTANCE_CM) };
        let (sh, eh) = (start * fov.tan(), end * fov.tan());
        let (sr, su, er, eu) = (right * sh, upv * (sh / aspect), right * eh, upv * (eh / aspect));
        let (sc, ec) = (origin + dir * start, origin + dir * end);
        let v = [sc + sr + su, sc + sr - su, sc - sr + su, sc - sr - su, ec + er + eu, ec + er - eu, ec - er + eu, ec - er - eu];
        // far vertices weighted more (FarVertexWeightScale 10)
        let mut center = Vec3::ZERO;
        for (i, p) in v.iter().enumerate() { let w = if i > 3 { 1.0 / (4.0 + 4.0 / 10.0) } else { 1.0 / (4.0 + 4.0 * 10.0) }; center += *p * w; }
        let r = v.iter().map(|p| (center - *p).length_squared()).fold(0f32, f32::max).sqrt();
        // snap the centre to a multiple of the voxel size (FMath::Fmod = truncating remainder)
        let step = r * 2.0 / dim as f32;
        let c3 = Vec3::new(center.x - center.x % step, center.y - center.y % step, center.z - center.z % step);
        CascadeBox { min: (c3 - Vec3::splat(r)).to_array(), max: (c3 + Vec3::splat(r)).to_array() }
    })
}

/// `View.TranslucencyLightingVolumeMin[2]` then `View.TranslucencyLightingVolumeInvSize[2]` (SceneRendering.cpp:765-768): `(Min, 1 / Dim)`,
/// `(1 / (Max - Min), VoxelSize = (Max.X - Min.X) / Dim)`.
pub fn view_constants(b: &[CascadeBox; TVC_MAX], dim: u32) -> [[f32; 4]; 4] {
    let mn = |c: usize| [b[c].min[0], b[c].min[1], b[c].min[2], 1.0 / dim as f32];
    let inv = |c: usize| { let s = [b[c].max[0] - b[c].min[0], b[c].max[1] - b[c].min[1], b[c].max[2] - b[c].min[2]]; [1.0 / s[0], 1.0 / s[1], 1.0 / s[2], s[0] / dim as f32] };
    [mn(0), mn(1), inv(0), inv(1)]
}

/// `View.DirectionalLightColor` / `DirectionalLightDirection` (SceneRendering.cpp:641-650): the scene's `SimpleDirectionalLight` (the movable Athena sun):
/// `Proxy->GetColor() / PI` (rgb = `lighting::sun_radiance`, which already carries the 1 / pi; alpha = Intensity / pi, `FLinearColor * Brightness` scales alpha
/// too) and `-Proxy->GetDirection()` (towards the light, UE axes).
pub fn view_directional_light() -> ([f32; 4], [f32; 4]) {
    let st = crate::time_of_day::current();
    let c = crate::lighting::sun_radiance_of(&st);
    let d = crate::lighting::sun_direction();
    ([c[0], c[1], c[2], st.sun_brightness / std::f32::consts::PI], [d.x, d.z, d.y, 0.0])
}

/// LIVE DUMP (2026-10-07, MATCH track): `bAffectTranslucentLighting` is FALSE on the Athena `DayNightDirectionalLightComponent` (and its SkyLight) in the real
/// 1.8 client (C:FortniteReflighting_refmatchlightdump_00*_*.json; ULightComponent ctor default true, LightComponent.cpp:344). UE4.16
/// `AddLightForInjection` (TranslucentLighting.cpp:1451) then adds nothing: the volume stays at its clear value (0), only the clear + filter run. Was: the sun injected.
pub const SUN_AFFECTS_TRANSLUCENT_LIGHTING: bool = false;

/// Per-frame inputs of the injection.
pub struct VolumeFrame<'a> {
    /// The UNJITTERED window camera (what the app's FX `View` buffers are built from).
    pub eye: Vec3, pub forward: Vec3, pub up: Vec3, pub tan_half: (f32, f32), pub near_m: f32,
    pub cascades: &'a [Cascade; CASCADE_COUNT],
    /// Shadow-mapped cascades (0 = the sun is injected unshadowed).
    pub num_shadowed: usize,
    /// `CascadeTransitionFraction * TransitionScale` of the CSM setup.
    pub transition_fraction: f32,
    /// `GetDirectionalLightDistanceFadeParameters` far distance (metres) and `ShadowDistanceFadeoutFraction`.
    pub fade_far_m: f32, pub fade_fraction: f32,
    /// The light function's shadow factor (`dot(emissive, 0.3333)` of the sun's light function; 1 when light functions are off).
    pub light_function: f32,
    /// The sun's `bAffectTranslucentLighting` ([`SUN_AFFECTS_TRANSLUCENT_LIGHTING`] in the renderer): false = `AddLightForInjection` skips it.
    pub affects_translucent: bool,
    pub shadow_size: u32,
    pub shadow_view: &'a wgpu::TextureView,
    pub light_buf: &'a wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TlvUniform {
    vmin: [[f32; 4]; 2],
    vinv: [[f32; 4]; 2],
    view_origin: [f32; 4],
    view_forward: [f32; 4],
    /// xyz towards the light (UE), w light function factor
    light_dir: [f32; 4],
    /// rgb `LightColor / PI`, w unused
    light_color: [f32; 4],
    /// per CSM cascade: SplitNear (cm)
    split_near: [f32; 4],
    /// per CSM cascade: SplitFar incl. the fade extension (cm)
    split_far: [f32; 4],
    /// ShadowInjectParams.x / .y per cascade
    near_fade: [f32; 4],
    far_fade: [f32; 4],
    /// 1 = the cascade has a far clipping plane (not the last one)
    far_plane: [f32; 4],
    /// x shadowed cascades (0 = unshadowed injection), y dim, z / w ShadowInjectParams.zw (distance fade MAD)
    misc: [f32; 4],
    /// x shadow map size, y SHADOW_BORDER, zw unused
    shadow: [f32; 4],
}

struct Volume { dim: u32, finals: Vec<wgpu::TextureView>, scratch: Vec<wgpu::TextureView>, #[allow(dead_code)] final_textures: Vec<wgpu::Texture> }

pub struct TranslucentLighting {
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bgl_inject: wgpu::BindGroupLayout,
    bgl_filter: wgpu::BindGroupLayout,
    inject: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    volume: Option<Volume>,
    groups: Option<(wgpu::BindGroup, Option<wgpu::BindGroup>)>,
    active: bool,
}

fn storage(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::StorageTexture { access: wgpu::StorageTextureAccess::WriteOnly, format: VOLUME_FORMAT, view_dimension: wgpu::TextureViewDimension::D3 }, count: None }
}
fn uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }
}

impl TranslucentLighting {
    pub fn new(device: &wgpu::Device) -> TranslucentLighting {
        let sm = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("translucency lighting volume"), source: wgpu::ShaderSource::Wgsl(fn_live::wgsl!("../shaders/translucent_lighting.wgsl").into()) });
        let bgl_inject = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("tlv inject"), entries: &[
            uniform(0), uniform(1),
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
            storage(3), storage(4), storage(5), storage(6),
        ] });
        let tex = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }, count: None };
        let bgl_filter = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("tlv filter"), entries: &[
            uniform(0),
            wgpu::BindGroupLayoutEntry { binding: 11, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            tex(7), tex(8), tex(9), tex(10),
            storage(3), storage(4), storage(5), storage(6),
        ] });
        let cp = |label: &str, entry: &str, bgl: &wgpu::BindGroupLayout| {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(bgl)], immediate_size: 0 });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(label), layout: Some(&pl), module: &sm, entry_point: Some(entry), compilation_options: Default::default(), cache: None })
        };
        let inject = cp("tlv inject", "inject", &bgl_inject);
        let filter = cp("tlv filter", "filter_volume", &bgl_filter);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("tlv uniform"), size: std::mem::size_of::<TlvUniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        // FFilterTranslucentVolumePS: TStaticSamplerState<SF_Bilinear, AM_Clamp, AM_Clamp, AM_Clamp> (TranslucentLighting.cpp:645)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("tlv filter sampler"), address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, address_mode_w: wgpu::AddressMode::ClampToEdge, mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        TranslucentLighting { uniform, sampler, bgl_inject, bgl_filter, inject, filter, volume: None, groups: None, active: false }
    }

    pub fn active(&self) -> bool { self.active }

    /// Builds this frame's uniforms / bind groups; (re)creates the volume textures when the size changed and hands them to the FX pass. `needed` = the frame
    /// has translucent FX draws (nothing reads the volume otherwise).
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, f: &VolumeFrame, fx: &mut crate::fx::FxRenderer, needed: bool) {
        let settings = VolumeSettings::current();
        let dim = settings.dim.max(1);
        if self.volume.as_ref().map_or(true, |v| v.dim != dim) {
            let mk = |label: &str| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: dim, height: dim, depth_or_array_layers: dim }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3, format: VOLUME_FORMAT, usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
            // [AmbientInner, AmbientOuter, DirectionalInner, DirectionalOuter] (fx::TLV_* order)
            let final_textures: Vec<wgpu::Texture> = ["tlv ambient inner", "tlv ambient outer", "tlv directional inner", "tlv directional outer"].iter().map(|l| mk(l)).collect();
            let finals: Vec<wgpu::TextureView> = final_textures.iter().map(|t| t.create_view(&Default::default())).collect();
            let scratch: Vec<wgpu::TextureView> = ["tlv ambient inner (injected)", "tlv ambient outer (injected)", "tlv directional inner (injected)", "tlv directional outer (injected)"].iter().map(|l| mk(l).create_view(&Default::default())).collect();
            fx.set_translucency_volume([finals[0].clone(), finals[1].clone(), finals[2].clone(), finals[3].clone()]);
            self.volume = Some(Volume { dim, finals, scratch, final_textures });
        }
        self.active = needed;
        if !needed { return; }
        let bounds = volume_bounds(f.eye, f.forward, f.up, f.tan_half, dim);
        let vc = view_constants(&bounds, dim);
        let (origin, dir, _, _) = ue_basis(f.eye, f.forward, f.up);
        let (color, to_light) = view_directional_light();
        let n = f.num_shadowed.min(CASCADE_COUNT);
        let (mut split_near, mut split_far, mut near_fade, mut far_fade, mut far_plane) = ([0f32; 4], [0f32; 4], [1f32; 4], [1f32; 4], [0f32; 4]);
        for c in 0..n {
            // GetShadowSplitBounds (DirectionalLightComponent.cpp:615-666): SplitNear = the previous split end, SplitFar += FadeExtension except the last
            // cascade's plane, SplitNearFadeRegion = the previous cascade's (SplitFar - SplitNear) * TransitionFraction; FVolumeShadowingParameters::Set
            // (VolumeLighting.h:60-86): x = 1 / SplitNearFadeRegion (1 when 0), y = 1 / SplitFarFadeRegion, the far plane only below CascadeCount - 1
            let near_m = if c == 0 { f.near_m } else { f.cascades[c - 1].far };
            split_near[c] = near_m * 100.0;
            if c >= 1 { let prev_near = if c == 1 { f.near_m } else { f.cascades[c - 2].far }; let region = (f.cascades[c - 1].far - prev_near) * f.transition_fraction * 100.0; near_fade[c] = if region == 0.0 { 1.0 } else { 1.0 / region }; }
            if c + 1 < n {
                let ext = (f.cascades[c].far - near_m) * f.transition_fraction * 100.0;
                split_far[c] = f.cascades[c].far * 100.0 + ext;
                far_fade[c] = 1.0 / ext.max(1e-6);
                far_plane[c] = 1.0;
            }
        }
        // GetDirectionalLightDistanceFadeParameters (DirectionalLightComponent.cpp:308): Near = Far - Far * ShadowDistanceFadeoutFraction; z = 1 / (Far - Near), w = -Near * z
        let far_cm = f.fade_far_m * 100.0;
        let near_cm = far_cm - far_cm * f.fade_fraction;
        let fz = 1.0 / (far_cm - near_cm).max(1e-4);
        let u = TlvUniform {
            vmin: [vc[0], vc[1]], vinv: [vc[2], vc[3]],
            view_origin: [origin.x, origin.y, origin.z, 0.0], view_forward: [dir.x, dir.y, dir.z, 0.0],
            light_dir: [to_light[0], to_light[1], to_light[2], f.light_function],
            light_color: if f.affects_translucent { [color[0], color[1], color[2], 0.0] } else { [0.0; 4] }, // AddLightForInjection gate
            split_near, split_far, near_fade, far_fade, far_plane,
            misc: [n as f32, dim as f32, fz, -near_cm * fz],
            shadow: [f.shadow_size as f32, crate::lighting::SHADOW_BORDER as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&u));
        let v = self.volume.as_ref().unwrap();
        let blur = settings.blur;
        // r.TranslucencyVolumeBlur: inject into the scratch set and filter into the final one; without blur the final set is injected directly
        let out = if blur { &v.scratch } else { &v.finals };
        fn bt(b: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> { wgpu::BindGroupEntry { binding: b, resource: wgpu::BindingResource::TextureView(view) } }
        // storage bindings 3..6 = AmbientInner, DirectionalInner, AmbientOuter, DirectionalOuter (the shader's per-cascade pairs)
        let inject = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("tlv inject"), layout: &self.bgl_inject, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: f.light_buf.as_entire_binding() },
            bt(2, f.shadow_view), bt(3, &out[0]), bt(4, &out[2]), bt(5, &out[1]), bt(6, &out[3]),
        ] });
        let filter = blur.then(|| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("tlv filter"), layout: &self.bgl_filter, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() }, wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            bt(7, &v.scratch[0]), bt(8, &v.scratch[2]), bt(9, &v.scratch[1]), bt(10, &v.scratch[3]),
            bt(3, &v.finals[0]), bt(4, &v.finals[2]), bt(5, &v.finals[1]), bt(6, &v.finals[3]),
        ] }));
        self.groups = Some((inject, filter));
    }

    /// Clear + inject (+ filter): one compute pass, two dispatches (wgpu scopes storage / sampled usage per dispatch).
    pub fn encode(&self, pass: &mut wgpu::ComputePass<'_>) {
        let (Some(v), Some((inject, filter))) = (self.volume.as_ref(), self.groups.as_ref()) else { return };
        if !self.active { return; }
        let g = v.dim.div_ceil(4);
        pass.set_pipeline(&self.inject);
        pass.set_bind_group(0, inject, &[]);
        pass.dispatch_workgroups(g, g, (2 * v.dim).div_ceil(4));
        if let Some(fbg) = filter {
            pass.set_pipeline(&self.filter);
            pass.set_bind_group(0, fbg, &[]);
            pass.dispatch_workgroups(g, g, (2 * v.dim).div_ceil(4));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The cascade boxes follow the engine maths: the inner box holds the camera, the outer one reaches 50 m ahead; centres snapped to the voxel grid;
    /// View constants = (Min, 1 / Dim) and (1 / Size, VoxelSize).
    #[test] fn bounds_follow_calc_translucency_lighting_volume_bounds() {
        let b = volume_bounds(Vec3::new(1.0, 2.0, 3.0), Vec3::new(0.0, 0.0, -1.0), Vec3::Y, (1.0, 0.5625), 64);
        let eye_ue = Vec3::new(100.0, 300.0, 200.0);
        for c in 0..2 {
            let (mn, mx) = (Vec3::from(b[c].min), Vec3::from(b[c].max));
            let size = mx - mn;
            assert!((size.x - size.y).abs() < 1e-2 && (size.x - size.z).abs() < 1e-2, "cube: {size:?}");
            let step = size.x / 64.0;
            let centre = (mn + mx) * 0.5;
            for k in 0..3 { let r = centre[k] % step; assert!(r.abs() < 1e-2 * step || (step - r.abs()) < 1e-2 * step, "snapped centre {centre:?} step {step}"); }
            if c == 0 { assert!(mn.cmple(eye_ue).all() && mx.cmpge(eye_ue).all(), "inner box contains the eye"); }
        }
        // the far cascade reaches the 5000 cm end of its frustum straight ahead (forward = -Z port = -Y UE)
        assert!(b[1].min[1] <= eye_ue.y - 4999.0);
        let vc = view_constants(&b, 64);
        assert_eq!(vc[0][3], 1.0 / 64.0);
        assert!((vc[2][0] * (b[0].max[0] - b[0].min[0]) - 1.0).abs() < 1e-5 && (vc[2][3] - (b[0].max[0] - b[0].min[0]) / 64.0).abs() < 1e-3);
        // FOV clamp: a 30 deg half FOV is raised to 45 then rounded up to 50
        let narrow = volume_bounds(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), Vec3::Y, (30f32.to_radians().tan(), 30f32.to_radians().tan()), 64);
        let wide = volume_bounds(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), Vec3::Y, (49f32.to_radians().tan(), 49f32.to_radians().tan()), 64);
        for c in 0..2 { for k in 0..3 { let step = (wide[c].max[0] - wide[c].min[0]) / 64.0; assert!((narrow[c].min[k] - wide[c].min[k]).abs() <= step * 1.01, "{narrow:?} vs {wide:?}"); } }
    }
    fn f16(h: u16) -> f32 {
        let (s, e, m) = ((h >> 15) as u32, ((h >> 10) & 31) as i32, (h & 1023) as f32);
        let v = if e == 0 { m * 2f32.powi(-24) } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) };
        if s == 1 { -v } else { v }
    }
    fn read(g: &crate::Gpu, tex: &wgpu::Texture, dim: u32) -> Vec<[f32; 4]> {
        let bpr = (dim * 8).div_ceil(256) * 256;
        let buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: (bpr * dim * dim) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
        let mut enc = g.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(dim) } }, wgpu::Extent3d { width: dim, height: dim, depth_or_array_layers: dim });
        g.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();
        let d = slice.get_mapped_range().unwrap();
        let mut out = vec![];
        for z in 0..dim { for y in 0..dim { for x in 0..dim { let o = ((z * dim + y) * bpr + x * 8) as usize; out.push(std::array::from_fn(|i| f16(u16::from_le_bytes([d[o + 2 * i], d[o + 2 * i + 1]])))); } } }
        out
    }
    /// GPU: the unshadowed injection fills the volume with the directional light's SH (UE `InjectMainPS` without a shadow: Attenuation 1, ShadowFactor 1 in alpha);
    /// a CSM that shadows everything zeroes the in-front voxels' shadow term, a lit CSM keeps it 1 (cascade transitions sum to 1).
    #[test] fn gpu_injection_unshadowed_and_shadowed() {
        let Some(g) = crate::Gpu::headless().ok() else { eprintln!("no GPU adapter: skipped"); return };
        let dim = 8;
        VolumeSettings::set_current(VolumeSettings { dim, blur: true });
        let mut fx = crate::fx::FxRenderer::new(&g.device, &g.queue, wgpu::TextureFormat::Rgba16Float, wgpu::TextureFormat::Depth32Float);
        let mut tl = TranslucentLighting::new(&g.device);
        // light block: every cascade maps any point to uv 0.5, depth 0.5; no bias
        let mut lb = vec![0f32; 88];
        for c in 0..3 { let m = glam::Mat4::from_translation(Vec3::splat(0.5)) * glam::Mat4::from_scale(Vec3::ZERO); lb[12 + c * 16..28 + c * 16].copy_from_slice(&m.to_cols_array()); }
        let light_buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 352, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        g.queue.write_buffer(&light_buf, 0, bytemuck::cast_slice(&lb));
        let sh = g.device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 3 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Depth32Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let shadow_view = sh.create_view(&Default::default());
        let clear = |depth: f32| {
            let mut enc = g.device.create_command_encoder(&Default::default());
            for layer in 0..3 {
                let lv = sh.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: layer, array_layer_count: Some(1), ..Default::default() });
                let _p = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[], depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &lv, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(depth), store: wgpu::StoreOp::Store }), stencil_ops: None }), timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            }
            g.queue.submit([enc.finish()]);
        };
        let (eye, fwd, up, tan_half) = (Vec3::new(0.0, 1.7, 0.0), Vec3::new(0.0, 0.0, -1.0), Vec3::Y, (1.0f32, 0.5625f32));
        let view_inv = glam::Mat4::look_to_rh(eye, fwd, up).inverse();
        let cascades = crate::lighting::fit_cascades(view_inv, tan_half.0, tan_half.0 / tan_half.1, 0.1, Vec3::new(0.3, 0.8, -0.5).normalize(), 16);
        let run = |tl: &mut TranslucentLighting, fx: &mut crate::fx::FxRenderer, shadowed: usize| -> Vec<[f32; 4]> {
            let f = VolumeFrame { eye, forward: fwd, up, tan_half, near_m: 0.1, cascades: &cascades, num_shadowed: shadowed, transition_fraction: 0.1, fade_far_m: 45.0, fade_fraction: 0.1, light_function: 1.0, affects_translucent: true, shadow_size: 16, shadow_view: &shadow_view, light_buf: &light_buf };
            tl.prepare(&g.device, &g.queue, &f, fx, true);
            let mut enc = g.device.create_command_encoder(&Default::default());
            { let mut pass = enc.begin_compute_pass(&Default::default()); tl.encode(&mut pass); }
            g.queue.submit([enc.finish()]);
            read(&g, &tl.volume.as_ref().unwrap().final_textures[0], dim)
        };
        let (color, _) = view_directional_light();
        let v = run(&mut tl, &mut fx, 0);
        for t in &v { assert!((t[3] - 1.0).abs() < 1e-3 && (0..3).all(|i| (t[i] - color[i] * 0.282095).abs() <= 2e-3 * color[i].max(1e-3) + 1e-6), "{t:?} vs {color:?}"); }
        assert!(!SUN_AFFECTS_TRANSLUCENT_LIGHTING); // LIVE DUMP: the Athena sun is not injected (the renderer passes this flag)
        // the voxel 10 m straight ahead (cascade 1 interior: its blur neighbours are all in front of the camera)
        let b = volume_bounds(eye, fwd, up, tan_half, dim);
        let p = Vec3::new(eye.x, eye.z, eye.y) * 100.0 + Vec3::new(0.0, -1000.0, 0.0);
        let ix: [u32; 3] = std::array::from_fn(|k| (((p[k] - b[0].min[k]) / (b[0].max[k] - b[0].min[k])) * dim as f32).floor() as u32);
        let at = |v: &Vec<[f32; 4]>| v[((ix[2] * dim + ix[1]) * dim + ix[0]) as usize];
        clear(0.25);
        let dark = at(&run(&mut tl, &mut fx, 3));
        clear(1.0);
        let lit = at(&run(&mut tl, &mut fx, 3));
        VolumeSettings::set_current(VolumeSettings::DEFAULT);
        assert!(dark[3] < 0.02 && (lit[3] - 1.0).abs() < 0.02, "shadowed {dark:?} lit {lit:?} at {ix:?}");
    }
    #[test] fn effects_quality_table() {
        assert_eq!(VolumeSettings::for_effects_quality(0), VolumeSettings { dim: 24, blur: false });
        assert_eq!(VolumeSettings::for_effects_quality(2), VolumeSettings { dim: 48, blur: true });
        assert_eq!(VolumeSettings::for_effects_quality(3), VolumeSettings::DEFAULT);
    }
}
