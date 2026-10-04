# fn-ui: world feedback HUD (reticle, harvest popups, Target Info, interaction prompt, equip progress)

This section documents what was added on top of the HUD (`screens::hud`, quickbar, vitals, hud_extra). Everything is pure data -> `RenderList`; the wgpu translation of the two new materials is in `fn-ui-wgpu`.

## Contract (`screens/reticle_state.rs`) - unify with `fn-hud-adapter::feedback` here

`HudState` has a new field `world: HudWorld` (default = a quiet HUD; `fn-hud-adapter::hud_state` fills it with `..HudState::default()` until the feedback producer is wired):

| `HudWorld` field | meaning | adapter side (resync2 `feedback.rs`) |
|---|---|---|
| `reticle: ReticleState { on_target }` | aim ray found a prop within pickaxe reach -> yellow | `ReticleState::colour_building` (the adapter's `visible` = pickaxe equipped AND toggle on is computed inside fn-ui from `items[selected] == pickaxe` and `HudVisibility::reticle`) |
| `harvest_hits: Vec<HarvestHit>` | hits since the last `Hud::set_state`; **consumed and emptied by `set_state`** (a second `present` / empty `set_state` never replays them) | `fn_combat::HarvestEvent { object, resource, amount, destroyed, weakpoint }`: field-for-field (`HarvestResource` <- `Resource`) |
| `current_resource` | pack that gets the highlighted border (ASSUMED wood) | `PackView::highlighted` |
| `target: Option<TargetInfo>` | focused building: id, health, max, `HealthDisplayRule`, interaction number, projected anchor | (new) |
| `interaction: Option<InteractionPrompt>` | key text + description + projected anchor | (new) |
| `equip: Option<EquipProgress { elapsed, duration }>` | `GameplayCue.Athena.Equipping` timer | (new) |

The popup state machine, the 0.5 s count-up and the 999 dimming live in fn-ui (`screens::reticle::Popup`, `hud_extra::CountAnim`) so a renderer-free duplicate in `fn-combat` is only needed if a non-fn-ui front-end wants the same numbers; the adapter may either keep feeding raw `HarvestHit`s (preferred, nothing else to unify) or replace the structs by re-exports.

Driving it: `hud.set_state(&state)` (consumes `harvest_hits`), `hud.tick(dt)` (popup + count-up + Target Info timer), `hud.present(ctx, w, h, &mut list)`. The seven Settings > HUD rows are `HudVisibility { reticle, player_health, pickup_info, minimap, quick_bar, target_info, interaction_info }` (`HudVisibility::from_tags` over the `[HUD]` extras); **all seven now drive visibility** and the Settings page no longer marks them NOT IMPLEMENTED (`menu/pages/rows.rs::stored_only`). `HUD.Center.ResourceAggregation` (the `+N` popups) and the equip timer have no row and are never hidden by a toggle.

## What is drawn, and where the data comes from

* **Reticle** (`screens/reticle.rs`): `HUD_Xhair_Dot` 58x54 at screen centre, white / `FColor(255,221,0)` on target, only while the pickaxe is the equipped item and `HUD.Center.Reticle` is on. EXACT (data + native findings of the old `reticle-1.8-evidence.json`, copied to `testdata/`).
* **Harvest `+N` popups** (same file): `ResourceAggregationWidget` + `PotentialResourceWidget` bytecode state machine (same object aggregates, 3 s `ForceHideWidget`, destroy path waits for `FinishedShowing`, `ResetSourceOfResource`), the three MovieScene animations (`AggregateOutroAnim` 1.0 s, `ResourceIconPulseAnim` 0.35 s, `TranslateAndFadeUp` 1.0 s, cubic RichCurve keys incl. arrive tangents) evaluated like `FRichCurve::Eval`, weak-point variant (size 20, `Color-HarvestWeakPoint` cyan). EXACT; the keys are pinned against the evidence JSON by unit tests.
* **Packs** (`hud_extra.rs`): `PackResource.UpdateCount` count-up (`InterpolateToValue(count, 1, 0.5, 0)`: linear over 0.5 s; the roles of the first / last arguments are ASSUMED), text alpha 0.6 at `MaxStackSize` 999, highlighted border of the current resource = new `MaterialKind::PanelVGradHighlighted` (`M_Border_LightNavy_VGrad`: same compiled `M_Border_HUD_VGradient` shader, Global Alpha 0.7, Rotation 0, Scale 1, RGBA0 (0.122139,0.14996,0.266356,1), RGBA1 (0.08022,0.102242,0.201556,1), all read from the cooked instance with fn-inspect). EXACT apart from "current resource = wood".
* **Target Info** (`screens/target_info.rs`, tag `HUD.InWorld.BuildingInfoIndicator`): `BuildingInfoIndicator` + `BuildingBar_C` decoded from their Kismet: focus reset, `UpdateBuildingInfo` visibility switch by `HealthDisplayRule` x interaction type, health handler (3 s `HideHealth` timer for rule 1 + interaction None), the 7-way `Set Display Mode` colour tables, 288x24 bar (baseplate, `HUD-Bar_Material` fill / delta, lead-edge bevel with the `min(1, w / 28.8)` alpha, `{cur} | {max}` text with `Conv_IntToText` grouping, `Border_HUD_Line_Vert` divider). `Update Current HP Value` writes `Last := new`, so the delta bar never trails (implemented literally). APPROXIMATE: screen placement (bar centre on the producer's projected anchor; the Hidden `TrapVerticalBox` above the bar is ignored), the names of rule 1 (`OnDamage`) and interactions 1 / 2 are inferred, modes 1 and 6 draw no fill; whether trees / rocks use rule `OnDamage` is a property of the actor class (not in the HUD data), the producer decides.
* **Interaction prompt** (`screens/interaction.rs`): `InteractionIndicator` -> `BasicInteractionWidget` path: `Border-KeyBase-Center-M` panel (`Keybase-Center-L` box, margin L/R 0.5, tint (0,0,0,0.5), padding 5), Medium keycap (`UpdateFrameSize` table: 32 min size, font 15, padding 3.2, `Keybind-Frame-M` at alpha 0.6), description NotoSans-Bold 18 wrapped at 300, spacers 8, offset (65,-65) from the projected actor. APPROXIMATE: pivot of the native indicator canvas (panel centre placed at anchor + offset), greedy word wrap, the hold ring (`SetShowProgressCountDown`), pickup card and defender beacon variants are not drawn.
* **Equip progress** (`screens/equip_progress.rs`): `AthenaEquipProgress` (ScaleBox 0.75 over 105x105, `Results_TimerBacking`, `M_HealProgress` radial wipe, NotoSans-Regular 30 `{:.1}` remaining seconds, slot centre / bottom, padding 160). The wipe material is the Slate variant of `CircularProgressBar` (`shader_5125`) translated by hand: `material::hud::radial_angle` / `radial_mask` / `radial_progress` (CPU) and `fn-ui-wgpu/src/shaders/hud_radial.wgsl` (GPU, pinned to the CPU within 1/255 by `hud_materials_gpu_matches_cpu`). Starts at 12 o'clock and sweeps clockwise. APPROXIMATE: `Results_TimerBacking` is drawn as its plain texture (the parent material `Common1` was not disassembled).
* **Damage numbers / weak-point effect: not implemented, and not widget data.** The 1.8 packages define them as world-space meshes and materials (`UI/DamageText/*`: `damage_text` / `DamageNumbers_Mat` / `DamageNumberWPO` ... and Cascade `P_WeakPointEffect_01` with `Sm_Weakpoint_{Crosshair,Ring,Dot,Connector}_01`, `M_Weakpoint_*`); they belong in `fn-render`'s world pass, not in the UI draw list. (The `+N` harvest text and its weak-point colour, which are widget data, are done.)

## Textures
`HUD_Xhair_Dot`, `Results_TimerFill`, `Results_TimerBacking`, `Keybase-Center-L` were added to `data/ui-texture-map.tsv`. The last three exist only in the paks (not in the old exported folder): use `--assets pak|layered|auto` to see them; `pak_assets::every_scene_renders_identically...` compares those three scenes through the layered source.

## Scenes and numeric comparison (`screens/scenes_world.rs`, `fn-ui-demo --list | --screenshot | --diff`)
`ret-white`, `ret-yellow`, `ret-toggle-off` and the 11 `ret-harvest-*` popup captures replay the old `--reticle-test` script (old state log `reticle-test.txt`) and diff against `verification/ret-*.bmp`; `pack-cap-highlight`, `pack-countup-half`, `target-info-damaged`, `target-info-full`, `interaction-prompt`, `equip-progress-25/75` have no old capture (test inputs).

`fn-ui-demo --diff <scene> 1920x1080,1280x720 [--cpu|--gpu]` (needs `FN_UI_OLD_ROOT` = the old tree, `FN_UI_ASSETS` = its `assets`; the old captures contain the 3D world, so the metric that is valid is `masked` = mean abs RGB difference over pixels our UI covers with alpha >= 252):

| renderer | masked mean (0..255) | bad > 8 | scenes |
|---|---|---|---|
| CPU (`fn_ui::raster`) | 0.06 - 0.07 | 0.0 % | all 14 ret scenes at 1920x1080, 1280x720, white/yellow also 2560x1080, 1024x768, 640x480 |
| GPU (`fn-ui-wgpu`) | 1.09 (1080p) / 1.82 (720p) / 1.66 (1024x768) / 2.2 (640x480) | 1.3 - 4.4 % | same; the GPU-vs-CPU gap sits in the minimap and the scaled / minified texture and glyph sampling that predate this work |

Two test-side numbers that also cover the translucent edge pixels (`cargo test -p fn-ui old_ret -- --nocapture`): the reticle and every popup layer composited over the old capture without it differs from the old capture by mean <= 0.06 (max 2/255) over its bounding box; the one exception is `ret-white-1280x720`, whose old capture was taken before the 3D world drew its first frame (black world): composited over black the mean is 1.2 (max 3). The tests assert `< 0.5` (opaque) and `< 2.0` (overlay).

## Not covered / remains
Real producers for `target`, `interaction`, `equip` (no game state exists for them in the baseplate sandbox); `ReticleStatusWidget`, ranged reticle, build-mode icons, game phase, kill feed and the rest listed in the old README stay out of scope; the minimap interior, DPI curve default and fontdue glyph rasterisation remain the approximations documented in `hud_extra.rs`.
