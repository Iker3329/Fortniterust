//! Diagnostic: evaluate one Slate material brush from the cooked data and write it (and a summary) out.
//! `cargo run -p fn-ui --example mat_diag -- /Game/Path/Mat.Mat WxH [tint r,g,b,a] [out.png]`
use fn_ui::scene::UiContext;
use fn_ui::screens::athena_lobby::{slate_mat::SlateMat, store::Store};
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let path = a[0].clone();
    let (w, h): (u32, u32) = { let (x, y) = a[1].split_once('x').unwrap(); (x.parse().unwrap(), y.parse().unwrap()) };
    let tint: [f32; 4] = a.get(2).map(|s| { let v: Vec<f32> = s.split(',').map(|x| x.parse().unwrap()).collect(); [v[0], v[1], v[2], v[3]] }).unwrap_or([1.0; 4]);
    let install = fn_ui::pak_assets::default_install();
    let store = Store::open(&install).unwrap();
    let mut uic = UiContext::from_install(&install, None).unwrap();
    let m = SlateMat::load(&store, &mut uic.assets, &path).unwrap();
    println!("{}", m.describe());
    let t = std::time::Instant::now();
    let px = m.raster(w, h, (w as f32, h as f32), (w as f32, h as f32), [0.0; 4], false, tint, false, 0.0);
    println!("raster {w}x{h} in {:?}", t.elapsed());
    if let Some(out) = a.get(3) { image::save_buffer(out, &px, w, h, image::ColorType::Rgba8).unwrap(); println!("wrote {out}"); }
}
