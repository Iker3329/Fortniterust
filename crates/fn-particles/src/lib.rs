//! # fn-particles
//!
//! The Cascade particle systems (`UParticleSystem`) of Fortnite 1.8 (UE 4.16) in pure Rust: decoded data model ([`model`]), baked-table / curve distributions ([`dist`]),
//! a deterministic CPU simulator with the original module semantics ([`sim`]), and the asset catalog the game loads ([`assets`]); with the `import` feature the
//! pak -> asset-tree importer ([`import`], used by the `fn-particles-export` tool).
//!
//! See `README.md` of this crate for what is exact, what is reconstructed and what is missing.
pub mod assets;
pub mod dist;
pub mod model;
pub mod rng;
pub mod sim;
#[cfg(feature = "import")]
pub mod import;

pub use dist::Dist;
pub use model::{ModuleKind, ParticleSystem};
pub use rng::Rng;
pub use sim::{DrawKind, DrawParticle, EmitterDraw, Env, Instance, ParticleLight, Transform};
