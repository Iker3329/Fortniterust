//! BUILDING EDIT HOOK: app side of `fn_building::edit` (owner: editing agent).
//!
//! Holds the [`EditDb`] (loaded once from the paks) and the [`EditController`]. The build HUD / input agent binds the
//! 1.8 actions to these calls (action names in `fn_building::edit` docs; their default keys are in the encrypted
//! DefaultInput.ini, so they are bound from the settings table, not hard coded here):
//! * `PerformBuildingEditInteraction` pressed -> [`EditHook::edit_begin`] with the piece under the crosshair
//! * edit-tool fire pressed / released -> [`EditHook::edit_press`] / [`EditHook::edit_release`]
//! * `BuildingEditReset` pressed -> [`EditHook::edit_reset`]
//! * `CompleteBuildingEditInteraction` pressed (`GamepadEditBuilding` released) -> [`EditHook::edit_confirm`]
//! * edit tool unequipped / weapon switch -> [`EditHook::edit_cancel`] (1.8 cancels unless confirmed first)
//!
//! Every frame call [`EditHook::tick_frame`] with the pawn location, whether the pawn is dead, the camera ray and dt; it cancels
//! out of range (750) or on death, picks the hovered tile and advances the match flash / roof tiles. [`EditHook::visuals`] is
//! the render snapshot (tiles, stair indicators, preview class); [`EditHook::drain_sound_cues`] gives the `PlayerUIFeedback`
//! cues to play (2D, at the listener). Every pattern change (`vfunc_80` -> `LocalOnBlueprintEdit`) is forwarded to
//! `build_anim::on_edit_tile_changed` (the edit montage).

use std::path::Path;

use fn_building::edit::{BeginError, EditBlockers, EditController, EditDb, EditEvent, EditVisuals};
use fn_building::world::{BuildingWorld, PieceId};
use glam::Vec3;

pub struct EditHook {
    pub db: EditDb,
    pub ctl: EditController,
    /// Controller events not yet handed out (sounds), after the pattern changes were forwarded.
    pending: Vec<EditEvent>,
}

impl EditHook {
    /// Loads the edit data from an install (`FortniteGame` dir or its `Content/Paks`).
    pub fn load(install: &Path) -> Result<EditHook, String> {
        let set = fn_pak::PakSet::open_install(install).map_err(|e| e.to_string())?;
        Self::from_paks(&set)
    }

    pub fn from_paks(set: &fn_pak::PakSet) -> Result<EditHook, String> {
        Ok(EditHook { db: EditDb::load(set)?, ctl: EditController::new(), pending: Vec::new() })
    }

    pub fn is_editing(&self) -> bool { self.ctl.is_editing() }

    /// Moves the controller's events here; each `PatternChanged` (vfunc_80 -> `AFortPlayerController::LocalOnBlueprintEdit`
    /// @140834c70) plays the edit montage (`build_anim::on_edit_tile_changed`; it does not restart while playing).
    fn pump(&mut self) {
        for e in self.ctl.drain_events() {
            if e == EditEvent::PatternChanged { crate::build_anim::on_edit_tile_changed(); }
            self.pending.push(e);
        }
        // nobody draining the sounds: keep the most recent only
        if self.pending.len() > 256 { let n = self.pending.len() - 256; self.pending.drain(..n); }
    }

    /// `PerformBuildingEditInteraction` on `target`. The permission (`ABuildingSMActor::vfunc_330` @140494860) also needs the
    /// piece free of attachments (`vfunc_363(2)`: no trap / attachment actors exist in this port, so always false) and without a
    /// pending structural operation (`building_damage_hook::structural_pending`).
    pub fn edit_begin(&mut self, world: &BuildingWorld, target: Option<PieceId>, pawn: Vec3) -> Result<(), BeginError> {
        let blockers = EditBlockers { has_attachment: target.map_or(false, crate::trap_hook::piece_has_trap), structural_pending: target.map_or(false, crate::building_damage_hook::structural_pending) };
        let r = self.ctl.edit_begin_checked(&self.db, world, target, pawn, blockers);
        self.pump();
        r
    }
    pub fn edit_press(&mut self) { self.ctl.edit_press(); self.pump(); }
    pub fn edit_release(&mut self) { self.ctl.edit_release(); self.pump(); }
    pub fn edit_select_tile(&mut self, index: usize) { self.ctl.edit_select_tile(index); self.pump(); }
    pub fn edit_reset(&mut self) { self.ctl.edit_reset(); self.pump(); }
    /// `CompleteBuildingEditInteraction` (`FUN_140837210` @140837210): `LocalExitEditMode(false)`, then, the edit mode being
    /// over, the controller re-equips the focused quickbar slot (`FUN_14067ba90(QuickBars, focused bar, focused slot)`): the
    /// edit tool is switched off whether or not the edit applied. Here the session ends, so `BuildInput::is_editing` turns false
    /// and the focused bar's item (building piece or weapon) is back (`build_input::apply_edit_intents`).
    pub fn edit_confirm(&mut self, world: &mut BuildingWorld) -> bool { let r = self.ctl.edit_confirm(world); self.pump(); r }
    pub fn edit_cancel(&mut self) { self.ctl.edit_cancel(); self.pump(); }

    /// Per frame without death / dt (kept for older callers): see [`tick_frame`](Self::tick_frame).
    pub fn tick(&mut self, world: &BuildingWorld, pawn: Vec3, camera: Vec3, view_dir: Vec3) {
        self.tick_frame(world, pawn, false, camera, view_dir, 0.0);
    }

    /// Per frame (`FUN_14084f8a0` from `AFortPlayerController::PlayerTick`): cancel when out of range or `pawn_dead`, else hover /
    /// drag, match flash and roof tile easing over `dt`.
    pub fn tick_frame(&mut self, world: &BuildingWorld, pawn: Vec3, pawn_dead: bool, camera: Vec3, view_dir: Vec3, dt: f32) {
        self.ctl.tick_frame(world, pawn, pawn_dead, camera, view_dir, dt);
        self.pump();
    }

    pub fn visuals(&self, world: &BuildingWorld) -> Option<EditVisuals> { self.ctl.visuals(world) }

    /// Sound cue package paths (`/Game/Sounds/Fort_UI_Sounds/Gameplay/...`) to play since the last call.
    pub fn drain_sound_cues(&mut self) -> Vec<&'static str> {
        self.pump();
        std::mem::take(&mut self.pending)
            .into_iter()
            .filter_map(|e| match e { EditEvent::Sound(_, cue) => Some(cue), _ => None })
            .collect()
    }
}
