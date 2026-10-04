//! The held firearms drawn with their ORIGINAL material: the base-pass programs of the cooked `WeaponMat_Base` / `BackPackShrinkingEffect` shader maps, translated from
//! DXBC by `fn-dxbc` and driven by `fn-matshader` (permutation, material-instance parameters, textures, View / Primitive / bone buffers), drawn by fn-render's gun pass at
//! the player's `RightHand` socket. The weapon's own skeleton rig + clips (fire / reload montages) drive the bone matrices exactly as for the stand-in model.
use fn_matshader::gun::{FrameInput, GunModel, GunScene};
use fn_matshader::view::ViewInput;
use fn_pose::loadout::{Resources, ITEMS};
use fn_pose::{CharacterPose, CharacterRig, ClipStore};
use fn_animgraph::Node;
use fn_render::{CameraParams, Renderer};
use fn_render::gun_material::GunFrame;
use glam::Mat4;
use std::path::Path;

struct OrigGun { model: GunModel, rig: CharacterRig, clips: ClipStore, pose: CharacterPose }
struct Pending { gun: usize, to_world: Mat4, skin: Vec<Mat4> }

pub struct OrigGuns { scene: GunScene, guns: Vec<OrigGun>, pending: Option<Pending> }

/// Builds the three firearms (loadout items 0..3). Any error names the piece that could not be recovered; the caller then keeps the gun hidden.
pub fn install(renderer: &mut Renderer, res: &Resources, install_dir: &Path) -> Result<OrigGuns, String> {
    let set = fn_pak::PakSet::open_install(install_dir).map_err(|e| e.to_string())?;
    let mut scene = GunScene::new(renderer.gun());
    let mut guns = vec![];
    for i in 0..3 {
        let item = &res.items[i];
        let part = item.mesh.clone().ok_or("gun has no mesh")?;
        let mesh = fn_matshader::mesh::load_weapon_mesh(&set, ITEMS[i])?;
        let model = scene.build_gun(&set, renderer.gun(), &mesh, &part, &item.mesh_materials)?;
        guns.push(OrigGun { model, rig: CharacterRig::new(vec![part]), clips: res.weapon_clips(i)?, pose: CharacterPose::default() });
    }
    for l in &scene.log { eprintln!("{l}"); }
    Ok(OrigGuns { scene, guns, pending: None })
}

impl OrigGuns {
    /// Evaluates the held gun's skeleton for this tick; `attach` = `world * RightHand socket` (port space).
    pub fn update(&mut self, held: Option<(usize, Option<(String, f32)>)>, attach: Mat4) {
        self.pending = None;
        let Some((g, clip)) = held else { return };
        let Some(h) = self.guns.get_mut(g) else { return };
        let tree = match &clip { Some((c, t)) => Node::clip(c, *t, false), None => Node::Ref };
        h.rig.evaluate_into(&tree, &h.clips, &mut h.pose);
        if let Some(p) = h.pose.parts.first() { self.pending = Some(Pending { gun: g, to_world: attach, skin: p.skin_matrices.clone() }); }
    }
    /// Hands the frame's gun draws to the renderer (call before `Renderer::render`).
    pub fn submit(&mut self, renderer: &mut Renderer, camera: &CameraParams, time: f32) {
        let mut frame = GunFrame::default();
        if let Some(p) = &self.pending {
            let (tx, ty) = camera.tan_half();
            let view = ViewInput { eye: camera.eye, forward: camera.forward, up: camera.up, tan_half: (tx, ty), near_m: camera.near, far_m: camera.far, width: camera.width, height: camera.height, time };
            let input = FrameInput { view, gun_to_world: p.to_world, skin: &p.skin, sky_radiance: fn_render::lighting::sky_radiance(), time };
            self.guns[p.gun].model.frame(&input, &mut frame);
        }
        renderer.set_gun_frame(frame);
    }
    pub fn is_drawing(&self) -> bool { self.pending.is_some() }
}
