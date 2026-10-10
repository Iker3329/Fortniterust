fn main() {
    println!("cargo:rerun-if-changed=fortnite-rs.manifest");
    println!("cargo:rerun-if-changed=build.rs");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os == "windows" && env == "msvc" && std::env::var_os("FN_NO_SEGMENT_HEAP").is_none() {
        let m = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("fortnite-rs.manifest");
        println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", m.display());
    }
}
