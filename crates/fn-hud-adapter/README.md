# fn-hud-adapter

`fn_game::GameSim` + `fn_settings::Settings` (+ [`HudInputs`] for what the game does not model yet) -> the plain HUD data the UI draws. `fn-app` only passes it through.

* `hud_state(&sim, &settings, &inputs) -> fn_ui::screens::hud::HudState` (vitals, quickbar, resources, compass, minimap, toggles, loading).
* **World feedback** (`feedback.rs`, below): the reticle, the harvest `+N` popups, the pickup-manager packs and the hierarchical `[HUD]` tag rule. The engine is
  `fn_combat::reticle` (renderer-free port of the old `reticle.rs`, `hud_vis.rs` and the `hud_extra.rs` count-up), this crate exposes its output as plain data.

## Data contract (what a renderer draws; no behaviour, no globals)

Per rendered frame:
```rust
fb.tick(&mut sim, &settings, pickaxe_equipped /* quickbar slot 0 */, metal, dt);   // HudFeedback
draw_reticle(fb.reticle);  draw_popups(fb.popups.view());  draw_packs(fb.packs);
```

| type | fields | meaning |
|---|---|---|
| `ReticleState` | `visible`, `colour_building`, `tint: [f32;4]`, `image: "HUD_Xhair_Dot"`, `image_size: (58, 54)`, `centre_offset: (0,0)` | `visible` = pickaxe equipped AND `HUD.Center.Reticle` row on. `colour_building` = the 288 cm aim ray (camera eye + forward, 8192 cm long) found a LIVE prop within `RangeVSBuildings2D` 2.88 m / `RangeVSBuildingsZ` 3.84 m of the pawn: tint (255,221,0) instead of white. Centred in the safe zone (zero offset), texture size x DPI scale, snapped to whole pixels (`ReticleState::rect(w, h, dpi_scale)`). No melee hit marker exists in 1.8. |
| `HarvestPopups` | `on_hit(HarvestEvent)`, `tick(dt)`, `view() -> Option<PopupView>` | The `ResourceAggregationWidget` + `PotentialResourceWidget` Kismet state machine: same object -> aggregate += N, other object -> reset; every hit restarts `ResourceIconPulseAnim` (0.35 s) and `TranslateAndFadeUp` (1.0 s); a non-destroying hit (re)arms the 3 s `ForceHideWidget` timer, a destroying hit waits for the popup; then `AggregateOutroAnim` (1.0 s). Curves are the cooked cubic RichCurves. |
| `PopupView` | `aggregate: Option<AggregateView>`, `potential: Option<PotentialView>`, `icon`, `icon_size: 40` | |
| `AggregateView` | `resource`, `text "+N"`, `font 32`, `params: AggParams { hbox_ty, hbox_scale, text_alpha, shadow_alpha, icon_alpha, icon_scale, text_scale }` | row = 40x40 icon + text, black shadow copy at (3, 5) units; slot padding R400 / B300, SizeBox max 200 x 300 (`fn_combat::reticle::popup_layout`). |
| `PotentialView` | `text "+N"`, `weakpoint`, `font 16 / 20`, `colour` (white or cyan `Color-HarvestWeakPoint`), `params: PotParams { ty, shadow_ty, alpha, shadow_alpha }` | shadow copy at (1, 2), render scale 1.2 in the vertical box. |
| `PackView` | `shown: [u32;3]`, `target: [u32;3]`, `text_alpha: [f32;3]`, `highlighted` | Wood / Stone / Metal counters: `shown` counts up linearly over 0.5 s (`NumericTextCount.InterpolateToValue(count, 1, 0.5, 0)`), counts clamp to MaxStackSize 999, the text alpha is 0.6 at 999 (dimmed) else 1, `highlighted` (0 wood, ASSUMED) uses `Border_HUD_ResourceBg_Highlighted`. |
| `HudTagVisibility` | `from_settings`, `visible(widget_tag)`, `set` | GameplayTag hierarchy of the Settings > Game > HUD rows: a switched-off row `R` hides `R` and every `R.*` widget tag (`HUD.Quickbar` covers `HUD.Quickbar.Combat`); `HUD.Center.ResourceAggregation` has no row, so the popups cannot be hidden by a toggle. |

Field names match `fn_ui::screens::reticle_state` (`HarvestHit == fn_combat::HarvestEvent`, `ReticleState::on_target == colour_building`), so unifying is a field-for-field
map once that module is wired in (the hud-extras agent draws; replace its structs by these when ready).

APPROXIMATE / ASSUMED (kept from the old tree): the C++ default of `ReticleDefaultColor` (white), the aim trace's collision channel (props only, the baseplate is never a
hit), the melee range of the colour test (the datatable ranges, measured from the pawn eye), resource per hit (`HARVEST_DAMAGE` 50, provisional), current resource = wood,
no weak points on the placeholder props.

## Not owned here
Drawing (textures, text, shaders): `fn-ui` / `fn-ui-wgpu`. Loading the data JSON: `fn-testdata` (tests only).
