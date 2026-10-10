//! TEMPORAL AA + the other anti-aliasing / upscale passes of Fortnite's scalability levels (see `quality::AaScalability`):
//!
//! * High / Epic (`r.PostProcessAAQuality` 3 / 4, CookedIniVersion.txt L5072 / L5074): UE 4.16 temporal AA, `MainFastTemporalAA` / `MainTemporalAA`
//!   (shaders/taa.wgsl), on the HDR scene colour after the translucency and before bloom (PostProcessing.cpp:1449-1465, `AddTemporalAA` :874-898).
//!   - Jitter: `r.TemporalAASamples` 8 (exe default, thunk 0x1678e0), Halton(2, 3) of `TemporalAASampleIndex + 1` Box-Muller'd into a windowed
//!     Gaussian of sigma `0.47 * r.TemporalAAFilterSize` (SceneVisibility.cpp:2137-2251), index advanced before use (`OnFrameRenderingSetup`,
//!     ScenePrivate.h:798-818).
//!   - Filter weights: `r.TemporalAACatmullRom` = 0 in the 1.8 exe (exe-defaults.json, thunk 0x15e240; UE 4.16 source default is 1) -> Gaussian
//!     `exp(-2.29 * (dx^2 + dy^2))`, `dx = offset - jitter`, divided by `r.TemporalAAFilterSize` 1.0 (thunk 0x15e330); PlusWeights normalised over
//!     the 5 plus taps (PostProcessTemporalAA.cpp:150-200).
//!   - `r.TemporalAACurrentFrameWeight` 0.04 (exe, thunk 0x15e2b0). History: PF_FloatRGBA (`ComputeOutputDesc`, :722-732), so no dither.
//!   - `ClipToPrevClip` = no-AA inverse view-projection x previous no-AA view-projection (SceneView.cpp:2264-2268); previous matrices are reset
//!     to the current ones on a large camera move (45 deg / 10000 cm, `IsLargeCameraMovement`, SceneVisibility.cpp:2024-2037 and :2296-2306).
//!   - No history (first frame, resize, AA switched on): BlackDummy history with the responsive (1/4) blend, as on a camera cut (:881-883, :916-955).
//!   - Velocity: shaders/velocity.wgsl for the skinned models (VelocityShader.usf; the BR character parts `@invariant` + depth Equal = their clipped
//!     pre-pass coverage), cleared to 0 elsewhere (camera reprojection from depth).
//!   - GRASS MOTION (2026-10-07): the scene-lit FX draws (WPO grass / foliage / statics) write their base-pass SV_Target4 velocity (fn-fx `fs_velocity`,
//!     on by default, `FN_FX_VELOCITY=0` = off) from complete previous-view members ([`ue_view_slot::PREV_PAIRS`]), and their `View` buffers carry the
//!     view's jitter ([`apply_view_jitter`], `FN_FX_JITTER=0` = off).
//!   KNOWN GAPS (no velocity written): the held weapon / pickaxe (`gun_velocity_main` drawn only with FN_FX_VELOCITY=1, not re-verified, renderer.rs velocity node);
//!   particles / translucent FX (UE writes none for translucency either), the procedural props; those pixels reproject with the camera only and rely on the neighbourhood clamp. 
//!   - STORM TAA (2026-10-07): responsive AA. A translucent `bEnableResponsiveAA` material (the storm wall `M_SafeZoneSphere`) sets
//!     STENCIL_TEMPORAL_RESPONSIVE_AA_MASK on every pixel it draws (TranslucentRendering.cpp:531-557, cleared at the start of translucency,
//!     SceneRenderTargets.cpp:1374) and those pixels resolve with `FPostProcessTemporalAAPS<.., RESPONSIVE 1>` (PostProcessTemporalAA.cpp:1027-1060):
//!     history fetched and clamped as usual, `BlendFinal = 1/4` (PostProcessTemporalCommon.usf:114-123 / :565). The port's depth has no stencil: an R8 mask
//!     ([`RESPONSIVE_MASK_FORMAT`], cleared to 1, the FX mask twins write 0 = tagged, fx.rs `draw_responsive_aa`) is read by taa.wgsl. Was: untracked, so the
//!     wall's panning scanlines / grid kept 96 % of a history reprojected with the opaque depth behind the wall (smeared, ghosted bands).
//! * Medium (quality 2): FXAA 3.11 console path (shaders/fxaa_console.wgsl) on the tone-mapped LDR image.
//! * `r.ScreenPercentage` < 100 (`sg.ResolutionQuality`): the scene and the post chain run at the scaled view size, then the UE 4.16 upscale pass
//!   (shaders/upscale.wgsl, method = `r.Upscale.Quality`) writes the window.
use crate::camera::CameraParams;
use bytemuck::{Pod, Zeroable};


/// Exe cvar defaults (no ini key overrides them: CookedIniVersion.txt has no `r.TemporalAA*` line).
pub mod ue {
    pub const TEMPORAL_AA_SAMPLES: u32 = 8;
    pub const TEMPORAL_AA_FILTER_SIZE: f32 = 1.0;
    pub const TEMPORAL_AA_CATMULL_ROM: bool = false;
    pub const TEMPORAL_AA_CURRENT_FRAME_WEIGHT: f32 = 0.04;
    /// `IsLargeCameraMovement(View, .., 45.0f, 10000.0f)` (SceneVisibility.cpp:2298), metres here.
    pub const PREV_RESET_ROTATION_DEG: f32 = 45.;
    pub const PREV_RESET_TRANSLATION_M: f32 = 100.;
}

/// `Halton(Index, Base)` (SceneVisibility.cpp:2039-2051).
pub fn halton(mut index: u32, base: u32) -> f32 {
    let mut result = 0.0f32;
    let inv = 1.0 / base as f32;
    let mut fraction = inv;
    while index > 0 { result += (index % base) as f32 * fraction; index /= base; fraction *= inv; }
    result
}
/// Sub-pixel jitter (pixels, x right, y down) of `TemporalAASampleIndex` for `r.TemporalAASamples` > 5 (SceneVisibility.cpp:2217-2245).
pub fn jitter_pixels(sample_index: u32) -> [f32; 2] {
    let u1 = halton(sample_index + 1, 2);
    let u2 = halton(sample_index + 1, 3);
    let sigma = 0.47 * ue::TEMPORAL_AA_FILTER_SIZE;
    let out_window = 0.5f32;
    let in_window = (-0.5 * (out_window / sigma).powi(2)).exp();
    let theta = 2.0 * std::f32::consts::PI * u2;
    let r = sigma * (-2.0 * ((1.0 - u1) * in_window + u1).ln()).sqrt();
    [r * theta.cos(), r * theta.sin()]
}
/// NDC offset of a pixel jitter on a `w` x `h` view (`HackAddTemporalAAProjectionJitter(SampleX * 2 / W, SampleY * -2 / H)`).
pub fn jitter_ndc(j: [f32; 2], w: u32, h: u32) -> [f32; 2] { [j[0] * 2. / w.max(1) as f32, j[1] * -2. / h.max(1) as f32] }
/// `PlusWeights[5]` (taps (0,-1), (-1,0), (0,0), (1,0), (0,1)) for a pixel jitter (PostProcessTemporalAA.cpp:137-200).
pub fn plus_weights(j: [f32; 2]) -> [f32; 5] {
    let w = |ox: f32, oy: f32| {
        let (x, y) = ((ox - j[0]) / ue::TEMPORAL_AA_FILTER_SIZE, (oy - j[1]) / ue::TEMPORAL_AA_FILTER_SIZE);
        if ue::TEMPORAL_AA_CATMULL_ROM { catmull_rom(x) * catmull_rom(y) } else { (-2.29 * (x * x + y * y)).exp() }
    };
    let p = [w(0., -1.), w(-1., 0.), w(0., 0.), w(1., 0.), w(0., 1.)];
    let t: f32 = p.iter().sum();
    p.map(|x| x / t)
}
fn catmull_rom(x: f32) -> f32 { let ax = x.abs(); if ax > 1. { ((-0.5 * ax + 2.5) * ax - 4.) * ax + 2. } else { (1.5 * ax - 2.5) * ax * ax + 1. } }
/// `IsLargeCameraMovement`: right / up / forward vectors more than 45 deg apart, or the eye moved more than 100 m.
pub fn is_large_camera_movement(cur: &CameraParams, prev: &CameraParams) -> bool {
    let basis = |c: &CameraParams| { let f = c.forward.normalize(); let r = f.cross(c.up).normalize(); (r, r.cross(f), f) };
    let (a, b) = (basis(cur), basis(prev));
    let t = ue::PREV_RESET_ROTATION_DEG.to_radians().cos();
    a.0.dot(b.0) < t || a.1.dot(b.1) < t || a.2.dot(b.2) < t || (cur.eye - prev.eye).length_squared() > ue::PREV_RESET_TRANSLATION_M * ue::PREV_RESET_TRANSLATION_M
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TaaU { plus0: [f32; 4], plus1: [f32; 4], clip_to_prev: [[f32; 4]; 4], viewport: [f32; 4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct VelU { view_proj: [[f32; 4]; 4], view_proj_no_aa: [[f32; 4]; 4], prev_view_proj_no_aa: [[f32; 4]; 4] }
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vec4U { a: [f32; 4], b: [f32; 4] }

pub const VELOCITY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const HISTORY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// STORM TAA: the responsive-AA mask (UE's stencil bit 3): 1 = normal feedback, 0 = responsive (written by the FX mask twins, fx.rs `draw_responsive_aa`).
pub const RESPONSIVE_MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

struct Targets {
    size: (u32, u32),
    input: wgpu::Texture,
    input_view: wgpu::TextureView,
    history: [wgpu::TextureView; 2],
    /// STORM TAA: the responsive-AA mask ([`RESPONSIVE_MASK_FORMAT`], view size), cleared + filled by [`TemporalAa::begin_responsive_pass`] every TAA frame
    responsive: wgpu::TextureView,
    /// bind group reading history[i] (writes history[1 - i]); rebuilt with the scene depth / the velocity target (key = (depth gen, velocity gen))
    bgs: Option<([wgpu::BindGroup; 2], (u64, u64))>,
}

/// VELOCITY (2026-10-05): what the scene velocity buffer holds this frame (see [`TemporalAa::begin_velocity`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VelocityState {
    /// `ShouldRenderVelocities` (VelocityRendering.cpp:855-875): motion blur OR temporal AA (not on a camera cut) OR distance-field AO OR the SSR temporal pass.
    pub render: bool,
    /// `View.bCameraCut`: no primitive writes velocity (`HasVelocityOnBasePass` / `HasVelocity` return false, VelocityRendering.cpp:396 / :436), the buffer stays 0.
    pub camera_cut: bool,
    /// `bPrevTransformsReset` (first frame, camera cut or `IsLargeCameraMovement`, SceneVisibility.cpp:2296-2306): the previous view matrices are the current ones.
    pub prev_reset: bool,
}
/// VELOCITY: `EncodeVelocityToTexture` (Common.usf:1736-1741) into the PF_G16R16 target (`FVelocityRendering::GetRenderTargetDesc`, VelocityRendering.cpp:924):
/// 16-bit unorm quantisation (round to nearest), kept in the Rg32Float [`VELOCITY_FORMAT`] texture. WGSL helper shared by every velocity fragment entry.
pub const VELOCITY_ENCODE_WGSL: &str = "fn velocity_store_g16r16(encoded: vec2<f32>) -> vec4<f32> { return vec4<f32>(round(clamp(encoded, vec2<f32>(0.0), vec2<f32>(1.0)) * 65535.0) / 65535.0, 0.0, 0.0); }\n";
/// VELOCITY: `EncodeVelocityToTexture` + the PF_G16R16 unorm store (CPU reference of [`VELOCITY_ENCODE_WGSL`]).
pub fn encode_velocity(v: [f32; 2]) -> [f32; 2] { v.map(|x| ((x * (0.499 * 0.5) + 32767. / 65535.).clamp(0., 1.) * 65535.).round() / 65535.) }
/// VELOCITY: float4 slots of the original programs' `View` uniform buffer (fn-fx `view::view_slot`, fn-matshader `view::slot`; the cooked base-pass VS reads
/// `PrevTranslatedWorldToClip` at 83..86 and `PrevPreViewTranslation` at 105: fn-matshader gun VS `vcb1[83..86]`, `vcb1[105]`).
pub mod ue_view_slot {
    pub const TRANSLATED_WORLD_TO_CLIP: usize = 0;
    pub const PRE_VIEW_TRANSLATION: usize = 62;
    pub const PREV_TRANSLATED_WORLD_TO_CLIP: usize = 83;
    pub const PREV_PRE_VIEW_TRANSLATION: usize = 105;
    /// VELOCITY FIX (2026-10-07): the rest of the previous-view block (UE4.16 SceneView.h:579-591, +4 float4s in 1.8: `PreViewTranslation` at 62, see fn-fx
    /// view_slot): (current slot, previous slot, float4 count). `PrevWorldCameraOrigin` (103) / `PrevWorldViewOrigin` (104) were left 0: every material whose
    /// world position offset reads the camera position (the previous-frame WPO compile reads `ResolvedView.PrevWorldCameraOrigin`) then got a different
    /// previous WPO, i.e. own velocity on still geometry growing towards the camera (FN_TAA_DEBUG=2: > 8 px on a near tree trunk).
    /// GRASS MOTION (2026-10-07): 1.8's extra matrix of the 24..39 block is `ViewToClipNoAA` at 28 (the cooked base-pass PS reads the projection jitter at
    /// View[26].xy = ViewToClip row 2 and SVPositionToTranslatedWorld is at 40, FSceneView::SetupViewRectUniformBufferParameters 0x1423e1a30 writes +0x280), so
    /// ClipToView is 32; `PrevProjection` (63..66, its row 2 = View[65], the previous jitter the same PS subtracts) = the previous ViewToClip
    /// (SceneView.cpp:2209 `PrevProjection = PrevViewMatrices.GetProjectionMatrix()`).
    pub const PREV_PAIRS: [(usize, usize, usize); 11] = [
        (TRANSLATED_WORLD_TO_CLIP, PREV_TRANSLATED_WORLD_TO_CLIP, 4), (62, PREV_PRE_VIEW_TRANSLATION, 1),
        (8, 87, 4) /* TranslatedWorldToView */, (12, 91, 4) /* ViewToTranslatedWorld */, (16, 95, 4) /* TranslatedWorldToCameraView */, (20, 99, 4) /* CameraViewToTranslatedWorld */,
        (VIEW_TO_CLIP, 75, 4) /* ViewToClip -> PrevViewToClip */, (32, 79, 4) /* ClipToView -> PrevClipToView */, (VIEW_TO_CLIP, PREV_PROJECTION, 4) /* -> PrevProjection */,
        (59, 103, 1) /* WorldCameraOrigin */, (61, 104, 1) /* WorldViewOrigin */,
    ];
    pub const PREV_FLOAT4S: usize = 4 + 1 + 4 * 4 + 4 + 4 + 4 + 1 + 1;
    pub const WORLD_TO_CLIP: usize = 4;
    pub const VIEW_TO_CLIP: usize = 24;
    pub const SV_POSITION_TO_TRANSLATED_WORLD: usize = 40;
    pub const PREV_PROJECTION: usize = 63;
    pub const VIEW_SIZE_AND_INV_SIZE: usize = 121;
}
/// GRASS MOTION (2026-10-07): the view's temporal AA jitter on an application `View` buffer (built from the unjittered window camera, fn-fx `ViewParams`).
/// UE jitters every primitive of the view: `HackAddTemporalAAProjectionJitter` adds `(jx, jy)` to the projection's row 2 (SceneView.h:427-433; ViewToClip,
/// TranslatedWorldToClip, WorldToClip and SVPositionToTranslatedWorld all derive from it, SceneView.cpp:2186-2214 / SetupViewRectUniformBufferParameters),
/// and the cooked base-pass velocity subtracts it again (`View[26].xy` = ViewToClip[2].xy, `View[65].xy` = PrevProjection[2].xy, M_FieldGrassMatching
/// TBasePassPS: `Calculate2DVelocity`, VelocityCommon.usf:9-13 in 1.8's form). Without it the FX draws (WPO grass, POI statics) were the only unjittered
/// opaque geometry: not supersampled by the TAA (thin blades crawl / swim while the camera moves) and depth-tested against the jittered terrain.
/// `j` = NDC jitter ([`jitter_ndc`]). Call BEFORE [`ViewHistory::patch`] so the previous-view slots receive last frame's jittered matrices.
pub fn apply_view_jitter(bytes: &mut [u8], j: [f32; 2]) {
    use ue_view_slot::*;
    if bytes.len() < (VIEW_SIZE_AND_INV_SIZE + 1) * 16 { return; }
    let rd = |b: &[u8], slot: usize, k: usize| f32::from_le_bytes(b[slot * 16 + k * 4..slot * 16 + k * 4 + 4].try_into().unwrap());
    let wr = |b: &mut [u8], slot: usize, k: usize, v: f32| b[slot * 16 + k * 4..slot * 16 + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
    // idempotent: ViewToClip[2].xy holds the jitter already in the buffer (0 as fn-fx writes it); a frame whose writes the application did not renew is
    // prepared again with its bytes already patched, so only the difference to this frame's jitter is applied
    let applied = [rd(bytes, VIEW_TO_CLIP + 2, 0), rd(bytes, VIEW_TO_CLIP + 2, 1)];
    let j_new = j;
    let j = [j_new[0] - applied[0], j_new[1] - applied[1]];
    if j == [0.0; 2] { return; }
    // row-vector clip = p * M: clip.xy += j * clip.w  <=>  every row: (x, y) += j * w
    for row in (TRANSLATED_WORLD_TO_CLIP..TRANSLATED_WORLD_TO_CLIP + 4).chain(WORLD_TO_CLIP..WORLD_TO_CLIP + 4) {
        let w = rd(bytes, row, 3);
        for k in 0..2 { let v = rd(bytes, row, k) + j[k] * w; wr(bytes, row, k, v); }
    }
    // ViewToClip row 2 (x, y) = the jitter (the port leaves the rest of ViewToClip 0; only the velocity PS reads row 2 .xy)
    wr(bytes, VIEW_TO_CLIP + 2, 0, j_new[0]); wr(bytes, VIEW_TO_CLIP + 2, 1, j_new[1]);
    // SVPositionToTranslatedWorld = S * M^-1 -> S * J^-1 * M^-1: ndc -= j; rows 0 / 1 of the product are (2 / W) and (-2 / H) times M^-1's rows 0 / 1
    let (wd, ht) = (rd(bytes, VIEW_SIZE_AND_INV_SIZE, 0), rd(bytes, VIEW_SIZE_AND_INV_SIZE, 1));
    let s = SV_POSITION_TO_TRANSLATED_WORLD;
    for k in 0..4 { let v = rd(bytes, s + 3, k) - j[0] * 0.5 * wd * rd(bytes, s, k) + j[1] * 0.5 * ht * rd(bytes, s + 1, k); wr(bytes, s + 3, k, v); }
}
pub fn apply_view_size(bytes: &mut [u8], size: (u32, u32)) {
    use ue_view_slot::*;
    if bytes.len() < (VIEW_SIZE_AND_INV_SIZE + 1) * 16 || size.0 == 0 || size.1 == 0 { return; }
    let rd = |b: &[u8], slot: usize, k: usize| f32::from_le_bytes(b[slot * 16 + k * 4..slot * 16 + k * 4 + 4].try_into().unwrap());
    let wr = |b: &mut [u8], slot: usize, k: usize, v: f32| b[slot * 16 + k * 4..slot * 16 + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
    let (ow, oh) = (rd(bytes, VIEW_SIZE_AND_INV_SIZE, 0), rd(bytes, VIEW_SIZE_AND_INV_SIZE, 1));
    let (nw, nh) = (size.0 as f32, size.1 as f32);
    if !(ow > 0. && oh > 0.) || (ow == nw && oh == nh) { return; }
    let (sx, sy) = (ow / nw, oh / nh);
    let s = SV_POSITION_TO_TRANSLATED_WORLD;
    for k in 0..4 {
        let v = rd(bytes, s, k) * sx; wr(bytes, s, k, v);
        let v = rd(bytes, s + 1, k) * sy; wr(bytes, s + 1, k, v);
    }
    for (k, v) in [nw, nh, 1. / nw, 1. / nh].into_iter().enumerate() { wr(bytes, VIEW_SIZE_AND_INV_SIZE, k, v); }
}
/// VELOCITY (2026-10-05): the previous-view members of the application's `View` buffers. UE sets `PrevTranslatedWorldToClip` / `PrevPreViewTranslation` from the
/// view's previous `FViewMatrices` (SceneView.cpp:2211 / :2224), which are the last frame's matrices, or the current ones on `bPrevTransformsReset`
/// (SceneVisibility.cpp:2296-2306). The application builds every `View` buffer from this frame's camera only, so the renderer keeps, per buffer, the last frame's
/// `TranslatedWorldToClip` (0..3) and `PreViewTranslation` (62) and writes them into 83..86 / 105 of the next frame's write: the previous matrices of the very
/// camera the buffer was built with. A buffer not written in the previous frame, or a reset, takes its current matrices (no camera motion). The app's views are
/// unjittered (window camera), with `TemporalAAJitter` 0 (the cooked PS subtracts View[26].xy / View[65].xy, both left 0): `Calculate2DVelocity` then gives the
/// unjittered motion, which is what UE's jittered positions minus their jitters give.
#[derive(Default)]
pub struct ViewHistory { frame: u64, reset: bool, map: std::collections::HashMap<usize, (u64, Vec<[f32; 4]>, Vec<[f32; 4]>)> }
impl ViewHistory {
    /// Start of a rendered frame.
    pub fn begin_frame(&mut self, s: VelocityState) { self.frame += 1; self.reset = s.prev_reset; }
    /// Patches one whole-buffer write (from offset 0) of the `View` buffer `key`.
    pub fn patch(&mut self, key: usize, bytes: &mut [u8]) {
        use ue_view_slot::*;
        if bytes.len() < (PREV_PRE_VIEW_TRANSLATION + 1) * 16 { return; }
        debug_assert_eq!(PREV_PAIRS.iter().map(|p| p.2).sum::<usize>(), PREV_FLOAT4S);
        let read = |b: &[u8], slot: usize| -> [f32; 4] { std::array::from_fn(|k| f32::from_le_bytes(b[slot * 16 + k * 4..slot * 16 + k * 4 + 4].try_into().unwrap())) };
        let cur: Vec<[f32; 4]> = PREV_PAIRS.iter().flat_map(|&(c, _, n)| (0..n).map(move |i| c + i)).map(|sl| read(bytes, sl)).collect();
        let (frame, reset) = (self.frame, self.reset);
        let e = self.map.entry(key).or_insert_with(|| (0, cur.clone(), cur.clone()));
        let prev = if e.0 == frame { e.1.clone() } else if e.0 + 1 == frame && !reset { e.2.clone() } else { cur.clone() };
        let mut write = |slot: usize, v: [f32; 4]| for k in 0..4 { bytes[slot * 16 + k * 4..slot * 16 + k * 4 + 4].copy_from_slice(&v[k].to_le_bytes()); };
        let mut at = 0;
        for &(_, p, n) in PREV_PAIRS.iter() { for i in 0..n { write(p + i, prev[at]); at += 1; } }
        *e = (frame, prev, cur);
    }
}
/// VELOCITY: `DecodeVelocityFromTexture` (Common.usf:1743-1750).
pub fn decode_velocity(e: [f32; 2]) -> [f32; 2] { let inv = 1. / (0.499 * 0.5); e.map(|x| x * inv - 32767. / 65535. * inv) }

pub struct TemporalAa {
    device: wgpu::Device,
    vel_pipeline: wgpu::RenderPipeline,
    /// VELOCITY (2026-10-07): the BR character parts' twin (`@invariant`, depth Equal), see [`Self::set_velocity_char`].
    vel_char_pipeline: wgpu::RenderPipeline,
    vel_buf: wgpu::Buffer,
    vel_bg: wgpu::BindGroup,
    /// VELOCITY: layout of `vel_bg` (`VelU`, vertex stage) for the other velocity pipelines (statics, props).
    vel_bgl: wgpu::BindGroupLayout,
    /// VELOCITY: the scene velocity target (view size) + its generation (TAA bind groups follow it); exists while velocity or TAA runs.
    velocity_target: Option<((u32, u32), wgpu::TextureView, u64)>,
    velocity_gen: u64,
    /// VELOCITY: the integrator's motion blur switch ([`Self::set_velocity_wanted`]) and this frame's state.
    motion_blur: bool,
    velocity: VelocityState,
    /// VELOCITY: the velocity pass ran last frame (the previous object transforms / bones were recorded).
    velocity_last_frame: bool,
    velocity_this_frame: bool,
    vp_no_aa: glam::Mat4,
    prev_vp_no_aa: glam::Mat4,
    taa_pipeline: wgpu::RenderPipeline,
    taa_bgl: wgpu::BindGroupLayout,
    taa_buf: wgpu::Buffer,
    lin: wgpu::Sampler,
    targets: Option<Targets>,
    parity: usize,
    sample_index: u32,
    history_valid: bool,
    prev_cam: Option<CameraParams>,
    /// this frame: TAA on, its variant, the pixel jitter
    active: bool,
    fast: bool,
    jitter_px: [f32; 2],
    // FXAA console (Medium) and the screen-percentage upscale, pipelines per output format
    fxaa_bgl: wgpu::BindGroupLayout,
    fxaa_sm: wgpu::ShaderModule,
    upscale_sm: wgpu::ShaderModule,
    fxaa_buf: wgpu::Buffer,
    upscale_buf: wgpu::Buffer,
    pipes: std::collections::HashMap<(u8, wgpu::TextureFormat), wgpu::RenderPipeline>,
}

impl TemporalAa {
    pub fn new(device: &wgpu::Device, bgl_obj_skinned: &wgpu::BindGroupLayout, hdr_format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat, skinned_attrs: &[wgpu::VertexAttribute], skinned_stride: u64) -> TemporalAa {
        let (v, f, vf) = (wgpu::ShaderStages::VERTEX, wgpu::ShaderStages::FRAGMENT, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT);
        let e = |binding: u32, visibility: wgpu::ShaderStages, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility, ty, count: None };
        let ub = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let tex = |filterable: bool| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let buf = |label: &str, size: usize| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let sm = |label: &str, src: &str| device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
        let pl = |label: &str, bgls: &[&wgpu::BindGroupLayout]| { let b: Vec<Option<&wgpu::BindGroupLayout>> = bgls.iter().map(|x| Some(*x)).collect(); device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &b, immediate_size: 0 }) };
        // velocity
        let vel_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("velocity bgl"), entries: &[e(0, v, ub)] });
        let vel_buf = buf("velocity view", std::mem::size_of::<VelU>());
        let vel_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("velocity bg"), layout: &vel_bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: vel_buf.as_entire_binding() }] });
        let vel_sm = sm("velocity", fn_live::wgsl!("../shaders/velocity.wgsl"));
        let vel_layout = pl("velocity layout", &[&vel_bgl, bgl_obj_skinned, bgl_obj_skinned]);
        // VELOCITY (2026-10-07): two twins, `vel_pipeline` (vs_skinned, LessEqual + a small bias absorbing pipeline differences: the skinned prims without a
        // CharacterShader owner, whose scene position is not `@invariant`) and `vel_char_pipeline` (vs_skinned_char, `@invariant`, EQUAL against the clipped char
        // pre-pass depth = the masked material's coverage, velocity.wgsl). Both depth-tested against the finished scene depth, no write (VelocityRendering.cpp:
        // CF_DepthNearOrEqual).
        let vel_pipe = |label: &str, vs: &str, fs: &str, compare: wgpu::CompareFunction, bias: i32| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label), layout: Some(&vel_layout),
            vertex: wgpu::VertexState { module: &vel_sm, entry_point: Some(vs), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: skinned_stride, step_mode: wgpu::VertexStepMode::Vertex, attributes: skinned_attrs })] },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(false), depth_compare: Some(compare), stencil: Default::default(),
                bias: wgpu::DepthBiasState { constant: bias, slope_scale: 0., clamp: 0. } }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &vel_sm, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: VELOCITY_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        let vel_pipeline = vel_pipe("velocity (skinned)", "vs_skinned", "fs_velocity", wgpu::CompareFunction::LessEqual, -2);
        let vel_char_pipeline = vel_pipe("velocity (character)", "vs_skinned_char", "fs_velocity_char", wgpu::CompareFunction::Equal, 0);
        // temporal AA
        let taa_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("taa bgl"), entries: &[
            e(0, f, ub), e(1, f, tex(true)), e(2, f, tex(true)), e(3, f, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
            e(4, f, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
            e(5, f, tex(false)), e(6, f, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
            e(7, f, tex(false)), // STORM TAA: responsive-AA mask
        ] });
        let taa_sm = sm("taa", fn_live::wgsl!("../shaders/taa.wgsl"));
        let taa_layout = pl("taa layout", &[&taa_bgl]);
        let taa_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("temporal aa"), layout: Some(&taa_layout),
            vertex: wgpu::VertexState { module: &taa_sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: &taa_sm, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[
                Some(wgpu::ColorTargetState { format: HISTORY_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                Some(wgpu::ColorTargetState { format: hdr_format, blend: None, write_mask: wgpu::ColorWrites::ALL }),
            ] }),
            multiview_mask: None, cache: None,
        });
        let lin = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("taa bilinear clamp"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        let fxaa_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("fxaa console / upscale bgl"), entries: &[e(0, vf, ub), e(1, f, tex(true)), e(2, f, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering))] });
        TemporalAa {
            device: device.clone(), vel_pipeline, vel_char_pipeline, vel_buf, vel_bg, vel_bgl, velocity_target: None, velocity_gen: 0, motion_blur: false, velocity: VelocityState::default(),
            velocity_last_frame: false, velocity_this_frame: false, vp_no_aa: glam::Mat4::IDENTITY, prev_vp_no_aa: glam::Mat4::IDENTITY, taa_pipeline, taa_bgl, taa_buf: buf("taa params", std::mem::size_of::<TaaU>()), lin, targets: None,
            parity: 0, sample_index: 0, history_valid: false, prev_cam: None, active: false, fast: false, jitter_px: [0.; 2],
            fxaa_bgl, fxaa_sm: sm("fxaa console", fn_live::wgsl!("../shaders/fxaa_console.wgsl")), upscale_sm: sm("upscale", fn_live::wgsl!("../shaders/upscale.wgsl")),
            fxaa_buf: buf("fxaa console params", 32), upscale_buf: buf("upscale params", 32), pipes: Default::default(),
        }
    }

    /// Start of a frame: picks this frame's jitter. `taa` = `Some(fast)` while temporal AA is the method. Returns the NDC jitter for the camera.
    pub fn begin_frame(&mut self, taa: Option<bool>, view_w: u32, view_h: u32) -> [f32; 2] {
        match taa {
            Some(fast) => {
                if !self.active { self.history_valid = false; }
                self.active = true; self.fast = fast;
                // OnFrameRenderingSetup(TemporalAASamples): index advanced, then read
                self.sample_index = (self.sample_index + 1) % ue::TEMPORAL_AA_SAMPLES;
                self.jitter_px = jitter_pixels(self.sample_index);
                jitter_ndc(self.jitter_px, view_w, view_h)
            }
            None => { self.active = false; self.history_valid = false; self.sample_index = 0; self.jitter_px = [0.; 2]; [0.; 2] }
        }
    }
    pub fn active(&self) -> bool { self.active }
    /// UE SSAO: `FSceneViewState::GetCurrentTemporalAASampleIndex()` (0 while temporal AA is off: the index is reset above).
    pub fn sample_index(&self) -> u32 { self.sample_index }
    /// REFLECTION APPLY (SSR `PrevFrame`, ScreenSpaceReflections.cpp:512): the previous frame's history (`TemporalAAHistoryRT`) while TAA runs and it is valid.
    pub fn prev_history(&self) -> Option<&wgpu::TextureView> { self.targets.as_ref().filter(|_| self.active && self.history_valid).map(|t| &t.history[self.parity]) }
    /// REFLECTION APPLY / VELOCITY: this frame's scene velocity target, `Some` whenever the "velocity" pass runs ([`Self::velocity_pass`]: velocity is rendered
    /// or temporal AA reads it); 0 = no velocity of its own (reproject with the depth and `ClipToPrevClip`).
    pub fn velocity_view(&self) -> Option<&wgpu::TextureView> { self.velocity_target.as_ref().filter(|_| self.velocity_pass()).map(|t| &t.1) }
    /// VELOCITY: the integrator's motion blur switch (`IsMotionBlurEnabled(View)`, VelocityRendering.cpp:819-838: post processing + the MotionBlur show flag +
    /// MotionBlurAmount / MotionBlurMax > 0.001 + r.MotionBlurQuality > 0 + not a scene capture). Temporal AA, DFAO and the SSR temporal pass are added by the renderer.
    pub fn set_velocity_wanted(&mut self, motion_blur: bool) { self.motion_blur = motion_blur; }
    /// VELOCITY: this frame's velocity state (valid after [`Self::begin_velocity`]).
    pub fn velocity_state(&self) -> VelocityState { self.velocity }
    /// VELOCITY: the "velocity" pass runs this frame (velocity rendered, or temporal AA needs the cleared target).
    pub fn velocity_pass(&self) -> bool { self.velocity.render || self.active }
    /// VELOCITY: primitives draw their velocity this frame (rendered and no camera cut).
    pub fn object_velocity(&self) -> bool { self.velocity.render && !self.velocity.camera_cut }
    /// VELOCITY: the previous object transforms / bones are last frame's (the velocity pass ran last frame); otherwise every producer takes its current ones.
    pub fn velocity_history_valid(&self) -> bool { self.velocity_last_frame }
    /// VELOCITY: group of `VelU` (jittered / unjittered view-projection, previous unjittered view-projection; vertex stage) for other velocity pipelines.
    pub fn velocity_bind_group_layout(&self) -> &wgpu::BindGroupLayout { &self.vel_bgl }
    pub fn velocity_bind_group(&self) -> &wgpu::BindGroup { &self.vel_bg }
    /// VELOCITY: this frame's unjittered view-projection and the previous one (reset to the current one on `bPrevTransformsReset`).
    pub fn view_proj_no_aa(&self) -> (glam::Mat4, glam::Mat4) { (self.vp_no_aa, self.prev_vp_no_aa) }

    /// VELOCITY: decides `ShouldRenderVelocities` (VelocityRendering.cpp:855-875) and the previous view of this frame. Call every frame after [`Self::begin_frame`]
    /// with the jittered view camera, before the producers / consumers prepare. `dfao` = distance-field AO runs, `ssr_temporal` = `IsSSRTemporalPassRequired`.
    /// Camera cut (`View.bCameraCut`) = no previous view (first frame, or [`Self::reset`]); the previous matrices are also reset on a large camera move
    /// (`bPrevTransformsReset`, SceneVisibility.cpp:2296-2306), which is not a cut (object motion is still written).
    pub fn begin_velocity(&mut self, queue: &wgpu::Queue, cam: &CameraParams, dfao: bool, ssr_temporal: bool) {
        let cut = self.prev_cam.is_none();
        let reset = match &self.prev_cam { Some(p) => is_large_camera_movement(cam, p), None => true };
        let mut prev = match &self.prev_cam { Some(p) if !reset => *p, _ => { let mut c = *cam; c.jitter = [0.; 2]; c } };
        prev.width = cam.width; prev.height = cam.height;
        self.vp_no_aa = cam.proj_matrix_no_aa() * cam.view_matrix();
        self.prev_vp_no_aa = prev.proj_matrix_no_aa() * prev.view_matrix();
        let mut c = *cam; c.jitter = [0.; 2];
        self.prev_cam = Some(c);
        let taa = self.active && !cut;
        self.velocity = VelocityState { render: self.motion_blur || taa || dfao || ssr_temporal, camera_cut: cut, prev_reset: reset };
        self.velocity_last_frame = self.velocity_this_frame;
        self.velocity_this_frame = self.velocity_pass();
        if !self.velocity_pass() { return; }
        let size = (cam.width.max(1), cam.height.max(1));
        if self.velocity_target.as_ref().map(|t| t.0) != Some(size) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: Some("scene velocity"), size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1,
                dimension: wgpu::TextureDimension::D2, format: VELOCITY_FORMAT, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
            self.velocity_gen += 1;
            self.velocity_target = Some((size, tex.create_view(&Default::default()), self.velocity_gen));
        }
        queue.write_buffer(&self.vel_buf, 0, bytemuck::bytes_of(&VelU {
            view_proj: (cam.proj_matrix() * cam.view_matrix()).to_cols_array_2d(), view_proj_no_aa: self.vp_no_aa.to_cols_array_2d(), prev_view_proj_no_aa: self.prev_vp_no_aa.to_cols_array_2d(),
        }));
    }
    /// This frame's pixel jitter (`TemporalJitterPixels`).
    pub fn jitter_px(&self) -> [f32; 2] { self.jitter_px }
    /// Forget the history (camera cut).
    pub fn reset(&mut self) { self.history_valid = false; self.prev_cam = None; }

    /// Targets + uniforms of this frame (`cam` = the jittered render camera of the view size).
    pub fn prepare(&mut self, queue: &wgpu::Queue, cam: &CameraParams, depth_view: &wgpu::TextureView, depth_gen: u64, exposure_buf: &wgpu::Buffer) {
        if !self.active { return; }
        let size = (cam.width.max(1), cam.height.max(1));
        if self.targets.as_ref().map(|t| t.size) != Some(size) {
            let d = &self.device;
            let mk = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| d.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] });
            let input = mk("taa input (scene colour copy)", crate::renderer::HDR_FORMAT, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
            let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            let h = [mk("taa history a", HISTORY_FORMAT, rt), mk("taa history b", HISTORY_FORMAT, rt)];
            let responsive = mk("taa responsive aa mask", RESPONSIVE_MASK_FORMAT, rt).create_view(&Default::default()); // STORM TAA
            self.targets = Some(Targets { size, input_view: input.create_view(&Default::default()), input, history: [h[0].create_view(&Default::default()), h[1].create_view(&Default::default())], responsive, bgs: None });
            self.history_valid = false;
        }
        // VELOCITY: the scene velocity target is owned by `begin_velocity` (it exists whenever TAA runs)
        let Some((_, velocity_view, vgen)) = self.velocity_target.as_ref() else { return };
        let t = self.targets.as_mut().unwrap();
        if t.bgs.as_ref().map(|b| b.1) != Some((depth_gen, *vgen)) {
            let mk = |prev: &wgpu::TextureView| self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("taa bg"), layout: &self.taa_bgl, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.taa_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&t.input_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(prev) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.lin) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(depth_view) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(velocity_view) },
                wgpu::BindGroupEntry { binding: 6, resource: exposure_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&t.responsive) }, // STORM TAA
            ] });
            t.bgs = Some(([mk(&t.history[0]), mk(&t.history[1])], (depth_gen, *vgen)));
        }
        // previous view (bPrevTransformsReset on the first frame or a large move): computed by `begin_velocity` (VELOCITY)
        let (vp, prev_vp) = (self.vp_no_aa, self.prev_vp_no_aa);
        let pw = plus_weights(self.jitter_px);
        // VELOCITY DEBUG (verification aid, not UE): `FN_TAA_DEBUG=1` shows the velocity buffer instead of the resolve (red = a primitive wrote velocity,
        // green = |own velocity| in pixels / 10, blue = camera-only reprojection length in pixels / 10);
        // `FN_TAA_DEBUG=2`: bands of the own velocity only (red > 0.5 px, + green > 2 px, + blue > 8 px; black = none or < 0.5 px)
        static DEBUG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let debug = *DEBUG.get_or_init(|| fn_live::var("FN_TAA_DEBUG").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0).min(2));
        let flags = (if self.fast { 0 } else { 1 }) | (if self.history_valid { 0 } else { 2 }) | (if debug >= 1 { 4 } else { 0 }) | (if debug == 2 { 8 } else { 0 });
        queue.write_buffer(&self.taa_buf, 0, bytemuck::bytes_of(&TaaU {
            plus0: [pw[0], pw[1], pw[2], pw[3]], plus1: [pw[4], ue::TEMPORAL_AA_CURRENT_FRAME_WEIGHT, 1.0, flags as f32],
            clip_to_prev: (prev_vp * vp.inverse()).to_cols_array_2d(),
            viewport: [size.0 as f32, size.1 as f32, 1. / size.0 as f32, 1. / size.1 as f32],
        }));
    }

    /// The velocity pass (cleared to 0 = no velocity); the caller binds group 1 / 2 (current / previous object + bones) and draws the skinned models.
    /// STORM TAA (2026-10-07): the responsive-AA mask pass of a TAA frame (UE: the stencil cleared at the start of translucency, SceneRenderTargets.cpp:1374,
    /// set by the responsive translucent draws): cleared to 1 (untagged), scene depth read-only; the caller records the FX mask twins (fx.rs `draw_responsive_aa`).
    pub fn begin_responsive_pass<'a>(&'a self, enc: &'a mut wgpu::CommandEncoder, depth_view: &'a wgpu::TextureView) -> Option<wgpu::RenderPass<'a>> {
        let view = self.targets.as_ref().filter(|_| self.active).map(|t| &t.responsive)?;
        Some(enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("taa responsive aa mask"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::WHITE), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth_view, depth_ops: None, stencil_ops: None }),
            timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        }))
    }
    pub fn begin_velocity_pass<'a>(&'a self, enc: &'a mut wgpu::CommandEncoder, depth_view: &'a wgpu::TextureView) -> Option<wgpu::RenderPass<'a>> {
        // VELOCITY: whenever velocity is rendered or TAA reads it (cleared to 0 = "use static background velocity", VelocityRendering.cpp:739)
        let view = self.velocity_target.as_ref().filter(|_| self.velocity_pass()).map(|t| &t.1)?;
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("velocity"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth_view, depth_ops: None, stencil_ops: None }),
            timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(&self.vel_pipeline);
        pass.set_bind_group(0, &self.vel_bg, &[]);
        Some(pass)
    }

    /// VELOCITY (2026-10-07): switches the velocity pass between the plain skinned twin and the BR character parts' one (`char` = the prim has a CharacterShader
    /// owner, renderer `char_bg`); group 0 stays bound (same layout).
    pub fn set_velocity_char<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, char: bool) { pass.set_pipeline(if char { &self.vel_char_pipeline } else { &self.vel_pipeline }); }

    /// Copy of the scene colour, then the TAA pass: history[1 - parity] and the scene colour (mip 0 of `hdr`) are written.
    pub fn encode_resolve(&self, enc: &mut wgpu::CommandEncoder, hdr: &wgpu::Texture, hdr_mip0: &wgpu::TextureView) {
        let Some(t) = self.targets.as_ref().filter(|_| self.active) else { return };
        let Some((bgs, _)) = &t.bgs else { return };
        enc.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo { texture: hdr, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyTextureInfo { texture: &t.input, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::Extent3d { width: t.size.0, height: t.size.1, depth_or_array_layers: 1 },
        );
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("temporal aa"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment { view: &t.history[1 - self.parity], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } }),
                Some(wgpu::RenderPassColorAttachment { view: hdr_mip0, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } }),
            ],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
        });
        pass.set_pipeline(&self.taa_pipeline);
        pass.set_bind_group(0, &bgs[self.parity], &[]);
        pass.draw(0..3, 0..1);
    }
    /// After the frame was submitted: the written history becomes the previous one.
    pub fn end_frame(&mut self) { if self.active && self.targets.is_some() { self.parity = 1 - self.parity; self.history_valid = true; } }

    pub fn prewarm(&mut self, output: wgpu::TextureFormat, ldr: wgpu::TextureFormat) {
        for f in [output, ldr] { let _ = self.pipe(0, f); }
        let _ = self.pipe(1, output);
    }
    fn pipe(&mut self, kind: u8, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        if let Some(p) = self.pipes.get(&(kind, format)) { return p.clone(); }
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("aa / upscale layout"), bind_group_layouts: &[Some(&self.fxaa_bgl)], immediate_size: 0 });
        let sm = if kind == 0 { &self.fxaa_sm } else { &self.upscale_sm };
        let p = self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(if kind == 0 { "fxaa console" } else { "upscale" }), layout: Some(&layout),
            vertex: wgpu::VertexState { module: sm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            fragment: Some(wgpu::FragmentState { module: sm, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
            multiview_mask: None, cache: None,
        });
        self.pipes.insert((kind, format), p.clone());
        p
    }
    /// Pipeline + bind group of the Medium FXAA (console) pass reading `src` (`src_size` texels) into a `format` target.
    pub fn fxaa_console(&mut self, queue: &wgpu::Queue, src: &wgpu::TextureView, src_size: (u32, u32), format: wgpu::TextureFormat, sampler: &wgpu::Sampler) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
        queue.write_buffer(&self.fxaa_buf, 0, bytemuck::bytes_of(&Vec4U { a: [1. / src_size.0.max(1) as f32, 1. / src_size.1.max(1) as f32, 0., 0.], b: [0.; 4] }));
        let bg = self.bg(&self.fxaa_buf, src, sampler);
        (self.pipe(0, format), bg)
    }
    /// Pipeline + bind group of the upscale pass (`method` = r.Upscale.Quality) from `src` (`src_size`) into a `format` target.
    pub fn upscale(&mut self, queue: &wgpu::Queue, src: &wgpu::TextureView, src_size: (u32, u32), method: i32, format: wgpu::TextureFormat, sampler: &wgpu::Sampler) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
        let (w, h) = (src_size.0.max(1) as f32, src_size.1.max(1) as f32);
        queue.write_buffer(&self.upscale_buf, 0, bytemuck::bytes_of(&Vec4U { a: [w, h, 1. / w, 1. / h], b: [method.clamp(1, 3) as f32, 0., 0., 0.] }));
        let bg = self.bg(&self.upscale_buf, src, sampler);
        (self.pipe(1, format), bg)
    }
    fn bg(&self, buf: &wgpu::Buffer, src: &wgpu::TextureView, sampler: &wgpu::Sampler) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("aa / upscale bg"), layout: &self.fxaa_bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
        ] })
    }
}

/// `GrainRandomFull` of frame `FrameIndexMod8` (PostProcessTonemap.h:20-38: GrainHalton(frame & 1023, 2 / 3)).
pub fn grain_random(frame_index_mod8: u32) -> [f32; 2] { [halton(frame_index_mod8 & 1023, 2), halton(frame_index_mod8 & 1023, 3)] }

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    #[test]
    fn halton_and_jitter_follow_scene_visibility() {
        assert_eq!(halton(1, 2), 0.5);
        assert!((halton(2, 3) - 2. / 3.).abs() < 1e-6);
        // 8 samples, all inside the [-0.5, 0.5] window, mean near the pixel centre
        let j: Vec<_> = (0..8).map(jitter_pixels).collect();
        assert!(j.iter().all(|p| p[0].abs() <= 0.5 + 1e-5 && p[1].abs() <= 0.5 + 1e-5), "{j:?}");
        let m = j.iter().fold([0f32; 2], |a, p| [a[0] + p[0] / 8., a[1] + p[1] / 8.]);
        assert!(m[0].abs() < 0.2 && m[1].abs() < 0.2, "{m:?}");
        // NDC: y flips (pixel y down, NDC y up)
        assert_eq!(jitter_ndc([0.5, 0.5], 100, 50), [0.01, -0.02]);
    }
    #[test]
    fn plus_weights_are_the_gaussian_of_the_jittered_offsets() {
        let w = plus_weights([0., 0.]);
        assert!((w.iter().sum::<f32>() - 1.).abs() < 1e-6);
        // centred: centre exp(0) = 1, sides exp(-2.29)
        let side = (-2.29f32).exp();
        assert!((w[2] - 1. / (1. + 4. * side)).abs() < 1e-6 && (w[0] - w[3]).abs() < 1e-7);
        // jitter towards +x favours the right tap
        let w = plus_weights([0.4, 0.]);
        assert!(w[3] > w[1]);
    }
    #[test]
    fn large_camera_moves_reset_the_previous_view() {
        let a = CameraParams::from_yaw_pitch(Vec3::ZERO, 0., 0., 64, 64);
        assert!(!is_large_camera_movement(&CameraParams::from_yaw_pitch(Vec3::new(1., 0., 0.), 10., 0., 64, 64), &a));
        assert!(is_large_camera_movement(&CameraParams::from_yaw_pitch(Vec3::ZERO, 50., 0., 64, 64), &a));
        assert!(is_large_camera_movement(&CameraParams::from_yaw_pitch(Vec3::new(101., 0., 0.), 0., 0., 64, 64), &a));
    }
    #[test]
    fn clip_to_prev_is_identity_for_a_still_camera_and_drops_the_jitter() {
        let mut c = CameraParams::from_yaw_pitch(Vec3::new(3., 2., 1.), 20., -5., 320, 180);
        c.jitter = jitter_ndc(jitter_pixels(3), 320, 180);
        let vp = c.proj_matrix_no_aa() * c.view_matrix();
        let m = vp * vp.inverse();
        assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-4));
        assert!(!(c.proj_matrix() * c.view_matrix()).abs_diff_eq(vp, 1e-6));
    }
    /// GRASS MOTION: the jittered View buffer moves a point by exactly the jitter on screen, its SVPositionToTranslatedWorld still maps the (jittered) pixel
    /// back to the point, and ViewToClip[2].xy (what the velocity PS subtracts) holds the jitter.
    #[test]
    fn view_jitter_moves_the_projection_and_keeps_sv_position_consistent() {
        use ue_view_slot::*;
        let (w, h) = (1280.0f32, 720.0f32);
        // a row-vector projection (row i = slot i): x' = 1.2 x + 0.1 z, y' = 2.0 y, z' = 0.5 z + 7, w' = z
        let rows = [[1.2f32, 0., 0., 0.], [0., 2.0, 0., 0.], [0.1, 0., 0.5, 1.], [0., 0., 7., 0.]];
        let g = Mat4::from_cols_array_2d(&rows); // columns = the rows: g * p = p * M
        let ginv = g.inverse();
        let minv: [[f32; 4]; 4] = std::array::from_fn(|i| ginv.col(i).to_array());
        let s = [[2. / w, 0., 0., 0.], [0., -2. / h, 0., 0.], [0., 0., 1., 0.], [-1., 1., 0., 1.]];
        let svp: [[f32; 4]; 4] = std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| s[i][k] * minv[k][j]).sum()));
        let mut v = vec![[0f32; 4]; 178];
        v[0..4].copy_from_slice(&rows); v[4..8].copy_from_slice(&rows); v[40..44].copy_from_slice(&svp); v[VIEW_SIZE_AND_INV_SIZE] = [w, h, 1. / w, 1. / h];
        let mut b: Vec<u8> = v.iter().flat_map(|r| r.iter().flat_map(|f| f.to_le_bytes())).collect();
        let j = jitter_ndc(jitter_pixels(3), w as u32, h as u32);
        // a re-prepared frame (bytes already jittered by another frame's jitter) ends with exactly this frame's jitter
        apply_view_jitter(&mut b, jitter_ndc(jitter_pixels(6), w as u32, h as u32));
        apply_view_jitter(&mut b, j);
        let once = b.clone();
        apply_view_jitter(&mut b, j);
        assert_eq!(b, once);
        let rd =|slot: usize| -> [f32; 4] { std::array::from_fn(|k| f32::from_le_bytes(b[slot * 16 + k * 4..slot * 16 + k * 4 + 4].try_into().unwrap())) };
        let mul = |p: [f32; 4], at: usize| -> [f32; 4] { let mut o = [0f32; 4]; for i in 0..4 { let r = rd(at + i); for k in 0..4 { o[k] += p[i] * r[k]; } } o };
        let p = [30.0f32, -12.0, 400.0, 1.0];
        let c0 = { let mut o = [0f32; 4]; for i in 0..4 { for k in 0..4 { o[k] += p[i] * rows[i][k]; } } o };
        let c1 = mul(p, TRANSLATED_WORLD_TO_CLIP);
        for k in 0..2 { assert!((c1[k] / c1[3] - (c0[k] / c0[3] + j[k])).abs() < 1e-5); }
        assert_eq!(mul(p, WORLD_TO_CLIP), c1);
        let (px, py) = ((c1[0] / c1[3] + 1.) * 0.5 * w, (1. - c1[1] / c1[3]) * 0.5 * h);
        let q = mul([px, py, c1[2] / c1[3], 1.], SV_POSITION_TO_TRANSLATED_WORLD);
        for k in 0..3 { assert!((q[k] / q[3] - p[k]).abs() < 1e-2, "{q:?}"); }
        assert_eq!(&rd(VIEW_TO_CLIP + 2)[..2], &j[..]);
    }
    #[test]
    fn view_size_moves_sv_position_to_the_rendered_view() {
        use ue_view_slot::*;
        let (w, h) = (1600.0f32, 900.0f32);
        let (vw, vh) = (800u32, 452u32);
        let rows = [[1.2f32, 0., 0., 0.], [0., 2.0, 0., 0.], [0.1, 0., 0.5, 1.], [0., 0., 7., 0.]];
        let g = Mat4::from_cols_array_2d(&rows);
        let ginv = g.inverse();
        let minv: [[f32; 4]; 4] = std::array::from_fn(|i| ginv.col(i).to_array());
        let s = [[2. / w, 0., 0., 0.], [0., -2. / h, 0., 0.], [0., 0., 1., 0.], [-1., 1., 0., 1.]];
        let svp: [[f32; 4]; 4] = std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| s[i][k] * minv[k][j]).sum()));
        let mut v = vec![[0f32; 4]; 178];
        v[0..4].copy_from_slice(&rows); v[4..8].copy_from_slice(&rows); v[40..44].copy_from_slice(&svp); v[VIEW_SIZE_AND_INV_SIZE] = [w, h, 1. / w, 1. / h];
        let mut b: Vec<u8> = v.iter().flat_map(|r| r.iter().flat_map(|f| f.to_le_bytes())).collect();
        apply_view_size(&mut b, (vw, vh));
        let once = b.clone();
        apply_view_size(&mut b, (vw, vh));
        assert_eq!(b, once);
        let j = jitter_ndc(jitter_pixels(2), vw, vh);
        apply_view_jitter(&mut b, j);
        let rd = |slot: usize| -> [f32; 4] { std::array::from_fn(|k| f32::from_le_bytes(b[slot * 16 + k * 4..slot * 16 + k * 4 + 4].try_into().unwrap())) };
        let mul = |p: [f32; 4], at: usize| -> [f32; 4] { let mut o = [0f32; 4]; for i in 0..4 { let r = rd(at + i); for k in 0..4 { o[k] += p[i] * r[k]; } } o };
        assert_eq!(rd(VIEW_SIZE_AND_INV_SIZE), [vw as f32, vh as f32, 1. / vw as f32, 1. / vh as f32]);
        let p = [30.0f32, -12.0, 400.0, 1.0];
        let c = mul(p, TRANSLATED_WORLD_TO_CLIP);
        let (px, py) = ((c[0] / c[3] + 1.) * 0.5 * vw as f32, (1. - c[1] / c[3]) * 0.5 * vh as f32);
        let q = mul([px, py, c[2] / c[3], 1.], SV_POSITION_TO_TRANSLATED_WORLD);
        for k in 0..3 { assert!((q[k] / q[3] - p[k]).abs() < 1e-2, "{q:?}"); }
    }
}
