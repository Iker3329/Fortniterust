use crate::gpu::{Decoded, TexInfo};
use fn_render::fx::{FxLevel, FxTexFormat};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;

pub const INDEX_FILE: &str = "index.json";
pub const FORMAT_VERSION: u64 = 1;

#[derive(Clone, Debug)]
pub struct Entry {
    pub pack: usize,
    pub offset: u64,
    pub format: FxTexFormat,
    pub name: String,
    pub info: TexInfo,
    pub source_size: (usize, usize),
    pub levels: Vec<(u32, u32, u32)>,
}

impl Entry {
    pub fn bytes(&self) -> usize { self.levels.iter().map(|l| l.2 as usize).sum() }
}

pub struct Index {
    pub dir: PathBuf,
    pub packs: Vec<String>,
    pub map: HashMap<String, Entry>,
}

static INDEX: OnceLock<Option<Index>> = OnceLock::new();
static HITS: AtomicUsize = AtomicUsize::new(0);
static MISSES: AtomicUsize = AtomicUsize::new(0);
static HIT_BYTES: AtomicU64 = AtomicU64::new(0);
static READ_US: AtomicU64 = AtomicU64::new(0);

pub fn format_name(f: FxTexFormat) -> &'static str {
    match f { FxTexFormat::Rgba8 => "Rgba8", FxTexFormat::Bc1 => "Bc1", FxTexFormat::Bc2 => "Bc2", FxTexFormat::Bc3 => "Bc3", FxTexFormat::Bc4 => "Bc4", FxTexFormat::Bc5 => "Bc5", FxTexFormat::Bc7 => "Bc7" }
}
pub fn parse_format(s: &str) -> Option<FxTexFormat> {
    Some(match s { "Rgba8" => FxTexFormat::Rgba8, "Bc1" => FxTexFormat::Bc1, "Bc2" => FxTexFormat::Bc2, "Bc3" => FxTexFormat::Bc3, "Bc4" => FxTexFormat::Bc4, "Bc5" => FxTexFormat::Bc5, "Bc7" => FxTexFormat::Bc7, _ => return None })
}

pub fn key_of(object_path: &str) -> String { object_path.to_ascii_lowercase() }
pub fn package_fallback_ok(object_path: &str) -> bool { match object_path.split_once('.') { None => true, Some((p, o)) => p.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case(o)) } }

fn configured_dir() -> Option<PathBuf> {
    if let Some(d) = fn_live::var_os("FN_WEBTEX_DIR") { return Some(PathBuf::from(d)); }
    if cfg!(target_arch = "wasm32") && fn_live::var("FNWEB_WEBTEX").map_or(true, |v| v != "0") { return Some(PathBuf::from("/fn/assets/webtex")); }
    None
}

pub fn parse_index(dir: &Path, json: &[u8]) -> Result<Index, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| format!("{}: {e}", dir.join(INDEX_FILE).display()))?;
    if v["version"].as_u64() != Some(FORMAT_VERSION) { return Err(format!("{}: version {} (expected {FORMAT_VERSION})", dir.join(INDEX_FILE).display(), v["version"])); }
    let packs: Vec<String> = v["packs"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default();
    let mut map = HashMap::new();
    if let Some(t) = v["textures"].as_object() {
        map.reserve(t.len());
        for (k, e) in t {
            let s = |n: &str| e[n].as_str().unwrap_or("").to_owned();
            let Some(format) = e["f"].as_str().and_then(parse_format) else { continue };
            let levels: Vec<(u32, u32, u32)> = e["l"].as_array().map(|a| a.iter().filter_map(|l| Some((l[0].as_u64()? as u32, l[1].as_u64()? as u32, l[2].as_u64()? as u32))).collect()).unwrap_or_default();
            let Some(&(w, h, _)) = levels.first() else { continue };
            let pack = e["p"].as_u64().unwrap_or(0) as usize;
            if pack >= packs.len() { continue; }
            let info = TexInfo { filter: s("fi"), address_x: s("ax"), address_y: s("ay"), srgb: e["s"].as_bool().unwrap_or(true), width: w as usize, height: h as usize, levels: levels.len(), compression: s("c") };
            let source_size = (e["sz"][0].as_u64().unwrap_or(w as u64) as usize, e["sz"][1].as_u64().unwrap_or(h as u64) as usize);
            map.insert(k.clone(), Entry { pack, offset: e["o"].as_u64().unwrap_or(0), format, name: s("n"), info, source_size, levels });
        }
    }
    Ok(Index { dir: dir.to_owned(), packs, map })
}

pub fn open_index(dir: &Path) -> Result<Index, String> {
    let json = std::fs::read(dir.join(INDEX_FILE)).map_err(|e| format!("{}: {e}", dir.join(INDEX_FILE).display()))?;
    parse_index(dir, &json)
}

pub fn index() -> Option<&'static Index> {
    INDEX.get_or_init(|| {
        let dir = configured_dir()?;
        if !dir.join(INDEX_FILE).exists() { return None; }
        let t0 = std::time::Instant::now();
        match open_index(&dir) {
            Ok(ix) => { eprintln!("[webtex] {} textures in {} packs from {} ({:.0} ms)", ix.map.len(), ix.packs.len(), dir.display(), t0.elapsed().as_secs_f32() * 1000.); Some(ix) }
            Err(e) => { eprintln!("[webtex] off: {e}"); None }
        }
    }).as_ref()
}

pub fn warm() { let _ = fn_pak::webcook::index(); let _ = crate::webcook::index(); let _ = index(); }

impl Index {
    pub fn find(&self, object_path: &str) -> Option<&Entry> {
        self.map.get(&key_of(object_path)).or_else(|| package_fallback_ok(object_path).then(|| self.map.get(&key_of(crate::data::long_package(object_path)))).flatten())
    }

    pub fn read(&self, e: &Entry) -> Result<Decoded, String> {
        let path = self.dir.join(&self.packs[e.pack]);
        let total = e.bytes();
        let mut blob = vec![0u8; total];
        let mut f = std::fs::File::open(&path).map_err(|x| format!("{}: {x}", path.display()))?;
        f.seek(SeekFrom::Start(e.offset)).map_err(|x| format!("{}: {x}", path.display()))?;
        f.read_exact(&mut blob).map_err(|x| format!("{} @{} +{total}: {x}", path.display(), e.offset))?;
        let mut starts = Vec::with_capacity(e.levels.len());
        let mut at = 0usize;
        for l in &e.levels { starts.push(at); at += l.2 as usize; }
        let mut levels = Vec::with_capacity(e.levels.len());
        for i in (1..e.levels.len()).rev() {
            let tail = blob.split_off(starts[i]);
            levels.push(FxLevel { width: e.levels[i].0, height: e.levels[i].1, depth: 1, rgba8: tail });
        }
        levels.push(FxLevel { width: e.levels[0].0, height: e.levels[0].1, depth: 1, rgba8: blob });
        levels.reverse();
        let (format, levels) = if e.format.is_bc() && !fn_render::fx::bc_native_hint() {
            (FxTexFormat::Rgba8, crate::gpu::decode_bc_levels(e.format, &levels).map_err(|x| format!("{}: {x}", e.name))?)
        } else { (e.format, levels) };
        Ok((e.name.clone(), e.info.clone(), format, levels))
    }
}

pub fn load(object_path: &str) -> Option<Result<Decoded, String>> {
    if let Some(r) = crate::webcook::load(object_path) { return Some(r); }
    let ix = index()?;
    let Some(e) = ix.find(object_path) else {
        let n = MISSES.fetch_add(1, Ordering::Relaxed);
        if n < 100 { eprintln!("[webtex] not in the index (pak path): {object_path}"); }
        return None;
    };
    let t0 = std::time::Instant::now();
    let r = ix.read(e);
    READ_US.fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
    match &r {
        Ok(_) => { HITS.fetch_add(1, Ordering::Relaxed); HIT_BYTES.fetch_add(e.bytes() as u64, Ordering::Relaxed); }
        Err(x) => { eprintln!("[webtex] read failed, pak path used: {x}"); return None; }
    }
    Some(r)
}

pub fn source_size(object_path: &str) -> Option<(f32, f32)> {
    if let Some(s) = crate::webcook::source_size(object_path) { return Some(s); }
    index()?.find(object_path).map(|e| (e.source_size.0 as f32, e.source_size.1 as f32))
}

pub fn stats() -> Option<String> {
    if let Some(c) = crate::webcook::stats() { return Some(format!("{c}; {}", fn_pak::webcook::stats().unwrap_or_default())); }
    let ix = INDEX.get()?.as_ref()?;
    Some(format!("webtex: {} hits ({:.1} MB, {:.0} ms reading), {} misses of {} indexed", HITS.load(Ordering::Relaxed), HIT_BYTES.load(Ordering::Relaxed) as f64 / 1048576., READ_US.load(Ordering::Relaxed) as f64 / 1000., MISSES.load(Ordering::Relaxed), ix.map.len()))
}

pub struct LowIndex {
    pub path: PathBuf,
    pub map: HashMap<String, (u64, usize)>,
}
static LOW: OnceLock<Option<LowIndex>> = OnceLock::new();
static LOW_HITS: AtomicUsize = AtomicUsize::new(0);
static LOW_BYTES: AtomicU64 = AtomicU64::new(0);

fn low_dir() -> Option<PathBuf> {
    if let Some(d) = fn_live::var_os("FN_WEBTEX_LOW_DIR") { return Some(PathBuf::from(d)); }
    if cfg!(target_arch = "wasm32") && fn_live::var("FNWEB_TEXLOW").map_or(true, |v| v != "0") { return Some(PathBuf::from("/fn/assets/webtex-low")); }
    None
}

pub fn low_index() -> Option<&'static LowIndex> {
    LOW.get_or_init(|| {
        index()?;
        let dir = low_dir()?;
        let json = std::fs::read(dir.join(INDEX_FILE)).ok()?;
        let v: serde_json::Value = match serde_json::from_slice(&json) { Ok(v) => v, Err(e) => { eprintln!("[webtex] low tier off: {e}"); return None; } };
        let pack = v["pack"].as_str()?.to_owned();
        let mut map = HashMap::new();
        if let Some(t) = v["textures"].as_object() {
            map.reserve(t.len());
            for (k, e) in t { if let (Some(o), Some(f)) = (e["o"].as_u64(), e["k"].as_u64()) { map.insert(k.clone(), (o, f as usize)); } }
        }
        eprintln!("[webtex] low tier: {} textures from {}", map.len(), dir.display());
        Some(LowIndex { path: dir.join(pack), map })
    }).as_ref()
}

pub fn streaming() -> bool { (cfg!(target_arch = "wasm32") && crate::webcook::index().is_some()) || low_index().is_some() }

pub fn load_low(object_path: &str) -> Option<(Decoded, bool)> {
    if let Some(r) = crate::webcook::load_low(object_path) { return Some(r); }
    let ix = index()?;
    let low = low_index()?;
    let key = key_of(object_path);
    let (key, e) = match ix.map.get(&key) { Some(e) => (key, e), None => { if !package_fallback_ok(object_path) { return None; } let k2 = key_of(crate::data::long_package(object_path)); let e = ix.map.get(&k2)?; (k2, e) } };
    let &(offset, first) = low.map.get(&key)?;
    if first >= e.levels.len() { return None; }
    let n: usize = e.levels[first..].iter().map(|l| l.2 as usize).sum();
    let mut blob = vec![0u8; n];
    let t0 = std::time::Instant::now();
    let r = std::fs::File::open(&low.path).and_then(|mut f| { f.seek(SeekFrom::Start(offset))?; f.read_exact(&mut blob) });
    READ_US.fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
    if let Err(x) = r { eprintln!("[webtex] low read failed: {}: {x}", low.path.display()); return None; }
    let mut levels = Vec::with_capacity(e.levels.len() - first);
    let mut at = 0usize;
    for l in &e.levels[first..] { levels.push(FxLevel { width: l.0, height: l.1, depth: 1, rgba8: blob[at..at + l.2 as usize].to_vec() }); at += l.2 as usize; }
    let (format, levels) = if e.format.is_bc() && (!fn_render::fx::bc_native_hint() || levels[0].width % 4 != 0 || levels[0].height % 4 != 0) {
        (FxTexFormat::Rgba8, crate::gpu::decode_bc_levels(e.format, &levels).ok()?)
    } else { (e.format, levels) };
    LOW_HITS.fetch_add(1, Ordering::Relaxed);
    LOW_BYTES.fetch_add(n as u64, Ordering::Relaxed);
    Some(((e.name.clone(), e.info.clone(), format, levels), first == 0))
}

pub fn low_stats() -> String { format!("low tier {} textures ({:.1} MB)", LOW_HITS.load(Ordering::Relaxed), LOW_BYTES.load(Ordering::Relaxed) as f64 / 1048576.) }
