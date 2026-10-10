use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

#[macro_export]
macro_rules! wgsl {
    ($p:literal) => {
        $crate::shader(include_str!($p), file!(), $p)
    };
}

static SHADER_GEN: AtomicU64 = AtomicU64::new(0);
static TUNE_GEN: AtomicU64 = AtomicU64::new(0);

pub fn shader(embedded: &'static str, file: &str, rel: &str) -> &'static str {
    #[cfg(all(fn_live_fast, not(target_arch = "wasm32")))]
    if let Some(s) = disk::shader(file, rel) {
        return s;
    }
    let _ = (file, rel);
    embedded
}

pub fn shader_root() -> Option<PathBuf> {
    #[cfg(all(fn_live_fast, not(target_arch = "wasm32")))]
    return disk::root().cloned();
    #[cfg(not(all(fn_live_fast, not(target_arch = "wasm32"))))]
    None
}

pub fn shader_generation() -> u64 {
    SHADER_GEN.load(Ordering::Relaxed)
}

pub fn tuning_generation() -> u64 {
    TUNE_GEN.load(Ordering::Relaxed)
}

pub fn poll() {
    #[cfg(all(fn_live_fast, not(target_arch = "wasm32")))]
    disk::poll_shaders();
    #[cfg(not(target_arch = "wasm32"))]
    tune::poll();
}

pub fn var(key: &str) -> Result<String, std::env::VarError> {
    match tune::get(key) {
        Some(Some(v)) => Ok(v),
        Some(None) => Err(std::env::VarError::NotPresent),
        None => std::env::var(key),
    }
}

pub fn var_os(key: &str) -> Option<OsString> {
    match tune::get(key) {
        Some(v) => v.map(OsString::from),
        None => std::env::var_os(key),
    }
}

pub fn f32(key: &str, default: f32) -> f32 {
    var(key).ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(default)
}

pub fn f32s(key: &str) -> Option<Vec<f32>> {
    let v = var(key).ok()?;
    v.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).map(|s| s.parse::<f32>().ok().filter(|x| x.is_finite())).collect()
}

pub fn tuning_path() -> Option<PathBuf> {
    tune::state().read().ok().and_then(|s| s.path.clone())
}

pub fn parse_tuning(text: &str) -> Result<HashMap<String, Option<String>>, String> {
    let mut out = HashMap::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if !line.ends_with(']') {
                return Err(format!("line {}: bad section header {line:?}", n + 1));
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { return Err(format!("line {}: expected KEY = value, got {line:?}", n + 1)) };
        let k = k.trim();
        if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("line {}: bad key {k:?}", n + 1));
        }
        let v = parse_value(v.trim()).map_err(|e| format!("line {}: {e}", n + 1))?;
        out.insert(k.to_owned(), v);
    }
    Ok(out)
}

fn parse_value(v: &str) -> Result<Option<String>, String> {
    if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let mut s = String::new();
        let mut chars = v[1..].chars();
        loop {
            match chars.next() {
                None => return Err(format!("unterminated string {v:?}")),
                Some('\\') if q == '"' => match chars.next() {
                    Some('n') => s.push('\n'),
                    Some('t') => s.push('\t'),
                    Some(c) => s.push(c),
                    None => return Err(format!("unterminated string {v:?}")),
                },
                Some(c) if c == q => break,
                Some(c) => s.push(c),
            }
        }
        let rest = chars.as_str().trim();
        if !rest.is_empty() && !rest.starts_with('#') {
            return Err(format!("text after the string: {rest:?}"));
        }
        return Ok(Some(s));
    }
    let v = v.split('#').next().unwrap_or("").trim();
    match v {
        "" => Err("missing value".to_owned()),
        "true" => Ok(Some("1".to_owned())),
        "false" => Ok(None),
        _ if v.contains(char::is_whitespace) => Err(format!("unquoted value with spaces {v:?} (quote it)")),
        _ => Ok(Some(v.to_owned())),
    }
}

mod tune {
    use super::*;

    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub struct State {
        pub path: Option<PathBuf>,
        pub mtime: Option<std::time::SystemTime>,
        pub vals: HashMap<String, Option<String>>,
        pub last_poll: Option<std::time::Instant>,
    }

    pub fn state() -> &'static RwLock<State> {
        static S: OnceLock<RwLock<State>> = OnceLock::new();
        S.get_or_init(|| {
            let path = resolve();
            let mut st = State { path, mtime: None, vals: HashMap::new(), last_poll: None };
            load(&mut st, true);
            RwLock::new(st)
        })
    }

    pub fn get(key: &str) -> Option<Option<String>> {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = key;
            None
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let s = state().read().ok()?;
            s.vals.get(key).cloned()
        }
    }

    fn resolve() -> Option<PathBuf> {
        if cfg!(target_arch = "wasm32") {
            return None;
        }
        if let Some(p) = std::env::var_os("FN_TUNING").filter(|p| !p.is_empty()) {
            return Some(PathBuf::from(p));
        }
        let args: Vec<String> = std::env::args().collect();
        for (i, a) in args.iter().enumerate() {
            if a == "--tuning" {
                if let Some(p) = args.get(i + 1) {
                    return Some(PathBuf::from(p));
                }
            } else if let Some(p) = a.strip_prefix("--tuning=") {
                return Some(PathBuf::from(p));
            }
        }
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf()));
        if exe_dir.as_ref().and_then(|d| d.file_name()).is_some_and(|n| n == "deps") {
            return None;
        }
        if let Some(p) = exe_dir.map(|d| d.join("tuning.toml")).filter(|p| p.is_file()) {
            return Some(p);
        }
        if cfg!(fn_live_fast) {
            return Some(Path::new(env!("FN_LIVE_REPO")).join("tuning.toml"));
        }
        None
    }

    fn load(st: &mut State, first: bool) {
        let Some(path) = st.path.clone() else { return };
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if !first && mtime == st.mtime {
            return;
        }
        st.mtime = mtime;
        if mtime.is_none() {
            if !st.vals.is_empty() {
                eprintln!("[fn-live] tuning {} is gone: keeping the last good values", path.display());
            } else if first {
                eprintln!("[fn-live] tuning {}: no file (environment only)", path.display());
            }
            return;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[fn-live] tuning {}: {e}; keeping the last good values", path.display());
                return;
            }
        };
        match parse_tuning(&text) {
            Ok(vals) => {
                let mut keys: Vec<String> = vals.iter().map(|(k, v)| match v { Some(v) => format!("{k}={v}"), None => format!("{k}=off") }).collect();
                keys.sort();
                let shadowed: Vec<&String> = vals.keys().filter(|k| std::env::var_os(k.as_str()).is_some()).collect();
                eprintln!("[fn-live] tuning {}{}: {} values [{}]", path.display(), if first { "" } else { " reloaded" }, vals.len(), keys.join(", "));
                if !shadowed.is_empty() {
                    eprintln!("[fn-live] tuning overrides these environment variables: {shadowed:?}");
                }
                st.vals = vals;
                TUNE_GEN.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => eprintln!("[fn-live] tuning {} {e}; keeping the last good values", path.display()),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll() {
        let now = std::time::Instant::now();
        {
            let Ok(s) = state().read() else { return };
            if s.path.is_none() || s.last_poll.is_some_and(|t| now.duration_since(t) < std::time::Duration::from_millis(250)) {
                return;
            }
        }
        let Ok(mut s) = state().write() else { return };
        s.last_poll = Some(now);
        load(&mut s, false);
    }
}

#[cfg(all(fn_live_fast, not(target_arch = "wasm32")))]
mod disk {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant, SystemTime};

    struct Entry {
        mtime: Option<SystemTime>,
        text: &'static str,
        seen: Option<SystemTime>,
    }

    struct Cache {
        files: HashMap<PathBuf, Entry>,
        missing: std::collections::HashSet<PathBuf>,
        last_poll: Option<Instant>,
        announced: bool,
    }

    fn cache() -> &'static Mutex<Cache> {
        static C: OnceLock<Mutex<Cache>> = OnceLock::new();
        C.get_or_init(|| Mutex::new(Cache { files: HashMap::new(), missing: Default::default(), last_poll: None, announced: false }))
    }

    pub fn root() -> Option<&'static PathBuf> {
        static R: OnceLock<Option<PathBuf>> = OnceLock::new();
        R.get_or_init(|| match std::env::var_os("FN_SHADER_DIR") {
            Some(v) if v == "0" || v == "off" => None,
            Some(v) if !v.is_empty() => Some(PathBuf::from(v)),
            _ => Some(PathBuf::from(env!("FN_LIVE_REPO"))),
        })
        .as_ref()
    }

    fn normalize(p: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                std::path::Component::ParentDir => {
                    if !out.pop() {
                        out.push("..");
                    }
                }
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    }

    pub fn shader(file: &str, rel: &str) -> Option<&'static str> {
        let root = root()?;
        let dir = Path::new(file).parent().unwrap_or(Path::new(""));
        let path = normalize(&root.join(dir).join(rel));
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let mut c = cache().lock().ok()?;
        if !c.announced {
            c.announced = true;
            eprintln!("[fn-live] shaders are read from {} (FN_SHADER_DIR=0 uses the embedded copies)", root.display());
        }
        let Some(mtime) = mtime else {
            if c.missing.insert(path.clone()) {
                eprintln!("[fn-live] shader {} not on disk: embedded copy used", path.display());
            }
            return None;
        };
        if let Some(e) = c.files.get(&path) {
            if e.mtime == Some(mtime) {
                return Some(e.text);
            }
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[fn-live] shader {}: {e}: embedded copy used", path.display());
                return None;
            }
        };
        let text: &'static str = Box::leak(text.into_boxed_str());
        let seen = c.files.get(&path).and_then(|e| e.seen).or(Some(mtime));
        c.files.insert(path, Entry { mtime: Some(mtime), text, seen });
        Some(text)
    }

    pub fn poll_shaders() {
        let Ok(mut c) = cache().lock() else { return };
        let now = Instant::now();
        if c.last_poll.is_some_and(|t| now.duration_since(t) < Duration::from_millis(500)) {
            return;
        }
        c.last_poll = Some(now);
        let mut changed = vec![];
        for (p, e) in c.files.iter_mut() {
            let m = std::fs::metadata(p).and_then(|m| m.modified()).ok();
            if m.is_some() && m != e.seen {
                e.seen = m;
                changed.push(p.display().to_string());
            }
        }
        if !changed.is_empty() {
            eprintln!("[fn-live] shader change: {}", changed.join(", "));
            SHADER_GEN.fetch_add(1, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_toml() {
        let v = parse_tuning("# c\n[move]\nFN_A = 1.5\nFN_B = \"x y\" # t\nFN_C = true\nFN_D = false\nFN_E = 1,2,3\n").unwrap();
        assert_eq!(v["FN_A"].as_deref(), Some("1.5"));
        assert_eq!(v["FN_B"].as_deref(), Some("x y"));
        assert_eq!(v["FN_C"].as_deref(), Some("1"));
        assert_eq!(v["FN_D"], None);
        assert_eq!(v["FN_E"].as_deref(), Some("1,2,3"));
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(parse_tuning("FN_A 1").is_err());
        assert!(parse_tuning("FN_A = \"open").is_err());
        assert!(parse_tuning("FN A = 1").is_err());
        assert!(parse_tuning("FN_A =").is_err());
        assert!(parse_tuning("FN_A = 1 2").is_err());
    }

    #[test]
    fn embedded_shader_without_disk() {
        assert_eq!(shader("x", "crates/none/src/a.rs", "../nope.wgsl"), "x");
    }

    #[test]
    fn unset_keys_fall_back_to_the_environment() {
        assert!(var("FN_LIVE_TEST_SURELY_UNSET_KEY").is_err());
        assert_eq!(f32("FN_LIVE_TEST_SURELY_UNSET_KEY", 2.5), 2.5);
    }
}
