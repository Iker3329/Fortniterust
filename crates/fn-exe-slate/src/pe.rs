//! Minimal PE64 loader on top of `object` (sections / image base) plus the `.pdata` RUNTIME_FUNCTION table.
use object::pe::{ImageNtHeaders64, IMAGE_DIRECTORY_ENTRY_EXCEPTION};
use object::read::pe::{ImageNtHeaders, ImageOptionalHeader, PeFile64};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Section {
    pub name: String,
    /// RVA of the section start.
    pub rva: u32,
    pub vsize: u32,
    pub raw_off: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}
impl Section {
    pub fn is_exec(&self) -> bool { self.characteristics & 0x2000_0000 != 0 }
    fn span(&self) -> u32 { self.vsize.max(self.raw_size) }
}

pub struct Pe {
    pub data: Vec<u8>,
    pub image_base: u64,
    pub sections: Vec<Section>,
    /// `.pdata` entries (begin RVA, end RVA), sorted.
    pub funcs: Vec<(u32, u32)>,
    /// UnwindInfo RVA of each `.pdata` entry (parallel to `funcs`); used to follow chained (cold / funclet) entries to their parent function.
    pub unwind: Vec<u32>,
}

impl Pe {
    pub fn open(path: impl AsRef<Path>) -> Result<Pe, String> {
        let data = std::fs::read(path.as_ref()).map_err(|e| format!("{}: {e}", path.as_ref().display()))?;
        Pe::from_bytes(data)
    }
    pub fn from_bytes(data: Vec<u8>) -> Result<Pe, String> {
        let (image_base, sections, exc) = {
            let file = PeFile64::parse(&*data).map_err(|e| e.to_string())?;
            let nt = file.nt_headers();
            let image_base = nt.optional_header().image_base();
            let sections: Vec<Section> = file.section_table().iter().map(|s| Section {
                name: String::from_utf8_lossy(s.raw_name()).trim_end_matches('\0').to_string(),
                rva: s.virtual_address.get(object::LittleEndian),
                vsize: s.virtual_size.get(object::LittleEndian),
                raw_off: s.pointer_to_raw_data.get(object::LittleEndian),
                raw_size: s.size_of_raw_data.get(object::LittleEndian),
                characteristics: s.characteristics.get(object::LittleEndian),
            }).collect();
            let exc = file.data_directories().get(IMAGE_DIRECTORY_ENTRY_EXCEPTION).map(|d| (d.virtual_address.get(object::LittleEndian), d.size.get(object::LittleEndian)));
            let _: Option<&ImageNtHeaders64> = None;
            (image_base, sections, exc)
        };
        let mut pe = Pe { data, image_base, sections, funcs: vec![], unwind: vec![] };
        if let Some((rva, size)) = exc {
            if size > 0 {
                if let Some(off) = pe.rva_to_off(rva) {
                    let n = size as usize / 12;
                    let mut v = Vec::with_capacity(n);
                    for i in 0..n {
                        let o = off + i * 12;
                        let b = u32::from_le_bytes(pe.data[o..o + 4].try_into().unwrap());
                        let e = u32::from_le_bytes(pe.data[o + 4..o + 8].try_into().unwrap());
                        let u = u32::from_le_bytes(pe.data[o + 8..o + 12].try_into().unwrap());
                        if b != 0 { v.push((b, e, u)); }
                    }
                    v.sort_unstable();
                    pe.funcs = v.iter().map(|x| (x.0, x.1)).collect();
                    pe.unwind = v.iter().map(|x| x.2).collect();
                }
            }
        }
        Ok(pe)
    }

    pub fn section_by_rva(&self, rva: u32) -> Option<&Section> { self.sections.iter().find(|s| rva >= s.rva && rva < s.rva + s.span()) }
    pub fn section_by_off(&self, off: u64) -> Option<&Section> { self.sections.iter().find(|s| s.raw_size > 0 && off >= s.raw_off as u64 && off < s.raw_off as u64 + s.raw_size as u64) }
    pub fn section_named(&self, n: &str) -> Option<&Section> { self.sections.iter().find(|s| s.name == n) }

    /// File offset -> RVA (None for headers beyond sections / outside the file image).
    pub fn off_to_rva(&self, off: u64) -> Option<u32> { self.section_by_off(off).map(|s| s.rva + (off - s.raw_off as u64) as u32) }
    /// RVA -> file offset (None when the RVA is in zero-fill beyond the raw data or outside any section).
    pub fn rva_to_off(&self, rva: u32) -> Option<usize> {
        let s = self.section_by_rva(rva)?;
        let d = rva - s.rva;
        if d >= s.raw_size { return None; }
        Some(s.raw_off as usize + d as usize)
    }
    pub fn va(&self, rva: u32) -> u64 { self.image_base + rva as u64 }
    pub fn rva_of_va(&self, va: u64) -> Option<u32> { va.checked_sub(self.image_base).filter(|r| *r < u32::MAX as u64).map(|r| r as u32) }

    /// Bytes at an RVA (clamped to the section's raw data).
    pub fn bytes_at_rva(&self, rva: u32, len: usize) -> Option<&[u8]> {
        let off = self.rva_to_off(rva)?;
        let s = self.section_by_rva(rva)?;
        let end = (off + len).min(s.raw_off as usize + s.raw_size as usize);
        self.data.get(off..end)
    }
    pub fn read_u32(&self, rva: u32) -> Option<u32> { self.bytes_at_rva(rva, 4).filter(|b| b.len() == 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())) }
    pub fn read_u64(&self, rva: u32) -> Option<u64> { self.bytes_at_rva(rva, 8).filter(|b| b.len() == 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())) }
    pub fn read_f32(&self, rva: u32) -> Option<f32> { self.read_u32(rva).map(f32::from_bits) }
    pub fn read_f64(&self, rva: u32) -> Option<f64> { self.read_u64(rva).map(f64::from_bits) }

    /// NUL-terminated ASCII string at a file offset (printable only, up to 512 bytes).
    pub fn ascii_at_off(&self, off: usize) -> String {
        self.data[off..].iter().take(512).take_while(|b| **b != 0 && (0x20..0x7f).contains(*b)).map(|b| *b as char).collect()
    }
    /// NUL-terminated UTF-16LE string at a file offset (printable ASCII range only, up to 512 chars).
    pub fn utf16_at_off(&self, off: usize) -> String {
        let mut s = String::new();
        let mut i = off;
        while i + 1 < self.data.len() && s.len() < 512 {
            let c = u16::from_le_bytes([self.data[i], self.data[i + 1]]);
            if c == 0 || !(0x20..0x7f).contains(&c) { break; }
            s.push(c as u8 as char);
            i += 2;
        }
        s
    }
    /// Both interpretations of the bytes at `off`: (ascii, utf16).
    pub fn probe_string(&self, off: usize) -> (String, String) { (self.ascii_at_off(off), self.utf16_at_off(off)) }

    /// Find every occurrence of `needle` as ASCII and as UTF-16LE (file offsets, encoding label).
    pub fn find_string(&self, needle: &str) -> Vec<(usize, &'static str)> {
        let mut out = vec![];
        let a = needle.as_bytes();
        let w: Vec<u8> = needle.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        for (pat, lbl) in [(a.to_vec(), "ascii"), (w, "utf16")] {
            let mut start = 0;
            while let Some(p) = memchr_find(&self.data[start..], &pat) { out.push((start + p, lbl)); start += p + 1; }
        }
        out.sort();
        out
    }

    /// Index into `funcs` of the function containing `rva` (by `.pdata` ranges).
    pub fn func_containing(&self, rva: u32) -> Option<(u32, u32)> {
        let i = self.funcs.partition_point(|f| f.0 <= rva);
        if i == 0 { return None; }
        let f = self.funcs[i - 1];
        // Chained/unwind-only entries have begin<=rva<end; fall back to nearest below when in a gap.
        if rva < f.1 { Some(f) } else { None }
    }
}

impl Pe {
    /// The function containing `rva` as the compiler sees it: UNWIND_INFO chains (cold paths / funclets, flag UNW_FLAG_CHAININFO) are followed to the parent
    /// RUNTIME_FUNCTION. Returns the root (begin, end). Adjacent but unrelated functions are NOT merged.
    pub fn func_group(&self, rva: u32) -> Option<(u32, u32)> {
        let (mut b, _) = self.func_containing(rva)?;
        for _ in 0..32 {
            let i = self.funcs.partition_point(|f| f.0 < b);
            if i >= self.funcs.len() || self.funcs[i].0 != b { break; }
            let u = self.unwind.get(i).copied().unwrap_or(0);
            let Some(off) = self.rva_to_off(u & !1) else { break };
            let flags = self.data[off] >> 3;
            if flags & 4 == 0 { break; }
            let codes = (self.data[off + 2] as usize + 1) & !1;
            let p = off + 4 + codes * 2;
            let Some(pb) = self.data.get(p..p + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())) else { break };
            if pb == b { break; }
            b = pb;
        }
        let i = self.funcs.partition_point(|f| f.0 < b);
        self.funcs.get(i).copied().filter(|f| f.0 == b)
    }
}

fn memchr_find(hay: &[u8], pat: &[u8]) -> Option<usize> {
    if pat.is_empty() || hay.len() < pat.len() { return None; }
    let first = pat[0];
    let mut i = 0;
    let last = hay.len() - pat.len();
    while i <= last {
        match hay[i..=last].iter().position(|b| *b == first) {
            None => return None,
            Some(p) => { i += p; if &hay[i..i + pat.len()] == pat { return Some(i); } i += 1; }
        }
    }
    None
}




#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memchr_find_locates_ascii_and_utf16_needles() {
        let hay = b"xx\0LOADING\0y\0L\0O\0A\0D\0";
        assert_eq!(memchr_find(hay, b"LOADING"), Some(3));
        assert_eq!(memchr_find(hay, b"missing"), None);
        assert_eq!(memchr_find(hay, b"L\0O\0A\0D\0"), Some(hay.len() - 8));
        assert_eq!(memchr_find(b"", b"a"), None);
    }
}

