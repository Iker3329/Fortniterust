use fn_ui::scene::UiContext;
use fn_ui::screens::athena_lobby::{slate_mat::SlateMat, store::Store};
use fn_dxbc_cli::uniform::Overrides;
use std::collections::HashMap;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let path = a[0].clone();
    let install = fn_ui::pak_assets::default_install();
    let store = Store::open(&install).unwrap();
    let mut uic = UiContext::from_install(&install, None).unwrap();
    let mut ov = Overrides::default();
    ov.vectors.insert("RGB_1".into(), [1.0, 0.0, 1.0, 1.0]);
    ov.vectors.insert("RGB_2".into(), [0.0, 1.0, 1.0, 1.0]);
    ov.vectors.insert("RGB_3".into(), [1.0, 1.0, 0.0, 1.0]);
    ov.vectors.insert("RGB_4".into(), [0.0, 0.0, 1.0, 1.0]);
    ov.scalars.insert("s_AnimateBorder".into(), 1.0);
    let m = SlateMat::load_with(&store, &mut uic.assets, &path, &ov, &HashMap::new()).unwrap();
    println!("{}", m.describe());
    let (w, h) = (192u32, 304u32);
    let px = m.raster(w, h, (w as f32, h as f32), (64.0, 64.0), [0.5; 4], true, [1.0; 4], false, 0.0);
    let nz = px.chunks(4).filter(|p| p[3] > 0).count();
    println!("non-transparent pixels: {nz} of {}", w * h);
    image::save_buffer(&a[1], &px, w, h, image::ColorType::Rgba8).unwrap();
}
