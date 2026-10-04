//! Little-endian cursor over a byte slice plus the UE4 primitive encodings (FString, FGuid, FName).

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct Reader<'a> {
    pub b: &'a [u8],
    pub p: usize,
}

impl<'a> Reader<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Reader { b, p: 0 }
    }
    pub fn at(b: &'a [u8], p: usize) -> Self {
        Reader { b, p }
    }
    pub fn pos(&self) -> usize {
        self.p
    }
    pub fn len(&self) -> usize {
        self.b.len()
    }
    pub fn is_empty(&self) -> bool {
        self.b.is_empty()
    }
    pub fn remaining(&self) -> usize {
        self.b.len().saturating_sub(self.p)
    }
    pub fn seek(&mut self, p: usize) -> Result<()> {
        if p > self.b.len() {
            return Err(Error::truncated("seek", p, self.b.len()));
        }
        self.p = p;
        Ok(())
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        if self.p + n > self.b.len() {
            return Err(Error::truncated("skip", self.p + n, self.b.len()));
        }
        self.p += n;
        Ok(())
    }
    #[inline]
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(Error::truncated("read", self.p + n, self.b.len()));
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    #[inline]
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn i8(&mut self) -> Result<i8> {
        Ok(self.u8()? as i8)
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    #[inline]
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    #[inline]
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    pub fn i64(&mut self) -> Result<i64> {
        Ok(self.u64()? as i64)
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.u64()?))
    }
    pub fn guid(&mut self) -> Result<[u8; 16]> {
        Ok(self.bytes(16)?.try_into().unwrap())
    }
    /// UE `bool` as serialised by `FArchive << bool` (4 bytes).
    pub fn bool32(&mut self) -> Result<bool> {
        Ok(self.i32()? != 0)
    }
    /// FString: i32 length (negative = UTF-16 code units), includes the terminating NUL.
    pub fn fstring(&mut self) -> Result<String> {
        let n = self.i32()?;
        if n == 0 {
            return Ok(String::new());
        }
        if n < 0 {
            let cnt = (-(n as i64)) as usize;
            if cnt > self.remaining() / 2 {
                return Err(Error::malformed(format!("FString length {n} exceeds buffer at {}", self.p)));
            }
            let raw = self.bytes(cnt * 2)?;
            let u: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let end = if u.last() == Some(&0) { u.len() - 1 } else { u.len() };
            Ok(String::from_utf16_lossy(&u[..end]))
        } else {
            let cnt = n as usize;
            if cnt > self.remaining() {
                return Err(Error::malformed(format!("FString length {n} exceeds buffer at {}", self.p)));
            }
            let raw = self.bytes(cnt)?;
            let end = if raw.last() == Some(&0) { raw.len() - 1 } else { raw.len() };
            Ok(match std::str::from_utf8(&raw[..end]) {
                Ok(s) => s.to_string(),
                Err(_) => raw[..end].iter().map(|&c| c as char).collect(),
            })
        }
    }
    /// Array count with sanity bound (each element needs at least `min_elem` bytes).
    pub fn count(&mut self, min_elem: usize) -> Result<usize> {
        let n = self.i32()?;
        if n < 0 || (n as usize).saturating_mul(min_elem.max(1)) > self.remaining() + 1 {
            return Err(Error::malformed(format!("bad array count {n} at {}", self.p - 4)));
        }
        Ok(n as usize)
    }
}
