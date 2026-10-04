//! Parameters of the post effects, with the 1.8 Athena Day values and their provenance.
//!
//! Units: every DOF / motion-blur distance is in CENTIMETRES (the unit of the serialized values). `DepthTransform` linearises in the
//! renderer's own unit and `PostFxParams::cm_per_unit` converts (100 for metres).
//!
//! Provenance tags used in comments: **RECOVERED** = a tagged value in the 1.8 cooked package (`TODM_A` + `TODM_A_Child_Athena`, merged,
//! see `reports/postfx-evidence.json`); **ENGINE_DEFAULT** = the field has `bOverride_X = true` but no tagged value, so it equals the UE 4.16
//! C++ constructor default (value recalled from the engine source); **ASSUMED** = host-side (C++) maths that is not in any shader.
use glam::Mat4;

/// `ConvertFromDeviceZ`: `z * x + y + 1 / (z * z_ - w)` (UE `View.InvDeviceZToWorldZTransform`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthTransform {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl DepthTransform {
    /// Standard (non-reversed) 0..1 perspective depth with finite near/far: `linear = n*f / (f - d*(f-n))`.
    pub fn from_perspective(near: f32, far: f32) -> Self {
        // 1/linear = d * (n - f) / (n f) + 1/n  =>  z = (n-f)/(nf), w = -1/n
        DepthTransform { x: 0.0, y: 0.0, z: (near - far) / (near * far), w: -1.0 / near }
    }
    pub fn linearize(&self, d: f32) -> f32 { d * self.x + self.y + 1.0 / (d * self.z - self.w) }
    /// Inverse of `linearize` for the finite-perspective transform (x = y = 0); used by tests to build a depth buffer from view depths.
    pub fn device_z_from_linear(&self, lin: f32) -> f32 { (1.0 / lin + self.w) / self.z }
}

/// Gaussian depth of field (`DepthOfFieldMethod == DOFM_Gaussian`), far/near layers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DofParams {
    pub enabled: bool,
    /// `DepthOfFieldFocalDistance` (Day: 15000 cm, RECOVERED child).
    pub focal_distance: f32,
    /// `DepthOfFieldFocalRegion` (Day: 0, ENGINE_DEFAULT).
    pub focal_region: f32,
    /// `DepthOfFieldNearTransitionRegion` (Day: 0, RECOVERED parent). Unused while `near_blur_size < 0.01`.
    pub near_transition: f32,
    /// `DepthOfFieldFarTransitionRegion` (Day: 50000 cm, RECOVERED child).
    pub far_transition: f32,
    /// `DepthOfFieldNearBlurSize` in percent of the view width (Day: 0, RECOVERED parent => near layer off).
    pub near_blur_size: f32,
    /// `DepthOfFieldFarBlurSize` in percent of the view width (Day: 0.02, RECOVERED parent).
    pub far_blur_size: f32,
    /// `DepthOfFieldSkyFocusDistance` (Day: 200000 cm, RECOVERED parent). `<= 0` disables the sky rule (UE passes 1e8).
    pub sky_focus_distance: f32,
}

impl DofParams {
    /// Gaussian DOF runs when either layer is at least 0.01 (UE `PostProcessing.cpp`, ASSUMED recalled threshold).
    pub fn near_active(&self) -> bool { self.near_blur_size >= 0.01 }
    pub fn far_active(&self) -> bool { self.far_blur_size >= 0.01 }
}

/// Tiled motion blur (4.16 `FPostProcessVelocityFlattenCS` / `GatherCS` / `FPostProcessMotionBlurPS<4>`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlurParams {
    pub enabled: bool,
    /// `MotionBlurAmount` (Day: 0.125, RECOVERED child).
    pub amount: f32,
    /// `MotionBlurMax` in percent of the view width (Day: 2.0, RECOVERED child).
    pub max_percent: f32,
    /// `MotionBlurPerObjectSize` (Day: 0.5 ENGINE_DEFAULT, not overridden by the volume): per-object streaks are scaled by this when a
    /// velocity buffer is bound (ASSUMED use).
    pub per_object_size: f32,
}

/// UE bloom: six Gaussians of `size` percent of the screen width, tinted, summed and added in the tonemapper times `intensity`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomParams {
    pub enabled: bool,
    /// `BloomIntensity` (ENGINE_DEFAULT 0.675).
    pub intensity: f32,
    /// `BloomThreshold` (-1.0: exe ctor default, recovered; the Day volume overrides it without a tagged value).
    pub threshold: f32,
    /// `BloomSizeScale` (Day: 1.0, RECOVERED parent; engine default 4.0).
    pub size_scale: f32,
    /// `Bloom1..6Size` (Day: 0.3, 1, 2, 10, 30 ENGINE_DEFAULT; Bloom6 = 0 RECOVERED parent).
    pub sizes: [f32; 6],
    /// `Bloom1..6Tint` (Day: 0.3465, 0.138, **0.5** (RECOVERED), 0.066, 0.066 ENGINE_DEFAULT; Bloom6 = **0** RECOVERED).
    pub tints: [[f32; 3]; 6],
}

/// Per-frame camera data the passes need.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewParams {
    pub width: u32,
    pub height: u32,
    /// Three rows `(rx, ry, rz, _)`: ray direction of pixel with NDC `(x, y)` (y up) is `x*rows[0] + y*rows[1] + rows[2]` in a basis whose
    /// `.z` is world up (UE: `View.ScreenToTranslatedWorld` columns). Only used by the DOF sky rule.
    pub screen_to_world_rows: [[f32; 4]; 3],
    /// Maps this frame's clip space (after perspective divide, `(ndc.x, ndc.y, device_z, 1)`) to the previous frame's clip space
    /// (UE `View.ClipToPrevClip`). Identity = no camera motion.
    pub clip_to_prev_clip: Mat4,
}

/// Everything `PostFx::encode` needs about the frame besides textures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PostFxParams {
    pub dof: DofParams,
    pub motion_blur: MotionBlurParams,
    pub bloom: BloomParams,
    pub depth: DepthTransform,
    pub view: ViewParams,
    /// Eye-adaptation exposure scale used by the bloom threshold (`FPostProcessBloomSetupPS` multiplies luma by it).
    pub exposure: f32,
    /// Centimetres per renderer world unit: 1.0 for UE units, 100.0 for a renderer in metres. Every DOF/motion-blur distance below is in cm.
    pub cm_per_unit: f32,
}

/// Fortnite 1.8 Athena Day post-process values (merged `TODM_A` + `TODM_A_Child_Athena`), distances in centimetres.
pub fn athena_day_cm() -> (DofParams, MotionBlurParams, BloomParams) {
    let dof = DofParams {
        enabled: true,
        focal_distance: 15000.0,
        focal_region: 0.0,
        near_transition: 0.0,
        far_transition: 50000.0,
        near_blur_size: 0.0,
        far_blur_size: 0.019999999552965164,
        sky_focus_distance: 200000.0,
    };
    let mb = MotionBlurParams { enabled: true, amount: 0.125, max_percent: 2.0, per_object_size: 0.5 };
    let bloom = BloomParams {
        enabled: true,
        intensity: 0.675,
        threshold: -1.0, // exe FPostProcessSettings ctor default (override flag, no tag in the Day volume)
        size_scale: 1.0,
        sizes: [0.3, 1.0, 2.0, 10.0, 30.0, 0.0],
        tints: [[0.3465; 3], [0.138; 3], [0.5; 3], [0.066; 3], [0.066; 3], [0.0; 3]],
    };
    (dof, mb, bloom)
}

impl PostFxParams {
    /// Athena Day parameters; `cm_per_unit` = 100 for a renderer working in metres (`depth` built for metres), 1 for UE units.
    pub fn athena_day(depth: DepthTransform, view: ViewParams, exposure: f32, cm_per_unit: f32) -> Self {
        let (dof, motion_blur, bloom) = athena_day_cm();
        PostFxParams { dof, motion_blur, bloom, depth, view, exposure, cm_per_unit }
    }
}

impl ViewParams {
    /// Pinhole camera looking along `forward` with `up_world` = world up, horizontal tangent `tan_x`, vertical `tan_y`.
    /// The rows are expressed in world axes with `.z` = world up when `up_world` is +Z; for a Y-up renderer pass the world axes
    /// permuted so `.z` is up (`ViewParams::from_basis` takes the already-permuted basis).
    pub fn from_basis(width: u32, height: u32, right: [f32; 3], up: [f32; 3], forward: [f32; 3], tan_x: f32, tan_y: f32, clip_to_prev_clip: Mat4) -> Self {
        let r = |v: [f32; 3], s: f32| [v[0] * s, v[1] * s, v[2] * s, 0.0];
        ViewParams { width, height, screen_to_world_rows: [r(right, tan_x), r(up, tan_y), r(forward, 1.0)], clip_to_prev_clip }
    }
}
