//! Scratch (weapon audio task): for every BR (Athena) ranged WID, walk the weapon Blueprint class chain (CDO properties, child first) and print the sound / animation
//! properties plus the sound notifies of every montage they name, as Markdown (the source of parity/guns/weapon-audio.md).
//! usage: weaponaudio <install> > out.md        (`--json` prints a machine-readable `bp\tkey\tvalue` list instead)
use fn_pak::PakSet;
use fn_uasset::{PackageData, Value};
use std::collections::BTreeMap;

fn long_of(p: &str) -> String {
    p.split('.').next().unwrap_or(p).to_string()
}
fn pak_path(long: &str) -> String {
    format!("FortniteGame/Content/{}.uasset", long.trim_start_matches("/Game/"))
}
fn val_str(pkg: &fn_uasset::Package, v: &Value) -> Option<String> {
    match v {
        Value::Object(i) if !i.is_null() => Some(pkg.object_path(*i)),
        Value::Object(_) => Some("<null>".to_string()), // an explicit null in a child CDO overrides the inherited value
        Value::SoftObject(s) if !s.is_empty() => Some(s.clone()),
        Value::Name(s) | Value::Str(s) => Some(s.clone()),
        Value::Float(f) => Some(format!("{f}")),
        Value::Bool(b) => Some(format!("{b}")),
        Value::Int(i) => Some(format!("{i}")),
        _ => None,
    }
}
/// (class, merged CDO props) child first.
fn chain(set: &PakSet, bp_long: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut out = vec![];
    let mut cur = bp_long.to_string();
    for _ in 0..8 {
        if !cur.starts_with("/Game/") {
            break;
        }
        let Ok(d) = PackageData::load(set, &pak_path(&cur)) else { break };
        let Ok(pkg) = d.package() else { break };
        let mut props = BTreeMap::new();
        if let Some(c) = pkg.default_object_export() {
            if let Ok(o) = pkg.read_export(c) {
                for p in o.props.iter() {
                    let key = if p.index > 0 { format!("{}[{}]", p.name, p.index) } else { p.name.clone() };
                    // arrays of objects (PrimaryFireSound is a static array in 1.8; others TArray)
                    if let Some(a) = p.value.as_array() {
                        for (i, x) in a.iter().enumerate() {
                            if let Some(s) = val_str(&pkg, x) {
                                props.insert(format!("{}[{}]", p.name, i), s);
                            }
                        }
                    } else if let Some(s) = val_str(&pkg, &p.value) {
                        props.insert(key, s);
                    }
                }
            }
        }
        let name = cur.rsplit('/').next().unwrap().to_string();
        let mut next = String::new();
        for i in 0..pkg.exports.len() {
            if pkg.exports[i].name.ends_with("_C") && !pkg.exports[i].name.starts_with("Default__") && pkg.export_class_name(i) == "BlueprintGeneratedClass" {
                next = long_of(&pkg.object_path(pkg.exports[i].super_index));
            }
        }
        out.push((name, props));
        cur = next;
    }
    out
}
fn interesting(k: &str) -> bool {
    let base = k.split('[').next().unwrap();
    if base.starts_with("ImpactPhysicalSurface") || base.starts_with("FireAudioChannels") {
        return false;
    }
    base.contains("Sound") || base.contains("Animation") || base.contains("Montage") || base == "UseShellsOnPump?"
}
/// time-ordered sound notifies of a montage / sequence package.
fn notifies(set: &PakSet, long: &str) -> Vec<String> {
    let Ok(d) = PackageData::load(set, &pak_path(long)) else { return vec!["<load failed>".into()] };
    let Ok(pkg) = d.package() else { return vec![] };
    let mut v: Vec<(f32, String)> = vec![];
    let Some(mi) = pkg.exports_of_class("AnimMontage").next() else { return vec![] };
    let Ok(m) = pkg.read_export(mi) else { return vec![] };
    if let Some(arr) = m.props.get("Notifies").and_then(Value::as_array) {
        for n in arr {
            let t = n.field("LinkValue").and_then(Value::as_f32).unwrap_or(0.) + n.field("TriggerTimeOffset").and_then(Value::as_f32).unwrap_or(0.);
            let Some(ob) = n.field("Notify").and_then(Value::as_object).and_then(|i| i.export_index()) else { continue };
            let cls = pkg.export_class_name(ob);
            if cls.starts_with("AnimNotify_PlaySound") || cls.starts_with("AnimNotify_PlayReloadFX") {
                if let Ok(o) = pkg.read_export(ob) {
                    let desc = if cls.starts_with("AnimNotify_PlayReloadFX") {
                        format!("PlayReloadFX({})", o.props.get("ReloadStage").and_then(Value::as_str).map(|s| s.rsplit("::").next().unwrap_or(s).to_string()).unwrap_or("ReloadStart (class default)".into()))
                    } else {
                        let cue = o.props.get("Sound").map(|s| val_str(&pkg, s).unwrap_or_default()).unwrap_or_default();
                        let pm = o.props.get("PitchMultiplier").and_then(Value::as_f32).unwrap_or(1.);
                        let vm = o.props.get("VolumeMultiplier").and_then(Value::as_f32).unwrap_or(1.);
                        let follow = o.props.get("bFollow").and_then(Value::as_bool).unwrap_or(false);
                        NOTIFY.with(|n| { n.borrow_mut().entry(long_of(&cue)).or_insert(pm); });
                        format!("PlaySound `{}` pitch {pm} vol {vm}{}", long_of(&cue), if follow { " follow" } else { "" })
                    };
                    v.push((t, desc));
                }
            }
        }
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().map(|(t, s)| format!("{t:.4}s {s}")).collect()
}

/// `Duration` of a SoundCue package (10000 = loops indefinitely).
fn cue_duration(set: &PakSet, long: &str) -> f32 {
    let Ok(d) = PackageData::load(set, &pak_path(long)) else { return 0. };
    let Ok(pkg) = d.package() else { return 0. };
    let Some(ci) = pkg.exports_of_class("SoundCue").next() else { return 0. };
    pkg.read_export(ci).ok().and_then(|o| o.props.get("Duration").and_then(Value::as_f32)).unwrap_or(0.)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let set = PakSet::open_install(&a[1]).unwrap();
    let mut wids: Vec<String> = set.glob("*WID_*Athena*.uasset").into_iter().map(|(_, e)| e.rel_path().to_owned()).collect();
    wids.sort();
    wids.dedup();
    let mut by_bp: BTreeMap<String, Vec<(String, BTreeMap<String, String>)>> = BTreeMap::new();
    let mut wid_bp: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rel in &wids {
        let long = format!("/Game/{}", rel.trim_start_matches("FortniteGame/Content/").trim_end_matches(".uasset"));
        if long.contains("Harvest") {
            continue;
        }
        let Ok(d) = PackageData::load(&set, rel) else { continue };
        let Ok(pkg) = d.package() else { continue };
        let Ok(o) = pkg.read_export(0) else { continue };
        let bp = o.props.get("WeaponActorClass").and_then(|v| val_str(&pkg, v)).map(|s| long_of(&s)).unwrap_or_default();
        let mut wid_props = BTreeMap::new();
        for k in ["PickupSound", "DropSound", "DestroyedSound"] {
            if let Some(s) = o.props.get(k).and_then(|v| val_str(&pkg, v)) {
                wid_props.insert(k.to_string(), s);
            }
        }
        let name = long.rsplit('/').next().unwrap().to_string();
        wid_bp.entry(bp.clone()).or_default().push(format!("{name} (pickup `{}`, drop `{}`)", long_of(wid_props.get("PickupSound").map(String::as_str).unwrap_or("-")), long_of(wid_props.get("DropSound").map(String::as_str).unwrap_or("-"))));
        if !bp.is_empty() && !by_bp.contains_key(&bp) {
            by_bp.insert(bp.clone(), chain(&set, &bp));
        }
    }
    let json = a.iter().any(|x| x == "--json");
    let rs = a.iter().any(|x| x == "--rs");
    let mut rows: Vec<(String, String, String)> = vec![];
    let mut bps: Vec<(String, Vec<String>)> = vec![];
    let mut fx: Vec<(String, bool, f32, [bool; 3])> = vec![];
    if !json && !rs {
        println!("# BR weapon audio (Fortnite 1.8, Athena)\n\nGenerated by `crates/fn-audio/examples/weaponaudio.rs` from the 1.8 paks: WID -> `WeaponActorClass` Blueprint -> CDO properties walked up the `SuperStruct` chain (child value wins), montage notifies read from the montage packages. Paths are long package names. `ReloadSounds[0..2]` = Start / Cartridge / End (stage = array index, exe `AFortWeapon::PlayReloadFX` virtual, RVA 0x935350). `PrimaryFireSound[0..2]` = Normal / LowAmmo / Degraded channels: channel 0 always plays, channel 1 layers when magazine <= clip x `LowAmmoPercentage` (default 0.25, RVA 0x9454f0 / 0x915f10), channel 2 from durability (`DegradedPercentage` default 0.1). `PrimaryFireStopSound[i]` plays at the weapon when the fire FX stops (RVA 0x93e020). `OutOfAmmoSound` plays only when the magazine is short and CanReload fails (RVA 0x9416d0). `<null>` = the child CDO explicitly nulls the inherited value. `PlayReloadFX(Max_None)` is a sentinel stage (no sound).\n");
    }
    for (bp, ws) in &wid_bp {
        let ch = by_bp.get(bp).cloned().unwrap_or_default();
        let mut merged: BTreeMap<String, (String, String)> = BTreeMap::new(); // key -> (value, class)
        for (cls, props) in ch.iter().rev() {
            for (k, v) in props {
                merged.insert(k.clone(), (v.clone(), cls.clone()));
            }
        }
        if bp.is_empty() {
            continue;
        }
        if rs {
            // normalise `X` -> `X[0]`; only object values that are not nulled
            let mut norm: BTreeMap<(String, usize), (String, String)> = BTreeMap::new();
            for (k, (v, c)) in &merged {
                if v == "<null>" || !v.starts_with("/Game/") {
                    continue;
                }
                let (base, idx) = match k.split_once('[') {
                    Some((b, r)) => (b.to_string(), r.trim_end_matches(']').parse::<usize>().unwrap_or(0)),
                    None => (k.clone(), 0),
                };
                norm.insert((base, idx), (long_of(v), c.clone()));
            }
            let bpn = bp.rsplit('/').next().unwrap();
            let mut push = |ev: String, cue: &str, prov: String| rows.push((ev, cue.to_string(), prov));
            for ((base, idx), (cue, c)) in &norm {
                let ev = match (base.as_str(), idx) {
                    ("PrimaryFireSound", 0) => "fire".to_string(),
                    ("PrimaryFireSound", 1) => "fire.lowammo".to_string(),
                    ("PrimaryFireSound", 2) => "fire.degraded".to_string(),
                    ("PrimaryFireStopSound", 0) => "fire.stop".to_string(),
                    ("PrimaryFireStopSound", 1) => "fire.stop.lowammo".to_string(),
                    ("PrimaryFireStopSound", 2) => "fire.stop.degraded".to_string(),
                    ("OutOfAmmoSound", _) => "empty".to_string(),
                    ("ReloadSounds", 0) => "reload.start".to_string(),
                    ("ReloadSounds", 1) => "reload.cartridge".to_string(),
                    ("ReloadSounds", 2) => "reload.end".to_string(),
                    ("ImpactPhysicalSurfaceSounds", n) => format!("impact.s{n}"),
                    _ => continue,
                };
                push(format!("bp.{bpn}.{ev}"), cue, format!("{bpn} CDO chain .{base}[{idx}] (set in {c})"));
            }
            // fire-FX mode (exe AFortWeapon vfunc_201 0x9355d0): `bPersistentFireFX` (AFortWeaponRanged +0xabc bit 4, default false), fade-out time (+0x4d8, ctor default 0.25),
            // and per channel whether the cue loops indefinitely (UE `GetDuration` = 10000 >= the exe's DAT_144008e54 threshold 10000.0)
            let persistent = merged.get("bPersistentFireFX").map(|v| v.0 == "true").unwrap_or(false);
            let fade: f32 = merged.get("PrimaryFireSoundFadeOutTime").and_then(|v| v.0.parse().ok()).unwrap_or(0.25);
            let mut loops = [false; 3];
            for i in 0..3 {
                if let Some((cue, _)) = norm.get(&("PrimaryFireSound".to_string(), i)) {
                    loops[i] = cue_duration(&set, cue) >= 10000.0;
                }
            }
            fx.push((bpn.to_string(), persistent, fade, loops));
            bps.push((bpn.to_string(), ws.iter().map(|w| w.split(' ').next().unwrap().to_string()).collect::<Vec<_>>()));
            for (k, (v, _)) in &merged {
                if (k.contains("Animation") || k.contains("Montage")) && v.starts_with("/Game/") {
                    let _ = notifies(&set, &long_of(v));
                }
            }
            continue;
        }
        if json {
            for (k, (v, c)) in &merged {
                if interesting(k) {
                    println!("{bp}\t{k}\t{}\t{c}", long_of(v));
                }
            }
            continue;
        }
        println!("## {}\n", bp.rsplit('/').next().unwrap_or(bp));
        println!("WIDs: {}\n", ws.iter().map(|w| format!("`{w}`")).collect::<Vec<_>>().join(", "));
        println!("Class chain: {}\n", ch.iter().map(|c| c.0.clone()).collect::<Vec<_>>().join(" -> "));
        println!("| property | asset | defined in |\n|---|---|---|");
        let mut montages = vec![];
        for (k, (v, c)) in &merged {
            if interesting(k) {
                println!("| {k} | `{}` | {c} |", long_of(v));
                if (k.contains("Animation") || k.contains("Montage")) && v.starts_with("/Game/") && !montages.contains(&long_of(v)) {
                    montages.push(long_of(v));
                }
            }
        }
        println!();
        for m in montages {
            let n = notifies(&set, &m);
            println!("- montage `{}`: {}", m, if n.is_empty() { "no sound notifies".to_string() } else { n.join("; ") });
        }
        println!();
    }
    if rs {
        println!("//! GENERATED by `crates/fn-audio/examples/weaponaudio.rs --rs` from the 1.8 paks (do not edit by hand): every BR (Athena) ranged weapon Blueprint's sound properties");
        println!("//! (CDO walked up the SuperStruct chain, child value wins, explicit nulls respected) and every `AnimNotify_PlaySound` cue of the montages the Blueprints name.");
        println!("use crate::events::Source::Recovered;\nuse crate::weapon_events::WeaponCue;\n");
        println!("/// (Blueprint class name, WIDs that use it).\npub const BPS: &[(&str, &[&str])] = &[");
        for (b, w) in &bps {
            println!("    ({b:?}, &[{}]),", w.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join(", "));
        }
        println!("];\n");
        println!("/// (Blueprint, bPersistentFireFX, PrimaryFireSoundFadeOutTime, PrimaryFireSound[i] cue loops indefinitely): `PlayWeaponFireFX` starts ONE sound per trigger hold for persistent weapons,\n/// one per shot otherwise; `StopWeaponFireFX` (stop sounds + fade) only does anything for persistent weapons.\npub const BP_FIRE_FX: &[(&str, bool, f32, [bool; 3])] = &[");
        for (b, p, f, l) in &fx {
            println!("    ({b:?}, {p}, {f:?}, {l:?}),");
        }
        println!("];\n");
        println!("/// `bp.<Blueprint>.<fire|fire.lowammo|fire.degraded|fire.stop|empty|reload.start|reload.cartridge|reload.end|impact.sN>` rows.\npub const BP_TABLE: &[WeaponCue] = &[");
        for (ev, cue, prov) in &rows {
            println!("    WeaponCue {{ event: {ev:?}, cue: {cue:?}, source: Recovered, provenance: {prov:?}, pitch: 1.0 }},");
        }
        println!("];\n");
        println!("/// `notify.<CueName>` rows: every `AnimNotify_PlaySound` cue of the weapon montages (pitch = the notify's PitchMultiplier; all 1.0 unless noted).\npub const NOTIFY_TABLE: &[WeaponCue] = &[");
        let n = NOTIFY.with(|n| n.borrow().clone());
        for (cue, pitch) in n {
            let name = cue.rsplit('/').next().unwrap();
            println!("    WeaponCue {{ event: \"notify.{name}\", cue: {cue:?}, source: Recovered, provenance: \"AnimNotify_PlaySound of a weapon montage\", pitch: {pitch:?} }},");
        }
        println!("];");
    }
}
thread_local! { static NOTIFY: std::cell::RefCell<BTreeMap<String, f32>> = Default::default(); }

