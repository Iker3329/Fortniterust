//! SKY REFLECTION HOOK: the Athena sky light's SPECULAR term, i.e. what UE 4.16 draws for a movable sky light when the scene has no reflection captures.
//! Shared by every lit path (group 0 bindings 11..14 of the renderer's globals, evaluated by `lit_common.wgsl` `reflection_inputs`); the gun pass, the character
//! paths (`char_scene.wgsl`, `scene.wgsl`) and the static meshes use it.
//!
//! # UE 4.16 pipeline reproduced
//! * Capture (`FScene::UpdateSkyCaptureContents`, ReflectionEnvironmentCapture.cpp:1518) of the SLS_SpecifiedCubemap `T_hdrsky_test03_low`
//!   (`sky_light.rs` holds the exe / asset evidence: CubemapResolution 128, SourceCubemapAngle 0, bLowerHemisphereIsBlack 0):
//!   `CopyCubemapToScratchCubemap` -> `FilterReflectionEnvironment` (ReflectionEnvironmentCapture.cpp:302): `NumMips = CeilLogTwo(128) + 1 = 8`,
//!   `CreateCubeMips` (bilinear `DownsamplePS`, `sky_light::downsample`), then every mip of the processed cube is `FilterPS` (ReflectionEnvironmentShaders.usf:262)
//!   over the downsampled chain with a trilinear clamp sampler (ReflectionEnvironmentCapture.cpp:419): roughness of the mip
//!   `ComputeReflectionCaptureRoughnessFromMip(Mip, NumMips - 1)`, GGX importance sampling (`Hammersley`, `ImportanceSampleGGX`, `E.y *= 0.995`, NoL weights,
//!   source mip `0.5 log2(SolidAngleSample / SolidAngleTexel)`) or, for roughness > 0.99, the cosine lobe; 32 samples below roughness 0.1, else 64.
//! * Lookup (`GetSkyLightReflection`, ReflectionEnvironmentShared.usf:35): `TextureCubeSampleLevel(SkyLightCubemap, R, ComputeReflectionCaptureMipFromRoughness(Roughness,
//!   SkyLightParameters.x)) * View.SkyLightColor`, with `SkyLightParameters.x = SkyMipCount - 1 = 7` (BasePassRendering.h:587).
//! * Apply: no Athena level registers a reflection capture, so `ShouldDoReflectionEnvironment()` is false (ReflectionEnvironment.cpp:907) and the deferred path is
//!   `RenderStandardDeferredImageBasedReflections` -> `ReflectionApplyPS` with APPLY_SKYLIGHT (ReflectionEnvironmentShaders.usf:533): `EnvBRDF` (the `PreIntegratedGF`
//!   LUT, [`preintegrated_gf`]), the off-specular-peak reflection vector, the AO specular occlusion and the DFAO specular occlusion (`GetDistanceFieldAOSpecularOcclusion`,
//!   SkyLightingShared.usf:26, with `r.AOSpecularOcclusionMode = 0` from the 1.8 cooked SystemSettings, CookedIniVersion.txt:977, and `r.SkySpecularOcclusionStrength` = 1,
//!   ReflectionEnvironment.cpp:96).
//!
//! ADAPTATION (documented, not data): the filtering runs once on the CPU (the result is the same constant every capture: the sky light never re-captures, see
//! `sky_light.rs`). The bilinear taps filter seamlessly across cube faces and the scratch mips are half float, as on D3D11 (`sky_light::Cube::sample`,
//! `sky_light::round_fp16`; SKY LIGHT 2026-10-06, they used to clamp inside the face in f32).
//! Since docs/spec/reflection_gaps.md the lit passes only RECORD the AO-independent parts (`lit_common.wgsl` `reflection_inputs`); the specular occlusion
//! with GBufferAO x the screen-space AO, the screen space reflections (r.SSR.Quality 2 / 3 at Effects High / Epic) and the composite run in the deferred
//! reflection apply (reflection.rs, `ssao_pass.rs` / `ssao_apply.wgsl`), as `ReflectionApplyPS` does.
use crate::sky_light::{copy_to_scratch, cubemap_vector, downsample, Cube, CUBEMAP_RESOLUTION, LOWER_HEMISPHERE_IS_BLACK, SOURCE_CUBEMAP_ANGLE_DEG};
use glam::Vec3;
use std::f32::consts::PI;

/// `FilterReflectionEnvironment`: `NumMips = FMath::CeilLogTwo(CubemapResolution) + 1`.
pub const NUM_MIPS: usize = CUBEMAP_RESOLUTION.trailing_zeros() as usize + 1;
/// `SkyLightParameters.x` = `SkyMipCount - 1` (BasePassRendering.h:587).
pub const MAX_MIP: f32 = (NUM_MIPS - 1) as f32;
/// `REFLECTION_CAPTURE_ROUGHEST_MIP` / `REFLECTION_CAPTURE_ROUGHNESS_MIP_SCALE` (ReflectionEnvironmentShared.usf:7).
pub const ROUGHEST_MIP: f32 = 1.0;
pub const ROUGHNESS_MIP_SCALE: f32 = 1.2;
/// `r.SkySpecularOcclusionStrength` (ReflectionEnvironment.cpp:96, default 1, not set by the 1.8 cooked ini); the shader gets `1 / max(x, 0.1)` (:353).
pub const SKY_SPECULAR_OCCLUSION_STRENGTH: f32 = 1.0;
/// `r.AOSpecularOcclusionMode` of the 1.8 cook (`SystemSettings:r.AOSpecularOcclusionMode=0`, CookedIniVersion.txt:977): the DFAO specular occlusion is the
/// bent normal length (SkyLightingShared.usf:40).
pub const AO_SPECULAR_OCCLUSION_MODE: f32 = 0.0;
/// `PreintegratedGF` size (SystemTextures.cpp:305): x = NoV, y = roughness.
pub const GF_SIZE: (usize, usize) = (128, 32);

/// `ComputeReflectionCaptureMipFromRoughness` (ReflectionEnvironmentShared.usf:13).
pub fn mip_from_roughness(roughness: f32, cubemap_max_mip: f32) -> f32 { cubemap_max_mip - 1. - (ROUGHEST_MIP - ROUGHNESS_MIP_SCALE * roughness.log2()) }
/// `ComputeReflectionCaptureRoughnessFromMip` (ReflectionEnvironmentShared.usf:22).
pub fn roughness_from_mip(mip: f32, cubemap_max_mip: f32) -> f32 { ((ROUGHEST_MIP - (cubemap_max_mip - 1. - mip)) / ROUGHNESS_MIP_SCALE).exp2() }

/// `Hammersley(Index, NumSamples, 0)` (MonteCarlo.usf:24).
pub fn hammersley(i: u32, n: u32) -> [f32; 2] { [(i as f32 / n as f32).fract(), i.reverse_bits() as f32 * 2.328_306_4e-10] }
/// `GetTangentBasis` (MonteCarlo.usf:11): rows X, Y, Z.
pub fn tangent_basis(z: Vec3) -> [Vec3; 3] {
    let up = if z.z.abs() < 0.999 { Vec3::Z } else { Vec3::X };
    let x = up.cross(z).normalize();
    [x, z.cross(x), z]
}
/// `D_GGX` (BRDF.usf:92).
pub fn d_ggx(roughness: f32, noh: f32) -> f32 { let a = roughness * roughness; let a2 = a * a; let d = (noh * a2 - noh) * noh + 1.; a2 / (PI * d * d) }
/// `ImportanceSampleGGX(E, Roughness).xyz` (MonteCarlo.usf:122).
pub fn importance_sample_ggx(e: [f32; 2], roughness: f32) -> Vec3 {
    let m = roughness * roughness;
    let m2 = m * m;
    let phi = 2. * PI * e[0];
    let cos_t = ((1. - e[1]) / (1. + (m2 - 1.) * e[1])).sqrt();
    let sin_t = (1. - cos_t * cos_t).max(0.).sqrt();
    Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t)
}
/// `CosineSampleHemisphere(E).xyz` (MonteCarlo.usf:70).
pub fn cosine_sample_hemisphere(e: [f32; 2]) -> Vec3 {
    let phi = 2. * PI * e[0];
    let cos_t = e[1].sqrt();
    let sin_t = (1. - cos_t * cos_t).max(0.).sqrt();
    Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t)
}

/// A cube mip chain sampled like a trilinear clamp sampler (`SampleLevel` with a fractional level, clamped to the chain).
pub struct MipChain<'a> { pub levels: &'a [Cube] }
impl MipChain<'_> {
    pub fn sample_level(&self, d: Vec3, mip: f32) -> [f32; 3] {
        let max = (self.levels.len() - 1) as f32;
        let m = if mip.is_finite() { mip.clamp(0., max) } else { 0. };
        let (m0, f) = (m.floor() as usize, m.fract());
        let a = self.levels[m0].sample(d);
        if f <= 0. || m0 + 1 >= self.levels.len() { return a; }
        let b = self.levels[m0 + 1].sample(d);
        std::array::from_fn(|c| a[c] * (1. - f) + b[c] * f)
    }
}

/// `FilterPS` for one texel direction `cube` of mip `mip` (ReflectionEnvironmentShaders.usf:262; `NumMips` = the chain length).
pub fn filter_texel(src: &MipChain, cube: Vec3, mip: usize, num_mips: usize) -> [f32; 3] {
    let n = cube.normalize();
    let tb = tangent_basis(n);
    let roughness = roughness_from_mip(mip as f32, (num_mips - 1) as f32);
    if roughness < 0.01 { return src.sample_level(cube, 0.); }
    let cube_size = (1usize << (num_mips - 1)) as f32;
    let solid_angle_texel = 4. * PI / (6. * cube_size * cube_size) * 2.;
    let num_samples: u32 = if roughness < 0.1 { 32 } else { 64 };
    let to_world = |l: Vec3| tb[0] * l.x + tb[1] * l.y + tb[2] * l.z; // mul(L, TangentToWorld)
    let mut acc = [0f32; 3];
    if roughness > 0.99 {
        for i in 0..num_samples {
            let l = cosine_sample_hemisphere(hammersley(i, num_samples));
            let pdf = l.z / PI;
            let sas = 1. / (num_samples as f32 * pdf);
            let m = 0.5 * (sas / solid_angle_texel).log2();
            let s = src.sample_level(to_world(l), m);
            for c in 0..3 { acc[c] += s[c]; }
        }
        return acc.map(|v| v / num_samples as f32);
    }
    let mut weight = 0.;
    for i in 0..num_samples {
        let mut e = hammersley(i, num_samples);
        e[1] *= 0.995;
        let h = importance_sample_ggx(e, roughness);
        let l = 2. * h.z * h - Vec3::Z;
        let (nol, noh) = (l.z, h.z);
        if nol > 0. {
            let pdf = d_ggx(roughness, noh) * 0.25;
            let sas = 1. / (num_samples as f32 * pdf);
            let m = 0.5 * (sas / solid_angle_texel).log2();
            let s = src.sample_level(to_world(l), m);
            for c in 0..3 { acc[c] += s[c] * nol; }
            weight += nol;
        }
    }
    acc.map(|v| v / weight)
}

/// The processed sky cube: `CopyCubemapToScratchCubemap` + `FilterReflectionEnvironment`, mip 0 = `CubemapResolution`, down to 1x1.
pub fn filter_reflection_environment(src: &Cube) -> Vec<Cube> {
    let scratch = copy_to_scratch(src, CUBEMAP_RESOLUTION, SOURCE_CUBEMAP_ANGLE_DEG.to_radians(), LOWER_HEMISPHERE_IS_BLACK);
    prefilter_chain(scratch)
}
/// `CreateCubeMips` from `top` (a power-of-two cube) and `FilterPS` of every mip.
pub fn prefilter_chain(top: Cube) -> Vec<Cube> {
    let mut chain = vec![top];
    while chain.last().unwrap().size > 1 { let d = downsample(chain.last().unwrap()); chain.push(d); }
    let num_mips = chain.len();
    let src = MipChain { levels: &chain };
    (0..num_mips).map(|mip| {
        let size = 1usize << (num_mips - mip - 1);
        let faces: [Vec<[f32; 3]>; 6] = std::thread::scope(|s| {
            let hs: Vec<_> = (0..6).map(|f| { let src = &src; s.spawn(move || {
                (0..size * size).map(|i| {
                    let uv = [((i % size) as f32 + 0.5) / size as f32 * 2. - 1., ((i / size) as f32 + 0.5) / size as f32 * 2. - 1.];
                    filter_texel(src, cubemap_vector(f, uv), mip, num_mips)
                }).collect::<Vec<_>>()
            }) }).collect();
            let mut v: Vec<Vec<[f32; 3]>> = hs.into_iter().map(|h| h.join().unwrap()).collect();
            std::array::from_fn(|f| std::mem::take(&mut v[f]))
        });
        Cube { size, faces }
    }).collect()
}

/// `GSystemTextures.PreintegratedGF` (SystemTextures.cpp:313-417): `(A, B)` per (NoV, roughness) texel, 128 samples, stored as G16R16 (`* 65535 + 0.5`).
pub fn preintegrated_gf() -> Vec<[f32; 2]> {
    let (w, h) = GF_SIZE;
    let mut out = vec![[0f32; 2]; w * h];
    for y in 0..h {
        let roughness = (y as f32 + 0.5) / h as f32;
        let m = roughness * roughness;
        let m2 = m * m;
        for x in 0..w {
            let nov = (x as f32 + 0.5) / w as f32;
            let v = Vec3::new((1. - nov * nov).sqrt(), 0., nov);
            let (mut a, mut b) = (0f32, 0f32);
            const N: u32 = 128;
            for i in 0..N {
                let e1 = i as f32 / N as f32;
                let e2 = (i.reverse_bits() as f64 / 4_294_967_296.0) as f32;
                let phi = 2. * PI * e1;
                let cos_t = ((1. - e2) / (1. + (m2 - 1.) * e2)).sqrt();
                let sin_t = (1. - cos_t * cos_t).max(0.).sqrt();
                let hv = Vec3::new(sin_t * phi.cos(), sin_t * phi.sin(), cos_t);
                let l = 2. * v.dot(hv) * hv - v;
                let nol = l.z.max(0.);
                let noh = hv.z.max(0.);
                let voh = v.dot(hv).max(0.);
                if nol > 0. {
                    let vis = 0.5 / (nol * (nov * (1. - m) + m) + nov * (nol * (1. - m) + m));
                    let nol_vis_pdf = nol * vis * (4. * voh / noh);
                    let mut fc = 1. - voh;
                    fc *= (fc * fc) * (fc * fc);
                    a += nol_vis_pdf * (1. - fc);
                    b += nol_vis_pdf * fc;
                }
            }
            let q = |v: f32| ((v / N as f32).clamp(0., 1.) * 65535. + 0.5).floor() / 65535.;
            out[y * w + x] = [q(a), q(b)];
        }
    }
    out
}

/// IEEE half from f32 (round to nearest even; overflow -> inf, NaN kept).
pub fn f32_to_f16(v: f32) -> u16 {
    let b = v.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let mant = b & 0x7f_ffff;
    if exp == 0xff { return sign | 0x7c00 | if mant != 0 { 0x200 } else { 0 }; }
    let e = exp - 127 + 15;
    if e >= 0x1f { return sign | 0x7c00; }
    if e <= 0 {
        if e < -10 { return sign; }
        let m = mant | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let r = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let r = if rem > half || (rem == half && r & 1 == 1) { r + 1 } else { r };
        return sign | r as u16;
    }
    let r = (e as u32) << 10 | (mant >> 13);
    let rem = mant & 0x1fff;
    let r = if rem > 0x1000 || (rem == 0x1000 && r & 1 == 1) { r + 1 } else { r };
    sign | r as u16
}

/// `SkyReflU` of lit_common.wgsl.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params { color: [f32; 4], params: [f32; 4] }

/// GPU side: the processed cube (binding 11), a trilinear clamp sampler (12, `SF_Trilinear` / `SF_Bilinear` + `AM_Clamp` as the sky cube and the GF LUT use),
/// the parameters (13) and the `PreIntegratedGF` LUT (14).
pub struct SkyReflection {
    cube_view: wgpu::TextureView,
    gf_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    buf: wgpu::Buffer,
    loaded: bool,
}

pub fn globals_layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let f = wgpu::ShaderStages::FRAGMENT;
    let tex = |binding, dim| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dim, multisampled: false }, count: None };
    vec![
        tex(11, wgpu::TextureViewDimension::Cube),
        wgpu::BindGroupLayoutEntry { binding: 12, visibility: f, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        wgpu::BindGroupLayoutEntry { binding: 13, visibility: f, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
        tex(14, wgpu::TextureViewDimension::D2),
    ]
}

fn cube_texture(device: &wgpu::Device, queue: &wgpu::Queue, mips: &[Cube]) -> wgpu::TextureView {
    let size = mips[0].size as u32;
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sky reflection cube"), size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 6 }, mip_level_count: mips.len() as u32, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
    });
    for (m, c) in mips.iter().enumerate() {
        for f in 0..6 {
            let data: Vec<u16> = c.faces[f].iter().flat_map(|p| [f32_to_f16(p[0]), f32_to_f16(p[1]), f32_to_f16(p[2]), f32_to_f16(1.)]).collect();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: m as u32, origin: wgpu::Origin3d { x: 0, y: 0, z: f as u32 }, aspect: wgpu::TextureAspect::All },
                bytemuck::cast_slice(&data), wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8 * c.size as u32), rows_per_image: Some(c.size as u32) },
                wgpu::Extent3d { width: c.size as u32, height: c.size as u32, depth_or_array_layers: 1 });
        }
    }
    tex.create_view(&wgpu::TextureViewDescriptor { label: Some("sky reflection cube"), dimension: Some(wgpu::TextureViewDimension::Cube), ..Default::default() })
}

impl SkyReflection {
    /// A black 1x1 cube (`GBlackTextureCube`, BasePassRendering.cpp:150) until [`set_cube`](Self::set_cube); the LUT is built here.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> SkyReflection {
        let black = Cube { size: 1, faces: std::array::from_fn(|_| vec![[0.; 3]]) };
        let cube_view = cube_texture(device, queue, &[black]);
        let (w, h) = GF_SIZE;
        let gf = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PreintegratedGF"), size: wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba16Float, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let data: Vec<u16> = preintegrated_gf().iter().flat_map(|ab| [f32_to_f16(ab[0]), f32_to_f16(ab[1]), 0, 0]).collect();
        queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &gf, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, bytemuck::cast_slice(&data),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(8 * w as u32), rows_per_image: Some(h as u32) }, wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sky reflection trilinear clamp"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default()
        });
        let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky reflection params"), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let s = SkyReflection { cube_view, gf_view: gf.create_view(&Default::default()), sampler, buf, loaded: false };
        s.update(queue);
        s
    }
    /// Uploads the processed cube ([`filter_reflection_environment`]); the caller rebuilds the globals bind group.
    pub fn set_cube(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, mips: &[Cube]) {
        if mips.is_empty() { return; }
        self.cube_view = cube_texture(device, queue, mips);
        self.loaded = true;
        self.update(queue);
    }
    pub fn loaded(&self) -> bool { self.loaded }
    /// Per frame: `View.SkyLightColor` of the current clock (`lighting::sky_light_tint_of`, SceneRendering.cpp:839) and `SkyLightParameters` (.y = 1 when the
    /// processed cube exists: `GetSkyParametersFromScene` needs `ProcessedTexture`, BasePassRendering.cpp:156).
    pub fn update(&self, queue: &wgpu::Queue) {
        let c = crate::lighting::sky_light_tint_of(&crate::time_of_day::state_at(crate::time_of_day::hour()));
        // `FN_SKY_REFLECTION=0`: debug switch for before / after captures (as FN_DISTANCE_FIELDS), never set in play
        let on = self.loaded && fn_live::var("FN_SKY_REFLECTION").map_or(true, |v| v != "0");
        let p = Params { color: [c[0], c[1], c[2], if on { 1. } else { 0. }], params: [MAX_MIP, 1. / SKY_SPECULAR_OCCLUSION_STRENGTH.max(0.1), AO_SPECULAR_OCCLUSION_MODE, 0.] };
        queue.write_buffer(&self.buf, 0, bytemuck::bytes_of(&p));
    }
    pub fn bind_entries(&self) -> Vec<wgpu::BindGroupEntry<'_>> {
        vec![
            wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&self.cube_view) },
            wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            wgpu::BindGroupEntry { binding: 13, resource: self.buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 14, resource: wgpu::BindingResource::TextureView(&self.gf_view) },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn uniform(size: usize, v: [f32; 3]) -> Cube { Cube { size, faces: std::array::from_fn(|_| vec![v; size * size]) } }
    #[test]
    fn mip_roughness_mapping_roundtrips_and_matches_ue() {
        for m in 0..NUM_MIPS { let r = roughness_from_mip(m as f32, MAX_MIP); assert!((mip_from_roughness(r, MAX_MIP) - m as f32).abs() < 1e-4); }
        // roughness 1 -> mip MaxMip - 2 (LevelFrom1x1 = REFLECTION_CAPTURE_ROUGHEST_MIP = 1)
        assert!((mip_from_roughness(1., MAX_MIP) - 5.).abs() < 1e-6);
        assert_eq!(NUM_MIPS, 8);
    }
    #[test]
    fn hammersley_matches_reverse_bits() {
        assert_eq!(hammersley(0, 32), [0., 0.]);
        assert!((hammersley(1, 32)[1] - 0.5).abs() < 1e-7 && (hammersley(2, 32)[1] - 0.25).abs() < 1e-7);
    }
    #[test]
    fn a_uniform_sky_stays_uniform_in_every_mip() {
        let mips = prefilter_chain(uniform(16, [2., 1., 0.5]));
        assert_eq!(mips.len(), 5);
        for (m, c) in mips.iter().enumerate() {
            assert_eq!(c.size, 16 >> m);
            for f in &c.faces { for p in f { for (k, want) in [2f32, 1., 0.5].into_iter().enumerate() { assert!((p[k] - want).abs() < 1e-3 * want, "mip {m} {p:?}"); } } }
        }
    }
    #[test]
    fn rough_mips_blur_a_bright_face() {
        let mut c = uniform(16, [0.; 3]);
        c.faces[4] = vec![[1.; 3]; 256]; // +Z bright
        let mips = prefilter_chain(c);
        let top = MipChain { levels: &mips };
        // the sharp mip keeps the face, the roughest sees the bright face from the side (cosine lobe)
        assert!(top.sample_level(Vec3::Z, 0.)[0] > 0.7, "{:?}", top.sample_level(Vec3::Z, 0.)); // mip 0 of a 16^2 chain is roughness 0.31
        assert!(top.sample_level(Vec3::X, 0.)[0] < 0.05);
        assert!(top.sample_level(Vec3::X, 4.)[0] > 0.1, "{:?}", top.sample_level(Vec3::X, 4.));
    }
    #[test]
    fn preintegrated_gf_matches_the_known_limits() {
        let gf = preintegrated_gf();
        let (w, h) = GF_SIZE;
        // smooth surface seen head-on: A + B ~ 1 (all energy, F0 weighting), B ~ 0
        let ab = gf[0 * w + (w - 1)];
        assert!(ab[0] + ab[1] > 0.9 && ab[1] < 0.05, "{ab:?}");
        // EnvBRDFApprox(SpecularColor, R, NoV) (BRDF.usf:206) is a (lossy) fit of this LUT: the same shape within 0.15
        let approx = |r: f32, nov: f32| { let c0 = [-1., -0.0275, -0.572, 0.022]; let c1 = [1., 0.0425, 1.04, -0.04]; let rr: [f32; 4] = std::array::from_fn(|i| r * c0[i] + c1[i]); let a004 = (rr[0] * rr[0]).min((-9.28 * nov).exp2()) * rr[0] + rr[1]; [-1.04 * a004 + rr[2], 1.04 * a004 + rr[3]] };
        for (x, y) in [(64usize, 16usize), (100, 8), (30, 24)] {
            let (nov, r) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let (got, fit) = (gf[y * w + x], approx(r, nov));
            assert!((got[0] - fit[0]).abs() < 0.15 && (got[1] - fit[1]).abs() < 0.15 && got[0] + got[1] <= 1.02, "nov {nov} r {r}: {got:?} vs {fit:?}");
        }
    }
    #[test]
    fn half_conversion() {
        for v in [0f32, 1., -2.5, 0.333, 65504., 1e-5, 3.0e-8] {
            let h = f32_to_f16(v);
            let back = { let s = if h & 0x8000 != 0 { -1. } else { 1. }; let e = ((h >> 10) & 0x1f) as i32; let m = (h & 0x3ff) as f32; if e == 0 { s * m * 2f32.powi(-24) } else { s * (1. + m / 1024.) * 2f32.powi(e - 15) } };
            assert!((back - v).abs() <= v.abs() * 1e-3 + 6e-8, "{v} -> {h:#x} -> {back}");
        }
    }
}
