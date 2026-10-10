//! `BuildingWorld`: the set of placed building pieces. Stable API shared by the building agents
//! (visuals read it, editing swaps classes, structural/damage changes health and removes pieces). See `../API.md`.
//!
//! Space: UE axes, centimetres, X/Y horizontal, Z up, the SAME space as `fn_movement::scene` (scene X/Y = sim X / sim Z - `Z_OFFSET`,
//! scene Z = sim Y, x100). The renderer's FX space (`fn_fx::view::to_ue`) is this + (0, `Z_OFFSET` * 100, 0)
//! (`fn-app/src/building_hook.rs::world_to_fx`). The build grid is anchored on the Athena LEVEL origin expressed in this space
//! (`crate::placement::PlacementEnv::grid_origin`), not on 0. `location` is the actor location (`AActor::GetActorLocation`) and
//! `yaw_deg` the actor yaw (UE `FRotator::Yaw`, degrees).
use std::collections::HashMap;

use glam::{IVec3, Vec3};

pub use crate::pieces::{Material, PieceKind};

/// Stable handle of a placed piece. Never reused within one `BuildingWorld`.
pub type PieceId = u32;

/// Grid slot a piece occupies. `cell` is the build-grid cell (see `crate::grid`); `rot` is the yaw in quarter turns (0..=3).
/// Walls are keyed by their canonical edge (`cell` = edge lattice index, `rot` = edge axis; see `crate::grid::key_for`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GridKey {
    pub kind: PieceKind,
    pub cell: IVec3,
    pub rot: u8,
}

/// Build-up state (`ABuildingSMActor::HealthToAutoBuild` +0xC70 / `AccumulatedAutoBuildTime` +0xC74, see [`BuildingWorld::tick`]).
/// `duration` is the class `BuildTime`; `elapsed` counts time since placement (for visuals); the piece is building while
/// `health_to_build > 0`.
#[derive(Clone, Copy, Debug)]
pub struct BuildProgress {
    pub elapsed: f32,
    pub duration: f32,
    pub start_health: f32,
    /// `HealthToAutoBuild`: health still to be added (MaxHealth - start at spawn).
    pub health_to_build: f32,
    /// `AccumulatedAutoBuildTime`.
    pub accumulated: f32,
}

impl BuildProgress {
    /// 0..=1 build fraction (1 = finished): time since placement over `BuildTime`, 1 once the health build-up has ended.
    pub fn fraction(&self) -> f32 {
        if self.done() || self.duration <= 0. { 1. } else { (self.elapsed / self.duration).clamp(0., 1.) }
    }
    pub fn done(&self) -> bool { self.health_to_build <= 0. }
}

/// `AccumulatedAutoBuildTime` threshold of one build-up step (DAT_143feb1d8 = 0.5 s, `ABuildingSMActor::Tick` @1404967f0).
pub const AUTO_BUILD_STEP: f32 = 0.5;

/// Edit state (written by the editing module via [`BuildingWorld::set_edit`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditState {
    /// Replacement building class (the edited BP, e.g. `/Game/.../PBWA_W1_Windows.PBWA_W1_Windows_C`). `None` = the
    /// piece's original `class_path`.
    pub class_path: Option<String>,
    /// `ServerEditBuildingActor(.., RotationIterations, bMirrored)`.
    pub rotation_iterations: i32,
    pub mirrored: bool,
}

/// EDITING (additive, editing agent): the new class's numbers for [`BuildingWorld::rebuild_for_edit`] (`fn_building::edit::data::ClassBuild`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditRebuild {
    pub max_health: f32,
    pub initial_health_pct: f32,
    pub build_time: f32,
    /// The class has an `AnimatingMaterial` (`ABuildingSMActor::vfunc_370`: +0xc40 != null).
    pub animates: bool,
}

#[derive(Clone, Debug)]
pub struct PlacedPiece {
    pub id: PieceId,
    pub kind: PieceKind,
    pub material: Material,
    /// The placed BP class (object path of the generated class, `..._C`).
    pub class_path: String,
    pub key: GridKey,
    pub location: Vec3,
    pub yaw_deg: f32,
    pub health: f32,
    pub max_health: f32,
    pub build: BuildProgress,
    pub edit: EditState,
    /// World time (s) at which the piece was placed.
    pub placed_time: f64,
}

impl PlacedPiece {
    /// The class currently in effect (edit replacement or the original class).
    pub fn effective_class(&self) -> &str { self.edit.class_path.as_deref().unwrap_or(&self.class_path) }
    pub fn is_building(&self) -> bool { !self.build.done() }
    pub fn health_fraction(&self) -> f32 { if self.max_health > 0. { (self.health / self.max_health).clamp(0., 1.) } else { 0. } }
}

/// What a caller passes to [`BuildingWorld::spawn`]. Placement validation happens before this (see `crate::placement`).
#[derive(Clone, Debug)]
pub struct PieceSpawn {
    pub kind: PieceKind,
    pub material: Material,
    pub class_path: String,
    pub key: GridKey,
    pub location: Vec3,
    pub yaw_deg: f32,
    pub max_health: f32,
    pub start_health: f32,
    pub build_time: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorldEvent {
    Placed(PieceId),
    /// Build-up finished this tick.
    Built(PieceId),
    HealthChanged { id: PieceId, old: f32, new: f32 },
    Edited(PieceId),
    Removed(PieceId),
}

/// Result of [`BuildingWorld::apply_damage`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DamageOutcome {
    NoSuchPiece,
    Damaged { health: f32 },
    /// Health reached 0; the piece was removed (a `Removed` event was queued).
    Destroyed,
}

const EVENT_CAP: usize = 8192;

#[derive(Default)]
pub struct BuildingWorld {
    pieces: Vec<PlacedPiece>,
    index: HashMap<PieceId, usize>,
    by_key: HashMap<GridKey, PieceId>,
    next_id: PieceId,
    revision: u64,
    time: f64,
    events: Vec<WorldEvent>,
    /// Sequence number of `events[0]`.
    events_base: u64,
}

impl BuildingWorld {
    pub fn new() -> Self { BuildingWorld { next_id: 1, ..Default::default() } }

    /// Bumped on every change (placement, build tick, health, edit, removal). Cheap change detection for renderers.
    pub fn revision(&self) -> u64 { self.revision }
    /// World time (s) accumulated by [`tick`](Self::tick).
    pub fn time(&self) -> f64 { self.time }
    pub fn len(&self) -> usize { self.pieces.len() }
    pub fn is_empty(&self) -> bool { self.pieces.is_empty() }
    pub fn iter(&self) -> impl Iterator<Item = &PlacedPiece> { self.pieces.iter() }
    pub fn get(&self, id: PieceId) -> Option<&PlacedPiece> { self.index.get(&id).map(|&i| &self.pieces[i]) }
    /// Mutable access. Prefer the dedicated setters (they emit events); if you mutate directly call [`touch`](Self::touch).
    pub fn get_mut(&mut self, id: PieceId) -> Option<&mut PlacedPiece> {
        let i = *self.index.get(&id)?;
        self.revision += 1;
        Some(&mut self.pieces[i])
    }
    pub fn touch(&mut self) { self.revision += 1; }
    pub fn at_key(&self, key: &GridKey) -> Option<PieceId> { self.by_key.get(key).copied() }
    pub fn is_occupied(&self, key: &GridKey) -> bool { self.by_key.contains_key(key) }

    /// Adds a piece (no validation). Returns its id. A piece already on the same `key` is replaced (removed first).
    pub fn spawn(&mut self, s: PieceSpawn) -> PieceId {
        if let Some(old) = self.by_key.get(&s.key).copied() { self.remove(old); }
        let id = self.next_id;
        self.next_id += 1;
        let start = s.start_health.min(s.max_health);
        let p = PlacedPiece {
            id, kind: s.kind, material: s.material, class_path: s.class_path, key: s.key, location: s.location, yaw_deg: s.yaw_deg,
            health: start, max_health: s.max_health,
            build: BuildProgress { elapsed: 0., duration: s.build_time.max(0.), start_health: start, health_to_build: if s.build_time > 0. { (s.max_health - start).max(0.) } else { 0. }, accumulated: 0. },
            edit: EditState::default(), placed_time: self.time,
        };
        self.index.insert(id, self.pieces.len());
        self.by_key.insert(p.key, id);
        self.pieces.push(p);
        self.revision += 1;
        self.push(WorldEvent::Placed(id));
        if s.build_time <= 0. {
            let p = self.get_mut(id).unwrap();
            p.health = p.max_health;
            self.push(WorldEvent::Built(id));
        }
        id
    }

    pub fn remove(&mut self, id: PieceId) -> Option<PlacedPiece> {
        let i = self.index.remove(&id)?;
        let p = self.pieces.swap_remove(i);
        if i < self.pieces.len() { let moved = self.pieces[i].id; self.index.insert(moved, i); }
        if self.by_key.get(&p.key) == Some(&id) { self.by_key.remove(&p.key); }
        self.revision += 1;
        self.push(WorldEvent::Removed(id));
        Some(p)
    }

    /// Sets health (clamped to 0..=max). Does not remove at 0: call [`remove`](Self::remove) (or use [`apply_damage`](Self::apply_damage)).
    pub fn set_health(&mut self, id: PieceId, health: f32) -> bool {
        let Some(&i) = self.index.get(&id) else { return false };
        let p = &mut self.pieces[i];
        let old = p.health;
        p.health = health.clamp(0., p.max_health);
        let new = p.health;
        self.revision += 1;
        if old != new { self.push(WorldEvent::HealthChanged { id, old, new }); }
        true
    }

    /// Subtracts `amount`; removes the piece when health reaches 0.
    pub fn apply_damage(&mut self, id: PieceId, amount: f32) -> DamageOutcome {
        let Some(p) = self.get(id) else { return DamageOutcome::NoSuchPiece };
        let h = (p.health - amount).max(0.);
        self.set_health(id, h);
        if h <= 0. { self.remove(id); DamageOutcome::Destroyed } else { DamageOutcome::Damaged { health: h } }
    }

    /// Editing: swap the class / rotation / mirror of a piece.
    pub fn set_edit(&mut self, id: PieceId, edit: EditState) -> bool {
        let Some(&i) = self.index.get(&id) else { return false };
        self.pieces[i].edit = edit;
        self.revision += 1;
        self.push(WorldEvent::Edited(id));
        true
    }

    /// EDITING (additive, editing agent; see `API.md`): an edit replaces the actor (`ReplaceBuildingActor`,
    /// `ABuildingSMActor::vfunc_329` @14048afb0) and the new actor is initialised with InitReason 2 and the old actor
    /// (`ABuildingSMActor::vfunc_247` @14047bed0). When the new class has a BuildTime > 0 (`vfunc_371` @14046c580) and an
    /// `AnimatingMaterial` (`vfunc_370`), it plays the construct animation (`MulticastRequestBuildingAnimation(4)`) and builds up
    /// through the normal path ([`tick`](Self::tick)):
    /// * health = `FUN_14046f440` (new MaxHealth * InitialHealthPct) * old `GetHealthPercent()` (clamped 0..1);
    /// * `HealthToAutoBuild` (vfunc_389) = new MaxHealth * max(old fraction, (old Health + old HealthToAutoBuild) / old MaxHealth)
    ///   - the start health above;
    /// * the rate base stays MaxHealth * InitialHealthPct (`vfunc_372`), so `build.start_health` = `initial_health`.
    /// Otherwise the new actor keeps its spawn health (MaxHealth) and is built. Call BEFORE [`set_edit`](Self::set_edit) (whose
    /// `Edited` event then sees `is_building()`). Returns false if `id` does not exist.
    pub fn rebuild_for_edit(&mut self, id: PieceId, r: EditRebuild) -> bool {
        let Some(&i) = self.index.get(&id) else { return false };
        let now = self.time;
        let p = &mut self.pieces[i];
        let old = p.health;
        let old_frac = p.health_fraction();
        let old_total = if p.max_health > 0. { (p.health + p.build.health_to_build.max(0.)) / p.max_health } else { 0. };
        if r.build_time > 0. && r.animates {
            let start = r.max_health * r.initial_health_pct * old_frac;
            let htab = r.max_health * old_total.max(old_frac) - start;
            p.max_health = r.max_health;
            p.health = start.clamp(0., r.max_health);
            p.build = BuildProgress { elapsed: 0., duration: r.build_time, start_health: r.max_health * r.initial_health_pct, health_to_build: htab, accumulated: 0. };
        } else {
            p.max_health = r.max_health;
            p.health = r.max_health;
            p.build = BuildProgress { elapsed: 0., duration: r.build_time.max(0.), start_health: r.max_health, health_to_build: 0., accumulated: 0. };
        }
        // a new actor
        p.placed_time = now;
        let new = p.health;
        self.revision += 1;
        if old != new { self.push(WorldEvent::HealthChanged { id, old, new }); }
        true
    }

    /// Advances the build-up of every piece still building, as `ABuildingSMActor::Tick` @1404967f0 does on the server:
    /// `AccumulatedAutoBuildTime += dt`; once it reaches 0.5 s: `amount = CeilToInt(rate * accumulated)` with
    /// `rate = (MaxHealth - MaxHealth * InitialHealthPct) / BuildTime` (vfunc_372 @14046bda0), accumulated = 0, heal
    /// `min(amount, HealthToAutoBuild)` (health clamped to MaxHealth, a "Healing" effect), `HealthToAutoBuild -= amount`;
    /// the build-up ends when health == MaxHealth or HealthToAutoBuild < 0. Damage taken while building is kept.
    pub fn tick(&mut self, dt: f32) {
        self.time += dt as f64;
        let mut built = Vec::new();
        let mut changed = Vec::new();
        for p in self.pieces.iter_mut() {
            if p.build.done() { continue; }
            p.build.elapsed += dt;
            p.build.accumulated += dt;
            if p.build.accumulated < AUTO_BUILD_STEP { continue; }
            let d = p.build.duration;
            let rate = if d > 0. { (p.max_health - p.build.start_health) / d } else { 0. };
            let amount = (rate * p.build.accumulated).ceil();
            p.build.accumulated = 0.;
            let heal = amount.min(p.build.health_to_build);
            let old = p.health;
            let new = (p.health + heal).max(p.health).min(p.max_health);
            p.health = new;
            p.build.health_to_build -= amount;
            if p.health == p.max_health || p.build.health_to_build < 0. { p.build.health_to_build = 0.; }
            if p.health != old { changed.push(WorldEvent::HealthChanged { id: p.id, old, new: p.health }); }
            if p.build.done() { built.push(WorldEvent::Built(p.id)); }
        }
        if !changed.is_empty() || !built.is_empty() { self.revision += 1; }
        for e in changed.into_iter().chain(built) { self.push(e); }
    }

    /// Events with sequence number >= `cursor`, and the cursor to pass next time. Each consumer keeps its own cursor
    /// (start at 0). The log keeps the most recent events only; a consumer that falls far behind skips the oldest.
    pub fn events_since(&self, cursor: u64) -> (&[WorldEvent], u64) {
        let start = cursor.max(self.events_base);
        let off = (start - self.events_base) as usize;
        let off = off.min(self.events.len());
        (&self.events[off..], self.events_base + self.events.len() as u64)
    }

    fn push(&mut self, e: WorldEvent) {
        if self.events.len() >= EVENT_CAP {
            let drop = EVENT_CAP / 2;
            self.events.drain(..drop);
            self.events_base += drop as u64;
        }
        self.events.push(e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spawn(w: &mut BuildingWorld, x: i32) -> PieceId {
        w.spawn(PieceSpawn {
            kind: PieceKind::Floor, material: Material::Wood, class_path: "C".into(),
            key: GridKey { kind: PieceKind::Floor, cell: IVec3::new(x, 0, 0), rot: 0 },
            location: Vec3::ZERO, yaw_deg: 0., max_health: 100., start_health: 10., build_time: 1.,
        })
    }
    #[test]
    fn build_up_and_damage() {
        let mut w = BuildingWorld::new();
        let a = spawn(&mut w, 0);
        let b = spawn(&mut w, 1);
        w.tick(0.4);
        assert_eq!(w.get(a).unwrap().health, 10.); // nothing before 0.5 s accumulates
        w.tick(0.1);
        assert!((w.get(a).unwrap().health - 55.).abs() < 1e-3); // ceil(90 / 1 * 0.5) = 45
        w.tick(0.5);
        assert_eq!(w.get(a).unwrap().health, 100.);
        assert!(!w.get(a).unwrap().is_building());
        assert_eq!(w.apply_damage(a, 150.), DamageOutcome::Destroyed);
        assert!(w.get(a).is_none());
        assert!(w.get(b).is_some());
        let (ev, cur) = w.events_since(0);
        assert!(ev.contains(&WorldEvent::Removed(a)));
        assert_eq!(w.events_since(cur).0.len(), 0);
    }
}
