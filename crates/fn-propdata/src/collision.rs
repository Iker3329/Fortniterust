//! `collision.json`: the cooked `UBodySetup` simple collision of the prop's static mesh (UE local space, cm).
use crate::util::*;
use serde_json::Value;
use std::path::Path;

/// One simple-collision primitive. All values are in the **mesh's local UE space** (cm, left handed, Z up); `rotation`
/// is a quaternion `(x, y, z, w)`.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// `FKBoxElem`: `half_extent` = half of the stored X/Y/Z lengths.
    Box { center: [f32; 3], rotation: [f32; 4], half_extent: [f32; 3] },
    /// `FKSphereElem`
    Sphere { center: [f32; 3], radius: f32 },
    /// `FKSphylElem` (capsule along local Z): `half_length` = half of the *cylinder* section; total half height = `half_length + radius`.
    Capsule { center: [f32; 3], rotation: [f32; 4], radius: f32, half_length: f32 },
    /// `FKConvexElem`: hull vertices (local space of the element, then `transform` applies). Faces are not stored by the
    /// cooked data we decode (they live in the PhysX blob): rebuild the hull from the points.
    Convex { vertices: Vec<[f32; 3]>, transform_translation: [f32; 3], transform_rotation: [f32; 4], transform_scale: [f32; 3], bounds_min: [f32; 3], bounds_max: [f32; 3] },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Collision {
    pub shapes: Vec<Shape>,
    /// `BodySetup.CollisionTraceFlag` (`CTF_UseDefault`, `CTF_UseSimpleAsComplex`, ...); empty when absent.
    pub trace_flag: String,
    /// `BodyInstance.CollisionProfileName` of the body setup default instance, e.g. `BlockAll`.
    pub profile: String,
    /// `ECollisionEnabled::...` string of the default instance.
    pub enabled: String,
    /// Channels the default instance responds to (`FCC_Trace_Weapon` -> `ECR_Block`, ...).
    pub responses: Vec<(String, String)>,
    /// Physical material object path when set.
    pub phys_material: Option<String>,
    /// Union box of all primitives (cm, local space).
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

impl Collision {
    pub fn load(path: &Path) -> Result<Collision> {
        Collision::from_json(&read_json(path)?)
    }

    pub fn from_json(j: &Value) -> Result<Collision> {
        let mut c = Collision {
            trace_flag: str_of(j, "trace_flag").unwrap_or_default(),
            profile: str_of(j, "profile").unwrap_or_default(),
            enabled: str_of(j, "collision_enabled").unwrap_or_default(),
            phys_material: str_of(j, "phys_material"),
            ..Default::default()
        };
        if let Some(r) = j.get("responses").and_then(|x| x.as_array()) {
            for e in r {
                if let (Some(a), Some(b)) = (str_of(e, "channel"), str_of(e, "response")) {
                    c.responses.push((a, b));
                }
            }
        }
        for s in j.get("shapes").and_then(|x| x.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            let ty = req_str(s, "type", "collision shape")?;
            let center = || arr_f32::<3>(s.get("center")).unwrap_or([0.0; 3]);
            let rot = || arr_f32::<4>(s.get("rotation")).unwrap_or([0.0, 0.0, 0.0, 1.0]);
            let shape = match ty.as_str() {
                "box" => Shape::Box { center: center(), rotation: rot(), half_extent: arr_f32::<3>(s.get("half_extent")).ok_or_else(|| Error::Schema("box without half_extent".into()))? },
                "sphere" => Shape::Sphere { center: center(), radius: f32_of(s, "radius").ok_or_else(|| Error::Schema("sphere without radius".into()))? },
                "capsule" => Shape::Capsule {
                    center: center(),
                    rotation: rot(),
                    radius: f32_of(s, "radius").ok_or_else(|| Error::Schema("capsule without radius".into()))?,
                    half_length: f32_of(s, "half_length").unwrap_or(0.0),
                },
                "convex" => {
                    let vs = s.get("vertices").and_then(|x| x.as_array()).ok_or_else(|| Error::Schema("convex without vertices".into()))?;
                    let vertices: Vec<[f32; 3]> = vs.iter().filter_map(|v| arr_f32::<3>(Some(v))).collect();
                    let t = s.get("transform");
                    Shape::Convex {
                        vertices,
                        transform_translation: t.and_then(|t| arr_f32::<3>(t.get("translation"))).unwrap_or([0.0; 3]),
                        transform_rotation: t.and_then(|t| arr_f32::<4>(t.get("rotation"))).unwrap_or([0.0, 0.0, 0.0, 1.0]),
                        transform_scale: t.and_then(|t| arr_f32::<3>(t.get("scale"))).unwrap_or([1.0; 3]),
                        bounds_min: arr_f32::<3>(s.get("bounds_min")).unwrap_or([0.0; 3]),
                        bounds_max: arr_f32::<3>(s.get("bounds_max")).unwrap_or([0.0; 3]),
                    }
                }
                other => return schema(format!("unknown collision shape '{other}'")),
            };
            c.shapes.push(shape);
        }
        c.bounds_min = arr_f32::<3>(j.get("bounds_min")).unwrap_or([0.0; 3]);
        c.bounds_max = arr_f32::<3>(j.get("bounds_max")).unwrap_or([0.0; 3]);
        Ok(c)
    }

    /// Does the prop have any simple collision?
    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Does the default instance block the pawn / weapon traces? (`BlockAll`-style profiles; derived from `responses` +
    /// `profile`, `false` when collision is disabled.)
    pub fn blocks_weapon_traces(&self) -> bool {
        if self.enabled.contains("NoCollision") {
            return false;
        }
        self.responses.iter().any(|(c, r)| c.contains("Weapon") && r.contains("Block")) || self.profile == "BlockAll"
    }
}
