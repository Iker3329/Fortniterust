//! Original material dependencies, without heuristic shader-slot assignment.
//! Keep the owning package: property object indices and cooked shader bytes
//! must not lose their context when the mesh's material references are loaded.
use crate::{loader, ClipSourceDir};
use fn_ueio::{Package, Reader, props::{read_props, Ctx, PropList, PropValue}};
use std::collections::{HashMap,HashSet};

pub struct Material {
    pub package: Package,
    pub properties: PropList,
    pub class: String,
    pub parent: Option<String>,
    /// Direct overrides only; NOT guessed effective inherited parameters.
    pub texture_parameters: HashMap<String,Option<String>>,
    pub imported_textures: Vec<String>,
    /// All original programs in this exact material export. No fabricated
    /// shader permutation choice; program/pass/vertex-factory mapping pending.
    pub shaders: Vec<CookedShader>,
}
pub struct CookedShader {
    /// ID is relative to the start of the owning material export, not a global
    /// shader name or the CLI's uexp-relative stream offset.
    pub blob: fn_dxbc::extract::Blob,
    pub program: fn_dxbc::Shader,
    pub program_hash: u64,
    pub resource_table: fn_dxbc::unreal_packet::ResourceTable,
    pub optional_fields: Vec<fn_dxbc::unreal_packet::OptionalField>,
    pub uniform_buffer_names: Option<Vec<String>>,
    pub resource_bindings: Vec<fn_dxbc::unreal_packet::NamedBinding>,
    pub serialized_stream: fn_dxbc::extract::SerializedStream,
    pub named_entry: fn_dxbc::unreal_entry::NamedEntry,
}
pub struct Texture {
    /// Preserve SRGB, sampler addressing, mip and compression metadata.
    pub source: fn_texture::Texture2D,
    /// Largest available original mip, raw colour values (no gamma conversion).
    pub image: fn_texture::DecodedImage,
}
/// One original material-parameter texture (full object path).
pub fn load_texture(src:&ClipSourceDir,path:&str)->Result<Texture,String> {
    let source=loader::texture_export(src,path)?;
    if source.num_slices!=1 {return Err(format!("{path}: unsupported non-2D material parameter"));}
    let mip=source.best_mip().ok_or_else(||format!("{path}: original texture has no available mip"))?;
    let image=source.decode(mip,fn_texture::DecodeOptions::default()).map_err(|e|format!("{path}: {e}"))?;
    Ok(Texture{source,image})
}
pub fn load_parameter_textures(src:&ClipSourceDir,materials:&HashMap<String,Material>)->Result<HashMap<String,Texture>,String> {
    let mut textures=HashMap::new();
    for path in materials.values().flat_map(|m|m.texture_parameters.values().flatten()) {
        if textures.contains_key(path){continue;}
        textures.insert(path.clone(),load_texture(src,path)?);
    }
    Ok(textures)
}
fn external(pkg:&Package,index:i32)->Result<Option<String>,String> {
    if index==0{return Ok(None);}
    if index>0{return Err("unsupported internal material object reference".into());}
    let path=pkg.import_path(index);
    if !path.starts_with("/Game/"){return Err(format!("unsupported material dependency {path}"));}
    Ok(Some(path))
}
impl Material {
    pub fn load(src:&ClipSourceDir,path:&str)->Result<Self,String> {
        let (long,object)=path.split_once('.').ok_or("material requires full object path")?;
        let pkg=loader::package(src,long)?;
        let idx=pkg.exports.iter().position(|e|e.object_name==object).ok_or_else(||format!("{path}: missing exact material export"))?;
        let class=pkg.export_class(idx);
        if !matches!(class.as_str(),"Material"|"MaterialInstanceConstant"|"MaterialInstance") {return Err(format!("{path}: unsupported material class {class}"));}
        let (start,end)=pkg.export_range(idx);
        let properties=read_props(&mut Reader::at(&pkg.data[..end],start),&Ctx{names:&pkg.names,file_version:pkg.summary.file_version}).map_err(|e|format!("{path}: {e}"))?;
        let parent=match properties.get("Parent") {None=>None,Some(PropValue::Object(i))=>external(&pkg,*i)?,_=>return Err("invalid material parent property".into())};
        let mut texture_parameters=HashMap::new();
        if let Some(values)=properties.array("TextureParameterValues") {
            for value in values {
                let fields=value.as_fields().ok_or("invalid texture parameter struct")?;
                let name=fields.text("ParameterName").or_else(||fields.fields("ParameterInfo").and_then(|p|p.text("Name"))).filter(|s|!s.is_empty()).ok_or("missing original texture parameter name")?;
                let index=fields.object("ParameterValue").ok_or("missing original texture parameter value")?;
                if texture_parameters.insert(name.to_owned(),external(&pkg,index)?).is_some(){return Err(format!("duplicate material texture parameter {name}"));}
            }
        }
        let imported_textures=pkg.imports.iter().enumerate().filter(|(_,i)|i.class_name=="Texture2D").map(|(n,_)|pkg.import_path(-(n as i32)-1)).collect();
        let mut shaders=vec![];
        let mut entry_search_start=0;
        for blob in fn_dxbc::extract::scan_blobs(&pkg.data[start..end]) {
            let program=fn_dxbc::Shader::parse(&blob.bytes).map_err(|e|format!("{path}: original shader stream {}: {e}",blob.id))?;
            let program_hash=fn_dxbc::extract::program_hash(&blob.bytes);
            let resource_table=fn_dxbc::unreal_packet::ResourceTable::parse(&blob.header).map_err(|e|format!("{path}: shader {} resource table: {e}",blob.id))?;
            let optional_fields=fn_dxbc::unreal_packet::optional_fields(&blob.trailer).map_err(|e|format!("{path}: shader {} optional fields: {e}",blob.id))?;
            let uniform_buffer_names=fn_dxbc::unreal_packet::uniform_buffer_names(&optional_fields)?;
            let resource_bindings=fn_dxbc::unreal_packet::named_bindings(&resource_table,uniform_buffer_names.as_deref(),&program.decls).map_err(|e|format!("{path}: shader {} bindings: {e}",blob.id))?;
            let serialized_stream=fn_dxbc::extract::serialized_stream(&pkg.data[start..end],&blob,&program).map_err(|e|format!("{path}: shader {} inline record: {e}",blob.id))?;
            let named_entry=fn_dxbc::unreal_entry::named_entry(&pkg.data[start..end],&pkg.names,entry_search_start,start,&serialized_stream).map_err(|e|format!("{path}: shader {} named entry: {e}",blob.id))?;
            entry_search_start=serialized_stream.tail.end;
            shaders.push(CookedShader{blob,program,program_hash,resource_table,optional_fields,uniform_buffer_names,resource_bindings,serialized_stream,named_entry});
        }
        Ok(Self{package:pkg,properties,class,parent,texture_parameters,imported_textures,shaders})
    }
}

/// Preserve every parent material, including its cooked native payload. Cycles
/// are rejected, not quietly treated as absent/default material parameters.
pub fn load_closure(src:&ClipSourceDir,roots:impl IntoIterator<Item=String>)->Result<HashMap<String,Material>,String> {
    let mut materials=HashMap::<String,Material>::new();
    for root in roots {
        let mut path=Some(root);let mut chain=HashSet::new();
        while let Some(current)=path {
            if !chain.insert(current.clone()){return Err(format!("original material parent cycle: {current}"));}
            if chain.len()>64{return Err("material parent depth limit".into());}
            if let Some(existing)=materials.get(&current) {path=existing.parent.clone();continue;}
            let material=Material::load(src,&current)?;
            path=material.parent.clone();
            materials.insert(current,material);
        }
    }
    Ok(materials)
}
