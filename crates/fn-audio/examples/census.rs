use fn_pak::PakSet; use fn_uasset::PackageData; use rayon::prelude::*; use std::collections::BTreeMap;
fn main(){
 let a:Vec<String>=std::env::args().collect();
 let set=PakSet::open_install(&a[1]).unwrap();
 let paths:Vec<String>=set.effective().filter_map(|(_,e)|{let p=e.rel_path(); (p.ends_with(".uasset")).then(||p.to_owned())}).collect();
 println!("uassets {}",paths.len());
 let res:Vec<(String,Vec<String>)>=paths.par_iter().filter_map(|p|{
   let d=PackageData::load(&set,p).ok()?; let pk=d.package().ok()?;
   let cls:Vec<String>=(0..pk.exports.len()).map(|i|pk.export_class_name(i)).filter(|c|c.contains(a[3].as_str())).collect();
   if cls.is_empty(){None}else{Some((p.clone(),cls))}}).collect();
 let mut m:BTreeMap<String,(usize,String)>=BTreeMap::new();
 for (p,c) in &res { for k in c { let e=m.entry(k.clone()).or_insert((0,p.clone())); e.0+=1; } }
 for (k,v) in &m { println!("{k}: {} e.g. {}",v.0,v.1); }
 std::fs::write(&a[2], res.iter().map(|(p,c)|format!("{p}\t{}",c.join(","))).collect::<Vec<_>>().join("\n")).unwrap();
}

