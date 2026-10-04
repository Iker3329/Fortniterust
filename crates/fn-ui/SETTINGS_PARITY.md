# Settings screen parity with Fortnite 1.8 (UE 4.16)

Sources: widget blueprints under `/Game/UI/Frontend/Settings/` decoded with `fn-inspect props/disasm`, plus read-only static analysis of the
1.8 EXE (`explore`). Nothing was run, no key was recovered, no encrypted `Config/*.ini` was decrypted.
The "old captures" in `FortniteRuntime/verification` are renders of the earlier prototype, not the real game: they are a regression
reference only, the widget data is the ground truth.

## Per tab

| Tab | 1.8 source asset | Result |
|---|---|---|
| Video | TabVideoOptions (+ OptionsMenuRowSelector, RotatorSelector) | 15 rows, order, labels, slot padding (2 / Quality bottom 20) and widths match. No difference found. |
| Game / HUD | TabGameOptions, TabGameOptionsMain, TabGameOptionsHud, OptionsMenuHudRotator | Row list, order, labels, hover texts match. Differences fixed: `Toggle Targeting` had no hover text (1.8: "Set ranged weapon targeting to be a press action instead of a hold action."); the Game tab TooltipDisplay is `TextStyle-Base-XS-Black` (colour 0,0,0), the port drew it in XS-Blue. `Matchmaking Region` exists in 1.8 (`Region` rotator, label key C3A228A3...); its options come from locres namespace `MMRegion` (Europe, North America, Oceania), so the hand-edited row is legitimate. Sub tabs use `SetStyle(ButtonStyle-Tab-Secondary)` as the port does. |
| Brightness | TabBrightnessOptions | Matches (diff 0.00 masked). |
| Audio | TabAudioOptions | Matches (8 rows, padding 5, XS-Blue tooltip). |
| Input | TabInputOptions, OptionsMenuInput, EXE `ConstructKeyBindList` @0x142ee4070 | Row geometry already matched. Row ORDER was invented; it is now the 46-entry name/text table of `ConstructKeyBindList` (Move Forward, Move Backward, Move Right, Move Left, Cursor Mode, Jump, Fire, Target, Reload, Use, Trap Equip/Picker, Building Edit, Repair/Upgrade, Rotate Building, Sprint, Change Building Material, Reset Building Edit, Weapon Slot 1-4, Ability Slot 1-3, Gadget Slot 1-2, Building Slot 1-4, Trap Slot, Switch Quickbar, Slot Up, Slot Down, Chat, Quick Chat Message, Place a Note, Spot Target, Selected Building Type, Toggle Map, Toggle Inventory, Skip Cutscene, Resurrect, Respawn, Crouch, Push To Talk). All 46 labels equal the port's. Also fixed: hover text now has the exact 1.8 string (`... game actions. \n\nBindings for {Input Row}.`, trailing space); removed the custom red duplicate-key colouring (1.8 draws none). |
| Controller | TabGamePadConfig | Matches (Configuration rotator, Controller Platform "XBox One" / "PlayStation®4", mapping diagram). |
| Live Streaming | TabAccountLinkage (WebsiteConfigurationBox) | Was a stack of invented paragraphs. Now: header "LIVE STREAMING FEATURES" (Header-L-NavyBlue, Burbank 48), EpicLogo 214x244 (new `TexId::EpicLogo`), AccountLinkText (Base-S-Blue 18, padding L/R 200). |

## Unrecoverable (and why)
* Default key of every binding: `DefaultInput.ini` is AES-encrypted. Defaults in `fn-settings/table.rs` stay ASSUMED (the port needs working keys).
* Whether the on-screen order equals the `ConstructKeyBindList` table order: the list is filtered/ordered through `FortClientSettingsRecord::GetActionBindings`
  (record populated from the encrypted input config). The table order is the only unencrypted evidence and is used; the final order is not provable offline.
* Quality-level tables (Scalability groups): `DefaultScalability.ini`/`BaseScalability.ini` are encrypted; the approximations in `fn-settings` remain, marked APPROXIMATION.
* HUD tag list (`ConstructHUDTagList`) is built from the controller's HUD visibility record at runtime; the existing list is kept.
* Live Streaming: the URL row (`GetAccountLinkingUIURL`) and the FortTwitchLogin configuration are server/account driven, not drawn. Region default and PS4 "Wireless Controller" text variants are platform-gated (PC shown).
* `Quality: Auto` message ("Automatically finding you the settings...") exists in OptionsMenu but needs the benchmark flow; not drawn.
* The `picker.rs` custom line "Save the World is not available in this build." is not present in `crates/fn-ui/src/screens/picker.rs` (no match for it anywhere in `crates/`); nothing to remove.

## Screenshots and numbers (1920x1080, `fn-ui-demo --diff ... --cpu --assets layered`)
In `parity/settings-ui/` (`*-ours.png`, `*-diff.png`, `numbers-1920x1080.txt`). Columns: mean abs diff over whole image / masked / % pixels off by >8.
video 0.11 / 0.1%, game 0.17 / 0.3%, hud 0.03 / 0.1%, brightness 0.00 / 0.0%, audio 0.00 / 0.0%, controller 0.02 / 0.0%.
input 4.18 / 3.1% and input-listen 4.10 / 3.0% (intended: the old capture has the invented order and "Unbound"; 1.8 draws "Not Bound" = KeyboardKeyNames.KeyNotBound),
livestream 7.38 / 9.1% (intended: old capture is the invented text stack).

## Tests
`cargo test --release -p fn-ui -p fn-settings`: fn-settings 24 passed; fn-ui 461 passed, 0 failed. `cargo build --release -p fn-app`: OK.
* `open_menu_settings_change_a_row_rebind_...`: the expectation was wrong versus 1.8. TabInputOptions only calls native `ChangeBinding` (no Apply handler), so a rebind reaches the game at once. Test now asserts that, and rebinds Fire/Jump (Crouch is now row 45, out of view). Side effect kept and documented: that immediate apply also carries any pending Video edit.
* `scenes_resemble_the_old_captures`: input* and livestream scenes are reported only (old captures differ from 1.8 as above); other settings18 scenes still asserted.
* Removed `duplicate_keys_..._shown_in_red` (custom UI removed). `pak_assets` scene test: livestream/startup scenes now compared through the layered source (EpicLogo is pak-only; startup-* scenes were previously unhandled).

## Files changed
`crates/fn-ui/src/menu/pages/{input.rs,layout_table.rs,rows.rs,slate.rs,input/tests.rs}`, `crates/fn-ui/src/menu/{mod.rs,tests.rs,live_tests.rs,scene_defs.rs}`, `crates/fn-ui/src/pak_assets.rs`.
