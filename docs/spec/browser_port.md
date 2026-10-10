# Browser port (wasm32 + WebGPU)

Two versions from one codebase:

| version | how to start | what it is |
|---|---|---|
| **exe** | `fortnite-rs.exe` as before (`cargo g`, `dist\FortniteRuntime.exe`) | unchanged: native window, DX12 / Vulkan, the user's own settings |
| **browser** | `dist\fortnite-web.exe` (opens the game as a tab of your Brave: `http://127.0.0.1:8780/`) | the same game compiled to wasm32, rendered with WebGPU, background loading on Web Worker threads, quality chosen and adapted automatically |

The browser version has two ways to get the game files:
* **launcher mode** (default): `fortnite-web.exe` serves the page and reads the user's own 1.8 install and extracted assets from this PC (localhost only);
* **remote mode** (the user's private site, e.g. `https://fortniteog.lol/fortnite/`): the page asks for a password and then reads the files from the
  user's own private storage through their file Worker (`tools/web/fn-web-worker`); see [Remote mode](#remote-mode-private-hosting).

Status: see [Stage log](#stage-log) at the end (what is verified, with evidence paths).

## Build and run

```bash
tools/web/build.sh            # dist/web/ (page + wasm) and dist/fortnite-web.exe
tools/web/build.sh --smoke    # the platform-layer smoke test instead (dist/web-smoke/)
dist/fortnite-web.exe         # serves dist/web + your own 1.8 files on localhost and opens the page in Brave
```

**Playing (and recording):** double-click `dist\fortnite-web.exe`. It finds the 1.8 install and the extracted assets the way the exe does, starts the
local server and opens the game as a normal tab of your Brave (a running Brave gets a new tab; F11 = fullscreen). `--app` opens a maximized app
window instead (no tabs / address bar). The normal start-up follows: splash, start-up loading, the game picker (Enter or click Battle Royale), the
match load (the loading screen stays up until the props' and the first-use effects' pipelines are compiled), the world. Click the game to capture the
mouse; Esc releases it and opens the menu. A small ring at the bottom right turns while the game is working. The chosen quality is shown at the top
left for 8 s after every change. For a fixed daytime start: `fortnite-web.exe --page-args "FN_TOD_HOUR=12.5"`; without the note:
`--page-args "fnweb_note=off"` (both: `"FN_TOD_HOUR=12.5&fnweb_note=off"`).

* First run installs what is missing: rustup toolchain `nightly-2026-10-06` with `rust-src`, and `wasm-bindgen-cli 0.2.129`. Nothing else changes in the
  workspace: no `rust-toolchain.toml`; the native build keeps the stable toolchain and its own target dirs. Build dirs: `D:/target-fn-web` (wasm,
  patched std sources, patched wgpu), `D:/target-fn-web-native` (the launcher); override with `FNWEB_TARGET_DIR` / `FNWEB_NATIVE_TARGET_DIR`.
* `fortnite-web.exe` finds the game files like the exe does: `--paks` / `--assets` flags, else `FN_PAKS` / `FN_ROOT` / `FN_ASSETS`, else a
  `FortniteRuntime.toml` / `paths.txt` beside it, else the known developer locations. Other flags: `--port N` (default 8780; a busy port falls back to
  a free one), `--no-browser`, `--browser <exe>`, `--browser-args "..."` (extra browser flags), `--app` (app window instead of a tab),
  `--page-args "k=v&..."`, `--user <dir>`, `--quiet`. (`--tab`, the old opt-in for the tab, is accepted and is the default now.) Test only:
  `--cors <origin> --test-password <pw>` make it answer like the private file Worker (CORS, `/login`, bearer tokens, the path in `X-Fnvfs-Path`).
* Page options (URL query): `args=...` appends native command-line flags (e.g. `?args=--skip-loading`); `fnweb_quality=off`, `fnweb_rung=N`,
  `fnweb_note=always|off`, `fnweb_audio=on` (sound, off by default: see Memory), `FN_WARM_WEB=0` (no first-use warm-ups), `fnweb_pool_cap=N`,
  `fnweb_heaptrack=1`, `fnweb_nogame=1` (tests: runtime and file system only), `FN_READLOG=1` (what the load reads from the paks:
  `__fnweb.fnweb_read_report(40)`, saved by `launch.mjs` as `reads.txt`); any other `KEY=value` becomes an environment variable of the game.
* Tests (Playwright; Brave with a temporary profile, never the user's), `npm install` in `tools/web/test` once, game runs under the GPU lock
  (`tools/web/test/locked.sh "<what>" <command>`):
  * `smoke.mjs` (platform layer); `play.mjs --args "--skip-loading" [--maximize] [--walk] [--profile]` (the game);
  * `launch.mjs` (the user's path: the launcher opens Brave, splash -> picker -> world, pointer lock, Esc menu). `--headless` without a window on
    the user's screens; `--cpu-profile` (main thread, Enter -> first frame: `cpuprof.mjs <out>/main-thread.cpuprofile [--from/--to ms] [--focus re]
    [--frames ms --frames-match re]` lists every long main-thread task with what it did); `--trace [--trace-categories gpu.dawn,...]` (a browser
    trace incl. the GPU process: `tracestat.mjs <out>/trace.json [--from-mark stage:Finishing --to-mark stage:Playing] [--match re]`); `--remote`
    (remote mode end to end, see below); `--twice` (a second launch in the same tab); `--profile-dir <dir>` (keep the browser profile);
  * `script.mjs <file.script> --env K=V ...` (an input script in the browser with a CPU profile, e.g. `verification/first_use/first_use.script`);
  * `remote.mjs [--worker <url>]` (remote mode against the file Worker: login, reads through the service worker, case, cache; 25 checks).

**Game files are never part of the page.** Nothing of Epic's is in `dist/web` or in the site folder. In launcher mode the launcher listens on 127.0.0.1
only, answers only requests addressed to localhost (DNS-rebinding guard) and reads the files from the user's disk on request.

### Remote mode (private hosting)
The user's private site serves the page; the game files come from the user's own private storage (Cloudflare R2, not public) through their file
Worker (`tools/web/fn-web-worker`, the same read-only `__fnvfs` API as the launcher plus `POST /login`), only after the site password.

```bash
tools/web/build.sh --no-launcher
tools/web/site.sh --vercel "C:/Users/Brady/Documents/JB/fortniteog" --worker https://<the file Worker> [--cache-version N]   # -> <site>/fortnite/ + vercel.json
tools/web/site.sh --to <dir> --worker http://127.0.0.1:8787                                                                 # a local test folder
```

`site.sh` copies the page files only (index.html, the wasm and JS, `sw.js`) and writes `fnweb-remote.json` (`{"worker": ..., "cacheVersion": ...}`);
with `--vercel` it also merges `vercel.json`: COOP same-origin / COEP require-corp / CORP same-origin / `no-cache` for `/fortnite/(.*)` (the shared
wasm memory needs a cross-origin isolated page) and a `/fortnite` -> `/fortnite/` redirect. It deploys nothing.

How the page runs in remote mode (only when `fnweb-remote.json` is beside it; launcher mode is untouched):
1. it registers `sw.js` (scope: its folder) and waits until it controls the page, so the game's worker threads start under it;
2. the login form: password -> `POST <dir>/__fnvfs/login` (the service worker passes it to the Worker's `/login`) -> `{token, expires}` (12 h), kept
   in this tab's `sessionStorage`; a reload in the same tab does not ask again;
3. `fnweb_vfs_remote(<dir>, token)`: the game's file system (fn-web-rt `vfs.rs`) now requests `<dir>/__fnvfs/<op>?p=<path>` on the page's own origin
   with `Authorization: Bearer <token>` (a header: the token is never in a URL);
4. the service worker answers from Cache Storage (`fnweb-files-v<cacheVersion>`, keyed by op + path + query + range; the caches of other versions are deleted) or asks the Worker:
   `<worker>/__fnvfs/<op>` with the path in `X-Fnvfs-Path` (one URL per op, so one CORS preflight per op per 2 h instead of one per file), the
   token and the Range passed on, and keeps the reply for the next launch;
5. the storage is case-sensitive (the Windows folders were not): every path is turned into its exact case through the root's cached tree before it
   is requested; `/fn/user` (the settings) lives in the browser (memory + the page's IndexedDB `fnweb-user`; worker threads post their writes to the
   page); `/fn/windows` and `/fn/bin` do not exist (the UI falls back from Segoe UI to the game's NotoSans); file blocks are 1 MiB; bundles stop at
   the Worker's 8 MB.

## Architecture

```
 Brave tab (cross-origin isolated: COOP same-origin + COEP require-corp)
 ├─ main thread: winit (canvas, input, rAF frames) ─ App (the exe's App) ─ wgpu WebGPU device / all GPU work
 ├─ Web Worker pool (started before Rust runs) ─ std::thread jobs (loaders, pak decode, clip decode, culling) + 2 I/O threads (main-thread reads)
 └─ shared WebAssembly.Memory (max 4 GiB)            │
                                                     ▼ synchronous XHR (Range)
 launcher mode:  fortnite-web.exe (localhost) ── /            dist/web (page, wasm, js)
                                              ── /__fnvfs/*  /fn/paks · /fn/assets (read-only) · /fn/user (settings) · /fn/bin
 remote mode:    <site>/fortnite/ (static) + sw.js (Cache Storage) ── <worker>/__fnvfs/* (+ /login) ── the user's private storage
```

### The game module
`tools/web/fn-web` is a separate cargo workspace whose library root **is** `crates/fn-app/src/main.rs`: the browser build compiles exactly the exe's
module tree (so modules other agents add to `main.rs` are picked up automatically; its dependency list must be kept in sync with
`crates/fn-app/Cargo.toml`). `main.rs` declares one extra module only for wasm32: `tools/web/fn-web/src/web.rs` (entry `fnweb_game_main`).

### std on the web: the FNWEB platform layer
Rather than editing every `std::fs` / `Instant` / `thread::spawn` call (hundreds of sites in crates other agents own), the browser build compiles std
from source (`-Z build-std`) with a small patch (`tools/web/std-patch/fnweb-std.patch`, applied by build.sh to a copy of the pinned nightly's
rust-src, located through cargo's `__CARGO_TESTS_ONLY_SRC_ROOT`). For `wasm32-unknown-unknown` + atomics the patch routes std's platform layer to
`extern "Rust"` hooks implemented in `tools/web/fn-web-rt`:

| std | browser implementation |
|---|---|
| `std::fs::*`, `Path::exists`, `read_dir` | virtual file system over the launcher's or the Worker's HTTP API (`vfs.rs`, below) |
| `Instant`, `SystemTime` | `performance.timeOrigin + performance.now()` (one origin for every thread) / `Date.now()` |
| `thread::spawn` / `scope` / `available_parallelism` | jobs for a pool of Web Workers started by the page before Rust runs (`pool.rs`) |
| `thread::sleep`, `Mutex`, `Condvar`, channels on the **main thread** | spin (the browser main thread may never execute `memory.atomic.wait`); workers block normally |
| stdout / stderr (`eprintln!`) | browser console; worker lines are forwarded to the main thread |
| `env::var`, `args`, `current_exe`, `temp_dir` | page-provided environment (URL parameters); the "exe folder", cwd and home are `/fn/user`, temp `/fn/tmp`, `WINDIR` = `/fn/windows` (only its `Fonts` are served, launcher mode) |
| the allocator (dlmalloc) | grows the shared memory in 32 MiB steps instead of 64 KiB (growing a shared memory is expensive: a profile showed 34 s of `memory.grow` in one load) |

A pool worker runs one std thread after another: between jobs its std "current thread" handle and id are reset (std refuses a second handle on one OS
thread). Thread-locals are not destroyed between jobs (std's TLS slots cannot be revived after destruction); they persist like in any thread pool.
The pool starts with hardwareConcurrency + 4 workers (12 to 24) and keeps a reserve of idle workers (it asks for 4 more whenever idle + requested fall
below the queue + 2): a job the main thread waits for must find a worker at once, because a worker created while the main thread waits cannot start,
and workers can be taken for good (rayon's pool, the audio thread) or blocked on jobs still queued (nested `thread::scope`s). A cap of 40 deadlocked the
first loading step that way (verification/browser/launch6, first attempt); the cap is now only a runaway guard (96, `?fnweb_pool_cap=N`), past which a
queue that makes no progress for 100 ms still gets workers.

File system details (`tools/web/fn-web-rt/src/vfs.rs`): the read-only folders (`/fn/paks`, `/fn/assets`, `/fn/bin`, `/fn/windows/Fonts`) are listed once
(`/__fnvfs/tree`) and every `stat` / `read_dir` / `exists` below them is answered locally, case-insensitively like the Windows folders behind them. Every
request costs a round trip through the browser's network service (a few ms locally, tens of ms over the internet), so:
* reads smaller than a block (256 KiB; 1 MiB in remote mode) go through a shared block cache (96 MiB while loading, 16 MiB once the world is up); a
  read of a block or more is one ranged request;
* the small text files of an asset folder (JSON / txt / ini / csv up to 256 KiB below `/fn/assets/<folder>/`: the props and particle catalogs, hundreds
  of files) arrive in one request (`/__fnvfs/bundle`) the first time any of them is read (start_loading went from 9.4 s to 0.3 s, launch5 -> launch7);
* the main thread's reads (the Window's synchronous XHR can only return text) are made by two dedicated I/O threads started at boot: handed to the
  general pool, they waited behind the loading jobs (the loading screen's first frame waited 6.7 s for its images, launch6);
* the `[fnweb-stats]` console line (every 5 s) shows requests, time inside requests and how long the main thread waited for files.

The settings file lives beside the "exe", i.e. in `/fn/user` = `%LOCALAPPDATA%\FortniteRuntime-web\user` on disk in launcher mode (it persists like the
exe's; not in localStorage: game code writes files from worker threads too), in the page's IndexedDB in remote mode.

### Third-party patch: wgpu
`tools/web/std-patch/fnweb-wgpu.patch` (applied by build.sh to a copy of the registry's wgpu 30.0.1, used through `--config patch.crates-io.wgpu.path`):
* wgpu's WebGPU backend hands WGSL to the browser verbatim, and Chrome's Tint enforces derivative uniformity (naga does not check it): the patch
  prepends `diagnostic(off, derivative_uniformity);` on line 1 of every module that does not set it (line numbers of compiler messages stay right);
* `Device::fnweb_create_render_pipeline_async(desc) -> PendingRenderPipeline` (WebGPU backend only): the same descriptor mapping as
  `create_render_pipeline` through `GPUDevice.createRenderPipelineAsync`; `poll()` gives the pipeline (or its error) once the browser resolved it.

One DXBC-translated pattern also needed a web-only fix: `bitcast<u32>(bitcast<f32>(0xffffffffu))` is a NaN constant expression, which Tint rejects; on
wasm fn-dxbc's codegen folds the round trip to the integer literal (`codegen.rs` `conv`, cfg wasm32; native text unchanged).

### Pipelines: compiled asynchronously in the browser
In the browser `createRenderPipeline` returns at once, but the GPU process compiles the pipeline (WGSL -> HLSL -> DXC) synchronously on its main thread:
0.5-1.1 s for one of the translated UE4 programs. Every frame and every queue write of the page waits behind it (`DawnClientSerializer::GetCommandSpace`
on the page thread). A browser trace of one load (verification/browser/launch8) showed 155 pipelines, 17.5 s, and the first-use freezes of new
building pieces (0.6-0.8 s each, verification/browser/first_use1) were this.
* `fn-render/src/fx.rs` + `fx_async.rs` (wasm only): the FX pipelines (`create_pipeline`, `create_scene_lit_pipeline`, `create_shadow_pipeline` and
  their depth / velocity twins) go through `createRenderPipelineAsync`, compiled in parallel on the browser's worker threads; until a program's
  pipelines have all resolved its pipe holds a placeholder and its draws are skipped (the object appears a few frames later instead of freezing).
  `FxRenderer::pending_pipelines()` (0 natively); the loading screen's Finishing stage waits for it (`App::finishing_frame`, capped at 30 s).
* Content dedup (both builds): a pipeline request identical to an earlier one (the whole description but the label, and its kind) gets the existing pipe
  (FX pipelines are never destroyed; bind groups are made against the pipe's layouts). `pipeline_cache_report()`; the browser logs it at the first frame
  (a load: 28 made, 25 reused; with the first-use warm-ups 104 made, 36 reused).
* With that the first-use warm-ups (`building_render::prebuild_step`, `pickups_fx::prebuild_step`: every standard piece, its effects and the pickup
  effects drawn once during the loading screen) run in the browser too (`?FN_WARM_WEB=0` turns them off).
* Not converted (synchronous): the renderer's own pipelines (made at load: ground 0.9 s, char scene 0.8 s, backdrop 0.4 s, 12 UI materials 1.3 s,
  ...) and `gun_material.rs` (gun pass, made at load).

### What had to change in the game code (all `#[cfg(target_arch = "wasm32")]`, native behaviour identical, unless marked "both")
* **GPU objects stay on the main thread.** With wasm atomics, wgpu's WebGPU types are `!Send`. The native loader builds the renderer and uploads
  everything from a worker thread; the browser build runs the same CPU work on the same worker pools and does every GPU step (renderer creation,
  prop / ground-cover / backdrop uploads, `Scene::new`) on the main thread, one step per frame (`tools/web/fn-web/src/loader_web.rs`).
* **No blocking on GPU promises.** Adapter / device are awaited before the event loop starts (`Gpu::new_async`); validation error scopes are popped
  without waiting (`fn_render::gpu::pop_error_scope`); the per-frame surface scope in `Gfx::acquire` is skipped on the web.
* **Frame pacing** by requestAnimationFrame (`ControlFlow::Wait` + `request_redraw`); the frame limiter is not used on the web.
* **Window**: the canvas fills the page; window-mode / resolution changes are no-ops (`platform.rs`); pointer lock replaces the cursor clip.
* **Sound** (opt-in, `?fnweb_audio=on`): fn-audio's cpal output uses cpal's WebAudio host (feature `wasm-bindgen`, enabled from
  `tools/web/fn-web/Cargo.toml` only); the page resumes the AudioContext on the first click or key.
* **Finishing** waits for the FX pipelines still compiling (`App::finishing_frame`, `fx.pending_pipelines() == 0`, always true natively).
* **Building warm-up reads** (both, a no-op natively): `building_render::prewarm` reads the piece classes and their meshes inside
  `fn_pak::retain_reads`, so the warm-up's `load_def` / `load_mesh` on the page's main thread find them in memory.
* **Static props' mip chains** (both): built on the loading thread (`static_loader::prepare_kind` / `prepare_ground_cover` ->
  `statics::prebuilt_mips`, handed to `create_material` by `statics::with_prebuilt_mips`): the same functions on the same images, off the render
  thread (1-4 s of the browser's main thread per load before).
* **Props catalog** (both): `props::discover` parses each props folder once per process (df_hook's first frame parsed it a second time).

### fn-render edits (owned by the optimization agent; small, behaviour-preserving)
* `gpu.rs`: `Gpu::new_async` (+ `new` = `block_on(new_async)`), `backends_from_env` returns `BROWSER_WEBGPU` on wasm, `pop_error_scope` helper;
  `TEXTURE_COMPRESSION_BC` requested when the adapter has it (both; Brave has it: the BC passthrough uploads cooked blocks).
* `fx.rs` (3 sites), `gun_material.rs`, `scope_pass.rs` (2 sites): `pollster::block_on(scope.pop())` -> `crate::gpu::pop_error_scope(scope)`.
* `fx.rs`: the pipeline-creation calls through `render_pipeline` (async on wasm), the draw skips of pending programs, `web_poll` at the top of
  `prepare`, `pending_pipelines`, the content dedup (both) and `pipeline_cache_report`; new file `fx_async.rs` (wasm only).
* `Cargo.toml`: `wasm-bindgen-futures` for wasm32 only.

### fn-pak (load time + memory, both builds; same results)
* `glob_match` matches ASCII paths without allocating (a parity test checks it against the char version) and `PakSet::glob` first checks the literal
  suffix after the last wildcard. `ClipStore::load` globbed once per animation clip: ~1.1 s per clip natively, 7-10 s in the browser.
* One mounted index per install for the whole process: every later `open_install` / `open_dir` / `open_files` of the same paks is a cheap clone
  sharing it, and a thread that asks for paks another thread is mounting waits for that mount (the game opens the install in ~20 places, some on the UI
  thread in play; each re-parse of the 238k-entry index cost 0.1-0.2 s natively, ~1 s in the browser). Test: `reopening_shares_the_live_index`.
* Browser only: no memory map; the footer, the index and each entry are one ranged read each, through per-thread file handles; `retain_reads(|| ..)`
  keeps the entries read in the closure (LRU, 96 MB, `drop_retained_reads()`) for later reads of the same entries; `FN_READLOG=1` tallies the
  stored bytes and reads per entry (`read_log_report`).

### The launcher (`crates/fn-web-serve`, `fortnite-web.exe`, std only)
Serves `dist/web` and the virtual folders (`/__fnvfs/stat|list|tree|file|bundle|mkdir|rmdir|rename`), with COOP/COEP headers so the page is cross-origin
isolated, a localhost-only Host check, and keep-alive connections. `bundle` returns the small files of a folder with the asked extensions in one body
(records: path length, path, size, bytes; at most 32 MiB). The path may also come in `X-Fnvfs-Path`. Tests: range parsing, path containment, percent
decoding, bundle contents.

## Differences from the exe
| | exe | browser |
|---|---|---|
| quality | the user's saved settings | chosen and adapted automatically (below); the saved settings are not changed |
| window | windowed / borderless / fullscreen, chosen resolution | a browser tab; the canvas fills it (F11 = the browser's fullscreen); 3D resolution scaled by the auto quality |
| frame pacing | vsync or the frame limiter | requestAnimationFrame (the display's refresh) |
| mouse | cursor clip + raw input | pointer lock (click the game; Esc releases it and opens the menu, as in the exe) |
| settings file | beside the exe | launcher mode: `%LOCALAPPDATA%\FortniteRuntime-web\user\FortniteSettings.ini`; remote mode: the page's IndexedDB |
| audio | WASAPI | off by default; `?fnweb_audio=on`: WebAudio (starts with the first click / key; +~300 MB heap) |
| gamepad | XInput (gilrs) | off for now |
| new pipelines | compiled synchronously | compiled asynchronously by the browser; an object whose pipeline is not ready yet is drawn a few frames later |
| shader validation errors | caught per module (fallback paths) | reported asynchronously in the console; a broken module shows up as later validation errors |
| logs | `fortnite-rs.log` + stderr | the browser console (F12) |

## Memory design (browser)
The wasm32 address space is 4 GiB and the whole game lives in one shared `WebAssembly.Memory`; GPU resources (textures, buffers) live in the
browser's GPU process, outside it. What counts is what Rust keeps on the CPU side. Measured with the build's own counting allocator (live / peak bytes,
`[fnweb-stats]` every 5 s) and an allocation-site tracker (`?fnweb_heaptrack=1`, `__fnweb.fnweb_heap_report(60)` in the console).
* One pak index for the whole process (fn-pak): -500 MB (boot2 -> boot3).
* Worker thread stacks: 4 MiB each in the heap (Rust gives spawned threads 2 MiB natively); a normal load uses 28-37 workers.
* Texture predecode (`fn_fx::gpu::PREDECODE_BUDGET`): 128 MB in the browser (512 MB natively). Held RGBA8 chains raised the peak heap ~0.5 GB
  (launch3, budget then set to 0); with the BC passthrough the held chains are the cooked blocks, ~5x smaller.
* Sound is off by default: the decoded cues added ~300 MB (launch6: steady 2.23 GB, wasm memory 3.1 GB).
* File block cache: 96 MiB while loading, trimmed to 16 MiB 15 s into play (the small-file bundles are dropped then too).
* The allocator grows the memory in 32 MiB steps.

Current (launch16, BC passthrough, predecode 128 MB, first-use warm-ups on): steady heap 1.90 GB, peak 2.41 GB, wasm memory 2.78 GB of 4 GiB.

## Auto quality (browser)
`tools/web/fn-web/src/quality_web.rs` (a child module of `app.rs`, wasm only). Target: 60 fps, or the display's refresh when that is lower. Every frame
records the presented-frame interval (requestAnimationFrame pacing) and the GPU time of the frame from WebGPU timestamp queries when the adapter has
them. Every window of at least 1 s / 30 frames is judged on its 90th-percentile interval: too slow (p90 > 1.15 x target, and the GPU time, when known,
above 0.75 x target: a slow window with an idle GPU is the main thread or a first-use hitch, which a lower rung would not fix) two windows in a row =
one rung down; comfortably fast (p90 < 0.85 x target and GPU < 0.70 x target) three windows in a row = one rung up. After every change one window is
not judged, nor are the first 4 s of play. A rung that had to be left downward within 15 s of being reached upward is blocked from going up again for
30 s, doubling each time. The rungs (best first):

| rung | 3D resolution | shadows | effects | post | view distance |
|---|---|---|---|---|---|
| 0 | 100 % | epic | epic | epic | epic |
| 1 / 2 | 90 / 80 % | epic | epic | epic | epic |
| 3 | 80 % | high | epic | epic | epic |
| 4 | 75 % | high | high | epic | epic |
| 5 | 70 % | high | high | high | epic |
| 6 | 70 % | medium | high | high | high |
| 7 / 8 | 62 / 55 % | medium | medium | high / medium | high |
| 9 / 10 | 50 % | medium / low | medium | medium | medium |
| 11 | 42 % | low | low | low | medium |

The resolution goes first (cheapest to give up: UE's upscale pass), then the scalability groups in the exe's own tables (`fn_render::quality`). The
first rung is the rung the last session settled on when its canvas was about the same size (`/fn/user/fnweb-quality.txt`), else a guess from the
canvas size measured on the RTX 2060 (>= 3 MP starts at 3, >= 2 MP at 2, >= 1.2 MP at 1, else 0). The chosen settings are printed in the console
(`[auto-quality] ...`) and shown at the top left of the page. `?fnweb_quality=off` disables it, `?fnweb_rung=N` starts at a rung.

## Known gaps
* Load time: Enter -> first frame 25-38 s in headless Brave on a quiet machine (26.7 s in launch14; the exe ~5 s); with the first-use warm-ups on,
  the loading screen stays up longer. What remains is mostly main-thread work the browser cannot move to a worker as the exe does, because WebGPU
  objects live on the main thread: sRGB mip building in `StaticRenderer::create_material`, `Scene::new` (model textures), the guns' material build,
  material parse / DXBC translation of first-use materials. Their CPU halves can move to workers once split from the GPU upload.
* The renderer's own pipelines and the gun pass are still compiled synchronously by the browser (about 6 s of GPU-process time during the load).
* Gamepad is off in the browser build (the Gamepad API is the next step). Sound is opt-in (memory).
* Remote mode: the first launch downloads ~1.5 GB through the service worker (1 MiB blocks); the second launch in the same profile reads it from Cache
  Storage. The real deployment (the user's Worker URL, `site.sh --vercel`) is not tested yet: only against the local `wrangler dev` (3 test files) and
  the launcher's Worker emulation with the real files.

## Stage log
### Stage 1 (boot)
* 2026-10-06: platform-layer smoke test in Brave: all 21 checks pass (`node tools/web/test/smoke.mjs`; `verification/browser/smoke.png`,
  `verification/browser/smoke-console.txt`).
* Game boot in Brave (`play.mjs --args "--skip-loading" --profile`), 1600x900, the user's 1.8 paks through the launcher:

  | run | change | world loaded | frames flowing | HTTP requests | heap after load |
  |---|---|---|---|---|---|
  | `verification/browser/boot1` | first boot | 335 s | ~340 s | 9 833 | 2 270 MB |
  | `boot2` | glob without allocations | 149 s | 150 s | 9 866 | 2 313 MB |
  | `boot3` | main-thread fetches by workers, SIMD, shared pak index | 83 s | 86 s | 9 623 | 1 790 MB |
  | `boot4` | allocator grows in 32 MiB steps, cached read-only trees | **16 s** | **~35 s** | 2 124 | 1 783 MB |

* Native after the browser changes: fn-render, fn-ui-wgpu, fn-app, fn-pak, fn-dxbc and fn-web-serve tests pass; the native forest walk
  (`verification/browser/native-walk`) runs as before (RESULT ok, load 7.2 s, 117 fps at 1280x720). Later: `cargo check -p fn-render` / `-p fn-app`
  pass with the async-pipeline, dedup and fn-pak changes; fn-pak (10) and fn-web-serve (4) tests pass.

### Stage 2 (memory)
Brave, maximized window on the 2560x1440 display (launch.mjs / play.mjs --walk) or headless 2544x1224:

| run | what | steady heap | peak heap | wasm memory | workers |
|---|---|---|---|---|---|
| `walk2` | 2560x1255, 80 s walk | 1 558 MB | 1 999 MB | 2 368 MB | 35 |
| `launch2` | user's path, 30 s walk | 1 594 MB | 2 052 MB | 2 400 MB | 36 |
| `launch3` | + texture predecode (RGBA8) | 1 588 MB | **2 537 MB** | **2 880 MB** | **57** |
| `launch4` | predecode off on wasm; the user played (built, pickaxed) | 1 579 MB | 1 998 MB | 2 368 MB | 34 |
| `launch6` | + sound (WebAudio) | 2 228 MB | 2 746 MB | 3 136 MB | 44 -> sound made opt-in |
| `launch15` | first-use warm-ups on | ~1 820 MB | 2 182 MB | 2 560 MB | 28 |
| `launch16` | + BC passthrough, predecode 128 MB (raw blocks) | 1 901 MB | 2 410 MB | 2 784 MB | 36 |

### Stage 3 (auto quality, demo path, load time)
* `launch2` / `launch3`: one launch (the launcher opens Brave, picker, Enter, world): first in-game frame 37.9 s after start (30.4 s after Enter),
  steady 70-84 fps at 2560x1369, auto quality settled at rung 1; pointer lock and the Esc menu work. Screenshots `launch3/1-splash.png` .. `6-menu.png`.
* `launch4` (someone played in the test window): 77-91 fps steady once warm, auto quality between rungs 1 and 3.
* `launch5` (headless, CPU profile): Enter -> first frame 54 s with the profiler; the profile found ~10 s of one-file-per-request JSON reads and a
  second pak-index parse on the main thread -> bundles, the shared mount, the I/O threads (launch7: main thread waited 3.5 s for files, was 12.4 s).
* `launch8` / `launch14` (browser traces): 17.5 s of synchronous pipeline compiles in the GPU process -> async FX pipelines; launch14: 43 FX pipelines
  compiled on the GPU process' worker threads (13.2 s of worker time in parallel, 2 ms on its main thread), Enter -> first frame 26.7 s.
* `first_use1` -> `first_use2` (`script.mjs verification/first_use/first_use.script`, headless 2528x1208, epic): frame max 786 -> 292 ms, p99 251 ->
  45 ms, mean 32.8 -> 20.1 ms; building pieces: no freeze (prep distance fields max 777 -> 44 ms); weapon install 210-263 -> 1-6 ms (main's fixes).
  Left: the first draw of a new pickup (83-222 ms) and the first pickup effect (82-109 ms).
* Remote mode: `remote.mjs` 25/25 against the local `wrangler dev` (`verification/browser/remote2`); the whole game through the service worker
  (`launch.mjs --remote`, the launcher's Worker emulation with the real files): `remote-cold` Enter -> first frame 25.7 s, 1 532 requests, 908 MB in
  Cache Storage; `remote-twice` (a loaded machine): 74.7 s cold, 54.4 s for the second launch in the same tab (building warm-up 10.3 -> 2.2 s).

### Stage 4 (2026-10-07: remote streaming, current tree)
What changed (browser agent; all browser-only unless marked "both"):
* **Builds again from the current tree**: `fn-terrain` added to `tools/web/fn-web/Cargo.toml`; `loader_web.rs` now loads the POIs (`poi::load`, as native
  `build_world`) and bounds the decoded-prop queue (`sync_channel(6)`: unbounded, it ran the 4 GiB heap out of memory on the start-up screen).
* **Main-thread hang fixed**: `lobby_scene.rs` popped a WebGPU error scope with `pollster::block_on` (never resolves on the page's main thread) -> `fn_render::gpu::pop_error_scope` (both; same natively).
* **Memory**: `fn-fx cooked.rs` decodes a shader map's programs on first use (`Cooked::find`) and drops the SM4 duplicates on wasm: the decoded IR of
  every permutation was ~1.4 GB of the heap (`verification/browser/remote-heap3`). Peak heap with the full map: ~2.5 GB. The heap tracker samples small
  allocations (1:64). `build.sh` patches wasm-bindgen's glue so return pointers above 2 GiB are read unsigned.
* **WebGPU limits**: FX pipelines over the browser's 16 samplers / 16 textures per stage are refused synchronously (`FxRenderer::web_stage_limits`) instead of
  poisoning every frame's command buffer; FX samplers clamp anisotropy unless all filters are linear; asynchronous WebGPU errors are logged, not fatal
  (`gfx.rs`).
* **Cache (`sw.js`)**: file / bundle reads carry `&v=<size>-<upload time>` (vfs.rs `version_of`) and are cached under it: a file replaced in the bucket gets a
  new key, a returning player re-downloads nothing else, and no `--cache-version` bump is needed after uploads. Listings are network-first (cached copy
  offline). Identical in-flight requests are fetched once. `navigator.storage.persist()` is requested.
* **Boot prefetch**: `www/fnweb-boot-manifest.json` (written by `launch.mjs --save-manifest` from a recorded load, `?FNWEB_RECORD=1`) lists every file
  range a load reads; `www/fnweb-prefetch.js` requests them right after the login, 16 at a time, neighbouring ranges merged into one request (gap <= 1 MB,
  run <= 32 MB; the service worker cuts the run into the game's own cache entries). Page shows "game files X of Y MB". `?fnweb_prefetch=off` disables it.
* **POI streaming without main-thread reads** (`tools/web/fn-web/src/stream_web.rs`, a wasm child module of `statics_render`): worker lanes read each POI
  kind's packages (retained), parse its materials and decode its textures before `poi_build` builds it; tiers: what the main thread asks now (never
  capped), the resident groups, the 1.5x ring (both capped at 48 read-but-unbuilt). `?FNWEB_POI_PREFETCH=0` turns it off.
* **Loading gate** (both): the lobby (game picker) already builds the spawn's POI groups, building pieces and pickup effects (`picker_frame`, 4 ms a frame);
  Finishing waits for them, the landscape materials (`landscape_render::prewarm` on a worker + `prebuild_step`) and the async pipelines, with progress
  ("Preparing the world (n / m)"); the browser's give-up is 240 s (30 s natively).
* **Input**: the page owns Pointer Lock (`__fnweb_lock`, `unadjustedMovement` with fallback); a refused / dropped lock is released by the game so the next
  click retries; keys the canvas misses are forwarded to it; Play enters fullscreen with `navigator.keyboard.lock(['Escape'])` (`?fnweb_fullscreen=off`).
* **Measurement**: `[hitch]` (every frame > 20 ms in the first 60 s of play with its cause, summary at 60 s), `[perf]` (every 10 s: fps, frame interval,
  main-thread time, GPU time + top passes), `[spawn]` (pawn vs landscape vs collision floor), `[fnweb-heap]` load checkpoints. `launch.mjs`: `--latency-ms` /
  `--mbps` (stand-in Worker over a simulated internet), `--page-port`, `--lobby-secs`, `--console-test`, `--play-profile`, `--heap-report`,
  `--stand-in-assets`, `--site-overlay`, `--save-manifest`.
