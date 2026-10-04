//! `ADPCM` format: UE stores a complete RIFF/WAVE image holding Microsoft ADPCM (`wFormatTag = 2`) and decodes it
//! with `FADPCMAudioInfo`. No wave in the 1.8 install uses it (all are `OGG`/`OPUS`), so this is implemented from the
//! format definition and verified with a round trip through a reference encoder in the unit tests.

use crate::error::{AudioError, Result};
use crate::wave::DecodedWave;

const STD_COEF: [(i32, i32); 7] = [(256, 0), (512, -256), (0, 0), (192, 64), (240, 0), (460, -208), (392, -232)];
const ADAPT: [i32; 16] = [230, 230, 230, 230, 307, 409, 512, 614, 768, 614, 512, 409, 307, 230, 230, 230];

fn bad<T>(d: &str) -> Result<T> {
    Err(AudioError::Decode { codec: "ms-adpcm", detail: d.to_string() })
}

/// Parse the RIFF image and decode every complete block.
pub fn decode_wav_adpcm(bytes: &[u8]) -> Result<DecodedWave> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return bad("missing RIFF/WAVE header");
    }
    let (mut p, mut fmt, mut data): (usize, Option<&[u8]>, Option<&[u8]>) = (12, None, None);
    while p + 8 <= bytes.len() {
        let sz = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
        let body = p + 8;
        let end = body.saturating_add(sz).min(bytes.len());
        match &bytes[p..p + 4] {
            b"fmt " => fmt = Some(&bytes[body..end]),
            b"data" => data = Some(&bytes[body..end]),
            _ => {}
        }
        p = body.saturating_add(sz).saturating_add(sz & 1);
    }
    let (Some(f), Some(d)) = (fmt, data) else { return bad("missing fmt or data chunk") };
    if f.len() < 16 || u16::from_le_bytes([f[0], f[1]]) != 2 {
        return bad("not WAVE_FORMAT_ADPCM");
    }
    let channels = u16::from_le_bytes([f[2], f[3]]);
    let rate = u32::from_le_bytes(f[4..8].try_into().unwrap());
    let align = u16::from_le_bytes([f[12], f[13]]) as usize;
    let mut coefs = STD_COEF;
    if f.len() >= 22 {
        let ncoef = u16::from_le_bytes([f[20], f[21]]) as usize;
        if ncoef == 7 && f.len() >= 22 + 28 {
            for (i, c) in coefs.iter_mut().enumerate() {
                let o = 22 + i * 4;
                *c = (i16::from_le_bytes([f[o], f[o + 1]]) as i32, i16::from_le_bytes([f[o + 2], f[o + 3]]) as i32);
            }
        }
    }
    decode_blocks(d, channels, rate, align, &coefs)
}

struct ChanState {
    pred: usize,
    delta: i32,
    s1: i32,
    s2: i32,
}

impl ChanState {
    fn nibble(&mut self, n: u8, coefs: &[(i32, i32); 7]) -> f32 {
        let (c1, c2) = coefs[self.pred];
        let sn = if n & 8 != 0 { n as i32 - 16 } else { n as i32 };
        let v = (((self.s1 * c1 + self.s2 * c2) >> 8) + sn * self.delta).clamp(-32768, 32767);
        self.s2 = self.s1;
        self.s1 = v;
        self.delta = ((ADAPT[n as usize] * self.delta) >> 8).max(16);
        v as f32 / 32768.0
    }
}

pub fn decode_blocks(data: &[u8], channels: u16, rate: u32, block_align: usize, coefs: &[(i32, i32); 7]) -> Result<DecodedWave> {
    let ch = channels as usize;
    if !(1..=2).contains(&ch) || rate == 0 || block_align < 7 * ch {
        return bad("bad channel count, rate or block alignment");
    }
    let mut out: Vec<f32> = Vec::new();
    for block in data.chunks(block_align) {
        if block.len() < 7 * ch {
            break;
        }
        let mut st: Vec<ChanState> = Vec::with_capacity(ch);
        for c in 0..ch {
            let pred = block[c] as usize;
            if pred >= 7 {
                return bad("predictor index out of range");
            }
            st.push(ChanState { pred, delta: 0, s1: 0, s2: 0 });
        }
        let rd = |o: usize| i16::from_le_bytes([block[o], block[o + 1]]) as i32;
        for (c, s) in st.iter_mut().enumerate() {
            s.delta = rd(ch + 2 * c);
            s.s1 = rd(ch + 2 * ch + 2 * c);
            s.s2 = rd(ch + 4 * ch + 2 * c);
        }
        for s in &st {
            out.push(s.s2 as f32 / 32768.0);
        }
        for s in &st {
            out.push(s.s1 as f32 / 32768.0);
        }
        for &b in &block[7 * ch..] {
            if ch == 1 {
                out.push(st[0].nibble(b >> 4, coefs));
                out.push(st[0].nibble(b & 15, coefs));
            } else {
                out.push(st[0].nibble(b >> 4, coefs));
                out.push(st[1].nibble(b & 15, coefs));
            }
        }
    }
    Ok(DecodedWave::new(out, rate, channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reference MS ADPCM block encoder (mono; picks the best of the 7 standard predictors).
    fn encode_block(pcm: &[i16]) -> Vec<u8> {
        let (mut best, mut best_err) = (Vec::new(), i64::MAX);
        for (pi, &(c1, c2)) in STD_COEF.iter().enumerate() {
            let mut blk = vec![pi as u8];
            let mut delta = 16i32.max((pcm[2] as i32 - pcm[1] as i32).abs() / 4);
            blk.extend_from_slice(&(delta as i16).to_le_bytes());
            blk.extend_from_slice(&pcm[1].to_le_bytes());
            blk.extend_from_slice(&pcm[0].to_le_bytes());
            let (mut s1, mut s2) = (pcm[1] as i32, pcm[0] as i32);
            let mut err = 0i64;
            let mut nibs = Vec::new();
            for &x in &pcm[2..] {
                let predv = (s1 * c1 + s2 * c2) >> 8;
                let q = (((x as i32 - predv) as f32 / delta as f32).round() as i32).clamp(-8, 7);
                let n = (q & 15) as u8;
                let v = (predv + q * delta).clamp(-32768, 32767);
                err += ((v - x as i32) as i64).pow(2);
                s2 = s1;
                s1 = v;
                delta = ((ADAPT[n as usize] * delta) >> 8).max(16);
                nibs.push(n);
            }
            for pair in nibs.chunks(2) {
                blk.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
            }
            if err < best_err {
                best_err = err;
                best = blk;
            }
        }
        best
    }

    fn wav(blocks: &[Vec<u8>], align: usize, spb: u16) -> Vec<u8> {
        let data: Vec<u8> = blocks.concat();
        let mut f = Vec::new();
        f.extend_from_slice(b"RIFF\0\0\0\0WAVEfmt ");
        f.extend_from_slice(&(50u32).to_le_bytes());
        f.extend_from_slice(&2u16.to_le_bytes());
        f.extend_from_slice(&1u16.to_le_bytes());
        f.extend_from_slice(&22050u32.to_le_bytes());
        f.extend_from_slice(&0u32.to_le_bytes());
        f.extend_from_slice(&(align as u16).to_le_bytes());
        f.extend_from_slice(&4u16.to_le_bytes());
        f.extend_from_slice(&32u16.to_le_bytes());
        f.extend_from_slice(&spb.to_le_bytes());
        f.extend_from_slice(&7u16.to_le_bytes());
        for (a, b) in STD_COEF {
            f.extend_from_slice(&(a as i16).to_le_bytes());
            f.extend_from_slice(&(b as i16).to_le_bytes());
        }
        f.extend_from_slice(b"data");
        f.extend_from_slice(&(data.len() as u32).to_le_bytes());
        f.extend_from_slice(&data);
        f
    }

    #[test]
    fn sine_round_trip_through_reference_encoder() {
        let n = 2 + 2 * 250;
        let pcm: Vec<i16> = (0..n * 2).map(|i| ((i as f32 * 0.12).sin() * 12000.0) as i16).collect();
        let blocks: Vec<Vec<u8>> = pcm.chunks(n).filter(|c| c.len() == n).map(encode_block).collect();
        let align = blocks[0].len();
        let w = decode_wav_adpcm(&wav(&blocks, align, n as u16)).unwrap();
        assert_eq!(w.frames(), n * 2);
        let mut e = 0f64;
        for i in 0..w.frames() {
            e += ((w.samples[i] * 32768.0) as f64 - pcm[i] as f64).powi(2);
        }
        let rms_err = (e / w.frames() as f64).sqrt();
        assert!(rms_err < 600.0, "rms error {rms_err}");
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode_wav_adpcm(b"nope").is_err());
        assert!(decode_wav_adpcm(&[0u8; 64]).is_err());
    }
}
