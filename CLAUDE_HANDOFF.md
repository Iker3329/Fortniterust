# FortniteRuntime-rs handoff

## What is now verified

- `cargo check --offline -p fn-app` passes.
- Normal startup now includes the existing game-mode picker: `Splash -> StartupLoading -> GamePicker -> Loading -> Playing`. The picker was already fully implemented in `crates/fn-app/src/startup.rs`; this handoff only changes the normal default in `crates/fn-app/src/cli.rs`.
- The original-material scripted run is now proven with the existing exported asset tree plus 1.8 Paks. Its held-frame capture is `verification/weapons-original/held-1280x720.png`; it shows the original player, AR mesh, three recovered firearm slots, original item icons, animations and weapon/ammo data.
- The recovered ranged reticle is now wired end-to-end. `HUD_Xhair_2x10_line.png` is the original 10x10 `B_Ranged_Generic.ReticleImage`, rendered through `HudInputs -> HudWorld -> ReticleHud`; a verified capture is `verification/weapons-reticle/held-1280x720.png`.
- The sniper’s exact cooked `HUD_SniperCrossHairs_v1_Athena` texture is decoded from the Paks and follows the recovered ADS timeline. `verification/sniper-scope-composited/sniper-scope-1280x720.png` verifies its 15-FOV ADS transition and compositing over the zoomed scene. The exact UE post-process bytecode (vignette, refraction, and fringe) remains a fidelity gap, but the runtime no longer displays its SceneColor sampling area as black.
- No original game installation files were changed. The 1.8 package root for testing is `C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame\Content\Paks`.

## Blocker encountered while validating weapon rendering

The release runtime reads the 1.8 Paks successfully (5 Paks / 238,484 entries), but cannot start from Paks alone because its required exported support files are absent:

`T_Grasslands_D.png`, `dome_optimized.bin`, and `sunmoon.bin`.

It also reports that no `asset-manifest.json` is available to let `fn-export` generate those files. Do not misdiagnose this as a weapon model failure. The old prototype has assets under `C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\assets` and a manifest under `C:\Users\Brady\Desktop\Fortnite Rust\FortniteRuntime\tools\asset-manifest.json`; first establish a supported export/copy route or point the runtime at a complete asset directory, then capture the result.

## Weapon models: exact current state

- `crates/fn-app/src/scene.rs` has the original firearm meshes, original rig clips, right-hand attachment, and `Scene::install_original_guns`.
- `crates/fn-app/src/gun_orig.rs` is the original `WeaponMat_Base` material path using `fn-matshader`.
- The normal app now attempts the original `WeaponMat_Base` path automatically whenever packages are available. It only uses the Prop-shaded fallback under `--standin-gun-material` or a scripted explicit request. The original-material path has succeeded in the scripted run; capture an interactive run before declaring it fully parity-verified.
- The load point is `crates/fn-app/src/app.rs` around `install_original_guns`. Once a complete asset source exists, run `builtin:weapons-assumed` with `--original-gun-material`, inspect its held-frame capture, and only then make the proven original-material path the default.

## Reticle and sniper scope: exact current state

- Native recovered ADS data is decoded in `crates/fn-pose/src/loadout_ads.rs` and driven by `crates/fn-app/src/ads.rs`.
- The sniper profile recovers the original `HUD_SniperCrossHairs_v1_Athena` texture, `ScopeMesh1P`, timeline outputs, 15 FOV, and `DownSightPostProcessAmount`.
- `ads.rs` exposes `Frame { scope_amount, scope_mesh, reticle_hidden }`, but its module header accurately says the post-process material and firearm reticle are not yet rendered.
- HUD data flows: `fn-app::drive_arms` -> `fn_hud_adapter::HudInputs` -> `fn_ui::screens::reticle_state::HudWorld` -> `fn_ui::screens::hud::Hud`.
- The regular firearm reticle is complete in `crates/fn-ui/src/screens/reticle.rs`, `fn-hud-adapter`, and `fn-app::drive_arms`. Do not replace it with custom artwork. Next, add a typed scope-overlay field and render the recovered original scope through the renderer's post-process path. Hide the normal firearm reticle when `Frame::reticle_hidden` is true.

## Suggested test sequence

1. Fix/export the three missing support files without modifying the original Paks.
2. `cargo run --offline --release -p fn-app -- --startup-capture --paks <Paks> --out verification/startup` and inspect picker capture / timeline.
3. `cargo run --offline --release -p fn-app -- --script builtin:weapons-assumed --original-gun-material --paks <Paks> --out verification/weapons-original`; inspect the held AR, shotgun, and sniper captures.
4. Add a scripted sniper ADS capture. Verify normal reticle visible for AR/shotgun and hidden only after the recovered sniper scope timeline says so.
