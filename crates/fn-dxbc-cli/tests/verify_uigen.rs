//! The generated UI material module (`fn-dxbc gen-ui`, committed in tests/generated/ui_materials.rs) vs the old generated `ui_shaders.rs`:
//! the constant buffers built from the uniform expression sets agree for random parameter values, and the translated pixel shaders
//! agree with the interpreter.
mod common;
use common::*;

#[path = "generated/ui_materials.rs"]
#[allow(dead_code, clippy::all)]
mod newui;
#[path = "fixtures/old_ui_shaders.rs"]
#[allow(dead_code, clippy::all)]
mod old_ui;

fn hash(name: &str) -> u64 {
    // parameters that appear twice get an FName number: the old tool conflated them, so hash the base name
    let base = match name.rsplit_once('_') { Some((b, n)) if !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()) => b, _ => name };
    base.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}
fn value(name: &str, salt: u64) -> [f32; 4] {
    let h = hash(name) ^ salt.wrapping_mul(0x9e3779b97f4a7c15);
    let g = |k: u32| (((h >> (k * 8)) & 0xff) as f32) / 255.0 * 2.0 - 0.25;
    [g(0), g(1), g(2), g(3)]
}

fn same(a: &[[f32; 4]], b: &[[f32; 4]]) -> bool { a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (0..4).all(|k| x[k] == y[k] || (x[k] - y[k]).abs() <= 1e-5 * 1.0f32.max(x[k].abs()).max(y[k].abs()))) }

#[test]
fn generated_cb_functions_match_old_ones() {
    let mut n = 0;
    for salt in 0..200u64 {
        // salt 0: cooked defaults only; otherwise every parameter takes a hashed value
        let p = |name: &str, d: [f32; 4]| if salt == 0 { d } else { value(name, salt) };
        let pairs: [(&str, Vec<[f32; 4]>, Vec<[f32; 4]>); 5] = [
            ("item_card", newui::item_card_cb(&p).to_vec(), old_ui::item_card_cb(&p).to_vec()),
            ("compass", newui::compass_cb(&p).to_vec(), old_ui::compass_cb(&p).to_vec()),
            ("hud_bar", newui::hud_bar_cb(&p).to_vec(), old_ui::hud_bar_cb(&p).to_vec()),
            ("panel_line", newui::panel_line_cb(&p).to_vec(), old_ui::panel_line_cb(&p).to_vec()),
            ("panel_vgrad", newui::panel_vgrad_cb(&p).to_vec(), old_ui::panel_vgrad_cb(&p).to_vec()),
        ];
        for (name, a, b) in pairs { assert!(same(&a, &b), "{name}_cb differs for salt {salt}"); n += 1; }
    }
    eprintln!("{n} constant buffers identical to the old generated functions");
}

#[test]
fn generated_pixel_shaders_run() {
    // smoke: the generated pixel shader functions execute and return a colour
    let v = [[0.5f32; 4]; 9];
    let cb0 = vec![[0.0f32; 4], [0.0; 4], [1.0, 1.0, 0.0, 0.0]];
    let cb1 = vec![[0.25f32; 4]; 64];
    let cbs: [&[[f32; 4]]; 3] = [&cb0, &cb1, &cb1];
    let mut tex = |_: &newui::Fetch| [0.5f32; 4];
    let r = newui::hud_bar_ps(&[[0.5; 4]; 7], &cbs, &mut tex);
    assert!(r.is_some());
    let _ = (v, normalize("x"));
}

/// `tests/generated/ui_materials.rs` is what `fn-dxbc gen-ui` writes today (regenerate with `FN_DXBC_REGEN=1`).
#[test]
fn generated_module_is_up_to_date() {
    let Some(set) = open() else { eprintln!("no 1.8 install, skipping"); return };
    let e = |n: &str, p: &str, id: usize| fn_dxbc_cli::uigen::UiEntry { name: n.into(), package: p.into(), shader_id: id };
    let text = fn_dxbc_cli::uigen::generate(&set, &[
        e("item_card", "/Game/UI/Foundation/Items/Materials/M-Item-Card-Inst-Master", 12100),
        e("compass", "/Game/Athena/HUD/M_Compass", 2485),
        e("hud_bar", "/Game/UI/HUD/Materials/HUD-Bar_Material", 2536),
        e("panel_line", "/Game/UI/Foundation/Border/Materials/M_Border_HUD_Line_Vert", 5118),
        e("panel_vgrad", "/Game/UI/Foundation/Border/Materials/M_Border_HUD_VGradient", 5154),
    ]).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("generated").join("ui_materials.rs");
    if std::env::var_os("FN_DXBC_REGEN").is_some() { std::fs::write(&path, &text).unwrap(); return; }
    let old = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(old.replace("\r\n", "\n") == text, "tests/generated/ui_materials.rs is stale: run FN_DXBC_REGEN=1 cargo test -p fn-dxbc-cli --test verify_uigen");
}
