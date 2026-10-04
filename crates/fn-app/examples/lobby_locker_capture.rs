//! Headless captures of the BR lobby with the Locker: 3D scene + UI, any tab, picker open, outfit swap (`LobbyScene::set_character`).
//! `cargo run --release -p fn-app --example lobby_locker_capture -- <out dir> [--w 1920] [--h 1080] [--ui-only] [--tab lobby|locker|N] [--pick outfit|pickaxe|glider]
//!  [--skins CID_030_Athena_Commando_M_Halloween,CID_029_Athena_Commando_F_Halloween] [--all-owned] [--name tag] [--hover SlotName] [--scroll N]`
//! One PNG per skin (`<name>-<skin>.png`) is written; `--ui-only` skips the 3D scene (dark backdrop) for quick UI iterations.
#![allow(dead_code)]
#[path = "../src/lobby_world.rs"]
mod lobby_world;
#[path = "../src/lobby_scene.rs"]
mod lobby_scene;
#[path = "../src/lobby_ui.rs"]
mod lobby_ui;
#[path = "../src/lobby_profile.rs"]
mod lobby_profile;
#[path = "../src/lobby_locker.rs"]
mod lobby_locker;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let out = std::path::PathBuf::from(args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| "parity/lobby".into()));
    let (w, h): (u32, u32) = (get("--w").and_then(|s| s.parse().ok()).unwrap_or(1920), get("--h").and_then(|s| s.parse().ok()).unwrap_or(1080));
    let ui_only = args.iter().any(|a| a == "--ui-only");
    let name = get("--name").unwrap_or_else(|| "locker".into());
    std::fs::create_dir_all(&out).ok();
    let install = fn_ui::pak_assets::default_install();
    let set = fn_pak::PakSet::open_install(&install).expect("paks");
    let glue = lobby_locker::LockerGlue::load(&set);
    let mut profile = lobby_profile::Profile::new(glue.default_loadout());
    if args.iter().any(|a| a == "--all-owned") {
        for c in glue.catalog.items.iter().filter(|c| !c.test_asset) { profile.grant(&c.id); }
    }
    let skins: Vec<String> = get("--skins").map(|s| s.split(',').map(str::to_owned).collect()).unwrap_or_else(|| vec!["CID_005_Athena_Commando_M_Default".into()]);
    let mut ui = lobby_ui::LobbyUi::load(&install).unwrap_or_else(|e| panic!("ui: {e}"));
    ui.inner_mut().set_card_budget(usize::MAX);
    let locker_tab = ui.tab_index_of(fn_ui::screens::athena_locker::TAB_CLASS).expect("locker tab registration");
    let tab = match get("--tab").as_deref() { Some("lobby") => 0, Some("locker") | None => locker_tab, Some(n) => n.parse().unwrap_or(locker_tab) };
    let mut state = lobby_ui::LobbyState::default();
    state.selected_tab = tab;
    state.mtx_balance = profile.balance();
    glue.sync(&profile, &mut state.locker);
    if let Some(p) = get("--pick") {
        let slot = match p.as_str() { "pickaxe" => lobby_ui::LockerSlot::Pickaxe, "glider" => lobby_ui::LockerSlot::Glider, _ => lobby_ui::LockerSlot::Outfit };
        state.locker.picking = Some(slot);
        state.locker.picker_t = fn_ui::screens::athena_locker::INTRO_PICKER_END;
        state.locker.scroll_row = get("--scroll").and_then(|s| s.parse().ok()).unwrap_or(0);
    }
    let gpu = fn_render::Gpu::headless().expect("gpu");
    let mut locker_scene = None;
    let mut scene = if ui_only { None } else {
        let hero = glue.catalog.find(&skins[0]).and_then(|c| c.hero.clone());
        let assets = lobby_scene::LobbyAssets::load(&install, hero.as_deref()).unwrap_or_else(|e| panic!("scene assets: {e}"));
        // the LOCKER view: the Cosmetics camera, its backdrop and the Vault hero placement (own scene, shares the hero and its clips)
        if tab == locker_tab {
            let t0 = std::time::Instant::now();
            let la = lobby_scene::LobbyAssets::load_cosmetics(&install, assets.character.as_ref().map(|c| c.share())).unwrap_or_else(|e| panic!("locker scene assets: {e}"));
            if let Some(p) = la.data.placements.first() { eprintln!("[locker] vault placement root {:?} {:?} / character {:?} {:?}; item preview {:?}", p.root.t, p.root.rotator_deg(), p.character.t, p.character.rotator_deg(), la.data.item_preview.map(|x| (x.t, x.rotator_deg(), x.s))); }
            for l in &la.data.lights { eprintln!("[locker]   light {} {} {:?} vis {} affects {} intensity {} at {:?}", l.actor, l.component, l.kind, l.visible, l.affects_world, l.intensity, l.xf.t); }
            eprintln!("[locker] cosmetics scene data: {} statics, {} lights, camera {:?}", la.data.statics.len(), la.data.lights.len(), la.data.camera.as_ref().map(|c| (c.xf.t, c.fov_deg)));
            locker_scene = Some(lobby_scene::LobbyScene::from_assets(la, &gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8Unorm).unwrap_or_else(|e| panic!("locker scene: {e}")));
            eprintln!("[locker] locker scene ready in {:?}", t0.elapsed());
        }
        Some(lobby_scene::LobbyScene::from_assets(assets, &gpu.device, &gpu.queue, wgpu::TextureFormat::Rgba8Unorm).unwrap_or_else(|e| panic!("scene: {e}")))
    };
    let mut t = 8.0f32;
    for (k, skin) in skins.iter().enumerate() {
        let Some(c) = glue.catalog.find(skin) else { eprintln!("unknown skin {skin}"); continue };
        if k > 0 {
            let t0 = std::time::Instant::now();
            let asset = lobby_scene::load_hero_parts(&set, c.hero.as_deref().unwrap()).unwrap_or_else(|e| panic!("{skin}: {e}"));
            let t_load = t0.elapsed();
            if let Some(s) = locker_scene.as_mut() { assert!(s.set_hero(asset.clone())); }
            if let Some(s) = scene.as_mut() { assert!(s.set_hero(asset)); }
            eprintln!("[locker] {skin}: character assets decoded in {t_load:?}, uploaded in {:?}", t0.elapsed() - t_load);
            t += 8.0;
        }
        // equip it in the profile like a picker click would (grant first: the capture shows any item)
        profile.grant(&c.id);
        let _ = profile.equip(lobby_locker::to_profile_slot_of(c), &c.id);
        glue.sync(&profile, &mut state.locker);
        let target = fn_render::OffscreenTarget::new(&gpu.device, w, h, wgpu::TextureFormat::Rgba8Unorm);
        let mut px = match locker_scene.as_mut().or(scene.as_mut()) {
            Some(s) => { s.render(&target.view, w, h, t); target.read_rgba8(&gpu) }
            None => { let mut v = vec![0u8; (w * h * 4) as usize]; for p in v.chunks_exact_mut(4) { p.copy_from_slice(&[10, 14, 30, 255]); } v }
        };
        if tab == 0 {
            if let Some(s) = scene.as_ref() {
                let mut pads = vec![];
                for sl in s.party_slots().iter().filter(|s| !s.hidden) {
                    if sl.occupied { if let Some([x, y]) = s.project(sl.top_widget_cm, w, h) { pads.push(lobby_ui::Pad { x, y, pivot: sl.top_pivot, kind: lobby_ui::PadKind::Player { name: None, leader: true, ready: false } }); } }
                    else if let Some([x, y]) = s.project(sl.add_widget_cm, w, h) { pads.push(lobby_ui::Pad { x, y, pivot: sl.add_pivot, kind: lobby_ui::PadKind::Empty }); }
                }
                state.pads = pads;
            }
        }
        let t0 = std::time::Instant::now();
        let frame = ui.draw(w as f32, h as f32, &state);
        eprintln!("[locker] ui frame in {:?}; unresolved: {:?}", t0.elapsed(), frame.missing);
        let a = ui.ctx_mut();
        let layer = fn_ui::raster::render(&frame.list, &mut a.assets, w, h, None);
        for (d, s) in px.chunks_exact_mut(4).zip(layer.chunks_exact(4)) {
            let ia = 255 - s[3] as u32;
            for c in 0..3 { d[c] = (s[c] as u32 + (d[c] as u32 * ia + 127) / 255).min(255) as u8; }
            d[3] = 255;
        }
        let path = out.join(format!("{name}-{}.png", skin.to_ascii_lowercase()));
        image::save_buffer(&path, &px, w, h, image::ColorType::Rgba8).expect("png");
        eprintln!("wrote {}", path.display());
    }
}
