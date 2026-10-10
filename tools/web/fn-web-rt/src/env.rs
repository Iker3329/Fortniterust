//! Environment variables and the command line of the browser build (std::env::var / args read these).
//! Defaults point the process at the virtual file system: home, the per-user app data AND the "executable" folder are `/fn/user` (the writable
//! per-user folder: the settings file lives beside the exe), temp is `/fn/tmp`, `WINDIR` is `/fn/windows` (the launcher serves only its Fonts). The page adds its URL parameters (`?FN_TOD_HOUR=12` sets `FN_TOD_HOUR`).
use std::collections::BTreeMap;
use std::sync::Mutex;

static VARS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
static ARGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn with_defaults(m: &mut BTreeMap<String, String>) {
    if m.is_empty() {
        for (k, v) in [
            ("HOME", "/fn/user"), ("USERPROFILE", "/fn/user"), ("LOCALAPPDATA", "/fn/user/AppData/Local"), ("APPDATA", "/fn/user/AppData/Roaming"),
            ("TMPDIR", "/fn/tmp"), ("TEMP", "/fn/tmp"), ("TMP", "/fn/tmp"), ("FNWEB_CWD", "/fn/user"), ("FNWEB_EXE", "/fn/user/fortnite-rs.exe"), ("WINDIR", "/fn/windows"),
        ] {
            m.insert(k.to_owned(), v.to_owned());
        }
    }
}

pub fn get(key: &str) -> Option<String> {
    let mut m = VARS.lock().ok()?;
    with_defaults(&mut m);
    m.get(key).cloned()
}

pub fn set(key: &str, value: Option<&str>) {
    if let Ok(mut m) = VARS.lock() {
        with_defaults(&mut m);
        match value { Some(v) => { m.insert(key.to_owned(), v.to_owned()); } None => { m.remove(key); } }
    }
}

pub fn all() -> Vec<(String, String)> {
    let Ok(mut m) = VARS.lock() else { return vec![] };
    with_defaults(&mut m);
    m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

/// The program name first, like a native argv.
pub fn args() -> Vec<String> { ARGS.lock().map(|a| a.clone()).unwrap_or_default() }

pub fn set_args(a: Vec<String>) { if let Ok(mut g) = ARGS.lock() { *g = a; } }

/// Copies the page's URL parameters into the environment (main thread). Returns them.
pub fn import_url_params() -> Vec<(String, String)> {
    let arr = crate::js::fnweb_url_params();
    let mut out = vec![];
    for kv in arr.iter() {
        let kv = js_sys::Array::from(&kv);
        let (Some(k), Some(v)) = (kv.get(0).as_string(), kv.get(1).as_string()) else { continue };
        set(&k, Some(&v));
        out.push((k, v));
    }
    out
}
