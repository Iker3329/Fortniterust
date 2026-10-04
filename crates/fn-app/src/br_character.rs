//! BR CHARACTER: scene-side glue for the real Battle Royale (Athena) character parts (`fn_pose::br_character`).
//!
//! The parts, their materials and textures are read straight from the 1.8 packages (the pak), so this works in both start-up modes (asset
//! tree + md5 clips, or paks): the parts follow the master pose by bone name exactly as the current body / head do. ANY failure (no paks,
//! a missing package, an incompatible skeleton) returns `None` and the loader keeps the current character; nothing here panics.
//! `FN_BR_CHARACTER=0` forces the current character (the pose-hash parity logs of the old runtime were recorded on it).
use crate::loader::Source;
use fn_pose::br_character::{BrCharacter, BrPart, SectionTextures, DEFAULT_HERO};
use fn_pose::{ClipSourceDir, Part};
use fn_render::{MaterialImages, MaterialKind, SkinnedPrimitive, SkinnedVertex};
use std::path::PathBuf;
use std::sync::Arc;

/// The offline lobby writes this value after a local Locker selection.  The
/// normal Rust game uses it too, so leaving the selector does not silently
/// revert the visible pawn to a different hero.  Only package paths that this
/// 1.8 install actually ships are accepted.
fn selected_offline_hero() -> String {
    const DEFAULT: &str = DEFAULT_HERO;
    let path = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
        .join("FortniteRuntime-rs").join("offline-lobby-profile.json");
    let Ok(raw) = std::fs::read_to_string(path) else { return DEFAULT.into() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else { return DEFAULT.into() };
    let Some(hero) = v.get("hero").and_then(|x| x.as_str()) else { return DEFAULT.into() };
    // A hero selection must be a regular Athena HID package.  Package opening
    // remains the authoritative validation; malformed or missing entries fall
    // back to DEFAULT in `load_for` below.
    if hero.starts_with("/Game/Athena/Heroes/HID_") && !hero.contains("..") { hero.to_owned() } else { DEFAULT.into() }
}

/// The outfit the lobby's Locker equipped: `loadout.outfit` of the offline profile (`lobby_profile.rs`: `fortnite-offline-profile.json` next to the executable) -> the `CID_*` item
/// definition -> its `HeroDefinition`. Falls back to the legacy `offline-lobby-profile.json` hero, then to [`DEFAULT_HERO`].
fn selected_hero(set: &fn_pak::PakSet) -> String {
    let profile = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.join("fortnite-offline-profile.json")));
    let id = profile.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("loadout").and_then(|l| l.get("outfit")).and_then(|o| o.as_str()).map(str::to_owned));
    if let Some(id) = id {
        if let Some(h) = fn_pose::cosmetics::Catalog::load(set).find(&id).and_then(|c| c.hero.clone()) { return h; }
    }
    selected_offline_hero()
}

/// Everything the loader needs to swap the player model.
pub struct BrModel {
    pub hero: String,
    /// `[body, head]` skeletons (master IK bones appended): becomes `CharacterRig::parts`.
    pub rig_parts: Vec<Part>,
    pub body: Vec<SkinnedPrimitive>,
    pub head: Vec<SkinnedPrimitive>,
    /// Human-readable load report (printed once): what came from the packages, what is missing / not applied.
    pub report: Vec<String>,
}

fn image(t: &Option<fn_pose::br_character::RgbaTexture>) -> Option<image::RgbaImage> {
    let t = t.as_ref()?;
    image::RgbaImage::from_raw(t.width as u32, t.height as u32, t.rgba.clone())
}

/// A fixed (non-parameter) texture of an original material for the renderer.
fn fixed(t: &fn_pose::br_material::FixedTex) -> Option<fn_render::char_material::FixedTexture> {
    use fn_render::char_material::{Addr, FixedTexture};
    let addr = |s: &str| match s { "TA_Clamp" => Addr::Clamp, "TA_Mirror" => Addr::Mirror, _ => Addr::Wrap };
    Some(FixedTexture { image: image::RgbaImage::from_raw(t.tex.width as u32, t.tex.height as u32, t.tex.rgba.clone())?, srgb: t.srgb, address: [addr(&t.address[0]), addr(&t.address[1])] })
}

/// The original compiled material of a section (head / hair / body skin shader, or the eyeshadow modulate overlay), when the section has one.
fn ue_material(prim: &fn_pose::SkinPrim, s: &SectionTextures) -> Option<Arc<fn_render::char_material::UeMaterial>> {
    use fn_render::char_material::{CharMaterial, CharPerm, EyeshadowOverlay, UeMaterial};
    if let Some(u) = &s.ue {
        let perm = match u.perm { "head" => CharPerm::Head, "hair" => CharPerm::Hair, _ => CharPerm::Body };
        return Some(Arc::new(UeMaterial::Char(CharMaterial { perm, cb: u.cb.clone(), noise: fixed(&u.noise)? })));
    }
    if let Some(e) = &s.eyeshadow {
        // vertex colour of the painted eyeshadow mask (a mesh without vertex colours supplies white)
        let colors: Vec<[u8; 4]> = if prim.colors.len() == prim.positions.len() { prim.colors.iter().map(|c| c.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8)).collect() } else { vec![[255; 4]; prim.positions.len()] };
        return Some(Arc::new(UeMaterial::Eyeshadow(EyeshadowOverlay { cb: e.cb.clone(), colors, tex0: fixed(&e.tex[0])?, tex1: fixed(&e.tex[1])?, tex2: fixed(&e.tex[2])? })));
    }
    None
}

/// Renderer primitives of a BR part: one per mesh section (hidden sections dropped), textures from the section's own material.
pub fn primitives(p: &BrPart) -> Vec<SkinnedPrimitive> {
    p.part.prims.iter().zip(&p.sections).filter(|(_, s): &(_, &SectionTextures)| s.hidden.is_none()).map(|(prim, s)| {
        let vertices = (0..prim.positions.len()).map(|i| SkinnedVertex { pos: prim.positions[i], normal: prim.normals[i], uv: prim.uvs[i], joints: prim.joints[i], weights: prim.weights[i] }).collect();
        let material = MaterialImages { diffuse: image(&s.diffuse), normal: image(&s.normal), spec: image(&s.spec), mask: image(&s.mask), ue: ue_material(prim, s) };
        SkinnedPrimitive { vertices, indices: prim.indices.clone(), material, kind: MaterialKind::Character }
    }).collect()
}

fn from_character(ch: &BrCharacter) -> BrModel {
    let mut report = vec![format!("hero {} -> {} part(s)", ch.hero, ch.parts.len())];
    for p in &ch.parts {
        let drawn = p.sections.iter().filter(|s| s.hidden.is_none()).count();
        report.push(format!("{:?} {} -> {} ({} of {} sections drawn, {} bones, overrides {:?})", p.def.kind, p.def.package, p.def.skeletal_mesh, drawn, p.sections.len(), p.part.tree.len(), p.def.material_overrides));
        for (i, s) in p.sections.iter().enumerate() {
            report.push(format!("  section {i}: {:?}{}", s.material, match &s.hidden { Some(h) => format!(" [not drawn: {h}]"), None => String::new() }));
        }
    }
    report.extend(ch.notes.iter().cloned());
    BrModel { hero: ch.hero.clone(), rig_parts: ch.rig_parts(), body: primitives(&ch.parts[0]), head: primitives(&ch.parts[1]), report }
}

/// Loads the BR character from the packages. `paks` is the mounted pak set of pak mode; in asset-tree mode a known Paks folder / install is
/// opened for this (meshes + textures only; the animations stay as they are). `None` = keep the current character.
pub fn load_for(source: &Source, paks: Option<&Arc<fn_pak::PakSet>>) -> Option<BrModel> {
    if std::env::var("FN_BR_CHARACTER").is_ok_and(|v| v == "0") { eprintln!("[br-character] disabled by FN_BR_CHARACTER=0: current character"); return None; }
    let own;
    let set: &fn_pak::PakSet = match paks {
        Some(p) => p,
        None => {
            let dir = crate::assets::candidate_paks(None).into_iter().next().or_else(|| { let d = fn_ui::pak_assets::default_install(); d.exists().then_some(d) });
            let Some(dir) = dir else { eprintln!("[br-character] no Paks folder found ({}): current character", match source { Source::Dir => "asset-tree mode", Source::Paks(_) => "pak mode" }); return None };
            match fn_pak::PakSet::open_install(&dir) { Ok(s) => { own = s; &own } Err(e) => { eprintln!("[br-character] cannot open the paks in {}: {e}: current character", dir.display()); return None } }
        }
    };
    let hero = selected_hero(set);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| BrCharacter::load(&ClipSourceDir::Pak(set), &hero)));
    match result {
        Ok(Ok(ch)) => {
            crate::part_anim_glue::install(&ch, &ClipSourceDir::Pak(set)); // PART ANIM HOOK
            let m = from_character(&ch);
            for l in &m.report { eprintln!("[br-character] {l}"); }
            Some(m)
        }
        Ok(Err(e)) if hero != DEFAULT_HERO => {
            eprintln!("[br-character] selected {hero} cannot load ({e}); falling back to {DEFAULT_HERO}");
            match BrCharacter::load(&ClipSourceDir::Pak(set), DEFAULT_HERO).map(|ch| from_character(&ch)) {
                Ok(m) => Some(m), Err(fallback) => { eprintln!("[br-character] default fallback cannot load: {fallback}"); None }
            }
        }
        Ok(Err(e)) => { eprintln!("[br-character] cannot load the BR character ({e}): current character"); None }
        Err(_) => { eprintln!("[br-character] the BR character loader panicked: current character"); None }
    }
}
