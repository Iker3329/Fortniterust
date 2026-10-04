//! HARVEST HOOK (fn-app): the pickaxe harvest feedback glue. The LOGIC is `fn_combat::harvest_fx` (weak point placement / relocation, the 50 / 100 critical damage, damage
//! numbers, break, the aimed prop); the DRAWING of the screen-space parts is the original 1.8 widgets of `fn-ui` (Target Info = `BuildingInfoIndicator` / `BuildingBar`,
//! the `+N` popups with the cyan weak-point variant, the reticle with its building colour). This file only connects the two: once per presented frame [`feed`] drains the
//! strike events into `HudWorld::harvest_hits`, says whether the crosshair is on a prop (reticle colour), and hands the aimed prop to Target Info with its projected anchor.
//!
//! NOT DRAWN (reported, not faked): the weak-point ring (`WeakSpot_C` -> `P_WeakPointEffect_01` mesh particles with the `M_Weakpoint_*` unlit translucent materials) and the world
//! damage numbers (`ImpactNumbers_C` mesh digits with `DamageNumbers_Mat` / `DamageNumberWPO`): `fn-render` has no translucent / unlit / vertex-animated mesh material path, and
//! `ComponentLifespan` of the numbers is a native member that is not in the packages. Their data and state are complete in `world.fx` (`Fx::props[..].weak`, `Fx::numbers`).
use fn_combat::harvest_fx::{self, FxEvent};
use fn_game::sim::EYE;
use fn_game::GameSim;
use fn_render::CameraParams;
use fn_ui::screens::hud::{Hud, HudState};
use fn_ui::screens::reticle_state::{HarvestHit, HarvestResource, HealthDisplayRule, TargetInfo};
use glam::{Vec3, Vec4};
use std::sync::Mutex;

/// What one frame of feedback produced (for the log and for a future world-space layer).
#[derive(Debug, Default)]
pub struct Frame { pub fx_events: Vec<FxEvent>, pub hits: usize, pub on_target: bool, pub target: Option<TargetInfo> }

/// World point -> pixel (top-left origin) for the camera of this frame; `None` behind the camera or non-finite.
pub fn project(cam: &CameraParams, p: Vec3, w: f32, h: f32) -> Option<(f32, f32)> {
    let c = cam.proj_matrix() * cam.view_matrix() * Vec4::new(p.x, p.y, p.z, 1.);
    if !(c.w > 1e-6) { return None; }
    let (x, y) = (c.x / c.w, c.y / c.w);
    let px = ((x * 0.5 + 0.5) * w, (0.5 - y * 0.5) * h);
    (px.0.is_finite() && px.1.is_finite()).then_some(px)
}

fn resource(r: fn_combat::Resource) -> HarvestResource { match r { fn_combat::Resource::Wood => HarvestResource::Wood, fn_combat::Resource::Stone => HarvestResource::Stone, fn_combat::Resource::Metal => HarvestResource::Metal } }

/// The one call per presented frame (after `hud_state`, before `present_hud`). Drains the harvest events (they would otherwise pile up: nothing else reads them).
pub fn feed(sim: &mut GameSim, st: &mut HudState, cam: &CameraParams, w: f32, h: f32) -> Frame {
    let mut f = Frame::default();
    // 1. every strike of the frame: the `+N` popup input (OnDamagedResourceBuilding); weakpoint -> the cyan size-20 text
    let events = std::mem::take(&mut sim.world.events);
    f.hits = events.len();
    for e in &events {
        st.world.harvest_hits.push(HarvestHit { object: e.object, resource: resource(e.resource), amount: e.amount, destroyed: e.destroyed, weakpoint: e.weakpoint });
        log(format!("f{:05} strike object={} amount={} weakpoint={} destroyed={}", sim.frames, e.object, e.amount, e.weakpoint, e.destroyed));
    }
    f.fx_events = sim.world.fx.take_events();
    for e in &f.fx_events { log(format!("f{:05} fx {}", sim.frames, describe(e))); }
    crate::particles::on_events(&f.fx_events, sim); // PARTICLE HOOK: death particles on the break, pickaxe impact particles on the hits
    // Scripted-run test aid (never set in play): FNRT_AIM_WEAKSPOT=1 turns the camera towards the first weak spot, what a player does with the blue circle, so a script that
    // only holds the attack button can exercise the critical hit. Applies to the next tick's camera ray.
    if std::env::var_os("FNRT_AIM_WEAKSPOT").is_some() {
        st.select(0); // a script cannot press the quickbar when the app has no loadout resources: show the pickaxe as the held item
        let eye = sim.cam_view.eye;
        if let Some(s) = sim.world.props.iter().enumerate().find_map(|(i, p)| sim.world.fx.weak_spot(i, p)) {
            let d = s.pos - eye; sim.yaw = d.x.atan2(-d.z).to_degrees().rem_euclid(360.); sim.pitch = d.y.atan2((d.x * d.x + d.z * d.z).sqrt()).to_degrees();
        }
    }
    // 2. crosshair on a prop (the reticle's building colour) and the aimed prop (Target Info)
    let (eye, fwd) = (sim.cam_view.eye, sim.cam_view.forward);
    let pawn = Vec3::new(sim.x, sim.y - EYE + fn_combat::CHEST_HEIGHT, sim.z);
    f.on_target = fn_combat::reticle::aim_on_target(&sim.world.props, eye, fwd, pawn);
    st.world.reticle.on_target = f.on_target;
    let focus = if st.pickaxe_equipped() { harvest_fx::focus(&sim.world, eye, fwd, pawn) } else { None };
    if let Ok(mut last) = LAST_FOCUS.lock() { let now = (focus.map(|f| (f.object, f.health as i32)), st.pickaxe_equipped(), f.on_target); if *last != Some(now) { *last = Some(now); log(format!("f{:05} reticle on_target={} pickaxe_equipped={} (selected slot {}, icon {:?}) focus {}", sim.frames, f.on_target, st.pickaxe_equipped(), st.selected, st.items.get(st.selected).and_then(|i| i.as_ref()).map(|i| i.icon.clone()), focus.map_or("none".to_owned(), |f| format!("object={} hp={}/{} anchor=({:.2},{:.2},{:.2}) screen={:?}", f.object, f.health, f.max_health, f.anchor.x, f.anchor.y, f.anchor.z, project(cam, f.anchor, w, h).map(|p| (p.0.round(), p.1.round())))))); } }
    if let Some(focus) = focus {
        {
            if let Some(screen) = project(cam, focus.anchor, w, h) {
                // HealthDisplayRule::Allowed (= the widget's `OnDamage`): shown after damage, hidden 3 s after the last change; interaction None
                let t = TargetInfo { id: focus.object, health: focus.health, max_health: focus.max_health, rule: HealthDisplayRule::OnDamage, interaction: 0, screen: Some(screen) };
                st.world.target = Some(t); f.target = Some(t);
            }
        }
    }
    f
}

fn describe(e: &FxEvent) -> String {
    match e {
        FxEvent::WeakSpotSpawned { object, pos } => format!("weak-spot spawned object={object} at ({:.2},{:.2},{:.2})", pos.x, pos.y, pos.z),
        FxEvent::WeakSpotHit { object, pos, level, moved_to } => format!("weak-spot HIT object={object} crack-level={level} at ({:.2},{:.2},{:.2}) moved_to={}", pos.x, pos.y, pos.z, moved_to.map_or("-".to_owned(), |m| format!("({:.2},{:.2},{:.2})", m.x, m.y, m.z))),
        FxEvent::Hit { object, pos, critical, .. } => format!("hit object={object} critical={critical} at ({:.2},{:.2},{:.2})", pos.x, pos.y, pos.z),
        FxEvent::Broke { object, pos, kind, resource, proto } => format!("BROKE object={object} {kind:?} {resource:?} proto={proto} at ({:.2},{:.2},{:.2})", pos.x, pos.y, pos.z),
    }
}

// ---- the run log ----------------------------------------------------------------------------------------------------------------
static LOG: Mutex<Vec<String>> = Mutex::new(Vec::new());
static LAST_BAR: Mutex<Option<(bool, i32, i32)>> = Mutex::new(None);
static LAST_FOCUS: Mutex<Option<(Option<(u64, i32)>, bool, bool)>> = Mutex::new(None);
fn log(s: String) { if let Ok(mut l) = LOG.lock() { if l.len() < 20_000 { l.push(s); } } }

/// Logs the Target Info bar when its visibility / numbers change (call after `present_hud`).
pub fn observe_hud(frame: u32, hud: &Hud) {
    let t = &hud.target;
    let now = (t.shown(), t.bar_current.round() as i32, t.bar_max.round() as i32);
    if let Ok(mut last) = LAST_BAR.lock() {
        if *last != Some(now) { *last = Some(now); log(format!("f{frame:05} target-info bar shown={} hp={}/{} hide_timer={:?}", now.0, now.1, now.2, t.hide_timer)); }
    }
}

/// Epilogue of a scripted run: writes `<name>-harvest.txt` and returns the report section.
pub fn run_report(out: &std::path::Path, name: &str) -> String {
    let lines = LOG.lock().map(|l| l.clone()).unwrap_or_default();
    if lines.is_empty() { return String::new(); }
    let path = out.join(format!("{name}-harvest.txt"));
    let _ = std::fs::write(&path, lines.join("\n") + "\n");
    let strikes = lines.iter().filter(|l| l.contains(" strike ")).count();
    let crits = lines.iter().filter(|l| l.contains("weakpoint=true")).count();
    let broke = lines.iter().filter(|l| l.contains(" BROKE ")).count();
    format!("harvest feedback: {strikes} strikes, {crits} weak-point (100), {} ordinary (50), {broke} broken, log {}\n", strikes - crits, path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_combat::{Harvestable, Kind};
    use fn_hud_adapter::{hud_state, HudInputs};
    use fn_game::settings::Settings;

    fn cam(sim: &GameSim, w: u32, h: u32) -> CameraParams { crate::scene::camera_for(sim, w, h) }

    #[test] fn a_point_straight_ahead_projects_to_the_screen_centre_and_one_behind_is_none() {
        let sim = GameSim::new(); let c = cam(&sim, 1280, 720);
        let ahead = c.eye + c.forward * 5.; let p = project(&c, ahead, 1280., 720.).unwrap();
        assert!((p.0 - 640.).abs() < 0.5 && (p.1 - 360.).abs() < 0.5, "{p:?}");
        assert!(project(&c, c.eye - c.forward * 5., 1280., 720.).is_none());
        let up = project(&c, ahead + c.up * 1., 1280., 720.).unwrap(); assert!(up.1 < 360., "up is smaller y: {up:?}");
    }
    /// The whole chain on the real game state: swings at a tree 1.8 m ahead through `GameSim::step` -> hits, weak spot, events -> HudState -> the real fn-ui widgets.
    #[test] fn swinging_at_a_tree_feeds_popups_a_reticle_colour_and_a_target_info_bar() {
        let mut sim = GameSim::new(); sim.world.props.clear(); sim.world.props.push(Harvestable::new(Kind::Tree, Vec3::new(0., 0., 12. - 1.8)));
        let settings = Settings::default(); let mut hud = Hud::new();
        let (w, h) = (1280u32, 720u32);
        let mut crit_seen = false; let mut bar_seen = false; let mut on_target_seen = false; let mut saw_popup_weak = false; let (mut bar_when_damaged, mut bar_shown_when_damaged) = (0u32, 0u32);
        for frame in 0..900u32 {
            let mut inp = fn_game::FrameInput::default(); inp.attack = true;
            // aim the camera at the weak point once there is one (what a player does with the blue circle)
            if let Some(s) = sim.world.fx.weak_spot(0, &sim.world.props[0]) {
                let d = s.pos - sim.cam_view.eye; inp.set_yaw = Some(d.x.atan2(-d.z).to_degrees()); inp.set_pitch = Some((d.y.atan2((d.x * d.x + d.z * d.z).sqrt())).to_degrees());
            }
            sim.step(&inp, 1. / 60.);
            let c = cam(&sim, w, h);
            let mut st = hud_state(&sim, &settings, &HudInputs::default());
            let f = feed(&mut sim, &mut st, &c, w as f32, h as f32);
            on_target_seen |= f.on_target; saw_popup_weak |= st.world.harvest_hits.iter().any(|x| x.weakpoint);
            hud.set_state(&st); hud.target.update(hud.state.world.target.as_ref(), w as f32, h as f32); /* what Hud::present does */ hud.tick(1. / 60.);
            if f.target.is_some() && sim.world.props[0].alive() && sim.world.props[0].health < sim.world.props[0].max_hp { bar_when_damaged += 1; bar_shown_when_damaged += hud.target.shown() as u32; }
            bar_seen |= hud.target.shown(); crit_seen |= sim.world.fx.critical_hits > 0;
            if !sim.world.props[0].alive() { break; }
            let _ = frame;
        }
        assert!(!sim.world.props[0].alive(), "the tree broke");
        assert!(sim.world.fx.critical_hits >= 1 && sim.world.fx.normal_hits >= 1, "ordinary {} and weak-point {} hits", sim.world.fx.normal_hits, sim.world.fx.critical_hits);
        assert!(crit_seen && saw_popup_weak && on_target_seen && bar_seen, "crit {crit_seen} popup {saw_popup_weak} reticle {on_target_seen} bar {bar_seen}");
        assert!(bar_when_damaged > 0 && bar_shown_when_damaged > 0 && hud.target.bar_max == 600., "bar {bar_shown_when_damaged}/{bar_when_damaged} max {}", hud.target.bar_max);
        assert!(sim.world.events.is_empty(), "feed() drains the harvest events");
        assert_eq!(sim.world.wood as usize, 50 * sim.world.fx.normal_hits as usize + 100 * sim.world.fx.critical_hits as usize);
    }
}
