use fn_pak::PakSet;
use std::collections::BTreeMap;

pub fn run(set: &PakSet, tsv: &str) {
    let mut methods: BTreeMap<(u32, bool), (usize, u64, u64)> = BTreeMap::new();
    for (_, e) in set.effective() { let m = methods.entry((e.method, e.encrypted)).or_default(); m.0 += 1; m.1 += e.compressed_size; m.2 += e.size; }
    for ((m, enc), (n, c, s)) in &methods { println!("method {m} encrypted {enc}: {n} entries, {:.1} MB stored, {:.1} MB raw", *c as f64 / 1048576., *s as f64 / 1048576.); }
    for p in set.paks() { println!("pak {} index {:.1} MB, {} entries", p.name, p.info.index_size as f64 / 1048576., p.entries.len()); }
    let text = std::fs::read_to_string(tsv).unwrap();
    let rows: Vec<Vec<&str>> = text.lines().map(|l| l.split('\t').collect()).collect();
    let mut by_cls: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for r in &rows { if r.len() < 6 { continue; } let ext = r[2].rsplit('.').next().unwrap_or("").to_ascii_lowercase(); by_cls.entry(format!("{} .{ext}", r[5])).or_default().push((r[1].to_owned(), r[2].to_owned())); }
    let mut keys: Vec<_> = by_cls.iter().map(|(k, v)| (k.clone(), v.len())).collect();
    keys.sort_by(|a, b| b.1.cmp(&a.1));
    let mut tot = [0u64; 4];
    for (k, _) in keys.iter().take(24) {
        let v = &by_cls[k];
        let step = (v.len() / 120).max(1);
        let mut raw = vec![];
        let mut single = 0u64;
        let mut n = 0;
        for (_, path) in v.iter().step_by(step).take(120) {
            let Ok(b) = set.read(path) else { continue };
            single += zstd::bulk::compress(&b, 19).map(|c| c.len() as u64).unwrap_or(b.len() as u64);
            raw.extend_from_slice(&b);
            n += 1;
        }
        let chunked = |sz: usize| -> u64 { raw.chunks(sz).map(|c| zstd::bulk::compress(c, 19).map(|x| x.len() as u64).unwrap_or(c.len() as u64)).sum() };
        let (c64, c256) = (chunked(64 << 10), chunked(256 << 10));
        tot[0] += raw.len() as u64; tot[1] += single; tot[2] += c64; tot[3] += c256;
        println!("{k:<44} n={n:>4} raw {:>8.1} KB  single {:.3}  chunk64 {:.3}  chunk256 {:.3}", raw.len() as f64 / 1024., single as f64 / raw.len().max(1) as f64, c64 as f64 / raw.len().max(1) as f64, c256 as f64 / raw.len().max(1) as f64);
    }
    println!("overall ratio: single {:.3} chunk64 {:.3} chunk256 {:.3}", tot[1] as f64 / tot[0] as f64, tot[2] as f64 / tot[0] as f64, tot[3] as f64 / tot[0] as f64);
}

pub fn dedupe(set: &PakSet, tsv: &str, class: &str) {
    let text = std::fs::read_to_string(tsv).unwrap();
    let mut raw = vec![];
    let mut single = 0u64;
    let mut n = 0;
    for l in text.lines() {
        let r: Vec<&str> = l.split('\t').collect();
        if r.len() < 6 || r[5] != class || !r[2].ends_with(".uexp") { continue; }
        let Ok(b) = set.read(r[2]) else { continue };
        single += zstd::bulk::compress(&b, 19).unwrap().len() as u64;
        raw.extend_from_slice(&b);
        n += 1;
    }
    let mut enc = zstd::stream::Encoder::new(Vec::new(), 19).unwrap();
    enc.long_distance_matching(true).unwrap();
    enc.window_log(27).unwrap();
    std::io::Write::write_all(&mut enc, &raw).unwrap();
    let long = enc.finish().unwrap().len() as u64;
    let seg = |sz: usize| -> u64 { raw.chunks(sz).map(|c| { let mut e = zstd::stream::Encoder::new(Vec::new(), 19).unwrap(); e.long_distance_matching(true).unwrap(); e.window_log(25).unwrap(); std::io::Write::write_all(&mut e, c).unwrap(); e.finish().unwrap().len() as u64 }).sum() };
    let s8 = seg(8 << 20);
    let s32 = seg(32 << 20);
    println!("{class} uexp: {n} entries, raw {:.1} MB, per-entry zstd {:.1} MB, one long stream {:.1} MB, 8MB segments {:.1} MB, 32MB segments {:.1} MB", raw.len() as f64 / 1048576., single as f64 / 1048576., long as f64 / 1048576., s8 as f64 / 1048576., s32 as f64 / 1048576.);
}

pub fn fields(format: &str) -> Option<&'static [usize]> {
    Some(match format { "Bc1" => &[4, 4], "Bc4" => &[2, 6], "Bc3" => &[2, 6, 4, 4], "Bc5" => &[2, 6, 2, 6], "Bc2" => &[8, 4, 4], _ => return None })
}

pub fn split(b: &[u8], f: &[usize]) -> Vec<u8> {
    let bs: usize = f.iter().sum();
    let n = b.len() / bs;
    let mut out = Vec::with_capacity(b.len());
    let mut off = 0;
    for &w in f { for i in 0..n { out.extend_from_slice(&b[i * bs + off..i * bs + off + w]); } off += w; }
    out.extend_from_slice(&b[n * bs..]);
    out
}

pub fn bcsplit(webtex: &str) {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(format!("{webtex}/index.json")).unwrap()).unwrap();
    let packs: Vec<String> = v["packs"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_owned()).collect();
    let mut st: BTreeMap<String, (u64, u64, u64)> = BTreeMap::new();
    let t = v["textures"].as_object().unwrap();
    for (i, (_, e)) in t.iter().enumerate() {
        if i % 12 != 0 { continue; }
        let f = e["f"].as_str().unwrap();
        let Some(fl) = fields(f) else { continue };
        let total: u64 = e["l"].as_array().unwrap().iter().map(|l| l[2].as_u64().unwrap()).sum();
        let mut fh = std::fs::File::open(format!("{webtex}/{}", packs[e["p"].as_u64().unwrap() as usize])).unwrap();
        use std::io::{Read, Seek, SeekFrom};
        fh.seek(SeekFrom::Start(e["o"].as_u64().unwrap())).unwrap();
        let mut b = vec![0u8; total as usize];
        fh.read_exact(&mut b).unwrap();
        let a = zstd::bulk::compress(&b, 19).unwrap().len() as u64;
        let s = zstd::bulk::compress(&split(&b, fl), 19).unwrap().len() as u64;
        let x = st.entry(f.to_owned()).or_default();
        x.0 += total; x.1 += a; x.2 += s;
    }
    for (f, (r, a, s)) in st { println!("{f}: raw {:.1} MB, zstd {:.3}, split+zstd {:.3}", r as f64 / 1048576., a as f64 / r as f64, s as f64 / r as f64); }
}

pub fn dictexp(set: &PakSet, tsv: &str) {
    let text = std::fs::read_to_string(tsv).unwrap();
    let mut ents: Vec<Vec<u8>> = vec![];
    for l in text.lines() {
        let r: Vec<&str> = l.split('\t').collect();
        if r.len() < 6 || !(r[5] == "MaterialInstanceConstant" || r[5] == "Material") || !r[2].ends_with(".uexp") { continue; }
        if let Ok(b) = set.read(r[2]) { ents.push(b.into_owned()); }
    }
    let raw: u64 = ents.iter().map(|e| e.len() as u64).sum();
    let single: u64 = ents.iter().map(|e| zstd::bulk::compress(e, 19).unwrap().len() as u64).sum();
    println!("MIC+Material uexp: {} entries {:.1} MB raw, per-entry {:.1} MB", ents.len(), raw as f64 / 1048576., single as f64 / 1048576.);
    for mb in [2usize, 8, 32] {
        let t0 = std::time::Instant::now();
        let dict = match zstd::dict::from_samples(&ents, mb << 20) { Ok(d) => d, Err(e) => { println!("dict {mb} MB: {e}"); continue } };
        let mut c = zstd::bulk::Compressor::with_dictionary(19, &dict).unwrap();
        let with: u64 = ents.iter().map(|e| c.compress(e).unwrap().len() as u64).sum();
        let mut fd = ruzstd::decoding::FrameDecoder::new();
        fd.add_dict(ruzstd::decoding::Dictionary::decode_dict(&dict).unwrap()).unwrap();
        let z = c.compress(&ents[0]).unwrap();
        let mut out = Vec::with_capacity(ents[0].len());
        fd.decode_all_to_vec(&z, &mut out).unwrap();
        println!("dict {mb} MB (actual {:.1} MB, train {:.0} s): entries {:.1} MB + dict -> {:.1} MB total; ruzstd roundtrip {}", dict.len() as f64 / 1048576., t0.elapsed().as_secs_f32(), with as f64 / 1048576., (with + dict.len() as u64) as f64 / 1048576., out == ents[0]);
    }
}
