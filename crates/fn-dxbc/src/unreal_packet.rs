//! UE4.16 cooked D3D shader packet metadata recovered from the original data.
//! Resource-table header: bits, SRV map, sampler map, UAV map, layout hashes,
//! texture map. Trailer: keyed length-prefixed fields, final total byte count.
//! This does NOT identify the draw pass, vertex factory, or material defaults.

#[derive(Debug,Clone,PartialEq,Eq)]
pub struct ResourceTable {
    pub bits:u32,
    pub srv_map:Vec<u32>,
    pub sampler_map:Vec<u32>,
    pub uav_map:Vec<u32>,
    pub layout_hashes:Vec<u32>,
    pub texture_map:Vec<u32>,
}
struct Reader<'a>{data:&'a[u8],at:usize}
impl<'a> Reader<'a>{
    fn take(&mut self,n:usize)->Result<&'a[u8],String>{let end=self.at.checked_add(n).ok_or("packet range overflow")?;let v=self.data.get(self.at..end).ok_or("truncated shader packet")?;self.at=end;Ok(v)}
    fn u32(&mut self)->Result<u32,String>{Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))}
    fn array(&mut self)->Result<Vec<u32>,String>{let n=self.u32()? as usize;if n>(self.data.len()-self.at)/4{return Err("shader array exceeds packet".into());}(0..n).map(|_|self.u32()).collect()}
    fn string(&mut self)->Result<String,String>{
        let n=self.u32()? as i32;
        if n==0{return Ok(String::new());}
        if n>0 {let bytes=self.take(n as usize)?;if bytes.last()!=Some(&0){return Err("unterminated shader string".into());}String::from_utf8(bytes[..bytes.len()-1].to_vec()).map_err(|e|e.to_string())}
        else {let units=n.checked_neg().ok_or("shader string length overflow")? as usize;let bytes=self.take(units.checked_mul(2).ok_or("shader string range overflow")?)?;let mut text=bytes.chunks_exact(2).map(|c|u16::from_le_bytes([c[0],c[1]])).collect::<Vec<_>>();if text.pop()!=Some(0){return Err("unterminated wide shader string".into());}String::from_utf16(&text).map_err(|e|e.to_string())}
    }
}
impl ResourceTable {
    pub fn parse(data:&[u8])->Result<Self,String>{
        let mut r=Reader{data,at:0};
        let out=Self{bits:r.u32()?,srv_map:r.array()?,sampler_map:r.array()?,uav_map:r.array()?,layout_hashes:r.array()?,texture_map:r.array()?};
        if r.at!=data.len(){return Err("trailing resource table bytes".into());}Ok(out)
    }
    /// Exact serialization round-trip; do not reorder map entries or sentinels.
    pub fn encode(&self)->Vec<u8>{
        let mut out=self.bits.to_le_bytes().to_vec();
        for map in [&self.srv_map,&self.sampler_map,&self.uav_map,&self.layout_hashes,&self.texture_map]{out.extend_from_slice(&(map.len() as u32).to_le_bytes());for value in map{out.extend_from_slice(&value.to_le_bytes());}}
        out
    }
    /// Observe the original packed (uniform buffer, member, register) entries.
    /// Separate from resolving a member to a material texture/default value.
    pub fn bindings(&self,map:&[u32])->Result<Vec<Binding>,String>{
        let mut bindings=vec![];
        for buffer in 0..32u8 {
            if self.bits&(1u32<<buffer)==0 {continue;}
            let start=*map.get(buffer as usize).ok_or("missing resource map offset")? as usize;
            if start==0{continue;}
            if start>=map.len(){return Err("resource map offset outside array".into());}
            let mut at=start;
            while let Some(&token)=map.get(at) {
                if token==u32::MAX || (token>>24) as u8!=buffer{break;}
                bindings.push(Binding{buffer,member:((token>>8)&0xffff) as u16,register:(token&0xff) as u8});at+=1;
            }
        }
        Ok(bindings)
    }
}
#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub struct Binding{pub buffer:u8,pub member:u16,pub register:u8}

#[derive(Debug,Clone,Copy,PartialEq,Eq)]
pub enum ResourceKind{Texture,ShaderResourceView,Sampler,UnorderedAccessView}
#[derive(Debug,Clone,PartialEq,Eq)]
pub struct NamedBinding{
    pub kind:ResourceKind,
    pub binding:Binding,
    pub buffer_name:String,
    pub layout_hash:u32,
}
/// Validated original resource member -> shader register association. This
/// does not resolve a member to an actual texture or choose a shader variant.
pub fn named_bindings(table:&ResourceTable,names:Option<&[String]>,decls:&crate::ir::Decls)->Result<Vec<NamedBinding>,String>{
    let mut out=vec![];
    for (kind,map) in [(ResourceKind::Texture,&table.texture_map),(ResourceKind::ShaderResourceView,&table.srv_map),(ResourceKind::Sampler,&table.sampler_map),(ResourceKind::UnorderedAccessView,&table.uav_map)]{
        for binding in table.bindings(map)?{
            let slot=binding.register as u32;
            let declared=match kind{
                ResourceKind::Sampler=>decls.samplers.iter().any(|d|d.slot==slot),
                ResourceKind::UnorderedAccessView=>decls.uavs.iter().any(|d|d.slot==slot),
                _=>decls.resources.iter().any(|d|d.slot==slot),
            };
            if !declared{return Err(format!("{kind:?} register {slot} missing from shader declarations"));}
            let buffer_name=names.and_then(|n|n.get(binding.buffer as usize)).filter(|n|!n.is_empty()).ok_or("resource binding lacks named uniform-buffer slot")?.clone();
            let layout_hash=*table.layout_hashes.get(binding.buffer as usize).ok_or("resource binding lacks uniform-buffer layout hash")?;
            out.push(NamedBinding{kind,binding,buffer_name,layout_hash});
        }
    }
    Ok(out)
}

#[derive(Debug,Clone,PartialEq,Eq)]
pub struct OptionalField{pub key:u8,pub payload:Vec<u8>}
pub fn optional_fields(data:&[u8])->Result<Vec<OptionalField>,String>{
    if data.len()<4{return Err("missing shader optional-data size".into());}
    let declared=u32::from_le_bytes(data[data.len()-4..].try_into().unwrap()) as usize;
    if declared!=data.len(){return Err("shader optional-data size mismatch".into());}
    let mut r=Reader{data:&data[..data.len()-4],at:0};let mut fields=vec![];
    while r.at<r.data.len(){let key=r.take(1)?[0];let n=r.u32()? as usize;fields.push(OptionalField{key,payload:r.take(n)?.to_vec()});}
    Ok(fields)
}
pub fn uniform_buffer_names(fields:&[OptionalField])->Result<Option<Vec<String>>,String>{
    let Some(field)=fields.iter().find(|f|f.key==b'u') else{return Ok(None);};
    let mut r=Reader{data:&field.payload,at:0};let n=r.u32()? as usize;
    if n>(r.data.len()-r.at)/4{return Err("uniform-buffer names exceed packet".into());}
    let names=(0..n).map(|_|r.string()).collect::<Result<Vec<_>,_>>()?;
    if r.at!=r.data.len(){return Err("trailing uniform-buffer-name bytes".into());}Ok(Some(names))
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn named_binding_rejects_missing_declaration_name_and_hash(){
        let mut t=ResourceTable{bits:1,srv_map:vec![0,u32::MAX],sampler_map:vec![0,u32::MAX],uav_map:vec![0,u32::MAX],layout_hashes:vec![123],texture_map:vec![1,0x00000502,u32::MAX]};
        let names=vec!["Material".into()];let mut d=crate::ir::Decls::default();
        assert!(named_bindings(&t,Some(&names),&d).is_err());
        d.resources.push(crate::ir::ResourceDecl{slot:2,dim:3,ret:[5;4],samples:0,kind:crate::ir::ResKind::Typed});
        let b=named_bindings(&t,Some(&names),&d).unwrap();assert_eq!(b.len(),1);assert_eq!(b[0].binding.member,5);assert_eq!(b[0].binding.register,2);assert_eq!(b[0].buffer_name,"Material");assert_eq!(b[0].layout_hash,123);
        assert!(named_bindings(&t,None,&d).is_err());assert!(named_bindings(&t,Some(&[String::new()]),&d).is_err());t.layout_hashes.clear();assert!(named_bindings(&t,Some(&names),&d).is_err());
    }
    #[test]fn empty_table_roundtrip_and_truncation(){let t=ResourceTable{bits:0,srv_map:vec![],sampler_map:vec![],uav_map:vec![],layout_hashes:vec![],texture_map:vec![]};let b=t.encode();assert_eq!(b.len(),24);assert_eq!(ResourceTable::parse(&b).unwrap(),t);for n in 0..b.len(){assert!(ResourceTable::parse(&b[..n]).is_err());}let mut extra=b;extra.push(0);assert!(ResourceTable::parse(&extra).is_err());}
    #[test]fn packed_map_keeps_member_and_register_distinct(){let t=ResourceTable{bits:4,srv_map:vec![0,0,0,u32::MAX],sampler_map:vec![],uav_map:vec![],layout_hashes:vec![0,0,0x8580958],texture_map:vec![0,0,3,0x02000000,0x02000201,u32::MAX]};assert_eq!(ResourceTable::parse(&t.encode()).unwrap(),t);assert_eq!(t.bindings(&t.texture_map).unwrap(),[Binding{buffer:2,member:0,register:0},Binding{buffer:2,member:2,register:1}]);assert!(t.bindings(&[0,0,40]).is_err());}
    #[test]fn optional_fields_preserve_unknown_keys_and_buffer_slots(){let mut payload=2u32.to_le_bytes().to_vec();payload.extend_from_slice(&0u32.to_le_bytes());payload.extend_from_slice(&9u32.to_le_bytes());payload.extend_from_slice(b"Material\0");let mut data=vec![b'u'];data.extend_from_slice(&(payload.len() as u32).to_le_bytes());data.extend(payload);data.push(b'?');data.extend_from_slice(&1u32.to_le_bytes());data.push(99);let len=data.len()+4;data.extend_from_slice(&(len as u32).to_le_bytes());let fields=optional_fields(&data).unwrap();assert_eq!(uniform_buffer_names(&fields).unwrap(),Some(vec!["".into(),"Material".into()]));assert_eq!(fields[1].payload,[99]);for n in 0..data.len(){assert!(optional_fields(&data[..n]).is_err());}}
}
