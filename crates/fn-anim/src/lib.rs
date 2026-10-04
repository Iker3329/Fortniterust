//! `fn-anim`: UE 4.16 (Fortnite 1.8) animation decoding, sampling and baking.
//!
//! * [`AnimSequence::parse`]: tagged properties + native tail of a cooked `UAnimSequence` (see [`sequence`] for the
//!   byte-exact serialization order and the `CompressedTrackOffsets` layouts) and all key codecs ([`codec`]):
//!   `ConstantKeyLerp`, `VariableKeyLerp`, `PerTrackCompression` x `Float96NoW`, `Fixed48NoW`, `Fixed32NoW`,
//!   `IntervalFixed32NoW`, `Float32NoW`, `Identity`, plus raw (uncooked) tracks.
//! * [`SkeletonInfo`]: bones / hierarchy / reference pose / translation retarget modes / retarget source poses from a
//!   `USkeleton` asset (own parser; `SkeletonInfo::from_mesh_skeleton` behind the `mesh` feature converts from
//!   `fn_mesh::Skeleton`).
//! * [`AnimSequence::pose_at`], [`AnimSequence::pose_at_frame`], [`AnimSequence::bake_frames`]: local-space bone
//!   transforms (UE axes, centimetres) with UE's key spacing and nlerp blending.
//! * [`retarget`]: translation retargeting helpers (`Animation`, `Skeleton`, `AnimationScaled`, ...), equivalent to
//!   the old runtime's `retarget_pose`/`fit`.
//! * [`BlendSpace::parse`]: `BlendSpace`, `BlendSpace1D`, `AimOffsetBlendSpace` assets, with UE's
//!   `GetSamplesFromBlendInput` grid evaluation ([`BlendSpace::sample_weights`]).
//! * [`AnimMontage::parse`], [`AnimComposite::parse`]: slot tracks, sections, segments, notifies (no evaluation).
//! * [`md5`]: writer/reader for the old UEViewer `.md5anim` text format (same axis conventions and comment
//!   metadata); the writer reproduces all 388 existing exports **byte for byte**.
//! * [`fnanim`]: compact binary `.fnanim` container (baked frames + metadata) loadable without any UE code
//!   (~19x smaller than md5anim in lossless F32 mode, ~26x with Q16).
//! * [`verify`]: numeric comparison helpers used by the tests/examples.
//!
//! # Conventions
//! Native values are UE's: centimetres, X forward / Y right / Z up, left-handed, quaternions `[x, y, z, w]`.
//! The md5 export additionally flips Y (`MIRROR_MESH`), conjugates rotations (non-root bones at load, the root at
//! export), drops `w` (sign normalised so that `w >= 0`) and scales positions by accumulated ancestor scale.
//!
//! # Facts discovered on the Fortnite 1.8 data (388 `AnimSequence` assets, one 266-bone skeleton)
//!
//! * Serialization order, offsets table layout and key packing are documented in [`sequence`] and [`codec`].
//! * **All 388 clips use `AKF_PerTrackCompression`**; the sequence-level translation/rotation/scale formats are
//!   `ACF_Identity` (the real format is in every track header). Per-track formats seen: translation `Float96NoW`
//!   (masks 1/3/5/6/7), `IntervalFixed32NoW` (3/5/7), `Fixed48NoW` (7), none (offset -1); rotation `Fixed48NoW` (masks
//!   1/3/4/5/6/7), `Float96NoW` (7), none (offset -1); scale (220 clips) `Fixed48NoW` / `Float96NoW` /
//!   `IntervalFixed32NoW`. Every track has either 1 key or `NumFrames` keys, no frame-index tables (the
//!   variable-key paths are covered by hand-built tests only). `Float32NoW` and `Fixed32NoW` do not occur.
//! * A per-track offset of `-1` means "no data": UE substitutes the reference pose (the additive identity for
//!   additive clips); UEViewer exports a null key (0 vector / identity) instead - [`Track`] keeps the channel empty,
//!   [`md5::export`] emulates UEViewer.
//! * The 4.16 per-track byte stream has the scale data in a different order than the offsets describe when scale
//!   offsets exist ("ByteSwapOut" bug); [`sequence`] re-lays it out like UEViewer's `TransferPerTrackData`
//!   (220 clips; positions/rotations of those clips verify bit-exactly).
//! * Additive scale keys hold `scale - 1` (all within +-0.16); `BoneTransform::ADDITIVE_IDENTITY` has scale 0.
//! * Curves: `FSmartName` is `FName + u16 UID`; `FRichCurveKey` elements are 27 native bytes behind an array-of-struct
//!   tag (the generic property reader returns them as `Raw`; decoded in [`sequence`]).
//!
//! # Verified discrepancies of the old md5anim export (not decoder bugs)
//!
//! 1. `frameRate = NumFrames / SequenceLength * RateScale`, but UE samples keys at `(NumFrames - 1) / SequenceLength`;
//!    the old runtime therefore plays every clip `NumFrames/(NumFrames-1)` too fast (25 frames: +4.2 %).
//!    [`AnimSequence::pose_at`] uses UE's spacing, [`AnimSequence::frame_rate`] the md5 value.
//! 2. Bones/channels without data are exported as zero position / identity rotation, not the reference pose.
//! 3. Rotations between frames: UEViewer slerps (with double precision `sin`/`atan2`/`sqrt`), UE nlerps;
//!    [`RotationBlend`] selects either. Differences are <= ~1 degree for the largest frame-to-frame motions.
//! 4. UEViewer's key lookup for evenly spaced keys is `Frame / NumFrames * NumKeys`, which yields tiny fractional
//!    interpolation noise at integer frames (reproduced in [`md5`] to get identical text).
//! 5. `AnimationRelative` retargeting is not implemented by UEViewer (nor by [`retarget`]).

pub mod blendspace;
pub mod codec;
pub mod fnanim;
pub mod md5;
pub mod montage;
pub mod retarget;
pub mod sequence;
pub mod skeleton;
pub mod verify;

pub use blendspace::{BlendAxis, BlendSample, BlendSpace, BlendSpaceKind};
pub use codec::{CompressionFormat, KeyFormat};
pub use fnanim::{FnAnim, Precision};
pub use montage::{AnimComposite, AnimMontage};
pub use sequence::{AdditiveAnimType, AdditiveInfo, AnimSequence, CurveKey, FloatCurve, Interpolation, Notify, RefPoseType, RotationBlend, SyncMarker, Track, TrackSample};
pub use skeleton::{BoneTransform, RetargetSource, SkeletonInfo, TranslationRetarget};

use std::fmt;

/// Errors of this crate.
#[derive(Debug, Clone)]
pub enum Error {
    /// Underlying package / reader error.
    Io(fn_ueio::Error),
    /// Malformed or unsupported data.
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<fn_ueio::Error> for Error {
    fn from(e: fn_ueio::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Format(msg.into()))
}
