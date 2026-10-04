//! Verification of the skeletal mesh decoder + glTF exporter against the old UEViewer exports.
//!
//! Gated on the extracted test packages and the old exports existing (otherwise the tests print a note and pass).
//! Run with `cargo test -p fn-mesh --release -- --nocapture` to see the comparison table.

mod common;
use common::*;
use fn_mesh::gltf::{self, Options};
use fn_mesh::SkeletalMesh;
use serde_json::Value;
use std::collections::HashMap;

/// (directory below `Content` / `Game`, asset name)
const MESHES: &[(&str, &str)] = &[
    ("Characters/Player/Male/Medium/Bodies/M_Med_Soldier_04/Meshes", "SK_M_Med_Soldier_04"),
    ("Athena/Heroes/Meshes/Heads", "SK_M_MED_Head_01_ATH"),
    ("Weapons/FORT_Melee/Meshes", "SK_Pick"),
    ("Weapons/FORT_Melee/Meshes", "SK_Pickaxe_Basic"),
    ("Characters/Player/Male/Male_Avg_Base", "Fortnite_M_Avg_Player"),
];

fn normalise_text(s: &str) -> String {
    s.replace("\r\n", "\n")
        .lines()
        .filter(|l| !l.contains("\"generator\""))
        .collect::<Vec<_>>()
        .join("\n")
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap()).collect()).unwrap_or_default()
}

struct Row {
    name: String,
    section_verts: Vec<usize>,
    tris: usize,
    bbox_err: f64,
    bones: usize,
    bones_ok: bool,
    pos_err: f64,
    nrm_err: f64,
    tan_err: f64,
    uv_err: f64,
    uv_err_default: f64,
    joints_err: f64,
    weights_err: f64,
    ibm_err: f64,
    idx_err: f64,
    color_err: f64,
    decode_pos_err: f64,
    decode_uv_err: f64,
    bin_identical: bool,
    bin_identical_ex_tw: bool,
    tan_w_mismatch: usize,
    tan_w_total: usize,
    text_identical: bool,
}

fn compare(name: &str, mesh: &SkeletalMesh, old: &Gltf) -> Row {
    let (json, bin) = gltf::skeletal_mesh_lod_to_gltf_with(mesh, 0, name, &Options { legacy_uev_half_uv: true }).unwrap();
    let new = Gltf::from_parts(&json, bin.clone());
    let (json_d, bin_d) = gltf::skeletal_mesh_lod_to_gltf(mesh, 0, name).unwrap();
    let newd = Gltf::from_parts(&json_d, bin_d);

    let mut row = Row {
        name: name.to_string(),
        section_verts: vec![],
        tris: 0,
        bbox_err: 0.0,
        bones: 0,
        bones_ok: true,
        pos_err: 0.0,
        nrm_err: 0.0,
        tan_err: 0.0,
        uv_err: 0.0,
        uv_err_default: 0.0,
        joints_err: 0.0,
        weights_err: 0.0,
        ibm_err: 0.0,
        idx_err: 0.0,
        color_err: 0.0,
        decode_pos_err: 0.0,
        decode_uv_err: 0.0,
        bin_identical: false,
        bin_identical_ex_tw: false,
        tan_w_mismatch: 0,
        tan_w_total: 0,
        text_identical: false,
    };

    // --- skeleton: node names, hierarchy, transforms
    let on = old.json["nodes"].as_array().unwrap();
    let nn = new.json["nodes"].as_array().unwrap();
    assert_eq!(on.len(), nn.len(), "{name}: node count");
    row.bones = on.len() - 1;
    let mut old_parent: HashMap<String, String> = HashMap::new();
    for (i, n) in on.iter().enumerate() {
        if let Some(ch) = n["children"].as_array() {
            for c in ch {
                old_parent.insert(on[c.as_u64().unwrap() as usize]["name"].as_str().unwrap().to_string(), on[i]["name"].as_str().unwrap().to_string());
            }
        }
        assert_eq!(n["name"], nn[i]["name"], "{name}: node {i} name");
        assert_eq!(n["children"], nn[i]["children"], "{name}: node {i} children");
        assert_eq!(nums(&n["translation"]), nums(&nn[i]["translation"]), "{name}: node {i} translation");
        assert_eq!(nums(&n["rotation"]), nums(&nn[i]["rotation"]), "{name}: node {i} rotation");
    }
    // mesh bone names + parent relation straight from the decoded RefSkeleton
    assert_eq!(mesh.ref_skeleton.len(), row.bones, "{name}: bone count");
    let mesh_node = on[0]["name"].as_str().unwrap().to_string();
    for b in &mesh.ref_skeleton {
        // the old export hangs the root bone below the mesh node
        let want = if b.parent < 0 { mesh_node.clone() } else { mesh.ref_skeleton[b.parent as usize].name.clone() };
        let got = old_parent.get(&b.name).cloned();
        let ok = got.as_deref() == Some(want.as_str());
        if !ok {
            row.bones_ok = false;
            eprintln!("{name}: bone {} parent mismatch: decoded {:?} old {:?}", b.name, want, got);
        }
    }
    // skin
    let os = &old.json["skins"][0];
    assert_eq!(os["joints"], new.json["skins"][0]["joints"], "{name}: joints");
    let (_, a) = old.values(os["inverseBindMatrices"].as_u64().unwrap() as usize);
    let (_, b) = new.values(new.json["skins"][0]["inverseBindMatrices"].as_u64().unwrap() as usize);
    row.ibm_err = max_abs_err(&a, &b);

    // materials
    let om: Vec<&str> = old.json["materials"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    let nm: Vec<&str> = new.json["materials"].as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(om, nm, "{name}: material names");

    // --- primitives
    let op = old.json["meshes"][0]["primitives"].as_array().unwrap();
    let np = new.json["meshes"][0]["primitives"].as_array().unwrap();
    let npd = newd.json["meshes"][0]["primitives"].as_array().unwrap();
    assert_eq!(op.len(), np.len(), "{name}: primitive count");
    let lod = &mesh.lods[0];
    for (pi, (po, pn)) in op.iter().zip(np).enumerate() {
        assert_eq!(po["material"], pn["material"]);
        let count = old.accessor(po["attributes"]["POSITION"].as_u64().unwrap() as usize).1;
        row.section_verts.push(count);
        row.tris += old.accessor(po["indices"].as_u64().unwrap() as usize).1 / 3;
        let mut cmp = |attr: &str, err: &mut f64, other: Option<(&Gltf, &Value)>| {
            let ao = po["attributes"][attr].as_u64().unwrap() as usize;
            let (nl, np_) = other.unwrap_or((&new, pn));
            let an = np_["attributes"][attr].as_u64().unwrap() as usize;
            let (_, vo) = old.values(ao);
            let (_, vn) = nl.values(an);
            *err = err.max(max_abs_err(&vo, &vn));
            if attr == "POSITION" && other.is_none() {
                let mo: Vec<f64> = nums(&old.json["accessors"][ao]["min"]).into_iter().chain(nums(&old.json["accessors"][ao]["max"])).collect();
                let mn: Vec<f64> = nums(&new.json["accessors"][an]["min"]).into_iter().chain(nums(&new.json["accessors"][an]["max"])).collect();
                row.bbox_err = row.bbox_err.max(max_abs_err(&mo, &mn));
            }
        };
        let mut e = 0.0;
        cmp("POSITION", &mut e, None);
        row.pos_err = row.pos_err.max(e);
        let mut e = 0.0;
        cmp("NORMAL", &mut e, None);
        row.nrm_err = row.nrm_err.max(e);
        // tangent xyz must match exactly; the sign (w) is checked separately (see below)
        {
            let (_, vo) = old.values(po["attributes"]["TANGENT"].as_u64().unwrap() as usize);
            let (_, vn) = new.values(pn["attributes"]["TANGENT"].as_u64().unwrap() as usize);
            for (a, b) in vo.chunks(4).zip(vn.chunks(4)) {
                row.tan_err = row.tan_err.max(max_abs_err(&a[..3], &b[..3]));
                row.tan_w_total += 1;
                if a[3] != b[3] {
                    row.tan_w_mismatch += 1;
                }
            }
        }
        let mut e = 0.0;
        cmp("JOINTS_0", &mut e, None);
        row.joints_err = row.joints_err.max(e);
        let mut e = 0.0;
        cmp("WEIGHTS_0", &mut e, None);
        row.weights_err = row.weights_err.max(e);
        if po["attributes"].get("COLOR_0").is_some() {
            let mut e = 0.0;
            cmp("COLOR_0", &mut e, None);
            row.color_err = row.color_err.max(e);
        }
        let mut k = 0;
        while po["attributes"].get(format!("TEXCOORD_{k}")).is_some() {
            let a = format!("TEXCOORD_{k}");
            let mut e = 0.0;
            cmp(&a, &mut e, None);
            row.uv_err = row.uv_err.max(e);
            let mut e = 0.0;
            cmp(&a, &mut e, Some((&newd, &npd[pi])));
            row.uv_err_default = row.uv_err_default.max(e);
            k += 1;
        }
        let (_, vo) = old.values(po["indices"].as_u64().unwrap() as usize);
        let (_, vn) = new.values(pn["indices"].as_u64().unwrap() as usize);
        row.idx_err = row.idx_err.max(max_abs_err(&vo, &vn));

        // decoder level (independent of the exporter): local vertex order = first use in the section's indices
        let s = &lod.sections[pi];
        let mut seen: HashMap<u32, usize> = HashMap::new();
        let mut order = vec![];
        for i in 0..s.num_triangles as usize * 3 {
            let v = lod.indices[s.base_index as usize + i];
            seen.entry(v).or_insert_with(|| {
                order.push(v);
                order.len() - 1
            });
        }
        assert_eq!(order.len(), count, "{name}: section {pi} unique vertex count");
        let (_, opos) = old.values(po["attributes"]["POSITION"].as_u64().unwrap() as usize);
        let exp: Vec<f64> = order.iter().flat_map(|&v| {
            let p = lod.positions[v as usize];
            [(p[0] * 0.01f32) as f64, (p[2] * 0.01f32) as f64, (p[1] * 0.01f32) as f64]
        }).collect();
        row.decode_pos_err = row.decode_pos_err.max(max_abs_err(&opos, &exp));
        let (_, ouv) = old.values(po["attributes"]["TEXCOORD_0"].as_u64().unwrap() as usize);
        let exp: Vec<f64> = order.iter().flat_map(|&v| lod.uvs[0][v as usize].map(|x| x as f64)).collect();
        // old UVs carry UEViewer's half bug (<= 2^-15 + denormal offset); compare with a correct decode
        row.decode_uv_err = row.decode_uv_err.max(max_abs_err(&ouv, &exp));
    }

    // whole-file identity
    let oldbin = &old.bin;
    row.bin_identical = *oldbin == bin;
    {
        // identical apart from the TANGENT.w floats
        let mask = |g: &Gltf, bin: &[u8]| {
            let mut b = bin.to_vec();
            for p in g.json["meshes"][0]["primitives"].as_array().unwrap() {
                let (_, count, _, off) = g.accessor(p["attributes"]["TANGENT"].as_u64().unwrap() as usize);
                for k in 0..count {
                    b[off + k * 16 + 12..off + k * 16 + 16].fill(0);
                }
            }
            b
        };
        row.bin_identical_ex_tw = mask(old, &old.bin) == mask(&new, &bin);
    }
    // text: compare against old gltf text
    let oldtxt = std::fs::read_to_string(old_gltf_path(name)).unwrap();
    row.text_identical = normalise_text(&oldtxt) == normalise_text(&json);
    row
}

fn old_gltf_path(name: &str) -> std::path::PathBuf {
    let dir = MESHES.iter().find(|m| m.1 == name).unwrap().0;
    old_exports().unwrap().join(dir).join(format!("{name}.gltf"))
}

#[test]
fn skeletal_meshes_match_old_uevewer_exports() {
    let (Some(td), Some(old)) = (testdata(), old_exports()) else {
        eprintln!("NOTE: test data or old exports missing; skipping skeletal verification");
        return;
    };
    let mut rows = vec![];
    for (dir, name) in MESHES {
        let pkg = load_package(&td.join(dir).join(name));
        let mesh = SkeletalMesh::parse(&pkg).unwrap();
        let g = Gltf::load(&old.join(dir).join(name));
        rows.push(compare(name, &mesh, &g));
    }
    println!("\nskeletal mesh vs old UEViewer export (legacy-UEViewer-half mode; max abs error; 0 = bit exact)");
    println!(
        "{:<24} {:>5} {:>6} {:<22} {:>9} {:>6} {:>5} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7} {:>6} {:>6}",
        "mesh", "bones", "tris", "verts/section", "bbox", "bonesOK", "pos", "normal", "tangent", "uv", "uv(def)", "joints", "weights", "invbind", "color", "bin=", "gltf="
    );
    for r in &rows {
        println!(
            "{:<24} {:>5} {:>6} {:<22} {:>9.2e} {:>6} {:>5.0e} {:>8.1e} {:>8.1e} {:>8.1e} {:>8.1e} {:>8.1e} {:>8.1e} {:>8.1e} {:>7.0e} {:>6} {:>6}",
            r.name,
            r.bones,
            r.tris,
            format!("{:?}", r.section_verts),
            r.bbox_err,
            r.bones_ok,
            r.pos_err,
            r.nrm_err,
            r.tan_err,
            r.uv_err,
            r.uv_err_default,
            r.joints_err,
            r.weights_err,
            r.ibm_err,
            r.color_err,
            r.bin_identical,
            r.text_identical
        );
        println!(
            "{:<24} decoder-level position err {:.2e}, decoder UV err vs old (incl. old half bug) {:.2e}; .bin identical except tangent.w: {}; tangent.w sign differs on {}/{} vertices",
            "", r.decode_pos_err, r.decode_uv_err, r.bin_identical_ex_tw, r.tan_w_mismatch, r.tan_w_total
        );
    }
    for r in &rows {
        assert!(r.bones_ok, "{}: bone hierarchy", r.name);
        assert_eq!(r.pos_err, 0.0, "{}", r.name);
        assert_eq!(r.nrm_err, 0.0, "{}", r.name);
        assert_eq!(r.tan_err, 0.0, "{}", r.name);
        assert_eq!(r.uv_err, 0.0, "{}", r.name);
        assert!(r.uv_err_default < 3.2e-5, "{}: uv (correct half decode) differs by {}", r.name, r.uv_err_default);
        assert_eq!(r.joints_err, 0.0, "{}", r.name);
        assert_eq!(r.weights_err, 0.0, "{}", r.name);
        assert_eq!(r.ibm_err, 0.0, "{}", r.name);
        assert_eq!(r.idx_err, 0.0, "{}", r.name);
        assert_eq!(r.color_err, 0.0, "{}", r.name);
        assert!(r.decode_pos_err < 1e-9, "{}", r.name);
        assert!(r.decode_uv_err < 3.2e-5, "{}", r.name);
        assert!(r.bin_identical_ex_tw, "{}: .bin differs from the old export (beyond tangent.w)", r.name);
        assert!(r.text_identical, "{}: .gltf text differs from the old export", r.name);
    }
    // body mesh facts from the task
    let body = &rows[0];
    assert_eq!(body.section_verts[0], 7493);
}
