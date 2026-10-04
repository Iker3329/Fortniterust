//! Scratch: print the SoundClass tree (name, parent, volume, flags) and SoundMix summaries.
use fn_pak::PakSet;
use fn_uasset::{PackageData, Value};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let set = PakSet::open_install(&a[1]).unwrap();
    let list = std::fs::read_to_string(&a[2]).unwrap();
    let mut rows = vec![];
    for l in list.lines() {
        let mut it = l.split('\t');
        let p = it.next().unwrap();
        let c = it.next().unwrap_or("");
        if !c.split(',').any(|x| x == "SoundClass") {
            continue;
        }
        let d = PackageData::load(&set, p).unwrap();
        let pk = d.package().unwrap();
        for i in pk.exports_of_class("SoundClass").collect::<Vec<_>>() {
            let o = pk.read_export(i).unwrap();
            let parent = o.get("ParentClass").and_then(Value::as_object).map(|i| pk.object_path(i)).unwrap_or_default();
            let props = o.get("Properties");
            let f = |n: &str| props.and_then(|p| p.field(n)).map(|v| format!("{v:?}")).unwrap_or_default();
            rows.push(format!(
                "{:45} parent={:60} vol={} pitch={} ui={} music={} fx={} alwaysplay={} reverb={}",
                o.name,
                parent.rsplit('/').next().unwrap_or(""),
                f("Volume"),
                f("Pitch"),
                f("bIsUISound"),
                f("bIsMusic"),
                f("bApplyEffects"),
                f("bAlwaysPlay"),
                f("bReverb")
            ));
        }
    }
    rows.sort();
    for r in rows {
        println!("{r}");
    }
}
