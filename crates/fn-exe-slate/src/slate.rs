//! Slate knowledge recovered from the 1.8 client: the dictionary of builder/setter/Construct functions, the exe-wide sweep that confirms it,
//! and the locator that finds `SInitialFortniteLoadingScreen::Construct` from the string cross references.
//!
//! All addresses are RVAs (image base 0x140000000 -> VA = 0x140000000 + RVA); the README quotes VAs.
use crate::emu::Emu;
use crate::pe::Pe;
use crate::xref::{XrefIndex, XrefKind};
use iced_x86::{Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind, Register};
use std::collections::{BTreeMap, HashMap};

/// `SWidget::SetDebugInfo(const ANSICHAR* Type, const ANSICHAR* File, int32 Line)` - every `SNew` expands to a call of it with the class name,
/// the source path and the line of the `SNew`. rdx = type string, r8 = file, r9d = line.
pub const SET_DEBUG_INFO: u32 = 0x14ef730;
/// `SWidget::SWidgetConstruct(this, ToolTip, Cursor, EnabledState, Visibility, RenderTransform, RenderTransformPivot, Tag, bForceVolatile, MetaData)`
/// (the `SLATE_ARGUMENT`s every widget shares: pointers to `FArguments+0x60,+0x70,+0xc0,+0x110,+0x160,+0x1c0,+0x210,+0x218,+0x220`).
pub const SWIDGET_CONSTRUCT: u32 = 0x14a81c0;
pub const CHKSTK: u32 = 0x3ed62d0;

/// What a function in the dictionary is. `xmm`/`stack` = floating-point / stack arguments it reads (so traces do not print stale registers).
#[derive(Clone, Copy, Debug)]
pub struct Entry { pub rva: u32, pub name: &'static str, pub xmm: u8, pub stack: u8, pub fluent: bool, pub dump: Option<(i64, i64)> }

const fn e(rva: u32, name: &'static str) -> Entry { Entry { rva, name, xmm: 0, stack: 0, fluent: false, dump: None } }

/// The dictionary (built by reading the construct function and confirmed by [`sweep_constructs`] over the whole exe).
pub fn dictionary() -> Vec<Entry> {
    let args = Some((0x230, 0x700));
    vec![
        e(SET_DEBUG_INFO, "SWidget::SetDebugInfo(Type,File,Line)"),
        e(SWIDGET_CONSTRUCT, "SWidget::SWidgetConstruct(base SLATE_ARGUMENTs)"),
        // widget Construct(this, const FArguments&)
        Entry { dump: args, ..e(0x15c3160, "SBorder::Construct") },
        Entry { dump: args, ..e(0x15c3440, "SBox::Construct") },
        Entry { dump: args, ..e(0x15c62a0, "SImage::Construct") },
        Entry { dump: args, ..e(0x15c3d80, "SCircularThrobber::Construct") },
        Entry { dump: args, ..e(0x1600210, "SProgressBar::Construct") },
        Entry { dump: args, ..e(0x1630c40, "STextBlock::Construct") },
        Entry { dump: args, ..e(0x1600430, "SSafeZone::Construct") },
        Entry { dump: args, ..e(0x14a8120, "SBoxPanel::Construct (SHorizontalBox / SVerticalBox)") },
        Entry { dump: args, ..e(0x1248480, "SInitialFortniteLoadingScreen::Construct(this, style, bFlag)") },
        // FArguments constructors (defaults)
        e(0x45c3f0, "SBorder::FArguments::FArguments"),
        e(0x45c5e0, "SBox::FArguments::FArguments"),
        e(0x45c7c0, "SProgressBar::FArguments::FArguments"),
        e(0x45c8d0, "STextBlock::FArguments::FArguments"),
        e(0x4d1830, "SImage::FArguments::FArguments"),
        e(0x12478c0, "SCircularThrobber::FArguments::FArguments"),
        e(0x1247a10, "SSafeZone::FArguments::FArguments"),
        e(0x90a360, "SOverlay::FArguments::FArguments"),
        // slot / fluent setters
        Entry { xmm: 3, stack: 1, fluent: true, ..e(0x124c400, "SOverlay::FSlot::Padding(L,T,R,B)") },
        Entry { xmm: 3, stack: 1, fluent: true, ..e(0x124c4a0, "SBoxPanel::FSlot::Padding(L,T,R,B)") },
        Entry { xmm: 1, fluent: true, ..e(0x1561eb0, "SBoxPanel::FSlot::Padding(uniform)") },
        Entry { fluent: true, ..e(0x4632e0, "SBoxPanel::FSlot::AutoWidth/AutoHeight()") },
        Entry { fluent: true, ..e(0x1582900, "FSlot::operator[](content)") },
        e(0x124c810, "SOverlay::Slot() (new slot, Fill/Fill)"),
        e(0x45d840, "SBoxPanel::FSlot::FSlot (default SizeParam = Stretch(1))"),
        Entry { fluent: true, ..e(0x1647600, "SOverlay::FArguments::operator+(slot)") },
        Entry { fluent: true, ..e(0x12481a0, "FArguments.Content()[..] (SBox/SBorder/SSafeZone content)") },
        Entry { fluent: true, ..e(0x15ed660, "SBorder::FArguments::ContentColorAndOpacity(this, &Getter)") },
        Entry { fluent: true, ..e(0x15829c0, "SBorder::FArguments::operator[](content)") },
        Entry { fluent: true, ..e(0x1247150, "SImage::FArguments::ColorAndOpacity(this, &Getter)") },
        Entry { fluent: true, ..e(0x12475c0, "SProgressBar::FArguments::Percent(this, &Getter)") },
        Entry { fluent: true, ..e(0x12480e0, "SDPIScaler::FArguments::Content()[..]") },
        e(0x4f8b40, "SImage::FArguments ctor helper (TAttribute init)"),
        e(0x1248260, "TSlateDecl<SDPIScaler>::operator<<=(FArguments) -> TSharedRef"),
        e(0x8e3820, "TSlateDecl<SOverlay>::operator<<=(FArguments) -> TSharedRef"),
        // value helpers
        e(0x1494180, "FSlateFontInfo::FSlateFontInfo(UObject* Font, int32 Size, FName Typeface)"),
        e(0x12b4810, "FText::FromNamespaceKeySource(Namespace,Key,Source)  (NSLOCTEXT)"),
        e(0x142e560, "StaticLoadObject(Class,Outer,Name,Filename,LoadFlags,Sandbox)"),
        e(0x25aec50, "UTexture2D::StaticClass()"),
        e(0x258e570, "UFont::StaticClass()"),
        e(0x1247ab0, "FSlateDynamicImageBrush::FSlateDynamicImageBrush(UTexture*, ImageSize, Name, Tint, ...)"),
        e(0x1333f60, "FName::FName(const TCHAR*, EFindName)"),
        Entry { stack: 0, ..e(0x24fb740, "UUserInterfaceSettings::GetDPIScaleBasedOnSize(FIntPoint)") },
        e(0x216f9e0, "GetSystemResolution(int& ResX,int& ResY,EWindowMode&)"),
        e(0x124b4c0, "SInitialFortniteLoadingScreen::MakeAtlasBrush()"),
        e(0x124b790, "SInitialFortniteLoadingScreen::MakePlateBrush()"),
        e(0x124ba60, "SInitialFortniteLoadingScreen::MakeProgressBarBrushes()"),
        e(0x124af10, "LoadAtlas (1920x1080 gate)"),
        e(0x124b050, "LoadProgressBarTextures"),
        e(0x124b3d0, "PreloadAssets (atlas, plate, font)"),
    ]
}

/// Configure an emulator with the dictionary.
pub fn configure(emu: &mut Emu) {
    emu.names.insert(CHKSTK, "__chkstk".into());
    for en in dictionary() {
        emu.names.insert(en.rva, en.name.to_string());
        emu.sigs.insert(en.rva, (en.xmm, en.stack));
        if en.fluent { emu.ret_this.insert(en.rva); }
        if let Some(r) = en.dump { emu.dump_range.insert(en.rva, r); }
    }
    // noise: shared-pointer release helpers, TAttribute destructors, delegate copies
    for s in [SWIDGET_CONSTRUCT, 0x1281200, 0x73ce10, 0x27c1a0, 0x15b50e0, 0x3a34900, 0x1247d10, 0x27c1a0, 0x9d40c0] { emu.skip.insert(s); }
    emu.skip_indirect = true;
}

/// Find `SInitialFortniteLoadingScreen::Construct`: the function with the most references to the `FortniteLoadingScreen.cpp` path string (each `SNew` passes
/// it), i.e. the one function that builds a whole widget tree. Returns (function begin RVA, number of references).
pub fn locate_construct(pe: &Pe, idx: &XrefIndex) -> Option<(u32, usize)> { locate_by_source_file(pe, idx, "FortniteLoadingScreen.cpp").into_iter().next() }

/// Functions that reference (any copy of) the source file path ending in `file_tail`, with the number of references: the compiler emits one string literal per use in
/// some modules (SFortLoadingScreen.cpp) and one shared literal in others (FortniteLoadingScreen.cpp), so all copies are searched. Result: (function-group begin, refs), most first.
pub fn locate_by_source_file(pe: &Pe, idx: &XrefIndex, file_tail: &str) -> Vec<(u32, usize)> {
    let mut per_fn: HashMap<u32, usize> = HashMap::new();
    for (off, enc) in pe.find_string(file_tail) {
        if enc != "ascii" { continue; }
        // a path string starts earlier than the tail: walk back to the drive letter
        let mut start = off; while start > 0 && pe.data[start - 1] != 0 && off - start < 300 { start -= 1; }
        let Some(rva) = pe.off_to_rva(start as u64) else { continue };
        for x in idx.refs_to(rva) { if x.kind == XrefKind::Lea { if let Some(g) = pe.func_group(x.from) { *per_fn.entry(g.0).or_default() += 1; } } }
    }
    let mut v: Vec<_> = per_fn.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// One `SNew` found in the exe: the class string and the line.
#[derive(Clone, Debug)]
pub struct SNewSite { pub at: u32, pub class: String, pub file_tail: String, pub line: u32, pub construct: Option<u32> }

fn ascii_at(pe: &Pe, va: u64) -> Option<String> {
    let rva = pe.rva_of_va(va)?;
    let off = pe.rva_to_off(rva)?;
    let s = pe.ascii_at_off(off);
    if s.is_empty() { None } else { Some(s) }
}

/// Decode every call site of `SetDebugInfo` (via the xref index) and recover (class, file, line, Construct target).
/// The pattern the compiler emits: `lea rdx,[class]; lea r8,[file]; mov r9d,line; call SetDebugInfo; ...; call SWidgetConstruct; ...; call <Class>::Construct`.
pub fn sweep_constructs(pe: &Pe, idx: &XrefIndex) -> Vec<SNewSite> {
    let mut out = vec![];
    for &site in idx.callers_of(SET_DEBUG_INFO) {
        // look back for the three argument loads
        let back_start = site.saturating_sub(48);
        let Some(bytes) = pe.bytes_at_rva(back_start, 48 + 16) else { continue };
        // decode forward from several alignments; keep the decode that lands exactly on `site`
        let mut best: Option<(String, String, u32)> = None;
        for skew in 0..16u32 {
            let start = site.saturating_sub(40 + skew);
            let Some(b) = pe.bytes_at_rva(start, (site - start + 5) as usize) else { continue };
            let mut d = Decoder::with_ip(64, b, pe.image_base + start as u64, DecoderOptions::NONE);
            let mut ins = Instruction::default();
            let (mut class, mut file, mut line) = (None, None, None);
            let mut landed = false;
            while d.can_decode() {
                d.decode_out(&mut ins);
                let at = (ins.ip() - pe.image_base) as u32;
                if at == site { landed = ins.mnemonic() == Mnemonic::Call; break; }
                if at > site { break; }
                if ins.mnemonic() == Mnemonic::Lea && ins.is_ip_rel_memory_operand() {
                    match ins.op0_register() { Register::RDX => class = ascii_at(pe, ins.ip_rel_memory_address()), Register::R8 => file = ascii_at(pe, ins.ip_rel_memory_address()), _ => {} }
                }
                if ins.mnemonic() == Mnemonic::Mov && ins.op0_kind() == OpKind::Register && ins.op0_register() == Register::R9D && matches!(ins.op1_kind(), OpKind::Immediate32 | OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32to64 | OpKind::Immediate8to32) { line = Some(ins.immediate(1) as u32); }
            }
            if landed { if let (Some(c), Some(f), Some(l)) = (class, file, line) { best = Some((c, f, l)); break; } }
        }
        let _ = bytes;
        let Some((class, file, line)) = best else { continue };
        // forward: the first call after SWidgetConstruct
        let mut construct = None;
        if let Some(b) = pe.bytes_at_rva(site, 400) {
            let mut d = Decoder::with_ip(64, b, pe.image_base + site as u64, DecoderOptions::NONE);
            let mut ins = Instruction::default();
            let mut seen_swc = false; let mut n = 0;
            while d.can_decode() && n < 120 {
                d.decode_out(&mut ins); n += 1;
                if ins.flow_control() == FlowControl::Return { break; }
                if ins.mnemonic() == Mnemonic::Call && ins.op0_kind() == OpKind::NearBranch64 {
                    let t = (ins.near_branch_target() - pe.image_base) as u32;
                    if seen_swc { construct = Some(t); break; }
                    if t == SWIDGET_CONSTRUCT { seen_swc = true; }
                }
            }
        }
        let tail = file.rsplit(['\\', '/']).next().unwrap_or(&file).to_string();
        out.push(SNewSite { at: site, class, file_tail: tail, line, construct });
    }
    out.sort_by_key(|s| s.at);
    out
}

/// class name -> [(Construct RVA, count)] sorted by count descending.
pub fn construct_table(sites: &[SNewSite]) -> BTreeMap<String, Vec<(u32, usize)>> {
    let mut m: BTreeMap<String, HashMap<u32, usize>> = BTreeMap::new();
    for s in sites { if let Some(c) = s.construct { *m.entry(s.class.clone()).or_default().entry(c).or_default() += 1; } }
    m.into_iter().map(|(k, v)| { let mut v: Vec<_> = v.into_iter().collect(); v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0))); (k, v) }).collect()
}





/// SNew-like call sites found by emulating one function: any call whose rdx is a class-name string (`S...`), r8 a `.cpp` path string and r9d a line.
/// (SFortLoadingScreen.cpp expands SNew to per-class thunks instead of SetDebugInfo, so the pattern is matched by arguments, not by callee.)
pub fn emu_snew_list(pe: &Pe, begin: u32, end: u32) -> Vec<SNewSite> {
    use crate::emu::{Event, Val};
    let mut emu = Emu::new(pe);
    configure(&mut emu);
    emu.run(begin, end);
    let mut out = vec![];
    for e in &emu.events {
        let Event::Call(c) = e else { continue };
        let (Val::Img(c_rva), Val::Img(f_rva), Some(line)) = (&c.regs[1], &c.regs[2], c.regs[3].imm()) else { continue };
        let (Some(class), Some(file)) = (ascii_at(pe, pe.va(*c_rva)), ascii_at(pe, pe.va(*f_rva))) else { continue };
        if !class.starts_with('S') || !file.ends_with(".cpp") { continue; }
        let tail = file.rsplit(['\\', '/']).next().unwrap_or(&file).to_string();
        out.push(SNewSite { at: c.at, class, file_tail: tail, line: line as u32, construct: c.target });
    }
    out
}


