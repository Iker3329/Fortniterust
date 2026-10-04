//! World-feedback HUD contract: the pickaxe [`ReticleState`], the harvest `+N` [`HarvestPopups`] (ResourceAggregationWidget + PotentialResourceWidget), the pickup
//! manager's [`PackView`] (Wood / Stone / Metal counters) and the hierarchical `[HUD]` tag rule ([`HudTagVisibility`]). PLAIN DATA out, no drawing, no globals:
//! the engine is `fn_combat::reticle` (a renderer-free port of the old `src/reticle.rs` + `hud_vis.rs` + the `hud_extra.rs` count-up), a renderer (`fn-ui` /
//! `fn-ui-wgpu`) reads the views and draws textures / text from them.
//!
//! Per frame the app calls [`HudFeedback::tick`] with the sim (it drains `sim.world.events`, runs the aim-ray test, steps the animations) and draws
//! `feedback.reticle`, `feedback.popups.view()` and `feedback.packs`. Field names match `fn_ui::screens::reticle_state` (`object, resource, amount, destroyed, weakpoint`;
//! `ReticleState::on_target` == [`ReticleState::colour_building`]).
use fn_game::combat::reticle::{self as r, AggParams, PotParams, Popup, CountAnim};
use fn_game::combat::{HarvestEvent, Resource};
use fn_game::settings::Settings;
use fn_game::GameSim;
use glam::Vec3;

// ---- reticle -----------------------------------------------------------------------------------------------------------------
/// What the reticle widget draws this frame. The picture is the weapon's `ReticleImage` (`HUD_Xhair_Dot`, 58x54 texture px) centred on the screen (zero offset:
/// only ranged weapons project the aim point), white, or `ReticleBuildingColor` (255,221,0) while the aim ray finds a prop within pickaxe reach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReticleState {
    /// Draw it at all: the pickaxe (quickbar slot 0) is equipped AND the `HUD.Center.Reticle` row is on (and the HUD is not covered by a menu: the caller's call).
    pub visible: bool,
    /// The aim ray found a live prop within `RangeVSBuildings2D` (2.88 m) / `RangeVSBuildingsZ` (3.84 m): colour = `ReticleBuildingColor` (yellow).
    pub colour_building: bool,
    /// RGBA tint, linear 0..1 as the old `reticle_tint` (white or (255,221,0)/255).
    pub tint: [f32; 4],
    /// Texture name (without extension) and its size in texture pixels (= the `FSlateImageBrush` size; multiply by the DPI scale).
    pub image: &'static str,
    pub image_size: (f32, f32),
    /// Offset from the safe-zone centre in Slate units. Always (0, 0) for melee.
    pub centre_offset: (f32, f32),
}
impl Default for ReticleState { fn default() -> Self { Self::new(false, false) } }
impl ReticleState {
    pub fn new(visible: bool, colour_building: bool) -> Self {
        ReticleState { visible, colour_building: visible && colour_building, tint: r::reticle_tint(visible && colour_building), image: r::RETICLE_IMAGE_NAME, image_size: r::RETICLE_IMAGE, centre_offset: (0., 0.) }
    }
    /// Pixel rectangle on a `w` x `h` screen at DPI scale `scale` (snapped to whole pixels like Slate): `(x, y, w, h)`.
    pub fn rect(&self, w: f32, h: f32, scale: f32) -> (f32, f32, f32, f32) { let q = r::reticle_rect(w, h, scale); (q.x, q.y, q.w, q.h) }
}

// ---- harvest popups ----------------------------------------------------------------------------------------------------------
/// The `+N` aggregate row (icon + total, the big text) with every animated value already evaluated for this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct AggregateView {
    pub resource: Resource,
    /// "+N" (TextStyle-Base-L-B = NotoSans-Bold 32 white over a black copy translated `AGG_SHADOW` = (3, 5) units).
    pub text: String, pub font: f32,
    /// Row transform / alphas / pulse scales: translate Y (units), uniform scale about the row centre, text / shadow / icon alpha, icon and text pulse scale.
    pub params: AggParams,
}
/// The small per-hit `+N` that rises and fades (`TranslateAndFadeUp`, 1.0 s) with its black shadow copy (offset (1, 2), alpha 0.5 -> 0).
#[derive(Clone, Debug, PartialEq)]
pub struct PotentialView {
    pub text: String, pub weakpoint: bool,
    /// 16 normal / 20 weak point (18 = the designer size before the first hit).
    pub font: f32,
    /// sRGB colour of the text: white, or `Color-HarvestWeakPoint` cyan.
    pub colour: [f32; 3],
    pub params: PotParams,
}
/// Everything the harvest popup widget draws this frame. `None` from [`HarvestPopups::view`] = nothing (collapsed).
#[derive(Clone, Debug, PartialEq)]
pub struct PopupView {
    /// The aggregate row (visible while the widget is Visible; fades out in `AggregateOutroAnim`).
    pub aggregate: Option<AggregateView>,
    /// The rising per-hit text (while `TranslateAndFadeUp` plays).
    pub potential: Option<PotentialView>,
    /// Texture of the resource icon: wood `T-Icon-Logs`, stone `T-Icon-S-Loot-Stone`, metal `T-Icon-IBeam`.
    pub icon: &'static str,
    /// Measured-text-independent layout constants of the overlay slot (Padding R400 / B300 units, SizeBox max 200 x 300, icon 40, potential render scale 1.2).
    pub icon_size: f32,
}
/// The harvest popup state machine (`fn_combat::reticle::Popup`) behind a view API.
#[derive(Clone, Debug, Default)]
pub struct HarvestPopups { popup: Popup }
impl HarvestPopups {
    /// `FortHUDContext.OnDamagedResourceBuilding` (the sim's `world.events` entries, in order).
    pub fn on_hit(&mut self, e: HarvestEvent) { self.popup.on_event(e); }
    /// Game-time step (call once per rendered frame with the frame dt: the timers and animations are wall-of-game-time, not tick rate).
    pub fn tick(&mut self, dt: f32) { self.popup.tick(dt); }
    pub fn reset(&mut self) { self.popup = Popup::default(); }
    /// The raw machine (tests / captures).
    pub fn state(&self) -> &Popup { &self.popup }
    pub fn active(&self) -> bool { self.popup.active() }
    /// The values to draw, or `None` when the widget is collapsed and nothing rises.
    pub fn view(&self) -> Option<PopupView> {
        let st = &self.popup; if !st.active() { return None }
        let pot = st.potential.map(|p| (p.amount, p.weak)).or(st.last_potential);
        let font = if st.last_potential.is_none() && st.potential.is_none() { r::DESIGNER_FONT } else if pot.is_some_and(|p| p.1) { r::WEAKPOINT_FONT } else { r::NORMAL_FONT };
        let aggregate = st.visible.then(|| AggregateView { resource: st.resource, text: format!("+{}", st.aggregate), font: r::AGG_FONT, params: r::agg_params(st.outro_t, st.pulse_t) });
        let potential = st.potential.map(|p| PotentialView { text: format!("+{}", p.amount), weakpoint: p.weak, font, colour: if p.weak { r::weakpoint_color() } else { [1., 1., 1.] }, params: r::potential_params(p.t) });
        let icon = match st.resource { Resource::Wood => "T-Icon-Logs", Resource::Stone => "T-Icon-S-Loot-Stone", Resource::Metal => "T-Icon-IBeam" };
        Some(PopupView { aggregate, potential, icon, icon_size: r::ICON_SIZE })
    }
}

// ---- pickup manager packs ------------------------------------------------------------------------------------------------------
/// The Wood | Stone | Metal pack counters of the pickup manager (`PackResource.UpdateCount`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackView {
    /// Number to print per pack (counts up linearly over 0.5 s to the target, clamped to 999).
    pub shown: [u32; 3],
    /// The real counts (clamped to MaxStackSize 999).
    pub target: [u32; 3],
    /// Text alpha per pack: 1.0, or 0.6 once the pack holds `MaxStackSize` (999).
    pub text_alpha: [f32; 3],
    /// Pack of `GetCurrentResourceItemDefinition()` (0 wood, 1 stone, 2 metal): it gets `Border_HUD_ResourceBg_Highlighted` instead of the plain backing. ASSUMED wood.
    pub highlighted: usize,
}
impl Default for PackView { fn default() -> Self { PackView { shown: [0; 3], target: [0; 3], text_alpha: [1.; 3], highlighted: 0 } } }

// ---- hierarchical [HUD] tags (old hud_vis.rs) ----------------------------------------------------------------------------------
/// Settings rows (tags) whose element this runtime draws and whose toggle it honours; the settings page badges the others "not implemented".
pub const IMPLEMENTED_TAGS: [&str; 5] = ["HUD.Center.Reticle", "HUD.TopLeft.PlayerHitPoints", "HUD.TopLeft.PickupManager", "HUD.TopRight.Minimap", "HUD.Quickbar"];
pub const TAG_RETICLE: &str = "HUD.Center.Reticle";
pub const TAG_PLAYER_HEALTH: &str = "HUD.TopLeft.PlayerHitPoints";
pub const TAG_PICKUP: &str = "HUD.TopLeft.PickupManager";
pub const TAG_MINIMAP: &str = "HUD.TopRight.Minimap";
/// Row tag of the quick bar; the widget tags are `HUD.Quickbar.Combat` (primary) and `HUD.Quickbar.Build` (secondary, not drawn).
pub const TAG_QUICKBAR: &str = "HUD.Quickbar";
/// `HUD.Center.ResourceAggregation` has no row of its own in the 1.8 HUD tab (7 rows), so no toggle can hide the popups; `HUD.Center.Reticle` is not a parent of it.
pub const TAG_RESOURCE_AGGREGATION: &str = "HUD.Center.ResourceAggregation";
/// Same rule as the settings page: missing = ON; `false` / `0` / `off` = OFF.
pub fn parse_hud_flag(v: Option<&str>) -> bool { match v { Some(v) => !(v.eq_ignore_ascii_case("false") || v == "0" || v.eq_ignore_ascii_case("off")), None => true } }
/// The `[HUD]` toggles of the live settings with GameplayTag matching: a switched-off row `R` hides every widget tag that equals `R` or lives below it (`R.` prefix).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudTagVisibility { rows: Vec<(String, bool)> }
impl HudTagVisibility {
    /// Copies the `[HUD]` extras of the implemented rows (default ON).
    pub fn from_settings(s: &Settings) -> Self { HudTagVisibility { rows: IMPLEMENTED_TAGS.iter().map(|t| (t.to_string(), parse_hud_flag(s.extra("HUD", t)))).collect() } }
    pub fn set(&mut self, row: &str, on: bool) { match self.rows.iter_mut().find(|r| r.0 == row) { Some(r) => r.1 = on, None => self.rows.push((row.to_owned(), on)) } }
    pub fn clear(&mut self) { self.rows.clear(); }
    /// Is an element with this widget tag shown?
    pub fn visible(&self, widget_tag: &str) -> bool {
        !self.rows.iter().any(|(row, on)| !*on && (widget_tag == row || (widget_tag.starts_with(row.as_str()) && widget_tag.as_bytes().get(row.len()) == Some(&b'.'))))
    }
    /// Does this runtime honour the toggle of this settings row?
    pub fn implemented(row_tag: &str) -> bool { IMPLEMENTED_TAGS.contains(&row_tag) }
}

// ---- the whole feedback layer ------------------------------------------------------------------------------------------------------
/// Reticle + popups + pack counters + tag toggles, driven from the sim once per rendered frame.
#[derive(Clone, Debug, Default)]
pub struct HudFeedback {
    pub reticle: ReticleState,
    pub popups: HarvestPopups,
    pub packs: PackView,
    pub tags: HudTagVisibility,
    count: CountAnim,
}
impl HudFeedback {
    /// One frame of game time: `pickaxe_equipped` = quickbar slot 0 selected. Drains `sim.world.events` into the popups, copies the Wood / Stone counters into the
    /// packs (`metal` comes from the caller, the sandbox has none), evaluates the reticle colour rule with the camera aim ray against the live props (the pawn eye
    /// is `(sim.x, sim.y, sim.z)`), and syncs the `[HUD]` toggles from `settings`.
    pub fn tick(&mut self, sim: &mut GameSim, settings: &Settings, pickaxe_equipped: bool, metal: u32, dt: f32) {
        self.tags = HudTagVisibility::from_settings(settings);
        for e in sim.world.events.drain(..) { self.popups.on_hit(e); }
        self.popups.tick(dt);
        let target = [r::clamp_resource(sim.world.wood), r::clamp_resource(sim.world.stone), r::clamp_resource(metal)];
        self.count.step(target, dt);
        self.packs = PackView { shown: [0, 1, 2].map(|i| self.count.shown(i, target)), target, text_alpha: target.map(|n| if n >= r::MAX_STACK { r::COUNT_ALPHA_FULL } else { 1. }), highlighted: 0 };
        let visible = pickaxe_equipped && self.tags.visible(TAG_RETICLE);
        let v = sim.cam_view; let pawn = Vec3::new(sim.x, sim.y, sim.z);
        self.reticle = ReticleState::new(visible, visible && r::aim_on_target(&sim.world.props, v.eye, v.forward, pawn));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_game::combat::{Harvestable, Kind};
    fn ev(obj: u64, amount: u32, destroyed: bool, weak: bool) -> HarvestEvent { HarvestEvent { object: obj, resource: Resource::Wood, amount, destroyed, weakpoint: weak } }

    #[test] fn reticle_is_white_by_default_and_yellow_only_when_visible_and_aimed() {
        let w = ReticleState::new(true, false); assert!(w.visible && !w.colour_building && w.tint == [1., 1., 1., 1.] && w.image == "HUD_Xhair_Dot" && w.image_size == (58., 54.) && w.centre_offset == (0., 0.));
        let y = ReticleState::new(true, true); assert!(y.colour_building && (y.tint[1] - 221. / 255.).abs() < 1e-6 && y.tint[2] == 0.);
        let hidden = ReticleState::new(false, true); assert!(!hidden.visible && !hidden.colour_building, "a hidden reticle never reports a target colour");
        assert_eq!(ReticleState::default(), ReticleState::new(false, false));
        assert_eq!(ReticleState::default().rect(1920., 1080., 1.), (931., 513., 58., 54.));
    }
    #[test] fn popup_view_follows_the_widget_state_machine() {
        let mut p = HarvestPopups::default(); assert!(p.view().is_none());
        p.on_hit(ev(7, 50, false, false));
        let v = p.view().unwrap(); let a = v.aggregate.unwrap(); let q = v.potential.unwrap();
        assert_eq!((a.text.as_str(), a.font, q.text.as_str(), q.font, q.weakpoint, q.colour), ("+50", 32., "+50", 16., false, [1., 1., 1.]));
        assert_eq!(v.icon, "T-Icon-Logs"); assert_eq!(a.params.icon_scale, 1.); assert!((q.params.alpha - 1.).abs() < 1e-6);
        p.tick(0.1); let a = p.view().unwrap().aggregate.unwrap(); assert!((a.params.icon_scale - 1.5).abs() < 1e-5, "pulse peaks 1.5 at 0.1 s: {}", a.params.icon_scale);
        p.on_hit(ev(7, 50, false, true)); let v = p.view().unwrap(); assert_eq!(v.aggregate.unwrap().text, "+100", "same object aggregates"); let q = v.potential.unwrap(); assert!(q.weakpoint && q.font == 20. && q.colour[0] < 0.5 && q.colour[1] > 0.9);
        p.tick(1.0); assert!(p.view().unwrap().potential.is_none(), "the rising text is done after 1 s while the aggregate waits for the 3 s timer");
        p.tick(2.1); p.tick(1.1); assert!(p.view().is_none(), "timer, outro, hidden");
        p.on_hit(HarvestEvent { object: 9, resource: Resource::Stone, amount: 10, destroyed: true, weakpoint: false }); assert_eq!(p.view().unwrap().icon, "T-Icon-S-Loot-Stone"); p.reset(); assert!(!p.active());
    }
    #[test] fn tag_rule_is_hierarchical_and_reads_the_settings_rows() {
        let mut t = HudTagVisibility::default(); assert!(t.visible("HUD.Center.Reticle") && t.visible("HUD.Quickbar.Combat"));
        t.set(TAG_RETICLE, false); assert!(!t.visible("HUD.Center.Reticle") && t.visible(TAG_RESOURCE_AGGREGATION), "other HUD.Center tags are unaffected");
        t.set(TAG_QUICKBAR, false); assert!(!t.visible("HUD.Quickbar.Combat") && !t.visible("HUD.Quickbar.Build") && !t.visible("HUD.Quickbar") && t.visible("HUD.QuickbarX"), "prefix must end at a '.'");
        let mut s = Settings::default(); s.set_extra("HUD", TAG_RETICLE, "False"); s.set_extra("HUD", TAG_MINIMAP, "True");
        let t = HudTagVisibility::from_settings(&s); assert!(!t.visible(TAG_RETICLE) && t.visible(TAG_MINIMAP) && t.visible(TAG_PLAYER_HEALTH));
        assert!(parse_hud_flag(None) && parse_hud_flag(Some("True")) && !parse_hud_flag(Some("off")) && !parse_hud_flag(Some("0")) && parse_hud_flag(Some("1")));
        for tag in IMPLEMENTED_TAGS { assert!(HudTagVisibility::implemented(tag)); } assert!(!HudTagVisibility::implemented("HUD.TopRight.TargetInfo"));
    }
    #[test] fn feedback_layer_end_to_end_harvest_counters_and_aim_colour() {
        let mut sim = GameSim::new(); sim.world.props.clear();
        sim.world.props.push(Harvestable::new(Kind::Tree, Vec3::new(0., 0., 12. - 1.8)));
        let mut fb = HudFeedback::default(); let s = Settings::default();
        // a swing that connects: hold fire for a while at 60 fps, the HUD ticks every frame
        let mut step = |sim: &mut GameSim, fb: &mut HudFeedback, attack: bool| { sim.step(&fn_game::FrameInput { attack, ..Default::default() }, 1. / 60.); fb.tick(sim, &s, true, 0, 1. / 60.); };
        for _ in 0..30 { step(&mut sim, &mut fb, false); }
        assert!(fb.reticle.visible && fb.reticle.colour_building, "the camera ray finds the tree 1.8 m ahead of the pawn: {:?} eye {:?} fwd {:?}", fb.reticle, sim.cam_view.eye, sim.cam_view.forward);
        for _ in 0..300 { step(&mut sim, &mut fb, true); }
        assert!(sim.world.hits >= 3 && sim.world.events.is_empty(), "the HUD drained the events");
        assert!(fb.popups.view().is_some() || fb.popups.state().aggregate > 0 || sim.world.hits > 0);
        assert_eq!(fb.packs.target[0], sim.world.wood.min(999)); assert_eq!(fb.packs.text_alpha, [1.; 3]);
        // the count-up: the number shown trails the real count for half a second
        let mut fb2 = HudFeedback::default(); sim.world.wood = 10; fb2.tick(&mut sim, &s, true, 0, 0.016); sim.world.wood = 999; fb2.tick(&mut sim, &s, true, 0, 0.1);
        assert!(fb2.packs.shown[0] > 10 && fb2.packs.shown[0] < 999, "{}", fb2.packs.shown[0]); fb2.tick(&mut sim, &s, true, 0, 0.5);
        assert_eq!((fb2.packs.shown[0], fb2.packs.text_alpha[0]), (999, 0.6), "999 cap dims the number");
        // not equipped / toggled off: no reticle
        fb.tick(&mut sim, &s, false, 0, 0.016); assert!(!fb.reticle.visible && !fb.reticle.colour_building);
        let mut off = Settings::default(); off.set_extra("HUD", TAG_RETICLE, "False"); fb.tick(&mut sim, &off, true, 0, 0.016); assert!(!fb.reticle.visible);
    }
}
