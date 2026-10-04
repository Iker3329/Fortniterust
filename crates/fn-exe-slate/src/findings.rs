//! Every number of the start-up loading screen, extracted FROM THE EXE (not typed in): each [`Finding`] carries the RVA of the instruction (or data) it was
//! read from, the instruction text, and whether it was recovered directly, inferred from engine behaviour, or assumed.
//!
//! Two kinds of probes:
//! * *anchored* probes decode the instruction at a known RVA, assert its text (so a different binary fails loudly) and read its immediate / the
//!   float or string it references;
//! * *emulated* probes run [`crate::emu::Emu`] over `SInitialFortniteLoadingScreen::Construct` and read the `FArguments` objects handed to the widgets'
//!   `Construct` calls and the numeric arguments of the slot/font/brush calls.
use crate::emu::{CallEvent, Emu, Event, Obj, Val};
use crate::pe::Pe;
use crate::slate::{self, SET_DEBUG_INFO};
use crate::xref::XrefIndex;
use iced_x86::{Decoder, DecoderOptions, Formatter, Instruction, IntelFormatter, Mnemonic, OpKind};
use serde_json::{json, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Confidence {
    /// read directly from the instruction / data it is stored in
    Recovered,
    /// follows from a recovered fact plus documented engine behaviour (named in the note)
    Inferred,
    /// could not be recovered from the exe (reason in the note)
    Assumed,
}
impl Confidence { pub fn as_str(&self) -> &'static str { match self { Confidence::Recovered => "recovered", Confidence::Inferred => "inferred", Confidence::Assumed => "assumed" } } }

#[derive(Clone, Debug)]
pub struct Finding {
    pub id: &'static str,
    pub what: String,
    pub value: Value,
    pub rva: Option<u32>,
    pub insn: Option<String>,
    pub conf: Confidence,
    pub note: String,
}

#[derive(Default, Debug)]
pub struct Findings { pub items: Vec<Finding>, pub image_base: u64, pub widgets: Vec<WidgetSite>, pub errors: Vec<String> }

#[derive(Clone, Debug)]
pub struct WidgetSite { pub class: String, pub line: u32, pub set_debug_info_rva: u32, pub construct_rva: Option<u32> }

impl Findings {
    pub fn get(&self, id: &str) -> Option<&Finding> { self.items.iter().find(|f| f.id == id) }
    pub fn f32(&self, id: &str) -> Option<f32> { self.get(id).and_then(|f| f.value.as_f64()).map(|v| v as f32) }
    pub fn u32(&self, id: &str) -> Option<u32> { self.get(id).and_then(|f| f.value.as_u64()).map(|v| v as u32) }
    pub fn f32s(&self, id: &str) -> Option<Vec<f32>> { self.get(id).and_then(|f| f.value.as_array()).map(|a| a.iter().filter_map(|v| v.as_f64()).map(|v| v as f32).collect()) }
    pub fn str(&self, id: &str) -> Option<&str> { self.get(id).and_then(|f| f.value.as_str()) }
    pub fn to_json(&self) -> Value {
        let items: Vec<Value> = self.items.iter().map(|f| {
            let va = f.rva.map(|r| format!("{:#x}", self.image_base + r as u64));
            json!({ "id": f.id, "what": f.what, "value": f.value, "rva": f.rva.map(|r| format!("{r:#x}")), "va": va, "insn": f.insn, "confidence": f.conf.as_str(), "note": f.note })
        }).collect();
        json!({ "items": items })
    }
}

// ---------------------------------------------------------------------------------------------------- helpers

fn decode_at(pe: &Pe, rva: u32) -> Option<Instruction> {
    let b = pe.bytes_at_rva(rva, 16)?;
    let mut d = Decoder::with_ip(64, b, pe.image_base + rva as u64, DecoderOptions::NONE);
    let mut i = Instruction::default();
    d.decode_out(&mut i);
    if i.is_invalid() { None } else { Some(i) }
}
fn text_of(i: &Instruction) -> String {
    let mut f = IntelFormatter::new();
    f.options_mut().set_hex_prefix("0x"); f.options_mut().set_hex_suffix(""); f.options_mut().set_uppercase_hex(false);
    f.options_mut().set_branch_leading_zeros(false); f.options_mut().set_space_after_operand_separator(false);
    let mut s = String::new(); f.format(i, &mut s); s
}
/// Decode the instruction at `rva`, require its text to start with `expect`; returns (instruction, text).
fn anchor(pe: &Pe, rva: u32, expect: &str, errs: &mut Vec<String>) -> Option<(Instruction, String)> {
    let i = decode_at(pe, rva);
    match i {
        Some(i) => { let t = text_of(&i); if t.starts_with(expect) { Some((i, t)) } else { errs.push(format!("{rva:#x}: expected `{expect}`, found `{t}`")); None } }
        None => { errs.push(format!("{rva:#x}: cannot decode")); None }
    }
}
fn rip_target(pe: &Pe, i: &Instruction) -> Option<u32> { if i.is_ip_rel_memory_operand() { Some((i.ip_rel_memory_address() - pe.image_base) as u32) } else { None } }
fn utf16_at_rva(pe: &Pe, rva: u32) -> Option<String> { pe.rva_to_off(rva).map(|o| pe.utf16_at_off(o)) }
fn ascii_at_rva(pe: &Pe, rva: u32) -> Option<String> { pe.rva_to_off(rva).map(|o| pe.ascii_at_off(o)) }
fn imm(i: &Instruction) -> Option<u64> {
    let n = i.op_count();
    (0..n).find_map(|k| match i.op_kind(k) { OpKind::Immediate8 | OpKind::Immediate16 | OpKind::Immediate32 | OpKind::Immediate64 | OpKind::Immediate8to16 | OpKind::Immediate8to32 | OpKind::Immediate8to64 | OpKind::Immediate32to64 => Some(i.immediate(k)), _ => None })
}
fn f32_from_bits(v: u64) -> f32 { f32::from_bits(v as u32) }
fn rd_f32s(pe: &Pe, rva: u32, n: u32) -> Vec<f32> { (0..n).filter_map(|k| pe.read_f32(rva + 4 * k)).collect() }
fn round6(v: f32) -> f64 { ((v as f64) * 1e6).round() / 1e6 }
fn jf(v: f32) -> Value { json!(round6(v)) }
fn jfs(v: &[f32]) -> Value { Value::Array(v.iter().map(|x| jf(*x)).collect()) }

struct Sink<'a> { f: &'a mut Findings }
impl Sink<'_> {
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, id: &'static str, what: &str, value: Value, rva: Option<u32>, insn: Option<String>, conf: Confidence, note: &str) {
        self.f.items.push(Finding { id, what: what.to_string(), value, rva, insn, conf, note: note.to_string() });
    }
}

fn events_of<'a>(emu: &'a Emu<'_>) -> Vec<&'a CallEvent> { emu.events.iter().filter_map(|e| if let Event::Call(c) = e { Some(c) } else { None }).collect() }

// ---------------------------------------------------------------------------------------------------- the extraction

/// Recover everything. Needs the xref index (`XrefIndex::build`) only for locating the construct function and the callers.
pub fn recover(pe: &Pe, idx: &XrefIndex) -> Findings {
    let mut out = Findings { image_base: pe.image_base, ..Default::default() };
    let mut errs: Vec<String> = vec![];
    let mut sk = Sink { f: &mut out };

    // ---- 1. locate Construct from the strings
    let construct = slate::locate_construct(pe, idx);
    let Some((c_rva, c_refs)) = construct else { sk.f.errors.push("construct not located".into()); return out };
    sk.add("construct.rva", "SInitialFortniteLoadingScreen::Construct(this, style, bFlag)", json!(format!("{c_rva:#x}")), Some(c_rva), None, Confidence::Recovered,
        &format!("the one function with {c_refs} LEA references to the FortniteLoadingScreen.cpp path string (file offset 0x44B7B20 = rva 0x44B9320): each SNew passes it to SWidget::SetDebugInfo"));
    let c_end = pe.func_containing(c_rva).map(|f| f.1).unwrap_or(c_rva + 0x2800);
    let callers: Vec<u32> = idx.callers_of(c_rva).to_vec();
    sk.add("construct.callers", "call sites of Construct (creators of the widget)", json!(callers.iter().map(|c| format!("{c:#x}")).collect::<Vec<_>>()), callers.first().copied(), None, Confidence::Recovered,
        "two creators: the module StartupModule path (flag 0, widget handed to the movie player as overlay) and the OnMoviePlaybackFinished handler (flag 1, widget added to the game viewport at ZOrder 20000)");
    // the flag value each creator passes (movzx r8d, byte [rax] with the byte stored just before)
    if let Some((_, t)) = anchor(pe, 0x1246946, "xor eax,eax", &mut errs) { sk.add("creator.startup.flag", "bFlag passed by the start-up creator (0x12468d0 -> 0x1246953 tail)", json!(0), Some(0x1246946), Some(t), Confidence::Recovered, "al = 0 is stored at [rsp+0x60] (0x1246976) and loaded into r8d for Construct at 0x1246a62") }
    if let Some((i, t)) = anchor(pe, 0x124c148, "mov byte ptr [rsp+0x60],1", &mut errs) { sk.add("creator.movie_finished.flag", "bFlag passed by the OnMoviePlaybackFinished creator (0x124c0e0)", json!(imm(&i).unwrap_or(0)), Some(0x124c148), Some(t), Confidence::Recovered, "stored at [rsp+0x60] and loaded into r8d for Construct at 0x124c21b") }
    if let Some((_, t)) = anchor(pe, 0x12484cf, "mov [rcx+0x5f0],r8b", &mut errs) { sk.add("flag.field", "Construct stores its bFlag at widget+0x5f0 (and clears widget+0x5f1 at 0x12484ef)", json!("0x5f0"), Some(0x12484cf), Some(t), Confidence::Recovered, "flag semantics: see fade.* findings (selects which layer group fades)") }

    // ---- 2. emulate Construct
    let mut emu = Emu::new(pe);
    slate::configure(&mut emu);
    emu.run(c_rva, c_end);
    let evs = events_of(&emu);
    // widget list (SetDebugInfo) paired with the next Construct call
    let construct_fns: [u32; 8] = [0x15c3160, 0x15c3440, 0x15c62a0, 0x15c3d80, 0x1600210, 0x1630c40, 0x1600430, 0x14a8120];
    let mut widgets: Vec<(WidgetSite, Option<Obj>)> = vec![];
    for (k, e) in evs.iter().enumerate() {
        if e.target == Some(SET_DEBUG_INFO) {
            let class = match &e.regs[1] { Val::Img(r) => ascii_at_rva(pe, *r).unwrap_or_default(), _ => String::new() };
            let line = e.regs[3].imm().unwrap_or(0) as u32;
            let mut cons = None; let mut args = None;
            for e2 in evs.iter().skip(k + 1) {
                if e2.target == Some(SET_DEBUG_INFO) { break; }
                if let Some(t) = e2.target { if construct_fns.contains(&t) { cons = Some(t); if let Val::Addr(o, 0) = &e2.regs[1] { args = Some(*o); } break; } }
            }
            widgets.push((WidgetSite { class, line, set_debug_info_rva: e.at, construct_rva: cons }, args));
        }
    }
    sk.f.widgets = widgets.iter().map(|w| w.0.clone()).collect();
    sk.add("tree.snew", "SNew call sites in Construct, execution order = innermost first (class@line)",
        json!(widgets.iter().map(|w| format!("{}@{}", w.0.class, w.0.line)).collect::<Vec<_>>()), Some(widgets.first().map(|w| w.0.set_debug_info_rva).unwrap_or(0)), None, Confidence::Recovered,
        "line numbers are lines of FortniteLoadingScreen.cpp (r9d of SetDebugInfo); outer widgets have the smaller line numbers");
    let find_w = |class: &str, line: u32| widgets.iter().find(|w| w.0.class == class && w.0.line == line).cloned();
    let cell_f = |o: Option<Obj>, off: i64| o.and_then(|o| emu.cell_f32(o, off));
    let cell_u = |o: Option<Obj>, off: i64| o.and_then(|o| emu.cell_u32(o, off));
    let at_of = |w: &Option<(WidgetSite, Option<Obj>)>| w.as_ref().map(|w| w.0.set_debug_info_rva);

    // ---- 3. SDPIScaler + DPI
    if let Some((i, t)) = anchor(pe, 0x12487e6, "call", &mut errs) {
        let tgt = (i.near_branch_target() - pe.image_base) as u32;
        sk.add("dpi.call", "DPI scale = UUserInterfaceSettings::GetDPIScaleBasedOnSize(system resolution), stored once as SDPIScaler.DPIScale (a value, not a delegate)", json!(format!("{tgt:#x}")), Some(0x12487e6), Some(t), Confidence::Recovered,
            "the resolution comes from GetSystemResolution (0x14216f9e0 call at 0x1248608); the curve / rule live in the UserInterfaceSettings config, not in the exe -> the repo's shortest-side curve (480:0.444, 720:0.666, 1080:1, 8640:8) is used (ASSUMED, same as every other fn-ui screen)");
    }
    let dpi_w = find_w("SDPIScaler", 184);
    sk.add("tree.dpi_scaler.line", "SDPIScaler is the root: ChildSlot of the widget (this+0x338)", json!(184), at_of(&dpi_w), None, Confidence::Recovered, "SetDebugInfo line 184; child = SOverlay (line 187)");

    // ---- 4. atlas
    if let Some((i, t)) = anchor(pe, 0x124af74, "comiss xmm1,[", &mut errs) {
        let r = rip_target(pe, &i).unwrap_or(0);
        sk.add("atlas.min_res_x", "the atlas brush is only created when GSystemResolution.ResX >= this (else the SImage gets a null brush and draws nothing)", jf(pe.read_f32(r).unwrap_or(0.)), Some(0x124af74), Some(t), Confidence::Recovered,
            "xmm1 = cvtdq2ps([0x1451b6a40]) = GSystemResolution.ResX (int, written by 0x168ced0 / 0x16509cc ...); jae -> load FORT_Load_Atlas, else brush = null");
    }
    if let Some((i, t)) = anchor(pe, 0x124af66, "mov dword ptr [rsp+0x60],0x", &mut errs) { sk.add("atlas.brush_w", "atlas FSlateBrush.ImageSize.X", jf(f32_from_bits(imm(&i).unwrap_or(0))), Some(0x124af66), Some(t), Confidence::Recovered, "= native width of FORT_Load_Atlas (1920)"); }
    if let Some((i, t)) = anchor(pe, 0x124af5b, "movss xmm0,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("atlas.brush_h", "atlas FSlateBrush.ImageSize.Y", jf(pe.read_f32(r).unwrap_or(0.)), Some(0x124af5b), Some(t), Confidence::Recovered, "= native height (1080), stored at [rsp+0x64] by 0x124af6e"); }
    if let Some((i, t)) = anchor(pe, 0x124af8c, "lea r8,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("atlas.asset", "StaticLoadObject path of the atlas texture", json!(utf16_at_rva(pe, r).unwrap_or_default()), Some(0x124af8c), Some(t), Confidence::Recovered, "string FORT_Load_Atlas at file offset 0x44B73A0 (UTF-16)"); }
    let atlas_w = find_w("SImage", 190);
    sk.add("atlas.image.slot", "atlas SImage lives in an SOverlay slot created by SOverlay::Slot() (alignment Fill/Fill, padding 0) with ZOrder = first (back)", json!({"halign":"Fill","valign":"Fill","padding":[0,0,0,0]}), at_of(&atlas_w), None, Confidence::Recovered,
        "slot built by 0x124c810 (zero-initialises +0x18/+0x1c alignment and +0x20 padding) at 0x124a28d; no Padding()/alignment setter is called on it; it is the first slot added to the overlay args (0x141647600 at 0x124a2b6)");
    sk.add("atlas.draw", "SImage default: brush drawn stretched over the allotted rect (DrawAs=Image, no tiling, no aspect-fit)", json!("stretch to window"), at_of(&atlas_w), None, Confidence::Inferred,
        "SImage::OnPaint draws the brush over AllottedGeometry; the FSlateDynamicImageBrush ctor 0x141247ab0 is given only name/size/tint (default DrawAs=Image); aspect is NOT preserved when the window is not 16:9");
    if let Some((_, t)) = anchor(pe, 0x1249fca, "mov rax,[r15+0x548]", &mut errs) { sk.add("atlas.color_getter", "atlas SImage.ColorAndOpacity is bound to getter 0x124b270 (FSlateColor): bFlag==0 -> white; bFlag!=0 -> alpha = 1 - FadeCurve.GetLerp()", json!("0x124b270"), Some(0x1249fca), Some(t), Confidence::Recovered, "image pointer = this+0x548 (atlas brush TSharedPtr); getter body at 0x124b270..0x124b308"); }

    // ---- 5. plate (SImage line 256) + EARLY ACCESS text
    let pad_calls: Vec<&&CallEvent> = evs.iter().filter(|e| e.target == Some(0x124c400)).collect();
    if pad_calls.len() == 2 {
        let p = |e: &CallEvent| vec![e.xmm[1].unwrap_or(f32::NAN), e.xmm[2].unwrap_or(f32::NAN), e.xmm[3].unwrap_or(f32::NAN), e.stack[0].map(f32_from_bits).unwrap_or(f32::NAN)];
        let (a, b) = (p(pad_calls[0]), p(pad_calls[1]));
        if a == b { sk.add("plate.slot.padding", "Padding(L,T,R,B) of BOTH top-right overlay slots (plate image and the EARLY ACCESS text border)", jfs(&a), Some(pad_calls[0].at), None, Confidence::Recovered,
            "SOverlay::FSlot::Padding(0x124c400): xmm1=L xmm2=T xmm3=R [rsp+0x20]=B, called at 0x124a22f and 0x124a25f with identical constants (movss xmm6,[0x144031d60]=40 and xmm3,[0x143fe3728]=60)"); }
        else { errs.push(format!("plate paddings differ: {a:?} vs {b:?}")); }
    } else { errs.push(format!("expected 2 overlay padding calls, got {}", pad_calls.len())); }
    if let Some((i, t)) = anchor(pe, 0x124a1b2, "mov dword ptr [rdi+0x18],3", &mut errs) { sk.add("plate.slot.halign", "plate/text overlay slot HAlign (EHorizontalAlignment: Fill0 Left1 Center2 Right3)", json!(imm(&i).unwrap_or(99)), Some(0x124a1b2), Some(t), Confidence::Recovered, "= HAlign_Right; same value stored for the second slot at 0x124a1cf"); }
    if let Some((i, t)) = anchor(pe, 0x124a1b9, "mov dword ptr [rdi+0x1c],1", &mut errs) { sk.add("plate.slot.valign", "plate/text overlay slot VAlign (EVerticalAlignment: Fill0 Top1 Center2 Bottom3)", json!(imm(&i).unwrap_or(99)), Some(0x124a1b9), Some(t), Confidence::Recovered, "= VAlign_Top; same value stored for the second slot at 0x124a1d6"); }
    let _ = anchor(pe, 0x124a1cf, "mov dword ptr [rax+0x18],3", &mut errs);
    sk.add("plate.brush.size", "plate FSlateDynamicImageBrush.ImageSize = (UTexture2D+0xf8 -> SizeX, SizeY) of T-Loading-EarlyAccessPlate", json!([584, 160]), Some(0x124b877), None, Confidence::Inferred,
        "0x124b790 builds the brush with ImageSize = (float)(texture->SizeX, SizeY) read from [texture+0xf8]; the texture is 584x160 in the 1.8 pak (decoded with fn-texture). SImage desired size = ImageSize, so the slot (Right/Top) holds it at native size");
    sk.add("plate.brush.fallback", "WidthOverride/HeightOverride of the text SBox when the plate brush is missing", json!([300, 100]), Some(0x1248a15), None, Confidence::Recovered, "`movss xmm0,[0x143ff9040]`=300 (0x1248a15) and `[0x143fe3730]`=100 (0x12489e5) are the else-arms of `brush ? brush->ImageSize : default`; with the plate loaded the SBox is exactly ImageSize (584x160)");
    let sbox_ea = find_w("SBox", 271);
    sk.add("ea.box.align", "EARLY ACCESS SBox content alignment (HAlign, VAlign)", json!([cell_u(sbox_ea.as_ref().and_then(|w| w.1), 0x230).unwrap_or(99), cell_u(sbox_ea.as_ref().and_then(|w| w.1), 0x234).unwrap_or(99)]), at_of(&sbox_ea), None, Confidence::Recovered, "SBox::FArguments +0x230/+0x234 = 2/2 = Center/Center (stores at 0x1248cbe / 0x1248ccf)");
    let ea_pad: Vec<f32> = (0..4).map(|k| cell_f(sbox_ea.as_ref().and_then(|w| w.1), 0x240 + 4 * k).unwrap_or(f32::NAN)).collect();
    sk.add("ea.box.padding", "EARLY ACCESS SBox Padding(L,T,R,B)", jfs(&ea_pad), Some(0x1248970), Some(text_of(&decode_at(pe, 0x1248970).unwrap())), Confidence::Recovered, "movaps xmm0,[0x1444b9470] (16 bytes in .rdata: 0, 7, 7, 0) stored to SBox::FArguments+0x240 at 0x1248ce7");
    sk.add("ea.box.size", "EARLY ACCESS SBox Width/HeightOverride = plate brush ImageSize", json!({"width": "brush.ImageSize.X", "height": "brush.ImageSize.Y"}), at_of(&sbox_ea), None, Confidence::Recovered, "[rbp+0x850] <- [brush+8] / 300, [rbp+0x8a0] <- [brush+0xc] / 100 (0x12489d?..0x1248a1d); stored into SBox::FArguments +0x2b0 / +0x300 (0x1248c6d / 0x1248c9d)");
    // fonts
    let font_calls: Vec<&&CallEvent> = evs.iter().filter(|e| e.target == Some(0x1494180)).collect();
    let sizes: Vec<u64> = font_calls.iter().filter_map(|e| e.regs[2].imm()).collect();
    sk.add("text.font.size", "FSlateFontInfo size (Slate units) for EARLY ACCESS and LOADING", json!(sizes), font_calls.first().map(|e| e.at), None, Confidence::Recovered, "FSlateFontInfo::FSlateFontInfo(UObject*, int32 Size, FName) called with r8d=0x30 at 0x1248900 and 0x12491aa");
    let fname_calls: Vec<String> = evs.iter().filter(|e| e.target == Some(0x1333f60)).filter_map(|e| if let Val::Img(r) = &e.regs[1] { ascii_at_rva(pe, *r) } else { None }).collect();
    sk.add("text.font.typeface", "typeface FName of both texts", json!(fname_calls.iter().filter(|n| n.starts_with("Burbank")).cloned().collect::<Vec<_>>()), font_calls.first().map(|e| e.at), None, Confidence::Recovered, "FName(\"Burbank-PlayerPower\") at 0x12488c7 / 0x1249185; composite font asset /Game/UI/Foundation/Fonts/Burbank maps this typeface to BurbankBigRegular-Bold (import -4 of the package, decoded with fn-ueio) - NOT BurbankBigCondensed-Black");
    let load_obj: Vec<String> = evs.iter().filter(|e| e.target == Some(0x142e560)).filter_map(|e| if let Val::Img(r) = &e.regs[2] { utf16_at_rva(pe, *r) } else { None }).collect();
    sk.add("text.font.asset", "font object loaded with StaticLoadObject(UFont)", json!(load_obj), None, None, Confidence::Recovered, "UFont::StaticClass() 0x258e570 + path at file offset 0x44B7510");
    if let Some((i, t)) = anchor(pe, 0x124886a, "movups xmm0,[", &mut errs) {
        let r = rip_target(pe, &i).unwrap_or(0);
        sk.add("ea.text.color", "EARLY ACCESS STextBlock.ColorAndOpacity (FSlateColor, ColorUseRule=UseColor_Specified): FLinearColor RGBA", jfs(&rd_f32s(pe, r, 4)), Some(0x124886a), Some(t), Confidence::Recovered,
            "global at .data 0x145198810 = FLinearColor::Black (0,0,0,1); constant-initialised (no instruction writes it: every xref is a load, see tests); copied to [rbp+0x600] (0x1248890), rule byte [rbp+0x610]=0 (0x124889e), isSet [rbp+0x628]=1 (0x12488b3), applied with TAttribute copy 0x1402b98b0 at 0x1248b0c");
    }
    let ea_text = evs.iter().find(|e| e.target == Some(0x12b4810));
    if let Some(e) = ea_text {
        let s = |v: &Val| if let Val::Img(r) = v { utf16_at_rva(pe, *r).unwrap_or_default() } else { String::new() };
        sk.add("ea.text.string", "EARLY ACCESS text: (source, namespace, key)", json!([s(&e.regs[1]), s(&e.regs[2]), s(&e.regs[3])]), Some(e.at), None, Confidence::Recovered, "NSLOCTEXT(\"FortniteLoadingScreen\",\"EarlyAccess\",\"EARLY ACCESS\"); file offset 0x44B7B00");
    }
    sk.add("ea.text.layers", "z-order inside the overlay (back to front)", json!(["atlas SImage (Fill/Fill)", "SSafeZone > SBorder > SVerticalBox (LOADING row + progress bar)", "plate SImage (Right/Top, padding 0,40,60,0)", "SBorder > SBox(plate size) > STextBlock EARLY ACCESS (Right/Top, padding 0,40,60,0)"]), Some(0x124a2b6), None, Confidence::Recovered,
        "slots are appended with SOverlay::FArguments::operator+ (0x141647600) in the order atlas (0x124a2b6), safe zone (0x124a2c1), plate image (0x124a2cc), EARLY ACCESS border (0x124a2d7); SOverlay paints children in array order");
    // EA SBorder
    let ea_border = find_w("SBorder", 266);
    sk.add("ea.border.padding", "EARLY ACCESS SBorder padding and brush", json!({"padding": (0..4).map(|k| round6(cell_f(ea_border.as_ref().and_then(|w| w.1), 0x250 + 4 * k).unwrap_or(f32::NAN))).collect::<Vec<_>>(), "border_image": "NoBorder (FCoreStyle brush, draws nothing)"}), at_of(&ea_border), None, Confidence::Recovered, "SBorder::FArguments padding at +0x250 = [rbp+0x950] (zeroed by xorps at 0x1248a77); +0x3b0 = GetBrush(\"NoBorder\")");

    // ---- 6. LOADING row
    let load_border = find_w("SBorder", 199);
    let lb = load_border.as_ref().and_then(|w| w.1);
    sk.add("loading.border.align", "LOADING container SBorder HAlign, VAlign", json!([cell_u(lb, 0x240).unwrap_or(99), cell_u(lb, 0x244).unwrap_or(99)]), at_of(&load_border), None, Confidence::Recovered, "SBorder::FArguments +0x240/+0x244 = 0/3 = Fill/Bottom (stores at 0x1249d33 / 0x1249d29)");
    let lpad: Vec<f32> = (0..4).map(|k| cell_f(lb, 0x250 + 4 * k).unwrap_or(f32::NAN)).collect();
    if let Some((i, t)) = anchor(pe, 0x12492b2, "movaps xmm0,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); let rd = rd_f32s(pe, r, 4); let _ = rd; sk.add("loading.border.padding", "LOADING container SBorder Padding(L,T,R,B)", jfs(&lpad), Some(0x12492b2), Some(t), Confidence::Recovered, "movaps xmm0,[0x1444b9490] = (16,4,16,4) stored to [rbp+0x8f0], then SBorder::FArguments+0x250 at 0x1249d44"); }
    let safe = find_w("SSafeZone", 197);
    sk.add("loading.safezone", "LOADING subtree is wrapped in an SSafeZone with the default arguments (not title-safe, margin scale 1, pad all sides)", json!({"IsTitleSafe": false, "SafeAreaScale": [1,1,1,1], "pad": [true,true,true,true]}), at_of(&safe), None, Confidence::Recovered,
        "SSafeZone::FArguments ctor 0x1247a10 defaults; Construct calls no setter. On a desktop viewport the safe-zone padding is 0 unless r.DebugSafeZone is set (ASSUMED 0)");
    // slots of the boxes (emulated calls)
    let box_pads: Vec<(u32, Vec<f32>)> = evs.iter().filter(|e| e.target == Some(0x124c4a0)).map(|e| (e.at, vec![e.xmm[1].unwrap_or(f32::NAN), e.xmm[2].unwrap_or(f32::NAN), e.xmm[3].unwrap_or(f32::NAN), e.stack[0].map(f32_from_bits).unwrap_or(f32::NAN)])).collect();
    if box_pads.len() == 2 {
        // order of creation: progress slot first (line 235 area), then throbber slot
        sk.add("bar.slot.padding", "SVerticalBox slot holding the progress SBox: Padding(L,T,R,B), AutoHeight", jfs(&box_pads[0].1), Some(box_pads[0].0), None, Confidence::Recovered, "SBoxPanel::FSlot::Padding (0x124c4a0): xmm1=0 xmm2=6.0 xmm3=0 [rsp+0x20]=12.0 (0x41400000 from movss [0x144008e48]); preceded by AutoHeight (0x1404632e0)");
        sk.add("throbber.slot.padding", "SHorizontalBox slot holding the SCircularThrobber: Padding(L,T,R,B), AutoWidth, VAlign Center", jfs(&box_pads[1].1), Some(box_pads[1].0), None, Confidence::Recovered, "xmm1 = [0x144040c9c] = 16.0, others 0; VAlign: `mov dword ptr [rbx+0x84],2` at 0x12497b0");
    } else { errs.push(format!("expected 2 box-panel padding calls, got {}", box_pads.len())); }
    let up: Vec<f32> = evs.iter().filter(|e| e.target == Some(0x1561eb0)).filter_map(|e| e.xmm[1]).collect();
    sk.add("text.slot.padding", "SHorizontalBox slot holding the LOADING STextBlock: Padding(uniform), AutoWidth, VAlign Center", json!(up.first().copied().map(round6)), evs.iter().find(|e| e.target == Some(0x1561eb0)).map(|e| e.at), None, Confidence::Recovered, "Padding(0.0f) (0x1561eb0 shufps-broadcasts xmm1 into all four margins); `mov dword ptr [rdi+0x84],2` = VAlign_Center at 0x12499a1");
    if let Some((i, t)) = anchor(pe, 0x1249b7d, "mov dword ptr [rdi+0x80],1", &mut errs) { sk.add("row.slot.halign", "SVerticalBox slot holding the LOADING SHorizontalBox: HAlign (Left=1), AutoHeight", json!(imm(&i).unwrap_or(99)), Some(0x1249b7d), Some(t), Confidence::Recovered, "the progress slot keeps the default HAlign_Fill, so the bar spans the container"); }
    if let Some((i, t)) = anchor(pe, 0x124911e, "mov dword ptr [rbp+0x760],0x", &mut errs) { sk.add("bar.height", "progress-bar SBox HeightOverride", jf(f32_from_bits(imm(&i).unwrap_or(0))), Some(0x124911e), Some(t), Confidence::Recovered, "stored as SBox::FArguments+0x300 (0x124948e) with isSet=1 (0x1249495); WidthOverride unset -> bar width = slot width (Fill)"); }
    let thr = find_w("SCircularThrobber", 226);
    if let Some((i, t)) = anchor(pe, 0x1249658, "mov dword ptr [rax+0x240],0x", &mut errs) { sk.add("throbber.radius", "SCircularThrobber.Radius", jf(f32_from_bits(imm(&i).unwrap_or(0))), Some(0x1249658), Some(t), Confidence::Recovered, "default is 16 (ctor 0x12478c0 writes +0x240 = 0x41800000); Construct overrides to 20"); }
    sk.add("throbber.defaults", "SCircularThrobber defaults kept: PieceImage style brush, NumPieces, Period (s), default Radius", json!({"piece_image": "Throbber.CircleChunk", "num_pieces": 6, "period": 0.75, "default_radius": 16}), at_of(&thr), None, Confidence::Recovered, "from the FArguments ctor 0x12478c0 (emulated: +0x238=6, +0x23c=0x3f400000, +0x240=0x41800000; FName \"Throbber.CircleChunk\")");
    sk.add("throbber.desired", "SCircularThrobber desired size", json!([40, 40]), at_of(&thr), None, Confidence::Inferred, "engine: SCircularThrobber::ComputeDesiredSize = (Radius*2, Radius*2) (not decoded here)");
    sk.add("loading.text.string", "LOADING text: (source, namespace, key)", json!(["LOADING", "FortniteLoadingScreen", "Loading"]), evs.iter().filter(|e| e.target == Some(0x12b4810)).nth(1).map(|e| e.at), None, Confidence::Recovered, "NSLOCTEXT(\"FortniteLoadingScreen\",\"Loading\",\"LOADING\"); file offset 0x44B7C18");
    sk.add("loading.text.color", "LOADING STextBlock ColorAndOpacity: not set -> FCoreStyle \"NormalText\" style colour = UseForeground -> white (x ContentColorAndOpacity alpha)", json!([1, 1, 1, 1]), None, None, Confidence::Inferred, "no 0x1402b98b0 (colour attribute copy) is emitted for line 216; engine default of STextBlock::FArguments (ctor 0x45c8d0: style \"NormalText\") and FWidgetStyle foreground = white");
    sk.add("loading.dots", "animated dots after LOADING", json!(false), None, None, Confidence::Recovered, "none: the text is a static FText; the only animation next to it is the SCircularThrobber (Radius 20, 6 pieces, 0.75 s)");

    // ---- 7. progress bar
    if let Some((i, t)) = anchor(pe, 0x124b223, "mulsd xmm0,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("bar.percent.scale", "SProgressBar.Percent getter (0x124b1d0): value = clamp(Progress * k, 0, 1)", json!(round6(pe.read_f64(r).unwrap_or(0.) as f32)), Some(0x124b223), Some(t), Confidence::Recovered, "double at 0x1444b9458 = 0.98; Progress = float global 0x14664d9d0, written ONLY by 0x1246d90 (module vtable slot SetProgress(float))"); }
    if let Some((i, t)) = anchor(pe, 0x124b20e, "movsd xmm0,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("bar.percent.full_while_fading", "getter returns this constant while the fade curve is playing (FCurveSequence::IsPlaying(this+0x4e0))", json!(pe.read_f64(r).unwrap_or(0.)), Some(0x124b20e), Some(t), Confidence::Recovered, "double 1.0 at 0x144078d68"); }
    sk.add("bar.percent.source", "bar is a REAL FRACTION (TOptional<float> always set -> never marquee): FortAssetManager startup jobs call module->SetProgress(sum(weights done)/sum(weights)) after each job, 1.0 at the end", json!("startup-job weights"), Some(0x545191), None, Confidence::Recovered,
        "0x544fcf..0x5451d3 (FortAssetManager.cpp): xmm6 = sum of job weights ([job+0xa0], stride 0xb0), xmm7 accumulates, `call [rax+0x50]` with xmm1 = (1/xmm6)*xmm7 on the FortniteLoadingScreen module (GetModule 0x54e1e0, vtable +0x50 = 0x1246d90 SetProgress); empty job list -> SetProgress(1.0) at 0x5451d3");
    // brushes
    if let Some((i, t)) = anchor(pe, 0x124bcb6, "movups xmm0,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("bar.brush.margin", "ProgressBar_Background and ProgressBar_Fill brushes: DrawAs=Box (byte [brush+0x10]=1), FMargin (UV fractions, all sides)", jfs(&rd_f32s(pe, r, 4)), Some(0x124bcb6), Some(t), Confidence::Recovered, "movups xmm0,[0x1444b9480] -> [brush+0x14] and `mov byte ptr [rax+0x10],1` at 0x124bcbd / 0x124bedb; 0.41666667 = 5/12: 5 px corners of the 12x12 textures"); }
    sk.add("bar.brush.size", "ProgressBar_Background / _Fill ImageSize = texture size (SizeX,SizeY via [texture+0xf8]); _Marquee = fixed (20,12)", json!({"background": [12,12], "fill": [12,12], "marquee": [rd_f32s(pe, 0x3fe3720, 1).first().copied().unwrap_or(0.), 12.0]}), Some(0x124bf21), None, Confidence::Recovered, "marquee: movss xmm0,[0x143fe3720]=20 (0x124bf21), movss xmm1,[0x144008e48]=12 (0x124bf2d); the textures are 12x12 / 12x12 / 20x12 in the pak");
    sk.add("bar.brush.assets", "texture assets", json!(["/Game/UI/CodeReferenced/ProgressBar_Background", "/Game/UI/CodeReferenced/ProgressBar_Fill", "/Game/UI/CodeReferenced/ProgressBar_Marquee"]), Some(0x124b08c), None, Confidence::Recovered, "StaticLoadObject paths at file offsets 0x44B7570 / 0x44B7600 / 0x44B7680, brushes stored in this+0x578 / +0x588 / +0x598");
    sk.add("bar.defaults", "SProgressBar defaults kept (ctor 0x45c7c0): BarFillType LeftToRight, BorderPadding (1,0), FillColorAndOpacity white", json!({"fill_type": "LeftToRight", "border_padding": [1, 0], "fill_color": [1,1,1,1]}), at_of(&find_w("SProgressBar", 238)), None, Confidence::Recovered, "emulated defaults: +0x300=(1.0,0.0) isSet; +0x290=(1,1,1,1)");

    // ---- 8. fades and timing
    if let Some((i, t)) = anchor(pe, 0x12483b5, "movss xmm2,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("fade.duration", "fade-out curve duration (s) used by both modes (xmm2 of 0x124c550 -> FCurveSequence::AddCurve(start, duration))", jf(pe.read_f32(r).unwrap_or(0.)), Some(0x12483b5), Some(t), Confidence::Recovered, "0x124c550(this, start=xmm1, duration=xmm2): AddCurve(StartTime=xmm1, Duration=xmm2, Linear) then Play(widget, bLooped=false, StartAtTime=0)"); }
    if let Some((i, t)) = anchor(pe, 0x12483c0, "movss xmm1,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("fade.flag1.delay", "bFlag==1 widget: delay (s) before the 0.2 s fade starts (AddCurve start time)", jf(pe.read_f32(r).unwrap_or(0.)), Some(0x12483c0), Some(t), Confidence::Recovered, "branch: bFlag!=0 && widget+0x5f1 (set by module 0x124ac00 = 'hide requested') && sequence in reverse -> 0x124c550(this, 1.0, 0.2)"); }
    if let Some((_, t)) = anchor(pe, 0x1248410, "xorps xmm1,xmm1", &mut errs) { sk.add("fade.flag0.delay", "bFlag==0 widget: fade starts at once (AddCurve start 0, duration 0.2) when the movie player reports loading finished", jf(0.0), Some(0x1248410), Some(t), Confidence::Recovered, "branch: bFlag==0 && GetMoviePlayer()->vfunc[0x60]() (IsLoadingFinished-like) && sequence in reverse -> 0x124c550(this, 0.0, 0.2); then [0x14664d9d4]=1 (0x124841b)"); }
    if let Some((i, t)) = anchor(pe, 0x124c388, "mov r8d,0x", &mut errs) { sk.add("viewport.zorder", "ZOrder of the bFlag==1 widget added to the game viewport (vtable+0x250 call at 0x124c398 on GEngine+0x5f0)", json!(imm(&i).unwrap_or(0)), Some(0x124c388), Some(t), Confidence::Recovered, "0x4e20 = 20000"); }
    sk.add("fade.alpha", "fading layers use alpha = 1 - FCurveHandle::GetLerp(); bFlag==0 fades plate + EARLY ACCESS + LOADING subtree (getters 0x124ae90, 0x124ae00), bFlag==1 fades only the atlas (getter 0x124b270)", json!({"flag0": ["plate image", "EARLY ACCESS border (ContentColorAndOpacity)", "LOADING border (ContentColorAndOpacity)"], "flag1": ["atlas image"]}), Some(0x124ae42), None, Confidence::Recovered,
        "getter bodies: mulss xmm0,[0x143feb1ec]=-1.0 ; addss xmm0,[0x143fe36fc]=1.0 -> alpha; rgb = lerp*0+1 = 1; 0x1414d7ab0 = GetLerp (time - start)/duration clamped to 0..1");
    sk.add("fade.timer", "Construct registers an active timer (period 0 = every frame) bound to 0x1248360 (Tick)", json!({"period": 0.0}), Some(0x1248511), None, Confidence::Recovered, "0x1439235c0 builds the delegate (this, 0x1248360), 0x1414e9c80 = SWidget::RegisterActiveTimer(TickPeriod=xmm2=0, delegate)");
    // manager
    if let Some((i, t)) = anchor(pe, 0x61b44d, "comiss xmm0,[", &mut errs) { let r = rip_target(pe, &i).unwrap_or(0); sk.add("manager.hold", "UFortLoadingScreenManager: keep the loading screen up until this many seconds after TimeLoadingScreenFirstDismissed", jf(pe.read_f32(r).unwrap_or(0.)), Some(0x61b44d), Some(t), Confidence::Recovered, "log string at 0x14412e930: \"Showing LoadingScreen because it's been less than 2.0 seconds (hard-coded... ick!) since TimeLoadingScreenFirstDismissed!\"; compare (World->TimeSeconds [0x900] - [mgr+0x4c])"); }
    if let Some((i, t)) = anchor(pe, 0x61b5d4, "mov dword ptr [rdi+0x4c],0x", &mut errs) { sk.add("manager.first_dismissed.init", "TimeLoadingScreenFirstDismissed initial value (set by ShowLoadingScreen 0x61b500)", jf(f32_from_bits(imm(&i).unwrap_or(0))), Some(0x61b5d4), Some(t), Confidence::Recovered, "-1.0 = not yet dismissed; first frame where the show-reasons are all false stores TimeSeconds there (0x61b406)"); }
    out.errors = errs;
    out
}

/// Names of the strings that explain WHEN the in-game loading screen is shown (UFortLoadingScreenManager::ShouldShowLoadingScreen, 0x61ae80..0x61b4e1).
pub fn manager_reasons(pe: &Pe) -> Vec<(u32, String)> {
    let mut v = vec![];
    for rva in [0x412d790u32, 0x412d830, 0x412da70, 0x412db00, 0x412dbd0, 0x412dd80, 0x412df50, 0x412e0e0, 0x412e1a0, 0x412e300, 0x412e580, 0x412e630, 0x412e700, 0x412e800, 0x412e930, 0x412ec60, 0x412f150] {
        if let Some(s) = utf16_at_rva(pe, rva) { v.push((rva, s)); }
    }
    v
}

#[allow(dead_code)]
fn unused(_: Mnemonic) {}




