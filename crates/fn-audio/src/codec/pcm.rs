//! Raw PCM16.

use crate::error::{AudioError, Result};
use crate::wave::DecodedWave;

/// Little-endian interleaved signed 16-bit PCM.
pub fn decode_pcm16(bytes: &[u8], sample_rate: u32, channels: u16) -> Result<DecodedWave> {
    if channels == 0 || sample_rate == 0 {
        return Err(AudioError::Decode { codec: "pcm", detail: "zero channels or rate".into() });
    }
    let usable = bytes.len() - bytes.len() % (2 * channels as usize);
    let s = bytes[..usable].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    Ok(DecodedWave::new(s, sample_rate, channels))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcm_basic_and_ragged_tail() {
        let w = decode_pcm16(&[0, 0, 0, 128, 255, 127, 1], 48000, 1).unwrap();
        assert_eq!(w.frames(), 3);
        assert!((w.samples[1] + 1.0).abs() < 1e-6);
        let st = decode_pcm16(&[0, 0, 0, 0, 1, 0], 8000, 2).unwrap();
        assert_eq!(st.frames(), 1);
        assert!(decode_pcm16(&[0, 0], 8000, 0).is_err());
    }
}
