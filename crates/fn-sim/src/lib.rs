//! Deterministic headless simulation harness. Runs the game loop of `fn-game` (movement + camera + animation graph + combat) over the old
//! prototype's scripted scenarios and writes per-frame logs that the old executable (or any earlier build of this harness) can be compared
//! against frame by frame:
//!
//! * [`series`]    `--movement-test` / `--jump-air-test` text series with pass/fail checks (ported verbatim from movement_series.rs)
//! * [`scenarios`] `--pickaxe-test` / `--anim-test` / `--dir-test` / `--camera-test` / `--camera-prop-test` / `--camera-video`
//! * [`rig`]       optional pose evaluation (soldier skeleton + clips via `fn-legacy`) for bone positions and pose hashes
//! * [`compare`]   tolerance-based line diff of two logs (`fn-sim compare`)
//!
//! Determinism: no wall clock, no threads, fixed 1/60 s steps (unless a scenario says otherwise), f32 math only; two runs of the same scenario
//! produce byte-identical logs (tested).
pub mod compare;
pub mod move_trace;
pub mod face_test;
pub mod pick_script;
pub mod rig;
pub mod scenarios;
pub mod series;
pub mod time_test;
pub mod video_script;
