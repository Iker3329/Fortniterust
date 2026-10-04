//! Cross-checks and the "previously assumed" values of the Rust code, so every recovered number can be compared.

/// Console variables whose UE 4.16 source default is known independently of this tool (used to validate the extraction method).
pub const KNOWN_INT_CVARS: &[(&str, i64)] = &[
    ("r.Shadow.MaxResolution", 2048),
    ("r.Shadow.MaxCSMResolution", 2048),
    ("r.Shadow.MinResolution", 32),
    ("r.Shadow.FadeResolution", 64),
    ("r.Shadow.PreShadowFadeResolution", 16),
    ("r.Shadow.CSM.MaxCascades", 10),
    ("r.TonemapperFilm", 1),
    ("r.BloomQuality", 5),
    ("r.DepthOfFieldQuality", 2),
    ("r.MotionBlurQuality", 4),
    ("r.EyeAdaptationQuality", 2),
    ("r.SceneColorFormat", 4),
    ("r.SSR.Quality", 3),
    ("r.LightShaftQuality", 1),
    ("r.Fog", 1),
    ("r.TemporalAASamples", 8),
    ("r.DefaultFeature.Bloom", 1),
    ("r.DefaultFeature.AutoExposure", 1),
    ("r.DefaultFeature.MotionBlur", 1),
    ("r.DefaultFeature.AmbientOcclusion", 1),
    ("r.DefaultFeature.AntiAliasing", 2),
];
pub const KNOWN_FLOAT_CVARS: &[(&str, f32)] = &[
    ("r.Shadow.RadiusThreshold", 0.03),
    ("r.Shadow.TexelsPerPixel", 1.27324),
    ("r.ScreenPercentage", 100.0),
    ("r.Gamma", 1.0),
    ("r.AmbientOcclusionRadiusScale", 1.0),
    ("r.AmbientOcclusionMaxQuality", 100.0),
    ("r.TonemapperGamma", 0.0),
    ("r.Tonemapper.Sharpen", 0.0),
];

/// What the Rust workspace currently assumes for a value the engine decides natively.
pub struct Assumed {
    /// reflected name (`FPostProcessSettings` field or `Class.Property`)
    pub name: &'static str,
    /// assumed value in the units of the *engine* property (cm for lengths)
    pub value: f32,
    /// where the Rust code uses it (file:line, constant)
    pub used_in: &'static [&'static str],
    pub note: &'static str,
}

pub const ASSUMED_PPS: &[Assumed] = &[
    Assumed { name: "AutoExposureMaxBrightness", value: 2.0, used_in: &["crates/fn-render/src/tonemap.rs:19 AUTO_EXPOSURE_MAX (-> renderer.rs:713 MeterParams.max_brightness)"], note: "README 'ASSUMED engine default'" },
    Assumed { name: "AutoExposureSpeedUp", value: 3.0, used_in: &["crates/fn-render/src/tonemap.rs:29 ADAPT_UP"], note: "'UE eye-adaptation speeds (defaults, unproven)'" },
    Assumed { name: "AutoExposureSpeedDown", value: 1.0, used_in: &["crates/fn-render/src/tonemap.rs:30 ADAPT_DOWN"], note: "'UE eye-adaptation speeds (defaults, unproven)'" },
    Assumed { name: "BloomIntensity", value: 0.675, used_in: &["crates/fn-render/src/tonemap.rs:25 BLOOM_INTENSITY", "crates/fn-postfx/src/params.rs:128 athena_day_cm().bloom.intensity"], note: "README assumed 0.675" },
    Assumed { name: "BloomThreshold", value: 1.0, used_in: &["crates/fn-render/src/tonemap.rs:26 BLOOM_THRESHOLD (renderer.rs:701)", "crates/fn-postfx/src/params.rs:76/129 BloomParams.threshold (Day volume overrides it without a tagged value, i.e. = ctor default)"], note: "ENGINE_DEFAULT 1.0 recalled from source" },
    Assumed { name: "VignetteIntensity", value: 0.4, used_in: &["crates/fn-render/src/tonemap.rs:27 VIGNETTE_INTENSITY"], note: "README assumed 0.4" },
    Assumed { name: "AmbientOcclusionRadius", value: 40.0, used_in: &["crates/fn-render/src/tonemap.rs:24 AO_RADIUS_M = 0.4 (m) (-> renderer.rs:703 ao[1], post.wgsl:75)"], note: "README assumed 40 cm" },
    Assumed { name: "FilmBlackClip", value: 0.0, used_in: &["crates/fn-render/src/tonemap.rs:16 FILM_BLACK_CLIP"], note: "README assumed 0" },
    Assumed { name: "FilmWhiteClip", value: 0.04, used_in: &["crates/fn-render/src/tonemap.rs:17 FILM_WHITE_CLIP"], note: "README assumed 0.04" },
    Assumed { name: "WhiteTemp", value: 6500.0, used_in: &["crates/fn-render/src/tonemap.rs:64 white_balance_matrix reference white"], note: "'UE's WhiteTemp default is 6500 = identity'" },
    Assumed { name: "BloomSizeScale", value: 4.0, used_in: &["crates/fn-postfx/src/params.rs:78 (comment only; Day value 1.0 is serialized)"], note: "'engine default 4.0'" },
    Assumed { name: "Bloom1Size", value: 0.3, used_in: &["crates/fn-postfx/src/params.rs:131 sizes[0]"], note: "ENGINE_DEFAULT" },
    Assumed { name: "Bloom2Size", value: 1.0, used_in: &["crates/fn-postfx/src/params.rs:131 sizes[1]"], note: "ENGINE_DEFAULT" },
    Assumed { name: "Bloom3Size", value: 2.0, used_in: &["crates/fn-postfx/src/params.rs:131 sizes[2]"], note: "ENGINE_DEFAULT" },
    Assumed { name: "Bloom4Size", value: 10.0, used_in: &["crates/fn-postfx/src/params.rs:131 sizes[3]"], note: "ENGINE_DEFAULT" },
    Assumed { name: "Bloom5Size", value: 30.0, used_in: &["crates/fn-postfx/src/params.rs:131 sizes[4]"], note: "ENGINE_DEFAULT" },
    Assumed { name: "DepthOfFieldFocalRegion", value: 0.0, used_in: &["crates/fn-postfx/src/params.rs:37/118 focal_region"], note: "ENGINE_DEFAULT" },
    Assumed { name: "MotionBlurPerObjectSize", value: 0.5, used_in: &["crates/fn-postfx/src/params.rs:65/125 per_object_size"], note: "ENGINE_DEFAULT" },
];

pub const ASSUMED_CLASS: &[(&str, &str, f32, &[&str], &str)] = &[
    ("UDirectionalLightComponent", "DynamicShadowCascades", 3.0, &["crates/fn-render/src/lighting.rs:15 CASCADE_COUNT"], "'3 is the assumed default (unproven)'"),
    ("UDirectionalLightComponent", "LightSourceAngle", 0.5357, &["crates/fn-render/src/lighting.rs:24 LIGHT_SOURCE_ANGLE_DEG (-> renderer.rs:685 PCF penumbra)"], "README: 'UE 4.18 default assumed from memory'"),
    ("UDirectionalLightComponent", "CascadeDistributionExponent", 2.0, &["crates/fn-render/src/lighting.rs:13 CASCADE_EXPONENT"], "serialized 2 in TODM_A (default only used where not serialized)"),
    ("USkyLightComponent", "Intensity", 1.0, &["README 'Grass colour / exposure fix' (Intensity 1, alpha)"], "ASSUMED Intensity 1"),
];
