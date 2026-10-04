//! Dev tool: `find_unparsed <package> <export index|all> [struct-name filter] [limit]` - prints the raw bytes of every property
//! value the reader kept as `Value::Unparsed` (used to derive native struct layouts). Install root from `$FN_ROOT`.
use fn_pak::PakSet;
use fn_uasset::{PackageData, Value};

fn walk(v: &Value, filter: &str, path: &str, limit: &mut usize) {
    match v {
        Value::Unparsed { ty, bytes, reason } => {
            if ty.contains(filter) && *limit > 0 {
                *limit -= 1;
                println!("{path}: {ty} size={} reason={reason}\n   {}", bytes.len(), bytes.iter().take(256).map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(""));
            }
        }
        Value::Array(a) | Value::Set(a) => a.iter().enumerate().for_each(|(i, x)| walk(x, filter, &format!("{path}[{i}]"), limit)),
        Value::Map(m) => m.iter().enumerate().for_each(|(i, (k, x))| {
            walk(k, filter, &format!("{path}.k{i}"), limit);
            walk(x, filter, &format!("{path}.v{i}"), limit)
        }),
        Value::Struct { props, .. } => props.iter().for_each(|p| walk(&p.value, filter, &format!("{path}.{}", p.name), limit)),
        _ => {}
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let set = PakSet::open_install(&root).unwrap();
    let d = PackageData::load(&set, &a[1]).unwrap();
    let p = d.package().unwrap();
    let filter = a.get(3).cloned().unwrap_or_default();
    let range: Vec<usize> = match a.get(2).map(|s| s.as_str()) {
        Some("all") | None => (0..p.exports.len()).collect(),
        Some(s) => vec![s.parse().unwrap()],
    };
    let mut limit = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(6usize);
    for i in range {
        let o = p.read_export(i).unwrap();
        for pr in o.props.iter() {
            walk(&pr.value, &filter, &format!("#{i}.{}", pr.name), &mut limit);
        }
    }
}
