# FortniteRuntime-rs: architecture and plan

Pure-Rust rewrite of the standalone Fortnite 1.8 (UE 4.16, CL 3724489) runtime. The OpenGL/Win32/Node prototype in `..\FortniteRuntime` stays
the **read-only behavioural reference** until the rewrite reaches parity; nothing here edits it, and the Fortnite client is never run.

Status legend: **done** (in the tree, tested, written by the fn-game agent), **exists / in progress** (crate present in the workspace, owned and still being written by another agent; its status here is only what was visible when this plan was written), **planned**.

## 1. Crates, what each owns, and the dependency graph

| crate | owns | status |
|---|---|---|
| `fn-pak` | `.pak` v1-v8 reader (index, entries) | exists (other agent) |
| `fn-ueio` | minimal cooked-package reader on byte slices (summary, names, imports, exports, tagged properties, bulk data) shared by texture/mesh/anim | in progress |
| `fn-uasset` | full package parser, property reader, Kismet bytecode disassembler | exists (other agent) |
| `fn-inspect` | CLI to inspect installs/packages (replaces `inspect-paks.cjs`, `package-references.cjs`) | exists (other agent) |
| `fn-texture` | Texture2D decode (BCn, raw) to RGBA8/f16 + PNG | in progress |
| `fn-mesh` | SkeletalMesh / Skeleton / StaticMesh decode and glTF export | in progress |
| `fn-anim` | AnimSequence decode + codecs, BlendSpace parser, pose sampler (cooked data) | in progress |
| `fn-export` | extraction CLI: paks -> PNG / glTF / animation / material assets, evidence reports (replaces `export-assets.cjs`, UEViewer, the evidence `.cjs` tools, one by one) | in progress |
| `fn-camera` | Athena third-person camera, recovered mode data, penetration feelers | **done** (fn-game agent) |
| `fn-combat` | pickaxe swing/montage state machine, TriggerAbility notify, melee trace, placeholder harvestables and their mesh recipe | **done** |
| `fn-movement` | `UCharacterMovementComponent` 4.16 port (walk/fall/step/slope/perch/crouch/jump), capsule-vs-world scene, `Controller` | **done** |
| `fn-props` | collision solids of the placeholder props (camera probe sweeps, player blockers) | **done** |
| `fn-animgraph` | recovered AnimGraph (state machines, blend spaces, additive jumps, equip montage, `plan()` -> pose-node tree), `Skeleton` + pose evaluator (mesh-space additive with the tracked-bone mask, HeadTrack, TwoBoneIk, hand retarget) + two-bone IK, `ClipSource`/`ClipLibrary` traits, renderer-free CPU skinning (`skin`: `SkinnedMesh::pose`, rigid-face diagnostics) shared with `fn-pose` | **done** |
| `fn-legacy` | adapter: old md5anim clips + glTF skeletons / skinned part meshes (`load_skinned_part`, `load_head_model`) behind `ClipSource`/`Skeleton`/`SkinnedMesh`; retires when fn-anim/fn-mesh feed the same traits | **done** (temporary) |
| `fn-settings` | settings model, ini persistence, apply layer; `PlatformHost`, `VideoHooks`/`InputHooks`/`AudioHooks`, `LiveVideo`, `NullPlatform`, frame limiter, `version` | **done** |
| `fn-game` | facade + `GameSim` (the old `App::update` as one `step(FrameInput, dt)`), `InputSource`, focus/cursor policy, gamepad mapping (`PadBackend`), jump-air harness | **done** |
| `fn-sim` | deterministic headless harness (lib + `fn-sim` binary): scripted scenarios, per-frame logs, pose hashes, `compare`; parity tests against the old exe's golden logs | **done** |
| `fn-testdata` | finds the old tree (`reports/`, `assets/`, logs) for parity tests (`FN_OLD_ROOT`) | **done** |
| `fn-render` | wgpu renderer: HDR scene, cascaded shadows, sky, UE 4.16 tonemapper, skinned meshes; WGSL shaders | in progress |
| `fn-viewer` | winit viewer and headless screenshot tool for `fn-render` | in progress |
| `fn-ui` | renderer-agnostic UI core: layout, draw list, text, menu toolkit, HUD / pause menu / settings / loading screens as pure logic. Settings screens edit the real `fn-settings` model through an owned `Session` (no globals) fed by a per-frame `UiContext` (`&mut Settings` + `PlatformHost` + `ApplyBackend`); raw winit-agnostic input via `router::InputRouter` (`MenuSystem::input`); `HudState` is the plain HUD contract; `pak_assets::PakAssets` serves UI textures/fonts straight from the install (fn-pak + fn-texture, decode on demand) | in progress |
| `fn-hud-adapter` | `fn_game::GameSim` + `fn_settings::Settings` (+ `HudInputs` for what the game does not model yet) -> `fn_ui::screens::hud::HudState`; `fn-app` only passes it through | done |
| `fn-ui-wgpu` | wgpu backend of the `fn-ui` draw list (quads, glyph atlases, clip, WGSL UI materials) | in progress |
| `fn-ui-demo` | winit demo + screenshot/pixel-diff tool for `fn-ui`; `--live` runs the real menu + Settings + HUD in a window (`--script` drives the full flow itself), `--assets dir|pak|layered|auto` | in progress |
| `fn-app` | **the shipping binary**: winit window + wgpu + audio + gamepad; implements `PlatformHost`, `VideoHooks`, `InputSource`, `PadBackend`; loads extracted assets; drives `fn_game::GameSim` | planned |

Dependency graph (arrows = "depends on"; `fn-testdata` is a dev-dependency of every logic crate and is omitted):

```
                                   fn-app  (winit + wgpu + audio + gamepad)
        ┌───────────┬───────────┬────┴─────┬────────────┬───────────┐
     fn-game     fn-render   fn-ui-wgpu  fn-anim      fn-mesh    fn-texture      (loaders feed the traits fn-animgraph / fn-render define)
     │ │ │ │ │       │          │          │            │           │
     │ │ │ │ └─ fn-settings ─┐  │       fn-ueio ───────┴───────────┘
     │ │ │ └─── fn-animgraph │  fn-ui                                  fn-export ── fn-ueio, fn-pak, fn-texture, fn-mesh, fn-anim, fn-uasset
     │ │ └───── fn-props ────┤                                         fn-inspect ─ fn-pak, fn-uasset
     │ └─────── fn-combat    │                                         fn-uasset ── fn-pak
     └───────── fn-camera ◄──┘  (fn-settings -> fn-camera, fn-movement; fn-props -> fn-camera/-combat/-movement)
                fn-movement
     fn-sim ──► fn-game, fn-legacy (-> fn-animgraph), fn-movement          fn-viewer ► fn-render        fn-ui-demo ► fn-ui, fn-ui-wgpu
```

Rules that keep the graph clean:

* **Logic crates (`fn-camera`, `-combat`, `-movement`, `-props`, `-animgraph`, `-settings`, `-game`) depend only on `glam`** (and each other).
  No windowing, GPU, audio, file-format or OS crate; `unsafe_code = "forbid"` in every one of them (Cargo `[lints]`).
* **Seams are traits**, defined in the logic crates and implemented by the front-end: `fn_settings::PlatformHost` (display modes, window mode,
  vsync, vblank wait, cursor capture, focus; the old Win32/WGL behaviour is documented in `fn-settings/src/platform.rs`), `VideoHooks` (renderer
  settings sink), `InputHooks`, `AudioHooks`, `fn_game::InputSource` (per-frame `FrameInput`), `fn_game::inputdev::pad::PadBackend` (XInput-style raw
  state), `fn_animgraph::ClipSource`/`ClipLibrary` (animation data), `fn_animgraph::Skeleton` (bones).
* **Data flows one way**: pak -> `fn-export` -> files on disk (PNG, glTF, anim, material) -> `fn-app` loaders -> traits. `fn-app` never reads a pak
  at run time unless `fn-export` is linked in for a first-run extraction step.
* The simulation runs on a **fixed 30 Hz tick with render interpolation** (the old exe's default since its `sim_tick.rs`; `FNRT_SIM_HZ=0` = once per frame):
  `fn_game::TickDriver` (`SimClock` accumulator, at most 5 ticks per frame, latched jump / attack taps, interpolation of position / height / facing / camera state /
  bone poses) is the loop `fn-app` calls once per rendered frame, see `crates/fn-game/README.md`. `GameSim::step` is the per-frame (`SIM_HZ=0`) path the harness and the
  movement logs use. Real frame times are clamped to `fn_game::time_audit::MAX_DELTA_TIME` = 0.1 s like the old `App::update` (it was 50 ms until the old tree's
  time audit showed that ran the game in slow motion below 20 fps; the controller's own clamp is 0.25 s, the camera's 0.1 s: nothing clamps tighter). 1.8 itself has a
  30 Hz *server* tick cap and per-frame client prediction; the client-side 30 Hz lock is the user's direction, not a 1.8 file fact.

## 2. Definition of "pure Rust" for this project

1. **Everything that ships or runs is Rust source**: the game logic (above), the renderer, the shaders, the UI, and the extraction pipeline.
   The shipping tree builds with `cargo build --release` and nothing else: no C/C++ compiler, no `build.rs` that shells out, no Node, PowerShell,
   Python, UEViewer or any external tool, no prebuilt blobs.
2. **Dependencies**: crates.io crates written in Rust (`glam`, `winit`, `wgpu`, `naga`, an audio backend, a gamepad backend, `serde_json` for tools).
   Platform bindings that are thin Rust FFI declarations (`windows-sys`, `ash`, `objc2`, ...) are allowed **only inside `fn-app` and the GPU/audio
   backends they come with**; they are never reachable from a logic crate. `cargo tree -p fn-game` must stay free of any `-sys` crate (checked in CI).
3. **`unsafe`**: forbidden in all logic, loader and tool crates. Allowed only where a dependency's API forces it, each use with a `// SAFETY:` note
   and listed in the crate's README. Today there is none in the 10 game-logic crates.
4. **Shaders**: written in **WGSL first** (validated with `naga` in unit tests, compiled by wgpu at run time). The old GLSL (`src/shaders/*.glsl`) and the
   recovered DXBC-derived constants are ported to WGSL one pass at a time, each pass pinned by a numeric/pixel test against the old output.
   After parity, shaders **may** migrate to `rust-gpu` (Rust compiled to SPIR-V, loaded through wgpu's SPIR-V path) pass by pass; that migration is
   optional, gated on rust-gpu toolchain stability, and never blocks a milestone. A pass is "pure" once its source of truth is WGSL or Rust.
5. **Extraction**: `fn-pak` + `fn-ueio` + `fn-texture`/`-mesh`/`-anim` + `fn-export` replace UEViewer (`tools/UEViewer-source`, `umodel`) and the Node
   exporters. A clean machine with only a Fortnite 1.8 install and `cargo` must be able to produce every file the runtime loads.
6. **Research-only analysis scripts are not shipping code.** They produced the *evidence* (`reports/*.json`, generated `src/*.rs` tables) that the
   recovered constants are checked against. They stay in the old tree, are never required to build or run the rewrite, and are ported to Rust **last**
   (section 5) so their outputs stay reproducible from Rust.

## 3. What was moved out of the old tree (fn-game agent) and how

The sources were **copied** (the old tree is read-only and other agents keep editing it) and refactored; the (now retired, PowerShell) `tools/port-from-old.ps1` was the
reproducible record of the mechanical port: it re-imported the listed old files and re-applied every edit (imports, removal of GL/Win32 code, test data
paths). It was deleted when the project went pure Rust (the port is finished, the crates have diverged from the old tree, and a re-run would overwrite them); the crates are the source of truth now.

| old file | new home | change |
|---|---|---|
| `movement_cmc.rs`, `movement_scene.rs`, `movement_params.rs`, `movement.rs`, `movement_tests.rs` | `fn-movement` (`cmc`, `scene`, `params`, `controller`, `tests`) | `Blocker`/`PLAYER_*` moved from propcol into `scene` (no movement -> props dependency); `MOUSE_*` constants no longer test-only |
| `propcol.rs` | `fn-props` | re-exports `Blocker`; sets the prop-collision switch (`set_disabled`) used by the camera-prop scenario |
| `camera.rs` | `fn-camera` | unchanged logic; `include_str!` of a report replaced by a runtime read |
| `combat.rs` | `fn-combat` | OpenGL `World::draw` removed; `prop_vertex`/`prop_normal`/`prop_wobble` expose the same math to any renderer |
| `anim_graph.rs`, `anim_core.rs`, `anim_data.rs` | `fn-animgraph::graph` | unchanged; evaluation moved to `Skeleton` |
| `assets.rs` (pose part: `Model::eval_pose`, masks, IK, `bone_position`) | `fn-animgraph::skeleton` | GL/skinning/texture code stays in the renderer; `Model` -> `Skeleton` + `Pose`; clips behind `ClipSource` |
| `assets.rs` (`Clip` md5anim loader, glTF skeleton build) + its tests | `fn-legacy` | `Md5Clip: ClipSource`; `load_body_skeleton`; tests rebased on `Skeleton` with a small `Model` shim |
| `settings.rs`, `settings_table.rs`, `version.rs` | `fn-settings` | `win` module (Win32/WGL) replaced by `PlatformHost`/`LiveVideo`; `VblankWait::wait` takes a host; `base_level_for` moved in |
| `inputdev.rs` | `fn-game::inputdev` | XInput DLL loading replaced by `PadBackend`; mapping, dead zones, Start latch, wheel pulses unchanged |
| `main.rs` (`App::update`, `update_camera`, `step_anim`, `animation_stage`, `horizontal_step`, focus/cursor helpers) | `fn-game::{sim, host}` | `GameSim::step(FrameInput, dt)`; HUD, menu, GL, window code is not game logic and stays for `fn-app` |
| `jump_air_test.rs`, `movement_series.rs`, `pick_test.rs`, `video_test.rs` (script part), `camera_prop_test.rs`, `main.rs` (`camera_script`) | `fn-game::jump_air`, `fn-sim` | scripts and series ported verbatim; screenshot/GL parts dropped |
| `jump_feel.rs`, `time_audit.rs` | `fn-game::{jump_feel, time_audit}` | mechanical (`cmc::Tuning` replaces the env reads; `clamp_frame_dt` added); `time_audit` keeps its wall-clock bookkeeping, `fn-sim::time_test` is its deterministic twin |
| `sim_tick.rs` | `fn-game::sim_tick` (+ hand-written `fn-game::tick_driver`) | mechanical imports; `TickClock` = `SimClock`; the old `App::update` / `App::sim_tick` split is `GameSim::{begin_frame, tick, snapshot, apply_view}` and `TickDriver` wires them with the latches / snapshot pair |
| `assets.rs` `Clip::key_position` / `sample_at` (engine-exact sampling), `anim_stride.rs` | `fn-legacy::Md5Clip`, `fn-animgraph/src/anim_stride.rs` | hand port of the sampler; the stride table is seeded from the old tree once and then regenerated from the Rust pipeline (`FNRT_WRITE_STRIDE=1 cargo test -p fn-legacy clip_stride_table_is_current`) |
| `reticle.rs`, `hud_vis.rs`, `hud_extra.rs` `CountAnim` | `fn-combat::reticle` (engine), `fn-hud-adapter::feedback` (data contract) | hand port of the renderer-free logic; drawing stays in the UI crates |
| `main.rs` hotbar-selection rule (drop the swing, ignore attack for non-melee) | `FrameInput::{selection_changed, non_melee}` in `fn-game::sim` | hand port |
| `assets.rs` (mesh-space additive tracked mask, `tracked_bones`, `LEGACY_MESH_ADDITIVE`, `skin_and_cache`, `rigid_snapshot`, `bone_in_frame`, `skinned_bone_names`) | `fn-animgraph::{skeleton, skin}` | `MeshAdditiveMode` replaces the global switch; skinning is plain data (`SkinnedMesh`) so `fn-pose` and the renderer share it |
| `face_test.rs` | `fn-sim::face_test` | numbers only (the old run's PNG close-ups need a renderer); identical log lines |
| `movement_cmc.rs` `FNRT_GRAVITY_SCALE` / `FNRT_JUMPZ` | `fn-movement::cmc::{Tuning, Params::with_tuning}` | documented tunable; `Params::default()` is pure, `Tuning::from_env()` is the only env read (the front-end calls it) |
| `post.rs` `METER_EVERY` / `FNRT_METER_EVERY` | note for `fn-render` | auto-exposure is metered every 3rd frame (first frame immediately) from an async readback ring; adaptation (`adapt_ev` with the real dt) still runs every frame, so the cadence changes CPU cost only, not the adaptation speed. `fn-render` meters on the GPU with no readback: no knob needed, only the per-frame real-dt adaptation must be kept |

Not movable (and why): the Win32/WGL implementation of `PlatformHost` (it is OS code: it goes to `fn-app`, specified in `platform.rs`);
`min_track_size` (WM_GETMINMAXINFO) and the raw-input/`ClipCursor` code (Win32); the HUD/quickbar/vitals/menu/loading logic (UI crates own it);
`assets.rs` texture/mesh upload and skinning (renderer); `frameprof`, `post`, `shadow`, `sky`, `lighting` (renderer).

## 4. Milestones

| # | milestone | exit criterion | status |
|---|---|---|---|
| M0 | workspace, shared test data locator | `cargo test -p fn-testdata` finds the old tree | done |
| M1 | game logic extracted | all applicable old tests ported and green; scripted scenarios reproduce the old exe's logs line for line | **done** (section 6) |
| M2 | extraction without UEViewer/Node | `fn-export` regenerates `assets/` byte-identical (PNG/glTF) or numerically identical (anim) to the old export for the files the runtime loads; `fn-inspect` reproduces `reports/pak-index.json` | in progress |
| M3 | loaders feed the traits | `fn-anim` implements `ClipSource`, `fn-mesh` produces `Skeleton` + skinned vertex data; `fn-sim` runs with them and `fn-legacy` is removed; pose hashes of the golden scenarios stay within the tolerance table (section 6) | planned |
| M4 | renderer parity | `fn-render` + `fn-viewer` reproduce the old `verification/*.bmp` captures (scene, sky, shadows, post) within the pixel-diff budget; WGSL ports of every GLSL pass | in progress |
| M5 | UI parity | `fn-ui` layouts match `reports/settings-ui-exact.json` within 1 px; HUD/menu/loading captures match; `fn-ui-wgpu` draws them | in progress |
| M6 | `fn-app` | single binary: window, wgpu, audio, gamepad, settings file, pause menu; `GameSim` driven by real input through `InputSource`; smoke scripted run exits 0 | planned |
| M7 | retire the prototype | every old test mode has a headless or pixel-diff equivalent that passes; old exe no longer needed for any reference | planned |
| M8 | research scripts ported | the Node/PowerShell/DXBC tools of section 5 are Rust subcommands of `fn-export`; optional rust-gpu migration of shaders | planned (last) |

## 5. Research-only analysis scripts (to be ported last) and owners

None of these ship. "Owner" = the crate whose agent ports it into a Rust subcommand/test (`fn-export evidence <name>` unless noted) once M7 is done.

| scripts (old `tools/`) | what they produce | owner |
|---|---|---|
| `kismet-disasm.cjs`, `menu-kismet.cjs`, `hud-bytecode.cjs`, `uasset-props.cjs`, `menu-uasset.cjs`, `package-references.cjs` | Kismet/property dumps used by every evidence tool | `fn-uasset` (the Rust Kismet disassembler already exists; add 4.16 layout parity tests) |
| `dxbc-to-rust.cjs`, `sky-disasm.ps1`, `hud-shaders.ps1`, `tonemap-shaders.ps1`, `settings-ui-disasm.ps1`, `settings-ui-asm.cjs`, `sky-shader-extract.cjs`, `menu-shaders.cjs`, `ui-material-gen.cjs`, `settings-ui-materials.cjs`, `settings-ui-uimat-gen.cjs` (+ `settings-ui-uimat.tpl`) | DXBC extraction/disassembly from shader caches (877 tonemap blobs, sky/HUD/UI materials), generated GLSL/Rust ports | `fn-render` (new `fn-dxbc` helper crate: DXBC container + SM5 disassembler written in Rust; no `d3dcompiler`) |
| `export-assets.cjs`, `export-locomotion.cjs`, `extract-pak-raw.cjs`, `inspect-paks.cjs`, `hud-export-textures.cjs`, `hud-texture-png.cjs`, `sky-mesh-export.cjs`, `export-sky.ps1`, `asset-manifest.json`, `fnroot.cjs/.ps1` | the exported `assets/` tree | `fn-export` (manifest becomes a Rust/JSON config) |
| `camera-evidence.cjs`, `movement-evidence.cjs`, `movement-full-evidence.cjs`, `movement-name-scan.cjs`, `exe-cmc-ctor.cjs`, `montage-evidence.cjs` | `reports/camera-modes.json`, `movement-full.json` (incl. the static read of the exe's CMC constructor), `melee-montage.json` | `fn-game` crates (tests already read these reports; the Rust generators replace them) |
| `state-machine-evidence.cjs`, `animation-graph-evidence.cjs`, `animation-bs-evidence.cjs`, `blendspace-evidence.cjs`, `animation-metadata.cjs`, `animation-quantitative-check.cjs` | `reports/animation-*.json` + generated `anim_data.rs` / `anim_core.rs` | `fn-animgraph` + `fn-anim` (generator emits the same Rust tables; the quantitative check becomes a test over `ClipSource`) |
| `lighting-evidence.cjs`, `sun-shadow-evidence.cjs`, `terrain-grass-evidence.cjs`, `sky-evidence.cjs`, `sky-params-gen.cjs`, `sky-uniforms.cjs`, `version-diff.cjs` | lighting/sky/terrain reports, `sky_params.rs`, 1.8-vs-1.10 diff | `fn-render` (version diff: `fn-export`) |
| `menu-build.cjs`, `menu-dump.cjs`, `menu-evidence.cjs`, `menu-hosts.cjs`, `menu-layouts.cjs`, `menu-locres.cjs`, `menu-material-eval.cjs`, `menu-pages-gen.cjs`, `menu-run.cjs`, `settings-evidence.cjs`, `settings-ui-extract.cjs`, `settings-ui-layout.cjs`, `settings-ui-uniforms.cjs`, `settings-ui-uset.cjs`, `widget-evidence.cjs`, `hud-extra-evidence.cjs`, `hud18-evidence.cjs`, `hud-pkg.cjs`, `hud-save.cjs`, `hud-tree.cjs`, `hud-uniforms.cjs`, `loading-evidence.cjs`, `quickbar-evidence.cjs`, `audio-evidence.cjs` | widget trees, layouts, strings, settings catalog (`settings_table.rs`, `reports/settings-catalog.json`), HUD/loading/quickbar/audio evidence | `fn-ui` (settings catalog generator: `fn-settings` consumes its output) |
| `tools/UEViewer-source`, `umodel`, `BuildTools` | C++ extraction toolchain | retired by M2 (`fn-export`); nothing to port |

Until a script is ported, its **output files remain the committed inputs** of the parity tests (`..\FortniteRuntime\reports\*.json`, read through
`fn-testdata`; a snapshot of the golden logs lives in `testdata/old-logs`).

## 6. Verification strategy

Three layers, each numeric where possible; every claim in the old README that a number depends on has a test that fails if the number moves.

1. **Recovered-data tests** (ported with the code): constants are compared with the evidence reports (`camera-modes.json`, `movement-full.json`,
   `animation-state-machines.json`, `melee-montage.json`, `settings-catalog.json`) read through `fn_testdata::report_text`. When `fn-export` can
   regenerate a report, the same test also runs against the regenerated file and the two files must be equal.
2. **Numeric parity against the old prototype** (`crates/fn-sim/tests/parity.rs`, golden logs in `testdata/old-logs`): every scripted old test mode is
   re-run headlessly and its log must equal the one the old executable wrote, **line for line, tolerance 0**:
   `--movement-test`, `--jump-air-test` (the headless physics/animation-wiring series), `--jump-feel-test` (input -> screen chain at 30..240 Hz, gravity table), `--face-test` and `--face-test --legacy` (face-rigidity numbers of the skinned head part), `--camera-test`, `--camera-prop-test` (with and without prop
   collision), `--camera-video`, `--pickaxe-test`, `--dir-test` (per-frame pelvis/hands/weapon/feet bone positions and the graph's BlendSpace
   weights). `fn-sim compare A B [--tol X]` does the same diff for any pair of logs (e.g. old exe vs a new build after a refactor, or new loaders
   vs `fn-legacy`). Refreshing the goldens = snapshot the old `src`, build with a separate `CARGO_TARGET_DIR`, run the flags next to a junction of the
   old `assets`, copy `verification\*.txt` (procedure in `testdata/old-logs/README.txt`).
3. **Determinism**: `fn-sim` has no wall clock, threads or randomness; two runs give byte-identical logs, including a bit-exact FNV hash of the local
   pose of every bone per frame (`pose` column of `*-frames.txt`). That hash is the regression net for the loader swap (M3): when `fn-anim`/`fn-mesh`
   replace `fn-legacy`, hashes may change only where the decoders legitimately differ, and then the bone-position logs must stay within **1e-4 m**
   (the old exe's own print precision).
4. **Rendering/UI** (M4/M5, owned by the renderer/UI agents): pixel-diff of `fn-viewer`/`fn-ui-demo` captures against the old `verification/*.bmp`
   with per-scenario budgets (mean abs error, max channel error, % of pixels over threshold), WGSL validated by `naga` in unit tests, layout rects
   against `reports/settings-ui-exact.json` within 1 px.

Parity results at the time of writing (M1):

| scenario | lines compared | identical | reference |
|---|---|---|---|
All reference logs were regenerated on 2026-10-03 from ONE snapshot of the old source (06:25) built into a private target dir; the fn-* crates were ported from that same snapshot.

| scenario | lines compared | identical | reference |
|---|---|---|---|
| `--movement-test` | 267 | all | old exe, headless mode |
| `--jump-air-test` | 161 | all | same |
| `--jump-feel-test` (new) | 135 | all | same |
| `--camera-test` | 260 | all | old exe, window mode |
| `--camera-prop-test` after / before | 409 / 409 | all | same |
| `--camera-video` | 961 | all | same |
| `--dir-test` bones (`pickaxe-frames`), graph state (`dir-frames`), captures | 2520 / 2520 / 27 | all | same |
| `--pickaxe-test` bones, captures | 1434 / 45 | all | same (two identical runs, see `testdata/old-logs/README.txt`) |
| `--face-test` / `--face-test --legacy` (new) | 438 / 438 | all | same (head part skinned on the CPU: dev 0.00000 m fixed, 0.19519 m legacy) |
| `--time-test` | - | - | real wall clock, no golden; `fn-sim::time_test` runs the same audit on a virtual clock (unit tests: sim/wall = 1.0 up to 10 fps, only frames > 0.1 s lose time) |

Commands: `cargo test -p fn-camera -p fn-combat -p fn-movement -p fn-props -p fn-animgraph -p fn-legacy -p fn-settings -p fn-game -p fn-sim`
(set `CARGO_TARGET_DIR` outside the repo); `cargo run -p fn-sim --release -- all --out sim-out`; `cargo run -p fn-sim -- compare old.txt new.txt`.

## 7. Re-porting from the old tree (the animation agent keeps editing it)

Historical (the PowerShell tool is retired): `tools\port\last-port.txt` still holds the hashes of the old sources at the last port. The mechanical ones (anim_graph, anim_core,
anim_data, camera, combat, movement_*, propcol, settings*, version, inputdev, jump_feel, time_audit, jump_air_test, pick_test, movement_series, video_test, assets.rs tests)
were re-imported by that script (now done by hand where an old source changes);
the HAND-ported ones are only reported: `main.rs` -> `fn-game::sim` / `host` / `time_audit` (compare `App::update`, `step_anim`, `update_camera`, the focus helpers),
`assets.rs` pose part -> `fn-animgraph::skeleton` / `skin` (diff `eval`, `mesh_additive`, `tracked_bones`, `two_bone_ik`, `apply_pose`), `face_test.rs` -> `fn-sim::face_test`,
`post.rs` -> `fn-render`. After a re-port: `cargo test --workspace`, then refresh the goldens if the scripted behaviour changed (procedure in `testdata/old-logs/README.txt`).

## 8. Working agreements

* The old tree is the reference and is **never edited** from here; the port from it is finished (the former `tools/port-from-old.ps1` is gone); `tools/port/last-port.txt` is the last recorded hash list.
* Each crate has a single owner (table in section 1). Cross-crate changes go through the traits above, not through reaching into another crate's modules.
* New shared dependencies are added surgically to the root `[workspace.dependencies]` (`glam`, `serde_json`, `gltf` today).
* Every behaviour-bearing number carries its provenance word (RECOVERED / EXE / ASSUMED), copied from the old comments; do not upgrade an ASSUMED
  to RECOVERED without evidence.
