//! `fn-export props ...`: harvestable building-prop discovery and export (trees, rocks, ...).
//!
//! Sub-commands (all take `--paks <Paks dir>`):
//! * `survey --out <file.json>`   scan every Blueprint prop class, report resource type / health row / mesh
//! * `layout ...`                 extract Athena_Terrain placements (see `layout.rs`)
//! * `export ...`                 decode meshes, materials, textures, collision, data (see `export.rs`)
pub mod bp;
mod backdrop; // BACKDROP HOOK (terrain / ocean / skybox export)
mod backdrop_actors;
mod collision;
mod export;
mod foliage; // FOLIAGE HOOK (ambient ground cover export)
mod layout;
mod material;
mod poi; // POI: the static-mesh actors + foundation-streamed buildings around a point (`props poi`)
mod refs;
mod sheet;
mod splines; // ROADS / SPLINES (2026-10-08): the landscape splines' SplineMeshComponents (`props splines`)
mod survey;

use crate::Source;
use std::path::PathBuf;

pub struct PropsArgs {
    pub cmd: String,
    pub paks: PathBuf,
    pub out: PathBuf,
    pub file: Option<PathBuf>,
    pub jobs: usize,
    pub rest: Vec<String>,
}

pub fn parse(args: &[String]) -> Result<PropsArgs, String> {
    let mut a = PropsArgs {
        cmd: args.first().cloned().unwrap_or_default(),
        paks: PathBuf::from("C:/Users/Brady/Downloads/1.8/Fortnite/Fortnite/FortniteGame/Content/Paks"),
        out: PathBuf::from("assets/props"),
        file: None,
        jobs: std::thread::available_parallelism().map_or(4, |n| n.get()).min(8),
        rest: vec![],
    };
    let mut it = args.iter().skip(1);
    while let Some(x) = it.next() {
        match x.as_str() {
            "--paks" => a.paks = PathBuf::from(it.next().ok_or("--paks needs a value")?),
            "--out" => a.out = PathBuf::from(it.next().ok_or("--out needs a value")?),
            "--file" => a.file = Some(PathBuf::from(it.next().ok_or("--file needs a value")?)),
            "--jobs" => a.jobs = it.next().ok_or("--jobs needs a value")?.parse().map_err(|_| "bad --jobs")?,
            other => a.rest.push(other.to_string()),
        }
    }
    Ok(a)
}

pub fn main(args: &[String]) -> i32 {
    let a = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let src = match Source::open(&a.paks) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let r = match a.cmd.as_str() {
        "survey" => survey::run(&src, &a),
        "meshcheck" => survey::meshcheck(&src, &a),
        "refs" => refs::run(&src, &a),
        "layout" => layout::run(&src, &a),
        "foliage-inspect" => foliage::inspect(&src, &a), // FOLIAGE HOOK
        "foliage" => foliage::run(&src, &a),             // FOLIAGE HOOK
        "backdrop-terrain" => backdrop::terrain_cmd(&src, &a), // BACKDROP HOOK
        "backdrop-actors" => backdrop_actors::run(&src, &a), // BACKDROP HOOK
        "backdrop" => backdrop::terrain_cmd(&src, &a).and_then(|_| backdrop_actors::run(&src, &a)), // BACKDROP HOOK
        "export" => export::run(&src, &a),
        "poi" => poi::run(&src, &a),
        "splines" => splines::run(&src, &a), // ROADS / SPLINES
        "sheet" => sheet::run(&a.out).map(|f| println!("{}", f.join(", "))),
        c => Err(format!("unknown props command '{c}' (survey | layout | export)")),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}



