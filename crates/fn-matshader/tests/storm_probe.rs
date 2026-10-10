use fn_pak::PakSet;
fn set() -> PakSet { PakSet::open_install(std::env::var_os("FN_TEST_ROOT").unwrap()).unwrap() }
#[test]
#[ignore]
fn probe_materials() {
    let set = set();
    for p in ["/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere", "/Game/Athena/SafeZone/M_SafeZoneStorm.M_SafeZoneStorm"] {
        match fn_fx::material::Material::load(&set, p) {
            Err(e) => println!("{p}: LOAD FAILED {e}"),
            Ok(m) => {
                println!("{p}: flags {:?}", m.flags);
                println!("  vectors {} scalars {} tex2d {}", m.uniform.vectors.len(), m.uniform.scalars.len(), m.uniform.textures2d.len());
                for v in &m.uniform.vectors { println!("   V {v:?}"); }
                for v in &m.uniform.scalars { println!("   S {v:?}"); }
                for (i, t) in m.uniform.textures2d.iter().enumerate() { println!("   T{i} {t:?} -> {:?}", m.texture_slot(i)); }
                println!("  referenced {:?}", m.referenced_textures);
            }
        }
    }
}

#[test]
#[ignore]
fn probe_indicator_lerp() {
    let set = set();
    let data = fn_uasset::PackageData::load(&set, "/Game/Athena/SafeZone/SafeZoneIndicator").unwrap();
    let pkg = data.package().unwrap();
    let f = fn_uasset::classes::read_function(&pkg, 68).unwrap();
    let s = f.script;
    for chunk in s[0x5f0..0x660].chunks(16) { println!("{}", chunk.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")); }
}

#[test]
#[ignore]
fn probe_uniform_names() {
    let set = set();
    let (a, x, b) = fn_fx::data::package_files(&set, "/Game/Athena/SafeZone/M_SafeZoneStorm").unwrap();
    let p = fn_ueio::Package::parse(&a, &x, &b).unwrap();
    for n in &p.names { if n.starts_with("FMaterialUniformExpression") || n.contains("Collection") { println!("name {n}"); } }
}

#[test]
#[ignore]
fn probe_collection_usage() {
    let set = set();
    let m = fn_fx::material::Material::load(&set, "/Game/Athena/SafeZone/M_SafeZoneStorm.M_SafeZoneStorm").unwrap();
    for ty in ["TBasePassVSFNoLightMapPolicy", "TBasePassPSFNoLightMapPolicy", "FDepthOnlyPS"] {
        let p = m.cooked.find("FLocalVertexFactory", ty).unwrap();
        let w = fn_fx::cooked::translated_function_wgsl(&p.shader, "x").unwrap();
        println!("== {ty}: ubo names {:?}", p.uniform_buffer_names);
        let slot = p.uniform_buffer_names.as_ref().unwrap().iter().position(|n| n == "MaterialCollection0").unwrap();
        let name = format!("cb{slot}[");
        let mut seen = std::collections::BTreeSet::new();
        for l in w.lines() { if l.contains("var<uniform>") { println!("  decl: {l}"); } let mut rest = l; while let Some(i) = rest.find(&name) { let tail = &rest[i + name.len()..]; let idx: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect(); let after: String = tail[idx.len()..].chars().skip(1).take(2).collect(); seen.insert(format!("{idx}{after}")); rest = &rest[i + name.len()..]; } }
        println!("  cb{slot} reads: {seen:?}");
    }
}

#[test]
#[ignore]
fn dump_sphere_textures() {
    let set = set();
    let out = std::path::PathBuf::from(std::env::var("TEX_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    for p in ["/Game/Gadgets/Assets/Teleporter/Textures/T_Teleporter_GridSquares_01", "/Game/Characters/Enemies/Smasher/Mesh/TilingWeakPointNormal"] {
        let (a, x, b) = fn_fx::data::package_files(&set, p).unwrap();
        let pk = fn_ueio::Package::parse(&a, &x, &b).unwrap();
        let t = fn_texture::Texture2D::parse(&pk).unwrap();
        let i = t.available_mips()[0];
        let img = t.decode(i, fn_texture::DecodeOptions::default()).unwrap();
        println!("{p}: {}x{} srgb {} addr {} {} filter {:?} comp {}", img.width, img.height, t.srgb, t.address_x, t.address_y, t.props.text("Filter"), t.compression_settings);
        let mut im = image::RgbaImage::new(img.width as u32, img.height as u32);
        for (k, px) in img.to_rgba8().chunks_exact(4).enumerate() { im.put_pixel((k % img.width) as u32, (k / img.width) as u32, image::Rgba([px[0], px[1], px[2], 255])); }
        im.save(out.join(format!("{}.png", p.rsplit('/').next().unwrap()))).unwrap();
        let mut al = image::RgbaImage::new(img.width as u32, img.height as u32);
        for (k, px) in img.to_rgba8().chunks_exact(4).enumerate() { al.put_pixel((k % img.width) as u32, (k / img.width) as u32, image::Rgba([px[3], px[3], px[3], 255])); }
        al.save(out.join(format!("{}_alpha.png", p.rsplit('/').next().unwrap()))).unwrap();
    }
}

#[test]
#[ignore]
fn probe_view_reads() {
    let set = set();
    for (path, tys) in [("/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere", ["TBasePassVSFNoLightMapPolicy", "TBasePassPSFNoLightMapPolicy"]), ("/Game/Athena/SafeZone/M_SafeZoneStorm.M_SafeZoneStorm", ["TBasePassVSFNoLightMapPolicy", "TBasePassPSFNoLightMapPolicy"])] {
        let m = fn_fx::material::Material::load(&set, path).unwrap();
        let v = fn_fx::view::ViewParams { eye: glam::Vec3::ZERO, forward: glam::Vec3::Z, up: glam::Vec3::Y, tan_half: (1.0, 0.6), near_m: 0.1, far_m: 3000., width: 1280, height: 720, time: 3.0, real_time: 3.0, delta_world: 0.0 }.view_buffer();
        for ty in tys {
            let p = m.cooked.find("FLocalVertexFactory", ty).unwrap();
            let w = fn_fx::cooked::translated_function_wgsl(&p.shader, "x").unwrap();
            let slot = p.uniform_buffer_names.as_ref().unwrap().iter().position(|n| n == "View").unwrap();
            let name = format!("cb{slot}[");
            let mut seen = std::collections::BTreeMap::new();
            for l in w.lines() { let mut rest = l; while let Some(i) = rest.find(&name) { let tail = &rest[i + name.len()..]; let idx: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect(); if let Ok(n) = idx.parse::<usize>() { seen.insert(n, v.get(n).copied().unwrap_or([f32::NAN; 4])); } rest = &rest[i + name.len()..]; } }
            println!("== {} {ty}: View slots read (idx -> our value)", path.rsplit('.').next().unwrap());
            for (n, val) in seen { println!("   {n}: {val:?}{}", if val == [0.0; 4] { "   <-- ZERO" } else { "" }); }
        }
    }
}

#[test]
#[ignore]
fn probe_sphere_ps_unnamed() {
    let set = set();
    let m = fn_fx::material::Material::load(&set, "/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere").unwrap();
    let p = m.cooked.find("FLocalVertexFactory", "TBasePassPSFNoLightMapPolicy").unwrap();
    let w = fn_fx::cooked::translated_function_wgsl(&p.shader, "x").unwrap();
    for l in w.lines().filter(|l| l.contains("var<uniform>") || l.contains("var t") || l.contains("sampler")) { println!("DECL {l}"); }
    for l in w.lines().filter(|l| l.contains("cb0[") ) { println!("CB0 {}", l.trim()); }
    for l in w.lines().filter(|l| l.contains("textureSample") || l.contains("textureLoad")) { println!("TEX {}", l.trim()); }
    println!("bindings {:?}", p.bindings.iter().map(|b| (b.kind, b.binding.register, b.binding.member)).collect::<Vec<_>>());
}

#[test]
#[ignore]
fn probe_sphere_ps_rdef() {
    let set = set();
    let m = fn_fx::material::Material::load(&set, "/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere").unwrap();
    let p = m.cooked.find("FLocalVertexFactory", "TBasePassPSFNoLightMapPolicy").unwrap();
    let c = fn_dxbc::Container::parse(&p.blob.bytes).unwrap();
    println!("chunks {:?}", c.chunks.iter().map(|x| x.tag_str()).collect::<Vec<_>>());
    println!("rdef {:?}", c.rdef().map(|r| r.cbuffers.iter().map(|b| (b.name.clone(), b.vars.iter().map(|v| (v.name.clone(), v.offset)).collect::<Vec<_>>())).collect::<Vec<_>>()));
    println!("resource_table {:?}", p.resource_table);
    for o in fn_dxbc::unreal_packet::optional_fields(&p.blob.trailer).unwrap() { println!("opt {} {:?}", o.key as char, String::from_utf8_lossy(&o.payload)); }
}

#[test]
#[ignore]
fn probe_sphere_ps_wgsl() {
    let set = set();
    let m = fn_fx::material::Material::load(&set, "/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere").unwrap();
    let p = m.cooked.find("FLocalVertexFactory", "TBasePassPSFNoLightMapPolicy").unwrap();
    let w = fn_fx::cooked::translated_function_wgsl(&p.shader, "x").unwrap();
    std::fs::write(std::env::var("WGSL_OUT").unwrap(), w).unwrap();
}

#[test]
#[ignore]
fn probe_sphere_names() {
    let set = set();
    let (a, x, b) = fn_fx::data::package_files(&set, "/Game/Athena/SafeZone/M_SafeZoneSphere").unwrap();
    let p = fn_ueio::Package::parse(&a, &x, &b).unwrap();
    for n in &p.names { if n.starts_with("FMaterialUniformExpression") || n.contains("Time") || n.contains("Panner") { println!("name {n}"); } }
}

#[test]
#[ignore]
fn probe_sphere_fmod_sites() {
    let set = set();
    let (a, x, b) = fn_fx::data::package_files(&set, "/Game/Athena/SafeZone/M_SafeZoneSphere").unwrap();
    let p = fn_ueio::Package::parse(&a, &x, &b).unwrap();
    let idx = |n: &str| p.names.iter().position(|s| s == n).unwrap() as i32;
    let (fm, tm) = (idx("FMaterialUniformExpressionFmod"), idx("FMaterialUniformExpressionTime"));
    for (label, id) in [("Fmod", fm), ("Time", tm)] {
        let mut at = vec![];
        for o in 0..x.len().saturating_sub(8) { if i32::from_le_bytes(x[o..o + 4].try_into().unwrap()) == id && i32::from_le_bytes(x[o + 4..o + 8].try_into().unwrap()) == 0 { at.push(o); } }
        println!("{label}: {} sites {:?}", at.len(), &at[..at.len().min(24)]);
    }
}

#[test]
#[ignore]
fn probe_per_frame() {
    let set = set();
    for p in ["/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere", "/Game/Athena/SafeZone/M_SafeZoneStorm.M_SafeZoneStorm"] {
        let m = fn_fx::material::Material::load(&set, p).unwrap();
        println!("{p}: per-frame scalars {} vectors {}", m.uniform.per_frame_scalars.len(), m.uniform.per_frame_vectors.len());
        for e in &m.uniform.per_frame_scalars { println!("  S {e:?}"); }
        for e in &m.uniform.per_frame_vectors { println!("  V {e:?}"); }
    }
}
