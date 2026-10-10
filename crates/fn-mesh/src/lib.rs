//! `fn-mesh`: UE 4.16 (Fortnite 1.8) `Skeleton` / `SkeletalMesh` / `StaticMesh` decoder and glTF exporter.
//!
//! ```no_run
//! # fn main() -> fn_mesh::Result<()> {
//! // `uasset`, `uexp`, `ubulk` are the raw file images of one package.
//! # let (uasset, uexp, ubulk): (Vec<u8>, Vec<u8>, Vec<u8>) = Default::default();
//! let pkg = fn_ueio::Package::parse(&uasset, &uexp, &ubulk)?;
//! let mesh = fn_mesh::SkeletalMesh::parse(&pkg)?;               // also: Skeleton::parse, StaticMesh::parse
//! fn_mesh::gltf::export_skeletal_mesh(&mesh, std::path::Path::new("out"), &mesh.name)?;
//! # Ok(()) }
//! ```
//!
//! # Modules
//!
//! * [`skeleton`]: [`Skeleton`] (reference pose, translation retargeting modes, sockets, virtual bones, curves).
//! * [`skeletal`]: [`SkeletalMesh`] (bounds, material slots, RefSkeleton, LODs with sections, bone maps, indices and
//!   struct-of-arrays vertex data incl. skin weights, colours and extra UV sets).
//! * [`static_mesh`]: [`StaticMesh`] (LODs, sections, vertex streams, indices, materials, bounds).
//! * [`gltf`]: exporters, byte compatible with the old UEViewer glTF exports (and [`fmt_g`](gltf::fmt_g), a C `%g`).
//! * [`skym`]: the compact `SKYM` static mesh files the old sky renderer loads.
//!
//! All positions of the decoded structs are **UE space** (centimetres, left handed, Z up); conversion helpers to
//! the glTF convention live in [`gltf`] (`ue_to_gltf_position` ...). Errors are `fn_ueio::Error`.
//!
//! # Format facts (Fortnite 1.8, UE 4.16 snapshot, UEViewer game tag `fn18`)
//!
//! The per-structure byte layouts are documented in the modules ([`skeleton`], [`skeletal`], [`static_mesh`]);
//! the cross-cutting facts and quirks are:
//!
//! * **Packages are unversioned** (file version 0, treated as 513) and have an empty custom version table, so the
//!   engine's own latest custom versions apply. UEViewer emulates them with `GAME_FN18 = GAME_UE4(16)+3`: most
//!   version gates are those of UE 4.16, but `FSkeletalMeshCustomVersion` is **7** (the real 4.16 value is 9):
//!   `TriangleSorting` (u8) *and* `ClothAssetSubmeshIndex` (i16) are still serialised in every section, there is no
//!   `FClothingSectionData`, skin weights live in a separate buffer, and `FStaticLODModel` is the old single
//!   structure (no `FSkeletalMeshLODRenderData` split, no editor/render duplication).
//! * Cooked data: `FStripDataFlags` (2 bytes: global, class) precede most structures; global bit 0 (editor data
//!   stripped) is set everywhere. Native `bool` members are **32 bit**; `FName` is `i32 index, i32 number`;
//!   `TArray::BulkSerialize` is `i32 elemSize, i32 count, raw items`.
//! * After the tagged properties of every UObject export comes an `i32 bHasGuid` (0), then the native data.
//! * `FPackedNormal` (tangents) are 4 bytes **without the 0x80 offset** that 4.20+ uses: a byte `b` means
//!   `b/127.5 - 1` (UEViewer decodes `(b ^ 0x80) as i8 / 127`, which is what the old exports contain).
//!   `TangentZ.w` is the binormal sign (>= 128: +1). Skeletal vertices store TangentX and TangentZ only
//!   (TangentY = cross(Z, X) * sign).
//! * Skeletal vertices: `TangentX u8x4, TangentZ u8x4, Position f32x3, UV (f16x2 or f32x2) x NumTexCoords`
//!   (stride `8+12+4*N` with half UVs); skin weights are a separate bulk array of `u8 bone[4|8], u8 weight[4|8]`
//!   per vertex, bone indices are *section local* (`Section::bone_map` -> mesh bone). 8 influences
//!   (`extra_bone_influences`) occur on the body and head meshes. `FColor` vertex colours are stored in memory
//!   order B, G, R, A.
//! * Index buffers are u16 (all test meshes) or u32 (`FMultisizeIndexContainer` data size byte).
//! * `FSkeletalMaterial` is `ObjectRef, FName slot, FMeshUVChannelInfo(24 bytes)`; material references are package
//!   indices (imports of `MaterialInstanceConstant` / `Material`). The sky meshes reference
//!   `/Engine/EngineMaterials/WorldGridMaterial`.
//! * **StaticMesh quirks** (fn18 differs from final 4.16): no `FStaticMeshSectionAreaWeightedTriangleSampler`
//!   data after the index buffers, and no `bLODsShareStaticLighting` after `Bounds` (`Bounds` is directly followed
//!   by the 8 `ScreenSize` floats, then `bHasSpeedTreeWind`, then `StaticMaterials`, then the export ends). The
//!   distance field block (`FDistanceFieldVolumeData`: compressed `u8` array, size, box, min/max, 3 bools) is present
//!   per LOD when the leading bool is set.
//! * **USkeleton**: `ReferenceSkeleton`, `AnimRetargetSources`, `Guid`, `SmartNames` (`GuidMap` still present,
//!   curve meta data without `MaxLOD`), nothing else. The player skeleton (`Fortnite_M_Avg_Player_Skeleton`) has
//!   **266** bones (the `Fortnite_M_Avg_Player` mesh uses 86 of them; the body skeleton has 99, the body mesh 92 -
//!   "93" is the node count of the old body glTF: 92 bones + the mesh node), 50 sockets, 101 curves, 18 retarget
//!   sources; `BoneTree` modes: 227 `Skeleton`, 22 `AnimationScaled`, 13 `Animation`, 4 `AnimationRelative`.
//!
//! # Old (UEViewer) exports: conventions and bugs
//!
//! [`gltf`] reproduces `ExportGLTF.cpp` of the UEViewer fork bit for bit (see its docs for the node layout, bone
//! sorting and axis conversion). Differences that are *bugs of the old exports*, not of this decoder:
//!
//! 1. UEViewer's `half2float` ignores zero/denormal halves: a half `0` becomes `3.0517578e-5`, so zero UVs differ by
//!    up to 3.1e-5 from the engine value. The exporter converts correctly by default; [`gltf::Options::legacy_uev_half_uv`]
//!    emulates the bug (used by the tests to prove bit exactness).
//! 2. The binormal sign (`TANGENT.w`) of skeletal meshes is partly overwritten from uninitialised memory in
//!    UEViewer (`FPackedNormal Normal[1]` is never serialised). 0/10454 (body), 1/3836 (head), 162/310 (pick), 0
//!    (others) vertices differ; this decoder uses the stored sign.
//! 3. The material references of the sky static meshes were lost in the old PSKX exports (`material_0`).
//!
//! # Verification
//!
//! `cargo test -p fn-mesh --release -- --nocapture` (gated on the extracted test data and the old exports) compares
//! against the old `.gltf/.bin` (every accessor, skin, node, material), the old `.pskx` and `SKYM` files, checks the
//! skeleton against the meshes that use it, and scans all Skeleton/SkeletalMesh/StaticMesh exports of the test data.

mod common;
pub mod gltf;
pub mod gltf_lods;
pub mod skeletal;
pub mod skeleton;
pub mod skym;
pub mod static_mesh;

pub use common::{tangent_sign, unpack_byte_engine, unpack_byte_uev, unpack_normal, unpack_normal_unit, Bounds, Transform};
pub use fn_ueio::{Error, Result};
pub use skeletal::{Influences, LodInfo, Section, SkeletalLod, SkeletalMaterial, SkeletalMesh, UvChannelInfo};
pub use skeleton::{Bone, CurveMeta, RetargetSource, SlotGroup, Skeleton, Socket, TranslationRetarget, VirtualBone};
pub use gltf_lods::{export_static_mesh_lods, static_mesh_to_gltf_lods, PropMaterial};
pub use static_mesh::{DistanceFieldVolumeData, LodCheck, StaticLod, StaticMaterial, StaticMesh, StaticSection};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_normal_decoding() {
        assert_eq!(unpack_byte_engine(255), 1.0);
        assert_eq!(unpack_byte_engine(0), -1.0);
        // UEViewer decode: byte 128 is zero, 255 is +1, 0 is -1.0079
        assert_eq!(unpack_byte_uev(128), 0.0);
        assert_eq!(unpack_byte_uev(255), 1.0);
        assert!((unpack_byte_uev(0) + 128.0 / 127.0).abs() < 1e-6);
        assert_eq!(tangent_sign([0, 0, 0, 255]), 1.0);
        assert_eq!(tangent_sign([0, 0, 0, 0]), -1.0);
        let n = unpack_normal_unit([255, 127, 127, 255]);
        assert!((n[0] * n[0] + n[1] * n[1] + n[2] * n[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn bounds_box() {
        let b = Bounds { origin: [1.0, 2.0, 3.0], extent: [1.0, 1.0, 2.0], radius: 3.0 };
        assert_eq!(b.box_min_max(), ([0.0, 1.0, 1.0], [2.0, 3.0, 5.0]));
    }
}
