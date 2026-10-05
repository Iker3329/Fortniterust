//! Weapon mesh socket -> emitter transform.
//!
//! The port keeps meshes / poses in "game space" (metres, Y up: UE `(x, y, z) -> (x, z, y) * 0.01`, the swap of the glTF export); `fn_particles::Transform` is the UE-space
//! component transform (cm). A rotation `R` of UE space is `P R P` in game space (`P` = the y/z swap), so the conversion back is the same sandwich.
use crate::table::SocketInfo;
use fn_particles::sim::{game_to_ue, Transform};
use glam::{Mat3, Mat4, Vec3};

fn swap() -> Mat3 { Mat3::from_cols(Vec3::X, Vec3::Z, Vec3::Y) }

/// `FRotationMatrix(FRotator(pitch, yaw, roll))` (degrees) in UE axes: yaw about Z, pitch about the right axis (positive = nose up), roll about X (positive = Y toward -Z).
pub fn rotator_matrix(pitch_yaw_roll_deg: [f32; 3]) -> Mat3 {
    let [p, y, r] = pitch_yaw_roll_deg.map(f32::to_radians);
    Mat3::from_rotation_z(y) * Mat3::from_rotation_y(-p) * Mat3::from_rotation_x(-r)
}

/// The socket's `FTransform` relative to its bone, in game space.
pub fn socket_local_game(s: &SocketInfo) -> Mat4 {
    let t = Vec3::new(s.location[0] * 0.01, s.location[2] * 0.01, s.location[1] * 0.01);
    let r = swap() * rotator_matrix(s.rotation) * swap();
    let sc = Vec3::new(s.scale[0], s.scale[2], s.scale[1]);
    Mat4::from_translation(t) * Mat4::from_mat3(r) * Mat4::from_scale(sc)
}

/// A game-space matrix (`weapon to world * bone * socket`) -> UE-space component transform (position cm, rotation, uniform scale = mean axis length).
pub fn transform_from_game(m: Mat4) -> Transform {
    let (x, y, z) = (m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate());
    let scale = ((x.length() + y.length() + z.length()) / 3.0).max(1e-6);
    let r = Mat3::from_cols(x.normalize_or_zero(), y.normalize_or_zero(), z.normalize_or_zero());
    Transform { pos: game_to_ue(m.w_axis.truncate().to_array()), rot: swap() * r * swap(), scale }
}

/// The transform `GetSocketTransform(MuzzleSocketName, RTS_World)` gives: `bone_world` = the weapon bone's game-space matrix in the world (weapon attach matrix * `bone_mesh[bone]`).
pub fn muzzle_transform(bone_world: Mat4, s: &SocketInfo) -> Transform { transform_from_game(bone_world * socket_local_game(s)) }

#[cfg(test)]
mod tests {
    use super::*;
    fn sock(loc: [f32; 3], rot: [f32; 3]) -> SocketInfo { SocketInfo { name: "Muzzle".into(), bone: "b".into(), location: loc, rotation: rot, scale: [1.0; 3], owner: "mesh".into() } }
    #[test] fn socket_position_follows_the_bone_in_game_space_and_x_stays_the_barrel() {
        // bone 2 m up and 1 m along game X; socket 50 cm along UE X (= game X)
        let t = muzzle_transform(Mat4::from_translation(Vec3::new(1.0, 2.0, 0.0)), &sock([50.0, 0.0, 0.0], [0.0; 3]));
        let g = fn_particles::sim::ue_to_game(t.pos);
        assert!((g[0] - 1.5).abs() < 1e-5 && (g[1] - 2.0).abs() < 1e-5 && g[2].abs() < 1e-5, "{g:?}");
        assert!((t.rot * Vec3::X - Vec3::X).length() < 1e-5 && (t.scale - 1.0).abs() < 1e-6);
    }
    #[test] fn yaw_turns_the_emitter_x_axis_and_game_yaw_matches() {
        // socket yawed 90 degrees about UE Z: emitter X -> UE +Y; the bone is turned 90 degrees about game Y (up) which is UE Z too (right-handed game vs left-handed UE flips the sense)
        let t = muzzle_transform(Mat4::IDENTITY, &sock([0.0; 3], [0.0, 90.0, 0.0]));
        let x = t.rot * Vec3::X; assert!((x - Vec3::Y).length() < 1e-5, "{x:?}");
        let pitched = muzzle_transform(Mat4::IDENTITY, &sock([0.0; 3], [30.0, 0.0, 0.0])); let x = pitched.rot * Vec3::X; assert!(x.z > 0.49 && x.z < 0.51, "positive pitch = nose up: {x:?}");
    }
}
