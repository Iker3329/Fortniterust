//! Lists the AES-encrypted entries of an install and, with `FN_PAK_AES_KEY` set, checks that each decrypts (prints the first bytes):
//! `encrypted <install dir>`.
fn main() {
    let dir = std::env::args().nth(1).expect("install dir");
    let set = fn_pak::PakSet::open_install(&dir).expect("paks");
    for p in set.paks() {
        for e in p.entries.iter().filter(|e| e.encrypted) {
            let head = match p.read(e) { Ok(b) => format!("{:02x?}", &b[..b.len().min(8)]), Err(err) => format!("ERR {err}") };
            println!("{}  {} B  {}", e.path, e.size, head);
        }
    }
}
