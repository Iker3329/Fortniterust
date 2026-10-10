//! Building editing (owner: editing agent). Athena 1.8, data from the cooked edit patterns / piece BPs and the 1.8 exe.
//!
//! * [`pattern`] - tile patterns, rotate / mirror / equals and the pattern -> class matcher (`vfunc_93`/`vfunc_94`).
//! * [`data`] - [`data::EditDb`]: player building classes, their `EditModePatternData`, tile grids, edit-tool assets.
//! * [`session`] - [`session::EditSession`]: the edit-mode state machine (selection, drag, reset, confirm, cancel).
//! * [`EditController`] - what the input layer binds keys to.
//!
//! Key actions (1.8 input action names from `AFortPlayerController::SetupInputComponent` @14084a570; the default key
//! for each lives in the encrypted DefaultInput.ini, so binding them is the input agent's job):
//! | action | event | call |
//! |---|---|---|
//! | `PerformBuildingEditInteraction` | pressed | [`EditController::edit_begin`] |
//! | fire (edit tool) | pressed / released | [`EditController::edit_press`] / [`EditController::edit_release`] |
//! | `BuildingEditReset` | pressed | [`EditController::edit_reset`] |
//! | `CompleteBuildingEditInteraction` | pressed | [`EditController::edit_confirm`] |
//! | `GamepadEditBuilding` (edit mode) | released | [`EditController::edit_confirm`] |
//! | edit tool unequipped | - | [`EditController::edit_cancel`] |

#[path = "edit_data.rs"]
pub mod data;
#[path = "edit_pattern.rs"]
pub mod pattern;
#[path = "edit_session.rs"]
pub mod session;

use glam::Vec3;

use crate::world::{BuildingWorld, PieceId};
pub use data::EditDb;
pub use session::{BeginError, EditBlockers, EditEvent, EditInputMode, EditSession, EditTarget, EditVisuals};

/// Owns the current edit (if any) and exposes the bindable actions.
#[derive(Default)]
pub struct EditController {
    pub session: Option<EditSession>,
    /// Input state for the edit permission (`FUN_140816020`); default = keyboard / mouse, which always passes.
    pub input_mode: EditInputMode,
    events: Vec<EditEvent>,
}

impl EditController {
    pub fn new() -> Self { Self::default() }
    pub fn is_editing(&self) -> bool { self.session.is_some() }
    pub fn editing_piece(&self) -> Option<PieceId> { self.session.as_ref().map(|s| s.piece) }

    /// Edit key on the targeted piece. If already editing, the same key does nothing here (in 1.8 the edit-mode input
    /// component is on top while editing; confirm is `CompleteBuildingEditInteraction`).
    pub fn edit_begin(&mut self, db: &EditDb, world: &BuildingWorld, target: Option<PieceId>, pawn_location: Vec3) -> Result<(), BeginError> {
        self.edit_begin_checked(db, world, target, pawn_location, EditBlockers::default())
    }

    /// [`edit_begin`](Self::edit_begin) with the target's attachment / pending-structural state (`ABuildingSMActor::vfunc_330`
    /// @140494860) and [`input_mode`](Self::input_mode) (`vfunc_331` -> `FUN_140816020`).
    pub fn edit_begin_checked(&mut self, db: &EditDb, world: &BuildingWorld, target: Option<PieceId>, pawn_location: Vec3, blockers: EditBlockers) -> Result<(), BeginError> {
        if self.session.is_some() { return Ok(()); }
        let id = target.ok_or(BeginError::NoSuchPiece)?;
        let mut s = EditSession::begin_checked(db, world, id, pawn_location, self.input_mode, blockers)?;
        self.events.extend(s.drain_events());
        self.session = Some(s);
        Ok(())
    }

    pub fn edit_press(&mut self) { if let Some(s) = &mut self.session { s.begin_action(); } }
    pub fn edit_release(&mut self) {
        if let Some(s) = &mut self.session { s.end_action(); self.events.extend(s.drain_events()); }
    }

    /// Hovered target this frame (normally from [`EditController::tick`]'s picking).
    pub fn edit_hover(&mut self, target: Option<EditTarget>) {
        if let Some(s) = &mut self.session { s.update_hover(target); self.events.extend(s.drain_events()); }
    }

    /// Select (click) one tile without a drag.
    pub fn edit_select_tile(&mut self, index: usize) {
        if let Some(s) = &mut self.session { s.click(EditTarget::Tile(index)); self.events.extend(s.drain_events()); }
    }

    pub fn edit_reset(&mut self) {
        if let Some(s) = &mut self.session { s.reset(); self.events.extend(s.drain_events()); }
    }

    /// Confirm: applies the edit to the world when the pattern is valid and differs. Returns whether it was applied.
    pub fn edit_confirm(&mut self, world: &mut BuildingWorld) -> bool {
        let Some(s) = self.session.take() else { return false };
        let (applied, ev) = s.confirm(world);
        self.events.extend(ev);
        applied
    }

    pub fn edit_cancel(&mut self) {
        if let Some(s) = self.session.take() { self.events.extend(s.cancel()); }
    }

    /// Per-frame without time / death (kept for callers that have neither): [`tick_frame`](Self::tick_frame) with dt 0, alive.
    pub fn tick(&mut self, world: &BuildingWorld, pawn_location: Vec3, camera: Vec3, view_dir: Vec3) {
        self.tick_frame(world, pawn_location, false, camera, view_dir, 0.0);
    }

    /// Per-frame (`AFortPlayerController::PlayerTick` -> `FUN_14084f8a0` @14084f8a0): cancels (`LocalExitEditMode(true)`) when
    /// the pawn is out of range of the piece (`FUN_140832610`) or dead (`FUN_1407eae70`: health <= 0 or dying), or the piece is
    /// gone; else picks the hovered tile from the view ray (`EditModeTrace`) and runs the support tick (`vfunc_76`: hover, drag,
    /// match-flash timeline, roof tile easing) with `dt`.
    pub fn tick_frame(&mut self, world: &BuildingWorld, pawn_location: Vec3, pawn_dead: bool, camera: Vec3, view_dir: Vec3, dt: f32) {
        let Some(s) = &mut self.session else { return };
        if pawn_dead || !s.still_valid(world, pawn_location) {
            self.edit_cancel();
            return;
        }
        let t = s.pick(world, camera, view_dir, pawn_location);
        s.update_hover(t);
        s.tick(dt);
        self.events.extend(s.drain_events());
    }

    pub fn visuals(&self, world: &BuildingWorld) -> Option<EditVisuals> { self.session.as_ref().and_then(|s| s.visuals(world)) }
    pub fn drain_events(&mut self) -> Vec<EditEvent> { std::mem::take(&mut self.events) }
}

#[cfg(test)]
mod tests;
