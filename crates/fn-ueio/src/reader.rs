use crate::{f16_to_f32, Error, Result};

/// Little-endian cursor over a byte slice.
#[derive(Clone)]
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

macro_rules! num {
    ($name:ident, $t:ty, $n:expr) => {
        pub fn $name(&mut self) -> Result<$t> {
            let b = self.bytes($n)?;
            Ok(<$t>::from_le_bytes(b.try_into().unwrap()))
        }
    };
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }
    pub fn at(data: &'a [u8], pos: usize) -> Self {
        Reader { data, pos }
    }
    pub fn tell(&self) -> usize {
        self.pos
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }
    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(Error::Eof { pos, wanted: 0 });
        }
        self.pos = pos;
        Ok(())
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.bytes(n).map(|_| ())
    }
    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(Error::Eof { pos: self.pos, wanted: n })?;
        if end > self.data.len() {
            return Err(Error::Eof { pos: self.pos, wanted: n });
        }
        let s = &self.data[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    num!(u8, u8, 1);
    num!(i8, i8, 1);
    num!(u16, u16, 2);
    num!(i16, i16, 2);
    num!(u32, u32, 4);
    num!(i32, i32, 4);
    num!(u64, u64, 8);
    num!(i64, i64, 8);
    num!(f32, f32, 4);
    num!(f64, f64, 8);
    pub fn f16(&mut self) -> Result<f32> {
        Ok(f16_to_f32(self.u16()?))
    }
    /// UE `bool` serialised as int32.
    pub fn bool32(&mut self) -> Result<bool> {
        Ok(self.i32()? != 0)
    }
    /// FString: i32 length; >0 ANSI (incl. NUL), <0 UTF-16 (-len chars incl. NUL).
    pub fn fstring(&mut self) -> Result<String> {
        let len = self.i32()?;
        if len == 0 {
            return Ok(String::new());
        }
        if len > 0 {
            let b = self.bytes(len as usize)?;
            let b = if b.last() == Some(&0) { &b[..b.len() - 1] } else { b };
            Ok(b.iter().map(|&c| c as char).collect())
        } else {
            let n = (-(len as i64)) as usize;
            let b = self.bytes(n * 2)?;
            let v: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            let v = if v.last() == Some(&0) { &v[..v.len() - 1] } else { &v[..] };
            Ok(String::from_utf16_lossy(v))
        }
    }
    pub fn vec3(&mut self) -> Result<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    pub fn quat(&mut self) -> Result<[f32; 4]> {
        Ok([self.f32()?, self.f32()?, self.f32()?, self.f32()?])
    }
    pub fn guid(&mut self) -> Result<[u8; 16]> {
        Ok(self.bytes(16)?.try_into().unwrap())
    }
}
