//! The data contract of the 1.8 "world feedback" HUD pieces: pickaxe reticle, harvest `+N` popups, Target Info bar, interaction prompt, equip progress.
//!
//! PLAIN DATA ONLY (no behaviour, no renderer): the game side fills a [`HudWorld`] each frame and hands it over inside
//! [`crate::screens::hud::HudState::world`]; `Hud::set_state` consumes the harvest hits, `Hud::present` draws the rest.
//!
//! This module is the ONE place the contract is defined. The fn-combat / fn-hud-adapter side already has the same shape under other names
//! (`fn_combat::HarvestEvent { object, resource, amount, destroyed, weakpoint }`), so unifying is a field-for-field map:
//!  * `HarvestHit`  <- `fn_combat::HarvestEvent` (`HarvestResource` <- `fn_combat::Resource`),
//!  * `ReticleState::on_target` <- the aim-ray test (`fn_ui::screens::reticle::aim_on_target` is a renderer-free helper for it, or the adapter's own),
//!  * `current_resource` <- `FortInventoryContext.GetCurrentResourceItemDefinition()` (ASSUMED wood while the sandbox has no build-material choice).
//! When the adapter grows its own `ReticleState` / `HarvestPopups`, replace these structs by re-exports; the field names are chosen to match.

/// The three harvestable resources (index = pack order Wood | Stone | Metal).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum HarvestResource { #[default] Wood, Stone, Metal }
impl HarvestResource { pub fn index(self) -> usize { match self { HarvestResource::Wood => 0, HarvestResource::Stone => 1, HarvestResource::Metal => 2 } } }

/// One harvest hit: the arguments of `FortHUDContext.OnDamagedResourceBuilding(object, item, destroyed, weakpoint)` as the popups use them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HarvestHit {
    /// Identity of the damaged prop (`ResourceHoldingObject`); a respawned prop must get a new id.
    pub object: u64,
    pub resource: HarvestResource,
    /// Resources gained by this hit (the potential-resource stack count).
    pub amount: u32,
    /// This hit destroyed the prop (the aggregate popup then waits for the `+N` animation instead of the 3 s timer).
    pub destroyed: bool,
    /// Weak-point hit: the `+N` text uses size 20 and the `Color-HarvestWeakPoint` cyan.
    pub weakpoint: bool,
}

/// Reticle inputs. The picture and colour rule live in `screens::reticle`; the game only says whether the aim ray finds a prop in reach.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReticleState {
    /// `SFortReticle::UpdateColor`: the weapon's aim trace found an actor within its range -> `ReticleBuildingColor` (255, 221, 0) instead of white.
    pub on_target: bool,
}

/// `EFortBuildingHealthDisplayRule` (BuildingInfoIndicator bytecode, `UpdateBuildingInfo`): 0 Never, 1 = shown after damage (value names INFERRED
/// from the visibility switch: only the damage handler makes the bar visible for rule 1), 2 Always.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HealthDisplayRule { #[default] Never, OnDamage, Always }

/// `FortFocusedBuildingInfo` as far as the Target Info bar reads it (BuildingInfoIndicator.UpdateBuildingInfo / HandleBuildingHealthChanged).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TargetInfo {
    /// Identity of the focused building (`FocusedBuilding`); a change of identity resets the bar (HandleBuildingFocused).
    pub id: u64,
    pub health: f32,
    pub max_health: f32,
    pub rule: HealthDisplayRule,
    /// `EFortBuildingInteraction` as its number (0 None; 1 Build and 2 Repair are the only values the bytecode names by number; the Target Info
    /// bar's colours / text mode are switched on it, see `screens::target_info::DisplayMode`).
    pub interaction: u8,
    /// Screen position (pixels, top-left origin) of the indicator's world anchor (actor location + `IndicatorRelativeLocation`, projected by the
    /// game's camera); None = the screen centre.
    pub screen: Option<(f32, f32)>,
}

/// An interaction prompt (`InteractionIndicator` showing `BasicInteractionWidget`): the key cap + a description.
#[derive(Clone, Debug, PartialEq)]
pub struct InteractionPrompt {
    /// `KeybindWidget` text of the bound action `Use` (e.g. "E").
    pub key: String,
    /// `FortInteraction.ContextText` (`BasicInteractionWidget.Description`).
    pub text: String,
    /// Projected screen position of the interacted actor (pixels); None = the screen centre. The widget is offset by (65, -65) units from it
    /// (`ScreenSpaceRelativeOffset`).
    pub screen: Option<(f32, f32)>,
}

/// `GameplayCue.Athena.Equipping` (WhileActive, magnitude = duration) as `AthenaEquipProgress` shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EquipProgress {
    /// Seconds since the cue started (`GetGameTimeInSeconds - StartTime`).
    pub elapsed: f32,
    /// `Parameters.RawMagnitude`; the widget draws nothing while this is 0.
    pub duration: f32,
}

/// Original sniper scope post-process input. The image is the exact texture bound to the recovered `Reticle` material parameter;
/// `amount` is the recovered `DownSightPostProcessAmount` timeline value.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeOverlay { pub image: String, pub amount: f32 }

/// Everything the world feedback widgets need for one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudWorld {
    pub reticle: ReticleState,
    /// Native `SFortReticle` image for a ranged weapon. The texture is an original cooked UI asset; `None` means melee / a consumable.
    pub firearm_reticle: Option<String>,
    /// One or more real ranged hit-registration impacts occurred since the previous HUD frame. `SFortReticle` restarts its
    /// `B_Ranged_Generic.HitNotifyReticleImage` timer from this event; it is deliberately not emitted for melee swings or misses.
    pub firearm_hit: bool,
    pub scope_overlay: Option<ScopeOverlay>,
    /// Harvest hits since the last `Hud::set_state` (the producer clears its queue every frame; `set_state` consumes and empties this vector).
    pub harvest_hits: Vec<HarvestHit>,
    /// Pack of `GetCurrentResourceItemDefinition()` (gets the highlighted border). ASSUMED wood.
    pub current_resource: HarvestResource,
    pub target: Option<TargetInfo>,
    pub interaction: Option<InteractionPrompt>,
    pub equip: Option<EquipProgress>,
    /// The held firearm's ammo counter (AthenaEquippedItemWidget); None = the pickaxe / nothing ranged (the widget's switcher is collapsed).
    pub ammo: Option<crate::screens::equipped_item::EquippedAmmo>,
    /// WIDGETS3: the running cooldown of the item in each primary quickbar slot (`QuickbarSlotCooldown`, see `screens::slot_cooldown`).
    pub slot_cooldowns: [Option<crate::screens::slot_cooldown::SlotCooldown>; 6],
}
impl HudWorld {
    /// Non-finite numbers repaired (a producer bug must never poison the draw list).
    pub fn sanitized(&self) -> HudWorld {
        let mut w = self.clone();
        let fin = |v: f32| if v.is_finite() { v } else { 0. };
        for h in &mut w.harvest_hits { h.amount = h.amount.min(99_999); }
        if let Some(t) = w.target.as_mut() {
            t.health = fin(t.health).max(0.); t.max_health = fin(t.max_health).max(0.);
            t.screen = t.screen.filter(|p| p.0.is_finite() && p.1.is_finite());
        }
        if let Some(i) = w.interaction.as_mut() { i.screen = i.screen.filter(|p| p.0.is_finite() && p.1.is_finite()); }
        if let Some(e) = w.equip.as_mut() { e.elapsed = fin(e.elapsed).max(0.); e.duration = fin(e.duration).max(0.); }
        for c in w.slot_cooldowns.iter_mut().flatten() { c.remaining = fin(c.remaining).max(0.); c.duration = fin(c.duration).max(0.); }
        if w.firearm_reticle.as_ref().is_some_and(|name| name.is_empty()) { w.firearm_reticle = None; }
        if let Some(scope) = w.scope_overlay.as_mut() {
            scope.amount = fin(scope.amount).clamp(0., 1.);
            if scope.image.is_empty() || scope.amount <= 0. { w.scope_overlay = None; }
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn defaults_are_a_quiet_hud() { let w = HudWorld::default(); assert!(!w.reticle.on_target && w.harvest_hits.is_empty() && w.target.is_none() && w.interaction.is_none() && w.equip.is_none() && w.current_resource == HarvestResource::Wood); }
    #[test] fn resource_index_is_the_pack_order() { assert_eq!([HarvestResource::Wood, HarvestResource::Stone, HarvestResource::Metal].map(HarvestResource::index), [0, 1, 2]); }
    #[test] fn sanitizing_repairs_non_finite_values() {
        let mut w = HudWorld::default();
        w.target = Some(TargetInfo { id: 1, health: f32::NAN, max_health: f32::INFINITY, rule: HealthDisplayRule::Always, interaction: 0, screen: Some((f32::NAN, 1.)) });
        w.equip = Some(EquipProgress { elapsed: f32::NAN, duration: -3. });
        let s = w.sanitized(); let t = s.target.unwrap(); assert_eq!((t.health, t.max_health, t.screen), (0., 0., None)); assert_eq!(s.equip.unwrap(), EquipProgress { elapsed: 0., duration: 0. });
    }
}
