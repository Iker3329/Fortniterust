//! A tiny SM5 assembler for hand-written test shaders (token level, no text parser): enough to exercise flow control, relative
//! addressing and modifiers that the 1.8 corpus only touches rarely.
#![allow(dead_code)]

pub const TEMP: u32 = 0;
pub const OUTPUT: u32 = 2;
pub const INDEXABLE: u32 = 3;
pub const IMM32: u32 = 4;
pub const CB: u32 = 8;
pub const INPUT: u32 = 1;

pub const SW_XYZW: [u32; 4] = [0, 1, 2, 3];
pub fn swz(c: u32) -> [u32; 4] { [c; 4] }

fn tok(ncomp: u32, selmode: u32, sel: u32, ty: u32, dim: u32, reps: [u32; 3]) -> u32 {
    ncomp | (selmode << 2) | (sel << 4) | (ty << 12) | (dim << 20) | (reps[0] << 22) | (reps[1] << 25) | (reps[2] << 28)
}

/// destination `<file><idx>.<mask>`
pub fn dst(ty: u32, idx: u32, mask: u32) -> Vec<u32> { vec![tok(2, 0, mask, ty, 1, [0; 3]), idx] }
/// destination `x<reg>[<relative temp>.<comp> + <imm>].<mask>`
pub fn dst_x_rel(reg: u32, rel_temp: u32, rel_comp: u32, imm: u32, mask: u32) -> Vec<u32> {
    vec![tok(2, 0, mask, INDEXABLE, 2, [0, 3, 0]), reg, imm, tok(2, 2, rel_comp, TEMP, 1, [0; 3]), rel_temp]
}
fn swz_bits(s: [u32; 4]) -> u32 { s[0] | (s[1] << 2) | (s[2] << 4) | (s[3] << 6) }
/// source `<file><idx>.<swizzle>`
pub fn src(ty: u32, idx: u32, s: [u32; 4]) -> Vec<u32> { vec![tok(2, 1, swz_bits(s), ty, 1, [0; 3]), idx] }
/// source with a modifier (1 = neg, 2 = abs, 3 = -abs)
pub fn src_mod(ty: u32, idx: u32, s: [u32; 4], m: u32) -> Vec<u32> {
    let mut t = tok(2, 1, swz_bits(s), ty, 1, [0; 3]);
    t |= 1 << 31;
    vec![t, 1 | (m << 6), idx]
}
/// single-component source `<file><idx>.<c>`
pub fn src1(ty: u32, idx: u32, c: u32) -> Vec<u32> { vec![tok(2, 2, c, ty, 1, [0; 3]), idx] }
/// `cb<slot>[<elem>].<swizzle>`
pub fn cb(slot: u32, elem: u32, s: [u32; 4]) -> Vec<u32> { vec![tok(2, 1, swz_bits(s), CB, 2, [0; 3]), slot, elem] }
/// `cb<slot>[<relative temp>.<comp> + <imm>].<swizzle>`
pub fn cb_rel(slot: u32, rel_temp: u32, rel_comp: u32, imm: u32, s: [u32; 4]) -> Vec<u32> {
    vec![tok(2, 1, swz_bits(s), CB, 2, [0, 3, 0]), slot, imm, tok(2, 2, rel_comp, TEMP, 1, [0; 3]), rel_temp]
}
/// `x<reg>[<relative temp>.<comp> + <imm>].<swizzle>`
pub fn x_rel(reg: u32, rel_temp: u32, rel_comp: u32, imm: u32, s: [u32; 4]) -> Vec<u32> {
    vec![tok(2, 1, swz_bits(s), INDEXABLE, 2, [0, 3, 0]), reg, imm, tok(2, 2, rel_comp, TEMP, 1, [0; 3]), rel_temp]
}
/// destination `x<reg>[<elem>].<mask>` with an immediate element index
pub fn dst_x(reg: u32, elem: u32, mask: u32) -> Vec<u32> { vec![tok(2, 0, mask, INDEXABLE, 2, [0; 3]), reg, elem] }
/// `cb<slot>[<elem>].<swizzle>` with a modifier (1 = neg, 2 = abs, 3 = -abs)
pub fn cb_mod(slot: u32, elem: u32, s: [u32; 4], m: u32) -> Vec<u32> {
    let mut t = tok(2, 1, swz_bits(s), CB, 2, [0; 3]);
    t |= 1 << 31;
    vec![t, 1 | (m << 6), slot, elem]
}
pub fn imm4(v: [u32; 4]) -> Vec<u32> { vec![tok(2, 0, 0, IMM32, 0, [0; 3]), v[0], v[1], v[2], v[3]] }
pub fn imm1(v: u32) -> Vec<u32> { vec![tok(1, 0, 0, IMM32, 0, [0; 3]), v] }
pub fn f(x: f32) -> u32 { x.to_bits() }

pub struct Prog { pub w: Vec<u32> }
impl Prog {
    pub fn new(program_type: u32, major: u32, minor: u32) -> Prog { Prog { w: vec![(program_type << 16) | (major << 4) | minor, 0] } }
    /// one instruction: opcode, flags (bits 11..), operand word lists
    pub fn ins(&mut self, opcode: u32, flags: u32, ops: &[Vec<u32>]) -> &mut Self {
        let n: usize = ops.iter().map(|o| o.len()).sum();
        self.w.push(opcode | (flags << 11) | (((n + 1) as u32) << 24));
        for o in ops { self.w.extend_from_slice(o); }
        self
    }
    pub fn raw(&mut self, opcode: u32, flags: u32, words: &[u32]) -> &mut Self {
        self.w.push(opcode | (flags << 11) | (((words.len() + 1) as u32) << 24));
        self.w.extend_from_slice(words);
        self
    }
    pub fn temps(&mut self, n: u32) -> &mut Self { self.raw(104, 0, &[n]) }
    pub fn indexable_temp(&mut self, reg: u32, size: u32) -> &mut Self { self.raw(105, 0, &[reg, size, 4]) }
    pub fn cbuffer(&mut self, slot: u32, size: u32) -> &mut Self { self.raw(89, 0, &[tok(2, 0, 0, CB, 2, [0; 3]), slot, size]) }
    pub fn input_ps(&mut self, reg: u32, mask: u32) -> &mut Self { self.ins(98, 2, &[dst(INPUT, reg, mask)]) }
    pub fn output(&mut self, reg: u32, mask: u32) -> &mut Self { self.ins(101, 0, &[dst(OUTPUT, reg, mask)]) }
    pub fn global_flags(&mut self) -> &mut Self { self.raw(106, 1, &[]) }
    pub fn finish(&mut self) -> Vec<u8> {
        let words = self.w.len() as u32;
        self.w[1] = words;
        let prog: Vec<u8> = self.w.iter().flat_map(|w| w.to_le_bytes()).collect();
        // container with a single SHEX chunk
        let mut out = vec![];
        out.extend_from_slice(b"DXBC");
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&1u32.to_le_bytes());
        let total = 32 + 4 + 8 + prog.len() as u32;
        out.extend_from_slice(&total.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&36u32.to_le_bytes());
        out.extend_from_slice(b"SHEX");
        out.extend_from_slice(&(prog.len() as u32).to_le_bytes());
        out.extend_from_slice(&prog);
        fn_dxbc::resign(&out)
    }
}

// opcode numbers
pub const ADD: u32 = 0;
pub const BREAK: u32 = 2;
pub const BREAKC: u32 = 3;
pub const CASE: u32 = 6;
pub const DEFAULT: u32 = 10;
pub const ELSE: u32 = 18;
pub const ENDIF: u32 = 21;
pub const ENDLOOP: u32 = 22;
pub const ENDSWITCH: u32 = 23;
pub const FTOI: u32 = 27;
pub const IADD: u32 = 30;
pub const IF: u32 = 31;
pub const IGE: u32 = 33;
pub const ITOF: u32 = 43;
pub const LOOP: u32 = 48;
pub const LT: u32 = 49;
pub const MAD: u32 = 50;
pub const MOV: u32 = 54;
pub const MUL: u32 = 56;
pub const RET: u32 = 62;
pub const SWITCH: u32 = 76;
/// instruction flag: `_nz` test (bit 18 of the opcode token = flag bit 7)
pub const NZ: u32 = 1 << 7;
/// instruction flag: saturate (bit 13 = flag bit 2)
pub const SAT: u32 = 1 << 2;

/// The hand-written shaders: (name, dxbc bytes).
pub fn handmade() -> Vec<(String, Vec<u8>)> {
    let xyzw = 0xf;
    let mut v = vec![];

    // sum of 0..9 with a loop and breakc
    let mut p = Prog::new(0, 5, 0);
    p.global_flags().output(0, xyzw).temps(1);
    p.ins(MOV, 0, &[dst(TEMP, 0, 1), imm1(0)]);
    p.ins(MOV, 0, &[dst(TEMP, 0, 2), imm1(0)]);
    p.ins(LOOP, 0, &[]);
    p.ins(IGE, 0, &[dst(TEMP, 0, 4), src(TEMP, 0, swz(0)), imm1(10)]);
    p.ins(BREAKC, NZ, &[src1(TEMP, 0, 2)]);
    p.ins(IADD, 0, &[dst(TEMP, 0, 2), src(TEMP, 0, swz(1)), src(TEMP, 0, swz(0))]);
    p.ins(IADD, 0, &[dst(TEMP, 0, 1), src(TEMP, 0, swz(0)), imm1(1)]);
    p.ins(ENDLOOP, 0, &[]);
    p.ins(ITOF, 0, &[dst(OUTPUT, 0, 1), src(TEMP, 0, swz(1))]);
    p.ins(RET, 0, &[]);
    v.push(("h_loop_sum".to_string(), p.finish()));

    // switch on ftoi(cb0[0].x): 1 -> 10, 2 -> 20 (falls through the case label list 3 | 4 -> 30), default -> -1
    let mut p = Prog::new(0, 5, 0);
    p.global_flags().cbuffer(0, 1).output(0, xyzw).temps(1);
    p.ins(FTOI, 0, &[dst(TEMP, 0, 1), cb(0, 0, swz(0))]);
    p.ins(SWITCH, 0, &[src1(TEMP, 0, 0)]);
    p.ins(CASE, 0, &[imm1(1)]);
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 1), imm1(f(10.0))]);
    p.ins(BREAK, 0, &[]);
    p.ins(CASE, 0, &[imm1(2)]);
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 1), imm1(f(20.0))]);
    p.ins(BREAK, 0, &[]);
    p.ins(CASE, 0, &[imm1(3)]);
    p.ins(CASE, 0, &[imm1(4)]);
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 1), imm1(f(30.0))]);
    p.ins(BREAK, 0, &[]);
    p.ins(DEFAULT, 0, &[]);
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 1), imm1(f(-1.0))]);
    p.ins(BREAK, 0, &[]);
    p.ins(ENDSWITCH, 0, &[]);
    p.ins(RET, 0, &[]);
    v.push(("h_switch".to_string(), p.finish()));

    // if / else on cb0[0].x < 0.5, nested if inside the else branch
    let mut p = Prog::new(0, 5, 0);
    p.global_flags().cbuffer(0, 1).output(0, xyzw).temps(1);
    p.ins(LT, 0, &[dst(TEMP, 0, 1), cb(0, 0, swz(0)), imm1(f(0.5))]);
    p.ins(IF, NZ, &[src1(TEMP, 0, 0)]);
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 0xf), imm4([f(1.0), f(2.0), f(3.0), f(4.0)])]);
    p.ins(ELSE, 0, &[]);
    p.ins(LT, 0, &[dst(TEMP, 0, 2), cb(0, 0, swz(0)), imm1(f(0.75))]);
    p.ins(IF, NZ, &[src1(TEMP, 0, 1)]);
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 0xf), imm4([f(5.0), f(6.0), f(7.0), f(8.0)])]);
    p.ins(ELSE, 0, &[]);
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 0xf), imm4([f(9.0), f(10.0), f(11.0), f(12.0)])]);
    p.ins(ENDIF, 0, &[]);
    p.ins(ENDIF, 0, &[]);
    p.ins(RET, 0, &[]);
    v.push(("h_if_else".to_string(), p.finish()));

    // indexable temps and relative constant buffer addressing, saturate + modifiers
    let mut p = Prog::new(0, 5, 0);
    p.global_flags().cbuffer(0, 4).output(0, xyzw).temps(2).indexable_temp(0, 4);
    p.ins(FTOI, 0, &[dst(TEMP, 0, 1), cb(0, 0, swz(1))]); // r0.x = int(cb0[0].y)
    for k in 0..4u32 { p.ins(MOV, 0, &[dst_x(0, k, 0xf), imm4([f(k as f32), f(k as f32 * 2.0), f(k as f32 * 3.0), f(k as f32 * 4.0)])]); }
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 1), x_rel(0, 0, 0, 0, swz(2))]); // o0.x = x0[r0.x].z
    p.ins(MOV, 0, &[dst(OUTPUT, 0, 2), cb_rel(0, 0, 0, 1, swz(3))]); // o0.y = cb0[r0.x + 1].w
    p.ins(MUL, SAT, &[dst(OUTPUT, 0, 4), cb_mod(0, 0, swz(0), 3), imm1(f(2.0))]); // o0.z = sat(-|cb0[0].x| * 2)
    p.ins(ADD, 0, &[dst(OUTPUT, 0, 8), cb_mod(0, 0, swz(0), 1), imm1(f(0.25))]); // o0.w = -cb0[0].x + 0.25
    p.ins(RET, 0, &[]);
    v.push(("h_indexable".to_string(), p.finish()));
    v
}


