//! `WidgetAnimation` evaluation (UMG `MovieScene` tracks serialized in the widget package): vector tracks (`LayoutData.Anchors.Minimum/Maximum`) and colour
//! tracks (`ContentColorAndOpacity`) of 4.16 `FRichCurve` sections. Only the track kinds the Locker's `IntroPicker` animation uses are evaluated.
//!
//! `FRichCurve` (4.16): between two keys `RCIM_Linear` (0) lerps, `RCIM_Constant` (1) holds the earlier value, `RCIM_Cubic` (2) is a cubic Hermite spline
//! built from the earlier key's `LeaveTangent` and the later key's `ArriveTangent` (both scaled by the key interval); keys with auto tangents are stored
//! with zero tangents in the cooked data, which makes a two-key cubic curve the smooth step `3s^2 - 2s^3`.
use crate::screens::athena_lobby::store::{jf, Pkg};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Key { pub t: f32, pub v: f32, pub interp: u8, pub arrive: f32, pub leave: f32 }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve { pub keys: Vec<Key> }

impl Curve {
    pub fn from_json(v: Option<&Value>) -> Curve {
        let keys = v.and_then(|c| c.get("Keys")).and_then(|k| k.as_array()).map(|a| a.iter().map(|k| Key {
            t: k.get("Time").and_then(jf).unwrap_or(0.0),
            v: k.get("Value").and_then(jf).unwrap_or(0.0),
            interp: k.get("InterpMode").and_then(|x| x.as_u64()).unwrap_or(0) as u8,
            arrive: k.get("ArriveTangent").and_then(jf).unwrap_or(0.0),
            leave: k.get("LeaveTangent").and_then(jf).unwrap_or(0.0),
        }).collect()).unwrap_or_default();
        Curve { keys }
    }
    pub fn eval(&self, t: f32) -> f32 {
        let k = &self.keys;
        let (Some(first), Some(last)) = (k.first(), k.last()) else { return 0.0 };
        if t <= first.t { return first.v; }
        if t >= last.t { return last.v; }
        let i = k.iter().rposition(|x| x.t <= t).unwrap_or(0);
        let (a, b) = (&k[i], &k[i + 1]);
        let dt = (b.t - a.t).max(1e-9);
        let s = (t - a.t) / dt;
        match a.interp {
            1 => a.v,
            2 => {
                let (m0, m1) = (a.leave * dt, b.arrive * dt);
                let (s2, s3) = (s * s, s * s * s);
                (2.0 * s3 - 3.0 * s2 + 1.0) * a.v + (s3 - 2.0 * s2 + s) * m0 + (-2.0 * s3 + 3.0 * s2) * b.v + (s3 - s2) * m1
            }
            _ => a.v + (b.v - a.v) * s,
        }
    }
}

/// What a track animates.
#[derive(Clone, Debug, PartialEq)]
pub enum TrackKind {
    /// `LayoutData.Anchors.Minimum` / `Maximum` of the canvas slot of `widget`
    AnchorMin, AnchorMax,
    /// `ContentColorAndOpacity` (border) / `ColorAndOpacity` of `widget`
    ContentColor, Color,
}
#[derive(Clone, Debug)]
pub struct Track { pub widget: String, pub kind: TrackKind, pub curves: Vec<Curve> }

/// The tracks of a `MovieScene` export named `anim` (e.g. `IntroPicker`) of `pkg`, and the playback end time.
pub fn load(pkg: &Pkg, anim: &str) -> Option<(Vec<Track>, f32)> {
    let scene = pkg.exports.iter().find(|e| e.class == "MovieScene" && e.name == anim && e.props.get("ObjectBindings").is_some())?;
    let end = scene.props.get("PlaybackRange").and_then(|r| r.get("UpperBound")).and_then(|u| u.get("Value")).and_then(jf).unwrap_or(0.0);
    let possessables: Vec<(String, String, String)> = scene.props.get("Possessables").and_then(|p| p.as_array()).map(|a| a.iter().map(|p| (
        p.get("Guid").and_then(|v| v.as_str()).unwrap_or("").to_owned(), p.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_owned(), p.get("ParentGuid").and_then(|v| v.as_str()).unwrap_or("").to_owned())).collect()).unwrap_or_default();
    let mut out = vec![];
    for b in scene.props.get("ObjectBindings").and_then(|b| b.as_array()).cloned().unwrap_or_default() {
        let guid = b.get("ObjectGuid").and_then(|v| v.as_str()).unwrap_or("");
        let bname = b.get("BindingName").and_then(|v| v.as_str()).unwrap_or("");
        // a slot binding (`CanvasPanelSlot_0`) belongs to its parent possessable's widget
        let widget = match possessables.iter().find(|p| p.0 == guid) {
            Some(p) if p.1.contains("Slot") => possessables.iter().find(|q| q.0 == p.2).map(|q| q.1.clone()).unwrap_or_else(|| bname.to_owned()),
            _ => bname.to_owned(),
        };
        for t in b.get("Tracks").and_then(|t| t.as_array()).cloned().unwrap_or_default() {
            let Some(track) = t.as_str().and_then(|p| pkg.get(p)) else { continue };
            let path = track.props.get("PropertyPath").and_then(|v| v.as_str()).unwrap_or("");
            let kind = match (track.class.as_str(), path) {
                ("MovieSceneVectorTrack", p) if p.ends_with("Anchors.Minimum") => TrackKind::AnchorMin,
                ("MovieSceneVectorTrack", p) if p.ends_with("Anchors.Maximum") => TrackKind::AnchorMax,
                ("MovieSceneColorTrack", "ContentColorAndOpacity") => TrackKind::ContentColor,
                ("MovieSceneColorTrack", "ColorAndOpacity") => TrackKind::Color,
                _ => continue,
            };
            let Some(section) = track.props.get("Sections").and_then(|s| s.as_array()).and_then(|s| s.first()).and_then(|s| s.as_str()).and_then(|p| pkg.get(p)) else { continue };
            let sp = &section.props;
            let curves = match kind {
                TrackKind::AnchorMin | TrackKind::AnchorMax => vec![Curve::from_json(sp.get("Curves")), Curve::from_json(sp.get("Curves[1]"))],
                _ => vec![Curve::from_json(sp.get("RedCurve")), Curve::from_json(sp.get("GreenCurve")), Curve::from_json(sp.get("BlueCurve")), Curve::from_json(sp.get("AlphaCurve"))],
            };
            out.push(Track { widget: widget.clone(), kind, curves });
        }
    }
    Some((out, end))
}

impl Track {
    pub fn value(&self, t: f32) -> Vec<f32> { self.curves.iter().map(|c| c.eval(t)).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_cubic_keys_with_zero_tangents_are_a_smooth_step() {
        let c = Curve { keys: vec![Key { t: 0.0, v: 0.0, interp: 2, arrive: 0.0, leave: 0.0 }, Key { t: 1.0, v: 1.0, interp: 2, arrive: 0.0, leave: 0.0 }] };
        assert!((c.eval(0.5) - 0.5).abs() < 1e-6);
        assert!((c.eval(0.25) - (3.0 * 0.0625 - 2.0 * 0.015625)).abs() < 1e-6);
        assert_eq!((c.eval(-1.0), c.eval(2.0)), (0.0, 1.0));
    }
    #[test]
    fn linear_and_constant_keys() {
        let l = Curve { keys: vec![Key { t: 0.0, v: -1.0, interp: 0, arrive: 0.0, leave: 0.0 }, Key { t: 0.25, v: 0.0, interp: 0, arrive: 0.0, leave: 0.0 }] };
        assert!((l.eval(0.125) + 0.5).abs() < 1e-6);
        let c = Curve { keys: vec![Key { t: 0.0, v: 3.0, interp: 1, arrive: 0.0, leave: 0.0 }, Key { t: 1.0, v: 9.0, interp: 1, arrive: 0.0, leave: 0.0 }] };
        assert_eq!(c.eval(0.9), 3.0);
    }
}
