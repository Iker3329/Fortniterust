//! FRONT END verification capture (CPU rasteriser, no window): the cooked "Logging In..." screen and the game picker (BATTLE ROYALE / QUIT hovered).
//! `cargo run --release -p fn-ui --example front_end -- <out dir> [w h]`; assets from the 1.8 install (`FN_ROOT` or the default install).
use fn_ui::screens::frontend_cooked::FrontEnd;
use fn_ui::screens::picker::{Item, Picker};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| "verification/front_end".into()));
    let (w, h): (u32, u32) = (args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1920), args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1080));
    std::fs::create_dir_all(&out).unwrap();
    let install = fn_ui::pak_assets::default_install();
    let mut ctx = fn_ui::scene::UiContext::from_install(&install, Some(&fn_ui::assets::DirAssets::default_root())).expect("ui context");
    let t = std::time::Instant::now();
    let fe = FrontEnd::load(&install);
    println!("composed in {:.0} ms ({:?})", t.elapsed().as_secs_f32() * 1000., fe.note);
    let save = |name: &str, list: &fn_ui::draw::RenderList, ctx: &mut fn_ui::scene::UiContext| {
        let px = fn_ui::raster::render(list, &mut ctx.assets, w, h, Some([0., 0., 0., 1.]));
        let p = out.join(format!("{name}-{w}x{h}.png"));
        image::save_buffer(&p, &px, w, h, image::ColorType::Rgba8).unwrap();
        println!("{}", p.display());
    };
    if let Some(mut l) = fe.login {
        for (i, tt) in [0.0f32, 0.25, 0.5].iter().enumerate() {
            let t = std::time::Instant::now();
            let list = l.draw(&mut ctx, w as f32, h as f32, *tt, 5);
            println!("login draw {i}: {:.1} ms", t.elapsed().as_secs_f32() * 1000.);
            save(&format!("login-{i}"), &list, &mut ctx);
        }
    }
    if let Some(mut p) = fe.picker {
        for (name, focus) in [("picker-br", Item::BattleRoyale), ("picker-quit", Item::Quit)] {
            let mut st = Picker::new(); st.focus = focus;
            let t = std::time::Instant::now();
            let list = p.draw(&mut ctx, w as f32, h as f32, &st);
            println!("{name} draw: {:.1} ms", t.elapsed().as_secs_f32() * 1000.);
            save(name, &list, &mut ctx);
        }
    }
}
