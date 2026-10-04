# fn-game

Game-logic facade of the Fortnite 1.8 runtime: `GameSim` (movement + animation graph + camera + combat), the fixed 30 Hz tick driver, input-device mapping,
focus/cursor policy. Pure Rust, no OS / GPU / audio dependency (`unsafe` forbidden).

## Simulation clock: `SimClock` / `TickDriver` (old `src/sim_tick.rs`, "Fixed 30 Hz tick")

The old exe simulates gameplay on a fixed `1/SIM_HZ` step (`SIM_HZ = 30`, override `FNRT_SIM_HZ=<hz>`, `0` = once per rendered frame with the real frame time) and
draws the interpolation of the last two ticks. 1.8 itself has a 30 Hz *server* tick cap and per-frame client prediction (see the old README, evidence tools
`tick-name-scan.cjs`, `tick-rate-evidence.cjs`); the 30 Hz lock of this single-player build is the user's explicit direction. What `fn-app` calls:

| layer | what it does |
|---|---|
| `SimClock` (= `sim_tick::TickClock`) | accumulator: `advance(frame_dt) -> Advance { ticks, step, alpha, dropped }`. `floor(acc/h)` ticks of exactly `h = 1/hz`, at most `MAX_TICKS_PER_FRAME = 5` per frame (the rest of a hitch is dropped, the fraction kept), `alpha = acc/h`. hz 0 = one tick of the real `dt`, alpha 1. |
| `GameSim::begin_frame / tick / snapshot / apply_view` | the old `App::update` (per frame) and `App::sim_tick` (per tick) as methods. `GameSim::step(inp, dt)` = per-frame mode (one tick, no interpolation): what the harness and the movement logs use. |
| `TickDriver` | `SimClock` + `GameSim` + latched jump / attack taps + the snapshot pair. One call per rendered frame. |

Per FRAME (any fps): input polling, mouse / stick look (camera yaw / pitch are input), HUD, harvestable shake decay (`world.update`), crouch / sprint latches, the render.
Per TICK: swing montage + TriggerAbility hit trace, `Controller::step` (the CMC port), jump / fall stage, animation-graph variables (state machines, blend weights,
montage clocks), camera smoothing (pivot lag, offsets, feeler traces: `trace_interval` therefore counts ticks) and, through your hook, the bone-pose evaluation.
Input is not lost between ticks: jump and attack go through `Latched` (down now OR pressed since the last tick). Interpolated: pawn position / floor height, visible
facing (shortest arc), the camera smoothing state (`CamState`) and every bone pose (translation lerp, rotation slerp); a jump of more than 3 m between two ticks
(respawn / R) is shown outright (`TickDriver::snap`). The camera VIEW is composed every frame (`Camera::compose`) from the interpolated state, the interpolated
feet and the CURRENT yaw / pitch, hard-clipped against the main 8 cm probe so a mouse move between two ticks can never put the eye inside geometry.

### Wiring it into `fn-app` (do not duplicate any of this in the app)

```rust
// startup (instead of `GameSim::new()` + calling `sim.step_real(..)` every frame):
let mut driver = TickDriver::from_env(GameSim::with_tuning(Tuning::from_env()));   // FNRT_SIM_HZ: unset = 30, 0 = per frame
// every rendered frame, after polling keyboard / mouse / pad into a FrameInput (axes merged, keys raw, `yaw_delta`/`pitch_delta` already * dt):
let rep = driver.frame_real(&input, raw_frame_seconds, &mut |sim| {
    // called after EVERY tick: evaluate the pose tree once per skinned model (cheap enough at 30 Hz, this is what gets interpolated)
    models.iter().map(|m| m.eval_pose(&sim.pose_tree(), &clips)).collect()
});
// draw: `driver.sim` holds the INTERPOLATED state (x / y / z = pawn eye, cam_view); bones from `driver.drawn()`:
let poses = driver.drawn().map(|s| &s.poses);            // None before the first tick / after a teleport: evaluate `sim.pose_tree()` yourself
let facing = driver.drawn_facing();                       // interpolated visible yaw for the model matrix
// selection / respawn / pause:
//   input.selection_changed = true on the frame the hotbar slot changes (drops the outgoing swing + equip + latched attack); input.non_melee = !pickaxe_selected;
//   input.equip = true on the frame the pickaxe slot becomes selected (and once at gameplay start); input.reset = R key (driver snaps the history);
//   pause menu open: do not call `frame` at all (the simulation is paused, no tick, no accumulator growth).
// rep.ticks() / rep.alpha() / driver.ticks_total are for the frame-time overlay and `--time-test`-style audits.
```
`frame_real` clamps `raw_dt` to `time_audit::MAX_DELTA_TIME` (0.1 s = 3 ticks) first; `frame(&input, dt, ..)` takes an exact dt (scripts, tests).
Everything else the old `App::update` did per frame (HUD tick, FPS meter, `HudFeedback::tick` of `fn-hud-adapter`, window / cursor policy in `host`) stays in the app.

`GameSim::new()` ends with `settle_anim()` (1.5 s of standing idle through the graph, the old `App::settle_anim`), so a render-only path that never steps the sim
draws the idle pose of the game, not a never-ticked graph.

### Determinism
`fn-sim tick-determinism` (and `fn_sim::scenarios::tick_determinism`) runs one per-tick input script (run-up, sprint, two jumps, crouch, yaw sweep, swing) at 30 / 60 /
144 / 240 fps display rate and compares the per-tick simulation state bit for bit (pawn, floor height, graph state, camera smoothing state); `tick_driver` tests pin the
same plus latch, clamp, teleport and pose-blend behaviour. The old exe's per-frame logs (`FNRT_SIM_HZ=0`) and fixed-30 Hz logs are goldens in
`testdata/old-logs` and `testdata/old-logs-30hz` (`fn-sim` parity tests, tolerance 0).

## Other modules
`sim` (`GameSim`, `FrameInput`, `TickInput`), `inputdev` (wheel pulses, pad mapping), `host` (focus / cursor / key gating), `jump_air`, `jump_feel` (`--jump-feel-test`),
`time_audit` (`MAX_DELTA_TIME`, `clamp_frame_dt`), `sim_tick` (ported mechanically from the old prototype; now maintained here), `tick_driver` (hand written).
