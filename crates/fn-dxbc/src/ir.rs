//! Typed IR of a decoded SM4/SM5 program: operands, instructions, declarations.

use crate::opcode::Opcode;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct OperandType(pub u8);
impl OperandType {
    pub const TEMP: u8 = 0;
    pub const INPUT: u8 = 1;
    pub const OUTPUT: u8 = 2;
    pub const INDEXABLE_TEMP: u8 = 3;
    pub const IMMEDIATE32: u8 = 4;
    pub const IMMEDIATE64: u8 = 5;
    pub const SAMPLER: u8 = 6;
    pub const RESOURCE: u8 = 7;
    pub const CONSTANT_BUFFER: u8 = 8;
    pub const IMMEDIATE_CONSTANT_BUFFER: u8 = 9;
    pub const LABEL: u8 = 10;
    pub const INPUT_PRIMITIVE_ID: u8 = 11;
    pub const OUTPUT_DEPTH: u8 = 12;
    pub const NULL: u8 = 13;
    pub const RASTERIZER: u8 = 14;
    pub const OUTPUT_COVERAGE_MASK: u8 = 15;
    pub const STREAM: u8 = 16;
    pub const FUNCTION_BODY: u8 = 17;
    pub const FUNCTION_TABLE: u8 = 18;
    pub const INTERFACE: u8 = 19;
    pub const FUNCTION_INPUT: u8 = 20;
    pub const FUNCTION_OUTPUT: u8 = 21;
    pub const OUTPUT_CONTROL_POINT_ID: u8 = 22;
    pub const INPUT_FORK_INSTANCE_ID: u8 = 23;
    pub const INPUT_JOIN_INSTANCE_ID: u8 = 24;
    pub const INPUT_CONTROL_POINT: u8 = 25;
    pub const OUTPUT_CONTROL_POINT: u8 = 26;
    pub const INPUT_PATCH_CONSTANT: u8 = 27;
    pub const INPUT_DOMAIN_POINT: u8 = 28;
    pub const THIS_POINTER: u8 = 29;
    pub const UAV: u8 = 30;
    pub const THREAD_GROUP_SHARED_MEMORY: u8 = 31;
    pub const INPUT_THREAD_ID: u8 = 32;
    pub const INPUT_THREAD_GROUP_ID: u8 = 33;
    pub const INPUT_THREAD_ID_IN_GROUP: u8 = 34;
    pub const INPUT_COVERAGE_MASK: u8 = 35;
    pub const INPUT_THREAD_ID_IN_GROUP_FLATTENED: u8 = 36;
    pub const INPUT_GS_INSTANCE_ID: u8 = 37;
    pub const OUTPUT_DEPTH_GE: u8 = 38;
    pub const OUTPUT_DEPTH_LE: u8 = 39;
    pub const CYCLE_COUNTER: u8 = 40;
    pub const OUTPUT_STENCIL_REF: u8 = 41;
    pub const INNER_COVERAGE: u8 = 42;
    pub fn is(self, k: u8) -> bool { self.0 == k }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modifier { None, Neg, Abs, AbsNeg }

/// Component selection of a 4-component operand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sel {
    /// 0- or 1-component operand (sampler, label, scalar immediate, ...)
    None,
    /// write mask (bit 0 = x)
    Mask(u8),
    /// four source components, each 0..=3
    Swizzle([u8; 4]),
    /// single source component 0..=3
    Select1(u8),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Index {
    pub imm: u64,
    /// an immediate part was encoded (always true without a relative part)
    pub has_imm: bool,
    /// relative part (register holding the dynamic index), if any
    pub rel: Option<Box<Operand>>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Operand {
    pub ty: OperandType,
    /// raw component-count field: 0, 1, 2 (= four components), 3 (= N)
    pub ncomp: u8,
    pub sel: Sel,
    pub modifier: Modifier,
    pub min_precision: u8,
    pub nonuniform: bool,
    pub index: Vec<Index>,
    /// immediate payload (type IMMEDIATE32: 1 or 4 dwords; IMMEDIATE64: 2 or 8 dwords stored in order)
    pub imm: Vec<u32>,
}
impl Operand {
    pub fn reg(ty: u8, n: u32, sel: Sel) -> Operand {
        Operand { ty: OperandType(ty), ncomp: 2, sel, modifier: Modifier::None, min_precision: 0, nonuniform: false, index: vec![Index { imm: n as u64, has_imm: true, rel: None }], imm: vec![] }
    }
    pub fn is(&self, k: u8) -> bool { self.ty.0 == k }
    pub fn is_imm(&self) -> bool { self.ty.0 == OperandType::IMMEDIATE32 }
    /// first index as a plain number (register number); `None` if absent or relative
    pub fn reg_index(&self) -> Option<u32> { self.index.first().filter(|i| i.rel.is_none()).map(|i| i.imm as u32) }
    pub fn mask(&self) -> u8 { match self.sel { Sel::Mask(m) => m, Sel::Swizzle(_) | Sel::Select1(_) => 0xf, Sel::None => 0xf } }
    /// source component `c` (0..4) -> register component
    pub fn src_comp(&self, c: usize) -> usize {
        match self.sel { Sel::Swizzle(s) => s[c] as usize, Sel::Select1(s) => s as usize, Sel::Mask(_) | Sel::None => c }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ext {
    /// sample offsets (aoffimmi)
    pub offset: Option<[i8; 3]>,
    /// resource dimension (SM5 sample/ld/gather)
    pub res_dim: Option<u8>,
    pub res_stride: u8,
    /// per-component return types
    pub ret_types: Option<[u8; 4]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instruction {
    pub op: Opcode,
    /// raw opcode token (bits 11..23 carry instruction specific flags)
    pub token: u32,
    pub ext: Ext,
    pub operands: Vec<Operand>,
    /// trailing literal dwords (declaration counts, return types, ...) or custom data payload
    pub data: Vec<u32>,
    /// custom data class (opcode `customdata` only)
    pub custom_class: u32,
    /// dword offset in the program chunk
    pub offset: usize,
}
impl Instruction {
    pub fn saturate(&self) -> bool { self.token & (1 << 13) != 0 }
    /// `true` for `_nz`, `false` for `_z` (if/breakc/continuec/retc/callc/discard)
    pub fn test_nz(&self) -> bool { self.token & (1 << 18) != 0 }
    pub fn flags(&self) -> u32 { (self.token >> 11) & 0x1fff }
    pub fn dst(&self) -> &[Operand] { let (d, _) = self.op.arity(); &self.operands[..d.min(self.operands.len())] }
    pub fn src(&self) -> &[Operand] { let (d, _) = self.op.arity(); &self.operands[d.min(self.operands.len())..] }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProgramType { Pixel, Vertex, Geometry, Hull, Domain, Compute, Other(u16) }
impl ProgramType {
    pub fn from_u16(n: u16) -> ProgramType { match n { 0 => ProgramType::Pixel, 1 => ProgramType::Vertex, 2 => ProgramType::Geometry, 3 => ProgramType::Hull, 4 => ProgramType::Domain, 5 => ProgramType::Compute, n => ProgramType::Other(n) } }
    pub fn prefix(self) -> &'static str { match self { ProgramType::Pixel => "ps", ProgramType::Vertex => "vs", ProgramType::Geometry => "gs", ProgramType::Hull => "hs", ProgramType::Domain => "ds", ProgramType::Compute => "cs", ProgramType::Other(_) => "xs" } }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CbufferDecl { pub slot: u32, pub size: u32, pub dynamic: bool }
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceDecl { pub slot: u32, pub dim: u8, pub ret: [u8; 4], pub samples: u32, pub kind: ResKind }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResKind { Typed, Raw, Structured(u32) }
#[derive(Clone, Debug, PartialEq)]
pub struct SamplerDecl { pub slot: u32, pub mode: u8 }
#[derive(Clone, Debug, PartialEq)]
pub struct UavDecl { pub slot: u32, pub dim: u8, pub ret: [u8; 4], pub kind: ResKind, pub flags: u32 }
#[derive(Clone, Debug, PartialEq)]
pub struct TgsmDecl { pub slot: u32, /** stride in bytes (0 = raw) */ pub stride: u32, /** raw: size in bytes; structured: element count */ pub count: u32 }
#[derive(Clone, Debug, PartialEq)]
pub struct IoDecl {
    pub file: u8,
    pub reg: u32,
    /// second index (geometry shader input vertex count / control points)
    pub reg2: Option<u32>,
    pub mask: u8,
    pub sysvalue: Option<u32>,
    pub interp: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Decls {
    pub global_flags: u32,
    pub temps: u32,
    pub indexable_temps: Vec<(u32, u32, u32)>,
    pub cbuffers: Vec<CbufferDecl>,
    pub samplers: Vec<SamplerDecl>,
    pub resources: Vec<ResourceDecl>,
    pub uavs: Vec<UavDecl>,
    pub tgsm: Vec<TgsmDecl>,
    pub inputs: Vec<IoDecl>,
    pub outputs: Vec<IoDecl>,
    pub icb: Option<Vec<[u32; 4]>>,
    pub thread_group: Option<[u32; 3]>,
    pub max_output_vertices: Option<u32>,
    pub gs_input_primitive: Option<u32>,
    pub gs_output_topology: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Shader {
    pub program_type: ProgramType,
    pub major: u8,
    pub minor: u8,
    pub instructions: Vec<Instruction>,
    pub decls: Decls,
}
impl Shader {
    pub fn version_string(&self) -> String { format!("{}_{}_{}", self.program_type.prefix(), self.major, self.minor) }
    /// executable instructions (everything that is not a declaration / custom data)
    pub fn body(&self) -> impl Iterator<Item = &Instruction> {
        self.instructions.iter().filter(|i| !i.op.is_decl() && i.op != Opcode::CustomData)
    }
    pub fn cbuffer(&self, slot: u32) -> Option<&CbufferDecl> { self.decls.cbuffers.iter().find(|c| c.slot == slot) }
    pub fn resource(&self, slot: u32) -> Option<&ResourceDecl> { self.decls.resources.iter().find(|c| c.slot == slot) }
}



