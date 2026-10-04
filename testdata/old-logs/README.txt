Golden logs written by the OLD prototype executable (FortniteRuntime.exe, OpenGL/Win32) for the scripted test modes; the fn-sim parity
tests (crates/fn-sim/tests/parity.rs) regenerate them headlessly from the new crates and require identical lines (tolerance 0).

Two sets:
  testdata\old-logs\        PER-FRAME mode (FNRT_SIM_HZ=0): one simulation tick per rendered frame with the real frame time. movement-test, jump-air-test,
                            jump-feel-test, sim-tick-test, camera-prop-test, camera-video, face-test never depend on the tick rate (headless / own stepping).
  testdata\old-logs-30hz\   the exe's DEFAULT mode (FNRT_SIM_HZ unset = the fixed 30 Hz tick with render interpolation): camera-test, pickaxe-test, dir-test only
                            (the scripted frames are 1/60 s, so one tick every other frame and bone poses blended by alpha).

Provenance (last regeneration 2026-10-03 ~09:15-09:50, `fn-parity regen-old-logs`, formerly tools\regen-old-logs.ps1)
  Old exe built in release mode from a snapshot copy of ..\FortniteRuntime\src + Cargo.toml/Cargo.lock taken 2026-10-03 09:13 (junctions to the old `reports`, `assets`
  and `verification` folders, CARGO_TARGET_DIR %TEMP%\cargo-old-resync2). The fn-* crates were ported from the same snapshot with the former tools\port-from-old.ps1 (FN_OLD_ROOT
  pointing at it), so source and goldens are one consistent revision. That revision includes: engine-exact clip sampling (every pose / bone log changed),
  the fixed 30 Hz tick, the 1.8 verification fixes (pickaxe damage 50, reach 2.88 m, radius 0.2 m, tree 600 / rock 450 HP, step-function directional speed curve),
  the reticle / harvest popup state, `App::settle_anim` (1.5 s of idle through the graph before the first frame).
  movement-test/, jump-air-test/, jump-feel-test/, sim-tick-test/   headless modes: `--movement-test`, `--jump-air-test`, `--jump-feel-test`, `--sim-tick-test`
  camera-test/, camera-prop-test/, camera-video/    `--camera-test`, `--camera-prop-test [--no-prop-collision]`, `--camera-video` (window mode)
  pickaxe-test/, dir-test/                          `--pickaxe-test`, `--dir-test`
  face-test/                                        `--face-test` and `--face-test --legacy` (face-test.txt, face-test-legacy.txt: the numbers; the
                                                    close-up PNGs of the old run need the GL renderer and are not part of the parity)
  KNOWN: the old exe's own `--movement-test` reports 1 FAILED check ("run strafe 90 deg: 328.0 cm/s (expected 297.2)"): its series still expects the old linearly
  interpolated strafe speed (410 * 0.725) after the verification audit made DirectionalSpeedMultiplierCurve a step curve (strafe 0.8 -> 328 cm/s, the right value).
  The Rust port is verbatim, prints the same line, and the parity test pins that this is the ONLY failure.
  Not here: `--time-test` (real wall-clock audit of the windowed loop, non-reproducible by design; fn-sim has a virtual-clock twin with unit
  tests) and `--anim-test` (determinism-tested only).
  Source edits made to the OLD tree after the snapshot (main.rs: drop a swing on hotbar selection change / ignore attack for non-melee slots; hud.rs `melee_equipped`;
  loading.rs) do not affect any scripted mode here (selection never changes in them); the selection rule is ported (`FrameInput::selection_changed` / `non_melee`).

Window-mode caveats found while regenerating (tools\regen-old-logs.ps1 does these):
  * The window-mode exe reads the real keyboard while its window is foreground. Every mode is run TWICE and kept only when both runs are byte-identical.
  * `dir-frames.txt` is APPENDED to by the old exe (it is not deleted at start like pickaxe-frames.txt): a clean verification folder per run.
  * `powershell -File` passes `-Only a,b` as ONE string: the script splits it.

To refresh:  powershell -File tools\regen-old-logs.ps1 -Build          (snapshots ..\FortniteRuntime when %TEMP%\fn-snap-resync2 does not exist, builds, runs every mode twice
for both rates, writes both golden sets; clears FNRT_* variables: FNRT_GRAVITY_SCALE / FNRT_JUMPZ change the physics)  then
  $env:FN_OLD_ROOT=<snapshot>; powershell -File tools\port-from-old.ps1   and   cargo test -p fn-sim --test parity   and   cargo test -p fn-pose
The old exe is the reference; do not edit these files by hand.
