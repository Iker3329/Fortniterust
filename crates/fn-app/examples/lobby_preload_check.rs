//! Headless run of the lobby preload (`src/lobby_preload.rs`): runs the whole preload behind the start-up loading screen, saves one capture of that screen
//! (`parity/lobby/loading-1920x1080.png`, taken from the real screen at ~half progress), prints the per-category timings, then plays the first lobby frames
//! and an interactive session (every tab, every Locker picker page, hover / press, outfit swaps) and prints how much grew after the hand-off (must be 0).
//! `cargo run --release -p fn-app --example lobby_preload_check -- [out dir] [WxH]`   (default `parity/lobby`, 1280x720 = the lobby window)
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
#[path = "../src/lobby_preload.rs"]
mod lobby_preload;

use lobby_preload::{counters, Preloader};
use std::time::{Duration, Instant};

fn unpremultiply(px: &[u8]) -> Vec<u8> {
    let mut rgba = px.to_vec();
    for p in rgba.chunks_exact_mut(4) { let a = p[3] as u32; if a > 0 && a < 255 { for c in 0..3 { p[c] = ((p[c] as u32 * 255 + a / 2) / a).min(255) as u8; } } }
    rgba
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(args.first().cloned().unwrap_or_else(|| "parity/lobby".into()));
    let (w, h): (u32, u32) = args.get(1).and_then(|s| s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))).unwrap_or((1280, 720));
    std::fs::create_dir_all(&out).ok();
    let install = fn_ui::pak_assets::default_install();
    let gpu = fn_render::Gpu::headless().expect("gpu");
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let target = fn_render::OffscreenTarget::new(&gpu.device, w, h, format);

    // ---- preload behind the loading screen
    let reads0 = fn_pak::read_count();
    let t0 = Instant::now();
    let mut pre = Preloader::start(&install, gpu.device.clone(), gpu.queue.clone(), format, (w, h)).unwrap_or_else(|e| panic!("preload: {e}"));
    eprintln!("[check] loading screen art ready {:.2} s after start", t0.elapsed().as_secs_f32());
    let (mut captured, mut frames, mut max_slice, mut first_frame) = (false, 0u32, Duration::ZERO, None);
    let products = loop {
        let s = Instant::now();
        match pre.frame(&target.view, w, h).unwrap_or_else(|e| panic!("preload failed: {e}")) { Some(p) => break p, None => {} }
        max_slice = max_slice.max(s.elapsed());
        if first_frame.is_none() { first_frame = Some(t0.elapsed()); }
        frames += 1;
        if !captured && pre.fraction() >= 0.5 {
            let px = pre.raster_boot(1920, 1080);
            let path = out.join("loading-1920x1080.png");
            image::save_buffer(&path, &unpremultiply(&px), 1920, 1080, image::ColorType::Rgba8).expect("png");
            eprintln!("[check] wrote {} at {:.0} % progress", path.display(), pre.fraction() * 100.);
            captured = true;
        }
        std::thread::sleep(Duration::from_millis(4));
    };
    eprintln!("[check] loading screen: {frames} frames, longest frame {:?}, first frame drawn {:?} after start; handed off after {:.2} s ({} pak reads during the preload)", max_slice, first_frame.unwrap_or_default(), t0.elapsed().as_secs_f32(), fn_pak::read_count() - reads0);
    for l in products.report.lines() { eprintln!("{l}"); }

    // ---- the lobby after the hand-off
    let lobby_preload::Products { mut ui, mut renderer, mut scene, mut heroes, glue, counters: at_handoff, .. } = products;
    let outfits: Vec<String> = glue.catalog.of(fn_pose::cosmetics::Category::Outfit, false).iter().filter_map(|c| c.hero.clone()).collect();
    let mut state = lobby_ui::LobbyState::default();
    // the user owns every Battle Royale item: the Locker lists the whole catalog
    let mut all = lobby_profile::Profile::new(glue.default_loadout());
    for cat in [fn_pose::cosmetics::Category::Outfit, fn_pose::cosmetics::Category::Pickaxe, fn_pose::cosmetics::Category::Glider] { for c in glue.catalog.of(cat, false) { all.grant(&c.id); } }
    glue.sync(&all, &mut state.locker);
    let mut frame_ms: Vec<f32> = vec![];
    let mut first_frame_growth: Option<Vec<String>> = None;
    let mut uploads = 0u32;
    let n_tabs = ui.tab_labels().len();
    let total_frames = 240usize;
    let t_start = Instant::now();
    for f in 0..total_frames {
        // scripted session: first 20 frames idle on the Lobby tab, then every tab, then the Locker pickers (scrolling), then hover / press, outfit swaps
        match f {
            20..=79 => state.selected_tab = (f - 20) / 10 % n_tabs.max(1),
            80..=139 => {
                if let Some(lt) = ui.tab_index_of(fn_ui::screens::athena_locker::TAB_CLASS) {
                    state.selected_tab = lt;
                    let slot = lobby_ui::LockerSlot::ALL[(f - 80) / 20 % 3];
                    state.locker.picking = Some(slot); state.locker.picker_t = fn_ui::screens::athena_locker::INTRO_PICKER_END; state.locker.scroll_row = (f - 80) % 20 / 2;
                }
            }
            _ => {}
        }
        if f == 150 || f == 170 || f == 190 { if let Some(hero) = outfits.get((f / 20) * 3 % outfits.len().max(1)) { let ok = heroes.swap_in(&mut scene, hero); if !ok { eprintln!("[check] skin {hero} was not cached"); } } }
        let s = Instant::now();
        let t = t_start.elapsed().as_secs_f32();
        scene.render(&target.view, w, h, t);
        let frame = ui.draw(w as f32, h as f32, &state);
        renderer.render(&frame.list, &mut ui.ctx_mut().assets, &target.view, w, h, None);
        uploads += renderer.stats().texture_uploads;
        let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
        frame_ms.push(s.elapsed().as_secs_f32() * 1000.);
        if f == 0 { first_frame_growth = Some(counters(&mut ui, &renderer).grew_since(&at_handoff)); }
    }
    let after = counters(&mut ui, &renderer);
    let mut sorted = frame_ms.clone(); sorted.sort_by(|a, b| a.total_cmp(b));
    eprintln!("[check] first lobby frame: {:.1} ms, growth vs hand-off: {:?}", frame_ms[0], first_frame_growth.unwrap_or_default());
    eprintln!("[check] {total_frames} scripted frames (tabs, Locker pickers, hover, 3 skin swaps from the cache): median {:.1} ms, p99 {:.1} ms, max {:.1} ms (CPU + GPU wait)", sorted[sorted.len() / 2], sorted[sorted.len() * 99 / 100], sorted[sorted.len() - 1]);
    eprintln!("[check] LOADS AFTER THE HAND-OFF: pak reads +{}, UI textures +{}, UI unresolved +{}, renderer textures +{}, renderer texture uploads {}  (hand-off {:?}, now {:?})",
        after.pak_reads - at_handoff.pak_reads, after.ui_textures - at_handoff.ui_textures, after.ui_missing - at_handoff.ui_missing, after.renderer_textures - at_handoff.renderer_textures, uploads, at_handoff, after);
}
