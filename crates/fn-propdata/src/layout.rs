//! Placement data extracted from `Athena_Terrain` (`layout/all.json`, `layout/sample.json`).
use crate::util::*;
use std::path::Path;

/// One placed prop. Positions / rotations are in **UE world space** (cm, left handed, Z up), exactly as stored in the
/// level; use [`PropInstance::game_position`] / [`PropInstance::game_yaw`] for the runtime's metre / Y-up convention.
#[derive(Debug, Clone, PartialEq)]
pub struct PropInstance {
    /// Prop kind id (the Blueprint class name, e.g. `Athena_Tree_Pine_3`), a key of [`crate::PropCatalog`].
    pub kind: String,
    /// World position, cm. In a sample layout: relative to the sample centre (`abs_cm` has the world position).
    pub pos_cm: [f32; 3],
    /// Absolute world position in cm (equal to `pos_cm` for the full layout).
    pub abs_cm: [f32; 3],
    /// `FRotator` (pitch, yaw, roll) in degrees.
    pub rot_deg: [f32; 3],
    pub scale: [f32; 3],
}

impl PropInstance {
    /// `(x, y, z)_UE cm -> (x, z, y)` metres: the convention of the exported glTF meshes.
    pub fn game_position(&self) -> [f32; 3] {
        ue_to_game_position(self.pos_cm)
    }
    /// Rotation about +Y (right handed, radians) that reproduces the UE yaw after the `(x, z, y)` axis swap
    /// (the swap is a reflection, so the angle changes sign). Pitch and roll are ignored (see `rot_deg`).
    pub fn game_yaw(&self) -> f32 {
        -self.rot_deg[1].to_radians()
    }
    /// Mean of the three scale components (the placed scales are uniform in the Athena level).
    pub fn uniform_scale(&self) -> f32 {
        (self.scale[0] + self.scale[1] + self.scale[2]) / 3.0
    }
}

pub fn ue_to_game_position(p: [f32; 3]) -> [f32; 3] {
    [p[0] * 0.01, p[2] * 0.01, p[1] * 0.01]
}

/// A reference layout: a subset (the 40-instance sample) or the whole level (~9600 harvestables).
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub map: String,
    /// World position (cm) the relative `pos_cm` values are measured from; `[0;3]` for the full layout.
    pub center_cm: [f32; 3],
    pub radius_cm: f32,
    pub instances: Vec<PropInstance>,
}

impl Layout {
    /// `layout/sample.json`
    pub fn load_sample(path: &Path) -> Result<Layout> {
        let j = read_json(path)?;
        let mut l = Layout { map: str_of(&j, "map").unwrap_or_default(), center_cm: arr_f32::<3>(j.get("center_abs")).unwrap_or([0.0; 3]), radius_cm: f32_of(&j, "radius_cm").unwrap_or(0.0), instances: vec![] };
        for e in j.get("instances").and_then(|x| x.as_array()).ok_or_else(|| Error::Schema("sample: instances".into()))? {
            l.instances.push(PropInstance {
                kind: req_str(e, "class", "sample instance")?,
                pos_cm: arr_f32::<3>(e.get("pos")).ok_or_else(|| Error::Schema("sample instance: pos".into()))?,
                abs_cm: arr_f32::<3>(e.get("abs")).unwrap_or([0.0; 3]),
                rot_deg: arr_f32::<3>(e.get("rot")).unwrap_or([0.0; 3]),
                scale: arr_f32::<3>(e.get("scale")).unwrap_or([1.0; 3]),
            });
        }
        Ok(l)
    }

    /// `layout/all.json`
    pub fn load_all(path: &Path) -> Result<Layout> {
        let j = read_json(path)?;
        let classes: Vec<String> = j.get("classes").and_then(|x| x.as_array()).ok_or_else(|| Error::Schema("all: classes".into()))?.iter().filter_map(|c| c.as_str().map(String::from)).collect();
        let mut l = Layout { map: str_of(&j, "map").unwrap_or_default(), ..Default::default() };
        for e in j.get("instances").and_then(|x| x.as_array()).ok_or_else(|| Error::Schema("all: instances".into()))? {
            let a = e.as_array().ok_or_else(|| Error::Schema("all: instance row".into()))?;
            if a.len() < 10 {
                return schema("all: instance row shorter than 10");
            }
            let f = |i: usize| a[i].as_f64().unwrap_or(0.0) as f32;
            let ci = a[0].as_u64().unwrap_or(0) as usize;
            let kind = classes.get(ci).cloned().ok_or_else(|| Error::Schema(format!("all: class index {ci}")))?;
            let pos = [f(1), f(2), f(3)];
            l.instances.push(PropInstance { kind, pos_cm: pos, abs_cm: pos, rot_deg: [f(4), f(5), f(6)], scale: [f(7), f(8), f(9)] });
        }
        Ok(l)
    }

    /// Instances per kind id.
    pub fn counts(&self) -> std::collections::BTreeMap<&str, usize> {
        let mut m = std::collections::BTreeMap::new();
        for i in &self.instances {
            *m.entry(i.kind.as_str()).or_insert(0) += 1;
        }
        m
    }
}
