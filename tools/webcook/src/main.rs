mod analyze;
mod cook;
mod sample;
mod shaders;
mod pngwebp;
mod v2;

use fn_pak::PakSet;

const DEFAULT_PAKS: &str = r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame\Content\Paks";
const DEFAULT_ASSETS: &str = r"C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\assets";
const USAGE: &str = "usage: webcook <analyze|sample|tex|pak|all|verify> [--paks D] [--assets D] [--webtex D] [--out D] [--manifest F]... [--tsv F] [--level N] [--threads N] [--no-closure] [--part-mb N] [--verify-every N]";

fn main() {
    let v: Vec<String> = std::env::args().skip(1).collect();
    let cmd = v.first().cloned().unwrap_or_default();
    let mut paks = DEFAULT_PAKS.to_owned();
    let mut tsv = None;
    let mut every = 1usize;
    let mut o = cook::Opts { out: "D:/webcook/v1".into(), webtex: "D:/webtex/out".into(), assets: DEFAULT_ASSETS.into(), manifests: vec![], level: 19, threads: std::thread::available_parallelism().map_or(8, |n| n.get()), closure: true, part_bytes: 256 << 20, reads: vec![], solid_bytes: 0 };
    let mut i = 1;
    while i < v.len() {
        let next = v.get(i + 1).cloned().unwrap_or_default();
        match v[i].as_str() {
            "--paks" => { paks = next; i += 1 }
            "--assets" => { o.assets = next.into(); i += 1 }
            "--webtex" => { o.webtex = next.into(); i += 1 }
            "--out" => { o.out = next.into(); i += 1 }
            "--manifest" => { o.manifests.push(next); i += 1 }
            "--tsv" => { tsv = Some(next); i += 1 }
            "--level" => { o.level = next.parse().unwrap_or(19); i += 1 }
            "--threads" => { o.threads = next.parse().unwrap_or(o.threads); i += 1 }
            "--part-mb" => { o.part_bytes = next.parse::<u64>().unwrap_or(256) << 20; i += 1 }
            "--verify-every" => { every = next.parse().unwrap_or(1); i += 1 }
            "--no-closure" => o.closure = false,
            "--reads" => { o.reads.push(next); i += 1 }
            "--solid-mb" => { o.solid_bytes = next.parse::<u64>().unwrap_or(0) << 20; i += 1 }
            _ => { eprintln!("{USAGE}"); std::process::exit(2) }
        }
        i += 1;
    }
    let set = PakSet::open_install(&paks).unwrap_or_else(|e| { eprintln!("{paks}: {e}"); std::process::exit(1) });
    let fail = |r: Result<(), String>| if let Err(e) = r { eprintln!("[webcook] {e}"); std::process::exit(1) };
    match cmd.as_str() {
        "analyze" => analyze::run(&set, &o.manifests, tsv.as_deref()),
        "sample" => sample::run(&set, tsv.as_deref().unwrap_or("D:/webcook/touched-webtex.tsv")),
        "tex" => { fail(cook::cook_tex(&o)); fail(cook::verify_tex(&o, 400)); }
        "pak" => { fail(cook::cook_pak(&set, &o)); fail(cook::verify_pak(&set, &o, every)); }
        "all" => { fail(cook::cook_tex(&o)); fail(cook::cook_pak(&set, &o)); fail(cook::verify_tex(&o, 400)); fail(cook::verify_pak(&set, &o, every)); }
        "files" => fail(cook::cook_files(&o)),
        "bcsplit" => sample::bcsplit("D:/webtex/out"),
        "readlog" => analyze::readlog(&set, tsv.as_deref().unwrap_or("D:/fn-verify/webopt/runs/c2rec2/reads.txt"), "D:/webtex/out/index.json"),
        "dictexp" => sample::dictexp(&set, tsv.as_deref().unwrap_or("D:/webcook/touched-low.tsv")),
        "dedupe" => { for c in ["MaterialInstanceConstant", "Material", "StaticMesh"] { sample::dedupe(&set, tsv.as_deref().unwrap_or("D:/webcook/touched-low.tsv"), c); } }
        "shaderstrip" => shaders::run(&set, tsv.as_deref().unwrap_or("D:/fn-verify/wc/boot-entries.tsv"), o.level),
        "texmiss" => analyze::texmiss(tsv.as_deref().unwrap_or("D:/fn-verify/wc/boot-entries.tsv"), "D:/fn-verify/wc/tex"),
        "pngwebp" => pngwebp::run(o.manifests.first().map(String::as_str).unwrap_or(""), &o.assets, o.level as f32, None),
        "v2files" => fail(v2::cook_files(&o.assets, &o.out, &o.manifests, o.level, 90.)),
        "v2pak" => fail(v2::cook_pak(&set, std::path::Path::new("D:/fn-verify/wc/pak"), &o.out, o.level, o.reads.first().map(String::as_str))),
        "pakverify" => { let _ = cook::verify_pak(&set, &o, every); }
        "menubytes" => { v2::menu_bytes(&set, std::path::Path::new("D:/webcook/v2/pak"), o.reads.first().map(String::as_str).unwrap_or("")); v2::menu_bytes(&set, std::path::Path::new("D:/fn-verify/wc/pak"), o.reads.first().map(String::as_str).unwrap_or("")); }
        "v2tex" => fail(v2::cook_tex(&set, &[tsv.clone().unwrap_or("D:/fn-verify/wc/boot-entries.tsv".into())], std::path::Path::new(&std::env::var("V1TEX").unwrap_or("D:/fn-verify/wc/tex".into())), &o.out, o.level)),
        "texhas" => { let ks: Vec<String> = o.manifests.clone(); let r: Vec<&str> = ks.iter().map(String::as_str).collect(); analyze::texhas(tsv.as_deref().unwrap_or("D:/fn-verify/wc/tex"), &r); }
        "posecheck" => v2::pose_check(&set, std::path::Path::new("D:/webcook/v2test/tex"), &o.manifests),
        "bootmap" => analyze::bootmap(&set, o.manifests.first().map(String::as_str).unwrap_or(""), "D:/fn-verify/wc/pak", tsv.as_deref()),
        "verify" => { fail(cook::verify_tex(&o, 4045)); fail(cook::verify_pak(&set, &o, every)); }
        _ => { eprintln!("{USAGE}"); std::process::exit(2) }
    }
}
