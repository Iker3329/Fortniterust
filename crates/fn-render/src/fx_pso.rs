use super::*;
use std::sync::Mutex;

static RECORD: Mutex<Option<(Vec<u8>, usize)>> = Mutex::new(None);

fn recording_on() -> bool { static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *ON.get_or_init(|| fn_live::var("FN_PSO_RECORD").is_ok_and(|v| v == "1")) }

pub(super) fn record(kind: u8, d: &FxPipelineDesc) {
    if kind == 7 || !recording_on() { return; }
    let Ok(mut g) = RECORD.lock() else { return };
    let (buf, n) = g.get_or_insert_with(|| (Vec::new(), 0));
    encode(buf, kind, d);
    *n += 1;
}

pub fn pso_recording() -> Vec<u8> {
    let raw = RECORD.lock().ok().and_then(|g| g.as_ref().map(|r| r.0.clone())).unwrap_or_default();
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(raw.len() as u32).to_le_bytes());
    out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec(&raw, 6));
    out
}
pub fn pso_recorded() -> usize { RECORD.lock().ok().and_then(|g| g.as_ref().map(|r| r.1)).unwrap_or(0) }

const MAGIC: &[u8; 4] = b"PSO1";
const FMTS: [FxFmt; 12] = [FxFmt::Float32x2, FxFmt::Float32x3, FxFmt::Float32x4, FxFmt::Unorm8x4, FxFmt::Unorm8x4Bgra, FxFmt::Float16x2, FxFmt::Float16x4, FxFmt::Sint32x4, FxFmt::Uint32x4, FxFmt::Uint8x4, FxFmt::Uint32, FxFmt::Snorm16x4];
const BLENDS: [FxBlend; 4] = [FxBlend::Opaque, FxBlend::Translucent, FxBlend::Additive, FxBlend::Modulate];
const CULLS: [FxCull; 3] = [FxCull::None, FxCull::BackCw, FxCull::BackCcw];

fn put_u32(b: &mut Vec<u8>, v: u32) { b.extend_from_slice(&v.to_le_bytes()); }
fn put_str(b: &mut Vec<u8>, s: &str) { put_u32(b, s.len() as u32); b.extend_from_slice(s.as_bytes()); }

fn encode(b: &mut Vec<u8>, kind: u8, d: &FxPipelineDesc) {
    b.push(kind);
    put_str(b, &d.label); put_str(b, &d.wgsl); put_str(b, &d.vs_entry); put_str(b, &d.fs_entry);
    put_u32(b, d.vertex_buffers.len() as u32);
    for v in &d.vertex_buffers {
        put_u32(b, v.stride); b.push(v.instance as u8); put_u32(b, v.attrs.len() as u32);
        for a in &v.attrs { put_u32(b, a.location); put_u32(b, a.offset); b.push(FMTS.iter().position(|f| *f == a.format).unwrap_or(0) as u8); }
    }
    put_u32(b, d.groups.len() as u32);
    for g in &d.groups {
        put_u32(b, g.len() as u32);
        for e in g {
            put_u32(b, e.binding);
            let (t, f) = match e.ty { FxBindTy::Uniform => (0, false), FxBindTy::Tex2D { filterable } => (1, filterable), FxBindTy::Tex3D { filterable } => (2, filterable), FxBindTy::Cube { filterable } => (3, filterable), FxBindTy::SceneDepth => (4, false), FxBindTy::Sampler { filtering } => (5, filtering), FxBindTy::StorageRead => (6, false) };
            b.extend_from_slice(&[t, f as u8, e.vertex as u8, e.fragment as u8]);
        }
    }
    b.push(BLENDS.iter().position(|x| *x == d.blend).unwrap_or(0) as u8);
    b.extend_from_slice(&[d.depth.test as u8, d.depth.write as u8, CULLS.iter().position(|x| *x == d.cull).unwrap_or(0) as u8, d.translucent as u8, d.separate_translucency as u8, d.responsive_aa as u8]);
}

struct Rd<'a> { b: &'a [u8], p: usize }
impl Rd<'_> {
    fn u8(&mut self) -> Option<u8> { let v = *self.b.get(self.p)?; self.p += 1; Some(v) }
    fn u32(&mut self) -> Option<u32> { let s = self.b.get(self.p..self.p + 4)?; self.p += 4; Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]])) }
    fn str(&mut self) -> Option<String> { let n = self.u32()? as usize; let s = self.b.get(self.p..self.p + n)?; self.p += n; Some(String::from_utf8_lossy(s).into_owned()) }
}

fn decode(r: &mut Rd) -> Option<(u8, FxPipelineDesc)> {
    let kind = r.u8()?;
    let (label, wgsl, vs_entry, fs_entry) = (r.str()?, r.str()?, r.str()?, r.str()?);
    let mut vertex_buffers = vec![];
    for _ in 0..r.u32()? {
        let (stride, instance) = (r.u32()?, r.u8()? != 0);
        let mut attrs = vec![];
        for _ in 0..r.u32()? { let (location, offset) = (r.u32()?, r.u32()?); attrs.push(FxAttr { location, offset, format: *FMTS.get(r.u8()? as usize)? }); }
        vertex_buffers.push(FxVertexBuffer { stride, instance, attrs });
    }
    let mut groups = vec![];
    for _ in 0..r.u32()? {
        let mut g = vec![];
        for _ in 0..r.u32()? {
            let binding = r.u32()?;
            let (t, f, vertex, fragment) = (r.u8()?, r.u8()? != 0, r.u8()? != 0, r.u8()? != 0);
            let ty = match t { 0 => FxBindTy::Uniform, 1 => FxBindTy::Tex2D { filterable: f }, 2 => FxBindTy::Tex3D { filterable: f }, 3 => FxBindTy::Cube { filterable: f }, 4 => FxBindTy::SceneDepth, 5 => FxBindTy::Sampler { filtering: f }, _ => FxBindTy::StorageRead };
            g.push(FxLayoutEntry { binding, ty, vertex, fragment });
        }
        groups.push(g);
    }
    let blend = *BLENDS.get(r.u8()? as usize)?;
    let depth = FxDepth { test: r.u8()? != 0, write: r.u8()? != 0 };
    let cull = *CULLS.get(r.u8()? as usize)?;
    let (translucent, separate_translucency, responsive_aa) = (r.u8()? != 0, r.u8()? != 0, r.u8()? != 0);
    Some((kind, FxPipelineDesc { label, wgsl, vs_entry, fs_entry, vertex_buffers, groups, blend, depth, cull, translucent, separate_translucency, responsive_aa }))
}

pub(super) struct Warm { raw: Vec<u8>, at: usize, made: usize, failed: usize, t0: std::time::Instant, logged: bool }

fn cache_path() -> String { fn_live::var("FNWEB_PSO_CACHE").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| "/fn/assets/webcook/pso/pso.bin".to_owned()) }

fn load() -> Option<Vec<u8>> {
    let b = std::fs::read(cache_path()).ok()?;
    if b.get(..4)? != MAGIC { return None; }
    let n = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?) as usize;
    let raw = miniz_oxide::inflate::decompress_to_vec_with_limit(b.get(8..)?, n).ok()?;
    (raw.len() == n).then_some(raw)
}

impl FxRenderer {
    pub fn pso_warm_step(&mut self, budget_ms: f32) -> bool {
        if fn_live::var("FNWEB_PSO_WARM").is_ok_and(|v| v == "0") { return true; }
        if self.pso_warm.is_none() {
            let raw = load();
            match &raw { Some(r) => eprintln!("[pso] cache {}: {:.1} MB of pipeline descriptors", cache_path(), r.len() as f64 / 1048576.), None => eprintln!("[pso] no pipeline cache at {}", cache_path()) }
            self.pso_warm = Some(Warm { raw: raw.unwrap_or_default(), at: 0, made: 0, failed: 0, t0: std::time::Instant::now(), logged: false });
        }
        if self.scene.is_none() { return false; }
        let t = std::time::Instant::now();
        let backdrop = std::mem::replace(&mut self.building_backdrop, false);
        loop {
            let Some(w) = self.pso_warm.as_mut() else { break };
            if w.at >= w.raw.len() { break; }
            let mut r = Rd { b: &w.raw, p: w.at };
            let Some((kind, d)) = decode(&mut r) else { w.at = w.raw.len(); break };
            w.at = r.p;
            let ok = match kind { 0 => self.create_pipeline(&d), 1 => self.create_scene_lit_pipeline(&d), _ => self.create_shadow_pipeline(&d) }.is_ok();
            if let Some(w) = self.pso_warm.as_mut() { if ok { w.made += 1 } else { w.failed += 1 } }
            if t.elapsed().as_secs_f32() * 1000. >= budget_ms { break; }
        }
        self.building_backdrop = backdrop;
        let pending = self.pending_pipelines();
        let Some(w) = self.pso_warm.as_mut() else { return true };
        let done = w.at >= w.raw.len() && pending == 0;
        if done && !w.logged {
            w.logged = true;
            if !w.raw.is_empty() { eprintln!("[pso] warmed {} pipelines ({} failed) in {:.1} s", w.made, w.failed, w.t0.elapsed().as_secs_f32()); }
            w.raw = Vec::new();
        }
        done
    }
    pub fn pso_warm_progress(&self) -> (usize, usize, usize) { self.pso_warm.as_ref().map_or((0, 0, 0), |w| (w.made, w.at, w.raw.len())) }
}
