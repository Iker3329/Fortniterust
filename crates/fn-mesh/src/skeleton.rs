//! `USkeleton` and `FReferenceSkeleton` decoding (UE 4.16 / Fortnite 1.8).
//!
//! # USkeleton native layout (fn18, file version 513, `PKG_FilterEditorOnly`)
//!
//! After the tagged properties (`BoneTree`, `Sockets`, `BlendProfiles`, `SlotGroups`, optional `VirtualBones`) and
//! the 4 byte `bHasGuid` flag:
//!
//! | data | layout |
//! |------|--------|
//! | `ReferenceSkeleton` | `RefBoneInfo`: `i32 n`, n x (`FName` 8, `i32 parent`); `RefBonePose`: `i32 n`, n x `FTransform` (quat xyzw, translation, scale3D = 40 bytes); `NameToIndexMap`: `i32 n`, n x (`FName`, `i32`) |
//! | `AnimRetargetSources` | `TMap<FName, FReferencePose>`: `i32 n`, n x (`FName key`, `FName PoseName`, `TArray<FTransform>`) |
//! | `Guid` | 16 bytes |
//! | `SmartNames` | `TMap<FName, FSmartNameMapping>`: `i32 n`, n x (`FName`, `GuidMap`: `TMap<FName,FGuid>`, `CurveMetaDataMap`: `TMap<FName, FCurveMetaData>`) |
//!
//! `FCurveMetaData` is `bMaterial` (i32 bool), `bMorphtarget` (i32 bool), `LinkedBones` (`i32 n` x `FName`); the
//! `MaxLOD` byte only exists from `FAnimPhysObjectVersion::AddLODToCurveMetaData` (UE 4.18), and the `GuidMap`
//! only before `SmartNameRefactorForDeterministicCooking` (4.17); fn18 is on the old side of both. The
//! `StoreMarkerNamesOnSkeleton` strip-flags block (UE 4.21) is also absent, so the export ends right after
//! `SmartNames` (the decoder verifies this).
//!
//! The reference pose of the player skeleton has 93 bones. `RefBoneInfo[0].ParentIndex` is `-1`.
//!
//! `FBoneNode.TranslationRetargetingMode` (BoneTree property) is the enum
//! `EBoneTranslationRetargetingMode::{Animation,Skeleton,AnimationScaled,AnimationRelative,OrientAndScale}`; a
//! missing tag means the default `Animation`.

use crate::common::*;
use fn_ueio::props::PropValue;
use fn_ueio::{invalid, Package, Reader, Result};

/// `EBoneTranslationRetargetingMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TranslationRetarget {
    /// Use the translation stored in the animation (default).
    #[default]
    Animation,
    /// Use the translation of the target skeleton's reference pose.
    Skeleton,
    /// Animation translation scaled by `|TargetBoneTranslation| / |SourceBoneTranslation|`.
    AnimationScaled,
    /// Animation translation plus the difference to the source reference pose.
    AnimationRelative,
    OrientAndScale,
}

impl TranslationRetarget {
    fn from_text(s: &str) -> Option<TranslationRetarget> {
        let t = s.rsplit("::").next().unwrap_or(s);
        Some(match t {
            "Animation" => TranslationRetarget::Animation,
            "Skeleton" => TranslationRetarget::Skeleton,
            "AnimationScaled" => TranslationRetarget::AnimationScaled,
            "AnimationRelative" => TranslationRetarget::AnimationRelative,
            "OrientAndScale" => TranslationRetarget::OrientAndScale,
            _ => return None,
        })
    }
    fn from_index(i: u8) -> TranslationRetarget {
        match i {
            1 => TranslationRetarget::Skeleton,
            2 => TranslationRetarget::AnimationScaled,
            3 => TranslationRetarget::AnimationRelative,
            4 => TranslationRetarget::OrientAndScale,
            _ => TranslationRetarget::Animation,
        }
    }
}

/// One bone of a reference skeleton (UE space, cm, left handed Z-up, unmodified file data).
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Parent bone index, `-1` for the root.
    pub parent: i32,
    /// Reference pose translation (cm).
    pub ref_pos: [f32; 3],
    /// Reference pose rotation quaternion (x, y, z, w) exactly as stored by UE.
    pub ref_rot: [f32; 4],
    pub ref_scale: [f32; 3],
    /// Translation retargeting mode (from the Skeleton `BoneTree`; `Animation` for mesh-embedded skeletons).
    pub retarget: TranslationRetarget,
}

/// `USkeletalMeshSocket`.
#[derive(Debug, Clone, PartialEq)]
pub struct Socket {
    pub name: String,
    pub bone: String,
    pub location: [f32; 3],
    /// `FRotator` as stored: (pitch, yaw, roll) in degrees.
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}

/// `FVirtualBone` (UE 4.17+; never present in fn18 data, kept for forward compatibility).
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualBone {
    pub source: String,
    pub target: String,
    pub name: String,
}

/// `FAnimSlotGroup`.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotGroup {
    pub name: String,
    pub slots: Vec<String>,
}

/// `FReferencePose` of `AnimRetargetSources`.
#[derive(Debug, Clone, PartialEq)]
pub struct RetargetSource {
    pub name: String,
    pub pose: Vec<Transform>,
}

/// `FCurveMetaData` entry of the skeleton smart-name container.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveMeta {
    pub name: String,
    pub material: bool,
    pub morph_target: bool,
    pub linked_bones: Vec<String>,
}

/// Decoded `USkeleton`.
#[derive(Debug, Clone)]
pub struct Skeleton {
    /// Asset name of the export (e.g. `Fortnite_M_Avg_Player_Skeleton`).
    pub name: String,
    pub bones: Vec<Bone>,
    pub sockets: Vec<Socket>,
    pub virtual_bones: Vec<VirtualBone>,
    pub slot_groups: Vec<SlotGroup>,
    pub retarget_sources: Vec<RetargetSource>,
    pub guid: [u8; 16],
    /// Curve names registered in the `SmartNames` container (animation curve names, with metadata).
    pub curves: Vec<CurveMeta>,
}

impl Skeleton {
    /// Decode the first `Skeleton` export of the package.
    pub fn parse(pkg: &Package) -> Result<Skeleton> {
        let idx = find_class(pkg, "Skeleton")?;
        let ExportBody { props, mut reader, end } = export_body(pkg, idx)?;
        let names = &pkg.names;
        let mut bones = read_ref_skeleton(&mut reader, names)?;

        // AnimRetargetSources
        let n = array_count(&mut reader, 8)?;
        let mut retarget_sources = Vec::with_capacity(n);
        for _ in 0..n {
            let _key = fname(&mut reader, names)?;
            let name = fname(&mut reader, names)?;
            let m = array_count(&mut reader, 40)?;
            let mut pose = Vec::with_capacity(m);
            for _ in 0..m {
                pose.push(Transform::read(&mut reader)?);
            }
            retarget_sources.push(RetargetSource { name, pose });
        }
        let guid = reader.guid()?;

        // SmartNames: TMap<FName, FSmartNameMapping>
        let mut curves = Vec::new();
        let n = array_count(&mut reader, 8)?;
        for _ in 0..n {
            let _container_name = fname(&mut reader, names)?;
            let g = array_count(&mut reader, 24)?; // GuidMap (UE4.13-4.16)
            reader.skip(g * 24)?;
            let c = array_count(&mut reader, 16)?; // CurveMetaDataMap
            for _ in 0..c {
                let name = fname(&mut reader, names)?;
                let material = bool32(&mut reader)?;
                let morph_target = bool32(&mut reader)?;
                let l = array_count(&mut reader, 8)?;
                let mut linked_bones = Vec::with_capacity(l);
                for _ in 0..l {
                    linked_bones.push(fname(&mut reader, names)?);
                }
                curves.push(CurveMeta { name, material, morph_target, linked_bones });
            }
        }
        if reader.tell() != end {
            return invalid(format!("Skeleton export not fully consumed: stopped at +{} of {}", reader.tell() - pkg.export_range(idx).0, end - pkg.export_range(idx).0));
        }

        // BoneTree (translation retargeting)
        if let Some(arr) = props.array("BoneTree") {
            for (i, v) in arr.iter().enumerate() {
                let Some(b) = bones.get_mut(i) else { break };
                if let Some(f) = v.as_fields() {
                    if let Some(pv) = f.get("TranslationRetargetingMode") {
                        b.retarget = match pv {
                            PropValue::Enum(s) | PropValue::Name(s) | PropValue::Str(s) => TranslationRetarget::from_text(s).unwrap_or_default(),
                            PropValue::Byte(x) => TranslationRetarget::from_index(*x),
                            PropValue::Int(x) => TranslationRetarget::from_index(*x as u8),
                            _ => TranslationRetarget::Animation,
                        };
                    }
                }
            }
        }

        let sockets = read_sockets(pkg, props.array("Sockets"));
        let virtual_bones = props
            .array("VirtualBones")
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        let f = v.as_fields()?;
                        Some(VirtualBone {
                            source: f.text("SourceBoneName").unwrap_or("").to_string(),
                            target: f.text("TargetBoneName").unwrap_or("").to_string(),
                            name: f.text("VirtualBoneName").unwrap_or("").to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let slot_groups = props
            .array("SlotGroups")
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        let f = v.as_fields()?;
                        let slots = match f.get("SlotNames") {
                            Some(PropValue::Array(s)) => s.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
                            _ => Vec::new(),
                        };
                        Some(SlotGroup { name: f.text("GroupName").unwrap_or("").to_string(), slots })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Skeleton {
            name: pkg.exports[idx].object_name.clone(),
            bones,
            sockets,
            virtual_bones,
            slot_groups,
            retarget_sources,
            guid,
            curves,
        })
    }

    /// Bone index by name (exact match).
    pub fn bone_index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    /// Bone index by name, ASCII case-insensitive (UE bone names are case-insensitive).
    pub fn bone_index_ci(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }
}

/// `FReferenceSkeleton` native serialisation: bone infos, reference pose, name->index map (map is validated and dropped).
pub(crate) fn read_ref_skeleton(r: &mut Reader, names: &[String]) -> Result<Vec<Bone>> {
    let n = array_count(r, 12)?;
    let mut bones = Vec::with_capacity(n);
    for _ in 0..n {
        let name = fname(r, names)?;
        let parent = r.i32()?;
        bones.push(Bone { name, parent, ref_pos: [0.0; 3], ref_rot: [0.0, 0.0, 0.0, 1.0], ref_scale: [1.0; 3], retarget: TranslationRetarget::Animation });
    }
    let m = array_count(r, 40)?;
    if m != n {
        return invalid(format!("RefBonePose count {m} != RefBoneInfo count {n}"));
    }
    for b in bones.iter_mut() {
        let t = Transform::read(r)?;
        b.ref_rot = t.rotation;
        b.ref_pos = t.translation;
        b.ref_scale = t.scale;
    }
    let k = array_count(r, 12)?;
    r.skip(k * 12)?; // NameToIndexMap: FName + i32 each
    if let Some(first) = bones.first_mut() {
        if first.parent != -1 {
            first.parent = -1; // VER_UE4_FIXUP_ROOTBONE_PARENT
        }
    }
    Ok(bones)
}

/// Decode `USkeletalMeshSocket` objects referenced from an `Object` array property (same-package exports).
pub(crate) fn read_sockets(pkg: &Package, refs: Option<&Vec<PropValue>>) -> Vec<Socket> {
    let mut out = Vec::new();
    let Some(refs) = refs else { return out };
    for v in refs {
        let PropValue::Object(i) = v else { continue };
        if *i <= 0 || *i as usize > pkg.exports.len() {
            continue;
        }
        let Ok(body) = export_body(pkg, *i as usize - 1) else { continue };
        let p = &body.props;
        let vec3 = |name: &str, def: [f32; 3]| match p.get(name) {
            Some(PropValue::Vec3(v)) => *v,
            _ => def,
        };
        out.push(Socket {
            name: p.text("SocketName").unwrap_or("").to_string(),
            bone: p.text("BoneName").unwrap_or("").to_string(),
            location: vec3("RelativeLocation", [0.0; 3]),
            rotation: vec3("RelativeRotation", [0.0; 3]),
            scale: vec3("RelativeScale", [1.0; 3]),
        });
    }
    out
}
