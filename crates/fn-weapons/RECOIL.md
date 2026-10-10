# Firearm camera recoil (1.8): recovered from the exe + data

Scope: only the view kick when firing. Everything below comes from the 1.8 `FortniteClient-Win64-Shipping.exe` (static disassembly, never executed; RVAs are
image offsets, base 0x140000000) and the 1.8 `AthenaRangedWeapons` rows. Evidence: `parity/recoil-evidence/recoil-natives-disasm.json` (whole-function
instruction dumps with RVAs), `recoil-fields-and-rows.json` (offsets, row values, cvar), `bp/` (Blueprint dumps). Port: `src/recovered_recoil.rs`.

## Data: recoil fields of the three starter rows (`FFortRangedWeaponStats`, offsets from the reflection registration, RVA 0x11dff80)

| field (offset) | Assault_Auto_Athena_C_Ore_T03 | Shotgun_Standard_Athena_C_Ore_T03 | Sniper_BoltAction_Scope_Athena_R_Ore_T03 |
|---|---|---|---|
| RecoilVert (0x194) | 5.0 | 9.9 | 5.0 |
| RecoilVertScale (0x198) / ...Gamepad (0x19C) | 1.0 / 0.5 | 1.0 / 0.75 | 1.0 / 0.5 |
| VertRecoilDownChance (0x1A0) | 0 | 0 | 0 |
| RecoilHoriz (0x1A4) | 0.25 | 2.0 | 0.0 |
| RecoilHorizScale (0x1A8) / ...Gamepad (0x1AC) | 1.0 / 1.0 | 1.0 / 1.0 | 1.0 / 1.0 |
| RecoilInterpSpeed (0x1B0) | 8.0 | 12.0 | 12.0 |
| RecoilRecoveryInterpSpeed (0x1B4) | 6.0 | 3.6 | 6.0 |
| RecoilRecoveryDelay (0x1B8) | 0.0 | 0.0 | 0.0 |
| RecoilRecoveryFraction (0x1BC) | 1.0 | 1.0 | 1.0 |
| RecoilDownsightsMultiplier (0x1C0) | 0.5 | 0.85 | 0.4 |
| AthenaRecoilMagnitudeMin / Max / Scale (0x1C4 / 0x1C8 / 0x1CC) | 4 / 4 / 1 | 9 / 9 / 1 | 5 / 5 / 1 |
| AthenaRecoilAngleMin / Max (0x1D0 / 0x1D4) | -25 / 25 | -25 / 25 | -25 / 25 |
| AthenaRecoilRollMagnitudeMin / Max (0x1D8 / 0x1DC) | 0 / 0 | 0 / 0 | 0 / 0 |
| AthenaRecoilInterpSpeed (0x1E0) / RecoveryInterpSpeed (0x1E4) | 12 / 6 | 12 / 4 | 12 / 6 |
| AthenaRecoilDownsightsMultiplier (0x1E8) | 0.5 | 0.85 | 0.4 |

There is no `KickbackForce` / `Kick*` column in the 1.8 row struct. The weapon Blueprints (`B_Assault_Auto_Athena`, `B_Shotgun_Standard_Athena`,
`B_Rifle_Sniper_Athena`, `B_Ranged_Generic`) and the camera mode Blueprints (`Athena_PlayerCameraMode{Base,Ranged,RangedTargeting,Sniper,TargetingAssault,
TargetingShotgun,TargetingScope}`) contain no recoil / kick / shake property, class entry or Kismet bytecode: recoil is native code + the stat row only.
(`ImpactCameraShake` on the weapon class is the impact shake, not recoil.)

## Decoded native functions

| RVA | what it is |
|---|---|
| 0x853440 | `AFortPlayerController::UpdateRotation(dt)`: `ViewRotation = GetControlRotation()` (vcall +0x5F0), `DeltaRot = RotationInput` (`this+0x588`); calls the held weapon's vtable slot **+0x760** `(this=weapon, dt, &ViewRotation, &DeltaRot)`, then `PlayerCameraManager` vtable +0x6D0 (`ProcessViewRotation`), then `SetControlRotation`. **Recoil is added to `DeltaRot` (the look-input delta, degrees: pitch, yaw, roll).** |
| 0x936200 | weapon vtable +0x760 = UpdateRecoil (`AFortWeaponRanged`). Contains the targeting aim-adjust block (0x9362af..0x9366b6, not recoil) followed by the recoil step (0x9366be..0x936e55). |
| 0x917930 | AddRecoil: called once at the end of the fire function (vtable +0x940, RVA 0x933630, line 0x934256), next to the spread heat add 0x917e40. |
| 0x92bab0 / 0x925660 | vertical / horizontal recoil value: `(IsUsingGamepad ? *ScaleGamepad : *Scale) * Recoil*`, then the gameplay-effect aggregate of attribute `WeaponVerticalRecoil` / `WeaponHorizontalRecoil` (u"..." at 0x1442911e8 / 0x144290dc0; base value expected 0, logged otherwise), then `maxss(x, 0)`. |
| 0x5c9540 -> 0x22c4030 | `IsUsingGamepad()` (proven by 0x5ffc10 logging `IsUsingGamepad() : %d` on its result). |
| 0x7c8b10 | `UFortWeaponItemDefinition::GetWeaponStats` (returns the `FFortRangedWeaponStats*` all the code above reads). |
| 0x12db910 | `FMath::FInterpTo`. |
| 0x921cf0 | fire notification: `LastFireTime (+0x954) = World->TimeSeconds` (`[GetWorld()+0x900]`, vcall +0x130). Ctor 0x914b60 initialises `LastFireTime = -10.0`. |
| 0x92de40 + cvar `Weapon.UseAthenaRecoil` | the Athena recoil path is used when the owner is an Athena pawn AND the cvar is non-zero. Cvar registered at 0xafd10 ("Let's us turn on/off new athena recoil code. Eventually this will not be a cvar and will just be on."); storage 0x14662C1A0 is `.bss` and nothing in the exe writes it, so **the exe default is 0 = legacy `Recoil*` columns**. |
| 0x143fa67b8 | IAT slot of the CRT `rand` (import-by-name hint "rand"); every random draw below is `(float)rand() * 0x38000100` (1/32767). |

Weapon native members used (not UPROPERTYs): `+0xB9C` pending kick (pitch, yaw, roll), `+0xBA8` offset-to-recover, `+0xBB4` manually-recovered (names from the
engine's own log text at 0x1442917a0), `+0x965` `bIsTargeting`, `+0x954` `LastFireTime`, `+0x80` `CustomTimeDilation`.

## Ported formula (`recovered_recoil.rs`)

All arithmetic is f32 in the compiled operation order. Rotators are (pitch, yaw, roll) in degrees; pitch up and yaw right are positive.

**Per shot (AddRecoil, once per fire call, `LastFireTime = now` first).** `V = max(scale_v * RecoilVert, 0)`, `H = max(scale_h * RecoilHoriz, 0)` (gamepad scales when
`IsUsingGamepad`).
* Legacy path (cvar 0, the exe default). Four `rand()` draws `f = rand * 1/32767` in this order:
  `pitch = f1 * (V - V/2) + V/2`; `yaw = f2 * (H - H/2) + H/2`; `if !(f3 >= VertRecoilDownChance) pitch = -pitch`; `if !(f4 >= 0.5) yaw = -yaw`;
  `if targeting { pitch, yaw *= RecoilDownsightsMultiplier }`; `pending += (pitch, yaw, 0)`; then `pending.pitch` is clamped to +-V and `pending.yaw` to +-H.
* Athena path (cvar != 0). Three draws: `mag = (f1 * (MagMax - MagMin) + MagMin) * MagScale`; `ang = (f2 * (AngleMax - AngleMin) + AngleMin) deg`;
  `roll = (f3 * (RollMax - RollMin) + RollMin) * MagScale`; `(sin, cos) = SinCos(ang)` (the compiled polynomial, `sin_cos`);
  `pending += (cos*mag, sin*mag, roll) [* AthenaRecoilDownsightsMultiplier when targeting]`; no clamp.

**Per frame (UpdateRecoil)**, with `input = DeltaRot` on entry, `speed/fraction` chosen by the path (Athena: `AthenaRecoilInterpSpeed`, fraction fixed 1.0,
`AthenaRecoilRecoveryInterpSpeed`; legacy: `RecoilInterpSpeed`, `RecoilRecoveryFraction`, `RecoilRecoveryInterpSpeed`):
1. `if !pending.IsNearlyZero(0.5) || !toRecover.IsNearlyZero(0.5) { manual -= input } else { manual = 0 }`.
2. If `pending != 0`: `step = pending - FInterpTo(pending, 0, dt, speed)` per axis; `pending -= step`; `DeltaRot += step`; `toRecover += fraction * step`;
   if every step is exactly 0, `pending = 0`.
3. If `toRecover != 0`: reconcile `manual` against `toRecover` (pitch: four cases incl. the quirks as compiled; yaw: two cases; roll none) - see `pitch_block` / `yaw_block`.
4. If `toRecover != 0` and `(now - LastFireTime) * CustomTimeDilation > RecoilRecoveryDelay`: `step = toRecover - FInterpTo(toRecover, 0, dt, recoverySpeed)`;
   `toRecover -= step`; `DeltaRot -= step`.
5. If `toRecover.IsNearlyZero(1e-4)`: `toRecover = 0`.

Net effect with the starter rows (`RecoilRecoveryFraction 1`, delay 0): each shot kicks the view up (and a little sideways) by an eased amount and the same amount is
eased back out afterwards unless the player pulls against it (the manual-compensation state shrinks what recovery gives back).

## Wiring (`// RECOIL HOOK`)

`crates/fn-app/src/hitreg_hook.rs` (`on_arms`): per new shot of gun g -> `on_fire(now)` + `add_recoil(...)`; per frame the held gun's `update(...)` adds the
step to `inp.pitch_delta` / `inp.yaw_delta` (the frame's look delta, after the ADS look scaling), which `GameSim::begin_frame` adds to yaw / pitch (pitch clamp as before).
This replaces the `fn_hitreg::Recoil` stand-in (no longer used by the app). `crates/fn-app/src/app.rs` passes the frame input and the gamepad flag. The recoil is on by
default (not behind `--assume-unrecovered`).

## Missing / not proven (left out, not approximated)

* **`Weapon.UseAthenaRecoil` effective value: 0 (legacy `Recoil*` columns), resolved 2026-10-05.** The exe default is 0 (`.bss` storage, no writer; reads only in
  AddRecoil 0x917930 / UpdateRecoil 0x936200). The cooked config (`C:\FortniteRef\scratch\CookedIniVersion.txt`, see `docs/spec/cooked_ini_audit.md` A4/W2) has no
  `Weapon.UseAthenaRecoil` in `[SystemSettings]` (L966-997), `[ConsoleVariables]` (L1917-1936) or any DeviceProfile (L5587-5590). The app uses the legacy path:
  `crates/fn-app/src/hitreg_hook.rs` `ATHENA_RECOIL = USE_ATHENA_RECOIL_EXE_DEFAULT` (false). Only an online hotfix ini could set it to 1 (not covered); both paths stay ported.
* **Seed of the CRT `rand()` stream.** `rand` is imported (not in the exe); `CrtRand` is the documented UCRT LCG (test pins 41, 18467, 6334, ...). The exe's `srand` seed
  is not recovered, the hook seeds it from the clock (the original is not deterministic either).
* **`IsUsingGamepad` update logic.** The byte read (0x22c4030) is proven to be `IsUsingGamepad()`; which input events flip it is not recovered. The hook uses "the right stick is
  the looking device" (same predicate the ADS look rate already uses).
* **`PlayerCameraManager::ProcessViewRotation` (vtable +0x6D0, `UpdateRotation` 0x8534c1)** is not decoded; the port adds recoil to the look delta and relies on `GameSim`'s existing pitch clamp.
* **Targeting aim-adjust block of 0x936200** (`MaxTargetingAimAdjustment`, `TargetingRotAdjustmentTotal`, gated by `[weapon+0xABC] & 4`): not recoil, not ported.
* **Gameplay-effect modifiers** of the `WeaponVerticalRecoil` / `WeaponHorizontalRecoil` attributes (aggregator call 0x3d74360): there are no gameplay effects here, so the aggregate is the stat value.
* Which callers reach the fire function (0x933630) per cartridge for the shotgun: AddRecoil is called once per execution of that function (not per pellet inside it); the exact
  trigger-to-call order relative to `LastFireTime` (0x921cf0) is not traced (it only matters for the tick the shot lands in, since `RecoilRecoveryDelay` is 0).
* `cvttps2dq` overflow in `IsNearlyZero` for |x| > 7.7e11 degrees is not reproduced (Rust saturates).

## Tests

`cargo test --release -p fn-weapons`: 75 passed (14 new in `recovered_recoil`: recoil values / gamepad scales per row, FInterpTo, IsNearlyZero, CRT rand, SinCos polynomial,
legacy and Athena AddRecoil arithmetic incl. clamp / downsights / down-chance, UpdateRecoil step, recovery delay + time dilation, manual-compensation branches,
Athena speeds + fraction, full kick-and-recover round trip). `cargo build --release -p fn-app`: ok. Smoke test (`--smoke-test`, 1.8 paks + assets): exit code 0, `RESULT: ok`.

## Files changed

* new `crates/fn-weapons/src/recovered_recoil.rs`, `crates/fn-weapons/RECOIL.md`; `crates/fn-weapons/src/lib.rs` (`pub mod recovered_recoil; // RECOIL HOOK`)
* `crates/fn-app/src/hitreg_hook.rs`, `crates/fn-app/src/app.rs` (one call site), `crates/fn-app/src/arms.rs` (recoil removed from the "not recovered" text only)
* new research tool `crates/fn-exe-defaults/src/bin/scan.rs` (whole-function disassembly, displacement scan, callers, JSON dump)
* evidence: `parity/recoil-evidence/{recoil-natives-disasm.json, recoil-fields-and-rows.json, bp/}`
