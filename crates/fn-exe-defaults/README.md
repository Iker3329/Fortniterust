# fn-exe-defaults

Static, read-only recovery of UE 4.16 **native defaults** from the 1.8 client exe
(`FortniteClient-Win64-Shipping.exe`, x64, `.text` not encrypted). The exe is never executed; it is parsed with
`fn-exe-slate` (PE loader, `.pdata`, xref index) and disassembled with `iced-x86`.

Output: `reports/exe-defaults.json` (regenerate with `cargo run --release -p fn-exe-defaults --bin report`;
`FN_EXE` overrides the exe path). Tests: `cargo test -p fn-exe-defaults` (skip with a note when the exe is absent).

## What is recovered, and how

| Area | Method | Confidence |
|---|---|---|
| Console variables (`r.*`, `sg.*`, `fx.*`, ... 1127 resolved of 1152 variable registrations) | Each `TAutoConsoleVariable<T>(Name, Default, Help, Flags)` / `FAutoConsoleVariableRef` is inlined into a tiny static-init thunk that calls `IConsoleManager::Get()->RegisterConsoleVariable*` through a vtable slot (0x00 string, 0x08 float via `xmm2`, 0x10 int via `r8d`, 0x20 ref-float, 0x28 ref-int). The name is `lea rdx,[UTF-16 literal]`; the default is the argument register at the `call`, tracked by the straight-line constant tracker in `emu.rs`; flags are `[rsp+0x20]`. | high (29 cvars cross-checked against known UE 4.16 source defaults, all match) |
| `FPostProcessSettings` defaults (135 fields) | The struct ctor (`0x4D3210`, 1716 bytes) does `memset(this,0,0x4C0)` then stores every non-zero default. It is found by a 12-value default fingerprint and confirmed by `UPostProcessComponent::UPostProcessComponent` calling it for `Settings` (+0x2A0). Names/offsets come from `Z_Construct_UScriptStruct_FPostProcessSettings` (each UProperty ctor gets `CPP_PROPERTY_BASE` offset in `r9d`). | high |
| `ULightComponentBase`, `ULightComponent`, `UDirectionalLightComponent`, `USkyLightComponent`, `UExponentialHeightFogComponent`, `UPostProcessComponent`, `APostProcessVolume` | `GetPrivateStaticClass` (literal `+2` pointer bump) -> `ClassConstructor` thunk -> `InternalConstructor` -> real ctor; property offsets from the class's `Z_Construct_UClass_*`; base-to-derived ctor chain merged. | high |

## Not recovered (honest list)

* **bool bit-field properties** (`bOverride_*`, `AmbientOcclusionRadiusInWS`, `bLowerHemisphereIsBlack`, `bAffectsWorld`, ...): UE 4.16 registers them with a runtime bit probe, so their names cannot be paired with offsets statically. Raw `or/and [this+disp],imm` bit ops are listed in the JSON (`bit_field_or_and_ops`) without names.
* **Config overrides**: the paks' `.ini` are encrypted; code-level defaults can be overridden by `DefaultEngine.ini`/Scalability at runtime and by serialized assets. The JSON values are the C++ defaults, nothing else.
* 25 variable registrations whose default is computed at runtime (strings built on the stack, `FRAME`/`BeginMFUpdate` false positives, `Slate.bAllowThrottling`).
* Cascade-shadow/exposure *behaviour* (what the engine does with a value) is not read here, only the values.
* `FPostProcessSettings` fields of array/object type (Blendables, textures) have no default to report.

## Layout

* `ana.rs` string search + disassembly helpers; `emu.rs` register/stack constant tracker; `cvar.rs` cvar registrations;
* `reflect.rs` property name -> offset from `Z_Construct_*`; `classes.rs` class -> ctor + properties;
* `ctor.rs` constant stores (`mov [this+d],imm`, `movss`, 128-bit `.rdata` stores, `or/and` bit ops); `pps.rs` `FPostProcessSettings`;
* `known.rs` cross-check values and the values the Rust code currently assumes; `bin/report.rs` writes the JSON;
* `bin/dx.rs`, `bin/cvars.rs`, `bin/classdump.rs`, `bin/props.rs`, `bin/stores.rs` are exploration tools (disassembly, xrefs, ctor dumps).
