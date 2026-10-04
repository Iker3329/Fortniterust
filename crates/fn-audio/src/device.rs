//! cpal (WASAPI on Windows) output. Compiled with the `device-output` feature.
//!
//! `open_default()` opens the default output device with its default config, builds a [`Mixer`] at the device sample
//! rate and runs it inside the cpal callback. The callback only drains the lock-free command queue and renders; it
//! never blocks or locks (waves/cues arrive as `Arc`s through the same queue).

use crate::error::{AudioError, Result};
use crate::mixer::{engine_pair, EngineHandle, Mixer};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

/// Keeps the output stream alive; dropping it stops audio. (cpal streams are not `Send` on every platform: keep this
/// on the thread that created it.)
pub struct DeviceStream {
    _stream: cpal::Stream,
    pub device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

fn dev_err(e: impl std::fmt::Display) -> AudioError {
    AudioError::Device(e.to_string())
}

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
