//! Research tool: dump the record order of a cooked material's FMaterialShaderMap (gun material recovery).
//!   fn-shadermap <package>                list every shader record (map index, type name, stage, vertex factories named inside)
//!   fn-shadermap <package> hex a b        byte dump
//!   fn-shadermap <package> words a b      u32 dump with FName resolution
use fn_dxbc_cli::spec::{default_install, package_bytes};
fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let p = args.first().ok_or("package")?;
    let set = fn_pak::PakSet::open_install(&default_install()).map_err(|e| e.to_string())?;
    let bytes = package_bytes(&set, p)?;
    let ua = set.read_game(p, "uasset").map_err(|e| e.to_string())?;
    let names = fn_dxbc_cli::uniform::package_names(&ua, &bytes)?;
    let pk = fn_uasset::package::Package::parse(&ua, &bytes).map_err(|e| e.to_string())?;
    match args.get(1).map(|s| s.as_str()) {
        Some("words") => {
            let (a, b): (usize, usize) = (args[2].parse().unwrap(), args[3].parse().unwrap());
            let mut i = a;
            while i + 4 <= b {
                let w = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
                let n2 = if i + 8 <= bytes.len() { u32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap()) } else { 99 };
                let note = if (w as usize) < names.len() && n2 <= 4 { format!("  <FName {}_{}>", names[w as usize], n2) } else { String::new() };
                println!("{i:8}: {w:10} {w:#010x} {:12.6e}{note}", f32::from_bits(w));
                i += 4;
            }
            return Ok(());
        }
        Some("hex") => {
            let (a, b): (usize, usize) = (args[2].parse().unwrap(), args[3].parse().unwrap());
            let mut i = a;
            while i < b {
                let e = (i + 16).min(b);
                let h: Vec<String> = bytes[i..e].iter().map(|x| format!("{x:02x}")).collect();
                let t: String = bytes[i..e].iter().map(|&x| if (32..127).contains(&x) { x as char } else { '.' }).collect();
                println!("{i:8}: {:<48} {t}", h.join(" "));
                i = e;
            }
            return Ok(());
        }
        Some("packet") => {
            let id: usize = args[2].parse().unwrap();
            let blobs = fn_dxbc::extract::scan_blobs(&bytes);
            let b = blobs.iter().find(|b| b.id == id).ok_or("no such stream")?;
            let sh = fn_dxbc::Shader::parse(&b.bytes).map_err(|e| e.to_string())?;
            let rt = fn_dxbc::unreal_packet::ResourceTable::parse(&b.header)?;
            let of = fn_dxbc::unreal_packet::optional_fields(&b.trailer)?;
            println!("header bits={:#x} srv={:?} samp={:?} uav={:?} hashes={:x?} tex={:?}", rt.bits, rt.srv_map, rt.sampler_map, rt.uav_map, rt.layout_hashes, rt.texture_map);
            for f in &of { println!("optional '{}' {} bytes {:?}", f.key as char, f.payload.len(), if f.payload.len() < 200 { String::from_utf8_lossy(&f.payload).to_string() } else { String::new() }); }
            let names2 = fn_dxbc::unreal_packet::uniform_buffer_names(&of)?;
            println!("ubo names {:?}", names2);
            let nb = fn_dxbc::unreal_packet::named_bindings(&rt, names2.as_deref(), &sh.decls)?;
            for n in nb { println!("{:?} buf={}({}) member={} reg={} hash={:#x}", n.kind, n.buffer_name, n.binding.buffer, n.binding.member, n.binding.register, n.layout_hash); }
            return Ok(());
        }
        _ => {}
    }
    let blobs = fn_dxbc::extract::scan_blobs(&bytes);
    let starts = [10957usize, 192894, 374831, 556488, 693614, 830740];
    let mut min = 0;
    for b in &blobs {
        let sh = fn_dxbc::Shader::parse(&b.bytes).map_err(|e| e.to_string())?;
        let st = fn_dxbc::extract::serialized_stream(&bytes, b, &sh)?;
        let hdr = pk.summary.total_header_size as usize;
        let ne = fn_dxbc::unreal_entry::named_entry(&bytes, &names, min, hdr, &st)?;
        min = st.tail.end;
        let mut vfs = vec![];
        for i in ne.offset..st.start.saturating_sub(7) {
            let idx = i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
            let num = i32::from_le_bytes(bytes[i + 4..i + 8].try_into().unwrap());
            if idx >= 0 && num == 0 {
                if let Some(n) = names.get(idx as usize) { if n.contains("VertexFactory") { vfs.push(format!("{n}@{}", i - ne.offset)); } }
            }
        }
        let map = starts.iter().rposition(|&s| ne.offset >= s).map(|m| m as i32).unwrap_or(-1);
        println!("map{map} @{} {} stage={} {} vf={:?} stream={}", ne.offset, ne.name, st.frequency, sh.version_string(), vfs, b.id);
    }
    Ok(())
}
