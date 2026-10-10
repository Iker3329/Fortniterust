//! Smoke test of the browser platform layer (fn-web-rt + the patched std). Each check prints `PASS name: detail` or `FAIL name: detail`;
//! the page shows the lines and `tools/web/test/smoke.mjs` (Playwright) asserts them.
#![cfg(target_arch = "wasm32")]
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wasm_bindgen::prelude::*;

fn fnv(b: &[u8]) -> u64 { b.iter().fold(0xcbf29ce484222325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x100000001b3)) }

fn read_range(path: &str, off: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(off))?;
    let mut v = vec![0u8; len];
    f.read_exact(&mut v)?;
    Ok(v)
}

fn check(out: &mut Vec<String>, name: &str, ok: bool, detail: String) {
    let line = format!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    eprintln!("[smoke] {line}");
    out.push(line);
}

const PAK: &str = "/fn/paks/pakchunk0-WindowsClient.pak";

#[wasm_bindgen]
pub fn smoke_threads() -> String {
    let mut out = vec![];
    // 1. spawn from the main thread, join on the main thread (spins)
    let t = Instant::now();
    let r = std::thread::spawn(|| (1..=1000u64).sum::<u64>()).join();
    check(&mut out, "spawn_join_main", r.as_ref().ok() == Some(&500500), format!("{:?} in {:.2} ms", r.ok(), t.elapsed().as_secs_f64() * 1e3));
    // 2. a worker spawns and joins (it can block)
    let r = std::thread::spawn(|| std::thread::spawn(|| 7u32).join().unwrap() * 6).join();
    check(&mut out, "spawn_join_worker", r.as_ref().ok() == Some(&42), format!("{:?}", r.ok()));
    // 3. thread::scope on the main thread
    let mut parts = vec![0u64; 6];
    std::thread::scope(|s| { for (i, p) in parts.iter_mut().enumerate() { s.spawn(move || { std::thread::sleep(Duration::from_millis(5)); *p = i as u64 * 10; }); } });
    check(&mut out, "scope_main", parts == vec![0, 10, 20, 30, 40, 50], format!("{parts:?}"));
    // 4. worker reuse: 64 short threads one after the other, every one a distinct std thread
    let mut ids = std::collections::HashSet::new();
    for _ in 0..64 { ids.insert(std::thread::spawn(|| format!("{:?}", std::thread::current().id())).join().unwrap()); }
    check(&mut out, "thread_identity_after_reuse", ids.len() == 64, format!("{} distinct ids of 64", ids.len()));
    // 5. Instant across threads: an Instant from the main thread, elapsed measured by a worker after a 50 ms sleep
    let t0 = Instant::now();
    let el = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(50)); t0.elapsed() }).join().unwrap();
    check(&mut out, "instant_across_threads", el >= Duration::from_millis(50) && el < Duration::from_millis(500), format!("{:.1} ms", el.as_secs_f64() * 1e3));
    // 6. lock contention: a worker holds a mutex for 30 ms; the main thread locks it (spin) instead of trapping
    let m = Arc::new(Mutex::new(0u32));
    let (m2, started) = (m.clone(), Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let s2 = started.clone();
    let h = std::thread::spawn(move || { let mut g = m2.lock().unwrap(); s2.store(true, std::sync::atomic::Ordering::SeqCst); std::thread::sleep(Duration::from_millis(30)); *g += 1; });
    while !started.load(std::sync::atomic::Ordering::SeqCst) { std::hint::spin_loop(); }
    let t = Instant::now();
    *m.lock().unwrap() += 1;
    let waited = t.elapsed();
    h.join().unwrap();
    check(&mut out, "mutex_contention_main", *m.lock().unwrap() == 2 && waited >= Duration::from_millis(15), format!("main waited {:.1} ms", waited.as_secs_f64() * 1e3));
    // 7. channel recv on the main thread
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(10)); tx.send(99u8).unwrap(); });
    check(&mut out, "channel_recv_main", rx.recv() == Ok(99), "ok".into());
    // 8. available_parallelism
    check(&mut out, "available_parallelism", std::thread::available_parallelism().is_ok(), format!("{:?}", std::thread::available_parallelism().map(|n| n.get())));
    out.join("\n")
}

#[wasm_bindgen]
pub fn smoke_files() -> String {
    let mut out = vec![];
    // 1. stat + read_dir of the paks folder
    let md = std::fs::metadata(PAK);
    check(&mut out, "stat_pak", md.as_ref().map(|m| m.len() > 1 << 30).unwrap_or(false), format!("{:?}", md.map(|m| m.len())));
    let names: Vec<String> = std::fs::read_dir("/fn/paks").map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    check(&mut out, "read_dir_paks", names.iter().filter(|n| n.ends_with(".pak")).count() == 5, format!("{names:?}"));
    // 2. 1 MiB at 1 MiB from the main thread and from a worker: same bytes
    let t = Instant::now();
    let main = read_range(PAK, 1 << 20, 1 << 20);
    let main_ms = t.elapsed().as_secs_f64() * 1e3;
    let worker = std::thread::spawn(|| { let t = Instant::now(); (read_range(PAK, 1 << 20, 1 << 20), t.elapsed().as_secs_f64() * 1e3) }).join().unwrap();
    let (a, b) = (main.as_ref().map(|v| fnv(v)).ok(), worker.0.as_ref().map(|v| fnv(v)).ok());
    check(&mut out, "ranged_read_main_vs_worker", a.is_some() && a == b, format!("main {a:x?} ({main_ms:.1} ms) worker {b:x?} ({:.1} ms)", worker.1));
    // 3. the pak footer (last 512 bytes: the magic 0x5A6F12E1 must be there)
    let len = std::fs::metadata(PAK).map(|m| m.len()).unwrap_or(0);
    let tail = read_range(PAK, len.saturating_sub(512), 512).unwrap_or_default();
    let magic = tail.windows(4).any(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]) == 0x5A6F_12E1);
    check(&mut out, "pak_footer_magic", magic, format!("{} bytes read", tail.len()));
    // 4. many small sequential reads (BufReader) hit the block cache
    let (r0, _, h0) = fn_web_rt::vfs::stats();
    let mut br = std::io::BufReader::with_capacity(8192, std::fs::File::open(PAK).unwrap());
    let mut buf = vec![0u8; 4 << 20];
    let ok = br.read_exact(&mut buf).is_ok();
    let (r1, _, h1) = fn_web_rt::vfs::stats();
    check(&mut out, "buffered_reads_use_cache", ok && r1 - r0 <= 8, format!("{} requests, {} cache hits for 4 MiB in 8 KiB reads", r1 - r0, h1 - h0));
    // 5. an extracted asset file, whole
    let s = std::fs::read("/fn/assets/sky/dome_optimized.bin").or_else(|_| std::fs::read("/fn/assets/Splash.bmp"));
    check(&mut out, "read_asset_file", s.as_ref().map(|v| !v.is_empty()).unwrap_or(false), format!("{:?} bytes", s.map(|v| v.len())));
    // 6. writable user folder: write, read back, remove
    let p = "/fn/user/smoke-test.txt";
    let w = std::fs::write(p, b"hello from the browser build");
    let back = std::fs::read_to_string(p);
    let _ = std::fs::remove_file(p);
    check(&mut out, "user_folder_write_read", w.is_ok() && back.as_deref().ok() == Some("hello from the browser build"), format!("{w:?} {back:?}"));
    // 7. read-only folders refuse writes
    check(&mut out, "paks_read_only", std::fs::write("/fn/paks/x.txt", b"no").is_err(), "write refused".into());
    // 8. env + paths
    check(&mut out, "env_and_paths", std::env::var("LOCALAPPDATA").is_ok() && std::env::temp_dir().starts_with("/fn/tmp") && std::env::current_exe().is_ok(), format!("{:?} {:?}", std::env::var("LOCALAPPDATA"), std::env::current_exe()));
    out.join("\n")
}

/// WebGPU: adapter + device awaited on the main thread, the canvas cleared every animation frame (a slowly cycling colour) until `frames` frames.
#[wasm_bindgen]
pub async fn smoke_gpu(frames: u32) -> String {
    let mut out = vec![];
    let window = web_sys::window().unwrap();
    let canvas: web_sys::HtmlCanvasElement = window.document().unwrap().get_element_by_id("canvas").unwrap().dyn_into().unwrap();
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::BROWSER_WEBGPU, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let surface = match instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone())) { Ok(s) => s, Err(e) => { check(&mut out, "gpu_surface", false, e.to_string()); return out.join("\n"); } };
    let adapter = match instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, compatible_surface: Some(&surface), ..Default::default() }).await {
        Ok(a) => a, Err(e) => { check(&mut out, "gpu_adapter", false, e.to_string()); return out.join("\n"); }
    };
    let info = adapter.get_info();
    let (device, queue) = match adapter.request_device(&wgpu::DeviceDescriptor { required_features: adapter.features() & (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::RG11B10UFLOAT_RENDERABLE | wgpu::Features::FLOAT32_FILTERABLE), ..Default::default() }).await {
        Ok(d) => d, Err(e) => { check(&mut out, "gpu_device", false, e.to_string()); return out.join("\n"); }
    };
    check(&mut out, "gpu_device", true, format!("{} ({:?}), features {:?}, max_texture_2d {}", info.name, info.backend, device.features(), device.limits().max_texture_dimension_2d));
    let (w, h) = (canvas.width().max(1), canvas.height().max(1));
    let mut config = surface.get_default_config(&adapter, w, h).unwrap();
    let caps = surface.get_capabilities(&adapter);
    config.format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(config.format);
    surface.configure(&device, &config);
    let t0 = Instant::now();
    for i in 0..frames {
        next_frame().await;
        let Ok(frame) = surface.get_current_texture().into_result() else { continue };
        let view = frame.texture.create_view(&Default::default());
        let mut enc = device.create_command_encoder(&Default::default());
        let c = (i as f64 / 60.0).sin() * 0.5 + 0.5;
        enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: c, b: 0.6, a: 1.0 }), store: wgpu::StoreOp::Store } })], ..Default::default() });
        queue.submit([enc.finish()]);
        queue.present(frame);
    }
    let s = t0.elapsed().as_secs_f64();
    check(&mut out, "gpu_clear_frames", true, format!("{frames} frames in {s:.2} s ({:.1} fps), format {:?}", frames as f64 / s, config.format));
    out.join("\n")
}

trait IntoResult { fn into_result(self) -> Result<wgpu::SurfaceTexture, ()>; }
impl IntoResult for wgpu::CurrentSurfaceTexture {
    fn into_result(self) -> Result<wgpu::SurfaceTexture, ()> { match self { wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => Ok(t), _ => Err(()) } }
}

async fn next_frame() {
    let p = js_sys::Promise::new(&mut |res, _| { let _ = web_sys::window().unwrap().request_animation_frame(&res); });
    let _ = wasm_bindgen_futures::JsFuture::from(p).await;
}
