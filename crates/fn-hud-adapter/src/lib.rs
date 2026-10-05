//! fn-hud-adapter: the HUD wiring contract between the game and the UI.
//!
//! * [`fn_ui::screens::hud::HudState`] is a plain struct (health, shield, selected slot, items with rarity / icon / count, resources, kills,
//!   players left, compass yaw, minimap position and rotation, Show FPS and frame stats, element toggles, loading progress).
//! * [`hud_state`] produces it from what the game owns: `fn_game::GameSim` (pawn pose, harvested resources), `fn_settings::Settings` (Show FPS,
//!   the `[HUD]` element toggles) and a [`HudInputs`] for the parts the game logic does not model yet (vitals, inventory, match info, loading).
//! * `fn-app` only passes it through: `hud.set_state(&hud_state(&sim, &settings, &inputs)); hud.tick(dt); hud.present(ctx, w, h, &mut list)`.
//!
//! The adapter has no state of its own except [`FrameClock`] (a smoothed fps meter); it never reads a clock, a window or a file.
pub mod feedback;
pub use feedback::{AggregateView, HarvestPopups, HudFeedback, HudTagVisibility, PackView, PopupView, PotentialView, ReticleState};
use fn_game::{settings::{FpsMeter, Settings}, GameSim, HALF};
use fn_ui::screens::hud::{HudFrameStats, HudItem, HudLoading, HudMinimap, HudState, HudVisibility};

/// What the HUD shows that `fn-game` does not model yet. `Default` = a fresh spawn: full health, no shield, the pickaxe in slot 1, no kills, no
/// match data (players left shows the widget's designer default), nothing in the backpack, no frame numbers, no loading screen.
#[derive(Clone, Debug, PartialEq)]
pub struct HudInputs {
    /// (health, shield), 0..=100 each.
    pub vitals: (u16, u16),
    /// Quickbar contents; None entries are empty slots. `None` for the whole value = only the pickaxe.
    pub items: Option<[Option<HudItem>; 6]>,
    pub selected: usize,
    pub kills: u32,
    pub players_left: Option<u32>,
    /// (items, max) shown next to the backpack icon; None = the widget default (empty).
    pub backpack: Option<(u32, u32)>,
    /// Metal count (the sandbox world only knows wood and stone).
    pub metal: u32,
    pub frame: HudFrameStats,
    pub loading: Option<HudLoading>,
    /// The held firearm's AthenaEquippedItemWidget numbers (None = nothing ranged is held).
    pub ammo: Option<fn_ui::screens::equipped_item::EquippedAmmo>,
    /// `GameplayCue.Athena.Equipping` as the use-progress ring (`AthenaEquipProgress`) shows it while a consumable is being used.
    pub equip: Option<fn_ui::screens::reticle_state::EquipProgress>,
    /// WIDGETS3: the running cooldown of each primary quickbar slot's item (`QuickbarSlotCooldown`).
    pub slot_cooldowns: [Option<fn_ui::screens::slot_cooldown::SlotCooldown>; 6],
    /// Original native ranged reticle texture selected by the held weapon. None for melee/consumables.
    pub firearm_reticle: Option<fn_ui::screens::reticle_state::FirearmReticle>,
    /// A ranged hit was confirmed by the local hit-registration result this frame. This is an event, not persistent state.
    pub firearm_hit: bool,
    /// Recovered sniper post-process input, only present while its native ADS timeline has non-zero weight.
    pub scope_overlay: Option<fn_ui::screens::reticle_state::ScopeOverlay>,
}
impl Default for HudInputs {
    fn default() -> Self { HudInputs { vitals: (100, 0), items: None, selected: 0, kills: 0, players_left: None, backpack: None, metal: 0, frame: HudFrameStats::default(), loading: None, ammo: None, equip: None, slot_cooldowns: [None; 6], firearm_reticle: None, firearm_hit: false, scope_overlay: None } }
}

/// Smoothed frames-per-second counter that feeds `HudInputs::frame` (the old runtime's `FpsMeter`).
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameClock { meter: FpsMeter, last_dt: f32 }
impl FrameClock {
    pub fn tick(&mut self, dt: f32) { self.meter.tick(dt); if dt.is_finite() && dt > 0. { self.last_dt = dt; } }
    pub fn stats(&self) -> HudFrameStats { HudFrameStats { fps: self.meter.fps, frame_ms: self.last_dt * 1000. } }
}

/// Is the `[HUD]` element switched on? Missing key = on; `False` / `0` / `Off` (any case) = off (the Settings screen's rule).
pub fn hud_flag(settings: &Settings, tag: &str) -> bool {
    match settings.extra("HUD", tag) { Some(v) => !(v.eq_ignore_ascii_case("false") || v == "0" || v.eq_ignore_ascii_case("off")), None => true }
}
/// The element toggles from the settings.
pub fn hud_visibility(settings: &Settings) -> HudVisibility { HudVisibility::from_tags(|tag| hud_flag(settings, tag)) }

/// World position -> minimap UV (north up). The map texture is taken to cover the whole square baseplate (`2 * HALF` metres, APPROXIMATE: the 1.8
/// island map is not in the sandbox), u grows with +x and v with +z (south), clamped to the texture.
pub fn minimap_uv(x: f32, z: f32) -> (f32, f32) {
    let u = (x + HALF) / (2. * HALF);
    let v = (z + HALF) / (2. * HALF);
    (if u.is_finite() { u.clamp(0., 1.) } else { 0.5 }, if v.is_finite() { v.clamp(0., 1.) } else { 0.5 })
}

/// The HUD state of this frame.
pub fn hud_state(sim: &GameSim, settings: &Settings, inputs: &HudInputs) -> HudState {
    let mut items: [Option<HudItem>; 6] = inputs.items.clone().unwrap_or_else(|| { let mut i: [Option<HudItem>; 6] = Default::default(); i[0] = Some(HudItem::pickaxe()); i });
    if items.iter().all(|i| i.is_none()) { items[0] = Some(HudItem::pickaxe()); } // the pickaxe cannot be dropped
    let mut s = HudState {
        health: inputs.vitals.0, shield: inputs.vitals.1,
        selected: inputs.selected, items,
        resources: [sim.world.wood, sim.world.stone, inputs.metal],
        backpack: inputs.backpack.unwrap_or(HudState::default().backpack),
        kills: inputs.kills, players_left: inputs.players_left,
        compass_yaw_deg: sim.yaw,
        minimap: HudMinimap { pos: minimap_uv(sim.x, sim.z), rotation_deg: sim.yaw },
        show_fps: settings.video.show_fps, frame: inputs.frame,
        visible: hud_visibility(settings),
        loading: inputs.loading.clone(),
        // `HudState::world` (reticle / harvest hits / Target Info / interaction / equip progress, `fn_ui::screens::reticle_state`): filled by the world-feedback
        // producer, empty until then (the `..` keeps this literal compiling when the contract grows).
        ..HudState::default()
    };
    s.world.ammo = inputs.ammo.clone();
    s.world.equip = inputs.equip;
    s.world.slot_cooldowns = inputs.slot_cooldowns; // WIDGETS3
    s.world.firearm_reticle = inputs.firearm_reticle.clone();
    s.world.firearm_hit = inputs.firearm_hit;
    s.world.scope_overlay = inputs.scope_overlay.clone();
    // an empty or missing selection falls back to the first filled slot (HudState::select never lands on an empty one)
    if s.items.get(s.selected).map_or(true, |i| i.is_none()) { s.selected = s.items.iter().position(|i| i.is_some()).unwrap_or(0); }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_game::settings::Settings;
    use fn_ui::draw::RenderList;
    use fn_ui::screens::hud::Hud;

    #[test] fn the_held_firearms_ammo_reaches_the_equipped_item_widget_state() {
        let sim = GameSim::new(); let ammo = fn_ui::screens::equipped_item::EquippedAmmo { magazine: 30, remaining: 150, icon: "T-Items-AmmoMedium.png".into() };
        let st = hud_state(&sim, &Settings::default(), &HudInputs { ammo: Some(ammo.clone()), ..Default::default() }); assert_eq!(st.world.ammo, Some(ammo));
        assert!(hud_state(&sim, &Settings::default(), &HudInputs::default()).world.ammo.is_none());
    }
    #[test] fn a_fresh_spawn_is_the_default_hud() {
        let sim = GameSim::new();
        let st = hud_state(&sim, &Settings::default(), &HudInputs::default());
        assert_eq!((st.health, st.shield, st.selected), (100, 0, 0));
        assert_eq!(st.occupied(), [true, false, false, false, false, false], "just the pickaxe");
        assert_eq!(st.items[0], Some(HudItem::pickaxe()));
        assert_eq!(st.resources, [0, 0, 0]);
        assert!(st.visible == HudVisibility::default() && !st.show_fps && st.loading.is_none());
        assert_eq!(st.compass_yaw_deg, sim.yaw);
        assert_eq!(st.minimap.rotation_deg, sim.yaw);
        assert_eq!(st.minimap.pos, (0.5, 0.5 + 12. / 512.), "the spawn is near the middle of the baseplate (z = 12)");
    }
    #[test] fn game_state_flows_into_the_hud() {
        let mut sim = GameSim::new();
        sim.world.wood = 123; sim.world.stone = 45; sim.yaw = 270.; sim.x = -256.; sim.z = 256.;
        let st = hud_state(&sim, &Settings::default(), &HudInputs { metal: 9, kills: 3, players_left: Some(57), ..Default::default() });
        assert_eq!(st.resources, [123, 45, 9]);
        assert_eq!((st.kills, st.players_left), (3, Some(57)));
        assert_eq!(st.compass_yaw_deg, 270.);
        assert_eq!(st.minimap.pos, (0., 1.), "corner of the baseplate = corner of the map");
        sim.x = f32::NAN; sim.z = 1e9;
        let st = hud_state(&sim, &Settings::default(), &HudInputs::default());
        assert_eq!(st.minimap.pos, (0.5, 1.), "NaN falls back to the centre, far away clamps");
    }
    #[test] fn settings_drive_show_fps_and_the_element_toggles() {
        let sim = GameSim::new();
        let mut s = Settings::default();
        s.set("video.show_fps", fn_game::settings::Value::Bool(true)).unwrap();
        s.set_extra("HUD", "HUD.TopRight.Minimap", "False");
        s.set_extra("HUD", "HUD.Quickbar", "off");
        s.set_extra("HUD", "HUD.TopLeft.PickupManager", "0");
        s.set_extra("HUD", "HUD.Center.Reticle", "True");
        let st = hud_state(&sim, &s, &HudInputs::default());
        assert!(st.show_fps);
        let v = st.visible;
        assert!(!v.minimap && !v.quick_bar && !v.pickup_info);
        assert!(v.reticle && v.player_health && v.target_info && v.interaction_info, "unset and True stay on");
        assert!(hud_flag(&s, "HUD.Unknown") && !hud_flag(&s, "HUD.Quickbar"));
    }
    #[test] fn tags_match_the_settings_screens_hud_tab() {
        assert_eq!(HudVisibility::TAGS.len(), 7);
        let all_off = HudVisibility::from_tags(|_| false);
        assert!(!all_off.reticle && !all_off.player_health && !all_off.pickup_info && !all_off.minimap && !all_off.quick_bar && !all_off.target_info && !all_off.interaction_info);
        for (i, tag) in HudVisibility::TAGS.iter().enumerate() {
            let one = HudVisibility::from_tags(|t| t == *tag);
            let n = [one.reticle, one.player_health, one.pickup_info, one.minimap, one.quick_bar, one.target_info, one.interaction_info];
            assert_eq!(n.iter().filter(|b| **b).count(), 1, "{tag}");
            assert!(n[i], "{tag} is toggle #{i}");
        }
    }
    #[test] fn inventory_rules() {
        let sim = GameSim::new();
        // custom items, selection on an empty slot falls back to a filled one, an emptied bar keeps the pickaxe
        let mut items: [Option<HudItem>; 6] = Default::default();
        items[2] = Some(HudItem::new(3, "T-Icon-Test.png", Some(7)));
        let st = hud_state(&sim, &Settings::default(), &HudInputs { items: Some(items), selected: 0, ..Default::default() });
        assert_eq!(st.selected, 2);
        assert_eq!(st.items[2].as_ref().unwrap().count, Some(7));
        let st = hud_state(&sim, &Settings::default(), &HudInputs { items: Some(Default::default()), selected: 4, ..Default::default() });
        assert_eq!((st.selected, st.items[0].clone()), (0, Some(HudItem::pickaxe())));
    }
    #[test] fn frame_clock_smooths_and_reports_milliseconds() {
        let mut c = FrameClock::default();
        assert_eq!(c.stats(), HudFrameStats::default());
        for _ in 0..600 { c.tick(1. / 120.); }
        let s = c.stats();
        assert!((s.fps - 120.).abs() < 1., "{}", s.fps);
        assert!((s.frame_ms - 1000. / 120.).abs() < 1e-3);
        c.tick(f32::NAN); c.tick(0.); c.tick(-1.);
        assert!(c.stats().fps.is_finite());
    }
    #[test] fn loading_passes_through_and_the_state_draws_through_fn_ui() {
        let sim = GameSim::new();
        let mut inputs = HudInputs::default();
        inputs.loading = Some(HudLoading { done: 3, total: 10, label: "Loading HUD".into(), tip: 0, fade: 0. });
        assert_eq!(hud_state(&sim, &Settings::default(), &inputs).loading.unwrap().done, 3);
        // the contract end to end (skipped without the exported UI assets): produce -> set_state -> present
        let Ok(mut ctx) = fn_ui::scene::UiContext::from_default_assets() else { eprintln!("skipping: no UI assets"); return };
        let mut hud = Hud::new();
        hud.set_state(&hud_state(&sim, &Settings::default(), &HudInputs::default()));
        hud.tick(1. / 60.);
        let mut out = RenderList::new();
        hud.present(&mut ctx, 1920., 1080., &mut out);
        assert!(!out.is_empty());
        let mut out2 = RenderList::new();
        let mut hud2 = Hud::new(); hud2.set_state(&hud_state(&sim, &Settings::default(), &HudInputs::default())); hud2.tick(1. / 60.); hud2.present(&mut ctx, 1920., 1080., &mut out2);
        assert_eq!(out.cmds, out2.cmds, "same state, same draw list");
    }
}
