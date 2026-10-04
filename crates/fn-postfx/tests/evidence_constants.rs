//! `params::athena_day_cm()` must equal the values recovered from the install (merged TODM_A + TODM_A_Child_Athena Day settings).
//! Skipped when the 1.8 install is not present.
use fn_pak::PakSet;
use fn_postfx::{evidence, params};
use serde_json::Value;

fn day() -> Option<Value> {
    let root = std::path::PathBuf::from(std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into()));
    if !root.exists() { return None; }
    let set = PakSet::open_install(&root).ok()?;
    Some(evidence::todm_section(&set)["phases"]["Day"].clone())
}

fn val(day: &Value, name: &str) -> f64 {
    let v = &day["applied_tagged"][name]["value"];
    v.as_f64().or_else(|| v.as_array().and_then(|a| a[0].as_f64())).unwrap_or_else(|| panic!("{name} not an applied tagged value"))
}

#[test]
fn athena_day_constants_match_the_install() {
    let Some(day) = day() else { eprintln!("no install, skipping"); return };
    let (dof, mb, bloom) = params::athena_day_cm();
    assert_eq!(dof.focal_distance as f64, val(&day, "DepthOfFieldFocalDistance"));
    assert_eq!(dof.far_transition as f64, val(&day, "DepthOfFieldFarTransitionRegion"));
    assert_eq!(dof.near_transition as f64, val(&day, "DepthOfFieldNearTransitionRegion"));
    assert_eq!(dof.near_blur_size as f64, val(&day, "DepthOfFieldNearBlurSize"));
    assert_eq!(dof.far_blur_size as f64, val(&day, "DepthOfFieldFarBlurSize"));
    assert_eq!(dof.sky_focus_distance as f64, val(&day, "DepthOfFieldSkyFocusDistance"));
    assert_eq!(day["applied_tagged"]["DepthOfFieldMethod"]["value"], "DOFM_Gaussian");
    assert_eq!(mb.amount as f64, val(&day, "MotionBlurAmount"));
    assert_eq!(mb.max_percent as f64, val(&day, "MotionBlurMax"));
    assert_eq!(bloom.size_scale as f64, val(&day, "BloomSizeScale"));
    assert_eq!(bloom.sizes[5] as f64, val(&day, "Bloom6Size"));
    assert_eq!(bloom.tints[2][0] as f64, val(&day, "Bloom3Tint"));
    assert_eq!(bloom.tints[5][0] as f64, val(&day, "Bloom6Tint"));
    // untagged-but-overridden fields are the engine defaults the params assume
    let untagged: Vec<&str> = day["overridden_untagged_equals_engine_default"].as_array().unwrap().iter().filter_map(|v| v.as_str()).collect();
    for n in ["BloomIntensity", "BloomThreshold", "Bloom1Size", "Bloom2Size", "Bloom3Size", "Bloom4Size", "Bloom5Size", "Bloom1Tint", "Bloom2Tint", "Bloom4Tint", "Bloom5Tint", "DepthOfFieldFocalRegion", "SceneFringeIntensity"] {
        assert!(untagged.contains(&n), "{n} should be overridden-but-untagged");
    }
    // the effects that must NOT be active
    assert_eq!(val(&day, "LensFlareIntensity"), 0.0);
    // volumetric DOF-adjacent fields that are tagged but ignored
    for n in ["DepthOfFieldVignetteSize", "DepthOfFieldOcclusion", "DepthOfFieldDepthBlurRadius"] {
        assert!(!day["tagged_but_not_overridden"][n].is_null(), "{n} should be tagged-but-ignored");
    }
}
