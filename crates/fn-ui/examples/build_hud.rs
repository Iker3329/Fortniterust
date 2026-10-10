//! BUILD HUD verification capture (CPU rasteriser, no window): the in-game HUD with the building quickbar in combat (minimised) and build mode (maximised,
//! each piece selected, each material highlighted). `cargo run --release -p fn-ui --example build_hud -- <out dir>`; assets from the 1.8 install (`FN_ROOT`
//! or the default install) layered over the exported asset folder.
use fn_ui::draw::RenderList;
use fn_ui::screens::hud::{Hud, HudItem, HudState};
use fn_ui::screens::reticle_state::HarvestResource;
fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "verification/building/hud".into()));
    std::fs::create_dir_all(&out).unwrap();
    let install = fn_ui::pak_assets::default_install();
    let mut ctx = fn_ui::scene::UiContext::from_install(&install, Some(&fn_ui::assets::DirAssets::default_root())).expect("ui context");
    let (w, h) = (1920u32, 1080u32);
    let cases: [(&str, bool, usize, HarvestResource, f32); 6] = [
        ("combat-minimised", false, 0, HarvestResource::Wood, 1.),
        ("build-wall-wood", true, 0, HarvestResource::Wood, 1.),
        ("build-floor-stone", true, 1, HarvestResource::Stone, 1.),
        ("build-stairs-metal", true, 2, HarvestResource::Metal, 1.),
        ("build-roof-wood", true, 3, HarvestResource::Wood, 1.),
        ("build-maximise-halfway", true, 0, HarvestResource::Wood, 0.05),
    ];
    for (name, building, slot, res, t) in cases {
        let mut hud = Hud::new();
        let mut st = HudState::default();
        st.items[1] = Some(HudItem::new(1, "T-Icon-Weapons-SK-Pick.png", None));
        st.resources = [120, 45, 0];
        st.world.current_resource = res;
        hud.build.shown = true;
        hud.build.state.focused_secondary = building;
        hud.build.state.building_slot = slot;
        hud.set_state(&st);
        if t >= 1. { hud.build.snap(); hud.tick(0.); } else { hud.tick(t); }
        let mut list = RenderList::new();
        hud.present(&mut ctx, w as f32, h as f32, &mut list);
        let px = fn_ui::raster::render(&list, &mut ctx.assets, w, h, Some([0.32, 0.42, 0.30, 1.]));
        let p = out.join(format!("{name}-{w}x{h}.png"));
        image::save_buffer(&p, &px, w, h, image::ColorType::Rgba8).unwrap();
        // close-up of the bottom-right quarter
        let (cx, cy, cw, ch) = (w / 2, h / 2, w / 2, h / 2);
        let mut crop = vec![0u8; (cw * ch * 4) as usize];
        for y in 0..ch { let s = (((cy + y) * w + cx) * 4) as usize; crop[(y * cw * 4) as usize..((y + 1) * cw * 4) as usize].copy_from_slice(&px[s..s + (cw * 4) as usize]); }
        image::save_buffer(out.join(format!("{name}-bottomright.png")), &crop, cw, ch, image::ColorType::Rgba8).unwrap();
        println!("{}", p.display());
    }
}
