# Edit mode: every cancel and confirm path in 1.8 (Battle Royale)

Source: the 1.8 exe (`C:\FortniteRef\decomp\named`; the vtable slots were read from `FortniteClient-Win64-Shipping.exe`), the 1.8 SDK
(`C:\FortniteRef\Fortnite_1.8.0_SDK`) and the cooked input config (`C:\FortniteRef\scratch\CookedIniVersion.txt`).
In that file the `ConfigD_Athena` preset is the only copy; every line carries a `Mac.Input:` prefix.
`DefaultPresetNameAthena = ConfigD_Athena` (L2206). Verified 2026-10-05.

## 0. Core primitives

| Symbol | What it does |
|---|---|
| `AFortPlayerController::LocalExitEditMode(PC, bCancel)` @1408341e0 | Local controllers only. While in edit mode (`FUN_140834ae0`: `PC+0x1a48` EditActor != null) it calls `FUN_140837140(PC, bCancel)`. Otherwise, if a server edit request is pending (`PC+0x22e8`, set by `ServerBeginEditingBuildingActor` in `FUN_140833550`), it calls `ServerEndEditingBuildingActor`. |
| `FUN_140837140(PC, bCancel)` | Calls `FUN_1408172e0`, then `PopInputComponent(EditModeInputComponent0)` (`PC+0x1a40`). Then EditActor vfunc 0xa90 = `ABuildingSMActor::vfunc_338(EditActor, PC, bCancel)`. Then, when `bCancel == 0` and the pawn state allows it, `FUN_1407f69f0(Pawn, EditActor)`. When the EditActor is the build preview (`PC+0x1788`), it calls `SetEditingPlayer(null)` (vfunc 0xa70). Last: EditActor = null, broadcast `PC+0x14e0`, `ServerHandleMissionEvent_ToggledEditMode(.., 0)`. |
| `ABuildingSMActor::vfunc_338(Actor, PC, bCancel)` @140482180 | Shows the mesh again: vfunc_292(true) = `StaticMeshComponent->SetVisibility(true)`; the blueprint mesh only when Health < MaxHealth. Then `EditModeSupport->vfunc 0x240`. When **`bCancel != 0` or there is no pending pattern** (vfunc 0xa80 false) it sends `ServerEndEditingBuildingActor` (CANCEL: nothing changes). Otherwise it sends `ServerEditBuildingActor(Actor, NewClass, RotationIterations, bMirrored)` (CONFIRM: the piece is replaced). `ABuildingWall::vfunc_338` @1404bc540 also re-shows `DoorComponent`. |
| `AFortWeap_EditingTool::bEditConfirmed` (+0xAB0, SDK) | Cleared in `OnEquip` @14092f7f0. Set by `AFortWeap_EditingTool::vfunc_292` @14091c2b0, the confirm path in §2, which also plays the tool's force feedback (+0x8d8). |
| `AFortWeap_EditingTool::OnUnEquip` @140932760 | **Any unequip of the edit tool** calls `LocalExitEditMode(PC, bCancel = !bEditConfirmed)`. Before a confirm, unequipping is therefore a CANCEL. |
| `AFortPlayerController::ClientForceCancelBuildingTool` -> vfunc_508 @140817e90 | If the pawn's current weapon is an `AFortDecoTool` or an `AFortWeap_BuildingToolBase` (the edit tool derives from it) and pawn vfunc 0xc70(0) is false, it calls pawn vfunc 0x998, which switches away from the tool. The resulting unequip cancels as above. |

Edit-mode input: `EditModeInputComponent0` (`PC+0x1a40`) is created in `AFortPlayerController::SetupInputComponent` @14084a570. It is pushed on edit start (vfunc_684 @140836d70, `FUN_1423a2cc0`) and popped on exit. It binds only these actions:

| Action (event) | Handler | Effect |
|---|---|---|
| `Fire` (pressed) | `FUN_140834d80` | EditActor vfunc 0xa98 (339) -> `EditModeSupport` vfunc 0x248: starts selecting tiles. Gated by the controller's action-tag check vfunc_620 (tag `FortGlobalTags+0x2c0`). |
| `Fire` (released) | `LAB_140834d40` | EditActor vfunc 0xaa0 (340) -> `EditModeSupport::vfunc_74` @1404bbcd0: ends the drag. **It does not confirm.** |
| `BuildingEditReset` (pressed) | `LAB_140834d60` | EditActor vfunc 0xaa8 (341) -> `EditModeSupport` vfunc_75 (`BinaryToggle` @1404bbd40, `Roof` @1404bbdb0): resets every tile to the unedited pattern. **The player stays in edit mode. It is not a cancel.** |
| `CompleteBuildingEditInteraction` (pressed) | `FUN_140837210` | CONFIRM, see §2. |
| `GamepadEditBuilding` (released) | `FUN_140837210` | CONFIRM (gamepad). This action has no `ConfigD_Athena` mapping. |

Every other binding, including the hotbar keys, `SwitchQuickbar`, `SlotUp` / `SlotDown` and `EscapeKey`, stays live underneath on the base InputComponent. Edit mode does **not** block weapon switching or the build-mode toggle; those actions **cancel** the edit (§3).

## 1. Keybinds (ConfigD_Athena ActionMappings)

| Action | Keys | Line |
|---|---|---|
| `PerformBuildingEditInteraction` (start editing) | G | L2661 |
| `CompleteBuildingEditInteraction` (confirm) | G, Gamepad_Facebutton_Right | L2662, L2660 |
| `BuildingEditReset` | RightMouseButton, Gamepad_RightThumbstick | L2671, L2672 |
| `Fire` (select tiles) | LeftMouseButton, Gamepad_RightTrigger | L2652, L2653 |
| `SwitchQuickbar` (build-mode toggle) | Q, ThumbMouseButton | L2663, L2664 |
| `HotbarKey1..6` / `HotbarKeyF1..` | One.. / F1.. | L2641.., L2677.. |
| `SlotUp` / `SlotDown` | MouseScrollDown / MouseScrollUp | L2665, L2666 |
| `EscapeKey` | Escape, F10, Gamepad_Special_Right | L2737, L2738, L2739 |
| `GamepadSwitchModeOrEditBuilding` | Gamepad_Facebutton_Right | L2659 |

Key aliases: `PerformBuildingEditInteraction` KeyAlias = Gamepad_Facebutton_Right, Hold (L2619). `BuildingEditReset` = RightMouseButton, Click (L2625).
Input groups: `Fire`, `Jump` and `BuildingEditReset` are in the `Edit` group (L2235-2237). This only affects how conflicts are shown in the keybind UI.
`bEditModeUsableFromCombatMode=true` (L2596): editing can start from either quickbar.

Because G is bound to both start and confirm, **pressing the edit key again while editing CONFIRMS; it does not cancel.** The edit-mode component sits on top of the base component and consumes G.

## 2. Confirm paths

1. **Edit key again** (`CompleteBuildingEditInteraction`: G or Gamepad_Facebutton_Right, on press), `FUN_140837210`:
   - If the action-tag gate (vfunc_620, tag +0x2c0) allows it and the player is in edit mode: `LocalExitEditMode(PC, 0)`. This calls `vfunc_338(.., bCancel = 0)`, which sends `ServerEditBuildingActor` when the pattern changed and `ServerEndEditingBuildingActor` when it did not.
   - Then, if the edited actor is not the build preview: when the current weapon is the edit tool, `EditingTool::vfunc_292` (sets `bEditConfirmed = 1`, plays force feedback). After that, `FUN_14067ba90(Quickbars, FocusedBar, FocusedSlot)` re-activates the focused quickbar slot, so the previous weapon or building piece comes back.
   - The unequip that follows finds the controller no longer in edit mode, so it does nothing.
2. **Gamepad**: `GamepadEditBuilding` released, same handler.
3. **No confirm-on-release setting exists in 1.8.**
   - `UFortClientSettingsRecord` (SDK) has only `bUseGamepadEditModeAimAssist` (+0x161) for edit mode.
   - `FortPlayerInputSettings` has `bEditModeUsableFromCombatMode`.
   - The exe has no "release confirms" string or flag. Releasing `Fire` only ends the tile drag (`EditModeSupport::vfunc_74`).
   - "Edit release confirms" came later; the 1.8 settings catalog (`crates/fn-ui/testdata/settings-catalog.json`) has no such row.

## 3. Cancel paths

| # | Trigger | Path | Result |
|---|---|---|---|
| C1 | **Unequip or weapon switch**: HotbarKeyN (1..6 = One..Six), `SlotUp`/`SlotDown` (wheel), `HotbarKeyF1..` | `FUN_14067ba90` -> `FUN_140698930` -> `FUN_140698b20` (focus change), then `ServerActivateSlotInternal` equips the slot item. The edit tool unequips: `EditingTool::OnUnEquip` -> `LocalExitEditMode(PC, !bEditConfirmed)` = `(PC, 1)` | CANCEL. A key for an empty slot returns -1 and changes nothing (edit continues). The key of the already-focused slot still re-activates it, so it also cancels. |
| C2 | **Build-mode toggle** (`SwitchQuickbar`: Q, ThumbMouseButton) | `FUN_14083d040`: vfunc 0x13e0, pawn vfunc 0x938(0), then `FUN_140698120(Quickbars, otherBar)` -> `ServerActivateSlotInternal` equips the other bar's item -> OnUnEquip of the edit tool | CANCEL, and the other bar becomes focused (leaves or enters build mode). |
| C3 | **Escape** (`EscapeKey`: Escape, F10, Gamepad_Special_Right) | `FUN_140836fa0`: when the **building bar is focused** it runs the `SwitchQuickbar` handler (`FUN_14083d040`) = C2. When the combat bar is focused and the player is in edit mode: `LocalExitEditMode(PC, 1)`, then `FUN_14067ba90` re-activates the focused slot. | CANCEL. The menu does not open on this press. |
| C4 | **Edit key again** (G) | See §2 | CONFIRM, not cancel. |
| C5 | **Reset key** (`BuildingEditReset`: RMB, Gamepad_RightThumbstick) | `EditModeSupport` vfunc_75 | NOT a cancel: the pattern resets and editing continues. |
| C6 | **Range**, every PlayerTick (`AFortPlayerController::PlayerTick` @14083ee10 -> `FUN_14084f8a0` while in edit mode) | `FUN_140832610(PC, EditActor)`: \|EditActor centroid (vfunc 0x7f0) - pawn root location (`PC+0x390` RootComponent +0x170)\|² <= `UFortGameData::EditModeCancelDistance`² (+0x4c0, ctor default 750.0, not cooked; `fn-building/src/edit_data.rs`). The same test also gates edit start (`FUN_140833550`). When it fails: a placed piece gets `FUN_14067ba90(focused bar, focused slot)` (re-equip, so C1's unequip cancels it); the build preview gets `LocalExitEditMode(PC, 1)`. | CANCEL |
| C7 | **Piece destroyed while editing** | Server: the building's death path `ABuildingSMActor::vfunc_240` @14047aa10 sees `EditingPlayer` (+0xCA0) set, calls `SetEditingPlayer(null)` and `ClientForceCancelBuildingTool(EditingPC)` -> vfunc_508 -> pawn switches off the tool -> OnUnEquip cancels. Client: the EditActor going null also fails `FUN_140832610` (null actor = false) in C6. | CANCEL |
| C8 | **Piece damaged while editing** | No path: no damage handler clears `EditingPlayer` or calls an exit. The only callers of `SetEditingPlayer(null)` on a building are vfunc_240 (death), `ServerEditBuildingActor_Implementation` @140846a20 (confirm) and `FUN_140837140` (exit). | Editing continues. On exit vfunc_338 shows the blueprint mesh again because Health < MaxHealth. |
| C9 | **Death** | `AFortPlayerControllerZone::ClientOnPawnDied_Implementation` @14086a660 calls `LocalExitEditMode(PC, 1)`. PlayerTick C6 also checks `AFortPawn::IsDead`. | CANCEL |
| C10 | **Building tool unequipped while editing the build preview** | `AFortWeap_BuildingTool::OnUnEquip` @140932630: if the preview is the EditActor (`FUN_140834bf0`), `LocalExitEditMode(PC, 1)`. `AFortWeap_BuildingTool::vfunc_292` @140938d80 does the same. | CANCEL |

## 4. Consequences for the port

* The port's block on weapon switching and leaving build mode while editing is **not 1.8 behaviour**. In 1.8:
  * Hotbar keys and the wheel (C1), Q / ThumbMouseButton (C2) and Escape (C3) all cancel the edit.
  * The focused bar's slot then becomes the active item.
* The edit key (G) confirms (§2). It does not toggle the edit off with a cancel.
* RMB resets the pattern and stays in edit mode (C5).
* Damage never cancels (C8). Destruction (C7), death (C9) and range (C6, 750 cm to the centroid, checked every PlayerTick) cancel.
* There is no confirm-on-release option in 1.8.
* Unresolved names (behaviour from the call sites only): pawn vfunc 0xc70 / 0x998 in vfunc_508, `FUN_1408172e0`, controller vfunc 0x13e0, `FUN_1407f69f0`.
