//! Dev tool: a static mesh's LOD-0 vertices: position (UE cm) and UV0, plus the extremes. `cargo run -p fn-mesh --example uvdump -- <path without extension>`
use fn_ueio::Package;
fn main() {
    let base = std::env::args().nth(1).expect("path without extension");
    let rd = |e: &str| std::fs::read(format!("{base}.{e}")).unwrap_or_default();
    let pkg = Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap();
    let m = fn_mesh::StaticMesh::parse(&pkg).unwrap();
    let l = &m.lods[0];
    println!("{} verts {} bounds {:?}", m.name, l.positions.len(), m.bounds);
    for (i, p) in l.positions.iter().enumerate().take(40) { println!("{i}: pos {:?} uv0 {:?}", p, l.uvs.first().and_then(|u| u.get(i))); }
}
