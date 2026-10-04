//! Token-stream decoder: DXBC program chunk (SHDR/SHEX) -> [`Shader`].

use crate::container::{Container, DxbcError, Result};
use crate::ir::*;
use crate::opcode::Opcode;

struct Cur<'a> {
    d: &'a [u32],
    p: usize,
    end: usize,
}
impl Cur<'_> {
    fn next(&mut self) -> Result<u32> {
        if self.p >= self.end { return Err(DxbcError::Decode("operand runs past instruction end".into())); }
        let v = self.d[self.p];
        self.p += 1;
        Ok(v)
    }
}

fn read_operand(c: &mut Cur, depth: u32) -> Result<Operand> {
    if depth > 4 { return Err(DxbcError::Decode("relative index nesting".into())); }
    let tok = c.next()?;
    let ncomp = (tok & 3) as u8;
    let sel_mode = (tok >> 2) & 3;
    let sel = if ncomp == 2 {
        match sel_mode {
            0 => Sel::Mask(((tok >> 4) & 0xf) as u8),
            1 => Sel::Swizzle([((tok >> 4) & 3) as u8, ((tok >> 6) & 3) as u8, ((tok >> 8) & 3) as u8, ((tok >> 10) & 3) as u8]),
            2 => Sel::Select1(((tok >> 4) & 3) as u8),
            _ => return Err(DxbcError::Decode("bad selection mode".into())),
        }
    } else {
        Sel::None
    };
    let ty = OperandType(((tok >> 12) & 0xff) as u8);
    let index_dim = ((tok >> 20) & 3) as usize;
    let mut modifier = Modifier::None;
    let mut min_precision = 0;
    let mut nonuniform = false;
    let mut ext = tok >> 31 != 0;
    while ext {
        let e = c.next()?;
        if e & 0x3f == 1 {
            modifier = match (e >> 6) & 0xff { 1 => Modifier::Neg, 2 => Modifier::Abs, 3 => Modifier::AbsNeg, _ => Modifier::None };
            min_precision = ((e >> 14) & 7) as u8;
            nonuniform = (e >> 17) & 1 != 0;
        }
        ext = e >> 31 != 0;
    }
    let mut imm = vec![];
    if ty.0 == OperandType::IMMEDIATE32 {
        let n = match ncomp { 1 => 1, 2 => 4, _ => return Err(DxbcError::Decode("immediate with bad component count".into())) };
        for _ in 0..n { imm.push(c.next()?); }
    } else if ty.0 == OperandType::IMMEDIATE64 {
        let n = match ncomp { 1 => 2, 2 => 8, _ => return Err(DxbcError::Decode("immediate64 with bad component count".into())) };
        for _ in 0..n { imm.push(c.next()?); }
    }
    let mut index = Vec::new();
    for dim in 0..index_dim {
        let rep = (tok >> (22 + 3 * dim)) & 7;
        let mut idx = Index { imm: 0, has_imm: true, rel: None };
        match rep {
            0 => idx.imm = c.next()? as u64,
            1 => { let lo = c.next()? as u64; let hi = c.next()? as u64; idx.imm = lo | (hi << 32); }
            2 => { idx.has_imm = false; idx.rel = Some(Box::new(read_operand(c, depth + 1)?)); }
            3 => { idx.imm = c.next()? as u64; idx.rel = Some(Box::new(read_operand(c, depth + 1)?)); }
            4 => { let lo = c.next()? as u64; let hi = c.next()? as u64; idx.imm = lo | (hi << 32); idx.rel = Some(Box::new(read_operand(c, depth + 1)?)); }
            _ => return Err(DxbcError::Decode(format!("bad index representation {rep}"))),
        }
        index.push(idx);
    }
    Ok(Operand { ty, ncomp, sel, modifier, min_precision, nonuniform, index, imm })
}

/// (operand count, trailing dword count) of a declaration; `None` = every remaining dword is data.
fn decl_layout(op: Opcode) -> Option<(usize, Option<usize>)> {
    use Opcode::*;
    Some(match op {
        DclResource | DclIndexRange | DclInputSgv | DclInputSiv | DclInputPsSgv | DclInputPsSiv | DclOutputSgv | DclOutputSiv | DclUavTyped | DclUavStructured | DclTgsmRaw | DclResourceStructured => (1, Some(1)),
        DclTgsmStructured => (1, Some(2)),
        DclConstantBuffer | DclSampler | DclInput | DclInputPs | DclOutput | DclStream | DclUavRaw | DclResourceRaw => (1, Some(0)),
        DclMaxOutputVertexCount | DclTemps | DclHsMaxTessFactor | DclHsForkPhaseInstanceCount | DclHsJoinPhaseInstanceCount | DclGsInstanceCount | DclFunctionBody => (0, Some(1)),
        DclIndexableTemp | DclThreadGroup => (0, Some(3)),
        DclFunctionTable | DclInterface => (0, None),
        _ => return None,
    })
}

pub fn decode_program(words: &[u32]) -> Result<Shader> {
    if words.len() < 2 { return Err(DxbcError::Truncated("program header")); }
    let ver = words[0];
    let (minor, major, ty) = ((ver & 0xf) as u8, ((ver >> 4) & 0xf) as u8, (ver >> 16) as u16);
    let total = (words[1] as usize).min(words.len());
    let mut p = 2usize;
    let mut instructions = Vec::new();
    while p < total {
        let tok = words[p];
        let op = Opcode::from_u16((tok & 0x7ff) as u16);
        let mut len = ((tok >> 24) & 0x7f) as usize;
        if op == Opcode::CustomData {
            let n = *words.get(p + 1).ok_or(DxbcError::Truncated("customdata"))? as usize;
            if n < 2 || p + n > total { return Err(DxbcError::Decode(format!("bad customdata length {n}"))); }
            instructions.push(Instruction { op, token: tok, ext: Ext::default(), operands: vec![], data: words[p + 2..p + n].to_vec(), custom_class: tok >> 11, offset: p });
            p += n;
            continue;
        }
        if len == 0 || p + len > total { return Err(DxbcError::Decode(format!("bad instruction length {len} at dword {p}"))); }
        let end = p + len;
        let mut c = Cur { d: words, p: p + 1, end };
        let mut ext = Ext::default();
        let mut more = tok >> 31 != 0;
        while more {
            let e = c.next()?;
            match e & 0x3f {
                1 => {
                    let sx = |v: u32| (((v & 0xf) as i8) << 4) >> 4;
                    ext.offset = Some([sx(e >> 9), sx(e >> 13), sx(e >> 17)]);
                }
                2 => { ext.res_dim = Some(((e >> 6) & 0x1f) as u8); ext.res_stride = ((e >> 11) & 0x1ff) as u8; }
                3 => ext.ret_types = Some([((e >> 6) & 0xf) as u8, ((e >> 10) & 0xf) as u8, ((e >> 14) & 0xf) as u8, ((e >> 18) & 0xf) as u8]),
                _ => {}
            }
            more = e >> 31 != 0;
        }
        let mut operands = Vec::new();
        let mut data = Vec::new();
        if let Some((nops, ntrail)) = decl_layout(op) {
            for _ in 0..nops { operands.push(read_operand(&mut c, 0)?); }
            match ntrail { Some(n) => for _ in 0..n { data.push(c.next()?); }, None => while c.p < end { data.push(c.next()?); } }
        } else if let Opcode::Unknown(_) = op {
            // unknown opcode: keep every operand we can decode, never fail the whole shader
            while c.p < end {
                let save = c.p;
                match read_operand(&mut c, 0) { Ok(o) => operands.push(o), Err(_) => { c.p = save; break; } }
            }
            while c.p < end { data.push(c.next()?); }
        } else {
            let (d, s) = op.arity();
            for _ in 0..d + s { operands.push(read_operand(&mut c, 0)?); }
            while c.p < end { data.push(c.next()?); }
        }
        instructions.push(Instruction { op, token: tok, ext, operands, data, custom_class: 0, offset: p });
        len = end - p;
        p += len;
    }
    let mut sh = Shader { program_type: ProgramType::from_u16(ty), major, minor, instructions, decls: Decls::default() };
    sh.decls = collect_decls(&sh.instructions);
    Ok(sh)
}

fn io_decl(i: &Instruction, with_sys: bool) -> Option<IoDecl> {
    let o = i.operands.first()?;
    let reg = o.index.first().map(|x| x.imm as u32).unwrap_or(0);
    let reg2 = o.index.get(1).map(|x| x.imm as u32);
    Some(IoDecl { file: o.ty.0, reg, reg2, mask: o.mask(), sysvalue: if with_sys { i.data.first().copied() } else { None }, interp: ((i.token >> 11) & 0xf) as u8 })
}

fn ret_types(d: u32) -> [u8; 4] { [(d & 0xf) as u8, ((d >> 4) & 0xf) as u8, ((d >> 8) & 0xf) as u8, ((d >> 12) & 0xf) as u8] }

fn collect_decls(instrs: &[Instruction]) -> Decls {
    use Opcode::*;
    let mut d = Decls::default();
    for i in instrs {
        match i.op {
            DclGlobalFlags => d.global_flags = (i.token >> 11) & 0x1fff,
            DclTemps => d.temps = i.data[0],
            DclIndexableTemp => d.indexable_temps.push((i.data[0], i.data[1], i.data[2])),
            DclConstantBuffer => {
                if let Some(o) = i.operands.first() {
                    let slot = o.index.first().map(|x| x.imm as u32).unwrap_or(0);
                    let size = o.index.get(1).map(|x| x.imm as u32).unwrap_or(0);
                    d.cbuffers.push(CbufferDecl { slot, size, dynamic: (i.token >> 11) & 1 != 0 });
                }
            }
            DclSampler => if let Some(o) = i.operands.first() { d.samplers.push(SamplerDecl { slot: o.reg_index().unwrap_or(0), mode: ((i.token >> 11) & 0xf) as u8 }) },
            DclResource | DclResourceRaw | DclResourceStructured => {
                if let Some(o) = i.operands.first() {
                    let (kind, ret) = match i.op {
                        DclResourceRaw => (ResKind::Raw, [5; 4]),
                        DclResourceStructured => (ResKind::Structured(i.data[0]), [5; 4]),
                        _ => (ResKind::Typed, ret_types(i.data[0])),
                    };
                    let dim = if i.op == DclResourceRaw { 11 } else if i.op == DclResourceStructured { 12 } else { ((i.token >> 11) & 0x1f) as u8 };
                    d.resources.push(ResourceDecl { slot: o.reg_index().unwrap_or(0), dim, ret, samples: (i.token >> 16) & 0x7f, kind });
                }
            }
            DclUavTyped | DclUavRaw | DclUavStructured => {
                if let Some(o) = i.operands.first() {
                    let (kind, ret) = match i.op { DclUavRaw => (ResKind::Raw, [5; 4]), DclUavStructured => (ResKind::Structured(i.data[0]), [5; 4]), _ => (ResKind::Typed, ret_types(i.data[0])) };
                    let dim = if i.op == DclUavRaw { 11 } else if i.op == DclUavStructured { 12 } else { ((i.token >> 11) & 0x1f) as u8 };
                    d.uavs.push(UavDecl { slot: o.reg_index().unwrap_or(0), dim, ret, kind, flags: i.token >> 16 });
                }
            }
            DclTgsmRaw => if let Some(o) = i.operands.first() { d.tgsm.push(TgsmDecl { slot: o.reg_index().unwrap_or(0), stride: 0, count: i.data[0] }) },
            DclTgsmStructured => if let Some(o) = i.operands.first() { d.tgsm.push(TgsmDecl { slot: o.reg_index().unwrap_or(0), stride: i.data[0], count: i.data[1] }) },
            DclInput | DclInputPs => if let Some(x) = io_decl(i, false) { d.inputs.push(x) },
            DclInputSgv | DclInputSiv | DclInputPsSgv | DclInputPsSiv => if let Some(x) = io_decl(i, true) { d.inputs.push(x) },
            DclOutput => if let Some(x) = io_decl(i, false) { d.outputs.push(x) },
            DclOutputSgv | DclOutputSiv => if let Some(x) = io_decl(i, true) { d.outputs.push(x) },
            DclThreadGroup => d.thread_group = Some([i.data[0], i.data[1], i.data[2]]),
            DclMaxOutputVertexCount => d.max_output_vertices = Some(i.data[0]),
            DclGsInputPrimitive => d.gs_input_primitive = Some((i.token >> 11) & 0x3f),
            DclGsOutputPrimitiveTopology => d.gs_output_topology = Some((i.token >> 11) & 0x3f),
            CustomData if i.custom_class == 3 => d.icb = Some(i.data.chunks(4).filter(|c| c.len() == 4).map(|c| [c[0], c[1], c[2], c[3]]).collect()),
            _ => {}
        }
    }
    d
}

impl Shader {
    /// Decode the program chunk of a container.
    pub fn from_container(c: &Container) -> Result<Shader> {
        let ch = c.program().ok_or(DxbcError::BadChunk("no SHDR/SHEX chunk".into()))?;
        let words: Vec<u32> = ch.data.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        decode_program(&words)
    }
    pub fn parse(bytes: &[u8]) -> Result<Shader> {
        let c = Container::parse(bytes)?;
        Shader::from_container(&c)
    }
}



