//! Glue between the harvest state (`fn_combat::harvest_fx::Fx`: weak spots, strike events, damage numbers) and the FX pass: owns the GPU resources of the ring and the
//! numbers, turns `FxEvent`s / new `DamageNumber`s into live effects and produces the [`FxFrame`] of every rendered frame. The application calls [`FxWorld::load`] once (original
//! pak install), [`FxWorld::events`] with the drained `FxEvent`s, and [`FxWorld::frame`] once per rendered frame before `Renderer::render`.
use crate::gpu::{SharedBuffers, TextureCache};
use crate::numbers::{draws as number_draws, LiveNumber, NumberAssets, SLOTS};
use crate::ring::{draws as ring_draws, RingAssets, RingEffect};
use crate::view::{bytes, sprite_vf_buffer, ViewParams, VIEW_FLOAT4S};
use fn_combat::harvest_fx::{DamageNumber, Fx, FxEvent};
use fn_pak::PakSet;
use fn_render::fx::*;
use glam::Vec3;
use std::collections::HashMap;

pub const RING_SLOTS: usize = 6;

pub struct FxWorld {
    ring: RingAssets,
    numbers: NumberAssets,
    view_buf: FxBufId,
    light_buf: FxBufId,
    ring_prims: Vec<FxBufId>,
    ring_binds: HashMap<(String, usize), Vec<(u32, FxBindId)>>,
    effects: Vec<RingEffect>,
    live: Vec<LiveNumber>,
    seen_hits: u32,
    seed: u64,
    /// Human readable notes of what could not be loaded / drawn (logged by the application).
    pub notes: Vec<String>,
}

impl FxWorld {
    pub fn load(set: &PakSet, fx: &mut FxRenderer) -> Result<FxWorld, String> {
        let view_buf = fx.create_buffer("fx view", VIEW_FLOAT4S * 16);
        let ring_prims: Vec<FxBufId> = (0..RING_SLOTS).map(|_| fx.create_buffer("fx ring primitive", 16 * 16)).collect();
        let sprite_vf = fx.create_buffer("fx sprite vf", 10 * 16);
        fx.write_buffer(sprite_vf, &bytes(&sprite_vf_buffer()));
        let light_buf = fx.create_buffer("fx light", 48);
        let shared = SharedBuffers { view: view_buf, primitive: ring_prims[0], sprite_vf, light: light_buf };
        let mut cache = TextureCache::default();
        let ring = RingAssets::load(set, fx, &mut cache, shared)?;
        let numbers = NumberAssets::load(set, fx, &mut cache, shared)?;
        let mut ring_binds = HashMap::new();
        for (path, m) in &ring.materials { for (s, b) in ring_prims.iter().enumerate() { ring_binds.insert((path.clone(), s), m.bind_variant(fx, *b, None)); } }
        let notes = ring.notes.clone();
        Ok(FxWorld { ring, numbers, view_buf, light_buf, ring_prims, ring_binds, effects: vec![], live: vec![], seen_hits: 0, seed: 1, notes })
    }

    fn free_ring_slot(&self) -> usize { (0..RING_SLOTS).find(|s| !self.effects.iter().any(|e| e.slot == *s)).unwrap_or(0) }

    /// `CrackHitCount == 0` is the first spot of a prop; relocated spots (after a weak-point hit) are not.
    pub fn events(&mut self, events: &[FxEvent]) {
        for e in events {
            self.seed = self.seed.wrapping_add(0x9E37_79B9);
            match *e {
                FxEvent::WeakSpotSpawned { object, pos } => { let slot = self.free_ring_slot(); self.effects.retain(|x| x.slot != slot); self.effects.push(RingEffect::spawn(&self.ring, object, pos, true, slot, self.seed)); }
                FxEvent::WeakSpotHit { object, pos, moved_to, .. } => {
                    // the hit spot starts its ScaleDown timeline; a relocated spot appears at once
                    let hit = self.effects.iter_mut().filter(|x| x.object == object && x.hit_at.is_none()).min_by(|a, b| (a.pos_ue - crate::view::to_ue(pos)).length().partial_cmp(&(b.pos_ue - crate::view::to_ue(pos)).length()).unwrap());
                    if let Some(h) = hit { h.hit(); }
                    if let Some(n) = moved_to { let slot = self.free_ring_slot(); self.effects.retain(|x| x.slot != slot); self.effects.push(RingEffect::spawn(&self.ring, object, n, false, slot, self.seed)); }
                }
                FxEvent::Broke { object, .. } => self.effects.retain(|x| x.object != object || x.hit_at.is_some()),
                FxEvent::Hit { .. } => {}
            }
        }
    }

    /// New damage numbers since the last call (`Fx::numbers` keeps the newest entries; the strike counters say how many were added).
    pub fn numbers_from(&mut self, fx_state: &Fx, view: &ViewParams, now: f32) {
        let total = fx_state.critical_hits + fx_state.normal_hits;
        let new = (total.saturating_sub(self.seen_hits)) as usize;
        self.seen_hits = total;
        let fresh: Vec<DamageNumber> = fx_state.numbers.iter().rev().take(new).cloned().collect();
        for n in fresh.into_iter().rev() {
            let slot = (0..SLOTS).find(|s| !self.live.iter().any(|l| l.slot == *s && l.alive(now))).unwrap_or_else(|| self.live.iter().min_by(|a, b| a.spawn_time.partial_cmp(&b.spawn_time).unwrap()).map_or(0, |l| l.slot));
            self.live.retain(|l| l.slot != slot);
            self.seed = self.seed.wrapping_add(0x9E37_79B9);
            self.live.push(LiveNumber::spawn(&self.numbers, slot, n.pos, n.amount, n.critical, view, now, self.seed));
        }
    }

    /// Advances the effects by `dt` and builds the FX frame for `view`.
    pub fn frame(&mut self, view: &ViewParams, dt: f32, now: f32) -> FxFrame {
        let mut frame = FxFrame::default();
        let eye_cm = crate::view::to_ue(view.eye);
        for e in &mut self.effects { e.update(&self.ring, dt, (eye_cm - e.pos_ue).length() as f32); }
        self.effects.retain(|e| !e.finished);
        self.live.retain(|l| l.alive(now));
        frame.buffer_writes.push((self.view_buf, bytes(&view.view_buffer())));
        let (s, r, k) = (fn_render::lighting::sun_direction(), fn_render::lighting::sun_radiance(), fn_render::lighting::sky_radiance());
        frame.buffer_writes.push((self.light_buf, bytes(&[[s.x, s.y, s.z, 0.], [r[0], r[1], r[2], 0.], [k[0], k[1], k[2], 0.]])));
        for e in &self.effects { ring_draws(&self.ring, e, view, &self.ring_binds, &self.ring_prims, &mut frame); }
        number_draws(&self.numbers, &self.live, now, &mut frame);
        frame
    }
    pub fn active_effects(&self) -> (usize, usize) { (self.effects.len(), self.live.len()) }
    #[allow(dead_code)]
    fn _v(_: Vec3) {}
}
