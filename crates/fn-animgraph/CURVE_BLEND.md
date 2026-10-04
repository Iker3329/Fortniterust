# Animation float-curve combination (1.8 exe)

Static disassembly of `FortniteClient-Win64-Shipping.exe` only (never executed; RVAs are image offsets, base 0x140000000).
Evidence: `parity/curve-blend-evidence/` (`curve-blend-rules.json` = decoded rules + RVAs, `curve-blend-natives-disasm.json` +
`curve-blend-leaf-accumulate.json` = whole-function instruction dumps). Port: `skeleton.rs` `sample_blended_curve`, `graph.rs` `CurveBlend`.

Common facts: a curve array element is `{f32 Value; u8 bValid}` (stride 8). `ZERO_ANIMWEIGHT_THRESH` = 1e-5 (RVA 0x4008e1c).
`FBlendedCurve` helpers: `InitFrom` 0x1ac8da0 (zeroed, all invalid), `CopyFrom` 0x1f9f350, `Override(other, w)` 0x1f9f3d0
(copy; if |w-1| > 1e-8 then `Value = w*Value`, flags copied), `Accumulate(other, w)` 0x1f848b0 (if w > 1e-5: `valid |= other.valid`,
`Value += w*other.Value`, regardless of other's flag), `Lerp(A,B,a)` 0x1f9c230 (a<=1e-5 -> A; |a-1|<=1e-5 -> B; else `(B-A)*a + A`,
`valid = vA || vB`). No element-wise `max` / Combine / UseMaxValue exists in this code. A curve present in only one input behaves as
value 0 in the other for lerp/accumulate, with `valid` OR-ed.

`ECurveBlendOption` (enum registration 0x277c3e0): **MaxWeight = 0, NormalizeByWeight = 1, BlendByWeight = 2**.
`FAnimNode_LayeredBoneBlend`: sizeof 0xB0, vtable 0x469a418, Evaluate 0x1ab3660, `CurveBlendOption` byte at +0x79, `bMeshSpaceRotationBlend` +0x78.

| rule | exe RVA | decoded arithmetic | port before | port after | test |
|---|---|---|---|---|---|
| LayeredBoneBlend MaxWeight (0) | BlendPosesPerBoneFilter 0x1f87ed0 `0x1f887c3-0x1f88833`, mesh 0x1f888c0 `0x1f89755-0x1f897d7` | `Result = copy(Base)`; every blend curve element with `bValid` REPLACES the result element (value and flag); no weight, no max(). Gated only by the blend pose being evaluated (BlendWeights[i] > 1e-5, Evaluate 0x1ab37c4) | `max(base, blend)` (wrong) | valid blend curve overrides the base; absent curve keeps base | `layered_max_weight_overrides_with_valid_blend_curve_instead_of_max` |
| LayeredBoneBlend BlendByWeight (2) | 0x1f87ed0 `0x1f88418-0x1f884c6` (and mesh `0x1f892ea-0x1f893a5`) | `copy(Base)` then for blend pose i with w_i > 1e-5: `Value += w_i * blend`, `valid |=`; w_i = max over bones of clamp(boneWeight,0,1) | `base + blend*w`, gate `w <= 1e-4` | same arithmetic, gate 1e-5 | `layered_blend_by_weight_accumulates_down_to_the_1e_5_threshold` |
| LayeredBoneBlend NormalizeByWeight (1) | 0x1f87ed0 `0x1f884cb`, mesh `0x1f893aa` | weights (incl. base 1.0) divided by their sum if sum > 1e-5, then Override + Accumulate | not modelled | not modelled (no 1.8 node uses it, LAYERED.md) | - |
| Rotation-offset blend space (aim offset) | `FAnimNode_RotationOffsetBlendSpace` vtable 0x4690048, Evaluate 0x1ab5160 -> `0x1f852d0` called directly at 0x1ab5246 | `0x1f852d0` (mesh-space rotation additive -> 0x1f84fa0) never reads or writes the curves; Output.Curve stays the BasePose curve | `base + samples*alpha` (wrong) | base curve only | `rotation_offset_blend_space_keeps_only_the_base_curve` |
| ApplyAdditive / ApplyMeshSpaceAdditive | Evaluate 0x1ab2c40 / 0x1fda7a0 -> `AccumulateAdditivePose` 0x1f84f30 -> `Accumulate` 0x1f848b0 (call 0x1f84f75) | Alpha <= 1e-5: base only; else `base + alpha*add`, `valid |=`; Alpha NOT clamped | clamp(alpha,0,1) | raw alpha, 1e-5 gate | `apply_additive_uses_unclamped_alpha_and_the_1e_5_gate` |
| Mix / blend-space (BlendPosesTogether) | 0x1f89870 (variants 0x1f89af0, 0x1f89dc0; per-bone 0x1f8a040 / 0x1f8a380); blend-space dispatcher 0x1ff96e0 | `Override(c0, w0)` then `Accumulate(ci, wi)` for i>=1 with wi > 1e-5; weights used AS GIVEN, no normalisation inside | normalised weighted sum | unchanged semantics (still normalised, identical when weights sum to 1); gate 1e-5; flags OR-ed | covered by existing tests (no behaviour change) |
| State-machine crossfade | StateMachine vtable 0x4960350, Evaluate 0x1fdb150 -> 0x1fdbbc0 -> 0x1fdbd70 (curve `0x1fdbfea-0x1fdc094`) | `Override(c1, 1-a)` then (a > 1e-5) `Accumulate(c2, a)` = `(1-a)*c1 + a*c2` | same | same (a <= 1e-5 now returns `(1-a)*c1`) | no new test (value-identical) |
| TwoWayBlend (BlendTwoPosesTogether) | 0x1ab5360 -> 0x1f8aad0 -> `Lerp` 0x1f9c230 | see Lerp above | n/a (no node) | n/a | - |
| Slot, full-pose montage | Slot Evaluate 0x1fdac90 -> `SlotEvaluatePose` 0x1fcb6b0; weights `GetSlotWeight` 0x1fc2e90 (via Slot Update 0x1fea7f0) | non-additive montage gets its instance weight w, source gets `SourceWeight = 1 - sum(nonAdditive weights)` (normalised if total > 1), combined with BlendPosesTogether: `w*m + (1-w)*s`; slot weight <= 1e-5 -> source copy | `lerp(source, montage, w)` | unchanged: already identical; `Node` has no Slot variant so it lives outside `sample_blended_curve` | - |

## UNVERIFIED / limits
* **Curve presence.** The exe tracks `bValid` per curve. `ClipSource::sample_float_curve` returns only `f32`, so `sample_blended_curve` uses
  `value != 0` as the validity proxy (documented in the function). It matters only for MaxWeight: an explicitly authored zero in the blend pose
  replaces the base value in the exe but keeps it in the port. Exact behaviour needs a `has_float_curve(name)` on `ClipSource`
  (a trait change, deliberately outside this task's edit scope).
* Mix / blend space: the engine function does not normalise weights; the port normalises like its pose path. Which engine caller normalises the
  blend-space sample weights was not decoded.
* Slot: the arithmetic is decoded, but the port has no Slot node in `sample_blended_curve`; the equivalence `SourceWeight = 1 - w` holds for one
  full-pose montage (GetSlotWeight 0x1fc2e90).
* The blend-space additive path (0x1ff9db0, `Accumulate` call at 0x1ffae2a) and `NormalizeByWeight` are located but not ported.

## Tests
`cargo test --release -p fn-animgraph`: 48 passed (4 new, module `skeleton::curve_blend_tests`). `cargo build --release -p fn-app`: ok.
