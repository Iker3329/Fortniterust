//! DXBC container: header, chunk directory, checksum, input/output signatures (ISGN/OSGN/PCSG and the `1`/`5` variants) and the RDEF
//! resource definition chunk (constant buffers with variables, resource bindings).

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DxbcError {
    Truncated(&'static str),
    BadMagic,
    BadChunk(String),
    Decode(String),
}
impl fmt::Display for DxbcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DxbcError::Truncated(w) => write!(f, "truncated DXBC ({w})"),
            DxbcError::BadMagic => write!(f, "not a DXBC container"),
            DxbcError::BadChunk(s) => write!(f, "bad chunk: {s}"),
            DxbcError::Decode(s) => write!(f, "decode error: {s}"),
        }
    }
}
impl std::error::Error for DxbcError {}
pub type Result<T> = std::result::Result<T, DxbcError>;

pub fn rd_u32(d: &[u8], o: usize) -> Option<u32> { d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) }
pub fn rd_u16(d: &[u8], o: usize) -> Option<u16> { d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]])) }
fn cstr(d: &[u8], o: usize) -> String {
    let Some(s) = d.get(o..) else { return String::new() };
    let n = s.iter().position(|&b| b == 0).unwrap_or(s.len());
    String::from_utf8_lossy(&s[..n]).into_owned()
}

#[derive(Clone, Copy)]
pub struct Chunk<'a> {
    pub tag: [u8; 4],
    pub data: &'a [u8],
}
impl fmt::Debug for Chunk<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "Chunk({} {}B)", self.tag_str(), self.data.len()) }
}
impl Chunk<'_> {
    pub fn tag_str(&self) -> String { String::from_utf8_lossy(&self.tag).into_owned() }
}

#[derive(Debug, Clone)]
pub struct Container<'a> {
    pub data: &'a [u8],
    pub hash: [u8; 16],
    pub version: u32,
    pub total_size: u32,
    pub chunks: Vec<Chunk<'a>>,
}

impl<'a> Container<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Container<'a>> {
        if data.len() < 32 { return Err(DxbcError::Truncated("header")); }
        if &data[0..4] != b"DXBC" { return Err(DxbcError::BadMagic); }
        let mut hash = [0u8; 16];
        hash.copy_from_slice(&data[4..20]);
        let version = rd_u32(data, 20).unwrap();
        let total_size = rd_u32(data, 24).unwrap();
        let count = rd_u32(data, 28).unwrap() as usize;
        if count > 64 { return Err(DxbcError::BadChunk(format!("{count} chunks"))); }
        let mut chunks = Vec::with_capacity(count);
        for i in 0..count {
            let off = rd_u32(data, 32 + 4 * i).ok_or(DxbcError::Truncated("chunk table"))? as usize;
            let tag = data.get(off..off + 4).ok_or(DxbcError::Truncated("chunk tag"))?;
            let size = rd_u32(data, off + 4).ok_or(DxbcError::Truncated("chunk size"))? as usize;
            let body = data.get(off + 8..off + 8 + size).ok_or(DxbcError::Truncated("chunk body"))?;
            chunks.push(Chunk { tag: [tag[0], tag[1], tag[2], tag[3]], data: body });
        }
        Ok(Container { data, hash, version, total_size, chunks })
    }
    pub fn chunk(&self, tag: &[u8; 4]) -> Option<&Chunk<'a>> { self.chunks.iter().find(|c| &c.tag == tag) }
    /// The shader program chunk (`SHDR` for SM4, `SHEX` for SM5).
    pub fn program(&self) -> Option<&Chunk<'a>> { self.chunk(b"SHEX").or_else(|| self.chunk(b"SHDR")) }
    pub fn input_signature(&self) -> Option<Signature> {
        for t in [b"ISGN", b"ISG1"] { if let Some(c) = self.chunk(t) { return Signature::parse(c).ok(); } }
        None
    }
    pub fn output_signature(&self) -> Option<Signature> {
        for t in [b"OSGN", b"OSG5", b"OSG1"] { if let Some(c) = self.chunk(t) { return Signature::parse(c).ok(); } }
        None
    }
    pub fn patch_signature(&self) -> Option<Signature> { self.chunk(b"PCSG").and_then(|c| Signature::parse(c).ok()) }
    pub fn rdef(&self) -> Option<Rdef> { self.chunk(b"RDEF").and_then(|c| Rdef::parse(c.data).ok()) }
    /// `STAT` chunk: instruction count (first dword), or `None` if the chunk is missing (UE strips it).
    pub fn stat_instruction_count(&self) -> Option<u32> { self.chunk(b"STAT").and_then(|c| rd_u32(c.data, 0)) }
    /// Recomputes the Microsoft DXBC checksum of this blob.
    pub fn compute_checksum(&self) -> [u8; 16] { checksum(self.data) }
    pub fn checksum_ok(&self) -> bool { self.compute_checksum() == self.hash }
}

// ---------------------------------------------------------------- checksum (MD5 variant with Microsoft padding)

fn md5_block(st: &mut [u32; 4], w: &[u32; 16]) {
    const S: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
    let [mut a, mut b, mut c, mut d] = *st;
    for i in 0..64usize {
        let (f, g) = match i >> 4 {
            0 => ((b & c) | (!b & d), i),
            1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
            2 => (b ^ c ^ d, (3 * i + 5) % 16),
            _ => (c ^ (b | !d), (7 * i) % 16),
        };
        let k = ((i as f64 + 1.0).sin().abs() * 4294967296.0).floor() as u64 as u32;
        let f = f.wrapping_add(a).wrapping_add(k).wrapping_add(w[g]);
        a = d;
        d = c;
        c = b;
        let s = S[(i >> 4) * 4 + (i & 3)];
        b = b.wrapping_add(f.rotate_left(s));
    }
    st[0] = st[0].wrapping_add(a);
    st[1] = st[1].wrapping_add(b);
    st[2] = st[2].wrapping_add(c);
    st[3] = st[3].wrapping_add(d);
}
fn words(b: &[u8]) -> [u32; 16] {
    let mut w = [0u32; 16];
    for (i, w) in w.iter_mut().enumerate() { *w = u32::from_le_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]); }
    w
}
/// The DXBC hash: MD5 over everything after the 20-byte (magic + hash) prefix, with the odd length-in-bits padding the D3D runtime uses.
pub fn checksum(blob: &[u8]) -> [u8; 16] {
    let data = if blob.len() >= 20 { &blob[20..] } else { &[][..] };
    let size = data.len();
    let bits = (size as u32).wrapping_mul(8);
    let bits2 = (bits >> 2) | 1;
    let mut st = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    let full = size / 64;
    for i in 0..full { md5_block(&mut st, &words(&data[i * 64..i * 64 + 64])); }
    let left = size & 63;
    let mut blk = [0u8; 64];
    if left >= 56 {
        blk[..left].copy_from_slice(&data[full * 64..]);
        blk[left] = 0x80;
        md5_block(&mut st, &words(&blk));
        let mut b2 = [0u8; 64];
        b2[0..4].copy_from_slice(&bits.to_le_bytes());
        b2[60..64].copy_from_slice(&bits2.to_le_bytes());
        md5_block(&mut st, &words(&b2));
    } else {
        blk[0..4].copy_from_slice(&bits.to_le_bytes());
        blk[4..4 + left].copy_from_slice(&data[full * 64..]);
        blk[4 + left] = 0x80;
        blk[60..64].copy_from_slice(&bits2.to_le_bytes());
        md5_block(&mut st, &words(&blk));
    }
    let mut out = [0u8; 16];
    for (i, v) in st.iter().enumerate() { out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes()); }
    out
}
/// Returns a copy of `blob` with its checksum field recomputed (UE stores the container with a zeroed hash).
pub fn resign(blob: &[u8]) -> Vec<u8> {
    let mut v = blob.to_vec();
    if v.len() >= 20 {
        let h = checksum(&v);
        v[4..20].copy_from_slice(&h);
    }
    v
}

// ---------------------------------------------------------------- signatures

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigElement {
    pub name: String,
    pub semantic_index: u32,
    pub sysvalue: u32,
    pub comp_type: u32,
    pub register: u32,
    pub mask: u8,
    pub rw_mask: u8,
    pub stream: u32,
    pub min_precision: u32,
}
impl SigElement {
    pub fn sysvalue_name(&self) -> &'static str {
        if self.sysvalue == 0 {
            let n = self.name.to_ascii_lowercase();
            return match n.as_str() { "sv_target" => "TARGET", "sv_depth" => "DEPTH", "sv_coverage" => "COVERAGE", "sv_depthgreaterequal" => "DEPTHGE", "sv_depthlessequal" => "DEPTHLE", _ => "NONE" };
        }
        sysvalue_short(self.sysvalue)
    }
    pub fn format_name(&self) -> &'static str {
        match self.comp_type { 0 => "unknown", 1 => "uint", 2 => "int", 3 => "float", _ => "unknown" }
    }
}
#[derive(Debug, Clone, Default)]
pub struct Signature { pub tag: [u8; 4], pub elements: Vec<SigElement> }

pub fn sysvalue_short(v: u32) -> &'static str {
    match v {
        0 => "NONE", 1 => "POS", 2 => "CLIPDST", 3 => "CULLDST", 4 => "RTINDEX", 5 => "VPINDEX", 6 => "VERTID", 7 => "PRIMID", 8 => "INSTID",
        9 => "FFACE", 10 => "SAMPLE", 11 => "QUADEDGE", 12 => "QUADINT", 13 => "TRIEDGE", 14 => "TRIINT", 15 => "LINEDET", 16 => "LINEDEN",
        64 => "TARGET", 65 => "DEPTH", 66 => "COVERAGE", 67 => "DEPTHGE", 68 => "DEPTHLE", _ => "NONE",
    }
}

impl Signature {
    pub fn parse(c: &Chunk) -> Result<Signature> {
        let d = c.data;
        let count = rd_u32(d, 0).ok_or(DxbcError::Truncated("signature"))? as usize;
        let (stride, has_stream, has_prec) = match &c.tag {
            b"OSG5" => (28, true, false),
            b"OSG1" => (32, true, true),
            b"ISG1" => (28, false, true),
            _ => (24, false, false),
        };
        let mut elements = Vec::new();
        for i in 0..count {
            let mut o = 8 + i * stride;
            let mut stream = 0;
            if has_stream { stream = rd_u32(d, o).ok_or(DxbcError::Truncated("sig element"))?; o += 4; }
            let g = |k: usize| rd_u32(d, o + 4 * k).ok_or(DxbcError::Truncated("sig element"));
            let name_off = g(0)? as usize;
            let semantic_index = g(1)?;
            let sysvalue = g(2)?;
            let comp_type = g(3)?;
            let register = g(4)?;
            let m = g(5)?;
            let min_precision = if has_prec { g(6)? } else { 0 };
            elements.push(SigElement { name: cstr(d, name_off), semantic_index, sysvalue, comp_type, register, mask: (m & 0xff) as u8, rw_mask: ((m >> 8) & 0xff) as u8, stream, min_precision });
        }
        Ok(Signature { tag: c.tag, elements })
    }
}

// ---------------------------------------------------------------- RDEF

#[derive(Debug, Clone)]
pub struct RdefType {
    pub class: u16,
    pub ty: u16,
    pub rows: u16,
    pub cols: u16,
    pub elements: u16,
    pub members: Vec<(String, RdefType)>,
}
#[derive(Debug, Clone)]
pub struct RdefVar {
    pub name: String,
    pub offset: u32,
    pub size: u32,
    pub flags: u32,
    pub ty: RdefType,
    pub default: Option<Vec<u8>>,
}
#[derive(Debug, Clone)]
pub struct RdefCbuffer {
    pub name: String,
    pub size: u32,
    pub flags: u32,
    pub kind: u32,
    pub vars: Vec<RdefVar>,
}
#[derive(Debug, Clone)]
pub struct RdefBinding {
    pub name: String,
    pub kind: u32,
    pub return_type: u32,
    pub dimension: u32,
    pub num_samples: u32,
    pub bind_point: u32,
    pub bind_count: u32,
    pub flags: u32,
}
#[derive(Debug, Clone)]
pub struct Rdef {
    pub major: u8,
    pub minor: u8,
    pub program_type: u16,
    pub flags: u32,
    pub creator: String,
    pub cbuffers: Vec<RdefCbuffer>,
    pub bindings: Vec<RdefBinding>,
}

impl RdefType {
    pub fn hlsl_name(&self) -> String {
        let base = match self.ty { 0 => "void", 1 => "bool", 2 => "int", 3 => "float", 4 => "string", 19 => "uint", 20 => "uint8", 39 => "double", _ => "unknown" };
        let s = match self.class {
            0 => base.to_string(),
            1 if self.cols == 1 => format!("{base}{}", self.rows.max(1)).replace(&format!("{base}1"), base),
            1 => format!("{base}{}", self.cols),
            2 | 3 => format!("{base}{}x{}", self.rows, self.cols),
            5 => "struct".to_string(),
            _ => base.to_string(),
        };
        s
    }
}

impl Rdef {
    pub fn parse(d: &[u8]) -> Result<Rdef> {
        let t = |w| DxbcError::Truncated(w);
        let cb_count = rd_u32(d, 0).ok_or(t("rdef"))? as usize;
        let cb_off = rd_u32(d, 4).ok_or(t("rdef"))? as usize;
        let rb_count = rd_u32(d, 8).ok_or(t("rdef"))? as usize;
        let rb_off = rd_u32(d, 12).ok_or(t("rdef"))? as usize;
        let ver = rd_u32(d, 16).ok_or(t("rdef"))?;
        let flags = rd_u32(d, 20).ok_or(t("rdef"))?;
        let creator_off = rd_u32(d, 24).ok_or(t("rdef"))? as usize;
        let sm5 = d.get(28..32) == Some(b"RD11");
        let (major, minor, program_type) = (((ver >> 8) & 0xff) as u8, (ver & 0xff) as u8, (ver >> 16) as u16);
        let var_stride = if sm5 { 40 } else { 24 };
        let ty_extra = if sm5 { 16 } else { 0 };
        let _ = ty_extra;
        fn read_type(d: &[u8], off: usize, depth: u32) -> Option<RdefType> {
            let class = rd_u16(d, off)?;
            let ty = rd_u16(d, off + 2)?;
            let rows = rd_u16(d, off + 4)?;
            let cols = rd_u16(d, off + 6)?;
            let elements = rd_u16(d, off + 8)?;
            let mcount = rd_u16(d, off + 10)? as usize;
            let moff = rd_u32(d, off + 12)? as usize;
            let mut members = vec![];
            if depth < 8 {
                for i in 0..mcount {
                    let mo = moff + i * 12;
                    let name = cstr(d, rd_u32(d, mo)? as usize);
                    let tt = read_type(d, rd_u32(d, mo + 4)? as usize, depth + 1)?;
                    members.push((name, tt));
                }
            }
            Some(RdefType { class, ty, rows, cols, elements, members })
        }
        let mut cbuffers = vec![];
        for i in 0..cb_count {
            let o = cb_off + i * 24;
            let name = cstr(d, rd_u32(d, o).ok_or(t("cbuffer"))? as usize);
            let vcount = rd_u32(d, o + 4).ok_or(t("cbuffer"))? as usize;
            let voff = rd_u32(d, o + 8).ok_or(t("cbuffer"))? as usize;
            let size = rd_u32(d, o + 12).ok_or(t("cbuffer"))?;
            let cflags = rd_u32(d, o + 16).ok_or(t("cbuffer"))?;
            let kind = rd_u32(d, o + 20).ok_or(t("cbuffer"))?;
            let mut vars = vec![];
            for j in 0..vcount {
                let vo = voff + j * var_stride;
                let g = |k: usize| rd_u32(d, vo + 4 * k).ok_or(t("variable"));
                let vname = cstr(d, g(0)? as usize);
                let (offset, vsize, vflags, toff, doff) = (g(1)?, g(2)?, g(3)?, g(4)? as usize, g(5)? as usize);
                let ty = read_type(d, toff, 0).ok_or(t("type"))?;
                let default = if doff != 0 { d.get(doff..doff + vsize as usize).map(|s| s.to_vec()) } else { None };
                vars.push(RdefVar { name: vname, offset, size: vsize, flags: vflags, ty, default });
            }
            cbuffers.push(RdefCbuffer { name, size, flags: cflags, kind, vars });
        }
        let mut bindings = vec![];
        for i in 0..rb_count {
            let o = rb_off + i * 32;
            let g = |k: usize| rd_u32(d, o + 4 * k).ok_or(t("binding"));
            bindings.push(RdefBinding { name: cstr(d, g(0)? as usize), kind: g(1)?, return_type: g(2)?, dimension: g(3)?, num_samples: g(4)?, bind_point: g(5)?, bind_count: g(6)?, flags: g(7)? });
        }
        Ok(Rdef { major, minor, program_type, flags, creator: cstr(d, creator_off), cbuffers, bindings })
    }
}


