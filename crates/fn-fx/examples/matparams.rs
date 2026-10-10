//! Dev tool: a material's uniform expressions with the parameter names they read, the instance overrides, and the packed values.
//! `cargo run --release -p fn-fx --example matparams -- <material object path>`
fn main() {
    let path = std::env::args().nth(1).expect("material path");
    let install = std::env::var("FN_ROOT").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame"));
    let set = fn_pak::PakSet::open_install(&install).expect("paks");
    let m = fn_fx::material::Material::load(&set, &path).expect("material");
    println!("overrides vectors {:?}", m.overrides.vectors);
    println!("overrides scalars {:?}", m.overrides.scalars);
    for (i, e) in m.uniform.vectors.iter().enumerate() { println!("vec[{i}] {e:?}"); }
    for (i, e) in m.uniform.scalars.iter().enumerate() { println!("scl[{i}] {e:?}"); }
    let p = fn_fx::uniform::evaluate(&m.uniform, &m.overrides, 0.0);
    for (i, r) in p.cb.iter().enumerate() { println!("cb[{i}] {r:?}"); }
}
