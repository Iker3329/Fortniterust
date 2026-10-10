//! Renderer-free port of the old `src/reticle.rs` (1.8 reticle and harvest feedback): constants, the aim-ray colour rule, the cooked MovieScene curves and the
//! `ResourceAggregationWidget` + `PotentialResourceWidget` state machine. Drawing (textures, text, GL) is NOT here: `fn-ui` / `fn-ui-wgpu` draw what
//! `fn_hud_adapter::{ReticleState, HarvestPopups}` expose (this module is their engine; see the crate README of fn-hud-adapter).
//!
//! EVIDENCE: `tools/reticle-evidence.cjs` -> `reports/reticle-1.8-evidence.json` (data + native findings), `reports/hud-bytecode/{Reticle,ResourceAggregationWidget,
//! PotentialResourceWidget,PackResource}.txt` (Kismet), `reports/melee-stats-pickaxe.json` (AthenaMeleeWeapons row).
//!
//! RETICLE (what 1.8 actually does)
//! * `Reticle_C` is an EMPTY widget blueprint on the native `FortReticle`; the picture is painted by the native `SFortReticle` from WEAPON properties. The pickaxe
//!   chain sets `ReticleImage = HUD_Xhair_Dot` (58x54) and `ReticleBuildingColor = FColor(255,221,0,255)`; no `HitNotifyReticleImage`, no corner angles.
//! * For a non-ranged weapon the image sits at the safe-zone centre (= screen centre; the camera's centre ray IS the aim ray). Colour: white unless the weapon's
//!   `DetermineReticleColor` aim trace finds an actor in range, then `ReticleBuildingColor` (yellow). No melee hit marker exists in 1.8.
//!
//! HARVEST FEEDBACK: `FortHUDContext.OnDamagedResourceBuilding(object, item, destroyed, weakpoint)` -> [`Popup::on_event`]; layout numbers, styles, the Kismet state
//! machine and the three MovieScene animations are recovered exactly. Weak-point hits use font size 20 and `Color-HarvestWeakPoint` (cyan) instead of 16 / white.
//!
//! NOT RECOVERED (explicit): C++ ctor defaults of ReticleDefaultColor/EnemyColor, the aim trace's collision channel (the baseplate floor is never a hit here), the
//! melee range of the native colour test (`RangeVSBuildings2D` 288 cm is used), the per-hit resource amount (`HARVEST_DAMAGE`, provisional), tree weak points.
use crate::{trace_cylinders, HarvestEvent, Harvestable, Resource};
use glam::Vec3;

// ---- reticle (recovered) -----------------------------------------------------------------------------------------------------
/// `HUD_Xhair_Dot` texture size = the FSlateImageBrush size (reports: textures.HUD_Xhair_Dot).
pub const RETICLE_IMAGE: (f32, f32) = (58., 54.);
/// Texture the pickaxe reticle draws (`B_Melee_Generic.ReticleImage`).
pub const RETICLE_IMAGE_NAME: &str = "HUD_Xhair_Dot";
/// `B_Melee_Generic.ReticleBuildingColor` FColor r,g,b,a.
pub const COLOR_BUILDING: [u8; 4] = [255, 221, 0, 255];
/// `FLinearColor::White` (native default colour of SFortReticle). `ReticleDefaultColor` itself is a native ctor default that is not recoverable: ASSUMED white.
pub const COLOR_DEFAULT: [u8; 4] = [255, 255, 255, 255];
/// Pickaxe `RangeVSBuildings2D` / `RangeVSBuildingsZ` in metres (AthenaMeleeWeapons row Harvest_Pickaxe_Athena_C_T01, 288 / 384 cm).
pub const RANGE_2D: f32 = 2.88;
pub const RANGE_Z: f32 = 3.84;
/// Length of the aim trace in the native colour test (start + dir * 8192 cm).
pub const AIM_TRACE: f32 = 81.92;

pub fn reticle_tint(on_target: bool) -> [f32; 4] { let c = if on_target { COLOR_BUILDING } else { COLOR_DEFAULT }; [c[0] as f32 / 255., c[1] as f32 / 255., c[2] as f32 / 255., c[3] as f32 / 255.] }
/// Axis-aligned pixel rectangle (the UI crates have their own; this one keeps the layout math here dependency free).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
impl Rect { pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect { Rect { x, y, w, h } } }
/// Pixel rectangle of the reticle image for a `w` x `h` screen at DPI scale `scale` (`fn_ui::layout::dpi_scale(w, h)`): centred in the (zero-margin) safe zone,
/// snapped to whole pixels like Slate box/image elements.
pub fn reticle_rect(w: f32, h: f32, scale: f32) -> Rect {
    let (iw, ih) = (RETICLE_IMAGE.0 * scale, RETICLE_IMAGE.1 * scale);
    Rect::new(((w - iw) * 0.5).round(), ((h - ih) * 0.5).round(), iw, ih)
}
/// The native colour test: does the camera aim ray (camera eye + forward) find a live prop within the pickaxe's reach of the pawn?
/// `pawn` = pawn eye position. APPROXIMATE (see module docs): props only (the floor is not a hit), 2D range 288 cm, Z range 384 cm.
pub fn aim_on_target(props: &[Harvestable], eye: Vec3, forward: Vec3, pawn: Vec3) -> bool { hit_on_target(trace_cylinders(props, eye, forward, AIM_TRACE, 0.), eye, forward, pawn) }
pub fn hit_on_target(hit: Option<crate::TraceHit>, eye: Vec3, forward: Vec3, pawn: Vec3) -> bool {
    let Some(hit) = hit else { return false };
    let d = forward.normalize_or_zero(); let p = eye + d * (AIM_TRACE * hit.fraction);
    let flat = ((p.x - pawn.x).powi(2) + (p.z - pawn.z).powi(2)).sqrt();
    flat <= RANGE_2D && (p.y - pawn.y).abs() <= RANGE_Z
}

// ---- animation curves (MovieScene RichCurves, cubic keys, decoded by tools/reticle-evidence.cjs) ---------------------------
/// One cooked RichCurve key (interp 2 = cubic; tangents in value / second).
#[derive(Clone, Copy, Debug)]
pub struct Key { pub t: f32, pub v: f32, pub arrive: f32, pub leave: f32 }
const fn k(t: f32, v: f32) -> Key { Key { t, v, arrive: 0., leave: 0. } }
/// UE FRichCurve::Eval for cubic keys with constant extrapolation (Hermite on each interval).
pub fn eval(keys: &[Key], t: f32) -> f32 {
    let (first, last) = (keys[0], keys[keys.len() - 1]);
    if t <= first.t { return first.v; } if t >= last.t { return last.v; }
    for w in keys.windows(2) { let (a, b) = (w[0], w[1]); if t >= a.t && t <= b.t {
        let dt = b.t - a.t; if dt <= 0. { return b.v; } let s = ((t - a.t) / dt).clamp(0., 1.); let (s2, s3) = (s * s, s * s * s);
        return (2. * s3 - 3. * s2 + 1.) * a.v + (s3 - 2. * s2 + s) * (a.leave * dt) + (-2. * s3 + 3. * s2) * b.v + (s3 - s2) * (b.arrive * dt);
    } }
    last.v
}
/// ResourceAggregationWidget.AggregateOutroAnim (1.0 s): HBox translation Y, HBox scale (X = Y), aggregate text / icon / shadow alpha.
pub const OUTRO_LEN: f32 = 1.0;
pub const OUTRO_HBOX_TY: [Key; 2] = [k(0., 0.), k(1., -120.)];
pub const OUTRO_HBOX_SCALE: [Key; 4] = [k(0., 1.), k(0.05, 1.3), k(0.15, 1.), k(1., 1.)];
pub const OUTRO_TEXT_ALPHA: [Key; 4] = [k(0., 1.), k(0.05, 1.), Key { t: 0.45, v: 1., arrive: -1.05263, leave: 0. }, k(1., 0.)];
pub const OUTRO_ICON_ALPHA: [Key; 3] = [k(0., 1.), Key { t: 0.45, v: 1., arrive: -1., leave: 0. }, k(1., 0.)];
pub const OUTRO_SHADOW_ALPHA: [Key; 4] = [k(0., 0.5), k(0.05, 0.5), Key { t: 0.45, v: 0.5, arrive: -0.52632, leave: 0. }, k(1., 0.)];
/// ResourceIconPulseAnim (0.35 s): icon scale peaks 1.5 at 0.1 s, texts 1.3.
pub const PULSE_LEN: f32 = 0.35;
pub const PULSE_ICON_SCALE: [Key; 3] = [k(0., 1.), k(0.1, 1.5), k(0.35, 1.)];
pub const PULSE_TEXT_SCALE: [Key; 3] = [k(0., 1.), k(0.1, 1.3), k(0.35, 1.)];
/// PotentialResourceWidget.TranslateAndFadeUp (1.0 s): text rises 100 units and fades; the shadow (black, alpha 0.5 -> 0) starts at (1, 2) and rises with it.
pub const POTENTIAL_LEN: f32 = 1.0;
pub const POTENTIAL_TY: [Key; 2] = [k(0., 0.), k(1., -100.)];
pub const POTENTIAL_ALPHA: [Key; 2] = [k(0., 1.), k(1., 0.)];
pub const POTENTIAL_SHADOW_TY: [Key; 2] = [k(0., 2.), k(1., -98.)];
pub const POTENTIAL_SHADOW_ALPHA: [Key; 2] = [k(0., 0.5), k(1., 0.)];
pub const POTENTIAL_SHADOW_TX: f32 = 1.;

// ---- widget numbers (recovered) --------------------------------------------------------------------------------------------------
pub const SLOT_PADDING_RIGHT: f32 = 400.; // OverlaySlot Padding (L0, T0, R400, B300), HAlign Center, VAlign Bottom
pub const SLOT_PADDING_BOTTOM: f32 = 300.;
pub const MAX_WIDTH: f32 = 200.; // root SizeBox MaxDesiredWidth (MinDesiredHeight 300 is not enabled; MaxDesiredHeight 300)
pub const MAX_HEIGHT: f32 = 300.;
pub const ICON_SIZE: f32 = 40.; // ImageResourceIcon brush ImageSize / Standard-Icon-Size[3]
pub const AGG_FONT: f32 = 32.; // TextStyle-Base-L-B (size from TextStyle-Base-L)
pub const AGG_SHADOW: (f32, f32) = (3., 5.); // TextAggregateResourceCollectedShadow RenderTransform translation
pub const POTENTIAL_SCALE: f32 = 1.2; // PotentialResourceWidget RenderTransform scale in the VerticalBox slot
pub const NORMAL_FONT: f32 = 16.; // PotentialResourceWidget NormalFontSize
pub const WEAKPOINT_FONT: f32 = 20.; // WeakpointFontSize
pub const DESIGNER_FONT: f32 = 18.; // TextStyle-Base-S size used until SetupFont runs
pub const HIDE_DELAY: f32 = 3.; // HandleDamagedResourceObject: SetTimer(ForceHideWidget, 3, false)
/// `FortUIStylesheet.Color-HarvestWeakPoint` (linear).
pub const WEAKPOINT_LINEAR: [f32; 3] = [0.127563, 0.847657, 0.875];
pub fn srgb_encode(c: f32) -> f32 { let c = c.clamp(0., 1.); if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1. / 2.4) - 0.055 } }
pub fn weakpoint_color() -> [f32; 3] { WEAKPOINT_LINEAR.map(srgb_encode) }

// ---- the widget state machine (ResourceAggregationWidget + PotentialResourceWidget bytecode) ------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Potential { pub t: f32, pub amount: u32, pub weak: bool }
#[derive(Clone, Debug)]
pub struct Popup {
    /// `ResourceHoldingObject` (object identity of the prop being harvested).
    pub holding: Option<u64>, pub resource: Resource, pub aggregate: u32,
    /// Widget visibility: false = Collapsed.
    pub visible: bool, pub activate: bool, pub ignore_hide: bool,
    /// `HideWidgetTimer` seconds left.
    pub hide_timer: Option<f32>,
    pub pulse_t: Option<f32>, pub outro_t: Option<f32>,
    /// TranslateAndFadeUp playing (PotentialResourceWidget Visible); None = Hidden.
    pub potential: Option<Potential>,
    /// Last text of the PotentialResourceWidget (a Hidden widget keeps its layout size).
    pub last_potential: Option<(u32, bool)>,
}
impl Default for Popup { fn default() -> Self { Self { holding: None, resource: Resource::Wood, aggregate: 0, visible: false, activate: false, ignore_hide: false, hide_timer: None, pulse_t: None, outro_t: None, potential: None, last_potential: None } } }
impl Popup {
    /// `HandleDamagedResourceObject(DamagedResourceObject, PotentialResource, Destroyed, WeakpointDamage)`.
    pub fn on_event(&mut self, e: HarvestEvent) {
        if self.holding != Some(e.object) {
            // ResetSourceOfResource: new holder, icon from the item, aggregate := stack, PlayAnimationTo(AggregateOutroAnim, 0, 1e-6) jumps the outro to t=0
            // (its OnAnimationFinished -> HideWidget only clears bIgnoreHide, so the flag is consumed right away).
            self.holding = Some(e.object); self.resource = e.resource; self.aggregate = e.amount; self.activate = false; self.ignore_hide = false; self.outro_t = None;
        } else { self.aggregate += e.amount; } // UpdateAggregateResourceCollected
        self.pulse_t = Some(0.); // ResourceIconPulseAnim restarts
        self.potential = Some(Potential { t: 0., amount: e.amount, weak: e.weakpoint }); self.last_potential = Some((e.amount, e.weakpoint)); // ShowPotentialResource
        self.visible = true; self.hide_timer = None; // SetVisibility(4); HBox visible; ClearAndInvalidateTimerHandle
        if e.destroyed { self.activate = true; } else { self.hide_timer = Some(HIDE_DELAY); }
    }
    fn trigger_aggregate_animation(&mut self) { if self.activate { self.activate = false; self.ignore_hide = false; self.outro_t = Some(0.); } }
    fn hide_widget(&mut self) { if self.ignore_hide { self.ignore_hide = false; } else { self.visible = false; self.holding = None; } }
    pub fn tick(&mut self, dt: f32) {
        let dt = dt.max(0.);
        if let Some(t) = self.hide_timer { let t = t - dt; if t <= 0. { self.hide_timer = None; self.activate = true; self.trigger_aggregate_animation(); } else { self.hide_timer = Some(t); } } // ForceHideWidget
        if let Some(t) = self.pulse_t { let t = t + dt; self.pulse_t = if t >= PULSE_LEN { None } else { Some(t) }; }
        if let Some(mut p) = self.potential { p.t += dt; if p.t >= POTENTIAL_LEN { self.potential = None; self.trigger_aggregate_animation(); } else { self.potential = Some(p); } } // FinishedShowing
        if let Some(t) = self.outro_t { let t = t + dt; if t >= OUTRO_LEN { self.outro_t = None; self.hide_widget(); } else { self.outro_t = Some(t); } }
    }
    pub fn active(&self) -> bool { self.visible || self.potential.is_some() }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AggParams { pub hbox_ty: f32, pub hbox_scale: f32, pub text_alpha: f32, pub shadow_alpha: f32, pub icon_alpha: f32, pub icon_scale: f32, pub text_scale: f32 }
/// Animated values of the aggregate row. `outro_t` None = AggregateOutroAnim parked at t=0; `pulse_t` None = ResourceIconPulseAnim at rest.
pub fn agg_params(outro_t: Option<f32>, pulse_t: Option<f32>) -> AggParams {
    let o = outro_t.unwrap_or(0.); let p = pulse_t.unwrap_or(PULSE_LEN);
    AggParams { hbox_ty: eval(&OUTRO_HBOX_TY, o), hbox_scale: eval(&OUTRO_HBOX_SCALE, o), text_alpha: eval(&OUTRO_TEXT_ALPHA, o).clamp(0., 1.), shadow_alpha: eval(&OUTRO_SHADOW_ALPHA, o).clamp(0., 1.), icon_alpha: eval(&OUTRO_ICON_ALPHA, o).clamp(0., 1.),
        icon_scale: eval(&PULSE_ICON_SCALE, p), text_scale: eval(&PULSE_TEXT_SCALE, p) }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PotParams { pub ty: f32, pub shadow_ty: f32, pub alpha: f32, pub shadow_alpha: f32 }
pub fn potential_params(t: f32) -> PotParams { PotParams { ty: eval(&POTENTIAL_TY, t), shadow_ty: eval(&POTENTIAL_SHADOW_TY, t), alpha: eval(&POTENTIAL_ALPHA, t).clamp(0., 1.), shadow_alpha: eval(&POTENTIAL_SHADOW_ALPHA, t).clamp(0., 1.) } }

// ---- layout (pure math; `scale` = DPI scale of the screen) --------------------------------------------------------------------
/// Uniform scale + translation (RenderTransform compositions of scale about the pivot and translation: UMG applies scale, then translation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf { pub k: f32, pub bx: f32, pub by: f32 }
impl Xf {
    pub const ID: Xf = Xf { k: 1., bx: 0., by: 0. };
    /// Scale `k` about pivot `c`, then translate by `t`.
    pub fn about(c: (f32, f32), k: f32, t: (f32, f32)) -> Xf { Xf { k, bx: c.0 - k * c.0 + t.0, by: c.1 - k * c.1 + t.1 } }
    /// Apply `self`, then `o`.
    pub fn then(self, o: Xf) -> Xf { Xf { k: o.k * self.k, bx: o.k * self.bx + o.bx, by: o.k * self.by + o.by } }
    pub fn rect(self, r: Rect) -> Rect { Rect::new(self.k * r.x + self.bx, self.k * r.y + self.by, self.k * r.w, self.k * r.h) }
}
pub fn centre(r: Rect) -> (f32, f32) { (r.x + r.w * 0.5, r.y + r.h * 0.5) }

/// Pixel layout of the popup at rest (before any render transform).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PopupLayout { pub scale: f32, pub widget: Rect, pub potential: Rect, pub hbox: Rect, pub icon: Rect, pub agg_text: Rect }
/// `*_w/_h` are measured text sizes in pixels. Overlay slot: area = screen minus Padding (R400, B300); HAlign Center, VAlign Bottom. Widget = SizeBox(max 200 x 300)
/// around a VerticalBox [Potential (HAlign Right)] [HBox: icon 40 | text].
pub fn popup_layout(w: f32, h: f32, scale: f32, agg_w: f32, agg_h: f32, pot_w: f32, pot_h: f32) -> PopupLayout {
    let s = scale; let icon = ICON_SIZE * s;
    let (hbox_w, hbox_h) = (icon + agg_w, icon.max(agg_h));
    let (ww, hh) = (hbox_w.max(pot_w).min(MAX_WIDTH * s), (pot_h + hbox_h).min(MAX_HEIGHT * s));
    let (area_w, area_h) = (w - SLOT_PADDING_RIGHT * s, h - SLOT_PADDING_BOTTOM * s);
    let (x0, y0) = ((area_w - ww) * 0.5, area_h - hh);
    let hbox = Rect::new(x0, y0 + pot_h, ww, hbox_h);
    PopupLayout { scale: s, widget: Rect::new(x0, y0, ww, hh), potential: Rect::new(x0 + ww - pot_w, y0, pot_w, pot_h), hbox,
        icon: Rect::new(x0, hbox.y + (hbox_h - icon) * 0.5, icon, icon), agg_text: Rect::new(x0 + icon, hbox.y + (hbox_h - agg_h) * 0.5, agg_w, agg_h) }
}

// ---- pickup manager counters (PackResource.UpdateCount, hud_extra.rs of the old tree) -----------------------------------------
/// `FortInventoryItemDefinition.MaxStackSize` of wood / stone / metal.
pub const MAX_STACK: u32 = 999;
/// Resource count text as Slate shows it, clamped to the item's MaxStackSize.
pub fn clamp_resource(n: u32) -> u32 { n.min(MAX_STACK) }
/// PackResource.UpdateCount: `NumericTextCount.InterpolateToValue(ResourceCount, 1, 0.5, 0)` (reports/hud-bytecode/PackResource.txt). The roles of the
/// first and last arguments (exponent / delay) are ASSUMED; the 0.5 s duration and the target are bytecode literals.
pub const COUNT_INTERP_SECS: f32 = 0.5;
/// PackResource.UpdateCount: count text alpha 1.0 while ResourceCount < MaxStackSize, 0.6 at the cap (Select between the literals 1 and 0.6).
pub const COUNT_ALPHA_FULL: f32 = 0.6;
/// Linear count-up of the three pack numbers (see `COUNT_INTERP_SECS`).
#[derive(Clone, Copy, Debug)]
pub struct CountAnim { from: [f32; 3], to: [u32; 3], t: [f32; 3], started: bool }
impl Default for CountAnim { fn default() -> Self { Self { from: [0.; 3], to: [0; 3], t: [COUNT_INTERP_SECS; 3], started: false } } }
impl CountAnim {
    /// Advance by `dt`; a changed target restarts the interpolation from the number currently shown. The first call adopts the target without animating.
    pub fn step(&mut self, target: [u32; 3], dt: f32) {
        if !self.started { self.started = true; self.to = target; self.from = target.map(|v| v as f32); self.t = [COUNT_INTERP_SECS; 3]; return; }
        for i in 0..3 { if target[i] != self.to[i] { self.from[i] = self.value(i, target); self.to[i] = target[i]; self.t[i] = 0.; } self.t[i] = (self.t[i] + dt.max(0.)).min(COUNT_INTERP_SECS); }
    }
    /// Number shown for pack `i` (before any `step` call: the target itself).
    pub fn value(&self, i: usize, target: [u32; 3]) -> f32 {
        if !self.started { return target[i] as f32; }
        let k = (self.t[i] / COUNT_INTERP_SECS).clamp(0., 1.); self.from[i] + (self.to[i] as f32 - self.from[i]) * k
    }
    pub fn shown(&self, i: usize, target: [u32; 3]) -> u32 { self.value(i, target).round().max(0.) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, World, HARVEST_DAMAGE, TREE_HEALTH};
    fn evidence() -> serde_json::Value { serde_json::from_str(&fn_testdata::report_text("reticle-1.8-evidence.json")).unwrap() }
    fn f(v: &serde_json::Value) -> f32 { v.as_f64().unwrap() as f32 }
    fn keys_of(v: &serde_json::Value) -> Vec<Key> { v["keys"].as_array().unwrap().iter().map(|q| Key { t: f(&q["t"]), v: f(&q["v"]), arrive: f(&q["arrive"]), leave: f(&q["leave"]) }).collect() }
    fn same(a: &[Key], b: &[Key], what: &str) { assert_eq!(a.len(), b.len(), "{what}: key count"); for (x, y) in a.iter().zip(b) { assert!((x.t - y.t).abs() < 1e-4 && (x.v - y.v).abs() < 2e-4 && (x.arrive - y.arrive).abs() < 2e-4 && (x.leave - y.leave).abs() < 2e-4, "{what}: {x:?} vs {y:?}"); } }
    fn ev(obj: u64, amount: u32, destroyed: bool, weak: bool) -> HarvestEvent { HarvestEvent { object: obj, resource: Resource::Wood, amount, destroyed, weakpoint: weak } }
    /// `fn_ui::layout::dpi_scale` (UIScaleRule Vertical, curve 540 -> 0.5, 1080 -> 1.0, 2160 -> 2.0, constant outside; CookedIniVersion.txt L531-532), restated so fn-combat needs no UI crate.
    fn dpi(_w: f32, h: f32) -> f32 { let s = (h as i32) as f32; let c = [(540., 0.5f32), (1080., 1.0), (2160., 2.0)]; if s <= c[0].0 { return c[0].1; } for p in c.windows(2) { if s <= p[1].0 { return p[0].1 + (p[1].1 - p[0].1) * (s - p[0].0) / (p[1].0 - p[0].0); } } c[2].1 }

    // ---- evidence ---------------------------------------------------------------------------------------------------
    #[test] fn reticle_constants_match_the_1_8_data() {
        let e = evidence();
        assert_eq!((f(&e["textures"]["HUD_Xhair_Dot"][0]), f(&e["textures"]["HUD_Xhair_Dot"][1])), RETICLE_IMAGE, "FSlateImageBrush size = texture size");
        let c = &e["weapon"]["effective_pickaxe"]["ReticleBuildingColor_FColor_rgba"]; for i in 0..4 { assert_eq!(c[i].as_u64().unwrap() as u8, COLOR_BUILDING[i]); }
        assert_eq!(e["weapon"]["reticle_props_by_blueprint"]["B_Melee_Generic"]["ReticleImage"], "HUD_Xhair_Dot#212");
        let b = &e["weapon"]["reticle_props_by_blueprint"]["B_Melee_Generic"]["ReticleBuildingColor"]; assert_eq!((b["r"].as_u64(), b["g"].as_u64(), b["b"].as_u64(), b["a"].as_u64()), (Some(255), Some(221), Some(0), Some(255)));
        assert!(e["weapon"]["weapon_properties_with_reticle"].as_array().unwrap().is_empty(), "the pickaxe item definition has no reticle properties; everything comes from the weapon class");
        for bp in ["B_Melee_Impact_Pickaxe_Athena", "B_Melee_Impact_Generic", "B_Melee_Generic"] { assert!(e["weapon"]["reticle_props_by_blueprint"][bp].get("HitNotifyReticleImage").is_none(), "{bp}: melee has no hit-marker"); }
        assert!(e["weapon"]["reticle_props_by_blueprint"]["B_Ranged_Generic"].get("HitNotifyReticleImage").is_some(), "only the ranged base class has one");
        assert_eq!(e["reticle_widget"]["widget_tree_widgets"], 0, "Reticle_C is an empty widget blueprint (native SFortReticle paints)");
        assert_eq!(e["reticle_widget"]["hud_element_tags"][0], "HUD.Center.Reticle");
        assert_eq!((e["reticle_widget"]["slot"][0]["hAlign"].as_str(), e["reticle_widget"]["slot"][0]["vAlign"].as_str()), (Some("HAlign_Fill"), Some("VAlign_Fill")));
        let s = &e["pickaxe_stats"]["stats"]; assert_eq!(f(&s["RangeVSBuildings2D"]) / 100., RANGE_2D); assert_eq!(f(&s["RangeVSBuildingsZ"]) / 100., RANGE_Z);
        assert!(WEAKPOINT_LINEAR.iter().zip([0.127563f32, 0.847657, 0.875]).all(|(a, b)| (a - b).abs() < 1e-6));
    }
    #[test] fn popup_constants_match_the_1_8_widgets() {
        let e = evidence(); let p = &e["resource_popup"];
        assert_eq!(p["aggregation_tag"], "HUD.Center.ResourceAggregation");
        assert_eq!((f(&p["aggregation_slot"]["padding"][2]), f(&p["aggregation_slot"]["padding"][3])), (SLOT_PADDING_RIGHT, SLOT_PADDING_BOTTOM));
        assert_eq!((p["aggregation_slot"]["hAlign"].as_str(), p["aggregation_slot"]["vAlign"].as_str()), (Some("HAlign_Center"), Some("VAlign_Bottom")));
        assert_eq!((f(&p["root_sizebox"]["MaxDesiredWidth"]), f(&p["root_sizebox"]["MaxDesiredHeight"])), (MAX_WIDTH, MAX_HEIGHT));
        assert_eq!(p["root_sizebox"]["bOverride_MinDesiredHeight"], false, "MinDesiredHeight 300 is serialised but not enabled");
        assert_eq!(f(&p["icon_designer_brush"]["ImageSize"][0]), ICON_SIZE);
        assert_eq!((f(&p["potential_text"]["NormalFontSize"]), f(&p["potential_text"]["WeakpointFontSize"])), (NORMAL_FONT, WEAKPOINT_FONT));
        assert_eq!(f(&p["aggregate_text"]["shadow_translation"][0]), AGG_SHADOW.0); assert_eq!(f(&p["aggregate_text"]["shadow_translation"][1]), AGG_SHADOW.1);
        assert_eq!(f(&p["potential_text"]["shadow_translation"][0]), POTENTIAL_SHADOW_TX);
        assert!(p["aggregate_text"]["style"].as_str().unwrap().contains("size 32"));
        for (a, b) in p["weakpoint_colour"]["linear_rgb"].as_array().unwrap().iter().zip(WEAKPOINT_LINEAR) { assert!((f(a) - b).abs() < 1e-5); }
    }
    #[test] fn animation_keys_match_the_decoded_movie_scenes() {
        let e = evidence(); let a = &e["resource_popup"]["animations"];
        let o = &a["AggregateOutroAnim"]["bindings"];
        same(&OUTRO_HBOX_TY, &keys_of(&o["HBoxTotalAggregation"]["TranslationY"]), "outro hbox ty");
        same(&OUTRO_HBOX_SCALE, &keys_of(&o["HBoxTotalAggregation"]["ScaleX"]), "outro hbox scale x"); same(&OUTRO_HBOX_SCALE, &keys_of(&o["HBoxTotalAggregation"]["ScaleY"]), "outro hbox scale y");
        same(&OUTRO_TEXT_ALPHA, &keys_of(&o["TextAggregateResourceCollected"]["AlphaCurve"]), "outro text alpha");
        same(&OUTRO_ICON_ALPHA, &keys_of(&o["ImageResourceIcon"]["AlphaCurve"]), "outro icon alpha");
        same(&OUTRO_SHADOW_ALPHA, &keys_of(&o["TextAggregateResourceCollectedShadow"]["AlphaCurve"]), "outro shadow alpha");
        let p = &a["ResourceIconPulseAnim"]["bindings"];
        same(&PULSE_ICON_SCALE, &keys_of(&p["ImageResourceIcon"]["ScaleX"]), "pulse icon"); same(&PULSE_TEXT_SCALE, &keys_of(&p["TextAggregateResourceCollected"]["ScaleX"]), "pulse text");
        same(&PULSE_TEXT_SCALE, &keys_of(&p["TextAggregateResourceCollectedShadow"]["ScaleY"]), "pulse shadow");
        let t = &a["TranslateAndFadeUp"]["bindings"];
        same(&POTENTIAL_TY, &keys_of(&t["TextResourceCount"]["TranslationY"]), "potential ty"); same(&POTENTIAL_ALPHA, &keys_of(&t["TextResourceCount"]["AlphaCurve"]), "potential alpha");
        same(&POTENTIAL_SHADOW_TY, &keys_of(&t["TextResourceCountShadow"]["TranslationY"]), "shadow ty"); same(&POTENTIAL_SHADOW_ALPHA, &keys_of(&t["TextResourceCountShadow"]["AlphaCurve"]), "shadow alpha");
        assert_eq!(f(&t["TextResourceCountShadow"]["TranslationX"]["keys"][0]["v"]), POTENTIAL_SHADOW_TX);
        for (len, v) in [(OUTRO_LEN, &o["HBoxTotalAggregation"]["TranslationY"]), (PULSE_LEN, &p["ImageResourceIcon"]["ScaleX"]), (POTENTIAL_LEN, &t["TextResourceCount"]["TranslationY"])] { assert!((len - f(&v["end"])).abs() < 1e-5, "animation length {len}"); }
        assert_eq!(a["AggregateOutroAnim"]["bindings"]["HBoxTotalAggregation"]["TranslationY"]["keys"][0]["interp"], 2, "all keys are cubic");
    }

    // ---- curve evaluation ---------------------------------------------------------------------------------------------
    #[test] fn cubic_curves_are_smoothstep_with_zero_tangents_and_use_the_arrive_tangent() {
        assert!((eval(&POTENTIAL_TY, 0.5) - -50.).abs() < 1e-4 && (eval(&POTENTIAL_TY, 0.25) - -15.625).abs() < 1e-3, "smoothstep: 3s^2-2s^3 at .25 = .15625");
        assert_eq!(eval(&POTENTIAL_TY, 2.), -100.); assert_eq!(eval(&POTENTIAL_TY, -1.), 0.);
        assert!((eval(&PULSE_ICON_SCALE, 0.1) - 1.5).abs() < 1e-6 && (eval(&PULSE_ICON_SCALE, 0.35) - 1.).abs() < 1e-6 && eval(&PULSE_ICON_SCALE, 0.225) > 1.2);
        assert!((eval(&OUTRO_TEXT_ALPHA, 0.45) - 1.).abs() < 1e-6 && eval(&OUTRO_TEXT_ALPHA, 1.).abs() < 1e-6 && (eval(&OUTRO_TEXT_ALPHA, 0.725) - 0.5).abs() < 1e-3, "0.45..1.0 is a smoothstep down to 0");
        assert!(eval(&OUTRO_TEXT_ALPHA, 0.3) >= 1., "the arrive tangent overshoots slightly above 1 (clamped when applied)");
        assert!((eval(&OUTRO_HBOX_SCALE, 0.05) - 1.3).abs() < 1e-6 && (eval(&OUTRO_HBOX_SCALE, 0.15) - 1.).abs() < 1e-6 && (eval(&OUTRO_HBOX_TY, 1.) + 120.).abs() < 1e-5);
    }
    #[test] fn animated_parameters_at_rest_and_at_key_times() {
        let r = agg_params(None, None); assert_eq!((r.hbox_ty, r.hbox_scale, r.text_alpha, r.shadow_alpha, r.icon_alpha, r.icon_scale, r.text_scale), (0., 1., 1., 0.5, 1., 1., 1.));
        let o = agg_params(Some(1.), None); assert!(o.text_alpha == 0. && o.icon_alpha == 0. && o.shadow_alpha == 0. && (o.hbox_ty + 120.).abs() < 1e-4);
        let p = agg_params(None, Some(0.1)); assert!((p.icon_scale - 1.5).abs() < 1e-6 && (p.text_scale - 1.3).abs() < 1e-6);
        let q = potential_params(0.); assert_eq!((q.ty, q.shadow_ty, q.alpha, q.shadow_alpha), (0., 2., 1., 0.5));
        let q = potential_params(1.); assert!((q.ty + 100.).abs() < 1e-4 && (q.shadow_ty + 98.).abs() < 1e-4 && q.alpha == 0. && q.shadow_alpha == 0.);
    }

    // ---- state machine (bytecode) -----------------------------------------------------------------------------------
    #[test] fn consecutive_hits_on_one_object_aggregate_and_hide_three_seconds_after_the_last() {
        let mut p = Popup::default(); assert!(!p.active());
        p.on_event(ev(7, 20, false, false)); assert!(p.visible && p.aggregate == 20 && p.holding == Some(7) && p.hide_timer == Some(3.) && p.pulse_t == Some(0.) && p.potential.unwrap().amount == 20);
        p.tick(0.5); p.on_event(ev(7, 20, false, false)); assert_eq!(p.aggregate, 40); assert_eq!(p.hide_timer, Some(3.), "timer restarted by the second hit");
        p.tick(2.9); assert!(p.outro_t.is_none() && p.visible, "{p:?}");
        p.tick(0.2); assert_eq!(p.outro_t.map(|t| t > 0.), Some(true)); assert!(!p.activate, "TriggerAggregateAnimation consumed bActivateAggregateAnim");
        p.tick(1.01); assert!(!p.visible && p.holding.is_none() && p.outro_t.is_none(), "AggregateOutroAnim finished -> HideWidget");
    }
    #[test] fn destroying_hit_waits_for_the_popup_then_plays_the_outro_without_a_timer() {
        let mut p = Popup::default(); p.on_event(ev(1, 20, true, false)); assert!(p.activate && p.hide_timer.is_none());
        p.tick(0.9); assert!(p.outro_t.is_none() && p.potential.is_some(), "outro waits for PotentialResourceWidget.FinishedShowing (1.0 s)");
        p.tick(0.2); assert!(p.potential.is_none() && p.outro_t.is_some() && !p.activate);
        p.tick(1.1); assert!(!p.visible);
    }
    #[test] fn a_different_object_resets_the_aggregate() {
        let mut p = Popup::default(); p.on_event(ev(1, 20, false, false)); p.tick(0.1); p.on_event(ev(1, 20, false, false)); assert_eq!(p.aggregate, 40);
        p.on_event(HarvestEvent { object: 2, resource: Resource::Stone, amount: 20, destroyed: false, weakpoint: false }); assert_eq!((p.aggregate, p.holding, p.resource), (20, Some(2), Resource::Stone));
        p.on_event(ev(2, 0, false, true)); assert!(p.potential.unwrap().weak && p.last_potential == Some((0, true)));
    }
    #[test] fn pulse_restarts_per_hit_and_lasts_035_seconds() {
        let mut p = Popup::default(); p.on_event(ev(1, 20, false, false)); p.tick(0.3); assert!(p.pulse_t.is_some()); p.on_event(ev(1, 20, false, false)); assert_eq!(p.pulse_t, Some(0.)); p.tick(0.36); assert!(p.pulse_t.is_none());
    }
    #[test] fn world_strikes_emit_hud_events_with_stable_object_ids() {
        let mut w = World { props: vec![Harvestable::new(Kind::Tree, Vec3::new(0., 0., -3.))], ..Default::default() }; let eye = Vec3::new(0., 0., -1.);
        // 1.8 numbers: 600 HP tree / 50 damage = 12 swings (was 5 with the old provisional 100 HP / 20)
        let n = (TREE_HEALTH / HARVEST_DAMAGE).ceil() as usize;
        for _ in 0..n { assert!(w.strike(eye, Vec3::NEG_Z)); }
        assert_eq!(w.events.len(), n); assert!(w.events.iter().all(|e| e.object == 0 && e.resource == Resource::Wood && e.amount == HARVEST_DAMAGE as u32 && !e.weakpoint));
        assert_eq!(w.events.iter().filter(|e| e.destroyed).count(), 1, "only the last hit destroys the prop"); assert!(w.events[n - 1].destroyed);
        w.update(9.); w.events.clear(); assert!(w.strike(eye, Vec3::NEG_Z)); assert_ne!(w.events[0].object, 0, "a respawned prop is a new ResourceHoldingObject");
        let mut p = Popup::default(); for e in w.events.drain(..) { p.on_event(e); } assert!(p.visible && w.events.is_empty());
    }

    // ---- layout ---------------------------------------------------------------------------------------------------------
    #[test] fn reticle_is_centred_and_texture_sized_at_every_aspect() {
        for (w, h) in [(1920., 1080.), (1280., 720.), (2560., 1080.), (1024., 768.), (640., 480.), (800., 1280.), (3440., 1440.)] {
            let s = dpi(w, h); let r = reticle_rect(w, h, s);
            assert!((r.w - 58. * s).abs() < 1e-3 && (r.h - 54. * s).abs() < 1e-3, "{w}x{h}");
            assert!((r.x + r.w * 0.5 - w * 0.5).abs() <= 0.5 + 1e-3 && (r.y + r.h * 0.5 - h * 0.5).abs() <= 0.5 + 1e-3, "{w}x{h} centre {:?}", r);
            assert_eq!(r.x, r.x.round()); assert_eq!(r.y, r.y.round());
        }
        assert_eq!(reticle_rect(1920., 1080., 1.), Rect::new(931., 513., 58., 54.));
    }
    #[test] fn reticle_colour_white_until_the_aim_ray_finds_a_prop_within_pickaxe_reach() {
        assert_eq!(reticle_tint(false), [1., 1., 1., 1.]); let y = reticle_tint(true); assert_eq!((y[0], y[2], y[3]), (1., 0., 1.)); assert!((y[1] - 221. / 255.).abs() < 1e-6);
        let tree = Harvestable::new(Kind::Tree, Vec3::new(0., 0., 10.2)); let pawn = Vec3::new(0., 1.6, 12.); let eye = Vec3::new(0.7, 2.0, 14.5); // third-person camera behind/right of the pawn
        let toward = |t: Vec3| (t - eye).normalize();
        assert!(aim_on_target(&[tree.clone()], eye, toward(Vec3::new(0., 1.5, 10.2)), pawn), "tree 1.4 m ahead of the pawn");
        assert!(!aim_on_target(&[tree.clone()], eye, toward(Vec3::new(8., 1.5, 10.2)), pawn), "aiming away");
        let far = Harvestable::new(Kind::Tree, Vec3::new(0., 0., 4.)); assert!(!aim_on_target(&[far], eye, toward(Vec3::new(0., 1.5, 4.)), pawn), "8 m away is out of the 2.88 m reach");
        assert!(!aim_on_target(&[], eye, toward(Vec3::new(0., 1.5, 10.2)), pawn), "nothing there");
        let mut dead = tree; dead.health = 0.; assert!(!aim_on_target(&[dead], eye, toward(Vec3::new(0., 1.5, 10.2)), pawn), "a harvested prop is gone");
    }
    #[test] fn popup_sits_left_of_centre_and_300_units_above_the_bottom() {
        let l = popup_layout(1920., 1080., 1., 80., 44., 40., 22.);
        assert_eq!(l.scale, 1.); assert_eq!(l.widget.y + l.widget.h, 1080. - 300.); assert!((l.widget.x + l.widget.w * 0.5 - (1920. - 400.) * 0.5).abs() < 1e-3);
        assert_eq!(l.widget.w, 120.); assert_eq!(l.widget.h, 22. + 44.);
        assert_eq!(l.potential.x + l.potential.w, l.widget.x + l.widget.w, "Potential is right aligned"); assert_eq!(l.potential.y, l.widget.y);
        assert_eq!(l.hbox.y, l.potential.y + 22.); assert_eq!((l.icon.w, l.icon.h), (40., 40.)); assert_eq!(l.agg_text.x, l.icon.x + 40.);
        assert!((l.icon.y + 20. - (l.hbox.y + 22.)).abs() < 1e-3, "icon vertically centred in the row");
        let c = popup_layout(1920., 1080., 1., 400., 44., 40., 22.); assert_eq!(c.widget.w, 200., "SizeBox MaxDesiredWidth 200");
        let s = popup_layout(1280., 720., dpi(1280., 720.), 80., 44., 40., 22.); assert!((s.widget.y + s.widget.h - (720. - 300. * (2. / 3.))).abs() < 1e-2 && s.scale < 0.7);
    }
    #[test] fn transforms_compose_scale_about_pivot_then_translation() {
        let r = Rect::new(10., 10., 20., 10.); let c = centre(r); let x = Xf::about(c, 2., (5., 0.)).rect(r);
        assert!((x.x - (c.0 - 20. + 5.)).abs() < 1e-4 && (x.w - 40.).abs() < 1e-4 && ((x.x + x.w * 0.5) - (c.0 + 5.)).abs() < 1e-4, "{x:?}");
        let both = Xf::about(c, 1., (0., -10.)).then(Xf::about(c, 1.2, (0., 0.))).rect(r); assert!((both.y + both.h * 0.5 - (c.1 + 1.2 * -10.)).abs() < 1e-3, "child translation is scaled by the parent's 1.2: {both:?}");
        assert_eq!(Xf::ID.rect(r), r);
    }
    #[test] fn weakpoint_colour_is_cyan() { let c = weakpoint_color(); assert!(c[1] > 0.9 && c[2] > 0.9 && c[0] < 0.5 && c[0] > 0.3, "{c:?}"); assert_eq!(srgb_encode(0.), 0.); assert!((srgb_encode(1.) - 1.).abs() < 1e-6); }

    // ---- pickup manager counters -------------------------------------------------------------------------------------
    #[test] fn resource_counts_clamp_to_max_stack_and_dim_at_the_cap() { assert_eq!(clamp_resource(0), 0); assert_eq!(clamp_resource(999), 999); assert_eq!(clamp_resource(5000), 999); assert_eq!(COUNT_ALPHA_FULL, 0.6); }
    #[test] fn pack_counters_count_up_linearly_over_half_a_second() {
        let mut c = CountAnim::default(); c.step([5, 0, 0], 0.016); assert_eq!(c.shown(0, [5, 0, 0]), 5, "the first target is adopted without animating");
        c.step([25, 0, 0], 0.0); assert_eq!(c.shown(0, [25, 0, 0]), 5, "a new target restarts from the number shown");
        c.step([25, 0, 0], 0.25); assert_eq!(c.shown(0, [25, 0, 0]), 15); c.step([25, 0, 0], 0.25); assert_eq!(c.shown(0, [25, 0, 0]), 25); c.step([25, 0, 0], 5.); assert_eq!(c.shown(0, [25, 0, 0]), 25);
        c.step([25, 3, 0], 0.1); assert_eq!(c.shown(1, [25, 3, 0]), 1, "each pack counts up on its own clock");
    }
}
