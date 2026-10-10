//! cpal (WASAPI on Windows) output. Compiled with the `device-output` feature.
//!
//! `open_default()` opens the default output device with its default config, builds a [`Mixer`] at the device sample
//! rate and runs it inside the cpal callback. The callback only drains the lock-free command queue and renders; it
//! never blocks or locks (waves/cues arrive as `Arc`s through the same queue).

#[cfg(not(target_arch = "wasm32"))]
use crate::error::{AudioError, Result};
#[cfg(not(target_arch = "wasm32"))]
use crate::mixer::EngineHandle;
#[cfg(not(target_arch = "wasm32"))]
use crate::mixer::{engine_pair, Mixer};
#[cfg(not(target_arch = "wasm32"))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
#[cfg(not(target_arch = "wasm32"))]
use cpal::{FromSample, SampleFormat, SizedSample};

/// Keeps the output stream alive; dropping it stops audio. (cpal streams are not `Send` on every platform: keep this
/// on the thread that created it.)
pub struct DeviceStream {
    #[cfg(not(target_arch = "wasm32"))]
    _stream: cpal::Stream,
    #[cfg(target_arch = "wasm32")]
    _ring: std::sync::Arc<web::Ring>,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[cfg(not(target_arch = "wasm32"))]
fn dev_err(e: impl std::fmt::Display) -> AudioError {
    AudioError::Device(e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, mut mixer: Mixer) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let mut tmp: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let frames = data.len() / channels.max(1);
                if tmp.len() < frames * 2 {
                    tmp.resize(frames * 2, 0.0);
                }
                mixer.render(&mut tmp[..frames * 2]);
                for f in 0..frames {
                    let (l, r) = (tmp[f * 2].clamp(-1.0, 1.0), tmp[f * 2 + 1].clamp(-1.0, 1.0));
                    let frame = &mut data[f * channels..(f + 1) * channels];
                    match channels {
                        1 => frame[0] = T::from_sample((l + r) * 0.5),
                        _ => {
                            frame[0] = T::from_sample(l);
                            frame[1] = T::from_sample(r);
                            for s in &mut frame[2..] {
                                *s = T::from_sample(0.0);
                            }
                        }
                    }
                }
            },
            |e| eprintln!("fn-audio: output stream error: {e}"),
            None,
        )
        .map_err(dev_err)
}

/// Open the default output device. Returns the command handle and the running stream.
#[cfg(not(target_arch = "wasm32"))]
pub fn open_default() -> Result<(EngineHandle, DeviceStream)> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or_else(|| AudioError::Device("no default output device".into()))?;
    let name = device.name().unwrap_or_else(|_| "<unnamed>".into());
    let supported = device.default_output_config().map_err(dev_err)?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();
    let (handle, mixer) = engine_pair(config.sample_rate.0);
    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, mixer)?,
        SampleFormat::I16 => build::<i16>(&device, &config, mixer)?,
        SampleFormat::U16 => build::<u16>(&device, &config, mixer)?,
        SampleFormat::I32 => build::<i32>(&device, &config, mixer)?,
        other => return Err(AudioError::Device(format!("unsupported sample format {other:?}"))),
    };
    stream.play().map_err(dev_err)?;
    Ok((handle, DeviceStream { _stream: stream, device_name: name, sample_rate: config.sample_rate.0, channels: config.channels }))
}

#[cfg(target_arch = "wasm32")]
pub use web::open_default;

#[cfg(target_arch = "wasm32")]
pub mod web {
    use super::DeviceStream;
    use crate::error::{AudioError, Result};
    use crate::mixer::{engine_pair, EngineHandle};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, Instant};

    pub const RATE: u32 = 48_000;
    pub const CAPACITY: u32 = 16_384;
    pub const TARGET: u32 = 7_168;
    pub const BLOCK: u32 = 256;

    pub struct Ring {
        pub write: AtomicU32,
        pub read: AtomicU32,
        pub max_block_us: AtomicU32,
        pub max_gap_us: AtomicU32,
        pub data: Box<[AtomicU32]>,
    }

    static RING: OnceLock<Arc<Ring>> = OnceLock::new();

    pub fn ring_layout() -> Option<Vec<u32>> {
        let r = RING.get()?;
        Some(vec![&r.write as *const AtomicU32 as u32, &r.read as *const AtomicU32 as u32, r.data.as_ptr() as u32, CAPACITY, RATE, TARGET, &r.max_block_us as *const AtomicU32 as u32, &r.max_gap_us as *const AtomicU32 as u32])
    }

    pub fn open_default() -> Result<(EngineHandle, DeviceStream)> {
        if RING.get().is_some() { return Err(AudioError::Device("the web audio ring is already open".into())); }
        let ring = Arc::new(Ring { write: AtomicU32::new(0), read: AtomicU32::new(0), max_block_us: AtomicU32::new(0), max_gap_us: AtomicU32::new(0), data: (0..CAPACITY * 2).map(|_| AtomicU32::new(0)).collect() });
        let (handle, mut mixer) = engine_pair(RATE);
        let r = ring.clone();
        std::thread::Builder::new()
            .name("fn-audio-mixer".into())
            .spawn(move || {
                let mut tmp = vec![0f32; (BLOCK * 2) as usize];
                let block = Duration::from_secs_f64(BLOCK as f64 / RATE as f64);
                let mut last_read = (r.read.load(Ordering::Acquire), Instant::now());
                let mut next_drop = Instant::now();
                let mut last_iter = Instant::now();
                let mut target = TARGET;
                let mut calm_since = Instant::now();
                loop {
                    let now_iter = Instant::now();
                    r.max_gap_us.fetch_max(now_iter.duration_since(last_iter).as_micros() as u32, Ordering::Relaxed);
                    last_iter = now_iter;
                    let w = r.write.load(Ordering::Relaxed);
                    let rd = r.read.load(Ordering::Acquire);
                    if rd != last_read.0 { last_read = (rd, Instant::now()); }
                    let fill = w.wrapping_sub(rd);
                    if fill < BLOCK && w > TARGET && last_read.1.elapsed() < Duration::from_millis(300) {
                        target = (target + 2048).min(CAPACITY - 2 * BLOCK);
                        calm_since = Instant::now();
                    } else if target > TARGET && calm_since.elapsed() > Duration::from_secs(5) {
                        target = (target - BLOCK).max(TARGET);
                        calm_since = Instant::now();
                    }
                    if fill + BLOCK <= target {
                        let t = Instant::now();
                        mixer.render(&mut tmp);
                        let us = t.elapsed().as_micros() as u32;
                        r.max_block_us.fetch_max(us, Ordering::Relaxed);
                        for f in 0..BLOCK {
                            let at = ((w.wrapping_add(f)) % CAPACITY) as usize * 2;
                            r.data[at].store(tmp[f as usize * 2].clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed);
                            r.data[at + 1].store(tmp[f as usize * 2 + 1].clamp(-1.0, 1.0).to_bits(), Ordering::Relaxed);
                        }
                        r.write.store(w.wrapping_add(BLOCK), Ordering::Release);
                    } else if last_read.1.elapsed() > Duration::from_millis(300) {
                        let now = Instant::now();
                        if now >= next_drop {
                            mixer.render(&mut tmp);
                            next_drop = now.max(next_drop) + block;
                        } else {
                            std::thread::sleep(next_drop - now);
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(3));
                    }
                }
            })
            .map_err(|e| AudioError::Device(format!("mixer thread: {e}")))?;
        let _ = RING.set(ring.clone());
        Ok((handle, DeviceStream { _ring: ring, device_name: "WebAudio AudioWorklet".into(), sample_rate: RATE, channels: 2 }))
    }
}
