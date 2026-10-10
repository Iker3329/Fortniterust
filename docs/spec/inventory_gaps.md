# Inventory (AthenaInventoryPanel) - gaps found by verify2 (2026-10-05)

Verified build: `fortnite-rs` (fast profile) from the working tree on 2026-10-05, 1920x1080.
Evidence: `verification/inventory/verify2/`. The scripts used are in the report's run logs (`h*`, `l*`, `o*`, `s*` shots).

## G1 - Item details header is drawn in its "schematic" look (MEDIUM, visible)

- Symptom: `ItemDetailsHeader`'s `RarityBorder` draws as a white or light-blue blueprint grid with white text on it, for every rarity.
  You can see it in `l2_tile0_selected` (Rare sniper) and `o4_qsel` (Common wood). The rarity, type and tag texts are almost unreadable.
- Cause: `ItemDetailsHeader.UpdateItemsForWidgets` sets `RarityBorderMID.SetScalarParameterValue(bIsSchematic, 0)`. `inventory_cooked.rs` stores
  this in `mat_scalars` of the `RarityBorder` widget. But `athena_lobby/paint.rs::paint_border` only copies `mat_vectors` into the material
  overrides, not `mat_scalars`. The material instance `/Game/UI/Foundation/Border/Materials/M-Border-Item-DetailsHeader-Inst` has
  `ScalarParameterValues: bIsSchematic = 1.0`, so the schematic branch runs.
- Fix: in `paint_border`, set `self.c.ui.cur_scalars = self.c.ui.w[id].mat_scalars.clone()` before `draw_brush`, then clear it after (the same as
  `paint_image`). The draw_material cache key already includes the scalars.
- Check: re-shoot `l2_tile0_selected`. The header should be the Rare blue gradient (RarityData Rare Color2 `[0, 0.4287, 1]`, Color3 and Color4 darker
  blues), with the white name readable.

## G2 - `BasicStrokeBox` "not decodable" is a name collision, not a fn-texture format gap (LOW, the slider thumb or bar is not drawn)

- `/Game/UI/Style/Materials/BasicStrokeBox` samples `FillTexture` and `StrokeTexture` = `/Game/UI/Style/BasicStrokeBox`. That is a normal
  Texture2D: 256x64, `PF_DXT5`, one mip of 16384 bytes, bulk flags 0x48 (inline). fn-texture already handles that layout.
- `slate_mat.rs:306` looks the texture up by file name only (`assets.image("BasicStrokeBox.png")`). Then `PakAssets::package_of` picks
  the first package with that stem. Two packages share it: `UI/Main/BasicStrokeBox` and `UI/Style/BasicStrokeBox`. The stems list is sorted by
  path length, so `UI/Main/BasicStrokeBox` wins, and it is not a Texture2D. Decoding fails.
- Fix (either one):
  1. Have `slate_mat` resolve a texture parameter by its full object path (`/Game/UI/Style/BasicStrokeBox`). For example, add
     `PakAssets::decode_package(long_path)` and use it when the material gives a path.
  2. Make `package_of` skip candidates whose first export class is not `Texture2D` (or `TextureRenderTarget2D`) when it resolves a `.png` stem.
- Also check: other stems with more than one package, which can silently pick the wrong texture in the lobby or the HUD as well.

## G3 - Cursor-mode entry does not cancel sprint (LOW)

- Exe: `UFortHUDContext::EnterCursorMode` (0x142e18200) calls the controller's cursor-mode toggle FUN_140824b40. On entry that function:
  sets `bHoldingSprint` (+0xF16) = 0; sets `bWantsToSprint` (+0xF15) = 0 unless `bSprintToggleable` (+0xF17); runs FUN_14084ea60, which clears a
  leftover `Gameplay.Action.Player.Sprint` tag; and calls pawn vfuncs +0x980 and +0x938 with 0 (not identified yet; likely stop fire and
  stop targeting).
- Port: `app.rs` `cursor_mode` only zeroes attack and look. A held sprint key keeps sprinting with the panel open.
- Fix: when the panel opens (`toggle_inventory`, open branch), latch sprint off. Ignore the sprint key's held state until it is released and pressed
  again, unless sprint is set to toggle. Identify vfuncs 0x980 and 0x938 before porting them.
- Movement itself stays live in 1.8. `UAthenaInventoryPanelBase::NotifyPanelActivated` (0x142dd0f50) pauses nothing. It only binds
  `HandleCursorModeChanged` and refreshes the filter and sorter.

## G4 - Scripted runs do not exercise cursor mode (TEST GAP)

- `update_sim`'s scripted branch (`if let Some(run)`) returns before the `cursor_mode` block. In scripts with the panel open, `attack`, look and the
  mouse still reach the pawn. Live play is correct, but no script can check claim 3. Apply the same cursor-mode masking in the scripted branch
  when `ui.inv.panel.open`.

## G5 - Not drawn yet (spec)

- Slider thumb: blocked by G2 (`SliderStyle_MouseKeyboard` `NormalThumbImage` / bar = the `BasicStrokeBox` material).
- `BackgroundBlur_2` of `QuantitySelector_C` (`BlurStrength` 2). It needs a backend blur pass over the region already rendered behind the modal:
  UE 4.16 `SBackgroundBlur::OnPaint` / `ComputeEffectiveKernelSize` use `Strength = BlurStrength * (bApplyAlphaToBlur ? tint alpha : 1)`,
  `KernelSize = RoundToInt(Strength * 3)` (= 6) and sigma `max(0.5, Strength)` (divided by `DownsampleAmount` when the kernel is downsampled). It is a
  separable Gaussian post-process pass over its geometry, with the `Lightbox` content painted on top.
- Native leaf widgets with no widget tree in the data: `FortItemCategoryIndicator`, `FortTierIndicator`, `FortLevelIndicator`, and the content of
  `ItemRatingIndicator_C`. For 1.8 BR world items, `user_friendly_tags` already gives the category text. Tier and level are not shown for BR
  weapons (no `DisplayTier`), so their absence probably matches 1.8. Confirm from the native `RebuildWidget`s before drawing anything.
- `QuantitySelector` buttons (`IconTextButton_C` with no `Style` set and `ControllerInputStyle = ButtonStyle-Primary-M`) draw as text only. Check
  the `UCommonButton` path when `Style` is null (the CommonUI default style from the settings) before you call this correct.

## Confirmed (no action)

Header 40 (`SizeBox_2` has `HeightOverride` 48 but no `bOverride_HeightOverride`. In UE 4.16, `USizeBox::SynchronizeProperties` calls
`ClearHeightOverride()`). The drop slot fills 1100x175, with the 80x80 `T_AthenaDropItem` at left/top +15 (`UOverlaySlot` defaults are Left/Top).
The tile cell is 254x439 with SListPanel EvenlyDistributed padding. Sort: default 8 (BySubtype, constructor 0x142dada70 writes +0x408). Supported
sorts are {1,4,2,0,8}. The subtype map matches FUN_140687180 exactly. The filter is {0x10,0x11,1,0xe,0x13}. The split rules work: a tile Shift-drag
dropped 22 of 45 ammo, plain tile and slot drags dropped the full stack. The quantity math matches the Blueprint (0.25 of 401 gives 101, Max gives
401). ToggleInventory = I (CookedIniVersion.txt line 2740). Tab = ToggleFullScreenMap (line 2743). The HUD hint shows "I". The HUD before and
after the first open (when the paks are attached) is pixel-identical apart from the world behind translucent elements.

# verify3 (2026-10-05): independent check of the fix3 changes (UI-wide)

Build: `fortnite-rs` / `fortnite-lobby` (fast profile) from the working tree. Evidence: `verification/inventory/verify3/` (in-game script shots and logs;
`ui/new` = current tree, `ui/base` = the same tree with only the four fix3 UI changes reverted, `ui/diff` = |new - base| x4; `lobby_runtime/` = the
`fortnite-lobby` window during its `FN_LOBBY_SCRIPT` session).

Result: G1 (header), G2 (texture lookup), G3 (cursor-mode entry), G4 (scripted cursor mode) and the blur maths are correct. Pixel-identical before/after
fix3: every Item Shop and console capture, the settings tabs Video / Game / Brightness / Audio / Input / Controller / HUD, the in-game HUD, quickbar,
build bar and every inventory shot except the header and the quantity selector. Changed: the inventory header (G1, now right), the quantity selector
(blur, slider thumb, Primary-M buttons), settings Account (`Sign In` button), and the lobby / Locker CPU captures (blur regions only). No texture went
missing on any screen (no `unresolved`, `missing: []` everywhere; lobby runtime `unresolved +0`).

## V1 - CPU blur over a transparent UI layer turns the blurred region opaque (MEDIUM, capture tools only)

- `raster::blur_rect` / the GPU pass write alpha 1 (correct for UE: `GaussianBlurMain` blurs the back buffer, which already holds the scene). But
  `lobby_ui_capture` (`ui-*.png`), `lobby_composite` and `lobby_locker_capture` rasterise the UI on a TRANSPARENT layer and composite it over the
  scene afterwards. Every lobby `BackgroundBlur` (top-bar band y 80-92, season panel, Match Type / Play panel) now blurs transparent black, so it
  comes out as an opaque dark box and hides the scene in the composite (`ui/diff/ui-1920x1080-diff.png`, `ui/new/ui-1920x1080.png` bottom right).
- Fix: tools that composite must rasterise the UI over the scene pixels (one framebuffer, the way the game does with `render_texture` on the frame),
  or skip `Cmd::Blur` when rasterising a transparent layer. Do not regenerate `parity/lobby/ui-*.png` with the current tree before that.

## V2 - The lobby runtime never draws its blurs (LOW, inconsistency)

- `fortnite-lobby` draws with `UiRenderer::render` (no target texture), which skips `Cmd::Blur`, so the lobby / Locker `BackgroundBlur`s that the CPU
  captures now draw are not drawn on screen (`lobby_runtime/lobby_t025.png`: the Play panel is unblurred). The surface already has `COPY_SRC`
  (`gfx.rs` is shared with the lobby binary), so the fix is `render_texture(&frame_tex.texture, ...)` in `fortnite-lobby.rs` `draw()`.

## V3 - Default button style: native subclasses of UCommonButton (LOW, latent)

- Exe: `UCommonButton` ctor 0x143928dd0 sets `Style` (+0x290) = `UCommonUISettings` CDO `DefaultButtonStyle` (+0x50), loaded in
  `UCommonUISettings::PostInitProperties` 0x143947810 by `StaticLoadClass` from `DefaultButtonStyle_StringRef` (+0x40; ini line 3979 = Primary-M).
  No consumer substitutes the default when Style is null (`GetStyle` 0x143939840 returns 0; `SetStyle` 0x143955450 ignores null). So the port's rule
  "no Style serialized in the instance or Blueprint chain -> Primary-M; an explicit null stays null" is right.
- But `UCommonRotator` ctor 0x1439298e0 chains to the same CommonButton ctor and does not touch +0x290, so an unstyled rotator also gets Primary-M.
  The port applies the default only when the nearest native class is exactly `/Script/CommonUI.CommonButton`. No visible effect today
  (`AthenaRotator_C` CDO Style = `ButtonStyle-Base`, `TextRotator_C` = `ButtonStyle-RotatorBorder`). Fix: apply it to every native class whose
  constructor does not set Style (CommonRotator at least).
- Screens whose buttons changed: inventory quantity selector (Max / Drop / Cancel: were bare text, now Primary-M; matches the exe default since the
  `IconTextButton_C` CDO has no Style) and settings Account (`Sign In`). No other lobby, Locker, shop or settings button changed. Not checkable
  without a 1.8 capture: Drop / Cancel are 32 px tall while Max is 50 px, and the labels sit top-left inside the min-size button (the same generic
  CommonButton content placement as the lobby `Play` button, which matched its reference).

## V4 - Blur pass allocates per frame (LOW, performance)

- `UiRenderer::encode_blur` creates three full-region textures, two to four uniform buffers and bind groups for every `Cmd::Blur` every frame
  (1920x1080 for the quantity selector). Two runs of 340 frames with the selector open vs closed gave inconsistent medians (13.3 vs 14.9 ms,
  22.5 vs 12.8 ms; `perf_*.txt` / reports), so there is no reliable measurement. Cache the textures by size. Adding `COPY_SRC` to the swapchain usage
  is only done when `caps.usages` allows it, `resize` keeps it (it reuses `config`), and the lobby path never copies from the surface.

## V5 - Cursor-mode entry: what the two omitted controller calls are (spec, no port impact today)

- `FUN_140824b40` calls controller vfunc +0x14a0 (slot 660) and +0x1380 (slot 624) first (`AFortPlayerControllerAthena::vftable` 0x144070898).
- Slot 660 = 0x140437540 -> `AFortPlayerController` 0x140830340: if the `AFortUIZone` (+0x418) has the full-screen map open (+0x6c8), `HideFullScreenMap`
  (0x140900ea0, FortUIZone.cpp); then the Athena part `FUN_1423a12b0(this, +0x2fd0)` removes one entry from an array at +0x630. Opening the inventory
  closes the full-screen map. The port has no full-screen map yet: port this together with it.
- Slot 624 = 0x140426c10: unless byte +0x31B8, jumps to 0x140826560: if +0x490 is set, `FUN_1424f9050`; if the input settings flag +0x68 (gamepad in
  use) is set, it calls controller vfunc +0xb78 with four FKeys (`DAT_14678f758` = `Gamepad_LeftX`, then the three next stick axes) and zero
  amounts, which flushes the analog stick axes once. The axes are polled again next frame, so this has no lasting effect.
- The rest of the fix3 claim checks out: +0x980 = slot 304 = `AFortPawn::PawnStopFire` 0x1407f8b80 (`bPrimaryInputHeld` +0xC58 = 0, weapon +0x688);
  +0x938 = slot 295 = 0x1407f8be0 (+0xC59 = 0, weapon slot 212 = 0x140937c30 -> weapon slot 260 = 0x14093af20 `SetTargeting(false)` when +0x965).
  In game (`fix3_cursor`): sprint 550 -> 410 on open, the held key stays ignored, re-press sprints again (550 at frame 130).

## V6 - Leaf widgets of the item details header (spec; MEDIUM, visible: the tier stars and category icon are missing)

Header tree (`ItemDetails/HeaderWidget`): `HorizontalBox_1` = [`CategoryIndicator`, `IconSpacer`, `ItemDetailsHeaderTagListText`]; `TertiaryRow` =
[`CategoryIndicatorTertiary`, `IconSpacerTertiary`]; `LevelBar` (CommonBorder, shown for item types 28/15/16/17/18/14/27/41/29/8) > `SizeBox_0` >
`Overlay_1` = [`ItemRatingIndicator` (Collapsed), `EvolutionAndTierBox` = [`EvolveIndicator`, `TierIndicator`], `UpgradeAndLevelBox` =
[`UpgradeIndicator`, `LevelIndicator` (Visibility Hidden in the data)]].

- Item level (PROVEN storage, INFERRED value): `FFortItemEntry.Level` (+0x1C; item +0x1FC). `UFortWorldItem` slot 70 (+0x230) = thunk 0x1406b18b0
  returning the raw entry level (no clamp). The entry ctor 0x140678730 sets Level 0. The client has no BR item-creation code (`PickLootDrops`
  0x140b25aa0 and `K2_SpawnPickupInWorld` 0x140b16580 call `return 0` stubs; loot rows only carry Min/MaxWorldLevel filters). Every Athena WID / TID
  cooks `MinLevel -1`. So BR world items carry level 0 and `LevelIndicator` stays Hidden: draw nothing (matches the port). Consumables cook
  `MaxLevel 1`, so a server-sent level 1 would show "LV 1"; ignore that offline.
- `FortTierIndicator` (instance: `BrushSize Small`; native `InterPipPadding` not written by the ctor 0x143004fd0, so 0, INFERRED):
  - Pip count = `UFortItem::GetTier` 0x14068d3b0 -> definition slot +0x278 (0x14068d3e0) = `Tier` byte (+0x145). Cooked BR values: Common 1,
    Uncommon 2, Rare 3, Epic 4, Legendary 5 (weapons); ammo / materials / consumables / traps / pickaxes 0. World items use `Tier` only (no MaxTier).
  - Pip i < Tier: `FilledTierPipMultiSizeBrushAsset` (`/Game/UI/Foundation/Items/Parts/`), Small entry = `T-Stars-Full-S`
    (`/Game/UI/Foundation/Textures/Icons/Stars/`, 24 px), colour (1,1,1,1). Unfilled / preview pips do not occur for world items (no max / preview).
  - `SHorizontalBox` of `SImage`s, each slot centred H and V, padding left 0.5 x InterPipPadding (not on the first) and right 0.5 x InterPipPadding
    (not on the last). Tier 0 draws nothing.
  - Placement: `EvolutionAndTierBox` (the probe puts it at x 1516, y 182.8 of the 668x40 overlay at 1920x1080, i.e. centred). Expected: the
    Rare sniper (`l2_tile0_selected`) shows 3 white stars centred in the dark `LevelBar`. The port shows the bar empty.
- `FortItemCategoryIndicator` = `UFortIconWithLabel` (ctor 0x142f668a0: `ShouldShowIcon` 1, `ShouldCollapseIconWhenNotShown` 1, `ShouldShowLabel` 0;
  layout 0x142f7e5b0: HBox [`SFortMultiSizeImage` at `IconBrushSize`] [`SSpacer` 8x0] [`STextBlock` `LabelTextStyle`]). Data comes from item slot
  +0x2d0 `TryGetCategory` 0x140698750 (refresh 0x142fee080). Icon Visible if shown and a brush exists, else Collapsed; label Visible if shown and not empty.
  - `CategoryIndicator` (header, ordinal Secondary, icon Medium, no label): the definition's GameplayTags (+0xD0) / `SecondaryCategoryOverrideTags`
    (+0xF8) matched against `GameData.ItemCategories.SecondaryCategories[].TagContainer` -> `CategoryBrush` (multi-size, Medium entry). So BR
    weapons show the weapon-class icon left of "Ranged | Sniper Rifle" (with `IconSpacer` already made visible by the port for those types).
  - `CategoryIndicatorTertiary` (ordinal Tertiary, label on, `TextStyle-Base-XS-B-70pc`; row shown only for type 17 with `Show Ammo Header`): ranged
    weapons -> 0x14092b210: the ammo definition's display name + `AmmoIconBrush` (ammo def +0x4D8), icon Medium.
  - `AmmoTypeIndicator` (`ItemCalledOutAttributesDetailWidget`, Tertiary, icon only): the ammo icon.
  - Ordinal Primary (not used in this panel): label from `PrimaryCategories` by item type, never an icon.
- `ItemRatingIndicator_C`: rating = definition `GetRatingAtLevel` 0x14068bfa0 = 0 (no Athena `RatingLookup`); the panel keeps it Collapsed. Nothing to draw.
- `FortLevelIndicator` for completeness (UMG, update 0x143065040, rebuild 0x1430501e0): HBox ["LV" (`FortniteUI.LevelIndicator.LevelLabel`,
  right pad 8, Collapsed unless `ShouldShowLabel`)] [level, numeric] [arrow, left pad 4] ["/" (pad 4, INFERRED)] [max level] [arrow]; all in
  `TextStyle` (instance: `TextStyle-Base-S-B`); whole widget Hidden when level == 0; "/" + max Collapsed unless `ShouldShowMaximumLevel` and max >= 1.

## V7 - Not caused by fix3 (noted while comparing)

- Build bar slots F1-F4 draw the slot frames but no wall / floor / stairs / roof icons, identically before fix3 (`verification/building/hud/verify/
  v03_build_wall_wood`) and after (`verify3/u2_build_bar`).
