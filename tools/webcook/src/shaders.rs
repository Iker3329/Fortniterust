use fn_pak::PakSet;

pub fn strip_sm4(data: &[u8]) -> (Vec<u8>, usize, usize) {
    let mut out = data.to_vec();
    let (mut kept, mut zeroed) = (0, 0);
    for b in fn_dxbc::extract::scan_blobs(data) {
        let Ok(s) = fn_dxbc::Shader::parse(&b.bytes) else { kept += 1; continue };
        if s.major >= 5 { kept += 1; continue; }
        let Some(h) = b.id.checked_sub(20).and_then(|st| data.get(st..b.id)) else { kept += 1; continue };
        let count = u32::from_le_bytes(h[16..20].try_into().unwrap()) as usize;
        let Some(dst) = out.get_mut(b.id..b.id + count) else { kept += 1; continue };
        dst.fill(0);
        zeroed += 1;
    }
    (out, kept, zeroed)
}

fn sig(c: &fn_fx::cooked::Cooked) -> Vec<(usize, String, String, Vec<u8>)> {
    c.programs.iter().filter(|p| p.shader.major >= 5).map(|p| (p.stream, p.shader_type.clone(), p.vertex_factory.clone(), p.blob.bytes.clone())).collect()
}

pub fn run(set: &PakSet, tsv: &str, level: i32) {
    let text = std::fs::read_to_string(tsv).expect("tsv");
    let (mut before, mut after, mut n, mut bad, mut zeroed) = (0u64, 0u64, 0usize, 0usize, 0usize);
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 2 || !f[0].to_ascii_lowercase().ends_with(".uexp") { continue; }
        if !matches!(f[1], "MaterialInstanceConstant.uexp" | "Material.uexp" | "World(umap).uexp") { continue; }
        let uexp_path = f[0];
        let base = &uexp_path[..uexp_path.len() - 5];
        let Ok(uexp) = set.read(uexp_path) else { continue };
        let Ok(asset) = set.read(&format!("{base}.uasset")).or_else(|_| set.read(&format!("{base}.umap"))) else { continue };
        let (s, _, z) = strip_sm4(&uexp);
        let pa = fn_ueio::Package::parse(&asset, &uexp, &[]);
        let pb = fn_ueio::Package::parse(&asset, &s, &[]);
        let same = match (&pa, &pb) {
            (Ok(a), Ok(b)) => {
                let h = a.summary.total_header_size as usize;
                let ca = fn_fx::cooked::Cooked::parse(&uexp, &a.names, h).map(|c| sig(&c));
                let cb = fn_fx::cooked::Cooked::parse(&s, &b.names, h).map(|c| sig(&c));
                match (ca, cb) { (Ok(x), Ok(y)) => x == y, (Err(_), Err(_)) => true, _ => false }
            }
            (Err(_), Err(_)) => true,
            _ => false,
        };
        if !same { bad += 1; eprintln!("DIFF {uexp_path}"); }
        before += zstd::bulk::compress(&uexp, level).unwrap().len() as u64;
        after += zstd::bulk::compress(if same { &s } else { &uexp }, level).unwrap().len() as u64;
        n += 1; zeroed += z;
        if n % 200 == 0 { eprintln!("{n} files: {:.1} -> {:.1} MB", before as f64 / 1048576., after as f64 / 1048576.); }
    }
    println!("material shader maps: {n} files, {zeroed} SM4 streams zeroed, zstd {:.1} MB -> {:.1} MB, {bad} parse differences", before as f64 / 1048576., after as f64 / 1048576.);
}

pub fn strip_checked(set: &PakSet, uexp_path: &str, uexp: &[u8]) -> Option<Vec<u8>> {
    let base = &uexp_path[..uexp_path.len() - 5];
    let asset = set.read(&format!("{base}.uasset")).or_else(|_| set.read(&format!("{base}.umap"))).ok()?;
    let (s, _, z) = strip_sm4(uexp);
    if z == 0 { return None; }
    let a = fn_ueio::Package::parse(&asset, uexp, &[]).ok()?;
    let b = fn_ueio::Package::parse(&asset, &s, &[]).ok()?;
    let h = a.summary.total_header_size as usize;
    let ca = fn_fx::cooked::Cooked::parse(uexp, &a.names, h).ok().map(|c| sig(&c))?;
    let cb = fn_fx::cooked::Cooked::parse(&s, &b.names, h).ok().map(|c| sig(&c))?;
    (ca == cb).then_some(s)
}
