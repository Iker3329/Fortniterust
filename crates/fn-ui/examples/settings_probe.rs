//! Research tool for the 1.8 Settings screen: list the pak packages matching a glob, or dump a widget package's composed tree.
//! `cargo run --release -p fn-ui --example settings_probe -- glob "**/Settings/**/*.uasset"`
//! `cargo run --release -p fn-ui --example settings_probe -- tree /Game/UI/Frontend/Settings/OptionsMenu.OptionsMenu_C`
use fn_ui::screens::athena_lobby::store::Store;
use fn_ui::screens::athena_lobby::umg::Ui;
use std::rc::Rc;
fn show(store: &Store, v: &fn_ui::screens::athena_settings::kismet::Val) -> String {
    use fn_ui::screens::athena_settings::kismet::{resolve_text, Val};
    match v {
        Val::Text { src, key, .. } => format!("T{:?}{}", resolve_text(store, v).unwrap_or_else(|| format!("<locres miss {key}; src {src:?}>")), ""),
        Val::Array(a) => format!("[{}]", a.iter().map(|x| show(store, x)).collect::<Vec<_>>().join(", ")),
        o => format!("{o:?}"),
    }
}
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let install = fn_ui::pak_assets::default_install();
    let store = Rc::new(Store::open(&install).unwrap());
    match args.first().map(|s| s.as_str()) {
        Some("glob") => {
            let mut v: Vec<String> = store.pak_set().glob(&args[1]).into_iter().map(|(_, e)| fn_pak::pak_path_to_game(e.rel_path()).unwrap_or_else(|| e.path.clone())).collect();
            v.sort(); v.dedup();
            for p in v { println!("{p}"); }
        }
        Some("tree") => {
            let mut ui = Ui::new(store);
            let id = ui.add_class_instance(&args[1], None, "", serde_json::json!({}), None);
            println!("root = {id:?}; log = {:?}", ui.log);
            if let Some(id) = id { let mut s = String::new(); ui.dump(id, &mut s, 0, None); println!("{s}"); }
        }
        Some("row") => {
            // row <DataTable package> <export name> <row>: a decoded DataTable row as JSON
            let p = store.pkg(&args[1]).expect("pkg");
            let e = p.named(&args[2]).expect("export");
            println!("{}", serde_json::to_string_pretty(&e.props["Rows"][&args[3]]).unwrap());
        }
        Some("mat") => {
            // mat </Game/..Name.Name>: describe a Slate material (textures, shader permutations, parameters)
            let mut uic = fn_ui::scene::UiContext::from_install(&install, None).expect("ui context");
            match fn_ui::screens::athena_lobby::slate_mat::SlateMat::load(&store, &mut uic.assets, &args[1]) { Ok(m) => println!("{}", m.describe()), Err(e) => println!("ERR {e}") }
        }
        Some("ns") => { for (k, v) in store.locres().unwrap().namespace(&args[1]) { println!("{k} = {v:?}"); } }
        Some("memoffs") => {
            let st = store.function(&args[1], &args[2]).expect("fn");
            let mo = fn_ui::screens::athena_settings::kismet::mem_offsets(&st);
            let set: std::collections::HashSet<usize> = mo.iter().copied().collect();
            for (s, m) in st.iter().zip(mo.iter()) {
                use fn_uasset::kismet::Expr;
                let t = match &s.expr { Expr::Jump(t) => Some(*t as usize), Expr::JumpIfNot { target, .. } => Some(*target as usize), Expr::PushExecutionFlow(t) => Some(*t as usize), _ => None };
                if let Some(t) = t { if !set.contains(&t) { let near: Vec<&usize> = mo.iter().filter(|x| x.abs_diff(t) < 12).collect(); println!("BAD target {t} at mem {m}: near {near:?}"); } }
            }
            for (s, m) in st.iter().zip(mo) { println!("{:5} disk {:5} mem {:5}  {}", s.offset, s.offset, m, { let r = s.expr.render(); if r.len() > 110 { r[..110].to_string() } else { r } }); }
        }
        Some("run") => {
            // run <package> <function>...: execute the functions in order (BR / PC natives) and print the calls
            use fn_ui::screens::athena_settings::kismet::{Natives, Val, Vm};
            struct Pc;
            impl Natives for Pc { fn call(&self, f: &str, _: &[Val]) -> Option<Val> { match f { "IsAthena" | "IsInZone" => Some(Val::Bool(true)), "IsXboxPlatform" | "IsPS4Platform" | "ShouldShowSafeZoneOption" | "IsConsole" => Some(Val::Bool(false)), _ => None } } }
            let mut vm = Vm::new(&store, &args[1], &Pc);
            for f in &args[2..] {
                // `Name` or `Name:param=value,param=value` (values: true/false/int)
                let (n, ps) = f.split_once(':').unwrap_or((f, ""));
                let params: Vec<(String, Val)> = ps.split(',').filter(|s| !s.is_empty()).map(|p| { let (k, v) = p.split_once('=').unwrap(); (k.to_owned(), match v { "true" => Val::Bool(true), "false" => Val::Bool(false), i => Val::Int(i.parse().unwrap()) }) }).collect();
                vm.exec_with(n, None, params);
            }
            for c in &vm.run.calls {
                let a: Vec<String> = c.args.iter().map(|v| show(&store, v)).collect();
                let s = format!("{:5x} {}.{}({})", c.offset, c.on.as_deref().unwrap_or("self"), c.func, a.join(", "));
                println!("{}", if s.len() > 300 { format!("{}...", &s[..300]) } else { s });
            }
            for n in &vm.run.notes { println!("NOTE {n}"); }
        }
        Some("calls") => {
            // calls <package> <function>: the calls the function makes with resolved constant arguments
            let cs = fn_ui::screens::athena_settings::kismet::calls(&store, &args[1], &args[2]).expect("function not found");
            for c in cs {
                let a: Vec<String> = c.args.iter().map(|v| show(&store, v)).collect();
                println!("{:5x} {}.{}({})", c.offset, c.on.as_deref().unwrap_or("self"), c.func, a.join(", "));
            }
        }
        Some("allcalls") => {
            for f in store.function_names(&args[1]) {
                if f.starts_with("ExecuteUbergraph") { continue; }
                println!("== {f}");
                let Some(cs) = fn_ui::screens::athena_settings::kismet::calls(&store, &args[1], &f) else { continue };
                for c in cs {
                    let a: Vec<String> = c.args.iter().map(|v| show(&store, v)).collect();
                    let s = format!("{:5x} {}.{}({})", c.offset, c.on.as_deref().unwrap_or("self"), c.func, a.join(", "));
                    println!("{}", if s.len() > 320 { format!("{}...", &s[..320]) } else { s });
                }
            }
        }
        _ => eprintln!("usage"),
    }
}
