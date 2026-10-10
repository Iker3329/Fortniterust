//! FN-WEB-WORKER (2026-10-06): the Cloudflare Worker of the browser build's REMOTE mode. The user keeps their own copy of the 1.8 install and of the
//! port's extracted assets in a PRIVATE R2 bucket; the page (fortniteog.lol/fortnite) logs in with one password and then reads the files through
//! this Worker with the same read-only `/__fnvfs` API the local launcher (`fn-web-serve`) answers, so the page's VFS only changes its base URL
//! and adds `Authorization: Bearer <token>`.
//!
//! * `POST /login` `{"password": "..."}` -> `{"token": "...", "expires": <unix s>}` (12 h). The password is the `SITE_PASSWORD` secret, set by the
//!   user with `wrangler secret put SITE_PASSWORD`; it is never in the code, the page or a URL. The token is `<expires>.<hex HMAC-SHA256>` keyed by
//!   a hash of the password (changing the password invalidates every token).
//! * `GET|HEAD /__fnvfs/{stat,list,tree,file,bundle}` with the path in the `X-Fnvfs-Path: /fn/<root>/<path>` header (the page: one CORS preflight per
//!   op, not per file; replies `no-store`) or in `?p=` (curl / debugging) (token required): the launcher's formats (see crates/fn-web-serve
//!   `vfs`); `file` honours `Range: bytes=a-b | a- | -n` (206); `bundle` = the small files below a folder in one body (records: u32 LE path length,
//!   path, u32 LE size, bytes). Roots: `paks`, `assets`, `bin` -> the `*_PREFIX` vars of wrangler.toml. Writes are refused (the page keeps
//!   `/fn/user` and `/fn/tmp` in browser storage).
//! * CORS for the `ALLOWED_ORIGINS` var only, `Cross-Origin-Resource-Policy: cross-origin` (the page is cross-origin isolated), `Cache-Control:
//!   private` (Brave caches what it fetched; nothing is cached by shared caches).
//! Private use: there is no unauthenticated path to a file.
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use worker::*;

type HmacSha256 = Hmac<Sha256>;

/// Lifetime of a login (seconds).
const TOKEN_SECS: u64 = 12 * 3600;
/// Most bytes one bundle reply carries (the Worker copies them: kept small enough for the free plan's CPU budget; files that do not fit are read
/// one by one by the page, as with the launcher's cap).
const BUNDLE_MAX: u64 = 8 << 20;
/// Most entries one `tree` reply lists (the launcher's limit).
const TREE_MAX: usize = 200_000;
/// Parallel R2 reads of one bundle.
const BUNDLE_PARALLEL: usize = 32;

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let origin = req.headers().get("Origin")?.unwrap_or_default();
    let allowed = allowed_origin(&env, &origin);
    let mut resp = match handle(req, &env).await {
        Ok(r) => r,
        Err(e) => Response::error(format!("error: {e}"), 500)?,
    };
    let h = resp.headers_mut();
    if let Some(o) = allowed {
        h.set("Access-Control-Allow-Origin", &o)?;
        h.set("Access-Control-Allow-Methods", "GET, HEAD, POST, OPTIONS")?;
        h.set("Access-Control-Allow-Headers", "Authorization, Range, Content-Type, X-Fnvfs-Path")?;
        h.set("Access-Control-Expose-Headers", "Content-Range, Content-Length, Accept-Ranges, ETag")?;
        h.set("Access-Control-Max-Age", "7200")?;
    }
    h.append("Vary", "Origin")?;
    h.set("Cross-Origin-Resource-Policy", "cross-origin")?;
    Ok(resp)
}

/// The `Origin` when the `ALLOWED_ORIGINS` var (comma-separated) lists it.
fn allowed_origin(env: &Env, origin: &str) -> Option<String> {
    let list = env.var("ALLOWED_ORIGINS").map(|v| v.to_string()).unwrap_or_default();
    list.split(',').map(str::trim).find(|o| !o.is_empty() && o.eq_ignore_ascii_case(origin)).map(|_| origin.to_owned())
}

async fn handle(mut req: Request, env: &Env) -> Result<Response> {
    let url = req.url()?;
    let path = url.path().to_owned();
    match (req.method(), path.as_str()) {
        (Method::Options, _) => Response::empty().map(|r| r.with_status(204)),
        (Method::Post, "/login") => login(&mut req, env).await,
        (Method::Get | Method::Head, p) if p.starts_with("/__fnvfs/") => {
            if !authorized(&req, env)? { return Response::error("login required", 401); }
            let op = p.trim_start_matches("/__fnvfs/").to_owned();
            let q = |k: &str| url.query_pairs().find(|(n, _)| n == k).map(|(_, v)| v.into_owned());
            // the page sends the path in `X-Fnvfs-Path` (one URL per op = one CORS preflight per op instead of one per file); `?p=` for curl / debugging
            let from_header = req.headers().get("X-Fnvfs-Path")?;
            let header_path = from_header.is_some();
            let p = from_header.or_else(|| q("p")).unwrap_or_default();
            let mut resp = vfs(&req, env, &op, &p, q("max"), q("ext")).await?;
            // one URL serves every path: the HTTP cache must not keep it (the page has its own range cache)
            if header_path { resp.headers_mut().set("Cache-Control", "no-store")?; resp.headers_mut().append("Vary", "X-Fnvfs-Path")?; }
            Ok(resp)
        }
        (_, p) if p.starts_with("/__fnvfs/") => Response::error("read-only", 405),
        _ => Response::error("not found", 404),
    }
}

// ---- login --------------------------------------------------------------------------------------------------------------------------------

/// The `SITE_PASSWORD` secret; empty while it is not set (every login and token is then refused with 401).
fn password(env: &Env) -> Result<String> { Ok(env.secret("SITE_PASSWORD").map(|s| s.to_string()).unwrap_or_default()) }
/// The token key: a hash of the password, so a new password invalidates every token.
fn token_key(pw: &str) -> [u8; 32] { Sha256::digest(format!("fn-web-worker token v1\0{pw}").as_bytes()).into() }
fn sign(pw: &str, expires: u64) -> String {
    let mut m = HmacSha256::new_from_slice(&token_key(pw)).expect("any key length");
    m.update(format!("fnweb-token:{expires}").as_bytes());
    hex::encode(m.finalize().into_bytes())
}
fn now_secs() -> u64 { Date::now().as_millis() / 1000 }

async fn login(req: &mut Request, env: &Env) -> Result<Response> {
    let body: serde_json::Value = match req.json().await { Ok(v) => v, Err(_) => return Response::error("expected {\"password\": \"...\"}", 400) };
    let given = body.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let pw = password(env)?;
    // constant-time comparison: HMAC both with one key and compare the digests with `verify_slice`
    let digest = |s: &str| { let mut m = HmacSha256::new_from_slice(b"fn-web-worker compare").expect("any key length"); m.update(s.as_bytes()); m };
    let expected = digest(&pw).finalize().into_bytes();
    let public = env.var("PUBLIC").map(|v| v.to_string() == "1").unwrap_or(false);
    if pw.is_empty() || (!public && digest(given).verify_slice(&expected).is_err()) {
        Delay::from(std::time::Duration::from_millis(400)).await; // slows guessing down
        return Response::error("wrong password", 401);
    }
    let expires = now_secs() + TOKEN_SECS;
    Response::from_json(&serde_json::json!({ "token": format!("{expires}.{}", sign(&pw, expires)), "expires": expires }))
}

fn authorized(req: &Request, env: &Env) -> Result<bool> {
    let Some(auth) = req.headers().get("Authorization")? else { return Ok(false) };
    let Some(token) = auth.strip_prefix("Bearer ") else { return Ok(false) };
    let Some((exp, sig)) = token.trim().split_once('.') else { return Ok(false) };
    let (Ok(expires), Ok(sig)) = (exp.parse::<u64>(), hex::decode(sig)) else { return Ok(false) };
    if expires <= now_secs() { return Ok(false); }
    let pw = password(env)?;
    if pw.is_empty() { return Ok(false); }
    let mut m = HmacSha256::new_from_slice(&token_key(&pw)).expect("any key length");
    m.update(format!("fnweb-token:{expires}").as_bytes());
    Ok(m.verify_slice(&sig).is_ok())
}

// ---- the /__fnvfs API over R2 ----------------------------------------------------------------------------------------------------------------

const ROOTS: [(&str, &str); 3] = [("paks", "PAKS_PREFIX"), ("assets", "ASSETS_PREFIX"), ("bin", "BIN_PREFIX")];

/// `/fn/<root>/<rel>` -> (R2 key prefix of the root, rel); `None` outside the served roots or for `..` segments.
fn map(env: &Env, p: &str) -> Option<(String, String)> {
    let rest = p.strip_prefix("/fn/")?;
    let (root, rel) = rest.split_once('/').unwrap_or((rest, ""));
    let (_, var) = ROOTS.iter().find(|(n, _)| *n == root)?;
    let prefix = env.var(var).ok()?.to_string();
    if prefix.is_empty() { return None; }
    let rel = rel.trim_matches('/').to_owned();
    if rel.split('/').any(|s| s == ".." || s == ".") { return None; }
    Some((prefix, rel))
}
fn key_of(prefix: &str, rel: &str) -> String { format!("{prefix}{rel}") }
fn dir_prefix(prefix: &str, rel: &str) -> String { if rel.is_empty() { prefix.to_owned() } else { format!("{prefix}{rel}/") } }
fn text(body: String) -> Result<Response> { Response::ok(body) }
fn private_cache(r: &mut Response, secs: u32) -> Result<()> { r.headers_mut().set("Cache-Control", &format!("private, max-age={secs}")) }

async fn vfs(req: &Request, env: &Env, op: &str, p: &str, max: Option<String>, ext: Option<String>) -> Result<Response> {
    // the virtual directories above the roots
    if p == "/" || p == "/fn" {
        return match op {
            "stat" => text("2 0 0".into()),
            "list" => text(if p == "/" { "2\t0\t0\tfn\n".into() } else { ROOTS.iter().filter(|(_, v)| env.var(v).map(|x| !x.to_string().is_empty()).unwrap_or(false)).map(|(n, _)| format!("2\t0\t0\t{n}\n")).collect() }),
            _ => Response::error("not found", 404),
        };
    }
    let Some((prefix, rel)) = map(env, p) else { return Response::error("outside the served folders", 404) };
    let bucket = env.bucket("FILES")?;
    match op {
        "stat" => {
            if !rel.is_empty() { if let Some(o) = bucket.head(key_of(&prefix, &rel)).await? { return text(format!("1 {} {}", o.size(), o.uploaded().as_millis())); } }
            let l = bucket.list().prefix(dir_prefix(&prefix, &rel)).limit(1).execute().await?;
            if rel.is_empty() || !l.objects().is_empty() || !l.delimited_prefixes().is_empty() { text("2 0 0".into()) } else { Response::error("not found", 404) }
        }
        "list" => {
            let dp = dir_prefix(&prefix, &rel);
            let mut body = String::new();
            let mut cursor: Option<String> = None;
            loop {
                let mut b = bucket.list().prefix(dp.clone()).delimiter("/");
                if let Some(c) = cursor.take() { b = b.cursor(c); }
                let page = b.execute().await?;
                for d in page.delimited_prefixes() { let name = d[dp.len()..].trim_end_matches('/'); if !name.is_empty() { body.push_str(&format!("2\t0\t0\t{name}\n")); } }
                for o in page.objects() { let k = o.key(); let name = &k[dp.len()..]; if !name.is_empty() { body.push_str(&format!("1\t{}\t{}\t{name}\n", o.size(), o.uploaded().as_millis())); } }
                if !page.truncated() { break; }
                cursor = page.cursor();
                if cursor.is_none() { break; }
            }
            let mut r = text(body)?; private_cache(&mut r, 3600)?; Ok(r)
        }
        "tree" => {
            let dp = dir_prefix(&prefix, &rel);
            let mut body = String::new();
            let mut dirs = std::collections::HashSet::new();
            let mut n = 0usize;
            let mut cursor: Option<String> = None;
            'pages: loop {
                let mut b = bucket.list().prefix(dp.clone());
                if let Some(c) = cursor.take() { b = b.cursor(c); }
                let page = b.execute().await?;
                for o in page.objects() {
                    let k = o.key();
                    let r = &k[dp.len()..];
                    if r.is_empty() || r.ends_with('/') { continue; }
                    // every folder above the file, once
                    let mut at = 0;
                    while let Some(i) = r[at..].find('/') { let d = &r[..at + i]; if dirs.insert(d.to_owned()) { body.push_str(&format!("2\t0\t0\t{d}\n")); n += 1; } at += i + 1; }
                    body.push_str(&format!("1\t{}\t{}\t{r}\n", o.size(), o.uploaded().as_millis()));
                    n += 1;
                    if n >= TREE_MAX { break 'pages; }
                }
                if !page.truncated() { break; }
                cursor = page.cursor();
                if cursor.is_none() { break; }
            }
            let mut r = text(body)?; private_cache(&mut r, 3600)?; Ok(r)
        }
        "file" => file(req, &bucket, &key_of(&prefix, &rel)).await,
        "bundle" => {
            let max = max.and_then(|v| v.parse::<u64>().ok()).unwrap_or(256 << 10).min(1 << 20);
            let exts: Vec<String> = ext.map(|v| v.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect()).unwrap_or_default();
            let dp = dir_prefix(&prefix, &rel);
            // the wanted files, up to the cap
            let mut wanted: Vec<(String, String, u64)> = vec![]; // (key, rel path, size)
            let mut total = 0u64;
            let mut cursor: Option<String> = None;
            'list: loop {
                let mut b = bucket.list().prefix(dp.clone());
                if let Some(c) = cursor.take() { b = b.cursor(c); }
                let page = b.execute().await?;
                for o in page.objects() {
                    let k = o.key();
                    let r = k[dp.len()..].to_owned();
                    let name = r.rsplit('/').next().unwrap_or(&r);
                    let ok = name.rsplit_once('.').is_some_and(|(_, x)| exts.iter().any(|e| e.eq_ignore_ascii_case(x)));
                    if !ok || o.size() > max { continue; }
                    if total + o.size() + 8 + r.len() as u64 > BUNDLE_MAX { break 'list; }
                    total += o.size() + 8 + r.len() as u64;
                    wanted.push((k, r, o.size()));
                }
                if !page.truncated() { break; }
                cursor = page.cursor();
                if cursor.is_none() { break; }
            }
            let mut body = Vec::with_capacity(total as usize);
            for chunk in wanted.chunks(BUNDLE_PARALLEL) {
                let reads = chunk.iter().map(|(k, _, _)| { let b = &bucket; async move { match b.get(k.clone()).execute().await { Ok(Some(o)) => match o.body() { Some(body) => body.bytes().await.ok(), None => None }, _ => None } } });
                let got = futures::future::join_all(reads).await;
                for ((_, r, _), data) in chunk.iter().zip(got) {
                    let Some(data) = data else { continue };
                    body.extend_from_slice(&(r.len() as u32).to_le_bytes());
                    body.extend_from_slice(r.as_bytes());
                    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
                    body.extend_from_slice(&data);
                }
            }
            let mut resp = Response::from_bytes(body)?;
            resp.headers_mut().set("Content-Type", "application/octet-stream")?;
            private_cache(&mut resp, 3600)?;
            Ok(resp)
        }
        _ => Response::error("unsupported", 405),
    }
}

/// `Range: bytes=a-b | a- | -n` -> (offset, length) within `size`; `None` = the whole object; `Err` = unsatisfiable.
fn parse_range(h: Option<String>, size: u64) -> std::result::Result<Option<(u64, u64)>, ()> {
    let Some(h) = h else { return Ok(None) };
    let Some(spec) = h.trim().strip_prefix("bytes=") else { return Ok(None) };
    if spec.contains(',') { return Ok(None); } // multi-range: the whole object, as allowed
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => { let n: u64 = n.parse().map_err(|_| ())?; if n == 0 { return Err(()); } (size.saturating_sub(n), size.saturating_sub(1)) }
        (a, "") => (a.parse().map_err(|_| ())?, size.saturating_sub(1)),
        (a, b) => (a.parse().map_err(|_| ())?, b.parse::<u64>().map_err(|_| ())?.min(size.saturating_sub(1))),
    };
    if size == 0 || start >= size || end < start { return Err(()); }
    Ok(Some((start, end - start + 1)))
}

fn edge_key(key: &str, range: &str) -> String {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_./".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    format!("https://fnweb-edge.invalid/{}?r={}", enc(key), enc(range))
}

async fn file(req: &Request, bucket: &Bucket, key: &str) -> Result<Response> {
    if req.method() == Method::Get {
        let ck = edge_key(key, &req.headers().get("Range")?.unwrap_or_default());
        let cache = Cache::default();
        if let Some(mut hit) = cache.get(ck.as_str(), true).await? {
            let src = hit.headers().clone();
            let ranged = src.get("Content-Range")?;
            let data = hit.bytes().await?;
            let mut resp = Response::from_bytes(data)?.with_status(if ranged.is_some() { 206 } else { 200 });
            let h = resp.headers_mut();
            for name in ["Content-Type", "Accept-Ranges", "ETag", "Content-Range", "Content-Length"] {
                if let Some(v) = src.get(name)? { h.set(name, &v)?; }
            }
            private_cache(&mut resp, 86400)?;
            return Ok(resp);
        }
        let mut resp = file_r2(req, bucket, key).await?;
        let status = resp.status_code();
        let len: u64 = resp.headers().get("Content-Length")?.and_then(|v| v.parse().ok()).unwrap_or(u64::MAX);
        if (status == 200 || status == 206) && len <= 33_554_432 {
            let src = resp.headers().clone();
            let data = resp.bytes().await?;
            let mut copy = Response::from_bytes(data.clone())?.with_status(200);
            let mut out = Response::from_bytes(data)?.with_status(status);
            for name in ["Content-Type", "Accept-Ranges", "ETag", "Content-Range", "Content-Length", "Cache-Control"] {
                if let Some(v) = src.get(name)? { copy.headers_mut().set(name, &v)?; out.headers_mut().set(name, &v)?; }
            }
            copy.headers_mut().set("Cache-Control", "public, max-age=2592000")?;
            let _ = cache.put(ck.as_str(), copy).await;
            return Ok(out);
        }
        return Ok(resp);
    }
    file_r2(req, bucket, key).await
}

async fn file_r2(req: &Request, bucket: &Bucket, key: &str) -> Result<Response> {
    let Some(head) = bucket.head(key).await? else { return Response::error("not found", 404) };
    let size = head.size();
    let range = match parse_range(req.headers().get("Range")?, size) {
        Ok(r) => r,
        Err(()) => { let mut r = Response::error("range not satisfiable", 416)?; r.headers_mut().set("Content-Range", &format!("bytes */{size}"))?; return Ok(r); }
    };
    let etag = head.http_etag();
    let mut resp = if req.method() == Method::Head {
        Response::empty()?.with_status(if range.is_some() { 206 } else { 200 })
    } else {
        let mut g = bucket.get(key);
        if let Some((offset, length)) = range { g = g.range(Range::OffsetWithLength { offset, length }); }
        let Some(obj) = g.execute().await? else { return Response::error("not found", 404) };
        let Some(body) = obj.body() else { return Response::error("no body", 500) };
        Response::from_body(body.response_body()?)?.with_status(if range.is_some() { 206 } else { 200 })
    };
    let h = resp.headers_mut();
    h.set("Content-Type", "application/octet-stream")?;
    h.set("Accept-Ranges", "bytes")?;
    h.set("ETag", &etag)?;
    match range {
        Some((offset, length)) => { h.set("Content-Range", &format!("bytes {offset}-{}/{size}", offset + length - 1))?; h.set("Content-Length", &length.to_string())?; }
        None => { h.set("Content-Length", &size.to_string())?; }
    }
    private_cache(&mut resp, 86400)?;
    Ok(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn ranges_parse_like_http() {
        assert_eq!(parse_range(None, 100), Ok(None));
        assert_eq!(parse_range(Some("bytes=0-9".into()), 100), Ok(Some((0, 10))));
        assert_eq!(parse_range(Some("bytes=90-".into()), 100), Ok(Some((90, 10))));
        assert_eq!(parse_range(Some("bytes=-5".into()), 100), Ok(Some((95, 5))));
        assert_eq!(parse_range(Some("bytes=50-500".into()), 100), Ok(Some((50, 50))));
        assert_eq!(parse_range(Some("bytes=100-".into()), 100), Err(()));
        assert_eq!(parse_range(Some("bytes=0-1,5-6".into()), 100), Ok(None));
    }
    #[test] fn tokens_are_bound_to_the_password_and_the_expiry() {
        let (a, b) = (sign("hunter2", 1000), sign("hunter2", 1001));
        assert_ne!(a, b);
        assert_ne!(sign("other", 1000), a);
        assert_eq!(a.len(), 64);
    }
}
