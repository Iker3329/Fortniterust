//! Edit-mode tile patterns (`UBuildingEditModeMetadata*`) and the pattern -> building class matcher.
//!
//! Everything here is a re-expression of the 1.8 exe (FortniteClient-Win64-Shipping.exe, Ghidra names from
//! `C:\FortniteRef\decomp\named`); the per-function source is cited at each item. The patterns themselves are the cooked
//! `/Game/Building/EditModePatterns/**/EMP_*` assets (loaded in `edit_data`), never a hand table.
//!
//! Layout facts (exe):
//! * `UBuildingEditModeMetadata::+0x50` = grid size N, tiles are a row-major N x N array (`TileData`).
//!   Wall N = 3 (`UBuildingEditModeMetadata_Wall::__DefaultConstructor` @140ad0240: `+0x50 = 3`, TileData 9 zeroes),
//!   Floor N = 2 (`..._Floor::__DefaultConstructor` @140ad0180: `+0x50 = 2`, 4 zeroes),
//!   Roof N = 2 (`UBuildingEditModeMetadata_Roof::ctor` @1404a2cf0: `+0x50 = 2`, 4 zero bytes),
//!   Stair N = 2 (`UBuildingEditModeMetadata_Stair::ctor` @1404a2e00: `+0x50 = 2`, 4 zero int32).
//! * Binary tiles are `EBinaryToggleValues` (SDK): BTV_Active = 0, BTV_Inactive = 1, BTV_Either = 2.
//!   Stair tiles are int32 path step numbers (0 = not on the path).

use crate::pieces::PieceKind;

/// `EBinaryToggleValues` (SDK 1.8 FN_FortniteGame_structs.hpp).
pub const BTV_ACTIVE: u8 = 0;
pub const BTV_INACTIVE: u8 = 1;
pub const BTV_EITHER: u8 = 2;

/// Tile data of one metadata object. `Binary` = Wall / Floor / Roof (`UBuildingEditModeMetadata_BinaryToggle`),
/// `Stair` = `UBuildingEditModeMetadata_Stair` (`TArray<int32> TileData`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tiles {
    Binary(Vec<u8>),
    Stair(Vec<i32>),
}

/// One edit pattern (a `UBuildingEditModeMetadata` instance: a cooked EMP asset, a preview or a scratchpad copy).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// Metadata class, which also decides `Equals` compatibility (`IsA` checks in the `Equals` overrides).
    pub kind: PieceKind,
    /// Grid size N (`+0x50`).
    pub n: usize,
    pub tiles: Tiles,
}

impl Pattern {
    /// The class-default tile data of each metadata class (see module doc): every binary tile `BTV_Active`, every stair
    /// tile 0. Cooked EMP assets that omit `TileData` (delta serialisation, e.g. `EMP_Wall_Solid`) keep these.
    pub fn class_default(kind: PieceKind) -> Pattern {
        match kind {
            PieceKind::Wall => Pattern { kind, n: 3, tiles: Tiles::Binary(vec![BTV_ACTIVE; 9]) },
            PieceKind::Floor | PieceKind::Roof => Pattern { kind, n: 2, tiles: Tiles::Binary(vec![BTV_ACTIVE; 4]) },
            PieceKind::Stairs => Pattern { kind, n: 2, tiles: Tiles::Stair(vec![0; 4]) },
        }
    }

    pub fn len(&self) -> usize {
        match &self.tiles {
            Tiles::Binary(t) => t.len(),
            Tiles::Stair(t) => t.len(),
        }
    }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    /// Binary tile value (`BTV_*`); stair patterns return `None`.
    pub fn binary(&self, i: usize) -> Option<u8> {
        match &self.tiles {
            Tiles::Binary(t) => t.get(i).copied(),
            Tiles::Stair(_) => None,
        }
    }
    /// Stair step value; binary patterns return `None`.
    pub fn stair(&self, i: usize) -> Option<i32> {
        match &self.tiles {
            Tiles::Stair(t) => t.get(i).copied(),
            Tiles::Binary(_) => None,
        }
    }
    pub fn set_binary(&mut self, i: usize, v: u8) {
        if let Tiles::Binary(t) = &mut self.tiles { if let Some(x) = t.get_mut(i) { *x = v; } }
    }
    pub fn set_stair(&mut self, i: usize, v: i32) {
        if let Tiles::Stair(t) = &mut self.tiles { if let Some(x) = t.get_mut(i) { *x = v; } }
    }

    /// `RotateMetadata` (metadata vtable slot 71). Binary: thunk @1404c37c0 -> `FUN_14049da60(TileData, N)`;
    /// Stair: thunk @1404c3800 -> `FUN_14049d6f0` (same algorithm on int32); Roof @1404c37d0 also rotates its
    /// `AuxEditTileMeshData` (all cooked roof EMPs leave that at its default, so it is not modelled).
    /// In-place 4-cycle rotation; only applied when the array is exactly N*N. Result: `new[r][c] = old[c][N-1-r]`.
    pub fn rotate(&mut self) {
        let n = self.n;
        match &mut self.tiles {
            Tiles::Binary(t) => rotate_square(t, n),
            Tiles::Stair(t) => rotate_square(t, n),
        }
    }

    /// `MirrorMetadata` (slot 72). Binary: thunk @1404bb840 -> `FUN_14049cf40(TileData, N)`; Stair: thunk @1404bb880 ->
    /// `FUN_14049cd30`. Swaps column c with N-1-c in every row (only when the array is N*N).
    pub fn mirror(&mut self) {
        let n = self.n;
        match &mut self.tiles {
            Tiles::Binary(t) => mirror_square(t, n),
            Tiles::Stair(t) => mirror_square(t, n),
        }
    }

    /// `Equals` (slot 69) as called by the class search: `self` is the candidate class's cooked metadata, `other` the
    /// scratchpad. Binary @1404ace70: `other` must be a BinaryToggle metadata with the same tile count; a tile pair
    /// matches when either side is `BTV_Either` or both are equal. Stair @1404acf20: `other` must be stair metadata,
    /// same count, `memcmp` of the int arrays.
    pub fn equals(&self, other: &Pattern) -> bool {
        match (&self.tiles, &other.tiles) {
            (Tiles::Binary(a), Tiles::Binary(b)) => {
                a.len() == b.len()
                    && a.iter().zip(b).all(|(&x, &y)| x == BTV_EITHER || y == BTV_EITHER || x == y)
            }
            (Tiles::Stair(a), Tiles::Stair(b)) => a == b,
            _ => false,
        }
    }
}

/// `FUN_14049da60` / `FUN_14049d6f0`: for layer i in 0..N/2, j in 0..(N+1)/2, with
/// a=(j, N-1-i), b=(N-1-i, N-1-j), c=(N-1-j, i), d=(i, j): `tmp=A[a]; A[a]=A[b]; A[b]=A[c]; A[c]=A[d]; A[d]=tmp`.
fn rotate_square<T: Copy>(t: &mut [T], n: usize) {
    if t.len() != n * n { return; }
    for i in 0..n / 2 {
        for j in 0..(n + 1) / 2 {
            let a = j * n + (n - 1 - i);
            let b = (n - 1 - i) * n + (n - 1 - j);
            let c = (n - 1 - j) * n + i;
            let d = i * n + j;
            let tmp = t[a];
            t[a] = t[b];
            t[b] = t[c];
            t[c] = t[d];
            t[d] = tmp;
        }
    }
}

/// `FUN_14049cf40` / `FUN_14049cd30`.
fn mirror_square<T: Copy>(t: &mut [T], n: usize) {
    if t.len() != n * n || n == 0 { return; }
    for r in 0..n {
        for k in 0..n / 2 {
            t.swap(r * n + k, r * n + (n - 1 - k));
        }
    }
}

/// Per piece type flags of the `UBuildingEditModeSupport_*` constructors (`+0x240` bit 0 = `bCanMirrorMetadataToMatch`,
/// bit 1 = `bCanRotateMetadataToMatch`):
/// Wall @1404a3a10 `&~2 | 1` (mirror only), Floor @1404a3360 `&~1 | 2` (rotate only),
/// Roof @1404a34b0 `&~1 | 2` (rotate only), Stair @1404a3630 `| 3` (both).
pub fn support_flags(kind: PieceKind) -> (bool, bool) {
    match kind {
        PieceKind::Wall => (true, false),
        PieceKind::Floor | PieceKind::Roof => (false, true),
        PieceKind::Stairs => (true, true),
    }
}

/// `FEditModeState` (SDK 0x10 bytes): the class to switch to, the extra yaw quarter turns and the mirror flag, exactly
/// the arguments of `AFortPlayerController::ServerEditBuildingActor(Actor, NewBuildingClass, RotationIterations, bMirrored)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditMatch {
    /// Index into the candidate list passed to [`find_match`].
    pub class_index: usize,
    pub rotation_iterations: i32,
    pub mirrored: bool,
}

/// `UBuildingEditModeSupport::vfunc_93` @1404ade30 (FindMatchingEditState) + `vfunc_94` @1404add60 (first candidate whose
/// metadata `Equals` the scratchpad) + the `UBuildingEditModeSupport_Wall::vfunc_93` @1404ae1b0 override.
///
/// `preview` is the edited tile state (`PreviewMetadata`), `owner_mirrored` the edited actor's `bMirrored`
/// (`+0x780` bit 1). `candidates` are the cooked patterns of `PossibleEditClasses` in order (`None` = class without
/// `EditModePatternData`, skipped by `vfunc_94`).
///
/// Order of attempts (each re-initialises the scratchpad from `preview` via `InitializeFromMetadata`, slot 73):
/// 1. scratch = preview (mirrored once if the owner is mirrored); rotation 0, mirror false.
/// 2. if CanRotate: rotate the scratch cumulatively 1, 2, 3 times (rotation = k).
/// 3. if CanMirror: rotation 0, mirror true; scratch = preview, mirrored once if the owner is NOT mirrored.
/// 4. if CanMirror && CanRotate: for k = 1..3: scratch = preview, mirror if owner mirrored, rotate k times, mirror.
///
/// Walls then turn a mirrored hit into `rotation 2, mirror false` (Wall override).
pub fn find_match(kind: PieceKind, preview: &Pattern, owner_mirrored: bool, candidates: &[Option<&Pattern>]) -> Option<EditMatch> {
    let (can_mirror, can_rotate) = support_flags(kind);
    let search = |scratch: &Pattern| -> Option<usize> {
        candidates.iter().position(|c| c.map_or(false, |p| p.equals(scratch)))
    };
    let fresh = |mirror: bool| -> Pattern {
        let mut s = preview.clone();
        if mirror { s.mirror(); }
        s
    };
    let mut found: Option<EditMatch> = None;
    // 1.
    let mut scratch = fresh(owner_mirrored);
    if let Some(i) = search(&scratch) {
        found = Some(EditMatch { class_index: i, rotation_iterations: 0, mirrored: false });
    }
    // 2.
    if found.is_none() && can_rotate {
        for k in 1..4 {
            scratch.rotate();
            if let Some(i) = search(&scratch) {
                found = Some(EditMatch { class_index: i, rotation_iterations: k, mirrored: false });
                break;
            }
        }
    }
    // 3.
    if found.is_none() && can_mirror {
        let s = fresh(!owner_mirrored);
        if let Some(i) = search(&s) {
            found = Some(EditMatch { class_index: i, rotation_iterations: 0, mirrored: true });
        }
        // 4.
        if found.is_none() && can_rotate {
            for k in 1..4 {
                let mut s = fresh(owner_mirrored);
                for _ in 0..k { s.rotate(); }
                s.mirror();
                if let Some(i) = search(&s) {
                    found = Some(EditMatch { class_index: i, rotation_iterations: k, mirrored: true });
                    break;
                }
            }
        }
    }
    // Wall override @1404ae1b0: a mirrored wall match becomes a 180 degree turn.
    if kind == PieceKind::Wall {
        if let Some(m) = found.as_mut() {
            if m.mirrored {
                m.rotation_iterations = 2;
                m.mirrored = false;
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bin(kind: PieceKind, n: usize, t: &[u8]) -> Pattern { Pattern { kind, n, tiles: Tiles::Binary(t.to_vec()) } }

    #[test]
    fn rotate_is_the_exe_cycle() {
        // 3x3 indices 0..9, one rotation: new[r][c] = old[c][N-1-r].
        let mut p = bin(PieceKind::Wall, 3, &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        p.rotate();
        assert_eq!(p.tiles, Tiles::Binary(vec![2, 5, 8, 1, 4, 7, 0, 3, 6]));
        let mut q = bin(PieceKind::Floor, 2, &[0, 1, 2, 3]);
        q.rotate();
        assert_eq!(q.tiles, Tiles::Binary(vec![1, 3, 0, 2]));
        for _ in 0..3 { q.rotate(); }
        assert_eq!(q.tiles, Tiles::Binary(vec![0, 1, 2, 3]));
    }

    #[test]
    fn mirror_swaps_columns() {
        let mut p = bin(PieceKind::Wall, 3, &[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        p.mirror();
        assert_eq!(p.tiles, Tiles::Binary(vec![2, 1, 0, 5, 4, 3, 8, 7, 6]));
    }

    #[test]
    fn either_matches_anything() {
        let a = bin(PieceKind::Wall, 3, &[2; 9]);
        let b = bin(PieceKind::Wall, 3, &[1, 0, 1, 0, 1, 0, 1, 0, 1]);
        assert!(a.equals(&b));
        let c = Pattern { kind: PieceKind::Stairs, n: 2, tiles: Tiles::Stair(vec![0; 4]) };
        assert!(!a.equals(&c));
    }

    #[test]
    fn wall_mirror_becomes_rotation_two() {
        let target = bin(PieceKind::Wall, 3, &[0, 0, 0, 1, 0, 0, 1, 0, 0]);
        let preview = bin(PieceKind::Wall, 3, &[0, 0, 0, 0, 0, 1, 0, 0, 1]);
        let m = find_match(PieceKind::Wall, &preview, false, &[Some(&target)]).unwrap();
        assert_eq!((m.rotation_iterations, m.mirrored), (2, false));
    }
}
