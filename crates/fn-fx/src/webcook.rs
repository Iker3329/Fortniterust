use crate::gpu::{Decoded, TexInfo};
use fn_pak::webcook::{unzstd, Rd};
use fn_render::fx::{FxLevel, FxTexFormat};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub const MAGIC: &[u8; 4] = b"WCT2";
pub const MAGIC_V1: &[u8; 4] = b"WCT1";
pub const INDEX_FILE: &str = "index.bin";
pub const TIER_DIMS: [u32; 4] = [256, 512, 1024, u32::MAX];
pub const TIER_FILES: [&str; 4] = ["low.bin", "t512.bin", "t1k.bin", "t2k.bin"];

pub fn tier_of(w: u32, h: u32) -> u8 { let m = w.max(h); TIER_DIMS.iter().position(|&d| m <= d).unwrap_or(3) as u8 }

#[derive(Clone, Debug, Default)]
pub struct Tier { pub tier: u8, pub first: u8, pub count: u8, pub file: u16, pub off: u64, pub clen: u32, pub xf: u8 }

pub fn bc_fields(f: FxTexFormat) -> Option<&'static [usize]> {
    Some(match f { FxTexFormat::Bc1 => &[4, 4], FxTexFormat::Bc4 => &[2, 6], FxTexFormat::Bc3 => &[2, 6, 4, 4], FxTexFormat::Bc5 => &[2, 6, 2, 6], FxTexFormat::Bc2 => &[8, 4, 4], _ => return None })
}

pub fn split_fields(b: &[u8], f: &[usize]) -> Vec<u8> {
    let bs: usize = f.iter().sum();
    let n = b.len() / bs;
    let mut out = Vec::with_capacity(b.len());
    let mut off = 0;
    for &w in f { for i in 0..n { out.extend_from_slice(&b[i * bs + off..i * bs + off + w]); } off += w; }
    out.extend_from_slice(&b[n * bs..]);
    out
}

pub fn unsplit_fields(b: &[u8], f: &[usize]) -> Vec<u8> {
    let bs: usize = f.iter().sum();
    let n = b.len() / bs;
    let mut out = vec![0u8; b.len()];
    let (mut off, mut at) = (0, 0);
    for &w in f { for i in 0..n { out[i * bs + off..i * bs + off + w].copy_from_slice(&b[at..at + w]); at += w; } off += w; }
    out[n * bs..].copy_from_slice(&b[at..]);
    out
}

#[derive(Clone, Debug)]
pub struct Tex {
    pub key: String,
    pub name: String,
    pub format: FxTexFormat,
    pub exact: bool,
    pub info: TexInfo,
    pub source: (u32, u32),
    pub levels: Vec<(u32, u32, u32)>,
    pub tiers: Vec<Tier>,
}

pub struct Index { pub dir: PathBuf, pub files: Vec<String>, pub map: HashMap<String, Tex> }

fn ws(o: &mut Vec<u8>, s: &str) { o.extend_from_slice(&(s.len() as u16).to_le_bytes()); o.extend_from_slice(s.as_bytes()); }

pub fn to_bytes(files: &[String], texs: &[Tex]) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(MAGIC);
    o.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for f in files { ws(&mut o, f); }
    o.extend_from_slice(&(texs.len() as u32).to_le_bytes());
    for t in texs {
        ws(&mut o, &t.key); ws(&mut o, &t.name); ws(&mut o, crate::webtex::format_name(t.format));
        o.push(t.info.srgb as u8); o.push(t.exact as u8);
        ws(&mut o, &t.info.filter); ws(&mut o, &t.info.address_x); ws(&mut o, &t.info.address_y); ws(&mut o, &t.info.compression);
        o.extend_from_slice(&t.source.0.to_le_bytes()); o.extend_from_slice(&t.source.1.to_le_bytes());
        o.push(t.levels.len() as u8);
        for l in &t.levels { for v in [l.0, l.1, l.2] { o.extend_from_slice(&v.to_le_bytes()); } }
        o.push(t.tiers.len() as u8);
        for r in &t.tiers { o.push(r.tier); o.push(r.first); o.push(r.count); o.extend_from_slice(&r.file.to_le_bytes()); o.extend_from_slice(&r.off.to_le_bytes()); o.extend_from_slice(&r.clen.to_le_bytes()); o.push(r.xf); }
    }
    o
}

pub fn from_bytes(dir: &Path, b: &[u8]) -> Result<Index, String> {
    let mut r = Rd { b, p: 0 };
    let magic = r.take(4)?;
    let v2 = magic == MAGIC;
    if !v2 && magic != MAGIC_V1 { return Err("webcook tex index: bad magic".into()); }
    let mut files = vec![];
    for _ in 0..r.u32()? { files.push(r.s()?); }
    let n = r.u32()? as usize;
    let mut map = HashMap::with_capacity(n);
    for _ in 0..n {
        let key = r.s()?; let name = r.s()?; let f = r.s()?;
        let format = crate::webtex::parse_format(&f).ok_or_else(|| format!("webcook tex index: format {f}"))?;
        let srgb = r.u8()? != 0; let exact = r.u8()? != 0;
        let (filter, address_x, address_y, compression) = (r.s()?, r.s()?, r.s()?, r.s()?);
        let source = (r.u32()?, r.u32()?);
        let mut levels = vec![];
        for _ in 0..r.u8()? { levels.push((r.u32()?, r.u32()?, r.u32()?)); }
        let mut tiers = vec![];
        for _ in 0..r.u8()? { tiers.push(Tier { tier: r.u8()?, first: r.u8()?, count: r.u8()?, file: r.u16()?, off: r.u64()?, clen: r.u32()?, xf: if v2 { r.u8()? } else { 0 } }); }
        let (w, h) = levels.first().map(|l| (l.0 as usize, l.1 as usize)).unwrap_or((0, 0));
        let info = TexInfo { filter, address_x, address_y, srgb, width: w, height: h, levels: levels.len(), compression };
        map.insert(key.clone(), Tex { key, name, format, exact, info, source, levels, tiers });
    }
    Ok(Index { dir: dir.to_owned(), files, map })
}

pub fn write_index(path: &Path, raw: &[u8], z: &[u8]) -> std::io::Result<()> {
    let mut o = (raw.len() as u32).to_le_bytes().to_vec();
    o.extend_from_slice(z);
    std::fs::write(path, o)
}

pub fn open_index(dir: &Path) -> Result<Index, String> {
    let p = dir.join(INDEX_FILE);
    let z = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    if z.len() < 8 { return Err(format!("{}: too short", p.display())); }
    let raw = u32::from_le_bytes(z[..4].try_into().unwrap()) as usize;
    from_bytes(dir, &unzstd(&z[4..], raw)?)
}

static INDEX: OnceLock<Option<Index>> = OnceLock::new();
static HITS: AtomicU64 = AtomicU64::new(0);
static LOW_HITS: AtomicU64 = AtomicU64::new(0);
static WIRE: AtomicU64 = AtomicU64::new(0);
static RAW: AtomicU64 = AtomicU64::new(0);
static DECODE_US: AtomicU64 = AtomicU64::new(0);

fn configured_dir() -> Option<PathBuf> {
    if let Some(d) = fn_live::var_os("FN_WEBCOOK_TEX_DIR") { return Some(PathBuf::from(d)); }
    if cfg!(target_arch = "wasm32") && fn_live::var("FNWEB_WEBCOOK").map_or(true, |v| v != "0") { return Some(PathBuf::from(fn_pak::webcook::web_base()).join("tex")); }
    None
}

pub fn index() -> Option<&'static Index> {
    INDEX.get_or_init(|| {
        let dir = configured_dir()?;
        if !dir.join(INDEX_FILE).exists() { eprintln!("[webcook] WARNING: tex pack MISSING ({} not found): textures fall back to the raw webtex store (about 2.6x the download)", dir.join(INDEX_FILE).display()); return None; }
        let t0 = std::time::Instant::now();
        match open_index(&dir) {
            Ok(ix) => { eprintln!("[webcook] tex pack: {} textures, cap {} px, from {} ({:.0} ms)", ix.map.len(), cap(), dir.display(), t0.elapsed().as_secs_f32() * 1000.); Some(ix) }
            Err(e) => { eprintln!("[webcook] WARNING: tex pack unreadable ({e}): textures fall back to the raw webtex store"); None }
        }
    }).as_ref()
}

static INDEX2: OnceLock<Option<Index>> = OnceLock::new();
pub fn index2() -> Option<&'static Index> {
    INDEX2.get_or_init(|| {
        if !(cfg!(target_arch = "wasm32") && fn_live::var("FNWEB_WEBCOOK_V2").is_ok_and(|v| v == "1")) { return None; }
        let dir = PathBuf::from("/fn/assets/webcook/v2/tex");
        if !dir.join(INDEX_FILE).exists() { return None; }
        match open_index(&dir) { Ok(ix) => { eprintln!("[webcook] tex pack v2 supplement: {} textures", ix.map.len()); Some(ix) } Err(e) => { eprintln!("[webcook] tex pack v2 supplement unreadable: {e}"); None } }
    }).as_ref()
}
fn find_any(object_path: &str) -> Option<(&'static Index, &'static Tex)> {
    if let Some(ix) = index() { if let Some(t) = ix.find(object_path) { return Some((ix, t)); } }
    let ix = index2()?;
    ix.find(object_path).map(|t| (ix, t))
}

pub fn load_raw(object_path: &str) -> Option<(FxTexFormat, Vec<FxLevel>)> {
    let (ix, t) = find_any(object_path)?;
    let full = ix.full_tier(t);
    let mut tiers: Vec<&Tier> = t.tiers.iter().filter(|r| r.tier <= full).collect();
    tiers.sort_by_key(|r| r.first);
    let mut levels = vec![];
    for r in tiers { levels.extend(ix.read_tier(t, r).ok()?); }
    if levels.is_empty() { return None; }
    HITS.fetch_add(1, Ordering::Relaxed);
    Some((t.format, levels))
}

pub fn cap() -> u32 {
    static CAP: OnceLock<u32> = OnceLock::new();
    *CAP.get_or_init(|| fn_live::var("FNWEB_TEXCAP").or_else(|_| fn_live::var("FN_TEXCAP")).ok().and_then(|v| v.parse().ok()).unwrap_or(1024))
}
fn cap_tier() -> u8 { tier_of(cap(), cap()) }

impl Index {
    pub fn find(&self, object_path: &str) -> Option<&Tex> {
        self.map.get(&crate::webtex::key_of(object_path)).or_else(|| crate::webtex::package_fallback_ok(object_path).then(|| self.map.get(&crate::webtex::key_of(crate::data::long_package(object_path)))).flatten())
    }

    fn read_tier(&self, t: &Tex, r: &Tier) -> Result<Vec<FxLevel>, String> {
        let path = self.dir.join(self.files.get(r.file as usize).ok_or("webcook: bad tex file")?);
        let mut z = vec![0u8; r.clen as usize];
        std::fs::File::open(&path).and_then(|mut f| { f.seek(SeekFrom::Start(r.off))?; f.read_exact(&mut z) }).map_err(|e| format!("{} @{}+{}: {e}", path.display(), r.off, r.clen))?;
        let lv = &t.levels[r.first as usize..(r.first + r.count) as usize];
        let raw: usize = lv.iter().map(|l| l.2 as usize).sum();
        let t0 = std::time::Instant::now();
        let mut b = unzstd(&z, raw)?;
        DECODE_US.fetch_add(t0.elapsed().as_micros() as u64, Ordering::Relaxed);
        WIRE.fetch_add(r.clen as u64, Ordering::Relaxed);
        RAW.fetch_add(raw as u64, Ordering::Relaxed);
        let mut out = Vec::with_capacity(lv.len());
        let fields = if r.xf == 1 { bc_fields(t.format) } else { None };
        for l in lv.iter().rev() { let tail = b.split_off(b.len() - l.2 as usize); let tail = match fields { Some(f) => unsplit_fields(&tail, f), None => tail }; out.push(FxLevel { width: l.0, height: l.1, depth: 1, rgba8: tail }); }
        out.reverse();
        Ok(out)
    }

    pub fn read(&self, t: &Tex, max_tier: u8) -> Result<Decoded, String> {
        let lowest = t.tiers.iter().map(|r| r.tier).min().unwrap_or(0);
        let mut tiers: Vec<&Tier> = t.tiers.iter().filter(|r| r.tier <= max_tier.max(lowest)).collect();
        tiers.sort_by_key(|r| r.first);
        let mut levels = vec![];
        for r in &tiers { levels.extend(self.read_tier(t, r)?); }
        if levels.is_empty() { return Err(format!("{}: no levels", t.name)); }
        let mut info = t.info.clone();
        info.width = levels[0].width as usize;
        info.height = levels[0].height as usize;
        info.levels = levels.len();
        let (format, levels) = if t.format.is_bc() && (!fn_render::fx::bc_native_hint() || levels[0].width % 4 != 0 || levels[0].height % 4 != 0) {
            (FxTexFormat::Rgba8, crate::gpu::decode_bc_levels(t.format, &levels).map_err(|e| format!("{}: {e}", t.name))?)
        } else { (t.format, levels) };
        Ok((t.name.clone(), info, format, levels))
    }

    pub fn full_tier(&self, t: &Tex) -> u8 { if t.exact { 3 } else { cap_tier() } }
}

pub fn load(object_path: &str) -> Option<Result<Decoded, String>> {
    let (ix, t) = find_any(object_path)?;
    match ix.read(t, ix.full_tier(t)) {
        Ok(d) => { HITS.fetch_add(1, Ordering::Relaxed); Some(Ok(d)) }
        Err(e) => { eprintln!("[webcook] {object_path}: {e} (fallback)"); None }
    }
}

pub fn load_low(object_path: &str) -> Option<(Decoded, bool)> {
    let (ix, t) = find_any(object_path)?;
    let full = ix.full_tier(t);
    let lowest = t.tiers.iter().map(|r| r.tier).min().unwrap_or(0);
    let has_more = t.tiers.iter().any(|r| r.tier > lowest && r.tier <= full);
    match ix.read(t, lowest) {
        Ok(d) => { LOW_HITS.fetch_add(1, Ordering::Relaxed); Some((d, !has_more)) }
        Err(e) => { eprintln!("[webcook] low {object_path}: {e} (fallback)"); None }
    }
}

pub fn source_size(object_path: &str) -> Option<(f32, f32)> {
    find_any(object_path).map(|(_, t)| (t.source.0 as f32, t.source.1 as f32))
}

pub fn stats() -> Option<String> {
    let ix = INDEX.get()?.as_ref()?;
    Some(format!("webcook tex: {} full + {} low loads, {:.1} MB over the wire -> {:.1} MB, {:.0} ms zstd, cap {} px, {} indexed",
        HITS.load(Ordering::Relaxed), LOW_HITS.load(Ordering::Relaxed), WIRE.load(Ordering::Relaxed) as f64 / 1048576., RAW.load(Ordering::Relaxed) as f64 / 1048576., DECODE_US.load(Ordering::Relaxed) as f64 / 1000., cap(), ix.map.len()))
}
