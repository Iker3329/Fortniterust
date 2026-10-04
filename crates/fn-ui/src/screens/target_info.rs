//! Target Info: the health bar of the focused building / prop (`BuildingInfoIndicator` + its `BuildingBar` widget, HUD row "Target Info" = tag
//! `HUD.InWorld.BuildingInfoIndicator`).
//!
//! SOURCES (decoded from the 1.8 paks with fn-inspect / the old Kismet dumps `reports/hud-bytecode/BuildingInfoIndicator.txt` and `BuildingBar.txt`):
//! * `BuildingInfoIndicator_C` (parent `FortActorIndicatorWidget`, MaxDistance 1000, canvas = `[Actor Canvas] Indicators`) binds `FortHUDContext.OnBuildingFocused /
//!   OnFocusedBuildingStateChanged / OnFocusedBuildingHealthChanged` and drives the nested `BuildingBar_C` ("[Widget] Building HP & Trap Bars").
//!   - `HandleBuildingFocused(actor)`: indicator Visible(4) when the actor is valid else Collapsed(1); `Current Health = 0`; `BuildingInfo` := defaults (rule Never, max 0,
//!     interaction None); health timer cleared.
//!   - `UpdateBuildingInfo(info)`: `Update Max HP Value(MaxHealth)`, `Set Display Mode(InteractionType)`, bar visibility = switch(`HealthDisplayRule`): Never -> Hidden(2), Always ->
//!     SelfHitTestInvisible(4), rule 1 -> by interaction type (None -> Hidden, 1..5 -> visible, 6 -> Hidden); health timer cleared.
//!   - `HandleBuildingHealthChanged(h)`: if `h != Current Health`: `Update Current HP Value(h)`; for rule 1 and `h < MaxHealth` the bar becomes Visible(4) and, for interaction
//!     None, `HideHealth` (-> Hidden(2)) is scheduled 3 s later (`K2_SetTimer("HideHealth", 3, false)`).
//! * `BuildingBar_C` is the same artwork as the player's hit-point bar at a fixed size: `[Size Box] HP Bar` 288 x 24 (VerticalBoxSlot padding 0,6,0,6): baseplate
//!   `T-Border-HUD-HitPointBar-Bg` (Box margin 0.4, tint (0,0.00121,0.01033,0.6)), `[ProgressBar] HP Delta` then `[ProgressBar] HP Fill` (`HUD-Bar_Material`, Fill ColorA
//!   (0.07619,0.50289,0.0356) / ColorB (0.17789,0.73046,0.08022), Delta ColorA (0.783538,0.05448,0.05448) / ColorB (1,0.165132,0.165132)), lead-edge bevel
//!   (`T-Border-HUD-HitPointBar-LeadEdge`, SizeBox width = Current/Max * 288, alpha min(1, width / (24*1.2))), stat text overlay padding Left 16: "{cur} | {max}"
//!   (TextStyle-Base-XS-B = NotoSans-Bold 16, `Conv_IntToText(FCeil(v), grouping)`, divider = `Border_HUD_Line_Vert` 4 wide with padding 4,2,4,2) or "{x} HP" in build modes.
//! * `Set Display Mode` (7 modes, bytecode switch tables) recolours the bar: [`DisplayMode`].
//! * `Update Current HP Value` writes `Last HP Value := Current` (the NEW value) before starting the fade timer, so the Delta bar never trails in 1.8 (the lerp in
//!   `Update` finds Last == Current): implemented literally, the delta width always equals the fill width. (The player's own bar is different.)
//!
//! NOT RECOVERED / APPROXIMATE: where on screen the indicator sits (the native `SFortActorIndicator` projection / alignment is not serialised): the bar's CENTRE is put
//! on the producer's projected anchor (`TargetInfo::screen`, default screen centre) and the invisible `TrapVerticalBox` slot above the bar (Hidden keeps layout space) is
//! ignored; display modes 1 and 6 use fill colours (0,0,0,0) which would draw black with the opaque `HUD-Bar_Material` alpha: drawn as no fill here. Whether resource props
//! (trees, rocks) use `HealthDisplayRule::OnDamage` is a property of the actor class and not in the HUD data: the producer decides.
use crate::draw::{MaterialKind, RenderList, TextureId};
use crate::layout::{dpi_scale, Rect};
use crate::scene::UiContext;
use crate::screens::reticle_state::{HealthDisplayRule, TargetInfo};
use crate::screens::{art, ascent, atlas_px, line_height, mat_tex, put_text, srgb, srgb3, Snap, WHITE};
use crate::text::FontId;

pub const BAR_W: f32 = 288.; // [Size Box] HP Bar WidthOverride
pub const BAR_H: f32 = 24.; // HeightOverride
pub const BAR_PAD_V: f32 = 6.; // VerticalBoxSlot_0 Padding (0,6,0,6)
pub const TEXT_LEFT: f32 = 16.; // OverlaySlot_2 padding Left
pub const FONT_SIZE: f32 = 16.; // TextStyle-Base-XS-B
pub const TEXT_TOP: f32 = -2.5; // stat text slots Padding.Top
pub const DIVIDER: f32 = 4.; // SizeBox_1 WidthOverride (Border_HUD_Line_Vert)
pub const DIVIDER_PAD: (f32, f32) = (4., 2.); // HorizontalBoxSlot_1 padding (4,2,4,2)
pub const BEVEL_FADE: f32 = 1.2;
pub const BEVEL_W: f32 = 24.;
pub const HIDE_DELAY: f32 = 3.; // HandleBuildingHealthChanged: SetTimer("HideHealth", 3, false)
pub const FADE_UPDATE_RATE: f32 = 0.05; // HP FadeUpdateRate (timer period and lerp alpha of `Update`)
pub const PLATE_MARGIN: [f32; 4] = [0.4, 0., 0.4, 0.];
pub const PLATE_TINT: [f32; 4] = [0., 0.00121, 0.01033, 0.6]; // Default Baseplate Color (linear RGB, alpha)
pub const ALT_PLATE_TINT: [f32; 4] = [1., 1., 1., 0.8]; // Alternate Baseplate Color (never selected by Set Display Mode)
pub const EDGE_MARGIN: [f32; 4] = [0., 0., 0.9, 0.];
pub const LINE_VERT_ALPHA: f32 = 0.2; // M_Border_HUD_Line_Vert Global Alpha
pub const DEFAULT_A: [f32; 3] = [0.07619, 0.50289, 0.0356];
pub const DEFAULT_B: [f32; 3] = [0.17789, 0.73046, 0.08022];
pub const DEFAULT_C: [f32; 3] = [0.47932, 0.87962, 0.23074];
pub const ALT_A: [f32; 3] = [0.66539, 0.2462, 0.02956];
pub const ALT_B: [f32; 3] = [0.82279, 0.55201, 0.0648];
pub const ALT_C: [f32; 3] = [0.9131, 0.76052, 0.14996];
pub const DELTA_A: [f32; 3] = [0.783538, 0.05448, 0.05448]; // Initialize Bar: [ProgressBar] HP Delta ColorA
pub const DELTA_B: [f32; 3] = [1., 0.165132, 0.165132]; // ... ColorB
/// The designer-default bevel tint (`ImageHPBevel.ColorAndOpacity`) that stays until `Set Display Mode` runs with a mode other than 0.
pub const DESIGNER_BEVEL: [f32; 3] = [0.32778, 0.80695, 0.0185];
/// `EFortBuildingInteraction` values used for the display switch (the bytecode switches over 0..=6).
pub const MODES: usize = 7;

/// The 7-way tables of `BuildingBar.Set Display Mode`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayMode {
    /// HUD-Bar_Material ColorA / ColorB of `[ProgressBar] HP Fill`; None = `(0,0,0,0)` (see the module docs).
    pub fill: Option<([f32; 3], [f32; 3])>,
    /// ImageHPBevel tint (linear RGB + alpha); `(0,0,0,0)` for modes 1 (alpha 0.2: `(0,0,0,0.2)`) and 6.
    pub bevel: [f32; 4],
    /// `SwitcherHPText` index: 0 = "{cur} | {max}", 1 = "{x} HP".
    pub text_index: u8,
    /// `[Size Box] HP Bevel` and `[ProgressBar] HP Delta` visible.
    pub bevel_and_delta: bool,
}
pub fn display_mode(mode: u8) -> DisplayMode {
    let m = (mode as usize).min(MODES - 1);
    let alt = matches!(m, 2 | 5);
    let none = matches!(m, 1 | 6);
    DisplayMode {
        fill: if none { None } else if alt { Some((ALT_A, ALT_B)) } else { Some((DEFAULT_A, DEFAULT_B)) },
        bevel: match m { 1 => [0., 0., 0., 0.2], 6 => [0., 0., 0., 0.], 2 | 5 => [ALT_C[0], ALT_C[1], ALT_C[2], 1.], _ => [DEFAULT_C[0], DEFAULT_C[1], DEFAULT_C[2], 1.] },
        text_index: if matches!(m, 1 | 6) { 1 } else { 0 },
        bevel_and_delta: !matches!(m, 1 | 6),
    }
}
/// Bar visibility chosen by `UpdateBuildingInfo` (true = Visible(4), false = Hidden(2)).
pub fn bar_visible_for(rule: HealthDisplayRule, interaction: u8) -> bool {
    match rule { HealthDisplayRule::Never => false, HealthDisplayRule::Always => true, HealthDisplayRule::OnDamage => (1..=5).contains(&interaction) }
}

/// `Conv_IntToText(FCeil(v), grouping = true, min digits 1)`: "1,234".
pub fn int_text(v: f32) -> String {
    let n = v.max(0.).ceil() as u64; let s = n.to_string(); let mut out = String::new();
    for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); } out.push(c); }
    out
}

/// The widget's state (BuildingInfoIndicator + BuildingBar variables).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetInfoHud {
    /// `IndicatedActor` identity; None = Collapsed.
    pub focused: Option<u64>,
    /// `BuildingInfo` as last applied (max health, rule, interaction).
    pub max_health: f32, pub rule: HealthDisplayRule, pub interaction: u8,
    /// `Current Health` (indicator) / `Current HP Value`, `Last HP Value`, `Max HP Value` (bar; the CDO defaults are Max 1, Last 1000).
    pub current_health: f32, pub bar_current: f32, pub bar_last: f32, pub bar_max: f32,
    /// `[Widget] Building HP & Trap Bars` visibility (true = Visible(4), false = Hidden(2)).
    pub bar_visible: bool,
    /// `HealthFadeTimerHandle` (HideHealth in seconds).
    pub hide_timer: Option<f32>,
    /// `BuildingBar.DisplayMode` as applied; None = designer defaults (`Set Display Mode(0)` returns early on the equal default).
    pub mode: Option<u8>,
    /// Anchor in pixels from the last `update`.
    pub anchor: Option<(f32, f32)>,
}
impl Default for TargetInfoHud {
    fn default() -> Self { Self { focused: None, max_health: 0., rule: HealthDisplayRule::Never, interaction: 0, current_health: 0., bar_current: 0., bar_last: 1000., bar_max: 1., bar_visible: false, hide_timer: None, mode: None, anchor: None } }
}
impl TargetInfoHud {
    pub fn new() -> Self { Self::default() }
    /// Indicator visible (an actor is focused).
    pub fn active(&self) -> bool { self.focused.is_some() }
    /// Is the bar drawn right now?
    pub fn shown(&self) -> bool { self.active() && self.bar_visible }

    fn handle_focused(&mut self, id: Option<u64>) {
        self.focused = id;
        self.current_health = 0.; self.max_health = 0.; self.rule = HealthDisplayRule::Never; self.interaction = 0;
        self.hide_timer = None;
        if id.is_none() { self.bar_visible = false; }
    }
    fn update_building_info(&mut self, t: &TargetInfo) {
        self.max_health = t.max_health; self.rule = t.rule; self.interaction = t.interaction;
        // BuildingBar.Update Max HP Value (early out on an equal value)
        if t.max_health != self.bar_max { self.bar_max = t.max_health; }
        // BuildingBar.Set Display Mode (early out on the current mode; the initial mode is 0)
        let cur = self.mode.unwrap_or(0);
        if t.interaction != cur { self.mode = Some(t.interaction); }
        self.bar_visible = bar_visible_for(t.rule, t.interaction);
        self.hide_timer = None; // K2_ClearAndInvalidateTimerHandle(HealthFadeTimerHandle)
    }
    fn health_changed(&mut self, h: f32) {
        if h == self.current_health { return; }
        self.current_health = h;
        // BuildingBar.Update Current HP Value: Last := Current (the new value), Current := value
        if h != self.bar_current { self.bar_last = h; self.bar_current = h; }
        if self.rule == HealthDisplayRule::OnDamage && self.current_health < self.max_health {
            self.bar_visible = true;
            if self.interaction == 0 { self.hide_timer = Some(HIDE_DELAY); }
        }
    }
    /// Per-frame input: the focused building (None = nothing focused) as the game reports it.
    pub fn update(&mut self, target: Option<&TargetInfo>, w: f32, h: f32) {
        match target {
            None => { if self.focused.is_some() { self.handle_focused(None); } self.anchor = None; }
            Some(t) => {
                let new_focus = self.focused != Some(t.id);
                if new_focus { self.handle_focused(Some(t.id)); }
                if new_focus || t.max_health != self.max_health || t.rule != self.rule || t.interaction != self.interaction { self.update_building_info(t); }
                self.health_changed(t.health);
                self.anchor = Some(t.screen.unwrap_or((w * 0.5, h * 0.5)));
            }
        }
    }
    /// Game-time step: the `HideHealth` timer.
    pub fn tick(&mut self, dt: f32) {
        if let Some(t) = self.hide_timer { let t = t - dt.max(0.); if t <= 0. { self.hide_timer = None; self.bar_visible = false; } else { self.hide_timer = Some(t); } }
    }
}

/// Pixel rectangle of the 288x24 bar centred on `anchor`.
pub fn bar_rect(anchor: (f32, f32), scale: f32) -> Rect { Rect::new((anchor.0 - BAR_W * scale * 0.5).round(), (anchor.1 - BAR_H * scale * 0.5).round(), BAR_W * scale, BAR_H * scale) }

/// Textures of the bar (resolved lazily).
#[derive(Clone, Copy)]
struct TargetTex { plate: crate::assets::TexInfo, edge: crate::assets::TexInfo, line: TextureId }

impl TargetInfoHud {
    fn tex(ctx: &mut UiContext) -> TargetTex {
        TargetTex { plate: art(ctx, "T-Border-HUD-HitPointBar-Bg.png"), edge: art(ctx, "T-Border-HUD-HitPointBar-LeadEdge.png"), line: mat_tex(ctx, "T-Border-HUD-Line-Vert.png", false).id }
    }
    /// Draw the bar when shown.
    pub fn build(&self, ctx: &mut UiContext, w: f32, h: f32, out: &mut RenderList) {
        let (true, Some(anchor)) = (self.shown(), self.anchor) else { return };
        let tex = Self::tex(ctx);
        let s = dpi_scale(w, h);
        let row = bar_rect(anchor, s);
        let mode = display_mode(self.mode.unwrap_or(0));
        let designer = self.mode.is_none();
        let max = self.bar_max.max(0.);
        let frac = |v: f32| if max <= 0. { 0. } else { (v / max).clamp(0., 1.) };
        // 1. baseplate (Default Baseplate Color for every mode)
        out.nine_slice(tex.plate, row, PLATE_MARGIN, s, [srgb(PLATE_TINT[0]), srgb(PLATE_TINT[1]), srgb(PLATE_TINT[2]), PLATE_TINT[3]]);
        // 2. Delta then Fill (HUD-Bar_Material, ColorA left -> ColorB right over the filled quad)
        let fill_frac = frac(self.bar_current); let delta_frac = frac(self.bar_last);
        if mode.bevel_and_delta { let d = Rect::new(row.x, row.y, row.w * delta_frac, row.h); if d.w > 0.5 { out.material(MaterialKind::HudBar { a: DELTA_A, b: DELTA_B }, d, WHITE, false); } }
        let f = Rect::new(row.x, row.y, row.w * fill_frac, row.h);
        if let Some((a, b)) = mode.fill { if f.w > 0.5 { out.material(MaterialKind::HudBar { a, b }, f, WHITE, false); } }
        // 3. lead-edge bevel: SizeBox width = Padding.Right (0) + Current/Max * 288; alpha = min(1, width / (24 * 1.2))
        if mode.bevel_and_delta && f.w > 0.5 {
            let width_units = fill_frac * BAR_W;
            let alpha = if width_units <= BEVEL_W * BEVEL_FADE { width_units / (BEVEL_W * BEVEL_FADE) } else { 1. };
            let rgb = if designer { DESIGNER_BEVEL } else { [mode.bevel[0], mode.bevel[1], mode.bevel[2]] };
            let c = srgb3(rgb);
            // (the bevel is only visible in the modes whose table colour has alpha 1, so the alpha is the fade formula alone)
            out.nine_slice(tex.edge, Rect::new(f.x, row.y + (row.h - BEVEL_W * s) * 0.5, f.w, BEVEL_W * s), EDGE_MARGIN, s, [c[0], c[1], c[2], alpha]);
        }
        // 4. stat text (SwitcherHPText): overlay slot Padding.Left 16, VAlign centre, text slots Padding.Top -2.5
        let px = atlas_px(FONT_SIZE, s);
        let lh = line_height(ctx, FontId::Bold, px); let asc = ascent(ctx, FontId::Bold, px);
        let base = (row.y + (row.h - lh) * 0.5 + asc + (TEXT_TOP * 0.5 * px as f32 / FONT_SIZE)).round();
        let x0 = row.x + TEXT_LEFT * s;
        if mode.text_index == 0 {
            let (cur, max_t) = (int_text(self.bar_current), int_text(self.bar_max));
            let cur_w = ctx.fonts.measure(FontId::Bold, &cur, px as f32);
            put_text(ctx, out, FontId::Bold, px, x0, base, &cur, WHITE, None, Snap::Pen);
            let cell_x = x0 + cur_w;
            let line_h = ((lh + TEXT_TOP * s) - 2. * DIVIDER_PAD.1 * s).max(2.);
            let div = Rect::new(cell_x + DIVIDER_PAD.0 * s, row.y + (row.h - (line_h + 2. * DIVIDER_PAD.1 * s)) * 0.5 + DIVIDER_PAD.1 * s, DIVIDER * s, line_h);
            if tex.line != TextureId::NONE { out.material(MaterialKind::PanelLine { mask: tex.line, global_alpha: LINE_VERT_ALPHA }, div, WHITE, false); }
            put_text(ctx, out, FontId::Bold, px, cell_x + (DIVIDER + 2. * DIVIDER_PAD.0) * s, base, &max_t, WHITE, None, Snap::Pen);
        } else {
            put_text(ctx, out, FontId::Bold, px, x0, base, &format!("{} HP", int_text(self.bar_max)), WHITE, None, Snap::Pen);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::Cmd;
    use crate::screens::testutil::ctx;
    fn info(id: u64, health: f32, max: f32, rule: HealthDisplayRule) -> TargetInfo { TargetInfo { id, health, max_health: max, rule, interaction: 0, screen: None } }
    fn mats(l: &RenderList) -> Vec<&crate::draw::MaterialCmd> { l.cmds.iter().filter_map(|c| if let Cmd::Material(m) = c { Some(m) } else { None }).collect() }

    #[test] fn constants_are_the_decoded_building_bar_numbers() {
        // BuildingBar.txt: [Size Box] HP Bar 288x24, slot padding 0,6,0,6, text overlay left 16, FadeUpdateRate 0.05, colours of Default__BuildingBar_C
        assert_eq!((BAR_W, BAR_H, BAR_PAD_V, TEXT_LEFT, FADE_UPDATE_RATE), (288., 24., 6., 16., 0.05));
        assert_eq!(DEFAULT_A, [0.07619, 0.50289, 0.0356]); assert_eq!(DEFAULT_B, [0.17789, 0.73046, 0.08022]); assert_eq!(DEFAULT_C, [0.47932, 0.87962, 0.23074]);
        assert_eq!(ALT_A, [0.66539, 0.2462, 0.02956]); assert_eq!(ALT_B, [0.82279, 0.55201, 0.0648]); assert_eq!(ALT_C, [0.9131, 0.76052, 0.14996]);
        assert_eq!(DELTA_B, [1., 0.165132, 0.165132]); assert_eq!(DELTA_A, [0.783538, 0.05448, 0.05448]);
        assert!(PLATE_TINT.iter().zip(crate::screens::vitals::PLATE_TINT).all(|(a, b)| (a - b).abs() < 1e-3), "same baseplate as the player's bar (the player's constants are the rounded values)");
        assert_eq!((DEFAULT_A, DEFAULT_B), (crate::screens::vitals::HEALTH.fill_a, crate::screens::vitals::HEALTH.fill_b), "default HP colours are the player's health colours");
        assert!(DELTA_A.iter().chain(&DELTA_B).zip(crate::screens::vitals::HEALTH.delta_a.iter().chain(&crate::screens::vitals::HEALTH.delta_b)).all(|(a, b)| (a - b).abs() < 1e-4), "delta colours are the player's (full precision here)");
        assert_eq!(HIDE_DELAY, 3.);
    }
    #[test] fn display_mode_tables_follow_the_set_display_mode_bytecode() {
        let m0 = display_mode(0); assert_eq!(m0.fill, Some((DEFAULT_A, DEFAULT_B))); assert_eq!((m0.text_index, m0.bevel_and_delta), (0, true));
        for m in [2u8, 5] { let d = display_mode(m); assert_eq!(d.fill, Some((ALT_A, ALT_B)), "mode {m}"); assert_eq!(d.bevel, [ALT_C[0], ALT_C[1], ALT_C[2], 1.]); assert_eq!(d.text_index, 0); }
        for m in [3u8, 4] { assert_eq!(display_mode(m).fill, Some((DEFAULT_A, DEFAULT_B)), "mode {m}"); }
        for m in [1u8, 6] { let d = display_mode(m); assert!(d.fill.is_none() && d.text_index == 1 && !d.bevel_and_delta, "mode {m}: {{x}} HP text, bevel/delta hidden"); }
        assert_eq!(display_mode(1).bevel, [0., 0., 0., 0.2]); assert_eq!(display_mode(6).bevel, [0., 0., 0., 0.]);
        assert_eq!(display_mode(200), display_mode(6), "out of range clamps to the last case");
    }
    #[test] fn visibility_switch_of_update_building_info() {
        use HealthDisplayRule::*;
        for i in 0..7 { assert!(!bar_visible_for(Never, i)); assert!(bar_visible_for(Always, i)); }
        let on: Vec<bool> = (0..7).map(|i| bar_visible_for(OnDamage, i)).collect();
        assert_eq!(on, [false, true, true, true, true, true, false], "Temp_byte_Variable2..11 literals: 2 / 4 per interaction type");
    }
    #[test] fn int_text_groups_thousands() { assert_eq!(int_text(0.), "0"); assert_eq!(int_text(599.2), "600"); assert_eq!(int_text(1000.), "1,000"); assert_eq!(int_text(1234567.), "1,234,567"); assert_eq!(int_text(-4.), "0"); }

    #[test] fn on_damage_props_show_the_bar_after_a_hit_and_hide_it_three_seconds_later() {
        let mut t = TargetInfoHud::new();
        t.update(Some(&info(1, 600., 600., HealthDisplayRule::OnDamage)), 1920., 1080.);
        assert!(t.active() && !t.shown(), "full health, rule OnDamage, interaction None -> Hidden");
        assert_eq!((t.bar_current, t.bar_max), (600., 600.));
        t.update(Some(&info(1, 550., 600., HealthDisplayRule::OnDamage)), 1920., 1080.);
        assert!(t.shown() && t.hide_timer == Some(3.));
        t.tick(2.9); assert!(t.shown()); t.update(Some(&info(1, 500., 600., HealthDisplayRule::OnDamage)), 1920., 1080.); assert_eq!(t.hide_timer, Some(3.), "SetTimer restarts");
        t.tick(3.05); assert!(!t.shown() && t.hide_timer.is_none());
        assert_eq!((t.bar_current, t.bar_last), (500., 500.), "Last := new value: no delta trail");
    }
    #[test] fn always_rule_shows_immediately_and_never_hides() {
        let mut t = TargetInfoHud::new(); t.update(Some(&info(2, 100., 100., HealthDisplayRule::Always)), 1280., 720.); assert!(t.shown());
        t.update(Some(&info(2, 60., 100., HealthDisplayRule::Always)), 1280., 720.); assert!(t.shown() && t.hide_timer.is_none()); t.tick(10.); assert!(t.shown());
        let mut n = TargetInfoHud::new(); n.update(Some(&info(3, 100., 100., HealthDisplayRule::Never)), 1280., 720.); n.update(Some(&info(3, 10., 100., HealthDisplayRule::Never)), 1280., 720.); assert!(!n.shown(), "Never stays hidden even when damaged");
    }
    #[test] fn losing_the_focus_collapses_and_a_new_building_resets_the_bar() {
        let mut t = TargetInfoHud::new(); t.update(Some(&info(1, 600., 600., HealthDisplayRule::Always)), 1920., 1080.); assert!(t.shown());
        t.update(None, 1920., 1080.); assert!(!t.active() && !t.shown() && t.current_health == 0.);
        t.update(Some(&info(9, 200., 300., HealthDisplayRule::Always)), 1920., 1080.); assert_eq!((t.bar_current, t.bar_max), (200., 300.));
        t.update(Some(&info(10, 50., 50., HealthDisplayRule::OnDamage)), 1920., 1080.); assert!(!t.shown(), "new focus: rule re-evaluated, hidden until damaged");
    }
    #[test] fn bar_is_centred_on_the_anchor_at_288_by_24_units() {
        let r = bar_rect((960., 540.), 1.); assert_eq!(r, Rect::new(816., 528., 288., 24.));
        let r = bar_rect((1000., 500.), 0.666); assert!((r.w - 288. * 0.666).abs() < 1e-3 && ((r.x + r.w * 0.5) - 1000.).abs() <= 0.5);
    }
    #[test] fn building_bar_draws_baseplate_delta_fill_bevel_text_and_divider() {
        let Some(mut c) = ctx() else { return };
        let mut t = TargetInfoHud::new();
        t.update(Some(&info(1, 300., 600., HealthDisplayRule::Always)), 1920., 1080.);
        let mut out = RenderList::new(); t.build(&mut c, 1920., 1080., &mut out);
        let m = mats(&out);
        let bars: Vec<_> = m.iter().filter(|m| matches!(m.kind, MaterialKind::HudBar { .. })).collect();
        assert_eq!(bars.len(), 2, "delta + fill");
        assert!(bars.iter().all(|b| (b.rect.w - 144.).abs() < 1e-3 && b.rect.h == 24.), "half of 288");
        assert!(matches!(bars[0].kind, MaterialKind::HudBar { a, .. } if a == DELTA_A) && matches!(bars[1].kind, MaterialKind::HudBar { a, .. } if a == DEFAULT_A), "delta first, fill on top");
        assert_eq!(m.iter().filter(|m| matches!(m.kind, MaterialKind::PanelLine { .. })).count(), 1, "one divider between current and max");
        assert!(out.quad_count() >= 2 + 6, "baseplate + bevel (L/M/R cells) + digits: {}", out.quad_count());
        // hidden -> nothing
        let mut h = TargetInfoHud::new(); h.update(Some(&info(1, 600., 600., HealthDisplayRule::OnDamage)), 1920., 1080.); let mut o2 = RenderList::new(); h.build(&mut c, 1920., 1080., &mut o2); assert!(o2.is_empty());
    }
}
