//! `fn-dxbc`: disassemble / evaluate / translate DXBC shaders and extract them from a Fortnite 1.8 install.
use fn_dxbc::codegen::{rust_prelude, translate_rust, translate_wgsl};
use fn_dxbc::disasm::{disassemble_container, DisasmOptions};
use fn_dxbc::interp::{Env, Machine, Options, TexOp, TexReq};
use fn_dxbc::{Container, Shader};
use fn_dxbc_cli::spec::{default_install, load_blob};
use fn_dxbc_cli::sweep::{layout_json, report, sweep, SweepOptions};
use std::path::PathBuf;

const USAGE: &str = "fn-dxbc <command>

  disasm [--hex] <file|package#id>        d3dcompiler-format listing (package = /Game/... path or GlobalShaderCache, id = zlib offset)
  layout <file|package#id>                constant buffer / resource layout (JSON)
  eval <file|package#id> [--cb SLOT:ELEM=x,y,z,w]... [--in REG=x,y,z,w]... [--tex SLOT=r,g,b,a]... [--div0] [--no-discard]
                                          run a pixel/vertex shader on the CPU and print the output registers
  translate rust|wgsl <file|package#id> [--name fn_name]
  extract <install> <out-dir> [--asm] [--validate-wgsl] [--filter text] [--limit n] [--global-only]
                                          slice every shader out of the install, translate all, print the coverage report
  scan <install> <package>                list the shaders of one package (id, type, size)
  material <out-dir> <package>...         export every shader of the given materials (/Game/... paths): dxbc, listing, layout, uniforms json
  tonemap <out-dir>                       the UE 4.16 tonemapper / CombineLUTs / eye adaptation shaders of the global shader cache
  menu-shaders <out-dir> [--widgets file] the Esc-menu material shaders and their evaluated constant buffers (needs reports/menu-widgets.json)
  gen-ui <out.rs> <fn>=<package>#<shader id>...   Rust module with <fn>_cb (constant buffer from the uniform expressions) and <fn>_ps (translated pixel shader)
  survey <dir>                            opcode usage per program type of all .dxbc files in a directory

The install defaults to $FN_INSTALL or the 1.8 install path of this workstation.";

fn parse_vec(s: &str) -> Result<[f32; 4], String> {
    let v: Result<Vec<f32>, _> = s.split(',').map(|x| x.trim().parse::<f32>()).collect();
    let v = v.map_err(|e| format!("bad vector '{s}': {e}"))?;
    let mut o = [0.0f32; 4];
    for (i, x) in v.iter().take(4).enumerate() { o[i] = *x; }
    if v.len() == 1 { o = [v[0]; 4]; }
    Ok(o)
}

struct CliEnv { tex: Vec<(u32, [f32; 4])> }
impl Env for CliEnv {
    fn texture(&mut self, r: &TexReq) -> [f32; 4] {
        if matches!(r.op, TexOp::Resinfo) { return [1.0, 1.0, 1.0, 1.0]; }
        self.tex.iter().find(|(s, _)| *s == r.texture).map(|(_, c)| *c).unwrap_or([0.5, 0.5, 0.5, 1.0])
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(|s| s.as_str()) else { println!("{USAGE}"); return Ok(()) };
    let rest = &args[1..];
    let flag = |n: &str| rest.iter().any(|a| a == n);
    let value = |n: &str| rest.iter().position(|a| a == n).and_then(|i| rest.get(i + 1)).cloned();
    let values = |n: &str| -> Vec<String> { rest.iter().enumerate().filter(|(_, a)| *a == n).filter_map(|(i, _)| rest.get(i + 1).cloned()).collect() };
    let positional: Vec<&String> = {
        let mut v = vec![];
        let mut skip = false;
        for a in rest {
            if skip { skip = false; continue; }
            if matches!(a.as_str(), "--cb" | "--in" | "--tex" | "--name" | "--filter" | "--limit" | "--widgets") { skip = true; continue; }
            if a.starts_with("--") { continue; }
            v.push(a);
        }
        v
    };
    let install = default_install();
    match cmd {
        "record-windows"=>{
            let p=positional.first().ok_or("record-windows needs package")?;
            let set=fn_pak::PakSet::open_install(&install).map_err(|e|e.to_string())?;
            let bytes=fn_dxbc_cli::spec::package_bytes(&set,p)?;
            let ua=set.read_game(p,"uasset").map_err(|e|e.to_string())?;
            let names=fn_dxbc_cli::uniform::package_names(&ua,&bytes)?;
            let package=fn_uasset::package::Package::parse(&ua,&bytes).map_err(|e|e.to_string())?;
            let header_size=usize::try_from(package.summary.total_header_size).map_err(|_|"negative package header size")?;
            let blobs=fn_dxbc::extract::scan_blobs(&bytes);
            let docs=blobs.iter().map(|b|{
                let start=b.id.saturating_sub(2048);
                let f=fn_dxbc_cli::record::fname_candidates(&bytes,&names,start,b.id).unwrap();
                let s=fn_dxbc_cli::record::string_candidates(&bytes,start,b.id).unwrap();
                let tables=fn_dxbc_cli::record::uniform_parameter_candidates(&bytes,start,b.id).unwrap();
                let stream=fn_dxbc::Shader::parse(&b.bytes).map_err(|e|e.to_string()).and_then(|shader|fn_dxbc::extract::serialized_stream(&bytes,b,&shader));
                let stream_json=stream.as_ref().ok().map(|s|{
                    let tail_end=s.end.saturating_add(64).min(bytes.len());
                    let tail=&bytes[s.end..tail_end];
                    let words=tail.chunks_exact(4).map(|v|u32::from_le_bytes(v.try_into().unwrap())).collect::<Vec<_>>();
                    let program=fn_dxbc::Shader::parse(&b.bytes).unwrap();let sampler_count=program.decls.samplers.len();
                    let stat=fn_dxbc::Container::parse(&b.bytes).unwrap().stat_instruction_count();
                    let absolute_end=header_size.checked_add(s.tail.end).unwrap() as u64;
                    let end_names=fn_dxbc_cli::record::end_offset_candidates(&bytes,&names,start,b.id,absolute_end).unwrap();
                    serde_json::json!({"start":s.start,"end":s.end,"frequency":s.frequency,"platform":s.platform,"preceding_tag":s.preceding_tag,"inflated_size":b.header.len()+b.bytes.len()+b.trailer.len(),"tail_size_matches":words.get(7).copied().map(|n|n as usize==b.header.len()+b.bytes.len()+b.trailer.len()),"tail_sampler_matches":words.get(6).copied().map(|n|n as usize==sampler_count),"stat_instruction_count":stat,"absolute_record_end":absolute_end,"end_name_candidates":end_names.iter().map(|n|serde_json::json!({"offset":n.offset,"name":n.name})).collect::<Vec<_>>(),"following_u32":words,"following_hex":tail.iter().map(|v|format!("{v:02x}")).collect::<String>()})
                });
                serde_json::json!({"stream":b.id,"start":start,"serialized_stream":stream_json,"stream_error":stream.err(),"parameter_tables":tables.iter().map(|t|serde_json::json!({"start":t.start,"end":t.end,"roundtrip_exact":t.encode()==bytes[t.start..t.end],"parameters":t.parameters.iter().map(|p|serde_json::json!({"type_name":p.type_name,"slot":p.slot,"bound":p.bound})).collect::<Vec<_>>()})).collect::<Vec<_>>(),"fnames":f.iter().map(|n|serde_json::json!({"offset":n.offset,"name":n.name})).collect::<Vec<_>>(),"strings":s.iter().map(|n|serde_json::json!({"offset":n.offset,"name":n.name})).collect::<Vec<_>>(),"before_hex":bytes[start..b.id].iter().map(|v|format!("{v:02x}")).collect::<String>()})
            }).collect::<Vec<_>>();
            println!("{}",serde_json::to_string_pretty(&serde_json::json!({"package":p,"package_header_size":header_size,"note":"Nearby byte-pattern candidates only, not shader record ownership.","windows":docs})).unwrap());
        }
        "uniform-candidates"=>{
            let p=positional.first().ok_or("uniform-candidates needs package")?;
            let set=fn_pak::PakSet::open_install(&install).map_err(|e|e.to_string())?;
            let bytes=fn_dxbc_cli::spec::package_bytes(&set,p)?;
            let ua=set.read_game(p,"uasset").map_err(|e|e.to_string())?;
            let names=fn_dxbc_cli::uniform::package_names(&ua,&bytes)?;
            let candidates=fn_dxbc_cli::uniform::set_candidates(&bytes,&names);
            let docs=candidates.iter().map(|c|{
                let mut expressions=fn_dxbc_cli::material::uniforms_json(p,&c.set,&Default::default());
                // Existing legacy export's tN label is only list position.
                // Do not present it as a recovered binding in this audit.
                if let Some(textures)=expressions["textures2d"].as_array_mut(){for (i,t) in textures.iter_mut().enumerate(){if let Some(t)=t.as_object_mut(){t.remove("register");t.insert("expression_index".into(),serde_json::json!(i));}}}
                let roundtrip=fn_dxbc_cli::uniform_encode::encode_candidate(c,&names).map(|encoded|encoded==bytes[c.start..c.end]);
                let before_start=c.start.saturating_sub(128);
                let after_end=(c.end+512).min(bytes.len());
                let before=bytes[before_start..c.start].iter().map(|b|format!("{b:02x}")).collect::<String>();
                let after=bytes[c.end..after_end].iter().map(|b|format!("{b:02x}")).collect::<String>();
                let label=p.rsplit('/').next().unwrap_or(p);
                let owner_label_anchor=fn_dxbc_cli::uniform::preceding_label(&bytes,c.start,label);
                let description=fn_dxbc_cli::uniform::compilation_description(&bytes,c,label);
                let description_json=description.as_ref().ok().map(|d|serde_json::json!({"start":d.start,"end":d.end,"text":d.text,"unknown_prefix":d.preceding_bytes}));
                serde_json::json!({"start":c.start,"end":c.end,"trailing_count":c.trailing_count,"roundtrip_exact":roundtrip.as_ref().ok(),"roundtrip_error":roundtrip.err(),"owner_label_anchor":owner_label_anchor,"compilation_description":description_json,"description_error":description.err(),"context_before_start":before_start,"context_before_hex":before,"context_after_hex":after,"expressions":expressions})
            }).collect::<Vec<_>>();
            println!("{}",serde_json::to_string_pretty(&serde_json::json!({"package":p,"note":"Pattern candidates only; no verified shader-map ownership or texture register mapping.","candidates":docs})).unwrap());
        }
        "disasm" => {
            let spec = positional.first().ok_or("disasm needs a shader spec")?;
            let b = load_blob(&install, spec)?;
            let c = Container::parse(&b.bytes).map_err(|e| e.to_string())?;
            print!("{}", disassemble_container(&c, DisasmOptions { hex_literals: flag("--hex") }).map_err(|e| e.to_string())?);
        }
        "layout" => {
            let b = load_blob(&install, positional.first().ok_or("layout needs a shader spec")?)?;
            let c = Container::parse(&b.bytes).map_err(|e| e.to_string())?;
            let sh = Shader::from_container(&c).map_err(|e| e.to_string())?;
            println!("{}", serde_json::to_string_pretty(&layout_json(&c, &sh)).unwrap());
        }
        "eval" => {
            let b = load_blob(&install, positional.first().ok_or("eval needs a shader spec")?)?;
            let sh = Shader::parse(&b.bytes).map_err(|e| e.to_string())?;
            let mut m = Machine::new(&sh);
            m.opts = Options { div_zero_is_zero: flag("--div0"), ignore_discard: flag("--no-discard"), ..Default::default() };
            for s in values("--cb") {
                let (k, v) = s.split_once('=').ok_or("--cb SLOT:ELEM=x,y,z,w")?;
                let (slot, elem) = k.split_once(':').ok_or("--cb SLOT:ELEM=x,y,z,w")?;
                let (slot, elem): (usize, usize) = (slot.parse().map_err(|_| "bad slot")?, elem.parse().map_err(|_| "bad element")?);
                if m.cb.len() <= slot { m.cb.resize(slot + 1, vec![]); }
                if m.cb[slot].len() <= elem { m.cb[slot].resize(elem + 1, [0; 4]); }
                let v = parse_vec(v)?;
                m.cb[slot][elem] = [v[0].to_bits(), v[1].to_bits(), v[2].to_bits(), v[3].to_bits()];
            }
            for s in values("--in") {
                let (k, v) = s.split_once('=').ok_or("--in REG=x,y,z,w")?;
                m.set_input(k.parse().map_err(|_| "bad register")?, parse_vec(v)?);
            }
            let mut env = CliEnv { tex: vec![] };
            for s in values("--tex") {
                let (k, v) = s.split_once('=').ok_or("--tex SLOT=r,g,b,a")?;
                env.tex.push((k.parse().map_err(|_| "bad slot")?, parse_vec(v)?));
            }
            let out = m.run(&mut env).map_err(|e| e.to_string())?;
            println!("{}", sh.version_string());
            if out.discarded { println!("DISCARDED"); }
            for (i, o) in out.o.iter().enumerate() { println!("o{i} = {:.7} {:.7} {:.7} {:.7}", o[0], o[1], o[2], o[3]); }
        }
        "translate" => {
            let target = positional.first().ok_or("translate rust|wgsl <shader>")?;
            let b = load_blob(&install, positional.get(1).ok_or("translate rust|wgsl <shader>")?)?;
            let sh = Shader::parse(&b.bytes).map_err(|e| e.to_string())?;
            let name = value("--name").unwrap_or_else(|| format!("shader_{}", b.id));
            match target.as_str() {
                "rust" => { println!("{}", rust_prelude()); println!("{}", translate_rust(&sh, &name).map_err(|e| e.0)?); }
                "wgsl" => println!("{}", translate_wgsl(&sh, &name).map_err(|e| e.0)?),
                t => return Err(format!("unknown target {t}")),
            }
        }
        "extract" | "sweep" => {
            let inst = positional.first().map(|s| PathBuf::from(s.as_str())).unwrap_or(install);
            let out = if cmd == "extract" { positional.get(1).map(|s| PathBuf::from(s.as_str())) } else { None };
            let opts = SweepOptions { out: out.clone(), write_asm: flag("--asm"), limit: value("--limit").and_then(|s| s.parse().ok()), filter: value("--filter"), global_only: flag("--global-only"), validate_wgsl: flag("--validate-wgsl") };
            let t = std::time::Instant::now();
            let stats = sweep(&inst, &opts)?;
            let text = format!("{}elapsed: {:.1}s\n", report(&stats), t.elapsed().as_secs_f64());
            print!("{text}");
            if let Some(o) = out { let _ = std::fs::write(o.join("report.txt"), &text); }
        }
        "material" => {
            let out = PathBuf::from(positional.first().ok_or("material <out-dir> <package>...")?.as_str());
            let set = fn_pak::PakSet::open_install(&install).map_err(|e| e.to_string())?;
            for p in &positional[1..] {
                let r = fn_dxbc_cli::material::export_package(&set, p, &out, true)?;
                println!("{p}: {} shaders{}", r.shaders, if r.uniforms { ", uniforms json written" } else { "" });
            }
        }
        "tonemap" => {
            let out = PathBuf::from(positional.first().ok_or("tonemap <out-dir>")?.as_str());
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            let set = fn_pak::PakSet::open_install(&install).map_err(|e| e.to_string())?;
            let bytes = fn_dxbc_cli::spec::package_bytes(&set, "GlobalShaderCache")?;
            let blobs = fn_dxbc_cli::extract::scan_blobs(&bytes);
            let mut n = 0;
            for id in [1182809usize, 1070630, 769199, 801686, 923320] {
                let b = blobs.iter().find(|b| b.id == id).ok_or(format!("global shader cache: no shader stream at {id}"))?;
                std::fs::write(out.join(format!("shader_{id}.dxbc")), &b.bytes).map_err(|e| e.to_string())?;
                let c = Container::parse(&b.bytes).map_err(|e| e.to_string())?;
                std::fs::write(out.join(format!("shader_{id}.asm")), disassemble_container(&c, DisasmOptions::default()).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                n += 1;
            }
            println!("wrote {n} shaders to {}", out.display());
        }
        "menu-shaders" => {
            let out = PathBuf::from(positional.first().ok_or("menu-shaders <out-dir>")?.as_str());
            let widgets_path = value("--widgets").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\reports\menu-widgets.json"));
            let txt = std::fs::read_to_string(&widgets_path).map_err(|e| format!("{}: {e}", widgets_path.display()))?;
            let widgets: serde_json::Value = serde_json::from_str(txt.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
            let set = fn_pak::PakSet::open_install(&install).map_err(|e| e.to_string())?;
            let r = fn_dxbc_cli::menu::run(&set, &widgets, &out)?;
            println!("menu-shaders: {} material instances, roles {} -> {}", r["materials"].as_object().map_or(0, |m| m.len()), r["roles"].as_object().map_or(String::new(), |m| m.keys().cloned().collect::<Vec<_>>().join(", ")), out.display());
        }
        "gen-ui" => {
            let out = PathBuf::from(positional.first().ok_or("gen-ui <out.rs> <fn>=<package>#<id>...")?.as_str());
            let mut entries = vec![];
            for a in &positional[1..] {
                let (name, rest) = a.split_once('=').ok_or("gen-ui entries are <fn>=<package>#<shader id>")?;
                let (pkg, id) = rest.rsplit_once('#').ok_or("gen-ui entries are <fn>=<package>#<shader id>")?;
                entries.push(fn_dxbc_cli::uigen::UiEntry { name: name.to_string(), package: pkg.to_string(), shader_id: id.trim_start_matches("shader_").parse().map_err(|_| "bad shader id")? });
            }
            let set = fn_pak::PakSet::open_install(&install).map_err(|e| e.to_string())?;
            let text = fn_dxbc_cli::uigen::generate(&set, &entries)?;
            std::fs::write(&out, &text).map_err(|e| e.to_string())?;
            println!("wrote {} ({} bytes, {} materials)", out.display(), text.len(), entries.len());
        }
        "survey" => {
            // survey <dir-with-.dxbc>: in how many shaders each opcode occurs, per program type, with one example file
            let dir = PathBuf::from(positional.first().ok_or("survey <dir>")?.as_str());
            let mut stats: std::collections::BTreeMap<(String, &'static str), (usize, String)> = Default::default();
            let mut n = 0usize;
            for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
                let p = e.path();
                if p.extension().is_none_or(|x| x != "dxbc") { continue; }
                let Ok(bytes) = std::fs::read(&p) else { continue };
                let Ok(sh) = Shader::parse(&bytes) else { continue };
                n += 1;
                for (op, _) in fn_dxbc::analysis::opcode_histogram(&sh) {
                    let ent = stats.entry((sh.program_type.prefix().to_string(), op)).or_insert((0, p.display().to_string()));
                    ent.0 += 1;
                }
            }
            println!("{n} shaders");
            for ((pt, op), (c, ex)) in stats { println!("{pt} {op:<22} {c:>7}  e.g. {ex}"); }
        }
        "scan" | "scan-headers" => {
            let inst = positional.first().map(|s| PathBuf::from(s.as_str())).unwrap_or(install);
            let pkg = positional.get(1).ok_or("scan <install> <package>")?;
            let set = fn_pak::PakSet::open_install(&inst).map_err(|e| e.to_string())?;
            let bytes = fn_dxbc_cli::spec::package_bytes(&set, pkg)?;
            let blobs=fn_dxbc_cli::extract::scan_blobs(&bytes);
            if cmd=="scan-headers" {
                let entries=blobs.iter().map(|b|serde_json::json!({"id":b.id,"shader":Shader::parse(&b.bytes).map(|s|s.version_string()).ok(),"header":b.header,"trailer":b.trailer})).collect::<Vec<_>>();
                println!("{}",serde_json::to_string_pretty(&entries).map_err(|e|e.to_string())?);
                return Ok(());
            }
            for b in blobs {
                let kind = Shader::parse(&b.bytes).map(|s| s.version_string()).unwrap_or_else(|e| format!("error: {e}"));
                println!("shader_{} prefix={} bytes={} {kind}", b.id, b.prefix, b.bytes.len());
            }
        }
        _ => println!("{USAGE}"),
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}



