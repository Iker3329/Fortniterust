use fn_pak::{PakEntry, PakSet};
use std::collections::{BTreeMap, HashMap};

pub struct Touched {
    pub pak: String,
    pub path: String,
    pub first: usize,
    pub stored: u64,
    pub size: u64,
}

pub fn manifest_ranges(path: &str) -> Vec<(String, u64, u64, usize)> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect("manifest json");
    let mut out = vec![];
    for (i, r) in v.as_array().into_iter().flatten().enumerate() {
        if r[0].as_str() != Some("file") { continue; }
        let p = r[1].as_str().unwrap_or("");
        out.push((p.to_owned(), r[2].as_u64().unwrap_or(0), r[3].as_u64().unwrap_or(0), i));
    }
    out
}

pub fn stored_span(e: &PakEntry, header: u64) -> (u64, u64) {
    let start = e.offset;
    let end = e.blocks.last().map(|b| b.1).unwrap_or(e.offset + header + e.compressed_size).max(e.offset + header + e.compressed_size);
    (start, end)
}

pub fn touched_entries(set: &PakSet, manifests: &[String]) -> Vec<Touched> {
    let mut by_pak: HashMap<String, Vec<(u64, u64, usize)>> = HashMap::new();
    let mut rank = 0usize;
    for m in manifests {
        for (p, a, b, i) in manifest_ranges(m) {
            if let Some(name) = p.strip_prefix("/fn/paks/") { by_pak.entry(name.to_owned()).or_default().push((a, b, rank + i)); }
        }
        rank += 1_000_000;
    }
    let mut out: HashMap<(String, u64), Touched> = HashMap::new();
    for pf in set.paks() {
        let Some(ranges) = by_pak.get(&pf.name) else { continue };
        let mut ents: Vec<(&PakEntry, u64, u64)> = pf.entries.iter().map(|e| { let (s, t) = stored_span(e, 0); (e, s, t) }).collect();
        ents.sort_by_key(|x| x.1);
        for &(a, b, r) in ranges {
            let lo = ents.partition_point(|x| x.2 <= a);
            for &(e, s, t) in &ents[lo..] {
                if s >= b { break; }
                if t <= a { continue; }
                let k = (pf.name.clone(), e.offset);
                let ent = out.entry(k).or_insert_with(|| Touched { pak: pf.name.clone(), path: e.rel_path().to_owned(), first: r, stored: t - s, size: e.size });
                ent.first = ent.first.min(r);
            }
        }
    }
    let mut v: Vec<Touched> = out.into_values().collect();
    v.sort_by_key(|t| (t.first, t.pak.clone(), t.path.clone()));
    v
}

pub fn package_class(set: &PakSet, rel: &str) -> String {
    let lower = rel.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    if !matches!(ext, "uasset" | "uexp" | "ubulk" | "umap") { return format!(".{ext}"); }
    if ext == "umap" { return "World(umap)".into(); }
    let base = &rel[..rel.len() - ext.len() - 1];
    let ua = format!("{base}.uasset");
    let Ok(a) = set.read(&ua) else { if set.read(&format!("{base}.umap")).is_ok() { return format!("World(umap).{ext}"); } return format!("?.{ext}") };
    let Ok(p) = fn_ueio::Package::parse(&a, &[], &[]) else { return format!("?parse.{ext}") };
    let mut best: Option<(usize, String)> = None;
    for i in 0..p.exports.len() {
        let c = p.export_class(i);
        let sz = p.exports[i].serial_size as usize;
        if best.as_ref().map_or(true, |b| sz > b.0) { best = Some((sz, c)); }
    }
    best.map(|b| b.1).unwrap_or_else(|| "?".into())
}

pub fn run(set: &PakSet, manifests: &[String], out_tsv: Option<&str>) {
    let mut asset_bytes: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    let mut pak_range_bytes = 0u64;
    for m in manifests {
        for (p, a, b, _) in manifest_ranges(m) {
            if p.starts_with("/fn/paks/") { pak_range_bytes += b - a; continue; }
            let parts: Vec<&str> = p.split('/').collect();
            let k = format!("{} .{}", parts.iter().take(4).cloned().collect::<Vec<_>>().join("/"), p.rsplit('.').next().unwrap_or(""));
            let e = asset_bytes.entry(k).or_default();
            e.0 += 1;
            e.1 += b - a;
        }
    }
    let touched = touched_entries(set, manifests);
    let mut cls_cache: HashMap<String, String> = HashMap::new();
    let mut by_class: BTreeMap<String, (usize, u64, u64)> = BTreeMap::new();
    let mut by_dir: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    let mut lines = vec![];
    for t in &touched {
        let lower = t.path.to_ascii_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("").to_owned();
        let base = lower[..lower.len() - ext.len()].to_owned();
        let cls = cls_cache.entry(base).or_insert_with(|| { let c = package_class(set, &t.path); c.split('.').next().unwrap_or("").to_owned() }).clone();
        let k = format!("{cls} .{ext}");
        let e = by_class.entry(k).or_default();
        e.0 += 1; e.1 += t.stored; e.2 += t.size;
        let d: Vec<&str> = t.path.split('/').skip(2).take(2).collect();
        let de = by_dir.entry(d.join("/")).or_default();
        de.0 += 1; de.1 += t.stored;
        lines.push(format!("{}\t{}\t{}\t{}\t{}\t{}", t.first, t.pak, t.path, t.stored, t.size, cls));
    }
    let mb = |b: u64| b as f64 / 1048576.;
    println!("pak ranges requested: {:.1} MB; distinct entries touched: {} ({:.1} MB stored, {:.1} MB uncompressed)", mb(pak_range_bytes), touched.len(), mb(touched.iter().map(|t| t.stored).sum()), mb(touched.iter().map(|t| t.size).sum()));
    println!("\nby main export class + extension (entries, stored MB, raw MB):");
    let mut v: Vec<_> = by_class.into_iter().collect();
    v.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (k, (n, s, z)) in v.iter().take(45) { println!("  {k:<44} {n:>6} {:>9.1} {:>9.1}", mb(*s), mb(*z)); }
    println!("\nby content dir (entries, stored MB):");
    let mut d: Vec<_> = by_dir.into_iter().collect();
    d.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (k, (n, s)) in d.iter().take(25) { println!("  {k:<44} {n:>6} {:>9.1}", mb(*s)); }
    println!("\nnon-pak files (requests, MB):");
    let mut a: Vec<_> = asset_bytes.into_iter().collect();
    a.sort_by(|x, y| y.1 .1.cmp(&x.1 .1));
    for (k, (n, b)) in a.iter().take(20) { println!("  {k:<44} {n:>6} {:>9.1}", mb(*b)); }
    if let Some(p) = out_tsv { let _ = std::fs::write(p, lines.join("\n") + "\n"); println!("\nentry list: {p}"); }
}

pub fn read_log_entries(path: &str) -> Vec<(String, u64, u32)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out = vec![];
    let mut on = false;
    for l in text.lines() {
        if l.starts_with("largest entries") { on = true; continue; }
        if !on { continue; }
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 4 { continue; }
        let mb: f64 = f[0].parse().unwrap_or(0.);
        let n: u32 = f[2].trim_end_matches('x').parse().unwrap_or(1);
        out.push((f[3..].join(" "), (mb * 1048576. / n.max(1) as f64) as u64, n));
    }
    out
}

pub fn readlog(set: &PakSet, path: &str, webtex_index: &str) {
    let ents = read_log_entries(path);
    let tex: std::collections::HashSet<String> = serde_json::from_slice::<serde_json::Value>(&std::fs::read(webtex_index).unwrap()).unwrap()["textures"].as_object().unwrap().keys().cloned().collect();
    let mut by: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    let mut tex_dup = 0u64;
    for (p, _, _) in &ents {
        let full = format!("FortniteGame/Content/{p}");
        let Some((_, e)) = set.find(&full) else { continue };
        let cls = package_class(set, &full);
        let ext = p.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
        let k = format!("{} .{ext}", cls.split('.').next().unwrap_or(""));
        let x = by.entry(k).or_default();
        x.0 += 1; x.1 += e.size;
        if cls.starts_with("Texture2D") && (ext == "uexp" || ext == "ubulk") {
            let g = format!("/game/{}", p[..p.len() - ext.len() - 1].to_ascii_lowercase());
            if tex.contains(&g) { tex_dup += e.size; }
        }
    }
    let mut v: Vec<_> = by.into_iter().collect();
    v.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    let tot: u64 = v.iter().map(|x| x.1 .1).sum();
    println!("exact read log: {} entries, {:.1} MB unique raw; Texture2D uexp/ubulk read from paks although in the tex index: {:.1} MB", ents.len(), tot as f64 / 1048576., tex_dup as f64 / 1048576.);
    for (k, (n, s)) in v.iter().take(30) { println!("  {k:<44} {n:>6} {:>8.1} MB", *s as f64 / 1048576.); }
}

pub fn bootmap(set: &PakSet, manifest: &str, wc_pak: &str, out_tsv: Option<&str>) {
    let ix = fn_pak::webcook::Index::open(std::path::Path::new(wc_pak)).expect("webcook index");
    let mut by_file: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
    let mut other: BTreeMap<String, u64> = BTreeMap::new();
    for (p, a, b, _) in manifest_ranges(manifest) {
        if let Some(n) = p.strip_prefix("/fn/assets/webcook/v1/pak/") { by_file.entry(n.to_owned()).or_default().push((a, b + 1)); }
        else { let k = p.split('/').take(5).collect::<Vec<_>>().join("/"); *other.entry(k).or_default() += b + 1 - a; }
    }
    let mut chunk_ids: Vec<u32> = vec![];
    let mut fetched = 0u64;
    for (fi, f) in ix.files.iter().enumerate() {
        let Some(rs) = by_file.get(f) else { continue };
        for &(a, b) in rs { fetched += b - a; }
        for (ci, c) in ix.chunks.iter().enumerate() {
            if c.file as usize != fi { continue; }
            if rs.iter().any(|&(a, b)| c.off < b && c.off + c.clen as u64 > a) { chunk_ids.push(ci as u32); }
        }
    }
    let wanted: std::collections::HashSet<u32> = chunk_ids.iter().copied().collect();
    let chunk_wire: u64 = chunk_ids.iter().map(|&c| ix.chunks[c as usize].clen as u64).sum();
    let by_off: HashMap<(String, u64), &PakEntry> = set.paks().iter().flat_map(|p| p.entries.iter().map(move |e| ((p.name.to_ascii_lowercase(), e.offset), e))).collect();
    let mut cls_cache: HashMap<String, String> = HashMap::new();
    let mut by_class: BTreeMap<String, (usize, u64, f64)> = BTreeMap::new();
    let mut rows = vec![];
    for ((pid, off), l) in &ix.entries {
        if !wanted.contains(&l.chunk) { continue; }
        let c = &ix.chunks[l.chunk as usize];
        let pak = ix.paks[*pid as usize].name.to_ascii_lowercase();
        let Some(e) = by_off.get(&(pak, *off)) else { continue };
        let rel = e.rel_path().to_owned();
        let lower = rel.to_ascii_lowercase();
        let ext = lower.rsplit('.').next().unwrap_or("").to_owned();
        let base = lower[..lower.len() - ext.len()].to_owned();
        let cls = cls_cache.entry(base).or_insert_with(|| { let c = package_class(set, &rel); c.split('.').next().unwrap_or(&c).to_owned() }).clone();
        let key = format!("{cls}.{ext}");
        let wire = c.clen as f64 * l.size as f64 / c.raw.max(1) as f64;
        let s = by_class.entry(key.clone()).or_default();
        s.0 += 1; s.1 += l.size as u64; s.2 += wire;
        rows.push((rel, key, l.size, wire));
    }
    let m = |b: f64| b / 1048576.;
    println!("pak ranges fetched {:.1} MB, chunks touched {} ({:.1} MB wire)", m(fetched as f64), chunk_ids.len(), m(chunk_wire as f64));
    let mut v: Vec<_> = by_class.into_iter().collect();
    v.sort_by(|a, b| b.1 .2.partial_cmp(&a.1 .2).unwrap());
    for (k, (n, raw, wire)) in v.iter().take(30) { println!("{:>8.1} MB wire {:>8.1} MB raw {:>6} {k}", m(*wire), m(*raw as f64), n); }
    for (k, b) in &other { println!("other {:>8.1} MB {k}", m(*b as f64)); }
    if let Some(t) = out_tsv {
        rows.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
        let s: String = rows.iter().map(|r| format!("{}\t{}\t{}\t{:.0}\n", r.0, r.1, r.2, r.3)).collect();
        let _ = std::fs::write(t, s);
    }
}

pub fn texmiss(tsv: &str, tex_dir: &str) {
    let ix = fn_fx::webcook::open_index(std::path::Path::new(tex_dir)).expect("tex index");
    let text = std::fs::read_to_string(tsv).expect("tsv");
    let (mut hit, mut miss, mut hb, mut mbb) = (0, 0, 0f64, 0f64);
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 4 || f[1] != "Texture2D.ubulk" { continue; }
        let rel = f[0].strip_prefix("FortniteGame/Content/").unwrap_or(f[0]);
        let long = format!("/Game/{}", &rel[..rel.len() - 6]);
        let w: f64 = f[3].parse().unwrap_or(0.);
        if ix.map.contains_key(&long.to_ascii_lowercase()) { hit += 1; hb += w; } else { miss += 1; mbb += w; if miss <= 8 { println!("missing {long}"); } }
    }
    println!("boot Texture2D.ubulk: {hit} in the tex cook ({:.1} MB), {miss} not in it ({:.1} MB)", hb / 1048576., mbb / 1048576.);
}

pub fn texhas(dir: &str, keys: &[&str]) {
    let ix = fn_fx::webcook::open_index(std::path::Path::new(dir)).expect("tex index");
    for k in keys { println!("{} {}", if ix.map.contains_key(&k.to_ascii_lowercase()) { "IN " } else { "NOT" }, k); }
}
