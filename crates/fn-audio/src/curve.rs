//! `FRichCurve` evaluation (linear / constant / cubic-Hermite keys), as used by `SoundNodeEnveloper` and custom
//! attenuation curves.

use fn_uasset::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub time: f32,
    pub value: f32,
    /// `ERichCurveInterpMode`: 0 linear, 1 constant, 2 cubic, 3 none.
    pub interp: u8,
    pub arrive_tangent: f32,
    pub leave_tangent: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve(pub Vec<Key>);

impl Curve {
    /// Build from a list of `RichCurveKey` struct values (the `Keys` array of `EditorCurveData` / `FloatCurve`).
    pub fn from_keys(keys: &[Value]) -> Curve {
        let mut v: Vec<Key> = keys
            .iter()
            .filter_map(|k| {
                Some(Key {
                    time: k.field("Time")?.as_f32()?,
                    value: k.field("Value")?.as_f32()?,
                    interp: k.field("InterpMode").and_then(Value::as_i64).unwrap_or(0) as u8,
                    arrive_tangent: k.field("ArriveTangent").and_then(Value::as_f32).unwrap_or(0.0),
                    leave_tangent: k.field("LeaveTangent").and_then(Value::as_f32).unwrap_or(0.0),
                })
            })
            .collect();
        v.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
        Curve(v)
    }

    /// Build from a `RuntimeFloatCurve` value (`EditorCurveData.Keys`).
    pub fn from_runtime_curve(v: Option<&Value>) -> Curve {
        match v.and_then(|c| c.field("EditorCurveData")).and_then(|c| c.field("Keys")).and_then(Value::as_array) {
            Some(k) => Curve::from_keys(k),
            None => Curve::default(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Value at `t`; holds the first/last key outside the range; `default` for an empty curve.
    pub fn eval(&self, t: f32, default: f32) -> f32 {
        let k = &self.0;
        let (Some(first), Some(last)) = (k.first(), k.last()) else { return default };
        if t <= first.time {
            return first.value;
        }
        if t >= last.time {
            return last.value;
        }
        let i = k.partition_point(|key| key.time <= t).saturating_sub(1);
        let (a, b) = (k[i], k[(i + 1).min(k.len() - 1)]);
        let dt = b.time - a.time;
        if dt <= 1e-9 {
            return b.value;
        }
        let alpha = (t - a.time) / dt;
        match a.interp {
            1 => a.value,
            2 => {
                // cubic Hermite with leave/arrive tangents (per unit time) scaled by the segment length
                let (p0, p1) = (a.value, b.value);
                let (m0, m1) = (a.leave_tangent * dt, b.arrive_tangent * dt);
                let (a2, a3) = (alpha * alpha, alpha * alpha * alpha);
                (2.0 * a3 - 3.0 * a2 + 1.0) * p0 + (a3 - 2.0 * a2 + alpha) * m0 + (-2.0 * a3 + 3.0 * a2) * p1 + (a3 - a2) * m1
            }
            3 => a.value,
            _ => a.value + (b.value - a.value) * alpha,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(t: f32, v: f32, i: u8, at: f32, lt: f32) -> Key {
        Key { time: t, value: v, interp: i, arrive_tangent: at, leave_tangent: lt }
    }

    #[test]
    fn linear_constant_cubic() {
        let c = Curve(vec![k(0.0, 0.0, 0, 0.0, 0.0), k(1.0, 1.0, 1, 0.0, 0.0), k(2.0, 3.0, 2, 0.0, 0.0), k(3.0, 3.0, 0, 0.0, 0.0)]);
        assert!((c.eval(0.5, 9.0) - 0.5).abs() < 1e-6);
        assert_eq!(c.eval(1.5, 9.0), 1.0); // constant segment holds key 1's value
        // cubic with zero tangents is smoothstep between 1 -> 3... segment 2..3 has equal values: flat
        assert!((c.eval(2.5, 9.0) - 3.0).abs() < 1e-6);
        assert_eq!(c.eval(-1.0, 9.0), 0.0);
        assert_eq!(c.eval(10.0, 9.0), 3.0);
        assert_eq!(Curve::default().eval(1.0, 9.0), 9.0);
    }

    #[test]
    fn cubic_hits_midpoint_of_smoothstep() {
        let c = Curve(vec![k(0.0, 0.0, 2, 0.0, 0.0), k(1.0, 1.0, 0, 0.0, 0.0)]);
        assert!((c.eval(0.5, 0.0) - 0.5).abs() < 1e-6);
        assert!(c.eval(0.25, 0.0) < 0.25); // ease-in
    }
}
