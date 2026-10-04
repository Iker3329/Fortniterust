//! ADS recovery probe (development aid). `ads_probe row <FortniteGame> <table pkg> <row>` dumps a stat-table row;
//! `ads_probe tex <FortniteGame> <object path> <out.png>` decodes a texture from the install.
use fn_pose::{loadout_stats::data_table_row, loader, ClipSourceDir};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let paks = fn_pak::PakSet::open_install(&a[2]).unwrap();
    let src = ClipSourceDir::Pak(&paks);
    match a[1].as_str() {
        "row" => { let r = data_table_row(&src, &a[3], &a[4]).unwrap(); for p in &r.0 { println!("{:?}", p); } }
        "tex" => {
            let (long, object) = a[3].split_once('.').unwrap();
            let pkg = loader::package(&src, long).unwrap();
            let idx = pkg.exports.iter().position(|e| e.object_name == object).unwrap();
            let t = fn_texture::Texture2D::parse_export(&pkg, idx).unwrap();
            let mip = t.best_mip().unwrap();
            let im = t.decode(mip, fn_texture::DecodeOptions::default()).unwrap();
            println!("{}x{}", im.width, im.height);
            let mut rgba = im.to_rgba8();
            let (mut amin, mut amax, mut rmin, mut rmax) = (255u8, 0u8, 255u8, 0u8);
            for p in rgba.chunks(4) { amin = amin.min(p[3]); amax = amax.max(p[3]); rmin = rmin.min(p[0]); rmax = rmax.max(p[0]); }
            println!("alpha {amin}..{amax} red {rmin}..{rmax}; corner px {:?} centre px {:?}", &rgba[0..4], &rgba[((im.height / 2) * im.width + im.width / 2 + 40) * 4..][..4]);
            im.write_png(&a[4]).unwrap();
            for p in rgba.chunks_mut(4) { let v = p[3]; p[0] = v; p[1] = v; p[2] = v; p[3] = 255; }
            let alpha = fn_texture::DecodedImage { width: im.width, height: im.height, pixels: fn_texture::Pixels::Rgba8(rgba) };
            alpha.write_png(format!("{}.alpha.png", a[4])).unwrap();
        }
        _ => panic!("row|tex"),
    }
}
