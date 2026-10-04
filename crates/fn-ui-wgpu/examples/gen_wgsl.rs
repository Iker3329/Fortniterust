//! Generator: mechanically translates the straight-line Rust pixel shaders of fn-ui (material/uimat.rs ps_0..ps_29 and
//! material/ui_shaders.rs item_card_ps/compass_ps/hud_bar_ps/panel_line_ps/panel_vgrad_ps) to WGSL.
//!
//!   cargo run -p fn-ui-wgpu --example gen_wgsl [-- path-to-fn-ui/src/material]
//!
//! Writes src/shaders/gen/*.wgsl (one `fn ps_main(v: array<vec4<f32>,12>) -> vec4<f32>` per shader), src/shaders/gen/uimat_tex.bin
//! (the embedded Settings textures, expanded to RGBA8) and src/gen_tables.rs (instance / texture tables).
//! (Rust port of the former tools/gen_wgsl.cjs; verified byte-for-byte against its output.)
//!
//! The Rust subset handled is exactly what the old DXBC-to-Rust converter emitted: register arrays r/o, `let a=[..]` vec4 temporaries, scalar lets,
//! + - * / & comparisons, .abs/.floor/.sqrt/.exp2/.log2/.sin/.cos/.max/.min/.clamp/.to_bits, sat(), m(), nz(), f32::from_bits(), if/else
//! expressions, `{ let x = e; tail }` block expressions, closures `(|x:f32| x.sin())(a)` and tex(slot,u,v,lod) / tex[slot](u,v).
//! Anything else panics, so a new construct can never be silently mistranslated.
use regex::bytes::{Regex as BRegex, RegexBuilder};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const TOOL: &str = "crates/fn-ui-wgpu/examples/gen_wgsl.rs";

fn fail(msg: impl AsRef<str>) -> ! { panic!("{}", msg.as_ref()) }

// ---------------------------------------------------------------- tokenizer
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum T { Int, Float, Id, Op }
#[derive(Clone, Debug)]
struct Tok { t: T, v: String }

fn bre(p: &str) -> BRegex { RegexBuilder::new(p).unicode(false).build().unwrap() }

fn tokenize(src: &str) -> Vec<Tok> {
    let hex = bre(r"^0x[0-9A-Fa-f_]+(?:_?u32)?");
    let flt = bre(r"^(?:\d[\d_]*\.\d[\d_]*(?:[eE][-+]?\d+)?(?:_?f32)?|\d[\d_]*[eE][-+]?\d+(?:_?f32)?|\d[\d_]*_?f32)");
    let int = bre(r"^\d[\d_]*(?:_?u32|_?i32)?");
    let id = bre(r"^[A-Za-z_][A-Za-z_0-9]*");
    let b = src.as_bytes();
    let n = b.len();
    let mut toks = vec![];
    let mut i = 0;
    while i < n {
        let c = b[i];
        if matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c) { i += 1; continue; }
        if c == b'/' && b.get(i + 1) == Some(&b'/') { while i < n && b[i] != b'\n' { i += 1; } continue; }
        let rest = &b[i..(i + 80).min(n)];
        let mut hit = None;
        for (re, t) in [(&hex, T::Int), (&flt, T::Float), (&int, T::Int), (&id, T::Id)] {
            if let Some(m) = re.find(rest) { hit = Some((m.end(), t)); break; }
        }
        if let Some((len, t)) = hit { toks.push(Tok { t, v: String::from_utf8_lossy(&rest[..len]).into_owned() }); i += len; continue; }
        if i + 1 < n {
            let two = &src[i..i + 2];
            if ["==", "!=", ">=", "<=", "::", "&&", "||"].contains(&two) { toks.push(Tok { t: T::Op, v: two.to_owned() }); i += 2; continue; }
        }
        toks.push(Tok { t: T::Op, v: String::from_utf8_lossy(&b[i..i + 1]).into_owned() });
        i += 1;
    }
    toks
}

// ---------------------------------------------------------------- AST
#[derive(Clone, Debug)]
enum N {
    Float(String), Int(String), Paren(Box<N>), Neg(Box<N>), Not(Box<N>), Id(String), Path(String), Arr(Vec<N>),
    Bin(String, Box<N>, Box<N>), Index(Box<N>, Box<N>), If(Box<N>, Box<N>, Box<N>), Method(Box<N>, String, Vec<N>),
    Call(Box<N>, Vec<N>), Closure(String, Box<N>), BlockExpr(Vec<S>),
}
#[derive(Clone, Debug)]
enum S { Block(Vec<S>), Decl(String, usize), Let(String, N), Assign(N, N), Expr(N), Tail(N), Nop }

fn b(n: N) -> Box<N> { Box::new(n) }

struct Parser { t: Vec<Tok>, p: usize }
impl Parser {
    fn peek(&self, o: usize) -> Option<&Tok> { self.t.get(self.p + o) }
    fn next(&mut self) -> Tok { let k = self.t.get(self.p).cloned().unwrap_or_else(|| fail("unexpected end of tokens")); self.p += 1; k }
    fn is(&self, v: &str) -> bool { self.is_at(v, 0) }
    fn is_at(&self, v: &str, o: usize) -> bool { self.peek(o).map_or(false, |k| k.v == v && k.t != T::Float && k.t != T::Int) }
    fn ctx(&self) -> String { self.t[self.p.saturating_sub(12)..(self.p + 12).min(self.t.len())].iter().map(|x| x.v.as_str()).collect::<Vec<_>>().join(" ") }
    fn eat(&mut self, v: &str) -> Tok {
        if !self.is(v) { fail(format!("expected '{v}' got '{}' at token {}: {}", self.peek(0).map_or("", |t| &t.v), self.p, self.ctx())); }
        self.next()
    }
    fn stmt(&mut self) -> S {
        if self.is("{") { self.next(); return S::Block(self.block_body()); }
        if self.is("let") {
            self.next();
            let mut mutable = false;
            if self.is("mut") { self.next(); mutable = true; }
            let name = self.next().v;
            self.eat("=");
            if name == "_" { while !self.is(";") { self.next(); } self.next(); return S::Nop; }
            if mutable && self.is("[") && self.is_at("[", 1) {
                // [[0.0; 4]; N]
                self.next(); self.eat("["); self.next(); self.eat(";"); self.next(); self.eat("]"); self.eat(";");
                let cnt = parse_int10(&self.next().v).parse::<usize>().unwrap_or_else(|_| fail("bad array length")); self.eat("]"); self.eat(";");
                return S::Decl(name, cnt);
            }
            let init = self.expr();
            self.eat(";");
            return S::Let(name, init);
        }
        // lvalue = expr ;   or tail expression
        let e = self.expr();
        if self.is("=") { self.next(); let rhs = self.expr(); self.eat(";"); return S::Assign(e, rhs); }
        if self.is(";") { self.next(); return S::Expr(e); }
        S::Tail(e)
    }
    fn block_body(&mut self) -> Vec<S> { let mut out = vec![]; while !self.is("}") { out.push(self.stmt()); } self.eat("}"); out }
    fn expr(&mut self) -> N { self.cmp() }
    fn cmp(&mut self) -> N {
        let mut l = self.bor();
        while ["==", "!=", ">=", "<=", "<", ">"].iter().any(|o| self.is(o)) { let op = self.next().v; let r = self.bor(); l = N::Bin(op, b(l), b(r)); }
        l
    }
    fn bor(&mut self) -> N { let mut l = self.band(); while self.is("|") && !self.is_at("|", 1) { self.next(); let r = self.band(); l = N::Bin("|".into(), b(l), b(r)); } l }
    fn band(&mut self) -> N { let mut l = self.add(); while self.is("&") && !self.is_at("&", 1) { self.next(); let r = self.add(); l = N::Bin("&".into(), b(l), b(r)); } l }
    fn add(&mut self) -> N { let mut l = self.mul(); while self.is("+") || self.is("-") { let op = self.next().v; let r = self.mul(); l = N::Bin(op, b(l), b(r)); } l }
    fn mul(&mut self) -> N { let mut l = self.unary(); while self.is("*") || self.is("/") { let op = self.next().v; let r = self.unary(); l = N::Bin(op, b(l), b(r)); } l }
    fn unary(&mut self) -> N {
        if self.is("-") { self.next(); return N::Neg(b(self.unary())); }
        if self.is("!") { self.next(); return N::Not(b(self.unary())); }
        self.postfix()
    }
    fn postfix(&mut self) -> N {
        let mut e = self.primary();
        loop {
            if self.is(".") {
                self.next();
                let name = self.next().v;
                if self.is("(") { self.next(); let args = self.args(); e = N::Method(b(e), name, args); }
                else { fail(format!("field access unsupported: {name}")); }
            } else if self.is("[") { self.next(); let idx = self.expr(); self.eat("]"); e = N::Index(b(e), b(idx)); }
            else if self.is("(") && matches!(e, N::Id(_) | N::Path(_) | N::Index(..) | N::Closure(..)) { self.next(); let args = self.args(); e = N::Call(b(e), args); }
            else { break; }
        }
        e
    }
    fn args(&mut self) -> Vec<N> { let mut a = vec![]; while !self.is(")") { a.push(self.expr()); if self.is(",") { self.next(); } } self.eat(")"); a }
    fn primary(&mut self) -> N {
        let t = self.peek(0).cloned().unwrap_or_else(|| fail("unexpected end of tokens"));
        if t.t == T::Float { self.next(); return N::Float(t.v); }
        if t.t == T::Int { self.next(); return N::Int(t.v); }
        if self.is("(") {
            self.next();
            if self.is("|") { // closure
                self.next(); let param = self.next().v; if self.is(":") { self.next(); self.next(); } self.eat("|");
                let body = self.expr(); self.eat(")");
                return N::Closure(param, b(body));
            }
            let e = self.expr(); self.eat(")"); return N::Paren(b(e));
        }
        if self.is("[") { self.next(); let mut el = vec![]; while !self.is("]") { el.push(self.expr()); if self.is(",") { self.next(); } } self.eat("]"); return N::Arr(el); }
        if self.is("if") {
            self.next(); let cond = self.expr(); self.eat("{"); let a = self.expr(); self.eat("}"); self.eat("else"); self.eat("{"); let bb = self.expr(); self.eat("}");
            return N::If(b(cond), b(a), b(bb));
        }
        if self.is("{") { self.next(); return N::BlockExpr(self.block_body()); }
        if t.t == T::Id {
            self.next();
            if self.is("::") { self.next(); let n2 = self.next().v; return N::Path(format!("{}::{}", t.v, n2)); }
            return N::Id(t.v);
        }
        fail(format!("unexpected token {} ctx {}", t.v, self.ctx()))
    }
}

/// JavaScript `parseInt(s, 10)` rendered as a string ("NaN" when there are no leading digits).
fn parse_int10(s: &str) -> String {
    let d: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if d.is_empty() { return "NaN".into(); }
    let t = d.trim_start_matches('0');
    if t.is_empty() { "0".into() } else { t.to_owned() }
}
fn int_of(s: &str) -> i64 { parse_int10(&s.replace('_', "")).parse::<i64>().unwrap_or_else(|_| fail(format!("bad integer '{s}'"))) }

// ---------------------------------------------------------------- emitter
struct Emitter {
    cbmap: Vec<(i64, &'static str)>,
    lines: Vec<String>,
    tmp: usize,
    max_slot: i64,
    max_mat: usize,
    env: HashMap<String, String>,
    pre: Vec<String>,
}
impl Emitter {
    fn new(cbmap: Vec<(i64, &'static str)>) -> Emitter { Emitter { cbmap, lines: vec![], tmp: 0, max_slot: -1, max_mat: 0, env: HashMap::new(), pre: vec![] } }
    fn fl(v: &str) -> String {
        let mut s = v.strip_suffix("f32").unwrap_or(v).replace('_', "");
        if !s.contains(['.', 'e', 'E']) { s.push_str(".0"); }
        s.push('f');
        s
    }
    fn il(v: &str) -> String {
        let s = v.strip_suffix("u32").or_else(|| v.strip_suffix("i32")).unwrap_or(v).replace('_', "");
        format!("{s}u")
    }
    fn e(&mut self, n: &N) -> String {
        match n {
            N::Float(v) => Self::fl(v),
            N::Int(v) => Self::il(v),
            N::Paren(x) => format!("({})", self.e(x)),
            N::Neg(x) => format!("(-{})", self.e(x)),
            N::Not(x) => format!("(!{})", self.e(x)),
            N::Id(v) => self.env.get(v).cloned().unwrap_or_else(|| v.clone()),
            N::Arr(el) => { let parts: Vec<String> = el.iter().map(|x| self.e(x)).collect(); format!("vec4<f32>({})", parts.join(", ")) }
            N::Bin(op, l, r) => { let l = self.e(l); let r = self.e(r); format!("({l} {op} {r})") }
            N::Index(inner, idx) => {
                // cb[K][I][C]
                let mut flat: Vec<&N> = vec![idx];
                let mut x: &N = inner;
                while let N::Index(e2, i2) = x { flat.insert(0, i2); x = e2; }
                if matches!(x, N::Id(v) if v == "cb") { return self.cb(&flat); }
                let base = self.e(inner);
                format!("{base}[{}]", self.idx(idx))
            }
            N::If(cond, a, bb) => { let bs = self.e(bb); let a = self.e(a); let c = self.e(cond); format!("select({bs}, {a}, {c})") }
            N::Method(recv, name, args) => self.method(recv, name, args),
            N::Call(f, args) => self.call(f, args),
            N::BlockExpr(body) => {
                // { let x = e; ...; tail }  -> hoist the lets as pre statements
                let mut last: Option<&N> = None;
                let saved = self.env.clone();
                for s in body {
                    match s {
                        S::Let(name, init) => {
                            let nm = format!("h{}", self.tmp); self.tmp += 1;
                            let rhs = self.e(init);
                            self.pre.push(format!("let {nm} = {rhs};"));
                            self.env.insert(name.clone(), nm);
                        }
                        S::Tail(e) => last = Some(e),
                        _ => fail("unsupported statement in block expression"),
                    }
                }
                let r = self.e(last.unwrap_or_else(|| fail("block expression without a tail")));
                self.env = saved;
                r
            }
            N::Path(_) | N::Closure(..) => fail("cannot emit path/closure"),
        }
    }
    fn idx(&self, n: &N) -> String { match n { N::Int(v) => parse_int10(&v.replace('_', "")), _ => fail("non-constant index") } }
    fn cb(&mut self, flat: &[&N]) -> String {
        let idxs: Vec<String> = flat.iter().map(|f| self.idx(f)).collect();
        let (k, i, c) = (&idxs[0], &idxs[1], &idxs[2]);
        let ki: i64 = k.parse().unwrap_or(-1); let ii: i64 = i.parse().unwrap_or(-1);
        let kind = self.cbmap.iter().find(|(key, _)| *key == ki).map(|x| x.1);
        let base = match kind {
            Some("cb0") => { if ii > 2 { fail("cb0 index"); } 8 + ii }
            Some("view") => { if ii != 131 { fail(format!("view cb index {ii}")); } 11 }
            Some("mat") => { self.max_mat = self.max_mat.max((ii + 1) as usize); 16 + ii }
            _ => fail(format!("bad cb {k}")),
        };
        format!("P[{base}][{c}]")
    }
    fn method(&mut self, recv: &N, name: &str, args: &[N]) -> String {
        match name {
            "abs" | "floor" | "sqrt" | "exp2" | "log2" | "sin" | "cos" => format!("{name}({})", self.e(recv)),
            "max" | "min" => { let r = self.e(recv); let a = self.e(&args[0]); format!("{name}({r}, {a})") }
            "clamp" => { let r = self.e(recv); let a = self.e(&args[0]); let b2 = self.e(&args[1]); format!("clamp({r}, {a}, {b2})") }
            "to_bits" => format!("bitcast<u32>({})", self.e(recv)),
            _ => fail(format!("unsupported method {name}")),
        }
    }
    fn call(&mut self, f: &N, args: &[N]) -> String {
        match f {
            N::Id(name) => match name.as_str() {
                "sat" => format!("saturate({})", self.e(&args[0])),
                "m" => format!("bitcast<f32>(select(0u, 0xFFFFFFFFu, {}))", self.e(&args[0])),
                "nz" => format!("(bitcast<u32>({}) != 0u)", self.e(&args[0])),
                "tex" => {
                    let slot = match &args[0] { N::Int(v) | N::Float(v) => int_of(v), _ => fail("tex slot") };
                    self.max_slot = self.max_slot.max(slot);
                    let u = self.e(&args[1]); let v = self.e(&args[2]);
                    format!("tex_sample_{slot}({u}, {v})")
                }
                _ => fail(format!("unsupported call {name}")),
            },
            N::Path(p) if p == "f32::from_bits" => format!("bitcast<f32>({})", self.e(&args[0])),
            N::Index(e, idx) if matches!(&**e, N::Id(v) if v == "tex") => {
                let slot = match &**idx { N::Int(v) | N::Float(v) => int_of(v), _ => fail("tex slot") };
                self.max_slot = self.max_slot.max(slot);
                let u = self.e(&args[0]); let v = self.e(&args[1]);
                format!("tex_sample_{slot}({u}, {v})")
            }
            N::Closure(param, body) => {
                let saved = self.env.clone();
                let a = format!("({})", self.e(&args[0]));
                self.env.insert(param.clone(), a);
                let r = self.e(body);
                self.env = saved;
                r
            }
            _ => fail("unsupported call target"),
        }
    }
    fn stmt(&mut self, s: &S, ind: usize) {
        let pad = "    ".repeat(ind);
        match s {
            S::Nop => {}
            S::Decl(name, n) => self.lines.push(format!("{pad}var {name}: array<vec4<f32>, {n}>;")),
            S::Block(body) => {
                self.lines.push(format!("{pad}{{"));
                let saved = self.env.clone();
                for x in body { self.stmt(x, ind + 1); }
                self.env = saved;
                self.lines.push(format!("{pad}}}"));
            }
            S::Let(name, init) => {
                self.pre = vec![];
                let rhs = self.e(init);
                for p in std::mem::take(&mut self.pre) { self.lines.push(format!("{pad}{p}")); }
                self.lines.push(format!("{pad}let {name} = {rhs};"));
            }
            S::Assign(lhs, rhs) => {
                self.pre = vec![];
                let rhs = self.e(rhs);
                let lhs = self.e(lhs);
                for p in std::mem::take(&mut self.pre) { self.lines.push(format!("{pad}{p}")); }
                self.lines.push(format!("{pad}{lhs} = {rhs};"));
            }
            S::Tail(e) | S::Expr(e) => {
                self.pre = vec![];
                let mut e = e;
                if let N::Call(f, args) = e { if matches!(&**f, N::Id(v) if v == "Some") { e = &args[0]; } }
                let rhs = self.e(e);
                for p in std::mem::take(&mut self.pre) { self.lines.push(format!("{pad}{p}")); }
                if matches!(s, S::Tail(_)) { self.lines.push(format!("{pad}return {rhs};")); }
            }
        }
    }
}

struct Translated { text: String, max_slot: i64, max_mat: usize }

fn translate(name: &str, body: &str, cbmap: Vec<(i64, &'static str)>) -> Translated {
    let toks = tokenize(body);
    let mut p = Parser { t: toks, p: 0 };
    let mut stmts = vec![];
    while p.p < p.t.len() { stmts.push(p.stmt()); }
    let mut em = Emitter::new(cbmap);
    em.lines.push("fn ps_main(v: array<vec4<f32>, 12>) -> vec4<f32> {".into());
    for s in &stmts { em.stmt(s, 1); }
    em.lines.push("}".into());
    Translated { text: format!("// GENERATED by {TOOL} from fn-ui {name}. Do not edit.\n{}\n", em.lines.join("\n")), max_slot: em.max_slot, max_mat: em.max_mat }
}

// ---------------------------------------------------------------- source scraping
fn fn_body(src: &str, header: &str) -> String {
    let lines: Vec<&str> = src.split('\n').collect();
    let i = lines.iter().position(|l| l.starts_with(header)).unwrap_or_else(|| fail(format!("fn not found {header}")));
    // skip to the line that opens the body (ends with '{')
    let mut j = i;
    while !lines[j].trim_end().ends_with('{') { j += 1; }
    let mut out = vec![];
    for l in &lines[j + 1..] { if l.starts_with('}') { break; } out.push(*l); }
    out.join("\n")
}

struct Inst { name: String, ps: i64, ps_dis: i64, tex: Vec<i64>, view: bool }
struct TexMeta { w: usize, h: usize, srgb: bool, wx: bool, wy: bool, rgb: Option<[u8; 3]>, data: usize, off: usize }

fn num<T: std::str::FromStr>(s: &str) -> T { s.parse().unwrap_or_else(|_| fail(format!("bad number '{s}'"))) }
fn js_plus(s: &str) -> i64 { let t = s.trim(); if t.is_empty() { 0 } else { num(t) } }

fn main() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mat_dir = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| root.join("../fn-ui/src/material"));
    let out_dir = root.join("src/shaders/gen");
    std::fs::create_dir_all(&out_dir).unwrap();
    let write = |p: &Path, data: &[u8]| std::fs::write(p, data).unwrap_or_else(|e| fail(format!("{}: {e}", p.display())));
    let read = |n: &str| std::fs::read_to_string(mat_dir.join(n)).unwrap_or_else(|e| fail(format!("{}: {e}", mat_dir.join(n).display())));
    let uimat = read("uimat.rs");
    let uis = read("ui_shaders.rs");

    // INSTS table
    let inst_re = Regex::new(r#"Inst \{ name: "([^"]+)", ps: (\d+), ps_dis: (\d+|usize::MAX), cb: cb_(\d+), params: &PARAMS(\d+), tex: &\[([^\]]*)\], view: (true|false) \}"#).unwrap();
    let insts: Vec<Inst> = inst_re.captures_iter(&uimat).map(|m| Inst {
        name: m[1].to_owned(), ps: num(&m[2]), ps_dis: if &m[3] == "usize::MAX" { -1 } else { num(&m[3]) },
        tex: if m[6].trim().is_empty() { vec![] } else { m[6].split(',').map(js_plus).collect() }, view: &m[7] == "true",
    }).collect();
    if insts.len() != 40 { fail(format!("expected 40 insts, got {}", insts.len())); }
    let mut ps_view: HashMap<i64, bool> = HashMap::new();
    for it in &insts {
        for p in [it.ps, it.ps_dis] {
            if p < 0 { continue; }
            if let Some(v) = ps_view.get(&p) { if *v != it.view { fail(format!("inconsistent view flag for ps {p}")); } }
            ps_view.insert(p, it.view);
        }
    }

    // TEXS
    let mut tex_data: HashMap<usize, Vec<u8>> = HashMap::new();
    let data_re = Regex::new(r"static TEX(\d+): \[u8; (\d+)\] = \[([^\]]*)\]").unwrap();
    for m in data_re.captures_iter(&uimat) {
        tex_data.insert(num(&m[1]), m[3].split(',').filter(|s| !s.trim().is_empty()).map(|s| js_plus(s) as u8).collect());
    }
    let tm = Regex::new(r"static TEXS: \[Tex; (\d+)\] = \[(.*)\];").unwrap().captures(&uimat).unwrap_or_else(|| fail("TEXS table not found"));
    let meta_re = Regex::new(r"Tex \{ w: (\d+), h: (\d+), srgb: (true|false), wrap_x: (true|false), wrap_y: (true|false), rgb: (Some\(\[(\d+), (\d+), (\d+)\]\)|None), data: &TEX(\d+) \}").unwrap();
    let mut tex_meta: Vec<TexMeta> = meta_re.captures_iter(&tm[2]).map(|m| TexMeta {
        w: num(&m[1]), h: num(&m[2]), srgb: &m[3] == "true", wx: &m[4] == "true", wy: &m[5] == "true",
        rgb: if &m[6] == "None" { None } else { Some([num::<i64>(&m[7]) as u8, num::<i64>(&m[8]) as u8, num::<i64>(&m[9]) as u8]) }, data: num(&m[10]), off: 0,
    }).collect();
    if tex_meta.len() != num::<usize>(&tm[1]) { fail("tex count mismatch"); }
    let mut tex_bin: Vec<u8> = vec![];
    for t in tex_meta.iter_mut() {
        let d = tex_data.get(&t.data).unwrap_or_else(|| fail(format!("TEX{} data missing", t.data)));
        let mut px = vec![0u8; t.w * t.h * 4];
        for i in 0..t.w * t.h {
            match t.rgb {
                Some(rgb) => { px[i * 4..i * 4 + 3].copy_from_slice(&rgb); px[i * 4 + 3] = d[i]; }
                None => { let s = (i * 4).min(d.len()); let e = (i * 4 + 4).min(d.len()); px[i * 4..i * 4 + (e - s)].copy_from_slice(&d[s..e]); }
            }
        }
        t.off = tex_bin.len();
        tex_bin.extend_from_slice(&px);
    }
    write(&out_dir.join("uimat_tex.bin"), &tex_bin);

    // ---------------------------------------------------------------- translate
    struct PsInfo { n: usize, view: bool, max_slot: i64, max_mat: usize }
    let mut ps_info = vec![];
    for n in 0..30usize {
        let body = fn_body(&uimat, &format!("pub fn ps_{n}("));
        let view = *ps_view.get(&(n as i64)).unwrap_or_else(|| fail(format!("ps {n} unused")));
        let r = translate(&format!("uimat::ps_{n}"), &body, vec![(0, "cb0"), (1, if view { "view" } else { "mat" }), (2, "mat")]);
        write(&out_dir.join(format!("uimat_ps_{n}.wgsl")), r.text.as_bytes());
        ps_info.push(PsInfo { n, view, max_slot: r.max_slot, max_mat: r.max_mat });
    }
    let hud: [(&str, Vec<(i64, &'static str)>); 5] = [
        ("item_card", vec![(0, "cb0"), (1, "view"), (2, "mat")]),
        ("compass", vec![(0, "cb0"), (1, "mat")]),
        ("hud_bar", vec![(0, "cb0"), (1, "mat")]),
        ("panel_line", vec![(0, "cb0"), (1, "mat")]),
        ("panel_vgrad", vec![(0, "cb0"), (1, "mat")]),
    ];
    let mut hud_info: Vec<(&str, i64, usize)> = vec![];
    for (f, cbmap) in hud {
        let body = fn_body(&uis, &format!("pub fn {f}_ps("));
        let r = translate(&format!("ui_shaders::{f}_ps"), &body, cbmap);
        write(&out_dir.join(format!("hud_{f}.wgsl")), r.text.as_bytes());
        hud_info.push((f, r.max_slot, r.max_mat));
    }
    let max_mat = ps_info.iter().map(|p| p.max_mat).chain(hud_info.iter().map(|h| h.2)).max().unwrap_or(0);
    if 16 + max_mat > 256 { fail(format!("material cb too large: {max_mat}")); }

    // ---------------------------------------------------------------- tables
    let join = |v: Vec<String>, sep: &str| v.join(sep);
    let mut rs = format!("// GENERATED by {TOOL}. Do not edit.\n#![allow(dead_code)]\n\n");
    rs += &format!("/// Largest material constant buffer (vec4 count) over all translated shaders.\npub const MAX_MAT_CB: usize = {max_mat};\n\n");
    rs += &format!("pub static UIMAT_PS_WGSL: [&str; 30] = [\n{}\n];\n\n", join(ps_info.iter().map(|p| format!("    include_str!(\"shaders/gen/uimat_ps_{}.wgsl\"),", p.n)).collect(), "\n"));
    rs += &format!("pub static UIMAT_PS_TEXSLOTS: [u32; 30] = [{}];\n", join(ps_info.iter().map(|p| (p.max_slot + 1).to_string()).collect(), ", "));
    rs += &format!("pub static UIMAT_PS_VIEW: [bool; 30] = [{}];\n\n", join(ps_info.iter().map(|p| p.view.to_string()).collect(), ", "));
    rs += &format!("pub static HUD_WGSL: [&str; 5] = [\n{}\n];\n", join(hud_info.iter().map(|h| format!("    include_str!(\"shaders/gen/hud_{}.wgsl\"),", h.0)).collect(), "\n"));
    rs += &format!("pub static HUD_TEXSLOTS: [u32; 5] = [{}];\n\n", join(hud_info.iter().map(|h| (h.1 + 1).to_string()).collect(), ", "));
    rs += "/// Settings material instance: (cooked name, ps, ps for the DisabledEffect permutation (u32::MAX = none), texture slots -> UIMAT_TEX index).\n";
    rs += &format!("pub static UIMAT_INSTS: [(&str, u32, u32, &[u32]); {}] = [\n{}\n];\n\n", insts.len(), join(insts.iter().map(|i| {
        let dis = if i.ps_dis < 0 { "u32::MAX".to_owned() } else { i.ps_dis.to_string() };
        format!("    (\"{}\", {}, {}, &[{}]),", i.name, i.ps, dis, join(i.tex.iter().map(|x| x.to_string()).collect(), ", "))
    }).collect(), "\n"));
    rs += "/// Embedded Settings textures (straight RGBA8 in `uimat_tex.bin`): (w, h, srgb, wrap_x, wrap_y, byte offset).\n";
    rs += &format!("pub static UIMAT_TEX: [(u32, u32, bool, bool, bool, usize); {}] = [\n{}\n];\n", tex_meta.len(), join(tex_meta.iter().map(|t| format!("    ({}, {}, {}, {}, {}, {}),", t.w, t.h, t.srgb, t.wx, t.wy, t.off)).collect(), "\n"));
    rs += "pub static UIMAT_TEX_BIN: &[u8] = include_bytes!(\"shaders/gen/uimat_tex.bin\");\n";
    write(&root.join("src/gen_tables.rs"), rs.as_bytes());
    println!("generated {} shaders; max material cb = {max_mat} vec4; textures {}", ps_info.len() + hud_info.len(), tex_meta.len());
    println!("{}", ps_info.iter().map(|p| format!("ps_{}: view={} slots={} cb={}", p.n, p.view, p.max_slot + 1, p.max_mat)).collect::<Vec<_>>().join("\n"));
}
