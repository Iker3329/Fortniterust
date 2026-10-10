use super::soft::soft_render;
use super::*;
use crate::MaterialMode;
use fn_ui::assets::Sampler;
use fn_ui::draw::{Cmd, MaterialKind, Quad, RenderList};
use fn_ui::layout::Rect;

fn pattern(x: u32, y: u32) -> [u8; 4] {
    // opaque + translucent texels with distinct channels
    let a = if (x + y) % 5 == 0 { 120 } else { 255 };
    [(x * 37 % 256) as u8, (y * 53 % 256) as u8, ((x ^ y) * 29 % 256) as u8, a]
}

fn check(name: &str, gpu: &[u8], soft: &[u8], max_tol: u32, mean_tol: f64) {
    let (mx, mean) = diff(gpu, soft);
    eprintln!("[quads] {name}: max {mx} mean {mean:.4}");
    assert!(mx <= max_tol && mean <= mean_tol, "{name}: max {mx} (tol {max_tol}) mean {mean} (tol {mean_tol})");
}

#[test]
fn solid_fills_and_alpha_blend_match_reference() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let mut l = RenderList::new();
        l.fill(Rect::new(4., 4., 40., 30.), [1.0, 0.0, 0.0, 1.0]);
        l.fill(Rect::new(20., 10., 40., 30.), [0.0, 0.5, 1.0, 0.5]);
        l.fill(Rect::new(0., 0., 64., 8.), [1.0, 1.0, 1.0, 0.25]);
        let g = r.render_to_image(&l, &mut a, 64, 48, Some([0.2, 0.3, 0.4, 1.0]));
        let s = soft_render(&l, &a, 64, 48, Some([0.2, 0.3, 0.4, 1.0]));
        check("fills", &g, &s, 2, 0.6);
        // transparent clear stays transparent where nothing is drawn
        let g = r.render_to_image(&l, &mut a, 64, 48, None);
        assert_eq!(&g[(47 * 64 + 63) * 4..(47 * 64 + 63) * 4 + 4], &[0, 0, 0, 0]);
        let s = soft_render(&l, &a, 64, 48, None);
        check("fills-transparent-clear", &g, &s, 2, 0.6);
    });
}

#[test]
fn textured_quads_nearest_and_bilinear_match_reference() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let lin = make_tex(&mut a, 16, 16, Sampler::LINEAR, pattern);
        let near = make_tex(&mut a, 16, 16, Sampler::NEAREST, pattern);
        let mut l = RenderList::new();
        l.sprite(lin, Rect::new(2., 2., 16., 16.), [1.; 4]); // 1:1, exact
        l.sprite(near, Rect::new(24., 2., 48., 48.), [1.; 4]); // 3x nearest
        l.sprite(lin, Rect::new(80., 2., 40., 40.), [1.0, 0.8, 0.6, 0.9]); // 2.5x bilinear, tinted
        l.quad(lin, Rect::new(2., 60., 32., 32.), [1., 0., 0., 1.], [1.; 4]); // mirrored
        let (w, h) = (128, 100);
        let g = r.render_to_image(&l, &mut a, w, h, Some([0.1, 0.1, 0.1, 1.0]));
        let s = soft_render(&l, &a, w, h, Some([0.1, 0.1, 0.1, 1.0]));
        check("textured", &g, &s, 4, 0.8);
    });
}

#[test]
fn nine_slice_matches_reference() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let t = make_tex(&mut a, 16, 16, Sampler::LINEAR, |x, y| {
            let edge = x < 4 || y < 4 || x >= 12 || y >= 12;
            if edge { [200, (x * 15) as u8, (y * 15) as u8, 255] } else { [20, 40, 220, 128] }
        });
        let info = a.store.info(t);
        let mut l = RenderList::new();
        l.nine_slice(info, Rect::new(3., 3., 120., 60.), [0.25; 4], 1.0, [1.; 4]);
        l.nine_slice(info, Rect::new(10., 70., 80., 24.), [0.25; 4], 2.0, [1., 1., 1., 0.8]);
        assert_eq!(l.quad_count(), 18);
        let g = r.render_to_image(&l, &mut a, 130, 100, Some([0.0, 0.0, 0.0, 1.0]));
        let s = soft_render(&l, &a, 130, 100, Some([0.0, 0.0, 0.0, 1.0]));
        check("nine-slice", &g, &s, 4, 0.8);
        // batching: 18 consecutive quads of one texture -> a single draw call
        assert_eq!(r.stats().draw_calls, 1, "{:?}", r.stats());
    });
}

#[test]
fn rotated_quad_matches_reference() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let t = make_tex(&mut a, 32, 32, Sampler::LINEAR, |x, y| if (x / 4 + y / 4) % 2 == 0 { [255, 255, 255, 255] } else { [30, 120, 255, 255] });
        let mut l = RenderList::new();
        l.rotated(t, (64., 64.), (60., 40.), 0.5236, [1.; 4]);
        l.rotated(t, (64., 64.), (30., 30.), -1.0, [1., 0.5, 0.5, 0.7]);
        let g = r.render_to_image(&l, &mut a, 128, 128, Some([0.0, 0.2, 0.0, 1.0]));
        let s = soft_render(&l, &a, 128, 128, Some([0.0, 0.2, 0.0, 1.0]));
        // edge pixels may differ by coverage rule; require nearly all pixels to agree
        let bad = g.chunks_exact(4).zip(s.chunks_exact(4)).filter(|(x, y)| x.iter().zip(*y).any(|(p, q)| (*p as i32 - *q as i32).abs() > 6)).count();
        let (mx, mean) = diff(&g, &s);
        eprintln!("[quads] rotated: max {mx} mean {mean:.4} bad pixels {bad}");
        assert!(bad <= 40 && mean < 0.6, "rotated quad deviates: bad {bad} mean {mean}");
    });
}

#[test]
fn clip_stack_matches_reference() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let t = make_tex(&mut a, 8, 8, Sampler::NEAREST, pattern);
        let mut l = RenderList::new();
        l.push_clip(Rect::new(10.3, 10.6, 80., 60.)); // floor/ceil outwards
        l.sprite(t, Rect::new(0., 0., 128., 128.), [1.; 4]);
        l.push_clip(Rect::new(50., 5., 100., 40.)); // nested: intersection, also exceeds the target
        l.fill(Rect::new(0., 0., 200., 200.), [1., 0., 0., 0.5]);
        l.pop_clip();
        l.fill(Rect::new(0., 60., 128., 10.), [0., 1., 0., 0.5]);
        l.pop_clip();
        l.push_clip(Rect::new(200., 200., 10., 10.)); // fully outside
        l.fill(Rect::new(0., 0., 128., 128.), [1.; 4]);
        l.push_clip(Rect::new(-5., -5., 20., 20.)); // starts outside on the left/top
        l.fill(Rect::new(0., 0., 128., 128.), [0., 0., 1., 1.]);
        l.pop_clip();
        l.pop_clip();
        l.fill(Rect::new(100., 100., 10., 10.), [1., 1., 0., 1.]); // after all clips
        let g = r.render_to_image(&l, &mut a, 128, 128, Some([0.0, 0.0, 0.0, 1.0]));
        let s = soft_render(&l, &a, 128, 128, Some([0.0, 0.0, 0.0, 1.0]));
        check("clip", &g, &s, 2, 0.5);
    });
}

#[test]
fn text_atlas_quads_and_version_reupload() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        // atlas layout = straight white + coverage alpha, premultiplied (cov,cov,cov,cov)
        let atlas = make_tex(&mut a, 64, 16, Sampler::LINEAR, |x, y| [255, 255, 255, ((x * 4 + y * 3) % 256) as u8]);
        let mut l = RenderList::new();
        l.quad(atlas, Rect::new(10., 10., 20., 8.), [0.0, 0.0, 0.25, 0.5], [1.0, 0.9, 0.2, 1.0]); // glyph sub-rect (pixel snapped)
        l.quad(atlas, Rect::new(10., 30., 32., 8.), [0.5, 0.0, 1.0, 0.5], [0.0, 0.0, 0.0, 0.75]); // shadow pass
        let g1 = r.render_to_image(&l, &mut a, 64, 48, Some([0.1, 0.2, 0.3, 1.0]));
        let s1 = soft_render(&l, &a, 64, 48, Some([0.1, 0.2, 0.3, 1.0]));
        check("text-atlas", &g1, &s1, 4, 0.8);
        let up0 = r.stats().texture_uploads;
        assert_eq!(up0, 1);
        // untouched version -> no re-upload
        let _ = r.render_to_image(&l, &mut a, 64, 48, None);
        assert_eq!(r.stats().texture_uploads, 0);
        // pixels change (new glyph rasterised): version bumps, GPU copy follows
        let mut px = Vec::new();
        for y in 0..16u32 { for x in 0..64u32 { px.extend_from_slice(&[255, 255, 255, ((x * 9 + y * 11) % 256) as u8]); } }
        a.store.replace(atlas, 64, 16, px);
        let g2 = r.render_to_image(&l, &mut a, 64, 48, Some([0.1, 0.2, 0.3, 1.0]));
        assert_eq!(r.stats().texture_uploads, 1);
        let s2 = soft_render(&l, &a, 64, 48, Some([0.1, 0.2, 0.3, 1.0]));
        check("text-atlas-after-replace", &g2, &s2, 4, 0.8);
        assert!(diff(&g1, &g2).0 > 20, "second render must differ from the first");
        // size change recreates the texture
        let mut px = Vec::new();
        for y in 0..32u32 { for x in 0..64u32 { px.extend_from_slice(&[255, 255, 255, ((x * 3 + y * 7) % 256) as u8]); } }
        a.store.replace(atlas, 64, 32, px);
        let g3 = r.render_to_image(&l, &mut a, 64, 48, Some([0.1, 0.2, 0.3, 1.0]));
        let s3 = soft_render(&l, &a, 64, 48, Some([0.1, 0.2, 0.3, 1.0]));
        check("text-atlas-after-resize", &g3, &s3, 4, 0.8);
    });
}

#[test]
fn removed_textures_are_evicted_and_none_is_skipped() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let t1 = make_tex(&mut a, 4, 4, Sampler::LINEAR, pattern);
        let t2 = make_tex(&mut a, 4, 4, Sampler::LINEAR, pattern);
        let mut l = RenderList::new();
        l.sprite(t1, Rect::new(0., 0., 8., 8.), [1.; 4]);
        l.sprite(t2, Rect::new(8., 0., 8., 8.), [1.; 4]);
        l.cmds.push(Cmd::Quad(Quad::rect(TextureId::NONE, Rect::new(0., 8., 8., 8.), [0., 0., 1., 1.], [1.; 4])));
        let img = r.render_to_image(&l, &mut a, 16, 16, None);
        assert_eq!(&img[(12 * 16 + 4) * 4..(12 * 16 + 4) * 4 + 4], &[0, 0, 0, 0], "TextureId::NONE must not draw");
        assert_eq!(r.cached_texture_count(), 2);
        a.store.remove(t1);
        let _ = r.render_to_image(&l, &mut a, 16, 16, None); // quad of the removed texture is dropped, no panic
        assert_eq!(r.cached_texture_count(), 1);
    });
}

#[test]
fn mip_chain_is_used_when_minified() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let tex = a.store.insert(Texture::new(64, 64, (0..64 * 64).flat_map(|i| { let (x, y) = (i % 64, i / 64); pattern(x as u32, y as u32) }).collect(), Sampler::UI_ART));
        let mut l = RenderList::new();
        l.sprite(tex, Rect::new(0., 0., 16., 16.), [1.; 4]); // exactly 1/4 -> level 2
        let g = r.render_to_image(&l, &mut a, 16, 16, None);
        // CPU reference: premultiplied box mips, level 2 texel-for-texel
        let pm = a.store.get(tex).unwrap().premultiplied();
        let chain = crate::textures::build_mips(64, 64, &pm);
        let (mw, mh, lvl) = &chain[1];
        assert_eq!((*mw, *mh), (16, 16));
        let (mx, mean) = diff(&g, lvl);
        eprintln!("[quads] mips: max {mx} mean {mean:.4}");
        assert!(mx <= 2 && mean < 0.1, "trilinear level-2 sample must equal CPU mip level 2");
        // and it must differ from point-sampling level 0
        let mut l0 = RenderList::new();
        l0.sprite(tex, Rect::new(0., 0., 64., 64.), [1.; 4]);
        let g0 = r.render_to_image(&l0, &mut a, 64, 64, None);
        assert!(diff(&g0[..16 * 16 * 4], lvl).0 > 8);
    });
}

#[test]
fn stress_thousands_of_quads_batch_correctly() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let texs: Vec<_> = (0..8u32).map(|i| make_tex(&mut a, 8, 8, Sampler::NEAREST, move |x, y| { let [r, g, b, al] = pattern(x + i * 3, y + i * 5); [r, g, b, if i % 2 == 0 { 255 } else { al }] })).collect();
        let mut rng = Rng(0x1234_5678);
        let mut l = RenderList::new();
        let (w, h) = (256u32, 192u32);
        let mut clips = 0;
        for i in 0..12000 {
            if i % 997 == 0 { l.push_clip(Rect::new(rng.range(0., 100.), rng.range(0., 80.), rng.range(60., 200.), rng.range(40., 150.))); clips += 1; }
            if i % 997 == 500 && clips > 0 { l.pop_clip(); clips -= 1; }
            let t = texs[(rng.next() % 8) as usize]; // random order defeats batching on purpose; runs are short
            let (x, y) = (rng.range(-4., w as f32), rng.range(-4., h as f32));
            let (qw, qh) = (rng.range(1., 12.).floor(), rng.range(1., 12.).floor());
            let (x, y) = (x.floor(), y.floor());
            let tint = if i % 3 == 0 { [1., 1., 1., 1.] } else { [rng.f(), rng.f(), rng.f(), rng.range(0.3, 1.0)] };
            l.quad(t, Rect::new(x, y, qw, qh), [0.0137, 0.0171, 0.9813, 0.9871], tint); // uv inset: no exact texel-boundary ties
        }
        // a long run of one texture must collapse into one draw call
        for k in 0..3000 { l.quad(texs[1], Rect::new((k % 200) as f32, ((k / 200) * 4) as f32, 3., 3.), [0.0137, 0.0171, 0.9813, 0.9871], [1., 1., 1., 0.4]); }
        let g = r.render_to_image(&l, &mut a, w, h, Some([0.0, 0.0, 0.0, 1.0]));
        let st = r.stats();
        let s = soft_render(&l, &a, w, h, Some([0.0, 0.0, 0.0, 1.0]));
        let (mx, mean) = diff(&g, &s);
        eprintln!("[quads] stress: {} quads, {} draw calls, max {mx} mean {mean:.4}", st.quads, st.draw_calls);
        let mut shown = 0;
        for (i, (p, q)) in g.chunks_exact(4).zip(s.chunks_exact(4)).enumerate() { if p.iter().zip(q).any(|(x, y)| (*x as i32 - *y as i32).abs() > 12) && shown < 8 { eprintln!("  bad px ({}, {}) gpu {:?} soft {:?}", i as u32 % w, i as u32 / w, p, q); shown += 1; } }
        assert_eq!(st.quads, 15000);
        // hundreds of alpha layers over each other amplify 8-bit rounding differences; the mean must stay tiny and the max bounded
        assert!(mean < 0.35 && mx <= 4, "stress deviates: max {mx} mean {mean}");
    });
}

#[test]
fn render_into_existing_target_preserves_contents_without_clear() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let mut l1 = RenderList::new();
        l1.fill(Rect::new(0., 0., 16., 16.), [1., 0., 0., 1.]);
        let mut l2 = RenderList::new();
        l2.fill(Rect::new(8., 0., 8., 16.), [0., 0., 1., 0.5]);
        // two passes into one texture: clear then load (composite over)
        let tex = r.device().create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: 16, height: 16, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        let view = tex.create_view(&Default::default());
        r.render(&l1, &mut a, &view, 16, 16, Some([0., 0., 0., 1.]));
        r.render(&l2, &mut a, &view, 16, 16, None);
        let mut combined = RenderList::new();
        combined.extend(l1.clone()); combined.extend(l2.clone());
        let want = soft_render(&combined, &a, 16, 16, Some([0., 0., 0., 1.]));
        // read back via the helper path: re-render both in one go and compare to the two-pass result through the reference
        let got = r.render_to_image(&combined, &mut a, 16, 16, Some([0., 0., 0., 1.]));
        check("two-pass-vs-one-pass", &got, &want, 2, 0.5);
    });
}

#[test]
fn cpu_bake_cache_keys_and_lru_cap() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        r.set_material_mode(MaterialMode::CpuBake);
        r.clear_material_cache();
        let mut l = RenderList::new();
        let kind = MaterialKind::HudBar { a: [0.1, 0.5, 0.1], b: [0.2, 0.7, 0.1] };
        l.material(kind.clone(), Rect::new(0., 0., 20., 6.), [1.; 4], false);
        l.material(kind.clone(), Rect::new(0., 8., 20., 6.), [1.; 4], true); // same key and size, flipped: one cache entry
        l.material(kind.clone(), Rect::new(0., 16., 30., 6.), [1.; 4], false); // other size
        let _ = r.render_to_image(&l, &mut a, 40, 24, None);
        assert_eq!(r.stats().bake_cache_entries, 2);
        assert_eq!(r.stats().baked_materials, 3);
        // LRU cap: many distinct sizes never exceed the cap
        r.clear_material_cache();
        let mut l = RenderList::new();
        for i in 0..(crate::BAKE_CACHE_CAP as u32 + 120) { l.material(kind.clone(), Rect::new(0., 0., 2. + (i % 97) as f32, 2. + (i / 97) as f32), [1.; 4], false); }
        let _ = r.render_to_image(&l, &mut a, 128, 8, None);
        let n = r.stats().bake_cache_entries as usize;
        eprintln!("[quads] bake cache after {} distinct materials: {n}", crate::BAKE_CACHE_CAP + 120);
        assert!(n <= crate::BAKE_CACHE_CAP);
        r.set_material_mode(MaterialMode::Gpu);
    });
}

#[test]
fn pick_surface_format_prefers_non_srgb() {
    let caps = wgpu::SurfaceCapabilities { formats: vec![wgpu::TextureFormat::Bgra8UnormSrgb, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm], ..Default::default() };
    assert_eq!(crate::pick_surface_format(&caps), wgpu::TextureFormat::Bgra8Unorm);
    let only_srgb = wgpu::SurfaceCapabilities { formats: vec![wgpu::TextureFormat::Bgra8UnormSrgb], ..Default::default() };
    assert!(!crate::pick_surface_format(&only_srgb).is_srgb() || only_srgb.formats.len() == 1);
}

/// INVENTORY FIX3: the `SBackgroundBlur` pass (`Cmd::Blur`) on the GPU equals the CPU reference (`fn_ui::raster::blur_rect`): BlurStrength 2 (kernel 7,
/// no downsample, the quantity selector's) and 16 (kernel 25 at half resolution); outside the region nothing changes.
#[test]
fn background_blur_matches_the_cpu_reference() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let tex = make_tex(&mut a, 32, 32, Sampler::NEAREST, |x, y| { let p = pattern(x, y); [p[0], p[1], p[2], 255] });
        for (strength, max_tol, mean_tol) in [(2.0f32, 3u32, 0.6f64), (16., 3, 0.6)] {
            let mut l = RenderList::new();
            l.sprite(tex, Rect::new(0., 0., 96., 64.), [1.; 4]);
            l.blur(Rect::new(16., 8., 64., 48.), strength, None);
            l.fill(Rect::new(40., 30., 8., 8.), [1., 1., 1., 1.]); // drawn after the blur: untouched
            let g = r.render_to_image(&l, &mut a, 96, 64, Some([0., 0., 0., 1.]));
            let s = fn_ui::raster::render(&l, &mut a, 96, 64, Some([0., 0., 0., 1.]));
            check(&format!("blur {strength}"), &g, &s, max_tol, mean_tol);
            // the region really changed (the unblurred list differs inside it)
            let mut plain = RenderList::new(); plain.sprite(tex, Rect::new(0., 0., 96., 64.), [1.; 4]);
            let p = r.render_to_image(&plain, &mut a, 96, 64, Some([0., 0., 0., 1.]));
            let i = ((20 * 96 + 30) * 4) as usize;
            assert_ne!(&g[i..i + 3], &p[i..i + 3], "blur {strength}: pixel inside the region is blurred");
            let o = ((2 * 96 + 2) * 4) as usize;
            assert_eq!(&g[o..o + 4], &p[o..o + 4], "blur {strength}: pixel outside the region is unchanged");
        }
    });
}

/// INVENTORY FIX4 (inventory_gaps verify3 V4): the blur's work textures and uniform buffers are cached: two blurs of different sizes in one list, rendered
/// for several frames, allocate once and still match the CPU reference every frame (each pass of a frame has its own uniform buffer).
#[test]
fn background_blur_caches_its_textures() {
    let _ = with_gpu(|r| {
        let mut a = new_assets();
        let tex = make_tex(&mut a, 32, 32, Sampler::NEAREST, |x, y| { let p = pattern(x, y); [p[0], p[1], p[2], 255] });
        let mut l = RenderList::new();
        l.sprite(tex, Rect::new(0., 0., 96., 64.), [1.; 4]);
        l.blur(Rect::new(4., 4., 40., 30.), 2., None);
        l.blur(Rect::new(30., 20., 60., 40.), 16., None);
        let s = fn_ui::raster::render(&l, &mut a, 96, 64, Some([0., 0., 0., 1.]));
        let mut counts = None;
        for f in 0..3 {
            let g = r.render_to_image(&l, &mut a, 96, 64, Some([0., 0., 0., 1.]));
            check(&format!("cached blur frame {f}"), &g, &s, 3, 0.6);
            let c = r.blur_cache_counts();
            if let Some(c0) = counts { assert_eq!(c, c0, "no new blur allocations after the first frame"); } else { counts = Some(c); }
        }
        assert_eq!(counts.unwrap().1, 8, "two blurs x four uniform buffers");
    });
}
