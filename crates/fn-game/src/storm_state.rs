//! STORM HOOK (fn-game): the state of the BR safe zone / storm as the visuals read it: zone centre, radius and the visual fade-in.
//!
//! STATIC ZONE FOR NOW: nothing shrinks, no phases advance, no damage is dealt. This is only what `SafeZoneIndicator` / `MPC_SafeZoneClouds` need to draw one zone, plus the
//! F2 command `storm <radius_m | on | off | reset>` (fn-console) to move it around for testing.
//!
//! RECOVERED (Athena data + exe, see `fn_render::storm` for how the visuals consume it):
//! * the first phase is `Default.SafeZone.00` of `AthenaGameData`: radius 150 000 cm (1.5 km; wait 0 s, shrink 0 s: the zone simply exists at match start); the next ones are
//!   80 000 / 40 000 / 20 000 / 10 000 / 5 000 / 2 500 / 1 250 / 625 cm (see [`PHASE_RADII_CM`]).
//! * the visual fade: `SafeZoneIndicator.ReceiveBeginPlay` starts the `StormFadeIn` timeline (25 s, curve 0 -> 1 linear) which drives `MPC_SafeZoneClouds.MasterOpacity`.
//! INFERRED: the first phase is centred on the `DefaultMapInfo_C` actor of `Athena_Terrain` (UE world (32224, -25744, 1312) cm). `AFortAthenaMapInfo::PickSafeZoneLocation`
//! (exe `1403fea80`) returns its root location when the pick radius is not positive, which is the case of a first phase without a previous zone; the real server code that
//! calls it is not in the client exe, so this is the best reading of the data, not a recovered fact.

/// `Default.SafeZone.NN.Radius` of `AthenaGameData`, cm (phase 0 .. 8; phase 9 has radius 0).
pub const PHASE_RADII_CM: [f32; 9] = [150_000.0, 80_000.0, 40_000.0, 20_000.0, 10_000.0, 5_000.0, 2_500.0, 1_250.0, 625.0];
/// UE world position (cm) of the first phase's centre (see the module docs: INFERRED).
pub const FIRST_PHASE_CENTER_CM: [f32; 3] = [32224.0, -25744.0, 1312.0];
/// Length (s) of the `StormFadeIn` timeline of the indicator Blueprint.
pub const FADE_IN_SECONDS: f32 = 25.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StormState {
    /// The indicator exists (BeginPlay ran). Off = nothing is drawn.
    pub enabled: bool,
    /// Zone centre, UE world cm.
    pub center_cm: [f32; 3],
    /// Zone radius, cm.
    pub radius_cm: f32,
    /// Time (s) on the `StormFadeIn` timeline.
    pub fade_time: f32,
    /// F2 `storm here`: the front-end moves the centre to the pawn on its next frame (it owns the Unreal -> baseplate mapping) and clears the flag.
    pub here_requested: bool,
}

impl Default for StormState {
    /// The first phase, fully faded in (a static viewer starts with the storm in place; `on` after `off` replays the fade).
    fn default() -> Self { StormState { enabled: true, center_cm: FIRST_PHASE_CENTER_CM, radius_cm: PHASE_RADII_CM[0], fade_time: FADE_IN_SECONDS, here_requested: false } }
}

impl StormState {
    /// `MPC_SafeZoneClouds.MasterOpacity`: the `StormFadeIn` curve (linear 0 -> 1 over 25 s) while enabled, 0 when the indicator does not exist.
    pub fn master_opacity(&self) -> f32 { if self.enabled { (self.fade_time / FADE_IN_SECONDS).clamp(0.0, 1.0) } else { 0.0 } }
    /// Advances the fade timeline (call once per rendered frame).
    pub fn tick(&mut self, dt: f32) { if self.enabled { self.fade_time = (self.fade_time + dt.max(0.0)).min(FADE_IN_SECONDS); } }
    /// Spawns the indicator: the `StormFadeIn` timeline starts from 0 (`ReceiveBeginPlay`). No effect when it already exists.
    pub fn turn_on(&mut self) { if !self.enabled { self.enabled = true; self.fade_time = 0.0; } }
    pub fn turn_off(&mut self) { self.enabled = false; }
    pub fn set_radius_m(&mut self, metres: f32) { self.radius_cm = (metres * 100.0).max(1.0); }
    pub fn radius_m(&self) -> f32 { self.radius_cm * 0.01 }
    /// Back to the first phase (radius and centre); the enabled / fade state is kept.
    pub fn reset_zone(&mut self) { self.center_cm = FIRST_PHASE_CENTER_CM; self.radius_cm = PHASE_RADII_CM[0]; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn the_default_is_the_first_phase_fully_faded_in() {
        let s = StormState::default();
        assert_eq!((s.radius_cm, s.master_opacity(), s.enabled), (150_000.0, 1.0, true));
        assert_eq!(s.center_cm, [32224.0, -25744.0, 1312.0]);
    }
    #[test] fn turning_on_replays_the_25_second_linear_fade() {
        let mut s = StormState::default();
        s.turn_off(); assert_eq!(s.master_opacity(), 0.0);
        s.tick(5.0); assert_eq!(s.master_opacity(), 0.0, "a disabled indicator does not advance its timeline");
        s.turn_on(); assert_eq!(s.master_opacity(), 0.0);
        s.tick(12.5); assert!((s.master_opacity() - 0.5).abs() < 1e-6);
        s.tick(100.0); assert_eq!(s.master_opacity(), 1.0);
        s.turn_on(); assert_eq!(s.master_opacity(), 1.0, "already on: the timeline is not restarted");
    }
    #[test] fn the_radius_is_set_in_metres_and_reset_restores_the_phase_zero_zone() {
        let mut s = StormState::default();
        s.set_radius_m(800.0); assert_eq!(s.radius_cm, 80_000.0); assert_eq!(s.radius_m(), 800.0);
        s.center_cm = [1.0, 2.0, 3.0]; s.reset_zone(); assert_eq!((s.radius_cm, s.center_cm), (150_000.0, FIRST_PHASE_CENTER_CM));
    }
}
