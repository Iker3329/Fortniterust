//! Game logic facade of the Fortnite 1.8 runtime. Everything here is pure Rust with no OS, GPU or audio dependency: the front-end
//! (`fn-app`: winit + wgpu + audio + gamepad) feeds it a [`FrameInput`] per frame and reads the resulting pawn, camera and pose tree.
//!
//! * [`sim`]      [`GameSim`]: the old `App::update` (input -> latches -> movement -> animation graph -> camera, swing/harvest) as one `step`
//! * [`inputdev`] mouse-wheel pulses and XInput pad mapping (dead zones, sensitivity, Start latch); polling is a `PadBackend` of the app
//! * [`sim_tick`] the fixed `SIM_HZ` (30 Hz) tick: accumulator, latched taps, snapshots / interpolation, the headless `FixedSim` twin (old src/sim_tick.rs)
//! * [`tick_driver`] [`TickDriver`] / [`SimClock`]: what `fn-app` calls once per rendered frame (ticks + interpolation around a `GameSim`)
//! * [`host`]     window focus / cursor-capture / key-gating policy and raw-mouse look (old main.rs helpers)
//! * [`jump_air`] the headless twin of the controller + AdditiveJumps wiring used by the air-press regression tests
//! * [`jump_feel`] `--jump-feel-test`: the input -> screen chain of a jump at 30..240 Hz and the gravity / JumpZ experiment table
//! * [`time_audit`] `--time-test` bookkeeping and [`time_audit::MAX_DELTA_TIME`], the one global frame-time clamp (0.1 s)
//! * re-exports of the logic crates: [`movement`], [`camera`], [`props`], [`combat`], [`animgraph`], [`settings`]
//!
//! Platform seams (documented in `fn_settings::platform`): `PlatformHost` (display modes, vsync, vblank wait, cursor capture, focus),
//! `VideoHooks`/`InputHooks`/`AudioHooks` (renderer / input / audio sinks of the settings apply layer) and [`InputSource`] (per-frame input).
pub mod ads_view; // ADS HOOK (fn-game): hold / aim-down-sights camera blend + targeting speed limit
pub mod audio_events; // AUDIO HOOK (fn-game): renderer-free sound-moment detection (see fn-app README "Audio hooks")
pub mod host;
pub mod inputdev;
pub mod jump_air;
pub mod jump_feel;
pub mod sim;
pub mod sim_tick;
pub mod tick_driver;
pub mod time_audit;

pub use fn_animgraph as animgraph;
pub use fn_camera as camera;
pub use fn_combat as combat;
pub use fn_movement as movement;
pub use fn_props as props;
pub use fn_settings as settings;
pub use fn_settings::version;
pub use sim::*;
pub use sim_tick::{Advance, Latched, Pair, Snapshot, TickClock, MAX_TICKS_PER_FRAME, SIM_HZ};
pub use tick_driver::{FrameReport, Poses, SimClock, TickDriver};