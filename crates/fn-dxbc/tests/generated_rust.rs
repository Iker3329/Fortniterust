//! (3c) the generated Rust (committed `tests/generated/corpus.rs`, compiled by cargo) against the interpreter: every vertex / pixel shader
//! of the old corpus, random inputs, outputs must agree (bit exact or within 1e-5).
mod common;
use common::*;
use fn_dxbc::interp::{Env, Machine, Options, TexOp, TexReq};
use fn_dxbc::Shader;

#[path = "generated/corpus.rs"]
#[allow(dead_code, clippy::all)]
mod gen;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 { self.0 ^= self.0 << 13; self.0 ^= self.0 >> 7; self.0 ^= self.0 << 17; (self.0 >> 16) as u32 }
    fn unit(&mut self) -> f32 { (self.next() & 0xffffff) as f32 / 16777216.0 }
    fn range(&mut self, a: f32, b: f32) -> f32 { a + (b - a) * self.unit() }
    fn val(&mut self) -> f32 { match self.next() % 8 { 0 => 0.0, 1 => 1.0, 2 => self.range(-1.0, 0.0), 3 => self.range(0.0, 4.0), _ => self.range(0.0, 1.0) } }
}

/// deterministic stand-in for every resource access, shared by both sides
fn fake(op: u32, texture: u32, c: [f32; 4], ic: [i32; 4], arg: f32) -> [f32; 4] {
    let s = texture as f32 + 1.0 + op as f32 * 0.37;
    let base = if op == 6 || op == 7 { [ic[0] as f32, ic[1] as f32, ic[2] as f32, ic[3] as f32] } else { c };
    [(base[0] * 3.7 * s + arg).sin() * 0.5 + 0.5, (base[1] * 5.1 + s).cos() * 0.5 + 0.5, ((base[0] + base[1] + base[2]) * 2.3).fract(), (base[0] * base[1] * s).fract() + arg * 0.01]
}

struct IEnv;
impl Env for IEnv {
    fn texture(&mut self, r: &TexReq) -> [f32; 4] {
        let op = match r.op { TexOp::Sample => 0, TexOp::SampleB => 1, TexOp::SampleL => 2, TexOp::SampleD => 3, TexOp::SampleC => 4, TexOp::SampleCLz => 5, TexOp::Ld => 6, TexOp::LdMs => 7, TexOp::Resinfo => 9, TexOp::Gather4 => 10, TexOp::Lod => 11, _ => 99 };
        fake(op, r.texture, r.coord, r.icoord, r.arg)
    }
    fn ddx(&mut self, v: [f32; 4]) -> [f32; 4] { [v[0] * 0.5 + 1.0, v[1] * 0.5 + 1.0, v[2] * 0.5 + 1.0, v[3] * 0.5 + 1.0] }
    fn ddy(&mut self, v: [f32; 4]) -> [f32; 4] { [v[0] * 0.25 - 1.0, v[1] * 0.25 - 1.0, v[2] * 0.25 - 1.0, v[3] * 0.25 - 1.0] }
}

fn gen_tex(f: &gen::Fetch) -> [f32; 4] {
    use gen::Op::*;
    match f.op {
        Ddx => { let v = f.coord; [v[0] * 0.5 + 1.0, v[1] * 0.5 + 1.0, v[2] * 0.5 + 1.0, v[3] * 0.5 + 1.0] }
        Ddy => { let v = f.coord; [v[0] * 0.25 - 1.0, v[1] * 0.25 - 1.0, v[2] * 0.25 - 1.0, v[3] * 0.25 - 1.0] }
        op => {
            let code = match op { Sample => 0, SampleB => 1, SampleL => 2, SampleD => 3, SampleC => 4, SampleCLz => 5, Ld => 6, LdMs => 7, Resinfo => 9, Gather4 => 10, Lod => 11, _ => 99 };
            fake(code, f.texture, f.coord, f.icoord, f.arg)
        }
    }
}

fn same(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-5 * 1.0f32.max(a.abs()).max(b.abs())
}

#[test]
fn generated_rust_matches_interpreter() {
    let c = corpus();
    let shaders: std::collections::BTreeMap<String, Shader> = parse_all(&c).into_iter().collect();
    let (mut checked, mut runs, mut skipped_loops) = (0, 0, 0);
    let mut bad = vec![];
    for (name, nin, nout, run) in gen::ENTRIES {
        let sh = &shaders[*name];
        let mut rng = Rng(0x1234_5678 ^ name.len() as u64 ^ (*nin as u64) << 8);
        for it in 0..50 {
            let v: Vec<[f32; 4]> = (0..*nin).map(|_| [rng.val(), rng.val(), rng.val(), rng.val()]).collect();
            let mut cbs: Vec<Vec<[f32; 4]>> = vec![];
            for cb in &sh.decls.cbuffers {
                if cbs.len() <= cb.slot as usize { cbs.resize(cb.slot as usize + 1, vec![]); }
                cbs[cb.slot as usize] = (0..cb.size).map(|_| [rng.val(), rng.val(), rng.val(), rng.val()]).collect();
            }
            // interpreter first: skip inputs that make it loop for a very long time
            let mut m = Machine::new(sh);
            m.opts = Options { max_steps: 200_000, ..Default::default() };
            for (i, x) in v.iter().enumerate() { m.set_input(i, *x); }
            for (s, d) in cbs.iter().enumerate() { if !d.is_empty() { m.set_cb(s, d); } }
            let want = match m.run(&mut IEnv) { Ok(o) => o, Err(fn_dxbc::interp::InterpError::Limit) => { skipped_loops += 1; continue } Err(e) => panic!("{name}: interpreter: {e}") };
            let cb_refs: Vec<&[[f32; 4]]> = cbs.iter().map(|d| d.as_slice()).collect();
            let got = run(&v, &cb_refs, &mut gen_tex);
            runs += 1;
            match got {
                None => if !want.discarded { bad.push(format!("{name} iteration {it}: generated discarded, interpreter did not")); break; },
                Some(o) => {
                    if want.discarded { bad.push(format!("{name} iteration {it}: interpreter discarded, generated did not")); break; }
                    let mut ok = o.len() == *nout;
                    for r in 0..want.o.len().min(*nout) { for k in 0..4 { if !same(o[r][k], want.o[r][k]) { ok = false; } } }
                    if let Some(d) = want.depth { if !same(o[want.o.len()][0], d) { ok = false; } }
                    if !ok { bad.push(format!("{name} iteration {it}:\n  got  {o:?}\n  want {:?} depth {:?}", want.o, want.depth)); break; }
                }
            }
        }
        checked += 1;
    }
    eprintln!("generated Rust vs interpreter: {checked} shaders, {runs} runs, {skipped_loops} runs skipped (interpreter step limit), {} shaders differ", bad.len());
    for b in bad.iter().take(10) { eprintln!("{b}"); }
    assert!(bad.is_empty());
    assert!(checked >= 4);
}

