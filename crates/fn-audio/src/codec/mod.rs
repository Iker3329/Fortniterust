//! Audio codecs found in (or plausible for) UE 4.16 cooked SoundWaves.
//!
//! * `OGG`   - standard Ogg/Vorbis stream (every non-streaming wave of Fortnite 1.8): [`ogg`]
//! * `OPUS`  - UE's own framed Opus container ("UE4OPUS"), used by streaming waves (ambience, music): [`opus`]
//! * `PCM`   - raw little-endian 16-bit interleaved: [`pcm`]
//! * `ADPCM` - WAV-wrapped Microsoft ADPCM (UE's `FADPCMAudioInfo`): [`adpcm`]
//!
//! Only `OGG` and `OPUS` occur in the 1.8 install; the others are implemented from the UE source layout and
//! unit-tested synthetically.

pub mod adpcm;
pub mod ogg;
pub mod opus;
pub mod pcm;

use crate::error::{AudioError, Result};
use crate::soundwave::SoundWaveAsset;
use crate::wave::DecodedWave;

static PANICS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Number of decoder panics converted into errors so far (the sweep and fuzz tests assert this stays 0).
pub fn panics_caught() -> usize {
    PANICS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Run `f`, converting a panic inside a third-party decoder into an error (the sweep/fuzz tests assert none occur).
pub(crate) fn guarded<T>(codec: &'static str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(_) => {
            PANICS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(AudioError::Decode { codec, detail: "decoder panicked".into() })
        }
    }
}

/// Decode a parsed SoundWave to f32 using its cooked format. Header properties only fill in what the stream lacks.
pub fn decode_asset(a: &SoundWaveAsset) -> Result<DecodedWave> {
    let mut wave = if let Some(s) = &a.streamed {
        match s.format.as_str() {
            "OPUS" => opus::decode_ue_opus(&s.stream_bytes())?,
            other => return Err(AudioError::Unsupported(other.to_string())),
        }
    } else {
        let f = a.primary().ok_or_else(|| AudioError::Decode { codec: "soundwave", detail: "no payload (cooked format container is empty)".into() })?;
        match f.name.as_str() {
            "OGG" => ogg::decode_ogg(&f.payload)?,
            "OPUS" => opus::decode_ue_opus(&f.payload)?,
            "PCM" => pcm::decode_pcm16(&f.payload, a.info.sample_rate.unwrap_or(44100), a.info.channels.unwrap_or(1))?,
            "ADPCM" => adpcm::decode_wav_adpcm(&f.payload)?,
            other => return Err(AudioError::Unsupported(other.to_string())),
        }
    };
    if a.info.looping {
        wave.loop_start = Some(0);
        wave.loop_end = Some(wave.frames() as u64);
    }
    Ok(wave)
}

