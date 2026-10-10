//! Exact candidate serialization for an audit, not shader-map selection.
use crate::uniform::{Expr,SetCandidate};
struct Writer<'a>{bytes:Vec<u8>,names:&'a[String]}
impl Writer<'_>{
    fn u32(&mut self,v:u32){self.bytes.extend(v.to_le_bytes());}
    fn f32(&mut self,v:f32){self.u32(v.to_bits());}
    fn vec4(&mut self,v:&[f32;4]){for f in v{self.f32(*f);}}
    fn name(&mut self,name:&str)->Result<(),String>{
        let mut matches=vec![];
        for (i,base) in self.names.iter().enumerate(){for n in 0..=4u32{
            let spelled=if n==0{base.clone()}else{format!("{base}_{}",n-1)};
            if spelled==name{matches.push((i,n));}
        }}
        if matches.len()!=1{return Err(format!("FName {name:?} has {} encodings",matches.len()));}
        let (i,n)=matches[0];self.u32(u32::try_from(i).map_err(|_|"name index overflow")?);self.u32(n);Ok(())
    }
    fn expr(&mut self,e:&Expr,depth:u32)->Result<(),String>{
        if depth>32{return Err("expression nesting".into());}
        let ty=match e{Expr::VectorParameter{..}=>"VectorParameter",Expr::ScalarParameter{..}=>"ScalarParameter",Expr::Constant{..}=>"Constant",Expr::ComponentSwizzle{..}=>"ComponentSwizzle",Expr::FoldedMath{..}=>"FoldedMath",Expr::AppendVector{..}=>"AppendVector",Expr::TrigMath{..}=>"TrigMath",Expr::Max{..}=>"Max",Expr::Texture{..}=>"Texture",Expr::TextureParameter{..}=>"TextureParameter",Expr::Time=>"Time",Expr::RealTime=>"RealTime"};
        self.name(&format!("FMaterialUniformExpression{ty}"))?;
        match e{
            Expr::VectorParameter{name,default}=>{self.name(name)?;self.vec4(default);}
            Expr::ScalarParameter{name,default}=>{self.name(name)?;self.f32(*default);}
            Expr::Constant{value,value_type}=>{self.vec4(value);self.bytes.push(*value_type);}
            Expr::ComponentSwizzle{e,idx,count}=>{self.expr(e,depth+1)?;for i in idx{self.bytes.push(*i as u8);}self.bytes.push(*count);}
            Expr::FoldedMath{a,b,op,value_type}=>{self.expr(a,depth+1)?;self.expr(b,depth+1)?;self.bytes.push(*op);self.u32(*value_type);}
            Expr::AppendVector{a,b,num_a}=>{self.expr(a,depth+1)?;self.expr(b,depth+1)?;self.u32(*num_a);}
            Expr::TrigMath{x,y,op}=>{self.expr(x,depth+1)?;self.expr(y,depth+1)?;self.bytes.push(*op);}
            Expr::Max{a,b}=>{self.expr(a,depth+1)?;self.expr(b,depth+1)?;}
            Expr::Texture{index,sampler_source}=>{self.u32(*index as u32);self.u32(*sampler_source as u32);}
            Expr::TextureParameter{name,index,sampler_source}=>{self.name(name)?;self.u32(*index as u32);self.u32(*sampler_source as u32);}
            Expr::Time|Expr::RealTime=>{}
        }
        Ok(())
    }
    fn list(&mut self,exprs:&[Expr])->Result<(),String>{self.u32(u32::try_from(exprs.len()).map_err(|_|"list count overflow")?);for e in exprs{self.expr(e,0)?;}Ok(())}
}
pub fn encode_candidate(c:&SetCandidate,names:&[String])->Result<Vec<u8>,String>{
    let mut w=Writer{bytes:vec![],names};w.list(&c.set.vectors)?;w.list(&c.set.scalars)?;w.list(&c.set.textures2d)?;w.u32(c.trailing_count as u32);Ok(w.bytes)
}
#[cfg(test)]mod tests{
    use super::*;use crate::uniform::{UniformSet,set_candidates};
    #[test]fn roundtrips_every_supported_expression_and_numbered_fname(){
        let names=["VectorParameter","ScalarParameter","Constant","ComponentSwizzle","FoldedMath","AppendVector","TrigMath","Texture","TextureParameter","Time","RealTime"].map(|s|format!("FMaterialUniformExpression{s}"));
        let mut names=names.to_vec();names.push("Tint".into());
        let exprs=vec![
            Expr::VectorParameter{name:"Tint_1".into(),default:[1.,2.,3.,4.]},Expr::ScalarParameter{name:"Tint".into(),default:0.25},
            Expr::Constant{value:[1.;4],value_type:4},Expr::ComponentSwizzle{e:Box::new(Expr::Time),idx:[3,1,-1,-1],count:2},
            Expr::FoldedMath{a:Box::new(Expr::Time),b:Box::new(Expr::RealTime),op:4,value_type:0xf0000001},
            Expr::AppendVector{a:Box::new(Expr::Time),b:Box::new(Expr::RealTime),num_a:2},
            Expr::TrigMath{x:Box::new(Expr::Time),y:Box::new(Expr::RealTime),op:1},
            Expr::Texture{index:-1,sampler_source:2},Expr::TextureParameter{name:"Tint_2".into(),index:12,sampler_source:0},Expr::Time,Expr::RealTime,
        ];
        let c=SetCandidate{start:0,end:0,trailing_count:0,set:UniformSet{vectors:exprs,scalars:vec![],textures2d:vec![]}};
        let bytes=encode_candidate(&c,&names).unwrap();let decoded=set_candidates(&bytes,&names);let root=decoded.iter().find(|c|c.start==0).unwrap();assert_eq!(root.set,c.set);assert_eq!(encode_candidate(root,&names).unwrap(),bytes);
        names.push("Tint_1".into());assert!(encode_candidate(&c,&names).is_err());
    }
    #[test]fn preserves_float_bits_and_rejects_ambiguous_names(){
        let names=vec!["FMaterialUniformExpressionConstant".into()];
        let c=SetCandidate{start:0,end:0,trailing_count:0,set:UniformSet{vectors:vec![Expr::Constant{value:[-0.,f32::from_bits(0x7fc01234),f32::INFINITY,1.],value_type:3}],scalars:vec![],textures2d:vec![]}};
        let bytes=encode_candidate(&c,&names).unwrap();let decoded=set_candidates(&bytes,&names);assert_eq!(decoded.len(),1);assert_eq!(encode_candidate(&decoded[0],&names).unwrap(),bytes);
        assert!(encode_candidate(&c,&[names[0].clone(),names[0].clone()]).is_err());assert!(encode_candidate(&c,&[]).is_err());
    }
}
