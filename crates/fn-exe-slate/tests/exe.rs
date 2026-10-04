//! Tests against the real 1.8 client EXE (read-only). They skip with a note when the exe is not on this machine (set `FN_EXE`).
use fn_exe_slate::findings::{self, Findings};
use fn_exe_slate::*;
use std::collections::HashSet;
use std::sync::OnceLock;

struct Ctx { pe: Pe, idx: XrefIndex, f: Findings }
fn ctx() -> Option<&'static Ctx> {
    static C: OnceLock<Option<Ctx>> = OnceLock::new();
    C.get_or_init(|| {
        let p = exe_path();
        if !p.exists() { eprintln!("skipping: {} not found (set FN_EXE)", p.display()); return None; }
        let pe = Pe::open(&p).expect("parse PE");
        let idx = XrefIndex::build(&pe);
        let f = findings::recover(&pe, &idx);
        Some(Ctx { pe, idx, f })
    }).as_ref()
}

// ------------------------------------------------------------------------------------------------ PE mapping

#[test]
fn pe_headers_and_sections() {
    let Some(c) = ctx() else { return };
    assert_eq!(c.pe.image_base, 0x1_4000_0000);
    let rdata = c.pe.section_named(".rdata").expect(".rdata");
    assert_eq!((rdata.rva, rdata.raw_off), (0x03fa_5000, 0x03fa_3800), "the section the user's offsets live in");
    assert!(c.pe.section_named(".text").unwrap().is_exec());
    assert!(c.pe.funcs.len() > 200_000, ".pdata parsed: {}", c.pe.funcs.len());
}

/// The user's FILE offsets map to the strings they named (ASCII or UTF-16LE, both probed).
#[test]
fn user_file_offsets_map_to_the_named_strings() {
    let Some(c) = ctx() else { return };
    let expect: &[(&str, u64, &str, &str)] = &[
        ("SInitialFortniteLoadingScreen", 0x44B7AB8, "ascii", "SInitialFortniteLoadingScreen"),
        ("FortniteLoadingScreen.cpp", 0x44B7B20, "ascii", r"D:\Build\++Fortnite+Release-Live+Full\Sync\FortniteGame\Source\FortniteLoadingScreen\Private\FortniteLoadingScreen.cpp"),
        ("FORT_Load_Atlas", 0x44B73A0, "utf16", "/Game/UI/Foundation/Textures/LoadingScreens/FORT_Load_Atlas.FORT_Load_Atlas"),
        ("T-Loading-EarlyAccessPlate", 0x44B7440, "utf16", "/Game/UI/Foundation/Textures/LoadingScreens/T-Loading-EarlyAccessPlate.T-Loading-EarlyAccessPlate"),
        ("Burbank", 0x44B7510, "utf16", "/Game/UI/Foundation/Fonts/Burbank.Burbank"),
        ("ProgressBar_Background", 0x44B7570, "utf16", "/Game/UI/CodeReferenced/ProgressBar_Background.ProgressBar_Background"),
        ("ProgressBar_Fill", 0x44B7600, "utf16", "/Game/UI/CodeReferenced/ProgressBar_Fill.ProgressBar_Fill"),
        ("ProgressBar_Marquee", 0x44B7680, "utf16", "/Game/UI/CodeReferenced/ProgressBar_Marquee.ProgressBar_Marquee"),
        ("EARLY ACCESS", 0x44B7B00, "utf16", "EARLY ACCESS"),
        ("LOADING", 0x44B7C18, "utf16", "LOADING"),
        ("SFortLoadingScreen", 0x4AED1F8, "ascii", "SFortLoadingScreen"),
        ("SFortLoadingScreen.cpp", 0x4B3F980, "ascii", r"D:\Build\++Fortnite+Release-Live+Full\Sync\FortniteGame\Source\FortniteUI\Private\SFortLoadingScreen.cpp"),
        ("FortLoadingScreenManager.cpp", 0x412D5F0, "ascii", r"D:\Build\++Fortnite+Release-Live+Full\Sync\FortniteGame\Source\FortniteGame\Private\FortLoadingScreenManager.cpp"),
    ];
    assert_eq!(expect.len(), USER_STRING_OFFSETS.len());
    for (name, off, enc, text) in expect {
        let (a, w) = c.pe.probe_string(*off as usize);
        let got = if *enc == "ascii" { a } else { w };
        assert_eq!(&got, text, "{name} @ {off:#x} as {enc}");
        // the name the user gave is part of the string
        assert!(got.contains(name), "{name} in {got}");
    }
}

#[test]
fn file_offsets_and_rvas_round_trip() {
    let Some(c) = ctx() else { return };
    for (_, off) in USER_STRING_OFFSETS {
        let rva = c.pe.off_to_rva(*off).expect("in a section");
        assert_eq!(rva as u64, off + 0x1800, ".rdata delta");
        assert_eq!(c.pe.rva_to_off(rva), Some(*off as usize));
        assert_eq!(c.pe.section_by_rva(rva).unwrap().name, ".rdata");
    }
    // a few other mappings: .text start, .data start
    let t = c.pe.section_named(".text").unwrap();
    assert_eq!(c.pe.off_to_rva(t.raw_off as u64), Some(t.rva));
    assert_eq!(c.pe.rva_to_off(t.rva + 0x10), Some(t.raw_off as usize + 0x10));
    assert!(c.pe.off_to_rva(0x10).is_none(), "headers are outside every section");
    // virtual-only (bss) addresses have no file offset
    let data = c.pe.section_named(".data").unwrap();
    assert!(c.pe.rva_to_off(data.rva + data.raw_size + 0x10).is_none());
}

// ------------------------------------------------------------------------------------------------ xrefs and the call graph

fn group_begin(c: &Ctx, rva: u32) -> u32 { c.pe.func_group(rva).expect("pdata").0 }

/// Function groups whose code calls into `target` group (one level), using the call index.
fn callers_of_group(c: &Ctx, target: u32) -> HashSet<u32> {
    let (b, e) = c.pe.funcs.iter().copied().find(|f| f.0 == target).expect("root fn");
    let _ = e;
    // all fragments of the group are entered through calls to the root begin only
    c.idx.callers_of(b).iter().filter_map(|s| c.pe.func_group(*s).map(|g| g.0)).collect()
}
/// `from` is (transitively, up to `depth` levels) called by `to`'s group, i.e. `to` reaches `from`.
fn reaches(c: &Ctx, to: u32, from: u32, depth: usize) -> bool {
    if to == from { return true; }
    if depth == 0 { return false; }
    callers_of_group(c, from).into_iter().any(|g| reaches(c, to, g, depth - 1))
}

#[test]
fn xrefs_from_the_slate_strings_find_construct() {
    let Some(c) = ctx() else { return };
    let (construct, refs) = slate::locate_construct(&c.pe, &c.idx).expect("located");
    assert_eq!(construct, 0x124_8480);
    assert_eq!(refs, 15, "LEA references to the FortniteLoadingScreen.cpp path inside Construct");
    let lea_groups = |file_off: u64| -> HashSet<u32> {
        let rva = c.pe.off_to_rva(file_off).unwrap();
        c.idx.refs_to(rva).iter().filter(|x| x.kind == XrefKind::Lea).map(|x| group_begin(c, x.from)).collect()
    };
    // text strings are referenced from Construct itself
    assert_eq!(lea_groups(0x44B7B00), HashSet::from([construct]), "EARLY ACCESS");
    assert_eq!(lea_groups(0x44B7C18), HashSet::from([construct]), "LOADING");
    // the source path is referenced by Construct and by the two creators
    let path = lea_groups(0x44B7B20);
    assert!(path.contains(&construct) && path.len() == 3, "{path:x?}");
    // the class-name string is referenced by the two creators, and both of them call Construct
    let creators = lea_groups(0x44B7AB8);
    assert_eq!(creators, HashSet::from([0x124_68d0, 0x124_c0e0]));
    let callers = callers_of_group(c, construct);
    assert_eq!(callers, creators, "the creators are exactly the callers of Construct");
    // asset strings: referenced by loaders / preloaders that Construct reaches through its helper calls
    for (name, off) in [("atlas", 0x44B73A0u64), ("plate", 0x44B7440), ("burbank", 0x44B7510), ("bar bg", 0x44B7570), ("bar fill", 0x44B7600), ("bar marquee", 0x44B7680)] {
        let gs = lea_groups(off);
        assert!(!gs.is_empty(), "{name}");
        assert!(gs.iter().any(|g| reaches(c, construct, *g, 3)), "{name}: Construct (via a helper) reaches one of {gs:x?}");
    }
    // helper identities used in the README
    assert!(callers_of_group(c, 0x124_af10).contains(&0x124_b4c0), "atlas loader <- MakeAtlasBrush");
    assert!(callers_of_group(c, 0x124_b4c0).contains(&construct));
    assert!(callers_of_group(c, 0x124_b790).contains(&construct), "MakePlateBrush");
    assert!(callers_of_group(c, 0x124_ba60).contains(&construct), "MakeProgressBarBrushes");
    assert!(callers_of_group(c, 0x124_b050).contains(&0x124_ba60), "progress texture loader");
}

#[test]
fn xrefs_from_the_sfortloadingscreen_and_manager_strings() {
    let Some(c) = ctx() else { return };
    let groups = |file_off: u64| -> HashSet<u32> {
        let rva = c.pe.off_to_rva(file_off).unwrap();
        c.idx.refs_to(rva).iter().map(|x| group_begin(c, x.from)).collect()
    };
    // SFortLoadingScreen.cpp: one literal per SNew, so locate by counting references over all copies
    let sf = slate::locate_by_source_file(&c.pe, &c.idx, "SFortLoadingScreen.cpp");
    assert_eq!(sf.first().copied(), Some((0x301_dc90, 25)), "SFortLoadingScreen::Construct");
    assert!(groups(0x4B3F980).contains(&0x301_dc90), "the user's literal is one of them");
    // the class-name string is the vtable-reached GetType(); Construct has its own caller (the SNew site)
    assert_eq!(groups(0x4AED1F8), HashSet::from([0x2e9_0680]));
    assert!(!c.idx.callers_of(0x301_dc90).is_empty());
    // manager: the file literal is in ShowLoadingScreen
    assert_eq!(groups(0x412D5F0), HashSet::from([0x61b_500]));
}

#[test]
fn sweep_confirms_the_dictionary() {
    let Some(c) = ctx() else { return };
    let sites = slate::sweep_constructs(&c.pe, &c.idx);
    assert!(sites.len() > 700, "{} SNew sites", sites.len());
    let table = slate::construct_table(&sites);
    let top = |class: &str| table.get(class).and_then(|v| v.first()).copied().unwrap_or((0, 0));
    let dict: std::collections::HashMap<&str, u32> = [("SBorder", 0x15c3160), ("SBox", 0x15c3440), ("SImage", 0x15c62a0), ("SCircularThrobber", 0x15c3d80), ("SProgressBar", 0x1600210), ("SSafeZone", 0x1600430), ("SHorizontalBox", 0x14a8120)].into_iter().collect();
    for (class, rva) in dict { let (r, n) = top(class); assert_eq!(r, rva, "{class}"); assert!(n >= 2, "{class} sites"); }
    assert_eq!(top("SBorder"), (0x15c3160, 48));
    // every dictionary entry is code (tiny leaf functions have no .pdata entry)
    for e in slate::dictionary() { assert!(c.pe.section_by_rva(e.rva).map(|s| s.is_exec()).unwrap_or(false), "{} {:#x} must be code", e.name, e.rva); }
    // the Construct function's own SNews, innermost first
    let mine: Vec<(String, u32)> = sites.iter().filter(|s| s.file_tail == "FortniteLoadingScreen.cpp" && s.at >= 0x124_8480 && s.at < 0x124_ac00).map(|s| (s.class.clone(), s.line)).collect();
    let want = ["STextBlock@278", "SBox@271", "SBorder@266", "SImage@256", "SProgressBar@238", "SBox@235", "SCircularThrobber@226", "STextBlock@216", "SHorizontalBox@210", "SVerticalBox@206", "SBorder@199", "SSafeZone@197", "SImage@190", "SOverlay@187", "SDPIScaler@184"];
    assert_eq!(mine.iter().map(|(c, l)| format!("{c}@{l}")).collect::<Vec<_>>(), want);
}

// ------------------------------------------------------------------------------------------------ constants

#[test]
fn recovery_has_no_errors_and_the_expected_numbers() {
    let Some(c) = ctx() else { return };
    let f = &c.f;
    assert!(f.errors.is_empty(), "{:?}", f.errors);
    assert_eq!(f.get("construct.rva").map(|i| i.rva), Some(Some(0x124_8480)));
    assert_eq!(f.f32s("plate.slot.padding"), Some(vec![0., 40., 60., 0.]));
    assert_eq!((f.u32("plate.slot.halign"), f.u32("plate.slot.valign")), (Some(3), Some(1)));
    assert_eq!(f.f32s("ea.box.padding"), Some(vec![0., 7., 7., 0.]));
    assert_eq!(f.f32s("ea.text.color"), Some(vec![0., 0., 0., 1.]));
    assert_eq!(f.get("text.font.size").unwrap().value, serde_json::json!([48, 48]));
    assert_eq!(f.get("ea.text.string").unwrap().value, serde_json::json!(["EARLY ACCESS", "FortniteLoadingScreen", "EarlyAccess"]));
    assert_eq!(f.f32s("loading.border.padding"), Some(vec![16., 4., 16., 4.]));
    assert_eq!(f.get("loading.border.align").unwrap().value, serde_json::json!([0, 3]));
    assert_eq!(f.f32s("bar.slot.padding"), Some(vec![0., 6., 0., 12.]));
    assert_eq!(f.f32s("throbber.slot.padding"), Some(vec![16., 0., 0., 0.]));
    assert_eq!((f.f32("bar.height"), f.f32("throbber.radius")), (Some(6.), Some(20.)));
    assert_eq!(f.f32("atlas.min_res_x"), Some(1920.));
    assert_eq!((f.f32("atlas.brush_w"), f.f32("atlas.brush_h")), (Some(1920.), Some(1080.)));
    assert_eq!((f.f32("fade.duration"), f.f32("fade.flag1.delay"), f.f32("fade.flag0.delay")), (Some(0.2), Some(1.), Some(0.)));
    assert_eq!(f.u32("viewport.zorder"), Some(20000));
    assert_eq!(f.f32("manager.hold"), Some(2.));
    assert_eq!(f.f32("bar.percent.scale"), Some(0.98));
    assert_eq!(f.f32s("bar.brush.margin").map(|v| v.iter().map(|x| (x * 1e4).round() / 1e4).collect::<Vec<_>>()), Some(vec![0.4167; 4]));
    assert_eq!(f.get("tree.snew").unwrap().value.as_array().unwrap().len(), 15);
}

/// FLinearColor::Black at .data 0x145198810 is constant-initialised: every instruction that references it only reads it.
#[test]
fn the_text_colour_global_is_never_written() {
    use iced_x86::{Decoder, DecoderOptions, Instruction, OpKind};
    let Some(c) = ctx() else { return };
    let mut n = 0;
    for x in c.idx.refs_in(0x519_8810, 0x519_8820) {
        if x.kind != XrefKind::Mem { continue; }
        let b = c.pe.bytes_at_rva(x.from, 16).unwrap();
        let mut d = Decoder::with_ip(64, b, c.pe.image_base + x.from as u64, DecoderOptions::NONE);
        let mut i = Instruction::default(); d.decode_out(&mut i);
        assert_ne!(i.op0_kind(), OpKind::Memory, "a store to the colour global at {:#x}", x.from);
        n += 1;
    }
    assert!(n > 40, "{n} readers");
    assert_eq!(c.pe.bytes_at_rva(0x519_8810, 16).unwrap(), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x80, 0x3f], "(0,0,0,1)");
}

/// The JSON report contains every finding and parses back to the same numbers.
#[test]
fn report_json_round_trips() {
    let Some(c) = ctx() else { return };
    let v = report::build(&c.pe, &c.idx, &c.f);
    let text = serde_json::to_string_pretty(&v).unwrap();
    let back: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v, back);
    let items = back["findings"].as_array().unwrap();
    assert_eq!(items.len(), c.f.items.len());
    for it in &c.f.items {
        let j = items.iter().find(|x| x["id"] == it.id).unwrap_or_else(|| panic!("{}", it.id));
        assert_eq!(j["value"], it.value, "{}", it.id);
        assert_eq!(j["confidence"], it.conf.as_str());
        if let Some(r) = it.rva { assert_eq!(j["rva"], format!("{r:#x}")); assert_eq!(j["va"], format!("{:#x}", c.pe.image_base + r as u64)); }
    }
    assert_eq!(back["user_strings"].as_array().unwrap().len(), 13);
    assert!(back["not_recovered"].as_array().unwrap().len() >= 5);
}

/// The consumer file (`fn-ui` startup_layout.rs) carries exactly the recovered numbers.
#[test]
fn startup_layout_constants_match_the_findings() {
    let Some(c) = ctx() else { return };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fn-ui/src/screens/startup_layout.rs");
    let Ok(src) = std::fs::read_to_string(&path) else { eprintln!("skipping: {} not found", path.display()); return };
    let num = |name: &str| -> f32 {
        let key = format!("pub const {name}: f32 = ");
        let l = src.lines().find(|l| l.starts_with(&key)).unwrap_or_else(|| panic!("{name} missing"));
        let v = l[key.len()..].split(';').next().unwrap().trim().trim_end_matches('.');
        v.parse::<f32>().unwrap_or_else(|_| panic!("{name}: {v}"))
    };
    let arr = |name: &str| -> Vec<f32> {
        let key = format!("pub const {name}: [f32; 4] = [");
        let l = src.lines().find(|l| l.starts_with(&key)).unwrap_or_else(|| panic!("{name} missing"));
        l[key.len()..].split(']').next().unwrap().split(',').map(|x| x.trim().trim_end_matches('.').parse::<f32>().unwrap()).collect()
    };
    let f = &c.f;
    let pad = f.f32s("plate.slot.padding").unwrap();
    assert_eq!(num("BADGE_MARGIN_TOP"), pad[1]);
    assert_eq!(num("BADGE_MARGIN_RIGHT"), pad[2]);
    assert_eq!(arr("BADGE_TEXT_PAD"), f.f32s("ea.box.padding").unwrap());
    assert_eq!(arr("BADGE_TEXT_COLOR"), f.f32s("ea.text.color").unwrap());
    let size = f.get("text.font.size").unwrap().value.as_array().unwrap()[0].as_f64().unwrap() as f32;
    assert_eq!((num("FONT_SIZE"), num("BADGE_TEXT_PX"), num("LOADING_PX")), (size, size, size));
    let lp = f.f32s("loading.border.padding").unwrap();
    assert_eq!(num("LOADING_MARGIN_LEFT"), lp[0]);
    assert_eq!(num("BAR_MARGIN_X"), lp[0]);
    let bp = f.f32s("bar.slot.padding").unwrap();
    assert_eq!(num("BAR_MARGIN_BOTTOM"), lp[3] + bp[3], "border bottom padding + slot bottom padding");
    assert_eq!(num("BAR_SLOT_PAD_TOP"), bp[1]);
    assert_eq!(num("BAR_H"), f.f32("bar.height").unwrap());
    assert_eq!(num("THROBBER_RADIUS"), f.f32("throbber.radius").unwrap());
    assert_eq!(num("THROBBER_PAD_LEFT"), f.f32s("throbber.slot.padding").unwrap()[0]);
    assert_eq!(num("ART_MIN_RES_X"), f.f32("atlas.min_res_x").unwrap());
    assert_eq!((num("ART_BRUSH_W"), num("ART_BRUSH_H")), (f.f32("atlas.brush_w").unwrap(), f.f32("atlas.brush_h").unwrap()));
    assert_eq!(num("FADE_SECONDS"), f.f32("fade.duration").unwrap());
    assert_eq!(num("FLAG1_FADE_DELAY_SECONDS"), f.f32("fade.flag1.delay").unwrap());
    assert_eq!(num("MANAGER_MIN_SHOW_SECONDS"), f.f32("manager.hold").unwrap());
    assert!((num("BAR_PERCENT_SCALE") - f.f32("bar.percent.scale").unwrap()).abs() < 1e-6);
    assert!(src.contains("pub const BAR_BOX_MARGIN: f32 = 5. / 12.;") && (5. / 12. - f.f32s("bar.brush.margin").unwrap()[0]).abs() < 1e-6);
    assert!(src.contains("pub const VIEWPORT_ZORDER: i32 = 20000;") && f.u32("viewport.zorder") == Some(20000));
    assert!(src.contains("pub const PROVISIONAL: bool = false;"));
    assert!(src.contains("BurbankBigRegular-Bold.ufont") && src.contains("Burbank-PlayerPower"));
    // plate native size: the pak texture is 584x160 (checked by fn-ui's pak tests); the exe reads it from the texture
    assert_eq!(f.get("plate.brush.size").unwrap().value, serde_json::json!([num("BADGE_W") as u32, num("BADGE_H") as u32]));
}


