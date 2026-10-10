# Performance: spec of the GPU optimisations (2026-10-06)

Optimisations of the renderer for the user's play size (2544x1353, the maximised window on a 2560x1440 monitor) at the High settings. Each one either
does what UE4.16 / Fortnite 1.8 does (cited) or is pixel-identical. Measurements, image checks and the history of every step:
`verification/perf_assets/OPT_LOG.md`. Debug toggles named here are debug-only (A/B and verification), never set in play.

---

## 1. Full depth pre-pass

**1.8 behaviour.** The cooked `r.SelectiveBasePassOutputs=True` (CookedIniVersion.txt:80) and `r.StencilForLODDither=1` (:997) make
`ShouldForceFullDepthPass` true (UE4.16 DeferredShadingRenderer.cpp:146-156). `GetEarlyZPassMode` then sets `DDM_AllOpaque` with
`bEarlyZPassMovable` (:177-182). Every opaque and masked draw is in the depth pre-pass, static and dynamic (`RenderPrePassViewDynamic`,
DepthRendering.cpp:849-874). The base pass reads the depth only (:773 `bAllowReadonlyDepthBasePass`). Masked materials clip in the pre-pass with their
`FDepthOnlyPS` (`r.EarlyZPassOnlyMaterialMasking`). Opaque materials without WPO use the position-only depth (DepthRendering.cpp:652-669).

**Port.** Every opaque draw of the scene pass has a pre-pass twin: the same module and vertex entry, and no pixel stage (opaque) or the coverage
clip (masked). Its base-pass pipeline tests `Equal` with no depth write. This covers:

| Draw | Where |
|---|---|
| ground, static meshes | renderer.rs, statics.rs (before this pass) |
| masked scene-lit FX | fx.rs (before this pass) |
| BR characters | char_material.rs (before this pass) |
| procedural props, skinned prims without a CharacterShader | renderer.rs |
| gun / pickup programs | gun_material.rs `draw_depth` |
| opaque / masked particle batches | particles.rs `draw_opaque_depth`, particles.wgsl `fs_depth` |
| opaque scene-lit / scene-unlit FX | fn-fx program.rs `fs_main_eq` (+ `fs_depth` when the base program clips itself), fx.rs |

The image is identical (OPT_LOG item 1a). The screen-space distance-field shadow pass (section 2) depends on this depth.

## 2. Ray-traced distance-field shadows in screen space at half resolution

**1.8 behaviour.** UE4.16 DistanceFieldShadowing.cpp / .usf:
- `RayTraceShadows` dispatches `DistanceFieldShadowingCS` (scatter-tile culling) over `ScissorRect / GetDFShadowDownsampleFactor()`.
- `r.DFFullResolution` is 0, so the factor is `GAODownsampleFactor` = 2. In the 1.8 exe, @141803d90 computes `2 - (DAT_1467bf8f0 != 0)`.
- The output is the PF_G16R16F `RayTracedShadows` target, `DivideAndRoundDown(BufferSize, 2)`. The 1.8 desc there has format 0xf.
- Each thread reads the scene depth at full-res pixel (2x, 2y) and rebuilds the world position. It writes `(Result, SceneDepth)`, with `Result = 0` outside
  SplitNear - SplitNearFadeRegion .. SplitFar.
- `DistanceFieldShadowingUpsamplePS` then filters per pixel: bilinear times 1 / (|CornerDepth - SceneDepth| + 1e-4) over the 4 texels around the pixel,
  FarBlendFactor 1 (the last cascade), NearBlendFactor.
- The result goes into the light attenuation with `CW_RG + BO_Min` (ShadowRendering.cpp:420-424).

**Port.**
- `distance_field_shadow.rs` / `distance_field_shadow.wgsl` implement the pass. It is a full-screen pass, one fragment per texel. The scatter-tile CS uses
  no groupshared data, so the texels are the same.
- It renders into an Rg16Float target of floor(W/2) x floor(H/2). It is graph node `df shadow`, after the pre-pass and the backdrop depth merge (the one
  scene depth) and before the base pass.
- The lit shaders upsample per pixel (`lit_common.wgsl df_shadow_upsample`). They keep UE's corner placement, AM_Clamp per corner, and the depth weights,
  and min it into the CSM term as before. A 0/0 weight sum returns 0: UE's NaN becomes 0 in the UNORM light attenuation under BO_Min.
- Globals binding 16 = the target. The trace inputs (objects, light tiles, atlas, sampler) are only in the pass's bind group.
- Debug only: `FN_DEBUG_DF_FULLRES=1` = `r.DFFullResolution 1`; `FN_DEBUG_NO_DFSHADOW=1` turns the cascade off.

**Not ported.** The CS applies subsurface transmission for subsurface GBuffer pixels. The forward port has no GBuffer before its base pass, and the
per-fragment trace it replaces had none either.

## 3. Global distance field: partial (scrolled) and staggered clipmap updates

**1.8 behaviour.** UE4.16 GlobalDistanceField.cpp `ComputeUpdateRegionsAndUpdateViewState` (:865):
- Update index 0..4. Clipmap 0 updates every frame, 1 every second frame, 2 / 3 every fourth (phases 1 / 3).
- An updating clipmap snaps to the camera cell. It recomposites only:
  - the slabs the movement exposed (`AddUpdateRegionForAxis`);
  - the influence boxes of the objects added, removed or moved since its last update (`AddUpdateRegionForPrimitive`, fewer than 100, else a full update).
- Contained regions are dropped and overlapping ones trimmed. If the regions would cost more texels than one full update, it does a full update instead.
- Texels are addressed toroidally around `FullUpdateOrigin` (`ScrollOffset`, AM_Wrap). The composite writes `frac(GlobalUV) * Resolution`.
- 1.8 cvars `r.AOGlobalDistanceFieldPartialUpdates`, `StaggeredUpdates` and `CacheMostlyStaticSeparately` are all 1. Their decompiled registrations point at
  .data words holding 1. (exe-defaults.json mis-pairs these RVAs with other cvar names.)
- An atlas reallocation re-queues every primitive, which is a full update per clipmap on its next turn.

**Port.**
- `global_distance_field.rs` ports the region logic function for function, with its view state.
- `distance_field_ao.rs` / `.wgsl` composite each region:
  - the grid cull and composite run over the region;
  - a dynamic-offset region uniform carries the region data;
  - the write coordinate is toroidal;
  - the cone trace samples with the scroll (`clip_full`).
- `distance_field.rs` `modified_object_bounds` diffs the object list on every rebuild and feeds the modified bounds.
- The single clipmap set equals UE's min(mostly-static, movable) composite, since no port object is often-moving.

## 4. Character programs: constant buffers as values, not private arrays (pixel-identical)

**Problem.** The generated character programs (`char_material_gen.wgsl`, `char_depth_gen.wgsl`, `eyeshadow_gen.wgsl`, translated from the cooked DXBC by
fn-dxbc) hold every DXBC constant buffer in a `var<private>` array. The wrapper fills that array per pixel, about 650 vec4s across head, hair and body. The
driver keeps the arrays in local memory. One on-screen hero cost about 2.3 ms of the scene pass at 2544x1353.

**Port.** `char_material.rs fold_private_cbs` rewrites the generated text when the pipelines are built:
- a slot the wrapper writes becomes a private `vec4` (`<buffer>_<slot>`);
- a buffer the wrapper copied from a uniform is read from that uniform directly (`cmat.cb[k]` / `emat.cb1[k]`);
- every other slot becomes the zero it always held.

All indices in these programs are literals; this is asserted at build time. The wrappers (`char_scene.wgsl`, `char_overlay.wgsl`) write the folded names.
The values are the same, so the image is identical. The generated files are unchanged.

## 5. FX frame uploads through one staging buffer (pixel-identical)

**Problem.** `Queue::write_buffer` in wgpu-core creates (and later destroys) a staging buffer per call. The FX pass wrote about 300 small buffers per frame
(material / primitive / View constants, fog slots, dynamic geometry, instances).

**Port.** With `FxRenderer::set_staged_uploads(true)` (the renderer) `prepare` collects the frame's writes in one byte vector, in the order it used to issue
them, and places them in one buffer mapped at creation. The first frame-graph node, "fx uploads", records `copy_buffer_to_buffer` for each write before
any pass that reads the FX buffers. The bytes, destinations and order are unchanged, so the image is identical. `FN_DEBUG_FX_DIRECT_UPLOADS=1` (debug only)
keeps the old per-write path for A/B runs.

**Timing marks.** The renderer's CPU split now reports `prep finish` (`CommandEncoder::finish`, where wgpu-core encodes the whole frame into Vulkan
commands: about 3-4 ms, the large CPU spikes are here), `prep submit` (`Queue::submit`, about 0.7 ms) and `prep end frame` separately.

## 6. Cooked BC textures uploaded as blocks (texture memory, load time)

**Problem.** Every cooked texture of the FX path (foliage, trees, rocks, buildings, grass, storm, pickups) and of the gun pass was decoded on the CPU to RGBA8
(`fn_texture` decode of every stored mip) and uploaded uncompressed: CPU time on every load and first use, and 4-8x the GPU and heap memory of the cooked data.

**Port.** The device requests `Features::TEXTURE_COMPRESSION_BC` when the adapter has it (gpu.rs). `fn_fx::gpu::raw_bc_levels` keeps the cooked blocks of a
`PF_DXT1` / `PF_DXT3` / `PF_DXT5` / `PF_BC4` / `PF_BC5` / `PF_BC7` texture as they are (each level cut to its `ceil(w/4) * ceil(h/4)` blocks) when
- the device takes BC (`fx::bc_native_hint`, set when the device is created, because the loading-screen prewarm workers decode before the renderer exists),
- the first stored level is a multiple of 4 (wgpu rejects anything else for a BC texture, on every backend).
Everything else (B8G8R8A8, FloatRGBA, G8, odd sizes, no BC feature) takes the RGBA8 decode as before, unchanged. `FxRenderer::create_texture` /
`GunRenderer::create_texture_fmt` upload whole blocks per level (the copy covers the block-rounded size of the 2x2 / 1x1 levels).

- **Formats** (`FxTexFormat::wgpu`). UE4.16's D3D11 RHI maps PF_DXT1 / DXT3 / DXT5 / BC7 to TYPELESS formats whose view is `_UNORM_SRGB` for an sRGB texture,
  but PF_BC4 / PF_BC5 to `BC4_UNORM` / `BC5_UNORM` (D3D11Device.cpp:118-175), which `FindShaderResourceDXGIFormat` returns unchanged
  (D3D11RHIPrivate.h:912-949): BC4 / BC5 always sample linear. D3D samples BC5 as (r, g, 0, 1) and BC4 as (r, 0, 0, 1), and so do the wgpu formats. (The
  old CPU decode filled BC5 blue with a rebuilt Z, UEViewer style, and BC4 as grey.) Checked on every program loaded in the mpc views: all 36 BC5 sample
  sites read only `.xy` and rebuild Z with `sqrt(1 - dot(xy, xy))` (verification/mpc/bc_compare/bc5_channel_audit.txt).
- **Predecode.** The prewarm / first-use workers hold the same result the upload takes (blocks or RGBA8, with the format); the budgets count the held bytes.
- **Debug-only switches.** `FN_DEBUG_NO_BC=1` (everything decoded to RGBA8 as before, same exe), `FN_TEX_LOG=1` (every texture read: cooked format, size,
  GPU format), `FN_GPU_MEM=1` (gpu-allocator totals + FX / gun texture bytes per format every 300 frames), `FN_FX_DUMP_WGSL=<dir>` (assembled FX programs
  and their texture bindings).

**Result** (forest walk, 2544x1353, user settings; same exe, BC vs `FN_DEBUG_NO_BC=1`):

| | BC off (RGBA8) | BC on |
|---|---|---|
| FX textures | 509.7 MB (75 RGBA8) | 77.3 MB (44 BC1, 4 BC3, 20 BC5, 7 RGBA8) |
| GPU memory allocated (gpu-allocator) | 2398 MB | 1966 MB |
| process peak working set / private bytes | 4,145 / 6,602 MB | 3,483 / 5,665 MB |
| prewarm texture decode (building / statics) | 573-750 / 351-540 ms | 31-33 / 36-42 ms |
| `[load] done` total | 5.6 s (and one 11.8 s outlier) | 4.0-4.7 s |

Images: BC vs RGBA8 decode of the same mips differ by rounding (GPU vs CPU BC decoders) and, where a masked material's opacity crosses its clip value, by
single-pixel edge flips: mpc 1600x900 forest views 1.1 % of pixels changed by more than 2/255 (mean 0.30 / 0.38), the close-up under a canopy 9.5 % (mean
1.82, leaf edges); no surface changes as a whole. With `FN_DEBUG_NO_BC=1` the images are byte-identical to before.

**The hero (2026-10-06, approved by main).** The BR character's material textures (`Diffuse`, `Normals`, `SpecularMasks`, `M`) and the fixed textures of its
original materials (noise, CharacterResInEffect, eyeshadow) were the largest mip of each package decoded on the CPU, with the other mips generated by a
Triangle filter (`texture::upload_rgba8`). UE uploads the cooked mip chain. Now `fn_pose::br_character::CookedChain` carries the stored mips, fn-app
`br_character::cooked_of` turns them into a `fn_render::mesh::CookedTexture` (BC blocks where the device takes them, else every stored mip decoded to RGBA8),
and `MaterialImages::cooked` / `FixedTexture::cooked` upload that chain (same UNORM / sRGB choice as before: the model path stays UNORM, the fixed textures keep
their flag, BC4 / BC5 always UNORM). The top-mip decode in fn-pose still runs (its image decides the material kind and is the fallback); dropping it is a
follow-up. Gun / hero script at 2544x1353 (`props none`): GPU allocated 1498 -> 1268 MB, "Uploading character models" 3.2 s -> 0.9 s (one run each).
Image: the hero's surfaces differ only by rounding / the cooked mips (0.8-0.9 % of the frame's pixels changed by more than 2/255, mean 0.11-0.16, all on the
body; `verification/mpc/bc_compare/crop_char_back.png`).
