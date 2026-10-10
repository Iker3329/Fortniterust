# fn-postfx

UE 4.16 (Fortnite 1.8) post effects that `fn-render` lacks, as wgpu passes the renderer can slot in, with CPU references and the evidence tools
that decided which effects are active in a 1.8 gameplay frame. Library + two binaries:

* `postfx-evidence` -> `reports/postfx-evidence.json` (pure Rust: `fn-pak` + `fn-uasset`; optional `--features dxbc` for shader listings)
* `postfx-demo --screenshot` -> `reports/postfx-demo/*.png`, `perf.json` (synthetic scene through every effect at 1280x720, GPU-vs-CPU errors, per-pass GPU times)

This crate does **not** touch `fn-render`; nothing here requires an edit there beyond the integration below.

## What is active in 1.8 (baseplate, Athena Day, pickaxe camera)

The old extraction (`reports/day-postprocess.json`, 11 values) read only the **child delta** of `TODM_A_Child_Athena`. `TODM_A` (its parent
Blueprint) carries ~130 `bOverride_*` flags and most of the values; they are merged in `evidence::merge_phase`. Rule used for every field:
tagged value + `bOverride_X` true = **applied (RECOVERED)**; flag true but no tag = the C++ constructor default (**ENGINE_DEFAULT**, number recalled
from the 4.16 source, the install has no copy); tagged but flag not true = **ignored** by the volume blend.
`Athena_Terrain.umap` names `TODM_A_Child_Athena_C` as its time-of-day manager (`WorldTimeOfDayManager`), has no `PostProcessVolume`, and no
`Athena_PlayerCameraMode*` package carries any post-process property. The unreferenced prototype `Athena_TerrainTOD2` would set `MotionBlurAmount = 0`.

| effect | active | parameters (Day) | status here |
|---|---|---|---|
| Depth of field | **yes, nominal**: `DOFM_Gaussian`, far layer only | focal 15000 cm, focal region 0 (default), near transition 0, far transition 50000 cm, **near size 0 (layer off)**, **far size 0.02 %**, sky focus 200000 cm; vignette/occlusion/depth-blur tagged but **not overridden** | implemented. Far blur radius = 0.26 px @1280 (0.38 px @1920): the kernel collapses to one tap, what remains is the half-res setup/upsample softening of everything beyond ~275 m and of the sky |
| Motion blur (camera) | **yes** | amount 0.125, max 2 %, per-object size 0.5 (default) | implemented (tiled pipeline of 4.16: flatten CS -> gather CS -> `MotionBlurPS<4>`); a 1.5 deg/frame turn at 80 deg FOV blurs 1.9 px per side @1920 |
| Motion blur (objects) | **yes if a velocity buffer exists** | same | implemented; **fn-render must write a velocity target** (spec below) |
| Bloom | **yes** | intensity 0.675, threshold **-1** (exe ctor: no setup pass); sizes 0.3/1/2/10/30/**0 (off)**; tints 0.3465/0.138/**0.5**/0.066/0.066/**0**, each x TintScale **1/6**; `BloomSizeScale` **1.0**; no dirt mask | `bloom.rs` (`BloomChain`), **wired into fn-render** (BLOOM HOOK). Every host constant checked against the UE 4.16 source and the 1.8 exe decomp (see the module doc) |
| Volumetric fog | **undecidable from data -> not implemented** | `bEnableVolumetricFog` untagged on the Athena fog component; the fork's default is unknown; `TODM_Disabled` serializes it `false`, lobby/hexmap fogs serialize `true`; phase structs carry tuned Scattering/Absorption values (0.1 / 0.5) | even if on, optical depth over the stock 60 m volume is 0.0013 at Day (~0.1 % in-scatter) |
| Lens flare | **no** | `LensFlareIntensity` = 0.0 tagged, flag true | - |
| Scene colour fringe | **no** | flag true, untagged => default 0 | - |
| Ambient occlusion | yes (not ported) | intensity 0.56422, **radius 160 cm**, **bias 2.4**, fade 550000/200000 (never fades), mip threshold 0.009, rest default | parameters only |
| Vignette | yes, negligible | **0.01** (corner mask 0.9996) | fn-render assumes 0.4 (corner 0.57) |
| Eye adaptation | yes | min 0.75, **max 1.0**, bias 0.47; `HistogramLogMin/Max` tagged but ignored | fn-render assumes max 2.0 |
| Light-shaft bloom | yes, small (sun glow) | `bEnableLightShaftBloom`, `BloomScale` 0.1 | not implemented |

Also recovered and missing in the fn-render grading: `ColorSaturationShadows/Midtones` = 1.25 (overridden). The scan also found the storm
post-process `OutsideSafeZonePP0` on `PlayerPawn_Athena` (bloom 2.5, DOF far 2.8 %, fringe 0.7, vignette 1.1; it uses the same Bloom3Tint 0.5 /
Bloom6 off as the TOD, which independently confirms those are not engine defaults); it is not part of a baseplate frame.

Evidence files: `reports/postfx-evidence.json` (merged settings per phase, scan of all 113538 packages, effect table with provenance per value,
cvar scan: 0 packages bake a post-process cvar, 68 of 68 `*.ini` are encrypted so `r.*` defaults are **assumed** UE 4.16 code defaults, shader list),
`reports/postfx-shaders/*.asm` (+ `hex/` with bit-exact literals): the compiled shaders ported here, disassembled by `fn-dxbc` from
`Engine/GlobalShaderCache-PCD3D_SM5.bin` (877 DXBC blobs, named from the type-name strings that precede them: `shadercache.rs`).

## Integration contract

```rust
let mut fx = PostFx::new(&device, PostFxFormats { hdr: Rgba16Float });
// once per frame, before the post pass, with an encoder that is submitted after:
let r = fx.encode(&queue, &mut encoder,
    &PostFxInputs  { scene_color, depth, velocity /*Option*/, params: &PostFxParams },
    &PostFxOutputs { color: &post_color, bloom: Some(&bloom_half) });
// r.color_written  : false => neither DOF nor motion blur ran, keep using scene_color (post_color untouched)
// r.bloom_written  : bloom_half now holds the bloom image
```

* `scene_color`: linear HDR (`Rgba16Float`), `TEXTURE_BINDING`, **before exposure / tonemap**, size = `params.view.{width,height}`.
* `depth`: depth-only view (`TextureAspect::DepthOnly`) of the scene depth, `TEXTURE_BINDING`. `params.depth` = `DepthTransform::from_perspective(near, far)`
  for standard 0..1 depth (UE's `InvDeviceZToWorldZTransform` for other conventions).
* `post_color`: `Rgba16Float`, `RENDER_ATTACHMENT | TEXTURE_BINDING`, full size. `bloom_half`: same format, `half(width) x half(height)`.
* Units: DOF / motion-blur distances are centimetres (the serialized unit); `params.cm_per_unit` = 100 for a renderer in metres.
  `PostFxParams::athena_day(depth, view, exposure, cm_per_unit)` returns the recovered Day values (`params::athena_day_cm()`; a test pins them to the install).
* `params.exposure`: the eye-adaptation exposure scale of the previous frame (only the bloom threshold uses it, as in UE).
* `ViewParams::screen_to_world_rows`: `[right*tan_x, up*tan_y, forward]` in a basis whose `.z` is world up (only the DOF sky rule `saturate(3*normalize(ray).z)` uses it;
  for a y-up renderer pass the axes permuted so up lands in `.z`). `clip_to_prev_clip = prev_view_proj * cur_view_proj.inverse()` (same NDC/depth convention as `depth`).
* GPU timing: `fx.enable_timing(&queue)` (needs `TIMESTAMP_QUERY`) then `fx.read_timings()` after submit (blocking; the demo uses it). Not wired into `fn-render`'s profiler.

### Where it plugs into `fn-render`'s chain (UE order: DOF -> motion blur -> eye-adaptation metering -> bloom -> tonemap)

Current chain: shadows -> sky -> scene -> HDR mips -> exposure meter -> post (AO, bloom taps, exposure, vignette, LUT/film, sRGB). With this crate:

1. after the `scene` pass: `fx.encode(...)`; DOF/motion blur read `hdr_mip_views[0]` + depth and write `post_color`.
2. build the HDR mip chain / exposure metering from `post_color` when `color_written` (UE meters after DOF/MB), else from the scene target.
3. `post.wgsl`: replace `bloom_color()` by `textureSampleLevel(bloom_tex, samp, uv).rgb * BloomIntensity` and apply it **before** exposure:
   `c = (c_after_ao + bloom) * exposure` (UE: `(scene*tint + bloom*intensity) * exposure`, verified in `FPostProcessTonemapPS`; the bloom image is un-exposed).
4. tonemapper constants to correct at the same time: vignette 0.01 (not 0.4), `AutoExposureMax` 1.0 (not 2.0), min 0.75, bias 0.47, AO radius 160 cm / bias 2.4.

### Velocity buffer (motion blur of dynamic objects) - what fn-render must write

`Rgba16Float`, full size, cleared to 0. For every skinned/dynamic draw (character, pickaxe; static world needs nothing) write
`rg = cur_ndc.xy - prev_ndc.xy` (NDC units, y up, after perspective divide, previous frame = previous bone matrices AND previous view-projection) and `a = 1`.
Pixels with `a = 0` take their velocity from depth + `clip_to_prev_clip` (camera motion). Without the texture only camera motion blurs.
(UE stores `enc = (v + 2.003978) / 4.008016` in an RG16 target and tests `enc.x > 0`; the float + alpha form is equivalent and avoids the 16-bit-unorm feature.)

## Verification (RTX 2060, Vulkan, `cargo test --release -p fn-postfx [--features dxbc]`)

* **DXBC oracle** (`tests/dxbc_oracle.rs`, `--features dxbc`): the original compiled pixel shaders are executed with the `fn-dxbc` interpreter and compared to the CPU ports:
  `FPostProcessDOFSetupPS10` worst rel. diff 1.7e-7, `DOFRecombinePS100` 1.1e-6, `BloomSetupPS` 0, `MotionBlurPS4` 0 (uniform branch) / 1.2e-7 (weighted branch, 909 samples).
  The two compute shaders (velocity flatten / gather: TGSM + UAV) cannot be interpreted; they are ported from the listings and checked only against the CPU port.
* **GPU vs CPU** (`tests/gpu_vs_cpu.rs`, error = |a-b| / max(1,|ref|) so HDR values above 1 are relative; Rgba16Float targets are modelled in the CPU path):
  DOF Athena Day 0.57/255 max, DOF strong (3 % blur) 0.75/255, bloom 1.16/255 (HDR peak 19.5), motion blur camera+object 2.61/255 max (26 of 230400 px above 1/255, hard HDR edges: GPU bilinear weights are 8-bit),
  motion blur object-only (weighted branch) 0.30/255, 0 px above 1/255; flatten/tiles/gather textures within 2e-3.
  At 1280x720 in the demo the motion-blur max is 13/255 on the 30x HDR sun edge (mean 0.006/255).
* Visible effect of the **Athena Day** values on the synthetic frame (tonemapped, 8-bit): DOF mean 0.08/255 (max 166 on 1-px sun/pole edges), motion blur mean 0.45/255 (silhouette edges only).
* GPU time (RTX 2060, mean of 200 frames, `reports/postfx-demo/perf*.json`): 1280x720: DOF 0.139 ms (setup 0.056, blurs 0.022, recombine 0.061), motion blur 0.295 ms (flatten 0.130, gather 0.018, blur 0.146), bloom 0.105 ms, **all three 0.495 ms**.
  1920x1080: DOF 0.274, motion blur 0.659, bloom 0.213, **all three 1.197 ms**.

## Recalled, not recovered (host-side C++ is not in any shader)

**Now verified (2026-10-05, UE 4.16 source at `C:\UE4.16` + 1.8 exe decomp), see `bloom.rs`:** Gaussian taps `exp(-16.7 (x/R)^2)` in bilinear pairs, max 32
samples (exe `FUN_1418ff170` / `FUN_1418ef700`), radius `Size * level width / 200` (exe `* 0.005`) for bloom AND Gaussian DOF, fast blur (`r.FastBlurThreshold` 7),
border sampling, `TintScale = 1/6`, stage table {3,3,4,5,6}, threshold <= -1 skips the setup pass, downsample offset 1 source texel / uv = pixel*2/src.
The old guesses (3-sigma kernel, radius `size% * width`, tints not divided by 6, setup pass always on) made the in-game bloom ~6x too strong.

Still **ASSUMED** (each is one named constant in `cpu.rs` / `gpu.rs`): motion-blur velocity scale
(`amount * 0.5 * W/2` px per side, i.e. amount 1 = full-frame shutter), max velocity (`MotionBlurMax% * W * 0.5` px per side), edge-clamp scale `W/2`, no frame-time scaling,
sampler states of the flattened-velocity and tile textures (point) and of the DOF depth fetch (point), the engine defaults listed in `ENGINE_DEFAULTS_ASSUMED`,
and `r.DepthOfFieldQuality > 0`, `r.MotionBlurQuality = 4`, `r.BloomQuality = 5` (CONFIG-VERIFIED 2026-10-06: decrypted `BaseScalability.ini` [PostProcessQuality@3] r.MotionBlurQuality=4, r.DepthOfFieldQuality=2, r.BloomQuality=5; `DefaultScalability.ini` only changes r.LightShaftQuality).

## fn-render integration (BLOOM HOOK, done)

`renderer.rs` owns a `fn_postfx::bloom::BloomChain`: built with the HDR targets, recorded right after the particle pass (so every FX / translucent /
particle draw is in the scene colour before `SceneColorHalfRes`, the UE 4.16 order), its output is binding 5 of the post pass, and `post.wgsl`
computes `(scene + bloom * BloomIntensity) * exposure` (PostProcessTonemap.usf). The eye-adaptation histogram now reads the chain's
`SceneColorHalfRes` (4-tap downsample) instead of HDR mip 1. Replaced: the 3x3 mip-tap approximation (per-tap threshold, total gain 0.754, added after exposure).

## What remains

* DOF near layer, bokeh/circle methods and their vignette variants (inactive in Athena Day), motion-blur quality 1-3 shaders: not ported.
* Volumetric fog: needs one fact (is `bEnableVolumetricFog` true for the Athena TOD): a live capture, or the native `FortTimeOfDay` code.
* Light-shaft bloom, AO (SSAO shader family), the tonemapper corrections above: owned by `fn-render`.
* Compute shaders and the host constants above remain unverified against a running 1.8 client; the first reference screenshot would pin them.
