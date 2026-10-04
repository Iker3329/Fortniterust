//! `props refs <needle>`: list every package whose name table contains `needle` (case-insensitive substring).
use super::PropsArgs;
use crate::{parallel, Source};
use fn_uasset::PackageData;

pub fn run(src: &Source, a: &PropsArgs) -> Result<(), String> {
    let needle = a.rest.first().ok_or("needle required")?.to_lowercase();
    let mut pkgs: Vec<String> = vec![];
    for (_, e) in src.set.glob("FortniteGame/Content/**/*.uasset") {
        if let Some(g) = fn_pak::pak_path_to_game(&e.path) {
            if a.rest.get(1).map_or(true, |f| g.to_lowercase().contains(&f.to_lowercase())) {
                pkgs.push(g);
            }
        }
    }
    let hits = parallel(&pkgs, a.jobs, |p| {
        let Ok(pd) = PackageData::load(&src.set, p) else { return None };
        let Ok(pkg) = pd.package() else { return None };
        let m: Vec<&String> = pkg.names.iter().filter(|n| n.to_lowercase().contains(&needle)).collect();
        if m.is_empty() {
            None
        } else {
            Some(format!("{p}  [{}]", m.iter().take(3).map(|s| s.as_str()).collect::<Vec<_>>().join(", ")))
        }
    });
    for h in hits.into_iter().flatten() {
        println!("{h}");
    }
    Ok(())
}
