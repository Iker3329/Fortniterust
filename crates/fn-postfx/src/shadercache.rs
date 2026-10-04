//! Reader for the UE 4.16 `GlobalShaderCache-PCD3D_SM5.bin` (Fortnite 1.8): finds every zlib-compressed DXBC blob and names it with the
//! shader type string that precedes it in the file (`FPostProcessMotionBlurPS3`, `FPostProcessDOFSetupPS12`, ...).
//!
//! The cache is `FShaderCache`-style serialized global shader maps; the shader type names are plain ASCII (`FString`-like, not in a
//! name table), the DXBC is `FShaderResource::Code` = `zlib(stream)`. We do not parse the container, we scan: that is exactly what the old
//! `tools/sky-shader-extract.cjs` did (ids = byte offset of the zlib stream), plus the name assignment.

/// One compiled shader found in the cache.
#[derive(Clone, Debug)]
pub struct CachedShader {
    /// Byte offset of the zlib stream in the cache file (the id used by the old `reports/tonemap-shaders/`).
    pub id: usize,
    /// Shader type name preceding the blob (parameter-struct names are skipped), e.g. `FPostProcessMotionBlurPS3`.
    pub name: String,
    /// `"vs" | "ps" | "cs" | ...` and shader model from the DXBC `SHDR`/`SHEX` version token, e.g. `ps_5_0`.
    pub profile: String,
    /// The DXBC container (`DXBC` magic included), UE strips/zeroes the hash so it is NOT re-signed here.
    pub dxbc: Vec<u8>,
}

fn is_name_char(b: u8) -> bool { b.is_ascii_alphanumeric() || matches!(b, b'_' | b'<' | b'>' | b',' | b':') }

/// All plausible shader-type-name strings `(offset, text)` in the file.
pub fn scan_names(bytes: &[u8]) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() && (i == 0 || !is_name_char(bytes[i - 1])) {
            let mut j = i;
            while j < bytes.len() && is_name_char(bytes[j]) { j += 1; }
            if j - i >= 8 && matches!(bytes[i], b'F' | b'T') && bytes[i + 1].is_ascii_uppercase() {
                out.push((i, String::from_utf8_lossy(&bytes[i..j]).into_owned()));
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// Shader type names look like `...PS`, `...VS<false>`, `...CS001`, `...PixelShader...`; the uniform-buffer struct names that follow them
/// in the file (`FViewUniformShaderParameters`, `FGBufferResourceStruct`, `FDrawRectangleParameters`, `FAOSampleData2`, ...) do not.
fn is_shader_type_name(n: &str) -> bool {
    let has_stage = ["PS", "VS", "CS", "GS", "HS", "DS", "PixelShader", "VertexShader", "ComputeShader"].iter().any(|s| n.contains(s));
    let is_params = ["Parameters", "Struct", "UB", "Data", "UniformShader"].iter().any(|s| n.contains(s));
    has_stage && !is_params
}

fn dxbc_profile(blob: &[u8]) -> String {
    // chunk table: magic(4) hash(16) version(4) size(4) count(4) offsets[count]
    if blob.len() < 32 || &blob[0..4] != b"DXBC" { return "?".into(); }
    let count = u32::from_le_bytes(blob[28..32].try_into().unwrap()) as usize;
    for c in 0..count.min(32) {
        let o = u32::from_le_bytes(blob[32 + 4 * c..36 + 4 * c].try_into().unwrap()) as usize;
        if o + 12 > blob.len() { continue; }
        let tag = &blob[o..o + 4];
        if tag == b"SHDR" || tag == b"SHEX" {
            let v = u32::from_le_bytes(blob[o + 8..o + 12].try_into().unwrap());
            let kind = match v >> 16 { 0 => "ps", 1 => "vs", 2 => "gs", 3 => "hs", 4 => "ds", 5 => "cs", _ => "?" };
            return format!("{kind}_{}_{}", (v >> 4) & 0xf, v & 0xf);
        }
    }
    "?".into()
}

fn inflate_at(bytes: &[u8], at: usize) -> Option<Vec<u8>> {
    use miniz_oxide::inflate::core::{decompress, inflate_flags, DecompressorOxide};
    use miniz_oxide::inflate::TINFLStatus;
    let mut st = DecompressorOxide::new();
    let mut out = vec![0u8; 1 << 16];
    let mut in_pos = at;
    let mut out_pos = 0usize;
    loop {
        let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
        let (status, ic, oc) = decompress(&mut st, &bytes[in_pos..], &mut out, out_pos, flags);
        in_pos += ic;
        out_pos += oc;
        match status {
            TINFLStatus::Done => { out.truncate(out_pos); return Some(out); }
            TINFLStatus::HasMoreOutput => { out.resize(out.len() * 2, 0); }
            _ => return None,
        }
    }
}

/// Extracts and names every DXBC blob (`78 9C` zlib streams that contain a `DXBC` container).
pub fn extract(bytes: &[u8]) -> Vec<CachedShader> {
    let names: Vec<(usize, String)> = scan_names(bytes).into_iter().filter(|(_, n)| is_shader_type_name(n)).collect();
    let mut shaders = Vec::new();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == 0x78 && bytes[i + 1] == 0x9c {
            if let Some(raw) = inflate_at(bytes, i) {
                if let Some(o) = raw.windows(4).position(|w| w == b"DXBC") {
                    if raw.len() >= o + 28 {
                        let size = u32::from_le_bytes(raw[o + 24..o + 28].try_into().unwrap()) as usize;
                        if o + size <= raw.len() {
                            let blob = raw[o..o + size].to_vec();
                            let name = match names.binary_search_by(|(off, _)| off.cmp(&i)) {
                                Ok(k) | Err(k) => names[..k].last().map(|(_, n)| n.clone()).unwrap_or_default(),
                            };
                            let profile = dxbc_profile(&blob);
                            shaders.push(CachedShader { id: i, name, profile, dxbc: blob });
                        }
                    }
                }
            }
        }
        i += 1;
    }
    shaders
}
