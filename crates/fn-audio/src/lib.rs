//! Pure-Rust UE 4.16 / Fortnite 1.8 audio.
//!
//! * [`soundwave`] parses the cooked `USoundWave` native tail (format container or streamed chunks).
//! * [`codec`] decodes `OGG` (Vorbis) and `OPUS` ("UE4OPUS") - the only formats in the 1.8 install - plus
//!   `PCM`/`ADPCM`.
//! * [`install`] reads packages from a mounted install; [`sweep`] decodes everything and checks it against the
//!   assets' own properties.
//!
//! See the crate README for the recovered format facts.

pub mod ambient; // AMBIENCE HOOK: recovered ambient system (zones, emitters, bed controller)
pub mod ambient_scan; // AMBIENCE HOOK: build-time level scan that writes data/ambient_layout.json
pub mod analysis;
pub mod attenuation;
pub mod codec;
pub mod cue;
pub mod curve;
#[cfg(feature = "device-output")]
pub mod device;
pub mod error;
pub mod events;
pub mod install;
pub mod library;
pub mod mixer;
pub mod playadj; // AMBIENCE HOOK: per-play live adjustments (fade from / to, position, attenuation override, low-pass)
pub mod music; // MUSIC HOOK: recovered music tables (front-end tracks, loading mix, bus sounds)
pub mod soundclass;
pub mod soundwave;
pub mod stream;
pub mod sweep;
pub mod wave;
pub mod weapon_bp_table; // WEAPON BP TABLE: generated per-Blueprint sound rows of every BR weapon (examples/weaponaudio.rs --rs)
pub mod weapon_events;

pub use error::{AudioError, Result};
pub use install::Install;
pub use wave::{read_wav, wav_bytes, write_wav, DecodedWave, WaveStats};





