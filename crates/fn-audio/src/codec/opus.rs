//! UE's Opus container (`OPUS` format, `UE4OPUS`): used for streaming SoundWaves in 1.8.
//!
//! Layout (from `FOpusAudioInfo` in UE 4.16, confirmed on the install):
//!
//! ```text
//! "UE4OPUS\0"          8 bytes
//! u16 sample rate      24000 for all 1.8 content (Opus rates only: 8/12/16/24/48 kHz)
//! u32 true sample count (frames per channel after decoding, i.e. the exact length)
//! u8  channels
//! u16 frame count
//! frame count x { u16 packet size ; Opus packet }
//! ```
//!
//! Each packet is a single code-3 Opus packet holding three 20 ms frames (one 60 ms "UE frame" = 1440 samples at
//! 24 kHz). The streaming chunks are concatenated up to their `AudioDataSize` first (see
//! [`crate::soundwave::StreamedAudio::stream_bytes`]).

use super::guarded;
use crate::error::{AudioError, Result};
use crate::wave::DecodedWave;
use opus_decoder::OpusDecoder;

pub const UE_OPUS_ID: &[u8; 8] = b"UE4OPUS\0";

#[derive(Clone, Copy, Debug)]
pub struct OpusHeader {
    pub sample_rate: u32,
    pub true_sample_count: u32,
    pub channels: u8,
    pub frame_count: u16,
}

pub fn parse_header(b: &[u8]) -> Result<OpusHeader> {
    let bad = |d: &str| AudioError::Decode { codec: "ue-opus", detail: d.to_string() };
    if b.len() < 17 || &b[..8] != UE_OPUS_ID {
        return Err(bad("missing UE4OPUS header"));
    }
    Ok(OpusHeader {
        sample_rate: u16::from_le_bytes([b[8], b[9]]) as u32,
        true_sample_count: u32::from_le_bytes(b[10..14].try_into().unwrap()),
        channels: b[14],
        frame_count: u16::from_le_bytes([b[15], b[16]]),
    })
}

/// `opus-decoder` 0.1 rejects any packet over 1500 bytes (libopus only limits each *frame* to 1275), and 48 kHz
/// music packets of three 20 ms CELT frames reach ~1.8 KB. Re-packetize a code-3 packet into its frames, each as a
/// code-0 packet (`toc & 0xFC` + frame bytes). Decoding frame by frame is equivalent: the decoder state carries over.
pub fn split_code3(packet: &[u8]) -> Option<Vec<Vec<u8>>> {
    let toc = *packet.first()?;
    if toc & 3 != 3 {
        return None;
    }
    let body = &packet[1..];
    let ch = *body.first()?;
    let n = (ch & 0x3f) as usize;
    if n == 0 {
        return None;
    }
    let (padding, vbr) = (ch & 0x40 != 0, ch & 0x80 != 0);
    let mut idx = 1usize;
    let mut end = body.len();
    if padding {
        let mut pad = 0usize;
        loop {
            let p = *body.get(idx)? as usize;
            idx += 1;
            pad += if p == 255 { 254 } else { p };
            if p != 255 {
                break;
            }
        }
        end = end.checked_sub(pad)?;
    }
    let size = |idx: &mut usize| -> Option<usize> {
        let b0 = *body.get(*idx)? as usize;
        if b0 < 252 {
            *idx += 1;
            Some(b0)
        } else {
            let b1 = *body.get(*idx + 1)? as usize;
            *idx += 2;
            Some(b1 * 4 + b0)
        }
    };
    let mut sizes = Vec::with_capacity(n);
    if vbr {
        for _ in 0..n - 1 {
            sizes.push(size(&mut idx)?);
        }
        let used: usize = sizes.iter().sum();
        sizes.push(end.checked_sub(idx)?.checked_sub(used)?);
    } else {
        let rem = end.checked_sub(idx)?;
        if rem % n != 0 {
            return None;
        }
        sizes.resize(n, rem / n);
    }
    let mut out = Vec::with_capacity(n);
    for sz in sizes {
        let frame = body.get(idx..idx + sz)?;
        idx += sz;
        let mut p = Vec::with_capacity(sz + 1);
        p.push(toc & 0xFC);
        p.extend_from_slice(frame);
        out.push(p);
    }
    Some(out)
}

/// Decode a UE4OPUS elementary stream to interleaved f32.
pub fn decode_ue_opus(bytes: &[u8]) -> Result<DecodedWave> {
    guarded("ue-opus", || {
        let bad = |d: String| AudioError::Decode { codec: "ue-opus", detail: d };
        let h = parse_header(bytes)?;
        let ch = h.channels as usize;
        if !(1..=2).contains(&ch) {
            return Err(bad(format!("unsupported channel count {ch}")));
        }
        let mut dec = OpusDecoder::new(h.sample_rate, ch).map_err(|e| bad(format!("decoder init: {e:?}")))?;
        // 120 ms is the largest legal Opus packet.
        let max_frames = (h.sample_rate as usize / 1000) * 120;
        let mut pcm = vec![0f32; max_frames * ch];
        let mut samples: Vec<f32> = Vec::with_capacity((h.true_sample_count as usize).min(1 << 26) * ch);
        let mut p = 17usize;
        let mut decoded_frames = 0u16;
        while decoded_frames < h.frame_count {
            let Some(size_bytes) = bytes.get(p..p + 2) else { break };
            let size = u16::from_le_bytes([size_bytes[0], size_bytes[1]]) as usize;
            p += 2;
            let Some(packet) = bytes.get(p..p + size) else { break };
            p += size;
            let parts: Vec<Vec<u8>> = if packet.len() > 1500 { split_code3(packet).ok_or_else(|| bad(format!("packet {decoded_frames}: oversize and not splittable")))? } else { vec![packet.to_vec()] };
            let mut failed = None;
            for part in &parts {
                match dec.decode_float(part, &mut pcm, false) {
                    Ok(n) => samples.extend_from_slice(&pcm[..n.min(max_frames) * ch]),
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                }
            }
            if let Some(e) = failed {
                if samples.is_empty() {
                    return Err(bad(format!("packet {decoded_frames}: {e:?}")));
                }
                break;
            }
            decoded_frames += 1;
        }
        let want = h.true_sample_count as usize * ch;
        if samples.len() > want {
            samples.truncate(want);
        }
        Ok(DecodedWave::new(samples, h.sample_rate, h.channels as u16))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_code3_vbr_cbr_and_padding() {
        // VBR, 3 frames, sizes 3 and 2 coded, last = remainder
        let p = [0xDF, 0x83, 3, 2, 1, 1, 1, 2, 2, 3, 3, 3, 3];
        let parts = split_code3(&p).unwrap();
        assert_eq!(parts, vec![vec![0xDC, 1, 1, 1], vec![0xDC, 2, 2], vec![0xDC, 3, 3, 3, 3]]);
        // CBR, 2 frames of 2 bytes
        let p = [0xFF, 0x02, 9, 9, 8, 8];
        assert_eq!(split_code3(&p).unwrap(), vec![vec![0xFC, 9, 9], vec![0xFC, 8, 8]]);
        // padding (1 pad byte at the end)
        let p = [0xFF, 0x42, 1, 7, 7, 0];
        assert_eq!(split_code3(&p).unwrap(), vec![vec![0xFC, 7], vec![0xFC, 7]]);
        // 2-byte size (252 + 4*1 = 256)
        let mut p = vec![0xFF, 0x82, 252, 1];
        p.extend(vec![5u8; 256]);
        p.extend([6u8; 3]);
        let parts = split_code3(&p).unwrap();
        assert_eq!((parts[0].len(), parts[1].len()), (257, 4));
        // not code 3 / malformed
        assert!(split_code3(&[0xFC, 1, 2]).is_none());
        assert!(split_code3(&[0xFF, 0x83, 200]).is_none());
        assert!(split_code3(&[]).is_none());
    }

    #[test]
    fn header_and_garbage() {
        let mut v = b"UE4OPUS\0".to_vec();
        v.extend_from_slice(&24000u16.to_le_bytes());
        v.extend_from_slice(&481881u32.to_le_bytes());
        v.push(2);
        v.extend_from_slice(&335u16.to_le_bytes());
        let h = parse_header(&v).unwrap();
        assert_eq!((h.sample_rate, h.true_sample_count, h.channels, h.frame_count), (24000, 481881, 2, 335));
        // header only -> zero frames available, not a panic
        assert_eq!(decode_ue_opus(&v).map(|w| w.frames()).unwrap_or(0), 0);
        assert!(decode_ue_opus(b"short").is_err());
        v.extend_from_slice(&[3, 0, 1, 2, 3]);
        let _ = decode_ue_opus(&v);
    }
}

