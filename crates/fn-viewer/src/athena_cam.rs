//! The settled Athena melee third-person camera (`Athena_PlayerCameraModeMelee`), reduced from the prototype's camera.rs to a pure
//! function so the viewer can reproduce the old runtime's framing for a given pawn position and control yaw/pitch.
//!
//! Data (serialized Blueprint class defaults, cm): FOV 80, CameraSpaceForwardDistance -280, ViewTargetSpaceViewOffset (38, 31, 70)
//! standing / (38, 31, 30) crouched, pitch-sampled CameraSpaceViewOffset {0:(0,0,0), 90:(30,0,100), -90:(50,0,-20)}, pawn capsule
//! half height 75. NOT reproduced: the pivot lag interpolation (irrelevant once settled), the 5 penetration feelers (only the main
//! 8 cm probe against the ground plane is kept) and prop collision.
use glam::Vec3;

pub const CM: f32 = 0.01;
pub const STAND_HALF_HEIGHT: f32 = 75. * CM;
pub const FORWARD_DISTANCE_CM: f32 = -280.;
pub const VT_OFFSET_CM: [f32; 3] = [38., 31., 70.];
pub const VT_OFFSET_CROUCH_CM: [f32; 3] = [38., 31., 30.];
pub const CS_OFFSET: [(f32, [f32; 3]); 3] = [(-90., [50., 0., -20.]), (0., [0., 0., 0.]), (90., [30., 0., 100.])];
pub const PROBE_RADIUS_M: f32 = 8. * CM;
pub const PITCH_LIMIT: f32 = 89.9;

/// View-yaw basis (yaw 0 faces -Z; right = +X at yaw 0).
pub fn yaw_basis(yaw_deg: f32) -> (Vec3, Vec3) {
    let a = yaw_deg.to_radians();
    (Vec3::new(a.sin(), 0., -a.cos()), Vec3::new(a.cos(), 0., a.sin()))
}
/// Full camera basis (forward, right, up) for control yaw/pitch, roll 0.
pub fn view_basis(yaw_deg: f32, pitch_deg: f32) -> (Vec3, Vec3, Vec3) {
    let (fh, right) = yaw_basis(yaw_deg);
    let p = pitch_deg.to_radians();
    let forward = fh * p.cos() + Vec3::Y * p.sin();
    (forward, right, right.cross(forward))
}
/// Piecewise-linear lookup of the pitch-sampled camera-space offset (cm), clamped outside the range.
pub fn sample_cs_offset(pitch: f32) -> Vec3 {
    let at = |s: &(f32, [f32; 3])| Vec3::from(s.1);
    if pitch <= CS_OFFSET[0].0 { return at(&CS_OFFSET[0]); }
    for w in CS_OFFSET.windows(2) {
        if pitch <= w[1].0 {
            let t = (pitch - w[0].0) / (w[1].0 - w[0].0);
            return at(&w[0]).lerp(at(&w[1]), t);
        }
    }
    at(&CS_OFFSET[2])
}
/// Sphere sweep against the baseplate top surface (y = 0): hit fraction along from->to.
pub fn trace_ground(from: Vec3, to: Vec3, radius: f32) -> Option<f32> {
    if from.y < radius { return Some(0.); }
    if to.y >= radius { return None; }
    Some((from.y - radius) / (from.y - to.y))
}
/// Settled camera eye position for a pawn standing at `feet` (metres), control yaw/pitch in degrees.
pub fn settled_eye(feet: Vec3, crouched: bool, yaw_deg: f32, pitch_deg: f32) -> Vec3 {
    let pitch = pitch_deg.clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let center = feet + Vec3::Y * STAND_HALF_HEIGHT;
    let (fh, right_h) = yaw_basis(yaw_deg);
    let vt = Vec3::from(if crouched { VT_OFFSET_CROUCH_CM } else { VT_OFFSET_CM });
    let pivot = center + (fh * vt.x + right_h * vt.y + Vec3::Y * vt.z) * CM;
    let (forward, right, up) = view_basis(yaw_deg, pitch);
    let cs = sample_cs_offset(pitch);
    let desired = pivot + (forward * (cs.x + FORWARD_DISTANCE_CM) + right * cs.y + up * cs.z) * CM;
    // trace origin: the point of the capsule axis at pivot height (stays inside the capsule)
    let safe = Vec3::new(center.x, pivot.y.clamp(feet.y + 0.05, feet.y + 2. * STAND_HALF_HEIGHT - 0.05), center.z);
    let ray = desired - safe;
    let blocked = trace_ground(safe, desired, PROBE_RADIUS_M).unwrap_or(1.);
    safe + ray * blocked
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_framing_matches_hand_computation() {
        // feet (0,0,12), yaw 0, pitch -12 (the prototype's spawn view): cs offset = lerp(0, (50,0,-20), 12/90)
        let eye = settled_eye(Vec3::new(0., 0., 12.), false, 0., -12.);
        let cs = Vec3::new(50., 0., -20.) * (12. / 90.);
        let p = 12f32.to_radians();
        let forward = Vec3::new(0., -p.sin(), -p.cos());
        let up = Vec3::new(1., 0., 0.).cross(forward);
        let pivot = Vec3::new(0.31, 0.75 + 0.70, 12. - 0.38);
        let want = pivot + (forward * (cs.x - 280.) + up * cs.z) * CM;
        assert!((eye - want).length() < 1e-4, "{eye:?} vs {want:?}");
        assert!(eye.y > 1.9 && eye.y < 2.1 && eye.z > 14.0 && eye.z < 14.6, "{eye:?}");
    }
    #[test]
    fn camera_stays_above_the_ground_when_looking_up() {
        let eye = settled_eye(Vec3::ZERO, false, 0., -80.);
        assert!(eye.y >= PROBE_RADIUS_M - 1e-4);
        let eye = settled_eye(Vec3::ZERO, true, 30., 60.);
        assert!(eye.y >= PROBE_RADIUS_M - 1e-4);
    }
    #[test]
    fn crouch_lowers_the_boom_by_the_serialized_offset() {
        let a = settled_eye(Vec3::ZERO, false, 0., 0.);
        let b = settled_eye(Vec3::ZERO, true, 0., 0.);
        assert!((a.y - b.y - 0.4).abs() < 1e-5);
    }
    #[test]
    fn cs_offset_interpolates_by_pitch() {
        assert_eq!(sample_cs_offset(0.), Vec3::ZERO);
        assert_eq!(sample_cs_offset(90.), Vec3::new(30., 0., 100.));
        assert_eq!(sample_cs_offset(-120.), Vec3::new(50., 0., -20.));
        assert!((sample_cs_offset(45.) - Vec3::new(15., 0., 50.)).length() < 1e-5);
    }
}
