//! Diagnostic: prints the AnimNotifies (class, time) of every clip the animation graph can request, read straight from the 1.8 paks.
//! `cargo run -p fn-app --example audio_notifies -- <FortniteGame|Paks dir> [substring]`
use fn_anim::AnimSequence;
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = args.first().cloned().unwrap_or_else(|| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let filter = args.get(1).cloned().unwrap_or_default().to_lowercase();
    let set = fn_pak::PakSet::open_install(&root).expect("open install");
    let mut names = fn_animgraph::required_clips();
    names.sort();
    names.dedup();
    let mut total = 0;
    for name in names {
        if !filter.is_empty() && !name.to_lowercase().contains(&filter) { continue; }
        let pat = format!("fortnitegame/content/**/{}.uasset", name.to_lowercase());
        let hit = set.glob(&pat);
        let Some((_, e)) = hit.first() else { println!("{name}: NOT IN PAKS"); continue };
        let base = e.path.trim_end_matches(".uasset").to_owned();
        let rd = |ext: &str| set.read(&format!("{base}.{ext}")).map(|c| c.into_owned()).unwrap_or_default();
        let pkg = match fn_ueio::Package::parse(&rd("uasset"), &rd("uexp"), &rd("ubulk")) { Ok(p) => p, Err(e) => { println!("{name}: {e}"); continue } };
        let seq = match AnimSequence::parse(&pkg) { Ok(s) => s, Err(e) => { println!("{name}: {e}"); continue } };
        let mut ns: Vec<String> = seq.notifies.iter().map(|n| format!("{}@{:.4}", n.notify_class.clone().unwrap_or_else(|| n.notify_name.clone()), n.trigger_time())).collect();
        ns.sort();
        total += seq.notifies.len();
        println!("{name}: len {:.4}s loop-ish, {} notifies: {}", seq.length, seq.notifies.len(), ns.join(", "));
    }
    println!("total notifies {total}");
}
