//! `fn-pose`: the renderer-independent pose evaluator.
//!
//! ```text
//! fn_animgraph::AnimState::plan() -> Node (pose tree)  --\
//! Part skeletons (fn-mesh / old glTF) + master IK bones   >--> CharacterRig::evaluate --> CharacterPose { parts: [PoseResult { bone_local,
//! ClipLibrary (FnClip over fn-anim: .fnanim / packages / pak) /                                                  bone_mesh, skin_matrices }], sockets }
//! ```
//!
//! * [`eval`]       `PoseEvaluator`: the old `assets::Model::eval_pose` (clips, blends, additive and mesh-space additive with the face fix's
//!                  tracked-bone mask, layered blends / montage slots, TwoBoneIK, HandIKRetargeting, HeadTrack with its 75 degree clamp,
//!                  ModifyBone, RotateRoot, CopyBone, AimOffset).
//! * [`clip`]       `FnClip: fn_animgraph::ClipSource` over `fn-anim` (UE-exact sampling, or the old md5anim runtime's for parity tests),
//!                  `ClipStore` (name -> clip, loaded from raw packages / `.fnanim` files / the pak).
//! * [`part`]       `Part`: skeleton (+ the master's IK bones appended at load), skin joints, inverse bind matrices, CPU skinning.
//! * [`character`]  `CharacterRig` / `PoseResult` / `CharacterPose`, sockets, the GPU bone-palette layout of `fn-render`.
//! * [`diag`]       grip / face-rigidity / pose-hash helpers of the old test modes.
//! * `scenario`     (feature `scenario`) the scripted scenarios of the old test modes driven through `fn_game::GameSim`.
//!
//! Units / axes: metres, Y up, +Z forward, right = -X (the old glTF export convention).
pub mod br_character; // BR CHARACTER HOOK
pub mod br_material; // BR CHARACTER MATERIAL HOOK
pub mod br_material_gen; // BR CHARACTER MATERIAL HOOK (generated: fn-dxbc-cli example br_character_materials)
pub mod character;
pub mod clip;
pub mod cosmetics; // LOCKER HOOK: every cosmetic item definition of the 1.8 paks
pub mod diag;
pub mod eval;
pub mod loader;
pub mod loadout;
pub mod loadout_ads; // ADS HOOK
pub mod loadout_ammo; // AMMO HOOK
pub mod loadout_inventory; // WIDGETS3 HOOK
pub mod loadout_material;
pub mod loadout_stats;
pub mod part;
pub mod part_anim; // PART ANIM HOOK
pub mod skel;
#[cfg(test)]
mod tests;
#[cfg(feature = "scenario")]
pub mod scenario;

pub use character::{CharacterPose, CharacterRig, PoseResult, SocketDef, GPU_MATRIX_BYTES, PICK_SOCKET_YAW};
pub use clip::{AdditiveKind, ClipSourceDir, ClipStore, FnClip, SampleMode};
pub use eval::{EvalOptions, PoseEvaluator};
pub use part::{Part, SkinPrim};
pub use skel::{BoneTree, BoneXform, Pose};
