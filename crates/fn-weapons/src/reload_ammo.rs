//! Reload and ammo transfer, ported from the original 1.8 executable (static evidence, read-only; no client trace oracle).
//!
//! All RVAs are image-relative (image base 0x140000000). Weapon object offsets: `+0x3d0` item definition, `+0x9b8` AmmoCount (replicated),
//! `+0x9bc` server ammo, `+0x99c` time of the last ammo transfer, `+0x9a0` CurrentReloadDuration, `+0xa68` reload montage.
//!
//! Recovered chain (`UFortGameplayAbility_Reload`, native; vtable RVA 0x3ff7048, ActivateAbility slot 0x2a0):
//!
//! * `AFortWeapon::CanReload` (vslot 0x7b8, RVA 0x91ab30): needs an ammo definition on the item, `!IsReloading`, inventory ammo `> 0`
//!   (vslot 0x768 RVA 0x929fa0 = stack count of the item named by `AmmoData`, `0x7fffffff` when the weapon does not use inventory ammo),
//!   `AmmoCount < BulletsPerClip` and `now - LastAmmoTransferTime > 0.3` (RVA 0x940340, constant VA 0x144008E2C).
//! * `UFortGameplayAbility_Reload::CanActivateAbility` (RVA 0x28e660): `min(AmmoCount, ServerAmmo) != BulletsPerClip`.
//! * `ActivateAbility` (RVA 0x289530) plays the reload montage at `GetReloadPlayRate` (vslot 0x7f8, RVA 0x9296c0, see [`play_rate`]) and arms a
//!   timer for [`first_timer_delay`]; the timer handler (RVA 0x286e50) calls the transfer below, re-arms the timer for [`next_timer_delay`] when the magazine is
//!   still not full, and finishes (jumps to the last montage section) when it is.
//! * the transfer, `AFortWeapon` vslot 0x7d0 (RVA 0x9415c0): see [`transfer_amount`], then `AddAmmo` (vslot 0x780, RVA 0x92edd0, [`add_ammo`]) and the inventory
//!   ammo stack is reduced by the same amount (vslot 0x770, RVA 0x9456b0).
//!
//! NOT recovered (reported, not guessed): the `TriggerAbility` montage-notify callback (vslot 0x430 of the ranged-weapon ability, RVA 0x2a8630) also
//! calls the transfer; its interaction with the timer is not resolved (taken literally both would insert extra shells and break the identity
//! "full empty-to-full shotgun reload = ReloadTime" that the play rate formula is built for), so only the timer path drives the transfer here.
//! `ReloadScale` / alteration attribute multipliers of `CurrentReloadDuration` (RVA 0x9191d0), `IsReloading`'s ability bookkeeping (RVA 0x92e4c0), the
//! ability-system blocking-tag checks in `CanReload`, and the notify's blocked-section handling are native-only.

/// `CanReload` refuses while `now - LastAmmoTransferTime <= 0.3` (RVA 0x91aca4 compares against the float constant at VA 0x144008E2C).
pub const RELOAD_GUARD_SECONDS: f32 = 0.3;
/// `GetAmmoInInventory` result when the weapon needs no inventory ammo.
pub const INFINITE_INVENTORY_AMMO: u32 = 0x7fff_ffff;

/// A reload montage as the ability sees it: sequence length and its sections in time order (`GetSectionLength(i)` = next start - start, last = length - start).
#[derive(Clone, Debug, PartialEq)]
pub struct ReloadMontage { pub length: f32, pub starts: Vec<f32> }
impl ReloadMontage {
    pub fn new(length: f32, starts: Vec<f32>) -> Self { ReloadMontage { length, starts: if starts.is_empty() { vec![0.] } else { starts } } }
    pub fn sections(&self) -> usize { self.starts.len() }
    /// `UAnimMontage::GetSectionLength` (0 for an invalid index).
    pub fn section_length(&self, i: usize) -> f32 { match self.starts.get(i) { None => 0., Some(s) => self.starts.get(i + 1).copied().unwrap_or(self.length) - s } }
    pub fn section_start(&self, i: usize) -> f32 { self.starts.get(i).copied().unwrap_or(self.length) }
}

/// `CurrentReloadDuration` (`+0x9a0`) = the weapon table `ReloadTime` (the native `ReloadScale` / alteration multipliers, RVA 0x9191d0, are 1 for a loadout without alterations).
pub fn current_reload_duration(reload_time: f32) -> f32 { reload_time }

/// `GetReloadPlayRate` (RVA 0x9296c0): `bReloadIndividualBullets` montages (intro, loop, outro): `(clip * len(1) + len(0) + len(2)) / duration`
/// (float32 order of the SSE code); otherwise `sequence length / duration`.
pub fn play_rate(m: &ReloadMontage, clip: u32, individual: bool, duration: f32) -> f32 {
    if individual { (clip as f32 * m.section_length(1) + m.section_length(0) + m.section_length(2)) / duration } else { m.length / duration }
}
/// First timer set by `ActivateAbility` (RVA 0x289ab5..): `(len(0) + (section 1 exists ? len(1) : 0)) / rate`.
pub fn first_timer_delay(m: &ReloadMontage, rate: f32) -> f32 { (m.section_length(0) + if m.sections() > 1 { m.section_length(1) } else { 0. }) / rate }
/// Re-armed timer of the handler (RVA 0x286f65): `len(1) / rate` (a rate of 0 clears the timer, so a single-section montage never re-arms).
pub fn next_timer_delay(m: &ReloadMontage, rate: f32) -> f32 { m.section_length(1) / rate }

/// `CanReload` inputs (the parts that exist without the ability system).
#[derive(Clone, Copy, Debug)]
pub struct CanReloadInputs { pub has_ammo_definition: bool, pub is_reloading: bool, pub inventory_ammo: u32, pub magazine: u32, pub clip: u32, pub seconds_since_last_transfer: f32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal { NoAmmoDefinition, AlreadyReloading, NoInventoryAmmo, MagazineFull, TooSoon }
pub fn can_reload(i: CanReloadInputs) -> Result<(), Refusal> {
    if !i.has_ammo_definition { return Err(Refusal::NoAmmoDefinition); }
    if i.is_reloading { return Err(Refusal::AlreadyReloading); }
    if i.inventory_ammo == 0 { return Err(Refusal::NoInventoryAmmo); }
    if i.magazine >= i.clip { return Err(Refusal::MagazineFull); }
    // comiss xmm0, 0.3 / jbe: refused unless strictly greater
    if !(i.seconds_since_last_transfer > RELOAD_GUARD_SECONDS) { return Err(Refusal::TooSoon); }
    Ok(())
}

/// Ammo moved by one transfer call (RVA 0x9415c0): `min(inventory, clip - magazine)`; `0` moves nothing and returns false; with `bReloadIndividualBullets` a positive amount becomes 1.
pub fn transfer_amount(inventory: u32, magazine: u32, clip: u32, individual: bool) -> u32 {
    let room = clip.saturating_sub(magazine);
    let n = inventory.min(room);
    if n > 0 && individual { 1 } else { n }
}
/// `AFortWeapon::AddAmmo` (RVA 0x92edd0): `AmmoCount = clamp(AmmoCount + delta, 0, BulletsPerClip)`.
pub fn add_ammo(magazine: u32, delta: i32, clip: u32) -> u32 { let v = magazine as i64 + delta as i64; if v < 0 { 0 } else { (v as u64).min(clip as u64) as u32 } }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A transfer moved `n` rounds from the inventory stack into the magazine at `at` (seconds, same clock as `Run::start`).
    Moved { at: f32, n: u32 },
    /// The transfer moved nothing (empty stack or full magazine); montages with a loop section jump to their last section.
    Blocked { at: f32 },
    /// The ability ended with its montage.
    Finished { at: f32 },
}

/// One running reload ability (`UFortGameplayAbility_Reload`).
#[derive(Clone, Debug)]
pub struct Run {
    pub montage: ReloadMontage, pub clip: u32, pub individual: bool,
    pub start: f32, pub rate: f32, pub duration: f32,
    timer: Option<f32>, outro_from: Option<f32>, done: bool,
}
impl Run {
    pub fn start(now: f32, montage: ReloadMontage, clip: u32, reload_time: f32, individual: bool) -> Run {
        let duration = current_reload_duration(reload_time);
        let rate = play_rate(&montage, clip, individual, duration);
        let t = first_timer_delay(&montage, rate);
        Run { start: now, rate, duration, timer: (t > 0.).then_some(now + t), outro_from: None, done: false, clip, individual, montage }
    }
    pub fn active(&self) -> bool { !self.done }
    fn multi(&self) -> bool { self.montage.sections() > 1 }
    /// Absolute time the montage (and with it the ability) ends, when known.
    pub fn end_time(&self) -> Option<f32> {
        if !self.multi() { return Some(self.start + self.montage.length / self.rate); }
        self.outro_from.map(|f| f + self.montage.section_length(self.montage.sections() - 1) / self.rate)
    }
    fn go_outro(&mut self, at: f32) { if self.multi() && self.outro_from.is_none() { self.outro_from = Some(at); self.timer = None; } }
    /// Runs every timer / end event due up to `now`. `magazine` / `inventory` are the weapon's `AmmoCount` and the inventory ammo stack.
    pub fn advance(&mut self, now: f32, magazine: &mut u32, inventory: &mut u32) -> Vec<Event> {
        let mut out = vec![];
        while !self.done {
            let timer = self.timer.filter(|t| now > *t); // `FTimerManager::Tick` fires on `InternalTime > ExpireTime` (UE4.16 TimerManager.cpp:543), strictly
            let end = self.end_time().filter(|e| now > *e);
            match (timer, end) {
                (Some(t), e) if e.map_or(true, |e| t <= e) => {
                    self.timer = None;
                    let n = transfer_amount(*inventory, *magazine, self.clip, self.individual);
                    if n == 0 { out.push(Event::Blocked { at: t }); self.go_outro(t); continue; }
                    *magazine = add_ammo(*magazine, n as i32, self.clip); *inventory -= n;
                    out.push(Event::Moved { at: t, n });
                    if *magazine == self.clip { self.go_outro(t); } else { let d = next_timer_delay(&self.montage, self.rate); if d > 0. { self.timer = Some(t + d); } }
                }
                (_, Some(e)) => { self.done = true; self.timer = None; out.push(Event::Finished { at: e }); }
                _ => break,
            }
        }
        out
    }
    /// Montage position at `now`: the intro plays once, the loop section repeats (its `NextSection` is itself) until the finish jumps to the last section.
    pub fn montage_time(&self, now: f32) -> f32 {
        let raw = ((now - self.start) * self.rate).max(0.);
        if !self.multi() { return raw.min(self.montage.length); }
        let (ls, ll) = (self.montage.section_start(1), self.montage.section_length(1));
        let outro = self.montage.section_start(self.montage.sections() - 1);
        if let Some(f) = self.outro_from { let rf = (f - self.start) * self.rate; if raw >= rf { return (outro + raw - rf).min(self.montage.length); } }
        if raw < ls || ll <= 0. { raw } else { ls + (raw - ls).rem_euclid(ll) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ar() -> ReloadMontage { ReloadMontage::new(2.6666667, vec![0.]) }
    fn sg() -> ReloadMontage { ReloadMontage::new(2.7333333, vec![0., 0.7, 1.4333334]) }
    fn run_to(r: &mut Run, t: f32, mag: &mut u32, inv: &mut u32) -> Vec<Event> { let mut ev = vec![]; let mut now = r.start; while now < t { now += 1. / 240.; ev.extend(r.advance(now, mag, inv)); } ev }

    #[test] fn play_rate_makes_a_full_reload_last_reload_time() {
        let r = play_rate(&ar(), 30, false, 2.3); assert!((r - 2.6666667 / 2.3).abs() < 1e-6);
        let s = play_rate(&sg(), 5, true, 4.84); assert!((s - (5. * 0.7333333 + 0.7 + 1.3) / 4.84).abs() < 1e-3, "{s}");
        assert!((play_rate(&ReloadMontage::new(3., vec![0.]), 1, false, 3.) - 1.).abs() < 1e-6);
    }
    #[test] fn magazine_rifle_transfers_everything_at_reload_time() {
        let mut r = Run::start(10., ar(), 30, 2.3, false); let (mut mag, mut inv) = (4, 100);
        assert!((r.end_time().unwrap() - 12.3).abs() < 1e-4);
        let ev = run_to(&mut r, 12.0, &mut mag, &mut inv); assert!(ev.is_empty() && mag == 4, "nothing before ReloadTime: {ev:?}");
        let ev = r.advance(12.4, &mut mag, &mut inv);
        assert_eq!((mag, inv), (30, 74)); assert!(matches!(ev[0], Event::Moved { n: 26, .. }) && matches!(ev[1], Event::Finished { .. }));
        assert!(!r.active());
    }
    #[test] fn a_short_stack_only_moves_what_exists() { let mut r = Run::start(0., ar(), 30, 2.3, false); let (mut mag, mut inv) = (0, 7); r.advance(5., &mut mag, &mut inv); assert_eq!((mag, inv), (7, 0)); }
    #[test] fn shotgun_inserts_one_shell_per_loop_pass_and_takes_reload_time_from_empty() {
        let mut r = Run::start(0., sg(), 5, 4.84, true); let (mut mag, mut inv) = (0, 50);
        let ev = run_to(&mut r, 6., &mut mag, &mut inv);
        let moved: Vec<f32> = ev.iter().filter_map(|e| if let Event::Moved { at, .. } = e { Some(*at) } else { None }).collect();
        assert_eq!(moved.len(), 5); assert_eq!((mag, inv), (5, 45));
        let step = 0.7333333 / r.rate; assert!((moved[0] - (0.7 + 0.7333333) / r.rate).abs() < 1e-3);
        for w in moved.windows(2) { assert!((w[1] - w[0] - step).abs() < 1e-3); }
        let fin = ev.iter().find_map(|e| if let Event::Finished { at } = e { Some(*at) } else { None }).unwrap(); assert!((fin - 4.84).abs() < 1e-2, "ends at ReloadTime: {fin}");
    }
    #[test] fn shotgun_with_a_short_stack_ends_through_the_outro() {
        let mut r = Run::start(0., sg(), 5, 4.84, true); let (mut mag, mut inv) = (0, 2);
        let ev = run_to(&mut r, 8., &mut mag, &mut inv); assert_eq!((mag, inv), (2, 0));
        assert!(ev.iter().any(|e| matches!(e, Event::Blocked { .. }))); assert!(ev.iter().any(|e| matches!(e, Event::Finished { .. })));
    }
    #[test] fn montage_time_loops_the_loop_section_then_plays_the_outro() {
        let mut r = Run::start(0., sg(), 5, 4.84, true); let (mut mag, mut inv) = (0, 50);
        assert!((r.montage_time(0.1) - 0.1 * r.rate).abs() < 1e-5);
        let t = (0.7 + 0.7333333) / r.rate + 0.1; let m = r.montage_time(t); assert!(m >= 0.7 && m < 1.4334, "second pass restarts the loop: {m}");
        r.advance(10., &mut mag, &mut inv);
        let fin = r.end_time().unwrap(); assert!(r.montage_time(fin - 0.01) > 1.4333);
    }
    #[test] fn timers_fire_strictly_after_their_expire_time_like_the_engine_timer_manager() {
        let mut r = Run::start(0., ar(), 30, 2.3, false); let (mut mag, mut inv) = (4, 100);
        let t = r.timer.unwrap();
        assert!(!r.advance(t, &mut mag, &mut inv).iter().any(|e| matches!(e, Event::Moved { .. })), "now == ExpireTime does not fire (InternalTime > ExpireTime)");
        assert!(r.advance(t + 1e-3, &mut mag, &mut inv).iter().any(|e| matches!(e, Event::Moved { .. })));
    }
    #[test] fn can_reload_follows_the_native_order() {
        let ok = CanReloadInputs { has_ammo_definition: true, is_reloading: false, inventory_ammo: 10, magazine: 5, clip: 30, seconds_since_last_transfer: 1. };
        assert_eq!(can_reload(ok), Ok(()));
        assert_eq!(can_reload(CanReloadInputs { inventory_ammo: 0, ..ok }), Err(Refusal::NoInventoryAmmo));
        assert_eq!(can_reload(CanReloadInputs { magazine: 30, ..ok }), Err(Refusal::MagazineFull));
        assert_eq!(can_reload(CanReloadInputs { seconds_since_last_transfer: 0.3, ..ok }), Err(Refusal::TooSoon));
        assert_eq!(can_reload(CanReloadInputs { is_reloading: true, ..ok }), Err(Refusal::AlreadyReloading));
    }
    #[test] fn transfer_and_add_ammo_arithmetic() {
        assert_eq!(transfer_amount(100, 4, 30, false), 26); assert_eq!(transfer_amount(5, 4, 30, false), 5);
        assert_eq!(transfer_amount(100, 4, 5, true), 1); assert_eq!(transfer_amount(0, 4, 5, true), 0); assert_eq!(transfer_amount(9, 30, 30, false), 0);
        assert_eq!(add_ammo(28, 5, 30), 30); assert_eq!(add_ammo(2, -5, 30), 0); assert_eq!(add_ammo(2, 3, 30), 5);
    }
}
