//! Verifies the disassembler against the listings d3dcompiler_47 produced for the old tooling (reports/**/shader_*.asm next to *.dxbc).
use fn_dxbc::*;
use std::path::{Path, PathBuf};

/// Tolerant normalisation: NUL removed, blanks trimmed/collapsed, empty lines dropped, wrapped lines (unbalanced parentheses) joined.
pub fn normalize(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut acc = String::new();
    for l in text.replace(['\0', '\u{feff}'], "").lines() {
        let t = l.trim();
        if t.is_empty() && acc.is_empty() { continue; }
        acc = if acc.is_empty() { t.to_string() } else { format!("{acc}{t}") };
        let bal = acc.matches('(').count() as i32 - acc.matches(')').count() as i32;
        let braces = acc.matches('{').count() as i32 - acc.matches('}').count() as i32;
        if (bal > 0 || braces > 0) && !acc.starts_with("//") { continue; }
        out.push(acc.split_whitespace().collect::<Vec<_>>().join(" "));
        acc.clear();
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() { collect(&p, out); } else if p.extension().is_some_and(|x| x == "dxbc") { out.push(p); }
    }
}

#[test]
fn disasm_matches_old_listings() {
    let root = fn_testdata::reports();
    let mut files = vec![];
    collect(&root, &mut files);
    if files.is_empty() { eprintln!("no old reports found, skipping"); return; }
    files.sort();
    let (mut ok, mut bad, mut strict) = (0, vec![], 0);
    for f in &files {
        let asm = f.with_extension("asm");
        let Ok(want) = std::fs::read_to_string(&asm) else { continue };
        let bytes = std::fs::read(f).unwrap();
        let got = match disassemble(&bytes) { Ok(g) => g, Err(e) => { bad.push(format!("{}: error {e}", f.display())); continue } };
        let (a, b) = (normalize(&want), normalize(&got));
        let st = |s: &str| s.replace(['\0', '\u{feff}'], "").lines().map(|l| l.trim_end_matches('\r').to_string()).filter(|l| !l.trim().is_empty()).collect::<Vec<_>>();
        if st(&want) == st(&got) { strict += 1; }
        if a == b { ok += 1; continue; }
        let i = a.iter().zip(b.iter()).position(|(x, y)| x != y).unwrap_or(a.len().min(b.len()));
        bad.push(format!("{}: line {i}\n   want: {:?}\n   got : {:?}", f.display(), a.get(i), b.get(i)));
    }
    eprintln!("disasm match: {ok} ok ({strict} byte-identical modulo empty lines), {} bad of {}", bad.len(), files.len());
    let mut groups: std::collections::BTreeMap<String, (usize, String)> = Default::default();
    for b in &bad { let key: String = b.lines().skip(1).collect::<Vec<_>>().join("|").chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect(); let e = groups.entry(key).or_insert((0, b.clone())); e.0 += 1; }
    for (n, b) in groups.values().take(30) { eprintln!("[{n}x] {b}"); }
    assert!(bad.is_empty(), "{} listings differ", bad.len());
}




