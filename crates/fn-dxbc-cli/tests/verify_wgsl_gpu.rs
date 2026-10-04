//! (3c) the generated WGSL executed by wgpu (hardware adapter, WARP / lavapipe fallback) against the CPU interpreter.
//!
//! Every pixel shader of the old corpus that needs nothing exotic (no UAVs, buffers, builtin inputs, comparison samplers, texel loads) is translated to WGSL, its
//! `<name>_main` fragment entry is rendered into a 1x1 `Rgba32Float` target with random constant (flat) inputs, constant buffers filled with
//! random data and 1x1 constant-colour textures; the pixel is read back and compared with `fn_dxbc::interp` running the same program.
//! GPU transcendental functions are not bit-identical to the CPU's, so the tolerance is 1e-3 relative; the report also counts how many
//! samples agree to 1e-5.
mod common;
use common::*;
use fn_dxbc::codegen::translate_wgsl;
use fn_dxbc::interp::{Env, Machine, TexOp, TexReq};
use fn_dxbc::{Opcode, OperandType, ProgramType, Shader};
use std::collections::BTreeSet;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; (self.0 >> 16) as u32 }
    fn unit(&mut self) -> f32 { (self.next() & 0xffffff) as f32 / 16777216.0 }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.unit() }
    fn val(&mut self) -> f32 { match self.next() % 8 { 0 => 0.0, 1 => 1.0, 2 => self.range(-1.0, 0.0), 3 => self.range(0.0, 4.0), _ => self.range(0.0, 1.0) } }
}

fn tex_color(slot: u32) -> [f32; 4] { [0.3 + 0.1 * slot as f32, 0.6, 0.2 + 0.05 * slot as f32, 0.9] }
struct ConstTex;
impl Env for ConstTex {
    fn texture(&mut self, r: &TexReq) -> [f32; 4] { match r.op { TexOp::Sample | TexOp::SampleB | TexOp::SampleL | TexOp::SampleD => tex_color(r.texture), _ => [0.0; 4] } }
    fn ddx(&mut self, _v: [f32; 4]) -> [f32; 4] { [0.0; 4] }
    fn ddy(&mut self, _v: [f32; 4]) -> [f32; 4] { [0.0; 4] }
}

/// shaders the GPU harness can drive
fn eligible(sh: &Shader) -> bool {
    if sh.program_type != ProgramType::Pixel { return false; }
    if !sh.decls.uavs.is_empty() { return false; }
    if sh.decls.inputs.iter().any(|d| d.sysvalue.is_some() || d.reg >= 15) { return false; } // WebGPU: at most 16 inter-stage locations (the harness vertex stage uses one more for position-less setup)
    if sh.decls.outputs.iter().any(|d| d.file != OperandType::OUTPUT) { return false; }
    if sh.decls.samplers.iter().any(|s| s.mode != 0) { return false; }
    if sh.decls.resources.iter().any(|r| !matches!(r.dim, 3 | 5 | 6) || r.kind != fn_dxbc::ResKind::Typed) { return false; }
    if sh.decls.icb.is_some() && false { return false; }
    sh.body().all(|i| !matches!(i.op, Opcode::Ld | Opcode::LdMs | Opcode::SampleC | Opcode::SampleCLz | Opcode::Resinfo | Opcode::Gather4 | Opcode::Lod | Opcode::SampleInfo))
}

struct Gpu { device: wgpu::Device, queue: wgpu::Queue, info: String }
fn gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor { backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12, ..wgpu::InstanceDescriptor::new_without_display_handle() });
    let order = if std::env::var_os("FN_SOFTWARE").is_some() { [true, false] } else { [false, true] };
    for fallback in order {
        if let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, force_fallback_adapter: fallback, compatible_surface: None, apply_limit_buckets: false })) {
            let i = adapter.get_info();
            let info = format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type);
            if let Ok((device, queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("fn-dxbc test"), required_features: wgpu::Features::empty(), required_limits: wgpu::Limits::default().using_resolution(adapter.limits()), ..Default::default() })) {
                return Some(Gpu { device, queue, info });
            }
        }
    }
    None
}

fn bytes_of(v: &[[f32; 4]]) -> Vec<u8> { v.iter().flat_map(|e| e.iter().flat_map(|f| f.to_le_bytes())).collect() }

/// the fragment module text with every varying forced to `flat` (the vertex stage emits constants)
fn flatten(src: &str) -> String {
    src.lines().map(|l| {
        if !l.starts_with("@fragment") { return l.to_string(); }
        let mut s = l.to_string();
        for pat in ["@interpolate(linear, centroid) ", "@interpolate(perspective, centroid) ", "@interpolate(linear) ", "@interpolate(flat) "] { s = s.replace(pat, ""); }
        s.replace(") in", ") @interpolate(flat) in")
    }).collect::<Vec<_>>().join("\n")
}

struct Case { sh: Shader, fs_src: String, name: String }

fn run_case(g: &Gpu, c: &Case, runs: usize, seed: u64) -> Result<(usize, usize, usize, f32), String> {
    let sh = &c.sh;
    let nin = sh.decls.inputs.iter().map(|d| d.reg as usize + 1).max().unwrap_or(0);
    let mut input_regs: Vec<u32> = sh.decls.inputs.iter().map(|d| d.reg).collect();
    input_regs.sort();
    input_regs.dedup();
    // ---- vertex shader: constant flat varyings from a uniform buffer
    let mut vs = String::new();
    vs.push_str("struct VsOut {\n    @builtin(position) pos: vec4<f32>,\n");
    for r in &input_regs { vs.push_str(&format!("    @location({r}) @interpolate(flat) a{r}: vec4<f32>,\n")); }
    vs.push_str("}\n");
    vs.push_str(&format!("@group(3) @binding(0) var<uniform> vin: array<vec4<f32>, {}>;\n", nin.max(1)));
    vs.push_str("@vertex fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {\n    var p = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));\n    var o: VsOut;\n    o.pos = vec4<f32>(p[i], 0.0, 1.0);\n");
    for r in &input_regs { vs.push_str(&format!("    o.a{r} = vin[{r}];\n")); }
    vs.push_str("    return o;\n}\n");
    let vsm = g.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("vs"), source: wgpu::ShaderSource::Wgsl(vs.into()) });
    let fsm = g.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("fs"), source: wgpu::ShaderSource::Wgsl(c.fs_src.clone().into()) });
    // ---- layouts
    let vis = wgpu::ShaderStages::FRAGMENT;
    let ubo = |binding: u32, vis| wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None };
    let mut cb_slots: Vec<(u32, u32)> = sh.decls.cbuffers.iter().map(|c| (c.slot, c.size.max(1))).collect();
    cb_slots.sort();
    let used_tex: BTreeSet<u32> = sh.body().flat_map(|i| i.operands.iter()).filter(|o| o.is(OperandType::RESOURCE)).filter_map(|o| o.reg_index()).collect();
    let tex_decls: Vec<_> = sh.decls.resources.iter().filter(|r| used_tex.contains(&r.slot)).collect();
    let bgl0 = g.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &cb_slots.iter().map(|(s, _)| ubo(*s, vis)).collect::<Vec<_>>() });
    let tex_entries: Vec<_> = tex_decls.iter().map(|r| wgpu::BindGroupLayoutEntry {
        binding: r.slot, visibility: vis, count: None,
        ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: match r.dim { 5 => wgpu::TextureViewDimension::D3, 6 => wgpu::TextureViewDimension::Cube, _ => wgpu::TextureViewDimension::D2 }, multisampled: false },
    }).collect();
    let bgl1 = g.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &tex_entries });
    let smp_entries: Vec<_> = sh.decls.samplers.iter().map(|s| wgpu::BindGroupLayoutEntry { binding: s.slot, visibility: vis, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering), count: None }).collect();
    let bgl2 = g.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &smp_entries });
    let bgl3 = g.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[ubo(0, wgpu::ShaderStages::VERTEX)] });
    let layout = g.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&bgl0), Some(&bgl1), Some(&bgl2), Some(&bgl3)], immediate_size: 0 });
    let pipeline = g.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &vsm, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState { module: &fsm, entry_point: Some(&format!("{}_main", c.name)), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::Rgba32Float, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    // ---- resources
    let cb_bufs: Vec<(u32, wgpu::Buffer)> = cb_slots.iter().map(|(s, n)| (*s, g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: *n as u64 * 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }))).collect();
    let vin_buf = g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: nin.max(1) as u64 * 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut tex_views = vec![];
    for r in &tex_decls {
        let layers = if r.dim == 6 { 6 } else { 1 };
        let tex = g.device.create_texture(&wgpu::TextureDescriptor {
            label: None, size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: if r.dim == 5 { 1 } else { layers } }, mip_level_count: 1, sample_count: 1,
            dimension: if r.dim == 5 { wgpu::TextureDimension::D3 } else { wgpu::TextureDimension::D2 }, format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
        });
        let col = tex_color(r.slot);
        let px: Vec<u8> = (0..layers).flat_map(|_| col.iter().flat_map(|f| f.to_le_bytes())).collect();
        for l in 0..layers {
            g.queue.write_texture(wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: 0, origin: wgpu::Origin3d { x: 0, y: 0, z: l }, aspect: wgpu::TextureAspect::All }, &px[(l as usize * 16)..(l as usize * 16 + 16)], wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(16), rows_per_image: Some(1) }, wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
        }
        let view = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(match r.dim { 5 => wgpu::TextureViewDimension::D3, 6 => wgpu::TextureViewDimension::Cube, _ => wgpu::TextureViewDimension::D2 }), ..Default::default() });
        tex_views.push((r.slot, view));
    }
    let sampler = g.device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest, mipmap_filter: wgpu::MipmapFilterMode::Nearest, ..Default::default() });
    let bg0 = g.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl0, entries: &cb_bufs.iter().map(|(s, b)| wgpu::BindGroupEntry { binding: *s, resource: b.as_entire_binding() }).collect::<Vec<_>>() });
    let bg1 = g.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl1, entries: &tex_views.iter().map(|(s, v)| wgpu::BindGroupEntry { binding: *s, resource: wgpu::BindingResource::TextureView(v) }).collect::<Vec<_>>() });
    let bg2 = g.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl2, entries: &sh.decls.samplers.iter().map(|s| wgpu::BindGroupEntry { binding: s.slot, resource: wgpu::BindingResource::Sampler(&sampler) }).collect::<Vec<_>>() });
    let bg3 = g.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &bgl3, entries: &[wgpu::BindGroupEntry { binding: 0, resource: vin_buf.as_entire_binding() }] });
    let target = g.device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba32Float, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
    let tview = target.create_view(&Default::default());
    let readback = g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 256, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });

    let mut rng = Rng(seed);
    let (mut n, mut ok3, mut ok5, mut worst) = (0usize, 0usize, 0usize, 0f32);
    let mut first_bad: Option<String> = None;
    const SENTINEL: f32 = -12345.0;
    for _ in 0..runs {
        let mut v = vec![[0.0f32; 4]; nin];
        for r in &input_regs { v[*r as usize] = [rng.val(), rng.val(), rng.val(), rng.val()]; }
        let mut cbs: Vec<(u32, Vec<[f32; 4]>)> = vec![];
        for (s, size) in &cb_slots { cbs.push((*s, (0..*size).map(|_| [rng.val(), rng.val(), rng.val(), rng.val()]).collect())); }
        // CPU
        let mut m = Machine::new(sh);
        for (i, x) in v.iter().enumerate() { m.set_input(i, *x); }
        for (s, d) in &cbs { m.set_cb(*s as usize, d); }
        m.opts.max_steps = 300_000;
        let want = match m.run(&mut ConstTex) { Ok(w) => w, Err(fn_dxbc::interp::InterpError::Limit) => continue, Err(e) => return Err(format!("{e}")) };
        // GPU
        g.queue.write_buffer(&vin_buf, 0, &bytes_of(&v));
        for ((_, b), (_, d)) in cb_bufs.iter().zip(cbs.iter()) { g.queue.write_buffer(b, 0, &bytes_of(d)); }
        let mut enc = g.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &tview, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: SENTINEL as f64, g: SENTINEL as f64, b: SENTINEL as f64, a: SENTINEL as f64 }), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg0, &[]);
            pass.set_bind_group(1, &bg1, &[]);
            pass.set_bind_group(2, &bg2, &[]);
            pass.set_bind_group(3, &bg3, &[]);
            pass.draw(0..3, 0..1);
        }
        enc.copy_texture_to_buffer(wgpu::TexelCopyTextureInfo { texture: &target, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All }, wgpu::TexelCopyBufferInfo { buffer: &readback, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(1) } }, wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 });
        g.queue.submit([enc.finish()]);
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| { let _ = tx.send(r); });
        g.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| format!("poll: {e:?}"))?;
        rx.recv().unwrap().map_err(|e| format!("map: {e:?}"))?;
        let data = slice.get_mapped_range().map_err(|e| format!("range: {e:?}"))?;
        let got: [f32; 4] = std::array::from_fn(|k| f32::from_le_bytes(data[4 * k..4 * k + 4].try_into().unwrap()));
        drop(data);
        readback.unmap();
        n += 1;
        let discarded_gpu = got.iter().all(|&x| x == SENTINEL);
        if want.discarded || discarded_gpu {
            if want.discarded == discarded_gpu { ok3 += 1; ok5 += 1; continue; }
            return Err(format!("discard mismatch: cpu {} gpu {}", want.discarded, discarded_gpu));
        }
        let w = want.o.first().copied().unwrap_or([0.0; 4]);
        let mut e3 = true;
        let mut e5 = true;
        for k in 0..4 {
            if got[k].is_nan() && w[k].is_nan() || got[k] == w[k] { continue; }
            let d = (got[k] - w[k]).abs();
            let scale = 1.0f32.max(got[k].abs()).max(w[k].abs());
            if d.is_nan() || d > 1e-3 * scale { e3 = false; }
            if d.is_nan() || d > 1e-5 * scale { e5 = false; }
            if d.is_finite() { worst = worst.max(d / scale); }
        }
        if e3 { ok3 += 1; }
        if e5 { ok5 += 1; }
        if !e3 && first_bad.is_none() { first_bad = Some(format!("gpu {got:?} cpu {w:?}")); }
    }
    if let Some(f) = first_bad { eprintln!("  {}: first disagreement: {f}", c.name); }
    Ok((n, ok3, ok5, worst))
}

#[test]
fn generated_wgsl_matches_interpreter_on_gpu() {
    let c = corpus();
    if c.is_empty() { eprintln!("no old reports, skipping"); return; }
    let Some(g) = gpu() else { eprintln!("no wgpu adapter available, skipping"); return };
    eprintln!("adapter: {}", g.info);
    g.device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| eprintln!("wgpu error: {e}")));
    let mut cases = vec![];
    for (name, bytes) in &c {
        let sh = Shader::parse(bytes).unwrap();
        if !eligible(&sh) { continue; }
        let Ok(w) = translate_wgsl(&sh, name) else { continue };
        cases.push(Case { fs_src: flatten(&w), sh, name: name.clone() });
    }
    eprintln!("{} of {} corpus shaders are eligible for the GPU harness", cases.len(), c.len());
    let (mut total, mut ok3, mut ok5, mut shaders_ok, mut errors) = (0, 0, 0, 0, vec![]);
    let mut worst_overall = 0f32;
    let mut bad_shaders = vec![];
    for (i, case) in cases.iter().enumerate() {
        match run_case(&g, case, 40, 1000 + i as u64) {
            Ok((n, a, b, w)) => {
                total += n; ok3 += a; ok5 += b; worst_overall = worst_overall.max(w);
                if a == n { shaders_ok += 1; } else { bad_shaders.push(format!("{} {a}/{n} within 1e-3 (worst rel err {w:.2e})", case.name)); }
            }
            Err(e) => errors.push(format!("{}: {e}", case.name)),
        }
    }
    eprintln!("GPU vs CPU: {} shaders, {total} samples; {ok3} within 1e-3 ({shaders_ok} shaders fully), {ok5} within 1e-5; worst relative error {worst_overall:.3e}; {} shaders failed to run", cases.len(), errors.len());
    for b in bad_shaders.iter().take(20) { eprintln!("  {b}"); }
    for e in errors.iter().take(20) { eprintln!("  error {e}"); }
    assert!(errors.is_empty(), "{} shaders could not be executed", errors.len());
    assert!(ok3 * 100 >= total * 99, "only {ok3} of {total} samples agree to 1e-3");
}

/// The same check on pixel shaders sliced fresh out of the install: the whole global shader cache (tonemapper, post processing, ...) and the
/// settings-UI materials.  Skipped without the install / a wgpu adapter.
#[test]
fn generated_wgsl_matches_interpreter_on_gpu_install_sample() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let Some(g) = gpu() else { eprintln!("no wgpu adapter available, skipping"); return };
    g.device.on_uncaptured_error(std::sync::Arc::new(|e: wgpu::Error| eprintln!("wgpu error: {e}")));
    let mut sources: Vec<Vec<u8>> = vec![fn_dxbc_cli::spec::package_bytes(&set, "GlobalShaderCache").unwrap()];
    let index = PathIndex::new(&set);
    let root = fn_testdata::reports().join("settings-ui-shaders-1.8");
    if let Ok(rd) = std::fs::read_dir(&root) {
        for d in rd.flatten() {
            if let Some(game) = index.find(&d.file_name().to_string_lossy()) { if let Ok(b) = fn_dxbc_cli::spec::package_bytes(&set, &game) { sources.push(b); } }
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut cases = vec![];
    for src in &sources {
        for b in fn_dxbc_cli::extract::scan_blobs(src) {
            let h = program_hash(&b.bytes);
            if !seen.insert(h) { continue; }
            let Ok(sh) = Shader::parse(&b.bytes) else { continue };
            if !eligible(&sh) { continue; }
            let name = format!("u_{h:016x}");
            let Ok(w) = translate_wgsl(&sh, &name) else { continue };
            cases.push(Case { fs_src: flatten(&w), sh, name });
        }
    }
    eprintln!("{} eligible install shaders (adapter {})", cases.len(), g.info);
    let (mut total, mut ok3, mut ok5, mut shaders_ok) = (0, 0, 0, 0);
    let (mut errors, mut bad_shaders) = (vec![], vec![]);
    let mut worst_overall = 0f32;
    for (i, case) in cases.iter().enumerate() {
        match run_case(&g, case, 12, 5000 + i as u64) {
            Ok((n, a, b, w)) => {
                total += n; ok3 += a; ok5 += b; worst_overall = worst_overall.max(w);
                if a == n { shaders_ok += 1; } else { bad_shaders.push(format!("{} {a}/{n} within 1e-3 (worst rel err {w:.2e})", case.name)); }
            }
            Err(e) => errors.push(format!("{}: {e}", case.name)),
        }
    }
    eprintln!("GPU vs CPU (install sample): {} shaders, {total} samples; {ok3} within 1e-3 ({shaders_ok} shaders fully), {ok5} within 1e-5; worst relative error {worst_overall:.3e}; {} shaders failed to run", cases.len(), errors.len());
    for b in bad_shaders.iter().take(20) { eprintln!("  {b}"); }
    for e in errors.iter().take(20) { eprintln!("  error {e}"); }
    assert!(errors.is_empty(), "{} shaders could not be executed", errors.len());
    assert!(ok3 * 100 >= total * 99, "only {ok3} of {total} samples agree to 1e-3");
}



