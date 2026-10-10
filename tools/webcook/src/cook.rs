use fn_fx::webcook as wt;
use fn_pak::webcook as wp;
use fn_pak::PakSet;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct Opts {
    pub out: PathBuf,
    pub webtex: PathBuf,
    pub assets: PathBuf,
    pub manifests: Vec<String>,
    pub level: i32,
    pub threads: usize,
    pub closure: bool,
    pub part_bytes: u64,
    pub reads: Vec<String>,
    pub solid_bytes: u64,
}

fn par_map<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let slots: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= items.len() { break; }
                let r = f(&items[i]);
                *slots[i].lock().unwrap() = Some(r);
            });
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap().unwrap()).collect()
}

fn zst(b: &[u8], level: i32) -> Vec<u8> { zstd::bulk::compress(b, level).expect("zstd") }

fn mb(b: u64) -> f64 { b as f64 / 1048576. }

pub fn is_exact(key: &str, format: &str) -> bool {
    let k = key.to_ascii_lowercase();
    format == "Rgba8" || k.contains("/ui/") || k.contains("/hud/") || k.contains("/fonts/") || k.contains("athena_terrain") || k.rsplit('/').next().is_some_and(|l| l.contains('.'))
}

pub fn cook_tex(o: &Opts) -> Result<(), String> {
    let dir = o.out.join("tex");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&std::fs::read(o.webtex.join("index.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let packs: Vec<String> = v["packs"].as_array().unwrap().iter().map(|p| p.as_str().unwrap().to_owned()).collect();
    let mut items: Vec<(String, Value)> = v["textures"].as_object().unwrap().iter().map(|(k, e)| (k.clone(), e.clone())).collect();
    items.sort_by_key(|(_, e)| (e["p"].as_u64().unwrap_or(0), e["o"].as_u64().unwrap_or(0)));
    let maps: Vec<memmap2::Mmap> = packs.iter().map(|p| { let f = std::fs::File::open(o.webtex.join(p)).unwrap(); unsafe { memmap2::Mmap::map(&f).unwrap() } }).collect();
    let mut files: Vec<std::io::BufWriter<std::fs::File>> = wt::TIER_FILES.iter().map(|n| std::io::BufWriter::with_capacity(8 << 20, std::fs::File::create(dir.join(n)).unwrap())).collect();
    let mut lens = [0u64; 4];
    let mut raw_by_tier = [0u64; 4];
    let mut texs = Vec::with_capacity(items.len());
    let t0 = std::time::Instant::now();
    for (ci, chunk) in items.chunks(128).enumerate() {
        let res = par_map(chunk, o.threads, |(key, e)| {
            let levels: Vec<(u32, u32, u32)> = e["l"].as_array().unwrap().iter().map(|l| (l[0].as_u64().unwrap() as u32, l[1].as_u64().unwrap() as u32, l[2].as_u64().unwrap() as u32)).collect();
            let base = e["o"].as_u64().unwrap() as usize;
            let m = &maps[e["p"].as_u64().unwrap() as usize];
            let fields = fn_fx::webtex::parse_format(e["f"].as_str().unwrap_or("Rgba8")).and_then(wt::bc_fields);
            let mut groups: Vec<(u8, usize, usize, Vec<u8>)> = vec![];
            let mut at = base;
            for (i, l) in levels.iter().enumerate() {
                let t = wt::tier_of(l.0, l.1);
                let bytes = match fields { Some(f) => wt::split_fields(&m[at..at + l.2 as usize], f), None => m[at..at + l.2 as usize].to_vec() };
                let bytes = &bytes[..];
                at += l.2 as usize;
                match groups.last_mut() { Some(g) if g.0 == t => { g.2 += 1; g.3.extend_from_slice(bytes); } _ => groups.push((t, i, 1, bytes.to_vec())) }
            }
            let z: Vec<(u8, usize, usize, usize, Vec<u8>)> = groups.into_iter().map(|g| (g.0, g.1, g.2, g.3.len(), zst(&g.3, o.level))).collect();
            (key.clone(), e.clone(), levels, z, fields.is_some() as u8)
        });
        for (key, e, levels, z, xf) in res {
            let f = e["f"].as_str().unwrap_or("Rgba8").to_owned();
            let mut tiers = vec![];
            for (t, first, count, raw, data) in z {
                let ti = t as usize;
                files[ti].write_all(&data).map_err(|e| e.to_string())?;
                tiers.push(wt::Tier { tier: t, first: first as u8, count: count as u8, file: t as u16, off: lens[ti], clen: data.len() as u32, xf });
                lens[ti] += data.len() as u64;
                raw_by_tier[ti] += raw as u64;
            }
            let s = |n: &str| e[n].as_str().unwrap_or("").to_owned();
            let info = fn_fx::gpu::TexInfo { filter: s("fi"), address_x: s("ax"), address_y: s("ay"), srgb: e["s"].as_bool().unwrap_or(true), width: levels[0].0 as usize, height: levels[0].1 as usize, levels: levels.len(), compression: s("c") };
            let source = (e["sz"][0].as_u64().unwrap_or(levels[0].0 as u64) as u32, e["sz"][1].as_u64().unwrap_or(levels[0].1 as u64) as u32);
            texs.push(wt::Tex { exact: is_exact(&key, &f), key, name: s("n"), format: fn_fx::webtex::parse_format(&f).unwrap(), info, source, levels, tiers });
        }
        if ci % 8 == 0 { eprintln!("[webcook] tex {}/{} ({:.0} s)", (ci + 1) * 128, items.len(), t0.elapsed().as_secs_f32()); }
    }
    for f in &mut files { f.flush().map_err(|e| e.to_string())?; }
    let files_list: Vec<String> = wt::TIER_FILES.iter().map(|s| s.to_string()).collect();
    let raw = wt::to_bytes(&files_list, &texs);
    let z = zst(&raw, 19);
    wt::write_index(&dir.join(wt::INDEX_FILE), &raw, &z).map_err(|e| e.to_string())?;
    println!("tex: {} textures, index {:.2} MB ({:.2} MB raw)", texs.len(), mb(z.len() as u64 + 4), mb(raw.len() as u64));
    for i in 0..4 { println!("  {:<9} {:>8.1} MB zstd  <- {:>8.1} MB BC/RGBA ({:.3})", wt::TIER_FILES[i], mb(lens[i]), mb(raw_by_tier[i]), lens[i] as f64 / raw_by_tier[i].max(1) as f64); }
    let exact: Vec<&wt::Tex> = texs.iter().filter(|t| t.exact).collect();
    let ex_hi: u64 = exact.iter().flat_map(|t| t.tiers.iter().filter(|r| r.tier > 2)).map(|r| r.clen as u64).sum();
    for (cap, tier) in [(256u32, 0u8), (512, 1), (1024, 2), (2048, 3)] {
        let tot: u64 = texs.iter().flat_map(|t| t.tiers.iter().filter(move |r| t.exact || r.tier <= tier)).map(|r| r.clen as u64).sum();
        println!("  download at cap {cap:>4}: {:>8.1} MB (exact textures always full: {} textures, {:.1} MB above 1K)", mb(tot), exact.len(), mb(ex_hi));
    }
    Ok(())
}

pub fn verify_tex(o: &Opts, samples: usize) -> Result<(), String> {
    fn_render::fx::set_bc_native_hint(true);
    let ix = wt::open_index(&o.out.join("tex"))?;
    let wx = fn_fx::webtex::open_index(&o.webtex)?;
    let mut keys: Vec<&String> = ix.map.keys().collect();
    keys.sort();
    let step = (keys.len() / samples.max(1)).max(1);
    let (mut ok, mut bad, mut capped) = (0, 0, 0);
    for k in keys.iter().step_by(step) {
        let t = &ix.map[*k];
        let a = ix.read(t, 3)?;
        let b = wx.read(&wx.map[*k])?;
        let same = a.2 == b.2 && a.3.len() == b.3.len() && a.3.iter().zip(&b.3).all(|(p, q)| p.width == q.width && p.height == q.height && p.rgba8 == q.rgba8) && a.1.srgb == b.1.srgb && a.1.filter == b.1.filter;
        if same { ok += 1 } else { bad += 1; eprintln!("[webcook] TEX MISMATCH {k}"); }
        let c = ix.read(t, 2)?;
        let tail_same = c.3.iter().rev().zip(b.3.iter().rev()).all(|(p, q)| p.rgba8 == q.rgba8);
        if c.3.len() < b.3.len() { capped += 1; }
        if !tail_same || c.3[0].width.max(c.3[0].height) > if t.exact { u32::MAX } else { 1024 } { bad += 1; eprintln!("[webcook] CAP MISMATCH {k}"); }
    }
    println!("verify tex: {ok} of {} sampled textures identical to webtex at full tier (format, sampler, every level); cap-1024 reads are exact mip suffixes ({capped} capped); {bad} bad", ok + bad);
    if bad > 0 { Err(format!("{bad} tex mismatches")) } else { Ok(()) }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { if p.file_name().map_or(false, |n| n == "webtex") { continue; } walk(&p, out) } else if p.extension().map_or(false, |x| x.eq_ignore_ascii_case("json")) { out.push(p) }
    }
}

fn scan_paths(text: &str, out: &mut BTreeSet<String>) {
    for start in ["/Game/", "/Engine/"] {
        let mut from = 0;
        while let Some(i) = text[from..].find(start) {
            let s = from + i;
            let end = text[s..].find(|c: char| !(c.is_ascii_alphanumeric() || "_/.-:".contains(c))).map_or(text.len(), |e| s + e);
            if let Some(p) = text[s..end].split(['.', ':']).next().map(|p| p.trim_end_matches('/')) { if p.len() > 7 { out.insert(p.to_owned()); } }
            from = end.max(s + 1);
        }
    }
}

fn closure(set: &PakSet, assets: &Path) -> Vec<(String, bool)> {
    let mut roots = BTreeSet::new();
    let mut files = vec![];
    walk(assets, &mut files);
    for f in &files { if let Ok(t) = std::fs::read_to_string(f) { scan_paths(&t, &mut roots); } }
    for (_, e) in set.effective() {
        let r = e.rel_path().to_ascii_lowercase();
        if r.starts_with("fortnitegame/content/athena/") && r.ends_with(".umap") { if let Some(g) = fn_pak::pak_path_to_game(e.rel_path()) { roots.insert(g); } }
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = roots.into_iter().filter(|r| seen.insert(r.to_ascii_lowercase())).collect();
    let mut out = vec![];
    while let Some(long) = queue.pop_front() {
        let a = match set.read_game(&long, "uasset") { Ok(a) => a, Err(_) => match set.read_game(&long, "umap") { Ok(a) => a, Err(_) => continue } };
        let Ok(p) = fn_ueio::Package::parse(&a, &[], &[]) else { continue };
        let tex = (0..p.exports.len()).any(|i| matches!(p.export_class(i).as_str(), "Texture2D" | "TextureCube" | "LightMapTexture2D" | "ShadowMapTexture2D")) && !(0..p.exports.len()).any(|i| matches!(p.export_class(i).as_str(), "World" | "Level"));
        out.push((long.clone(), tex));
        for imp in &p.imports {
            if imp.class_name != "Package" { continue; }
            let n = &imp.object_name;
            if !(n.starts_with("/Game/") || n.starts_with("/Engine/")) { continue; }
            if seen.insert(n.to_ascii_lowercase()) { queue.push_back(n.clone()); }
        }
    }
    out
}

struct Sel<'a> { pak: String, offset: u64, path: String, size: u64, first: bool, pf: &'a fn_pak::PakFile, e: &'a fn_pak::PakEntry }

pub fn cook_pak<'s>(set: &'s PakSet, o: &Opts) -> Result<(), String> {
    let dir = o.out.join("pak");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let t0 = std::time::Instant::now();
    let mut chosen: HashSet<(String, u64)> = HashSet::new();
    let mut sel: Vec<Sel> = vec![];
    let mut add = |pf: &'s fn_pak::PakFile, e: &'s fn_pak::PakEntry, first: bool, sel: &mut Vec<Sel<'s>>| {
        let pak = pf.name.as_str();
        if e.encrypted || e.method != 0 || e.size > (u32::MAX as u64) { return; }
        if chosen.insert((pak.to_owned(), e.offset)) { sel.push(Sel { pak: pak.to_owned(), offset: e.offset, path: e.rel_path().to_owned(), size: e.size, first, pf, e }); }
    };
    let touched = crate::analyze::touched_entries(set, &o.manifests);
    let winners: HashSet<(String, u64)> = set.effective().map(|(p, e)| (p.name.clone(), e.offset)).collect();
    if o.reads.is_empty() {
        for t in &touched {
            let Some((pf, e)) = set.find(&t.path) else { continue };
            if !winners.contains(&(pf.name.clone(), e.offset)) { continue; }
            add(pf, e, true, &mut sel);
        }
    } else {
        let rank: HashMap<String, usize> = touched.iter().map(|t| (t.path.to_ascii_lowercase(), t.first)).collect();
        let mut ex = vec![];
        for r in &o.reads {
            for (p, _, _) in crate::analyze::read_log_entries(r) {
                let full = format!("FortniteGame/Content/{p}");
                let Some((pf, e)) = set.find(&full).or_else(|| set.find(&p)) else { continue };
                ex.push((rank.get(&e.rel_path().to_ascii_lowercase()).copied().unwrap_or(usize::MAX), e.rel_path().to_ascii_lowercase(), pf, e));
            }
        }
        ex.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        for (_, _, pf, e) in ex { add(pf, e, true, &mut sel); }
    }
    let n_first = sel.len();
    let first_raw: u64 = sel.iter().map(|s| s.size).sum();
    eprintln!("[webcook] {} recorded entries ({:.1} MB) ({:.0} s)", n_first, mb(first_raw), t0.elapsed().as_secs_f32());
    if o.closure {
        let pk = closure(set, &o.assets);
        let mut rest = vec![];
        for (long, tex) in &pk {
            for ext in ["uasset", "umap", "uexp", "ubulk"] {
                if *tex && (ext == "uexp" || ext == "ubulk") { continue; }
                let Some(p) = fn_pak::game_path_to_pak(long, ext) else { continue };
                if let Some((pf, e)) = set.find(&p) { rest.push((e.rel_path().to_ascii_lowercase(), pf, e)); }
            }
        }
        rest.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, pf, e) in rest { add(pf, e, false, &mut sel); }
        eprintln!("[webcook] closure: {} packages -> {} more entries ({:.1} MB) ({:.0} s)", pk.len(), sel.len() - n_first, mb(sel[n_first..].iter().map(|s| s.size).sum()), t0.elapsed().as_secs_f32());
    }
    const SMALL_CHUNK: u64 = 256 << 10;
    const BIG: u64 = 128 << 10;
    let mut cls_cache: HashMap<String, bool> = HashMap::new();
    let solid: Vec<bool> = sel.iter().map(|s| {
        if o.solid_bytes == 0 || !s.path.to_ascii_lowercase().ends_with(".uexp") { return false; }
        let base = s.path[..s.path.len() - 5].to_ascii_lowercase();
        *cls_cache.entry(base).or_insert_with(|| { let c = crate::analyze::package_class(set, &s.path); c.starts_with("MaterialInstanceConstant") || c.starts_with("Material.") || c == "Material" })
    }).collect();
    let mut plan: Vec<Vec<usize>> = vec![];
    for (lo, hi) in [(0usize, n_first), (n_first, sel.len())] {
        let mut sol: Vec<usize> = (lo..hi).filter(|&i| solid[i]).collect();
        sol.sort_by(|&a, &b| sel[a].path.to_ascii_lowercase().cmp(&sel[b].path.to_ascii_lowercase()));
        let mut cur: Vec<usize> = vec![];
        let mut cur_len = 0u64;
        for i in sol {
            cur.push(i);
            cur_len += sel[i].size;
            if cur_len >= o.solid_bytes { plan.push(std::mem::take(&mut cur)); cur_len = 0; }
        }
        if !cur.is_empty() { plan.push(std::mem::take(&mut cur)); cur_len = 0; }
        for i in lo..hi {
            if solid[i] { continue; }
            let s = &sel[i];
            if s.size >= BIG { plan.push(vec![i]); continue; }
            cur.push(i);
            cur_len += s.size;
            if cur_len >= SMALL_CHUNK { plan.push(std::mem::take(&mut cur)); cur_len = 0; }
        }
        if !cur.is_empty() { plan.push(cur); }
    }
    eprintln!("[webcook] solid material chunks: {} entries", solid.iter().filter(|x| **x).count());
    let mut ix = wp::Index::default();
    let mut pak_ids: HashMap<String, u16> = HashMap::new();
    for p in set.paks() {
        pak_ids.insert(p.name.clone(), ix.paks.len() as u16);
        ix.paks.push(wp::PakIdx { name: p.name.clone(), ..Default::default() });
    }
    let idx_name = "idx.bin".to_string();
    ix.files.push(idx_name.clone());
    {
        let mut w = std::fs::File::create(dir.join(&idx_name)).map_err(|e| e.to_string())?;
        let mut off = 0u64;
        for p in set.paks() {
            let b = p.bytes();
            let tail_len = b.len().min(512);
            let mut blob = b[b.len() - tail_len..].to_vec();
            blob.extend_from_slice(&b[p.info.index_offset as usize..(p.info.index_offset + p.info.index_size) as usize]);
            let z = zst(&blob, o.level);
            w.write_all(&z).map_err(|e| e.to_string())?;
            let id = pak_ids[&p.name] as usize;
            ix.paks[id] = wp::PakIdx { name: p.name.clone(), len: b.len() as u64, file: 0, off, clen: z.len() as u32, tail: tail_len as u32, index: p.info.index_size as u32 };
            off += z.len() as u64;
        }
        println!("pak indices: {:.1} MB -> {:.1} MB", mb(set.paks().iter().map(|p| p.info.index_size + 512).sum()), mb(off));
    }
    let mut part: Option<std::io::BufWriter<std::fs::File>> = None;
    let mut part_len = 0u64;
    let mut wire_first = 0u64;
    let mut wire_total = 0u64;
    let mut first_prefix: Vec<(String, u64)> = vec![];
    let mut by_class: HashMap<String, (u64, u64)> = HashMap::new();
    for (bi, batch) in plan.chunks(512).enumerate() {
        let res = par_map(batch, o.threads, |chunk| {
            let mut raw = Vec::new();
            let mut ats = vec![];
            for &i in chunk {
                let s = &sel[i];
                let b = s.pf.read(s.e).expect("pak read");
                ats.push(raw.len() as u32);
                raw.extend_from_slice(&b);
            }
            let z = zst(&raw, o.level);
            (raw.len(), ats, z)
        });
        for (chunk, (raw_len, ats, z)) in batch.iter().zip(res) {
            if part.is_none() || part_len + z.len() as u64 > o.part_bytes {
                if let Some(mut w) = part.take() { w.flush().map_err(|e| e.to_string())?; }
                let name = format!("pak-{:03}.bin", ix.files.len() - 1);
                part = Some(std::io::BufWriter::with_capacity(8 << 20, std::fs::File::create(dir.join(&name)).map_err(|e| e.to_string())?));
                ix.files.push(name);
                part_len = 0;
            }
            let file = (ix.files.len() - 1) as u16;
            part.as_mut().unwrap().write_all(&z).map_err(|e| e.to_string())?;
            let ci = ix.chunks.len() as u32;
            ix.chunks.push(wp::Chunk { file, off: part_len, clen: z.len() as u32, raw: raw_len as u32 });
            let is_first = chunk.iter().all(|&i| sel[i].first);
            for (&i, &at) in chunk.iter().zip(&ats) {
                let s = &sel[i];
                ix.entries.insert((pak_ids[&s.pak], s.offset), wp::EntryLoc { chunk: ci, at, size: s.size as u32 });
                let ext = s.path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
                let c = by_class.entry(ext).or_default();
                c.0 += s.size;
                c.1 += (z.len() as u64 * s.size) / (raw_len.max(1) as u64);
            }
            part_len += z.len() as u64;
            wire_total += z.len() as u64;
            if is_first {
                wire_first += z.len() as u64;
                let name = ix.files[file as usize].clone();
                match first_prefix.last_mut() { Some(l) if l.0 == name => l.1 = part_len, _ => first_prefix.push((name, part_len)) }
            }
        }
        if bi % 10 == 0 { eprintln!("[webcook] pak chunks {}/{} ({:.0} MB written, {:.0} s)", ((bi + 1) * 512).min(plan.len()), plan.len(), mb(wire_total), t0.elapsed().as_secs_f32()); }
    }
    if let Some(mut w) = part.take() { w.flush().map_err(|e| e.to_string())?; }
    let raw = ix.to_bytes();
    let z = zst(&raw, 19);
    let mut ob = (raw.len() as u32).to_le_bytes().to_vec();
    ob.extend_from_slice(&z);
    std::fs::write(dir.join(wp::INDEX_FILE), ob).map_err(|e| e.to_string())?;
    let raw_total: u64 = sel.iter().map(|s| s.size).sum();
    println!("pak entries: {} entries in {} chunks: {:.1} MB -> {:.1} MB zstd ({:.3}); index {:.2} MB", sel.len(), ix.chunks.len(), mb(raw_total), mb(wire_total), wire_total as f64 / raw_total.max(1) as f64, mb(z.len() as u64));
    println!("  recorded-session section: {} entries, {:.1} MB -> {:.1} MB; prefix: {:?}", n_first, mb(first_raw), mb(wire_first), first_prefix);
    let mut bc: Vec<_> = by_class.into_iter().collect();
    bc.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
    for (k, (r, w)) in bc.iter().take(12) { println!("  .{k:<8} {:>8.1} MB -> {:>8.1} MB", mb(*r), mb(*w)); }
    std::fs::write(o.out.join("pak-first-prefix.json"), serde_json::to_vec_pretty(&first_prefix.iter().map(|(n, e)| serde_json::json!({"file": format!("pak/{n}"), "end": e})).collect::<Vec<_>>()).unwrap()).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn verify_pak(set: &PakSet, o: &Opts, every: usize) -> Result<(), String> {
    let ix = wp::Index::open(&o.out.join("pak"))?;
    let (mut ok, mut bad) = (0usize, 0usize);
    for p in set.paks() {
        if let Some(r) = ix.pak_index(&p.name) {
            let (len, tail, idx) = r?;
            let b = p.bytes();
            let good = len == b.len() as u64 && tail == b[b.len() - tail.len()..] && idx == b[p.info.index_offset as usize..(p.info.index_offset + p.info.index_size) as usize];
            if good { ok += 1 } else { bad += 1; eprintln!("[webcook] PAK INDEX MISMATCH {}", p.name); }
        }
    }
    let by_off: HashMap<(String, u64), (&fn_pak::PakFile, &fn_pak::PakEntry)> = set.paks().iter().flat_map(|p| p.entries.iter().map(move |e| ((p.name.to_ascii_lowercase(), e.offset), (p, e)))).collect();
    let mut keys: Vec<(&(u16, u64), &wp::EntryLoc)> = ix.entries.iter().collect();
    keys.sort_by_key(|(k, l)| (l.chunk, k.1));
    for ((pid, off), l) in keys.into_iter().step_by(every.max(1)) {
        let (pf, e) = by_off[&(ix.paks[*pid as usize].name.to_ascii_lowercase(), *off)];
        let a = pf.read(e).map_err(|x| x.to_string())?;
        let b = ix.read_entry(l)?;
        if a.as_ref() == b.as_slice() { ok += 1 } else { bad += 1; eprintln!("[webcook] ENTRY MISMATCH {}", e.path); }
    }
    println!("verify pak: {ok} identical (pak indices + every {}th entry, byte for byte vs the paks), {bad} different", every.max(1));
    if bad > 0 { Err(format!("{bad} pak mismatches")) } else { Ok(()) }
}

fn walk_all(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() { let p = e.path(); if p.is_dir() { walk_all(&p, out) } else { out.push(p) } }
}

pub fn cook_files(o: &Opts) -> Result<(), String> {
    let dir = o.out.join("files");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut all = vec![];
    walk_all(&o.assets, &mut all);
    let mut rank: HashMap<String, usize> = HashMap::new();
    for m in &o.manifests { for (p, _, _, i) in crate::analyze::manifest_ranges(m) { rank.entry(p.to_ascii_lowercase()).or_insert(i); } }
    let mut items: Vec<(String, PathBuf, u64)> = vec![];
    for p in all {
        let rel = p.strip_prefix(&o.assets).unwrap().to_string_lossy().replace('\\', "/");
        let lower = rel.to_ascii_lowercase();
        if lower.starts_with("webtex") || lower.starts_with("webcook") { continue; }
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let ext = lower.rsplit('.').next().unwrap_or("");
        if size < 4096 || (size <= 256 << 10 && matches!(ext, "json" | "txt" | "ini" | "csv")) || matches!(ext, "png" | "ogg" | "mp3" | "jpg" | "webm") { continue; }
        items.push((format!("/fn/assets/{lower}"), p, size));
    }
    items.sort_by_key(|(k, _, _)| (rank.get(k).copied().unwrap_or(usize::MAX), k.clone()));
    let res = par_map(&items, o.threads, |(_, p, _)| { let b = std::fs::read(p).unwrap(); let z = zst(&b, o.level); (b.len(), z) });
    let name = "files-000.bin".to_string();
    let mut w = std::io::BufWriter::new(std::fs::File::create(dir.join(&name)).map_err(|e| e.to_string())?);
    let mut idx: Vec<u8> = b"WCF1".to_vec();
    let s = |o: &mut Vec<u8>, v: &str| { o.extend_from_slice(&(v.len() as u16).to_le_bytes()); o.extend_from_slice(v.as_bytes()); };
    idx.extend_from_slice(&1u32.to_le_bytes());
    s(&mut idx, &name);
    let mut ents = vec![];
    let (mut off, mut raw_t, mut first_end) = (0u64, 0u64, 0u64);
    for ((k, p, size), (n, z)) in items.iter().zip(res) {
        if z.len() as f64 > 0.92 * n as f64 { continue; }
        w.write_all(&z).map_err(|e| e.to_string())?;
        ents.push((k.clone(), *size, off, z.len() as u32, p.clone()));
        off += z.len() as u64;
        raw_t += n as u64;
        if rank.contains_key(k) { first_end = off; }
    }
    w.flush().map_err(|e| e.to_string())?;
    idx.extend_from_slice(&(ents.len() as u32).to_le_bytes());
    for (k, size, o2, c, _) in &ents { s(&mut idx, k); idx.extend_from_slice(&size.to_le_bytes()); idx.extend_from_slice(&0u16.to_le_bytes()); idx.extend_from_slice(&o2.to_le_bytes()); idx.extend_from_slice(&c.to_le_bytes()); }
    let z = zst(&idx, 19);
    let mut ob = (idx.len() as u32).to_le_bytes().to_vec();
    ob.extend_from_slice(&z);
    std::fs::write(dir.join("index.bin"), ob).map_err(|e| e.to_string())?;
    println!("files: {} files {:.1} MB -> {:.1} MB zstd; recorded-session prefix ends at {} ({:.1} MB)", ents.len(), mb(raw_t), mb(off), first_end, mb(first_end));
    let mut ok = 0;
    let all = std::fs::read(dir.join(&name)).map_err(|e| e.to_string())?;
    for (k, size, o2, c, p) in &ents {
        let a = std::fs::read(p).unwrap();
        let b = fn_pak::webcook::unzstd(&all[*o2 as usize..*o2 as usize + *c as usize], *size as usize)?;
        if a != b { return Err(format!("files verify: {k} differs")); }
        ok += 1;
    }
    println!("verify files: {ok} of {} identical to the assets folder", ents.len());
    Ok(())
}
