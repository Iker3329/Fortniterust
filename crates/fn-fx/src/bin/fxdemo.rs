//! Headless harness: renders the weak-point ring (original programs) over a flat grey scene at a few effect times and writes PNGs.
//! `fxdemo <out dir> [width height]`
use fn_fx::gpu::{SharedBuffers, TextureCache};
use fn_fx::numbers::{draws as number_draws, LiveNumber, NumberAssets};
use fn_fx::ring::{draws, RingAssets, RingEffect};
use fn_fx::view::{bytes, ViewParams};
use fn_render::fx::*;
use fn_render::{CameraParams, Gpu};
use glam::Vec3;
use std::collections::HashMap;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
use fn_render::wgpu;

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(args.get(1).ok_or("out dir")?);
    let (w, h) = (args.get(2).and_then(|s| s.parse().ok()).unwrap_or(960u32), args.get(3).and_then(|s| s.parse().ok()).unwrap_or(540u32));
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let gpu = Gpu::headless()?;
    eprintln!("gpu: {}", gpu.describe());
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let mut fx = FxRenderer::new(&gpu.device, &gpu.queue, HDR, DEPTH);
    let view_buf = fx.create_buffer("view", fn_fx::view::VIEW_FLOAT4S * 16);
    let prim_bufs: Vec<FxBufId> = (0..4).map(|_| fx.create_buffer("primitive", 16 * 16)).collect();
    let sprite_vf = fx.create_buffer("sprite vf", 10 * 16);
    fx.write_buffer(sprite_vf, &bytes(&fn_fx::view::sprite_vf_buffer()));
    let light = fx.create_buffer("fx light", 48);
    let shared = SharedBuffers { view: view_buf, primitive: prim_bufs[0], sprite_vf, light };
    let mut cache = TextureCache::default();
    let t0 = std::time::Instant::now();
    let assets = RingAssets::load(&set, &mut fx, &mut cache, shared)?;
    eprintln!("assets loaded in {:.2}s; notes: {:?}", t0.elapsed().as_secs_f32(), assets.notes);
    let numbers_assets = NumberAssets::load(&set, &mut fx, &mut cache, shared)?;
    eprintln!("number assets loaded; sections: standard {:?}, crit {:?}", numbers_assets.standard.sections.iter().map(|s| s.path.clone()).collect::<Vec<_>>(), numbers_assets.crit.sections.iter().map(|s| s.path.clone()).collect::<Vec<_>>());
    let mut live: Vec<LiveNumber> = vec![];
    let mut binds: HashMap<(String, usize), Vec<(u32, FxBindId)>> = HashMap::new();
    for (path, m) in &assets.materials { for (s, b) in prim_bufs.iter().enumerate() { binds.insert((path.clone(), s), m.bind_variant(&mut fx, *b, None)); } }

    let cam = CameraParams::from_yaw_pitch(Vec3::new(0., 1.6, 0.), 0., -8., w, h);
    let (tx, ty) = cam.tan_half();
    let spot = Vec3::new(0.15, 1.1, -2.5);
    let mut effect = RingEffect::spawn(&assets, 1, spot, true, 0, 7);

    let color = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("color"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: HDR, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
    let depth = gpu.device.create_texture(&wgpu::TextureDescriptor { label: Some("depth"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
    let (cv, dv) = (color.create_view(&Default::default()), depth.create_view(&Default::default()));

    let dt = 1. / 60.;
    let shots = [0.12f32, 0.2, 0.35, 0.5, 0.8, 1.1];
    let mut shot_i = 0;
    let mut t = 0f32;
    let hit_time: Option<f32> = std::env::var("FXDEMO_HIT").ok().and_then(|s| s.parse().ok());
    for frame_no in 0..240 {
        if let Some(ht) = hit_time { if t >= ht && effect.hit_at.is_none() { effect.hit(); } }
        effect.update(&assets, dt, (Vec3::from(cam.eye) - spot).length() * 100.);
        t += dt;
        let view = ViewParams { eye: cam.eye, forward: cam.forward, up: cam.up, tan_half: (tx, ty), near_m: cam.near, far_m: cam.far, width: w, height: h, time: t };
        let mut frame = FxFrame::default();
        frame.buffer_writes.push((view_buf, bytes(&view.view_buffer())));
        { let s = fn_render::lighting::sun_direction(); let r = fn_render::lighting::sun_radiance(); let k = fn_render::lighting::sky_radiance(); frame.buffer_writes.push((light, bytes(&[[s.x, s.y, s.z, 0.], [r[0], r[1], r[2], 0.], [k[0], k[1], k[2], 0.]]))); }
        if frame_no == 6 { live.push(LiveNumber::spawn(&numbers_assets, 0, spot + Vec3::new(-0.35, 0.0, 0.0), 50, false, &view, t, 11)); live.push(LiveNumber::spawn(&numbers_assets, 1, spot + Vec3::new(0.35, 0.0, 0.0), 100, true, &view, t, 12)); }
        number_draws(&numbers_assets, &live, t, &mut frame);
        draws(&assets, &effect, &view, &binds, &prim_bufs, &mut frame);
        let ndraws = frame.draws.len();
        fx.set_frame(frame);
        fx.prepare(&dv, 1);
        if shot_i < shots.len() && t >= shots[shot_i] {
            let mut enc = gpu.device.create_command_encoder(&Default::default());
            {
                let _clear = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("clear"), color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &cv, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.12, g: 0.14, b: 0.12, a: 1. }), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &dv, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }), timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
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
            let path = out.join(format!("ring_{:03}ms.png", (t * 1000.) as u32));
            write_png(&path, w, h, &px);
            let lit = px.chunks_exact(4).filter(|p| (p[0] - 0.12).abs() > 0.01 || (p[1] - 0.14).abs() > 0.01 || (p[2] - 0.12).abs() > 0.01).count();
            println!("frame {frame_no} t={t:.3}: {ndraws} draws, {lit} changed pixels -> {}", path.display());
            shot_i += 1;
        }
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
    // simple exposure-1 Reinhard-ish display transform (the harness has no post chain)
    let mut img = image::RgbaImage::new(w, h);
    for (i, p) in px.chunks_exact(4).enumerate() {
        let enc = |v: f32| { let v = v.max(0.); let v = v / (1. + v); (v.powf(1. / 2.2) * 255.).round().clamp(0., 255.) as u8 };
        img.put_pixel((i as u32) % w, (i as u32) / w, image::Rgba([enc(p[0]), enc(p[1]), enc(p[2]), 255]));
    }
    img.save(path).unwrap();
}




