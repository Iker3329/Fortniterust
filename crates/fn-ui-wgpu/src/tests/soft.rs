//! Software reference renderer for quads: premultiplied ONE/ONE_MINUS_SRC_ALPHA over an 8-bit framebuffer (quantised after every blend like the
//! GPU target), nearest/bilinear clamp sampling, parallelogram quads, floor/ceil clip rects intersected and clamped to the target.
use fn_ui::assets::{Filter, UiAssets};
use fn_ui::draw::{Cmd, RenderList, TextureId};

fn sample(px: &[u8], w: u32, h: u32, u: f32, v: f32, nearest: bool, repeat: bool) -> [f32; 4] {
    let at = |x: i64, y: i64| -> [f32; 4] {
        let (x, y) = if repeat { (x.rem_euclid(w as i64), y.rem_euclid(h as i64)) } else { (x.clamp(0, w as i64 - 1), y.clamp(0, h as i64 - 1)) };
        let o = ((y as u32 * w + x as u32) * 4) as usize;
        [px[o] as f32 / 255.0, px[o + 1] as f32 / 255.0, px[o + 2] as f32 / 255.0, px[o + 3] as f32 / 255.0]
    };
    let (x, y) = (u * w as f32, v * h as f32);
    if nearest { return at(x.floor() as i64, y.floor() as i64); }
    let (x, y) = (x - 0.5, y - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (a, b, c, d) = (at(x0 as i64, y0 as i64), at(x0 as i64 + 1, y0 as i64), at(x0 as i64, y0 as i64 + 1), at(x0 as i64 + 1, y0 as i64 + 1));
    let mut o = [0.0; 4];
    for k in 0..4 { o[k] = (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy; }
    o
}

/// Premultiplied RGBA8 of quads/clips only (materials are ignored here).
pub fn soft_render(list: &RenderList, assets: &UiAssets, w: u32, h: u32, clear: Option<[f32; 4]>) -> Vec<u8> {
    let mut fb = vec![0u8; (w * h * 4) as usize];
    if let Some(c) = clear {
        let p = [c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]];
        for px in fb.chunks_exact_mut(4) { for k in 0..4 { px[k] = (p[k] * 255.0 + 0.5) as u8; } }
    }
    let (wi, hi) = (w as i32, h as i32);
    let full = [0i32, 0, wi, hi];
    let mut clips: Vec<[i32; 4]> = Vec::new();
    for cmd in &list.cmds {
        match cmd {
            Cmd::PushClip(r) => {
                let p = clips.last().copied().unwrap_or(full);
                let n = [r.x.floor() as i32, r.y.floor() as i32, (r.x + r.w).ceil() as i32, (r.y + r.h).ceil() as i32];
                clips.push([n[0].max(p[0]).max(0), n[1].max(p[1]).max(0), n[2].min(p[2]).min(wi), n[3].min(p[3]).min(hi)]);
            }
            Cmd::PopClip => { clips.pop(); }
            Cmd::Material(_) => {}
            Cmd::Quad(q) => {
                if q.tex == TextureId::NONE { continue; }
                let Some(t) = assets.store.get(q.tex) else { continue };
                let pm = t.premultiplied();
                let nearest = t.sampler.filter == Filter::Nearest;
                let clip = clips.last().copied().unwrap_or(full);
                let (p0, p1, p3) = (q.pos[0], q.pos[1], q.pos[3]);
                let d1 = [p1[0] - p0[0], p1[1] - p0[1]];
                let d3 = [p3[0] - p0[0], p3[1] - p0[1]];
                let det = d1[0] * d3[1] - d1[1] * d3[0];
                if det.abs() < 1e-9 { continue; }
                let xs = q.pos.map(|p| p[0]); let ys = q.pos.map(|p| p[1]);
                let bx0 = (xs.iter().cloned().fold(f32::MAX, f32::min).floor() as i32).max(clip[0]);
                let bx1 = (xs.iter().cloned().fold(f32::MIN, f32::max).ceil() as i32).min(clip[2]);
                let by0 = (ys.iter().cloned().fold(f32::MAX, f32::min).floor() as i32).max(clip[1]);
                let by1 = (ys.iter().cloned().fold(f32::MIN, f32::max).ceil() as i32).min(clip[3]);
                let tint = [q.tint[0] * q.tint[3], q.tint[1] * q.tint[3], q.tint[2] * q.tint[3], q.tint[3]];
                for y in by0..by1 {
                    for x in bx0..bx1 {
                        let c = [x as f32 + 0.5 - p0[0], y as f32 + 0.5 - p0[1]];
                        let s = (c[0] * d3[1] - c[1] * d3[0]) / det;
                        let tt = (d1[0] * c[1] - d1[1] * c[0]) / det;
                        if !(0.0..1.0).contains(&s) || !(0.0..1.0).contains(&tt) { continue; }
                        let (u, v) = (q.uv[0] + s * (q.uv[2] - q.uv[0]), q.uv[1] + tt * (q.uv[3] - q.uv[1]));
                        let tx = sample(&pm, t.w, t.h, u, v, nearest, t.sampler.wrap);
                        let src = [tx[0] * tint[0], tx[1] * tint[1], tx[2] * tint[2], tx[3] * tint[3]];
                        let o = ((y as u32 * w + x as u32) * 4) as usize;
                        for k in 0..4 {
                            let d = fb[o + k] as f32 / 255.0;
                            fb[o + k] = ((src[k] + d * (1.0 - src[3])).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                        }
                    }
                }
            }
        }
    }
    fb
}
