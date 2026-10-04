//! Deterministic random streams.
//!
//! [`Rng`] is `FRandomStream` of UE 4.16 (`UnrealMathUtility`): the seed is an LCG (`Seed = Seed * 196314165 + 907633515`) and the fraction is built from the low 23 bits of the
//! seed as the mantissa of a float in [1, 2) minus 1. The `*_Seeded` modules of a particle system keep one such stream each, initialised from `RandomSeedInfo.RandomSeeds[0]`,
//! which is how the original makes the same system spawn the same pieces every time. The un-seeded modules used the global `FMath::SRand` (non deterministic) in the original;
//! here every emitter instance owns one stream seeded from (system seed, emitter index), so a run is reproducible.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng { seed: i32 }

impl Rng {
    pub fn new(seed: i32) -> Rng { Rng { seed } }
    /// Mixes two numbers into a seed for an emitter instance (never 0).
    pub fn from_pair(a: u64, b: u64) -> Rng {
        let mut z = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_add(0x7F4A_7C15_9E37_79B9);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9); z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB); z ^= z >> 31;
        Rng { seed: (z as i32) | 1 }
    }
    pub fn seed(&self) -> i32 { self.seed }
    #[inline] fn mutate(&mut self) { self.seed = self.seed.wrapping_mul(196_314_165).wrapping_add(907_633_515); }
    /// `FRandomStream::GetFraction`: [0, 1).
    #[inline] pub fn frac(&mut self) -> f32 {
        self.mutate();
        let bits = 0x3F80_0000u32 | (self.seed as u32 & 0x007F_FFFF);
        (f32::from_bits(bits) - 1.0).clamp(0.0, 0.999_999_9)
    }
    #[inline] pub fn range(&mut self, lo: f32, hi: f32) -> f32 { lo + (hi - lo) * self.frac() }
    /// Uniform integer in `0..n`.
    pub fn below(&mut self, n: u32) -> u32 { if n == 0 { 0 } else { ((self.frac() * n as f32) as u32).min(n - 1) } }
    /// Unit vector, uniform on the sphere.
    pub fn unit_vec(&mut self) -> [f32; 3] {
        let z = self.range(-1.0, 1.0); let a = self.range(0.0, std::f32::consts::TAU); let r = (1.0 - z * z).max(0.0).sqrt();
        [r * a.cos(), r * a.sin(), z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn same_seed_same_sequence_and_range() {
        let (mut a, mut b) = (Rng::new(15), Rng::new(15));
        for _ in 0..1000 { let (x, y) = (a.frac(), b.frac()); assert_eq!(x, y); assert!((0.0..1.0).contains(&x)); }
        let mut c = Rng::new(16); assert_ne!(Rng::new(15).frac(), c.frac());
    }
    #[test] fn fractions_are_roughly_uniform() {
        let mut r = Rng::from_pair(3, 9); let n = 20000; let mean: f32 = (0..n).map(|_| r.frac()).sum::<f32>() / n as f32;
        assert!((mean - 0.5).abs() < 0.02, "{mean}");
    }
}
