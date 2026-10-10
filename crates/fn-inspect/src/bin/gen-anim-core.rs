//! `gen-anim-core [--root <install>] [out.rs]`: generates `crates/fn-animgraph/src/anim_core.rs` straight from the 1.8 paks, in pure Rust (port of the part of the
//! former Node tool `tools/state-machine-evidence.cjs` that wrote CORE_ANIMS_MALE; the .cjs is not in this repo any more).
//!  * CORE_ANIMS_MALE: `CoreAnims_Male` (TMap<EFortWeaponCoreAnimation, PlayerAnimAssets_Struct>) of the `Fortnite_M_Avg_Player_AnimBlueprint` CDO, every field;
//!  * ANIM_DEFAULTS_MALE: the AnimBP's `Default*_Male` assets (what the graph falls back to when a weapon has no entry / a field is null);
//!  * BR_WEAPON_CORE_ANIM: every Athena (Battle Royale) item definition with a weapon actor -> the `WeaponCoreAnimation` its actor class resolves to
//!    (read from the Blueprint CDO, walking the Blueprint super-class chain; items whose chain reaches a native class without the property are listed as `""`).
//! Read-only on the install.
use fn_pak::PakSet;
use fn_uasset::PackageData;
use serde_json::Value;

const DEFAULT_ROOT: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame";
const ANIMBP: &str = "/Game/Characters/Player/Male/Male_Avg_Base/Fortnite_M_Avg_Player_AnimBlueprint";

/// `Foo_12_0123456789ABCDEF0123456789ABCDEF` -> `Foo` (Blueprint struct member names carry `_<index>_<GUID>`).
fn field_base(n: &str) -> &str {
    let b = n.as_bytes();
    if b.len() > 34 && b[b.len() - 33] == b'_' && b[b.len() - 32..].iter().all(|c| c.is_ascii_hexdigit()) {
        let head = &n[..n.len() - 33];
        if let Some(p) = head.rfind('_') { if head[p + 1..].bytes().all(|c| c.is_ascii_digit()) { return &head[..p]; } }
    }
    n
}
/// `/Game/x/Y.Y` -> `Y`; null -> "".
fn short(v: &Value) -> String { v.as_str().map(|s| s.rsplit('/').next().unwrap().split('.').next().unwrap().to_string()).unwrap_or_default() }

fn cdo_props(set: &PakSet, pkg_path: &str) -> Option<Value> {
    let d = PackageData::load(set, pkg_path).ok()?;
    let pkg = d.package().ok()?;
    let i = pkg.default_object_export()?;
    Some(pkg.read_export(i).ok()?.to_json(&pkg)["properties"].clone())
}
fn super_of(set: &PakSet, pkg_path: &str) -> Option<String> {
    let d = PackageData::load(set, pkg_path).ok()?;
    let pkg = d.package().ok()?;
    let j = fn_uasset::classes::package_classes_json(&pkg);
    let list = if j.is_array() { j.as_array().unwrap().clone() } else { vec![j] };
    list.iter().find(|c| c["kind"] == "class").and_then(|c| c["super"].as_str().map(|s| s.to_string()))
}
/// Package part of an object path (`/Game/a/b.b_C` -> `/Game/a/b`).
fn pkg_of(obj: &str) -> &str { obj.split('.').next().unwrap() }

/// The `WeaponCoreAnimation` an actor class resolves to: the first Blueprint CDO in its super chain that serializes the property.
/// `AFortWeapon::WeaponCoreAnimation` (UPROPERTY at +0xA80) as the 1.8 exe constructors initialise it (static read of decomp `AFortWeapon__AFortWeapon`: writes 0 = Melee;
/// `AFortWeaponRanged__AFortWeaponRanged`: writes 1 = Pistol; `AFortWeap_BuildingTool__AFortWeap_BuildingTool` (RVA 0x140914770) and `AFortWeap_EditingTool__AFortWeap_EditingTool`
/// (RVA 0x140914890) write `*(u8*)(param_1 + 0x150)` with `param_1: u64*`, i.e. byte +0xA80, = 3 = PaperBlueprint (BUILD ANIM HOOK; the earlier "keep Melee" reading missed the u64 scaling).
/// Blueprint CDOs only serialize it when it differs.
fn native_default(class: &str) -> String {
    match class { "/Script/FortniteGame.FortWeaponRanged" => "Pistol", "/Script/FortniteGame.FortWeapon" => "Melee", "/Script/FortniteGame.FortWeap_BuildingTool" | "/Script/FortniteGame.FortWeap_EditingTool" => "PaperBlueprint", _ => "" }.to_string()
}
/// Returns (core animation, "") when a Blueprint serializes it, else (the native constructor default, the native class the Blueprint chain ends in).
fn resolve_core(set: &PakSet, actor_class: &str) -> (String, String) {
    let mut path = pkg_of(actor_class).to_string();
    for _ in 0..32 {
        if let Some(p) = cdo_props(set, &path) { if let Some(c) = p["WeaponCoreAnimation"].as_str() { return (c.rsplit("::").next().unwrap().to_string(), String::new()); } }
        match super_of(set, &path) { Some(s) if s.starts_with("/Game/") => path = pkg_of(&s).to_string(), Some(s) => return (native_default(&s), s), None => return (String::new(), String::new()) }
    }
    (String::new(), String::new())
}

// ---- BR_WEAPON_ASSETS / WEAPON_MONTAGES: per-weapon montage, ADS-camera, muzzle and sound bindings of every BR ranged WID, from its OWN Blueprint chain ----
type Props = std::collections::BTreeMap<String, Value>;
/// Merged CDO properties of a weapon Blueprint chain (child first, first value wins; an explicit null is kept as `Null` so it overrides the parent), plus the class names of the chain.
fn chain_props(set: &PakSet, actor_class: &str) -> (Props, Vec<String>) {
    let mut merged = Props::new();
    let mut names = vec![];
    let mut path = pkg_of(actor_class).to_string();
    for _ in 0..32 {
        names.push(path.rsplit('/').next().unwrap().to_string());
        if let Some(Value::Object(p)) = cdo_props(set, &path) { for (k, v) in p { merged.entry(k).or_insert(v); } }
        match super_of(set, &path) { Some(s) if s.starts_with("/Game/") => path = pkg_of(&s).to_string(), _ => break }
    }
    (merged, names)
}
/// Property value as text: "" = no Blueprint in the chain serializes it; "<null>" = a Blueprint explicitly nulls it; object paths are reduced to the package path.
fn prop_str(m: &Props, k: &str) -> String {
    match m.get(k) {
        None => String::new(),
        Some(Value::Null) => "<null>".into(),
        Some(Value::String(s)) => if s.starts_with("/Game/") { pkg_of(s).to_string() } else { s.rsplit("::").next().unwrap().to_string() },
        Some(Value::Bool(b)) => b.to_string(),
        Some(v) if v.is_number() => v.to_string(),
        Some(_) => "<struct>".into(),
    }
}
struct MontageFacts { path: String, class: String, length: f64, slot: String, segments: usize, notifies: usize, sounds: Vec<String>, reload_fx: Vec<String> }
fn montage_facts(set: &PakSet, long: &str) -> Option<MontageFacts> {
    let d = PackageData::load(set, &format!("FortniteGame/Content/{}.uasset", long.trim_start_matches("/Game/"))).ok()?;
    let pkg = d.package().ok()?;
    let i = (0..pkg.exports.len()).find(|&i| matches!(pkg.export_class_name(i).as_str(), "AnimMontage" | "AnimComposite"))?;
    let class = pkg.export_class_name(i);
    let j = pkg.read_export(i).ok()?.to_json(&pkg)["properties"].clone();
    let track = j["SlotAnimTracks"].as_array().and_then(|a| a.first());
    let slot = track.and_then(|t| t["SlotName"].as_str()).unwrap_or("").to_string();
    let segments = track.and_then(|t| t["AnimTrack"]["AnimSegments"].as_array()).map_or_else(|| j["AnimationTrack"]["AnimSegments"].as_array().map_or(0, |a| a.len()), |a| a.len());
    let (mut sounds, mut reload_fx) = (vec![], vec![]);
    for e in 0..pkg.exports.len() {
        let c = pkg.export_class_name(e);
        if c.starts_with("AnimNotify_PlaySound") { if let Ok(o) = pkg.read_export(e) { sounds.push(short(&o.to_json(&pkg)["properties"]["Sound"])); } }
        else if c.starts_with("AnimNotify_PlayReloadFX") { if let Ok(o) = pkg.read_export(e) { reload_fx.push(o.to_json(&pkg)["properties"]["ReloadStage"].as_str().map(|s| s.rsplit("::").next().unwrap().to_string()).unwrap_or_else(|| "ReloadStart".into())); } }
    }
    Some(MontageFacts { path: long.to_string(), class, length: j["SequenceLength"].as_f64().unwrap_or(0.), slot, segments, notifies: j["Notifies"].as_array().map_or(0, |a| a.len()), sounds, reload_fx })
}

fn unique_name(set: &PakSet, v: &Value) -> String {
    let n = short(v);
    if n.is_empty() || !v.as_str().unwrap_or("").contains("/Female/") { return n; }
    if set.glob(&format!("fortnitegame/content/**/{}.uasset", n.to_lowercase())).len() == 1 { n } else { pkg_of(v.as_str().unwrap()).to_string() }
}
fn core_table(p: &Value, map: &str, name: &str, nm: &dyn Fn(&Value) -> String) -> String {
    let mut rs = format!("pub static {name}:&[(&str,CoreAnimSet)]=&[\n");
    let mut seen_fields = std::collections::BTreeSet::new();
    for e in p[map].as_array().unwrap_or_else(|| panic!("{map}")) {
        let key = e["key"].as_str().unwrap().rsplit("::").next().unwrap();
        let v = e["value"].as_object().unwrap();
        let get = |name: &str| -> Value { v.iter().find(|(k, _)| field_base(k) == name).map(|(_, x)| x.clone()).unwrap_or_else(|| panic!("{key}: field {name} missing")) };
        for k in v.keys() { seen_fields.insert(field_base(k).to_string()); }
        let s = |name: &str| format!("{:?}", nm(&get(name)));
        rs += &format!(" ({key:?},CoreAnimSet{{ao_full_body:{},targeting_pose:{},targeting_ao:{},nontargeting_pose:{},nontargeting_ao:{},relaxed_pose:{},relaxed_pose2:{},relaxed_ao:{},jog_additive:{},jog_additive_relaxed:{},jog_additive_relaxed2:{},sprint:{},crouch_targeting_pose:{},crouch_nontargeting_pose:{},crouch_relaxed_pose:{},crouch_walk_additive:{},crouch_jog_additive:{},crouch_jog_additive_relaxed:{},crouch_sprint:{}}}),\n",
            get("ShouldAimOffsetsApplytoFullBody").as_bool().unwrap(), s("TargetingPose"), s("TargetingAimOffset"), s("Non-targetingPose"), s("Non-targetingAimOffset"), s("RelaxedPose"), s("RelaxedPoseLevel2"), s("RelaxedAimOffset"),
            s("JogAdditiveBlendspace"), s("JogAdditiveBlendspaceRelaxed"), s("JogAdditiveBlendspaceRelaxedLevel2"), s("SprintAnimation"), s("CrouchTargetingPose"), s("CrouchNon-targetingPose"), s("CrouchRelaxedPose"),
            s("CrouchWalkAdditiveBlendspace"), s("CrouchJogAdditiveBlendspace"), s("CrouchJogAdditiveBlendspaceRelaxed"), s("CrouchSprintAnimation"));
    }
    rs += "];\n";
    eprintln!("{map}: PlayerAnimAssets_Struct fields in the paks: {:?}", seen_fields);
    rs
}

fn main() {
    let mut root = std::env::var("FN_ROOT").unwrap_or_else(|_| DEFAULT_ROOT.to_string());
    let mut out_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fn-animgraph/src/anim_core.rs").to_string();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() { match a.as_str() { "--root" => root = it.next().unwrap(), o => out_path = o.to_string() } }
    let set = PakSet::open_install(&root).unwrap_or_else(|e| panic!("open {root}: {e}"));
    let p = cdo_props(&set, ANIMBP).expect("AnimBP CDO");

    let mut rs = String::from("// GENERATED by `cargo run -p fn-inspect --bin gen-anim-core` (crates/fn-inspect/src/bin/gen-anim-core.rs) from CoreAnims_Male (TMap<EFortWeaponCoreAnimation, PlayerAnimAssets_Struct> in the\n// Fortnite_M_Avg_Player_AnimBlueprint CDO) -- do not edit by hand. \"\" = the map entry is null.\n");
    rs += "pub struct CoreAnimSet{pub ao_full_body:bool,pub targeting_pose:&'static str,pub targeting_ao:&'static str,pub nontargeting_pose:&'static str,pub nontargeting_ao:&'static str,pub relaxed_pose:&'static str,pub relaxed_pose2:&'static str,pub relaxed_ao:&'static str,pub jog_additive:&'static str,pub jog_additive_relaxed:&'static str,pub jog_additive_relaxed2:&'static str,pub sprint:&'static str,pub crouch_targeting_pose:&'static str,pub crouch_nontargeting_pose:&'static str,pub crouch_relaxed_pose:&'static str,pub crouch_walk_additive:&'static str,pub crouch_jog_additive:&'static str,pub crouch_jog_additive_relaxed:&'static str,pub crouch_sprint:&'static str}\n";
    let male_name = |v: &Value| short(v);
    rs += &core_table(&p, "CoreAnims_Male", "CORE_ANIMS_MALE", &male_name);
    rs += "/// The AnimBP's own `Default*_Male` assets (the fallbacks of the weapon-state selection).\npub static ANIM_DEFAULTS_MALE:&[(&str,&str)]=&[\n";
    for k in ["DefaultJogAdditive_Male", "DefaultCrouchJogAdditive_Male", "DefaultCrouchWalkAdditive", "DefaultSprint_Male", "DefaultCrouchSprint_Male", "DefaultUpperBodyPose_Male", "DefaultCrouchUpperBodyPose_Male", "DefaultAimOffset"] {
        rs += &format!(" ({k:?},{:?}),\n", short(&p[k]));
    }
    rs += "];\n";

    // every Athena item definition with a weapon actor class
    let mut rows: Vec<(String, String, String, String)> = vec![];
    let mut files: Vec<String> = vec![];
    let mut ranged: Vec<(String, String)> = vec![];
    for pat in ["FortniteGame/Content/Athena/Items/Weapons/**/*.uasset", "FortniteGame/Content/Athena/Items/Consumables/**/*.uasset", "FortniteGame/Content/Athena/Items/Traps/**/*.uasset",
                "FortniteGame/Content/Athena/Items/Schematics/Weapons/**/*.uasset", "FortniteGame/Content/Items/Weapons/BuildingTools/**/*.uasset", "FortniteGame/Content/Athena/Items/Gadgets/**/*.uasset"] {
        for (_, e) in set.glob(pat) { files.push(e.rel_path().to_string()); }
    }
    files.sort(); files.dedup();
    for f in files {
        let Ok(d) = PackageData::load(&set, &f) else { continue };
        let Ok(pkg) = d.package() else { continue };
        let Some(i) = pkg.default_object_export().or_else(|| (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i).ends_with("ItemDefinition"))) else { continue };
        let Ok(obj) = pkg.read_export(i) else { continue };
        let j = obj.to_json(&pkg);
        let Some(actor) = j["properties"]["WeaponActorClass"].as_str() else { continue };
        let name = f.rsplit('/').next().unwrap().trim_end_matches(".uasset").to_string();
        let (core, native) = resolve_core(&set, actor);
        if f.contains("/Athena/Items/Weapons/WID_") && pkg.export_class_name(i).contains("WeaponRangedItemDefinition") { ranged.push((name.clone(), actor.to_string())); }
        rows.push((name, short(&Value::String(actor.to_string())), core, native));
    }
    rs += "/// Athena (Battle Royale) item definition -> (weapon actor class, `WeaponCoreAnimation` of its Blueprint CDO chain, native class the chain ends in when no Blueprint serializes it: then the core animation is that class\x27s constructor default, see native_default in gen-anim-core.rs).\npub static BR_WEAPON_CORE_ANIM:&[(&str,&str,&str,&str)]=&[\n";
    for r in &rows { rs += &format!(" ({:?},{:?},{:?},{:?}),\n", r.0, r.1, r.2, r.3); }
    rs += "];\n";
    // BR_WEAPON_ASSETS -> crates/fn-animgraph/src/weapon_assets.rs (sibling of the output file)
    let mut ws = String::from("// GENERATED by `cargo run -p fn-inspect --bin gen-anim-core` (crates/fn-inspect/src/bin/gen-anim-core.rs) from the 1.8 paks (every BR ranged WID -> its WeaponActorClass Blueprint chain CDOs) -- do not edit by hand.
");
    ws += "/// One BR ranged weapon: every binding read from its OWN `WeaponActorClass` Blueprint chain (child first, first value wins). \"\" = no Blueprint in the chain serializes the property; \"<null>\" = a Blueprint explicitly nulls it (the data says none). Object values are package paths.\n/// `equip`/`fire`/`fire_ads`/`reload` = EquipAnimation / FireAnimation / FireDownsightsAnimation / ReloadAnimation (player montages); `w_*` = WeaponFireMontage / WeaponFireDownsightsMontage / WeaponReloadMontage (weapon-mesh montages).\n";
    ws += "pub struct WeaponAssets{pub wid:&'static str,pub actor:&'static str,pub chain:&'static [&'static str],pub equip:&'static str,pub fire:&'static str,pub fire_ads:&'static str,pub reload:&'static str,pub w_fire:&'static str,pub w_fire_ads:&'static str,pub w_reload:&'static str,pub camera_base:&'static str,pub camera_targeting:&'static str,pub muzzle_system:&'static str,pub muzzle_socket:&'static str,pub persistent_fire:bool,pub fire_fade_out:&'static str,pub fire_sound:[&'static str;3],pub fire_stop_sound:[&'static str;3],pub out_of_ammo:&'static str,pub reload_sound:[&'static str;3],pub scope_zoom_in:&'static str,pub extras:&'static [(&'static str,&'static str)]}\n";
    ws += "pub static BR_WEAPON_ASSETS:&[WeaponAssets]=&[\n";
    let mut montages: std::collections::BTreeSet<String> = Default::default();
    let mut chains: std::collections::BTreeMap<String, (Props, Vec<String>)> = Default::default();
    ranged.sort();
    for (wid, actor) in &ranged {
        let (m, names) = chains.entry(actor.clone()).or_insert_with(|| chain_props(&set, actor)).clone();
        let g = |k: &str| prop_str(&m, k);
        for k in ["EquipAnimation", "FireAnimation", "FireDownsightsAnimation", "ReloadAnimation", "WeaponFireMontage", "WeaponFireDownsightsMontage", "WeaponReloadMontage"] { let v = g(k); if v.starts_with("/Game/") { montages.insert(v); } }
        let idx = |k: &str| [g(k), g(&format!("{k}[1]")), g(&format!("{k}[2]"))];
        // the charge / burst / beam / muzzle-FX / pump properties this weapon's chain sets (visible here instead of guessed)
        let extras: Vec<(String, String)> = m.keys().filter(|k| { let b = k.split('[').next().unwrap(); (b.contains("Charge") || b.contains("Burst") || b.contains("Beam") || b.contains("Muzzle") || b.contains("Pump")) && b != "MuzzleParticleSystem" && b != "MuzzleSocketName" }).map(|k| (k.clone(), prop_str(&m, k))).filter(|(_, v)| v != "<struct>").collect();
        ws += &format!(" WeaponAssets{{wid:{wid:?},actor:{:?},chain:&{:?},equip:{:?},fire:{:?},fire_ads:{:?},reload:{:?},w_fire:{:?},w_fire_ads:{:?},w_reload:{:?},camera_base:{:?},camera_targeting:{:?},muzzle_system:{:?},muzzle_socket:{:?},persistent_fire:{},fire_fade_out:{:?},fire_sound:{:?},fire_stop_sound:{:?},out_of_ammo:{:?},reload_sound:{:?},scope_zoom_in:{:?},extras:&{:?}}},\n",
            short(&Value::String(actor.clone())), names, g("EquipAnimation"), g("FireAnimation"), g("FireDownsightsAnimation"), g("ReloadAnimation"), g("WeaponFireMontage"), g("WeaponFireDownsightsMontage"), g("WeaponReloadMontage"), g("CameraBase3PClass"), g("CameraTargeting3PClass"),
            g("MuzzleParticleSystem"), g("MuzzleSocketName"), g("bPersistentFireFX") == "true", g("PrimaryFireSoundFadeOutTime"), idx("PrimaryFireSound"), idx("PrimaryFireStopSound"), g("OutOfAmmoSound"), idx("ReloadSounds"), g("Sound_ScopeZoomIn"), extras);
    }
    ws += "];\n";
    ws += "/// A weapon montage package as the weapon Blueprints name it: class, `SequenceLength`, first slot track name + segment count, total notify count, `AnimNotify_PlaySound` cues and `AnimNotify_PlayReloadFX` stages it carries.\npub struct WeaponMontage{pub path:&'static str,pub class:&'static str,pub length:f32,pub slot:&'static str,pub segments:usize,pub notifies:usize,pub sounds:&'static [&'static str],pub reload_fx:&'static [&'static str]}\npub static WEAPON_MONTAGES:&[WeaponMontage]=&[\n";
    let mut missing_m = vec![];
    for p in &montages {
        match montage_facts(&set, p) {
            Some(m) => ws += &format!(" WeaponMontage{{path:{:?},class:{:?},length:{:?},slot:{:?},segments:{},notifies:{},sounds:&{:?},reload_fx:&{:?}}},\n", m.path, m.class, m.length as f32, m.slot, m.segments, m.notifies, m.sounds, m.reload_fx),
            None => missing_m.push(p.clone()),
        }
    }
    ws += "];\n";
    if !missing_m.is_empty() { eprintln!("montages named by a weapon Blueprint but not loadable: {missing_m:?}"); }
    println!("{} ranged WIDs, {} distinct weapon montages", ranged.len(), montages.len());
    let female_name = |v: &Value| unique_name(&set, v);
    rs += &core_table(&p, "CoreAnims_Female", "CORE_ANIMS_FEMALE", &female_name);
    rs += "pub static ANIM_DEFAULTS_FEMALE:&[(&str,&str)]=&[\n";
    for (k, f) in [("DefaultJogAdditive_Male", "DefaultJogAdditive_Female"), ("DefaultCrouchJogAdditive_Male", "DefaultCrouchJogAdditive_Female"), ("DefaultCrouchWalkAdditive", "DefaultCrouchWalkAdditive"), ("DefaultSprint_Male", "DefaultSprint_Female"), ("DefaultCrouchSprint_Male", "DefaultCrouchSprint_Female"), ("DefaultUpperBodyPose_Male", "DefaultUpperBodyPose_Female"), ("DefaultCrouchUpperBodyPose_Male", "DefaultCrouchUpperBodyPose_Female"), ("DefaultAimOffset", "DefaultAimOffset")] {
        rs += &format!(" ({k:?},{:?}),\n", female_name(&p[f]));
    }
    rs += "];\n";
    std::fs::write(&out_path, &rs).unwrap_or_else(|e| panic!("write {out_path}: {e}"));
    let ws_path = std::path::Path::new(&out_path).with_file_name("weapon_assets.rs");
    std::fs::write(&ws_path, &ws).unwrap_or_else(|e| panic!("write {}: {e}", ws_path.display()));
    println!("wrote {out_path}: {} bytes, {} weapon items", rs.len(), rows.len());
}
