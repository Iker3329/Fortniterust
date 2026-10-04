use fn_ui::scene::UiContext;
fn main() {
    let install = fn_ui::pak_assets::default_install();
    let mut uic = UiContext::from_install(&install, None).unwrap();
    let dir = std::env::var("TEMP").unwrap() + "\\shop";
    for n in std::env::args().skip(1) {
        match uic.assets.image(&format!("{n}.png")) {
            Some(img) => { println!("{n}: {}x{}", img.w, img.h); image::save_buffer(format!("{dir}\\{n}.png"), &img.rgba, img.w, img.h, image::ColorType::Rgba8).unwrap(); }
            None => println!("{n}: not found"),
        }
    }
}
