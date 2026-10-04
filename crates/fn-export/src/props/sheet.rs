//! Contact sheets: a minimal CPU rasteriser (orthographic projection, z-buffer, bilinear texture sampling, alpha-mask
//! clipping, two-sided foliage, lambert + hemisphere ambient, 2x supersampling) that renders the *exported files* through
//! `fn-propdata` (so the sheet also checks the loader). Writes `<out>/sheet/contact-sheet.png` (one 3/4 view of every kind)
//! and `contact-sheet-NN.png` detail pages (three angles per kind, caption with LOD0 counts, health and resource).
use fn_propdata::{AlphaMode, Image, LodMesh, Material, PropCatalog, PropKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 0.0 {
        [a[0] / l, a[1] / l, a[2] / l]
    } else {
        a
    }
}

fn srgb_to_lin(c: f32) -> f32 {
    c.powf(2.2)
}
fn lin_to_srgb(c: f32) -> f32 {
    c.clamp(0.0, 1.0).powf(1.0 / 2.2)
}

/// Texture + sampling state of one material slot.
pub struct Surface {
    base: Option<Rc<Image>>,
    mask: Option<Rc<Image>>,
    alpha_from_mask: bool,
    mask_has_alpha: bool,
    masked: bool,
    cutoff: f32,
    two_sided: bool,
    tint: [f32; 3],
}

fn sample(img: &Image, u: f32, v: f32) -> [f32; 4] {
    let x = u * img.width as f32 - 0.5;
    let y = v * img.height as f32 - 0.5;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let w = img.width as i64;
    let h = img.height as i64;
    let wrap = |a: i64, n: i64| (((a % n) + n) % n) as u32;
    let px = |ix: i64, iy: i64| -> [f32; 4] {
        let p = img.pixel(wrap(ix, w), wrap(iy, h));
        [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]
    };
    let (a, b, c, d) = (px(x0 as i64, y0 as i64), px(x0 as i64 + 1, y0 as i64), px(x0 as i64, y0 as i64 + 1), px(x0 as i64 + 1, y0 as i64 + 1));
    let mut o = [0.0; 4];
    for i in 0..4 {
        o[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
    }
    o
}

pub fn surface_of(kind: &PropKind, slot: usize, cache: &mut HashMap<PathBuf, Option<Rc<Image>>>) -> Surface {
    let mut load = |t: Option<&fn_propdata::TextureRef>| -> Option<Rc<Image>> {
        let t = t?;
        cache.entry(t.png.clone()).or_insert_with(|| Image::load(&t.png).ok().map(Rc::new)).clone()
    };
    match kind.load_material(slot) {
        Ok(m) => surface_from(&m, &mut load),
        Err(_) => Surface { base: None, mask: None, alpha_from_mask: false, mask_has_alpha: false, masked: false, cutoff: 0.5, two_sided: false, tint: [1.0; 3] },
    }
}

fn surface_from(m: &Material, load: &mut dyn FnMut(Option<&fn_propdata::TextureRef>) -> Option<Rc<Image>>) -> Surface {
    let base = load(m.base_color_texture());
    let mask = load(m.mask_texture());
    let alpha_from_mask = m.slots.opacity_from.as_deref() == Some("mask");
    let mask_has_alpha = mask.as_ref().map_or(false, |i| i.rgba.chunks_exact(4).any(|p| p[3] != 255));
    Surface { base, mask, alpha_from_mask, mask_has_alpha, masked: m.alpha_mode == AlphaMode::Masked, cutoff: m.alpha_cutoff, two_sided: m.two_sided, tint: m.tint_guess.unwrap_or([1.0; 3]) }
}

struct Frame {
    w: usize,
    color: Vec<[f32; 3]>,
    depth: Vec<f32>,
}

/// Camera basis for a view: yaw / pitch in degrees around the mesh centre (Y up).
struct Cam {
    right: V3,
    up: V3,
    toward: V3,
}

fn cam(yaw: f32, pitch: f32) -> Cam {
    let (y, p) = (yaw.to_radians(), pitch.to_radians());
    let toward = [p.cos() * y.sin(), p.sin(), p.cos() * y.cos()];
    let right = norm(cross([0.0, 1.0, 0.0], toward));
    let up = cross(toward, right);
    Cam { right, up, toward }
}

/// Render `lod` from one angle into a `size` x `size` RGB image (row-major, 8 bit) over a sky gradient.
pub fn render_view(lod: &LodMesh, surfaces: &[Surface], yaw: f32, pitch: f32, size: usize) -> Vec<u8> {
    const SS: usize = 2;
    let n = size * SS;
    let c = cam(yaw, pitch);
    // fit: project every vertex, centre and scale the projected box
    let proj: Vec<[f32; 3]> = lod.positions.iter().map(|&p| [dot(p, c.right), dot(p, c.up), dot(p, c.toward)]).collect();
    let (mut mn, mut mx) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &proj {
        for k in 0..2 {
            mn[k] = mn[k].min(p[k]);
            mx[k] = mx[k].max(p[k]);
        }
    }
    let span = (mx[0] - mn[0]).max(mx[1] - mn[1]).max(1e-3);
    let scale = n as f32 * 0.9 / span;
    let cx = (mn[0] + mx[0]) * 0.5;
    let cy = (mn[1] + mx[1]) * 0.5;
    let to_px = |p: [f32; 3]| -> [f32; 3] { [n as f32 * 0.5 + (p[0] - cx) * scale, n as f32 * 0.5 - (p[1] - cy) * scale, p[2]] };
    let mut f = Frame { w: n, color: vec![[f32::NAN; 3]; n * n], depth: vec![f32::MIN; n * n] };
    let light = norm([0.45, 0.8, 0.5]);
    let has_uv = !lod.uvs.is_empty();
    for sec in &lod.sections {
        let surf = surfaces.get(sec.material_slot);
        for t in sec.indices.chunks_exact(3) {
            let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
            let (p0, p1, p2) = (to_px(proj[i0]), to_px(proj[i1]), to_px(proj[i2]));
            let area = (p1[0] - p0[0]) * (p2[1] - p0[1]) - (p2[0] - p0[0]) * (p1[1] - p0[1]);
            if area.abs() < 1e-9 {
                continue;
            }
            // geometric normal in world space decides the facing (glTF: counter clockwise = front)
            let gn = norm(cross(sub(lod.positions[i1], lod.positions[i0]), sub(lod.positions[i2], lod.positions[i0])));
            let facing = dot(gn, c.toward);
            let two_sided = surf.map_or(false, |s| s.two_sided);
            if facing <= 0.0 && !two_sided {
                continue;
            }
            let back = facing <= 0.0;
            let minx = p0[0].min(p1[0]).min(p2[0]).floor().max(0.0) as usize;
            let maxx = (p0[0].max(p1[0]).max(p2[0]).ceil() as usize).min(n - 1);
            let miny = p0[1].min(p1[1]).min(p2[1]).floor().max(0.0) as usize;
            let maxy = (p0[1].max(p1[1]).max(p2[1]).ceil() as usize).min(n - 1);
            let inv = 1.0 / area;
            for y in miny..=maxy {
                for x in minx..=maxx {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    let w0 = ((p1[0] - px) * (p2[1] - py) - (p2[0] - px) * (p1[1] - py)) * inv;
                    let w1 = ((p2[0] - px) * (p0[1] - py) - (p0[0] - px) * (p2[1] - py)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * p0[2] + w1 * p1[2] + w2 * p2[2];
                    let di = y * f.w + x;
                    if z <= f.depth[di] {
                        continue;
                    }
                    let uv = if has_uv {
                        let (a, b, cc) = (lod.uvs[0][i0], lod.uvs[0][i1], lod.uvs[0][i2]);
                        [w0 * a[0] + w1 * b[0] + w2 * cc[0], w0 * a[1] + w1 * b[1] + w2 * cc[1]]
                    } else {
                        [0.0, 0.0]
                    };
                    let mut albedo = [0.55f32, 0.55, 0.55];
                    let mut opacity = 1.0f32;
                    if let Some(s) = surf {
                        if let Some(img) = &s.base {
                            let t = sample(img, uv[0], uv[1]);
                            albedo = [srgb_to_lin(t[0]), srgb_to_lin(t[1]), srgb_to_lin(t[2])];
                            opacity = t[3];
                        }
                        if s.alpha_from_mask {
                            if let Some(m) = &s.mask {
                                let t = sample(m, uv[0], uv[1]);
                                opacity = if s.mask_has_alpha { t[3] } else { t[0] };
                            }
                        }
                        if s.masked && opacity < s.cutoff {
                            continue;
                        }
                        for k in 0..3 {
                            albedo[k] *= s.tint[k];
                        }
                    }
                    let mut nrm = if lod.normals.is_empty() {
                        gn
                    } else {
                        let (a, b, cc) = (lod.normals[i0], lod.normals[i1], lod.normals[i2]);
                        norm([w0 * a[0] + w1 * b[0] + w2 * cc[0], w0 * a[1] + w1 * b[1] + w2 * cc[1], w0 * a[2] + w1 * b[2] + w2 * cc[2]])
                    };
                    if back {
                        nrm = [-nrm[0], -nrm[1], -nrm[2]];
                    }
                    let ndl = dot(nrm, light).max(0.0);
                    let sky = 0.30 + 0.30 * (nrm[1] * 0.5 + 0.5);
                    let lit = sky + 0.85 * ndl;
                    f.depth[di] = z;
                    f.color[di] = [albedo[0] * lit, albedo[1] * lit, albedo[2] * lit];
                }
            }
        }
    }
    // resolve: average the SS x SS samples over a sky gradient
    let mut out = vec![0u8; size * size * 3];
    for y in 0..size {
        let t = y as f32 / size as f32;
        let bg = [srgb_to_lin(0.62 + 0.18 * (1.0 - t)), srgb_to_lin(0.74 + 0.12 * (1.0 - t)), srgb_to_lin(0.88)];
        for x in 0..size {
            let mut acc = [0.0f32; 3];
            for sy in 0..SS {
                for sx in 0..SS {
                    let c = f.color[(y * SS + sy) * n + x * SS + sx];
                    let c = if c[0].is_nan() { bg } else { c };
                    for k in 0..3 {
                        acc[k] += c[k];
                    }
                }
            }
            for k in 0..3 {
                out[(y * size + x) * 3 + k] = (lin_to_srgb(acc[k] / (SS * SS) as f32) * 255.0 + 0.5) as u8;
            }
        }
    }
    out
}

// ---- tiny 5x7 font -------------------------------------------------------------------------------------------------

const FONT: &[(char, [&str; 7])] = &[
    ('A', [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
    ('B', ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."]),
    ('C', [".####", "#....", "#....", "#....", "#....", "#....", ".####"]),
    ('D', ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."]),
    ('E', ["#####", "#....", "#....", "####.", "#....", "#....", "#####"]),
    ('F', ["#####", "#....", "#....", "####.", "#....", "#....", "#...."]),
    ('G', [".####", "#....", "#....", "#.###", "#...#", "#...#", ".###."]),
    ('H', ["#...#", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"]),
    ('I', ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "#####"]),
    ('J', ["..###", "...#.", "...#.", "...#.", "...#.", "#..#.", ".##.."]),
    ('K', ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"]),
    ('L', ["#....", "#....", "#....", "#....", "#....", "#....", "#####"]),
    ('M', ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"]),
    ('N', ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"]),
    ('O', [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
    ('P', ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."]),
    ('Q', [".###.", "#...#", "#...#", "#...#", "#.#.#", "#..#.", ".##.#"]),
    ('R', ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"]),
    ('S', [".####", "#....", "#....", ".###.", "....#", "....#", "####."]),
    ('T', ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."]),
    ('U', ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."]),
    ('V', ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."]),
    ('W', ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"]),
    ('X', ["#...#", "#...#", ".#.#.", "..#..", ".#.#.", "#...#", "#...#"]),
    ('Y', ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."]),
    ('Z', ["#####", "....#", "...#.", "..#..", ".#...", "#....", "#####"]),
    ('0', [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."]),
    ('1', ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."]),
    ('2', [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"]),
    ('3', ["####.", "....#", "....#", ".###.", "....#", "....#", "####."]),
    ('4', ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."]),
    ('5', ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."]),
    ('6', [".###.", "#....", "#....", "####.", "#...#", "#...#", ".###."]),
    ('7', ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."]),
    ('8', [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."]),
    ('9', [".###.", "#...#", "#...#", ".####", "....#", "....#", ".###."]),
    ('_', [".....", ".....", ".....", ".....", ".....", ".....", "#####"]),
    ('.', [".....", ".....", ".....", ".....", ".....", ".##..", ".##.."]),
    ('-', [".....", ".....", ".....", "#####", ".....", ".....", "....."]),
    (':', [".....", ".##..", ".##..", ".....", ".##..", ".##..", "....."]),
    ('/', ["....#", "....#", "...#.", "..#..", ".#...", "#....", "#...."]),
    ('(', ["..#..", ".#...", "#....", "#....", "#....", ".#...", "..#.."]),
    (')', ["..#..", "...#.", "....#", "....#", "....#", "...#.", "..#.."]),
    ('%', ["##..#", "##.#.", "..#..", "..#..", ".#.##", "#..##", "....."]),
    (' ', [".....", ".....", ".....", ".....", ".....", ".....", "....."]),
];

fn draw_text(img: &mut [u8], w: usize, x: usize, y: usize, text: &str, color: [u8; 3]) {
    let mut cx = x;
    for ch in text.chars() {
        let up = ch.to_ascii_uppercase();
        if let Some((_, g)) = FONT.iter().find(|(c, _)| *c == up) {
            for (ry, row) in g.iter().enumerate() {
                for (rx, b) in row.bytes().enumerate() {
                    if b == b'#' {
                        let (px, py) = (cx + rx, y + ry);
                        let i = (py * w + px) * 3;
                        if i + 2 < img.len() && px < w {
                            img[i..i + 3].copy_from_slice(&color);
                        }
                    }
                }
            }
        }
        cx += 6;
    }
}

fn blit(dst: &mut [u8], dw: usize, src: &[u8], sw: usize, sh: usize, x0: usize, y0: usize) {
    for y in 0..sh {
        let d = ((y0 + y) * dw + x0) * 3;
        let s = y * sw * 3;
        if d + sw * 3 <= dst.len() {
            dst[d..d + sw * 3].copy_from_slice(&src[s..s + sw * 3]);
        }
    }
}

fn write_png(path: &Path, rgb: &[u8], w: usize, h: usize) -> Result<(), String> {
    use image::ImageEncoder;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    image::codecs::png::PngEncoder::new(std::io::BufWriter::new(f)).write_image(rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())
}

fn render_kind(kind: &PropKind, cache: &mut HashMap<PathBuf, Option<Rc<Image>>>, lod: usize, angles: &[(f32, f32)], size: usize) -> Result<Vec<Vec<u8>>, String> {
    let lods = kind.load_lods().map_err(|e| format!("{}: {e}", kind.id))?;
    let l = lods.get(lod).or_else(|| lods.first()).ok_or("no LOD")?;
    let surfaces: Vec<Surface> = (0..kind.materials.len().max(1)).map(|s| surface_of(kind, s, cache)).collect();
    Ok(angles.iter().map(|&(y, p)| render_view(l, &surfaces, y, p, size)).collect())
}

/// Render the whole catalog under `out` and return the written file names (relative to `out/sheet`).
pub fn run(out: &Path) -> Result<Vec<String>, String> {
    let cat = PropCatalog::load(out).map_err(|e| e.to_string())?;
    let mut cache: HashMap<PathBuf, Option<Rc<Image>>> = HashMap::new();
    let mut files = vec![];
    let dir = out.join("sheet");
    // ---- overview: one 3/4 view per kind ----
    let (cell, cols) = (160usize, 8usize);
    let rows = (cat.kinds.len() + cols - 1) / cols;
    let lab = 10;
    let (ow, oh) = (cols * cell, rows * (cell + lab));
    let mut ov = vec![255u8; ow * oh * 3];
    for (i, k) in cat.kinds.iter().enumerate() {
        let (cx, cy) = ((i % cols) * cell, (i / cols) * (cell + lab));
        match render_kind(k, &mut cache, 0, &[(35.0, 18.0)], cell) {
            Ok(v) => blit(&mut ov, ow, &v[0], cell, cell, cx, cy + lab),
            Err(e) => eprintln!("render {}: {e}", k.id),
        }
        let short: String = k.id.chars().take(cell / 6).collect();
        draw_text(&mut ov, ow, cx + 2, cy + 1, &short, [20, 20, 20]);
    }
    write_png(&dir.join("contact-sheet.png"), &ov, ow, oh)?;
    files.push("contact-sheet.png".to_string());
    // ---- detail pages: 3 angles per kind, 2 kinds per row ----
    let angles = [(35.0f32, 15.0f32), (125.0, 15.0), (215.0, 55.0)];
    let (cell, per_row, rows_per_page, lab) = (176usize, 2usize, 5usize, 12usize);
    let per_page = per_row * rows_per_page;
    let block_w = cell * 3;
    for (pi, chunk) in cat.kinds.chunks(per_page).enumerate() {
        let (w, h) = (block_w * per_row, rows_per_page * (cell + lab));
        let mut pg = vec![255u8; w * h * 3];
        for (i, k) in chunk.iter().enumerate() {
            let (bx, by) = ((i % per_row) * block_w, (i / per_row) * (cell + lab));
            match render_kind(k, &mut cache, 0, &angles, cell) {
                Ok(views) => {
                    for (vi, v) in views.iter().enumerate() {
                        blit(&mut pg, w, v, cell, cell, bx + vi * cell, by + lab);
                    }
                }
                Err(e) => eprintln!("render {}: {e}", k.id),
            }
            let l0 = k.lods.first();
            let cap = format!("{} LOD0 {}V {}T  HP {}  {:?}", k.id, l0.map_or(0, |l| l.vertices), l0.map_or(0, |l| l.triangles), k.health, k.resource.kind);
            draw_text(&mut pg, w, bx + 2, by + 2, &cap.to_uppercase(), [20, 20, 20]);
        }
        let name = format!("contact-sheet-{:02}.png", pi + 1);
        write_png(&dir.join(&name), &pg, w, h)?;
        files.push(name);
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_propdata::MeshSection;

    fn quad() -> LodMesh {
        LodMesh {
            positions: vec![[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 2.0, 0.0], [-1.0, 2.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            tangents: vec![],
            colors: vec![],
            uvs: vec![vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]],
            sections: vec![MeshSection { material_slot: 0, indices: vec![0, 1, 2, 0, 2, 3] }],
            screen_size: 1.0,
        }
    }

    #[test]
    fn front_face_is_drawn_and_back_face_culled() {
        let s = [Surface { base: None, mask: None, alpha_from_mask: false, mask_has_alpha: false, masked: false, cutoff: 0.5, two_sided: false, tint: [1.0; 3] }];
        let lit = |yaw: f32| {
            let img = render_view(&quad(), &s, yaw, 0.0, 32);
            // centre pixel vs. the left edge of the same row (sky)
            let c = &img[(16 * 32 + 16) * 3..(16 * 32 + 16) * 3 + 3];
            let k = &img[(16 * 32) * 3..(16 * 32) * 3 + 3];
            c != k
        };
        assert!(lit(0.0), "CCW quad seen from +Z must render");
        assert!(!lit(180.0), "seen from behind it is culled");
    }

    #[test]
    fn masked_texture_clips_pixels() {
        let mut rgba = vec![0u8; 4 * 4 * 4];
        for (i, p) in rgba.chunks_exact_mut(4).enumerate() {
            let x = i % 4;
            // right half transparent
            p.copy_from_slice(&[200, 40, 40, if x < 2 { 255 } else { 0 }]);
        }
        let img = Image { width: 4, height: 4, rgba };
        let s = [Surface { base: Some(Rc::new(img)), mask: None, alpha_from_mask: false, mask_has_alpha: false, masked: true, cutoff: 0.5, two_sided: true, tint: [1.0; 3] }];
        let out = render_view(&quad(), &s, 0.0, 0.0, 64);
        let px = |x: usize, y: usize| out[(y * 64 + x) * 3..(y * 64 + x) * 3 + 3].to_vec();
        let (l, r) = (px(26, 32), px(38, 32));
        assert!(l[0] > l[2] + 20, "left half shows the red texel, got {l:?}");
        assert!(r[2] >= r[0], "right half is clipped to the sky, got {r:?}");
    }

    #[test]
    fn font_covers_caption_characters() {
        for ch in "ATHENA_TREE_PINE_3 LOD0 123V HP 600.5 WOOD".chars() {
            assert!(FONT.iter().any(|(c, _)| *c == ch.to_ascii_uppercase()), "missing glyph {ch}");
        }
    }
}
