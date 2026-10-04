//! Builds `reports/exe-loading-screen.json`: the findings plus the evidence tables (user string offsets, SNew sweep, dictionary, framework facts, what could not be recovered).
use crate::findings::{self, Findings};
use crate::pe::Pe;
use crate::slate;
use crate::xref::XrefIndex;
use crate::{exe_path, USER_STRING_OFFSETS};
use serde_json::{json, Value};

/// What the static analysis could NOT recover (printed in the README as well).
pub const NOT_RECOVERED: &[(&str, &str)] = &[
    ("DPI curve / UIScaleRule", "UUserInterfaceSettings is configured by DefaultEngine.ini (not part of the exe); the repo's shortest-side curve (480:0.444 720:0.666 1080:1 8640:8) is ASSUMED"),
    ("exact Slate line metrics of Burbank at 48", "FreeType/Slate rounding is engine code, not decoded; BADGE_TEXT_BASELINE / LOADING_MARGIN_BOTTOM use fontdue ascent 45.6 / descent 14.4 of BurbankBigRegular-Bold (ASSUMED)"),
    ("SProgressBar / SCircularThrobber paint code", "engine code (OnPaint) was not decoded; only the arguments the game passes were. Fill inset = BorderPadding (1,0) and the throbber's piece layout are ENGINE BEHAVIOUR (inferred)"),
    ("movie-player settings (FLoadingScreenAttributes: bAutoCompleteWhenLoadingCompletes, bWaitForManualStop, bAllowInEarlyStartup, bMoviesAreSkippable, MinimumLoadingScreenDisplayTime, ...)", "this screen is NOT registered through SetupLoadingScreen(FLoadingScreenAttributes): the module hands its widget to the movie player with SetSlateOverlayWidget (inferred vtable slot). The ini keys [/Script/MoviePlayer.MoviePlayerSettings] bWaitForMoviesToComplete / bMoviesAreSkippable / StartupMovies exist as strings (rva 0x4767268..) but their values are config data"),
    ("identity of the movie-player object (vtable +0x28 / +0x60 / +0x50 / +0x90 of GetMoviePlayer() = 0x141ebf6c0)", "the slots are labelled by usage (SetSlateOverlayWidget, IsLoadingFinished-like, OnMoviePlaybackFinished delegate getter); the class vtable was not matched against engine headers"),
    ("window size / resolution the screen is created at", "comes from GetSystemResolution at run time"),
    ("title-safe padding of SSafeZone", "platform value; 0 on desktop unless a debug cvar is set (ASSUMED 0)"),
    ("SFortLoadingScreen (in-game screen) numbers", "only the widget tree (classes and lines) is listed; its numeric layout was not decoded (not needed for the start-up screen)"),
];

pub fn build(pe: &Pe, idx: &XrefIndex, f: &Findings) -> Value {
    let user: Vec<Value> = USER_STRING_OFFSETS.iter().map(|(name, off)| {
        let (a, w) = pe.probe_string(*off as usize);
        let rva = pe.off_to_rva(*off);
        let (enc, text) = if w.len() >= a.len() && !w.is_empty() { ("utf16", w) } else { ("ascii", a) };
        let xr: Vec<Value> = rva.map(|r| {
            // all references to the string start (the string table entries are contiguous: also accept refs a few bytes in)
            idx.refs_in(r, r + 1).iter().map(|x| json!({ "from": format!("{:#x}", x.from), "function_group": pe.func_group(x.from).map(|g| format!("{:#x}", g.0)) })).collect()
        }).unwrap_or_default();
        json!({ "name": name, "file_offset": format!("{off:#x}"), "rva": rva.map(|r| format!("{r:#x}")), "va": rva.map(|r| format!("{:#x}", pe.va(r))), "encoding": enc, "text": text, "xrefs": xr })
    }).collect();
    let sites = slate::sweep_constructs(pe, idx);
    let table = slate::construct_table(&sites);
    let sweep_classes: serde_json::Map<String, Value> = table.iter().map(|(k, v)| (k.clone(), json!(v.iter().take(3).map(|(r, n)| json!({"construct_va": format!("{:#x}", pe.va(*r)), "sites": n})).collect::<Vec<_>>()))).collect();
    let by_file = |tail: &str| sites.iter().filter(|s| s.file_tail == tail).map(|s| json!({"at": format!("{:#x}", s.at), "class": s.class, "line": s.line, "construct": s.construct.map(|c| format!("{:#x}", pe.va(c)))})).collect::<Vec<_>>();
    let dict: Vec<Value> = slate::dictionary().iter().map(|e| json!({"va": format!("{:#x}", pe.va(e.rva)), "name": e.name, "xmm_args": e.xmm, "stack_args": e.stack, "fluent_returns_this": e.fluent})).collect();
    let reasons: Vec<Value> = findings::manager_reasons(pe).into_iter().map(|(r, s)| json!({"rva": format!("{r:#x}"), "text": s})).collect();
    let sfls = slate::locate_by_source_file(pe, idx, "SFortLoadingScreen.cpp");
    let sfls_tree: Vec<Value> = sfls.first().and_then(|(b, _)| pe.funcs.iter().find(|f| f.0 == *b).map(|f| slate::emu_snew_list(pe, f.0, f.1))).unwrap_or_default().iter()
        .map(|s| json!({"at": format!("{:#x}", s.at), "class": s.class, "line": s.line, "thunk_va": s.construct.map(|c| format!("{:#x}", pe.va(c)))})).collect();
    json!({
        "binary": {
            "path": exe_path().display().to_string(), "size": pe.data.len(), "image_base": format!("{:#x}", pe.image_base), "pdata_functions": pe.funcs.len(),
            "sections": pe.sections.iter().map(|s| json!({"name": s.name, "rva": format!("{:#x}", s.rva), "vsize": format!("{:#x}", s.vsize), "raw_off": format!("{:#x}", s.raw_off), "raw_size": format!("{:#x}", s.raw_size)})).collect::<Vec<_>>(),
            "mapping_note": "the user's numbers are FILE offsets; .rdata raw 0x3fa3800 <-> rva 0x3fa5000 so rva = file offset + 0x1800 for all of them; strings are UTF-16 for asset paths, EARLY ACCESS and LOADING, ASCII for class and source-path strings",
        },
        "user_strings": user,
        "findings": f.to_json()["items"],
        "findings_errors": f.errors,
        "widget_sites": f.widgets.iter().map(|w| json!({"class": w.class, "line": w.line, "set_debug_info_rva": format!("{:#x}", w.set_debug_info_rva), "construct_va": w.construct_rva.map(|c| format!("{:#x}", pe.va(c)))})).collect::<Vec<_>>(),
        "sweep": { "snew_sites_in_exe": sites.len(), "classes": sweep_classes, "fortnite_loading_screen_cpp": by_file("FortniteLoadingScreen.cpp"), "sfortloadingscreen_cpp": by_file("SFortLoadingScreen.cpp"),
                   "sfortloadingscreen_tree_by_emulation": sfls_tree, "sfortloadingscreen_construct_candidates": sfls.iter().take(3).map(|(r, n)| json!({"group": format!("{:#x}", r), "refs": n})).collect::<Vec<_>>() },
        "dictionary": dict,
        "framework": framework(pe),
        "manager_reason_strings": reasons,
        "not_recovered": NOT_RECOVERED.iter().map(|(a, b)| json!({"what": a, "why": b})).collect::<Vec<_>>(),
    })
}

fn framework(pe: &Pe) -> Value {
    let v = |r: u32| format!("{:#x}", pe.va(r));
    json!({
        "module": {
            "name": "FortniteLoadingScreen (FFortniteLoadingScreenModule)", "factory_va": v(0x124b390), "object": "0x20 bytes: +0 vtable, +8 widget ptr, +0x10 refcount ctrl, +0x18 flags (bit0 viewport widget shown, bit1 'persist' requested, bit2 movie-player overlay active)",
            "vtable_va": v(0x44b8a78),
            "slots": { "+0x08 StartupModule": v(0x12468d0), "+0x20 ShutdownModule": v(0x1246d00), "+0x28/+0x30/+0x38 SupportsDynamicReloading/AutomaticShutdown/IsGameModule": "stubs returning true (0x141d36860)",
                       "+0x40 IsShowingInitialLoadingScreen (flags & 5)": v(0x1246d60), "+0x48 SetPersist(bool) (bit1; then 0x124ac00)": v(0x1246d70), "+0x50 SetProgress(float)": v(0x1246d90) },
        },
        "startup_sequence": [
            "StartupModule 0x12468d0: PreloadAssets 0x124b3d0 (atlas / plate / Burbank / progress textures via StaticLoadObject, synchronous)",
            "skip when -nullrhi (FParse cached at 0x14664d9c0, string at rva 0x44b8658) or when the dedicated-server globals 0x1466637e1/e2 say so (inferred)",
            "SNew(SInitialFortniteLoadingScreen).bFlag(false) -> Construct 0x1248480 (call 0x1246a66), module+0x18 |= 4",
            "GetMoviePlayer()->SetSlateOverlayWidget(widget) (vtable +0x28, inferred) so it is shown by the movie player while the startup movies / loading run",
            "GetMoviePlayer()->OnMoviePlaybackFinished().AddRaw(this, 0x124c0e0) (vtable +0x90, inferred)",
            "while loading: FortAssetManager startup jobs push progress through module->SetProgress (bar = clamp(progress*0.98))",
            "Tick (0x1248360, every frame): bFlag==0: when GetMoviePlayer()->vfunc[0x60]() says finished -> 0.2 s fade of plate/texts/bar (alpha=1-lerp), [0x14664d9d4]=1; when the fade ended -> vfunc[0x50] of the movie player, timer stops",
            "OnMoviePlaybackFinished 0x124c0e0: module+0x18 &= ~4; if [0x14664d9d4] and GEngine+0x5f0 (game viewport) exist: SNew(SInitialFortniteLoadingScreen).bFlag(true) added with AddViewportWidgetContent(widget, ZOrder 20000) (vtable +0x250), module+0x18 |= 1, then 0x124ac00",
            "0x124ac00: unless 'persist' (bit1) is set: widget+0x5b0 = completion delegate (0x124c6a0), widget+0x5f1 = 1 -> the bFlag==1 widget waits 1.0 s then fades ONLY the atlas over 0.2 s, then executes the delegate (remove) and stops its timer",
            "UFortLoadingScreenManager::ShowLoadingScreen (0x61b500): calls module->IsShowingInitialLoadingScreen (log 'Showing loading screen when IsShowingInitialLoadingScreen() is true/false'), then module->SetPersist(true) and TimeLoadingScreenFirstDismissed = -1",
            "UFortLoadingScreenManager::HideLoadingScreen (0x61b5e8 / 0x6102e7): module->SetPersist(false) -> the bFlag==1 widget starts its 1.0 s + 0.2 s exit",
            "UFortLoadingScreenManager::ShouldShowLoadingScreen (0x61ae80..0x61b4e1): reasons below; after they are all false the manager still shows for 2.0 s since TimeLoadingScreenFirstDismissed",
        ],
        "fade_semantics": {
            "curve": "FCurveSequence at widget+0x4e0, handle at +0x538: AddCurve(start, duration=0.2, EEasingFunc 0 = Linear); alpha = 1 - handle.GetLerp()",
            "bFlag_0": "plate image (getter 0x124ae90), EARLY ACCESS SBorder and LOADING SBorder ContentColorAndOpacity (getter 0x124ae00) fade; atlas (getter 0x124b270) stays white",
            "bFlag_1": "only the atlas fades; plate / texts / bar stay opaque until the widget is removed",
            "bar_fraction": "Percent getter 0x124b1d0: 1.0 while IsPlaying(widget+0x4e0), else clamp(float[0x14664d9d0] * 0.98, 0, 1)",
        },
    })
}

