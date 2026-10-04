//! Research tool: writes the largest mip of texture packages (by name, e.g. `T_StoreBGVista`) as PNG. `cargo run --release -p fn-ui --example tex_dump -- <out dir> <name>...`
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(&a[0]);
    std::fs::create_dir_all(&out).ok();
    let assets = fn_ui::pak_assets::PakAssets::open(fn_ui::pak_assets::default_install()).expect("paks");
    for n in &a[1..] {
        match assets.decode_texture(&format!("{n}.png")) {
            Ok(i) => { let p = out.join(format!("{n}.png")); image::save_buffer(&p, &i.rgba, i.w, i.h, image::ColorType::Rgba8).unwrap(); println!("{n}: {}x{} -> {}", i.w, i.h, p.display()); }
            Err(e) => println!("{n}: {e}"),
        }
    }
}
