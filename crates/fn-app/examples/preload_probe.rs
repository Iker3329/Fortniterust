//! Read-only probe used to cite data for the start-up loading screen.
//! `cargo run --release -p fn-app --example preload_probe -- glob <pattern>...`  lists pak entries
//! `cargo run --release -p fn-app --example preload_probe -- hash <TextureName.png>...`  decodes textures and prints size + FNV-1a of the RGBA
fn main() {
    let install = fn_ui::pak_assets::default_install();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("glob") => {
            let set = fn_pak::PakSet::open_install(&install).expect("paks");
            for pat in args { println!("== {pat}"); for (_, e) in set.glob(&pat).into_iter().take(80) { println!("{}  {} bytes", e.path, e.size); } }
        }
        Some("hash") => {
            let a = fn_ui::pak_assets::PakAssets::open(&install).expect("paks");
            for n in args {
                match a.decode_texture(&n) {
                    Ok(im) => { let mut h: u64 = 0xcbf29ce484222325; for b in &im.rgba { h ^= *b as u64; h = h.wrapping_mul(0x100000001b3); } println!("{n}: {}x{} fnv={h:016x}", im.w, im.h); }
                    Err(e) => println!("{n}: {e}"),
                }
            }
        }
        Some("dump") => {
            let out = std::path::PathBuf::from(args.next().expect("out dir"));
            let a = fn_ui::pak_assets::PakAssets::open(&install).expect("paks");
            for n in args { if let Ok(im) = a.decode_texture(&n) { image::save_buffer(out.join(&n), &im.rgba, im.w, im.h, image::ColorType::Rgba8).expect("png"); } }
        }
        _ => eprintln!("usage: glob <pattern>... | hash <texture.png>..."),
    }
}
