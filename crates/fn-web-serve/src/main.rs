//! `fortnite-web.exe`: the local launcher of the Fortnite 1.8 browser build.
//!
//! * serves the built page (`dist/web`: index.html, the wasm module, its JS) on `http://127.0.0.1:<port>/` with the cross-origin isolation headers the
//!   threaded wasm build needs (`Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Embedder-Policy: require-corp`);
//! * serves the user's OWN game files to that page through a small virtual file system API (`/__fnvfs/...`, see `tools/web/fn-web-rt/src/vfs.rs`) with
//!   HTTP Range support: `/fn/paks` = the 1.8 `Content/Paks` folder, `/fn/assets` = the extracted runtime assets; read-only. `/fn/user` is a writable
//!   per-user folder (`%LOCALAPPDATA%\FortniteRuntime-web\user`: settings). Nothing is uploaded anywhere: the server only listens on localhost and
//!   only answers requests whose `Host` is localhost (DNS-rebinding guard);
//! * opens the browser (Brave if installed, else the default browser) at the page.
//!
//! Folders: `--paks` / `--assets` / `--www` / `--user` flags, else `FN_PAKS` / `FN_ROOT` / `FN_ASSETS`, else a `FortniteRuntime.toml` / `paths.txt`
//! beside the exe (same format as the native exe), else the developer machine's known locations.
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

const DEFAULT_PORT: u16 = 8780;
const DEFAULT_ASSETS: &str = r"C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\assets";
const DEFAULT_PAKS: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";
const USAGE: &str = "fortnite-web [--paks <Fortnite 1.8 install or its Paks folder>] [--assets <extracted assets folder>] [--www <dist/web folder>]\n             [--user <writable data folder>] [--port <n>] [--no-browser] [--browser <exe>] [--browser-args \"...\"] [--app] [--page-args \"k=v&...\"] [--quiet]\n             test only: [--cors <origin>] [--test-password <pw>] (answer like the private file Worker of the browser build's remote mode)
             [--test-latency-ms <n>] [--test-mbps <n>] (each /__fnvfs reply delayed by n ms + its bytes at n Mbit/s: the internet path to the Worker)";

#[derive(Clone, Debug)]
struct Roots {
    www: PathBuf,
    /// virtual root name (`paks`, `assets`, `bin`, `user`) -> (folder, writable)
    vfs: Vec<(String, PathBuf, bool)>,
}

struct Stats { requests: AtomicU64, bytes: AtomicU64 }

fn exe_dir() -> PathBuf { std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from(".")) }

fn dir_has_pak(d: &Path) -> bool {
    fs::read_dir(d).map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("pak")))).unwrap_or(false)
}

/// The `Content/Paks` folder of an install given the install root, `FortniteGame`, or `Paks` itself.
fn find_paks(root: &Path) -> Option<PathBuf> {
    if dir_has_pak(root) { return Some(root.to_path_buf()); }
    ["Content/Paks", "FortniteGame/Content/Paks", "Fortnite/FortniteGame/Content/Paks", "Fortnite/Fortnite/FortniteGame/Content/Paks"].iter().map(|r| root.join(r)).find(|c| dir_has_pak(c))
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') { v[1..v.len() - 1].replace("\\\\", "\\").replace("\\\"", "\"") }
    else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') { v[1..v.len() - 1].to_owned() }
    else { v.split(" #").next().unwrap_or(v).trim().to_owned() }
}

/// `FortniteRuntime.toml` / `paths.txt` beside the exe (the native exe's format): (assets, paks).
fn config_file(dir: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    for n in ["fortnite-web.toml", "FortniteRuntime.toml", "paths.txt"] {
        let Ok(t) = fs::read_to_string(dir.join(n)) else { continue };
        let (mut a, mut p, mut bare) = (None, None, 0);
        let full = |s: String| if s.is_empty() { None } else { let x = PathBuf::from(s); Some(if x.is_absolute() { x } else { dir.join(x) }) };
        for line in t.trim_start_matches('\u{feff}').lines() {
            let l = line.trim();
            if l.is_empty() || l.starts_with('#') || l.starts_with('[') { continue; }
            match l.split_once('=') {
                Some((k, v)) => match k.trim().to_ascii_lowercase().as_str() {
                    "assets" | "assets_dir" | "asset_dir" => a = full(unquote(v)),
                    "paks" | "paks_dir" | "install" | "fortnite" | "fortnite_game" | "game" => p = full(unquote(v)),
                    _ => {}
                },
                None => { match bare { 0 => a = full(unquote(l)), 1 => p = full(unquote(l)), _ => {} } bare += 1; }
            }
        }
        return (a, p);
    }
    (None, None)
}

struct Opts { paks: Option<PathBuf>, assets: Option<PathBuf>, www: Option<PathBuf>, user: Option<PathBuf>, port: u16, browser: Option<PathBuf>, open: bool, quiet: bool, url_suffix: String, app: bool, browser_args: Vec<String>, cors: Option<String>, test_password: Option<String>, test_latency_ms: u64, test_mbps: f64 }

fn parse_args() -> Result<Opts, String> {
    let mut o = Opts { paks: None, assets: None, www: None, user: None, port: DEFAULT_PORT, browser: None, open: true, quiet: false, url_suffix: String::new(), app: false, browser_args: vec![], cors: None, test_password: None, test_latency_ms: 0, test_mbps: 0. };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match a.as_str() {
            "--paks" => o.paks = Some(val("--paks")?.into()),
            "--assets" => o.assets = Some(val("--assets")?.into()),
            "--www" => o.www = Some(val("--www")?.into()),
            "--user" => o.user = Some(val("--user")?.into()),
            "--port" => o.port = val("--port")?.parse().map_err(|_| "--port needs a number".to_owned())?,
            "--browser" => o.browser = Some(val("--browser")?.into()),
            "--no-browser" => o.open = false,
            "--quiet" => o.quiet = true,
            // TEST ONLY (tools/web/test): behave like the private file Worker of remote mode (tools/web/fn-web-worker) for a page on `origin`:
            // CORS for it, POST /login {password} -> a bearer token, every /__fnvfs request needs that token, the path may come in X-Fnvfs-Path
            "--cors" => o.cors = Some(val("--cors")?),
            "--test-password" => o.test_password = Some(val("--test-password")?),
            // TEST ONLY: the internet between the page and the file Worker (round trip + R2 first byte, and the line's bandwidth) for remote-mode measurements
            "--test-latency-ms" => o.test_latency_ms = val("--test-latency-ms")?.parse().map_err(|_| "--test-latency-ms needs a number".to_owned())?,
            "--test-mbps" => o.test_mbps = val("--test-mbps")?.parse().map_err(|_| "--test-mbps needs a number".to_owned())?,
            // extra query string for the page (e.g. --page-args "FN_TOD_HOUR=12&fnweb_quality=low")
            "--page-args" => o.url_suffix = val("--page-args")?,
            // the page opens as a normal tab of the user's browser (a running Brave gets a new tab); --app = a maximized app window instead
            // (no tabs / address bar). --tab (the old opt-in for the tab) is the default now.
            "--app" => o.app = true,
            "--tab" => o.app = false,
            // extra browser command-line arguments (space-separated), e.g. a separate profile: --browser-args "--user-data-dir=C:\tmp\p"
            "--browser-args" => o.browser_args = val("--browser-args")?.split_whitespace().map(str::to_owned).collect(),
            "-h" | "--help" => { println!("{USAGE}"); std::process::exit(0); }
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    Ok(o)
}

fn resolve(o: &Opts) -> Result<Roots, String> {
    let exe = exe_dir();
    let (cfg_assets, cfg_paks) = config_file(&exe);
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let paks_root = o.paks.clone().or_else(|| env("FN_PAKS")).or_else(|| env("FN_ROOT")).or(cfg_paks).unwrap_or_else(|| PathBuf::from(DEFAULT_PAKS));
    let paks = find_paks(&paks_root).ok_or_else(|| format!("no .pak files found under {} (give --paks <Fortnite 1.8 install or Content\\Paks>)", paks_root.display()))?;
    let assets = o.assets.clone().or_else(|| env("FN_ASSETS")).or(cfg_assets)
        .or_else(|| exe.ancestors().take(5).map(|a| a.join("FortniteRuntime").join("assets")).find(|p| p.is_dir()))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_ASSETS));
    if !assets.is_dir() { return Err(format!("the extracted assets folder {} does not exist (give --assets <folder>)", assets.display())); }
    let www = o.www.clone().unwrap_or_else(|| { let w = exe.join("web"); if w.join("index.html").is_file() { w } else { exe.join("..").join("dist").join("web") } });
    if !www.join("index.html").is_file() { return Err(format!("{} has no index.html (build with tools/web/build.sh, or give --www <dist/web>)", www.display())); }
    let user = o.user.clone().unwrap_or_else(|| env("LOCALAPPDATA").unwrap_or_else(|| exe.clone()).join("FortniteRuntime-web").join("user"));
    fs::create_dir_all(&user).map_err(|e| format!("cannot create {}: {e}", user.display()))?;
    let bin = exe.join("web-bin");
    let _ = fs::create_dir_all(&bin);
    // the system fonts the UI falls back to (Segoe UI): only the Fonts folder of WINDIR is reachable (see map_vfs)
    let windows = env("WINDIR").unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    Ok(Roots { www, vfs: vec![("paks".into(), paks, false), ("assets".into(), assets, false), ("bin".into(), bin, false), ("user".into(), user, true), ("windows".into(), windows, false)] })
}

// ------------------------------------------------------------------------------------------------------------------------------------------
// HTTP
// ------------------------------------------------------------------------------------------------------------------------------------------

struct Request { method: String, path: String, query: HashMap<String, String>, headers: HashMap<String, String>, body: Vec<u8> }

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                match u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) { Ok(v) => { out.push(v); i += 3; } Err(_) => { out.push(b'%'); i += 1; } }
            }
            b'+' => { out.push(b' '); i += 1; }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_request(r: &mut BufReader<TcpStream>) -> io::Result<Option<Request>> {
    let mut line = String::new();
    if r.read_line(&mut line)? == 0 { return Ok(None); }
    let line = line.trim_end();
    if line.is_empty() { return Ok(None); }
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else { return Err(io::Error::new(io::ErrorKind::InvalidData, "bad request line")) };
    let mut headers = HashMap::new();
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 { break; }
        let h = h.trim_end();
        if h.is_empty() { break; }
        if let Some((k, v)) = h.split_once(':') { headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned()); }
    }
    let (path, qs) = target.split_once('?').unwrap_or((target, ""));
    let mut query = HashMap::new();
    for kv in qs.split('&').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        query.insert(pct_decode(k), pct_decode(v));
    }
    let mut body = vec![];
    if let Some(n) = headers.get("content-length").and_then(|v| v.parse::<usize>().ok()) {
        if n > 256 << 20 { return Err(io::Error::new(io::ErrorKind::InvalidData, "body too large")); }
        body.resize(n, 0);
        r.read_exact(&mut body)?;
    }
    Ok(Some(Request { method: method.to_owned(), path: pct_decode(path), query, headers, body }))
}

fn content_type(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("txt") | Some("md") | Some("ini") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

thread_local! {
    /// TEST ONLY (`--cors`): CORS headers of the request being answered (added by [`head`]).
    static CORS_HEADERS: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// TEST ONLY: the file-Worker behaviour (`--cors`, `--test-password`): the allowed page origin, the password, the tokens handed out.
struct Api { cors: Option<String>, password: Option<String>, tokens: std::sync::Mutex<std::collections::HashSet<String>>, latency_ms: u64, mbps: f64 }

fn head(w: &mut impl Write, status: &str, ctype: &str, len: u64, extra: &[(&str, String)], keep: bool) -> io::Result<()> {
    let mut h = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {len}\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Embedder-Policy: require-corp\r\nCross-Origin-Resource-Policy: same-origin\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: {}\r\n", if keep { "keep-alive" } else { "close" });
    for (k, v) in extra { h.push_str(&format!("{k}: {v}\r\n")); }
    CORS_HEADERS.with(|c| h.push_str(&c.borrow()));
    h.push_str("\r\n");
    w.write_all(h.as_bytes())
}

fn reply(w: &mut impl Write, status: &str, ctype: &str, body: &[u8], keep: bool) -> io::Result<()> {
    head(w, status, ctype, body.len() as u64, &[], keep)?;
    w.write_all(body)
}

fn text(w: &mut impl Write, status: &str, msg: &str, keep: bool) -> io::Result<()> { reply(w, status, "text/plain; charset=utf-8", msg.as_bytes(), keep) }

/// `bytes=a-b` / `bytes=a-` / `bytes=-n` -> inclusive (start, end) clamped to `len`.
fn parse_range(v: &str, len: u64) -> Option<(u64, u64)> {
    let r = v.trim().strip_prefix("bytes=")?;
    let (a, b) = r.split_once('-')?;
    if len == 0 { return None; }
    let (s, e) = if a.is_empty() { let n: u64 = b.parse().ok()?; (len.saturating_sub(n), len - 1) } else { let s: u64 = a.parse().ok()?; let e: u64 = if b.is_empty() { len - 1 } else { b.parse().ok()? }; (s, e.min(len - 1)) };
    (s <= e && s < len).then_some((s, e))
}

/// Sends `path` (whole or the requested range), streaming in 1 MiB pieces.
fn send_file(w: &mut impl Write, path: &Path, range: Option<&String>, ctype: &str, keep: bool, stats: &Stats) -> io::Result<()> {
    let mut f = match File::open(path) { Ok(f) => f, Err(_) => return text(w, "404 Not Found", "not found", keep) };
    let len = f.metadata()?.len();
    let (status, start, end) = match range.and_then(|r| parse_range(r, len)) {
        Some((s, e)) => ("206 Partial Content", s, e),
        None if range.is_some() && len > 0 => return text(w, "416 Range Not Satisfiable", "bad range", keep),
        None => ("200 OK", 0, len.saturating_sub(1)),
    };
    let n = if len == 0 { 0 } else { end - start + 1 };
    let extra = if status.starts_with("206") { vec![("Content-Range", format!("bytes {start}-{end}/{len}")), ("Accept-Ranges", "bytes".to_owned())] } else { vec![("Accept-Ranges", "bytes".to_owned())] };
    head(w, status, ctype, n, &extra, keep)?;
    f.seek(SeekFrom::Start(start))?;
    let mut left = n;
    let mut buf = vec![0u8; (1 << 20).min(n.max(1) as usize)];
    while left > 0 {
        let k = (buf.len() as u64).min(left) as usize;
        f.read_exact(&mut buf[..k])?;
        w.write_all(&buf[..k])?;
        left -= k as u64;
    }
    stats.bytes.fetch_add(n, Ordering::Relaxed);
    Ok(())
}

/// Virtual path (`/fn/<root>/<rest>`) -> (real path, writable). None for anything outside the roots or with `..` / drive / stream syntax.
fn map_vfs(roots: &Roots, p: &str) -> Option<(PathBuf, bool)> {
    let rest = p.strip_prefix("/fn/")?;
    let mut segs = rest.split('/').filter(|s| !s.is_empty());
    let root = segs.next()?;
    let (_, base, writable) = roots.vfs.iter().find(|(n, ..)| n == root)?;
    let mut out = base.clone();
    let mut segs = segs.peekable();
    // %WINDIR% is served for its Fonts folder only
    if root == "windows" && !segs.peek().is_some_and(|s| s.eq_ignore_ascii_case("fonts")) { return None; }
    for s in segs {
        if s == "." || s == ".." || s.contains(':') || s.contains('\\') || s.contains('\0') { return None; }
        out.push(s);
    }
    Some((out, *writable))
}

fn mtime_ms(m: &fs::Metadata) -> u128 { m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis()) }

/// Most bytes one bundle reply carries.
const BUNDLE_MAX: u64 = 32 << 20;

/// The small files below `dir` in one body (the page asks once per asset folder instead of once per file: each request costs a round trip
/// through the browser's network service, ~3-8 ms, and the catalogs are hundreds of small JSON files). Files of at most `max` bytes whose
/// extension is one of `exts`, until [`BUNDLE_MAX`]; records: u32 LE length of the path relative to `dir` ('/'-separated, case as on disk),
/// the path, u32 LE size, the bytes.
fn bundle(dir: &Path, max: u64, exts: &[String]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, rel)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let Ok(m) = fs::metadata(e.path()) else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if m.is_dir() { stack.push((e.path(), r)); continue; }
            let wanted = name.rsplit_once('.').is_some_and(|(_, x)| exts.iter().any(|e| e.eq_ignore_ascii_case(x)));
            if !wanted || m.len() > max || body.len() as u64 + m.len() + 8 + r.len() as u64 > BUNDLE_MAX { continue; }
            let Ok(data) = fs::read(e.path()) else { continue };
            body.extend_from_slice(&(r.len() as u32).to_le_bytes());
            body.extend_from_slice(r.as_bytes());
            body.extend_from_slice(&(data.len() as u32).to_le_bytes());
            body.extend_from_slice(&data);
        }
    }
    body
}

fn vfs(w: &mut impl Write, req: &Request, roots: &Roots, keep: bool, stats: &Stats) -> io::Result<()> {
    let op = req.path.trim_start_matches("/__fnvfs/");
    let p = req.headers.get("x-fnvfs-path").cloned().or_else(|| req.query.get("p").cloned()).unwrap_or_default();
    // the virtual directories above the roots
    if op == "stat" && (p == "/" || p == "/fn") { return text(w, "200 OK", "2 0 0", keep); }
    if op == "list" && (p == "/" || p == "/fn") {
        let body = if p == "/" { "2\t0\t0\tfn\n".to_owned() } else { roots.vfs.iter().map(|(n, ..)| format!("2\t0\t0\t{n}\n")).collect() };
        return text(w, "200 OK", &body, keep);
    }
    let Some((real, writable)) = map_vfs(roots, &p) else { return text(w, "404 Not Found", "outside the served folders", keep) };
    match (req.method.as_str(), op) {
        ("GET", "stat") => match fs::metadata(&real) {
            Ok(m) => text(w, "200 OK", &format!("{} {} {}", if m.is_dir() { 2 } else { 1 }, m.len(), mtime_ms(&m)), keep),
            Err(_) => text(w, "404 Not Found", "not found", keep),
        },
        ("GET", "list") => match fs::read_dir(&real) {
            Ok(rd) => {
                let mut body = String::new();
                for e in rd.filter_map(|e| e.ok()) {
                    let Ok(m) = fs::metadata(e.path()) else { continue };
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.contains('\t') || name.contains('\n') { continue; }
                    body.push_str(&format!("{}\t{}\t{}\t{name}\n", if m.is_dir() { 2 } else { 1 }, m.len(), mtime_ms(&m)));
                }
                text(w, "200 OK", &body, keep)
            }
            Err(_) => text(w, "404 Not Found", "not found", keep),
        },
        // every file and folder below `p` (recursive, at most 200 000): lines `kind\tsize\tmtime\trelative/path` (the page caches read-only trees)
        ("GET", "tree") => {
            let mut body = String::new();
            let mut stack = vec![(real.clone(), String::new())];
            let mut n = 0usize;
            while let Some((dir, rel)) = stack.pop() {
                let Ok(rd) = fs::read_dir(&dir) else { continue };
                for e in rd.filter_map(|e| e.ok()) {
                    let Ok(m) = fs::metadata(e.path()) else { continue };
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.contains('\t') || name.contains('\n') { continue; }
                    let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                    body.push_str(&format!("{}\t{}\t{}\t{r}\n", if m.is_dir() { 2 } else { 1 }, m.len(), mtime_ms(&m)));
                    if m.is_dir() { stack.push((e.path(), r)); }
                    n += 1;
                    if n >= 200_000 { break; }
                }
            }
            text(w, "200 OK", &body, keep)
        }
        ("GET", "file") => send_file(w, &real, req.headers.get("range"), "application/octet-stream", keep, stats),
        // the small files below `p` in one reply (see `bundle`): `max` = largest file (at most 1 MiB), `ext` = comma-separated extensions
        ("GET", "bundle") => {
            let max = req.query.get("max").and_then(|v| v.parse::<u64>().ok()).unwrap_or(256 << 10).min(1 << 20);
            let exts: Vec<String> = req.query.get("ext").map(|v| v.split(',').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
            let body = bundle(&real, max, &exts);
            stats.bytes.fetch_add(body.len() as u64, Ordering::Relaxed);
            reply(w, "200 OK", "application/octet-stream", &body, keep)
        }
        (m, _) if !writable && m != "GET" => text(w, "403 Forbidden", "read-only folder", keep),
        ("PUT", "file") => {
            if let Some(d) = real.parent() { let _ = fs::create_dir_all(d); }
            let tmp = real.with_extension("fnweb-tmp");
            match fs::write(&tmp, &req.body).and_then(|_| fs::rename(&tmp, &real)) {
                Ok(_) => reply(w, "204 No Content", "text/plain", b"", keep),
                Err(e) => text(w, "500 Internal Server Error", &e.to_string(), keep),
            }
        }
        ("DELETE", "file") => match fs::remove_file(&real) { Ok(_) => reply(w, "204 No Content", "text/plain", b"", keep), Err(_) => text(w, "404 Not Found", "not found", keep) },
        ("POST", "mkdir") => match fs::create_dir_all(&real) { Ok(_) => reply(w, "204 No Content", "text/plain", b"", keep), Err(e) => text(w, "500 Internal Server Error", &e.to_string(), keep) },
        ("POST", "rmdir") => match fs::remove_dir(&real) { Ok(_) => reply(w, "204 No Content", "text/plain", b"", keep), Err(_) => text(w, "404 Not Found", "not found", keep) },
        ("POST", "rename") => {
            let to = req.query.get("to").and_then(|t| map_vfs(roots, t));
            match to { Some((dst, true)) => match fs::rename(&real, &dst) { Ok(_) => reply(w, "204 No Content", "text/plain", b"", keep), Err(_) => text(w, "404 Not Found", "not found", keep) }, _ => text(w, "403 Forbidden", "bad rename target", keep) }
        }
        _ => text(w, "405 Method Not Allowed", "unsupported", keep),
    }
}

fn host_ok(req: &Request, port: u16) -> bool {
    match req.headers.get("host") {
        Some(h) => { let h = h.to_ascii_lowercase(); [format!("127.0.0.1:{port}"), format!("localhost:{port}"), format!("[::1]:{port}")].contains(&h) }
        None => false,
    }
}

fn handle(stream: TcpStream, roots: Arc<Roots>, port: u16, stats: Arc<Stats>, quiet: bool, api: Arc<Api>) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let Ok(wstream) = stream.try_clone() else { return };
    let mut r = BufReader::with_capacity(64 << 10, stream);
    let mut w = io::BufWriter::with_capacity(256 << 10, wstream);
    loop {
        let req = match read_request(&mut r) { Ok(Some(q)) => q, _ => return };
        stats.requests.fetch_add(1, Ordering::Relaxed);
        let keep = !req.headers.get("connection").is_some_and(|c| c.eq_ignore_ascii_case("close"));
        // TEST ONLY: CORS for the allowed page origin
        let cors_ok = api.cors.as_deref().is_some_and(|o| req.headers.get("origin").is_some_and(|h| h == o));
        CORS_HEADERS.with(|c| *c.borrow_mut() = if cors_ok { format!("Access-Control-Allow-Origin: {}\r\nAccess-Control-Expose-Headers: Content-Range, Content-Length, Accept-Ranges\r\nVary: Origin\r\n", api.cors.as_deref().unwrap_or("")) } else { String::new() });
        let bearer_ok = || api.password.is_none() || req.headers.get("authorization").and_then(|v| v.strip_prefix("Bearer ")).is_some_and(|t| api.tokens.lock().map(|s| s.contains(t)).unwrap_or(false));
        let res = if !host_ok(&req, port) {
            text(&mut w, "403 Forbidden", "localhost only", false)
        } else if req.method == "OPTIONS" && cors_ok {
            head(&mut w, "204 No Content", "text/plain", 0, &[("Access-Control-Allow-Methods", "GET, HEAD, POST, OPTIONS".into()), ("Access-Control-Allow-Headers", "Authorization, Range, Content-Type, X-Fnvfs-Path".into()), ("Access-Control-Max-Age", "7200".into())], keep)
        } else if req.method == "POST" && req.path == "/login" && api.password.is_some() {
            let given = String::from_utf8_lossy(&req.body).split("\"password\"").nth(1).and_then(|r| r.split('"').nth(1)).unwrap_or("").to_owned();
            if Some(given.as_str()) == api.password.as_deref() {
                use std::hash::{BuildHasher, Hasher};
                let rnd = || std::collections::hash_map::RandomState::new().build_hasher().finish(); // randomly keyed per call: a test token
                let token = format!("{:016x}{:016x}", rnd(), rnd());
                if let Ok(mut t) = api.tokens.lock() { t.insert(token.clone()); }
                let expires = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) + 12 * 3600;
                reply(&mut w, "200 OK", "application/json", format!("{{\"token\":\"{token}\",\"expires\":{expires}}}").as_bytes(), keep)
            } else { text(&mut w, "401 Unauthorized", "wrong password", keep) }
        } else if req.path.starts_with("/__fnvfs/") && !bearer_ok() {
            text(&mut w, "401 Unauthorized", "login required", keep)
        } else if req.path.starts_with("/__fnvfs/") {
            // TEST ONLY (--test-latency-ms / --test-mbps): the reply leaves after the round trip plus its bytes at the line's rate
            let (t0, b0) = (std::time::Instant::now(), stats.bytes.load(Ordering::Relaxed));
            let r = vfs(&mut w, &req, &roots, keep, &stats);
            if api.latency_ms > 0 || api.mbps > 0. {
                let bytes = stats.bytes.load(Ordering::Relaxed).saturating_sub(b0) as f64;
                let want = Duration::from_millis(api.latency_ms) + if api.mbps > 0. { Duration::from_secs_f64(bytes * 8. / (api.mbps * 1e6)) } else { Duration::ZERO };
                if let Some(rest) = want.checked_sub(t0.elapsed()) { std::thread::sleep(rest); }
            }
            r
        } else if req.method == "GET" || req.method == "HEAD" {
            let rel = if req.path == "/" { "index.html".to_owned() } else { req.path.trim_start_matches('/').to_owned() };
            if rel.split('/').any(|s| s == ".." || s.contains(':') || s.contains('\\')) { text(&mut w, "404 Not Found", "not found", keep) }
            else {
                let p = roots.www.join(&rel);
                if !quiet && !rel.starts_with("pkg/snippets") { eprintln!("[web] GET /{rel}"); }
                send_file(&mut w, &p, req.headers.get("range"), content_type(&p), keep, &stats)
            }
        } else { text(&mut w, "405 Method Not Allowed", "unsupported", keep) };
        if res.is_err() || w.flush().is_err() || !keep { return; }
    }
}

fn find_browser(explicit: Option<&PathBuf>) -> Option<PathBuf> {
    if let Some(b) = explicit { return Some(b.clone()); }
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut c = vec![];
    for base in [env("LOCALAPPDATA"), env("ProgramFiles"), env("ProgramFiles(x86)")].into_iter().flatten() {
        c.push(base.join(r"BraveSoftware\Brave-Browser\Application\brave.exe"));
        c.push(base.join(r"Google\Chrome\Application\chrome.exe"));
    }
    c.into_iter().find(|p| p.is_file())
}

fn main() {
    let o = match parse_args() { Ok(o) => o, Err(e) => { eprintln!("error: {e}"); std::process::exit(2) } };
    let roots = match resolve(&o) { Ok(r) => r, Err(e) => { eprintln!("error: {e}"); std::process::exit(1) } };
    let listener = TcpListener::bind(("127.0.0.1", o.port)).or_else(|_| TcpListener::bind(("127.0.0.1", 0)));
    let listener = match listener { Ok(l) => l, Err(e) => { eprintln!("error: cannot listen on 127.0.0.1: {e}"); std::process::exit(1) } };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(o.port);
    let url = format!("http://127.0.0.1:{port}/{}", if o.url_suffix.is_empty() { String::new() } else { format!("?{}", o.url_suffix) });
    println!("Fortnite 1.8 browser build: {url}");
    println!("  page   {}", roots.www.display());
    for (n, p, w) in &roots.vfs { println!("  /fn/{n:<6} {}{}", p.display(), if *w { "  (writable)" } else { "" }); }
    println!("  localhost only; your game files are read from disk by this process and never leave this machine. Ctrl+C stops.");
    let roots = Arc::new(roots);
    let stats = Arc::new(Stats { requests: AtomicU64::new(0), bytes: AtomicU64::new(0) });
    let api = Arc::new(Api { cors: o.cors.clone(), password: o.test_password.clone(), tokens: Default::default(), latency_ms: o.test_latency_ms, mbps: o.test_mbps });
    if o.open {
        match find_browser(o.browser.as_ref()) {
            Some(b) => {
                // default: a normal tab (Brave already running: a new tab in its window; else Brave starts with the user's profile).
                // --app: an app window (no tabs / address bar: the game fills it; F11 = fullscreen), maximized when the browser starts fresh
                let mut cmd = std::process::Command::new(&b);
                cmd.args(&o.browser_args);
                if o.app { cmd.arg("--start-maximized").arg(format!("--app={url}")); } else { cmd.arg(&url); }
                if let Err(e) = cmd.spawn() { eprintln!("could not start {}: {e}; open {url} yourself", b.display()); }
            }
            None => { let _ = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn(); }
        }
    }
    {
        let stats = stats.clone();
        let t0 = Instant::now();
        let quiet = o.quiet;
        std::thread::spawn(move || {
            let mut last = 0;
            loop {
                std::thread::sleep(Duration::from_secs(10));
                let n = stats.requests.load(Ordering::Relaxed);
                if n != last && !quiet { println!("[web] {:>6.0} s: {n} requests, {:.1} MB served", t0.elapsed().as_secs_f32(), stats.bytes.load(Ordering::Relaxed) as f64 / 1048576.0); last = n; }
            }
        });
    }
    for s in listener.incoming() {
        let Ok(s) = s else { continue };
        let (roots, stats) = (roots.clone(), stats.clone());
        let quiet = o.quiet;
        let api = api.clone();
        let _ = std::thread::Builder::new().name("conn".into()).spawn(move || handle(s, roots, port, stats, quiet, api));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ranges() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=990-2000", 1000), Some((990, 999)));
        assert_eq!(parse_range("bytes=1000-1001", 1000), None);
        assert_eq!(parse_range("bytes=5-4", 1000), None);
    }
    #[test] fn vfs_paths_stay_inside_their_roots() {
        let r = Roots { www: "w".into(), vfs: vec![("paks".into(), PathBuf::from("C:/x/Paks"), false), ("user".into(), PathBuf::from("C:/u"), true)] };
        assert_eq!(map_vfs(&r, "/fn/paks/a.pak"), Some((PathBuf::from("C:/x/Paks").join("a.pak"), false)));
        assert_eq!(map_vfs(&r, "/fn/user/s/x.ini").map(|x| x.1), Some(true));
        for bad in ["/fn/paks/../secret", "/fn/paks/C:/Windows", "/fn/other/x", "/etc/passwd", "/fn/paks/a\\..\\b"] { assert_eq!(map_vfs(&r, bad), None, "{bad}"); }
        let w = Roots { www: "w".into(), vfs: vec![("windows".into(), PathBuf::from("C:/Windows"), false)] };
        assert!(map_vfs(&w, "/fn/windows/Fonts/segoeui.ttf").is_some());
        for bad in ["/fn/windows", "/fn/windows/System32/config", "/fn/windows/win.ini", "/fn/windows/Fonts/../win.ini"] { assert_eq!(map_vfs(&w, bad), None, "{bad}"); }
    }
    #[test] fn percent_decoding() { assert_eq!(pct_decode("%2Ffn%2Fpaks%20x"), "/fn/paks x"); assert_eq!(pct_decode("a%zz"), "a%zz"); }
    #[test] fn bundles_hold_the_small_files_with_the_asked_extensions() {
        let dir = std::env::temp_dir().join(format!("fnweb-bundle-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Kind/sub")).unwrap();
        fs::write(dir.join("a.json"), b"{\"a\":1}").unwrap();
        fs::write(dir.join("Kind/sub/B.Json"), b"[2]").unwrap();
        fs::write(dir.join("Kind/big.json"), vec![b' '; 300]).unwrap();
        fs::write(dir.join("Kind/mesh.bin"), b"xx").unwrap();
        let body = bundle(&dir, 100, &["json".into()]);
        let (mut i, mut got) = (0, vec![]);
        while i < body.len() {
            let n = u32::from_le_bytes(body[i..i + 4].try_into().unwrap()) as usize; i += 4;
            let p = String::from_utf8(body[i..i + n].to_vec()).unwrap(); i += n;
            let k = u32::from_le_bytes(body[i..i + 4].try_into().unwrap()) as usize; i += 4;
            got.push((p, body[i..i + k].to_vec())); i += k;
        }
        got.sort();
        assert_eq!(got, vec![("Kind/sub/B.Json".to_owned(), b"[2]".to_vec()), ("a.json".to_owned(), b"{\"a\":1}".to_vec())]);
        let _ = fs::remove_dir_all(&dir);
    }
}
