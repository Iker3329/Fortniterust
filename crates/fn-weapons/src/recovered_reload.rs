//! Static port of the original AFortWeapon reload-progress calculation only.
//!
//! Original 1.8 executable: registration 0xA57D40 associates GetReloadProgress
//! with exec thunk 0xB02670; thunk calls vtable slot 0x7F0. Constructor
//! 0x914B60 installs vtable VA 0x144297F90, whose slot points to RVA 0x9297E0.
//! Chained unwind regions: 0x9297FC, 0x929845, 0x92985B.
//!
//! Input provenance: original object +0x998 = LastReloadTime, +0x9A0 =
//! CurrentReloadDuration (confirmed by generated property constructors).
//! Native code samples [GetWorld()+0x900] and multiplies by object +0x80.
//! Clock/time-scale integration is intentionally NOT supplied here.
//! Upper-limit constant VA 0x143FE36FC = 0x3F800000 (1.0f).
//!
//! This has static instruction evidence, not an original-client trace oracle.
//! It neither starts a reload nor transfers ammo nor chooses a montage rate.

#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    /// Result of the original IsReloading virtual method (slot 0x7C0).
    pub is_reloading: bool,
    /// The original clock value sampled from GetWorld() +0x900.
    pub clock_sample: f32,
    pub last_reload_time: f32,
    /// Original object +0x80. Supply original semantics; no wall-clock fallback.
    pub time_scale: f32,
    /// Original CurrentReloadDuration, not necessarily the weapon-table ReloadTime.
    pub current_reload_duration: f32,
}

pub fn progress(input: Inputs) -> f32 {
    if !input.is_reloading { return 0.0; }
    // Preserve SUBSS -> MULSS -> DIVSS order and float32 precision.
    let elapsed = input.clock_sample - input.last_reload_time;
    let scaled_elapsed = elapsed * input.time_scale;
    let ratio = scaled_elapsed / input.current_reload_duration;
    // COMISS + JB selects zero for negative OR unordered input.
    if ratio.is_nan() || ratio < 0.0 { 0.0 }
    // MINSS with original 1.0f operand, preserving -0 when it is the first operand.
    else if ratio < 1.0 { ratio }
    else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inputs() -> Inputs { Inputs { is_reloading:true,clock_sample:11.0,last_reload_time:10.0,time_scale:1.0,current_reload_duration:2.0 } }
    #[test] fn idle_returns_zero_without_using_invalid_inputs() {
        assert_eq!(progress(Inputs {is_reloading:false,clock_sample:f32::NAN,..inputs()}).to_bits(),0);
    }
    #[test] fn uses_stored_duration_and_scale() {
        assert_eq!(progress(inputs()),0.5);
        assert_eq!(progress(Inputs {time_scale:0.5,..inputs()}),0.25);
        assert_eq!(progress(Inputs {current_reload_duration:4.0,..inputs()}),0.25);
    }
    #[test] fn original_negative_and_upper_limit_branches() {
        assert_eq!(progress(Inputs {clock_sample:9.0,..inputs()}),0.0);
        assert_eq!(progress(Inputs {clock_sample:20.0,..inputs()}),1.0);
    }
    #[test] fn original_divide_by_zero_is_not_replaced_by_epsilon() {
        assert_eq!(progress(Inputs {current_reload_duration:0.0,..inputs()}),1.0);
        assert_eq!(progress(Inputs {clock_sample:10.0,current_reload_duration:0.0,..inputs()}),0.0);
    }
    #[test] fn unordered_compare_and_signed_zero_follow_native_branches() {
        assert_eq!(progress(Inputs {clock_sample:f32::NAN,..inputs()}),0.0);
        assert_eq!(progress(Inputs {clock_sample:10.0,time_scale:-1.0,..inputs()}).to_bits(),(-0.0_f32).to_bits());
    }
}
