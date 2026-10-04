//! Full-install sweep: parse every `.uasset`/`.umap` of the installs (rayon), read every export, run the
//! class/function/property/enum/table decoders (including a full Kismet pass over every function) and require
//! zero hard failures. Skipped when the installs are not present.

use fn_pak::PakSet;
use fn_uasset::package::ParseOptions;
use fn_uasset::{Package, Value};
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;

#[derive(Default, Debug)]
struct Tally {
    packages: usize,
    clean: usize,
    partial: usize,
    failed: usize,
    exports: usize,
    values: usize,
    raw_values: usize,
    failures: Vec<String>,
}

fn count(v: &Value, raw: &mut usize) -> usize {
    1 + match v {
        Value::Unparsed { .. } => {
            *raw += 1;
            0
        }
        Value::Array(a) | Value::Set(a) => a.iter().map(|x| count(x, raw)).sum(),
        Value::Map(m) => m.iter().map(|(k, x)| count(k, raw) + count(x, raw)).sum(),
        Value::Struct { props, .. } => props.iter().map(|p| count(&p.value, raw)).sum(),
        _ => 0,
    }
}

fn sweep(root: &str, assumed_version: i32) -> Option<Tally> {
    let root = PathBuf::from(root);
    if !root.exists() {
        eprintln!("SKIP: {} not present", root.display());
        return None;
    }
    let set = PakSet::open_install(&root).unwrap();
    let mut files = set.glob("**/*.uasset");
    files.extend(set.glob("**/*.umap"));
    let t0 = Instant::now();
    let tally = files
        .par_iter()
        .fold(Tally::default, |mut t, (pf, e)| {
            t.packages += 1;
            let asset = pf.read(e).unwrap();
            let stem = e.rel_path().rsplit_once('.').unwrap().0.to_string();
            let uexp = set.read(&format!("{stem}.uexp")).ok();
            let uexp: &[u8] = uexp.as_deref().unwrap_or(&[]);
            let pkg = match Package::parse_with(&asset, uexp, ParseOptions { assumed_ue4_version: assumed_version }) {
                Ok(p) => p,
                Err(err) => {
                    t.failed += 1;
                    t.failures.push(format!("{}: header: {err}", e.rel_path()));
                    return t;
                }
            };
            let (mut failed, mut raw_here) = (false, 0usize);
            for i in 0..pkg.exports.len() {
                t.exports += 1;
                match pkg.read_export(i) {
                    Err(err) => {
                        failed = true;
                        t.failures.push(format!("{} #{i}: {err}", e.rel_path()));
                    }
                    Ok(o) => {
                        if let Some(err) = &o.error {
                            failed = true;
                            t.failures.push(format!("{} #{i} {}: {err}", e.rel_path(), o.class));
                            continue;
                        }
                        if let Err(err) = fn_uasset::classes::check_export(&pkg, i, &o) {
                            failed = true;
                            t.failures.push(format!("{} #{i} {}: {err}", e.rel_path(), o.class));
                        }
                        for p in o.props.iter() {
                            t.values += count(&p.value, &mut raw_here);
                        }
                    }
                }
            }
            t.raw_values += raw_here;
            if failed {
                t.failed += 1;
            } else if raw_here > 0 {
                t.partial += 1;
            } else {
                t.clean += 1;
            }
            t
        })
        .reduce(Tally::default, |mut a, b| {
            a.packages += b.packages;
            a.clean += b.clean;
            a.partial += b.partial;
            a.failed += b.failed;
            a.exports += b.exports;
            a.values += b.values;
            a.raw_values += b.raw_values;
            a.failures.extend(b.failures);
            a
        });
    eprintln!(
        "{}: {} packages, {} exports, {:.3}% clean, {:.3}% with raw values, {} failed; {:.4}% of {} property values decoded; {:?} on {} threads",
        root.display(),
        tally.packages,
        tally.exports,
        100.0 * tally.clean as f64 / tally.packages as f64,
        100.0 * tally.partial as f64 / tally.packages as f64,
        tally.failed,
        100.0 * (tally.values - tally.raw_values) as f64 / tally.values as f64,
        tally.values,
        t0.elapsed(),
        rayon::current_num_threads()
    );
    for f in tally.failures.iter().take(20) {
        eprintln!("FAILURE {f}");
    }
    Some(tally)
}

#[test]
fn sweep_1_8() {
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    if let Some(t) = sweep(&root, fn_uasset::package::DEFAULT_ASSUMED_UE4_VERSION) {
        assert!(t.packages > 100_000, "expected the full install, got {} packages", t.packages);
        assert_eq!(t.failed, 0, "hard failures");
        assert!(t.clean as f64 / t.packages as f64 > 0.995, "clean ratio");
        assert!((t.values - t.raw_values) as f64 / t.values as f64 > 0.9995, "decoded value ratio");
    }
}

/// Fortnite 1.10 packages are unversioned too but already use 4.18 soft object paths (FName + FString), so the
/// assumed version must be >= 514.
#[test]
fn sweep_1_10() {
    let root = std::env::var("FN_ROOT_110")
        .unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.10-Cert-CL-3790078\1.10-Cert-CL-3790078\Fortnite\Fortnite\FortniteGame".into());
    if let Some(t) = sweep(&root, 514) {
        assert!(t.packages > 100_000);
        assert_eq!(t.failed, 0, "hard failures");
        assert!(t.clean as f64 / t.packages as f64 > 0.99, "clean ratio");
    }
}

/// Mutation fuzzing on *real* packages (blueprints with bytecode, data tables, anim blueprints, components):
/// random byte damage must never panic, over-allocate or hang. Header and body are damaged independently.
#[test]
fn fuzz_real_packages() {
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let root = PathBuf::from(root);
    if !root.exists() {
        eprintln!("SKIP: install missing");
        return;
    }
    let set = PakSet::open_install(&root).unwrap();
    let names = [
        "/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint",
        "/Game/Balance/DataTables/Athena/SeasonXpTable",
        "/Game/Balance/DataTables/ConningTables",
        "/Game/TimeOfDay/TODM/TODM_A_Child_Athena",
        "/Game/Athena/HUD/AthenaHUD",
        "/Game/Abilities/Player/Pawns/PlayerPawn_Generic",
        "/Game/Blueprints/Camera/Original/MainPlayerCamera",
    ];
    let mut seed = 0xdead_beef_1234_5678u64;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut ran = 0;
    for name in names {
        let Ok(d) = fn_uasset::PackageData::load(&set, name) else { continue };
        for _ in 0..150 {
            let mut asset = d.asset.to_vec();
            let mut uexp = d.uexp.to_vec();
            for _ in 0..(1 + rnd() % 5) {
                let header = rnd() % 4 == 0 || uexp.is_empty();
                let t = if header { &mut asset } else { &mut uexp };
                let i = (rnd() as usize) % t.len();
                match rnd() % 3 {
                    0 => t[i] = rnd() as u8,
                    1 => t[i] ^= 1 << (rnd() % 8),
                    _ => {
                        for k in 0..4 {
                            if i + k < t.len() {
                                t[i + k] = [0xff, 0x7f, 0x00, 0x80][k];
                            }
                        }
                    }
                }
            }
            let Ok(pkg) = Package::parse(&asset, &uexp) else { continue };
            ran += 1;
            for i in 0..pkg.exports.len().min(400) {
                if let Ok(o) = pkg.read_export(i) {
                    let _ = fn_uasset::classes::check_export(&pkg, i, &o);
                    let _ = o.to_json(&pkg);
                }
            }
        }
    }
    assert!(ran > 100, "fuzz exercised too few packages ({ran})");
}