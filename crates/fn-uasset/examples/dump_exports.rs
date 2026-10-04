//! Dev tool: `dump_exports <long package path> [export index]` - header summary, every export with class/size/flags and the
//! raw bytes of the selected export. Install root from `$FN_ROOT`.
use fn_pak::PakSet;
use fn_uasset::PackageData;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let set = PakSet::open_install(&root).unwrap();
    let d = PackageData::load(&set, &a[1]).unwrap();
    let p = d.package().unwrap();
    println!("ver {} lic {} legacy {} header {} asset {} uexp {} names {} imports {} exports {} warn {:?}", p.summary.file_version_ue4, p.summary.file_version_licensee_ue4, p.summary.legacy_file_version, p.summary.total_header_size, d.asset.len(), d.uexp.len(), p.names.len(), p.imports.len(), p.exports.len(), p.warnings);
    println!("engine {:?} customs {}", p.summary.saved_by_engine_version, p.summary.custom_versions.len());
    for (i, e) in p.exports.iter().enumerate() {
        let o = p.read_export(i).unwrap();
        println!("[{i}] {} : {} size {} flags {:#x} props {} trailing {} err {:?}", e.name, o.class, e.serial_size, e.object_flags, o.props.len(), o.trailing.len(), o.error);
    }
    if a.len() > 2 {
        let i: usize = a[2].parse().unwrap();
        let o = p.read_export(i).unwrap();
        println!("{}", serde_json::to_string_pretty(&o.to_json(&p)).unwrap());
        let b = p.export_bytes(i).unwrap();
        println!("{:02x?}", &b[..b.len().min(400)]);
    }
}
