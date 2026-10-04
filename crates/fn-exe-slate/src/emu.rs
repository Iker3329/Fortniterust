//! Emulator-lite: straight-line symbolic execution of one x64 function (jumps are NOT followed; instructions are processed in address order).
//!
//! Purpose: UE's Slate builder pattern compiles to inline stores into stack-allocated `FArguments` structs followed by calls
//! (`SNew` -> `SWidget::SWidgetConstruct(...)`, `Slot().Padding(...)`, ...). Tracking constants through registers / the stack frame / returned
//! heap objects lets every call be printed with its arguments and the contents of the objects it receives.
//!
//! Model
//! * Values: [`Val::Imm`] (known integer / float bits), [`Val::Addr`] (pointer into the frame, an object returned by a call or an incoming parameter),
//!   [`Val::Img`] (pointer into the PE image) and [`Val::Sym`] (opaque, printable description such as `*(this+0x558)`).
//! * Memory is byte-granular for known constants and slot-granular for pointers/symbols. Frame offsets are relative to the entry RSP.
//! * Because jumps are ignored, `if/else` diamonds execute BOTH arms in address order; later assignments win. The trace says so where it matters.
use crate::pe::Pe;
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind, Register};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Obj {
    Frame,
    /// object returned by the n-th call of the trace
    Ret(u32),
    /// incoming parameter n (1 = rcx/this, 2 = rdx, 3 = r8, 4 = r9)
    Param(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Unk,
    Imm(u64),
    Addr(Obj, i64),
    Img(u32),
    Sym(String),
}
impl Val {
    pub fn imm(&self) -> Option<u64> { if let Val::Imm(v) = self { Some(*v) } else { None } }
}

#[derive(Clone, Debug)]
pub struct CallEvent {
    pub at: u32,
    pub target: Option<u32>,
    pub indirect: Option<String>,
    pub ret_id: u32,
    /// (label, rendered value) of the arguments the callee's signature says it reads
    pub args: Vec<(String, String)>,
    /// rendered dump of objects handed to the call (label -> field list)
    pub dumps: Vec<(String, String)>,
    /// raw rcx, rdx, r8, r9
    pub regs: [Val; 4],
    /// xmm0..xmm3 low lane as f32 (when a known constant)
    pub xmm: [Option<f32>; 4],
    /// outgoing stack arguments `[rsp+0x20+8k]` as raw 64-bit (low 32 bits valid for dword / float args) when known
    pub stack: Vec<Option<u64>>,
}

#[derive(Clone, Debug)]
pub enum Event {
    Call(CallEvent),
    /// a store that wrote a rdata constant / known value into a tracked object (only for `watch` objects)
    Note(u32, String),
}

#[derive(Default, Clone)]
struct Mem {
    bytes: BTreeMap<(Obj, i64), u8>,
    ptrs: BTreeMap<(Obj, i64), Val>,
}
impl Mem {
    fn kill(&mut self, o: Obj, off: i64, n: i64) {
        for i in 0..n { self.bytes.remove(&(o, off + i)); }
        let lo = off - 7;
        let keys: Vec<_> = self.ptrs.range((o, lo)..(o, off + n)).map(|(k, _)| *k).collect();
        for k in keys { self.ptrs.remove(&k); }
    }
    fn store_imm(&mut self, o: Obj, off: i64, size: usize, v: u64) {
        self.kill(o, off, size as i64);
        for i in 0..size { self.bytes.insert((o, off + i as i64), (v >> (8 * i)) as u8); }
    }
    fn store_val(&mut self, o: Obj, off: i64, size: usize, v: &Val) {
        match v {
            Val::Imm(x) => self.store_imm(o, off, size, *x),
            Val::Unk => self.kill(o, off, size as i64),
            other => { self.kill(o, off, size as i64); self.ptrs.insert((o, off), other.clone()); }
        }
    }
    fn load(&self, o: Obj, off: i64, size: usize) -> Val {
        if size == 8 { if let Some(p) = self.ptrs.get(&(o, off)) { return p.clone(); } }
        let mut v = 0u64;
        for i in 0..size {
            match self.bytes.get(&(o, off + i as i64)) { Some(b) => v |= (*b as u64) << (8 * i), None => return Val::Unk }
        }
        Val::Imm(v)
    }
}

pub struct Emu<'a> {
    pe: &'a Pe,
    regs: HashMap<Register, Val>,
    xmm: HashMap<Register, [Val; 4]>,
    mem: HashMap<Obj, Mem>,
    pub events: Vec<Event>,
    ret_counter: u32,
    rsp: i64,
    rbp_off: Option<i64>,
    /// names for callee RVAs (annotation only)
    pub names: HashMap<u32, String>,
    /// calls to these targets (and all indirect calls when `skip_indirect`) are omitted from [`Emu::render`]
    pub skip: std::collections::HashSet<u32>,
    pub skip_indirect: bool,
    /// callee RVA -> (number of xmm args read: 0..3, number of stack args read: 0..6); default (0, 0) = only rcx/rdx/r8/r9 are shown
    pub sigs: HashMap<u32, (u8, u8)>,
    /// callee RVAs that are fluent setters returning their `this` (rcx): rax keeps pointing at the same object
    pub ret_this: std::collections::HashSet<u32>,
    /// callee RVA -> (first, last) byte offset of pointer args to dump (default 0x0..0x40)
    pub dump_range: HashMap<u32, (i64, i64)>,
}

fn full(r: Register) -> Register { if r.is_xmm() { r.full_register() } else { r.full_register() } }

impl<'a> Emu<'a> {
    pub fn new(pe: &'a Pe) -> Emu<'a> {
        let mut e = Emu { pe, regs: HashMap::new(), xmm: HashMap::new(), mem: HashMap::new(), events: vec![], ret_counter: 0, rsp: 0, rbp_off: None, names: HashMap::new(), skip: Default::default(), skip_indirect: true, sigs: HashMap::new(), ret_this: Default::default(), dump_range: HashMap::new() };
        e.regs.insert(Register::RSP, Val::Addr(Obj::Frame, 0));
        e.regs.insert(Register::RCX, Val::Addr(Obj::Param(1), 0));
        e.regs.insert(Register::RDX, Val::Addr(Obj::Param(2), 0));
        e.regs.insert(Register::R8, Val::Addr(Obj::Param(3), 0));
        e.regs.insert(Register::R9, Val::Addr(Obj::Param(4), 0));
        e
    }

    fn reg(&self, r: Register) -> Val {
        if r == Register::None { return Val::Imm(0); }
        let f = r.full_register();
        let v = self.regs.get(&f).cloned().unwrap_or(Val::Unk);
        match (r.size(), &v) {
            (8, _) => v,
            (4, Val::Imm(x)) => Val::Imm(*x & 0xffff_ffff),
            (2, Val::Imm(x)) => Val::Imm(*x & 0xffff),
            (1, Val::Imm(x)) => {
                if r >= Register::AH && r <= Register::BH { Val::Imm((*x >> 8) & 0xff) } else { Val::Imm(*x & 0xff) }
            }
            _ => Val::Unk,
        }
    }
    fn set_reg(&mut self, r: Register, v: Val) {
        let f = r.full_register();
        let nv = match r.size() {
            8 => v,
            4 => match v { Val::Imm(x) => Val::Imm(x & 0xffff_ffff), _ => Val::Unk },
            _ => match (self.regs.get(&f), v) {
                (Some(Val::Imm(old)), Val::Imm(x)) => { let m = if r.size() == 1 { 0xff } else { 0xffff }; Val::Imm((old & !m) | (x & m)) }
                _ => Val::Unk,
            },
        };
        if f == Register::RSP { if let Val::Addr(Obj::Frame, o) = &nv { self.rsp = *o; } }
        if f == Register::RBP { if let Val::Addr(Obj::Frame, o) = &nv { self.rbp_off = Some(*o); } }
        self.regs.insert(f, nv);
    }
    fn get_xmm(&self, r: Register) -> [Val; 4] { self.xmm.get(&r.full_register()).cloned().unwrap_or([Val::Unk, Val::Unk, Val::Unk, Val::Unk]) }
    fn set_xmm(&mut self, r: Register, v: [Val; 4]) { self.xmm.insert(r.full_register(), v); }

    /// Effective address of a memory operand as a value.
    fn ea(&self, ins: &Instruction) -> Val {
        if ins.is_ip_rel_memory_operand() {
            let t = ins.ip_rel_memory_address();
            return Val::Img((t - self.pe.image_base) as u32);
        }
        let base = self.reg(ins.memory_base());
        let disp = ins.memory_displacement64() as i64;
        let idx = ins.memory_index();
        let index_part = if idx == Register::None { Some(0i64) } else {
            match self.reg(idx) { Val::Imm(v) => Some((v as i64).wrapping_mul(ins.memory_index_scale() as i64)), _ => None }
        };
        let Some(ip) = index_part else { return Val::Unk };
        match base {
            Val::Imm(b) => Val::Imm((b as i64).wrapping_add(disp).wrapping_add(ip) as u64),
            Val::Addr(o, off) => Val::Addr(o, off + disp + ip),
            Val::Img(r) => Val::Img((r as i64 + disp + ip) as u32),
            Val::Sym(s) => if ip == 0 { Val::Sym(format!("{s}{}{:#x}", if disp < 0 { "-" } else { "+" }, disp.abs())) } else { Val::Unk },
            Val::Unk => Val::Unk,
        }
    }
    fn mem_size(ins: &Instruction) -> usize { ins.memory_size().size().max(1) }

    fn load_mem(&self, ins: &Instruction, size: usize) -> Val {
        match self.ea(ins) {
            Val::Addr(o, off) => {
                let v = self.mem.get(&o).map(|m| m.load(o, off, size)).unwrap_or(Val::Unk);
                if v == Val::Unk {
                    let base = match o { Obj::Frame => "frame".to_string(), Obj::Ret(n) => format!("ret#{n}"), Obj::Param(1) => "this".into(), Obj::Param(n) => format!("arg{n}") };
                    if o == Obj::Frame { Val::Unk } else { Val::Sym(format!("*({base}{}{:#x})", if off < 0 { "-" } else { "+" }, off.abs())) }
                } else { v }
            }
            Val::Img(rva) => {
                let sec = self.pe.section_by_rva(rva);
                let writable = sec.map(|s| s.characteristics & 0x8000_0000 != 0).unwrap_or(true);
                if writable { return Val::Sym(format!("[g:{:#x}]", rva)); }
                match size { 1 => self.pe.bytes_at_rva(rva, 1).map(|b| Val::Imm(b[0] as u64)).unwrap_or(Val::Unk),
                    2 => self.pe.bytes_at_rva(rva, 2).filter(|b| b.len() == 2).map(|b| Val::Imm(u16::from_le_bytes([b[0], b[1]]) as u64)).unwrap_or(Val::Unk),
                    4 => self.pe.read_u32(rva).map(|v| Val::Imm(v as u64)).unwrap_or(Val::Unk),
                    8 => self.pe.read_u64(rva).map(Val::Imm).unwrap_or(Val::Unk), _ => Val::Unk }
            }
            Val::Sym(s) => Val::Sym(format!("*({s})")),
            _ => Val::Unk,
        }
    }
    fn store_mem(&mut self, ins: &Instruction, size: usize, v: &Val) {
        if let Val::Addr(o, off) = self.ea(ins) { self.mem.entry(o).or_default().store_val(o, off, size, v); }
    }
    fn kill_mem(&mut self, ins: &Instruction, size: usize) {
        if let Val::Addr(o, off) = self.ea(ins) { self.mem.entry(o).or_default().kill(o, off, size as i64); }
    }

    fn op_val(&self, ins: &Instruction, i: u32) -> Val {
        match ins.op_kind(i) {
            OpKind::Register => self.reg(ins.op_register(i)),
            OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32 | OpKind::Immediate64 | OpKind::Immediate8to16 | OpKind::Immediate8to32 | OpKind::Immediate8to64 | OpKind::Immediate32to64 => Val::Imm(ins.immediate(i)),
            OpKind::Memory => self.load_mem(ins, Self::mem_size(ins)),
            _ => Val::Unk,
        }
    }
    fn xmm_src(&self, ins: &Instruction, i: u32, lanes: usize) -> [Val; 4] {
        match ins.op_kind(i) {
            OpKind::Register => self.get_xmm(ins.op_register(i)),
            OpKind::Memory => {
                let mut out = [Val::Unk, Val::Unk, Val::Unk, Val::Unk];
                if let Val::Addr(..) | Val::Img(_) = self.ea(ins) {
                    for (l, slot) in out.iter_mut().enumerate().take(lanes) {
                        let mut tmp = *ins;
                        let _ = &mut tmp;
                        *slot = self.load_lane(ins, l as i64 * 4);
                    }
                }
                out
            }
            _ => [Val::Unk, Val::Unk, Val::Unk, Val::Unk],
        }
    }
    fn load_lane(&self, ins: &Instruction, off: i64) -> Val {
        match self.ea(ins) {
            Val::Addr(o, a) => self.mem.get(&o).map(|m| m.load(o, a + off, 4)).unwrap_or(Val::Unk),
            Val::Img(r) => {
                let writable = self.pe.section_by_rva(r).map(|s| s.characteristics & 0x8000_0000 != 0).unwrap_or(true);
                if writable { Val::Unk } else { self.pe.read_u32((r as i64 + off) as u32).map(|v| Val::Imm(v as u64)).unwrap_or(Val::Unk) }
            }
            _ => Val::Unk,
        }
    }
    fn store_lanes(&mut self, ins: &Instruction, lanes: &[Val], n: usize) {
        if let Val::Addr(o, off) = self.ea(ins) {
            for (l, v) in lanes.iter().enumerate().take(n) { self.mem.entry(o).or_default().store_val(o, off + 4 * l as i64, 4, v); }
        }
    }

    fn describe_target(&self, rva: u32) -> String { self.names.get(&rva).cloned().unwrap_or_default() }

    pub fn fmt_val(&self, v: &Val) -> String {
        match v {
            Val::Unk => "?".into(),
            Val::Imm(x) => if *x > 0xffff { format!("{x:#x}") } else { format!("{x}") },
            Val::Addr(Obj::Frame, off) => match self.rbp_off { Some(b) => format!("&[rbp{}{:#x}]", if *off - b < 0 { "-" } else { "+" }, (*off - b).abs()), None => format!("&frame{off:+#x}") },
            Val::Addr(Obj::Ret(n), off) => if *off == 0 { format!("ret#{n}") } else { format!("ret#{n}+{off:#x}") },
            Val::Addr(Obj::Param(1), off) => format!("this+{off:#x}"),
            Val::Addr(Obj::Param(n), off) => format!("arg{n}+{off:#x}"),
            Val::Img(r) => {
                let off = self.pe.rva_to_off(*r);
                match off {
                    Some(o) if self.pe.section_by_rva(*r).map(|s| s.name != ".text").unwrap_or(false) => {
                        let (a, w) = self.pe.probe_string(o);
                        if w.len() >= 3 { format!("L\"{}\" @rva{:#x}", w, r) } else if a.len() >= 3 { format!("\"{}\" @rva{:#x}", short(&a), r) } else { format!("&rva{:#x}", r) }
                    }
                    _ => format!("&{}rva{:#x}", self.describe_target(*r), r),
                }
            }
            Val::Sym(s) => s.clone(),
        }
    }
    fn fmt_lanes(&self, l: &[Val; 4]) -> String {
        match (&l[0], &l[1]) {
            (Val::Imm(a), _) => {
                let f = f32::from_bits(*a as u32);
                let fs = if f.is_finite() && f != 0.0 && f.abs() > 1e-6 && f.abs() < 1e7 { format!("{f}f") } else { String::new() };
                if fs.is_empty() { format!("{:#x}", a) } else { format!("{fs}") }
            }
            _ => self.fmt_val(&l[0]),
        }
    }

    /// Dump the known cells of the object a pointer value refers to (first `limit` known cells).
    fn dump_obj(&self, v: &Val, first: i64, last: i64) -> Option<String> {
        let Val::Addr(o, off0) = v else { return None };
        let m = self.mem.get(o)?;
        let (off, span) = (off0 + first, last - first);
        let mut s = String::new();
        let mut cells: BTreeMap<i64, String> = BTreeMap::new();
        for ((_, p), val) in m.ptrs.range((*o, off)..(*o, off + span)) { cells.insert(*p - off0, self.fmt_val(val)); }
        let mut i = off;
        while i < off + span {
            if cells.contains_key(&(i - off0)) { i += 8; continue; }
            // gather a dword if all four bytes known
            if (0..4).all(|k| m.bytes.contains_key(&(*o, i + k))) {
                let d = (0..4).fold(0u32, |a, k| a | (*m.bytes.get(&(*o, i + k)).unwrap() as u32) << (8 * k));
                let f = f32::from_bits(d);
                let txt = if d == 0 { "0".to_string() } else if f.is_finite() && f.abs() > 1e-4 && f.abs() < 1e6 && (d >> 23) & 0xff != 0 && d > 0x1000 { format!("{d:#x}={f}f") } else { format!("{d:#x}") };
                cells.insert(i - off0, txt);
                i += 4;
            } else if m.bytes.contains_key(&(*o, i)) {
                cells.insert(i - off0, format!("{:#x}b", m.bytes[&(*o, i)]));
                i += 1;
            } else { i += 1; }
        }
        for (k, v) in cells { let _ = write!(s, " +{k:#x}={v}"); }
        if s.is_empty() { None } else { Some(s) }
    }

    /// Value of a known 32-bit cell of an object (e.g. an `FArguments` field), by object id (`ret#n`) and offset.
    pub fn cell_u32(&self, obj: Obj, off: i64) -> Option<u32> {
        match self.mem.get(&obj)?.load(obj, off, 4) { Val::Imm(v) => Some(v as u32), _ => None }
    }
    pub fn cell_f32(&self, obj: Obj, off: i64) -> Option<f32> { self.cell_u32(obj, off).map(f32::from_bits) }

    fn do_call(&mut self, ins: &Instruction, at: u32) {
        let (target, indirect) = if ins.op0_kind() == OpKind::NearBranch64 {
            (Some((ins.near_branch_target() - self.pe.image_base) as u32), None)
        } else if ins.op0_kind() == OpKind::Memory {
            (None, Some(self.fmt_val(&self.ea(ins))))
        } else { (None, Some(self.fmt_val(&self.reg(ins.op0_register())))) };
        self.ret_counter += 1;
        let ret_id = self.ret_counter;
        let (nx, ns) = target.and_then(|t| self.sigs.get(&t).copied()).unwrap_or((0, 0));
        let (first, last) = target.and_then(|t| self.dump_range.get(&t).copied()).unwrap_or((0, 0x40));
        let mut args = vec![]; let mut dumps = vec![];
        let regs = [self.reg(Register::RCX), self.reg(Register::RDX), self.reg(Register::R8), self.reg(Register::R9)];
        for (lbl, v) in ["rcx", "rdx", "r8", "r9"].iter().zip(regs.iter()) {
            if *v != Val::Unk {
                args.push((lbl.to_string(), self.fmt_val(v)));
                if let Some(d) = self.dump_obj(v, first, last) { if !matches!(v, Val::Addr(Obj::Param(_), _)) { dumps.push((lbl.to_string(), d)); } }
            }
        }
        let mut xmm = [None; 4];
        for (i, r) in [Register::XMM0, Register::XMM1, Register::XMM2, Register::XMM3].iter().enumerate() {
            let l = self.get_xmm(*r);
            if let Val::Imm(b) = l[0] { xmm[i] = Some(f32::from_bits(b as u32)); }
            if i >= 1 && (i as u8) <= nx && l[0] != Val::Unk { args.push((format!("xmm{i}"), self.fmt_lanes(&l))); }
        }
        let rsp = self.rsp;
        let mut stack = vec![];
        for k in 0..6i64 {
            let m = self.mem.get(&Obj::Frame);
            let v8 = m.map(|m| m.load(Obj::Frame, rsp + 0x20 + 8 * k, 8)).unwrap_or(Val::Unk);
            let raw = match (&v8, m.map(|m| m.load(Obj::Frame, rsp + 0x20 + 8 * k, 4))) {
                (Val::Imm(x), _) => Some(*x),
                (_, Some(Val::Imm(d))) => Some(d),
                _ => None,
            };
            stack.push(raw);
            if k < ns as i64 {
                if let Val::Addr(..) | Val::Img(_) | Val::Sym(_) = v8 { args.push((format!("[rsp+{:#x}]", 0x20 + 8 * k), self.fmt_val(&v8))); }
                else if let Some(x) = raw { args.push((format!("[rsp+{:#x}]", 0x20 + 8 * k), if x > 0xffff { format!("{x:#x}") } else { format!("{x}") })); }
            }
        }
        let this_ptr = regs[0].clone();
        let ce = CallEvent { at, target, indirect, ret_id, args, dumps, regs, xmm, stack };
        self.events.push(Event::Call(ce));
        // clobber volatile registers; keep rax as the new return object
        for r in [Register::RCX, Register::RDX, Register::R8, Register::R9, Register::R10, Register::R11] { self.regs.insert(r, Val::Unk); }
        for r in [Register::XMM0, Register::XMM1, Register::XMM2, Register::XMM3, Register::XMM4, Register::XMM5] { self.xmm.insert(r, [Val::Unk, Val::Unk, Val::Unk, Val::Unk]); }
        // outgoing-argument area is dead after the call
        if let Some(m) = self.mem.get_mut(&Obj::Frame) { m.kill(Obj::Frame, rsp + 0x20, 0x30); }
        let fluent = target.map(|t| self.ret_this.contains(&t)).unwrap_or(false);
        self.regs.insert(Register::RAX, if fluent && this_ptr != Val::Unk { this_ptr } else { Val::Addr(Obj::Ret(ret_id), 0) });
        self.xmm.insert(Register::XMM0, [Val::Sym(format!("fret#{ret_id}")), Val::Unk, Val::Unk, Val::Unk]);
    }

    /// Run `[start, end)` (RVAs).
    pub fn run(&mut self, start: u32, end: u32) {
        let Some(bytes) = self.pe.bytes_at_rva(start, (end - start) as usize) else { return };
        let mut dec = Decoder::with_ip(64, bytes, self.pe.image_base + start as u64, DecoderOptions::NONE);
        let mut ins = Instruction::default();
        while dec.can_decode() {
            dec.decode_out(&mut ins);
            let at = (ins.ip() - self.pe.image_base) as u32;
            self.step(&ins, at);
        }
    }

    fn step(&mut self, ins: &Instruction, at: u32) {
        use Mnemonic::*;
        let m = ins.mnemonic();
        match m {
            Call => {
                if ins.op0_kind() == OpKind::NearBranch64 && self.names.get(&((ins.near_branch_target() - self.pe.image_base) as u32)).map(|n| n == "__chkstk").unwrap_or(false) { return; }
                self.do_call(ins, at);
            }
            Jmp | Je | Jne | Ja | Jae | Jb | Jbe | Jg | Jge | Jl | Jle | Js | Jns | Jp | Jnp | Jo | Jno | Jrcxz | Ret | Int3 | Nop | Test | Cmp | Comiss | Comisd | Ucomiss | Ucomisd | Prefetchw | Prefetcht0 | Pause | Endbr64 => {
                if m == Test || m == Cmp || m == Comiss || m == Comisd || m == Ucomiss || m == Ucomisd { /* flags only */ }
            }
            Push => {
                let v = self.op_val(ins, 0);
                self.rsp -= 8;
                self.regs.insert(Register::RSP, Val::Addr(Obj::Frame, self.rsp));
                let rsp = self.rsp; self.mem.entry(Obj::Frame).or_default().store_val(Obj::Frame, rsp, 8, &v);
            }
            Pop => {
                let rsp = self.rsp;
                let v = self.mem.get(&Obj::Frame).map(|mm| mm.load(Obj::Frame, rsp, 8)).unwrap_or(Val::Unk);
                self.set_reg(ins.op0_register(), v);
                self.rsp += 8;
                self.regs.insert(Register::RSP, Val::Addr(Obj::Frame, self.rsp));
            }
            Lea => { let v = self.ea(ins); self.set_reg(ins.op0_register(), v); }
            Mov | Movzx | Movsxd | Movsx => {
                let size = if ins.op0_kind() == OpKind::Memory { ins.op1_register().size().max(if ins.op1_kind() == OpKind::Register { 1 } else { 0 }) } else { 0 };
                let _ = size;
                match ins.op0_kind() {
                    OpKind::Register => {
                        let dst = ins.op0_register();
                        let mut v = if ins.op1_kind() == OpKind::Memory {
                            let sz = if m == Mov { dst.size() } else { Self::mem_size(ins) };
                            self.load_mem(ins, sz)
                        } else { self.op_val(ins, 1) };
                        if m == Movsxd || m == Movsx { if let Val::Imm(x) = v { let sz = if ins.op1_kind() == OpKind::Memory { Self::mem_size(ins) } else { ins.op1_register().size() }; let sh = 64 - 8 * sz as u32; v = Val::Imm((((x << sh) as i64) >> sh) as u64); } }
                        self.set_reg(dst, v);
                    }
                    OpKind::Memory => {
                        let sz = if ins.op1_kind() == OpKind::Register { ins.op1_register().size() } else { Self::mem_size(ins) };
                        let v = self.op_val(ins, 1);
                        let v = if let (Val::Imm(x), true) = (&v, sz < 8) { Val::Imm(*x & ((1u64 << (8 * sz)) - 1)) } else { v };
                        self.store_mem(ins, sz, &v);
                    }
                    _ => {}
                }
            }
            Xor | Sub | And | Or | Add | Inc | Dec | Shl | Shr | Sar | Neg | Not | Imul => {
                if ins.op0_kind() == OpKind::Register {
                    let dst = ins.op0_register();
                    if m == Xor && ins.op1_kind() == OpKind::Register && ins.op1_register().full_register() == dst.full_register() { self.set_reg(dst, Val::Imm(0)); return; }
                    let a = self.reg(dst);
                    let b = if matches!(m, Inc | Dec | Neg | Not) { Val::Imm(1) } else { self.op_val(ins, 1) };
                    let r = match (&a, &b, m) {
                        (Val::Imm(x), Val::Imm(y), Add | Inc) => Val::Imm(x.wrapping_add(*y)),
                        (Val::Imm(x), Val::Imm(y), Sub | Dec) => Val::Imm(x.wrapping_sub(*y)),
                        (Val::Imm(x), Val::Imm(y), And) => Val::Imm(x & y),
                        (Val::Imm(x), Val::Imm(y), Or) => Val::Imm(x | y),
                        (Val::Imm(x), Val::Imm(y), Xor) => Val::Imm(x ^ y),
                        (Val::Imm(x), Val::Imm(y), Shl) => Val::Imm(x << (y & 63)),
                        (Val::Imm(x), Val::Imm(y), Shr) => Val::Imm(x >> (y & 63)),
                        (Val::Imm(x), Val::Imm(y), Imul) => Val::Imm(x.wrapping_mul(*y)),
                        (Val::Addr(o, off), Val::Imm(y), Add) => Val::Addr(*o, off + *y as i64),
                        (Val::Addr(o, off), Val::Imm(y), Sub) => Val::Addr(*o, off - *y as i64),
                        _ => Val::Unk,
                    };
                    if dst.full_register() == Register::RSP && !matches!(r, Val::Addr(Obj::Frame, _)) {
                        // `sub rsp, rax` after a stack-probe call: use the probe size when rax is still known
                        if let (Val::Addr(Obj::Frame, off), Val::Imm(y), Sub) = (&a, &b, m) { let n = Val::Addr(Obj::Frame, off - *y as i64); self.set_reg(dst, n); return; }
                    }
                    self.set_reg(dst, r);
                } else if ins.op0_kind() == OpKind::Memory {
                    let sz = Self::mem_size(ins);
                    let cur = self.load_mem(ins, sz);
                    let b = if matches!(m, Inc | Dec) { Val::Imm(1) } else { self.op_val(ins, 1) };
                    let r = match (&cur, &b, m) {
                        (Val::Imm(x), Val::Imm(y), Or) => Val::Imm(x | y), (Val::Imm(x), Val::Imm(y), And) => Val::Imm(x & y),
                        (Val::Imm(x), Val::Imm(y), Add | Inc) => Val::Imm(x.wrapping_add(*y)), (Val::Imm(x), Val::Imm(y), Sub | Dec) => Val::Imm(x.wrapping_sub(*y)),
                        _ => Val::Unk,
                    };
                    let r = if let Val::Imm(x) = r { Val::Imm(x & (if sz >= 8 { u64::MAX } else { (1u64 << (8 * sz)) - 1 })) } else { r };
                    self.store_mem(ins, sz, &r);
                }
            }
            Xchg | Cmpxchg | Xadd => {
                if ins.op0_kind() == OpKind::Memory { self.kill_mem(ins, Self::mem_size(ins)); }
                else { self.set_reg(ins.op0_register(), Val::Unk); }
                if ins.op1_kind() == OpKind::Register && m != Cmpxchg { self.set_reg(ins.op1_register(), Val::Unk); }
            }
            Cmove | Cmovne | Cmova | Cmovae | Cmovb | Cmovbe | Cmovg | Cmovge | Cmovl | Cmovle | Cmovs | Cmovns => { self.set_reg(ins.op0_register(), Val::Unk); }
            Sete | Setne | Seta | Setae | Setb | Setbe | Setg | Setge | Setl | Setle | Sets | Setns => {
                if ins.op0_kind() == OpKind::Register { self.set_reg(ins.op0_register(), Val::Unk); } else { self.kill_mem(ins, 1); }
            }
            // ---- SSE moves
            Movss | Movd | Movq | Movsd => {
                let lanes = if m == Movss || m == Movd { 1 } else { 2 };
                match (ins.op0_kind(), ins.op1_kind()) {
                    (OpKind::Register, OpKind::Memory) => {
                        let d = ins.op0_register();
                        if d.is_xmm() {
                            let mut l = [Val::Imm(0), Val::Imm(0), Val::Imm(0), Val::Imm(0)];
                            for (i, slot) in l.iter_mut().enumerate().take(lanes) { *slot = self.load_lane(ins, 4 * i as i64); }
                            // symbolic loads from heap/param objects
                            if l[0] == Val::Unk { l[0] = self.load_mem(ins, 4); }
                            self.set_xmm(d, l);
                        } else { let sz = d.size(); let v = self.load_mem(ins, sz); self.set_reg(d, v); }
                    }
                    (OpKind::Register, OpKind::Register) => {
                        let (d, s) = (ins.op0_register(), ins.op1_register());
                        if d.is_xmm() && s.is_xmm() {
                            let sv = self.get_xmm(s);
                            if m == Movss || m == Movsd { let mut dv = self.get_xmm(d); dv[0] = sv[0].clone(); if m == Movsd { dv[1] = sv[1].clone(); } self.set_xmm(d, dv); } else { self.set_xmm(d, sv); }
                        } else if d.is_xmm() { let v = self.reg(s); let l = match v { Val::Imm(x) => [Val::Imm(x & 0xffff_ffff), if m == Movq { Val::Imm(x >> 32) } else { Val::Imm(0) }, Val::Imm(0), Val::Imm(0)], o => [o, Val::Unk, Val::Unk, Val::Unk] }; self.set_xmm(d, l); }
                        else { let sv = self.get_xmm(s); self.set_reg(d, match &sv[0] { Val::Imm(x) => if m == Movq { match &sv[1] { Val::Imm(h) => Val::Imm(x | (h << 32)), _ => Val::Unk } } else { Val::Imm(*x) }, o => o.clone() }); }
                    }
                    (OpKind::Memory, OpKind::Register) => {
                        let s = ins.op1_register();
                        if s.is_xmm() { let l = self.get_xmm(s); self.store_lanes(ins, &l, lanes); }
                        else { let v = self.reg(s); self.store_mem(ins, s.size(), &v); }
                    }
                    _ => {}
                }
            }
            Movaps | Movups | Movdqa | Movdqu | Movapd | Movupd | Lddqu => {
                match (ins.op0_kind(), ins.op1_kind()) {
                    (OpKind::Register, OpKind::Register) => { let l = self.get_xmm(ins.op1_register()); self.set_xmm(ins.op0_register(), l); }
                    (OpKind::Register, OpKind::Memory) => {
                        let mut l = self.xmm_src(ins, 1, 4);
                        // pointer-valued halves (e.g. a stored TSharedPtr pair) survive via the generic 8-byte slots
                        if let Val::Addr(o, off) = self.ea(ins) {
                            if let Some(mm) = self.mem.get(&o) {
                                if let Some(p) = mm.ptrs.get(&(o, off)) { l[0] = p.clone(); l[1] = Val::Unk; }
                                if let Some(p) = mm.ptrs.get(&(o, off + 8)) { l[2] = p.clone(); l[3] = Val::Unk; }
                            }
                        }
                        self.set_xmm(ins.op0_register(), l);
                    }
                    (OpKind::Memory, OpKind::Register) => {
                        let l = self.get_xmm(ins.op1_register());
                        // keep 8-byte pointer slots: when lane0/lane2 are Sym/Addr the halves are stored as pointers
                        if let Val::Addr(o, off) = self.ea(ins) {
                            let mm = self.mem.entry(o).or_default();
                            for half in 0..2 {
                                let (a, b) = (&l[2 * half], &l[2 * half + 1]);
                                let o2 = off + 8 * half as i64;
                                match (a, b) {
                                    (Val::Imm(x), Val::Imm(y)) => { mm.store_imm(o, o2, 8, x | (y << 32)); }
                                    (Val::Imm(_), _) | (_, Val::Imm(_)) => { mm.kill(o, o2, 8); }
                                    (Val::Unk, _) => mm.kill(o, o2, 8),
                                    (v, Val::Unk) => { mm.kill(o, o2, 8); mm.ptrs.insert((o, o2), v.clone()); }
                                    _ => mm.kill(o, o2, 8),
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Xorps | Pxor | Xorpd => {
                if ins.op0_kind() == OpKind::Register && ins.op1_kind() == OpKind::Register && ins.op0_register() == ins.op1_register() { self.set_xmm(ins.op0_register(), [Val::Imm(0), Val::Imm(0), Val::Imm(0), Val::Imm(0)]); }
                else if ins.op0_kind() == OpKind::Register { self.set_xmm(ins.op0_register(), [Val::Unk, Val::Unk, Val::Unk, Val::Unk]); }
            }
            Shufps => {
                if ins.op0_kind() == OpKind::Register {
                    let imm = ins.immediate8();
                    let d = self.get_xmm(ins.op0_register());
                    let s = if ins.op1_kind() == OpKind::Register { self.get_xmm(ins.op1_register()) } else { self.xmm_src(ins, 1, 4) };
                    let l = [d[(imm & 3) as usize].clone(), d[((imm >> 2) & 3) as usize].clone(), s[((imm >> 4) & 3) as usize].clone(), s[((imm >> 6) & 3) as usize].clone()];
                    self.set_xmm(ins.op0_register(), l);
                }
            }
            Cvtdq2ps => {
                if ins.op0_kind() == OpKind::Register {
                    let s = if ins.op1_kind() == OpKind::Register { self.get_xmm(ins.op1_register()) } else { self.xmm_src(ins, 1, 4) };
                    let l = s.map(|v| match v { Val::Imm(x) => Val::Imm((x as u32 as i32 as f32).to_bits() as u64), o => if o == Val::Unk { Val::Unk } else { Val::Sym(format!("(float){}", self.fmt_val(&o))) } });
                    self.set_xmm(ins.op0_register(), l);
                }
            }
            Addss | Subss | Mulss | Divss | Minss | Maxss => {
                if ins.op0_kind() == OpKind::Register {
                    let d = ins.op0_register();
                    let mut dv = self.get_xmm(d);
                    let s = if ins.op1_kind() == OpKind::Register { self.get_xmm(ins.op1_register()) } else { let mut l = [Val::Unk, Val::Unk, Val::Unk, Val::Unk]; l[0] = self.load_lane(ins, 0); l };
                    dv[0] = match (&dv[0], &s[0]) {
                        (Val::Imm(a), Val::Imm(b)) => {
                            let (a, b) = (f32::from_bits(*a as u32), f32::from_bits(*b as u32));
                            let r = match m { Addss => a + b, Subss => a - b, Mulss => a * b, Divss => a / b, Minss => a.min(b), _ => a.max(b) };
                            Val::Imm(r.to_bits() as u64)
                        }
                        (a, b) if *a != Val::Unk && *b != Val::Unk => {
                            let op = match m { Addss => "+", Subss => "-", Mulss => "*", Divss => "/", Minss => "min", _ => "max" };
                            Val::Sym(format!("({} {op} {})", self.fmt_lanes(&[a.clone(), Val::Unk, Val::Unk, Val::Unk]), self.fmt_lanes(&[b.clone(), Val::Unk, Val::Unk, Val::Unk])))
                        }
                        _ => Val::Unk,
                    };
                    self.set_xmm(d, dv);
                }
            }
            Cvtps2pd | Cvtpd2ps | Cvtss2sd | Cvtsd2ss | Cvtsi2ss | Cvtsi2sd | Cvttss2si | Cvttsd2si | Mulsd | Addsd | Subsd | Divsd | Sqrtss | Andps | Orps | Unpcklps | Pshufd | Movlhps | Punpckldq | Movhps | Movlps | Pand | Por => {
                if ins.op0_kind() == OpKind::Register {
                    let d = ins.op0_register();
                    if d.is_xmm() {
                        // keep lane0 value for plain float<->double width changes we cannot represent
                        self.set_xmm(d, [Val::Unk, Val::Unk, Val::Unk, Val::Unk]);
                    } else { self.set_reg(d, Val::Unk); }
                } else if ins.op0_kind() == OpKind::Memory { self.kill_mem(ins, Self::mem_size(ins)); }
            }
            _ => {
                // unknown instruction: destroy its destination
                if ins.op_count() > 0 {
                    match ins.op0_kind() {
                        OpKind::Register => { let r = ins.op0_register(); if r.is_xmm() { self.set_xmm(r, [Val::Unk, Val::Unk, Val::Unk, Val::Unk]); } else { self.set_reg(r, Val::Unk); } }
                        OpKind::Memory => self.kill_mem(ins, Self::mem_size(ins).max(1)),
                        _ => {}
                    }
                }
                if ins.flow_control() == FlowControl::Next { /* ok */ }
            }
        }
        let _ = full;
        let _ = at;
    }

    /// Dump every known cell of parameter `n` (1 = `this`) up to `span` bytes, e.g. to read the defaults a FArguments constructor writes.
    pub fn dump_param(&self, n: u8, span: i64) -> String {
        self.dump_obj(&Val::Addr(Obj::Param(n), 0), 0, span).unwrap_or_default()
    }

    /// Render the whole trace as text.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for e in &self.events {
            match e {
                Event::Call(c) => {
                    if c.target.map(|t| self.skip.contains(&t)).unwrap_or(self.skip_indirect) { continue; }
                    let tgt = match (c.target, &c.indirect) {
                        (Some(t), _) => format!("{:#x}{}", self.pe.image_base + t as u64, self.names.get(&t).map(|n| format!(" <{n}>")).unwrap_or_default()),
                        (None, Some(i)) => format!("[{i}]"),
                        _ => "?".into(),
                    };
                    let args: Vec<String> = c.args.iter().map(|(l, v)| format!("{l}={v}")).collect();
                    let _ = writeln!(s, "{:08x}: ret#{} = call {}({})", c.at, c.ret_id, tgt, args.join(", "));
                    for (l, d) in &c.dumps { let _ = writeln!(s, "            {l} -> {d}"); }
                }
                Event::Note(at, t) => { let _ = writeln!(s, "{at:08x}: ; {t}"); }
            }
        }
        s
    }
}

fn short(s: &str) -> String { if s.len() > 60 { format!("{}...{}", &s[..20], &s[s.len() - 30..]) } else { s.to_string() } }






