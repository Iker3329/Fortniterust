//! The cooked shader permutations of one material package, with the ownership the earlier tooling left open ("pass / vertex-factory ownership unrecovered"):
//! every serialised `FShader` entry of a UE 4.16 `FMaterialShaderMap` starts with the 12-byte `FName shader type + u32 end offset` header
//! (`fn_dxbc::unreal_entry`) and contains the `FName` of its **vertex factory type** (`FMeshParticleVertexFactory`, `FLocalVertexFactory`, ...) before the zlib stream
//! of the bytecode. Both are read from the original bytes here, no permutation is chosen from a shader name alone: [`Cooked::find`] needs the (vertex factory, shader
//! type) pair.
use fn_dxbc::extract::{scan_blobs, serialized_stream, Blob};
use fn_dxbc::unreal_packet::{named_bindings, optional_fields, uniform_buffer_names, NamedBinding, ResourceTable};
use fn_dxbc::{Container, Shader};

/// One cooked program with everything the package says about it.
pub struct Program {
    /// Name of the serialised shader type, e.g. `TBasePassVSFNoLightMapPolicy`.
    pub shader_type: String,
    /// Vertex factory type FName found inside the entry (empty for global / material-only shaders).
    pub vertex_factory: String,
    /// zlib stream offset inside the `.uexp` (the id of the old `shader_<id>` reports).
    pub stream: usize,
    pub blob: Blob,
    pub shader: Shader,
    pub resource_table: ResourceTable,
    pub uniform_buffer_names: Option<Vec<String>>,
    pub bindings: Vec<NamedBinding>,
}

/// Every program of a material package.
pub struct Cooked {
    pub programs: Vec<Program>,
    pub names: Vec<String>,
}

impl Cooked {
    /// `data` = the `.uexp` bytes, `names` the package name table, `header` the total header size of the `.uasset` (the end offsets inside the entries are absolute: `header + position in uexp`).
    pub fn parse(data: &[u8], names: &[String], header: usize) -> Result<Cooked, String> {
        let blobs = scan_blobs(data);
        let mut programs = vec![];
        let mut search_from = 0usize;
        for blob in blobs {
            let shader = Shader::parse(&blob.bytes).map_err(|e| format!("shader stream {}: {e}", blob.id))?;
            let st = serialized_stream(data, &blob, &shader).map_err(|e| format!("shader stream {} inline record: {e}", blob.id))?;
            let entry = fn_dxbc::unreal_entry::named_entry(data, names, search_from, header, &st).map_err(|e| format!("shader stream {} named entry: {e}", blob.id))?;
            // vertex factory FName: the entry carries it twice, between its header and the inline stream header (20 bytes before the zlib stream); some entries
            // start with one more, unrelated `FLocalVertexFactory` name, so the LAST such name is the owner
            let mut vf = String::new();
            let lo = entry.offset + 12;
            let hi = st.start;
            let mut i = lo;
            while i + 8 <= hi {
                let idx = i32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                let num = i32::from_le_bytes(data[i + 4..i + 8].try_into().unwrap());
                if idx >= 0 && num == 0 {
                    if let Some(n) = names.get(idx as usize) { if n.ends_with("VertexFactory") { vf = n.clone(); } }
                }
                i += 1;
            }            search_from = st.tail.end;
            let resource_table = ResourceTable::parse(&blob.header).map_err(|e| format!("shader stream {} resource table: {e}", blob.id))?;
            let optional = optional_fields(&blob.trailer).map_err(|e| format!("shader stream {} optional fields: {e}", blob.id))?;
            let uniform_buffer_names = uniform_buffer_names(&optional)?;
            let bindings = named_bindings(&resource_table, uniform_buffer_names.as_deref(), &shader.decls).unwrap_or_default();
            programs.push(Program { shader_type: entry.name.clone(), vertex_factory: vf, stream: blob.id, blob, shader, resource_table, uniform_buffer_names, bindings });
        }
        Ok(Cooked { programs, names: names.to_vec() })
    }

    /// The SM5 program of a (vertex factory, shader type) pair; vertex / pixel stage is part of the type name. SM4 duplicates (`vs_4_0`) are skipped.
    pub fn find(&self, vertex_factory: &str, shader_type: &str) -> Option<&Program> {
        self.programs.iter().find(|p| p.vertex_factory == vertex_factory && p.shader_type == shader_type && p.shader.major >= 5)
    }
    pub fn list(&self) -> Vec<String> {
        self.programs.iter().map(|p| format!("{:>7} {} {:<62} {}", p.stream, p.shader.version_string(), p.shader_type, p.vertex_factory)).collect()
    }
}

/// The names the translated WGSL must expose for [`Program`] (see `fn_dxbc::codegen`): `name_main` is the stock entry point, which the FX path replaces by its own wrapper.
pub fn translated_function_wgsl(shader: &Shader, name: &str) -> Result<String, String> {
    let w = fn_dxbc::codegen::translate_wgsl(shader, name).map_err(|e| e.0)?;
    // keep everything up to the stock `struct <Name>Out { .. }` / entry point
    match w.find("\nstruct ") { Some(p) => Ok(w[..p + 1].to_string()), None => Ok(w) }
}

pub fn container_input_registers(bytes: &[u8]) -> Vec<(String, u32, u32, u8)> {
    // (semantic name, semantic index, register, mask) of the input signature
    Container::parse(bytes).ok().and_then(|c| c.input_signature()).map(|s| s.elements.iter().map(|e| (e.name.clone(), e.semantic_index, e.register, e.mask)).collect()).unwrap_or_default()
}
pub fn container_output_registers(bytes: &[u8]) -> Vec<(String, u32, u32, u8)> {
    Container::parse(bytes).ok().and_then(|c| c.output_signature()).map(|s| s.elements.iter().map(|e| (e.name.clone(), e.semantic_index, e.register, e.mask)).collect()).unwrap_or_default()
}


