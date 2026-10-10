//! Writes every AES-encrypted entry (needs `FN_PAK_AES_KEY`) under `<out dir>` keeping its pak path: `extract_encrypted <install dir> <out dir>`.
//! The output is game data: write it OUTSIDE the repo.
fn main() {
    let mut a = std::env::args().skip(1);
    let (dir, out) = (a.next().expect("install dir"), std::path::PathBuf::from(a.next().expect("out dir")));
    let set = fn_pak::PakSet::open_install(&dir).expect("paks");
    let mut n = 0;
    for p in set.paks() {
        for e in p.entries.iter().filter(|e| e.encrypted) {
            let b = p.read(e).expect("decrypt (is FN_PAK_AES_KEY set?)");
            let path = out.join(e.path.trim_start_matches("../../../"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &b).unwrap(); n += 1;
        }
    }
    println!("{n} files -> {}", out.display());
}
