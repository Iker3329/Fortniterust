//! Ogg/Vorbis (`OGG` format): a plain Ogg logical stream (serial 0, libVorbis "ENCODER=UnrealEngine4").

use super::guarded;
use crate::error::{AudioError, Result};
use crate::wave::DecodedWave;
use lewton::inside_ogg::OggStreamReader;
use std::io::Cursor;

/// Granule position of the last Ogg page that carries one (= total PCM frames per channel), by walking page headers.
pub fn ogg_total_frames(bytes: &[u8]) -> Option<u64> {
    let mut p = 0usize;
    let mut last = None;
    while p + 27 <= bytes.len() && &bytes[p..p + 4] == b"OggS" {
        let granule = i64::from_le_bytes(bytes[p + 6..p + 14].try_into().ok()?);
        let nseg = bytes[p + 26] as usize;
        let table = bytes.get(p + 27..p + 27 + nseg)?;
        let body: usize = table.iter().map(|&b| b as usize).sum();
        if granule >= 0 {
            last = Some(granule as u64);
        }
        p += 27 + nseg + body;
    }
    last
}

/// Decode a complete Ogg/Vorbis stream to interleaved f32, trimmed to the final granule position.
pub fn decode_ogg(bytes: &[u8]) -> Result<DecodedWave> {
    guarded("ogg", || {
        let err = |e: lewton::VorbisError| AudioError::Decode { codec: "ogg/vorbis", detail: format!("{e:?}") };
        let mut r = OggStreamReader::new(Cursor::new(bytes)).map_err(err)?;
        let channels = r.ident_hdr.audio_channels as u16;
        let rate = r.ident_hdr.audio_sample_rate;
        if channels == 0 || rate == 0 {
            return Err(AudioError::Decode { codec: "ogg/vorbis", detail: "zero channels or rate".into() });
        }
        let mut samples: Vec<f32> = Vec::new();
        loop {
            match r.read_dec_packet_itl() {
                Ok(Some(p)) => samples.extend(p.into_iter().map(|x| x as f32 / 32768.0)),
                Ok(None) => break,
                // A truncated stream still yields everything decoded so far.
                Err(e) => {
                    if samples.is_empty() {
                        return Err(err(e));
                    }
                    break;
                }
            }
        }
        if let Some(total) = ogg_total_frames(bytes) {
            let want = (total as usize).saturating_mul(channels as usize);
            if want < samples.len() {
                samples.truncate(want);
            }
        }
        Ok(DecodedWave::new(samples, rate, channels))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_ogg(&[]).is_err());
        assert!(decode_ogg(b"OggS\0\x02garbage garbage garbage garbage garbage").is_err());
        assert_eq!(ogg_total_frames(b"nope"), None);
    }
}
