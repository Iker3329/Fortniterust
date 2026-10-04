//! Whole-install sweep: parse and decode every SoundWave, tabulate formats / placement / rates and compare the
//! decoded length against the asset's own `Duration`, `SampleRate`, `NumChannels` properties.

use crate::codec::decode_asset;
use crate::install::{read_ubulk, Install};
use crate::soundwave::{parse_soundwave, Placement};
use fn_uasset::PackageData;
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct WaveRow {
    pub path: String,
    pub name: String,
    pub format: String,
    pub streaming: bool,
    pub placements: Vec<Placement>,
    pub chunks: usize,
    pub payload_bytes: usize,
    pub prop_rate: Option<u32>,
    pub prop_channels: Option<u16>,
    pub prop_duration: Option<f64>,
    pub dec_rate: u32,
    pub dec_channels: u16,
    pub dec_frames: usize,
    pub decoded: bool,
    pub error: Option<String>,
    /// Frame difference vs `Duration * SampleRate` (decoded - expected).
    pub frame_delta: Option<i64>,
    pub duration_ok: bool,
    pub rate_ok: bool,
    pub channels_ok: bool,
    pub non_finite: usize,
    pub peak: f32,
    pub rms: f32,
    pub leftover: usize,
    pub tail_error: Option<String>,
    pub looping: bool,
    /// Loop-seam step / typical step (looping waves only; see [`crate::analysis::loop_seam_ratio`]).
    pub seam_ratio: Option<f32>,
}

/// Tolerance of the duration check: one frame or 0.5 %.
pub fn duration_matches(decoded_frames: usize, rate: u32, duration: f64) -> (i64, bool) {
    let expected = duration * rate as f64;
    let delta = decoded_frames as i64 - expected.round() as i64;
    let tol = (expected * 0.005).max(1.0);
    (delta, (decoded_frames as f64 - expected).abs() <= tol)
}

pub fn sweep_paths(install: &Install, paths: &[String]) -> Vec<WaveRow> {
    paths
        .par_iter()
        .flat_map(|p| {
            let mut rows = Vec::new();
            let Ok(data) = PackageData::load(&install.paks, p) else { return rows };
            let Ok(pkg) = data.package() else { return rows };
            let idxs: Vec<usize> = pkg.exports_of_class("SoundWave").collect();
            if idxs.is_empty() {
                return rows;
            }
            let ub = read_ubulk(&install.paks, p);
            for i in idxs {
                let mut row = WaveRow { path: p.clone(), ..Default::default() };
                match parse_soundwave(&pkg, i, &ub) {
                    Err(e) => row.error = Some(format!("parse: {e}")),
                    Ok(a) => {
                        row.name = a.info.name.clone();
                        row.format = a.format_name().unwrap_or("<none>").to_string();
                        row.streaming = a.info.streaming;
                        row.looping = a.info.looping;
                        row.prop_rate = a.info.sample_rate;
                        row.prop_channels = a.info.channels;
                        row.prop_duration = a.info.duration;
                        row.leftover = a.leftover;
                        row.tail_error = a.tail_error.clone();
                        if let Some(s) = &a.streamed {
                            row.chunks = s.chunks.len();
                            row.placements = s.chunks.iter().map(|c| c.placement).collect();
                            row.payload_bytes = s.chunks.iter().map(|c| c.audio_data_size.max(0) as usize).sum();
                        } else {
                            row.chunks = a.formats.len();
                            row.placements = a.formats.iter().map(|f| f.placement).collect();
                            row.payload_bytes = a.formats.iter().map(|f| f.payload.len()).sum();
                        }
                        if a.tail_error.is_some() || a.leftover != 0 {
                            row.error = Some(format!("tail: {:?} leftover {}", a.tail_error, a.leftover));
                        } else {
                            match decode_asset(&a) {
                                Err(e) => row.error = Some(format!("decode: {e}")),
                                Ok(w) => {
                                    row.decoded = true;
                                    row.dec_rate = w.sample_rate;
                                    row.dec_channels = w.channels;
                                    row.dec_frames = w.frames();
                                    let st = w.stats();
                                    row.non_finite = st.non_finite;
                                    row.peak = st.peak;
                                    if row.looping {
                                        row.seam_ratio = crate::analysis::loop_seam_ratio(&w);
                                    }
                                    row.rms = st.rms;
                                    // UE4OPUS streams are re-sampled to an Opus rate (32/44.1 kHz sources become 48 kHz); the duration check
                                    // below is time-based, so only non-Opus waves must keep the property rate.
                                    row.rate_ok = row.format == "OPUS" || row.prop_rate.map_or(true, |r| r == w.sample_rate);
                                    row.channels_ok = row.prop_channels.map_or(true, |c| c == w.channels);
                                    match row.prop_duration {
                                        Some(d) => {
                                            let (delta, ok) = duration_matches(w.frames(), w.sample_rate, d);
                                            row.frame_delta = Some(delta);
                                            row.duration_ok = ok;
                                        }
                                        None => row.duration_ok = true,
                                    }
                                }
                            }
                        }
                    }
                }
                rows.push(row);
            }
            rows
        })
        .collect()
}

/// Sweep every package of the install.
pub fn sweep_install(install: &Install) -> Vec<WaveRow> {
    sweep_paths(install, &install.all_asset_paths())
}

fn bump(m: &mut BTreeMap<String, usize>, k: impl Into<String>) {
    *m.entry(k.into()).or_default() += 1;
}

/// JSON summary: counts by format / placement / rate / channels, decode success, duration agreement, failures.
pub fn summarize(rows: &[WaveRow]) -> Value {
    let (mut fmt, mut place, mut rate, mut ch, mut errs) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
    let mut dec_rates: BTreeMap<String, usize> = BTreeMap::new();
    let mut shortest = f64::MAX;
    let (mut ok, mut dur_ok, mut rate_ok, mut ch_ok, mut bad_num) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut total_secs, mut max_dur, mut streaming) = (0f64, 0f64, 0usize);
    let mut worst: Vec<(i64, &WaveRow)> = Vec::new();
    let mut seams: Vec<f32> = Vec::new();
    let (mut exact, mut within1, mut looping) = (0usize, 0usize, 0usize);
    let mut peak_over_1 = 0usize;
    for r in rows {
        bump(&mut fmt, format!("{}{}", r.format, if r.streaming { " (streaming)" } else { "" }));
        for p in &r.placements {
            bump(&mut place, format!("{:?}", p));
        }
        if let Some(sr) = r.prop_rate {
            bump(&mut rate, sr.to_string());
        }
        if let Some(c) = r.prop_channels {
            bump(&mut ch, c.to_string());
        }
        if r.streaming {
            streaming += 1;
        }
        if r.decoded {
            ok += 1;
            dur_ok += r.duration_ok as usize;
            rate_ok += r.rate_ok as usize;
            ch_ok += r.channels_ok as usize;
            bad_num += (r.non_finite > 0) as usize;
            let secs = r.dec_frames as f64 / r.dec_rate.max(1) as f64;
            total_secs += secs;
            looping += r.looping as usize;
            if let Some(d) = r.frame_delta {
                exact += (d == 0) as usize;
                within1 += (d.abs() <= 1) as usize;
            }
            peak_over_1 += (r.peak > 1.0) as usize;
            if let Some(sr) = r.seam_ratio {
                seams.push(sr);
            }
            max_dur = max_dur.max(secs);
            shortest = shortest.min(secs);
            bump(&mut dec_rates, format!("{} Hz {}", r.dec_rate, r.format));
            if !r.duration_ok {
                worst.push((r.frame_delta.unwrap_or(0).abs(), r));
            }
        } else if let Some(e) = &r.error {
            let key: String = e.chars().take(120).collect();
            bump(&mut errs, key);
        }
    }
    worst.sort_by(|a, b| b.0.cmp(&a.0));
    seams.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let seam_median = seams.get(seams.len() / 2).copied();
    let seam_le3 = seams.iter().filter(|&&s| s <= 3.0).count();
    let n = rows.len().max(1);
    json!({
        "soundwaves": rows.len(),
        "streaming": streaming,
        "decoded": ok,
        "decode_success_percent": 100.0 * ok as f64 / n as f64,
        "duration_within_tolerance": dur_ok,
        "sample_rate_matches_property": rate_ok,
        "channels_match_property": ch_ok,
        "waves_with_non_finite_samples": bad_num,
        "decoder_panics_caught": crate::codec::panics_caught(),
        "frame_delta_exact": exact,
        "frame_delta_within_1": within1,
        "waves_with_peak_over_full_scale": peak_over_1,
        "looping_waves": looping,
        "looping_waves_seam_ratio_median": seam_median,
        "looping_waves_seam_ratio_le_3": seam_le3,
        "total_decoded_seconds": total_secs,
        "longest_decoded_seconds": max_dur,
        "shortest_decoded_seconds": if shortest == f64::MAX { 0.0 } else { shortest },
        "decoded_sample_rates_by_format": dec_rates,
        "formats": fmt, "bulk_placement": place, "sample_rates_property": rate, "channels_property": ch,
        "failures": errs,
        "worst_duration_mismatches": worst.iter().take(10).map(|(d, r)| json!({"path": r.path, "frame_delta": d, "dec_frames": r.dec_frames, "prop_duration": r.prop_duration, "rate": r.dec_rate})).collect::<Vec<_>>(),
    })
}




