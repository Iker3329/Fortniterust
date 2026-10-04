//! Esc-menu material shaders (port of tools/menu-shaders.cjs): per role, slice the compiled DXBC pixel shaders out of the owning
//! material, pick the plain and the Slate permutation, and evaluate the serialised uniform expression set with every instance's
//! effective parameters.  Output: `<role>.plain.asm`, `<role>.slate.asm` and `cbuffers.json` in the layout the old tool wrote.

use crate::extract::scan_blobs;
use crate::spec::package_bytes;
use crate::uniform::{evaluate, package_names, parse_set, Overrides};
use fn_dxbc::disasm::disassemble;
use fn_pak::PakSet;
use serde_json::{json, Map, Value};
use std::path::Path;

pub struct Role { pub name: &'static str, pub owner: &'static str, pub note: &'static str, pub instances: &'static [&'static str] }

pub const ROLES: [Role; 4] = [
    Role { name: "outline", owner: "/Game/UI/Foundation/Materials/M_MenuButton_Master", note: "ButtonStyle-Outline / OutlineLeftShade / Outline-Red (text + icon buttons of the Esc menu)",
        instances: &["M-Button-Outline-Base_Fill", "M-Button-Outline-Hovered", "M-Button-Outline-Pressed", "M-Button-Outline-Disabled", "M-Button-Outline-Base_Red", "M-Button-Outline-Pressed_Red",
            "M-Button-OutlineLeftShade-Base", "M-Button-OutlineLeftShade-Hovered", "M-Button-OutlineLeftShade-Pressed", "M-Button-OutlineLeftShade-Disabled", "M-Button-Outline-LeftShade-Base_Red", "M-Button-Outline-LeftShade-Pressed_Red"] },
    Role { name: "primary", owner: "/Game/UI/Foundation/Materials/M-Button-Master", note: "ButtonStyle-Primary-M (ConfirmationWindow buttons)",
        instances: &["M-Button-Primary-M-Base", "M-Button-Primary-M-Hovered", "M-Button-Primary-M-Pressed", "M-Button-Primary-M-Disabled"] },
    Role { name: "gradientPanel", owner: "/Game/UI/Foundation/Materials/M_UI_GradientPanel", note: "Border-MainModal / Border-ModalHeader (ConfirmationWindow panel + header)",
        instances: &["M-Border-MainModal", "M-Border-ModalHeader-procedural"] },
    Role { name: "shellUberBG", owner: "/Game/UI/Foundation/Border/Materials/M-Border-ShellUberBG", note: "TopBar.MainMenuBorder backdrop (sphere-gradient static permutation of M_UI_GradientPanel)",
        instances: &["M-Border-ShellUberBG"] },
];

/// Drops the d3dcompiler boilerplate comments, keeping the signature table (what tools/menu-shaders.cjs `small()` did).
pub fn small(asm: &str) -> String {
    let mut out = String::new();
    // the old pipeline read the listing with its UTF-8 BOM attached, so the regexp never matched (and removed) the very first line
    for (n, line) in asm.split_inclusive('\n').enumerate() {
        let keep = n == 0 || !line.starts_with("//") || ["Name", "---", "SV_", "TEXCOORD", "COLOR", "ORIGINAL"].iter().any(|k| line.contains(k));
        if keep { out.push_str(line); }
    }
    format!("{}\n", out.trim())
}

/// Input registers of the pixel shader as the old regex saw them: `dcl_input_ps <one-word mode> v<one digit>.<mask>`.
fn ps_inputs(asm: &str) -> String {
    let mut regs = vec![];
    for l in asm.lines() {
        let t: Vec<&str> = l.split_whitespace().collect();
        if t.len() == 3 && t[0] == "dcl_input_ps" {
            let b = t[2].as_bytes();
            if b.len() >= 4 && b[0] == b'v' && b[1].is_ascii_digit() && b[2] == b'.' { regs.push(format!("v{}", b[1] as char)); }
        }
    }
    regs.join(",")
}
fn matches_regs(inputs: &str, allowed: impl Fn(&str) -> bool) -> bool { !inputs.is_empty() && inputs.split(',').all(allowed) }

pub fn effective(widgets: &Value, instance: &str) -> Option<(String, Overrides)> {
    let mats = widgets.get("materials")?.as_object()?;
    let key = mats.keys().find(|k| k.ends_with(&format!("/{instance}")))?.clone();
    let eff = mats[&key].get("effective")?;
    let mut ov = Overrides::default();
    if let Some(v) = eff.get("vectors").and_then(|v| v.as_object()) {
        for (k, a) in v { if let Some(a) = a.as_array() { let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32; ov.vectors.insert(k.clone(), [g(0), g(1), g(2), g(3)]); } }
    }
    if let Some(s) = eff.get("scalars").and_then(|v| v.as_object()) {
        for (k, x) in s { if let Some(x) = x.as_f64() { ov.scalars.insert(k.clone(), x as f32); } }
    }
    Some((key, ov))
}

pub fn run(set: &PakSet, widgets: &Value, out: &Path) -> Result<Value, String> {
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut roles = Map::new();
    let mut materials = Map::new();
    for role in &ROLES {
        let uexp = package_bytes(set, role.owner)?;
        let uasset = set.read_game(role.owner, "uasset").map_err(|e| e.to_string())?.into_owned();
        let blobs = scan_blobs(&uexp);
        // (file name, listing) of every blob, ordered like the old directory listing
        let mut asms: Vec<(String, String)> = blobs.iter().filter_map(|b| disassemble(&b.bytes).ok().map(|t| (format!("shader_{}.asm", b.id), t))).collect();
        asms.sort_by(|a, b| a.0.cmp(&b.0));
        let ps: Vec<&(String, String)> = asms.iter().filter(|(_, t)| t.lines().any(|l| l.trim() == "ps_5_0")).collect();
        let pick = |pred: &dyn Fn(&str, &str) -> bool| -> Option<&(String, String)> {
            let mut c: Vec<&&(String, String)> = ps.iter().filter(|(_, t)| pred(&ps_inputs(t), t)).collect();
            c.sort_by_key(|(_, t)| t.chars().count());
            c.first().map(|x| **x)
        };
        let plain = pick(&|inp, t| matches_regs(inp, |r| matches!(r, "v2" | "v4" | "v5")) && !t.contains("dcl_output o1"));
        let slate = pick(&|inp, t| {
            let ok = inp == "v1,v3,v4,v8" || inp == "v1,v3,v4,v6,v8";
            ok && !t.contains("dcl_output o1") && !t.contains("sample_indexable") && !t.contains("0.590000")
        });
        if let Some((_, t)) = plain { std::fs::write(out.join(format!("{}.plain.asm", role.name)), small(t)).map_err(|e| e.to_string())?; }
        if let Some((_, t)) = slate { std::fs::write(out.join(format!("{}.slate.asm", role.name)), small(t)).map_err(|e| e.to_string())?; }
        let names = package_names(&uasset, &uexp)?;
        let us = parse_set(&uexp, &names).map_err(|e| format!("{}: {e}", role.owner))?;
        roles.insert(role.name.into(), json!({
            "owner": role.owner, "note": role.note, "shaderBlobs": blobs.len(),
            "plainPermutation": plain.map(|p| p.0.clone()), "slatePermutation": slate.map(|p| p.0.clone()),
            "vectors": us.vectors.len(), "scalars": us.scalars.len(),
        }));
        for inst in role.instances {
            let Some((key, ov)) = effective(widgets, inst) else { eprintln!("no effective parameters for {inst}"); continue };
            let ev = evaluate(&us, &ov, 0.0);
            let mut slots = Map::new();
            let mut derived = Map::new();
            for (reg, label) in &ev.labels {
                let (i, comp) = match reg.strip_prefix("cb[").and_then(|r| r.split_once(']')) { Some((i, c)) => (i.parse::<usize>().unwrap(), c.strip_prefix('.')), None => continue };
                let value: Value = match comp { Some(c) => json!(ev.cb[i]["xyzw".find(c).unwrap()]), None => json!(ev.cb[i]) };
                if !label.contains('(') { slots.insert(label.clone(), json!({"reg": reg, "value": value})); }
                else {
                    let expr = if label.chars().count() > 160 { format!("{}...", label.chars().take(157).collect::<String>()) } else { label.clone() };
                    derived.insert(reg.clone(), json!({"expression": expr, "value": value}));
                }
            }
            materials.insert((*inst).into(), json!({"package": key, "role": role.name, "registers": ev.registers, "cb": ev.cb, "slots": slots, "derived": derived}));
        }
    }
    let result = json!({"tool": "fn-dxbc menu-shaders", "encoding": "UE4.16 cooked material; see fn-dxbc-cli/src/uniform.rs for the uniform expression serialization", "roles": roles, "materials": materials});
    std::fs::write(out.join("cbuffers.json"), serde_json::to_string_pretty(&result).unwrap()).map_err(|e| e.to_string())?;
    Ok(result)
}

