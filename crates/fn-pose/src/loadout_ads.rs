//! Reads the aim-down-sights data of the starter firearms from the install (`fn_weapons::recovered_ads` documents what is recovered and what is native):
//! the weapon actor Blueprint chain (`CameraBase3PClass`, `CameraTargeting3PClass`, `bUseScopeTargeting`, ...), the camera mode Blueprints with the class
//! defaults of their whole `super` chain, the `B_Ranged_Generic` scope timeline + curve, the sniper's `DownScopePostProcess` component, the scope material
//! instance / parent parameters, and the stat row's `*Downsights*` columns. Nothing is typed in here except the two `Delay` immediates of the OnSetTargeting
//! bytecode ([`SCOPE_START_DELAY`], [`SCOPE_STOP_DELAY`]).
use crate::{loadout_stats::data_table_row, ClipSourceDir};
use fn_weapons::recovered_ads::{AdsProfile, CameraModeData, OffsetSample, ReticleData, ScopeData, ViewBlend, DEFAULT_CORNER_ANGLES};
use serde_json::Value as Json;

/// `B_Ranged_Generic.OnSetTargeting` (ExecuteUbergraph, memory offset 7336): `call Delay(self, f0.2, ...)` in the bNewIsTargeting branch.
pub const SCOPE_START_DELAY: f32 = 0.2;
/// The same function's other branch: `call Delay(self, f0.1, ...)` before the timeline is reversed.
pub const SCOPE_STOP_DELAY: f32 = 0.1;
/// Package of the generic ranged weapon Blueprint (owner of the scope timeline, sounds and HUD tags; the starter guns inherit from it).
pub const RANGED_GENERIC: &str = "/Game/Weapons/Blueprints/B_Ranged_Generic";

fn pkg_of(p: &str) -> &str { p.split('.').next().unwrap_or(p) }
fn short(p: &str) -> &str { pkg_of(p).rsplit('/').next().unwrap_or(p) }

/// A parsed export set of one package as JSON, plus its super class path.
struct Doc { path: String, exports: Vec<Json>, sup: Option<String> }
impl Doc {
    fn export(&self, name: &str) -> Option<&Json> { self.exports.iter().find(|e| e["name"] == name).map(|e| &e["properties"]) }
    fn cdo(&self) -> Option<&Json> { self.exports.iter().find(|e| e["name"].as_str().is_some_and(|n| n.starts_with("Default__"))).map(|e| &e["properties"]) }
}
fn load_doc(src: &ClipSourceDir, long: &str) -> Result<Doc, String> {
    let long = pkg_of(long);
    let (asset, exp) = match src {
        ClipSourceDir::Pak(set) => (set.read_game(long, "uasset").map_err(|e| format!("{long}: {e}"))?.into_owned(), set.read_game(long, "uexp").map(|c| c.into_owned()).unwrap_or_default()),
        ClipSourceDir::RawPackages(root) => { let base = root.join("FortniteGame/Content").join(long.trim_start_matches("/Game/")); (std::fs::read(format!("{}.uasset", base.display())).map_err(|e| format!("{long}: {e}"))?, std::fs::read(format!("{}.uexp", base.display())).unwrap_or_default()) }
        ClipSourceDir::FnAnimDir(_) => return Err("a .fnanim directory has no Blueprints".into()),
    };
    let pkg = fn_uasset::Package::parse(&asset, &exp).map_err(|e| format!("{long}: {e}"))?;
    let mut exports = vec![];
    for i in 0..pkg.exports.len() {
        let o = pkg.read_export(i).map_err(|e| format!("{long}: {e}"))?;
        exports.push(o.to_json(&pkg));
    }
    let sup = pkg.super_class().map(|r| r.path().to_owned());
    Ok(Doc { path: long.to_owned(), exports, sup })
}
/// The Blueprint class chain, leaf first, ending before the first native (`/Script/...`) super class.
fn chain(src: &ClipSourceDir, leaf: &str) -> Result<Vec<Doc>, String> {
    let mut out = vec![]; let mut cur = Some(pkg_of(leaf).to_owned());
    while let Some(p) = cur.take() {
        if p.starts_with("/Script/") { break; }
        let d = load_doc(src, &p)?;
        cur = d.sup.clone();
        out.push(d);
        if out.len() > 16 { return Err(format!("{leaf}: super chain too deep")); }
    }
    Ok(out)
}
/// First value of `key` in the CDOs along the chain (null counts as unset).
fn inherited<'a>(chain: &'a [Doc], key: &str) -> Option<&'a Json> { chain.iter().filter_map(|d| d.cdo()).filter_map(|c| c.get(key)).find(|v| !v.is_null()) }

fn f(v: &Json) -> Option<f32> { v.as_f64().map(|x| x as f32) }
fn f3(v: &Json) -> Option<[f32; 3]> { let a = v.as_array()?; Some([f(a.first()?)?, f(a.get(1)?)?, f(a.get(2)?)?]) }
fn f4(v: &Json) -> Option<[f32; 4]> { let a = v.as_array()?; Some([f(a.first()?)?, f(a.get(1)?)?, f(a.get(2)?)?, f(a.get(3)?)?]) }
fn samples(v: &Json) -> Option<Vec<OffsetSample>> {
    let pts = v.get("SamplePoints")?.as_array()?;
    pts.iter().map(|p| Some((f(p.get("PitchAngle")?)?, f3(p.get("ViewOffset")?)?))).collect()
}
fn req<T>(o: Option<T>, what: &str) -> Result<T, String> { o.ok_or_else(|| format!("camera mode lacks {what}")) }

/// One camera mode Blueprint resolved over its class chain. The chain's native root defaults are not needed: `Athena_PlayerCameraModeBase` serializes every field
/// except `HidePawnPenetrationPercent` (native default 0, as `fn_camera::BASE` records).
pub fn camera_mode(src: &ClipSourceDir, class_path: &str) -> Result<CameraModeData, String> {
    let ch = chain(src, class_path)?;
    let g = |k: &str| inherited(&ch, k);
    let blend_field = |k: &str| ch.iter().filter_map(|d| d.cdo()).filter_map(|c| c.get("TransitionParams")).filter_map(|t| t.get(k)).find(|v| !v.is_null());
    let name = format!("{}", short(class_path));
    Ok(CameraModeData {
        chain: ch.iter().map(|d| short(&d.path).to_owned()).collect(),
        fov: req(g("FOV").and_then(f), &format!("FOV ({name})"))?,
        forward_distance: req(g("CameraSpaceForwardDistance").and_then(f), "CameraSpaceForwardDistance")?,
        origin_interp: req(g("CameraOriginInterpSpeed").and_then(f3), "CameraOriginInterpSpeed")?,
        vt_offset: req(g("ViewTargetSpaceViewOffset").and_then(samples), "ViewTargetSpaceViewOffset")?,
        vt_offset_crouch: req(g("ViewTargetSpaceViewOffset_Crouch").and_then(samples), "ViewTargetSpaceViewOffset_Crouch")?,
        vt_interp: req(g("ViewTargetSpaceViewOffseInterpSpeed").and_then(f3), "ViewTargetSpaceViewOffseInterpSpeed")?,
        cs_offset: req(g("CameraSpaceViewOffset").and_then(samples), "CameraSpaceViewOffset")?,
        push_out: req(g("CollisionPushOutDistance").and_then(f), "CollisionPushOutDistance")?,
        hide_pawn_pct: g("HidePawnPenetrationPercent").and_then(f).unwrap_or(0.),
        transition_in: req(g("TransitionTime").and_then(f), "TransitionTime")?,
        transition_out: req(g("TransitionOutTime").and_then(f), "TransitionOutTime")?,
        blend: ViewBlend { function: blend_field("BlendFunction").and_then(|v| v.as_str()).unwrap_or(ViewBlend::DEFAULT_FUNCTION).to_owned(), blend_time: blend_field("BlendTime").and_then(f), blend_exp: blend_field("BlendExp").and_then(f) },
    })
}

fn scope(src: &ClipSourceDir, actor_chain: &[Doc]) -> Result<ScopeData, String> {
    // the timeline lives in the first Blueprint of the chain that defines it (B_Ranged_Generic)
    let owner = actor_chain.iter().find(|d| d.export("AnimateScopePostProcess_Template").is_some()).ok_or("no AnimateScopePostProcess timeline in the weapon chain")?;
    let tl = owner.export("AnimateScopePostProcess_Template").unwrap();
    let length = req(tl.get("TimelineLength").and_then(f), "TimelineLength")?;
    let track = tl.get("FloatTracks").and_then(|t| t.as_array()).and_then(|a| a.first()).ok_or("timeline has no float track")?;
    if track["TrackName"] != "DownSightPostProcessAmount" { return Err(format!("unexpected timeline track {}", track["TrackName"])); }
    let curve_obj = track["CurveFloat"].as_str().ok_or("timeline track has no curve")?;
    let curve_name = curve_obj.rsplit(':').next().unwrap_or(curve_obj);
    let keys = owner.export(curve_name).and_then(|c| c.get("FloatCurve")).and_then(|c| c.get("Keys")).and_then(|k| k.as_array()).ok_or("scope curve keys missing")?;
    let mut curve = vec![];
    for k in keys {
        if k["InterpMode"].as_i64().unwrap_or(0) != 0 { return Err("scope curve key is not RCIM_Linear".into()); }
        curve.push((req(k.get("Time").and_then(f), "Time")?, req(k.get("Value").and_then(f), "Value")?));
    }
    // the sniper's PostProcessComponent (template named DownScopePostProcess_GEN_VARIABLE on the weapon Blueprint that overrides it)
    let pp_doc = actor_chain.iter().find(|d| d.export("DownScopePostProcess_GEN_VARIABLE").and_then(|p| p.get("Settings")).is_some()).ok_or("weapon chain has no DownScopePostProcess_GEN_VARIABLE settings")?;
    let settings = pp_doc.export("DownScopePostProcess_GEN_VARIABLE").and_then(|p| p.get("Settings")).unwrap();
    let material = settings.pointer("/WeightedBlendables/Array/0/Object").and_then(|v| v.as_str()).ok_or("scope post-process has no blendable material")?.to_owned();
    // parent post-process settings (B_Ranged_Generic) supply the vignette / fringe when the sniper does not override them
    let generic_pp = actor_chain.iter().rev().find_map(|d| d.export("DownScopePostProcess_GEN_VARIABLE")).and_then(|p| p.get("Settings"));
    let setting = |k: &str| settings.get(k).and_then(f).or_else(|| generic_pp.and_then(|g| g.get(k)).and_then(f));
    // the material instance and its parent
    let mi = load_doc(src, &material)?;
    let mip = mi.export(short(&material)).ok_or("material instance export missing")?;
    // The sniper binds a MaterialInstanceConstant (Parent + a `Reticle` texture override). A weapon that inherits `B_Ranged_Generic`'s own template (the Triple Shot) points the blendable at the BASE
    // Material `DownScopePostProcess` itself: no Parent, the `Reticle` default is the `Texture` of its `MaterialExpressionTextureSampleParameter2D` named Reticle; `parent_material` then names the material itself.
    let (parent, reticle, pm) = match mip.get("Parent").and_then(|v| v.as_str()) {
        Some(parent) => {
            let reticle = mip.get("TextureParameterValues").and_then(|a| a.as_array()).and_then(|a| a.iter().find(|p| p["ParameterName"] == "Reticle")).and_then(|p| p["ParameterValue"].as_str()).ok_or("scope material instance does not bind the Reticle texture")?.to_owned();
            (parent.to_owned(), reticle, load_doc(src, parent)?)
        }
        None => {
            let reticle = mi.exports.iter().filter(|e| e["class"] == "MaterialExpressionTextureSampleParameter2D" && e["properties"]["ParameterName"] == "Reticle").find_map(|e| e["properties"]["Texture"].as_str()).ok_or("scope base material has no Reticle texture parameter")?.to_owned();
            (material.clone(), reticle, load_doc(src, &material)?)
        }
    };
    let (mut scalars, mut vectors) = (vec![], vec![]);
    for e in &pm.exports {
        let class = e["class"].as_str().unwrap_or_default();
        let p = &e["properties"];
        let Some(name) = p.get("ParameterName").and_then(|v| v.as_str()) else { continue };
        if class == "MaterialExpressionScalarParameter" { if let Some(v) = p.get("DefaultValue").and_then(f) { scalars.push((name.to_owned(), v)); } }
        if class == "MaterialExpressionVectorParameter" { if let Some(v) = p.get("DefaultValue").and_then(f4) { vectors.push((name.to_owned(), v)); } }
    }
    for s in mip.get("ScalarParameterValues").and_then(|a| a.as_array()).into_iter().flatten() {
        if let (Some(n), Some(v)) = (s["ParameterName"].as_str(), s.get("ParameterValue").and_then(f)) { match scalars.iter_mut().find(|(k, _)| k == n) { Some(e) => e.1 = v, None => scalars.push((n.to_owned(), v)) } }
    }
    for s in mip.get("VectorParameterValues").and_then(|a| a.as_array()).into_iter().flatten() {
        if let (Some(n), Some(v)) = (s["ParameterName"].as_str(), s.get("ParameterValue").and_then(f4)) { match vectors.iter_mut().find(|(k, _)| k == n) { Some(e) => e.1 = v, None => vectors.push((n.to_owned(), v)) } }
    }
    let tags = inherited(actor_chain, "ReticleHUDElementTags").and_then(|t| t.get("GameplayTags")).and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).ok_or("ReticleHUDElementTags missing")?;
    let text = |k: &str| inherited(actor_chain, k).and_then(|v| v.as_str()).map(str::to_owned).ok_or_else(|| format!("{k} missing"));
    let scope_mesh = actor_chain.iter().find_map(|d| d.export("ScopeMesh1P_GEN_VARIABLE")).and_then(|p| p.get("StaticMesh")).and_then(|v| v.as_str()).ok_or("ScopeMesh1P has no static mesh")?.to_owned();
    Ok(ScopeData {
        timeline_length: length, curve, start_delay: SCOPE_START_DELAY, stop_delay: SCOPE_STOP_DELAY,
        material, parent_material: parent, reticle_texture: reticle, scalars, vectors,
        reticle_hud_tags: tags, sound_zoom_in: text("Sound_ScopeZoomIn")?, sound_zoom_out: text("Sound_ScopeZoomOut")?,
        vignette_intensity: setting("VignetteIntensity"), scene_fringe_intensity: setting("SceneFringeIntensity"),
        scope_camera_offset_amount: req(inherited(actor_chain, "Scope Camera Offset Amount").and_then(f), "Scope Camera Offset Amount")?,
        scope_mesh,
    })
}

/// The ADS profile of one weapon item definition (`WID_*`).
pub fn profile(src: &ClipSourceDir, item_path: &str) -> Result<AdsProfile, String> {
    let wid = load_doc(src, item_path)?;
    let w = wid.cdo().or_else(|| wid.exports.first().map(|e| &e["properties"])).ok_or("WID has no properties")?;
    // the WID export is not a CDO in the pak: find the item definition export
    let w = wid.exports.iter().find(|e| e["class"].as_str().is_some_and(|c| c.contains("ItemDefinition"))).map(|e| &e["properties"]).unwrap_or(w);
    let actor = w.get("WeaponActorClass").and_then(|v| v.as_str()).ok_or("WID lacks WeaponActorClass")?.to_owned();
    let handle = w.get("WeaponStatHandle").ok_or("WID lacks WeaponStatHandle")?;
    let (table, row) = (handle["DataTable"].as_str().ok_or("stat handle lacks DataTable")?, handle["RowName"].as_str().ok_or("stat handle lacks RowName")?);
    let ach = chain(src, &actor)?;
    let class_of = |k: &str| inherited(&ach, k).and_then(|v| v.as_str()).map(str::to_owned).ok_or_else(|| format!("{actor}: {k} missing"));
    let base = camera_mode(src, &class_of("CameraBase3PClass")?)?;
    let targeting = camera_mode(src, &class_of("CameraTargeting3PClass")?)?;
    let flag = |k: &str| inherited(&ach, k).and_then(|v| v.as_bool()).unwrap_or(false);
    let use_scope = flag("bUseScopeTargeting");
    let r = data_table_row(src, table, row)?;
    let num = |k: &str| r.float(k).map(|v| v as f32).ok_or_else(|| format!("stat row {row} lacks {k}"));
    Ok(AdsProfile {
        item: item_path.to_owned(), actor, base, targeting, use_scope,
        scope: if use_scope { Some(scope(src, &ach)?) } else { None },
        spread_downsights: num("SpreadDownsights")?, spread_downsights_cooldown: num("SpreadDownsightsAdditionalCooldownTime")?,
        recoil_downsights_multiplier: num("RecoilDownsightsMultiplier")?, athena_recoil_downsights_multiplier: num("AthenaRecoilDownsightsMultiplier")?,
        maintain_aim_location: flag("bMaintainAimLocationDuringTargeting"), aim_from_muzzle_at_close_range: flag("bShouldAimFromMuzzleAtCloseRange"),
        reticle: reticle(w, &ach),
    })
}

/// File name (`<stem>.png`) of a texture reference as the property JSON prints it: a long object path (`/Game/UI/X.X`) or an import reference (`X#212`).
fn tex_file(v: &Json) -> Option<String> {
    let s = v.as_str()?;
    if s.is_empty() || s == "None" { return None; }
    let stem = s.rsplit('/').next()?.split(['.', '#']).next()?;
    let stem = stem.rsplit(':').next()?;
    (!stem.is_empty()).then(|| format!("{stem}.png"))
}

/// `SFortReticle` inputs of one weapon: WID first (`ReticleImage`, `ReticleCenterImage`, `HitNotifyImage`, `ReticleCornerAngles`, `bReticleCornerOutsideSpreadRadius`),
/// the weapon actor chain as the fallback for the three textures (`ReticleImage`, `ReticleCenterImage`, `HitNotifyReticleImage`), the native ctor default for the angles.
fn reticle(wid: &Json, actor_chain: &[Doc]) -> ReticleData {
    let tex = |wid_key: &str, actor_key: &str| wid.get(wid_key).and_then(tex_file).or_else(|| inherited(actor_chain, actor_key).and_then(tex_file));
    let corner_angles = match wid.get("ReticleCornerAngles").and_then(|v| v.as_array()) {
        Some(a) => a.iter().filter_map(f).collect(),
        None => DEFAULT_CORNER_ANGLES.to_vec(),
    };
    ReticleData {
        image: tex("ReticleImage", "ReticleImage"), center_image: tex("ReticleCenterImage", "ReticleCenterImage"), hit_image: tex("HitNotifyImage", "HitNotifyReticleImage"), muzzle_blocked_image: tex("MuzzleBlockedImage", "MuzzleBlockedReticleImage"), hit_duration: wid.get("HitNotifyDuration").and_then(f).unwrap_or(1.),
        corner_angles, corner_outside_spread_radius: wid.get("bReticleCornerOutsideSpreadRadius").and_then(|v| v.as_bool()).unwrap_or(false),
    }
}

/// Profiles of the starter firearms (`crate::loadout::ITEMS[..3]`), in order.
pub fn load(src: &ClipSourceDir, items: &[&str]) -> Result<Vec<AdsProfile>, String> { items.iter().map(|i| profile(src, i)).collect() }

#[cfg(test)]
mod tests {
    use super::*;
    fn set() -> fn_pak::PakSet { let root = fn_live::var_os("FN_TEST_ROOT").expect("set FN_TEST_ROOT to the local 1.8 install"); fn_pak::PakSet::open_install(root).unwrap() }
    #[test]
    fn starter_firearms_resolve_to_the_recovered_camera_and_scope_data() {
        let set = set(); let src = ClipSourceDir::Pak(&set);
        let ps = load(&src, &crate::loadout::ITEMS[..3]).unwrap();
        // AR: hold camera Ranged, ADS camera TargetingAssault (a CDO with no overrides: everything inherited from RangedTargeting)
        let ar = &ps[0];
        assert!(ar.base.chain[0].ends_with("Ranged") && ar.targeting.chain[0].ends_with("TargetingAssault"));
        assert!(ar.targeting.chain.iter().any(|c| c == "Athena_PlayerCameraModeRangedTargeting") && ar.targeting.chain.last().unwrap() == "Athena_PlayerCameraModeBase");
        assert_eq!((ar.targeting.fov, ar.targeting.forward_distance, ar.targeting.transition_in, ar.targeting.transition_out), (80., -160., 0.2, 0.2));
        assert_eq!(ar.targeting.vt_offset, vec![(0., [83., 31., 70.])]); assert_eq!(ar.targeting.vt_offset_crouch, vec![(0., [83., 31., 30.])]);
        assert_eq!(ar.targeting.origin_interp[0], 60.); assert_eq!(ar.targeting.vt_interp, [8.; 3]); assert_eq!(ar.targeting.hide_pawn_pct, 0.35);
        assert!(!ar.use_scope && ar.scope.is_none() && ar.maintain_aim_location);
        // shotgun: own CameraSpaceViewOffset override
        let sg = &ps[1];
        assert_eq!(sg.targeting.cs_offset[1], (90., [20., 0., 115.])); assert_eq!(sg.targeting.forward_distance, -160.);
        // sniper: scope camera FOV 15 and the TransitionParams override; hold camera = Athena_PlayerCameraModeSniper
        let sn = &ps[2];
        assert!(sn.use_scope); assert!(sn.base.chain[0].ends_with("Sniper"));
        assert_eq!((sn.targeting.fov, sn.targeting.forward_distance), (15., -90.));
        assert_eq!(sn.targeting.vt_offset, vec![(0., [83., 15., 50.])]); assert_eq!(sn.targeting.vt_offset_crouch, vec![(0., [83., 15., 15.])]);
        assert_eq!((sn.targeting.blend.function.as_str(), sn.targeting.blend.blend_time, sn.targeting.blend.blend_exp), ("VTBlend_EaseInOut", Some(0.1), Some(5.)));
        assert_eq!((sn.targeting.transition_in, sn.targeting.transition_out), (0.25, 0.25));
        let sc = sn.scope.as_ref().unwrap();
        assert_eq!(sc.timeline_length, 0.3); assert_eq!(sc.curve, vec![(0., 0.), (0.15, 0.), (0.2, 1.), (0.3, 1.)]);
        assert!(sc.reticle_texture.ends_with("HUD_SniperCrossHairs_v1_Athena.HUD_SniperCrossHairs_v1_Athena"), "{}", sc.reticle_texture);
        assert_eq!(sc.reticle_hud_tags, ["HUD.Center.Reticle", "HUD.InWorld.ActorCanvas"]);
        assert!(sc.scalars.iter().any(|(n, v)| n == "RefractionAmount" && (*v + 0.1).abs() < 1e-6) && sc.vectors.iter().any(|(n, v)| n == "Reticle Color" && *v == [0., 0., 0., 1.]));
        assert_eq!(sc.scope_camera_offset_amount, 30.);
        assert!((sn.recoil_downsights_multiplier - 0.4).abs() < 1e-6 && sn.spread_downsights == 0.);
    }
    #[test]
    #[ignore]
    fn dump_wid_reticle() {
        let set = set(); let src = ClipSourceDir::Pak(&set);
        for it in crate::loadout::ITEMS.iter().take(3) {
            let p = profile(&src, it).unwrap();
            eprintln!("RETICLE {it} {:?}", p.reticle);
            eprintln!("MODES {it} base in {} out {} blend {:?} | target in {} out {} blend {:?} chain {:?}", p.base.transition_in, p.base.transition_out, p.base.blend, p.targeting.transition_in, p.targeting.transition_out, p.targeting.blend, p.targeting.chain);
        }
        for pat in ["*Targeting*", "*Aiming*", "*Downsight*", "*ADS*"] { for (_, e) in set.glob(pat) { let p = e.rel_path(); if p.ends_with(".uasset") && (p.contains("Athena") || p.contains("Abilit") || p.contains("Effect")) { eprintln!("PAK {p}"); } } }
    }
    #[test]
    fn camera_mode_chain_resolution_reproduces_the_pickaxe_mode_the_runtime_already_uses() {
        let set = set(); let src = ClipSourceDir::Pak(&set);
        let m = camera_mode(&src, "/Game/Blueprints/Camera/Athena/Athena_PlayerCameraModeMelee").unwrap();
        // fn_camera::MELEE (verified against the extracted report there)
        assert_eq!((m.fov, m.forward_distance, m.origin_interp[0], m.origin_interp[2], m.hide_pawn_pct, m.push_out, m.transition_in), (80., -280., 30., 60., 0.15, 0.2, 0.45));
        assert_eq!(m.vt_offset, vec![(0., [38., 31., 70.])]); assert_eq!(m.vt_interp, [8.; 3]);
        assert_eq!(m.cs_offset, vec![(0., [0.; 3]), (90., [30., 0., 100.]), (-90., [50., 0., -20.])]);
    }
}
