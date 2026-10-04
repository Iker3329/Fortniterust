//! Dev tool: `dump_words <package> <export index>` - the native payload (after tagged properties + guid flag) as i32 words,
//! for reverse-engineering native serialisation layouts. Install root from `$FN_ROOT`.
use fn_pak::PakSet;
use fn_uasset::PackageData;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let set = PakSet::open_install(&root).unwrap();
    let d = PackageData::load(&set, &a[1]).unwrap();
    let p = d.package().unwrap();
    let i: usize = a[2].parse().unwrap();
    let o = p.read_export(i).unwrap();
    for (n, c) in o.trailing.chunks(4).enumerate() {
        let v = i32::from_le_bytes(c.try_into().unwrap_or([0; 4]));
        println!("{:3}: {:02x?}  {}", n * 4, c, v);
    }
    println!("names: {:?}", &p.names[..p.names.len().min(0)]);
}
