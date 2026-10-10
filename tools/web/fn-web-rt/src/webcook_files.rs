use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

fn dir() -> String { format!("{}/files", crate::env::get("FNWEB_WEBCOOK_DIR").filter(|v| !v.is_empty()).unwrap_or_else(|| if crate::env::get("FNWEB_WEBCOOK_V2").is_some_and(|v| v == "1") { "/fn/assets/webcook/v2" } else { "/fn/assets/webcook/v1" }.to_owned())) }
const MAGIC: &[u8; 4] = b"WCF1";
const MAGIC2: &[u8; 4] = b"WCF2";
const CACHE_BUDGET: usize = 32 << 20;

struct Ent { size: u64, raw: u64, file: u16, off: u64, clen: u32 }
struct Index { files: Vec<String>, map: HashMap<String, Ent> }

static INDEX: OnceLock<Option<Index>> = OnceLock::new();
static CACHE: Mutex<Option<(HashMap<String, Arc<Vec<u8>>>, VecDeque<String>, usize)>> = Mutex::new(None);
static HITS: AtomicU64 = AtomicU64::new(0);
static WIRE: AtomicU64 = AtomicU64::new(0);
static RAW: AtomicU64 = AtomicU64::new(0);

pub fn stats() -> (u64, f64, f64) { (HITS.load(Ordering::Relaxed), WIRE.load(Ordering::Relaxed) as f64 / 1048576., RAW.load(Ordering::Relaxed) as f64 / 1048576.) }

fn unzstd(z: &[u8], size: usize) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    ruzstd::decoding::FrameDecoder::new().decode_all_to_vec(z, &mut out).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("zstd: {e:?}")))?;
    if out.len() != size { return Err(io::Error::new(io::ErrorKind::InvalidData, "zstd: size mismatch")); }
    Ok(out)
}

fn whole(path: &str) -> io::Result<Vec<u8>> {
    let (kind, size, _) = super::vfs::stat(path)?;
    if kind != 1 { return Err(io::Error::new(io::ErrorKind::NotFound, path.to_owned())); }
    let h = super::vfs::open(path, 1)?;
    let mut v = vec![0u8; size as usize];
    let mut at = 0;
    while at < v.len() { let n = super::vfs::read_at(h, at as u64, &mut v[at..])?; if n == 0 { break; } at += n; }
    super::vfs::close(h);
    Ok(v)
}

fn parse(b: &[u8]) -> Option<Index> {
    let v2 = b.get(..4)? == MAGIC2;
    if !v2 && b.get(..4)? != MAGIC { return None; }
    let mut p = 4usize;
    let rd = |p: &mut usize, n: usize| -> Option<u64> { let s = b.get(*p..*p + n)?; *p += n; let mut a = [0u8; 8]; a[..n].copy_from_slice(s); Some(u64::from_le_bytes(a)) };
    let rs = |p: &mut usize| -> Option<String> { let n = rd(p, 2)? as usize; let s = b.get(*p..*p + n)?; *p += n; Some(String::from_utf8_lossy(s).into_owned()) };
    let mut files = vec![];
    for _ in 0..rd(&mut p, 4)? { files.push(rs(&mut p)?); }
    let n = rd(&mut p, 4)? as usize;
    let mut map = HashMap::with_capacity(n);
    for _ in 0..n {
        let k = rs(&mut p)?;
        let size = rd(&mut p, 8)?;
        let raw = if v2 { rd(&mut p, 8)? } else { size };
        map.insert(k, Ent { size, raw, file: rd(&mut p, 2)? as u16, off: rd(&mut p, 8)?, clen: rd(&mut p, 4)? as u32 });
    }
    Some(Index { files, map })
}

fn index() -> Option<&'static Index> {
    INDEX.get_or_init(|| {
        if crate::env::get("FNWEB_WEBCOOK").is_some_and(|v| v == "0") { return None; }
        let b = whole(&format!("{}/index.bin", dir())).ok()?;
        if b.len() < 8 { return None; }
        let raw = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize;
        let ix = unzstd(&b[4..], raw).ok().and_then(|r| parse(&r));
        match &ix { Some(i) => crate::js::fnweb_console(0, &format!("[webcook] files overlay: {} files", i.map.len())), None => crate::js::fnweb_console(1, "[webcook] files overlay: bad index") }
        ix
    }).as_ref()
}

pub fn file(path: &str, size: u64) -> Option<Arc<Vec<u8>>> {
    let lower = path.to_ascii_lowercase();
    if !lower.starts_with("/fn/assets/") || lower.starts_with("/fn/assets/webcook") || lower.starts_with("/fn/assets/webtex") { return None; }
    let ix = index()?;
    let e = ix.map.get(&lower)?;
    if e.size != size { return None; }
    if let Some(d) = CACHE.lock().ok()?.as_ref().and_then(|c| c.0.get(&lower).cloned()) { HITS.fetch_add(1, Ordering::Relaxed); return Some(d); }
    let pack = format!("{}/{}", dir(), ix.files.get(e.file as usize)?);
    let (off, clen, raw) = (e.off, e.clen as usize, e.raw as usize);
    let load = move || -> io::Result<Vec<u8>> {
        let h = super::vfs::open(&pack, 1)?;
        let mut z = vec![0u8; clen];
        let mut at = 0;
        while at < clen { let n = super::vfs::read_at(h, off + at as u64, &mut z[at..])?; if n == 0 { break; } at += n; }
        super::vfs::close(h);
        unzstd(&z, raw)
    };
    let d = if crate::pool::can_block() { load() } else { super::vfs::on_io_thread_pub(load) };
    let d = match d { Ok(d) => Arc::new(d), Err(x) => { crate::js::fnweb_console(1, &format!("[webcook] {path}: {x} (plain read)")); return None; } };
    HITS.fetch_add(1, Ordering::Relaxed);
    WIRE.fetch_add(clen as u64, Ordering::Relaxed);
    RAW.fetch_add(raw as u64, Ordering::Relaxed);
    if let Ok(mut g) = CACHE.lock() {
        let c = g.get_or_insert_with(|| (HashMap::new(), VecDeque::new(), 0));
        if d.len() <= CACHE_BUDGET / 2 && !c.0.contains_key(&lower) {
            c.2 += d.len();
            c.0.insert(lower.clone(), d.clone());
            c.1.push_back(lower);
            while c.2 > CACHE_BUDGET { let Some(k) = c.1.pop_front() else { break }; if let Some(x) = c.0.remove(&k) { c.2 -= x.len(); } }
        }
    }
    Some(d)
}
