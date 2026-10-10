//! Typed-UAV translation on the real 1.8 global shaders that were blocked by it (volumetric fog, eye-adaptation histogram, distance-field
//! shadowing, DFAO screen-grid cone tracing / culling).  The blobs in `tests/uav/` are the GlobalShaderCache programs named
//! `<shader type>__<cache id>.dxbc` (name = nearest preceding string of the cache, confirmed by the UAV / resource layout).  Each one must
//! translate to WGSL and validate with naga; the binding layout of the lighting ones is pinned so the renderer tracks can bind the rips.
use fn_dxbc::codegen::{translate_wgsl, translate_wgsl_with, WgslOptions};
use fn_dxbc::Shader;
use std::path::Path;

fn validate(src: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(src).map_err(|e| e.emit_to_string(src))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).map(|_| ()).map_err(|e| format!("{e:?}"))
}

fn blobs(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "dxbc"))
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_string(), std::fs::read(&p).ok()?))).collect()).unwrap_or_default();
    v.sort();
    v
}

fn uav_dir() -> std::path::PathBuf { Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("uav") }

fn wgsl_of(id: &str) -> String {
    let (_, b) = blobs(&uav_dir()).into_iter().find(|(n, _)| n.ends_with(&format!("__{id}"))).unwrap_or_else(|| panic!("tests/uav: no shader {id}"));
    translate_wgsl(&Shader::parse(&b).unwrap(), "shader").unwrap()
}

/// `@group(3)` binding lines of a module
fn uav_lines(w: &str) -> Vec<String> { w.lines().filter(|l| l.starts_with("@group(3)")).map(|l| l.to_string()).collect() }

#[test]
fn blocked_global_shaders_translate_and_validate() {
    // FN_DXBC_UAV_DIR=<dir of .dxbc> runs the same check on any directory (e.g. the whole global shader cache)
    let dir = std::env::var_os("FN_DXBC_UAV_DIR").map(std::path::PathBuf::from).unwrap_or_else(uav_dir);
    let all = blobs(&dir);
    assert!(all.len() >= 27 || std::env::var_os("FN_DXBC_UAV_DIR").is_some(), "tests/uav holds {} shaders", all.len());
    let mut bad = vec![];
    for (name, b) in &all {
        let sh = Shader::parse(b).unwrap();
        match translate_wgsl(&sh, "shader") {
            Ok(w) => if let Err(e) = validate(&w) { bad.push(format!("{name}: naga: {}", e.lines().take(6).collect::<Vec<_>>().join(" | "))); } else { eprintln!("ok {name}: {}", uav_lines(&w).join(" ; ")); },
            Err(e) => bad.push(format!("{name}: {e}")),
        }
    }
    for b in &bad { eprintln!("FAIL {b}"); }
    assert!(bad.is_empty(), "{} of {} shaders failed", bad.len(), all.len());
}

#[test]
fn volumetric_fog_bindings() {
    // FVolumetricFogMaterialSetupCS: VBufferA / VBufferB (RWTexture3D<float4>), PF_FloatRGBA in UE 4.16 -> rgba16float
    let w = wgsl_of("751700");
    assert_eq!(uav_lines(&w), ["@group(3) @binding(0) var u0: texture_storage_3d<rgba16float, write>;", "@group(3) @binding(1) var u1: texture_storage_3d<rgba16float, write>;"]);
    assert!(w.contains("@compute @workgroup_size(4, 4, 4)"));
    // the store sits inside the shader's own grid-size test and is additionally guarded with the texture size (D3D11 drops OOB stores)
    assert!(w.contains(" < textureDimensions(u0)) { textureStore(u0, "));
    for id in ["301453", "317200", "314004", "351150"] {
        assert_eq!(uav_lines(&wgsl_of(id)), ["@group(3) @binding(0) var u0: texture_storage_3d<rgba16float, write>;"], "light scattering {id}");
    }
    let w = wgsl_of("689183");
    assert_eq!(uav_lines(&w), ["@group(3) @binding(0) var u0: texture_storage_3d<rgba16float, write>;"]);
    assert!(w.contains("@compute @workgroup_size(8, 8, 1)"));
}

#[test]
fn histogram_and_df_shadow_bindings() {
    assert_eq!(uav_lines(&wgsl_of("992254")), ["@group(3) @binding(0) var u0: texture_storage_2d<rgba16float, write>;"]);
    for id in ["126292", "99145", "146589"] {
        assert_eq!(uav_lines(&wgsl_of(id)), ["@group(3) @binding(0) var u0: texture_storage_2d<rgba16float, write>;"], "DF shadowing {id}");
    }
}

#[test]
fn format_override_and_read_write_rule() {
    let (_, b) = blobs(&uav_dir()).into_iter().find(|(n, _)| n.ends_with("__126292")).unwrap();
    let sh = Shader::parse(&b).unwrap();
    let w = translate_wgsl_with(&sh, "shader", &WgslOptions { uav_formats: vec![(0, "rg32float".into())], ..Default::default() }).unwrap();
    assert!(w.contains("var u0: texture_storage_2d<rg32float, write>;"));
    validate(&w).unwrap();
}

/// typed buffers with atomics become `array<atomic<u32>>` (one R32 element per word); the DFAO object culling uses them
#[test]
fn typed_buffer_atomics() {
    let mut n = 0;
    for (name, b) in blobs(&uav_dir()) {
        let sh = Shader::parse(&b).unwrap();
        let w = translate_wgsl(&sh, "shader").unwrap();
        for u in &sh.decls.uavs {
            if u.kind != fn_dxbc::ResKind::Typed || u.dim != 1 { continue; }
            if let Some(l) = w.lines().find(|l| l.starts_with(&format!("@group(3) @binding({}) var<storage, read_write> u{}:", u.slot, u.slot))) {
                if l.contains("atomic<u32>") { n += 1; assert!(w.contains(&format!("(&u{}[", u.slot)), "{name}: atomic slot u{} unused", u.slot); }
            }
        }
    }
    assert!(n > 0, "no typed-buffer atomics in tests/uav");
}

/// R32 typed buffers (`r32_buffers` / `r32_srv_buffers`) are bound one word per element: FComputeCulledTilesStartOffsetCS stores scalar
/// R32_UINT values splatted to .xyzw into u0 / u1 and reads `Buffer<uint>` t0 / t1; u2 is atomic, so it is one word per element anyway
#[test]
fn r32_typed_buffer_override() {
    let (_, b) = blobs(&uav_dir()).into_iter().find(|(n, _)| n.ends_with("__701833")).unwrap();
    let sh = Shader::parse(&b).unwrap();
    let w = translate_wgsl_with(&sh, "shader", &WgslOptions { r32_buffers: vec![0, 1], r32_srv_buffers: vec![0, 1], ..Default::default() }).unwrap();
    validate(&w).unwrap();
    assert_eq!(uav_lines(&w), ["@group(3) @binding(0) var<storage, read_write> u0: array<u32>;", "@group(3) @binding(1) var<storage, read_write> u1: array<u32>;", "@group(3) @binding(2) var<storage, read_write> u2: array<atomic<u32>>;"]);
    assert!(w.contains("@group(1) @binding(0) var<storage, read> t0: array<u32>;") && w.contains("@group(1) @binding(1) var<storage, read> t1: array<u32>;"));
    let st: Vec<&str> = w.lines().map(str::trim_start).filter(|l| l.starts_with("u0[") || l.starts_with("u1[")).collect();
    assert!(st.len() >= 3 && st.iter().all(|l| !l.contains("vec4")), "u0 / u1 store one word per element: {st:?}");
    assert!(w.contains("bitcast<f32>(t0[") && w.contains("bitcast<f32>(t1["), "t0 / t1 load one word");
}

/// WebGPU default limits (naga does not check them): workgroup memory <= 16384 B, invocations <= 256, storage buffers <= 8,
/// storage textures <= 4, sampled textures <= 16 per stage; and the modules validate with naga's default capabilities plus
/// SHADER_FLOAT16_IN_FLOAT32 (`pack2x16float` / `unpack2x16float` of f32tof16 / f16tof32 are core WGSL builtins in WebGPU)
#[test]
fn blocked_global_shaders_fit_webgpu_default_limits() {
    let num = |s: &str| s.trim().trim_end_matches('>').parse::<u64>().unwrap_or(0);
    for (name, b) in blobs(&uav_dir()) {
        let w = translate_wgsl(&Shader::parse(&b).unwrap(), "shader").unwrap();
        let tgsm: u64 = w.lines().filter(|l| l.starts_with("var<workgroup>")).map(|l| 4 * num(l.rsplit(", ").next().unwrap_or("0").trim_end_matches(';'))).sum();
        let inv: u64 = w.lines().find_map(|l| l.split("workgroup_size(").nth(1)).map(|r| r.split(')').next().unwrap().split(',').map(num).product()).unwrap_or(1);
        let sbuf = w.lines().filter(|l| l.starts_with("@group") && l.contains("var<storage")).count();
        let stex = w.lines().filter(|l| l.contains("texture_storage_")).count();
        let tex = w.lines().filter(|l| l.starts_with("@group(1)") && l.contains(": texture_")).count();
        assert!(tgsm <= 16384 && inv <= 256 && sbuf <= 8 && stex <= 4 && tex <= 16, "{name}: workgroup {tgsm} B, {inv} invocations, {sbuf} storage buffers, {stex} storage textures, {tex} textures");
        let module = naga::front::wgsl::parse_str(&w).unwrap();
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default() | naga::valid::Capabilities::SHADER_FLOAT16_IN_FLOAT32).validate(&module).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    }
}
