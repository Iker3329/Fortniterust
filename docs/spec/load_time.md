# Load time (2026-10-06)

The user reported that the game "takes a LONG time to load" and "sits on Starting forever". Measured with the user's settings (normal launch: `--skip-loading`, and the scripted `verification/load_time/load_real.script`, real map props):

| step | before | after |
|---|---|---|
| launch -> Battle Royale picker | 1.9 s | 1.9 s (unchanged, it was fine) |
| match load (loading bar) | 32.9 s | 3.5 s |
| first in-game frame (screen frozen on the last loading frame, "Starting") | 3.8 s (much longer with the old pak scan) | 0.76 s |
| load -> playable | 40+ s | about 5 s |

Images are pixel-identical before and after: `verification/quickbar_ammo/run1` vs `run2`, max difference 0.

## Causes and fixes
1. **`PakSet::glob` scanned all ~238k pak entries per call and allocated two `Vec<char>` per entry**, ~0.3 s per call uncontended and ~1.1 s under load. `ClipStore::load` ran it once per animation (197 clips). The browser agent fixed it in fn-pak: an allocation-free ASCII matcher plus a literal-suffix pre-check, the same results with a parity test. Every glob caller benefits, including `Material::load` chains, pickups and inventory extras.
2. **Character models uploaded last, on the critical path.** Body, head and pickaxe took ~2.8 s of texture uploads, mostly CPU mip chains in `texture::upload_rgba8` (`image::imageops::resize` Triangle).
   - They now build with `Renderer::model_builder()` on three scoped threads right after the renderer exists, overlapping the prop uploads (`loader.rs` build_world).
   - `Scene::with_models` installs the prebuilt models.
3. **The first real-shader frame parsed every tree/rock/bush material one after another** (`statics_render.rs`): 21 materials at ~100-220 ms each in `Material::load`, almost all of it `Cooked::parse`.
   - `Cooked::parse` inflates and decodes every one of the 477-672 shader permutations of a shader map, even though a material uses only a few.
   - `statics_render::prewarm` (called by `start_loading`) now parses them on a pool during the loading screen, ~0.7 s for 21. The first frame waits for it and takes the parsed materials. Translation and pipeline creation (~0.75 s) still run on the first frame.

4. **Texture predecode (native).** The statics prewarm also decodes its materials' 55 textures' cooked mip chains (`fn_fx::gpu::predecode_texture`), so the first frame only uploads them. First frame: 0.76 -> 0.41 s.
   - Bounded by `PREDECODE_BUDGET`: 512 MB natively, 0 in the browser build. In the browser the held chains raised the peak heap ~0.5 GB for no gain, because WebGPU pipeline creation is its bottleneck.
5. **Shared shader maps.** `Material.cooked` is `Arc<Cooked>` (Material is `Clone`), and `shader_map()` keeps one parse per shader-map package while any material holds it (Weak). Instances of one parent share it.
6. **Building prewarm (native).** `building_render::prewarm` parses the 12 standard pieces' mesh / animating / damaged materials plus the blueprint and default-surface materials, and decodes their textures. That is 14 materials in ~0.4 s and 27 textures in ~0.4 s, off the UI thread.
   - `verification/load_time/build_hitch.script`: the first build-mode equip used to freeze 391 ms. Now no frame at equip or at the first wall / floor / stairs placement exceeds ~20 ms of render.

7. **Statics build under the loading screen.** `statics_render::prebuild_step` runs from `App::finishing_frame`: ~25 ms of kind and material builds per frame while the loading page keeps rendering. The flow holds Finishing until it is done (30 s cap). Scripted runs force Playing and keep the old first-frame build.
   - Normal launch: the first in-game frame is 35 ms, down from 0.41-3.8 s.
   - The same code runs in the browser build.

## Probes
`FN_FX_TIMING=1` prints, per material:
- `Material::load` split into chain / cooked / textures+functions / collections;
- `Cooked::parse` split into `scan_blobs` / `Shader::parse`;
- translation vs pipeline creation in `build_material_layout`.

9. **The building prewarm runs in the browser build too, and covers the construction / destruction particle materials.** That is every `lod.required.material` of the `particles-building` catalog, held to keep their shared shader maps alive for `pickups_dxbc`'s first-spawn `Material::load`.
   - Native: 23 materials parsed in ~0.5 s during loading. The scripted build session (`build_hitch.script`) shows max 50 ms (at start), p99 20 ms, no spike at placement.
   - Browser, before this: 0.6-0.7 s first-placement hitches (verification/browser/launch4).
8. **Root-material texture walk cached.** `collect_textures` of a root material (its ~46 material functions) now runs once per root (`ROOT_TEXTURES`). Together with the shared shader maps, the shotgun, sniper and pickaxe instances of WeaponMat_Base cost `cooked` 0.1-0.5 ms (was 144-217 ms).

## Open
- **The post-load setup (UI thread, screen at 100 %) is 632 ms, 590 ms of it `install_original_guns`.** `gun_orig::install` (and `add`, per new gun) also mounts its own `PakSet` (~0.1-0.2 s, plus a second copy of the 238k-entry index).
  - Fix: pass the loader's set, and parse the gun materials ahead (keep them alive until the install, so the caches hit).
  - Measured by the `[load] post-load setup` line.
- **`Cooked::parse` decodes all 477-672 permutations of a shader map.** A lazy or parallel parse that decodes only the requested permutations would cut every first use. It would also cut memory: the parsed maps are most of the browser heap.
- **The first frame still does the statics translation and pipelines** (~0.4 s native, ~6.5 s on the browser main thread). These could be spread over loading-screen frames.
- **CPU mip generation (`upload_rgba8`, `build_mips_srgb` powf)** could use the cooked mips, or a faster exact filter.

## First use (2026-10-06): freezes at a new weapon / a new building piece
The user: "still freezes like when i spawn a new weapon, place a new build ... all preloading ... should be running in background". Measured with
`verification/first_use/first_use.script` (5 weapons spawned with `FNRT_PICKUP_SPAWN`, picked up with `FNRT_PICKUP_USE`, slots 2-6 fired, wall / floor /
stairs / roof in wood, stone, metal); `[slow]` lines now time `pickups update`, `weapon poll`, `pickups frame`, `building frame`, `particles submit`, `gun submit`.

| native, 2544x1353 | before (run2) | after (run7) |
|---|---|---|
| worst frame of the run | 617 ms | 71 ms |
| weapon install (UI thread) | 136-154 ms | 1-9 ms |
| a new weapon pickup appearing | 180-540 ms | no `[slow]` line |
| first piece of each type / material | 28-57 ms each | no `[slow]` line |
| pickup effect first use | 109-163 ms, then 11-30 ms per new weapon | 44 ms once (first hologram) |
| extra loading-screen time | - | ~1.1 s (building 0.9 s, pickup effects 0.16 s) |

Browser (before, `verification/browser/first_use1`): pickup first draw 165-610 ms, install 210-263 ms, each new piece 150-510 ms of material work plus
a 0.6-0.8 s stall while the GPU process compiles the new pipeline (DXC, synchronous on its main thread).

Causes and fixes:
1. **Every weapon was built twice on the UI thread**, once for the dropped pickup (`pickup_orig`, its own `GunScene` and its own `PakSet`) and once when
   equipped (`gun_orig::OrigGuns::add`, which mounted a new `PakSet` per weapon). Now one shared `GunScene` (textures, materials, pipelines) and the shared
   pak mount (fn-pak, browser agent).
2. **The CPU half of a gun build ran on the UI thread** (mesh parse, material instances, texture mips). `fn_matshader::gun::prepare_skeletal` /
   `prepare_weapon` run it on the pickups mesh worker and the weapon runtime's worker; `textures::predecode_2d` holds the mips (384 MB native, 96 MB wasm).
3. **`ResolvedMaterial::load` re-parsed the root material's shader maps per instance** (WeaponMat_Base for every gun): `material::root_maps` shares them.
4. **The renderer model of every weapon pickup was uploaded (with CPU mips) and never drawn** when the gun pass draws it: now created only when used.
5. **The pickup hologram created its bind groups every frame** (FX bind groups are never freed): cached per (entry, slot).
6. **Building pieces, pickup effects and the pickaxe were built at first use**: `building_render::prebuild_step`, `pickups_fx::prebuild_step` and
   `pickups_hook::warm_pickaxe` draw them into scratch frames from `App::finishing_frame` (scripted runs: `FN_SCRIPT_WARM=1`). Off in the browser build
   until its pipelines compile asynchronously (`FN_WARM_WEB=1` to try), because each pipeline costs 0.5-1.1 s of synchronous GPU-process compile there.
7. **Identical DXBC->WGSL translations were repeated** (hologram per weapon mesh, blueprint MID per piece class): `fn_fx::gpu` `ASSEMBLED` keys them by
   shader map, vertex factory, program streams, assembly kind and prelude.

Open: the item holograms / static item materials (`FN_WARM_ITEMS=1`, +6 s natively while every hologram is its own identical pipeline: needs the
content-keyed FX pipeline cache), DF-atlas rebuilds (`prep distance fields` 10-29 ms when pieces are placed / while moving) and `CommandEncoder::finish`
spikes (opt agent, docs/spec/performance.md section 5).

### First use, round 2 (2026-10-06, after the shared / async FX pipelines)
* The browser agent's content-keyed FX pipeline cache (both builds) and asynchronous pipelines (browser) made the warm-ups cheap: the item holograms
  (29 identical weapon-hologram pipelines) went from +6.0 s to +2.0 s of loading natively (first_use run6 -> run8) and are now ON natively by default
  (`FN_WARM_ITEMS=0` turns them off; in the browser opt-in with `FN_WARM_ITEMS=1` until its per-item mesh / texture reads leave the main thread).
* Edit mode joined the building warm-up (`Warm::Edit`: the MGreen tiles with both tile textures, the stair indicator with its arrow / grid textures,
  `PreviewMaterial` on every standard piece): the first edit froze 19-29 ms natively (load_time/normal_warm1.log), none since (normal_warm2.log).
* The harvest / impact / death particles and the 45 weapons' muzzle flashes build their materials (PNG decode) and meshes under the loading screen
  (`particles::warm_catalog`, `particles::prebuild`, `muzzle_hook::prebuild`): their first spawn cost 21-25 ms (normal_warm2.log).
* Native loading-screen cost of all warm-ups: building ~0.9 s, pickups (effects + item holograms / statics) ~1.35 s.
* Browser (browser agent, verification/browser/first_use2, warm-ups on): frame p99 251 -> 44.6 ms, max 786 -> 292 ms (frame 0), no building freeze,
  weapon install 1-6 ms; its warm-up costs ~7.5 s of loading screen (main-thread pak reads in `load_def` / `load_mesh`).
