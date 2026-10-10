//! Athena 1.8 building: build grid, pieces, placement, editing, structural support and damage.
//! Every value is sourced from the cooked 1.8 assets, UE4.16 source or the 1.8 exe (cited at the use site).
//!
//! Module owners (parallel agents): add only your own `pub mod` line here.
//! - core / placement: `grid`, `pieces`, `placement`, `world`
//! - visuals: `visual`
//! - editing: `edit`
//! - structural / damage: `structural`, `damage`
pub mod pieces;
pub mod world;
pub mod grid;
pub mod placement;
pub mod edit;
pub mod structural;
pub mod damage;
pub mod visual;
// doors (2026-10-08): the ABuildingWall door state machine shared by map doors and player door edits
pub mod door;
