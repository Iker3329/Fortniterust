//! `fn-propdata`: loader for the exported **real Fortnite 1.8 harvestable props** (trees, rocks, bushes, cars, ...) written by
//! `fn-export props` into `assets/props/`. Pure Rust, no GPU / windowing / UE dependency: the game uses the data
//! (health, resource, collision, placements), the renderer uses the meshes / materials / textures.
//!
//! ```no_run
//! # fn main() -> Result<(), fn_propdata::Error> {
//! let cat = fn_propdata::PropCatalog::load("assets/props")?;
//! let pine = cat.get("Athena_Tree_Pine_3").unwrap();
//! println!("{}: {} HP, {:?}, {} LODs", pine.display, pine.health, pine.resource.kind, pine.lods.len());
//! let lods = pine.load_lods()?;                       // metres, Y up
//! let mat = pine.load_material(0)?;                   // .mat.json
//! let diffuse = mat.base_color_texture().unwrap().load_image()?;
//! let sample = cat.load_sample_layout()?;             // 40 placements around a dense tree + rock cluster
//! # let _ = (lods, diffuse, sample); Ok(()) }
//! ```
//! See `README.md` for the on-disk schema and the coordinate conventions.
mod catalog;
mod collision;
mod layout;
mod material;
mod mesh;
mod util;

pub use catalog::{Destruction, LodInfo, MaterialSlot, PropCatalog, PropKind, RenderInfo, ResourceInfo, ResourceKind};
pub use collision::{Collision, Shape};
pub use layout::{ue_to_game_position, Layout, PropInstance};
pub use material::{AlphaMode, Image, Material, Slots, TextureRef};
pub use mesh::{load_lods, LodMesh, MeshSection};
pub use util::{Error, Result};
