//! Byte observations near shader streams, not structural record ownership.
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct NameCandidate{pub offset:usize,pub name:String}
/// FName + absolute end-offset byte pattern. Structural ownership is still
/// separate from finding a matching offset in a bounded region.
pub fn end_offset_candidates(bytes:&[u8],names:&[String],start:usize,end:usize,absolute_end:u64)->Result<Vec<NameCandidate>,String>{
    Ok(fn_dxbc::unreal_entry::candidates(bytes,names,start,end,absolute_end)?.into_iter().map(|e|NameCandidate{offset:e.offset,name:e.name}).collect())
}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct UniformParameter{pub type_name:String,pub slot:u16,pub bound:bool}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct UniformParameterTable{pub start:usize,pub end:usize,pub parameters:Vec<UniformParameter>}
impl UniformParameterTable{
    pub fn encode(&self)->Vec<u8>{let mut b=(self.parameters.len() as u32).to_le_bytes().to_vec();for p in &self.parameters{b.extend(((p.type_name.len()+1) as u32).to_le_bytes());b.extend(p.type_name.as_bytes());b.push(0);b.extend(p.slot.to_le_bytes());b.extend((p.bound as u32).to_le_bytes());}b}
}
fn read_table(region:&[u8],start:usize)->Option<UniformParameterTable>{
    let mut at=0;
    let take=|at:&mut usize,n:usize|->Option<&[u8]>{let end=at.checked_add(n)?;let b=region.get(*at..end)?;*at=end;Some(b)};
    let count=u32::from_le_bytes(take(&mut at,4)?.try_into().ok()?) as usize;
    if !(1..=32).contains(&count){return None;}
    let mut parameters=vec![];
    for _ in 0..count{
        let n=u32::from_le_bytes(take(&mut at,4)?.try_into().ok()?) as usize;if !(3..=256).contains(&n){return None;}
        let bytes=take(&mut at,n)?;if bytes.last()!=Some(&0){return None;}
        let type_name=std::str::from_utf8(&bytes[..n-1]).ok()?;
        if !type_name.starts_with('F') || !type_name.ends_with("ShaderParameters") || !type_name.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_'){return None;}
        let slot=u16::from_le_bytes(take(&mut at,2)?.try_into().ok()?);
        let bound=u32::from_le_bytes(take(&mut at,4)?.try_into().ok()?);if bound>1{return None;}
        parameters.push(UniformParameter{type_name:type_name.into(),slot,bound:bound==1});
    }
    Some(UniformParameterTable{start,end:start.checked_add(at)?,parameters})
}
/// All accepted byte-pattern tables in a bounded window. Record ownership
/// must still be recovered; a nearby match is not automatically assigned.
pub fn uniform_parameter_candidates(bytes:&[u8],start:usize,end:usize)->Result<Vec<UniformParameterTable>,String>{
    let region=bytes.get(start..end).ok_or("invalid parameter-audit range")?;let mut out=vec![];
    for i in 0..region.len().saturating_sub(3){if let Some(table)=read_table(&region[i..],start+i){out.push(table);}}
    Ok(out)
}
pub fn fname_candidates(bytes:&[u8],names:&[String],start:usize,end:usize)->Result<Vec<NameCandidate>,String>{
    let region=bytes.get(start..end).ok_or("invalid name-audit range")?;let mut out=vec![];
    for (i,b) in region.windows(8).enumerate(){
        let index=i32::from_le_bytes(b[..4].try_into().unwrap());let number=i32::from_le_bytes(b[4..].try_into().unwrap());
        if index<0 || number!=0{continue;}
        let Some(name)=names.get(index as usize)else{continue;};
        if name.contains("Shader") || name.contains("VertexFactory") || name.starts_with("TBasePass") || name.starts_with("TDepth") {out.push(NameCandidate{offset:start+i,name:name.clone()});}
    }
    Ok(out)
}
pub fn string_candidates(bytes:&[u8],start:usize,end:usize)->Result<Vec<NameCandidate>,String>{
    let region=bytes.get(start..end).ok_or("invalid string-audit range")?;let mut out=vec![];
    for (i,b) in region.windows(4).enumerate(){
        let n=i32::from_le_bytes(b.try_into().unwrap());if !(3..=512).contains(&n){continue;}
        let Some(text)=region.get(i+4..i+4+n as usize)else{continue;};
        if text.last()!=Some(&0) || !text[..text.len()-1].iter().all(|b|(32..127).contains(b)){continue;}
        let name=std::str::from_utf8(&text[..text.len()-1]).unwrap();
        out.push(NameCandidate{offset:start+i,name:name.into()});
    }
    Ok(out)
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn end_offset_candidates_require_exact_32_bit_offset_and_complete_name(){let mut b=vec![255];b.extend(0i32.to_le_bytes());b.extend(2i32.to_le_bytes());b.extend(123456789u32.to_le_bytes());let names=vec!["FHitProxyVS".into()];assert_eq!(end_offset_candidates(&b,&names,0,b.len(),123456789).unwrap(),[NameCandidate{offset:1,name:"FHitProxyVS_1".into()}]);assert!(end_offset_candidates(&b,&names,0,b.len(),123456790).unwrap().is_empty());assert!(end_offset_candidates(&b,&names,0,b.len()-1,123456789).unwrap().is_empty());assert!(end_offset_candidates(&b,&[],0,b.len(),123456789).unwrap().is_empty());assert!(end_offset_candidates(&b,&names,0,b.len(),u64::MAX).is_err());b.extend(99u32.to_le_bytes());assert_eq!(end_offset_candidates(&b,&names,0,b.len(),123456789).unwrap().len(),1,"next word is not part of the end offset");}
    #[test]fn parameter_tables_roundtrip_and_reject_truncations_or_invalid_bound(){
        let t=UniformParameterTable{start:0,end:0,parameters:vec![UniformParameter{type_name:"FViewUniformShaderParameters".into(),slot:1,bound:true},UniformParameter{type_name:"FPrimitiveUniformShaderParameters".into(),slot:2,bound:false}]};let b=t.encode();let c=uniform_parameter_candidates(&b,0,b.len()).unwrap();assert_eq!(c.len(),2,"first bound=1 is also a nested candidate count, not ownership proof");let root=read_table(&b,0).unwrap();assert_eq!(root.parameters,t.parameters);assert_eq!(root.encode(),b);
        for n in 0..b.len(){assert!(read_table(&b[..n],0).is_none());}let mut bad=b;let n=bad.len();bad[n-4..].copy_from_slice(&2u32.to_le_bytes());assert!(read_table(&bad,0).is_none());assert!(uniform_parameter_candidates(&bad,n+1,n).is_err());
    }
    #[test]fn bounded_name_audits_report_offsets_without_claiming_ownership(){
        let mut bytes=vec![255];bytes.extend(0i32.to_le_bytes());bytes.extend(0i32.to_le_bytes());
        let names=vec!["TBasePassVS".into()];assert_eq!(fname_candidates(&bytes,&names,1,9).unwrap(),[NameCandidate{offset:1,name:"TBasePassVS".into()}]);assert!(fname_candidates(&bytes,&names,2,9).unwrap().is_empty());assert!(fname_candidates(&bytes,&names,10,9).is_err());
        let start=bytes.len();bytes.extend(4i32.to_le_bytes());bytes.extend(b"ABC\0");assert_eq!(string_candidates(&bytes,start,bytes.len()).unwrap(),[NameCandidate{offset:start,name:"ABC".into()}]);assert!(string_candidates(&bytes,start,bytes.len()-1).unwrap().is_empty());
    }
}
