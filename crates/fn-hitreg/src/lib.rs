//! Gun hit registration recovered from the 1.8 data (see `data` for what is read, `shoot` for the exact / native-only split).
pub mod data;
pub mod shoot;
pub use data::{load, GunHit, StatRow};
pub use shoot::{fire, AimState, Impact, Policy, Projectile, Recoil, Shot, ShotReport, Zone};

#[cfg(test)]
mod tests;
