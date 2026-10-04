//! Small signal-analysis helpers used by tests and the CLI to sanity-check decoded audio without listening:
//! radix-2 FFT, dominant frequency, spectral centroid / roll-off, loop-seam continuity.

use crate::wave::DecodedWave;

/// In-place iterative radix-2 FFT (`re`, `im` lengths must be equal powers of two).
pub fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    assert!(n.is_power_of_two() && im.len() == n);
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f64::consts::PI / len as f64;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0, 0.0);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spectrum {
    pub dominant_hz: f32,
    pub centroid_hz: f32,
    /// Frequency below which 95 % of the energy lies.
    pub rolloff95_hz: f32,
    /// Fraction of energy above half Nyquist (very high = noise/aliasing, ~0 = band-limited).
    pub high_band_fraction: f32,
}

/// Spectrum of the mono mix of the `window` frames starting at `start` (Hann-windowed; clamped to a power of two).
pub fn spectrum(w: &DecodedWave, start: usize, window: usize) -> Option<Spectrum> {
    let frames = w.frames();
    if frames < 64 || w.sample_rate == 0 {
        return None;
    }
    let start = start.min(frames - 64);
    let n = window.min(frames - start).next_power_of_two() / if (window.min(frames - start)).is_power_of_two() { 1 } else { 2 };
    let n = n.max(64);
    let ch = w.channels as usize;
    let mut re = vec![0f64; n];
    let mut im = vec![0f64; n];
    for i in 0..n {
        let f = start + i;
        let mut s = 0.0;
        for c in 0..ch {
            s += w.sample(f, c) as f64;
        }
        let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
        re[i] = s / ch as f64 * hann;
    }
    fft(&mut re, &mut im);
    let half = n / 2;
    let mag2: Vec<f64> = (0..half).map(|k| re[k] * re[k] + im[k] * im[k]).collect();
    let total: f64 = mag2.iter().skip(1).sum();
    if total <= 0.0 {
        return Some(Spectrum { dominant_hz: 0.0, centroid_hz: 0.0, rolloff95_hz: 0.0, high_band_fraction: 0.0 });
    }
    let bin = w.sample_rate as f64 / n as f64;
    let (mut best, mut best_k) = (0.0, 1);
    let (mut cen, mut acc, mut roll) = (0.0, 0.0, 0.0);
    let mut roll_set = false;
    let mut high = 0.0;
    for k in 1..half {
        if mag2[k] > best {
            best = mag2[k];
            best_k = k;
        }
        cen += mag2[k] * k as f64 * bin;
        acc += mag2[k];
        if !roll_set && acc >= 0.95 * total {
            roll = k as f64 * bin;
            roll_set = true;
        }
        if k > half / 2 {
            high += mag2[k];
        }
    }
    Some(Spectrum { dominant_hz: (best_k as f64 * bin) as f32, centroid_hz: (cen / total) as f32, rolloff95_hz: roll as f32, high_band_fraction: (high / total) as f32 })
}

/// Average power per octave band (upper edge Hz, dB relative to full scale) over up to 64 evenly spread 4096-frame
/// windows of the mono mix. Bands: <125, <250, <500, <1k, <2k, <4k, <8k, <16k, rest.
pub fn octave_bands_db(w: &DecodedWave) -> Vec<(f32, f32)> {
    const EDGES: [f32; 8] = [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
    let n = 4096usize;
    let frames = w.frames();
    if frames < n || w.sample_rate == 0 {
        return Vec::new();
    }
    let windows = (frames / n).clamp(1, 64);
    let ch = w.channels as usize;
    let mut acc = [0f64; 9];
    let bin = w.sample_rate as f64 / n as f64;
    for wi in 0..windows {
        let start = if windows == 1 { 0 } else { wi * (frames - n) / (windows - 1) };
        let mut re = vec![0f64; n];
        let mut im = vec![0f64; n];
        for i in 0..n {
            let s: f64 = (0..ch).map(|c| w.sample(start + i, c) as f64).sum::<f64>() / ch as f64;
            let hann = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos();
            re[i] = s * hann;
        }
        fft(&mut re, &mut im);
        for k in 1..n / 2 {
            let f = k as f64 * bin;
            let b = EDGES.iter().position(|&e| f < e as f64).unwrap_or(8);
            acc[b] += (re[k] * re[k] + im[k] * im[k]) / (n as f64 * n as f64 * 0.375);
        }
    }
    (0..9).map(|b| (if b < 8 { EDGES[b] } else { f32::INFINITY }, (10.0 * (acc[b] / windows as f64 + 1e-20).log10()) as f32)).collect()
}

/// Loop-seam discontinuity: |last frame -> first frame| step relative to the RMS frame-to-frame step of the signal
/// (mono mix). Around or below 1 means the seam is as smooth as the rest of the material.
pub fn loop_seam_ratio(w: &DecodedWave) -> Option<f32> {
    let n = w.frames();
    if n < 256 {
        return None;
    }
    let ch = w.channels as usize;
    let mono = |f: usize| (0..ch).map(|c| w.sample(f, c)).sum::<f32>() / ch as f32;
    let mut acc = 0f64;
    for f in 1..n {
        let d = (mono(f) - mono(f - 1)) as f64;
        acc += d * d;
    }
    let rms_step = (acc / (n - 1) as f64).sqrt().max(1e-9);
    let seam = (mono(0) - mono(n - 1)).abs() as f64;
    Some((seam / rms_step) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, rate: u32, secs: f32) -> DecodedWave {
        let n = (rate as f32 * secs) as usize;
        DecodedWave::new((0..n).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5).collect(), rate, 1)
    }

    #[test]
    fn fft_finds_a_sine() {
        let w = sine(1000.0, 24000, 1.0);
        let s = spectrum(&w, 0, 4096).unwrap();
        assert!((s.dominant_hz - 1000.0).abs() < 12.0, "{s:?}");
        assert!(s.high_band_fraction < 0.001);
        assert!(s.rolloff95_hz < 1200.0);
    }

    #[test]
    fn seam_ratio_distinguishes_clean_loop_from_a_jump() {
        // exactly 40 periods -> seamless
        let w = sine(600.0, 24000, 1.0);
        assert!(loop_seam_ratio(&w).unwrap() < 2.0, "{:?}", loop_seam_ratio(&w));
        // a jump at the seam
        let mut j = w.clone();
        let last = j.samples.len() - 1;
        j.samples[last] = 0.9;
        j.samples[0] = -0.9;
        assert!(loop_seam_ratio(&j).unwrap() > 10.0);
    }
}

