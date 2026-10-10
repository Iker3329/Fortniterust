# Reflection gaps: spec (verifier, 2026-10-05)

Follows `pickup_visual_gaps.md` section 7. Sources: UE 4.16 at `C:\UE4.16` and the 1.8 cooked ini `C:\FortniteRef\scratch\CookedIniVersion.txt` (written as "ini:N" below). Evidence for the shipped sky term is in `verification/gunpass/verify/`.

Context: Athena has no reflection captures and no planar reflections. A raw scan of all 5 paks found 0 compressed entries and 68 encrypted entries (ini only). In every package name table, the only `*ReflectionCapture*` class reference is in `FrontEndStore.umap`, and no package references `PlanarReflection`. So the full 4.16 specular-indirect chain for BR is:

`ReflectionApplyPS` (ReflectionEnvironmentShaders.usf:537) = SSR + sky light, with no reflection environment and no ALLOW_STATIC_LIGHTING (ini:973 `r.AllowStaticLighting=0`).

The port implements the sky part (`lit_common.wgsl` `sky_specular`). This spec covers what is still missing.

---

## 1. Screen-space reflections (SSR)

**Severity:** medium. Wet or flat smooth surfaces are rare in BR, but metal guns, windows and the AR receiver lose their screen-space reflections at High and Epic.

### Enable gate (`ShouldRenderScreenSpaceReflections`, PostProcess/ScreenSpaceReflections.cpp:58)
SSR runs when all of these hold:
* show flag on;
* a view state exists;
* `r.SSR.Quality > 0`;
* `FinalPostProcessSettings.ScreenSpaceReflectionIntensity >= 1`;
* not forward shading.

### Cooked values (`Scalability:EffectsQuality@N:r.SSR.Quality`)

| EffectsQuality | r.SSR.Quality | ini line |
|---|---|---|
| 0 Low | 0 (off) | ini:5286 |
| 1 Medium | 0 (off) | ini:5300 |
| 2 High | 2 | ini:5314 |
| 3 Epic | 3 | ini:5328 |
| Cine | 4 | ini:5342 |

No other `r.SSR.*` cvar is set in the cook, so the code defaults apply:
* `r.SSR.Temporal 0`
* `r.SSR.Stencil 0`
* `r.SSR.Cone 0`
* `r.SSR.MaxRoughness -1`, which means "use the PP value" (exe-defaults.json, SceneView.cpp:1812)

### Post-process values
The 1.8 exe `FPostProcessSettings` ctor gives (`crates/fn-exe-defaults/reports/exe-defaults.json`):
* `ScreenSpaceReflectionIntensity 100`
* `ScreenSpaceReflectionQuality 50`
* `ScreenSpaceReflectionMaxRoughness 0.6`

`reports/postfx-evidence.json` records no SSR override on the TODM_A / TODM_A_Child_Athena phase components. **Before implementing, confirm** that none of the four `DayPhasePostProcessComponent` settings sets `bOverride_ScreenSpaceReflection*`. If none does, use the ctor values.

### Effective shader quality
`ComputeSSRQuality(PP Quality)` (ScreenSpaceReflections.cpp:315) maps 50 to 2 (`>= 40 -> 2`), then `min(2, r.SSR.Quality)`. So **both High and Epic run SSR_QUALITY 2**: 16 steps, 1 ray, no jitter set (ScreenSpaceReflections.usf:168). Epic's cvar value of 3 is capped by the PP quality of 50.

### Parameters (`ComputeSSRParams`, ScreenSpaceReflections.cpp:117)
* `SSRParams.x` = clamp(Intensity * 0.01, 0, 1) = 1
* `SSRParams.y` = RoughnessMaskScale = (-2 / MaxRoughness) * (Quality < 3 ? 2 : 1) = -6.6667
* `SSRParams.w` = FrameRandom = TAA sample index * 1551. TAA is on, so there is no SSR temporal pass (`IsSSRTemporalPassRequired`, :92).

### Trace (`ScreenSpaceReflectionsPS`, ScreenSpaceReflections.usf:95; quality-2 branch :248-279)
1. `RoughnessFade = min(Roughness * SSRParams.y + 2, 1)`. Skip the pixel when it is <= 0, which is every pixel with roughness >= 0.3.
2. `StepOffset = InterleavedGradientNoise(SvPosition.xy, View.StateFrameIndexMod8) - 0.5`.
3. `R = reflect(-V, N)`. Do not use the off-specular peak here.
4. `RayCast(HZB, ..., NumSteps = 16, ...)` (ScreenSpaceRayCast.usf:40):
   * the ray end point is `Origin + R * SceneDepth` in view space;
   * the step is clipped to the viewport;
   * `USE_HZB 1` with `HZB_LEVEL_OFFSET 1` (`SCALAR_BRANCHLESS 0` path, :103 `#else`);
   * the HZB level per step is `Roughness * (i * 4 / NumSteps) + 1`;
   * `CompareTolerance = max(|RayStepUVz.z|, (RayStartScreen.z - RayDepthScreen.z) * 4) * Step`.
5. On a hit: `ReprojectHit` (velocity texture, else `ClipToPrevClip`, :462), then `SampleScreenColor` (:479). The source is the previous frame's TAA history when available (`PrevFrame = 1`, ScreenSpaceReflections.cpp:512), else the current scene colour. Alpha is 1, with the edge vignette `saturate(1 - dot(V, V))`, `V = saturate(|ScreenPos| * 5 - 4)`.
6. `OutColor *= RoughnessFade * SSRParams.x`.

### Port requirements
* An HZB of the scene depth: the furthest-depth mip chain. 4.16 builds it at a fixed 1024x512-based size (`HZBUvFactorAndInvFactor`, ScreenSpaceReflections.cpp:291).
* The TAA history (the port has `taa.rs`) and the velocity buffer (`velocity.wgsl`).
* A per-pixel GBuffer: N and Roughness. This forward renderer would need to write N and roughness to an extra target (for example Rgba16Float: oct-normal xy, roughness, `ShadingModelID != 0`).

### Composite (ReflectionApplyPS :585-665): the exact order
```
SpecularLighting = (0,0,0,1)
SpecularLighting.rgb = SSR.rgb + SpecularLighting.rgb * (1 - SSR.a)
SpecularLighting.a  *= 1 - SSR.a
SpecularLighting.rgb *= SpecularColor        // EnvBRDF * SpecularOcclusion (AO)
SpecularLighting.rgb += SpecularLighting.a * (Sky * SpecularColor * IndirectSpecularOcclusion + ExtraIndirectSpecular * SpecularColor)
```
So:
* SSR is scaled by EnvBRDF and the AO specular occlusion, but **not** by the DFAO `IndirectSpecularOcclusion`.
* The sky term is weighted by `(1 - SSR.a)`.

The port's `sky_specular` already returns the sky term. The SSR pass therefore has to run after the opaque pass and do the following:

`scene += SSR.rgb * SpecularColor - SSR.a * SkySpecular`

`SkySpecular` is the value `sky_specular` already added. That means the lit passes must also write `SkySpecular` and `SpecularColor` (or EnvBRDF * SO) to targets. Section 2 needs the same targets, so do both in one deferred "reflection apply" pass.

---

## 2. SSAO factor in the specular occlusion

**Severity:** low to medium. Contact darkening of reflections is missing in crevices, under the character and at gun/ground contact.

* **UE:** `AO = GBuffer.GBufferAO * ScreenSpaceData.AmbientOcclusion` and `SpecularOcclusion = saturate(Square(NoV + AO) - 1 + AO)` (ReflectionEnvironmentShaders.usf:561-563). `ScreenSpaceData.AmbientOcclusion` is the SSAO output with `AmbientOcclusionIntensity` / `Power` already applied (the same texture the port's `ssao_apply.wgsl` evaluates through `post.wgsl` `ambient_occlusion`).
* **Port:** `sky_specular(..., ao)` gets `GBufferAO` only (`ao` = 1 for the gun, static and architect paths; `g.ao` for char_scene). `ssao_pass.rs` multiplies only the ambient share (HDR alpha), and the sky specular is not part of that share.

The occlusion is non-linear in AO, so it cannot be fixed up after the fact by scaling. Spec:
1. The lit passes (`gun_resolve`, `char_scene`, `scene.wgsl` `shade_character` / `shade_architect`, `static_scene`, and the terrain from section 4) write one extra Rgba16Float MRT, the "reflection inputs" target:
   * `rgb` = `GetSkyLightReflection(R, Rough) * EnvBRDF(Spec, Rough, NoV) * IndirectSpecularOcclusion + ExtraIndirectSpecular * EnvBRDF`. This is the current `sky_specular` **without** the AO term.
   * `a` = NoV.

   They write GBufferAO into a second small target (R8). Alternatively, pack it as `a = NoV + 2 * floor(GBufferAO * 255)`.

   They stop adding the sky specular to the HDR colour.
2. `ssao_apply.wgsl`, after its ambient multiply, computes `AO = GBufferAO * ssao` and `SO = saturate((NoV + AO)^2 - 1 + AO)`, then adds `rgb * SO` to the scene. This keeps 4.16's order: AO and reflections are applied in the deferred passes, before fog.

   **Fog caveat:** the forward pass applies height fog inside `output_hdr`, but UE fogs the reflection too (the fog pass runs after `ReflectionApplyPS`). Store the fog transmittance (`fog.a`) in the reflection target, or apply `rgb * SO * fog.a` there.
3. When SSR (section 1) lands, the same pass does the SSR composite.

Acceptance: with SSAO off (`r.AmbientOcclusionLevels 0`, or PostProcessQuality@0 at ini:5177), the image matches the current output bit for bit.

---

## 3. Static-mesh Specular default

**Severity:** low to medium. It affects every static (props, rocks, trees, POI meshes) whose material has no resolvable Specular.

* **UE:** an unconnected Specular input compiles to `0.5` (`FMaterialAttributeDefinitionMap`, Engine/Private/Materials/MaterialShared.cpp:2894, `MP_Specular` default `FVector4(.5,0,0,0)`). An unconnected Roughness also compiles to **0.5** (:2895).
* **Port:** `fn-render/src/static_mesh.rs:84` `StaticMaterialData::default` uses `specular: 0.25` and `roughness: 0.85`. `static_loader.rs:291-293` only overrides them from MI scalar parameters (`Specular` / `SpecularValue` / `SpecularAmount`, `Roughness`...).

Spec:
1. Resolve the Specular and Roughness the material actually uses:
   * a scalar parameter value on the MI or any parent MI;
   * else the default value of the parameter expression in the base `UMaterial`;
   * else, when the material's Specular / Roughness input is unconnected, 0.5 / 0.5.

   When the input is connected to texture maths that cannot be evaluated, keep a documented ASSUMED value and log it.
2. Change the defaults to `specular: 0.5` and `roughness: 0.5` (MaterialShared.cpp:2894-2895).
3. Fix the sun specular in `static_scene.wgsl:63-64`. Today it is `ggx_spec(n, v, sun, rough, 0.04) * Specular`, which scales the Fresnel grazing term by Specular too. UE `F_Schlick` (BRDF.usf) is `saturate(50 * SpecularColor.g) * Fc + (1 - Fc) * SpecularColor` with `SpecularColor = 0.08 * Specular` (dielectric). Use the colour-F0 form the other paths use: `s0 + (s1 - s0) * 0.08 * Specular`, multiplied by `saturate(50 * 0.08 * Specular)` for the Fc term. Otherwise switching the default to 0.5 changes the sun highlight in the wrong way: F0 would become 0.02 and the grazing term 0.5.
4. The sky term at `static_scene.wgsl:67` already uses `0.08 * Specular` and needs no change beyond the new value.

Expected effect: about 2x the sky specular on default statics (F0 0.02 -> 0.04). Re-run `verification/dfshadow/ao_rock.script` and `far.script` at 13h, before and after.

---

## 4. Sky specular missing on terrain, backdrop and fallback props (found by the verifier)

**Severity:** medium. The landscape is the largest lit surface on screen, and it is the only major opaque path that now lacks the term the guns, characters, buildings and statics get. This shifts the relative brightness between the terrain and everything on it.

* `scene.wgsl` `shade_ground` (:108): only the sun GGX (`ggx_spec(..., 0.9, 0.04)`) and the SH diffuse. There is no `sky_specular` call.
* `backdrop.wgsl:110`: only the sun specular.
* `scene.wgsl` `fs_prop` (:254, vertex-colour fallback props and kind-3 Prop): diffuse only.

In UE every DefaultLit / TwoSidedFoliage GBuffer pixel goes through `ReflectionApplyPS`. Landscape is DefaultLit, with its material's Specular and Roughness (default 0.5 / 0.5 when unconnected, section 3).

For the grass (roughness about 0.9, F0 0.04), EnvBRDF is about 0.06-0.1 at NoV 0.3-0.7. Against a grass albedo around 0.1, the sky specular is the same order as the sky diffuse, so leaving it out is not negligible.

Spec: add `sky_specular(n, v, world_pos, vec3(0.08 * Specular), Roughness, 1.0)` in `shade_ground`, the backdrop and `fs_prop`, with the landscape material's resolved Specular and Roughness. Today `shade_ground` hard-codes roughness 0.9 and F0 0.04; resolve both from `M_Athena_Terrain_Landscape` the same way as section 3.

---

## 5. Minor, for the record

* `ScreenSpaceData.DirectionalOcclusion.g` (ReflectionEnvironmentShaders.usf:654) multiplies the sky specular. It is 1 unless capsule indirect shadows run. Fortnite characters use capsule shadows only if `bCastCapsuleIndirectShadow` is set on the mesh component; check the BR pawn CDO before treating it as 1.
* The `PreIntegratedGF` LUT is uploaded as Rgba16Float rather than G16R16 UNORM. The precision difference is at most 5e-4 near 1. Harmless.
* Cube-face edge filtering is clamped per face (D3D11 filters cube edges seamlessly). This is documented in `sky_reflection.rs`. Visible only at mip 0-1 on very smooth surfaces.
