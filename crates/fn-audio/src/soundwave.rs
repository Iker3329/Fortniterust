//! `USoundWave` (UE 4.16, cooked) parsing: tagged properties + the native tail.
//!
//! Native layout after the tagged property list and the 4-byte `bHasGuid` flag (verified on the whole 1.8 install,
//! see the crate README):
//!
//! ```text
//! i32   bCooked
//! i32   NumFormats                       (FFormatContainer)
//! NumFormats x { FName Name (i32 index, i32 number) ; FByteBulkData }
//! FGuid CompressedDataGuid               (16 bytes)
//! ```
//!
//! `FByteBulkData` = `u32 flags, i32 elementCount, i32 sizeOnDisk, i64 offsetInFile` followed by the payload when
//! the payload is inline. Placement is decided by the flags: `ForceInline (0x40)` -> right after the header,
//! `PayloadAtEndOfFile (0x01)` -> `offsetInFile + Summary.BulkDataStartOffset` in `.uasset ++ .uexp`,
//! `PayloadInSeperateFile (0x100)` -> `offsetInFile + Summary.BulkDataStartOffset` in the `.ubulk` file\n//! (the stored offset is negative; the start offset equals the size of `.uasset ++ .uexp`).

use crate::error::{malformed, Result};
use fn_uasset::{Package, Reader, Value};

pub const BULK_AT_END: u32 = 0x01;
pub const BULK_ZLIB: u32 = 0x02;
pub const BULK_SINGLE_USE: u32 = 0x08;
pub const BULK_UNUSED: u32 = 0x20;
pub const BULK_FORCE_INLINE: u32 = 0x40;
pub const BULK_SEPARATE: u32 = 0x100;
pub const BULK_OPTIONAL: u32 = 0x800;

/// Where a bulk payload physically lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Placement {
    /// Immediately after the bulk header inside the export (`.uexp`).
    Inline,
    /// `OffsetInFile + BulkDataStartOffset` in `.uasset ++ .uexp`.
    EndOfFile,
    /// `OffsetInFile + BulkDataStartOffset` in the `.ubulk` file.
    UbulkFile,
    Unused,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BulkHeader {
    pub flags: u32,
    pub element_count: i32,
    pub size_on_disk: i32,
    pub offset_in_file: i64,
}

impl BulkHeader {
    pub fn read(r: &mut Reader) -> Result<BulkHeader> {
        Ok(BulkHeader { flags: r.u32()?, element_count: r.i32()?, size_on_disk: r.i32()?, offset_in_file: r.i64()? })
    }
    pub fn placement(&self) -> Placement {
        if self.flags & BULK_UNUSED != 0 || self.size_on_disk <= 0 || self.flags & BULK_OPTIONAL != 0 {
            // OptionalPayload (0x800) lives in a `.uptnl` file, which is not read; none exists in the 1.8 install
            Placement::Unused
        } else if self.flags & BULK_SEPARATE != 0 {
            Placement::UbulkFile
        } else if self.flags & BULK_AT_END != 0 {
            Placement::EndOfFile
        } else {
            Placement::Inline
        }
    }
}

/// One entry of the compressed-format container.
#[derive(Clone, Debug)]
pub struct FormatBlock {
    /// `OGG`, `ADPCM`, `PCM`, `OPUS`, ... (the FName).
    pub name: String,
    pub bulk: BulkHeader,
    pub placement: Placement,
    /// The payload bytes (copied out of `.uexp` / `.ubulk`); empty when unused.
    pub payload: Vec<u8>,
}

/// Tagged properties of a `USoundWave` that matter for playback.
#[derive(Clone, Debug, Default)]
pub struct SoundWaveInfo {
    pub name: String,
    pub duration: Option<f64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
    pub raw_pcm_data_size: Option<u32>,
    pub looping: bool,
    pub streaming: bool,
    pub volume: Option<f32>,
    pub pitch: Option<f32>,
    pub compression_quality: Option<i32>,
    pub sound_group: Option<String>,
    /// `SoundClassObject` (object path) when set.
    pub sound_class: Option<String>,
    /// `AttenuationSettings` object path when set.
    pub attenuation: Option<String>,
    pub override_attenuation: bool,
    pub virtualize_when_silent: bool,
    pub total_samples: Option<u32>,
}

/// One `FStreamedAudioChunk`: `i32 bCooked, FByteBulkData, i32 DataSize, i32 AudioDataSize`.
#[derive(Clone, Debug)]
pub struct StreamChunk {
    pub bulk: BulkHeader,
    pub placement: Placement,
    /// Allocated chunk size (256 KiB for all but possibly the last chunk).
    pub data_size: i32,
    /// Bytes of the chunk that carry audio (the rest of `payload` is zero padding).
    pub audio_data_size: i32,
    pub payload: Vec<u8>,
}

/// `FStreamedAudioPlatformData`: `i32 NumChunks, FName AudioDataFormat, chunks...`.
#[derive(Clone, Debug)]
pub struct StreamedAudio {
    pub format: String,
    pub chunks: Vec<StreamChunk>,
}

impl StreamedAudio {
    /// The elementary stream: every chunk cut to its `AudioDataSize`, concatenated.
    pub fn stream_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for c in &self.chunks {
            let n = (c.audio_data_size.max(0) as usize).min(c.payload.len());
            out.extend_from_slice(&c.payload[..n]);
        }
        out
    }
}

#[derive(Clone, Debug, Default)]
pub struct SoundWaveAsset {
    pub info: SoundWaveInfo,
    pub cooked: bool,
    pub formats: Vec<FormatBlock>,
    /// Streaming waves (`bStreaming`): the chunked platform data instead of `formats`.
    pub streamed: Option<StreamedAudio>,
    pub guid: Option<[u8; 16]>,
    /// Bytes left in the export after the guid (0 in all 1.8 content; non-zero means an unmodelled tail).
    pub leftover: usize,
    /// Set when the native tail could not be read to the end (formats read so far are kept).
    pub tail_error: Option<String>,
}

fn obj_path(pkg: &Package, v: &Value) -> Option<String> {
    let i = v.as_object()?;
    if i.is_null() {
        None
    } else {
        Some(pkg.object_path(i))
    }
}

pub fn read_info(pkg: &Package, props: &fn_uasset::PropertyList, name: &str) -> SoundWaveInfo {
    let g = |n: &str| props.get(n);
    SoundWaveInfo {
        name: name.to_string(),
        duration: g("Duration").and_then(Value::as_f64),
        sample_rate: g("SampleRate").and_then(Value::as_i64).and_then(|v| u32::try_from(v).ok()),
        channels: g("NumChannels").and_then(Value::as_i64).and_then(|v| u16::try_from(v).ok()),
        raw_pcm_data_size: g("RawPCMDataSize").and_then(Value::as_i64).and_then(|v| u32::try_from(v).ok()),
        looping: g("bLooping").and_then(Value::as_bool).unwrap_or(false),
        streaming: g("bStreaming").and_then(Value::as_bool).unwrap_or(false),
        volume: g("Volume").and_then(Value::as_f32),
        pitch: g("Pitch").and_then(Value::as_f32),
        compression_quality: g("CompressionQuality").and_then(Value::as_i64).map(|v| v as i32),
        sound_group: g("SoundGroup").and_then(Value::as_str).map(str::to_string),
        sound_class: g("SoundClassObject").and_then(|v| obj_path(pkg, v)),
        attenuation: g("AttenuationSettings").and_then(|v| obj_path(pkg, v)),
        override_attenuation: g("bOverrideAttenuation").and_then(Value::as_bool).unwrap_or(false),
        virtualize_when_silent: g("bVirtualizeWhenSilent").and_then(Value::as_bool).unwrap_or(false),
        total_samples: g("TotalSamples").and_then(Value::as_i64).and_then(|v| u32::try_from(v).ok()),
    }
}

/// Parse export `export` (a `SoundWave`) of `pkg`. `ubulk` is the package's `.ubulk` bytes (empty when absent).
pub fn parse_soundwave(pkg: &Package, export: usize, ubulk: &[u8]) -> Result<SoundWaveAsset> {
    let obj = pkg.read_export(export)?;
    if let Some(e) = &obj.error {
        return malformed("SoundWave properties", e.to_string());
    }
    let info = read_info(pkg, &obj.props, &obj.name);
    let mut asset = SoundWaveAsset { info, ..Default::default() };
    let mut r = Reader::new(obj.trailing);
    if let Err(e) = read_tail(pkg, &mut r, ubulk, &mut asset) {
        asset.tail_error = Some(e.to_string());
    }
    asset.leftover = r.remaining();
    Ok(asset)
}

fn resolve_bulk(pkg: &Package, r: &mut Reader, ubulk: &[u8], bulk: &BulkHeader) -> Result<Vec<u8>> {
    let size = bulk.size_on_disk.max(0) as usize;
    Ok(match bulk.placement() {
        Placement::Unused => Vec::new(),
        Placement::Inline => r.bytes(size)?.to_vec(),
        Placement::EndOfFile => {
            let abs = bulk.offset_in_file + pkg.summary.bulk_data_start_offset;
            pkg.range_bytes(abs, size as i64)?.to_vec()
        }
        Placement::UbulkFile => {
            let abs = bulk.offset_in_file + pkg.summary.bulk_data_start_offset;
            if abs < 0 {
                return malformed("SoundWave .ubulk payload", format!("negative offset {abs}"));
            }
            let o = abs as usize;
            match o.checked_add(size).and_then(|e| ubulk.get(o..e)) {
                Some(s) => s.to_vec(),
                None => return malformed("SoundWave .ubulk payload", format!("{o}+{size} beyond {}", ubulk.len())),
            }
        }
    })
}

fn read_tail(pkg: &Package, r: &mut Reader, ubulk: &[u8], a: &mut SoundWaveAsset) -> Result<()> {
    a.cooked = r.i32()? != 0;
    if !a.cooked {
        return Ok(());
    }
    if a.info.streaming {
        // bStreaming: guid first, then the streamed platform data.
        a.guid = Some(r.guid()?);
        let n = r.i32()?;
        if !(0..=4096).contains(&n) {
            return malformed("SoundWave NumChunks", format!("{n}"));
        }
        let format = pkg.fname(r.i32()?, r.i32()?)?;
        let mut chunks = Vec::new();
        for _ in 0..n {
            let _chunk_cooked = r.i32()?;
            let bulk = BulkHeader::read(r)?;
            let payload = resolve_bulk(pkg, r, ubulk, &bulk)?;
            let data_size = r.i32()?;
            let audio_data_size = r.i32()?;
            chunks.push(StreamChunk { bulk, placement: bulk.placement(), data_size, audio_data_size, payload });
        }
        a.streamed = Some(StreamedAudio { format, chunks });
        return Ok(());
    }
    let n = r.i32()?;
    if !(0..=16).contains(&n) {
        return malformed("SoundWave NumFormats", format!("{n}"));
    }
    for _ in 0..n {
        let name = pkg.fname(r.i32()?, r.i32()?)?;
        let bulk = BulkHeader::read(r)?;
        let payload = resolve_bulk(pkg, r, ubulk, &bulk)?;
        a.formats.push(FormatBlock { name, bulk, placement: bulk.placement(), payload });
    }
    a.guid = Some(r.guid()?);
    Ok(())
}
impl SoundWaveAsset {
    /// Name of the audio codec (`OGG`, `OPUS`, ...).
    pub fn format_name(&self) -> Option<&str> {
        self.streamed.as_ref().map(|s| s.format.as_str()).or_else(|| self.primary().map(|f| f.name.as_str()))
    }
    /// First non-empty format block.
    pub fn primary(&self) -> Option<&FormatBlock> {
        self.formats.iter().find(|f| !f.payload.is_empty())
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_flag_values_match_fn_ueio() {
        assert_eq!(BULK_AT_END, fn_ueio::BULK_AT_END);
        assert_eq!(BULK_ZLIB, fn_ueio::BULK_ZLIB);
        assert_eq!(BULK_UNUSED, fn_ueio::BULK_UNUSED);
        assert_eq!(BULK_FORCE_INLINE, fn_ueio::BULK_FORCE_INLINE);
        assert_eq!(BULK_SEPARATE, fn_ueio::BULK_SEPARATE);
    }

    #[test]
    fn placements_of_the_flag_sets_seen_in_the_install() {
        let h = |flags| BulkHeader { flags, element_count: 1, size_on_disk: 10, offset_in_file: 0 };
        assert_eq!(h(0x48).placement(), Placement::Inline); // OGG: SingleUse | ForceInline
        assert_eq!(h(0x40).placement(), Placement::Inline); // Opus chunk 0
        assert_eq!(h(0x501).placement(), Placement::UbulkFile); // Opus chunks 1..: AtEnd | Separate | Force_NOT_Inline
        assert_eq!(h(0x1).placement(), Placement::EndOfFile);
        assert_eq!(h(0x20).placement(), Placement::Unused);
        assert_eq!(BulkHeader { flags: 0x40, element_count: 0, size_on_disk: 0, offset_in_file: 0 }.placement(), Placement::Unused);
    }
}

