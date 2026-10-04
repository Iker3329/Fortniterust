//! CPU reference implementations of every pass (the same maths as the WGSL, written independently and checked against the compiled DXBC
//! through the `fn-dxbc` interpreter in `tests/`). Images are `Vec<[f32; 4]>`, row 0 = top, texel centres at +0.5 like D3D.
//!
//! Sampling model: `sample` = bilinear with clamp-to-edge (what the GPU does with `FilterMode::Linear` + `ClampToEdge`; the GPU
//! quantises the interpolation weights to 8 bits, so GPU and CPU agree to about 1/256 of the local contrast), `load`/`point` = nearest.
use crate::params::*;

#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<[f32; 4]>,
}

impl Image {
    pub fn new(w: usize, h: usize) -> Self { Image { w, h, px: vec![[0.0; 4]; w * h] } }
    pub fn from_fn(w: usize, h: usize, mut f: impl FnMut(usize, usize) -> [f32; 4]) -> Self {
        let mut im = Image::new(w, h);
        for y in 0..h { for x in 0..w { im.px[y * w + x] = f(x, y); } }
        im
    }
    pub fn load(&self, x: i32, y: i32) -> [f32; 4] {
        let xi = x.clamp(0, self.w as i32 - 1) as usize;
        let yi = y.clamp(0, self.h as i32 - 1) as usize;
        self.px[yi * self.w + xi]
    }
    /// Nearest texel for a uv in [0,1] (what `textureLoad(floor(uv * size))` does).
    pub fn point(&self, u: f32, v: f32) -> [f32; 4] { self.load((u * self.w as f32).floor() as i32, (v * self.h as f32).floor() as i32) }
    /// Bilinear, clamp to edge, texel centres at +0.5.
    pub fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let x = u * self.w as f32 - 0.5;
        let y = v * self.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (xi, yi) = (x0 as i32, y0 as i32);
        let a = self.load(xi, yi);
        let b = self.load(xi + 1, yi);
        let c = self.load(xi, yi + 1);
        let d = self.load(xi + 1, yi + 1);
        let mut o = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * fx;
            let bot = c[i] + (d[i] - c[i]) * fx;
            o[i] = top + (bot - top) * fy;
        }
        o
    }
    /// Round every channel to f16 (the render targets are `Rgba16Float`).
    pub fn quantize_f16(&mut self) { for p in &mut self.px { for c in p.iter_mut() { *c = round_f16(*c); } } }
}

/// Round to the nearest representable f16 (ties to even), returned as f32.
pub fn round_f16(x: f32) -> f32 {
    if !x.is_finite() { return x; }
    let a = x.abs();
    if a >= 65520.0 { return f32::INFINITY.copysign(x); }
    if a < 6.103515625e-5 {
        // subnormal f16: multiples of 2^-24
        let q = (a * 16777216.0).round_ties_even() / 16777216.0;
        return q.copysign(x);
    }
    let e = a.log2().floor();
    let ulp = (2.0f32).powf(e - 10.0);
    ((a / ulp).round_ties_even() * ulp).copysign(x)
}

pub fn sat(x: f32) -> f32 { if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) } }

// ------------------------------------------------------------------------------------------------------------------------------
// Gaussian taps (ASSUMED host maths of FRCPassPostProcessWeightedSampleSum, see README "Recalled, not recovered")
// ------------------------------------------------------------------------------------------------------------------------------

pub const MAX_FILTER_SAMPLES: usize = 32;

/// `(offset in texels, weight)` bilinear tap pairs of a 1-D Gaussian of the given kernel radius (texels of the filtered image).
/// Radius is the 3-sigma extent; samples are taken two texels at a time with the offset placed between them (weights merged).
pub fn gaussian_taps(radius: f32) -> Vec<[f32; 2]> {
    let r = radius.clamp(1e-3, (MAX_FILTER_SAMPLES - 1) as f32);
    let n = (r.ceil() as i32).min(MAX_FILTER_SAMPLES as i32 - 1);
    let sigma = r / 3.0;
    let g = |x: f32| (-0.5 * (x / sigma) * (x / sigma)).exp();
    let mut taps = Vec::new();
    let mut sum = 0.0;
    let mut i = -n;
    while i <= n {
        let w0 = g(i as f32);
        let w1 = if i != n { g((i + 1) as f32) } else { 0.0 };
        let tw = w0 + w1;
        if tw > 0.0 {
            taps.push([i as f32 + w1 / tw, tw]);
            sum += tw;
        }
        i += 2;
    }
    for t in &mut taps { t[1] /= sum; }
    taps
}

/// One separable pass: `out = sum w * sample(src, uv + off * dir) * tint (+ prev)`; `dir` is in texels of `src` (target and source have the same size).
pub fn blur_pass(src: &Image, taps: &[[f32; 2]], dir: [f32; 2], tint: [f32; 4], prev: Option<&Image>) -> Image {
    Image::from_fn(src.w, src.h, |x, y| {
        let u = (x as f32 + 0.5) / src.w as f32;
        let v = (y as f32 + 0.5) / src.h as f32;
        let mut acc = [0.0f32; 4];
        for t in taps {
            let s = src.sample(u + dir[0] * t[0] / src.w as f32, v + dir[1] * t[0] / src.h as f32);
            for c in 0..4 { acc[c] += s[c] * t[1]; }
        }
        for c in 0..4 { acc[c] *= tint[c]; }
        if let Some(p) = prev {
            let s = p.sample(u, v);
            for c in 0..4 { acc[c] += s[c]; }
        }
        acc
    })
}

// ------------------------------------------------------------------------------------------------------------------------------
// Depth of field (far layer)
// ------------------------------------------------------------------------------------------------------------------------------

/// Half-resolution size used by the setup/blur buffers.
pub fn half(n: usize) -> usize { n.div_ceil(2).max(1) }

pub struct DofUniforms {
    pub depth: DepthTransform,
    pub focal_plus_region: f32,
    pub far_transition: f32,
    pub sky_distance: f32,
    pub cm_per_unit: f32,
    pub rows: [[f32; 4]; 3],
}

impl DofUniforms {
    pub fn new(p: &PostFxParams) -> Self {
        DofUniforms {
            depth: p.depth,
            focal_plus_region: p.dof.focal_distance + p.dof.focal_region,
            far_transition: p.dof.far_transition.max(1.0e-4),
            sky_distance: if p.dof.sky_focus_distance > 0.0 { p.dof.sky_focus_distance } else { 1.0e30 },
            cm_per_unit: p.cm_per_unit,
            rows: p.view.screen_to_world_rows,
        }
    }
    fn lin_cm(&self, d: f32) -> f32 { self.depth.linearize(d) * self.cm_per_unit }
    fn far_coc(&self, lin: f32) -> f32 { sat((lin - self.focal_plus_region) / self.far_transition) }
}

/// FPostProcessDOFSetupPS10: half-res far layer (premultiplied colour, coverage in alpha) from full-res colour + depth (`.x` of `depth`).
pub fn dof_setup_far(depth: &Image, color: &Image, u: &DofUniforms) -> Image {
    let (ow, oh) = (half(color.w), half(color.h));
    Image::from_fn(ow, oh, |x, y| {
        let uv = [(x as f32 + 0.5) / ow as f32, (y as f32 + 0.5) / oh as f32];
        let scr = [uv[0] * 2.0 - 1.0, -(uv[1] * 2.0 - 1.0)];
        let mut ray = [0.0f32; 3];
        for i in 0..3 { ray[i] = scr[0] * u.rows[0][i] + scr[1] * u.rows[1][i] + u.rows[2][i]; }
        let s = sat(3.0 * ray[2] / (ray[0] * ray[0] + ray[1] * ray[1] + ray[2] * ray[2]).sqrt());
        let bx = (uv[0] * depth.w as f32 - 0.5).floor() as i32;
        let by = (uv[1] * depth.h as f32 - 0.5).floor() as i32;
        let d = [depth.load(bx, by + 1)[0], depth.load(bx + 1, by + 1)[0], depth.load(bx + 1, by)[0], depth.load(bx, by)[0]];
        let offs = [[-0.5, 0.5], [0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]];
        let (tx, ty) = (1.0 / color.w as f32, 1.0 / color.h as f32);
        let mut acc = [0.0f32; 4];
        for i in 0..4 {
            let lin = u.lin_cm(d[i]);
            let far = u.far_coc(lin);
            let w = if u.sky_distance < lin { far * (1.0 - s) } else { far };
            let c = color.sample(uv[0] + offs[i][0] * tx, uv[1] + offs[i][1] * ty);
            acc[0] += c[0] * w;
            acc[1] += c[1] * w;
            acc[2] += c[2] * w;
            acc[3] += w;
        }
        [acc[0] * 0.25, acc[1] * 0.25, acc[2] * 0.25, acc[3] * 0.25]
    })
}

/// FPostProcessDOFRecombinePS100.
pub fn dof_recombine_far(depth: &Image, color: &Image, far: &Image, u: &DofUniforms) -> Image {
    let (fw, fh) = (far.w as f32, far.h as f32);
    let rect = [0.5 / fw, 0.5 / fh, 1.0 - 0.5 / fw, 1.0 - 0.5 / fh];
    Image::from_fn(color.w, color.h, |x, y| {
        let uv = [(x as f32 + 0.5) / color.w as f32, (y as f32 + 0.5) / color.h as f32];
        let lin = u.lin_cm(depth.load(x as i32, y as i32)[0]);
        let col = color.sample(uv[0], uv[1]);
        let far_c = u.far_coc(lin);
        let sharp = (0.25 - far_c).max(0.0) * 4.0;
        let sharp = sharp * sharp;
        let fb = far.sample(uv[0].clamp(rect[0], rect[2]), uv[1].clamp(rect[1], rect[3]));
        let sa = fb[3].sqrt();
        let a = sa * (3.0 - 2.0 * sa.sqrt());
        let mut o = [0.0f32; 4];
        for i in 0..3 {
            let norm = (fb[i] + col[i] * 0.0001) / (fb[3] + 0.0001);
            let far_mix = col[i] + a * (norm - col[i]);
            o[i] = far_mix + sharp * (col[i] - far_mix);
        }
        o
    })
}

/// Whole far-only Gaussian DOF as run on the GPU: setup -> H blur -> V blur -> recombine (blur radius law ASSUMED, see README).
pub fn dof_far_full(depth: &Image, color: &Image, p: &PostFxParams, quantize: bool) -> Image {
    let u = DofUniforms::new(p);
    let mut setup = dof_setup_far(depth, color, &u);
    if quantize { setup.quantize_f16(); }
    let radius = dof_blur_radius(p.dof.far_blur_size, setup.w);
    let taps = gaussian_taps(radius);
    let mut h = blur_pass(&setup, &taps, [1.0, 0.0], [1.0; 4], None);
    if quantize { h.quantize_f16(); }
    let mut v = blur_pass(&h, &taps, [0.0, 1.0], [1.0; 4], None);
    if quantize { v.quantize_f16(); }
    dof_recombine_far(depth, color, &v, &u)
}

/// Kernel radius (texels of the half-res buffer) of a DOF blur of `size_percent` of the view width (ASSUMED: percent of the buffer width).
pub fn dof_blur_radius(size_percent: f32, buffer_width: usize) -> f32 { size_percent * 0.01 * buffer_width as f32 }

// ------------------------------------------------------------------------------------------------------------------------------
// Motion blur
// ------------------------------------------------------------------------------------------------------------------------------

pub const TILE: usize = 16;

pub struct MbUniforms {
    pub width: usize,
    pub height: usize,
    /// H / W
    pub aspect: f32,
    /// pixels per W-normalised NDC unit and per side (amount * 0.5 * W/2)
    pub scale: f32,
    /// max velocity per side (px)
    pub max_px: f32,
    pub edge: f32,
    pub depth: DepthTransform,
    pub cm_per_unit: f32,
    pub clip_to_prev: glam::Mat4,
}

impl MbUniforms {
    pub fn new(p: &PostFxParams) -> Self {
        let (w, h) = (p.view.width as f32, p.view.height as f32);
        let mb = &p.motion_blur;
        MbUniforms {
            width: p.view.width as usize,
            height: p.view.height as usize,
            aspect: h / w,
            scale: mb.amount * 0.5 * (w * 0.5),
            max_px: mb.max_percent * 0.01 * w * 0.5,
            edge: 0.5 * w,
            depth: p.depth,
            cm_per_unit: p.cm_per_unit,
            clip_to_prev: p.view.clip_to_prev_clip,
        }
    }
    pub fn tiles(&self) -> (usize, usize) { (self.width.div_ceil(TILE), self.height.div_ceil(TILE)) }
}

fn atan2_fast(y: f32, x: f32) -> f32 {
    let (ax, ay) = (x.abs(), y.abs());
    let (mx, mn) = (ax.max(ay), ax.min(ay));
    if mx == 0.0 { return 0.0; }
    let a = mn / mx;
    let a2 = a * a;
    let p = (a2 * 0.087293 - 0.301895) * a2 + 1.0;
    let mut r = a * p;
    if ax < ay { r = 1.570796 - p * a; }
    if x < 0.0 { r = 3.141593 - r; }
    if y < 0.0 { r = -r; }
    r
}

/// Output of velocity flatten: per-pixel `(len, angle/2pi+0.5, depth_cm, depth_cm)` and per-tile `(min vec.xy, max vec.zw)`.
pub struct Flattened {
    pub flat: Image,
    pub tiles: Image,
}

/// `velocity` rgba: xy decoded NDC velocity, a = written flag (None = no dynamic velocity).
pub fn velocity_flatten(velocity: Option<&Image>, depth: &Image, u: &MbUniforms) -> Flattened {
    let (w, h) = (u.width, u.height);
    let (tw, th) = u.tiles();
    let mut flat = Image::new(w, h);
    let mut tiles = Image::new(tw, th);
    for ty in 0..th {
        for tx in 0..tw {
            let mut g0 = [[0.0f32; 4]; 256];
            for ly in 0..TILE {
                for lx in 0..TILE {
                    let (x, y) = (tx * TILE + lx, ty * TILE + ly);
                    let li = ly * TILE + lx;
                    let inside = x < w && y < h;
                    let dev_z = depth.load(x as i32, y as i32)[0];
                    let venc = velocity.map(|v| v.load(x as i32, y as i32)).unwrap_or([0.0; 4]);
                    let written = venc[3] > 0.5;
                    let uv = [(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32];
                    let cur = glam::Vec4::new(uv[0] * 2.0 - 1.0, 1.0 - uv[1] * 2.0, dev_z, 1.0);
                    let prev = u.clip_to_prev * cur;
                    let vcam = if prev.w.abs() > 1e-6 { [cur.x - prev.x / prev.w, cur.y - prev.y / prev.w] } else { [0.0, 0.0] };
                    let v = if written { [venc[0], venc[1]] } else { vcam };
                    let vy = v[1] * -u.aspect;
                    let len = (v[0] * v[0] + vy * vy).sqrt();
                    let ang = atan2_fast(vy, v[0]);
                    let lin = u.depth.linearize(dev_z) * u.cm_per_unit;
                    if inside { flat.px[y * w + x] = [len, ang * 0.159155 + 0.5, lin, lin]; }
                    let lc = (u.max_px / u.scale).min(len);
                    g0[li] = [if inside { lc } else { 2.0 }, ang, if inside { lc } else { 0.0 }, ang];
                }
            }
            let mut s = 128;
            while s >= 1 {
                let mut next = g0;
                for li in 0..s {
                    let (a, b) = (g0[li], g0[li + s]);
                    let mn = if a[0] < b[0] { [a[0], a[1]] } else { [b[0], b[1]] };
                    let mx = if b[2] < a[2] { [a[2], a[3]] } else { [b[2], b[3]] };
                    next[li] = [mn[0], mn[1], mx[0], mx[1]];
                }
                g0 = next;
                s >>= 1;
            }
            let r = g0[0];
            tiles.px[ty * tw + tx] = [r[0] * r[1].cos(), r[0] * r[1].sin(), r[2] * r[3].cos(), r[2] * r[3].sin()];
        }
    }
    Flattened { flat, tiles }
}

/// FPostProcessVelocityGatherCS: 7x7 dilation of the tile min/max velocities.
pub fn velocity_gather(tiles: &Image, u: &MbUniforms) -> Image {
    let tile_scale = u.scale / 16.0;
    let (gw, gh) = (tiles.w as i32, tiles.h as i32);
    Image::from_fn(tiles.w, tiles.h, |x, y| {
        let (x, y) = (x as i32, y as i32);
        let mut acc = tiles.load(x, y);
        for ox in -3i32..=3 {
            for oy in -3i32..=3 {
                if ox == 0 && oy == 0 { continue; }
                let (nx, ny) = (x + ox, y + oy);
                if nx < 0 || ny < 0 || nx >= gw || ny >= gh { continue; }
                let n = tiles.load(nx, ny);
                let v = [n[2] * tile_scale, n[3] * tile_scale];
                let d2 = v[0] * v[0] + v[1] * v[1];
                let yinv = f32::from_bits((0x5f3759dfi32 - ((d2.to_bits() as i32) >> 1)) as u32);
                let nrm = [v[0] * yinv, v[1] * yinv];
                let l1 = nrm[1].abs() + nrm[0].abs();
                let thr = [l1 * 0.99 + yinv * d2, l1 * 0.99];
                let d = [ox as f32, oy as f32];
                let along = nrm[0] * d[0] + nrm[1] * d[1];
                let perp = -nrm[1] * d[0] + nrm[0] * d[1];
                let reach = along.abs() < thr[0] && perp.abs() < thr[1];
                let dot = |a: [f32; 2]| a[0] * a[0] + a[1] * a[1];
                let mn = if dot([acc[0], acc[1]]) < dot([n[0], n[1]]) { [acc[0], acc[1]] } else { [n[0], n[1]] };
                let mx = if dot([n[2], n[3]]) < dot([acc[2], acc[3]]) { [n[2], n[3]] } else { [acc[2], acc[3]] };
                if reach { acc = [mn[0], mn[1], mx[0], mx[1]]; }
            }
        }
        acc
    })
}

/// FPostProcessMotionBlurPS<4> for pixel `(px, py)` (integer pixel, `in.pos` = centre).
pub fn motion_blur_pixel(color: &Image, flat: &Image, tiles: &Image, u: &MbUniforms, px: usize, py: usize) -> [f32; 3] {
    let fp = [px as f32, py as f32];
    let ign = |p: [f32; 2]| {
        let d = p[0] * 0.06711056 + p[1] * 0.00583715;
        let f = d - d.floor();
        let m = f * 52.9829189;
        m - m.floor()
    };
    let n = ign(fp);
    let n2 = ign([fp[0] + 32.665, fp[1] + 11.815]);
    let jit = [(n - 0.5) * 0.5, (n2 - 0.5) * 0.5];
    let rel = [px as f32 + 0.5, py as f32 + 0.5];
    let mut tc = [rel[0] * 0.0625 + jit[0], rel[1] * 0.0625 + jit[1]];
    tc[0] = tc[0].min(u.width as f32);
    tc[1] = tc[1].min(u.height as f32);
    let tl = tiles.point(tc[0] / tiles.w as f32, tc[1] / tiles.h as f32);
    let scale = u.scale;
    let uv = [(px as f32 + 0.5) / u.width as f32, (py as f32 + 0.5) / u.height as f32];
    let scr = [uv[0] * 2.0 - 1.0, -(uv[1] * 2.0 - 1.0)];
    let vmax = [tl[2] * scale, tl[3] * scale];
    let m = vmax[0] * vmax[0] + vmax[1] * vmax[1];
    let c0s = color.sample(uv[0], uv[1]);
    let c0 = [c0s[0], c0s[1], c0s[2]];
    if m < 0.25 { return c0; }
    let vmin = [tl[0] * scale, tl[1] * scale];
    let uniform_motion = m * 0.4 < vmin[0] * vmin[0] + vmin[1] * vmin[1];
    let vc = [tl[2] * scale + 0.00001, tl[3] * scale + 0.00001];
    let rc = [1.0 / vc[0], 1.0 / vc[1]];
    let e = u.edge;
    let tp = [rc[0] * e - rc[0] * e * scr[0], rc[1] * e - rc[1] * e * scr[1]];
    let tn = [-(rc[0] * e) - rc[0] * e * scr[0], -(rc[1] * e) - rc[1] * e * scr[1]];
    let fwd = [tp[0].max(tn[0]), tp[1].max(tn[1])];
    let bwd = [(-tp[0]).max(-tn[0]), (-tp[1]).max(-tn[1])];
    let cl_pos = sat(fwd[1].min(fwd[0]));
    let cl_neg = sat(bwd[1].min(bwd[0]));
    let (iw, ih) = (1.0 / u.width as f32, 1.0 / u.height as f32);
    let vpos = [vmax[0] * cl_pos * iw, vmax[1] * cl_pos * ih];
    let vneg = [vmax[0] * -cl_neg * iw, vmax[1] * -cl_neg * ih];
    let at = |t: f32, v: [f32; 2]| [uv[0] + t * v[0], uv[1] + t * v[1]];
    if uniform_motion {
        let mut acc = [0.0f32; 3];
        for k in 0..8 {
            let a = (n + k as f32) * 0.125;
            let b = ((k + 1) as f32 - n) * 0.125;
            for (t, v) in [(a, vpos), (b, vneg)] {
                let p = at(t, v);
                let s = color.sample(p[0], p[1]);
                for i in 0..3 { acc[i] += s[i]; }
            }
        }
        return [acc[0] * 0.0625, acc[1] * 0.0625, acc[2] * 0.0625];
    }
    let kinv = 8.0 / m.sqrt();
    let fc = flat.point(uv[0], uv[1]);
    let lc = (fc[0] * scale).min(u.max_px);
    let zc = fc[2];
    let mut acc = [0.0f32; 4];
    for k in 0..8i32 {
        let uva = at((n + k as f32) * 0.125, vpos);
        let uvb = at(((k + 1) as f32 - n) * 0.125, vneg);
        let fa = flat.point(uva[0], uva[1]);
        let fb = flat.point(uvb[0], uvb[1]);
        let ca = color.sample(uva[0], uva[1]);
        let cb = color.sample(uvb[0], uvb[1]);
        let la = (fa[0] * scale).min(u.max_px);
        let lb = (fb[0] * scale).min(u.max_px);
        let off = if k == 0 { 0.0 } else { n + (k - 1) as f32 };
        let dza = fa[2] - zc;
        let dzb = fb[2] - zc;
        let aa = [sat(dza + 0.5), sat(0.5 - dza)];
        let ab = [sat(dzb + 0.5), sat(0.5 - dzb)];
        let cova = [sat(kinv * lc - off), sat(kinv * la - off)];
        let covb = [sat(kinv * lc - off), sat(kinv * lb - off)];
        let wa = aa[0] * cova[0] + aa[1] * cova[1];
        let wb = ab[0] * covb[0] + ab[1] * covb[1];
        let c1 = la < lb;
        let c2 = fb[2] < fa[2];
        let wa2 = if c1 && c2 { wb } else { wa };
        let wb2 = if c1 || c2 { wb } else { wa };
        for i in 0..3 { acc[i] += ca[i] * wa2 + cb[i] * wb2; }
        acc[3] += wa2 + wb2;
    }
    let keep = 1.0 - acc[3] * 0.0625;
    [acc[0] * 0.0625 + c0[0] * keep, acc[1] * 0.0625 + c0[1] * keep, acc[2] * 0.0625 + c0[2] * keep]
}

pub fn motion_blur_full(color: &Image, velocity: Option<&Image>, depth: &Image, u: &MbUniforms) -> Image {
    let fl = velocity_flatten(velocity, depth, u);
    let tiles = velocity_gather(&fl.tiles, u);
    Image::from_fn(u.width, u.height, |x, y| {
        let c = motion_blur_pixel(color, &fl.flat, &tiles, u, x, y);
        [c[0], c[1], c[2], 0.0]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaussian_taps_are_normalised_and_symmetric() {
        for r in [0.13f32, 1.0, 2.5, 7.9, 31.0, 80.0] {
            let t = gaussian_taps(r);
            let sum: f32 = t.iter().map(|x| x[1]).sum();
            assert!((sum - 1.0).abs() < 1e-5, "radius {r}: weights sum to {sum}");
            assert!(t.len() <= MAX_FILTER_SAMPLES);
            // mean offset of a symmetric kernel is ~0
            let mean: f32 = t.iter().map(|x| x[0] * x[1]).sum();
            assert!(mean.abs() < 0.51, "radius {r}: mean offset {mean}");
        }
        // sub-texel radii collapse to a single centre tap (the Athena Day DOF case)
        let t = gaussian_taps(0.128);
        assert_eq!(t.len(), 1);
        assert!(t[0][0].abs() < 1e-6 && (t[0][1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn f16_rounding_matches_hardware_conversion() {
        for &x in &[0.0f32, 1.0, 0.1, 3.14159, 65504.0, 1e-5, 6.1e-5, -2.5, 1234.5678] {
            let bits = crate::harness::f32_to_f16_bits(x);
            assert_eq!(round_f16(x), crate::harness::f16_bits_to_f32(bits), "x = {x}");
        }
    }

    #[test]
    fn bilinear_sampling_is_texel_centred() {
        let im = Image::from_fn(4, 1, |x, _| [x as f32, 0.0, 0.0, 1.0]);
        assert_eq!(im.sample(0.125, 0.5)[0], 0.0); // centre of texel 0
        assert!((im.sample(0.25, 0.5)[0] - 0.5).abs() < 1e-6); // between texel 0 and 1
        assert_eq!(im.sample(0.99, 0.5)[0], 3.0); // clamped
    }

    #[test]
    fn depth_transform_roundtrip() {
        let t = DepthTransform::from_perspective(0.1, 6000.0);
        for lin in [0.5f32, 2.0, 150.0, 5000.0] {
            let d = t.device_z_from_linear(lin);
            // f32 device depth near the far plane is coarse (1 - d ~ near / lin): 1e-3 up to 150 m, 1e-2 beyond
            let tol = if lin <= 150.0 { 1e-3 } else { 1e-2 };
            assert!((t.linearize(d) - lin).abs() / lin < tol, "{lin}");
        }
    }
}

// ------------------------------------------------------------------------------------------------------------------------------
// Bloom
// ------------------------------------------------------------------------------------------------------------------------------

/// FPostProcessBloomSetupPS on a half-res target.
pub fn bloom_setup(scene: &Image, exposure: f32, threshold: f32) -> Image {
    let (ow, oh) = (half(scene.w), half(scene.h));
    Image::from_fn(ow, oh, |x, y| {
        let uv = [(x as f32 + 0.5) / ow as f32, (y as f32 + 0.5) / oh as f32];
        let s = scene.sample(uv[0], uv[1]);
        let c = [s[0].min(65536.0), s[1].min(65536.0), s[2].min(65536.0)];
        let luma = c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11;
        let a = sat((luma * exposure - threshold) * 0.5);
        [c[0] * a, c[1] * a, c[2] * a, 0.0]
    })
}

/// FPostProcessDownsamplePS1 (4x4 box via four bilinear taps one source texel off-centre).
pub fn downsample(src: &Image) -> Image {
    let (ow, oh) = (half(src.w), half(src.h));
    let (ox, oy) = (1.0 / src.w as f32, 1.0 / src.h as f32);
    Image::from_fn(ow, oh, |x, y| {
        let uv = [(x as f32 + 0.5) / ow as f32, (y as f32 + 0.5) / oh as f32];
        let a = src.sample(uv[0] - ox, uv[1] - oy);
        let b = src.sample(uv[0] + ox, uv[1] - oy);
        let c = src.sample(uv[0] - ox, uv[1] + oy);
        let d = src.sample(uv[0] + ox, uv[1] + oy);
        let mut o = [0.0f32; 4];
        for i in 0..4 { o[i] = (a[i] + b[i] + c[i] + d[i]) * 0.25; }
        for i in 0..3 { o[i] = o[i].max(0.0); }
        o
    })
}

/// Blur radius (texels) of bloom level `level` (0 = first blurred level, half-res): `size% * 0.01 * level width * size_scale` (ASSUMED law).
pub fn bloom_level_radius(b: &BloomParams, level: usize, level_width: usize) -> f32 { b.sizes[level] * 0.01 * level_width as f32 * b.size_scale }

/// Whole bloom chain: setup (half) -> downsample to level k -> Gaussian H+V with tint -> accumulate from the smallest level up.
/// Returns the half-res bloom image (RGB already includes the tints; the tonemapper multiplies by `BloomIntensity`).
pub fn bloom_chain(scene: &Image, p: &PostFxParams, quantize: bool) -> Image {
    let b = &p.bloom;
    let mut levels: Vec<Image> = Vec::new();
    let mut cur = bloom_setup(scene, p.exposure, b.threshold);
    if quantize { cur.quantize_f16(); }
    levels.push(cur.clone());
    for _ in 1..6 {
        cur = downsample(&cur);
        if quantize { cur.quantize_f16(); }
        levels.push(cur.clone());
    }
    let mut acc: Option<Image> = None;
    for k in (0..6).rev() {
        let tint = b.tints[k];
        if tint[0] == 0.0 && tint[1] == 0.0 && tint[2] == 0.0 || b.sizes[k] <= 0.0 { continue; }
        let src = &levels[k];
        let taps = gaussian_taps(bloom_level_radius(b, k, src.w));
        let mut h = blur_pass(src, &taps, [1.0, 0.0], [1.0; 4], None);
        if quantize { h.quantize_f16(); }
        let prev_up = acc.as_ref().map(|a| Image::from_fn(src.w, src.h, |x, y| a.sample((x as f32 + 0.5) / src.w as f32, (y as f32 + 0.5) / src.h as f32)));
        let mut v = blur_pass(&h, &taps, [0.0, 1.0], [tint[0], tint[1], tint[2], 1.0], prev_up.as_ref());
        if quantize { v.quantize_f16(); }
        acc = Some(v);
    }
    // bring to level 0 size
    match acc {
        Some(a) => Image::from_fn(levels[0].w, levels[0].h, |x, y| a.sample((x as f32 + 0.5) / levels[0].w as f32, (y as f32 + 0.5) / levels[0].h as f32)),
        None => Image::new(levels[0].w, levels[0].h),
    }
}
