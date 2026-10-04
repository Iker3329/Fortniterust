//! Original FName/u32 end-offset anchors. Enclosing pass/VF ownership remains
//! separate: don't choose a render permutation from a shader name alone.
use crate::extract::SerializedStream;
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct NamedEntry{pub offset:usize,pub end:usize,pub name_index:u32,pub number:u32,pub name:String}
impl NamedEntry{
    pub fn encode_header(&self,absolute_base:usize)->Result<[u8;12],String>{
        let absolute_end=absolute_base.checked_add(self.end).and_then(|n|u32::try_from(n).ok()).ok_or("legacy entry end exceeds u32")?;
        let mut b=[0;12];b[..4].copy_from_slice(&self.name_index.to_le_bytes());b[4..8].copy_from_slice(&self.number.to_le_bytes());b[8..].copy_from_slice(&absolute_end.to_le_bytes());Ok(b)
    }
}
pub fn candidates(data:&[u8],names:&[String],start:usize,end:usize,absolute_end:u64)->Result<Vec<NamedEntry>,String>{
    let absolute_end=u32::try_from(absolute_end).map_err(|_|"legacy entry end exceeds u32")?;
    let region=data.get(start..end).ok_or("invalid entry audit range")?;let mut out=vec![];
    for (i,b) in region.windows(12).enumerate(){
        if u32::from_le_bytes(b[8..12].try_into().unwrap())!=absolute_end{continue;}
        let index=i32::from_le_bytes(b[..4].try_into().unwrap());let number=i32::from_le_bytes(b[4..8].try_into().unwrap());
        if index<0 || !(0..=4).contains(&number){continue;}let Some(name)=names.get(index as usize).filter(|n|!n.is_empty())else{continue;};
        out.push(NamedEntry{offset:start+i,end:absolute_end as usize,name_index:index as u32,number:number as u32,name:if number==0{name.clone()}else{format!("{name}_{}",number-1)}});
    }
    // Candidate end is an absolute offset until ownership/base is supplied.
    Ok(out)
}
pub fn named_entry(data:&[u8],names:&[String],minimum:usize,absolute_base:usize,stream:&SerializedStream)->Result<NamedEntry,String>{
    if stream.tail.end>data.len() || stream.start>=stream.end || stream.end>=stream.tail.end{return Err("invalid serialized stream range".into());}
    let absolute_end=absolute_base.checked_add(stream.tail.end).ok_or("entry end overflow")?;
    let mut found=candidates(data,names,minimum,stream.start,absolute_end as u64)?;
    if found.len()!=1{return Err(format!("expected unique named entry end anchor, found {}",found.len()));}
    let mut entry=found.pop().unwrap();entry.end=stream.tail.end;
    if entry.encode_header(absolute_base)?!=data[entry.offset..entry.offset+12]{return Err("entry header roundtrip mismatch".into());}
    Ok(entry)
}
#[cfg(test)]mod tests{
    use super::*;use crate::extract::SerializedStreamTail;
    #[test]fn retains_original_name_tokens_and_rejects_missing_or_ambiguous_anchors(){
        let stream=SerializedStream{start:70,end:80,frequency:0,platform:0,preceding_tag:[0;8],tail:SerializedStreamTail{end:112,identifier:[0;20],unverified_count_word:0,sampler_count:0,inflated_size:0}};
        let mut data=vec![255;112];let names=vec!["FHitProxyVS".into()];let raw=NamedEntry{offset:20,end:112,name_index:0,number:2,name:"FHitProxyVS_1".into()};data[20..32].copy_from_slice(&raw.encode_header(1000).unwrap());
        assert_eq!(named_entry(&data,&names,0,1000,&stream).unwrap(),raw);assert!(named_entry(&data,&names,21,1000,&stream).is_err());assert!(named_entry(&data,&names,0,1001,&stream).is_err());assert!(named_entry(&data[..111],&names,0,1000,&stream).is_err());
        data[40..52].copy_from_slice(&raw.encode_header(1000).unwrap());assert!(named_entry(&data,&names,0,1000,&stream).is_err());
    }
}
