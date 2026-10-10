use fn_pak::webcook as wp;
use fn_pak::PakSet;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

fn mb(b: u64) -> f64 { b as f64 / 1048576. }
fn zst(b: &[u8], level: i32) -> Vec<u8> { zstd::bulk::compress(b, level).expect("zstd") }

fn par<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = std::thread::available_parallelism().map_or(8, |n| n.get());
    let slots: Vec<std::sync::Mutex<Option<R>>> = items.iter().map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| { for _ in 0..n { s.spawn(|| loop { let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed); if i >= items.len() { break; } let r = f(&items[i]); *slots[i].lock().unwrap() = Some(r); }); } });
    slots.into_iter().map(|m| m.into_inner().unwrap().unwrap()).collect()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() { let p = e.path(); if p.is_dir() { walk(&p, out) } else { out.push(p) } }
}

pub fn webp_of(lower: &str, bytes: &[u8], quality: f32) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    if lower.contains("height") || lower.contains("weight") { return None; }
    let dec = image::codecs::png::PngDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    if !matches!(dec.color_type(), image::ColorType::Rgba8 | image::ColorType::Rgb8 | image::ColorType::L8 | image::ColorType::La8) { return None; }
    let img = image::DynamicImage::from_decoder(dec).ok()?.to_rgba8();
    let ui = lower.contains("/ui/") || lower.starts_with("/fn/assets/ui/");
    let enc = webp::Encoder::from_rgba(img.as_raw(), img.width(), img.height());
    let w = if ui { enc.encode_lossless() } else { enc.encode(quality) }.to_vec();
    let back = image::load_from_memory(&w).ok()?.to_rgba8();
    if back.dimensions() != img.dimensions() { return None; }
    (w.len() < bytes.len()).then_some(w)
}

pub fn cook_files(assets: &Path, out: &Path, manifests: &[String], level: i32, quality: f32) -> Result<(), String> {
    let dir = out.join("files");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut all = vec![];
    walk(assets, &mut all);
    let mut rank: HashMap<String, usize> = HashMap::new();
    for m in manifests { for (p, _, _, i) in crate::analyze::manifest_ranges(m) { rank.entry(p.to_ascii_lowercase()).or_insert(i); } }
    let mut items: Vec<(String, PathBuf, u64)> = vec![];
    for p in all {
        let rel = p.strip_prefix(assets).unwrap().to_string_lossy().replace('\\', "/");
        let lower = rel.to_ascii_lowercase();
        if lower.starts_with("webtex") || lower.starts_with("webcook") { continue; }
        let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let ext = lower.rsplit('.').next().unwrap_or("");
        if size < 4096 || (size <= 256 << 10 && matches!(ext, "json" | "txt" | "ini" | "csv")) || matches!(ext, "ogg" | "mp3" | "jpg" | "webm") { continue; }
        items.push((format!("/fn/assets/{lower}"), p, size));
    }
    items.sort_by_key(|(k, _, _)| (rank.get(k).copied().unwrap_or(usize::MAX), k.clone()));
    let t0 = std::time::Instant::now();
    let res = par(&items, |(k, p, _)| {
        let b = std::fs::read(p).unwrap();
        let (body, webp) = if k.ends_with(".png") { match webp_of(k, &b, quality) { Some(w) => (w, true), None => (b, false) } } else { (b, false) };
        let z = zst(&body, level);
        (body.len(), z, webp)
    });
    eprintln!("[v2] files encoded in {:.0} s", t0.elapsed().as_secs_f32());
    let name = "files-000.bin".to_string();
    let mut w = std::io::BufWriter::new(std::fs::File::create(dir.join(&name)).map_err(|e| e.to_string())?);
    let mut idx: Vec<u8> = b"WCF2".to_vec();
    let s = |o: &mut Vec<u8>, v: &str| { o.extend_from_slice(&(v.len() as u16).to_le_bytes()); o.extend_from_slice(v.as_bytes()); };
    idx.extend_from_slice(&1u32.to_le_bytes());
    s(&mut idx, &name);
    let mut ents = vec![];
    let (mut off, mut orig_t, mut first_end, mut conv, mut conv_from, mut conv_to) = (0u64, 0u64, 0u64, 0usize, 0u64, 0u64);
    for ((k, _, size), (raw, z, webp)) in items.iter().zip(res) {
        if !webp && z.len() as f64 > 0.92 * raw as f64 { continue; }
        w.write_all(&z).map_err(|e| e.to_string())?;
        ents.push((k.clone(), *size, raw as u64, off, z.len() as u32));
        if webp { conv += 1; conv_from += size; conv_to += z.len() as u64; }
        off += z.len() as u64;
        orig_t += size;
        if rank.contains_key(k) { first_end = off; }
    }
    w.flush().map_err(|e| e.to_string())?;
    idx.extend_from_slice(&(ents.len() as u32).to_le_bytes());
    for (k, size, raw, o2, c) in &ents { s(&mut idx, k); idx.extend_from_slice(&size.to_le_bytes()); idx.extend_from_slice(&raw.to_le_bytes()); idx.extend_from_slice(&0u16.to_le_bytes()); idx.extend_from_slice(&o2.to_le_bytes()); idx.extend_from_slice(&c.to_le_bytes()); }
    let z = zst(&idx, 19);
    let mut ob = (idx.len() as u32).to_le_bytes().to_vec();
    ob.extend_from_slice(&z);
    std::fs::write(dir.join("index.bin"), ob).map_err(|e| e.to_string())?;
    println!("v2 files: {} files ({:.1} MB as stored) -> {:.1} MB; {conv} PNGs as WebP {:.1} -> {:.1} MB; recorded prefix ends at {:.1} MB", ents.len(), mb(orig_t), mb(off), mb(conv_from), mb(conv_to), mb(first_end));
    Ok(())
}

pub fn read_order(path: &str) -> (Vec<String>, Vec<String>) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut mark = u64::MAX;
    let (mut menu, mut rest, mut on) = (vec![], vec![], false);
    for l in text.lines() {
        if let Some(m) = l.strip_prefix("mark ") { if m.ends_with(" picker") { mark = m.split(' ').next().and_then(|x| x.parse().ok()).unwrap_or(u64::MAX); } continue; }
        if l.starts_with("read order:") { on = true; continue; }
        if !on { continue; }
        let t = l.trim();
        let Some((q, p)) = t.split_once(' ') else { continue };
        let Ok(q) = q.parse::<u64>() else { continue };
        if q < mark { menu.push(p.to_owned()) } else { rest.push(p.to_owned()) }
    }
    (menu, rest)
}

pub fn cook_pak(set: &PakSet, v1: &Path, out: &Path, level: i32, reads: Option<&str>) -> Result<(), String> {
    let ix = wp::Index::open(v1)?;
    let dir = out.join("pak");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let by_off: HashMap<(String, u64), (&fn_pak::PakFile, &fn_pak::PakEntry)> = set.paks().iter().flat_map(|p| p.entries.iter().map(move |e| ((p.name.to_ascii_lowercase(), e.offset), (p, e)))).collect();
    let mut per_chunk: Vec<Vec<((u16, u64), u32)>> = vec![vec![]; ix.chunks.len()];
    for (k, l) in &ix.entries { per_chunk[l.chunk as usize].push((*k, l.at)); }
    for c in &mut per_chunk { c.sort_by_key(|x| x.1); }
    let sizes: HashMap<(u16, u64), u32> = ix.entries.iter().map(|(k, l)| (*k, l.size)).collect();
    let key_of = |p: &str| -> Option<(u16, u64)> { let (pf, e) = set.find(&format!("FortniteGame/Content/{p}")).or_else(|| set.find(p))?; let id = *ix.pak_ids.get(&pf.name.to_ascii_lowercase())?; sizes.contains_key(&(id, e.offset)).then_some((id, e.offset)) };
    let mut seen: std::collections::HashSet<(u16, u64)> = Default::default();
    let mut groups: Vec<Vec<(u16, u64)>> = vec![];
    let (menu, rest) = reads.map(read_order).unwrap_or_default();
    for list in [menu, rest] { groups.push(list.iter().filter_map(|p| key_of(p)).filter(|k| seen.insert(*k)).collect()); }
    groups.push(per_chunk.iter().flatten().map(|(k, _)| *k).filter(|k| seen.insert(*k)).collect());
    eprintln!("[v2] order: {} menu entries, {} later reads, {} other", groups[0].len(), groups[1].len(), groups[2].len());
    let mut per_chunk: Vec<Vec<((u16, u64), u32)>> = vec![];
    for g in &groups { let mut cur = vec![]; let mut len = 0u64; for k in g { let n = sizes[k] as u64; if n >= 128 << 10 { per_chunk.push(vec![(*k, 0)]); continue; } cur.push((*k, 0)); len += n; if len >= 256 << 10 { per_chunk.push(std::mem::take(&mut cur)); len = 0; } } if !cur.is_empty() { per_chunk.push(cur); } }
    let mut nix = wp::Index { files: vec!["idx.bin".into()], paks: ix.paks.clone(), ..Default::default() };
    let idx_src = std::fs::read(v1.join("idx.bin")).map_err(|e| format!("v1 idx.bin: {e} (download it next to index.bin)"))?;
    std::fs::write(dir.join("idx.bin"), &idx_src).map_err(|e| e.to_string())?;
    let mut cls: HashMap<String, bool> = HashMap::new();
    let t0 = std::time::Instant::now();
    let (mut wire, mut raw_t, mut stripped) = (0u64, 0u64, 0usize);
    let mut part: Option<std::io::BufWriter<std::fs::File>> = None;
    let mut part_len = 0u64;
    let ids: Vec<usize> = (0..per_chunk.len()).collect();
    for batch in ids.chunks(512) {
        let todo: Vec<Vec<(String, (u16, u64), Vec<u8>)>> = batch.iter().map(|&ci| per_chunk[ci].iter().filter_map(|(k, _)| {
            let (pf, e) = by_off.get(&(ix.paks[k.0 as usize].name.to_ascii_lowercase(), k.1))?;
            Some((e.rel_path().to_owned(), *k, pf.read(e).ok()?.to_vec()))
        }).collect()).collect();
        let shader_pkg: Vec<Vec<bool>> = todo.iter().map(|ents| ents.iter().map(|(p, _, _)| {
            let l = p.to_ascii_lowercase();
            if !l.ends_with(".uexp") { return false; }
            *cls.entry(l).or_insert_with(|| { let c = crate::analyze::package_class(set, p); c.starts_with("MaterialInstanceConstant") || c.starts_with("Material") || c.starts_with("World") })
        }).collect()).collect();
        let res = par(&(0..batch.len()).collect::<Vec<_>>(), |&bi| {
            let mut raw = Vec::new();
            let mut ats = vec![];
            let mut n = 0;
            for (j, (p, k, b)) in todo[bi].iter().enumerate() {
                let body = if shader_pkg[bi][j] { match crate::shaders::strip_checked(set, p, b) { Some(s) => { n += 1; s } None => b.clone() } } else { b.clone() };
                ats.push((*k, raw.len() as u32, body.len() as u32));
                raw.extend_from_slice(&body);
            }
            let z = zst(&raw, level);
            (raw.len(), ats, z, n)
        });
        for (raw_len, ats, z, n) in res {
            if part.is_none() || part_len + z.len() as u64 > 256 << 20 {
                if let Some(mut w) = part.take() { w.flush().map_err(|e| e.to_string())?; }
                let name = format!("pak-{:03}.bin", nix.files.len() - 1);
                part = Some(std::io::BufWriter::with_capacity(8 << 20, std::fs::File::create(dir.join(&name)).map_err(|e| e.to_string())?));
                nix.files.push(name);
                part_len = 0;
            }
            let file = (nix.files.len() - 1) as u16;
            part.as_mut().unwrap().write_all(&z).map_err(|e| e.to_string())?;
            let ci = nix.chunks.len() as u32;
            nix.chunks.push(wp::Chunk { file, off: part_len, clen: z.len() as u32, raw: raw_len as u32 });
            for (k, at, size) in ats { nix.entries.insert(k, wp::EntryLoc { chunk: ci, at, size }); }
            part_len += z.len() as u64; wire += z.len() as u64; raw_t += raw_len as u64; stripped += n;
        }
        eprintln!("[v2] pak chunks {}/{} ({:.0} MB, {:.0} s)", nix.chunks.len(), ix.chunks.len(), mb(wire), t0.elapsed().as_secs_f32());
    }
    if let Some(mut w) = part.take() { w.flush().map_err(|e| e.to_string())?; }
    let raw = nix.to_bytes();
    let z = zst(&raw, 19);
    let mut ob = (raw.len() as u32).to_le_bytes().to_vec();
    ob.extend_from_slice(&z);
    std::fs::write(dir.join(wp::INDEX_FILE), ob).map_err(|e| e.to_string())?;
    let v1_wire: u64 = ix.chunks.iter().map(|c| c.clen as u64).sum();
    println!("v2 pak: {} chunks, {} entries, {stripped} shader packages stripped; {:.1} MB raw -> {:.1} MB (v1 {:.1} MB)", nix.chunks.len(), nix.entries.len(), mb(raw_t), mb(wire), mb(v1_wire));
    Ok(())
}

pub fn menu_bytes(set: &PakSet, dir: &Path, reads: &str) {
    let ix = wp::Index::open(dir).expect("index");
    let (menu, rest) = read_order(reads);
    for (name, list) in [("menu", &menu), ("match", &rest)] {
        let mut chunks = std::collections::BTreeSet::new();
        let mut raw = 0u64;
        let mut kinds: std::collections::BTreeMap<&str, f64> = Default::default();
        for p in list.iter() {
            let Some((pf, e)) = set.find(&format!("FortniteGame/Content/{p}")).or_else(|| set.find(p)) else { continue };
            let Some(l) = ix.find(&pf.name, e.offset) else { continue };
            chunks.insert(l.chunk); raw += l.size as u64;
            let c = &ix.chunks[l.chunk as usize];
            *kinds.entry(kind_of(p)).or_default() += c.clen as f64 * l.size as f64 / c.raw.max(1) as f64;
        }
        let wire: u64 = chunks.iter().map(|&c| ix.chunks[c as usize].clen as u64).sum();
        println!("{name}: {} entries, {} chunks, {:.1} MB raw read, {:.1} MB to download", list.len(), chunks.len(), mb(raw), mb(wire));
        let mut kv: Vec<_> = kinds.into_iter().collect();
        kv.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (k, b) in kv { println!("  {:>7.1} MB  {k}", b / 1048576.); }
    }
}

pub fn cook_tex(set: &PakSet, tsvs: &[String], v1_tex: &Path, out: &Path, level: i32) -> Result<(), String> {
    use fn_fx::webcook as wt;
    fn_render::fx::set_bc_native_hint(true);
    let v1 = if v1_tex.exists() { wt::open_index(v1_tex)? } else { wt::Index { dir: v1_tex.to_owned(), files: vec![], map: Default::default() } };
    let mut want: std::collections::BTreeSet<String> = Default::default();
    for t in tsvs {
        for line in std::fs::read_to_string(t).unwrap_or_default().lines() {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 2 || !f[1].starts_with("Texture2D.") { continue; }
            let rel = f[0].strip_prefix("FortniteGame/Content/").unwrap_or(f[0]);
            let Some(dot) = rel.rfind('.') else { continue };
            let long = format!("/Game/{}", &rel[..dot]);
            if v1.map.contains_key(&long.to_ascii_lowercase()) { continue; }
            let leaf = long.rsplit('/').next().unwrap_or("").to_owned();
            want.insert(format!("{long}.{leaf}"));
        }
    }
    let items: Vec<String> = want.into_iter().collect();
    eprintln!("[v2] tex supplement: {} textures not in the v1 tex pack", items.len());
    let dir = out.join("tex");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let res = par(&items, |p| fn_fx::gpu::decode_texture_from_paks(set, p).ok().map(|d| (p.clone(), d)));
    let mut files: Vec<std::io::BufWriter<std::fs::File>> = wt::TIER_FILES.iter().map(|n| std::io::BufWriter::new(std::fs::File::create(dir.join(n)).unwrap())).collect();
    let mut lens = [0u64; 4];
    let mut texs = vec![];
    let mut raw_total = 0u64;
    for (path, (name, info, format, lv)) in res.into_iter().flatten() {
        let long = path.split('.').next().unwrap_or(&path).to_owned();
        let fields = wt::bc_fields(format);
        let mut groups: Vec<(u8, usize, usize, Vec<u8>)> = vec![];
        for (i, l) in lv.iter().enumerate() {
            let t = wt::tier_of(l.width, l.height);
            let bytes = match fields { Some(f) => wt::split_fields(&l.rgba8, f), None => l.rgba8.clone() };
            raw_total += bytes.len() as u64;
            match groups.last_mut() { Some(g) if g.0 == t => { g.2 += 1; g.3.extend_from_slice(&bytes); } _ => groups.push((t, i, 1, bytes)) }
        }
        let mut tiers = vec![];
        for (t, first, count, data) in groups {
            let z = zst(&data, level);
            let ti = t as usize;
            files[ti].write_all(&z).map_err(|e| e.to_string())?;
            tiers.push(wt::Tier { tier: t, first: first as u8, count: count as u8, file: t as u16, off: lens[ti], clen: z.len() as u32, xf: fields.is_some() as u8 });
            lens[ti] += z.len() as u64;
        }
        let key = long.to_ascii_lowercase();
        let fname = fn_fx::webtex::format_name(format);
        texs.push(wt::Tex { exact: crate::cook::is_exact(&key, fname), key, name, format, info: info.clone(), source: (info.width as u32, info.height as u32), levels: lv.iter().map(|l| (l.width, l.height, l.rgba8.len() as u32)).collect(), tiers });
    }
    for f in &mut files { f.flush().map_err(|e| e.to_string())?; }
    let names: Vec<String> = wt::TIER_FILES.iter().map(|s| s.to_string()).collect();
    let raw = wt::to_bytes(&names, &texs);
    wt::write_index(&dir.join(wt::INDEX_FILE), &raw, &zst(&raw, 19)).map_err(|e| e.to_string())?;
    let capped: u64 = lens[..3].iter().sum();
    println!("v2 tex supplement: {} textures, {:.1} MB raw BC; zstd per tier low {:.1} / 512 {:.1} / 1k {:.1} / 2k {:.1} MB; download at the 1024 cap {:.1} MB", texs.len(), mb(raw_total), mb(lens[0]), mb(lens[1]), mb(lens[2]), mb(lens[3]), mb(capped));
    Ok(())
}

static TEST_TEX: std::sync::OnceLock<fn_fx::webcook::Index> = std::sync::OnceLock::new();
fn test_levels(p: &str) -> Option<Vec<(u32, u32, Vec<u8>)>> {
    let ix = TEST_TEX.get()?;
    let t = ix.find(p)?;
    let d = ix.read(t, 2).ok()?;
    d.2.is_bc().then(|| d.3.into_iter().map(|l| (l.width, l.height, l.rgba8)).collect())
}

pub fn pose_check(set: &PakSet, dir: &Path, paths: &[String]) {
    fn_render::fx::set_bc_native_hint(true);
    let _ = TEST_TEX.set(fn_fx::webcook::open_index(dir).expect("tex"));
    fn_pose::loader::set_cooked_texture_source(test_levels);
    let src = fn_pose::clip::ClipSourceDir::Pak(set);
    for p in paths {
        let (long, object) = p.split_once('.').unwrap();
        let pkg = fn_pose::loader::package(&src, long).unwrap();
        let idx = pkg.exports.iter().position(|e| e.object_name == object).unwrap();
        let orig = fn_texture::Texture2D::parse_export(&pkg, idx).unwrap();
        let hooked = fn_pose::loader::texture_export(&src, p);
        match hooked {
            Err(e) => println!("{p}: hook path error {e}"),
            Ok(h) => {
                let m = h.best_mip().unwrap();
                let a = h.decode(m, fn_texture::DecodeOptions::default()).unwrap();
                let om = orig.mips.iter().position(|x| x.width == h.mips[m].width && x.height == h.mips[m].height).unwrap();
                let b = orig.decode(om, fn_texture::DecodeOptions::default()).unwrap();
                let (ra, rb) = (a.to_rgba8(), b.to_rgba8());
                let diff: u64 = ra.iter().zip(&rb).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs() as u64).sum();
                let mean: u64 = ra.iter().map(|x| *x as u64).sum::<u64>() / ra.len().max(1) as u64;
                println!("{p}: {:?} best mip {m} {}x{} (orig top {}x{}), mean {mean}, abs diff vs original same-size mip {}", h.format, a.width, a.height, orig.size_x, orig.size_y, diff);
            }
        }
    }
}

fn kind_of(p: &str) -> &'static str {
    let n = p.rsplit('/').next().unwrap_or(p);
    let base = n.split('.').next().unwrap_or(n);
    let has = |s: &str| p.contains(s);
    if p.starts_with("Packages/") { "packages" }
    else if base.starts_with("T_") || base.starts_with("T-") || has("/Textures/") || has("/Texture/") { "textures" }
    else if base.starts_with("SM_") || base.starts_with("S_") || base.starts_with("SK_") || has("/Meshes/") || has("/Mesh/") { "meshes" }
    else if base.starts_with("M_") || base.starts_with("MI_") || base.starts_with("MF_") || has("/Materials/") || has("/Material/") { "materials" }
    else if has("Sounds/") || has("Audio") { "audio" }
    else if n.ends_with(".locres") { "localization" }
    else { "other" }
}
