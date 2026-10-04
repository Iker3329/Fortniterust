//! (3b) The CPU interpreter vs the old generated Rust: the 30 settings-UI pixel shaders (`old_uimat::ps_N`, extracted fresh from the 1.8
//! paks) and the 5 HUD pixel shaders (`old_ui_shaders::*_ps`, from the old reports' `.dxbc` files), 1000 random inputs each, tolerance 1e-5.
mod common;
use common::*;
use fn_dxbc::interp::{Env, Machine, Options, TexOp, TexReq};
use fn_dxbc::{Container, Shader};
use std::collections::HashMap;

#[path = "fixtures/old_uimat.rs"]
#[allow(dead_code, clippy::all)]
mod old_uimat;
#[path = "fixtures/old_ui_shaders.rs"]
#[allow(dead_code, clippy::all)]
mod old_ui;

type V4 = [f32; 4];

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; (self.0 >> 16) as u32 }
    fn unit(&mut self) -> f32 { (self.next() & 0xffffff) as f32 / 16777216.0 }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.unit() }
    /// constant-buffer-like value: mostly 0..1, sometimes exact 0 / 1 / small negative
    fn cbv(&mut self) -> f32 { match self.next() % 8 { 0 => 0.0, 1 => 1.0, 2 => self.range(-1.0, 0.0), 3 => self.range(0.0, 4.0), _ => self.range(0.0, 1.0) } }
}

fn texel(slot: usize, u: f32, v: f32) -> V4 {
    let s = slot as f32 + 1.0;
    [(u * 3.7 * s).sin() * 0.5 + 0.5, (v * 5.1 + s).cos() * 0.5 + 0.5, ((u + v) * 2.3).fract(), (u * v * s).fract()]
}
struct TexEnv;
impl Env for TexEnv {
    fn texture(&mut self, r: &TexReq) -> [f32; 4] {
        match r.op { TexOp::Sample | TexOp::SampleB | TexOp::SampleL => texel(r.texture as usize, r.coord[0], r.coord[1]), _ => [0.0; 4] }
    }
}

fn close(a: f32, b: f32) -> bool {
    if a.is_nan() && b.is_nan() { return true; }
    if a == b { return true; }
    (a - b).abs() <= 1e-5 * 1.0f32.max(a.abs()).max(b.abs())
}

struct Report { name: String, runs: usize, bad: usize, max_err: f32, first_bad: Option<String> }

/// Runs `old` and the interpreter on `n` random inputs (`nv` = number of input registers the old function takes); `patch` may edit the
/// decoded program first (used to reproduce what the old text translator did: dropping the Slate clip-quad prelude).
#[allow(clippy::too_many_arguments)]
fn compare_patched(name: &str, dxbc: &[u8], nv: usize, n: usize, seed: u64, opts: Options, fill: &dyn Fn(&mut Rng, &mut Vec<V4>), old: &dyn Fn(&[V4], &[Vec<V4>]) -> Option<V4>, patch: &dyn Fn(&mut Shader)) -> Report {
    let mut sh = Shader::parse(dxbc).unwrap();
    patch(&mut sh);
    let mut rng = Rng(seed);
    let mut rep = Report { name: name.into(), runs: 0, bad: 0, max_err: 0.0, first_bad: None };
    for _ in 0..n {
        let mut v = vec![[0.0f32; 4]; nv];
        fill(&mut rng, &mut v);
        let mut cbs: Vec<Vec<V4>> = vec![vec![]; 3];
        for c in &sh.decls.cbuffers {
            let mut d = vec![[0.0f32; 4]; c.size as usize];
            for e in d.iter_mut() { for k in 0..4 { e[k] = rng.cbv(); } }
            if (c.slot as usize) < cbs.len() { cbs[c.slot as usize] = d; }
        }
        // the old generated code addresses [cb0, cb1, cb2]; an undeclared slot is an empty slice
        let want = old(&v, &cbs);
        let mut m = Machine::new(&sh);
        m.opts = opts;
        for (i, x) in v.iter().enumerate() { m.set_input(i, *x); }
        for (s, d) in cbs.iter().enumerate() { if !d.is_empty() { m.set_cb(s, d); } }
        let out = m.run(&mut TexEnv).unwrap();
        rep.runs += 1;
        let got = out.o.first().copied().unwrap_or([0.0; 4]);
        let Some(want) = want else { continue };
        let mut worst = 0.0f32;
        let mut ok = true;
        for k in 0..4 {
            if !close(got[k], want[k]) { ok = false; }
            let e = (got[k] - want[k]).abs();
            if e.is_finite() { worst = worst.max(e); }
        }
        rep.max_err = rep.max_err.max(worst);
        if !ok {
            rep.bad += 1;
            if rep.first_bad.is_none() { rep.first_bad = Some(format!("v={v:?}\n got={got:?}\nwant={want:?}")); }
        }
    }
    rep
}


/// What the old translator did (`skipDiscard`): drop every instruction up to and including the Slate clip-quad `discard`.
fn strip_clip_prelude(sh: &mut Shader) {
    let first = sh.instructions.iter().position(|i| !i.op.is_decl() && i.op != fn_dxbc::Opcode::CustomData).unwrap();
    if let Some(d) = sh.instructions.iter().skip(first).position(|i| i.op == fn_dxbc::Opcode::Discard) { sh.instructions.drain(first..=first + d); }
}

fn to_cb(c: &[Vec<V4>], n: usize) -> Vec<&[V4]> { (0..n).map(|i| c[i].as_slice()).collect() }

// ------------------------------------------------------------------------------------------------ HUD shaders (old ui_shaders.rs)

#[test]
fn hud_shaders_match_old_ui_shaders_rs() {
    let root = fn_testdata::reports().join("hud-shaders");
    if !root.is_dir() { eprintln!("no old hud reports, skipping"); return; }
    let load = |dir: &str, id: u32| std::fs::read(root.join(dir).join(format!("shader_{id}.dxbc"))).unwrap();
    let tex_fns: [&dyn Fn(f32, f32) -> V4; 2] = [&|u, v| texel(0, u, v), &|u, v| texel(1, u, v)];
    let hud_fill = |r: &mut Rng, v: &mut Vec<V4>| {
        for e in v.iter_mut() { for k in 0..4 { e[k] = r.range(0.0, 1.0); } }
        if v.len() > 6 { v[6][2] = 1.0; v[6][3] = 1.0; }
    };
    // The old generator (a) parsed the *decimal* listing, so float immediates are rounded to six decimals (`imm_decimal6`),
    // (b) emitted `.clamp(0, 1)` for `_sat`, which keeps NaN (`saturate_nan_propagates`; D3D and our default give 0) and
    // (c) dropped the Slate clip-quad prelude, up to and including the discard.
    let opts = Options { imm_decimal6: true, saturate_nan_propagates: true, ..Default::default() };
    let mut reports = vec![];
    reports.push(compare_patched("item_card_ps", &load("M-Item-Card-Inst-Master", 12100), 9, 1000, 11, opts, &hud_fill, &|v, c| {
        let cb = to_cb(c, 3);
        let vv: [V4; 9] = v.try_into().unwrap();
        old_ui::item_card_ps(&vv, [cb[0], cb[1], cb[2]], tex_fns)
    }, &strip_clip_prelude));
    reports.push(compare_patched("compass_ps", &load("M_Compass", 2485), 7, 1000, 12, opts, &hud_fill, &|v, c| {
        let cb = to_cb(c, 2);
        let vv: [V4; 7] = v.try_into().unwrap();
        old_ui::compass_ps(&vv, [cb[0], cb[1]], tex_fns)
    }, &strip_clip_prelude));
    reports.push(compare_patched("hud_bar_ps", &load("HUD-Bar_Material", 2536), 7, 1000, 13, opts, &hud_fill, &|v, c| {
        let cb = to_cb(c, 2);
        let vv: [V4; 7] = v.try_into().unwrap();
        old_ui::hud_bar_ps(&vv, [cb[0], cb[1]], [])
    }, &strip_clip_prelude));
    reports.push(compare_patched("panel_line_ps", &load("M_Border_HUD_Line_Vert", 5118), 9, 1000, 14, opts, &hud_fill, &|v, c| {
        let cb = to_cb(c, 2);
        let vv: [V4; 9] = v.try_into().unwrap();
        old_ui::panel_line_ps(&vv, [cb[0], cb[1]], [tex_fns[0]])
    }, &strip_clip_prelude));
    reports.push(compare_patched("panel_vgrad_ps", &load("M_Border_HUD_VGradient", 5154), 9, 1000, 15, opts, &hud_fill, &|v, c| {
        let cb = to_cb(c, 2);
        let vv: [V4; 9] = v.try_into().unwrap();
        old_ui::panel_vgrad_ps(&vv, [cb[0], cb[1]], [])
    }, &strip_clip_prelude));
    let mut fail = 0;
    for r in &reports {
        eprintln!("{:<16} {} runs, {} outside 1e-5, max abs err {:.3e}", r.name, r.runs, r.bad, r.max_err);
        if let Some(f) = &r.first_bad { eprintln!("  first mismatch: {f}"); }
        fail += r.bad;
    }
    assert_eq!(fail, 0);
}

// ------------------------------------------------------------------------------------------------ settings-UI shaders (old uimat.rs)

type OldPs = fn(&[&[V4]; 3], &[V4; 12], &mut dyn FnMut(usize, f32, f32, f32) -> V4) -> V4;
macro_rules! ps_table { ($($n:ident),*) => { [$(old_uimat::$n as OldPs),*] }; }

/// instruction lines (without declarations, `ret` and the Slate clip-quad test up to and including the discard)
fn post_discard_lines(listing: &str) -> Vec<String> {
    let lines = normalize(listing);
    let mut body: Vec<String> = vec![];
    let mut started = false;
    for l in lines {
        if l.starts_with("//") { continue; }
        if !started {
            if l.starts_with("ps_") || l.starts_with("vs_") { started = true; }
            continue;
        }
        if l.starts_with("dcl_") { continue; }
        body.push(l);
    }
    if let Some(p) = body.iter().position(|l| l.starts_with("discard")) { body.drain(..=p); }
    body.retain(|l| l != "ret");
    body
}

fn old_comment_lines(src: &str, n: usize) -> Vec<String> {
    let head = format!("pub fn ps_{n}(");
    let start = src.find(&head).unwrap();
    let rest = &src[start..];
    let end = rest.find("\n}\n").map(|e| e + 3).unwrap_or(rest.len());
    rest[..end].lines().filter_map(|l| l.trim().strip_prefix("// ").map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))).collect()
}

#[test]
fn settings_ui_shaders_match_old_uimat_rs() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let root = fn_testdata::reports().join("settings-ui-shaders-1.8");
    if !root.is_dir() { eprintln!("no old settings-ui reports, skipping"); return; }
    let index = PathIndex::new(&set);
    let mut cache = PkgCache::new();
    let mut cands: Vec<(String, Vec<u8>)> = vec![]; // (listing in hex literal mode, bytes)
    let mut seen: HashMap<u64, ()> = HashMap::new();
    for d in std::fs::read_dir(&root).unwrap().flatten() {
        if !d.path().is_dir() { continue; }
        let name = d.file_name().to_string_lossy().to_string();
        let Some(g) = index.find(&name) else { continue };
        for b in cache.blobs(&set, &g) {
            let h = fn_dxbc_cli::extract::program_hash(&b.bytes);
            if seen.insert(h, ()).is_some() { continue; }
            let Ok(c) = Container::parse(&b.bytes) else { continue };
            let Ok(sh) = Shader::from_container(&c) else { continue };
            if sh.version_string() != "ps_5_0" { continue; }
            cands.push((fn_dxbc::disassemble_hex(&b.bytes).unwrap(), b.bytes.clone()));
        }
    }
    let src = include_str!("fixtures/old_uimat.rs");
    let table: [OldPs; 30] = ps_table!(ps_0, ps_1, ps_2, ps_3, ps_4, ps_5, ps_6, ps_7, ps_8, ps_9, ps_10, ps_11, ps_12, ps_13, ps_14, ps_15, ps_16, ps_17, ps_18, ps_19, ps_20, ps_21, ps_22, ps_23, ps_24, ps_25, ps_26, ps_27, ps_28, ps_29);
    let listings: Vec<(Vec<String>, &Vec<u8>)> = cands.iter().map(|(t, b)| (post_discard_lines(t), b)).collect();
    let mut total_bad = 0;
    let mut matched = 0;
    for n in 0..30 {
        let want = old_comment_lines(src, n);
        let Some((_, bytes)) = listings.iter().find(|(l, _)| *l == want) else {
            // show the closest candidate to ease debugging
            let best = listings.iter().map(|(l, _)| l.iter().zip(want.iter()).filter(|(a, b)| a == b).count()).max().unwrap_or(0);
            let cand = listings.iter().max_by_key(|(l, _)| l.iter().zip(want.iter()).filter(|(a, b)| a == b).count()).unwrap();
            let diffs: Vec<String> = cand.0.iter().zip(want.iter()).filter(|(a, b)| a != b).map(|(a, b)| format!("   ours: {a}\n   old : {b}")).collect();
            panic!("ps_{n}: no extracted shader has its instruction list ({} instructions; best candidate shares {best} lines, {} vs {} lines)\n{}", want.len(), cand.0.len(), want.len(), diffs.join("\n"));
        };
        matched += 1;
        let _ = std::fs::write(std::env::temp_dir().join(format!("old_ps_{n}.dxbc")), bytes);
        let f = table[n];
        let fill = |r: &mut Rng, v: &mut Vec<V4>| {
            for k in 0..4 { v[1][k] = r.range(0.0, 1.0); }
            v[6] = [r.range(0.0, 1.0), r.range(0.0, 1.0), 1.0, 1.0];
            v[7] = [0.0, 0.0, r.range(4.0, 400.0), r.range(4.0, 400.0)];
            v[8] = [r.range(0.0, 1.0), r.range(0.0, 1.0), 0.0, 0.0];
        };
        let opts = Options::default();
        let rep = compare_patched(&format!("ps_{n}"), bytes, 12, 1000, 100 + n as u64, opts, &fill, &|v, c| {
            let vv: [V4; 12] = v.try_into().unwrap();
            let cb = [c[0].as_slice(), c[1].as_slice(), c[2].as_slice()];
            let mut t = |slot: usize, u: f32, vv: f32, _lod: f32| texel(slot, u, vv);
            Some(f(&cb, &vv, &mut t))
        }, &strip_clip_prelude);
        eprintln!("ps_{n:<2} {} runs, {} outside 1e-5, max abs err {:.3e}", rep.runs, rep.bad, rep.max_err);
        if let Some(f) = &rep.first_bad { eprintln!("  first mismatch: {f}"); }
        total_bad += rep.bad;
    }
    eprintln!("{matched} of 30 settings-UI shaders identified in the fresh extraction, {total_bad} mismatching samples");
    assert_eq!(total_bad, 0);
}







