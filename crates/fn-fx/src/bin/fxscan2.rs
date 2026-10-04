//! Research tool: every VertexFactory FName (with its instance number) inside each shader entry of a material, next to the entry's uniform buffer names.
use fn_fx::data::package_files;
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pkg = args.first().ok_or("package")?;
    let filter = args.get(1).cloned().unwrap_or_default();
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let (asset, uexp, ubulk) = package_files(&set, pkg)?;
    let p = fn_ueio::Package::parse(&asset, &uexp, &ubulk).map_err(|e| e.to_string())?;
    let header = p.summary.total_header_size as usize;
    let blobs = fn_dxbc::extract::scan_blobs(&uexp);
    let mut from = 0;
    for b in &blobs {
        let sh = fn_dxbc::Shader::parse(&b.bytes).map_err(|e| e.to_string())?;
        let st = fn_dxbc::extract::serialized_stream(&uexp, b, &sh)?;
        let e = fn_dxbc::unreal_entry::named_entry(&uexp, &p.names, from, header, &st)?;
        from = st.tail.end;
        if sh.major < 5 || !e.name.contains(&filter) { continue; }
        let mut vfs = vec![];
        let mut i = e.offset + 12;
        while i + 8 <= st.start {
            let idx = i32::from_le_bytes(uexp[i..i + 4].try_into().unwrap()); let num = i32::from_le_bytes(uexp[i + 4..i + 8].try_into().unwrap());
            if idx >= 0 && (0..=4).contains(&num) { if let Some(n) = p.names.get(idx as usize) { if n.ends_with("VertexFactory") { vfs.push(format!("{n}#{num}@{}", i - e.offset)); } } }
            i += 1;
        }
        let opt = fn_dxbc::unreal_packet::optional_fields(&b.trailer).ok();
        let ubs = opt.and_then(|o| fn_dxbc::unreal_packet::uniform_buffer_names(&o).ok().flatten());
        println!("{:>7} {} {:<58} vf {:?} ubs {:?}", b.id, sh.version_string(), e.name, vfs, ubs);
    }
    Ok(())
}
