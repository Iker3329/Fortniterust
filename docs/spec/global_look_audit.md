# Global look audit (whole-frame pipeline vs UE 4.16 / Fortnite 1.8), 2026-10-05

This audit was stopped early when the coordinator called a wrap-up. Rows marked OPEN were found but not fixed. Rows marked "edited, unverified" were changed by an audit sub-agent that was stopped before it could build or test the change. Captures are in `verification/global_look/`:
- `run.sh` and `look.script` set up the runs.
- `before_h13`, `after_h13`, `before_h20.5` and `after_h20.5` hold the screenshots.
- `sbs_h*_yaw*.png` are side-by-side images, before on the left and after on the right.

The "before" build was taken from the shared working tree, so before/after also includes edits by the other agents working in parallel. For example, the tree canopies that were grey before are fixed in "after"; that is most likely the SpecularMasks sRGB fix in row 4 below.

## Mismatches, ranked by visual impact

| # | Item | Evidence (UE / DXBC / cooked / exe) | Port before | Status |
|---|---|---|---|---|
| 1 | **CombineLUTs `/ 1.05` missing**: the whole frame was 5 % too bright in display space and highlights clipped early | 1.8 CombineLUTs DXBC `GlobalShaderCache#1182809` ends with `mul o0.xyz, r0.xyzx, l(0.952381)`; the 4.16 source is `PostProcessCombineLUTs.usf:409 OutColor.rgb = OutDeviceColor / 1.05`. The tonemap PS `#1070630` then does `* 1.05`. | `post.wgsl` did `min(saturate(enc) * 1.05, 1)`. Mid grey came out at 123/255 instead of 117/255; linear 1.0 at 225 instead of 215; everything above 0.952 encoded clipped to white. | **FIXED** |
| 2 | **No real colour-grading LUT**: the port evaluated the transform analytically per pixel | UE bakes a 32^3 `PF_A2B10G10R10` volume LUT (`PostProcessCombineLUTs.cpp:1022`, `GLUTSize` 32) and samples it trilinearly at `saturate(log2(c)/14 + 0.610727)*31/32 + 0.5/32` (tonemap DXBC). The 1.8 build does NOT subtract `LogToLin(0)` (no such add in either DXBC). | A CPU model of the LUT vs the direct evaluation: mean 0.32/255; 5 % of colours differ by more than 1/255; max 29/255 (saturated darks). | **FIXED**: new `src/color_lut.rs` + `shaders/color_lut.wgsl` build the LUT every frame from the post uniform block (10-bit rounding); post.wgsl binding 10 samples it. The grain is now added after `LUT * 1.05`, before the unorm write (DXBC order). |
| 3 | **Time-of-day post-process blend mechanism**: `AutoExposureBias` 0.47 applies in every phase, not only Day | Exe: TODM ctor `14062c240` sets phase PPC `Priority = i - 4.0` (`DAT_143ff9024` = 4.0, read from the exe) and creates `<Phase>LowPriClonedPPC` for Day/Evening/Night with `Priority = i - 8`, all `bUnbound`. `FUN_14064ee70` sets `PPC[i].BlendWeight = (i <= to) ? 1 : 0`, copies `PPC[i].Settings` into each clone, and gives the clone weight 1. `FUN_14064e920` then sets `PPC[to].BlendWeight = t` during a transition. UE applies volumes in ascending priority (`UnrealClient.cpp:1022 InsertVolume`, `SceneView.cpp:1612`). So the order is: clones Day, Evening, Night (weight 1), then Morning (1), Day, Evening, Night (by weight). Only Day overrides `AutoExposureBias`, and its clone always applies, so the bias is 0.47 at every hour. Every other modelled field is overridden by every phase, so those results are unchanged. | `tonemap::PostSettings::blend` = `from` at weight 1 then `to` at weight t. Evening / Night / Morning get bias 0, which makes them 2^0.47 = 1.39x darker than 1.8. | **OPEN**. Fix: apply clones D, E, N at weight 1, then M at 1, then D/E/N with the weights above. Also check that `time_of_day` `blend` is the weight of `to`. |
| 4 | Prop/foliage SpecularMasks and opacity masks with SRGB=true read raw | 15 SpecularMasks (e.g. `Tree_Large_Canopy_S`, `T_Tree_Large_01_S`, `Shrub_02_S`) and 6 masks (e.g. `HeroTrees0_M`, `Tree_Large_Canopy_M`) are SRGB=true in the paks. UE decodes them when sampling (`D3D11RHIPrivate.h:912`). | `static_loader.rs` used the raw means. Specular and roughness came out too high on canopies, and leaf edges clipped differently. | Edited, unverified (sub-agent): `static_loader.rs` `sampled_channel`. It compiled in my build and the canopies changed; not otherwise verified. |
| 5 | Sun specular min roughness 0.08 missing | `DeferredLightingCommon.usf:164 max(LobeRoughness, MinRoughness)`. The 1.8 deferred light DXBC has a `max … cb1[4].x`; exe `ULightComponent` ctor `0x1420a4c90` stores 0.08 at +0x2d0, and TODM does not override it. | `lit_common.wgsl ggx_spec` had no clamp; the gun path clamped at 0.02. Smooth surfaces got highlights that were too sharp and too bright. | Edited, unverified (sub-agent). `lit_common.wgsl` constant `LIGHT_MIN_ROUGHNESS` 0.08; I renamed it from `SUN_MIN_ROUGHNESS`, which clashed with `backdrop.wgsl` and broke shader validation. Also NoV `saturate(abs(NoV)+1e-5)`, no Vis epsilon, `gun_resolve`/`scene.wgsl` routed through `ggx_spec_color`. Builds and runs; not otherwise tested. |
| 6 | Ocean is a hand-written approximation | `SM_Athena_Water_Ocean` / `SM_OceanFar` actors in `Athena_Terrain` use `M_Athena_Water_Ocean` (exact package path unconfirmed). | `backdrop.wgsl` / `backdrop_load.rs:67-71`; normal tiling and colour mix are assumed. | OPEN |
| 7 | Sky dome / clouds / sun use the wrong shader variant, and the phase interpolation is partial | `Athena_Terrain` WorldSettings `WorldTimeOfDayManager = TODM_A_Child_Athena_C`. Its materials are `M_AridSky_Inst_*` (dome), `M_AridClouds_Inst_*` (clouds) and `M_Sun_Arid`. The deferred base pass uses the `TBasePassPSFNoLightMapPolicy` programs: sky 10238, clouds 22102, sun 29957. The exe `FUN_14063eba0` blends phases with `K2_InterpolateMaterialInstanceParams` over every parameter. | Hand ports of the simple-lighting variants: 6890 (`sky_dome.wgsl`), 17639 (`sky_clouds.wgsl`), 25910 (`sky_sun.wgsl`). Only 8 parameters are lerped; the sun is drawn as a billboard rather than at `dir * distance` with `dir.Rotation()`. | OPEN. An unwired, uncompiled port is in `crates/fn-matshader/src/sky.rs`; it is not in `lib.rs`, so delete it or finish it. |
| 8 | Backdrop terrain/skybox textures decoded after filtering | All terrain layers and skybox textures are SRGB=true DXT. | `backdrop.rs:404` uploads them as linear and `backdrop.wgsl` decodes after filtering; the AO texture is never decoded. 0.4-5 % darkening in mips. | OPEN |
| 9 | Mips of sRGB textures built in gamma space | UE builds mips in linear float (`TextureCompressorModule.cpp:2030-2086`). | `texture.rs upload_rgba8` | Edited, unverified (sub-agent): uses `build_mips_srgb` |
| 10 | Fallback ground (`T_Grasslands_D`) decoded after filtering | SRGB=true DXT5 | `renderer.rs:251,635` + `scene.wgsl` `to_linear` | OPEN (under 1 %) |
| 11 | Storm post-process / storm fog | Not identified (hint: `fn-postfx/src/evidence.rs`, PlayerPawn_Athena) | Not implemented | OPEN, unproven |
| 12 | PreIntegratedGF stored as Rgba16Float instead of G16R16 | `SystemTextures.cpp:298-302` | `sky_reflection.rs` | Negligible (0.05 %) |
| 13 | Scene colour R11G11B10 at Effects@0-2 | cooked ini `r.SceneColorFormat` 3 / 4 | always Rgba16Float | Matches at Epic only |
| 14 | Back buffer 8-bit vs `PF_A2B10G10R10` | `D3D11Viewport.cpp:485` | Bgra8Unorm | Gamma is correct; banding only |

## Verified to match (no change)

- **Tonemapper and colour**:
  - White balance: the full `WhiteBalance`, incl. the 1.4388/1.438 correction (WhiteTint 0 in every phase).
  - ColorCorrectAll: per-region saturation, contrast and gamma; gain 1 / offset 0.
  - The ACES red modifier, which is active in the 1.8 DXBC although the 4.16 source has it `#if 0`.
  - RRT 0.96 / ODT 0.93 saturation.
  - The Film* curve. BlackClip 0 and WhiteClip 0.04 come from the exe ctor.
  - `InverseGamma.y = 2.2/DisplayGamma` (DisplayGamma 2.2 in the cooked ini).
  - MappingPolynomial: `r.Color.*` is not in the cooked ini, so it is identity.
  - `r.TonemapperFilm=1` (cooked ini L81).
  - Sharpen: 0.5/6 at AA@2/3.
  - Grain quantization (PostProcessQuality@2/3).
  - Vignette 0.01.
  - SceneFringeIntensity 0, GrainIntensity 0, SceneColorTint white.
  - The per-phase values in `tonemap::PHASES`, which match `reports/postfx-evidence.json`. There is no blue correction or expand gamut in 4.16.
- **Exposure**:
  - The histogram method (`r.DefaultFeature.AutoExposure.Method` is not in the cooked ini, so it is 0; `r.EyeAdaptationQuality` 2 at @2/3), on `max(r,g,b)` of SceneColorHalfRes.
  - 64 buckets from -8 to 4, with bucket 0 dropped.
  - Low/High 80 (Morning/Evening 50) / 98.3, Min/Max in luminance units, `2^bias / L`.
  - Speeds 3/1, exe-proven.
  - The bias issue in row 3 is still OPEN.
- **Bloom**:
  - Threshold -1 means no setup pass.
  - Intensity 0.675, from the exe ctor.
  - Tints: 1,2,4,5 from the ctor; Bloom3 0.5 and Bloom6 0 from TODM_A. Bloom6Size 0.
  - TintScale 1/6, sizes, Gaussian kernel, `r.BloomQuality` 5.
  - No dirt mask texture.
- **BRDF / sky light**:
  - D_GGX, Diffuse_Lambert, ComputeF0, Specular default 0.5, F_Schlick's `saturate(50*g)`.
  - The PreIntegratedGF LUT, `ComputeReflectionCaptureMipFromRoughness` (1 / 1.2, max mip 7), specular occlusion.
  - SH diffuse.
  - The sky light colour via `ToFColor`, times intensity.
  - Lower hemisphere not black.
  - No reflection captures in any Athena map; AmbientCubemapIntensity 0.
- **Colour spaces**:
  - Cooked-material textures follow the cooked SRGB flag.
  - BC5 normals: `rg*2-1`, `sqrt(saturate(1-dot))`, unorm, all SRGB=false.
  - The sky-light cube is linear f16.
  - Non-sRGB swapchain with a single encode in post.wgsl.
  - Vertex colours are raw /255.

## Files changed by this audit
- `crates/fn-render/src/color_lut.rs` (new) and `crates/fn-render/shaders/color_lut.wgsl` (new).
- `crates/fn-render/shaders/post.wgsl`: `lut_device_color`, `color_lookup_table`, binding 10, grain order.
- `crates/fn-render/src/renderer.rs`, at 6 marked spots `// COLOR LUT`: the field, the bgl entry, the ctor, the two post bind groups, and the dispatch in the post node.
- `crates/fn-render/src/lib.rs`: `pub mod color_lut`.
- `crates/fn-render/src/tonemap.rs`: the CPU mirror now includes the `/1.05`, docs updated, new test `lut_round_trip_is_identity_below_white`. All 14 tonemap tests pass.
- Sub-agent edits (rows 4, 5, 9): `static_loader.rs`, `texture.rs`, `lit_common.wgsl`, `scene.wgsl`, `gun_resolve.wgsl`.
- Orphan file, not compiled: `crates/fn-matshader/src/sky.rs`.
