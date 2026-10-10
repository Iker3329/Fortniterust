# smallfix follow-ups (verifier, 2026-10-05)

Specs for the open items of the `verification/smallfix` fixes. Every claim cites the 1.8 exe (decompile `C:\FortniteRef\decomp\named`, vtable reads
from `FortniteClient-Win64-Shipping.exe` .rdata) or UE4.16 source. Battle Royale only.

## 1. Equip montage when the blueprint tool changes (build piece switch, build <-> edit)

### Exe evidence
- Equip path: `AFortPawn::EquipWeaponDefinition` 0x1407d9590 -> `FUN_1407d94d0` -> `FUN_1407ea2a0` (set CurrentWeapon +0x830): the old weapon gets
  vslot 0x710 (`OnUnEquip`), the new one vslot 0x718 (`OnEquip(PrevWeapon)`) with `PrevWeapon` = the old CurrentWeapon.
- `AFortWeap_BuildingTool::OnEquip` 0x14092f5c0 -> `AFortWeap_BuildingToolBase::OnEquip` 0x14092f6d0 -> `AFortWeapon::OnEquip` 0x14092f8f0 ->
  tail `FUN_140931e30(this, Prev)` -> `AFortWeapon::PlayEquipAnimation(this, Prev)` 0x1409345d0.
- `PlayEquipAnimation` plays `EquipAnimation` (+0xA60) through pawn vslot 0x768 (PlayAnimMontage) only if vslot 0x730 `ShouldPlayEquipAnimation(Prev)` is true
  (and the pawn is not holstered / not first person); otherwise it calls `OnEquipFinished` (vslot 0x8d0, 0x140930830) at once.
- vslot 0x730:
  - `AFortWeapon` / `AFortWeaponRanged` (vtables 0x144297f90 / 0x14429f708) -> 0x14093b950: `return EquipAnimation != null`.
  - `AFortWeap_BuildingToolBase`, `AFortWeap_BuildingTool`, `AFortWeap_EditingTool` (vtables 0x14429d978 / 0x14429e348 / 0x14429ed28) -> 0x14093b910
    (bytes decoded by hand, the decompile has no body for it):
    ```
    bool ShouldPlayEquipAnimation(AFortWeapon* Prev) {
        bool prevIsBlueprint = Prev && Prev->WeaponData /*+0x3D0*/ && Prev->WeaponCoreAnimation /*+0xA80*/ == PaperBlueprint /*3*/;
        return EquipAnimation /*+0xA60*/ != nullptr && !prevIsBlueprint;
    }
    ```
- Both tools have `WeaponCoreAnimation` = 3 (`PaperBlueprint`, native ctors 0x140914770 / 0x140914890, see `fn_pose::loadout_build`).

### Required behaviour
| Transition | Prev core | Equip montage |
|---|---|---|
| gun / pickaxe / consumable / nothing -> building tool (Q) | not 3 | `EquipBuild` plays (current port: correct) |
| building piece A -> building piece B (F1..F4 in build mode) | 3 | **none** (current port: correct, `build_slot` just changes) |
| building tool -> edit tool (edit started in build mode) | 3 | **none** (port plays `EquipEdit`: MISMATCH) |
| gun / pickaxe -> edit tool (edit started outside build mode) | not 3 | `EquipEdit` plays (port: correct) |
| edit tool -> building tool (edit ends, back to build) | 3 | **none** (port plays `EquipBuild`: MISMATCH) |

Since no montage is played, nothing stops the montages already running (no `Montage_Play` group stop): they run to their own auto blend-out.
`OnEquipFinished` runs immediately (no equip delay).

### Port change (`crates/fn-app/src/build_anim.rs`)
- `handle`: `(Event::EditBegin, h)` plays `Which::EquipEdit` only when `h == Held::None`; from `Held::Build` it only switches the held tool
  (`equip_tool(Held::Edit, ..)` without `play`).
- `(Event::EditConfirm | Event::EditCancel, Held::Edit)` with `edit_return == Held::Build`: `equip_tool(Held::Build, ..)` without `play(Which::EquipBuild)`.
  (The confirmed-edit montage `Which::Edit` stays as it is.)
- Piece switch: keep as is (no event, no montage). Optional: log it as `OnEquip(prev = PaperBlueprint) -> no EquipAnimation`.
- `equip_tool` already skips `arms.holster` when a blueprint was held; keep that.

## 2. Build quickbar selection frame (the "F2 one frame late" in smallfix a2)

Not a frame of latency: the port fades the selection frame. `crates/fn-ui/src/screens/build_quickbar.rs:243` draws `Icon-BasicFrame-S` with alpha
`sel[i]`, which `tick()` (:183-193) blends over `SELECTED_TIME` 0.1 s, so 1-2 frames after F1 the old slot is still mostly framed.

1.8 (same frame, delegate): HotbarKeyF1 `FUN_14083a690` -> `FUN_14067ba90` -> `FUN_140698930` -> `FUN_140698b20` writes the focused slot and broadcasts
`AFortQuickBars+0x590` (`FUN_14067c4d0`) -> `FUN_142e43cb0` -> `UFortInventoryContext.OnQuickbarSlotFocusChanged` -> QuickbarBase handler ->
`QuickbarSlot.SetSelected`, which sets the `Selection` image Visible / Collapsed immediately; only the `Selected` widget animation (lift) plays,
forward on select, reverse from its end time on deselect.

Change: draw the selection frame at alpha 1 only for the focused slot (`focused_secondary && i == building_slot`), never from `sel[i]`; keep `sel[i]`
for the lift (:232). Same for the primary bar, `crates/fn-ui/src/screens/quickbar.rs:167` (frame only for `i == selected`). Exact match also needs
the deselect to start from the end of the lift (reverse play from `GetEndTime`), not a symmetric blend. UNPROVEN: that `Selected` has only a
translation track.

## 3. Fallback `shade_character` specular (`crates/fn-render/shaders/scene.wgsl:139-172`)

Who uses it in normal play (paks present): the held pickaxe (`loader.rs:273`, kind Prop), held guns unless `--original-gun-material`
(`scene.rs:135-147`), dropped weapons / ammo / consumables (`pickup_mesh.rs:154`). The default hero (HID_005) never does: every drawn section runs the
original CharacterShader DXBC (`char_scene`), see `verification/smallfix/stderr.txt` `[br-character]` lines. Character sections fall back only without
the BR load or for a cosmetic whose material chain has no CharacterShader permutation owner (`fn-pose/src/br_material.rs:159`).

Real CharacterShader G-buffer (translated shaders `crates/fn-render/shaders/char_material_gen.wgsl`, head :329-337, hair :805-813, body :1285-1294):
```
Metallic  = sat(S.g)
Specular  = sat(lerp(ClothSpecMultiplier, SkinSpecMultiply, M.b) * S.r)      // both 0.5 cooked -> 0.5 * S.r
Roughness = sat(lerp(SpecRoughnessMin, SpecRoughnessMax, S.b))               // head / hair
          = sat(1 - lerp(SpecRoughnessMin, SpecRoughnessMax, S.b))           // body (CharacterShader_Gloss)
AO        = sat(AOLevel * (M.r - 1) + 1)                                     // AOLevel 0 -> 1
```
UE DefaultLit: F0 = lerp(0.08 * Specular, BaseColor, Metallic), DiffuseColor = Base * (1 - Metallic). For typical texels (S.r ~ 1, S.g ~ 0) the real
lobe is ~1.0 x `ggx_spec(.., 0.04)`; the fallback's `(0.25 + 4 S.g) * 0.5` gives 0.125 (8x too dim), uses S.r as AO and does not invert the body
roughness. Its channel comment (:139-140) is wrong.

Changes:
- Character branch: implement the formula above (spec colour form as in `shade_architect`, scene.wgsl:194-197), with per-material Min/Max and the
  Gloss inversion as uniforms, AO = 1. Better: keep every CharacterShader-owned section on `char_scene` (already the case for HID_005).
- Prop branch (`is_prop`, what actually runs): drop `(0.25 + 4 S.g) * spec_mult` and the S/M constants; use a plain dielectric (Specular 0.5 -> F0 0.04,
  multiplier 1.0) labelled ASSUMED until the real `WeaponMat_Base` path is the default. Real materials: pickaxe `MI_FORT_Melee_Pick` -> `WeaponMat_Base`
  (SpecRoughnessMin/Max 0.2258 / 0.8221), guns / dropped weapons `WeaponMat_Base` instances. Hook to reuse: `pickup_orig::draw`
  (`crates/fn-app/src/pickup_orig.rs:50`) with `/Game/Weapons/FORT_Melee/Meshes/SK_Pick` (or the swapped pickaxe's mesh) at `world * socket`; that path is
  still opt-in (`app.rs:533-545`). UNPROVEN: `WeaponMat_Base`'s own channel mapping.

## 4. Melee DecisionWindow: secondary input (reference, no change needed now)

`AFortPawn::vfunc_334` 0x1407faeb0 (window NotifyEnd), with `bP` = bReceivedPrimaryInput && PrimaryInput.InputType == WindowClickOrHold(0),
`bS` = bReceivedSecondaryInput && SecondaryInput.InputType == 0:
1. `bAlreadyProcessedInput` -> nothing.
2. If CurrentWeapon has a SecondaryAbilitySpecHandle (+0x9FC != -1) && (secondary held +0xC59 || bS) && !bP -> SecondaryInput.NextSection / its
   TriggerAbilityTag (pawn vslot 0xA58).
3. Else if primary held (+0xC58) || bP -> PrimaryInput.NextSection / TriggerAbilityTag.
4. Else -> the window's default section (+0x60) if not None.
Presses: `PawnStartFire` 0x1407f8ac0 (+0xC58 = 1, `vfunc_343(primary=1, record=1)`), secondary start `vfunc_294` 0x1407f8b20 (+0xC59 = 1,
`vfunc_343(0, 1)`), the stops clear the held byte and call `vfunc_343(.., record=0)` (swallow only). `vfunc_343` records each input once; InputType
`InstantClick` (2) acts at the press and sets bAlreadyProcessedInput. Priority to note: a primary click inside the window beats secondary, but a
secondary hold/click beats a primary that is only held. The port may keep omitting the secondary branch while the BR pickaxe grants no
SecondaryFireAbility (WID `SecondaryFireAbility`, +0x4F0; not re-read from the paks by the verifier: UNPROVEN).
