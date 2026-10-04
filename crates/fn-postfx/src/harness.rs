//! Headless wgpu helpers shared by the tests and the `postfx-demo` binary: device creation, texture upload / read-back of
//! [`cpu::Image`]s, depth-buffer upload, PNG output.
use crate::cpu::Image;
use std::sync::Arc;

pub struct Headless {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Headless {
    /// Vulkan or DX12 device (env `FN_BACKEND`/`WGPU_BACKEND` = vulkan | dx12 restricts it); `None` if no adapter.
    pub fn new() -> Option<Headless> {
        let v = std::env::var("FN_BACKEND").or_else(|_| std::env::var("WGPU_BACKEND")).unwrap_or_default().to_ascii_lowercase();
        let backends = match v.as_str() {
            "vulkan" | "vk" => wgpu::Backends::VULKAN,
            "dx12" | "d3d12" => wgpu::Backends::DX12,
            _ => wgpu::Backends::VULKAN | wgpu::Backends::DX12,
        };
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends, ..wgpu::InstanceDescriptor::new_without_display_handle() });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, force_fallback_adapter: false, compatible_surface: None, apply_limit_buckets: false })).ok()?;
        let mut features = wgpu::Features::empty();
        if adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) { features |= wgpu::Features::TIMESTAMP_QUERY; }
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("postfx headless"), required_features: features, required_limits: wgpu::Limits::default(), ..Default::default() })).ok()?;
        device.on_uncaptured_error(Arc::new(|e: wgpu::Error| panic!("wgpu error: {e}")));
        Some(Headless { instance, adapter, device, queue })
    }
    pub fn describe(&self) -> String {
        let i = self.adapter.get_info();
        format!("{} ({:?})", i.name, i.backend)
    }
}

/// f32 -> IEEE half bits (round to nearest even).
pub fn f32_to_f16_bits(f: f32) -> u16 {
    let x = f.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32;
    let man = x & 0x007f_ffff;
    if exp == 0xff { return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 }; }
    let e = exp - 127 + 15;
    if e >= 0x1f { return sign | 0x7c00; }
    if e <= 0 {
        if e < -10 { return sign; }
        let m = man | 0x0080_0000;
        let shift = (14 - e) as u32;
        let mut h = (m >> shift) as u16;
        let rem = m & ((1 << shift) - 1);
        let half = 1 << (shift - 1);
        if rem > half || (rem == half && (h & 1) == 1) { h += 1; }
        return sign | h;
    }
    let mut h = sign | ((e as u16) << 10) | ((man >> 13) as u16);
    let rem = man & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (h & 1) == 1) { h += 1; }
    h
}

pub fn f16_bits_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if man == 0 { sign } else {
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 { m <<= 1; e -= 1; }
            sign | ((e as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 0x1f { sign | 0x7f80_0000 | (man << 13) } else { sign | ((exp + 127 - 15) << 23) | (man << 13) };
    f32::from_bits(bits)
}

pub struct GpuTex {
    pub tex: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub w: u32,
    pub h: u32,
}

const COPY_ALIGN: u32 = 256;

/// Uploads an `Rgba16Float` texture (`TEXTURE_BINDING | COPY_SRC | RENDER_ATTACHMENT`).
pub fn upload_rgba16f(h: &Headless, img: &Image, label: &str) -> GpuTex {
    let (w, ht) = (img.w as u32, img.h as u32);
    let tex = h.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: ht, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let mut data: Vec<u16> = Vec::with_capacity(img.px.len() * 4);
    for p in &img.px { for c in p { data.push(f32_to_f16_bits(*c)); } }
    h.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 8), rows_per_image: Some(ht) },
        wgpu::Extent3d { width: w, height: ht, depth_or_array_layers: 1 },
    );
    let view = tex.create_view(&Default::default());
    GpuTex { tex, view, w, h: ht }
}

/// Empty `Rgba16Float` render target of the given size.
pub fn render_target(h: &Headless, w: u32, ht: u32, label: &str) -> GpuTex {
    let tex = h.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: ht, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    GpuTex { tex, view, w, h: ht }
}

/// Reads back any 4-channel float texture (`Rgba16Float` or `Rgba32Float`).
pub fn read_image(h: &Headless, tex: &wgpu::Texture, w: u32, ht: u32) -> Image {
    let f32_fmt = tex.format() == wgpu::TextureFormat::Rgba32Float;
    let bpp = if f32_fmt { 16 } else { 8 };
    let bpr = (w * bpp).div_ceil(COPY_ALIGN) * COPY_ALIGN;
    let buf = h.device.create_buffer(&wgpu::BufferDescriptor { label: Some("postfx readback"), size: (bpr * ht) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    let mut enc = h.device.create_command_encoder(&Default::default());
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo { texture: tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(bpr), rows_per_image: Some(ht) } },
        wgpu::Extent3d { width: w, height: ht, depth_or_array_layers: 1 },
    );
    h.queue.submit([enc.finish()]);
    let slice = buf.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
    h.device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
    rx.recv().expect("map cb").expect("map");
    let data = slice.get_mapped_range().expect("range");
    let mut img = Image::new(w as usize, ht as usize);
    for y in 0..ht as usize {
        let row = &data[y * bpr as usize..];
        for x in 0..w as usize {
            let mut px = [0.0f32; 4];
            for c in 0..4 {
                px[c] = if f32_fmt {
                    f32::from_le_bytes(row[(x * 4 + c) * 4..(x * 4 + c) * 4 + 4].try_into().unwrap())
                } else {
                    f16_bits_to_f32(u16::from_le_bytes([row[(x * 4 + c) * 2], row[(x * 4 + c) * 2 + 1]]))
                };
            }
            img.px[y * w as usize + x] = px;
        }
    }
    drop(data);
    buf.unmap();
    img
}

/// Builds a `Depth32Float` texture holding `depth.x` (device depth) by drawing a full-screen triangle with `frag_depth`.
pub fn upload_depth(h: &Headless, depth: &Image, label: &str) -> GpuTex {
    let (w, ht) = (depth.w as u32, depth.h as u32);
    let src = h.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth source"),
        size: wgpu::Extent3d { width: w, height: ht, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let data: Vec<f32> = depth.px.iter().map(|p| p[0]).collect();
    h.queue.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &src, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        bytemuck::cast_slice(&data),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(ht) },
        wgpu::Extent3d { width: w, height: ht, depth_or_array_layers: 1 },
    );
    let dtex = h.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: ht, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let sm = h.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("depth fill"), source: wgpu::ShaderSource::Wgsl(r#"
@group(0) @binding(0) var src: texture_2d<f32>;
struct V { @builtin(position) p: vec4<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    var o: V; let x = f32(i32(i & 1u) * 4 - 1); let y = f32(i32((i >> 1u) & 1u) * 4 - 1); o.p = vec4<f32>(x, y, 0.0, 1.0); return o; }
struct F { @builtin(frag_depth) d: f32 }
@fragment fn fs(v: V) -> F { var o: F; o.d = textureLoad(src, vec2<i32>(v.p.xy), 0).x; return o; }
"#.into()) });
    let bgl = h.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }] });
    let layout = h.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
    let pipe = h.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("depth fill"),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &sm, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &sm, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[] }),
        multiview_mask: None,
        cache: None,
    });
    let sv = src.create_view(&Default::default());
    let bg = h.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sv) }] });
    let dv = dtex.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
    let mut enc = h.device.create_command_encoder(&Default::default());
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("depth fill"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &dv, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..3, 0..1);
    }
    h.queue.submit([enc.finish()]);
    GpuTex { tex: dtex, view: dv, w, h: ht }
}

/// Max and mean absolute difference over RGB, normalised by `max(1, |reference|)` (i.e. display-referred for values up to 1 and relative above).
pub fn compare_rgb(a: &Image, reference: &Image) -> (f32, f32) {
    assert_eq!((a.w, a.h), (reference.w, reference.h));
    let (mut mx, mut sum) = (0.0f32, 0.0f64);
    for (p, q) in a.px.iter().zip(&reference.px) {
        for c in 0..3 {
            let e = (p[c] - q[c]).abs() / q[c].abs().max(1.0);
            mx = mx.max(e);
            sum += e as f64;
        }
    }
    (mx, (sum / (a.px.len() * 3) as f64) as f32)
}

/// Writes RGB of an HDR image as an sRGB PNG after `scale` and clamp (simple gamma 2.2-ish sRGB encode).
pub fn save_png(img: &Image, path: &std::path::Path, scale: f32) {
    let enc = |v: f32| {
        let c = (v * scale).clamp(0.0, 1.0);
        let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5) as u8
    };
    let mut buf = Vec::with_capacity(img.px.len() * 3);
    for p in &img.px { buf.extend_from_slice(&[enc(p[0]), enc(p[1]), enc(p[2])]); }
    image::save_buffer(path, &buf, img.w as u32, img.h as u32, image::ColorType::Rgb8).expect("save png");
}
