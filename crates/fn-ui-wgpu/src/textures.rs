//! GPU texture cache: premultiplied quad textures (with CPU-built mip chains), straight "material input" textures, baked material textures.
use fn_ui::assets::{Filter, Texture};
use std::collections::HashMap;

/// A GPU texture holding premultiplied RGBA8 that quads sample (`Rgba8Unorm`, no sRGB conversion anywhere).
pub struct GpuTex {
    pub texture: wgpu::Texture,
    pub bind_group: wgpu::BindGroup,
    pub w: u32,
    pub h: u32,
    pub mips: u32,
    pub version: u32,
    /// sampler hint the bind group was built for (filter nearest, wrap)
    pub key: (bool, bool, bool),
}

/// Straight RGBA8 texture sampled by material shaders with `textureLoad` (no filtering hardware involved).
pub struct StraightTex {
    pub _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub version: u32,
}

/// Box-filtered mip chain of premultiplied RGBA8 (levels 1..). Premultiplied-space averaging so edges never halo.
pub fn build_mips(w: u32, h: u32, base: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    let (mut cw, mut ch) = (w, h);
    let mut prev: Vec<u8> = base.to_vec();
    while cw > 1 || ch > 1 {
        let (nw, nh) = ((cw / 2).max(1), (ch / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let xs = [(x * 2).min(cw - 1), (x * 2 + 1).min(cw - 1)];
                let ys = [(y * 2).min(ch - 1), (y * 2 + 1).min(ch - 1)];
                for c in 0..4 {
                    let mut s = 0u32;
                    for &yy in &ys { for &xx in &xs { s += prev[((yy * cw + xx) * 4 + c) as usize] as u32; } }
                    next[((y * nw + x) * 4 + c) as usize] = ((s + 2) / 4) as u8;
                }
            }
        }
        out.push((nw, nh, next.clone()));
        prev = next; cw = nw; ch = nh;
    }
    out
}

pub fn write_level(queue: &wgpu::Queue, tex: &wgpu::Texture, level: u32, w: u32, h: u32, data: &[u8]) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        data,
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
}

pub fn create_rgba8(device: &wgpu::Device, label: &str, w: u32, h: u32, mips: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: mips, sample_count: 1, dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

pub struct Samplers { s: [wgpu::Sampler; 4] }
impl Samplers {
    pub fn new(device: &wgpu::Device) -> Self {
        let mk = |nearest: bool, repeat: bool| {
            let f = if nearest { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear };
            let a = if repeat { wgpu::AddressMode::Repeat } else { wgpu::AddressMode::ClampToEdge };
            device.create_sampler(&wgpu::SamplerDescriptor { label: Some("ui sampler"), address_mode_u: a, address_mode_v: a, address_mode_w: a, mag_filter: f, min_filter: f, mipmap_filter: if nearest { wgpu::MipmapFilterMode::Nearest } else { wgpu::MipmapFilterMode::Linear }, ..Default::default() })
        };
        Samplers { s: [mk(false, false), mk(false, true), mk(true, false), mk(true, true)] }
    }
    pub fn get(&self, nearest: bool, repeat: bool) -> &wgpu::Sampler { &self.s[(nearest as usize) * 2 + repeat as usize] }
}

pub struct TexUploader<'a> { pub device: &'a wgpu::Device, pub queue: &'a wgpu::Queue, pub bgl: &'a wgpu::BindGroupLayout, pub samplers: &'a Samplers, pub max_dim: u32 }

impl TexUploader<'_> {
    fn bind_group(&self, tex: &wgpu::Texture, nearest: bool, repeat: bool) -> wgpu::BindGroup {
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("ui tex bg"), layout: self.bgl, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(self.samplers.get(nearest, repeat)) },
        ] })
    }

    /// Premultiplied pixels (already `Texture::premultiplied`) -> GPU, with a mip chain when `mips`.
    pub fn upload_premult(&self, existing: Option<GpuTex>, w: u32, h: u32, premult: &[u8], mips: bool, nearest: bool, repeat: bool, version: u32, label: &str) -> Option<GpuTex> {
        if w == 0 || h == 0 || w > self.max_dim || h > self.max_dim { eprintln!("fn-ui-wgpu: texture {label} {w}x{h} unsupported (max {})", self.max_dim); return None; }
        let levels = if mips { 32 - w.max(h).leading_zeros() } else { 1 };
        let chain = if mips { build_mips(w, h, premult) } else { Vec::new() };
        let key = (nearest, repeat, mips);
        let mut g = match existing {
            Some(g) if g.w == w && g.h == h && g.mips == levels => { let mut g = g; if g.key != key { g.bind_group = self.bind_group(&g.texture, nearest, repeat); g.key = key; } g }
            _ => { let texture = create_rgba8(self.device, label, w, h, levels); let bind_group = self.bind_group(&texture, nearest, repeat); GpuTex { texture, bind_group, w, h, mips: levels, version, key } }
        };
        write_level(self.queue, &g.texture, 0, w, h, premult);
        for (i, (mw, mh, d)) in chain.iter().enumerate() { write_level(self.queue, &g.texture, i as u32 + 1, *mw, *mh, d); }
        g.version = version;
        Some(g)
    }
}

pub fn sampler_flags(t: &Texture) -> (bool, bool, bool) { (t.sampler.filter == Filter::Nearest, t.sampler.wrap, t.sampler.mips) }

/// Keyed by TextureId.0
pub type QuadTexMap = HashMap<u32, GpuTex>;
