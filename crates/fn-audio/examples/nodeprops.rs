//! Scratch: property names/types per SoundNode*/SoundCue/SoundAttenuation/SoundClass/SoundMix class over the install.
use fn_pak::PakSet;
use fn_uasset::PackageData;
use rayon::prelude::*;
use std::collections::BTreeMap;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let set = PakSet::open_install(&a[1]).unwrap();
    let list = std::fs::read_to_string(&a[2]).unwrap();
    let want: Vec<&str> = a[3].split(',').collect();
    let paths: Vec<&str> = list.lines().filter(|l| l.split('\t').nth(1).map_or(false, |c| c.split(',').any(|x| want.iter().any(|w| x.starts_with(w))))).map(|l| l.split('\t').next().unwrap()).collect();
    println!("packages {}", paths.len());
    let rows: Vec<(String, String, String, String)> = paths
        .par_iter()
        .flat_map(|p| {
            let mut out = vec![];
            let Ok(d) = PackageData::load(&set, p) else { return out };
            let Ok(pk) = d.package() else { return out };
            for i in 0..pk.exports.len() {
                let c = pk.export_class_name(i);
                if !want.iter().any(|w| c.starts_with(w)) {
                    continue;
                }
                let Ok(o) = pk.read_export(i) else { continue };
                if o.error.is_some() {
                    out.push((c.clone(), "<ERROR>".into(), format!("{:?}", o.error), p.to_string()));
                }
                fn rec(out: &mut Vec<(String, String, String, String)>, c: &str, prefix: &str, props: &fn_uasset::PropertyList) {
                    for pr in props.iter() {
                        let ex = format!("{:?}", pr.value);
                        let name = format!("{prefix}{}", pr.name);
                        out.push((c.to_string(), name.clone(), pr.ty.clone(), ex.chars().take(110).collect()));
                        match &pr.value {
                            fn_uasset::Value::Struct { props, .. } => rec(out, c, &format!("{name}."), props),
                            fn_uasset::Value::Array(a) => {
                                for v in a.iter().take(1) {
                                    if let fn_uasset::Value::Struct { props, .. } = v {
                                        rec(out, c, &format!("{name}[]."), props)
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                rec(&mut out, &c, "", &o.props);
                out.push((c.clone(), "<trailing>".into(), format!("{}", o.trailing.len()), String::new()));
            }
            out
        })
        .collect();
    let mut m: BTreeMap<(String, String, String), (usize, String)> = BTreeMap::new();
    for (c, n, t, ex) in rows {
        let e = m.entry((c, n, t)).or_insert((0, ex));
        e.0 += 1;
    }
    for ((c, n, t), (k, ex)) in m {
        println!("{c:34} {n:34} {t:22} x{k:<6} {ex}");
    }
}

