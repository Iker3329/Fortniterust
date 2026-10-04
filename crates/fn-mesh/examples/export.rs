//! `export <package path without extension> <out dir> [name]`: decode a Skeletal/Static mesh package and write glTF.
use fn_ueio::Package;
use std::path::Path;

fn main() {
    let mut a = std::env::args().skip(1);
    let base = a.next().expect("package path without extension");
    let out = a.next().expect("output dir");
    let rd = |e: &str| std::fs::read(format!("{base}.{e}")).unwrap_or_default();
    let pkg = Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")).unwrap();
    if pkg.find_export("SkeletalMesh").is_some() {
        let m = fn_mesh::SkeletalMesh::parse(&pkg).unwrap();
        let name = a.next().unwrap_or(m.name.clone());
        fn_mesh::gltf::export_skeletal_mesh(&m, Path::new(&out), &name).unwrap();
    } else {
        let m = fn_mesh::StaticMesh::parse(&pkg).unwrap();
        let name = a.next().unwrap_or(m.name.clone());
        fn_mesh::gltf::export_static_mesh(&m, Path::new(&out), &name).unwrap();
    }
}
