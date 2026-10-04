//! Tests against real Fortnite 1.8 data. They skip (with a printed note) when the extracted test data or the old
//! prototype tree is not present.

use fn_texture::{DecodeOptions, PixelFormat, Pixels, Texture2D};
use std::path::{Path, PathBuf};

fn testdata() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var("TEMP").ok()?).join("fn-testdata");
    p.is_dir().then_some(p)
}

fn runtime() -> Option<PathBuf> {
    let p = PathBuf::from(r"C:\Users\Brady\Documents\Codex\2026-10-02\im-seeing-titkoks-where-people-are-2\FortniteRuntime");
    p.is_dir().then_some(p)
}

fn pkg_path(td: &Path, pkg: &str, ext: &str) -> PathBuf {
    if let Some(rest) = pkg.strip_prefix("Engine/") {
        td.join("Engine").join("Content").join(rest).with_extension(ext)
    } else if let Some(rest) = pkg.strip_prefix("Game/") {
        td.join("FortniteGame").join("Content").join(rest).with_extension(ext)
    } else {
        td.join(pkg).with_extension(ext)
    }
}

fn load(td: &Path, pkg: &str) -> Option<Texture2D> {
    let ua = pkg_path(td, pkg, "uasset");
    if !ua.exists() {
        return None;
    }
    let rd = |e: &str| std::fs::read(pkg_path(td, pkg, e)).unwrap_or_default();
    Some(Texture2D::from_files(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap_or_else(|e| panic!("{pkg}: {e}")))
}

/// Every manifest texture (and alias) must be pixel exact versus the old UEViewer PNG.
#[test]
fn manifest_textures_match_old_pngs_exactly() {
    let (Some(td), Some(rt)) = (testdata(), runtime()) else {
        println!("SKIP manifest_textures_match_old_pngs_exactly: testdata/runtime dir missing");
        return;
    };
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(rt.join("tools").join("asset-manifest.json")).unwrap()).unwrap();
    let mut jobs = Vec::new();
    for t in manifest["textures"].as_array().unwrap() {
        jobs.push((t["pkg"].as_str().unwrap().to_string(), t["out"].as_str().unwrap().to_string()));
    }
    for t in manifest["aliases"].as_array().unwrap() {
        jobs.push((t["pkg"].as_str().unwrap().to_string(), t["alias"].as_str().unwrap().to_string()));
    }
    let (mut checked, mut bad) = (0, Vec::new());
    for (pkg, out) in jobs {
        let png = rt.join("assets").join(&out);
        let Some(tex) = load(&td, &pkg) else { continue };
        if !png.exists() {
            continue;
        }
        let img = tex.decode(tex.best_mip().expect("mip"), DecodeOptions::default()).unwrap();
        let mut mine = img.to_rgba8();
        if mine.chunks_exact(4).all(|p| p[3] == 0) {
            mine.chunks_exact_mut(4).for_each(|p| p[3] = 255); // UEViewer's PNG writer drops an all-zero alpha channel
        }
        let old = image::open(&png).unwrap().to_rgba8();
        checked += 1;
        if old.dimensions() != (img.width as u32, img.height as u32) || old.as_raw() != &mine {
            bad.push(pkg);
        }
    }
    println!("compared {checked} textures, {} differ", bad.len());
    assert!(bad.is_empty(), "textures differing from old exports: {bad:?}");
    assert!(checked > 100, "expected the manifest textures to be present, only {checked} compared");
}

#[test]
fn cube_map_slices() {
    let Some(td) = testdata() else {
        println!("SKIP cube_map_slices");
        return;
    };
    let Some(tex) = load(&td, "Engine/EngineResources/DefaultTextureCube") else {
        println!("SKIP cube_map_slices: DefaultTextureCube not extracted");
        return;
    };
    assert!(tex.is_cube());
    assert_eq!((tex.num_slices, tex.format.clone(), tex.size_x), (6, PixelFormat::Dxt1, 128));
    let faces: Vec<_> = (0..6).map(|s| tex.decode_slice(0, s, DecodeOptions::default()).unwrap()).collect();
    assert!(faces.iter().all(|f| f.width == 128 && f.height == 128));
    assert!(tex.decode_slice(0, 6, DecodeOptions::default()).is_err());
}

#[test]
fn hdr_float_texture_and_streamed_bc4() {
    let Some(td) = testdata() else {
        println!("SKIP hdr_float_texture_and_streamed_bc4");
        return;
    };
    if let Some(tex) = load(&td, "Game/Effects/Fort_Effects/Gameplay/SmokeDustExplosions/Textures/T_RocketExplosion_01") {
        assert_eq!(tex.format, PixelFormat::FloatRGBA);
        let img = tex.decode(0, DecodeOptions::default()).unwrap();
        assert_eq!((img.width, img.height), (6901, 21));
        assert!(matches!(img.pixels, Pixels::RgbaF16(_)));
    } else {
        println!("SKIP hdr part: T_RocketExplosion_01 not extracted");
    }
    // FirstMip = 3: the mip array starts at 128x128 and its top mip lives in the .ubulk
    if let Some(tex) = load(&td, "Game/Maps/Test_Maps/WojciechMadry/T_MiniMapMask") {
        assert_eq!(tex.format, PixelFormat::Bc4);
        assert_eq!((tex.size_x, tex.mips[0].width), (1024, 128));
        assert!(tex.mips[0].in_ubulk());
        let best = tex.best_mip().unwrap();
        assert_eq!(best, 0);
        assert_eq!(tex.mip_for_max_dimension(32), Some(2));
        let img = tex.decode(best, DecodeOptions::default()).unwrap();
        assert_eq!((img.width, img.height), (128, 128));
    } else {
        println!("SKIP bc4 part: T_MiniMapMask not extracted");
    }
}

#[test]
fn light_profile_is_rejected_clearly() {
    let Some(td) = testdata() else {
        println!("SKIP light_profile_is_rejected_clearly");
        return;
    };
    let p = |e: &str| std::fs::read(pkg_path(&td, "Game/Building/IESTextures/HeadLight_IESTexture", e)).unwrap_or_default();
    let ua = p("uasset");
    if ua.is_empty() {
        println!("SKIP light profile: not extracted");
        return;
    }
    let err = Texture2D::from_files(&ua, &p("uexp"), &p("ubulk")).unwrap_err();
    assert!(err.to_string().contains("TextureLightProfile"), "{err}");
}

#[test]
fn decode_2048_dxt5_is_fast() {
    let Some(td) = testdata() else {
        println!("SKIP decode_2048_dxt5_is_fast");
        return;
    };
    let Some(tex) = load(&td, "Game/Athena/Environments/Textures/T_Athena_HF_Grass_1_D") else {
        println!("SKIP: T_Athena_HF_Grass_1_D not extracted");
        return;
    };
    assert_eq!((tex.format.clone(), tex.size_x, tex.size_y), (PixelFormat::Dxt5, 2048, 2048));
    let t = std::time::Instant::now();
    let img = tex.decode(0, DecodeOptions::default()).unwrap();
    let dt = t.elapsed();
    println!("2048x2048 DXT5 decode: {dt:?}");
    assert_eq!((img.width, img.height), (2048, 2048));
    assert!(dt.as_secs_f64() < 1.0, "too slow: {dt:?}");
}
