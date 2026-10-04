//! The 3D world as the renderer sees it: character body + head (GPU skinned, `MaterialKind::Character`), the pickaxe on the `RightHand` socket
//! (skinned character shading with constant spec / mask, `MaterialKind::Prop`, the same as `fn-viewer`, which is calibrated against the prototype), and
//! the harvestables (trees / rocks) through the renderer's own static-prop path `Renderer::set_prop_instances` (`fs_prop`: vertex-colour albedo, sun with
//! cascaded shadows, sky ambient, fog; a port of the prototype's untextured path). The props used to be pushed through the textured character material,
//! which sampled a 1x1 colour texture with a flat normal and rendered trunks / canopies much darker than the prototype.
//! Everything is driven every frame from [`fn_game::GameSim`]: pose tree (`GameSim::pose_tree`) -> `fn_pose::CharacterRig::evaluate_into`
//! (per-part skin matrices + sockets) -> `Renderer::update_model`.
use fn_animgraph::{ClipLibrary, sample_blended_curve};
use fn_game::{GameSim, EYE};
use fn_animgraph::Node;
use fn_pose::{CharacterPose, CharacterRig, ClipStore, Part as PosePart};
use fn_render::{CameraParams, FrameParams, FrameStats, MaterialImages, MaterialKind, ModelId, PropInstance, RenderOptions, Renderer, SkinnedPrimitive, SkinnedVertex, StaticMeshInstance};
use glam::{Mat4, Quat, Vec3};
use wgpu::TextureView;

// ---------------------------------------------------------------------------------------------------------------------------------
// meshes -> renderer primitives
// ---------------------------------------------------------------------------------------------------------------------------------
/// Texture names of one exported `.mat` file (`Diffuse=`, `Normal=`, `SpecPower=` = the SpecularMasks texture, `Other[n]=` ending `_M` = the mask).
#[derive(Default, Debug, PartialEq, Eq)]
pub struct MatSlots { pub diffuse: Option<String>, pub normal: Option<String>, pub spec: Option<String>, pub mask: Option<String> }
pub fn parse_mat(text: &str) -> MatSlots {
    let mut s = MatSlots::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().to_owned();
        match key.trim() {
            "Diffuse" => s.diffuse = Some(value),
            "Normal" => s.normal = Some(value),
            "SpecPower" => s.spec = Some(value),
            k if k.starts_with("Other[") && (value.ends_with("_M") || value.ends_with("_m")) => s.mask = Some(value),
            _ => {}
        }
    }
    s
}

/// Renderer primitives of a pose part: its skin primitives plus the textures its materials name (`<material>.mat` -> PNGs, all found by name).
pub fn part_primitives(part: &PosePart, roots: &crate::assets::AssetRoots, kind: MaterialKind) -> Vec<SkinnedPrimitive> {
    part.prims.iter().map(|p| {
        let slots = roots.find(&format!("{}.mat", p.material)).and_then(|f| std::fs::read_to_string(f).ok()).map(|t| parse_mat(&t)).unwrap_or_default();
        let load = |n: &Option<String>| n.as_ref().and_then(|n| roots.load_rgba(&format!("{n}.png")).ok());
        let material = MaterialImages { diffuse: load(&slots.diffuse), normal: load(&slots.normal), spec: load(&slots.spec), mask: load(&slots.mask), ue: None };
        let vertices = (0..p.positions.len()).map(|i| SkinnedVertex { pos: p.positions[i], normal: p.normals[i], uv: p.uvs[i], joints: p.joints[i], weights: p.weights[i] }).collect();
        SkinnedPrimitive { vertices, indices: p.indices.clone(), material, kind }
    }).collect()
}
/// SK_Pick is attached rigidly (its skin is ignored, like the prototype's `draw_attached`): joints -> 0, weight 1.
pub fn make_rigid(prims: &mut [SkinnedPrimitive]) { for p in prims { for v in &mut p.vertices { v.joints = [0; 4]; v.weights = [1., 0., 0., 0.]; } } }

// ---------------------------------------------------------------------------------------------------------------------------------
// the character rig + the scene
// ---------------------------------------------------------------------------------------------------------------------------------
/// Skeleton parts + animation clips + the reusable pose result.
pub struct Character {
    pub rig: CharacterRig,
    pub clips: Box<dyn ClipLibrary + Send>,
    /// The displayed pawn pose, including the recovered CopyBone and hand-IK
    /// post process from the Athena AnimBlueprint.  Every attached component
    /// uses this same final pose.
    pub pose: CharacterPose,
    /** PART ANIM HOOK: the BR parts' own AnimBPs (face rig, AnimDynamics); `None` = master pose only. */
    pub part_anim: Option<fn_pose::part_anim::PartAnimRig>,
}
impl Character {
    pub fn new(rig: CharacterRig, clips: Box<dyn ClipLibrary + Send>) -> Character {
        Character { rig, clips, pose: CharacterPose::default(), part_anim: None }
    }
    pub fn evaluate(&mut self, sim: &mut GameSim) {
        // The post-process nodes do not introduce clips, so the provisional
        // tree exposes the exact current sequence/blend weights used by UE's
        // float curves.  Feed those values back before constructing the final
        // CopyBone/IK chain for this frame.
        let curve_tree = sim.pose_tree();
        sim.anim.set_hand_ik_curves(
            sample_blended_curve(&curve_tree, &*self.clips, "SnapWeapon_LHand"),
            sample_blended_curve(&curve_tree, &*self.clips, "BothHandFK"),
            sample_blended_curve(&curve_tree, &*self.clips, "LeftHandFK"),
        );
        let tree = sim.pose_tree();
        // PART ANIM HOOK
        if let Some(pa) = self.part_anim.as_mut() {
            pa.evaluate_into(&self.rig, &tree, &*self.clips, sim.frame_dt, &crate::part_anim_glue::inputs(sim.anim.vars.speed2d), &mut self.pose);
        } else {
            self.rig.evaluate_into(&tree, &*self.clips, &mut self.pose);
        }
    }
    /// The body's local pose (what fn-sim hashes in its frame log).
    pub fn body_local(&self) -> &[(Vec3, Quat)] { self.pose.parts.first().map(|p| p.bone_local.as_slice()).unwrap_or(&[]) }
}

pub struct Scene {
    pub renderer: Renderer,
    pub character: Character,
    body: ModelId,
    head: ModelId,
    pick: ModelId,
    /// The prop list last uploaded with `set_prop_instances` (re-uploaded only when a prop appears, disappears, moves or shakes).
    props: Vec<PropInstance>,
    /// The REAL 1.8 props (textured static meshes, `fn_render::static_loader`): the mesh of every prop kind, set by the loader. `None` = the assets were missing: every
    /// harvestable is drawn as the procedural placeholder (`props` above).
    pub real: Option<crate::props::Real>,
    /// The static-mesh list last uploaded with `set_static_instances` (re-uploaded only when a prop appears, disappears or shakes; the wind is shader time).
    statics: Vec<StaticMeshInstance>,
    /// FOLIAGE HOOK: the ambient ground cover (set by the loader; its near blades are appended to the static-mesh list).
    pub grass: Option<crate::foliage::Grass>,
    pub models: Vec<ModelId>,
    pub last_stats: Option<FrameStats>,
    /// Frames drawn so far (for the `[slow]` log lines).
    draws: u64,
    pub show_pickaxe: bool,
    /// Original starter meshes and montage/sequence closure loaded off the UI thread.
    pub loadout: Option<fn_pose::loadout::Resources>,
    /// The three original firearm meshes (index = loadout item), uploaded with their original Diffuse / Normals textures.
    guns: Vec<HeldGun>,
    /// Set each frame by the app: the held firearm and the weapon-skeleton clip (sequence package, time) its fire / reload montage plays.
    pub held: Option<(usize, Option<(String, f32)>)>,
    /// World position (metres) of the point the held weapon is attached at: the player's `RightHand` socket (the weapon mesh component is snapped to it, and the weapon's
    /// `Normal` / `LowAmmo` / `Degraded` AudioComponents are attached to that mesh), refreshed by every [`Scene::update`]. Where weapon sounds are emitted.
    pub weapon_origin: Option<Vec3>,
    /// Clip names the pose tree asked for and the library lacked (reported once each).
    reported_missing: std::collections::HashSet<String>,
    /// GUN MATERIAL HOOK: the held firearms drawn with their ORIGINAL material (fn-matshader), see gun_orig.rs. `None` = not installed (gun hidden unless the stand-in is requested).
    pub orig_guns: Option<crate::gun_orig::OrigGuns>,
}

/// One held firearm: renderer model, its own skeleton rig and the clips of exactly its skeleton (never the player's).
struct HeldGun { model: ModelId, rig: CharacterRig, clips: ClipStore, pose: CharacterPose }

/// Gun material, labelled: the original WeaponMat_Base shader permutation is NOT executed (594 programs, pass / vertex-factory ownership unrecovered);
/// the closest honest material is the renderer's Prop shading (constant specular) with the material instance's original Diffuse and Normals`n/// textures. SpecularMasks, CustomizationMask and the emissive map are not applied.
pub fn gun_primitives(res: &fn_pose::loadout::Resources, item: usize) -> Result<Vec<SkinnedPrimitive>, String> {
    let it = res.items.get(item).ok_or("item out of range")?;
    let part = it.mesh.as_ref().ok_or("item has no mesh")?;
    let image = |mat: &str, param: &str| -> Option<image::RgbaImage> {
        let tex = res.materials.get(mat)?.texture_parameters.get(param)?.as_ref()?;
        let t = res.material_textures.get(tex)?;
        image::RgbaImage::from_raw(t.image.width as u32, t.image.height as u32, t.image.to_rgba8())
    };
    part.prims.iter().enumerate().map(|(i, p)| {
        let mat = it.mesh_materials.get(i).cloned().flatten().ok_or_else(|| format!("section {i} has no original material"))?;
        let material = MaterialImages { diffuse: image(&mat, "Diffuse"), normal: image(&mat, "Normals"), spec: None, mask: None, ue: None };
        let vertices = (0..p.positions.len()).map(|v| SkinnedVertex { pos: p.positions[v], normal: p.normals[v], uv: p.uvs[v], joints: p.joints[v], weights: p.weights[v] }).collect();
        Ok(SkinnedPrimitive { vertices, indices: p.indices.clone(), material, kind: MaterialKind::Prop })
    }).collect()
}

/// World matrix of the pawn: the old runtime rotates the model by (180 - facing) degrees about Y at the pawn's feet.
pub fn pawn_matrix(sim: &GameSim) -> Mat4 { Mat4::from_rotation_translation(Quat::from_rotation_y((180. - sim.controller.facing).to_radians()), Vec3::new(sim.x, sim.y - EYE, sim.z)) }
pub fn camera_for(sim: &GameSim, w: u32, h: u32) -> CameraParams {
    let v = &sim.cam_view;
    let mut c = CameraParams::from_yaw_pitch(v.eye, v.yaw_deg, v.pitch_deg, w.max(1), h.max(1));
    c.far = fn_camera::far_plane();
    if sim.ads.fov > 0. { c.fov_deg = sim.ads.fov; } // ADS HOOK: the blended hold / targeting mode FOV (AR / shotgun 80, scoped sniper 15)
    c
}
/// FNV-1a over the exact bits of a local pose (the same hash fn-sim logs).
pub fn pose_hash(pose: &[(Vec3, Quat)]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut put = |v: f32| { for b in v.to_bits().to_le_bytes() { h ^= b as u64; h = h.wrapping_mul(0x100000001b3); } };
    for (p, q) in pose { put(p.x); put(p.y); put(p.z); put(q.x); put(q.y); put(q.z); put(q.w); }
    h
}

/// Splits the live harvestables into the real static-mesh instances (a prop whose kind has an uploaded mesh) and the procedural fallback (everything else that
/// is alive). Destroyed props are in neither list: there is no stump mesh in the 1.8 export, a destroyed tree or rock simply disappears.
pub fn split_props(props: &[fn_combat::Harvestable], mesh_of: impl Fn(&fn_combat::Harvestable) -> Option<fn_render::StaticMeshHandle>) -> (Vec<StaticMeshInstance>, Vec<PropInstance>) {
    let (mut statics, mut procedural) = (vec![], vec![]);
    for p in props.iter().filter(|p| p.alive()) {
        match mesh_of(p) {
            Some(m) => statics.push(StaticMeshInstance::new(m, p.pos).with_yaw(p.yaw).with_scale(p.scale).with_shake(p.shake)),
            None => procedural.push(PropInstance::from_harvestable(p)),
        }
    }
    (statics, procedural)
}

impl Scene {
    /// `renderer` already holds nothing; the parts' primitives are uploaded here (body, head, pickaxe).
    pub fn new(mut renderer: Renderer, character: Character, body: &[SkinnedPrimitive], head: &[SkinnedPrimitive], pick: &[SkinnedPrimitive]) -> Scene {
        let palette = |i: usize| character.rig.parts.get(i).map(|p| p.palette_len()).unwrap_or(1);
        let (b, h) = (renderer.create_model(body, palette(0)), renderer.create_model(head, palette(1)));
        let p = renderer.create_model(pick, 1);
        Scene { renderer, character, body: b, head: h, pick: p, props: vec![], real: None, statics: vec![], grass: None, models: vec![], last_stats: None, draws: 0, show_pickaxe: true, loadout: None, guns: vec![], held: None, weapon_origin: None, reported_missing: Default::default(), orig_guns: None }
    }

    /// GUN MATERIAL HOOK: builds the three firearms with their original material (shader permutation, parameters, textures); on error nothing is installed.
    pub fn install_original_guns(&mut self, install: &std::path::Path) -> Result<(), String> {
        let Some(res) = self.loadout.as_ref() else { return Ok(()) };
        self.orig_guns = Some(crate::gun_orig::install(&mut self.renderer, res, install)?);
        Ok(())
    }

    /// Evaluates the animation graph for this tick, uploads the character / prop matrices and rebuilds the model list (draw + shadow casters).
    /// Uploads the three original firearms (loadout items 0..3) and prepares their skeleton evaluators. Runs once, on the UI thread, when the world is built.
    pub fn install_guns(&mut self) -> Result<(), String> {
        let Some(res) = self.loadout.as_ref() else { return Ok(()) };
        let mut guns = vec![];
        for i in 0..3 {
            let prims = gun_primitives(res, i)?;
            let part = res.items[i].mesh.clone().ok_or("gun has no mesh")?;
            let clips = res.weapon_clips(i)?;
            let model = self.renderer.create_model(&prims, part.palette_len());
            guns.push(HeldGun { model, rig: CharacterRig::new(vec![part]), clips, pose: CharacterPose::default() });
        }
        self.guns = guns;
        Ok(())
    }

    pub fn update(&mut self, sim: &mut GameSim) -> &[ModelId] {
        let world = pawn_matrix(sim);
        self.models.clear();
        self.character.evaluate(sim);
        for m in self.character.pose.missing_clips.iter() { if self.reported_missing.insert(m.clone()) { eprintln!("[anim] missing clip requested by the pose tree: {m}"); } }
        let pose = &self.character.pose;
        if let Some(p) = pose.parts.first() { self.renderer.update_model(self.body, world, &p.skin_matrices); }
        if let Some(p) = pose.parts.get(1) { self.renderer.update_model(self.head, world, &p.skin_matrices); }
        let socket = pose.socket("RightHand").unwrap_or(Mat4::IDENTITY);
        // In the original pawn the weapon mesh component snaps to RightHand
        // after the complete post-process graph: HandIKRetargeting, CopyBone,
        // then the hand IK nodes.  Guns, the pickaxe and weapon-origin use
        // that one final socket basis.
        let weapon_socket = socket;
        // `OrigGuns` renders the recovered mesh/material path itself.  The
        // older `guns` list below is strictly a fallback; drawing both places
        // two copies of a firearm on the same socket and makes the hands look
        // detached from the visible model.
        let using_original_gun = self.orig_guns.as_mut().map(|o| {
            o.update(if sim.cam_view.hide_pawn { None } else { self.held.clone() }, world * weapon_socket);
            o.is_drawing()
        }).unwrap_or(false);
        self.renderer.update_model(self.pick, world, &[socket]);
        self.weapon_origin = pose.socket("RightHand").map(|s| (world * s).w_axis.truncate());
        self.sync_props(sim);
        if !sim.cam_view.hide_pawn {
            self.models.extend([self.body, self.head]);
            if self.show_pickaxe { self.models.push(self.pick); }
            if !using_original_gun { if let Some((g, clip)) = self.held.clone() {
                if let Some(h) = self.guns.get_mut(g) {
                    // The weapon's own montage drives its skeleton; without one it
                    // rests in its bind pose. The component itself is snapped
                    // to the final player RightHand socket.
                    let tree = match &clip { Some((c, t)) => Node::clip(c, *t, false), None => Node::Ref };
                    h.rig.evaluate_into(&tree, &h.clips, &mut h.pose);
                    if let Some(p) = h.pose.parts.first() { self.renderer.update_model(h.model, world * weapon_socket, &p.skin_matrices); }
                    self.models.push(h.model);
                }
            }}
        }
        &self.models
    }

    /// Uploads the harvestables when (and only when) the live prop list differs from what the renderer already holds.
    fn sync_props(&mut self, sim: &GameSim) {
        let Some(real) = self.real.as_ref() else {
            // no real assets: every harvestable is the procedural placeholder
            let now = PropInstance::from_world(&sim.world);
            if now != self.props { self.renderer.set_prop_instances(&now); self.props = now; }
            return;
        };
        let (statics, procedural) = split_props(&sim.world.props, |h| real.mesh_of(h));
        // FOLIAGE HOOK: the grass near the camera rides along in the same static-mesh list (rebuilt when the camera moved a few metres)
        let grass_changed = self.grass.as_mut().map_or(false, |g| g.update(sim.cam_view.eye));
        if grass_changed || statics != self.statics {
            let mut all = statics.clone();
            if let Some(g) = &self.grass { all.extend_from_slice(g.instances()); }
            self.renderer.set_static_instances(&all);
            self.statics = statics;
        }
        if procedural != self.props { self.renderer.set_prop_instances(&procedural); self.props = procedural; }
    }

        /// Draws the scene prepared by [`Scene::update`] into `target`.
    pub fn draw(&mut self, camera: &CameraParams, time: f32, dt: f32, options: &RenderOptions, target: &TextureView) {
        let t = std::time::Instant::now();
        crate::particles::submit(&mut self.renderer, camera, dt); // PARTICLE HOOK: tick the live particle systems, hand their draw data to the renderer
        if let Some(o) = self.orig_guns.as_mut() { o.submit(&mut self.renderer, camera, time); } // GUN MATERIAL HOOK: the original-material gun draws of this frame
        let stats = self.renderer.render(&FrameParams { camera, time, dt, models: &self.models, options }, target);
        self.last_stats = Some(stats);
        let ms = t.elapsed().as_secs_f32() * 1000.;
        // the first frame at a new size re-creates the HDR / depth / mip targets, the first with FXAA creates the LDR target, a new shadow size the cascades
        if ms > crate::gfx::SLOW_MS * 2. { eprintln!("[slow] scene render {}x{} took {ms:.1} ms (frame {})", camera.width, camera.height, self.draws); }
        self.draws += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fn_render::MaterialKind;
    #[test] fn mat_slots() {
        let s = parse_mat("Diffuse=A_C\nNormal=A_N\nSpecPower=A_S\nOther[0]=A_M\nOther[1]=T_NoisePack_01\n");
        assert_eq!(s, MatSlots { diffuse: Some("A_C".into()), normal: Some("A_N".into()), spec: Some("A_S".into()), mask: Some("A_M".into()) });
        assert_eq!(parse_mat("Diffuse=X\nOther[0]=T_NoisePack_01\n").mask, None);
    }
    #[test] fn rigid_binds_everything_to_joint_zero() {
        let mut p = vec![SkinnedPrimitive { vertices: vec![SkinnedVertex { joints: [3, 4, 5, 6], weights: [0.4; 4], ..Default::default() }], indices: vec![0, 0, 0], material: MaterialImages::default(), kind: MaterialKind::Prop }];
        make_rigid(&mut p); assert_eq!(p[0].vertices[0].joints, [0; 4]); assert_eq!(p[0].vertices[0].weights, [1., 0., 0., 0.]);
    }
    #[test] fn props_go_through_the_static_prop_path_with_the_live_shake() {
        use fn_combat::{Harvestable, Kind};
        let mut g = GameSim::new(); g.step(&Default::default(), 1. / 60.);
        let inst = PropInstance::from_world(&g.world);
        assert_eq!(inst.len(), g.world.props.iter().filter(|p| p.alive()).count());
        assert!(inst.iter().all(|i| i.scale == 1. && i.yaw == 0.), "the sandbox props are placed exactly like the old runtime (no yaw / scale)");
        let mut p = Harvestable::new(Kind::Tree, Vec3::new(1., 0., 2.)); p.shake = 0.1;
        let i = PropInstance::from_harvestable(&p); assert_eq!((i.pos, i.shake), (p.pos, 0.1));
        // the same expansion the renderer does equals fn_combat's per-vertex sway (what the old runtime drew)
        let v = fn_render::PropTemplates::default().build(&[i]);
        let first = fn_combat::prop_mesh(Kind::Tree)[0].0[0];
        assert!((Vec3::from(v[0].pos) - fn_combat::prop_vertex(&p, first)).length() < 1e-4);
    }    #[test] fn pawn_matrix_places_the_feet() {
        let mut g = GameSim::new(); g.step(&Default::default(), 1. / 60.);
        assert!((pawn_matrix(&g).transform_point3(Vec3::ZERO) - Vec3::new(g.x, g.y - EYE, g.z)).length() < 1e-5);
    }
    #[test] fn camera_params_follow_the_view() {
        let mut g = GameSim::new(); g.step(&Default::default(), 1. / 60.);
        let c = camera_for(&g, 1280, 720);
        assert_eq!(c.eye, g.cam_view.eye); assert!((c.forward - g.cam_view.forward).length() < 1e-3, "{:?} {:?}", c.forward, g.cam_view.forward); assert_eq!((c.width, c.height), (1280, 720));
    }
    #[cfg(feature = "sim-compare")]
    #[test] fn pose_hash_matches_the_fn_sim_hash() {
        let pose = vec![(Vec3::new(1., 2., 3.), Quat::IDENTITY), (Vec3::ZERO, Quat::from_rotation_y(0.5))];
        assert_eq!(pose_hash(&pose), fn_sim::rig::pose_hash(&pose));
    }
}


