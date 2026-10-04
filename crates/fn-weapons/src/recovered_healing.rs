//! Original GA_Athena_Bandage Blueprint arithmetic, not an ability scheduler.
//! Source: loadout-evidence/GA_Athena_Bandage.disasm.txt K2_CanActivateAbility
//! and Triggered path. The caller supplies original CDO HealingCap and
//! MaxHealingAmount. TriggerDuration is NOT a guessed refill/heal timer.

/// Valid Athena-pawn branch: FCeil -> Conv_IntToFloat -> Less_FloatFloat.
/// Preconditions: finite game health and cap in the normal health domain.
pub fn bandage_health_gate(health: f32, healing_cap: f32) -> bool {
    ((health.ceil() as i32) as f32) < healing_cap
}

/// Only after the original ability's K2_CommitAbility succeeds, the Blueprint
/// computes FTrunc(FClamp(HealingCap - GetHealth(), 0, MaxHealingAmount)).
/// This is the EFFECT LEVEL passed to BP_ApplyGameplayEffectToOwner, not a
/// direct pawn-health write or proof of the native effect's scaling semantics.
pub fn bandage_effect_level(health: f32, healing_cap: f32, maximum: f32) -> i32 {
    let difference = healing_cap - health;
    let clamped = if difference < 0. { 0. } else if difference < maximum { difference } else { maximum };
    clamped.trunc() as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_ceil_gate_is_not_a_simple_health_less_than_cap() {
        assert!(bandage_health_gate(73.9, 75.));
        assert!(bandage_health_gate(74., 75.));
        assert!(!bandage_health_gate(74.01, 75.));
        assert!(!bandage_health_gate(75., 75.));
        assert!(!bandage_health_gate(100., 75.));
    }
    #[test]
    fn original_trigger_path_caps_then_truncates_the_effect_level() {
        for (health, expected) in [(0.,15),(50.,15),(60.,15),(61.,14),(74.,1),(74.5,0),(75.,0),(100.,0)] {
            assert_eq!(bandage_effect_level(health,75.,15.),expected);
        }
    }
}
