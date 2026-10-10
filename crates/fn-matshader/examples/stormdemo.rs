//! Headless harness for the storm visuals: renders the safe-zone wall and the cloud sheet (original programs, FX pass) over a flat grey scene and writes PNGs.
//!   stormdemo <out dir> [width height]
//! Env: STORM_RADIUS_M (default 1500), STORM_OPACITY (default 1), STORM_ONLY=<n> renders only view n.
use fn_fx::view::ViewParams;
use fn_matshader::storm::StormWorld;
use fn_render::fx::*;
use fn_render::storm::StormParams;
use fn_render::wgpu;
use fn_render::{CameraParams, Gpu};
use glam::Vec3;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(args.get(1).ok_or("out dir")?);
    let (w, h) = (args.get(2).and_then(|s| s.parse().ok()).unwrap_or(960u32), args.get(3).and_then(|s| s.parse().ok()).unwrap_or(540u32));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let gpu = Gpu::headless()?;
    eprintln!("gpu: {}", gpu.describe());
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let mut fx = FxRenderer::new(&gpu.device, &gpu.queue, HDR, DEPTH);
    let t0 = std::time::Instant::now();
    let mut world = StormWorld::load(&set, &mut fx)?;
    eprintln!("storm loaded in {:.2}s: {:?}", t0.elapsed().as_secs_f32(), world.notes);
    let radius_m: f32 = std::env::var("STORM_RADIUS_M").ok().and_then(|s| s.parse().ok()).unwrap_or(1500.);
    let opacity: f32 = std::env::var("STORM_OPACITY").ok().and_then(|s| s.parse().ok()).unwrap_or(1.);
    let only: Option<usize> = std::env::var("STORM_ONLY").ok().and_then(|s| s.parse().ok());
    // FX space: the zone centre at the origin, so the port world = FX / 100 with the axes swapped
    let params = StormParams { center_cm: [0., 0., 0.], radius_cm: radius_m * 100., master_opacity: opacity };
    // (label, eye (port m), yaw, pitch)
    let views0: [(&str, Vec3, f32, f32); 6] = [
        ("outside_facing_wall", Vec3::new(radius_m + 150., 1.6, 0.), -90., 4.),
        ("inside_at_wall", Vec3::new(radius_m - 120., 1.6, 0.), 90., 4.),
        ("inside_wall_far", Vec3::new(radius_m - 1000., 1.6, 0.), 90., 8.),
        ("centre_up", Vec3::new(0., 1.6, 0.), 0., 40.),
        ("centre_horizon", Vec3::new(0., 1.6, 0.), 0., 12.),
        ("outside_looking_in_up", Vec3::new(radius_m + 800., 120., 0.), -90., 25.),
    ];
    // STORM_DISTS=25,100,300: instead of the fixed views, stand that many metres inside the wall facing it
    let dist_views: Vec<(String, Vec3, f32, f32)> = std::env::var("STORM_DISTS").ok().map(|s| s.split(',').filter_map(|d| d.trim().parse::<f32>().ok()).map(|d| (format!("inside_{d}m_from_wall"), Vec3::new(radius_m - d, 1.6, 0.), 90., std::env::var("STORM_PITCH").ok().and_then(|p| p.parse().ok()).unwrap_or(4.))).collect()).unwrap_or_default();
    let views: Vec<(String, Vec3, f32, f32)> = if dist_views.is_empty() { views0.iter().map(|(a, b, c, d)| (a.to_string(), *b, *c, *d)).collect() } else { dist_views };
    let color = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("color"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: HDR, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
    let depth = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("depth"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
    let (cv, dv) = (color.create_view(&Default::default()), depth.create_view(&Default::default()));
    for (n, (label, eye, yaw, pitch)) in views.iter().enumerate() {
        if only.is_some_and(|o| o != n) { continue; }
        let cam = CameraParams::from_yaw_pitch(*eye, *yaw, *pitch, w, h);
        let (tx, ty) = cam.tan_half();
        let view = ViewParams { eye: cam.eye, forward: cam.forward, up: cam.up, tan_half: (tx, ty), near_m: cam.near, far_m: cam.far, width: w, height: h, time: 3.0, real_time: 3.0, delta_world: 0.0 };
        let frame = world.frame(&view, &params, [0.; 3], fn_fx::uniform::MaterialTime::single(3.0)); // (frame takes the material times now)
        let mut frame = frame;
        match std::env::var("STORM_PARTS").as_deref() { Ok("wall") => { let w = frame.draws.pop(); frame.draws.clear(); frame.draws.extend(w); } Ok("clouds") => { frame.draws.pop(); } _ => {} }
        let ndraws = frame.draws.len();
        fx.set_frame(frame);
        fx.prepare(&dv, 1 + n as u64);
        let mut enc = gpu.device.create_command_encoder(&Default::default());
        {
            let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("clear"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &cv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.12, g: 0.14, b: 0.12, a: 1. }), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &dv, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.), store: wgpu::StoreOp::Store }), stencil_ops: None }), timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
        }
        if fx.has_draws(false) {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("fx opaque"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &cv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })], depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &dv, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }), timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            fx.draw(&mut pass, false);
        }
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("fx translucent"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &cv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })], depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &dv, depth_ops: None, stencil_ops: None }), timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            fx.draw(&mut pass, true);
        }
        gpu.queue.submit([enc.finish()]);
        let px = read_hdr(&gpu, &color, w, h);
        let path = out.join(format!("{n}_{label}.png")); let mut px = px;
        write_png(&path, w, h, &px);
        let changed = px.chunks_exact(4).filter(|p| (p[0] - 0.12).abs() > 0.01 || (p[1] - 0.14).abs() > 0.01 || (p[2] - 0.12).abs() > 0.01).count();
        let max = px.chunks_exact(4).map(|p| p[0].max(p[1]).max(p[2])).fold(0f32, f32::max);
        println!("{label}: {ndraws} draws, {changed} changed pixels, max {max:.2} -> {}", path.display());
        if let Some(e) = &fx.last_error { println!("  fx error: {e}"); }
    }
    Ok(())
}

fn read_hdr(gpu: &Gpu, tex: &wgpu::Texture, w: u32, h: u32) -> Vec<f32> {
    let bpr = (w * 8).div_ceil(256) * 256;
    let buf = gpu.device.create_buffer(&wgpu::BufferDescriptor { label: Some("rb"), size: (bpr * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = gpu.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(h) } }, wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 });
    gpu.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    rx.recv().unwrap().unwrap();
    let data = slice.get_mapped_range().unwrap();
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h { for x in 0..w { let o = (y * bpr + x * 8) as usize; for c in 0..4 { out.push(fn_render::renderer::half_to_f32(u16::from_le_bytes([data[o + c * 2], data[o + c * 2 + 1]]))); } } }
    out
}

fn write_png(path: &std::path::Path, w: u32, h: u32, px: &[f32]) {
    // simple display transform (the harness has no post chain): Reinhard + gamma
    let mut img = image::RgbaImage::new(w, h);
    for (i, p) in px.chunks_exact(4).enumerate() {
        let enc = |v: f32| { let v = v.max(0.); let v = v / (1. + v); (v.powf(1. / 2.2) * 255.).round().clamp(0., 255.) as u8 };
        img.put_pixel((i as u32) % w, (i as u32) / w, image::Rgba([enc(p[0]), enc(p[1]), enc(p[2]), 255]));
    }
    img.save(path).unwrap();
}
