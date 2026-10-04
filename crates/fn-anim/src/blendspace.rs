//! `UBlendSpace`, `UBlendSpace1D`, `UAimOffsetBlendSpace` (and the `...1D` aim offset) assets.
//!
//! All data is serialized as tagged properties (UE 4.16, `UBlendSpaceBase`):
//!
//! | property | meaning |
//! |----------|---------|
//! | `BlendParameters[0..2]` (`FBlendParameter`) | `DisplayName`, `Min` (0), `Max` (100), `GridNum` (4), `bSnapToGrid`, `bWrapInput`; 1D assets only have index 0. Fixed-size array members are stored with `ArrayIndex`; defaults are omitted. |
//! | `InterpolationParam[0..2]` (`FInterpolationParameter`) | `InterpolationTime` (0) and `InterpolationType` (`BSIT_Average`, `BSIT_Linear`, `BSIT_Cubic`, `BSIT_EaseInOut`, `BSIT_ExponentialDecay`, `BSIT_SpringDamper`) per axis |
//! | `SampleData` (`FBlendSample`) | `Animation` (object ref), `SampleValue` (FVector: x, y, z), `RateScale` (1), `bIsValid` |
//! | `GridSamples` (`FEditorElement`) | `Indices[3]` (-1) and `Weights[3]` (0): the pre-computed triangulation. `(GridNum0 + 1) * (GridNum1 + 1)` elements, **axis 1 varies fastest** (`index = x * (GridNum1 + 1) + y`); 1D: `GridNum0 + 1` elements |
//! | `TargetWeightInterpolationSpeedPerSec` | speed of the weight smoothing (`FInterpTo`) |
//! | `SampleIndexWithMarkers` | sample used as the sync-marker leader (-1 default) |
//! | `AxisToScaleAnimation` | `BSA_None` / `BSA_X` / `BSA_Y` |
//! | `bRotationBlendInMeshSpace`, `NotifyTriggerMode`, `PerBoneBlend` (`FPerBoneInterpolation`: `BoneReference.BoneName`, `InterpolationSpeedPerSec`), `bInterpolateUsingGrid`, `Skeleton` | self-describing |
//!
//! Evaluating a blend input is exactly UE's `GetSamplesFromBlendInput`: normalise to grid coordinates, find the cell,
//! blend the 4 (2 for 1D) corner elements bilinearly and accumulate each element's per-sample weights.

use crate::{err, Error, Result};
use fn_ueio::props::{read_props, Ctx, Prop, PropList, PropValue};
use fn_ueio::{Package, Reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendSpaceKind {
    BlendSpace,
    BlendSpace1D,
    AimOffsetBlendSpace,
    AimOffsetBlendSpace1D,
}

impl BlendSpaceKind {
    pub fn from_class(c: &str) -> Option<Self> {
        Some(match c {
            "BlendSpace" => Self::BlendSpace,
            "BlendSpace1D" => Self::BlendSpace1D,
            "AimOffsetBlendSpace" => Self::AimOffsetBlendSpace,
            "AimOffsetBlendSpace1D" => Self::AimOffsetBlendSpace1D,
            _ => return None,
        })
    }
    pub fn is_1d(self) -> bool {
        matches!(self, Self::BlendSpace1D | Self::AimOffsetBlendSpace1D)
    }
    pub fn is_aim_offset(self) -> bool {
        matches!(self, Self::AimOffsetBlendSpace | Self::AimOffsetBlendSpace1D)
    }
}

/// One blend axis (`FBlendParameter` + `FInterpolationParameter`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlendAxis {
    pub name: String,
    pub min: f32,
    pub max: f32,
    /// `GridNum`: number of grid divisions (the grid has `GridNum + 1` points on this axis).
    pub grid_divisions: u32,
    pub snap_to_grid: bool,
    pub wrap_input: bool,
    /// Input smoothing time (`InterpolationTime`) in seconds.
    pub interp_time: f32,
    /// `InterpolationType` without the enum prefix, e.g. `BSIT_Cubic`.
    pub interp_type: String,
}

impl Default for BlendAxis {
    fn default() -> Self {
        BlendAxis { name: String::new(), min: 0.0, max: 100.0, grid_divisions: 4, snap_to_grid: false, wrap_input: false, interp_time: 0.0, interp_type: "BSIT_Average".into() }
    }
}

/// One sample point (`FBlendSample`).
#[derive(Debug, Clone, PartialEq)]
pub struct BlendSample {
    /// Object name of the referenced `AnimSequence` (`None` for a null reference).
    pub animation: Option<String>,
    /// Import path (`/Game/.../Package.Object`) of the animation, to locate the package.
    pub animation_path: Option<String>,
    /// `(x, y, z)`; 1D blend spaces only use `x`.
    pub position: [f32; 3],
    pub rate_scale: f32,
    pub is_valid: bool,
}

/// One grid point (`FEditorElement`): up to three samples with weights (a triangle).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridElement {
    pub indices: [i32; 3],
    pub weights: [f32; 3],
}

/// `FPerBoneInterpolation`.
#[derive(Debug, Clone, PartialEq)]
pub struct PerBoneBlend {
    pub bone: String,
    pub interpolation_speed_per_sec: f32,
}

#[derive(Debug, Clone)]
pub struct BlendSpace {
    pub name: String,
    pub kind: BlendSpaceKind,
    pub skeleton: Option<String>,
    pub axes: Vec<BlendAxis>,
    pub samples: Vec<BlendSample>,
    pub grid: Vec<GridElement>,
    pub target_weight_interpolation_speed_per_sec: f32,
    pub sample_index_with_markers: i32,
    /// `BSA_None`, `BSA_X` or `BSA_Y`.
    pub axis_to_scale_animation: String,
    pub rotation_blend_in_mesh_space: bool,
    /// `AllAnimations` / `HighestWeightedAnimation` / `None` (prefix stripped), `None` when unset.
    pub notify_trigger_mode: Option<String>,
    pub per_bone_blend: Vec<PerBoneBlend>,
    pub interpolate_using_grid: bool,
}

fn strip(s: &str) -> String {
    s.rsplit("::").next().unwrap_or(s).to_string()
}

/// All props with `name`, as `(array_index, value)`.
fn indexed<'a>(list: &'a PropList, name: &str) -> Vec<(usize, &'a PropValue)> {
    list.0.iter().filter(|p| p.name == name).map(|p: &Prop| (p.array_index.max(0) as usize, &p.value)).collect()
}

impl BlendSpace {
    /// Parse the (first) blend space export of a package.
    pub fn parse(pkg: &Package) -> Result<BlendSpace> {
        let idx = (0..pkg.exports.len())
            .find(|&i| BlendSpaceKind::from_class(&pkg.export_class(i)).is_some())
            .ok_or_else(|| Error::Format("package has no BlendSpace export".into()))?;
        let kind = BlendSpaceKind::from_class(&pkg.export_class(idx)).unwrap();
        let (start, end) = pkg.export_range(idx);
        let mut r = Reader::at(&pkg.data[..end], start);
        let ctx = Ctx { names: &pkg.names, file_version: pkg.summary.file_version };
        let props = read_props(&mut r, &ctx)?;

        let num_axes = if kind.is_1d() { 1 } else { 2 };
        let mut axes = vec![BlendAxis::default(); num_axes];
        for (i, v) in indexed(&props, "BlendParameters") {
            if i >= num_axes {
                continue;
            }
            if let Some(f) = v.as_fields() {
                let a = &mut axes[i];
                if let Some(n) = f.text("DisplayName") {
                    a.name = n.to_string();
                }
                if let Some(x) = f.float("Min") {
                    a.min = x as f32;
                }
                if let Some(x) = f.float("Max") {
                    a.max = x as f32;
                }
                if let Some(x) = f.int("GridNum") {
                    a.grid_divisions = x.max(0) as u32;
                }
                a.snap_to_grid = f.bool("bSnapToGrid").unwrap_or(false);
                a.wrap_input = f.bool("bWrapInput").unwrap_or(false);
            }
        }
        for (i, v) in indexed(&props, "InterpolationParam") {
            if i >= num_axes {
                continue;
            }
            if let Some(f) = v.as_fields() {
                let a = &mut axes[i];
                if let Some(x) = f.float("InterpolationTime") {
                    a.interp_time = x as f32;
                }
                if let Some(t) = f.text("InterpolationType") {
                    a.interp_type = strip(t);
                }
            }
        }

        let mut samples = Vec::new();
        if let Some(PropValue::Array(items)) = props.get("SampleData") {
            for it in items {
                let Some(f) = it.as_fields() else { continue };
                let obj = f.object("Animation").unwrap_or(0);
                samples.push(BlendSample {
                    animation: if obj == 0 { None } else { Some(pkg.index_name(obj)) },
                    animation_path: if obj < 0 { Some(pkg.import_path(obj)) } else { None },
                    position: match f.get("SampleValue") {
                        Some(PropValue::Vec3(v)) => *v,
                        _ => [0.0; 3],
                    },
                    rate_scale: f.float("RateScale").map(|x| x as f32).unwrap_or(1.0),
                    is_valid: f.bool("bIsValid").unwrap_or(true),
                });
            }
        }
        let mut grid = Vec::new();
        if let Some(PropValue::Array(items)) = props.get("GridSamples") {
            for it in items {
                let Some(f) = it.as_fields() else { continue };
                let mut e = GridElement { indices: [-1; 3], weights: [0.0; 3] };
                for (i, v) in indexed(f, "Indices") {
                    if let (true, PropValue::Int(x)) = (i < 3, v) {
                        e.indices[i] = *x as i32;
                    }
                }
                for (i, v) in indexed(f, "Weights") {
                    if let (true, Some(x)) = (i < 3, v.as_f64()) {
                        e.weights[i] = x as f32;
                    }
                }
                grid.push(e);
            }
        }
        let mut per_bone_blend = Vec::new();
        if let Some(PropValue::Array(items)) = props.get("PerBoneBlend") {
            for it in items {
                if let Some(f) = it.as_fields() {
                    per_bone_blend.push(PerBoneBlend {
                        bone: f.fields("BoneReference").and_then(|b| b.text("BoneName")).unwrap_or("").to_string(),
                        interpolation_speed_per_sec: f.float("InterpolationSpeedPerSec").unwrap_or(6.0) as f32,
                    });
                }
            }
        }
        Ok(BlendSpace {
            name: pkg.exports[idx].object_name.clone(),
            kind,
            skeleton: props.object("Skeleton").filter(|&i| i != 0).map(|i| pkg.index_name(i)),
            axes,
            samples,
            grid,
            target_weight_interpolation_speed_per_sec: props.float("TargetWeightInterpolationSpeedPerSec").unwrap_or(0.0) as f32,
            sample_index_with_markers: props.int("SampleIndexWithMarkers").map(|x| x as i32).unwrap_or(-1),
            axis_to_scale_animation: props.text("AxisToScaleAnimation").map(strip).unwrap_or_else(|| "BSA_None".into()),
            rotation_blend_in_mesh_space: props.bool("bRotationBlendInMeshSpace").unwrap_or(false),
            notify_trigger_mode: props.text("NotifyTriggerMode").map(strip),
            per_bone_blend,
            interpolate_using_grid: props.bool("bInterpolateUsingGrid").unwrap_or(false),
        })
    }

    /// Distinct animation names referenced by the samples, in sample order.
    pub fn animation_names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = Vec::new();
        for s in &self.samples {
            if let Some(a) = s.animation.as_deref() {
                if !v.contains(&a) {
                    v.push(a);
                }
            }
        }
        v
    }

    fn grid_coord(&self, axis: usize, value: f32) -> f32 {
        let a = &self.axes[axis];
        let range = a.max - a.min;
        if range <= 0.0 {
            return 0.0;
        }
        let mut v = value;
        if a.wrap_input {
            v = a.min + (v - a.min).rem_euclid(range);
        }
        let n = a.grid_divisions as f32;
        ((v - a.min) / range * n).clamp(0.0, n)
    }

    fn element(&self, x: usize, y: usize) -> Option<&GridElement> {
        let ny = if self.kind.is_1d() { 1 } else { self.axes[1].grid_divisions as usize + 1 };
        self.grid.get(x * ny + y)
    }

    /// `GetSamplesFromBlendInput`: `(sample index, normalised weight)` for the blend input `(x, y)` (`y` ignored for
    /// 1D). Zero weights are dropped; the result is sorted by sample index.
    pub fn sample_weights(&self, x: f32, y: f32) -> Vec<(usize, f32)> {
        let gx = self.grid_coord(0, x);
        let (x0, fx) = (gx.floor() as usize, gx - gx.floor());
        let (y0, fy) = if self.kind.is_1d() { (0usize, 0.0f32) } else {
            let gy = self.grid_coord(1, y);
            (gy.floor() as usize, gy - gy.floor())
        };
        let nx = self.axes[0].grid_divisions as usize;
        let ny = if self.kind.is_1d() { 0 } else { self.axes[1].grid_divisions as usize };
        let mut acc = vec![0.0f32; self.samples.len()];
        let mut add = |xi: usize, yi: usize, w: f32| {
            if w <= 0.0 {
                return;
            }
            if let Some(e) = self.element(xi.min(nx), yi.min(ny)) {
                for k in 0..3 {
                    if e.indices[k] >= 0 && (e.indices[k] as usize) < acc.len() {
                        acc[e.indices[k] as usize] += w * e.weights[k];
                    }
                }
            }
        };
        add(x0, y0, (1.0 - fx) * (1.0 - fy));
        add(x0 + 1, y0, fx * (1.0 - fy));
        if !self.kind.is_1d() {
            add(x0, y0 + 1, (1.0 - fx) * fy);
            add(x0 + 1, y0 + 1, fx * fy);
        }
        let total: f32 = acc.iter().sum();
        if total <= 0.0 {
            return Vec::new();
        }
        acc.iter().enumerate().filter(|(_, &w)| w > 1e-6).map(|(i, &w)| (i, w / total)).collect()
    }

    /// Like [`sample_weights`](Self::sample_weights) but summed per animation name (several samples may reference
    /// the same sequence).
    pub fn animation_weights(&self, x: f32, y: f32) -> Vec<(String, f32)> {
        let mut out: Vec<(String, f32)> = Vec::new();
        for (i, w) in self.sample_weights(x, y) {
            if let Some(a) = self.samples[i].animation.as_deref() {
                match out.iter_mut().find(|(n, _)| n == a) {
                    Some(e) => e.1 += w,
                    None => out.push((a.to_string(), w)),
                }
            }
        }
        out
    }

    /// Validate internal consistency (grid size, indices in range).
    pub fn validate(&self) -> Result<()> {
        let expect = if self.kind.is_1d() { self.axes[0].grid_divisions as usize + 1 } else { (self.axes[0].grid_divisions as usize + 1) * (self.axes[1].grid_divisions as usize + 1) };
        if self.grid.len() != expect {
            return err(format!("{}: grid has {} elements, expected {expect}", self.name, self.grid.len()));
        }
        for e in &self.grid {
            for k in 0..3 {
                if e.indices[k] >= self.samples.len() as i32 {
                    return err(format!("{}: grid index {} out of range", self.name, e.indices[k]));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_by_two() -> BlendSpace {
        // 3 samples; 2x2 grid points (GridNum 1 on both axes); each grid element picks one sample fully
        let axis = BlendAxis { name: "a".into(), min: 0.0, max: 10.0, grid_divisions: 1, ..Default::default() };
        let el = |i: i32| GridElement { indices: [i, -1, -1], weights: [1.0, 0.0, 0.0] };
        BlendSpace {
            name: "t".into(),
            kind: BlendSpaceKind::BlendSpace,
            skeleton: None,
            axes: vec![axis.clone(), axis],
            samples: (0..4).map(|i| BlendSample { animation: Some(format!("A{i}")), animation_path: None, position: [0.0; 3], rate_scale: 1.0, is_valid: true }).collect(),
            grid: vec![el(0), el(1), el(2), el(3)],
            target_weight_interpolation_speed_per_sec: 0.0,
            sample_index_with_markers: -1,
            axis_to_scale_animation: "BSA_None".into(),
            rotation_blend_in_mesh_space: false,
            notify_trigger_mode: None,
            per_bone_blend: vec![],
            interpolate_using_grid: false,
        }
    }

    #[test]
    fn bilinear_weights() {
        let b = two_by_two();
        b.validate().unwrap();
        // corner (x=0,y=0) -> element 0; axis 1 varies fastest so (0,10) -> element 1, (10,0) -> element 2
        assert_eq!(b.sample_weights(0.0, 0.0), vec![(0, 1.0)]);
        assert_eq!(b.sample_weights(0.0, 10.0), vec![(1, 1.0)]);
        assert_eq!(b.sample_weights(10.0, 0.0), vec![(2, 1.0)]);
        assert_eq!(b.sample_weights(10.0, 10.0), vec![(3, 1.0)]);
        let w = b.sample_weights(5.0, 5.0);
        assert_eq!(w.len(), 4);
        for (_, x) in &w {
            assert!((x - 0.25).abs() < 1e-6);
        }
        // clamped outside
        assert_eq!(b.sample_weights(-5.0, 99.0), vec![(1, 1.0)]);
    }

    #[test]
    fn one_dimensional_weights() {
        let axis = BlendAxis { min: -25.0, max: 25.0, grid_divisions: 2, ..Default::default() };
        let el = |i: i32| GridElement { indices: [i, -1, -1], weights: [1.0, 0.0, 0.0] };
        let b = BlendSpace {
            name: "t".into(),
            kind: BlendSpaceKind::BlendSpace1D,
            skeleton: None,
            axes: vec![axis],
            samples: (0..3).map(|i| BlendSample { animation: Some(format!("L{i}")), animation_path: None, position: [0.0; 3], rate_scale: 1.0, is_valid: true }).collect(),
            grid: vec![el(0), el(1), el(2)],
            target_weight_interpolation_speed_per_sec: 0.0,
            sample_index_with_markers: 0,
            axis_to_scale_animation: "BSA_None".into(),
            rotation_blend_in_mesh_space: false,
            notify_trigger_mode: None,
            per_bone_blend: vec![],
            interpolate_using_grid: false,
        };
        b.validate().unwrap();
        assert_eq!(b.sample_weights(-25.0, 0.0), vec![(0, 1.0)]);
        let w = b.sample_weights(12.5, 0.0);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].0, 1);
        assert!((w[0].1 - 0.5).abs() < 1e-6);
        assert_eq!(b.animation_weights(25.0, 0.0), vec![("L2".to_string(), 1.0)]);
    }

    #[test]
    fn triangle_elements_accumulate() {
        let mut b = two_by_two();
        b.grid[0] = GridElement { indices: [0, 1, 2], weights: [0.5, 0.25, 0.25] };
        let w = b.sample_weights(0.0, 0.0);
        assert_eq!(w.len(), 3);
        assert!((w.iter().map(|x| x.1).sum::<f32>() - 1.0).abs() < 1e-6);
    }
}
