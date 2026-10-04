//! Inspect / decode one texture package.
//!
//! usage: `tex <path-without-extension> [out.png] [mip] [--no-bc5-z] [--nearest]`
//! Prints format, size, properties and the mip table; with `out.png` decodes the chosen mip (default: first
//! available). Cube maps write `<out>_face<N>.png` for the six faces.

use fn_texture::{DecodeOptions, Rounding, Texture2D};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let base = pos.first().expect("path without extension");
    let rd = |e: &str| std::fs::read(format!("{base}.{e}")).unwrap_or_default();
    let tex = match Texture2D::from_files(&rd("uasset"), &rd("uexp"), &rd("ubulk")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "{} class={} {}x{} slices={} format={} firstMip={} srgb={} compression={} addr={}/{} lodGroup={} neverStream={}",
        tex.name, tex.class, tex.size_x, tex.size_y, tex.num_slices, tex.format.name(), tex.first_mip, tex.srgb, tex.compression_settings,
        tex.address_x, tex.address_y, tex.lod_group, tex.never_stream
    );
    for (i, m) in tex.mips.iter().enumerate() {
        println!(
            "  mip {i}: {}x{} (stored {}x{}) flags={:#x} size={} ubulk={} {}",
            m.width,
            m.height,
            m.stored_width,
            m.stored_height,
            m.bulk_flags,
            m.size_on_disk,
            m.in_ubulk(),
            m.missing_reason().unwrap_or("available")
        );
    }
    let mut opts = DecodeOptions::default();
    if args.iter().any(|a| a == "--no-bc5-z") {
        opts.bc5_reconstruct_z = false;
    }
    if args.iter().any(|a| a == "--nearest") {
        opts.rounding = Rounding::Nearest;
    }
    let Some(out) = pos.get(1) else { return };
    let mip = pos.get(2).and_then(|s| s.parse().ok()).or_else(|| tex.best_mip()).expect("no mip data");
    let t0 = std::time::Instant::now();
    if tex.is_cube() {
        for s in 0..6 {
            let img = tex.decode_slice(mip, s, opts).unwrap();
            let p = format!("{}_face{s}.png", out.trim_end_matches(".png"));
            img.write_png(&p).unwrap();
            println!("wrote {p} {}x{}", img.width, img.height);
        }
    } else {
        let img = tex.decode(mip, opts).unwrap();
        let t_decode = t0.elapsed();
        img.write_png(out.as_str()).unwrap();
        println!("wrote {out} {}x{} (decode {:?}, +png {:?})", img.width, img.height, t_decode, t0.elapsed());
    }
}
