//! LIGHT FUNCTION (2026-10-07): the Athena sun's light function at the Epic material quality, a port of the HIGH quality-level `FLightFunctionPS` of
//! `/Game/Athena/Prototype/Terrain/M_Athena_Fortress_Skybox_LF_Spinning_2` (TODM_A_Child_Athena `DayNightDirectionalLightComponent.LightFunctionMaterial`).
//!
//! Evidence:
//! * The parent `M_Athena_Fortress_Skybox_LF_Spinning` (MD_LightFunction) feeds EmissiveColor through `MaterialExpressionQualitySwitch_0`: Low = `Constant_9`,
//!   High (and Default) = `LinearInterpolate_0` (fn-inspect props). The live Epic dump (`lightdump_002_epic_tod12.json` cvars) has `r.MaterialQualityLevel` = 1
//!   (EMaterialQualityLevel::High, set by Scalability), so the real client runs the High shader map. The port used the LOW shader map before
//!   (stream 33143: emissive = the uniform constant 0.6, i.e. the sun was always x 0.6 within 250 m): the sunlit surfaces were ~1.6x too dark.
//! * The High `FLightFunctionPS` is stream 76146 of the instance package (`fn-dxbc material`, identical program 120669 for the Default level); its material
//!   uniform set is the candidate with 2 vectors / 13 scalars / 2 textures (cb2[2..5] = the scalars below in that order, MIC values over the parent defaults).
//!   It samples `T_FORT_LF` twice (t1) and `T_spiralLF` once (t0), wrap addressing. Both are TC_Grayscale imports but cooked as PF_B8G8R8A8 with SRGB = true
//!   (fn-texture header of the 1.8 packages), so D3D11 binds a B8G8R8A8_UNORM_SRGB view and the program reads LINEAR values (hardware decode before filtering;
//!   `ProcessMaterialGreyscaleTextureLookup` is a no-op on SM5, MaterialTemplate.usf:782).
//! * Inputs: `W` = the pixel's ABSOLUTE world position (cm; SceneDepth -> translated world - View.PreViewTranslation), `View.GameTime` (cb1[131].z),
//!   `View.WorldCameraOrigin` (cb1[59]); LightFunctionParameters2 (cb0[11]) = (LightFunctionFadeDistance, DisabledBrightness), cb0[2].y = ShadowFadeFraction (1 for
//!   the directional light: its bounds are WORLD_MAX). The result is written `sqrt` encoded into the B8G8R8A8 (unorm) light attenuation buffer, so values above
//!   1 saturate: the effective factor is `min(x, 1)` (the 8-bit quantisation is not reproduced).
use glam::Vec3;

pub const SPIRAL_PACKAGE: &str = "/Game/Environments/World/LightFunctions/T_spiralLF";
pub const FORT_PACKAGE: &str = "/Game/Environments/World/LightFunctions/T_FORT_LF";

/// The 13 scalars of the High uniform set, M_Athena_Fortress_Skybox_LF_Spinning_2 values (parent default where the instance has none: Speed_1, Speed_2).
pub mod p {
    pub const SPEED_1: f32 = 1.368_582;
    pub const SIZE: f32 = 11_682.961;
    pub const SPEED: f32 = 0.893_945;
    pub const SIZE_1: f32 = 7_685.473_6;
    pub const SPEED_2: f32 = 7.003_327;
    pub const SIZE_2: f32 = 8_437.12;
    pub const LERP_SUB_B: f32 = 0.0;
    pub const LERP_SUB_A: f32 = -0.149_712;
    pub const CONTRAST_HIGH: f32 = 1.416_333;
    pub const CONTRAST_LOW: f32 = 0.333_945;
    pub const LIGHTEN_SHADOW: f32 = -0.006_251;
    pub const BRIGHT_DISTANCE: f32 = 800_000.0;
    pub const DARK_DIST: f32 = 2_800_000.0;
}

/// Texture coordinates of the three samples: (T_spiralLF uv1, T_FORT_LF uv2, T_FORT_LF uv3).
pub fn uvs(w_cm: Vec3, game_time: f32) -> ([f32; 2], [f32; 2], [f32; 2]) {
    let s = |size: f32, speed: f32| [w_cm.x / size, w_cm.y / size + game_time * speed * 0.01];
    (s(p::SIZE, p::SPEED_1), s(p::SIZE_1, p::SPEED), s(p::SIZE_2, p::SPEED_2))
}

/// The emissive grey of the High program for the three texture reads (`spiral` = T_spiralLF(uv1).r, `fort2` = T_FORT_LF(uv2).r, `fort3` = T_FORT_LF(uv3).r) at
/// absolute world position `w_cm` (before the fade to DisabledBrightness and before the attenuation-buffer saturation).
pub fn material_grey(spiral: f32, fort2: f32, fort3: f32, w_cm: Vec3) -> f32 {
    let sat = |x: f32| x.clamp(0.0, 1.0);
    let m3 = (fort3 * fort3) * (p::LERP_SUB_B - p::LERP_SUB_A) + p::LERP_SUB_A;
    let prod = spiral * fort2;
    let a = sat(-(1.0 - prod) * m3 + prod);
    let a = sat(a * (p::CONTRAST_HIGH - p::CONTRAST_LOW) + p::CONTRAST_LOW);
    let y = (1.0 + p::LIGHTEN_SHADOW - a).clamp(0.05, 1.0);
    let x = (y * 2.0 + 0.5) * 0.666_667;
    let far = (x * x - 1.0) * 0.35 + 1.0 - x * x;
    let near = x * x;
    let d = w_cm.length();
    let fb = sat((d - 250_000.0) / p::BRIGHT_DISTANCE);
    let fd = sat((d - 16_000.0) / p::DARK_DIST);
    let v = fb * far + near;
    let v = (v - fd * v).max(0.0); // + SelectionColor (black at runtime), max 0
    v * 0.3333 * 3.0 // dot(v.rrr, 0.3333)
}

/// `FLightFunctionPS` tail: fade to `DisabledBrightness` over the last 20 % of `LightFunctionFadeDistance` (camera distance), then the unorm attenuation buffer.
pub fn faded(grey: f32, cam_dist_cm: f32, fade_distance_cm: f32, disabled: f32) -> f32 {
    let a = ((fade_distance_cm - cam_dist_cm) / (fade_distance_cm * 0.2)).clamp(0.0, 1.0);
    (disabled + (grey - disabled) * a).clamp(0.0, 1.0)
}

/// A decoded grayscale texture on the CPU (red channel, wrap addressing, bilinear) for CPU evaluation (volumetric fog factor, tests).
#[derive(Clone, Debug)]
pub struct GreyImage { pub width: u32, pub height: u32, pub texels: Vec<u8>, /** sRGB-encoded texels (the sampler decodes them) */ pub srgb: bool }
impl GreyImage {
    pub fn sample(&self, uv: [f32; 2]) -> f32 {
        if self.texels.is_empty() { return 0.0; }
        let (w, h) = (self.width as i64, self.height as i64);
        let (x, y) = (uv[0] * w as f32 - 0.5, uv[1] * h as f32 - 0.5);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let dec = |v: f32| if !self.srgb { v } else if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        let t = |i: i64, j: i64| dec(self.texels[(j.rem_euclid(h) * w + i.rem_euclid(w)) as usize] as f32 / 255.0);
        let (i, j) = (x0 as i64, y0 as i64);
        let a = t(i, j) + (t(i + 1, j) - t(i, j)) * fx;
        let b = t(i, j + 1) + (t(i + 1, j + 1) - t(i, j + 1)) * fx;
        a + (b - a) * fy
    }
}

/// CPU evaluation of the whole light function at an absolute world point.
pub fn eval_cpu(spiral: &GreyImage, fort: &GreyImage, w_cm: Vec3, game_time: f32) -> f32 {
    let (u1, u2, u3) = uvs(w_cm, game_time);
    material_grey(spiral.sample(u1), fort.sample(u2), fort.sample(u3), w_cm)
}

/// GPU half: the two textures + their sampler, bound in the renderer's globals (lit_common.wgsl bindings 20..22).
pub struct LightFunctionGpu {
    pub fort_view: wgpu::TextureView,
    pub spiral_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    /// true once the real textures were installed (until then 1x1 black: the program then yields ~0.994, i.e. no visible pattern)
    pub loaded: bool,
    pub cpu: Option<(GreyImage, GreyImage)>,
    /// texel counts (fort, spiral) for the shader's mip selection
    pub sizes: [f32; 2],
}
pub const BINDING_FORT: u32 = 20;
pub const BINDING_SPIRAL: u32 = 21;
pub const BINDING_SAMPLER: u32 = 22;

pub fn globals_layout_entries() -> Vec<wgpu::BindGroupLayoutEntry> {
    let f = wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE;
    let tex = |binding| wgpu::BindGroupLayoutEntry { binding, visibility: f, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None };
    vec![tex(BINDING_FORT), tex(BINDING_SPIRAL), wgpu::BindGroupLayoutEntry { binding: BINDING_SAMPLER, visibility: f, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None }]
}

fn grey_to_rgba(img: &GreyImage) -> image::RgbaImage {
    image::RgbaImage::from_fn(img.width, img.height, |x, y| { let v = img.texels[(y * img.width + x) as usize]; image::Rgba([v, v, v, 255]) })
}

impl LightFunctionGpu {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let black = crate::texture::solid_rgba8(device, queue, [0, 0, 0, 255], "light function placeholder");
        let v = black.create_view(&Default::default());
        let v2 = black.create_view(&Default::default());
        LightFunctionGpu { fort_view: v, spiral_view: v2, sampler: Self::sampler(device), loaded: false, cpu: None, sizes: [1.0, 1.0] }
    }
    fn sampler(device: &wgpu::Device) -> wgpu::Sampler {
        let a = wgpu::AddressMode::Repeat;
        device.create_sampler(&wgpu::SamplerDescriptor { label: Some("light function sampler"), address_mode_u: a, address_mode_v: a, address_mode_w: a,
            mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default() })
    }
    /// Installs the decoded textures (top cooked mip of each; mips rebuilt with a box filter).
    pub fn set_textures(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, spiral: GreyImage, fort: GreyImage) {
        let up = |g: &GreyImage, l: &str| crate::texture::upload_rgba8(device, queue, &grey_to_rgba(g), g.srgb, crate::texture::MipFilter::Box, l).create_view(&Default::default());
        self.spiral_view = up(&spiral, "T_spiralLF");
        self.fort_view = up(&fort, "T_FORT_LF");
        self.sizes = [fort.width.max(fort.height) as f32, spiral.width.max(spiral.height) as f32];
        self.cpu = Some((spiral, fort));
        self.loaded = true;
    }
    pub fn bind_entries(&self) -> Vec<wgpu::BindGroupEntry<'_>> {
        vec![
            wgpu::BindGroupEntry { binding: BINDING_FORT, resource: wgpu::BindingResource::TextureView(&self.fort_view) },
            wgpu::BindGroupEntry { binding: BINDING_SPIRAL, resource: wgpu::BindingResource::TextureView(&self.spiral_view) },
            wgpu::BindGroupEntry { binding: BINDING_SAMPLER, resource: wgpu::BindingResource::Sampler(&self.sampler) },
        ]
    }
}

/// The absolute UE world position (cm) of the renderer's world origin: UE = (x * 100 - off.x, z * 100 - off.y, y * 100 - off.z) for a renderer point (x, y, z) m.
/// Set by the application from the layout mapping (`fn-app` storm_hook::offset_cm, the same translation as props / backdrop / storm).
static UE_OFFSET_CM: std::sync::Mutex<[f32; 3]> = std::sync::Mutex::new([0.0; 3]);
pub fn set_ue_offset_cm(o: [f32; 3]) { if let Ok(mut m) = UE_OFFSET_CM.lock() { *m = o; } }
pub fn ue_offset_cm() -> [f32; 3] { UE_OFFSET_CM.lock().map(|m| *m).unwrap_or([0.0; 3]) }
/// `View.GameTime` the program scrolls with: the frame time, or `FN_LF_GAME_TIME=<s>` (comparison aid: the real reference poses record
/// `PlayerCameraManager.CameraCache.TimeStamp` = the world time of the capture, e.g. 3098.4978 s for epic_tod12_lodge_W).
pub fn game_time(frame_time: f32) -> f32 {
    static OVR: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    OVR.get_or_init(|| fn_live::var("FN_LF_GAME_TIME").ok().and_then(|v| v.parse().ok())).unwrap_or(frame_time)
}
/// Renderer world (m, Y up) -> absolute UE world (cm, Z up).
pub fn to_ue_abs_cm(p: Vec3) -> Vec3 { let o = ue_offset_cm(); Vec3::new(p.x * 100.0 - o[0], p.z * 100.0 - o[1], p.y * 100.0 - o[2]) }

#[cfg(test)]
mod tests {
    use super::*;
    /// The program's range: clear sky (texture product 0, no third-layer darkening) gives x^2 = 1.47 (saturated to 1 by the attenuation buffer), full cloud
    /// cover 0.16; the old Low-quality constant was 0.6 everywhere.
    #[test] fn range_of_the_high_program() {
        let w = Vec3::new(50652.0, 71640.0, 2045.0);
        let clear = material_grey(0.0, 0.0, 1.0, w);
        let cloud = material_grey(1.0, 1.0, 1.0, w);
        assert!(clear > 1.3 && clear < 1.5, "{clear}");
        assert!(cloud > 0.1 && cloud < 0.2, "{cloud}");
        assert_eq!(faded(clear, 100.0, 25000.0, 1.0), 1.0);
        // beyond the fade distance: DisabledBrightness
        assert_eq!(faded(cloud, 30000.0, 25000.0, 1.0), 1.0);
    }
    #[test] fn uv_scroll_uses_game_time() {
        let (a, _, _) = uvs(Vec3::new(p::SIZE, 0.0, 0.0), 100.0);
        assert!((a[0] - 1.0).abs() < 1e-6 && (a[1] - 100.0 * p::SPEED_1 * 0.01).abs() < 1e-5);
    }
    #[test] fn grey_image_wraps() {
        let g = GreyImage { width: 2, height: 1, texels: vec![0, 255], srgb: false };
        assert!((g.sample([0.25, 0.5]) - 0.0).abs() < 1e-6);
        assert!((g.sample([1.25, 0.5]) - 0.0).abs() < 1e-6);
        assert!((g.sample([0.5, 0.5]) - 0.5).abs() < 1e-6);
    }
}
