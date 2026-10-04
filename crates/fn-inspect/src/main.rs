//! `fn-inspect` - inspect a Fortnite pak install and its packages (pure Rust).
//!
//! ```text
//! fn-inspect [--root <install>] [--ue4-version N] <command> ...
//!   ls <glob> [-l]              list files (case-insensitive glob over engine-relative paths)
//!   cat <path>                  write a file's bytes to stdout
//!   info <package>              package header summary (JSON)
//!   exports <package>           export table with class names (JSON)
//!   props <package> [export]    tagged properties as JSON (export = index or name; default all; `cdo` = class default object)
//!   classes <package>           UClass/UFunction decoding (JSON)
//!   disasm <package> [fn]       Kismet bytecode disassembly of every (or one) function export
//!   paks                        per-pak summary (version, mount, entry/encrypted/compressed counts)
//!   index                       pak index as JSON on stdout (same schema as the Node tool's pak-index.json)
//!   diff-index <A> <B>          compare two indexes (install dirs or pak-index.json files)
//!   sweep [options]             parse every .uasset/.umap in the install (rayon) and report coverage
//! ```

use fn_pak::PakSet;
use fn_uasset::{Package, PackageData};
use rayon::prelude::*;
use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";

struct Opts {
    root: PathBuf,
    ue4_version: Option<i32>,
    args: Vec<String>,
}

fn parse_args() -> Opts {
    let mut root = PathBuf::from(std::env::var("FN_ROOT").unwrap_or_else(|_| DEFAULT_ROOT.to_string()));
    let mut ue4_version = std::env::var("FN_UE4_VERSION").ok().and_then(|v| v.parse().ok());
    let mut args = Vec::new();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--root" => root = PathBuf::from(it.next().unwrap_or_default()),
            "--ue4-version" => ue4_version = it.next().and_then(|v| v.parse().ok()),
            _ => args.push(a),
        }
    }
    Opts { root, ue4_version, args }
}

fn die(msg: &str) -> ! {
    eprintln!("fn-inspect: {msg}");
    std::process::exit(1);
}

fn open(o: &Opts) -> PakSet {
    match PakSet::open_install(&o.root) {
        Ok(s) => {
            for (n, e) in &s.skipped {
                eprintln!("warning: skipped {n}: {e}");
            }
            s
        }
        Err(e) => die(&format!("cannot open install {}: {e}", o.root.display())),
    }
}

fn parse_opts(o: &Opts) -> fn_uasset::package::ParseOptions {
    let mut p = fn_uasset::package::ParseOptions::default();
    if let Some(v) = o.ue4_version {
        p.assumed_ue4_version = v;
    }
    p
}

fn main() {
    let o = parse_args();
    let cmd = o.args.first().cloned().unwrap_or_default();
    match cmd.as_str() {
        "ls" => cmd_ls(&o),
        "cat" => cmd_cat(&o),
        "info" => cmd_info(&o),
        "exports" => cmd_exports(&o),
        "props" => cmd_props(&o),
        "classes" => cmd_classes(&o),
        "disasm" => cmd_disasm(&o),
        "paks" => cmd_paks(&o),
        "index" => cmd_index(&o),
        "diff-index" => cmd_diff_index(&o),
        "sweep" => cmd_sweep(&o),
        _ => {
            eprintln!("{}", include_str!("usage.txt"));
            std::process::exit(2);
        }
    }
}

// ------------------------------------------------------------------------------------------------

fn cmd_ls(o: &Opts) {
    let pat = o.args.get(1).cloned().unwrap_or_else(|| "**".into());
    let long = o.args.iter().any(|a| a == "-l");
    let set = open(o);
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for (pf, e) in set.glob(&pat) {
        if long {
            let _ = writeln!(w, "{:>12} {:<32} {}", e.size, pf.name, e.rel_path());
        } else {
            let _ = writeln!(w, "{}", e.rel_path());
        }
    }
}

fn cmd_cat(o: &Opts) {
    let p = o.args.get(1).unwrap_or_else(|| die("cat <path>"));
    let set = open(o);
    let path = if p.starts_with("/Game/") || p.starts_with("/Engine/") { fn_pak::game_path_to_pak(p, "uasset").unwrap() } else { p.clone() };
    match set.read(&path) {
        Ok(b) => {
            let _ = std::io::stdout().write_all(&b);
        }
        Err(e) => die(&e.to_string()),
    }
}

fn load<'a>(set: &'a PakSet, o: &Opts, p: &str) -> PackageData<'a> {
    let _ = o;
    PackageData::load(set, p).unwrap_or_else(|e| die(&format!("{p}: {e}")))
}

fn pkg_of<'d>(d: &'d PackageData<'_>, o: &Opts) -> Package<'d> {
    Package::parse_with(&d.asset, &d.uexp, parse_opts(o)).unwrap_or_else(|e| die(&format!("{}: {e}", d.name))).with_name(d.name.clone())
}

fn cmd_info(o: &Opts) {
    let p = o.args.get(1).unwrap_or_else(|| die("info <package>"));
    let set = open(o);
    let d = load(&set, o, p);
    let pkg = pkg_of(&d, o);
    let s = &pkg.summary;
    let j = json!({
        "package": d.name,
        "assetBytes": d.asset.len(), "uexpBytes": d.uexp.len(),
        "legacyFileVersion": s.legacy_file_version, "fileVersionUE4": s.file_version_ue4, "effectiveVersion": pkg.effective_version,
        "fileVersionLicenseeUE4": s.file_version_licensee_ue4, "customVersions": s.custom_versions.len(),
        "totalHeaderSize": s.total_header_size, "packageFlags": s.package_flags,
        "names": pkg.names.len(), "imports": pkg.imports.len(), "exports": pkg.exports.len(),
        "preloadDependencies": pkg.preload_dependencies.len(), "softPackageReferences": pkg.soft_package_references,
        "thumbnails": pkg.thumbnails.len(), "bulkDataStartOffset": s.bulk_data_start_offset,
        "warnings": pkg.warnings,
    });
    println!("{}", serde_json::to_string_pretty(&j).unwrap());
}

fn cmd_exports(o: &Opts) {
    let p = o.args.get(1).unwrap_or_else(|| die("exports <package>"));
    let set = open(o);
    let d = load(&set, o, p);
    let pkg = pkg_of(&d, o);
    let list: Vec<Json> = pkg
        .exports
        .iter()
        .enumerate()
        .map(|(i, e)| {
            json!({"index": i, "name": e.name, "class": pkg.export_class_name(i), "super": pkg.object_path(e.super_index), "outer": pkg.object_path(e.outer_index),
                   "offset": e.serial_offset, "size": e.serial_size, "flags": e.object_flags})
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&list).unwrap());
}

fn cmd_props(o: &Opts) {
    let p = o.args.get(1).unwrap_or_else(|| die("props <package> [export]"));
    let sel = o.args.get(2).cloned();
    let set = open(o);
    let d = load(&set, o, p);
    let pkg = pkg_of(&d, o);
    let objs: Vec<usize> = match sel.as_deref() {
        None => (0..pkg.exports.len()).collect(),
        Some("cdo") => vec![pkg.default_object_export().unwrap_or_else(|| die("no class default object export"))],
        Some(s) => match s.parse::<usize>() {
            Ok(i) if i < pkg.exports.len() => vec![i],
            _ => vec![pkg.find_export(s).unwrap_or_else(|| die(&format!("no export named {s}")))],
        },
    };
    let mut out = Vec::new();
    for i in objs {
        match pkg.read_export(i) {
            Ok(obj) => out.push(obj.to_json(&pkg)),
            Err(e) => out.push(json!({"index": i, "error": e.to_string()})),
        }
    }
    let j = if sel.is_some() && out.len() == 1 { out.pop().unwrap() } else { json!({"package": d.name, "exports": out}) };
    println!("{}", serde_json::to_string_pretty(&j).unwrap());
}

fn cmd_classes(o: &Opts) {
    let p = o.args.get(1).unwrap_or_else(|| die("classes <package>"));
    let set = open(o);
    let d = load(&set, o, p);
    let pkg = pkg_of(&d, o);
    println!("{}", serde_json::to_string_pretty(&fn_uasset::classes::package_classes_json(&pkg)).unwrap());
}

fn cmd_disasm(o: &Opts) {
    let p = o.args.get(1).unwrap_or_else(|| die("disasm <package> [function]"));
    let want = o.args.get(2).cloned();
    let set = open(o);
    let d = load(&set, o, p);
    let pkg = pkg_of(&d, o);
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::new(out.lock());
    for i in 0..pkg.exports.len() {
        let cn = pkg.export_class_name(i);
        if cn != "Function" && cn != "DelegateFunction" {
            continue;
        }
        if let Some(wn) = &want {
            if &pkg.exports[i].name != wn && wn != &i.to_string() {
                continue;
            }
        }
        match fn_uasset::classes::read_function(&pkg, i) {
            Ok(f) => {
                let _ = writeln!(w, "== [{}] {} (flags {:#x}, script {} bytes)", i, pkg.exports[i].name, f.function_flags, f.script.len());
                let dis = fn_uasset::kismet::disassemble(&pkg, f.script);
                for (off, text) in &dis.lines {
                    let _ = writeln!(w, "  {off:5x}: {text}");
                }
                if let Some(e) = &dis.error {
                    let _ = writeln!(w, "  !! {e}");
                    let last = dis.lines.last().map(|l| l.0).unwrap_or(0);
                    let hi = (last + 64).min(f.script.len());
                    let _ = writeln!(w, "  bytes from last good statement ({last:#x}): {}", f.script[last..hi].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "));
                }
            }
            Err(e) => {
                let _ = writeln!(w, "== [{}] {}: error {e}", i, pkg.exports[i].name);
            }
        }
    }
}

// ------------------------------------------------------------------------------------------------
// paks / index / diff
// ------------------------------------------------------------------------------------------------

fn cmd_paks(o: &Opts) {
    let set = open(o);
    let list: Vec<Json> = set
        .paks()
        .iter()
        .map(|p| {
            let enc = p.entries.iter().filter(|e| e.encrypted).count();
            let comp = p.entries.iter().filter(|e| e.method != 0).count();
            json!({
                "pak": p.name, "version": p.info.version, "mount": p.mount, "entries": p.entries.len(),
                "encryptedEntries": enc, "compressedEntries": comp, "encryptedIndex": p.info.encrypted_index,
                "indexOffset": p.info.index_offset, "indexSize": p.info.index_size, "fileBytes": p.bytes().len(),
            })
        })
        .collect();
    let j = json!({"install": o.root.display().to_string(), "paks": list, "totalEntries": set.total_entries(), "effectivePaths": set.len(),
                   "skipped": set.skipped.iter().map(|(n, e)| json!({"pak": n, "error": e})).collect::<Vec<_>>()});
    println!("{}", serde_json::to_string_pretty(&j).unwrap());
}

fn jstr(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

fn cmd_index(o: &Opts) {
    let set = open(o);
    let mut paks: Vec<&fn_pak::PakFile> = set.paks().iter().collect();
    paks.sort_by(|a, b| a.name.cmp(&b.name));
    let out = std::io::stdout();
    let mut w = std::io::BufWriter::with_capacity(1 << 20, out.lock());
    let _ = w.write_all(b"[");
    let mut first = true;
    for pf in paks {
        let pn = jstr(&pf.name);
        for e in &pf.entries {
            if !first {
                let _ = w.write_all(b",");
            }
            first = false;
            let _ = write!(
                w,
                r#"{{"pak":{},"file":{},"offset":{},"compressed":{},"uncompressed":{},"method":{},"encrypted":{}}}"#,
                pn,
                jstr(&e.path),
                e.offset,
                e.compressed_size,
                e.size,
                e.method,
                e.encrypted as u8
            );
        }
    }
    let _ = w.write_all(b"]");
}

/// (size, pak) per normalised path
fn load_index_map(src: &str, o: &Opts) -> HashMap<String, (u64, String)> {
    let p = Path::new(src);
    let mut m = HashMap::new();
    if p.is_file() && src.to_lowercase().ends_with(".json") {
        let data = std::fs::read(p).unwrap_or_else(|e| die(&format!("{src}: {e}")));
        let j: Json = serde_json::from_slice(&data).unwrap_or_else(|e| die(&format!("{src}: {e}")));
        for e in j.as_array().unwrap_or_else(|| die("index is not an array")) {
            let file = e["file"].as_str().unwrap_or("");
            m.insert(fn_pak::normalize_path(file), (e["uncompressed"].as_u64().unwrap_or(0), e["pak"].as_str().unwrap_or("").to_string()));
        }
    } else {
        let set = PakSet::open_install(p).unwrap_or_else(|e| die(&format!("{src}: {e}")));
        let _ = o;
        for (pf, e) in set.effective() {
            m.insert(e.key(), (e.size, pf.name.clone()));
        }
    }
    m
}

fn top_dir(path: &str) -> String {
    // engine-relative: first 3 components (e.g. fortnitegame/content/athena)
    path.split('/').take(3).collect::<Vec<_>>().join("/")
}

fn cmd_diff_index(o: &Opts) {
    let a = o.args.get(1).unwrap_or_else(|| die("diff-index <A> <B> [--list]"));
    let b = o.args.get(2).unwrap_or_else(|| die("diff-index <A> <B> [--list]"));
    let list = o.args.iter().any(|x| x == "--list");
    let (ma, mb) = std::thread::scope(|s| {
        let ha = s.spawn(|| load_index_map(a, o));
        let hb = s.spawn(|| load_index_map(b, o));
        (ha.join().unwrap(), hb.join().unwrap())
    });
    let mut removed = Vec::new();
    let mut changed = Vec::new();
    let mut same = 0usize;
    for (k, (sz, _)) in &ma {
        match mb.get(k) {
            None => removed.push(k.clone()),
            Some((sz2, _)) if sz2 != sz => changed.push(k.clone()),
            Some(_) => same += 1,
        }
    }
    let mut added: Vec<String> = mb.keys().filter(|k| !ma.contains_key(*k)).cloned().collect();
    removed.sort();
    changed.sort();
    added.sort();
    let group = |v: &Vec<String>| {
        let mut g: BTreeMap<String, usize> = BTreeMap::new();
        for k in v {
            *g.entry(top_dir(k)).or_default() += 1;
        }
        let mut gv: Vec<(String, usize)> = g.into_iter().collect();
        gv.sort_by(|x, y| y.1.cmp(&x.1));
        gv.truncate(40);
        gv.into_iter().map(|(k, n)| json!({"dir": k, "count": n})).collect::<Vec<_>>()
    };
    let mut j = json!({
        "a": a, "b": b,
        "entriesA": ma.len(), "entriesB": mb.len(),
        "identical": same, "removedInB": removed.len(), "addedInB": added.len(), "sizeChanged": changed.len(),
        "removedByDir": group(&removed), "addedByDir": group(&added), "changedByDir": group(&changed),
    });
    if list {
        j["removed"] = json!(removed);
        j["added"] = json!(added);
        j["changed"] = json!(changed);
    }
    println!("{}", serde_json::to_string_pretty(&j).unwrap());
}

// ------------------------------------------------------------------------------------------------
// sweep
// ------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Agg {
    packages: usize,
    clean: usize,
    partial: usize,
    failed: usize,
    exports: usize,
    exports_failed: usize,
    exports_partial: usize,
    props_unparsed: usize,
    values_total: usize,
    bytes: u64,
    /// cause -> (count, examples)
    failures: HashMap<String, (usize, Vec<String>)>,
    partials: HashMap<String, (usize, Vec<String>)>,
    /// class -> (exports, trailing-bytes histogram bucket counts)
    trailing: HashMap<String, (usize, usize)>,
}

impl Agg {
    fn merge(mut self, o: Agg) -> Agg {
        self.packages += o.packages;
        self.clean += o.clean;
        self.partial += o.partial;
        self.failed += o.failed;
        self.exports += o.exports;
        self.exports_failed += o.exports_failed;
        self.exports_partial += o.exports_partial;
        self.props_unparsed += o.props_unparsed;
        self.values_total += o.values_total;
        self.bytes += o.bytes;
        for (k, (n, ex)) in o.failures {
            let e = self.failures.entry(k).or_default();
            e.0 += n;
            for x in ex {
                if e.1.len() < 5 {
                    e.1.push(x);
                }
            }
        }
        for (k, (n, ex)) in o.partials {
            let e = self.partials.entry(k).or_default();
            e.0 += n;
            for x in ex {
                if e.1.len() < 5 {
                    e.1.push(x);
                }
            }
        }
        for (k, (n, t)) in o.trailing {
            let e = self.trailing.entry(k).or_default();
            e.0 += n;
            e.1 += t;
        }
        self
    }
}

/// Normalise numbers/identifiers out of an error message so causes group.
fn cause_key(msg: &str) -> String {
    let mut out = String::new();
    let mut in_num = false;
    for c in msg.chars() {
        if c.is_ascii_digit() {
            if !in_num {
                out.push('#');
            }
            in_num = true;
        } else {
            in_num = false;
            out.push(c);
        }
    }
    out.chars().take(160).collect()
}

fn note(map: &mut HashMap<String, (usize, Vec<String>)>, key: String, example: &str) {
    let e = map.entry(key).or_default();
    e.0 += 1;
    if e.1.len() < 5 {
        e.1.push(example.to_string());
    }
}

fn count_values(v: &fn_uasset::Value) -> usize {
    use fn_uasset::Value;
    1 + match v {
        Value::Array(a) | Value::Set(a) => a.iter().map(count_values).sum(),
        Value::Map(m) => m.iter().map(|(k, x)| count_values(k) + count_values(x)).sum(),
        Value::Struct { props, .. } => props.iter().map(|p| count_values(&p.value)).sum(),
        _ => 0,
    }
}

fn unparsed_reasons(v: &fn_uasset::Value, out: &mut Vec<(String, String)>) {
    use fn_uasset::Value;
    match v {
        Value::Unparsed { ty, reason, .. } => out.push((ty.clone(), reason.clone())),
        Value::Array(a) | Value::Set(a) => a.iter().for_each(|x| unparsed_reasons(x, out)),
        Value::Map(m) => m.iter().for_each(|(k, x)| {
            unparsed_reasons(k, out);
            unparsed_reasons(x, out)
        }),
        Value::Struct { props, .. } => props.iter().for_each(|p| unparsed_reasons(&p.value, out)),
        _ => {}
    }
}

fn sweep_one(set: &PakSet, pf: &fn_pak::PakFile, e: &fn_pak::PakEntry, popts: fn_uasset::package::ParseOptions, agg: &mut Agg, deep: bool) {
    agg.packages += 1;
    agg.bytes += e.size;
    let name = e.rel_path().to_string();
    let asset = match pf.read(e) {
        Ok(b) => b,
        Err(er) => {
            agg.failed += 1;
            note(&mut agg.failures, format!("pak read: {}", cause_key(&er.to_string())), &name);
            return;
        }
    };
    let stem = name.rsplit_once('.').map(|x| x.0).unwrap_or(&name).to_string();
    let uexp = set.read(&format!("{stem}.uexp")).ok();
    let uexp_bytes: &[u8] = uexp.as_deref().unwrap_or(&[]);
    let pkg = match Package::parse_with(&asset, uexp_bytes, popts) {
        Ok(p) => p,
        Err(er) => {
            agg.failed += 1;
            note(&mut agg.failures, format!("header {:?}: {}", er.kind, cause_key(&er.msg)), &name);
            return;
        }
    };
    let mut failed = false;
    let mut partial = false;
    for i in 0..pkg.exports.len() {
        agg.exports += 1;
        let class = pkg.export_class_name(i);
        match pkg.read_export(i) {
            Err(er) => {
                failed = true;
                agg.exports_failed += 1;
                let k = format!("export range {:?}: {}", er.kind, cause_key(&er.msg));
                note(&mut agg.failures, k, &format!("{name} #{i} {class}"));
            }
            Ok(obj) => {
                if let Some(er) = &obj.error {
                    failed = true;
                    agg.exports_failed += 1;
                    let k = format!("export props {:?} [{}]: {}", er.kind, class, cause_key(&er.msg));
                    note(&mut agg.failures, k, &format!("{name} #{i} {}", obj.name));
                    continue;
                }
                let t = agg.trailing.entry(class.clone()).or_default();
                t.0 += 1;
                t.1 += obj.trailing.len();
                if deep {
                    // run the class/function decoders too
                    if let Err(er) = fn_uasset::classes::check_export(&pkg, i, &obj) {
                        failed = true;
                        agg.exports_failed += 1;
                        let k = format!("class decode {:?} [{}]: {}", er.kind, class, cause_key(&er.msg));
                        note(&mut agg.failures, k, &format!("{name} #{i} {}", obj.name));
                        continue;
                    }
                }
                let mut reasons = Vec::new();
                for p in obj.props.iter() {
                    agg.values_total += count_values(&p.value);
                    unparsed_reasons(&p.value, &mut reasons);
                }
                if !reasons.is_empty() {
                    partial = true;
                    agg.exports_partial += 1;
                    agg.props_unparsed += reasons.len();
                    for (ty, reason) in reasons.into_iter().take(16) {
                        note(&mut agg.partials, format!("{ty}: {}", cause_key(&reason)), &format!("{name} #{i} {}", obj.name));
                    }
                }
            }
        }
    }
    if failed {
        agg.failed += 1;
    } else if partial {
        agg.partial += 1;
    } else {
        agg.clean += 1;
    }
}

fn cmd_sweep(o: &Opts) {
    let mut filter = "**/*.uasset".to_string();
    let mut limit = usize::MAX;
    let mut report: Option<String> = None;
    let mut deep = true;
    let mut include_umap = true;
    let mut it = o.args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--filter" => filter = it.next().cloned().unwrap_or_default(),
            "--limit" => limit = it.next().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX),
            "--report" => report = it.next().cloned(),
            "--no-deep" => deep = false,
            "--no-umap" => include_umap = false,
            "--threads" => {
                if let Some(n) = it.next().and_then(|v| v.parse::<usize>().ok()) {
                    rayon::ThreadPoolBuilder::new().num_threads(n).build_global().ok();
                }
            }
            _ => {}
        }
    }
    let t0 = Instant::now();
    let set = open(o);
    let t_open = t0.elapsed();
    let popts = parse_opts(o);
    let mut files: Vec<(&fn_pak::PakFile, &fn_pak::PakEntry)> = set.glob(&filter);
    if include_umap && filter == "**/*.uasset" {
        files.extend(set.glob("**/*.umap"));
    }
    files.truncate(limit);
    let t1 = Instant::now();
    let agg = files
        .par_iter()
        .fold(Agg::default, |mut a, (pf, e)| {
            sweep_one(&set, pf, e, popts, &mut a, deep);
            a
        })
        .reduce(Agg::default, Agg::merge);
    let t_parse = t1.elapsed();

    let pct = |n: usize| if agg.packages == 0 { 0.0 } else { 100.0 * n as f64 / agg.packages as f64 };
    println!("install            : {}", o.root.display());
    println!("mount time         : {:?} ({} paks, {} entries)", t_open, set.paks().len(), set.total_entries());
    println!("packages swept     : {} ({:.1} GiB of .uasset)", agg.packages, agg.bytes as f64 / (1u64 << 30) as f64);
    println!("parse time         : {:?} on {} threads ({:.0} packages/s)", t_parse, rayon::current_num_threads(), agg.packages as f64 / t_parse.as_secs_f64());
    println!("clean              : {} ({:.3}%)", agg.clean, pct(agg.clean));
    println!("parsed w/ unparsed : {} ({:.3}%)  [{} exports, {} property values kept as raw bytes]", agg.partial, pct(agg.partial), agg.exports_partial, agg.props_unparsed);
    println!("failed             : {} ({:.3}%)", agg.failed, pct(agg.failed));
    println!("parse w/o error    : {:.3}% (clean + unparsed-values)", pct(agg.clean + agg.partial));
    println!("exports            : {} total, {} failed", agg.exports, agg.exports_failed);
    println!("property values    : {} decoded in total, {} kept raw ({:.4}% decoded)", agg.values_total, agg.props_unparsed, 100.0 * (agg.values_total - agg.props_unparsed.min(agg.values_total)) as f64 / agg.values_total.max(1) as f64);
    let mut f: Vec<_> = agg.failures.iter().collect();
    f.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    println!("\nFAILURES by cause ({} causes):", f.len());
    for (k, (n, ex)) in f.iter().take(60) {
        println!("  {n:>7}  {k}\n           e.g. {}", ex.join(" | "));
    }
    let mut p: Vec<_> = agg.partials.iter().collect();
    p.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    println!("\nUNPARSED VALUES by type/cause ({} causes):", p.len());
    for (k, (n, ex)) in p.iter().take(60) {
        println!("  {n:>7}  {k}\n           e.g. {}", ex.join(" | "));
    }
    if let Some(path) = report {
        let j = json!({
            "install": o.root.display().to_string(),
            "packages": agg.packages, "clean": agg.clean, "partial": agg.partial, "failed": agg.failed,
            "percentParsedWithoutError": pct(agg.clean + agg.partial), "percentClean": pct(agg.clean),
            "exports": agg.exports, "exportsFailed": agg.exports_failed, "exportsPartial": agg.exports_partial, "propertyValues": agg.values_total, "propertyValuesRaw": agg.props_unparsed,
            "parseSeconds": t_parse.as_secs_f64(), "mountSeconds": t_open.as_secs_f64(),
            "failures": f.iter().map(|(k, (n, ex))| json!({"cause": k, "count": n, "examples": ex})).collect::<Vec<_>>(),
            "unparsedValues": p.iter().map(|(k, (n, ex))| json!({"cause": k, "count": n, "examples": ex})).collect::<Vec<_>>(),
            "trailingBytesByClass": agg.trailing.iter().map(|(k, (n, t))| (k.clone(), json!({"exports": n, "trailingBytes": t}))).collect::<BTreeMap<_, _>>(),
        });
        std::fs::write(&path, serde_json::to_string_pretty(&j).unwrap()).unwrap_or_else(|e| die(&format!("{path}: {e}")));
        println!("\nreport written to {path}");
    }
}




