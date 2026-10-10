//! The Athena sky light's DIFFUSE term: UE4.16 `SLS_SpecifiedCubemap` processing of `T_hdrsky_test03_low` into the 3-band SH irradiance
//! (`FSHVectorRGB3 IrradianceEnvironmentMap`), packed like `View.SkyIrradianceEnvironmentMap` and evaluated like `GetSkySHDiffuse`.
//!
//! # Evidence (1.8 exe + cooked assets)
//! * `AFortTimeOfDayManager` ctor (exe 0x14062c240) creates `SkyLightComponent` and stores, over the class ctor (exe 0x1420efd50):
//!   `+0x13e = 2` (Mobility Movable), `+0x2b8 = 1` (`SourceType` = SLS_SpecifiedCubemap; `SetCubemapBlend` exe 0x14210f490 returns unless
//!   `+0x2b8 == 1`, UE4.16 `SourceType == SLS_SpecifiedCubemap`), `+0x2d5 = 0` (`bool bLowerHemisphereIsBlack`, SDK offset 0x2D5; the class ctor stores 1).
//! * `TODM_A` serializes `SkyLightComponent.Cubemap = /Game/Packages/Fortress_Sky/TexturesHDR/T_hdrsky_test03_low` (TextureCube, PF_FloatRGBA,
//!   TC_HDR). Neither `TODM_A` nor `TODM_A_Child_Athena` serializes `SourceType`, `bLowerHemisphereIsBlack`, `LowerHemisphereColor` or
//!   `SourceCubemapAngle`; the TODM is spawned from `FortWorldSettings.WorldTimeOfDayManager` (a class, not a placed actor) and no
//!   `Athena_Terrain` level / sublevel contains another sky light (fn-inspect).
//! * Nothing changes the cubemap at run time: no native caller of `SetCubemap` / `SetCubemapBlend` (only their exec thunks), and the
//!   TODM Blueprint bytecode (`fn-inspect disasm`) never references the sky light. So there is no cubemap blend: `BlendDestinationCubemap`
//!   stays null, and the SH below is a constant. The time of day only drives `SetLightColor` / `SetIntensity` (exe `FUN_14064f050`),
//!   i.e. `View.SkyLightColor` = `lighting::sky_light_tint_of`.
//! * `CubemapResolution` = 128 (class ctor `+0x2cc = 0x80`), `SourceCubemapAngle` = 0 (written by neither ctor, not serialized).
//!
//! # UE4.16 pipeline reproduced by [`project_specified_cubemap`]
//! `FScene::UpdateSkyCaptureContents` (ReflectionEnvironmentCapture.cpp:1518): `CopyCubemapToScratchCubemap` renders a `CubemapResolution`
//! scratch cube with `CopyCubemapToCubeFaceColorPS` (bilinear `TextureCubeSampleLevel(.., 0)` of the source, rotated by SourceCubemapAngle, the
//! lower-hemisphere fill only when `bLowerHemisphereIsBlack`), `FilterReflectionEnvironment` -> `CreateCubeMips` (`DownsamplePS`, 9 bilinear taps
//! per texel) -> `ComputeDiffuseIrradiance` from the `GDiffuseIrradianceCubemapSize` (32) mip: `DiffuseIrradianceCopyPS` (texel weight
//! `4 / (sqrt(1+u^2+v^2) (1+u^2+v^2))` x `SHBasisFunction3`), summed to 1x1 and `AccumulateCubeFacesPS`: `4 pi * sum(L Y w) / sum(w)`.
//! ADAPTATION: done once on the CPU (the result is the same constant every capture). SKY LIGHT (2026-10-06): the bilinear taps now filter across cube
//! face edges and corners as the D3D11 hardware does ([`Cube::sample`]; they used to clamp inside the face) and every scratch-cube stage is rounded to
//! half float like the PF_FloatRGBA render targets ([`round_fp16`]); the SH accumulation passes (`DiffuseIrradianceCopyPS` / accumulate) still sum in
//! f32. The sky light's SPECULAR contribution (the filtered cube of the reflection environment) is `sky_reflection.rs`.
use glam::Vec3;

/// `USkyLightComponent::CubemapResolution`: class ctor (exe 0x1420efd50) `+0x2cc = 0x80`, not serialized.
pub const CUBEMAP_RESOLUTION: usize = 128;
/// `GDiffuseIrradianceCubemapSize` (ReflectionEnvironmentCapture.cpp:58).
pub const DIFFUSE_IRRADIANCE_CUBEMAP_SIZE: usize = 32;
/// `SourceCubemapAngle` (deg): written by neither ctor, not serialized -> 0.
pub const SOURCE_CUBEMAP_ANGLE_DEG: f32 = 0.;
/// `bLowerHemisphereIsBlack`: TODM ctor (exe 0x14062c240) `SkyLightComponent+0x2d5 = 0`.
pub const LOWER_HEMISPHERE_IS_BLACK: bool = false;
/// The specified cubemap (TODM_A `SkyLightComponent.Cubemap`).
pub const CUBEMAP_PACKAGE: &str = "/Game/Packages/Fortress_Sky/TexturesHDR/T_hdrsky_test03_low";

// GENERATED: FSHVectorRGB3 of T_hdrsky_test03_low ([R, G, B] x SH V[0..9], UE world axes). Regenerate with
// FN_REGEN_SKY_SH=1 FN_TEST_ROOT=<FortniteGame> cargo test -p fn-render --lib sky_light (writes src/sky_light_sh.in).
/// `USkyLightComponent::IrradianceEnvironmentMap` of the Athena sky light.
pub const IRRADIANCE_SH: [[f32; 9]; 3] = include!("sky_light_sh.in");

/// `GetCubemapVector` (ReflectionEnvironmentShaders.usf:83): face 0..5 = +X -X +Y -Y +Z -Z, `s` = UV * 2 - 1 (UV.y = 0 at the top row).
pub fn cubemap_vector(face: usize, s: [f32; 2]) -> Vec3 {
    match face {
        0 => Vec3::new(1., -s[1], -s[0]),
        1 => Vec3::new(-1., -s[1], s[0]),
        2 => Vec3::new(s[0], 1., s[1]),
        3 => Vec3::new(s[0], -1., -s[1]),
        4 => Vec3::new(s[0], -s[1], 1.),
        _ => Vec3::new(-s[0], -s[1], -1.),
    }
}
/// Inverse of [`cubemap_vector`]: (face, UV) of a direction (major axis like the hardware cube lookup).
pub fn cube_face_uv(d: Vec3) -> (usize, [f32; 2]) {
    let a = d.abs();
    let (face, s) = if a.x >= a.y && a.x >= a.z {
        if d.x > 0. { (0, [-d.z / a.x, -d.y / a.x]) } else { (1, [d.z / a.x, -d.y / a.x]) }
    } else if a.y >= a.z {
        if d.y > 0. { (2, [d.x / a.y, d.z / a.y]) } else { (3, [d.x / a.y, -d.z / a.y]) }
    } else if d.z > 0. { (4, [d.x / a.z, -d.y / a.z]) } else { (5, [-d.x / a.z, -d.y / a.z]) };
    (face, [s[0] * 0.5 + 0.5, s[1] * 0.5 + 0.5])
}

/// A linear-RGB cube map level: `faces[f][y * size + x]`, row 0 = top (UE face orientation).
#[derive(Clone, Debug)]
pub struct Cube { pub size: usize, pub faces: [Vec<[f32; 3]>; 6] }
impl Cube {
    /// SKY LIGHT (2026-10-06): one bilinear tap `(x, y)` of face `f`, at most one texel outside the face. D3D10+ cube sampling is seamless whatever the address mode
    /// (the capture runs on the D3D11 RHI; D3D11.3 functional spec 7.18.8 TextureCube edge / corner handling): a tap past an edge reads the texel of the ADJACENT
    /// face that holds the same direction (the texel centre on the extended face plane, re-projected), and a tap past a corner (both coordinates outside) does
    /// not exist: `None`, the caller substitutes the average of the three other taps.
    fn tap(&self, f: usize, x: i32, y: i32) -> Option<[f32; 3]> {
        let n = self.size as i32;
        let (inx, iny) = ((0..n).contains(&x), (0..n).contains(&y));
        if inx && iny { return Some(self.faces[f][(y * n + x) as usize]); }
        if !inx && !iny { return None; }
        let s = [(x as f32 + 0.5) / n as f32 * 2. - 1., (y as f32 + 0.5) / n as f32 * 2. - 1.];
        let (g, uv) = cube_face_uv(cubemap_vector(f, s));
        let (tx, ty) = (((uv[0] * n as f32) as i32).clamp(0, n - 1), ((uv[1] * n as f32) as i32).clamp(0, n - 1));
        Some(self.faces[g][(ty * n + tx) as usize])
    }
    /// Bilinear sample (`SF_Bilinear`) with the hardware's seamless cube filtering across face edges and corners (see [`Self::tap`]; the address mode does
    /// not apply to cube maps). It used to clamp inside the face (ADAPTATION, removed 2026-10-06).
    pub fn sample(&self, d: Vec3) -> [f32; 3] {
        let (f, uv) = cube_face_uv(d);
        let n = self.size as f32;
        let (x, y) = (uv[0] * n - 0.5, uv[1] * n - 0.5);
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let taps = [self.tap(f, x0, y0), self.tap(f, x0 + 1, y0), self.tap(f, x0, y0 + 1), self.tap(f, x0 + 1, y0 + 1)];
        let known: Vec<[f32; 3]> = taps.iter().flatten().copied().collect();
        let corner: [f32; 3] = std::array::from_fn(|c| known.iter().map(|t| t[c]).sum::<f32>() / known.len().max(1) as f32);
        let [t00, t10, t01, t11] = taps.map(|t| t.unwrap_or(corner));
        std::array::from_fn(|c| {
            let top = t00[c] * (1. - fx) + t10[c] * fx;
            let bot = t01[c] * (1. - fx) + t11[c] * fx;
            top * (1. - fy) + bot * fy
        })
    }
    fn build(size: usize, mut texel: impl FnMut(usize, [f32; 2]) -> [f32; 3]) -> Cube {
        let faces = std::array::from_fn(|f| {
            (0..size * size).map(|i| {
                let (x, y) = (i % size, i / size);
                texel(f, [(x as f32 + 0.5) / size as f32 * 2. - 1., (y as f32 + 0.5) / size as f32 * 2. - 1.])
            }).collect()
        });
        Cube { size, faces }
    }
}

/// SKY LIGHT (2026-10-06): a value written to a PF_FloatRGBA render target (the reflection scratch cubes, `FSceneRenderTargets::ReflectionColorScratchCubemap`):
/// rounded to half float, round-to-nearest-even like the D3D11 float32 -> float16 conversion. Each capture stage reads the previous one at that precision.
pub fn round_fp16(v: [f32; 3]) -> [f32; 3] { v.map(|x| f16_bits_to_f32(crate::sky_reflection::f32_to_f16(x))) }
/// IEEE half -> f32.
pub fn f16_bits_to_f32(h: u16) -> f32 {
    let s = if h & 0x8000 != 0 { -1f32 } else { 1. };
    let (e, m) = (((h >> 10) & 0x1f) as i32, (h & 0x3ff) as f32);
    match e { 0 => s * m * 2f32.powi(-24), 0x1f => if m == 0. { s * f32::INFINITY } else { f32::NAN }, _ => s * (1. + m / 1024.) * 2f32.powi(e - 15) }
}
/// `CopyCubemapToCubeFaceColorPS` into a `size` scratch cube (rotation about Z by `angle_rad`; with `lower_black` the lower hemisphere is set
/// to `LowerHemisphereColor` = black at alpha 1, the class default); the scratch cube is PF_FloatRGBA ([`round_fp16`]).
pub fn copy_to_scratch(src: &Cube, size: usize, angle_rad: f32, lower_black: bool) -> Cube {
    let (s, c) = angle_rad.sin_cos();
    Cube::build(size, |f, uv| {
        let mut d = cubemap_vector(f, uv);
        d = Vec3::new(d.x * c - d.y * s, d.x * s + d.y * c, d.z);
        round_fp16(if lower_black && d.z < 0. { [0.; 3] } else { src.sample(d) })
    })
}
/// `DownsamplePS` (ReflectionEnvironmentShaders.usf:169): one mip of `CreateCubeMips`, reading the level above.
pub fn downsample(src: &Cube) -> Cube {
    let size = src.size / 2;
    let offset = 2.0 * 2. / size as f32;
    const OFFS: [[f32; 2]; 8] = [[-0.7, -0.7], [0.7, -0.7], [-0.7, 0.7], [0.7, 0.7], [0., -1.], [-1., 0.], [1., 0.], [0., 1.]];
    Cube::build(size, |f, uv| {
        let cc = cubemap_vector(f, uv);
        let tz = cc.normalize();
        let tx = cubemap_vector(f, [uv[0], uv[1] + 1.]).cross(tz).normalize();
        let ty = tz.cross(tx);
        let mut acc = src.sample(cc);
        for o in OFFS {
            let v = src.sample(cc + tx * (o[0] * offset) + ty * (o[1] * offset));
            for k in 0..3 { acc[k] += v[k] * 0.375; }
        }
        round_fp16(acc.map(|v| v / (1. + 1. + 2.))) // the next scratch mip (PF_FloatRGBA)
    })
}
/// `SHBasisFunction3` (SHCommon.usf:150), V[0..9] = V0.xyzw, V1.xyzw, V2.
pub fn sh_basis3(d: Vec3) -> [f32; 9] {
    [0.282095, -0.488603 * d.y, 0.488603 * d.z, -0.488603 * d.x,
     1.092548 * d.x * d.y, -1.092548 * d.y * d.z, 0.315392 * (3. * d.z * d.z - 1.), -1.092548 * d.x * d.z,
     0.546274 * (d.x * d.x - d.y * d.y)]
}
/// `ComputeDiffuseIrradiance` of one cube level (`DiffuseIrradianceCopyPS` + accumulation + `AccumulateCubeFacesPS`).
pub fn project_sh(cube: &Cube) -> [[f32; 9]; 3] {
    let (mut sum, mut wsum) = ([[0f64; 9]; 3], 0f64);
    let n = cube.size;
    for f in 0..6 {
        for i in 0..n * n {
            let s = [((i % n) as f32 + 0.5) / n as f32 * 2. - 1., ((i / n) as f32 + 0.5) / n as f32 * 2. - 1.];
            let sq = 1. + s[0] * s[0] + s[1] * s[1];
            let w = 4. / (sq.sqrt() * sq);
            let y = sh_basis3(cubemap_vector(f, s).normalize());
            let l = cube.faces[f][i];
            for c in 0..3 { for k in 0..9 { sum[c][k] += (l[c] * y[k] * w) as f64; } }
            wsum += w as f64;
        }
    }
    std::array::from_fn(|c| std::array::from_fn(|k| (4. * std::f64::consts::PI * sum[c][k] / wsum.max(0.00001)) as f32))
}
/// The whole SLS_SpecifiedCubemap diffuse path: copy to `CUBEMAP_RESOLUTION`, mip chain down to `DIFFUSE_IRRADIANCE_CUBEMAP_SIZE`, SH.
pub fn project_specified_cubemap(src: &Cube) -> [[f32; 9]; 3] {
    let mut c = copy_to_scratch(src, CUBEMAP_RESOLUTION, SOURCE_CUBEMAP_ANGLE_DEG.to_radians(), LOWER_HEMISPHERE_IS_BLACK);
    while c.size > DIFFUSE_IRRADIANCE_CUBEMAP_SIZE { c = downsample(&c); }
    project_sh(&c)
}

/// `SetupSkyIrradianceEnvironmentMapConstants` (SceneRendering.cpp:531): the 7 float4 of `View.SkyIrradianceEnvironmentMap`, with the
/// `View.SkyLightColor` multiply folded in (`max(0, x) * c == max(0, x * c)` for c >= 0).
pub fn pack_sky_irradiance(sh: &[[f32; 9]; 3], tint: [f32; 3]) -> [[f32; 4]; 7] {
    let sqrt_pi = std::f32::consts::PI.sqrt();
    let c0 = 1.0 / (2. * sqrt_pi);
    let c1 = 3f32.sqrt() / (3. * sqrt_pi);
    let c2 = 15f32.sqrt() / (8. * sqrt_pi);
    let c3 = 5f32.sqrt() / (16. * sqrt_pi);
    let c4 = 0.5 * c2;
    let v = |c: usize| sh[c].map(|x| x * tint[c]);
    let (r, g, b) = (v(0), v(1), v(2));
    let mut o = [[0f32; 4]; 7];
    for (i, s) in [r, g, b].iter().enumerate() {
        o[i] = [-c1 * s[3], -c1 * s[1], c1 * s[2], c0 * s[0] - c3 * s[6]];
        o[3 + i] = [c2 * s[4], -c2 * s[5], 3. * c3 * s[6], -c2 * s[7]];
    }
    o[6] = [c4 * r[8], c4 * g[8], c4 * b[8], 1.];
    o
}
/// `GetSkySHDiffuse` (ReflectionEnvironmentShared.usf:67) for a UE-space normal.
pub fn eval_sky_sh(p: &[[f32; 4]; 7], n: Vec3) -> [f32; 3] {
    let nv = [n.x, n.y, n.z, 1.];
    let vb = [n.x * n.y, n.y * n.z, n.z * n.z, n.z * n.x];
    let vc = n.x * n.x - n.y * n.y;
    let dot = |a: [f32; 4], b: [f32; 4]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    std::array::from_fn(|c| (dot(p[c], nv) + dot(p[3 + c], vb) + p[6][c] * vc).max(0.))
}
/// Renderer world direction -> Unreal axes (inverse of `lighting::ue_to_world`, the canonical (x, z, y) swap; mirrored by `lit_common.wgsl` `sky_lookup`).
/// SUN-AXIS FIX (2026-10-05): was `(-z, x, y)`, the inverse of the old 90-deg-off sun map.
pub fn world_to_ue_dir(n: Vec3) -> Vec3 { Vec3::new(n.x, n.z, n.y) }

// ---- SKY LIGHT COVERAGE: the same View constants for the consumers that upload the cooked shaders' View buffer or the FX lit tail ----
/// `View.SkyLightColor` and the UNFOLDED `View.SkyIrradianceEnvironmentMap` of a time-of-day state, as UE uploads them into every View uniform buffer
/// (SceneRendering.cpp:839 `SkyLightColor = SkyLight->LightColor`, :855 `SetupSkyIrradianceEnvironmentMapConstants`). `LightColor` =
/// `FLinearColor(ToFColor(SkyLightColor)) * Intensity` (SkyLightComponent.cpp:118 / :633); alpha = the FColor alpha byte (linear, not sRGB) / 255 x Intensity.
pub fn view_sky_constants_of(st: &crate::time_of_day::TodState) -> ([f32; 4], [[f32; 4]; 7]) {
    let t = crate::lighting::sky_light_tint_of(st);
    let a = (st.sky_light_alpha.clamp(0., 1.) * 255.999).floor() / 255. * st.sky_light_alpha;
    ([t[0], t[1], t[2], a], pack_sky_irradiance(&IRRADIANCE_SH, [1., 1., 1.]))
}
/// [`view_sky_constants_of`] at the current clock.
pub fn view_sky_constants() -> ([f32; 4], [[f32; 4]; 7]) { view_sky_constants_of(&crate::time_of_day::state_at(crate::time_of_day::hour())) }
/// Size in bytes of [`fx_light_block`] (fn-fx `FxLight`, group 3 binding 0 of the lit DXBC FX programs).
pub const FX_LIGHT_BYTES: usize = 10 * 16;
/// fn-fx `FxLight`: to-sun direction, sun radiance, the legacy up-facing sky colour, then `View.SkyIrradianceEnvironmentMap` x `View.SkyLightColor`
/// (= `lighting::sky_sh`, 7 float4), which the lit tail evaluates with `GetSkySHDiffuse` on the GBuffer world normal (fn-fx program.rs).
pub fn fx_light_block() -> [[f32; 4]; 10] {
    let (s, r, k) = (crate::lighting::sun_direction(), crate::lighting::sun_radiance(), crate::lighting::sky_radiance());
    let sh = crate::lighting::sky_sh();
    let mut o = [[0f32; 4]; 10];
    o[0] = [s.x, s.y, s.z, 0.];
    o[1] = [r[0], r[1], r[2], 0.];
    o[2] = [k[0], k[1], k[2], 0.];
    o[3..].copy_from_slice(&sh);
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    fn uniform(size: usize, v: [f32; 3]) -> Cube { Cube { size, faces: std::array::from_fn(|_| vec![v; size * size]) } }
    #[test]
    fn face_uv_roundtrips_cubemap_vector() {
        for f in 0..6 {
            for s in [[-0.7f32, 0.3], [0.2, -0.9], [0., 0.]] {
                let (g, uv) = cube_face_uv(cubemap_vector(f, s));
                assert_eq!(g, f);
                assert!((uv[0] * 2. - 1. - s[0]).abs() < 1e-6 && (uv[1] * 2. - 1. - s[1]).abs() < 1e-6, "{f} {s:?} {uv:?}");
            }
        }
    }
    /// SKY LIGHT: D3D11 seamless cube filtering. On the edge between +X (value 1) and +Z (value 3) a bilinear sample is half one face, half the other (it
    /// used to return the face's own value); at the +X / +Y / +Z corner the missing tap is the average of the three faces; inside a face nothing changes.
    #[test]
    fn bilinear_sampling_is_seamless_across_faces() {
        let mut c = uniform(4, [0.; 3]);
        c.faces[0] = vec![[1.; 3]; 16]; c.faces[2] = vec![[2.; 3]; 16]; c.faces[4] = vec![[3.; 3]; 16];
        let edge = c.sample(Vec3::new(1., 0., 1.).normalize());
        assert!((edge[0] - 2.).abs() < 1e-5, "+X / +Z edge: {edge:?}");
        let corner = c.sample(Vec3::new(1., 1., 1.).normalize());
        assert!((corner[0] - 2.).abs() < 1e-5, "+X / +Y / +Z corner: {corner:?}");
        assert_eq!(c.sample(Vec3::new(1., 0.1, -0.2).normalize())[0], 1., "texels inside +X only");
        assert_eq!(round_fp16([1.0 + 1.0 / 4096.0, 0.1, 65520.0]), [1.0, 0.099_975_586, f32::INFINITY], "half float, round to nearest even");
    }
    #[test]
    fn uniform_sky_gives_its_radiance_for_every_normal() {
        let sh = project_specified_cubemap(&uniform(64, [2., 1., 0.5]));
        let p = pack_sky_irradiance(&sh, [1., 1., 1.]);
        for n in [Vec3::Z, -Vec3::Z, Vec3::X, Vec3::new(0.3, -0.5, 0.2).normalize()] {
            let e = eval_sky_sh(&p, n);
            for (c, want) in [2f32, 1., 0.5].into_iter().enumerate() { assert!((e[c] - want).abs() < 2e-3 * want, "{n:?} {e:?}"); }
        }
    }
    #[test]
    fn lower_hemisphere_reaches_down_facing_normals() {
        // a sky lit only from below: the down normal must see it (bLowerHemisphereIsBlack = 0), the up normal ~ nothing
        let mut c = uniform(64, [0.; 3]);
        c.faces[5] = vec![[1.; 3]; 64 * 64];
        let p = pack_sky_irradiance(&project_specified_cubemap(&c), [1., 1., 1.]);
        assert!(eval_sky_sh(&p, -Vec3::Z)[0] > 0.3 && eval_sky_sh(&p, Vec3::Z)[0] < 0.05);
        let black = copy_to_scratch(&c, 32, 0., true);
        assert!(black.faces[5].iter().all(|v| v[0] == 0.), "the flag zeroes the lower hemisphere");
    }
    #[test]
    fn view_constants_times_colour_equal_the_folded_sh() {
        // UE evaluates GetSkySHDiffuse(N) * View.SkyLightColor; the port's lit shaders use the folded form (lighting::sky_sh_of)
        for h in [6.5f32, 12., 18.5, 23.] {
            let st = crate::time_of_day::state_at(h);
            let (c, sh) = view_sky_constants_of(&st);
            let folded = crate::lighting::sky_sh_of(&st);
            for n in [Vec3::Z, -Vec3::Z, Vec3::X, Vec3::new(0.3, -0.6, 0.2).normalize()] {
                let (a, b) = (eval_sky_sh(&sh, n), eval_sky_sh(&folded, n));
                for k in 0..3 { assert!((a[k] * c[k] - b[k]).abs() <= 1e-5 * (1. + b[k]), "{h} {n:?} {a:?} x {c:?} vs {b:?}"); }
            }
        }
        assert_eq!(FX_LIGHT_BYTES, std::mem::size_of::<[[f32; 4]; 10]>());
    }
    #[test]
    fn generated_sh_matches_the_cooked_cubemap() {
        let Some(root) = fn_live::var_os("FN_TEST_ROOT") else { eprintln!("FN_TEST_ROOT unset: skipped"); return; };
        let set = fn_pak::PakSet::open_install(&root).expect("open paks");
        let rd = |ext: &str| set.read_game(CUBEMAP_PACKAGE, ext).map(|c| c.into_owned()).unwrap_or_default();
        let tex = fn_texture::Texture2D::from_files(&rd("uasset"), &rd("uexp"), &rd("ubulk")).expect("cubemap");
        assert!(tex.is_cube() && tex.format.name() == "PF_FloatRGBA", "{} {}", tex.class, tex.format.name());
        let mip = tex.best_mip().expect("mip");
        eprintln!("{CUBEMAP_PACKAGE}: {}x{} first_mip {} best mip {mip} ({}x{}) LODGroup {} {}", tex.size_x, tex.size_y, tex.first_mip, tex.mips[mip].width, tex.mips[mip].height, tex.lod_group, tex.compression_settings);
        let faces = std::array::from_fn(|f| {
            let img = tex.decode_slice(mip, f, Default::default()).expect("face");
            let fn_texture::Pixels::RgbaF16(px) = img.pixels else { panic!("not f16") };
            px.chunks(4).map(|p| [0, 1, 2].map(|c| { let v = fn_ueio::f16_to_f32(p[c]); if v.is_finite() { v } else { 0. } })).collect()
        });
        let sh = project_specified_cubemap(&Cube { size: tex.mips[mip].width, faces });
        let text = format!("[\n{}]\n", sh.iter().map(|c| format!("    [{}],\n", c.iter().map(|v| format!("{v:e}")).collect::<Vec<_>>().join(", "))).collect::<String>());
        if fn_live::var_os("FN_REGEN_SKY_SH").is_some() {
            std::fs::write(concat!(env!("CARGO_MANIFEST_DIR"), "/src/sky_light_sh.in"), &text).unwrap();
        }
        eprintln!("{text}");
        for c in 0..3 { for k in 0..9 { assert!((sh[c][k] - IRRADIANCE_SH[c][k]).abs() <= 1e-5 * (1. + sh[c][k].abs()), "[{c}][{k}] {} vs const {}", sh[c][k], IRRADIANCE_SH[c][k]); } }
    }
}
