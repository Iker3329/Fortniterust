//! BROWSER PORT: the entry of the browser build and its page glue. Compiled only for wasm32, as a module of the game's own module tree
//! (`crates/fn-app/src/main.rs` declares it), so it can drive the same start-up as `run_main`:
//!
//! 1. the page (`index.html` / `fnweb-boot.js`) instantiated the module on shared memory and started the Web Worker pool, then calls
//!    [`fnweb_game_main`] with the command line (`--assets /fn/assets --paks /fn/paks` + URL options);
//! 2. the WebGPU adapter and device are awaited here (the browser's main thread may not block on them), on the page's canvas;
//! 3. then exactly the native start-up: options, asset roots, settings check, `App::new`, and winit's event loop (`spawn_app`: it returns at once,
//!    the browser drives every frame through requestAnimationFrame).
use std::cell::RefCell;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

thread_local! {
    static GPU: RefCell<Option<(wgpu::Surface<'static>, fn_render::Gpu)>> = const { RefCell::new(None) };
    static CANVAS: RefCell<Option<web_sys::HtmlCanvasElement>> = const { RefCell::new(None) };
}

/// `Gfx::new` on the web: the surface + device created before the event loop started.
pub fn take_gpu() -> Result<(wgpu::Surface<'static>, fn_render::Gpu), String> {
    GPU.with(|g| g.borrow_mut().take()).ok_or_else(|| "the WebGPU device was not created (fnweb_game_main must run first)".into())
}

/// The game window's attributes on the web: the page's canvas, sized by the page (CSS fills the viewport; winit follows it with a ResizeObserver).
pub fn window_attributes(mut a: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    use winit::platform::web::WindowAttributesExtWebSys;
    a.inner_size = None;
    a.min_inner_size = None;
    let canvas = CANVAS.with(|c| c.borrow().clone());
    a.with_canvas(canvas).with_prevent_default(true).with_focusable(true)
}

/// The start-up flow entered a stage (`Splash`, `StartupLoading`, `GamePicker`, `Loading`, `Finishing`, `Playing`): tell the page.
pub fn stage(name: &str) {
    let f = js_sys::Reflect::get(&js_sys::global(), &"__fnweb_phase".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    if let Some(f) = f { let _ = f.call1(&JsValue::NULL, &format!("stage:{name}").into()); }
}

/// The game's event loop ended (Quit, Esc on a start-up screen, or a failure): tell the page.
pub fn exited(code: i32) {
    eprintln!("[web] the game loop ended (exit code {code})");
    let f = js_sys::Reflect::get(&js_sys::global(), &"__fnweb_phase".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok());
    if let Some(f) = f { let _ = f.call2(&JsValue::NULL, &"exited".into(), &JsValue::from(code)); }
}

fn page_status(text: &str) {
    let _ = js_sys::Reflect::get(&js_sys::global(), &"__fnweb_status".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()).map(|f| f.call1(&JsValue::NULL, &text.into()));
}

fn page_fatal(text: &str) {
    match js_sys::Reflect::get(&js_sys::global(), &"__fnweb_fatal".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) {
        Some(f) => { let _ = f.call1(&JsValue::NULL, &text.into()); }
        None => web_sys::console::error_1(&text.into()),
    }
}

/// Entry of the browser build (called once by the page after the worker pool is up). `args` is the native command line without the program name.
#[wasm_bindgen]
pub async fn fnweb_game_main(args: Vec<String>) -> Result<(), JsValue> {
    // panics: std's message (stderr -> console) plus the page's error overlay
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        let msg = format!("The game stopped: {info}");
        if fn_web_rt::is_main_thread() { page_fatal(&msg); }
    }));
    // the main thread's file requests get their own threads, started while the worker pool is idle (fn_web_rt::vfs::start_io_threads)
    fn_web_rt::vfs::start_io_threads();
    fn_pak::webcook::set_main_thread_check(fn_web_rt::is_main_thread);
    let mut argv = vec!["/fn/bin/fortnite-rs.exe".to_owned()];
    argv.extend(args.iter().cloned());
    fn_web_rt::env::set_args(argv);
    // every "default install" fallback of the game honours FN_ROOT: point them at the launcher's paks folder
    fn_web_rt::env::set("FN_ROOT", Some("/fn/paks"));
    let url_env = fn_web_rt::env::import_url_params();
    eprintln!("[web] fortnite-web {} starting: args {args:?}, url options {url_env:?}", env!("CARGO_PKG_VERSION"));

    // 1. the canvas
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let canvas: web_sys::HtmlCanvasElement = match document.get_element_by_id("fnweb-canvas") {
        Some(e) => e.dyn_into().map_err(|_| "#fnweb-canvas is not a canvas")?,
        None => { let c: web_sys::HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?; c.set_id("fnweb-canvas"); document.body().ok_or("no body")?.append_child(&c)?; c }
    };
    let dpr = window.device_pixel_ratio();
    let (cw, ch) = (canvas.client_width().max(1) as f64 * dpr, canvas.client_height().max(1) as f64 * dpr);
    canvas.set_width(cw as u32);
    canvas.set_height(ch as u32);
    CANVAS.with(|c| *c.borrow_mut() = Some(canvas.clone()));

    // 2. WebGPU: adapter + device awaited on the main thread
    page_status("requesting the WebGPU device");
    let instance = fn_render::Gpu::create_instance();
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone())).map_err(|e| format!("cannot create the WebGPU canvas surface: {e}"))?;
    let gpu = fn_render::Gpu::new_async(instance, Some(&surface)).await.map_err(|e| { page_fatal(&e); e })?;
    let info = gpu.adapter.get_info();
    eprintln!("[web] WebGPU: {} ({:?}), device features {:?} (adapter: {:?}), limits: max_texture_2d {}, max_buffer {} MB, max_storage_binding {} MB", info.name, info.backend, gpu.device.features(), gpu.adapter.features(), gpu.device.limits().max_texture_dimension_2d, gpu.device.limits().max_buffer_size >> 20, gpu.device.limits().max_storage_buffer_binding_size >> 20);
    GPU.with(|g| *g.borrow_mut() = Some((surface, gpu)));

    // 3. the native start-up (main.rs run_main without the process / console parts)
    page_status("starting the game");
    let args = crate::audio::strip_cli(args);
    let opts = crate::cli::parse(&args).map_err(|e| { let m = format!("bad options: {e}"); page_fatal(&m); m })?;
    crate::diag::set_dialogs(false);
    let opts = crate::paths::apply_defaults(opts, &crate::assets::exe_dir(), &crate::paths::Known::developer());
    crate::check_options(&opts).map_err(|e| { page_fatal(&e); e })?;
    let run = crate::build_run(&opts).map_err(|e| { page_fatal(&e); e })?;
    let (roots, source) = crate::resolve(&opts).map_err(|e| { let m = e.text().to_owned(); page_fatal(&m); m })?;
    eprintln!("[assets] roots: {:?}  source: {}", roots.roots, match &source { crate::loader::Source::Dir => "asset tree (glTF + md5anim)".to_owned(), crate::loader::Source::Paks(p) => format!("paks {}", p.display()) });
    crate::check_settings_file(&opts);
    let _ = std::thread::Builder::new().name("webtex index".into()).spawn(fn_fx::webtex::warm);
    let event_loop = winit::event_loop::EventLoop::new().map_err(|e| format!("cannot start the event loop: {e}"))?;
    let app = crate::app::App::new(opts, roots, source, run);
    use winit::platform::web::EventLoopExtWebSys;
    event_loop.spawn_app(app);
    page_status("");
    Ok(())
}

/// `FN_READLOG=1` (URL `?FN_READLOG=1`): what this session read from the paks, by folder and largest entries (console / test harness).
#[wasm_bindgen]
pub fn fnweb_read_report(top: u32) -> String { fn_pak::read_log_report(top as usize) }

// ---- POINTER LOCK (2026-10-07) ---------------------------------------------------------------------------------------------------------------
// The page owns the browser's Pointer Lock (index.html `__fnweb_lock`): `requestPointerLock({ unadjustedMovement: true })` (raw counts, like the
// exe's raw input; plain request where that is refused), inside the click that asks for it. The request is asynchronous and may be refused (no user
// gesture, Chrome's cool-down after the user left the lock with Esc, a hidden tab), so the game does not assume a capture succeeded: `lock_refused`
// tells it when a requested lock never came, and it releases its capture so the next click asks again (platform.rs / app.rs).
thread_local! { static LOCK_ASKED_MS: std::cell::Cell<f64> = const { std::cell::Cell::new(0.) }; }

fn page_fn(name: &str) -> Option<js_sys::Function> { js_sys::Reflect::get(&js_sys::global(), &name.into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) }
fn now_ms() -> f64 { js_sys::Date::now() }

/// The game wants the pointer captured (`true`) or released.
pub fn pointer_lock(want: bool) {
    LOCK_ASKED_MS.with(|c| c.set(if want { now_ms() } else { 0. }));
    if let Some(f) = page_fn("__fnweb_lock") { let _ = f.call1(&JsValue::NULL, &JsValue::from(want)); }
}
/// The page holds the pointer lock right now.
pub fn pointer_locked() -> bool { page_fn("__fnweb_locked").and_then(|f| f.call0(&JsValue::NULL).ok()).and_then(|v| v.as_bool()).unwrap_or(false) }
/// A lock the game asked for (and still wants) is not held 0.8 s later (refused, or lost without an Esc reaching the game).
pub fn lock_refused() -> bool {
    let asked = LOCK_ASKED_MS.with(|c| c.get());
    asked > 0. && now_ms() - asked > 800. && !pointer_locked()
}

#[wasm_bindgen]
pub fn fnweb_audio_ring() -> Vec<u32> { fn_audio::device::web::ring_layout().unwrap_or_default() }
