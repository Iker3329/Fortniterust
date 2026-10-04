//! Decoded PCM and WAV output.

use crate::error::{AudioError, Result};
use std::io::Write;
use std::path::Path;

/// Interleaved f32 PCM (nominal range -1..1).
#[derive(Clone, Debug, Default)]
pub struct DecodedWave {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    /// Loop points in frames (set from the asset's `bLooping`: `0..frames`).
    pub loop_start: Option<u64>,
    pub loop_end: Option<u64>,
}

impl DecodedWave {
    pub fn new(samples: Vec<f32>, sample_rate: u32, channels: u16) -> Self {
        DecodedWave { samples, sample_rate, channels, loop_start: None, loop_end: None }
    }
    pub fn frames(&self) -> usize {
        if self.channels == 0 {
            0
        } else {
            self.samples.len() / self.channels as usize
        }
    }
    pub fn duration_seconds(&self) -> f64 {
        if self.sample_rate == 0 {
            0.0
        } else {
            self.frames() as f64 / self.sample_rate as f64
        }
    }
    /// Interleaved frame `i` channel `c` (silence beyond the end).
    #[inline]
    pub fn sample(&self, frame: usize, ch: usize) -> f32 {
        self.samples.get(frame * self.channels as usize + ch).copied().unwrap_or(0.0)
    }
    /// Peak absolute sample, RMS over all samples, and the count of non-finite samples.
    pub fn stats(&self) -> WaveStats {
        let mut peak = 0f32;
        let mut sum = 0f64;
        let mut bad = 0usize;
        let mut clip_run = 0usize;
        let mut max_clip_run = 0usize;
        for &s in &self.samples {
            if !s.is_finite() {
                bad += 1;
                continue;
            }
            peak = peak.max(s.abs());
            sum += (s as f64) * (s as f64);
            if s.abs() >= 0.9999 {
                clip_run += 1;
                max_clip_run = max_clip_run.max(clip_run);
            } else {
                clip_run = 0;
            }
        }
        let n = self.samples.len().max(1) as f64;
        WaveStats { peak, rms: (sum / n).sqrt() as f32, non_finite: bad, max_clip_run }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveStats {
    pub peak: f32,
    pub rms: f32,
    pub non_finite: usize,
    /// Longest run of consecutive samples at full scale (clipping indicator).
    pub max_clip_run: usize,
}

/// Canonical 16-bit PCM RIFF/WAVE bytes.
pub fn wav_bytes(wave: &DecodedWave) -> Result<Vec<u8>> {
    let data = wave.samples.len().checked_mul(2).filter(|&n| n < u32::MAX as usize - 44).ok_or_else(|| AudioError::Decode { codec: "wav", detail: "too large".into() })? as u32;
    let mut f = Vec::with_capacity(44 + data as usize);
    f.extend_from_slice(b"RIFF");
    f.extend_from_slice(&(36 + data).to_le_bytes());
    f.extend_from_slice(b"WAVEfmt ");
    f.extend_from_slice(&16u32.to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&wave.channels.to_le_bytes());
    f.extend_from_slice(&wave.sample_rate.to_le_bytes());
    f.extend_from_slice(&(wave.sample_rate * wave.channels as u32 * 2).to_le_bytes());
    f.extend_from_slice(&(wave.channels * 2).to_le_bytes());
    f.extend_from_slice(&16u16.to_le_bytes());
    f.extend_from_slice(b"data");
    f.extend_from_slice(&data.to_le_bytes());
    for &s in &wave.samples {
        let v = if s.is_finite() { (s.clamp(-1.0, 1.0) * 32767.0).round() as i16 } else { 0 };
        f.extend_from_slice(&v.to_le_bytes());
    }
    Ok(f)
}

pub fn write_wav(path: impl AsRef<Path>, wave: &DecodedWave) -> Result<()> {
    let bytes = wav_bytes(wave)?;
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    f.write_all(&bytes)?;
    f.flush()?;
    Ok(())
}

/// Parse a 16-bit PCM WAV produced by [`wav_bytes`] (used by tests and the CLI analysis).
pub fn read_wav(bytes: &[u8]) -> Result<DecodedWave> {
    let bad = |d: &str| AudioError::Decode { codec: "wav", detail: d.to_string() };
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad("not a RIFF/WAVE file"));
    }
    let mut p = 12;
    let (mut ch, mut rate, mut bits) = (0u16, 0u32, 0u16);
    while p + 8 <= bytes.len() {
        let id = &bytes[p..p + 4];
        let sz = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
        let body = p + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            ch = u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]);
            rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().unwrap());
            bits = u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]);
        } else if id == b"data" {
            if bits != 16 || ch == 0 {
                return Err(bad("only 16-bit PCM supported"));
            }
            let end = (body + sz).min(bytes.len());
            let s = bytes[body..end].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
            return Ok(DecodedWave::new(s, rate, ch));
        }
        p = body + sz + (sz & 1);
    }
    Err(bad("no data chunk"))
}
