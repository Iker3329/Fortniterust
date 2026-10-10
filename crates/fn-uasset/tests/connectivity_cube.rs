//! `FConnectivityCube` native reader (`fn_uasset::native::read_connectivity_cube` and the `ConnectivityCube` struct layout).
//! Fixture `data/emp_connectivity_cube_hex.txt`: the raw 48-byte `ConnectivityCubeData` of all 57 1.8 `EMP_*` assets
//! (`<asset> <class> <hex>`), dumped during the structural verification (2026-10-05).

use fn_uasset::native::{read_connectivity_cube, CONNECTIVITY_CUBE_FACES};
use fn_uasset::Value;

fn fixture() -> Vec<(String, Vec<u8>)> {
    include_str!("data/emp_connectivity_cube_hex.txt").lines().filter(|l| !l.trim().is_empty()).map(|l| {
        let mut it = l.split_whitespace();
        let name = it.next().unwrap().to_owned();
        let _class = it.next().unwrap();
        let hex = it.next().unwrap();
        let bytes = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        (name, bytes)
    }).collect()
}

#[test]
fn reads_all_57_emp_cubes() {
    let f = fixture();
    assert_eq!(f.len(), 57);
    for (name, bytes) in &f {
        let c = read_connectivity_cube(bytes).unwrap_or_else(|| panic!("{name}"));
        for (n, w) in &c { assert_eq!((*n, w.len()), (25, 1), "{name}"); assert_eq!(w[0] >> 25, 0, "{name}: only 25 bits"); }
    }
    // EMP_Wall_Solid: the example of the spec (docs/spec/building_structural_gaps.md section 1)
    let solid = &f.iter().find(|(n, _)| n == "EMP_Wall_Solid").unwrap().1;
    let c = read_connectivity_cube(solid).unwrap();
    assert_eq!(c.map(|(_, w)| w[0]), [0x000f_c63f, 0x0008_4210, 0, 0x0000_8421, 0x01f0_0000, 0x01f0_0000]);
    // wrong sizes are refused
    assert!(read_connectivity_cube(&solid[..47]).is_none());
    let mut long = solid.clone(); long.push(0);
    assert!(read_connectivity_cube(&long).is_none());
}

/// The tagged reader now decodes `ConnectivityCubeData` of every EMP in the paks (no `Unparsed`) and equals the fixture.
/// FN_TEST_ROOT = the 1.8 `FortniteGame` directory.
#[test]
fn paks_decode_to_the_fixture() {
    let Ok(root) = std::env::var("FN_TEST_ROOT") else { return };
    let set = fn_pak::PakSet::open_install(std::path::Path::new(&root)).expect("paks");
    let want: std::collections::HashMap<String, Vec<u8>> = fixture().into_iter().collect();
    let mut paths: Vec<String> = set.effective().map(|(_, e)| e.rel_path().to_owned()).filter(|p| p.ends_with(".uasset") && p.rsplit('/').next().is_some_and(|n| n.starts_with("EMP_"))).collect();
    paths.sort();
    let mut seen = 0;
    for p in paths {
        let game = fn_pak::pak_path_to_game(&p).unwrap();
        let name = game.rsplit('/').next().unwrap().to_owned();
        let pd = fn_uasset::PackageData::load(&set, &game).expect("load");
        let pkg = pd.package().expect("parse");
        let mut found = false;
        for i in 0..pkg.exports.len() {
            let Ok(o) = pkg.read_export(i) else { continue };
            let Some(v) = o.get("ConnectivityCubeData") else { continue };
            let Value::Struct { name: sn, props } = v else { panic!("{name}: {v:?}") };
            assert_eq!(sn, "ConnectivityCube");
            let mut bytes = vec![];
            for (face, prop) in CONNECTIVITY_CUBE_FACES.iter().zip(props.iter()) {
                assert_eq!(&prop.name, face);
                let n = prop.value.field("NumBits").and_then(|v| v.as_i64()).unwrap();
                bytes.extend_from_slice(&(n as i32).to_le_bytes());
                for w in prop.value.field("Data").and_then(|v| v.as_array()).unwrap() { let Value::UInt(w) = w else { panic!() }; bytes.extend_from_slice(&(*w as u32).to_le_bytes()); }
            }
            assert_eq!(Some(&bytes), want.get(&name), "{name}");
            found = true;
        }
        assert!(found, "{name}: no ConnectivityCubeData");
        seen += 1;
    }
    assert_eq!(seen, 57);
}
