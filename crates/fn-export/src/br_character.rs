//! `fn-export br-character`: resolve and export the Battle Royale (Athena) default player character from the 1.8 packages.
//!
//! Everything comes from the data, nothing is assumed:
//!
//! * `CID_*_Athena_Commando_*_Default` (`AthenaCharacterItemDefinition`) -> `HeroDefinition` (`HID_*`, `FortHeroType`)
//!   -> `Specializations[]` (`HS_*`, `FortHeroSpecialization`) -> `CharacterParts[]` (`CustomCharacterPart` assets)
//!   -> per part `SkeletalMesh`, `MasterSkeletalMeshes`, `MaterialOverrides`, `AdditionalData` (head data / body anim class).
//! * `HeroDefinition.OverridePawnClass` -> the pawn Blueprint chain (`PlayerPawn_Athena` -> `_Generic` -> `_Generic_Parent`): merged
//!   `CharacterMesh0` (master mesh, anim class, relative location) and `CollisionCylinder` (capsule).
//! * Each part mesh is written as glTF (LOD0, the loader's convention) with the material names the part really uses
//!   (`MaterialOverrides` applied), every material is resolved through its `Parent` chain to `.mat` texture slots
//!   (`Diffuse=`, `Normal=`, `SpecPower=`, `Other[0]=` like the old UEViewer files), textures are exported as PNG, and a
//!   plain `Material` whose compiled pixel shaders discard every pixel (`discard_nz l(-1)`) is marked `hidden`.
//! * Files that already exist in the shared assets folder (by name, or a `.bin` with identical content) are reused, never duplicated.
//!
//! Usage: `fn-export br-character --paks <Paks dir> --out <assets dir> [--cid /Game/...CID_005_...] [--report <file.json>]`
//! Output: `<out>/exported/br_character.json` (the runtime manifest) plus the missing meshes/materials/textures.
use crate::props::bp::{merge, package_of};
use crate::source::Source;
use crate::write_file;
use fn_mesh::{gltf, SkeletalMesh, Skeleton};
use fn_texture::{DecodeOptions, Texture2D};
use fn_uasset::{ObjectRef, PackageData};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const DEFAULT_CID: &str = "/Game/Athena/Items/Cosmetics/Characters/CID_005_Athena_Commando_M_Default";
const DEFAULT_CIDS: &[&str] = &[
    "CID_001_Athena_Commando_F_Default",
    "CID_002_Athena_Commando_F_Default",
    "CID_003_Athena_Commando_F_Default",
    "CID_004_Athena_Commando_F_Default",
    "CID_005_Athena_Commando_M_Default",
    "CID_006_Athena_Commando_M_Default",
    "CID_007_Athena_Commando_M_Default",
    "CID_008_Athena_Commando_M_Default",
];

pub struct Args {
    pub paks: PathBuf,
    pub out: PathBuf,
    pub cid: String,
    pub report: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        paks: PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame/Content/Paks"),
        out: PathBuf::from("assets"),
        cid: DEFAULT_CID.to_string(),
        report: None,
    };
    let mut it = args.iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--paks" => a.paks = PathBuf::from(it.next().ok_or("--paks needs a value")?),
            "--out" => a.out = PathBuf::from(it.next().ok_or("--out needs a value")?),
            "--cid" => a.cid = it.next().ok_or("--cid needs a value")?.clone(),
            "--report" => a.report = Some(PathBuf::from(it.next().ok_or("--report needs a value")?)),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

pub fn main(args: &[String]) -> i32 {
    let a = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let src = match Source::open(&a.paks) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    match run(&src, &a) {
        Ok(manifest) => {
            let path = a.out.join("exported").join("br_character.json");
            if let Err(e) = write_file(&path, serde_json::to_string_pretty(&manifest).unwrap().as_bytes()) {
                eprintln!("{e}");
                return 1;
            }
            if let Some(r) = &a.report {
                let _ = write_file(r, serde_json::to_string_pretty(&manifest).unwrap().as_bytes());
            }
            println!("wrote {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("br-character: {e}");
            1
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// data access
// ---------------------------------------------------------------------------------------------------------------------

/// (export class, tagged properties as JSON) of one export; `export` None = the export named like the package (else export 0).
fn props_of(src: &Source, object_path: &str, export: Option<&str>) -> Result<(String, Value), String> {
    let pkg_path = package_of(object_path);
    let pd = PackageData::load(&src.set, pkg_path).map_err(|e| format!("{pkg_path}: {e}"))?;
    let pkg = pd.package().map_err(|e| format!("{pkg_path}: {e}"))?;
    let idx = match export {
        Some(n) => pkg.find_export(n).ok_or_else(|| format!("{pkg_path}: no export {n}"))?,
        None => pkg.find_export(base(pkg_path)).unwrap_or(0),
    };
    let class = pkg.export_class_name(idx).to_string();
    let o = pkg.read_export(idx).map_err(|e| format!("{pkg_path}[{idx}]: {e}"))?;
    Ok((class, o.props.to_json(&pkg)))
}

/// Asset name of an object/package path (`/Game/A/B.B_C` -> `B`; `/Game/A/B` -> `B`).
fn base(p: &str) -> &str {
    let pkg = package_of(p);
    pkg.rsplit('/').next().unwrap_or(pkg)
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str()).filter(|x| !x.is_empty())
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(|x| x.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------------------------------
// character resolution
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Part {
    pub package: String,
    pub name: String,
    /// "head" / "body" / other, from the class of `AdditionalData`
    pub role: String,
    pub part_type_tag: Option<String>,
    pub skeletal_mesh: String,
    pub master_meshes: Vec<String>,
    pub overrides: Vec<(usize, String)>,
    pub override_flags: i64,
    pub data_class: String,
    pub data: Value,
    pub body_types: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Character {
    pub cid: String,
    pub display_name: String,
    pub hid: String,
    pub specializations: Vec<String>,
    pub pawn_class: Option<String>,
    pub parts: Vec<Part>,
}

fn role_of_data_class(c: &str) -> String {
    match c {
        "CustomCharacterHeadData" => "head".into(),
        "CustomCharacterBodyPartData" => "body".into(),
        other => other.to_lowercase(),
    }
}

pub fn resolve_character(src: &Source, cid_path: &str) -> Result<Character, String> {
    let (_, cid) = props_of(src, cid_path, None)?;
    let hid_path = s(&cid, "HeroDefinition").ok_or("CID has no HeroDefinition")?.to_string();
    let (_, hid) = props_of(src, &hid_path, None)?;
    let specs = strings(&hid, "Specializations");
    let mut parts = vec![];
    for sp in &specs {
        let (_, hs) = props_of(src, sp, None)?;
        for cp in strings(&hs, "CharacterParts") {
            let name = base(&cp).to_string();
            let (class, p) = props_of(src, &cp, Some(&name))?;
            if class != "CustomCharacterPart" {
                return Err(format!("{cp}: expected CustomCharacterPart, got {class}"));
            }
            let (data_class, data) = match s(&p, "AdditionalData") {
                Some(ad) => {
                    let en = ad.rsplit(':').next().unwrap_or(ad);
                    props_of(src, ad, Some(en))?
                }
                None => (String::new(), Value::Null),
            };
            let mut overrides = vec![];
            if let Some(a) = p.get("MaterialOverrides").and_then(|x| x.as_array()) {
                for o in a {
                    if let (Some(i), Some(m)) = (o.get("MaterialOverrideIndex").and_then(|x| x.as_i64()), s(o, "OverrideMaterial")) {
                        overrides.push((i as usize, m.to_string()));
                    }
                }
            }
            parts.push(Part {
                package: package_of(&cp).to_string(),
                name,
                role: role_of_data_class(&data_class),
                part_type_tag: s(&p, "CharacterPartType").map(String::from),
                skeletal_mesh: s(&p, "SkeletalMesh").ok_or_else(|| format!("{cp}: no SkeletalMesh"))?.to_string(),
                master_meshes: strings(&p, "MasterSkeletalMeshes"),
                overrides,
                override_flags: p.get("MaterialOverrideFlags").and_then(|x| x.as_i64()).unwrap_or(0),
                data_class,
                data,
                body_types: s(&p, "BodyTypesPermitted").map(String::from),
            });
        }
    }
    let display_name = cid.get("DisplayName").and_then(|d| d.get("text")).and_then(|t| t.as_str()).unwrap_or("").to_string();
    Ok(Character {
        cid: cid_path.to_string(),
        display_name,
        hid: hid_path,
        specializations: specs,
        pawn_class: s(&hid, "OverridePawnClass").map(String::from),
        parts,
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// pawn chain
// ---------------------------------------------------------------------------------------------------------------------

/// Merged `CharacterMesh0` / `CollisionCylinder` templates of a pawn Blueprint chain (child overrides parent).
fn pawn_chain(src: &Source, class_path: &str) -> Result<Value, String> {
    let mut cur = package_of(class_path).to_string();
    let mut layers: Vec<(String, Map<String, Value>, Map<String, Value>, Map<String, Value>)> = vec![];
    let mut chain = vec![];
    for _ in 0..16 {
        let pd = PackageData::load(&src.set, &cur).map_err(|e| format!("{cur}: {e}"))?;
        let pkg = pd.package().map_err(|e| format!("{cur}: {e}"))?;
        chain.push(cur.clone());
        let get = |name: &str| -> Map<String, Value> {
            pkg.find_export(name)
                .and_then(|i| pkg.read_export(i).ok())
                .and_then(|o| if let Value::Object(m) = o.props.to_json(&pkg) { Some(m) } else { None })
                .unwrap_or_default()
        };
        let cdo = pkg.default_object_export().and_then(|i| pkg.read_export(i).ok()).and_then(|o| if let Value::Object(m) = o.props.to_json(&pkg) { Some(m) } else { None }).unwrap_or_default();
        layers.push((cur.clone(), get("CharacterMesh0"), get("CollisionCylinder"), cdo));
        match pkg.super_class() {
            Some(ObjectRef::Import { package: sp, .. }) if sp.starts_with("/Game/") => cur = sp,
            _ => break,
        }
    }
    let (mut mesh, mut cap, mut cdo) = (Map::new(), Map::new(), Map::new());
    for (_, m, c, d) in layers.iter().rev() {
        merge(&mut mesh, m);
        merge(&mut cap, c);
        merge(&mut cdo, d);
    }
    Ok(json!({
        "class": class_path,
        "chain": chain,
        "character_mesh": Value::Object(mesh),
        "collision_cylinder": Value::Object(cap),
        "cdo_mesh_ref": cdo.get("Mesh").cloned().unwrap_or(Value::Null),
        "cdo_capsule_ref": cdo.get("CapsuleComponent").cloned().unwrap_or(Value::Null),
    }))
}

// ---------------------------------------------------------------------------------------------------------------------
// file index (reuse, no duplicates)
// ---------------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Index {
    by_name: HashMap<String, PathBuf>,
    bins: Vec<(u64, PathBuf)>,
}

fn walk(dir: &Path, ix: &mut Index) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, ix);
        } else if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
            let lower = n.to_lowercase();
            if lower.ends_with(".bin") {
                if let Ok(m) = e.metadata() {
                    ix.bins.push((m.len(), p.clone()));
                }
            }
            ix.by_name.entry(lower).or_insert(p);
        }
    }
}

fn rel_path(from_dir: &Path, to: &Path) -> String {
    let f: Vec<_> = from_dir.components().collect();
    let t: Vec<_> = to.components().collect();
    let mut i = 0;
    while i < f.len() && i < t.len() && f[i] == t[i] {
        i += 1;
    }
    let mut v: Vec<String> = (i..f.len()).map(|_| "..".to_string()).collect();
    v.extend(t[i..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()));
    v.join("/")
}

// ---------------------------------------------------------------------------------------------------------------------
// materials
// ---------------------------------------------------------------------------------------------------------------------

struct Resolved {
    chain: Vec<String>,
    root_class: String,
    root_package: String,
    textures: BTreeMap<String, String>,
    scalars: BTreeMap<String, f64>,
    vectors: BTreeMap<String, Value>,
}

fn resolve_material(src: &Source, long: &str) -> Result<Resolved, String> {
    let mut chain = vec![];
    let mut layers: Vec<Value> = vec![];
    let mut cur = package_of(long).trim_start_matches('/').to_string();
    let (mut root_class, mut root_package) = (String::new(), String::new());
    for _ in 0..16 {
        let pkg = src.package(&cur)?;
        let j = crate::mat::material_json(&pkg, base(&cur))?;
        chain.push(cur.clone());
        let class = j["class"].as_str().unwrap_or("").to_string();
        let parent = j["parent"].as_str().filter(|p| !p.is_empty()).map(|p| package_of(p).trim_start_matches('/').to_string());
        layers.push(j);
        match parent {
            Some(p) if class.starts_with("MaterialInstance") => cur = p,
            _ => {
                root_class = class;
                root_package = cur.clone();
                break;
            }
        }
    }
    let (mut textures, mut scalars, mut vectors) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    for j in layers.iter().rev() {
        if let Some(m) = j["textures"].as_object() {
            for (k, v) in m {
                if let Some(t) = v.as_str() {
                    textures.insert(k.clone(), t.to_string());
                }
            }
        }
        if let Some(m) = j["scalars"].as_object() {
            for (k, v) in m {
                if let Some(f) = v.as_f64() {
                    scalars.insert(k.clone(), f);
                }
            }
        }
        if let Some(m) = j["vectors"].as_object() {
            for (k, v) in m {
                vectors.insert(k.clone(), v.clone());
            }
        }
    }
    Ok(Resolved { chain, root_class, root_package, textures, scalars, vectors })
}

fn export_texture(src: &Source, long: &str, out: &Path) -> Result<PathBuf, String> {
    let pkg = package_of(long).trim_start_matches('/').to_string();
    let raw = src.load_raw(&pkg)?;
    let tex = Texture2D::from_files(&raw.uasset, &raw.uexp, &raw.ubulk).map_err(|e| e.to_string())?;
    let mip = tex.best_mip().ok_or("no mip with data")?;
    let img = tex.decode(mip, DecodeOptions::default()).map_err(|e| e.to_string())?;
    let dest = out.join("exported").join(format!("{pkg}.png"));
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
    }
    img.write_png(&dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Pixel-shader evidence of a plain `Material` (compiled shader map inside its `.uexp`): (pixel shaders, with unconditional discard).
fn discard_evidence(src: &Source, package: &str) -> Result<(usize, usize), String> {
    let raw = src.load_raw(package)?;
    let blobs = fn_dxbc::extract::scan_blobs(&raw.uexp);
    let (mut ps, mut discarding) = (0, 0);
    for b in &blobs {
        let Ok(text) = fn_dxbc::disassemble(&b.bytes) else { continue };
        if !text.lines().any(|l| l.trim_start().starts_with("ps_")) {
            continue;
        }
        ps += 1;
        if text.lines().any(|l| l.trim() == "discard_nz l(-1)") {
            discarding += 1;
        }
    }
    Ok((ps, discarding))
}

fn write_mat(path: &Path, r: &Resolved) -> Result<Vec<String>, String> {
    // same keys the old UEViewer `.mat` used (src/materials.rs `parse_mat`): Diffuse / Normal / SpecPower / Other[n]
    let tex = |k: &str| r.textures.get(k).map(|t| base(t).to_string());
    let mut text = String::new();
    let mut names = vec![];
    for (key, param) in [("Diffuse", "Diffuse"), ("Normal", "Normals"), ("SpecPower", "SpecularMasks"), ("Other[0]", "M")] {
        if let Some(t) = tex(param) {
            text.push_str(&format!("{key}={t}\n"));
            names.push(t);
        }
    }
    write_file(path, text.as_bytes())?;
    Ok(names)
}

// ---------------------------------------------------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------------------------------------------------

fn export_material_slot(src: &Source, long: &str, out: &Path, ix: &mut Index, is_override: bool) -> Result<Value, String> {
    let name = base(long).to_string();
    let r = resolve_material(src, long)?;
    let mat_file = format!("{name}.mat").to_lowercase();
    let mut j = json!({
        "material": name, "package": package_of(long), "override": is_override, "chain": r.chain, "root_class": r.root_class,
        "textures": r.textures, "scalars": r.scalars, "vectors": r.vectors,
    });
    // texture PNGs: reuse by file name, export the missing ones
    let mut pngs = vec![];
    for (param, tpath) in &r.textures {
        let tname = base(tpath).to_string();
        let key = format!("{tname}.png").to_lowercase();
        let status = if ix.by_name.contains_key(&key) {
            "reused".to_string()
        } else {
            match export_texture(src, tpath, out) {
                Ok(p) => {
                    ix.by_name.insert(key, p);
                    "exported".to_string()
                }
                Err(e) => format!("MISSING: {e}"),
            }
        };
        pngs.push(json!({"param": param, "texture": tname, "status": status}));
    }
    j["pngs"] = Value::Array(pngs);
    // .mat slot file
    if ix.by_name.contains_key(&mat_file) {
        j["mat_file"] = json!("reused");
    } else {
        let dir = out.join("exported").join(package_of(long).trim_start_matches('/')).parent().map(PathBuf::from).unwrap_or_default();
        let dest = dir.join(format!("{name}.mat"));
        if r.textures.is_empty() {
            j["mat_file"] = json!("none (material has no texture parameters)");
        } else {
            write_mat(&dest, &r)?;
            ix.by_name.insert(mat_file, dest);
            j["mat_file"] = json!("written");
        }
    }
    // a plain Material with no texture slots: look at its compiled pixel shaders for an unconditional discard
    let mut hidden = false;
    if r.root_class == "Material" && r.textures.is_empty() {
        let (_, props) = props_of(src, &format!("/{}", r.root_package), None)?;
        let blend = props.get("BlendMode").and_then(|b| b.as_str()).unwrap_or("").to_string();
        let (ps, disc) = discard_evidence(src, &r.root_package)?;
        hidden = blend == "BLEND_Masked" && disc > 0;
        j["blend_mode"] = json!(blend);
        j["pixel_shaders"] = json!(ps);
        j["pixel_shaders_with_unconditional_discard"] = json!(disc);
        j["hidden_basis"] = json!("BLEND_Masked plain Material without textures whose compiled pass shaders (base pass, shadow depth, depth) contain `discard_nz l(-1)`; the opacity constant itself is not serialized");
    }
    j["hidden"] = json!(hidden);
    Ok(j)
}

fn export_part(src: &Source, p: &Part, out: &Path, ix: &mut Index) -> Result<Value, String> {
    let mesh_pkg = package_of(&p.skeletal_mesh).trim_start_matches('/').to_string();
    let pkg = src.package(&mesh_pkg)?;
    let m = SkeletalMesh::parse(&pkg).map_err(|e| format!("{mesh_pkg}: {e}"))?;
    let mesh_name = base(&mesh_pkg).to_string();
    let gltf_path = out.join("exported").join(format!("{mesh_pkg}.gltf"));
    // per slot final material: the mesh's own slot, replaced by the part's MaterialOverrides (`MaterialOverrideIndex` = slot index)
    let mut slots = vec![];
    for (i, sm) in m.materials.iter().enumerate() {
        let own = sm.material.clone().unwrap_or_default();
        let ov = p.overrides.iter().find(|(idx, _)| *idx == i).map(|(_, m)| m.clone());
        slots.push((i, sm.slot_name.clone(), own, ov));
    }
    let final_of = |i: usize| -> String { slots[i].3.clone().unwrap_or_else(|| slots[i].2.clone()) };
    let lod0 = m.lods.first().ok_or("mesh has no LOD")?;
    let sections: Vec<usize> = (0..lod0.sections.len()).map(|s| m.section_material_index(0, s)).collect();
    // glTF: write only if absent
    let gltf_status = if gltf_path.exists() {
        "reused".to_string()
    } else {
        let (doc, bin) = gltf::skeletal_mesh_to_gltf(&m, &mesh_name);
        let mut v: Value = serde_json::from_str(&doc).map_err(|e| e.to_string())?;
        // material names = what the part actually uses
        let prims = v["meshes"][0]["primitives"].as_array().cloned().unwrap_or_default();
        if prims.len() != sections.len() {
            return Err(format!("{mesh_name}: {} glTF primitives vs {} LOD0 sections", prims.len(), sections.len()));
        }
        for (si, prim) in prims.iter().enumerate() {
            let mi = prim["material"].as_u64().ok_or("primitive without material")? as usize;
            let slot = *sections.get(si).ok_or("section index")?;
            let fin = final_of(slot);
            if fin.is_empty() {
                continue;
            }
            v["materials"][mi]["name"] = json!(base(&fin));
        }
        let dir = gltf_path.parent().unwrap().to_path_buf();
        // bin: identical content already in the assets folder -> point the glTF at it
        let same = ix.bins.iter().find(|(len, p)| *len == bin.len() as u64 && std::fs::read(p).map_or(false, |b| b == bin)).map(|(_, p)| p.clone());
        let bin_status = match same {
            Some(existing) => {
                v["buffers"][0]["uri"] = json!(rel_path(&dir, &existing));
                format!("reused {}", existing.display())
            }
            None => {
                let bp = dir.join(format!("{mesh_name}.bin"));
                write_file(&bp, &bin)?;
                ix.bins.push((bin.len() as u64, bp));
                "written".to_string()
            }
        };
        write_file(&gltf_path, serde_json::to_string_pretty(&v).unwrap().as_bytes())?;
        ix.by_name.insert(format!("{mesh_name}.gltf").to_lowercase(), gltf_path.clone());
        format!("written; bin {bin_status}")
    };
    // materials
    let mut mats = vec![];
    let mut seen: Vec<String> = vec![];
    for (i, slot_name, _own, ov) in &slots {
        let fin = final_of(*i);
        if fin.is_empty() || seen.contains(&fin) {
            continue;
        }
        seen.push(fin.clone());
        mats.push(match export_material_slot(src, &fin, out, ix, ov.is_some()) {
            Ok(j) => j,
            Err(e) => json!({"material": base(&fin), "error": e, "slot_name": slot_name}),
        });
    }
    let hidden: Vec<String> = mats.iter().filter(|m| m["hidden"] == json!(true)).filter_map(|m| m["material"].as_str().map(String::from)).collect();
    let slot_json: Vec<Value> = slots
        .iter()
        .map(|(i, name, own, ov)| {
            let fin = ov.clone().unwrap_or_else(|| own.clone());
            json!({"slot": i, "slot_name": name, "mesh_material": base(own), "material": base(&fin), "override": ov.is_some()})
        })
        .collect();
    let lods: Vec<Value> = m.lods.iter().enumerate().map(|(i, l)| json!({"lod": i, "sections": l.sections.len(), "vertices": l.num_vertices, "triangles": l.indices.len() / 3})).collect();
    Ok(json!({
        "role": p.role, "part": p.name, "part_package": p.package, "part_type_tag": p.part_type_tag, "body_types": p.body_types,
        "skeletal_mesh": p.skeletal_mesh, "gltf": format!("{mesh_name}.gltf"), "gltf_status": gltf_status,
        "master_meshes": p.master_meshes, "material_override_flags": p.override_flags,
        "overrides": p.overrides.iter().map(|(i, m)| json!({"slot": i, "material": m})).collect::<Vec<_>>(),
        "data_class": p.data_class, "data": p.data,
        "skeleton": m.skeleton_path, "bones": m.ref_skeleton.len(),
        "slots": slot_json, "lod0_sections": sections, "lods": lods, "materials": mats, "hidden_materials": hidden,
        "sockets": m.sockets.iter().map(|k| json!({"name": k.name, "bone": k.bone, "location": k.location, "rotation": k.rotation, "scale": k.scale})).collect::<Vec<_>>(),
    }))
}

/// Skeleton comparison of a part mesh against the animation skeleton (the pawn master mesh's skeleton).
fn skeleton_compat(src: &Source, anim: &Skeleton, mesh_long: &str) -> Result<Value, String> {
    let pkg = src.package(package_of(mesh_long).trim_start_matches('/'))?;
    let m = SkeletalMesh::parse(&pkg).map_err(|e| e.to_string())?;
    let skel_path = m.skeleton_path.clone().ok_or("mesh has no Skeleton")?;
    let sk = Skeleton::parse(&src.package(package_of(&skel_path).trim_start_matches('/'))?).map_err(|e| e.to_string())?;
    let (mut not_in_anim, mut pose_diff, mut retarget_diff) = (vec![], vec![], vec![]);
    for b in &sk.bones {
        match anim.bone_index_ci(&b.name) {
            None => not_in_anim.push(b.name.clone()),
            Some(i) => {
                let a = &anim.bones[i];
                let d = (0..3).map(|k| (a.ref_pos[k] - b.ref_pos[k]).abs()).chain((0..4).map(|k| (a.ref_rot[k] - b.ref_rot[k]).abs())).fold(0f32, f32::max);
                if d > 1e-4 {
                    pose_diff.push(json!({"bone": b.name, "max_abs": d}));
                }
                if a.retarget != b.retarget {
                    retarget_diff.push(json!({"bone": b.name, "anim": format!("{:?}", a.retarget), "part": format!("{:?}", b.retarget)}));
                }
            }
        }
    }
    let mesh_bones_not_in_anim: Vec<String> = m.ref_skeleton.iter().filter(|b| anim.bone_index_ci(&b.name).is_none()).map(|b| b.name.clone()).collect();
    let anim_bones_not_in_mesh: Vec<String> = anim.bones.iter().filter(|b| m.bone_index(&b.name).is_none() && !m.ref_skeleton.iter().any(|x| x.name.eq_ignore_ascii_case(&b.name))).map(|b| b.name.clone()).collect();
    Ok(json!({
        "mesh": mesh_long, "skeleton": skel_path, "skeleton_bones": sk.bones.len(), "mesh_bones": m.ref_skeleton.len(),
        "same_skeleton_asset_as_anim": package_of(&skel_path) == package_of(&format!("/Game/Characters/Player/Male/Male_Avg_Base/{}", anim.name)),
        "skeleton_bones_not_in_anim_skeleton": not_in_anim,
        "mesh_bones_not_in_anim_skeleton": mesh_bones_not_in_anim,
        "anim_skeleton_bones_not_in_mesh": anim_bones_not_in_mesh,
        "ref_pose_differs_from_anim_skeleton": pose_diff,
        "translation_retarget_differs_from_anim_skeleton": retarget_diff,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install() -> Option<Source> {
        let p = PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame/Content/Paks");
        if !p.exists() {
            eprintln!("1.8 install not found, skipping");
            return None;
        }
        Source::open(&p).ok()
    }

    /// The default male Recruit resolves to the Athena body (with the BR01 override on slot 0) and the Athena head, on PlayerPawn_Athena.
    #[test]
    fn cid_005_resolves_to_the_athena_parts() {
        let Some(src) = install() else { return };
        let c = resolve_character(&src, DEFAULT_CID).unwrap();
        assert_eq!(c.display_name, "Recruit");
        assert_eq!(base(&c.hid), "HID_005_Athena_Commando_M");
        assert_eq!(c.pawn_class.as_deref(), Some("/Game/Athena/PlayerPawn_Athena.PlayerPawn_Athena_C"));
        assert_eq!(c.parts.len(), 2);
        let head = c.parts.iter().find(|p| p.role == "head").unwrap();
        let body = c.parts.iter().find(|p| p.role == "body").unwrap();
        assert_eq!(base(&head.skeletal_mesh), "SK_M_MED_Head_01_ATH");
        assert_eq!(base(&body.skeletal_mesh), "SK_M_Med_Soldier_04_ATH");
        assert_eq!(body.overrides.len(), 1);
        assert_eq!(body.overrides[0].0, 0);
        assert_eq!(base(&body.overrides[0].1), "M_Med_Soldier_04_CAU_BR01");
        assert_eq!(body.override_flags, 1);
    }

    /// The pawn chain merges the master mesh / anim class / capsule from the three Blueprint layers.
    #[test]
    fn pawn_chain_has_master_mesh_anim_class_and_capsule() {
        let Some(src) = install() else { return };
        let p = pawn_chain(&src, "/Game/Athena/PlayerPawn_Athena.PlayerPawn_Athena_C").unwrap();
        assert_eq!(p["chain"].as_array().unwrap().len(), 3);
        assert_eq!(p["collision_cylinder"]["CapsuleHalfHeight"], 75.0);
        assert_eq!(p["character_mesh"]["RelativeLocation"], json!([0.0, 0.0, -75.0]));
        assert!(p["character_mesh"]["AnimClass"].as_str().unwrap().ends_with("Fortnite_M_Avg_Player_AnimBlueprint_C"));
        assert!(p["character_mesh"]["SkeletalMesh"].as_str().unwrap().ends_with("Fortnite_M_Avg_Player"));
    }

    /// `M_Athena_InvisibleGrenades` (plain masked Material, no textures): its pass shaders discard unconditionally.
    #[test]
    fn invisible_grenades_discards() {
        let Some(src) = install() else { return };
        let (ps, disc) = discard_evidence(&src, "Game/Athena/Heroes/Meshes/Bodies/M_Athena_InvisibleGrenades").unwrap();
        assert!(ps > 0 && disc > 0, "{ps} {disc}");
        // a real character material must not look like that
        let (ps, disc) = discard_evidence(&src, "Game/Characters/Player/Male/Medium/Bodies/M_Med_Soldier_04/Materials/M_Med_Soldier_04").unwrap_or((0, 0));
        assert_eq!(disc, 0, "{ps}");
    }
}

pub fn run(src: &Source, a: &Args) -> Result<Value, String> {
    let mut ix = Index::default();
    walk(&a.out.join("exported"), &mut ix);
    let ch = resolve_character(src, &a.cid)?;
    let mut missing: Vec<String> = vec![];

    // the pawn
    let pawn_class = ch.pawn_class.clone().ok_or("HeroDefinition has no OverridePawnClass")?;
    let pawn = pawn_chain(src, &pawn_class)?;
    let master_mesh = pawn["character_mesh"]["SkeletalMesh"].as_str().map(String::from);
    let anim_skeleton = match &master_mesh {
        Some(mm) => {
            let mpkg = src.package(package_of(mm).trim_start_matches('/'))?;
            let msh = SkeletalMesh::parse(&mpkg).map_err(|e| e.to_string())?;
            let sp = msh.skeleton_path.clone().ok_or("master mesh has no Skeleton")?;
            Some((Skeleton::parse(&src.package(package_of(&sp).trim_start_matches('/'))?).map_err(|e| e.to_string())?, msh, sp))
        }
        None => None,
    };

    // parts
    let mut parts = vec![];
    for p in &ch.parts {
        match export_part(src, p, &a.out, &mut ix) {
            Ok(j) => parts.push(j),
            Err(e) => {
                missing.push(format!("{}: {e}", p.name));
                parts.push(json!({"role": p.role, "part": p.name, "skeletal_mesh": p.skeletal_mesh, "error": e}));
            }
        }
    }
    // skeleton compatibility (parts, declared master meshes, pawn master)
    let mut compat = vec![];
    if let Some((anim, master, skel_path)) = &anim_skeleton {
        for p in &ch.parts {
            compat.push(skeleton_compat(src, anim, &p.skeletal_mesh).unwrap_or_else(|e| json!({"mesh": p.skeletal_mesh, "error": e})));
            for mm in &p.master_meshes {
                compat.push(skeleton_compat(src, anim, mm).unwrap_or_else(|e| json!({"mesh": mm, "error": e})));
            }
        }
        compat.push(skeleton_compat(src, anim, master_mesh.as_deref().unwrap()).unwrap_or_else(|e| json!({"error": e})));
        let _ = (master, skel_path);
    }
    // pawn master mesh sockets (the pickaxe attaches to the weapon socket of this mesh/skeleton)
    let (master_json, anim_json) = match &anim_skeleton {
        Some((anim, master, skel_path)) => (
            json!({
                "mesh": master_mesh, "skeleton": skel_path, "bones": master.ref_skeleton.len(),
                "mesh_sockets": master.sockets.iter().map(|k| json!({"name": k.name, "bone": k.bone, "location": k.location, "rotation": k.rotation, "scale": k.scale})).collect::<Vec<_>>(),
                "skeleton_sockets": anim.sockets.iter().map(|k| json!({"name": k.name, "bone": k.bone, "location": k.location, "rotation": k.rotation, "scale": k.scale})).collect::<Vec<_>>(),
            }),
            json!({"skeleton": skel_path, "bones": anim.bones.len()}),
        ),
        None => (Value::Null, Value::Null),
    };
    // the pickaxe of the default loadout is not part of the character parts (DefaultPickaxe -> WID); recorded for the socket check
    let pickaxe = props_of(src, "/Game/Athena/Items/Cosmetics/Pickaxes/DefaultPickaxe", None)
        .and_then(|(_, p)| {
            let wid = s(&p, "WeaponDefinition").ok_or("no WeaponDefinition")?.to_string();
            let (_, w) = props_of(src, &wid, None)?;
            Ok(json!({"cosmetic": "DefaultPickaxe", "weapon_definition": wid, "weapon_mesh_override": w.get("WeaponMeshOverride"), "pickup_skeletal_mesh": w.get("PickupSkeletalMesh"), "weapon_actor_class": w.get("WeaponActorClass")}))
        })
        .unwrap_or_else(|e: String| json!({"error": e}));

    // all eight default CIDs, for the record (which one the exe uses is `source.cid`)
    let mut defaults = vec![];
    for n in DEFAULT_CIDS {
        let path = format!("/Game/Athena/Items/Cosmetics/Characters/{n}");
        defaults.push(match resolve_character(src, &path) {
            Ok(c) => json!({
                "cid": n, "hid": base(&c.hid), "specializations": c.specializations.iter().map(|s| base(s)).collect::<Vec<_>>(),
                "parts": c.parts.iter().map(|p| json!({"part": p.name, "role": p.role, "skeletal_mesh": p.skeletal_mesh, "master_meshes": p.master_meshes, "overrides": p.overrides.iter().map(|(i, m)| json!({"slot": i, "material": m})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                "pawn": c.pawn_class,
            }),
            Err(e) => json!({"cid": n, "error": e}),
        });
    }
    Ok(json!({
        "generated_by": "fn-export br-character",
        "source": {
            "cid": ch.cid, "display_name": ch.display_name, "hid": ch.hid, "specializations": ch.specializations, "pawn_class": ch.pawn_class,
            "basis": "AthenaCharacterItemDefinition.HeroDefinition -> FortHeroType.Specializations -> FortHeroSpecialization.CharacterParts -> CustomCharacterPart",
        },
        "parts": parts,
        "pawn": pawn,
        "master": master_json,
        "anim_skeleton": anim_json,
        "skeleton_compat": compat,
        "pickaxe": pickaxe,
        "default_cids": defaults,
        "missing": missing,
    }))
}
