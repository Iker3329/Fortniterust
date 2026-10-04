//! Research tool: `fxmesh <long package path>...` summarises static meshes (LOD 0 vertex count, bounds, UV sets, sections, colours).
use fn_fx::data::package_files;

fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    for pkg in std::env::args().skip(1) {
        let (a, x, b) = package_files(&set, &pkg)?;
        let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| e.to_string())?;
        let m = fn_mesh::StaticMesh::parse(&p).map_err(|e| format!("{pkg}: {e}"))?;
        println!("=== {} lods {} materials {:?}", m.name, m.lods.len(), m.materials.iter().map(|x| (x.slot_name.clone(), x.material.clone())).collect::<Vec<_>>());
        println!("bounds {:?}", m.bounds);
        for (li, l) in m.lods.iter().enumerate() {
            println!(" lod {li}: verts {} indices {} (32bit {}) uv sets {} full {} hp-tangents {} colors {} sections {:?}", l.positions.len(), l.indices.len(), l.indices_32bit, l.num_tex_coords, l.uv_full_precision, l.high_precision_tangents, l.colors.len(), l.sections.iter().map(|s| (s.material_index, s.first_index, s.num_triangles)).collect::<Vec<_>>());
            if li == 0 {
                let (mn, mx) = m.position_bounds(0).unwrap_or(([0.; 3], [0.; 3]));
                println!("   pos bounds {mn:?} {mx:?}");
                for i in 0..l.positions.len().min(6) { println!("   v{i} pos {:?} tx {:?} tz {:?} uv0 {:?} col {:?}", l.positions[i], l.tangent_x[i], l.tangent_z[i], l.uvs.first().and_then(|u| u.get(i)), l.colors.get(i)); }
            }
        }
    }
    Ok(())
}
