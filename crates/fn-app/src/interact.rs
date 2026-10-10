//! INTERACT (2026-10-08, shared): the Use-key state every interactable hook reads (doors, pickups, containers...). Generic on purpose: no door logic here.
//! * [`set_script_use`] / [`script_use`]: a scripted run's `key=use` (script.rs `build_key`), set once per frame by app.rs; a hook ORs it with the live key.
//! The prompt itself is `HudWorld::interaction` (`fn_ui::screens::reticle_state::InteractionPrompt`); app.rs fills it from the first hook that has a focus
//! (pickups, then doors, ...), and a hook skips the Use press when an earlier one is focused.
use std::sync::atomic::{AtomicBool, Ordering};

static SCRIPT_USE: AtomicBool = AtomicBool::new(false);
/// SCRIPT: the Use key row is held this frame (`key=use` / `key=e`).
pub fn set_script_use(down: bool) { SCRIPT_USE.store(down, Ordering::Relaxed); }
pub fn script_use() -> bool { SCRIPT_USE.load(Ordering::Relaxed) }
