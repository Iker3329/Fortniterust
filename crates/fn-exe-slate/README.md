# fn-exe-slate

Static, read-only reverse engineering of the 1.8 client (`FortniteClient-Win64-Shipping.exe`, UE 4.16, x64). The EXE is never executed. The crate recovers the
**start-up loading screen** (`SInitialFortniteLoadingScreen`, `FortniteLoadingScreen.cpp`) and the code that frames it (`FFortniteLoadingScreenModule`,
`UFortLoadingScreenManager`, `FortAssetManager` progress feed).

Outputs: `reports/exe-loading-screen.json` (every finding with RVA/VA, instruction text, confidence) and
`crates/fn-ui/src/screens/startup_layout.rs` (the consumer: constants + `StartupLayout`). Tests: `cargo test -p fn-exe-slate` (needs the exe; skips without it,
`FN_EXE` overrides the path) and `cargo test -p fn-ui --lib startup`.

```
cargo run -p fn-exe-slate --bin gen-report            # rewrites reports/exe-loading-screen.json
cargo run -p fn-exe-slate --bin explore -- <command>  # sections | strings | find <text> | xrefs <rva-lo> [hi] | xi <lo> [hi] | callers <rva> | ptrs <rva>
                                                      # dis <rva> [n] | func <rva> | funcs <lo> <hi> | emu <rva> | ctor <rva> | vt <rva> <n> | strs <lo> <hi>
                                                      # sweep | findings | bytes <rva> <n>     (all addresses are RVAs, hex; VA = 0x140000000 + RVA)
```

## Method

1. **PE loader** (`pe.rs`, `object` crate for headers/sections + own `.pdata` parser). The user's numbers are *file offsets*: `.rdata` is raw `0x3fa3800` <-> RVA
   `0x3fa5000`, so `RVA = file offset + 0x1800` for every one of them. Strings are probed both as ASCII and UTF-16LE: asset paths, `EARLY ACCESS` and `LOADING` are
   UTF-16 (`wchar_t` TEXT literals), class names and source paths are ASCII. `Pe::func_group` follows UNWIND_INFO chains so cold paths / funclets belong to their
   parent function (adjacent unrelated functions are not merged).
2. **Cross references** (`xref.rs`). Every `.pdata` function in `.text` is decoded with `iced-x86` (multi-threaded, ~0.6 s). RIP-relative `lea`/memory operands and direct
   `call`/`jmp` are indexed by target. `XrefIndex::refs_to(rva)`, `callers_of(fn)`; a call-graph walk is `callers_of` iterated over function groups (see `tests/exe.rs`).
3. **Locating Construct** (`slate::locate_construct`). Each `SNew` passes the source path literal to `SWidget::SetDebugInfo(Type, File, Line)`; the one function with
   the most LEA references to the `FortniteLoadingScreen.cpp` literal (15) is `SInitialFortniteLoadingScreen::Construct`, RVA `0x1248480` (VA `0x141248480`).
   Its callers (exactly the functions that reference the class-name string) are the two creators. For `SFortLoadingScreen.cpp` the compiler emitted one literal per `SNew`,
   so all copies are searched (`slate::locate_by_source_file`): 25 references -> `0x301dc90`.
4. **Emulator-lite** (`emu.rs`). UE's Slate builder pattern compiles to inline stores into stack `FArguments` structs plus calls. The emulator walks one function in address
   order (jumps are not followed, so an `if/else` executes both arms, later assignment wins; every such case is called out below), tracks constants through registers, the
   stack frame and objects returned by calls (`ret#n`), models `movss/movaps/cvtdq2ps/shufps/mulss/...`, and prints each call with its arguments and the contents of the
   objects it receives. `Emu::cell_u32/cell_f32` read fields back programmatically (used by `findings.rs`).
5. **Dictionary of setters** (`slate.rs::dictionary`, built by reading Construct and confirmed exe-wide by `slate::sweep_constructs`: 809 `SNew` sites decoded; e.g.
   `SBorder` -> `0x1415c3160` in 48/48 sites, `SBox` -> `0x1415c3440` 36/36, `SImage` -> `0x1415c62a0` 53/53, `SProgressBar` -> `0x141600210` 5/5, `SHorizontalBox` ->
   `0x1414a8120` 43/44). Setter offsets inside `FArguments` structs were read from the FArguments constructors with `explore ctor` (they write the defaults).
6. **Findings** (`findings.rs`). Every number is read from the instruction/data it lives in. *Anchored* probes decode the instruction at an RVA and assert its text (a different
   binary fails loudly); *emulated* probes read the `FArguments` objects and call arguments. `report.rs` serialises them; `tests/exe.rs` checks the report round-trips and
   that `startup_layout.rs` carries exactly these numbers.

## Dictionary (VA; names are inferred from usage and from the exe-wide sweep)

| VA | role |
|---|---|
| `0x1414ef730` | `SWidget::SetDebugInfo(Type, File, Line)` (every SNew) |
| `0x1414a81c0` | `SWidget::SWidgetConstruct(base SLATE_ARGUMENTs: +0x60 tooltip, +0x70 cursor, +0xc0 enabled, +0x110 visibility, +0x160 render transform, +0x1c0 pivot, +0x210 tag, +0x218 volatile, +0x220 metadata)` |
| `0x1415c3160 / 3440 / 62a0 / 3d80`, `0x141600210 / 430`, `0x141630c40`, `0x1414a8120` | `SBorder / SBox / SImage / SCircularThrobber / SProgressBar / SSafeZone / STextBlock / SBoxPanel (SHorizontalBox, SVerticalBox)::Construct(this, FArguments&)` |
| `0x14124c400` | `SOverlay::FSlot::Padding(L,T,R,B)` (xmm1 L, xmm2 T, xmm3 R, `[rsp+0x20]` B); slot object: +0x18 HAlign, +0x1c VAlign, +0x20 Padding |
| `0x14124c4a0` / `0x141561eb0` | `SBoxPanel::FSlot::Padding(L,T,R,B)` / `Padding(uniform)`; slot: +0x80 HAlign, +0x84 VAlign, +0x90 Padding |
| `0x1404632e0` | `AutoWidth()/AutoHeight()` (writes SizeRule 0 = Auto at +0x20; default from `0x14045d840` is Stretch(1)) |
| `0x14124c810`, `0x141582900`, `0x141647600` | `SOverlay::Slot()` (Fill/Fill, padding 0), `FSlot::operator[]`, `SOverlay::FArguments::operator+` |
| `0x141494180` | `FSlateFontInfo(UObject* Font, int32 Size, FName Typeface)` (r8d = size) |
| `0x1412b4810` | `NSLOCTEXT(Namespace, Key, Source)` |
| `0x141247ab0` | `FSlateDynamicImageBrush(UTexture*, ImageSize, Name, Tint, ...)` |
| `0x1424fb740` | `UUserInterfaceSettings::GetDPIScaleBasedOnSize(FIntPoint)` |
| `0x1415ed660 / 0x141247150 / 0x1412475c0` | fluent `ContentColorAndOpacity(this,&Getter)` (SBorder) / `ColorAndOpacity` (SImage) / `Percent` (SProgressBar) |
| FArguments ctors | `0x14045c3f0` SBorder, `0x14045c5e0` SBox, `0x14045c7c0` SProgressBar, `0x14045c8d0` STextBlock, `0x1404d1830` SImage, `0x1412478c0` SCircularThrobber, `0x141247a10` SSafeZone |
| enums | HAlign: Fill 0, Left 1, Center 2, Right 3. VAlign: Fill 0, Top 1, Center 2, Bottom 3 |

## What the start-up screen is (recovered)

```
SInitialFortniteLoadingScreen::Construct(this, style, bool bFlag)      0x141248480   (bFlag stored at this+0x5f0)
 SDPIScaler (184)  DPIScale = GetDPIScaleBasedOnSize(GetSystemResolution)  -- a value stored once, not a delegate
  SOverlay (187)  slots, back to front (SOverlay::FArguments::operator+ order 0x124a2b6, 2c1, 2cc, 2d7):
   [0] SImage (190)    atlas brush, ColorAndOpacity = getter 0x124b270; slot Fill/Fill, padding 0
   [1] SSafeZone (197) default args > SBorder (199) {HAlign Fill, VAlign Bottom, padding (16,4,16,4), NoBorder, ContentColorAndOpacity getter 0x124ae00}
         > SVerticalBox (206)
             slot {HAlign Left, AutoHeight}              > SHorizontalBox (210)
                  slot {AutoWidth, VAlign Center, Padding 0}      > STextBlock (216) "LOADING"
                  slot {AutoWidth, VAlign Center, Padding L16}    > SCircularThrobber (226) Radius 20
             slot {AutoHeight, Padding (0,6,0,12), HAlign Fill}   > SBox (235) HeightOverride 6 > SProgressBar (238)
   [2] SImage (256)    plate brush (= texture size 584x160), ColorAndOpacity getter 0x124ae90; slot {Right, Top, Padding (0,40,60,0)}
   [3] SBorder (266; NoBorder, padding 0, ContentColorAndOpacity 0x124ae00)
         > SBox (271; Width/HeightOverride = plate brush size, content Center/Center, Padding (0,7,7,0)) > STextBlock (278) "EARLY ACCESS"
       slot {Right, Top, Padding (0,40,60,0)}   -> the text box coincides with the plate rectangle
```

Visual rules at window `W x H` with `s = dpi_scale(W,H)` (units x s = pixels):

* atlas: stretched over the whole window (SImage in a Fill/Fill slot: aspect not preserved). Only exists when `GSystemResolution.ResX >= 1920` (else null brush, nothing drawn).
* plate: 584x160 units at `(W/s - 60 - 584, 40)`.
* EARLY ACCESS: `BurbankBigRegular-Bold` (typeface `Burbank-PlayerPower`) 48 units, **black** `(0,0,0,1)`, centred in the plate rect inset by padding `(L0,T7,R7,B0)`.
* LOADING: same font and size, white (style default `UseForeground`), left edge at 16 units, row bottom `4 + 12 + 6 + 6 = 28` units above the window bottom; a throbber
  (radius 20, 6 pieces, 0.75 s) sits 16 units to the right of the text, vertically centred in the row. **No dots.**
* progress bar: `x = 16 .. W/s - 16`, 6 units tall, bottom edge 16 units above the window bottom; Box brushes 12x12 with 5/12 margins. The value is a **real fraction**:
  `clamp(Progress * 0.98, 0, 1)`; Progress is pushed by `FortAssetManager` startup jobs (sum of finished job weights / total). No marquee (the `Percent` getter always returns a value).
* fade: curve of 0.2 s, alpha `1 - lerp`. `bFlag == 0` (the start-up widget, shown through the movie player): plate, EARLY ACCESS and the LOADING subtree fade when the
  movie player reports loading finished; the atlas stays. `bFlag == 1` (added to the game viewport at ZOrder 20000 when the movie finishes): after the module's
  "hide" request it waits **1.0 s**, then fades only the atlas over 0.2 s, then removes itself.

## Claims (generated from `reports/exe-loading-screen.json`)

`recovered` = read from the instruction/data (RVA given); `inferred` = recovered fact + documented engine behaviour (see the note in the JSON); `assumed` = not in the exe.

| id | confidence | rva | value | what |
|---|---|---|---|---|
| `construct.rva` | recovered | `0x1248480` | `"0x1248480"` | SInitialFortniteLoadingScreen::Construct(this, style, bFlag) |
| `construct.callers` | recovered | `0x1246a66` | `["0x1246a66","0x124c21f"]` | call sites of Construct (creators of the widget) |
| `creator.startup.flag` | recovered | `0x1246946` | `0` | bFlag passed by the start-up creator (0x12468d0 -> 0x1246953 tail) |
| `creator.movie_finished.flag` | recovered | `0x124c148` | `1` | bFlag passed by the OnMoviePlaybackFinished creator (0x124c0e0) |
| `flag.field` | recovered | `0x12484cf` | `"0x5f0"` | Construct stores its bFlag at widget+0x5f0 (and clears widget+0x5f1 at 0x12484ef) |
| `tree.snew` | recovered | `0x1248b91` | `["STextBlock@278","SBox@271","SBorder@266","SImage@256","SProgressBar@238","SBox@235","...` | SNew call sites in Construct, execution order = innermost first (class@line) |
| `dpi.call` | recovered | `0x12487e6` | `"0x24fb740"` | DPI scale = UUserInterfaceSettings::GetDPIScaleBasedOnSize(system resolution), stored once as SDPIScaler.DPIScale (a value, not a delegate) |
| `tree.dpi_scaler.line` | recovered | `0x124a381` | `184` | SDPIScaler is the root: ChildSlot of the widget (this+0x338) |
| `atlas.min_res_x` | recovered | `0x124af74` | `1920.0` | the atlas brush is only created when GSystemResolution.ResX >= this (else the SImage gets a null brush and draws nothing) |
| `atlas.brush_w` | recovered | `0x124af66` | `1920.0` | atlas FSlateBrush.ImageSize.X |
| `atlas.brush_h` | recovered | `0x124af5b` | `1080.0` | atlas FSlateBrush.ImageSize.Y |
| `atlas.asset` | recovered | `0x124af8c` | `"/Game/UI/Foundation/Textures/LoadingScreens/FORT_Load_Atlas.FORT_Load_Atlas"` | StaticLoadObject path of the atlas texture |
| `atlas.image.slot` | recovered | `0x124a084` | `{"halign":"Fill","padding":[0,0,0,0],"valign":"Fill"}` | atlas SImage lives in an SOverlay slot created by SOverlay::Slot() (alignment Fill/Fill, padding 0) with ZOrder = first (back) |
| `atlas.draw` | inferred | `0x124a084` | `"stretch to window"` | SImage default: brush drawn stretched over the allotted rect (DrawAs=Image, no tiling, no aspect-fit) |
| `atlas.color_getter` | recovered | `0x1249fca` | `"0x124b270"` | atlas SImage.ColorAndOpacity is bound to getter 0x124b270 (FSlateColor): bFlag==0 -> white; bFlag!=0 -> alpha = 1 - FadeCurve.GetLerp() |
| `plate.slot.padding` | recovered | `0x124a22f` | `[0.0,40.0,60.0,0.0]` | Padding(L,T,R,B) of BOTH top-right overlay slots (plate image and the EARLY ACCESS text border) |
| `plate.slot.halign` | recovered | `0x124a1b2` | `3` | plate/text overlay slot HAlign (EHorizontalAlignment: Fill0 Left1 Center2 Right3) |
| `plate.slot.valign` | recovered | `0x124a1b9` | `1` | plate/text overlay slot VAlign (EVerticalAlignment: Fill0 Top1 Center2 Bottom3) |
| `plate.brush.size` | inferred | `0x124b877` | `[584,160]` | plate FSlateDynamicImageBrush.ImageSize = (UTexture2D+0xf8 -> SizeX, SizeY) of T-Loading-EarlyAccessPlate |
| `plate.brush.fallback` | recovered | `0x1248a15` | `[300,100]` | WidthOverride/HeightOverride of the text SBox when the plate brush is missing |
| `ea.box.align` | recovered | `0x1248d30` | `[2,2]` | EARLY ACCESS SBox content alignment (HAlign, VAlign) |
| `ea.box.padding` | recovered | `0x1248970` | `[0.0,7.0,7.0,0.0]` | EARLY ACCESS SBox Padding(L,T,R,B) |
| `ea.box.size` | recovered | `0x1248d30` | `{"height":"brush.ImageSize.Y","width":"brush.ImageSize.X"}` | EARLY ACCESS SBox Width/HeightOverride = plate brush ImageSize |
| `text.font.size` | recovered | `0x124890e` | `[48,48]` | FSlateFontInfo size (Slate units) for EARLY ACCESS and LOADING |
| `text.font.typeface` | recovered | `0x124890e` | `["Burbank-PlayerPower","Burbank-PlayerPower"]` | typeface FName of both texts |
| `text.font.asset` | recovered | `` | `"/Game/UI/Foundation/Fonts/Burbank.Burbank"` | font object loaded with StaticLoadObject(UFont) |
| `ea.text.color` | recovered | `0x124886a` | `[0.0,0.0,0.0,1.0]` | EARLY ACCESS STextBlock.ColorAndOpacity (FSlateColor, ColorUseRule=UseColor_Specified): FLinearColor RGBA |
| `ea.text.string` | recovered | `0x1248771` | `["EARLY ACCESS","FortniteLoadingScreen","EarlyAccess"]` | EARLY ACCESS text: (source, namespace, key) |
| `ea.text.layers` | recovered | `0x124a2b6` | `["atlas SImage (Fill/Fill)","SSafeZone \u003e SBorder \u003e SVerticalBox (LOADING row ...` | z-order inside the overlay (back to front) |
| `ea.border.padding` | recovered | `0x1248e9b` | `{"border_image":"NoBorder (FCoreStyle brush, draws nothing)","padding":[0.0,0.0,0.0,0.0]}` | EARLY ACCESS SBorder padding and brush |
| `loading.border.align` | recovered | `0x1249dbc` | `[0,3]` | LOADING container SBorder HAlign, VAlign |
| `loading.border.padding` | recovered | `0x12492b2` | `[16.0,4.0,16.0,4.0]` | LOADING container SBorder Padding(L,T,R,B) |
| `loading.safezone` | recovered | `0x1249ee8` | `{"IsTitleSafe":false,"SafeAreaScale":[1,1,1,1],"pad":[true,true,true,true]}` | LOADING subtree is wrapped in an SSafeZone with the default arguments (not title-safe, margin scale 1, pad all sides) |
| `bar.slot.padding` | recovered | `0x1249606` | `[0.0,6.0,0.0,12.0]` | SVerticalBox slot holding the progress SBox: Padding(L,T,R,B), AutoHeight |
| `throbber.slot.padding` | recovered | `0x12497d9` | `[16.0,0.0,0.0,0.0]` | SHorizontalBox slot holding the SCircularThrobber: Padding(L,T,R,B), AutoWidth, VAlign Center |
| `text.slot.padding` | recovered | `0x12499b7` | `0.0` | SHorizontalBox slot holding the LOADING STextBlock: Padding(uniform), AutoWidth, VAlign Center |
| `row.slot.halign` | recovered | `0x1249b7d` | `1` | SVerticalBox slot holding the LOADING SHorizontalBox: HAlign (Left=1), AutoHeight |
| `bar.height` | recovered | `0x124911e` | `6.0` | progress-bar SBox HeightOverride |
| `throbber.radius` | recovered | `0x1249658` | `20.0` | SCircularThrobber.Radius |
| `throbber.defaults` | recovered | `0x12496c4` | `{"default_radius":16,"num_pieces":6,"period":0.75,"piece_image":"Throbber.CircleChunk"}` | SCircularThrobber defaults kept: PieceImage style brush, NumPieces, Period (s), default Radius |
| `throbber.desired` | inferred | `0x12496c4` | `[40,40]` | SCircularThrobber desired size |
| `loading.text.string` | recovered | `0x124914b` | `["LOADING","FortniteLoadingScreen","Loading"]` | LOADING text: (source, namespace, key) |
| `loading.text.color` | inferred | `` | `[1,1,1,1]` | LOADING STextBlock ColorAndOpacity: not set -> FCoreStyle "NormalText" style colour = UseForeground -> white (x ContentColorAndOpacity alpha) |
| `loading.dots` | recovered | `` | `false` | animated dots after LOADING |
| `bar.percent.scale` | recovered | `0x124b223` | `0.98` | SProgressBar.Percent getter (0x124b1d0): value = clamp(Progress * k, 0, 1) |
| `bar.percent.full_while_fading` | recovered | `0x124b20e` | `1.0` | getter returns this constant while the fade curve is playing (FCurveSequence::IsPlaying(this+0x4e0)) |
| `bar.percent.source` | recovered | `0x545191` | `"startup-job weights"` | bar is a REAL FRACTION (TOptional<float> always set -> never marquee): FortAssetManager startup jobs call module->SetProgress(sum(weights done)/sum(weights)) after each job, 1.0 at the end |
| `bar.brush.margin` | recovered | `0x124bcb6` | `[0.416667,0.416667,0.416667,0.416667]` | ProgressBar_Background and ProgressBar_Fill brushes: DrawAs=Box (byte [brush+0x10]=1), FMargin (UV fractions, all sides) |
| `bar.brush.size` | recovered | `0x124bf21` | `{"background":[12,12],"fill":[12,12],"marquee":[20.0,12.0]}` | ProgressBar_Background / _Fill ImageSize = texture size (SizeX,SizeY via [texture+0xf8]); _Marquee = fixed (20,12) |
| `bar.brush.assets` | recovered | `0x124b08c` | `["/Game/UI/CodeReferenced/ProgressBar_Background","/Game/UI/CodeReferenced/ProgressBar_...` | texture assets |
| `bar.defaults` | recovered | `0x1249392` | `{"border_padding":[1,0],"fill_color":[1,1,1,1],"fill_type":"LeftToRight"}` | SProgressBar defaults kept (ctor 0x45c7c0): BarFillType LeftToRight, BorderPadding (1,0), FillColorAndOpacity white |
| `fade.duration` | recovered | `0x12483b5` | `0.2` | fade-out curve duration (s) used by both modes (xmm2 of 0x124c550 -> FCurveSequence::AddCurve(start, duration)) |
| `fade.flag1.delay` | recovered | `0x12483c0` | `1.0` | bFlag==1 widget: delay (s) before the 0.2 s fade starts (AddCurve start time) |
| `fade.flag0.delay` | recovered | `0x1248410` | `0.0` | bFlag==0 widget: fade starts at once (AddCurve start 0, duration 0.2) when the movie player reports loading finished |
| `viewport.zorder` | recovered | `0x124c388` | `20000` | ZOrder of the bFlag==1 widget added to the game viewport (vtable+0x250 call at 0x124c398 on GEngine+0x5f0) |
| `fade.alpha` | recovered | `0x124ae42` | `{"flag0":["plate image","EARLY ACCESS border (ContentColorAndOpacity)","LOADING border ...` | fading layers use alpha = 1 - FCurveHandle::GetLerp(); bFlag==0 fades plate + EARLY ACCESS + LOADING subtree (getters 0x124ae90, 0x124ae00), bFlag==1 fades only the atlas (getter 0x124b270) |
| `fade.timer` | recovered | `0x1248511` | `{"period":0.0}` | Construct registers an active timer (period 0 = every frame) bound to 0x1248360 (Tick) |
| `manager.hold` | recovered | `0x61b44d` | `2.0` | UFortLoadingScreenManager: keep the loading screen up until this many seconds after TimeLoadingScreenFirstDismissed |
| `manager.first_dismissed.init` | recovered | `0x61b5d4` | `-1.0` | TimeLoadingScreenFirstDismissed initial value (set by ShowLoadingScreen 0x61b500) |

## Annotated disassembly of the key functions

All addresses are VAs. `;` comments are mine.

### Construct: DPI, text, fonts (0x141248480)

```
141248511 call 0x1439235c0            ; delegate(this, 0x141248360 = Tick)           -- RegisterActiveTimer(TickPeriod = xmm2 = 0, ...) at 0x14124852b
141248608 call 0x14216f9e0            ; GetSystemResolution(&x,&y,&mode)
14124862c call 0x14124b4c0            ; MakeAtlasBrush -> this+0x548        14124b790 MakePlateBrush -> this+0x558
141248744 call 0x14124ba60            ; MakeProgressBarBrushes -> this+0x578/+0x588/+0x598
141248755 lea r9,[0x1444b92e8]        ; L"EarlyAccess"   (key)
14124875c lea r8,[0x1440ef580]        ; L"FortniteLoadingScreen" (namespace)
141248763 lea rdx,[0x1444b9300]       ; L"EARLY ACCESS" (source)
141248771 call 0x1412b4810            ; NSLOCTEXT
1412487e6 call 0x1424fb740            ; xmm0 = DPI scale (xmm6) -> later [rbp+0x670] = SDPIScaler.DPIScale, isSet [rbp+0x674] = 1
1412487fa call 0x14258e570            ; UFont::StaticClass()
141248804 lea r8,[0x1444b8d10]        ; L"/Game/UI/Foundation/Fonts/Burbank.Burbank"   -> StaticLoadObject 0x14142e560 at 0x14124881c
14124886a movups xmm0,[0x145198810]   ; FLinearColor::Black (0,0,0,1)   (.data, constant-initialised; no instruction writes it)
14124887e lea rdx,[0x1444b93f0]       ; "Burbank-PlayerPower"  -> FName (0x141333f60 at 0x1412488c7)
141248890 movdqa [rbp+0x600],xmm0     ; FSlateColor.SpecifiedColor
14124889e mov [rbp+0x610],sil         ; ColorUseRule = 0 (UseColor_Specified)
1412488b3 mov byte ptr [rbp+0x628],1  ; TAttribute isSet
1412488eb mov r8d,0x30                ; size 48
14124890e call 0x141494180            ; FSlateFontInfo(Burbank, 48, FName)
141248970 movaps xmm0,[0x1444b9470]   ; (0, 7, 7, 0)  SBox padding -> [rbp+0x9b0]
141248b0c call 0x1402b98b0            ; STextBlock args +0x350 (ColorAndOpacity) <- [rbp+0x600]      (LOADING has no such call)
141248b91 call 0x1414ef730            ; SetDebugInfo("STextBlock", FortniteLoadingScreen.cpp, 278)
```

### Construct: the SBox around EARLY ACCESS (FArguments is the object printed by `explore emu 1248480`)

```
1412489e5 movss xmm0,[0x143fe3730] (100.0) / 141248a15 movss xmm0,[0x143ff9040] (300.0) ; else-arms of `brush ? brush->ImageSize.{Y,X} : {100,300}` (the emulator shows these default arms)
141248c6d movss [rax+0x2b0],xmm0      ; SBox.WidthOverride   (+0x2b4 isSet)       141248c9d movss [rbx+0x300],xmm0 ; HeightOverride (+0x304)
141248cbe mov dword ptr [rbx+0x230],2 ; HAlign Center          141248ccf mov dword ptr [rbx+0x234],2 ; VAlign Center
141248ce0 movaps xmm0,[rbp+0x9b0]     ; padding (0,7,7,0)       141248ce7 movups [rbx+0x240],xmm0 ; SBox.Padding
```

### Construct: LOADING row and bar

```
141249028 ...                         ; SImage plate: [rax+0x230] = brush ptr (*(this+0x558)), +0x238 isSet
14124911e mov dword ptr [rbp+0x760],0x40c00000   ; 6.0 -> SBox.HeightOverride of the progress bar (stored at 0x14124948e: [rax+0x300])
141249128 lea rdx,[0x1444b9418]       ; L"LOADING"      lea r9,[0x1444b9408] L"Loading"
141249658 mov dword ptr [rax+0x240],0x41a00000   ; SCircularThrobber.Radius = 20.0 (default 16, ctor 0x1412478c0)
1412497b0 mov dword ptr [rbx+0x84],2  ; throbber slot VAlign Center          1412497bf movss xmm1,[0x144040c9c] ; 16.0
1412497d9 call 0x14124c4a0            ; Padding(L=16, T=0, R=0, B=0)
141249606 call 0x14124c4a0            ; (progress slot) Padding(0, 6, 0, 12): xmm2 = [0x144078d7c] 6.0, [rsp+0x20] = [0x144008e48] 12.0
1412499b7 call 0x141561eb0            ; text slot Padding(0)          1412499a1 mov dword ptr [rdi+0x84],2 ; VAlign Center
141249b7d mov dword ptr [rdi+0x80],1  ; HBox slot in the VBox: HAlign Left
1412492b2 movaps xmm0,[0x1444b9490]   ; (16, 4, 16, 4) -> [rbp+0x8f0] -> SBorder.Padding (+0x250) at 141249d44
141249d29 mov dword ptr [rax+0x244],3 ; SBorder.VAlign Bottom            141249d33 mov dword ptr [rax+0x240],0 ; HAlign Fill
```

### Construct: overlay slots

```
14124a16f call 0x1405fa630 / ctor 0x141495610 ; slot object, vtable 0x1442853d0
14124a1b2 mov dword ptr [rdi+0x18],3  ; HAlign Right        14124a1b9 mov dword ptr [rdi+0x1c],1 ; VAlign Top      (second slot: 14124a1cf / 14124a1d6)
14124a20e movss xmm6,[0x144031d60]    ; 40.0                14124a21a movss xmm3,[0x143fe3728] ; 60.0
14124a22f call 0x14124c400            ; Padding(0, 40, 60, 0) for the EARLY ACCESS border slot   14124a25f: same for the plate image slot
14124a276 call 0x14124c810 / a28d     ; two default Fill/Fill slots: safe zone (content [rbp+0x120]) and atlas (content [rbp+0x130])
14124a2b6/2c1/2cc/2d7 call 0x141647600 ; operator+ : atlas, safe zone, plate image, EARLY ACCESS border   (back to front)
14124a3da call 0x1415f6510            ; ChildSlot (this+0x338) <- SDPIScaler
```

### Atlas gate and brushes

```
14124af53 movd xmm1,[0x1451b6a40]     ; GSystemResolution.ResX (int)
14124af63 cvtdq2ps xmm1,xmm1
14124af66 mov dword ptr [rsp+0x60],0x44f00000    ; ImageSize.X = 1920.0
14124af5b movss xmm0,[0x1444b9460] ; 1080.0  -> [rsp+0x64]
14124af74 comiss xmm1,[0x14401ee50]   ; 1920.0
14124af7b jae 0x14124af82             ; ResX >= 1920 -> StaticLoadObject(FORT_Load_Atlas) (lea r8,[0x1444b8ba0] at 14124af8c)   else rax = 0 (null brush)
14124b877 mov rax,[rbx+0xf8]          ; UTexture2D PlatformData: SizeX/SizeY  -> plate ImageSize (cvtdq2ps; ints at [rax], [rax+4])
14124bcb6 movups xmm0,[0x1444b9480]   ; (5/12 x4)  -> [brush+0x14] (FMargin);   14124bcbd mov byte ptr [rax+0x10],1  ; DrawAs = Box
14124bf21 movss xmm0,[0x143fe3720] 20.0, 14124bf2d movss xmm1,[0x144008e48] 12.0  ; marquee brush ImageSize (20,12)
```

### Fade getters, Tick, progress getter

```
; 0x14124b270 atlas SImage.ColorAndOpacity:                0x14124ae90 plate SImage:                    0x14124ae00 SBorder.ContentColorAndOpacity (FLinearColor)
14124b276 cmp byte ptr [rcx+0x5f0],0      ; bFlag             14124ae96 cmp ...,0 ; je -> fade (bFlag==0)        14124ae06 cmp ...,0 ; je -> fade (bFlag==0)
14124b280 je 0x14124b2e7                  ; bFlag == 0 -> (1,1,1,1)      (atlas)
14124b282 add rcx,0x538 ; call 0x1414d7ab0 ; FCurveHandle::GetLerp(this+0x538) -> xmm0
14124b293 mulss xmm0,[0x143feb1ec]        ; * -1.0
14124b2a2 addss xmm0,[0x143fe36fc]        ; + 1.0  => alpha = 1 - lerp ; rgb = lerp*0 + 1 = 1

; 0x141248360 Tick (active timer, period 0):
141248374 lea rbx,[rcx+0x4e0]             ; FCurveSequence
141248384 call 0x1414dff00 ; sete bpl     ; bpl = !IsInReverse()
141248392 call 0x1414e0050 ; movzx ebx,al ; bl  = IsPlaying()
141248397 cmp byte ptr [rsi+0x5f0],0 ; je 0x1412483ea   ; bFlag == 0 -> second branch
1412483a3 cmp byte ptr [rsi+0x5f1],0 ; je -> keep ticking (return 1)            ; bFlag==1: only after module set widget+0x5f1
1412483b0 test bpl,bpl ; jne 0x1412483d2
1412483b5 movss xmm2,[0x144008e28]        ; 0.2   (duration)   1412483c0 movss xmm1,[0x143fe36fc] ; 1.0 (start/delay)
1412483c8 call 0x14124c550                ; AddCurve(start=1.0, dur=0.2, Linear) + Play(widget, false, 0)
1412483d2 test bl,bl ; jne -> return 1    ; still playing
1412483e1 call 0x140320a40 ; xor al,al    ; fade finished: broadcast widget+0x5b0 delegate, return 0 (stop timer)
1412483ea call 0x141ebf6c0 ; call [rax+0x60]   ; bFlag == 0: GetMoviePlayer()->vfunc60() (loading finished?)
141248408 movss xmm2,[0x144008e28] 0.2 ; 141248410 xorps xmm1,xmm1 ; 141248416 call 0x14124c550   ; AddCurve(start=0, dur=0.2) + Play
14124841b mov byte ptr [0x14664d9d4],1    ; "fade started" -> OnMoviePlaybackFinished may create the bFlag==1 widget

; 0x14124b1d0 SProgressBar.Percent getter -> TOptional<float> {bSet=1, value}
14124b205 call 0x1414e0050                ; IsPlaying(this+0x4e0)
14124b20e movsd xmm0,[0x144078d68]        ; 1.0  (while the fade plays)
14124b218 movss xmm0,[0x14664d9d0]        ; Progress  (written only by 0x141246d90 = module SetProgress)
14124b223 mulsd xmm0,[0x1444b9458]        ; * 0.98     then clamp: comiss 0 / minss 1.0
```

### Framework

```
; module object factory 0x14124b390: new(0x20) { vtable 0x1444b8a78, widget ptr (+8), refctrl (+0x10), flags (+0x18) &= ~7 }
; vtable 0x1444b8a78:  +0x08 StartupModule 0x1412468d0   +0x20 ShutdownModule 0x141246d00   +0x28/30/38 stubs (mov al,1)
;                      +0x40 0x141246d60 IsShowingInitialLoadingScreen: flags & 5     +0x48 0x141246d70 SetPersist(bool): bit1, then 0x14124ac00
;                      +0x50 0x141246d90 SetProgress(float): movss [0x14664d9d0],xmm1
; StartupModule 0x1412468d0: 0x14124b3d0 preload; "nullrhi" (rva 0x44b8658) / server globals -> return; SNew(...).bFlag(0); flags |= 4;
;     GetMoviePlayer()->vfunc28(widget) (SetSlateOverlayWidget); GetMoviePlayer()->vfunc90() -> AddRaw(this, 0x14124c0e0)
; 0x14124c0e0 (movie finished): flags &= ~4; if ([0x14664d9d4] && GEngine->[0x5f0]) { SNew(...).bFlag(1); widget_slot = it;
;     viewport->vfunc250(widget, ZOrder = 0x4e20); flags |= 1; 0x14124ac00(this) }   [0x14664d9d4] = 0
; 0x14124ac00: if !(flags & 2) && widget: widget+0x5b0 = delegate(0x14124c6a0); widget+0x5f1 = 1
; progress feed (FortAssetManager.cpp, 0x140544fcf..): total = sum job[+0xa0] (stride 0xb0); per job: done += w; GetModule()->vfunc50(done / total); none: vfunc50(1.0)
; manager 0x14061ae80 ShouldShow: reasons (strings @ rva 0x412d790..0x412e930), ... 0x14061b44d comiss (TimeSeconds - TimeLoadingScreenFirstDismissed), [0x143ff9020] = 2.0
; manager 0x14061b500 Show: IsShowingInitialLoadingScreen log; module->vfunc48(true) (persist); TimeLoadingScreenFirstDismissed = -1.0 (0x14061b5d4)
; manager Hide (0x1406102e7..): module->vfunc48(false)
```

### Reasons the in-game loading screen is shown (UFortLoadingScreenManager::ShouldShowLoadingScreen log strings)

`Reason ... unknown` / no owning game instance (hide) / `bCurrentlyInLoadMap` / null WorldContext / pending travel (TravelURL not empty) / `PendingNetGame != nullptr` / no
World / `!World->HasBegunPlay` / GameState not yet replicated / UI manager state Invalid / PartyProgression Matchmaking or PostMatchmaking / WorldManager not initialised
/ `World->IsInSeamlessTravel()` / **less than 2.0 s since TimeLoadingScreenFirstDismissed**. The full text list with RVAs is `manager_reason_strings` in the JSON.

## What could not be recovered (stated plainly)

* **DPI curve / UIScaleRule** - `UUserInterfaceSettings` comes from `DefaultEngine.ini`, not from the exe. The repo's shortest-side curve (480:0.444, 720:0.666, 1080:1.0, 8640:8.0) is used (ASSUMED).
* **Exact Slate line metrics** of Burbank at 48 (FreeType/rounding): `BADGE_TEXT_BASELINE = 0.619` and `LOADING_MARGIN_BOTTOM = 20.4` assume fontdue ascent 45.6 / descent 14.4 of
  `BurbankBigRegular-Bold` (ASSUMED). Use `StartupLayout::badge_text_origin` / `loading_row` with the real atlas metrics.
* **Engine paint code**: `SProgressBar::OnPaint` (how `BorderPadding (1,0)` and the Box margins are applied), `SCircularThrobber::OnPaint` (piece layout, `ComputeDesiredSize = 2*Radius`)
  were not decoded; only the arguments the game passes were (the defaults come from the FArguments constructors).
* **Movie-player settings**: the start-up widget is NOT registered with `SetupLoadingScreen(FLoadingScreenAttributes)`: no `bAutoCompleteWhenLoadingCompletes / bWaitForManualStop /
  bAllowInEarlyStartup / bMoviesAreSkippable / MinimumLoadingScreenDisplayTime` is set by this module. It is handed to the movie player as an overlay widget (vtable slots of
  `GetMoviePlayer()`, 0x141ebf6c0, are labelled by usage, not matched to engine headers: +0x28 SetSlateOverlayWidget, +0x90 OnMoviePlaybackFinished, +0x60 loading-finished query, +0x50 stop).
  The ini keys `bWaitForMoviesToComplete`, `bMoviesAreSkippable`, `StartupMovies` (`[/Script/MoviePlayer.MoviePlayerSettings]`, rva 0x4767268..) exist as strings; their values are config.
* **Safe-zone padding** of `SSafeZone` is a platform value (0 on desktop unless a debug cvar is set) - ASSUMED 0.
* **SFortLoadingScreen** (the in-game screen with tips): only its widget tree is listed (`sweep.sfortloadingscreen_tree_by_emulation` in the JSON); no numbers.
* The emulator is straight-line: where an `if` chooses between values, **both arms run** and the later one is printed. The two cases that matter were resolved by reading the code:
  the plate SBox size (`brush ? ImageSize : 300x100`) and `ret#`-less null-path argument copies. Everything else in the claims table was read from a single instruction.

## Layout of the crate

* `src/pe.rs` PE loader, `.pdata`, unwind chains, string probes. `src/xref.rs` xref/call index. `src/emu.rs` emulator-lite. `src/slate.rs` dictionary, Construct locator, SNew sweep.
* `src/findings.rs` the extraction (`recover`). `src/report.rs` JSON builder. `src/bin/explore.rs` ad-hoc CLI. `src/bin/gen-report.rs` report writer. `tests/exe.rs` tests.

