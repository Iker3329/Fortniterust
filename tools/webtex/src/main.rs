use fn_fx::webtex;
use fn_pak::PakSet;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const DEFAULT_PAKS: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame\Content\Paks";
const DEFAULT_ASSETS: &str = r"C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\assets";
const DEFAULT_OUT: &str = "D:/webtex/out";
const USAGE: &str = "usage: webtex [--paks <dir>] [--assets <dir>] [--out <dir>] [--roots <file>]... [--order <file>] [--threads N] [--pack-mb N] [--exclude <path prefix>]... [--list-only] [--verify N] [--verify-only N]";

struct Args {
    paks: PathBuf,
    assets: PathBuf,
    out: PathBuf,
    roots: Vec<PathBuf>,
    order: Option<PathBuf>,
    threads: usize,
    pack_bytes: u64,
    list_only: bool,
    verify: usize,
    verify_only: Option<usize>,
    exclude: Vec<String>,
}

fn args() -> Args {
    let mut a = Args { paks: DEFAULT_PAKS.into(), assets: DEFAULT_ASSETS.into(), out: DEFAULT_OUT.into(), roots: vec![], order: None, threads: std::thread::available_parallelism().map_or(8, |n| n.get()), pack_bytes: 512 << 20, list_only: false, verify: 300, verify_only: None, exclude: vec![] };
    let v: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < v.len() {
        let next = |i: usize| v.get(i + 1).cloned().unwrap_or_else(|| { eprintln!("{USAGE}"); std::process::exit(2) });
        match v[i].as_str() {
            "--paks" => { a.paks = next(i).into(); i += 1 }
            "--assets" => { a.assets = next(i).into(); i += 1 }
            "--out" => { a.out = next(i).into(); i += 1 }
            "--roots" => { a.roots.push(next(i).into()); i += 1 }
            "--exclude" => { a.exclude.push(next(i).to_ascii_lowercase()); i += 1 }
            "--order" => { a.order = Some(next(i).into()); i += 1 }
            "--threads" => { a.threads = next(i).parse().unwrap_or(a.threads); i += 1 }
            "--pack-mb" => { a.pack_bytes = next(i).parse::<u64>().unwrap_or(512) << 20; i += 1 }
            "--list-only" => a.list_only = true,
            "--verify" => { a.verify = next(i).parse().unwrap_or(300); i += 1 }
            "--verify-only" => { a.verify_only = Some(next(i).parse().unwrap_or(300)); i += 1 }
            _ => { eprintln!("{USAGE}"); std::process::exit(2) }
        }
        i += 1;
    }
    a
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { if p.file_name().map_or(false, |n| n == "webtex") { continue; } walk(&p, out) } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("json")) { out.push(p) }
    }
}

fn long_of(path: &str) -> Option<String> {
    let p = path.split(['.', ':']).next()?.trim_end_matches('/');
    if (p.starts_with("/Game/") || p.starts_with("/Engine/")) && p.len() > 7 { Some(p.to_owned()) } else { None }
}

fn scan_paths(text: &str, out: &mut BTreeSet<String>) {
    for start in ["/Game/", "/Engine/"] {
        let mut from = 0;
        while let Some(i) = text[from..].find(start) {
            let s = from + i;
            let end = text[s..].find(|c: char| !(c.is_ascii_alphanumeric() || "_/.-:".contains(c))).map_or(text.len(), |e| s + e);
            if let Some(l) = long_of(&text[s..end]) { out.insert(l); }
            from = end.max(s + 1);
        }
    }
}

fn header(set: &PakSet, long: &str) -> Option<(fn_ueio::Package, bool)> {
    let (a, map) = match set.read_game(long, "uasset") { Ok(a) => (a, false), Err(_) => (set.read_game(long, "umap").ok()?, true) };
    fn_ueio::Package::parse(&a, &[], &[]).ok().map(|p| (p, map))
}

fn entry_size(set: &PakSet, long: &str, ext: &str) -> u64 {
    fn_pak::game_path_to_pak(long, ext).and_then(|p| set.find(&p).map(|(_, e)| e.compressed_size)).unwrap_or(0)
}

struct Inventory { textures: Vec<String>, packages: usize, roots: usize, pak_bytes: BTreeMap<String, u64> }

fn inventory(set: &PakSet, a: &Args) -> Inventory {
    let mut roots = BTreeSet::new();
    let mut files = vec![];
    walk(&a.assets, &mut files);
    for f in &files { if let Ok(t) = std::fs::read_to_string(f) { scan_paths(&t, &mut roots); } }
    for f in &a.roots { if let Ok(t) = std::fs::read_to_string(f) { scan_paths(&t, &mut roots); } }
    for (_, e) in set.effective() {
        let r = e.rel_path();
        if r.to_ascii_lowercase().starts_with("fortnitegame/content/athena/maps/") && r.to_ascii_lowercase().ends_with(".umap") { if let Some(g) = fn_pak::pak_path_to_game(r) { roots.insert(g); } }
    }
    eprintln!("[webtex] {} roots ({} asset json files)", roots.len(), files.len());
    let n_roots = roots.len();
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for r in roots { if seen.insert(r.to_ascii_lowercase()) { queue.push_back(r); } }
    let mut textures = BTreeSet::new();
    let mut pak_bytes = BTreeMap::new();
    let mut packages = 0;
    while let Some(long) = queue.pop_front() {
        let Some((p, map)) = header(set, &long) else { continue };
        packages += 1;
        if packages % 5000 == 0 { eprintln!("[webtex] closure: {packages} packages, {} textures, {} queued", textures.len(), queue.len()); }
        let leaf = long.rsplit('/').next().unwrap_or(&long).to_owned();
        let mut tex_exports = vec![];
        for i in 0..p.exports.len() { if p.export_class(i) == "Texture2D" { tex_exports.push(p.exports[i].object_name.clone()); } }
        if !tex_exports.is_empty() {
            if map { for o in tex_exports { textures.insert(format!("{long}.{o}")); } } else {
                textures.insert(format!("{long}.{leaf}"));
                pak_bytes.insert(webtex::key_of(&long), ["uasset", "uexp", "ubulk"].iter().map(|x| entry_size(set, &long, x)).sum());
            }
        }
        for imp in &p.imports {
            if imp.class_name != "Package" { continue; }
            let n = &imp.object_name;
            if !(n.starts_with("/Game/") || n.starts_with("/Engine/")) { continue; }
            if seen.insert(n.to_ascii_lowercase()) { queue.push_back(n.clone()); }
        }
    }
    let mut textures: Vec<String> = textures.into_iter().filter(|t| { let l = t.to_ascii_lowercase(); !a.exclude.iter().any(|x| l.starts_with(x.as_str())) }).collect();
    if let Some(o) = &a.order {
        let first: Vec<String> = std::fs::read_to_string(o).unwrap_or_default().lines().map(|l| l.trim().to_ascii_lowercase()).filter(|l| !l.is_empty()).collect();
        let rank: std::collections::HashMap<String, usize> = first.iter().enumerate().map(|(i, k)| (k.clone(), i)).collect();
        textures.sort_by_key(|t| (rank.get(&t.to_ascii_lowercase()).or_else(|| rank.get(&fn_fx::data::long_package(t).to_ascii_lowercase())).copied().unwrap_or(usize::MAX), t.to_ascii_lowercase()));
    }
    Inventory { textures, packages, roots: n_roots, pak_bytes }
}

fn index_key(object_path: &str, map_export: bool) -> String {
    if map_export { webtex::key_of(object_path) } else { webtex::key_of(fn_fx::data::long_package(object_path)) }
}

fn is_map_export(set: &PakSet, object_path: &str) -> bool { set.read_game(fn_fx::data::long_package(object_path), "uasset").is_err() }

struct PackWriter { dir: PathBuf, limit: u64, packs: Vec<String>, cur: Option<std::io::BufWriter<std::fs::File>>, cur_len: u64, total: u64 }

impl PackWriter {
    fn write(&mut self, bytes: &[&[u8]]) -> std::io::Result<(usize, u64)> {
        let n: u64 = bytes.iter().map(|b| b.len() as u64).sum();
        if self.cur.is_none() || (self.cur_len > 0 && self.cur_len + n > self.limit) {
            if let Some(mut w) = self.cur.take() { w.flush()?; }
            let name = format!("pack-{:03}.bin", self.packs.len());
            self.cur = Some(std::io::BufWriter::with_capacity(8 << 20, std::fs::File::create(self.dir.join(&name))?));
            self.packs.push(name);
            self.cur_len = 0;
        }
        let off = self.cur_len;
        let w = self.cur.as_mut().unwrap();
        for b in bytes { w.write_all(b)?; }
        self.cur_len += n;
        self.total += n;
        Ok((self.packs.len() - 1, off))
    }
    fn finish(&mut self) -> std::io::Result<()> { if let Some(mut w) = self.cur.take() { w.flush()?; } Ok(()) }
}

fn convert(set: &PakSet, a: &Args, inv: &Inventory) -> Result<(), String> {
    std::fs::create_dir_all(&a.out).map_err(|e| format!("{}: {e}", a.out.display()))?;
    for e in std::fs::read_dir(&a.out).map_err(|e| e.to_string())?.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if (n.starts_with("pack-") && n.ends_with(".bin")) || n == webtex::INDEX_FILE || n == "index.json.tmp" { let _ = std::fs::remove_file(e.path()); }
    }
    let mut pw = PackWriter { dir: a.out.clone(), limit: a.pack_bytes, packs: vec![], cur: None, cur_len: 0, total: 0 };
    let mut entries = Map::new();
    let mut by_format: BTreeMap<&'static str, (usize, u64)> = BTreeMap::new();
    let mut errors: BTreeMap<String, usize> = BTreeMap::new();
    let mut error_samples = vec![];
    let (mut pak_total, mut out_regular) = (0u64, 0u64);
    let t0 = std::time::Instant::now();
    const CHUNK: usize = 96;
    for (ci, chunk) in inv.textures.chunks(CHUNK).enumerate() {
        let results: Vec<Mutex<Option<(bool, Option<(usize, usize)>, Result<fn_fx::gpu::Decoded, String>)>>> = chunk.iter().map(|_| Mutex::new(None)).collect();
        let next = AtomicUsize::new(0);
        std::thread::scope(|s| {
            for _ in 0..a.threads.max(1) {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= chunk.len() { break; }
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fn_fx::gpu::decode_texture_from_paks(set, &chunk[i]))).unwrap_or_else(|_| Err("panicked".into()));
                    let m = is_map_export(set, &chunk[i]);
                    let sz = if r.is_ok() { source_size(set, &chunk[i], m) } else { None };
                    *results[i].lock().unwrap() = Some((m, sz, r));
                });
            }
        });
        for (i, slot) in results.into_iter().enumerate() {
            let (map_export, src, r) = slot.into_inner().unwrap().unwrap();
            let path = &chunk[i];
            match r {
                Ok((name, info, format, levels)) => {
                    let parts: Vec<&[u8]> = levels.iter().map(|l| l.rgba8.as_slice()).collect();
                    let (pack, off) = pw.write(&parts).map_err(|e| format!("pack write: {e}"))?;
                    let n: u64 = parts.iter().map(|p| p.len() as u64).sum();
                    let f = by_format.entry(webtex::format_name(format)).or_default();
                    f.0 += 1; f.1 += n;
                    let key = index_key(path, map_export);
                    if !map_export { pak_total += inv.pak_bytes.get(&key).copied().unwrap_or(0); out_regular += n; }
                    let src = src.unwrap_or((info.width, info.height));
                    entries.insert(key, json!({
                        "op": path, "p": pack, "o": off, "f": webtex::format_name(format), "n": name, "s": info.srgb, "fi": info.filter, "ax": info.address_x, "ay": info.address_y, "c": info.compression,
                        "sz": [src.0, src.1], "l": levels.iter().map(|l| json!([l.width, l.height, l.rgba8.len()])).collect::<Vec<_>>(),
                    }));
                }
                Err(e) => {
                    let kind = e.rsplit(": ").next().unwrap_or(&e).chars().take(60).collect::<String>();
                    *errors.entry(kind).or_default() += 1;
                    if error_samples.len() < 30 { error_samples.push(e); }
                }
            }
        }
        if ci % 20 == 0 { eprintln!("[webtex] {}/{} textures, {:.0} MB written, {:.0} s", ((ci + 1) * CHUNK).min(inv.textures.len()), inv.textures.len(), pw.total as f64 / 1048576., t0.elapsed().as_secs_f32()); }
    }
    pw.finish().map_err(|e| format!("pack flush: {e}"))?;
    let index = json!({ "version": webtex::FORMAT_VERSION, "packs": pw.packs, "textures": Value::Object(entries) });
    let tmp = a.out.join("index.json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&index).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, a.out.join(webtex::INDEX_FILE)).map_err(|e| e.to_string())?;
    let n_ok: usize = by_format.values().map(|v| v.0).sum();
    println!("webtex: {n_ok} of {} textures written to {} ({} packs, {:.1} MB) in {:.0} s", inv.textures.len(), a.out.display(), pw.packs.len(), pw.total as f64 / 1048576., t0.elapsed().as_secs_f32());
    for (f, (n, b)) in &by_format { println!("  {f:<6} {n:>6} textures {:>9.1} MB", *b as f64 / 1048576.); }
    println!("  texture packages: {:.1} MB in the paks (uasset + uexp + ubulk, as stored) -> {:.1} MB here", pak_total as f64 / 1048576., out_regular as f64 / 1048576.);
    if !errors.is_empty() {
        println!("  not converted (the game gets the same error from the paks):");
        for (k, n) in &errors { println!("    {n:>6}  {k}"); }
        for e in &error_samples { println!("    e.g. {e}"); }
    }
    Ok(())
}

fn source_size(set: &PakSet, object_path: &str, map_export: bool) -> Option<(usize, usize)> {
    if map_export { return None; }
    let (a, x, b) = fn_fx::data::package_files(set, fn_fx::data::long_package(object_path)).ok()?;
    let p = fn_ueio::Package::parse(&a, &x, &b).ok()?;
    fn_texture::Texture2D::parse(&p).ok().map(|t| (t.size_x, t.size_y))
}

fn verify(set: &PakSet, out: &Path, samples: usize) -> Result<(), String> {
    verify_with(set, out, samples, true)?;
    let r = verify_with(set, out, (samples / 8).max(1), false);
    fn_render::fx::set_bc_native_hint(true);
    r
}

fn verify_with(set: &PakSet, out: &Path, samples: usize, bc: bool) -> Result<(), String> {
    fn_render::fx::set_bc_native_hint(bc);
    let ix = webtex::open_index(out)?;
    let raw: Value = serde_json::from_slice(&std::fs::read(out.join(webtex::INDEX_FILE)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let paths: std::collections::HashMap<String, String> = raw["textures"].as_object().map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v["op"].as_str()?.to_owned()))).collect()).unwrap_or_default();
    let mut keys: Vec<&String> = ix.map.keys().collect();
    keys.sort();
    let step = (keys.len() / samples.max(1)).max(1);
    let (mut ok, mut bad) = (0, 0);
    for k in keys.iter().step_by(step).take(samples) {
        let e = &ix.map[*k];
        let object = paths.get(*k).cloned().unwrap_or_else(|| (*k).clone());
        let a = fn_fx::gpu::decode_texture_from_paks(set, &object);
        let b = ix.read(e);
        let same = match (&a, &b) {
            (Ok(x), Ok(y)) => x.0 == y.0 && format!("{:?}", x.1) == format!("{:?}", y.1) && x.2 == y.2 && x.3.len() == y.3.len() && x.3.iter().zip(&y.3).all(|(p, q)| p.width == q.width && p.height == q.height && p.rgba8 == q.rgba8),
            _ => false,
        };
        if same { ok += 1 } else { bad += 1; eprintln!("[webtex] VERIFY MISMATCH {k}: paks {:?} / webtex {:?}", a.as_ref().map(|x| (&x.1, x.2, x.3.len())), b.as_ref().map(|x| (&x.1, x.2, x.3.len()))); }
    }
    println!("verify ({}): {ok} of {} sampled textures identical to the pak path (format, sampler info, every level's bytes), {bad} different", if bc { "BC adapter" } else { "no-BC adapter: BC decoded to RGBA8" }, ok + bad);
    if bad > 0 { Err(format!("{bad} mismatches")) } else { Ok(()) }
}

fn main() {
    let a = args();
    if !fn_render::fx::set_bc_native_hint(true) { eprintln!("[webtex] FN_DEBUG_NO_BC=1 is set: unset it (the packs keep the cooked BC blocks)"); std::process::exit(2); }
    let set = PakSet::open_install(&a.paks).unwrap_or_else(|e| { eprintln!("{}: {e}", a.paks.display()); std::process::exit(1) });
    if let Some(n) = a.verify_only { std::process::exit(if verify(&set, &a.out, n).is_ok() { 0 } else { 1 }); }
    let t0 = std::time::Instant::now();
    let inv = inventory(&set, &a);
    println!("inventory: {} roots, {} packages in the import closure, {} textures ({:.0} s)", inv.roots, inv.packages, inv.textures.len(), t0.elapsed().as_secs_f32());
    if a.list_only {
        let list = a.out.join("textures.txt");
        let _ = std::fs::create_dir_all(&a.out);
        let lines: Vec<String> = inv.textures.iter().map(|t| format!("{t}\t{}", inv.pak_bytes.get(&webtex::key_of(fn_fx::data::long_package(t))).copied().unwrap_or(0))).collect();
        let _ = std::fs::write(&list, lines.join("\n") + "\n");
        let pak: u64 = inv.pak_bytes.values().sum();
        println!("list: {} ({:.1} MB of texture packages in the paks)", list.display(), pak as f64 / 1048576.);
        return;
    }
    if let Err(e) = convert(&set, &a, &inv) { eprintln!("[webtex] {e}"); std::process::exit(1); }
    if a.verify > 0 { if let Err(e) = verify(&set, &a.out, a.verify) { eprintln!("[webtex] verify: {e}"); std::process::exit(1); } }
}
