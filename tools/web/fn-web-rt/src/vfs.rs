//! Virtual file system of the browser build, over the local launcher's HTTP API (`fn-web-serve`, same origin):
//!
//! | path        | backing                                                          | access     |
//! |-------------|------------------------------------------------------------------|------------|
//! | `/fn/paks`  | the user's own `FortniteGame/Content/Paks` (never uploaded)      | read       |
//! | `/fn/assets`| the extracted runtime assets folder                              | read       |
//! | `/fn/bin`   | the launcher's folder (optional `FortniteRuntime.toml` etc.)     | read       |
//! | `/fn/user`  | a per-user folder on disk (settings); written back on flush/close| read/write |
//! | `/fn/tmp`   | memory only                                                      | read/write |
//!
//! HTTP API (all synchronous XHR; the main thread uses the x-user-defined text trick):
//! `GET /__fnvfs/stat?p=` -> `kind size mtime_ms` (kind 1 file, 2 dir) | 404; `GET /__fnvfs/list?p=` -> lines `kind\tsize\tmtime\tname`;
//! `GET /__fnvfs/file?p=` (+ `Range`) -> bytes; `PUT /__fnvfs/file?p=` (body) / `DELETE /__fnvfs/file?p=` / `POST /__fnvfs/mkdir?p=` /
//! `POST /__fnvfs/rmdir?p=` / `POST /__fnvfs/rename?p=&to=` -> 204; `GET /__fnvfs/bundle?p=&max=&ext=` -> the small files below `p` in one body.
//!
//! Every request costs a round trip through the browser's network service (a few ms; the profile of verification/browser/launch5 showed ~10 s of
//! one-file-per-request JSON reads on the main thread), so:
//! * remote reads smaller than a block go through a shared block cache (256 KiB blocks, LRU, [`set_cache_mb`]): the many small reads of pak
//!   parsing and `BufReader`s cost one request per block; a read of a block or more is one ranged request of exactly its bytes;
//! * small text files (JSON / txt / ini / csv up to 256 KiB) below an asset folder (`/fn/assets/<folder>/...`) come from that folder's bundle: the
//!   first such read fetches all of them in one request (see [`small_file`]).
//!
//! REMOTE MODE ([`set_remote`], from the page's login): the same API under the page's own prefix (`<page dir>/__fnvfs/...`, same origin), where the
//! page's service worker (www/sw.js) answers from its cache or forwards to the user's private file Worker; every request carries the login's bearer
//! token (`Authorization`, never in a URL). The Worker's storage is case-sensitive, so a path is turned into its exact case through the root's cached
//! tree before it is requested. `/fn/user` lives in the browser (memory, saved to IndexedDB by the page: `fnweb_user_save`), `/fn/windows` does not
//! exist (the UI falls back from the system fonts to the game's NotoSans), blocks are 1 MiB (fewer round trips over the internet).
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Block of the block cache; reads of at least one block are one ranged request of exactly their bytes (no block cache). 1 MiB in remote mode.
fn block() -> u64 { if remote() { 1 << 20 } else { 256 << 10 } }

static CACHE_CAP: AtomicUsize = AtomicUsize::new(96 << 20);
/// Sets the block cache budget and evicts down to it now (the small-file bundles are dropped too; a later read fetches its folder's bundle again).
pub fn set_cache_mb(mb: usize) {
    CACHE_CAP.store(mb << 20, Ordering::Relaxed);
    if let Ok(mut g) = SMALL.lock() { *g = None; }
    if let Ok(mut g) = BUNDLED.lock() { *g = None; }
    if let Ok(mut g) = CACHE.lock() {
        if let Some(c) = g.as_mut() {
            while c.bytes > (mb << 20) { let Some(k) = c.order.pop_front() else { break }; if let Some(d) = c.map.remove(&k) { c.bytes -= d.len(); } }
        }
    }
}

/// Statistics: (requests, bytes fetched, cache hits)
static REQUESTS: AtomicU64 = AtomicU64::new(0);
static FETCHED: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);
/// Microseconds inside requests (all threads), and microseconds the main thread waited for files (its own requests + delegated reads).
static HTTP_US: AtomicU64 = AtomicU64::new(0);
static MAIN_WAIT_US: AtomicU64 = AtomicU64::new(0);
/// Small files served from bundles.
static BUNDLE_HITS: AtomicU64 = AtomicU64::new(0);
pub fn stats() -> (u64, u64, u64) { (REQUESTS.load(Ordering::Relaxed), FETCHED.load(Ordering::Relaxed), HITS.load(Ordering::Relaxed)) }
/// (ms inside requests, ms the main thread waited for files, small files served from bundles)
pub fn timing() -> (f64, f64, u64) { (HTTP_US.load(Ordering::Relaxed) as f64 / 1000., MAIN_WAIT_US.load(Ordering::Relaxed) as f64 / 1000., BUNDLE_HITS.load(Ordering::Relaxed)) }
fn now_us() -> u64 { (crate::js::fnweb_now_ms() * 1000.) as u64 }
/// Runs `f`; on the main thread its duration counts as waiting for files.
fn main_wait<T>(what: &str, f: impl FnOnce() -> T) -> T {
    if crate::pool::can_block() { return f(); }
    let t = now_us();
    let r = f();
    let us = now_us().saturating_sub(t);
    MAIN_WAIT_US.fetch_add(us, Ordering::Relaxed);
    static LOG_MS: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
    if let Some(min) = *LOG_MS.get_or_init(|| crate::env::get("FNWEB_WAIT_LOG").and_then(|v| v.parse().ok())) {
        if us as f64 / 1000. >= min { eprintln!("[main wait] {:.1} ms {what} <- {}", us as f64 / 1000., crate::js::fnweb_stack_deep(40)); }
    }
    r
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// paths
// ------------------------------------------------------------------------------------------------------------------------------------------

/// `/`-separated absolute path with `.` / `..` resolved; backslashes are separators too. Relative paths start at the working directory.
pub fn normalize(p: &str) -> String {
    let p = p.replace('\\', "/");
    let base = if p.starts_with('/') { String::new() } else { crate::env::get("FNWEB_CWD").unwrap_or_else(|| "/fn/bin".into()) };
    let mut parts: Vec<&str> = vec![];
    for seg in base.split('/').chain(p.split('/')) {
        match seg {
            "" | "." => {}
            ".." => { parts.pop(); }
            s => parts.push(s),
        }
    }
    format!("/{}", parts.join("/"))
}

fn writable_local(p: &str) -> bool { p == "/fn/tmp" || p.starts_with("/fn/tmp/") }
fn writable_remote(p: &str) -> bool { p == "/fn/user" || p.starts_with("/fn/user/") }

fn enc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() * 3 / 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => o.push(b as char),
            _ => o.push_str(&format!("%{b:02X}")),
        }
    }
    o
}

fn url(op: &str, p: &str) -> String { format!("{}/__fnvfs/{op}?p={}", prefix(), enc(p)) }

/// REMOTE MODE: the content version of a read-only file, from its root's tree (`<size>-<uploaded ms>`): sent as `&v=` with every read of it, so the
/// service worker keys its persistent cache by content version (a file replaced in the bucket gets a new key; a returning browser re-downloads only
/// what changed, without a cache-version bump). The service worker strips it before asking the file Worker (one URL per op stays one CORS preflight).
fn version_of(p: &str) -> Option<String> {
    if !remote() { return None; }
    let (_, size, mtime) = *tree_root(p).and_then(tree)?.stat.get(&p.to_ascii_lowercase())?;
    Some(format!("{size}-{mtime:.0}"))
}
/// The `file` URL of a read-only file (+ its content version in remote mode).
fn file_url(p: &str) -> String {
    let mut u = url("file", p);
    if let Some(v) = version_of(p) { u.push_str("&v="); u.push_str(&v); }
    u
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// the file server: the local launcher (default) or the user's private storage (remote mode)
// ------------------------------------------------------------------------------------------------------------------------------------------

/// REMOTE MODE: the prefix of the page's own `__fnvfs` API (its service worker forwards it) and the login's bearer token. None = the launcher.
struct Server { prefix: String, token: String }
static SERVER: std::sync::RwLock<Option<Server>> = std::sync::RwLock::new(None);

/// Switches the file system to remote mode (the page calls it after its login, before the game starts): `prefix` = the page's directory
/// (e.g. `/fortnite`), `token` = the bearer token.
pub fn set_remote(prefix: &str, token: &str) {
    if let Ok(mut g) = SERVER.write() { *g = Some(Server { prefix: prefix.trim_end_matches('/').to_owned(), token: token.to_owned() }); }
}
pub fn remote() -> bool { SERVER.read().map(|g| g.is_some()).unwrap_or(false) }
fn prefix() -> String { SERVER.read().ok().and_then(|g| g.as_ref().map(|s| s.prefix.clone())).unwrap_or_default() }
fn token() -> Option<String> { SERVER.read().ok()?.as_ref().map(|s| s.token.clone()) }
/// REMOTE MODE: `/fn/user` is kept in the browser (memory + the page's IndexedDB copy), like `/fn/tmp` (memory only).
fn memory_backed(p: &str) -> bool { writable_local(p) || (writable_remote(p) && remote()) }
/// REMOTE MODE: the system fonts cannot be hosted and the storage has no launcher folder: `/fn/windows` and `/fn/bin` do not exist.
fn absent(p: &str) -> bool { remote() && ["/fn/windows", "/fn/bin"].iter().any(|r| p == *r || p.starts_with(&format!("{r}/"))) }
/// REMOTE MODE: a read-only path in the exact case of the storage (through the root's cached tree); unchanged otherwise.
fn exact_path(p: &str) -> String {
    if !remote() { return p.to_owned(); }
    tree_root(p).and_then(tree).and_then(|t| t.exact.get(&p.to_ascii_lowercase()).cloned()).unwrap_or_else(|| p.to_owned())
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// HTTP
// ------------------------------------------------------------------------------------------------------------------------------------------

fn js_err(what: &str, e: wasm_bindgen::JsValue) -> io::Error {
    let msg = e.as_string().or_else(|| js_sys::Reflect::get(&e, &"message".into()).ok().and_then(|m| m.as_string())).unwrap_or_else(|| format!("{e:?}"));
    let kind = if msg.contains("HTTP 403") { io::ErrorKind::PermissionDenied } else { io::ErrorKind::Other };
    io::Error::new(kind, format!("{what}: {msg}"))
}

/// BOOT MANIFEST (`FNWEB_RECORD=1`): every file / bundle request of this session in first-request order, as (op, exact path, range start, range
/// end (inclusive; -1 = whole), the other query parameters without `v`): `fnweb_fetch_log` hands it to the test harness, which writes
/// www/fnweb-boot-manifest.json; in remote mode the page's prefetch worker (www/fnweb-prefetch.js) requests those ranges in parallel at start-up.
static RECORD: Mutex<Option<(std::collections::HashSet<String>, Vec<String>)>> = Mutex::new(None);
fn record(u: &str, range: Option<(u64, u64)>) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| crate::env::get("FNWEB_RECORD").is_some_and(|v| v == "1")) { return; }
    let Some(i) = u.find("/__fnvfs/") else { return };
    let rest = &u[i + "/__fnvfs/".len()..];
    let (op, q) = rest.split_once('?').unwrap_or((rest, ""));
    if op != "file" && op != "bundle" { return; }
    let mut path = String::new();
    let mut other = vec![];
    for kv in q.split('&') {
        match kv.split_once('=') { Some(("p", v)) => path = dec(v), Some(("v", _)) => {} _ => if !kv.is_empty() { other.push(kv.to_owned()) } }
    }
    let (a, b) = range.map_or((-1i64, -1i64), |(a, b)| (a as i64, b as i64));
    let key = format!("[{},{},{a},{b},{}", json_str(op), json_str(&path), json_str(&other.join("&")));
    if let Ok(mut g) = RECORD.lock() { let (seen, list) = g.get_or_insert_with(Default::default); if seen.insert(key.clone()) { list.push(format!("{key},{:.0}]", crate::js::fnweb_wall_ms())); } }
}
/// The recorded requests as a JSON array (see [`record`]).
pub fn fetch_log() -> String { RECORD.lock().ok().and_then(|g| g.as_ref().map(|(_, l)| format!("[\n{}\n]", l.join(",\n")))).unwrap_or_else(|| "[]".into()) }
fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
/// Percent-decoding of a query value (the inverse of [`enc`]).
fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() { if let Ok(x) = u8::from_str_radix(&s[i + 1..i + 3], 16) { out.push(x); i += 3; continue; } }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// GET (optionally a byte range, inclusive end). Ok(None) = 404.
fn get(u: &str, range: Option<(u64, u64)>) -> io::Result<Option<js_sys::Uint8Array>> {
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    record(u, range);
    let (a, b) = range.map_or((-1.0, -1.0), |(a, b)| (a as f64, b as f64));
    let t = now_us();
    let r = crate::js::fnweb_http("GET", u, a, b, None, token());
    HTTP_US.fetch_add(now_us().saturating_sub(t), Ordering::Relaxed);
    let r = r.map_err(|e| js_err(u, e))?;
    if let Some(arr) = &r { FETCHED.fetch_add(arr.length() as u64, Ordering::Relaxed); }
    Ok(r)
}

fn send(method: &str, u: &str, body: Option<&[u8]>) -> io::Result<bool> {
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    let body = body.map(js_sys::Uint8Array::from);
    let r = crate::js::fnweb_http(method, u, -1.0, -1.0, body, token()).map_err(|e| js_err(u, e))?;
    Ok(r.is_some())
}

/// File bytes (optionally a range): like [get], but always as a Vec, and on the main thread the request is made by an I/O thread (a Window's
/// synchronous XHR can only return text, which JS converts byte by byte: slow for anything but small metadata replies).
fn fetch(u: &str, range: Option<(u64, u64)>) -> io::Result<Option<Vec<u8>>> {
    if crate::pool::can_block() { return Ok(get(u, range)?.map(|a| a.to_vec())); }
    on_io_thread(|| get(u, range).map(|r| r.map(|a| a.to_vec())))
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// I/O threads: the main thread's file requests
// ------------------------------------------------------------------------------------------------------------------------------------------

type IoJob = Box<dyn FnOnce() + Send + 'static>;
static IO_QUEUE: Mutex<VecDeque<IoJob>> = Mutex::new(VecDeque::new());
static IO_WAKE: std::sync::Condvar = std::sync::Condvar::new();
static IO_THREADS: AtomicUsize = AtomicUsize::new(0);

/// Starts the two threads that make the main thread's file requests. Call once at start-up, while the worker pool is idle: a main-thread read
/// handed to the general pool waited behind the loading jobs (verification/browser/launch6: the loading screen's first frame waited 6.7 s for its
/// images while every worker decoded clips and materials).
pub fn start_io_threads() {
    if IO_THREADS.swap(2, Ordering::SeqCst) != 0 { return; }
    for i in 0..2 {
        let spawned = std::thread::Builder::new().name(format!("fnweb-io-{i}")).spawn(|| loop {
            let job = {
                let mut q = IO_QUEUE.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(j) = q.pop_front() { break j; }
                    q = IO_WAKE.wait(q).unwrap_or_else(|e| e.into_inner());
                }
            };
            job();
        });
        if spawned.is_err() { IO_THREADS.fetch_sub(1, Ordering::SeqCst); }
    }
}

/// Runs `f` on an I/O thread and waits (spinning: the main thread cannot block) for its result. Without I/O threads, on a pool worker.
fn on_io_thread<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    if IO_THREADS.load(Ordering::SeqCst) == 0 {
        return std::thread::scope(|s| s.spawn(f).join()).unwrap_or_else(|_| panic!("fnweb vfs: read worker panicked"));
    }
    let mut out: Option<R> = None;
    let done = std::sync::atomic::AtomicBool::new(false);
    {
        let (out_ref, done_ref) = (&mut out, &done);
        let job: Box<dyn FnOnce() + Send + '_> = Box::new(move || { *out_ref = Some(f()); done_ref.store(true, Ordering::Release); });
        // SAFETY: this function does not return before the job ran (`done`), so everything the job borrows outlives it (as in `thread::scope`)
        let job: IoJob = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + '_>, IoJob>(job) };
        IO_QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push_back(job);
        IO_WAKE.notify_one();
    }
    while !done.load(Ordering::Acquire) { std::hint::spin_loop(); }
    out.expect("fnweb vfs: the I/O job did not produce a result")
}

pub(crate) fn on_io_thread_pub<R: Send>(f: impl FnOnce() -> R + Send) -> R { on_io_thread(f) }

fn text(arr: &js_sys::Uint8Array) -> String { String::from_utf8_lossy(&arr.to_vec()).into_owned() }

fn not_found(p: &str) -> io::Error { io::Error::new(io::ErrorKind::NotFound, format!("{p}: not found")) }

// ------------------------------------------------------------------------------------------------------------------------------------------
// memory files (/fn/tmp, and the open copies of /fn/user files)
// ------------------------------------------------------------------------------------------------------------------------------------------

type Data = Arc<Mutex<Vec<u8>>>;

/// path -> contents of every memory file (`/fn/tmp/*`) and of `/fn/user` files opened for writing this session.
static MEMFS: Mutex<Option<HashMap<String, Data>>> = Mutex::new(None);
/// Directories created under /fn/tmp.
static MEMDIRS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn mem_get(p: &str) -> Option<Data> { MEMFS.lock().ok()?.as_ref()?.get(p).cloned() }
fn mem_put(p: &str, d: Data) { if let Ok(mut m) = MEMFS.lock() { m.get_or_insert_with(HashMap::new).insert(p.to_owned(), d); } }
fn mem_remove(p: &str) -> Option<Data> { MEMFS.lock().ok()?.as_mut()?.remove(p) }

// ------------------------------------------------------------------------------------------------------------------------------------------
// block cache
// ------------------------------------------------------------------------------------------------------------------------------------------

struct Cache { map: HashMap<(Arc<str>, u64), Arc<Vec<u8>>>, order: VecDeque<(Arc<str>, u64)>, bytes: usize }
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn cache_get(path: &Arc<str>, block: u64) -> Option<Arc<Vec<u8>>> {
    let g = CACHE.lock().ok()?;
    g.as_ref()?.map.get(&(path.clone(), block)).cloned()
}

fn cache_put(path: &Arc<str>, block: u64, data: Arc<Vec<u8>>) {
    let Ok(mut g) = CACHE.lock() else { return };
    let c = g.get_or_insert_with(|| Cache { map: HashMap::new(), order: VecDeque::new(), bytes: 0 });
    let key = (path.clone(), block);
    if c.map.contains_key(&key) { return; }
    c.bytes += data.len();
    c.map.insert(key.clone(), data);
    c.order.push_back(key);
    let cap = CACHE_CAP.load(Ordering::Relaxed);
    while c.bytes > cap {
        let Some(k) = c.order.pop_front() else { break };
        if let Some(d) = c.map.remove(&k) { c.bytes -= d.len(); }
    }
}

/// Drops every cached block (e.g. after loading, to give the memory back).
pub fn cache_clear() { if let Ok(mut g) = CACHE.lock() { *g = None; } }

pub fn cache_bytes() -> usize { CACHE.lock().ok().and_then(|g| g.as_ref().map(|c| c.bytes)).unwrap_or(0) }

// ------------------------------------------------------------------------------------------------------------------------------------------
// small-file bundles
// ------------------------------------------------------------------------------------------------------------------------------------------

/// Bundled: files of at most this size ...
const BUNDLE_FILE_MAX: u64 = 256 << 10;
/// ... with one of these extensions ...
const BUNDLE_EXT: [&str; 4] = ["json", "txt", "ini", "csv"];
/// ... in a folder whose bundled files add up to at most this (the launcher stops there too).
const BUNDLE_MAX: u64 = 32 << 20;

/// lower-case path -> bytes of every bundled file fetched so far
static SMALL: Mutex<Option<HashMap<String, Arc<Vec<u8>>>>> = Mutex::new(None);
/// lower-case bundle folders already asked for (or being asked for, or too big)
static BUNDLED: Mutex<Option<std::collections::HashSet<String>>> = Mutex::new(None);

fn bundled_ext(lower: &str) -> bool {
    let name = lower.rsplit('/').next().unwrap_or(lower);
    name.rsplit_once('.').is_some_and(|(_, x)| BUNDLE_EXT.contains(&x))
}

/// `/fn/assets/<folder>/...` -> `/fn/assets/<folder>` (lower-case input).
fn bundle_folder(lower: &str) -> Option<String> {
    let rest = lower.strip_prefix("/fn/assets/")?;
    let (top, below) = rest.split_once('/')?;
    (!below.is_empty()).then(|| format!("/fn/assets/{top}"))
}

/// The bytes of `path` (`size` bytes) from its asset folder's bundle: the first read of a small text file below `/fn/assets/<folder>` fetches
/// every small text file of that folder in one request (when they add up to at most [`BUNDLE_MAX`]). None = not bundled: read it normally.
fn small_file(path: &str, size: u64) -> Option<Arc<Vec<u8>>> {
    if size > BUNDLE_FILE_MAX { return None; }
    let lower = path.to_ascii_lowercase();
    if !bundled_ext(&lower) { return None; }
    let folder = bundle_folder(&lower)?;
    if let Some(d) = SMALL.lock().ok()?.as_ref().and_then(|m| m.get(&lower).cloned()) { BUNDLE_HITS.fetch_add(1, Ordering::Relaxed); return Some(d); }
    // the folder's first miss: ask for its bundle once (a concurrent first read of the same folder reads its file normally meanwhile)
    if !BUNDLED.lock().ok()?.get_or_insert_with(Default::default).insert(folder.clone()) { return None; }
    let tree = tree("/fn/assets")?;
    let prefix = format!("{folder}/");
    let total: u64 = tree.stat.iter().filter(|(k, (kind, sz, _))| *kind == 1 && *sz <= BUNDLE_FILE_MAX && k.starts_with(&prefix) && bundled_ext(k)).map(|(_, (_, sz, _))| *sz).sum();
    // REMOTE MODE: the file Worker stops a bundle at 8 MB (its CPU budget); bigger folders read file by file
    let max = if remote() { 8 << 20 } else { BUNDLE_MAX };
    if total == 0 || total > max { return None; }
    // REMOTE MODE: the bundle's content version (see [`version_of`]): its files' count, total size and newest upload time (www/fnweb-prefetch.js
    // computes the same from the tree)
    let v = if remote() {
        let (n, newest) = tree.stat.iter().filter(|(k, (kind, sz, _))| *kind == 1 && *sz <= BUNDLE_FILE_MAX && k.starts_with(&prefix) && bundled_ext(k)).fold((0u64, 0f64), |(n, m), (_, (_, _, t))| (n + 1, m.max(*t)));
        format!("&v={n}-{total}-{newest:.0}")
    } else { String::new() };
    let u = format!("{}&max={BUNDLE_FILE_MAX}&ext={}{v}", url("bundle", &exact_path(&folder)), BUNDLE_EXT.join(","));
    let body = main_wait(&u, || fetch(&u, None)).ok()??;
    let mut files = HashMap::new();
    let mut i = 0usize;
    let u32_at = |i: usize| body.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    while let Some(n) = u32_at(i) {
        let Some(rel) = body.get(i + 4..i + 4 + n) else { break };
        let Some(k) = u32_at(i + 4 + n) else { break };
        let Some(data) = body.get(i + 8 + n..i + 8 + n + k) else { break };
        files.insert(format!("{prefix}{}", String::from_utf8_lossy(rel).to_ascii_lowercase()), Arc::new(data.to_vec()));
        i += 8 + n + k;
    }
    let count = files.len();
    let mut g = SMALL.lock().ok()?;
    let m = g.get_or_insert_with(HashMap::new);
    m.extend(files);
    crate::js::fnweb_console(0, &format!("[fnweb vfs] {folder}: {count} small files ({:.1} MB) in one request", body.len() as f64 / 1048576.));
    let d = m.get(&lower).cloned();
    if d.is_some() { BUNDLE_HITS.fetch_add(1, Ordering::Relaxed); }
    d
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// handles
// ------------------------------------------------------------------------------------------------------------------------------------------

enum Node {
    /// Read-only file of the launcher.
    Remote { path: Arc<str>, size: u64 },
    /// Memory file; `persist` = write back to the launcher (PUT) on flush / close when dirty.
    Mem { path: String, data: Data, persist: bool, dirty: std::sync::atomic::AtomicBool, writable: bool },
}

static HANDLES: Mutex<Option<HashMap<u64, Arc<Node>>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn handle(h: u64) -> io::Result<Arc<Node>> {
    HANDLES.lock().ok().and_then(|g| g.as_ref()?.get(&h).cloned()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "bad file handle"))
}

fn add(n: Node) -> u64 {
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut g) = HANDLES.lock() { g.get_or_insert_with(HashMap::new).insert(h, Arc::new(n)); }
    h
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// operations (the std hooks)
// ------------------------------------------------------------------------------------------------------------------------------------------

// ------------------------------------------------------------------------------------------------------------------------------------------
// read-only trees: listed once, then stat / read_dir / exists are answered locally (case-insensitive, like the Windows folders behind them)
// ------------------------------------------------------------------------------------------------------------------------------------------

/// The read-only roots whose content cannot change while the page runs.
const TREE_ROOTS: [&str; 4] = ["/fn/paks", "/fn/assets", "/fn/bin", "/fn/windows/Fonts"];

struct Tree {
    /// lower-case absolute path -> (kind, size, mtime)
    stat: HashMap<String, (u8, u64, f64)>,
    /// lower-case absolute directory -> entries (name as on disk, kind, size, mtime)
    dirs: HashMap<String, Vec<(String, u8, u64, f64)>>,
    /// lower-case absolute path -> the path as stored (REMOTE MODE: the storage is case-sensitive)
    exact: HashMap<String, String>,
}

static TREES: Mutex<Option<HashMap<&'static str, Option<Arc<Tree>>>>> = Mutex::new(None);

fn tree_root(p: &str) -> Option<&'static str> {
    let l = p.to_ascii_lowercase();
    TREE_ROOTS.iter().copied().find(|r| { let r = r.to_ascii_lowercase(); l == r || l.starts_with(&format!("{r}/")) })
}

/// The cached tree of `root` (listed on first use; None if the listing failed: then requests go to the launcher as before).
fn tree(root: &'static str) -> Option<Arc<Tree>> {
    if let Some(t) = TREES.lock().ok()?.get_or_insert_with(HashMap::new).get(root) { return t.clone(); }
    let listing = fetch(&url("tree", root), None).ok().flatten().map(|b| String::from_utf8_lossy(&b).into_owned());
    let t = listing.map(|body| {
        let base = root.to_ascii_lowercase();
        let mut stat = HashMap::new();
        let mut dirs: HashMap<String, Vec<(String, u8, u64, f64)>> = HashMap::new();
        let mut exact = HashMap::new();
        exact.insert(base.clone(), root.to_owned());
        stat.insert(base.clone(), (2u8, 0u64, 0f64));
        dirs.entry(base.clone()).or_default();
        for line in body.lines() {
            let mut f = line.splitn(4, '\t');
            let (Some(k), Some(s), Some(m), Some(rel)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
            let (k, s, m) = (k.parse().unwrap_or(1u8), s.parse().unwrap_or(0u64), m.parse().unwrap_or(0f64));
            let abs = format!("{base}/{}", rel.to_ascii_lowercase());
            let (parent, name) = match rel.rfind('/') { Some(i) => (format!("{base}/{}", rel[..i].to_ascii_lowercase()), &rel[i + 1..]), None => (base.clone(), rel) };
            stat.insert(abs.clone(), (k, s, m));
            exact.insert(abs.clone(), format!("{root}/{rel}"));
            if k == 2 { dirs.entry(abs).or_default(); }
            dirs.entry(parent).or_default().push((name.to_owned(), k, s, m));
        }
        Arc::new(Tree { stat, dirs, exact })
    });
    if let Ok(mut g) = TREES.lock() { g.get_or_insert_with(HashMap::new).insert(root, t.clone()); }
    t
}

fn parse_stat(s: &str) -> Option<(u8, u64, f64)> {
    let mut it = s.split_whitespace();
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

pub fn stat(path: &str) -> io::Result<(u8, u64, f64)> {
    let p = normalize(path);
    if let Some(d) = mem_get(&p) { return Ok((1, d.lock().map(|v| v.len() as u64).unwrap_or(0), crate::js::fnweb_wall_ms())); }
    if memory_backed(&p) {
        if p == "/fn/tmp" || p == "/fn/user" || MEMDIRS.lock().map(|d| d.contains(&p)).unwrap_or(false) { return Ok((2, 0, 0.0)); }
        return Err(not_found(&p));
    }
    if absent(&p) { return Err(not_found(&p)); }
    if remote() && (p == "/" || p == "/fn") { return Ok((2, 0, 0.0)); }
    if let Some(t) = tree_root(&p).and_then(tree) {
        return t.stat.get(&p.to_ascii_lowercase()).copied().ok_or_else(|| not_found(&p));
    }
    match get(&url("stat", &p), None)? {
        Some(a) => parse_stat(&text(&a)).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{p}: bad stat reply"))),
        None => Err(not_found(&p)),
    }
}

pub fn readdir(path: &str) -> io::Result<Vec<(String, u8, u64, f64)>> {
    let p = normalize(path);
    let mut out = vec![];
    if memory_backed(&p) {
        let prefix = if p.ends_with('/') { p.clone() } else { format!("{p}/") };
        if let Ok(m) = MEMFS.lock() {
            for (k, d) in m.iter().flat_map(|m| m.iter()) {
                if let Some(rest) = k.strip_prefix(&prefix) { if !rest.contains('/') { out.push((rest.to_owned(), 1, d.lock().map(|v| v.len() as u64).unwrap_or(0), 0.0)); } }
            }
        }
        if let Ok(d) = MEMDIRS.lock() { for k in d.iter() { if let Some(rest) = k.strip_prefix(&prefix) { if !rest.contains('/') { out.push((rest.to_owned(), 2, 0, 0.0)); } } } }
        return Ok(out);
    }
    if absent(&p) { return Err(not_found(&p)); }
    if remote() && p == "/" { return Ok(vec![("fn".into(), 2, 0, 0.0)]); }
    if remote() && p == "/fn" { return Ok(["paks", "assets", "user", "tmp"].iter().map(|n| (n.to_string(), 2u8, 0u64, 0f64)).collect()); }
    if let Some(t) = tree_root(&p).and_then(tree) {
        return t.dirs.get(&p.to_ascii_lowercase()).cloned().ok_or_else(|| not_found(&p));
    }
    let Some(a) = get(&url("list", &p), None)? else { return Err(not_found(&p)) };
    for line in text(&a).lines() {
        let mut f = line.splitn(4, '\t');
        let (Some(k), Some(s), Some(m), Some(name)) = (f.next(), f.next(), f.next(), f.next()) else { continue };
        out.push((name.to_owned(), k.parse().unwrap_or(1), s.parse().unwrap_or(0), m.parse().unwrap_or(0.0)));
    }
    Ok(out)
}

const F_READ: u32 = 1;
const F_WRITE: u32 = 2;
const F_APPEND: u32 = 4;
const F_TRUNCATE: u32 = 8;
const F_CREATE: u32 = 16;
const F_CREATE_NEW: u32 = 32;

pub fn open(path: &str, flags: u32) -> io::Result<u64> {
    let p = normalize(path);
    let wants_write = flags & (F_WRITE | F_APPEND | F_TRUNCATE | F_CREATE | F_CREATE_NEW) != 0;
    let local = memory_backed(&p);
    let persist = writable_remote(&p);
    if !wants_write && !local {
        if let Some(data) = mem_get(&p) { return Ok(add(Node::Mem { path: p, data, persist, dirty: false.into(), writable: false })); }
        let (kind, size, _) = stat(&p)?;
        if kind == 2 { return Ok(add(Node::Remote { path: p.into(), size: 0 })); } // a directory "opened" for metadata
        return Ok(add(Node::Remote { path: exact_path(&p).into(), size }));
    }
    if wants_write && !local && !persist {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{p}: read-only in the browser build (writable: /fn/user, /fn/tmp)")));
    }
    // memory node (existing memory copy, else the launcher's copy for /fn/user, else new)
    let existing = mem_get(&p).or_else(|| {
        if !persist || remote() { return None; } // REMOTE MODE: the page loaded the saved /fn/user files into memory at start-up
        match fetch(&url("file", &p), None) { Ok(Some(a)) => Some(Arc::new(Mutex::new(a))), _ => None }
    });
    let data = match existing {
        Some(d) => {
            if flags & F_CREATE_NEW != 0 { return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{p}: exists"))); }
            if flags & F_TRUNCATE != 0 { if let Ok(mut v) = d.lock() { v.clear(); } }
            d
        }
        None => {
            if flags & (F_CREATE | F_CREATE_NEW) == 0 { return Err(not_found(&p)); }
            Arc::new(Mutex::new(Vec::new()))
        }
    };
    mem_put(&p, data.clone());
    let created = flags & (F_CREATE | F_CREATE_NEW | F_TRUNCATE) != 0;
    let _ = F_READ;
    Ok(add(Node::Mem { path: p, data, persist, dirty: created.into(), writable: wants_write }))
}

fn read_remote(path: &Arc<str>, size: u64, off: u64, buf: &mut [u8]) -> io::Result<usize> {
    if off >= size || buf.is_empty() { return Ok(0); }
    let len = (buf.len() as u64).min(size - off) as usize;
    let u = file_url(path);
    let block_size = block();
    if len as u64 >= block_size {
        if crate::pool::can_block() {
            let a = get(&u, Some((off, off + len as u64 - 1)))?.ok_or_else(|| not_found(path))?;
            let n = (a.length() as usize).min(len);
            a.subarray(0, n as u32).copy_to(&mut buf[..n]);
            return Ok(n);
        }
        let v = fetch(&u, Some((off, off + len as u64 - 1)))?.ok_or_else(|| not_found(path))?;
        let n = v.len().min(len);
        buf[..n].copy_from_slice(&v[..n]);
        return Ok(n);
    }
    let mut done = 0usize;
    while done < len {
        let pos = off + done as u64;
        let block = pos / block_size;
        let data = match cache_get(path, block) {
            Some(d) => { HITS.fetch_add(1, Ordering::Relaxed); d }
            None => {
                let start = block * block_size;
                let end = (start + block_size).min(size) - 1;
                let d = Arc::new(fetch(&u, Some((start, end)))?.ok_or_else(|| not_found(path))?);
                cache_put(path, block, d.clone());
                d
            }
        };
        let in_block = (pos - block * block_size) as usize;
        if in_block >= data.len() { break; }
        let n = (data.len() - in_block).min(len - done);
        buf[done..done + n].copy_from_slice(&data[in_block..in_block + n]);
        done += n;
    }
    Ok(done)
}

/// Main-thread reads at least this large are done by a pool worker (see [`read_at`]).
const MAIN_DELEGATE: usize = 64 << 10;

pub fn read_at(h: u64, off: u64, buf: &mut [u8]) -> io::Result<usize> {
    let node = handle(h)?;
    if let Node::Remote { path, size } = &*node {
        if let Some(d) = crate::webcook_files::file(path, *size).or_else(|| small_file(path, *size)) {
            if off >= d.len() as u64 { return Ok(0); }
            let n = (d.len() - off as usize).min(buf.len());
            buf[..n].copy_from_slice(&d[off as usize..off as usize + n]);
            return Ok(n);
        }
    }
    match &*node {
        // The Window's synchronous XHR can only return text (x-user-defined) that JS converts byte by byte: a large read on the main thread is
        // done by an I/O thread instead (ArrayBuffer XHR, copied straight into `buf`, which lives in the shared memory); the main thread spins
        // meanwhile.
        Node::Remote { path, size } if !crate::pool::can_block() && buf.len().min(size.saturating_sub(off) as usize) >= MAIN_DELEGATE => {
            main_wait(&format!("{path} @{off}+{}", buf.len()), || on_io_thread(|| read_remote(path, *size, off, buf)))
        }
        Node::Remote { path, size } => main_wait(&format!("{path} @{off}+{}", buf.len()), || read_remote(path, *size, off, buf)),
        Node::Mem { data, .. } => {
            let v = data.lock().map_err(|_| io::Error::other("poisoned"))?;
            if off >= v.len() as u64 { return Ok(0); }
            let n = (v.len() - off as usize).min(buf.len());
            buf[..n].copy_from_slice(&v[off as usize..off as usize + n]);
            Ok(n)
        }
    }
}

pub fn write_at(h: u64, off: u64, buf: &[u8]) -> io::Result<usize> {
    match &*handle(h)? {
        Node::Remote { path, .. } => Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{path}: read-only"))),
        Node::Mem { data, dirty, writable, path, .. } => {
            if !*writable { return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{path}: opened read-only"))); }
            let mut v = data.lock().map_err(|_| io::Error::other("poisoned"))?;
            let end = off as usize + buf.len();
            if v.len() < end { v.resize(end, 0); }
            v[off as usize..end].copy_from_slice(buf);
            dirty.store(true, Ordering::Relaxed);
            Ok(buf.len())
        }
    }
}

pub fn size(h: u64) -> io::Result<u64> {
    match &*handle(h)? {
        Node::Remote { size, .. } => Ok(*size),
        Node::Mem { data, .. } => Ok(data.lock().map(|v| v.len() as u64).unwrap_or(0)),
    }
}

pub fn set_len(h: u64, len: u64) -> io::Result<()> {
    match &*handle(h)? {
        Node::Remote { path, .. } => Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{path}: read-only"))),
        Node::Mem { data, dirty, .. } => { data.lock().map_err(|_| io::Error::other("poisoned"))?.resize(len as usize, 0); dirty.store(true, Ordering::Relaxed); Ok(()) }
    }
}

fn persist_node(n: &Node) -> io::Result<()> {
    if let Node::Mem { path, data, persist: true, dirty, .. } = n {
        if dirty.swap(false, Ordering::SeqCst) {
            let bytes = data.lock().map(|v| v.clone()).unwrap_or_default();
            // REMOTE MODE: the page keeps /fn/user in IndexedDB (the storage is read-only)
            if remote() { crate::js::fnweb_user_save(path, &bytes); return Ok(()); }
            send("PUT", &url("file", path), Some(&bytes))?;
        }
    }
    Ok(())
}

pub fn flush(h: u64) -> io::Result<()> { persist_node(&*handle(h)?) }

pub fn close(h: u64) {
    let n = HANDLES.lock().ok().and_then(|mut g| g.as_mut()?.remove(&h));
    if let Some(n) = n {
        if let Err(e) = persist_node(&n) { crate::js::fnweb_console(2, &format!("[fnweb vfs] write-back failed: {e}")); }
    }
}

pub fn unlink(path: &str) -> io::Result<()> {
    let p = normalize(path);
    let had = mem_remove(&p).is_some();
    if writable_remote(&p) && remote() { crate::js::fnweb_user_delete(&p); return if had { Ok(()) } else { Err(not_found(&p)) }; }
    if writable_remote(&p) { send("DELETE", &url("file", &p), None)?; return Ok(()); }
    if had { Ok(()) } else if writable_local(&p) { Err(not_found(&p)) } else { Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{p}: read-only"))) }
}

pub fn rename(from: &str, to: &str) -> io::Result<()> {
    let (a, b) = (normalize(from), normalize(to));
    if writable_remote(&a) && writable_remote(&b) && remote() {
        let d = mem_remove(&a).ok_or_else(|| not_found(&a))?;
        let bytes = d.lock().map(|v| v.clone()).unwrap_or_default();
        mem_put(&b, d);
        crate::js::fnweb_user_save(&b, &bytes);
        crate::js::fnweb_user_delete(&a);
        return Ok(());
    }
    if writable_remote(&a) && writable_remote(&b) {
        if let Some(d) = mem_remove(&a) { mem_put(&b, d); }
        send("POST", &format!("{}&to={}", url("rename", &a), enc(&b)), None)?;
        return Ok(());
    }
    if writable_local(&a) && writable_local(&b) {
        let d = mem_remove(&a).ok_or_else(|| not_found(&a))?;
        mem_put(&b, d);
        return Ok(());
    }
    Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("rename {a} -> {b}: not within one writable folder")))
}

pub fn mkdir(path: &str) -> io::Result<()> {
    let p = normalize(path);
    if memory_backed(&p) { if let Ok(mut d) = MEMDIRS.lock() { if !d.contains(&p) { d.push(p); } } return Ok(()); }
    if writable_remote(&p) { send("POST", &url("mkdir", &p), None)?; return Ok(()); }
    match stat(&p) { Ok((2, ..)) => Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{p}: exists"))), _ => Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{p}: read-only"))) }
}

pub fn rmdir(path: &str) -> io::Result<()> {
    let p = normalize(path);
    if memory_backed(&p) { if let Ok(mut d) = MEMDIRS.lock() { d.retain(|x| x != &p); } return Ok(()); }
    if writable_remote(&p) { send("POST", &url("rmdir", &p), None)?; return Ok(()); }
    Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{p}: read-only")))
}

/// Reads a whole file through the cache-bypassing path (helper for callers that want one request).
pub fn read_all(path: &str) -> io::Result<Vec<u8>> {
    let p = normalize(path);
    if let Some(d) = mem_get(&p) { return Ok(d.lock().map(|v| v.clone()).unwrap_or_default()); }
    fetch(&file_url(&exact_path(&p)), None)?.ok_or_else(|| not_found(&p))
}

/// REMOTE MODE: a `/fn/user` file the page saved in an earlier session (IndexedDB), put back before the game starts.
pub fn preload_user_file(path: &str, bytes: Vec<u8>) {
    let p = normalize(path);
    if !writable_remote(&p) { return; }
    if let Some((dir, _)) = p.rsplit_once('/') { if dir != "/fn/user" { if let Ok(mut d) = MEMDIRS.lock() { let mut cur = String::new(); for seg in dir.split('/').filter(|s| !s.is_empty()) { cur.push('/'); cur.push_str(seg); if cur.len() > "/fn/user".len() && !d.contains(&cur) { d.push(cur.clone()); } } } } }
    mem_put(&p, Arc::new(Mutex::new(bytes)));
}
