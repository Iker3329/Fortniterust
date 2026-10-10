fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-check-cfg=cfg(fn_live_fast)");
    let out = std::env::var("OUT_DIR").unwrap_or_default();
    let parts: Vec<String> = std::path::Path::new(&out).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    let fast = parts.iter().rposition(|p| p == "build").and_then(|i| i.checked_sub(1)).is_some_and(|i| parts[i] == "fast");
    if fast { println!("cargo:rustc-cfg=fn_live_fast"); }
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest.parent().and_then(|p| p.parent()).map(|p| p.to_path_buf()).unwrap_or(manifest);
    println!("cargo:rustc-env=FN_LIVE_REPO={}", repo.display());
}
