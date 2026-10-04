use fn_ueio::Package;

fn main() {
    let base = std::env::args().nth(1).expect("path without extension");
    let rd = |e: &str| std::fs::read(format!("{base}.{e}")).unwrap_or_default();
    let pkg = Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap();
    if pkg.find_export("Skeleton").is_some() {
        match fn_mesh::Skeleton::parse(&pkg) {
            Ok(s) => {
                println!("skeleton {} bones {} sockets {} curves {} retarget srcs {}", s.name, s.bones.len(), s.sockets.len(), s.curves.len(), s.retarget_sources.len());
                for (i, b) in s.bones.iter().enumerate().take(100) {
                    println!("  [{i}] {} parent {} {:?} pos {:?} rot {:?} scale {:?}", b.name, b.parent, b.retarget, b.ref_pos, b.ref_rot, b.ref_scale);
                }
            }
            Err(e) => println!("skeleton ERR {e}"),
        }
    }
    if pkg.find_export("StaticMesh").is_some() {
        match fn_mesh::StaticMesh::parse(&pkg) {
            Ok(m) => {
                println!("static {} bounds {:?} materials {:?} lods {}", m.name, m.bounds, m.materials.iter().map(|x| (&x.slot_name, &x.material_name)).collect::<Vec<_>>(), m.lods.len());
                for (i, l) in m.lods.iter().enumerate() {
                    println!("  lod {i}: verts {} idx {} (32bit {}) uvs {} full {} hp {} colors {} sections {:?}", l.positions.len(), l.indices.len(), l.indices_32bit, l.num_tex_coords, l.uv_full_precision, l.high_precision_tangents, l.colors.len(), l.sections);
                }
            }
            Err(e) => println!("static ERR {e}"),
        }
    }
    if pkg.find_export("SkeletalMesh").is_some() {
        match fn_mesh::SkeletalMesh::parse(&pkg) {
            Ok(m) => {
                println!("skeleton path {:?} sockets {}", m.skeleton_path, m.sockets.len());
                println!("skeletal {} bones {} lods {} materials {:?} bounds {:?}", m.name, m.ref_skeleton.len(), m.lods.len(), m.materials.iter().map(|x| (&x.slot_name, &x.material_name)).collect::<Vec<_>>(), m.bounds);
                for (i, l) in m.lods.iter().enumerate() {
                    println!("  lod {i}: verts {} idx {} (32bit {}) uvs {} full {} extra {} colors {} sections {:?}", l.num_vertices, l.indices.len(), l.indices_32bit, l.num_tex_coords, l.uv_full_precision, l.extra_bone_influences, l.colors.len(), l.sections.iter().map(|s| (s.material_index, s.base_index, s.num_triangles, s.base_vertex, s.num_vertices, s.bone_map.len())).collect::<Vec<_>>());
                }
            }
            Err(e) => println!("skeletal ERR {e}"),
        }
    }
}
