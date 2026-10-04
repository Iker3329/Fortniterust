//! The 3D part of the Fortnite 1.8 Battle Royale lobby (what is behind the UI), drawn from the data of `lobby_world.rs`.
//!
//! Frame: shadow map of the lobby's directional light -> HDR scene (original cooked base-pass programs of every static mesh material, run through
//! `fn-dxbc` -> WGSL, then this file's own light resolve) -> translucent unlit FX meshes -> bloom chain -> UE 4.16 tonemapper (film curve + colour grade from the
//! stacked post-process volumes) + colour-grading LUT strip + vignette -> target.
//!
//! Everything is in UE space (centimetres, left-handed, X forward / Y right / Z up) and uses the same clip matrix the original programs get through their
//! `View` buffer, so original and substitute shaders agree. See `parity/lobby/SCENE.md` for what is original, what is a labelled substitute and what is missing.
use crate::lobby_world::{self as world, LightKind, LobbySceneData, Xf};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

pub const NEAR_CM: f32 = 10.0;
pub const FAR_CM: f32 = 6_000_000.0;
const MAX_LIGHTS: usize = 64;
const PRIM_STRIDE: u64 = 512;
const SHADOW_SIZE: u32 = 4096;
const PT_SHADOW_SIZE: u32 = 1024;
const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

// ---------------------------------------------------------------------------------------------------------------------------------
// camera
// ---------------------------------------------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
pub struct CameraFrame { pub eye: Vec3, pub fwd: Vec3, pub right: Vec3, pub up: Vec3, pub tan_x: f32, pub tan_y: f32, pub width: u32, pub height: u32 }
impl CameraFrame {
    pub fn new(xf: &Xf, fov_h_deg: f32, width: u32, height: u32) -> CameraFrame {
        let fwd = xf.q * Vec3::X;
        let up0 = xf.q * Vec3::Z;
        let right = up0.cross(fwd).normalize();
        let up = fwd.cross(right).normalize();
        let tan_x = (fov_h_deg.to_radians() * 0.5).tan();
        CameraFrame { eye: xf.t, fwd, right, up, tan_x, tan_y: tan_x * height as f32 / width as f32, width, height }
    }
    /// UE world -> clip (wgpu depth 0..1, `clip.w` = view depth), exactly the matrix of `TranslatedWorldToClip` + translation.
    pub fn view_proj(&self) -> Mat4 {
        let (sx, sy) = (1.0 / self.tan_x, 1.0 / self.tan_y);
        let (n, f) = (NEAR_CM, FAR_CM);
        let (a, b) = (f / (f - n), -f * n / (f - n));
        let r0 = Vec4::new(self.right.x * sx, self.right.y * sx, self.right.z * sx, -self.right.dot(self.eye) * sx);
        let r1 = Vec4::new(self.up.x * sy, self.up.y * sy, self.up.z * sy, -self.up.dot(self.eye) * sy);
        let r2 = Vec4::new(self.fwd.x * a, self.fwd.y * a, self.fwd.z * a, -self.fwd.dot(self.eye) * a + b);
        let r3 = Vec4::new(self.fwd.x, self.fwd.y, self.fwd.z, -self.fwd.dot(self.eye));
        Mat4::from_cols(Vec4::new(r0.x, r1.x, r2.x, r3.x), Vec4::new(r0.y, r1.y, r2.y, r3.y), Vec4::new(r0.z, r1.z, r2.z, r3.z), Vec4::new(r0.w, r1.w, r2.w, r3.w))
    }
    /// The 178-slot `View` uniform buffer of the original programs (conversion through the port's glTF-style axes, see fn-matshader/view.rs).
    pub fn view_buffer(&self, time: f32) -> Vec<[f32; 4]> {
        let sw = |v: Vec3| Vec3::new(v.x, v.z, v.y);
        let vi = fn_matshader::view::ViewInput {
            eye: sw(self.eye) * 0.01, forward: sw(self.fwd), up: Vec3::Y, tan_half: (self.tan_x, self.tan_y),
            near_m: NEAR_CM * 0.01, far_m: FAR_CM * 0.01, width: self.width, height: self.height, time,
        };
        vi.view_buffer([0.; 3])
    }
}

// ---------------------------------------------------------------------------------------------------------------------------------
// CPU assets
// ---------------------------------------------------------------------------------------------------------------------------------
pub struct TexAsset { pub levels: Vec<(u32, u32, Vec<u8>)>, pub srgb: bool, pub filter: String, pub address: String }

pub struct MeshAsset {
    pub path: String,
    pub lod: fn_mesh::StaticLod,
    pub default_materials: Vec<Option<String>>,
    pub center: Vec3,
    pub radius: f32,
}

/// 9 real SH coefficients (RGB) of the sky light cubemap radiance, times the light's intensity.
#[derive(Clone, Debug, Default)]
pub struct SkySh { pub c: [[f32; 3]; 9], pub source: String }

pub struct CharAsset {
    pub hero: String,
    pub parts: Vec<fn_pose::br_character::BrPart>,
    pub rig: fn_pose::CharacterRig,
    /// shared between the lobby and the locker scene (decoding every clip of the AnimBP takes ~40 s)
    pub clips: Arc<fn_pose::ClipStore>,
    pub notes: Vec<String>,
}

impl CharAsset {
    /// A second pawn of the same hero for another scene (the LOCKER view): parts cloned, rig rebuilt, clips shared.
    pub fn share(&self) -> CharAsset {
        CharAsset { hero: self.hero.clone(), parts: self.parts.clone(), rig: fn_pose::CharacterRig::new(self.parts.iter().take(2).map(|b| b.part.clone()).collect()), clips: self.clips.clone(), notes: self.notes.clone() }
    }
}

/// One party slot (`PartyCharacterPlacementHelper` Player0..4) in UE world centimetres.
#[derive(Clone, Copy, Debug)]
pub struct PartySlot { pub player_id: i32, pub hidden: bool, pub occupied: bool, pub pawn_world_cm: Vec3, pub pad_world_cm: Vec3, pub top_widget_cm: Vec3, pub add_widget_cm: Vec3,
    /// `UWidgetComponent.Pivot` of the name plate / "+" components (screen-space slot alignment, see `lobby_world::WidgetCompInfo`)
    pub top_pivot: [f32; 2], pub add_pivot: [f32; 2] }

#[derive(Default, Clone, Debug)]
pub struct SceneReport {
    pub lines: Vec<String>,
    pub original_materials: Vec<String>,
    pub substituted: Vec<(String, String)>,
    pub failed_meshes: Vec<(String, String)>,
}

pub struct LobbyAssets {
    pub data: LobbySceneData,
    pub meshes: HashMap<String, MeshAsset>,
    pub materials: HashMap<String, Arc<fn_fx::material::Material>>,
    pub textures: HashMap<String, TexAsset>,
    pub sky: SkySh,
    pub lut: Option<TexAsset>,
    pub character: Option<CharAsset>,
    pub report: SceneReport,
}

fn pak_pkg(set: &fn_pak::PakSet, long: &str) -> Result<fn_ueio::Package, String> {
    fn_pose::loader::package(&fn_pose::ClipSourceDir::Pak(set), long)
}

fn load_texture(set: &fn_pak::PakSet, path: &str) -> Result<TexAsset, String> {
    let long = path.split('.').next().unwrap_or(path);
    let pkg = pak_pkg(set, long)?;
    let t = fn_texture::Texture2D::parse(&pkg).map_err(|e| format!("{long}: {e}"))?;
    if t.is_cube() { return Err(format!("{long}: cube map")); }
    let mut levels = vec![];
    let mips = t.available_mips();
    let first = *mips.first().ok_or_else(|| format!("{long}: no mip"))?;
    let mut expect: Option<(u32, u32)> = None;
    for i in mips {
        if i < first { continue; }
        let img = t.decode(i, fn_texture::DecodeOptions::default()).map_err(|e| format!("{long}: mip {i}: {e}"))?;
        let (w, h) = (img.width as u32, img.height as u32);
        if let Some((pw, ph)) = expect { if w != (pw / 2).max(1) || h != (ph / 2).max(1) { break; } }
        expect = Some((w, h));
        levels.push((w, h, img.to_rgba8()));
    }
    let filter = t.props.text("Filter").map(|s| s.rsplit("::").next().unwrap_or(s).to_owned()).unwrap_or_else(|| "TF_Default".into());
    Ok(TexAsset { levels, srgb: t.srgb, filter, address: t.address_x.clone() })
}

/// Real SH (order 2) of a cube map's radiance (UE cube face convention = the D3D one applied to UE world axes), lower hemisphere black.
fn cube_sh(set: &fn_pak::PakSet, path: &str, intensity: f32, lower_black: bool) -> Result<SkySh, String> {
    let long = path.split('.').next().unwrap_or(path);
    let pkg = pak_pkg(set, long)?;
    let t = fn_texture::Texture2D::parse(&pkg).map_err(|e| format!("{long}: {e}"))?;
    if !t.is_cube() { return Err(format!("{long}: not a cube map")); }
    let mips = t.available_mips();
    let mip = mips.iter().copied().find(|&i| t.mips[i].width <= 64).unwrap_or(*mips.last().ok_or("no mip")?);
    let mut c = [[0f64; 3]; 9];
    let mut wsum = 0f64;
    for face in 0..6 {
        let img = t.decode_slice(mip, face, fn_texture::DecodeOptions::default()).map_err(|e| e.to_string())?;
        let n = img.width;
        let px: Vec<f32> = match &img.pixels {
            fn_texture::Pixels::RgbaF16(v) => v.iter().map(|&h| fn_ueio::f16_to_f32(h)).collect(),
            fn_texture::Pixels::Rgba8(v) => v.iter().map(|&b| (b as f32 / 255.).powf(2.2)).collect(),
        };
        for y in 0..n {
            for x in 0..n {
                let (u, v) = ((x as f32 + 0.5) / n as f32 * 2. - 1., (y as f32 + 0.5) / n as f32 * 2. - 1.);
                // D3D cube faces: sc / tc per major axis (v points down in the image)
                let d = match face { 0 => Vec3::new(1., -v, -u), 1 => Vec3::new(-1., -v, u), 2 => Vec3::new(u, 1., v), 3 => Vec3::new(u, -1., -v), 4 => Vec3::new(u, -v, 1.), _ => Vec3::new(-u, -v, -1.) };
                let dl = d.length();
                let dir = d / dl;
                let w = (1.0 / (dl * dl * dl)) as f64 * (2.0 / n as f64) * (2.0 / n as f64);
                let rgb = [px[(y * n + x) * 4], px[(y * n + x) * 4 + 1], px[(y * n + x) * 4 + 2]];
                let l = if lower_black && dir.z < 0. { [0.; 3] } else { rgb };
                let basis = sh_basis(dir);
                for k in 0..9 { for ch in 0..3 { c[k][ch] += (l[ch] as f64) * (basis[k] as f64) * w; } }
                wsum += w;
            }
        }
    }
    let _ = wsum; // wsum == 4 pi
    Ok(SkySh { c: std::array::from_fn(|k| std::array::from_fn(|ch| c[k][ch] as f32 * intensity)), source: path.to_owned() })
}
pub fn sh_basis(d: Vec3) -> [f32; 9] {
    [0.282095, 0.488603 * d.y, 0.488603 * d.z, 0.488603 * d.x, 1.092548 * d.x * d.y, 1.092548 * d.y * d.z, 0.315392 * (3. * d.z * d.z - 1.), 1.092548 * d.x * d.z, 0.546274 * (d.x * d.x - d.y * d.y)]
}

fn mesh_asset(set: &fn_pak::PakSet, path: &str) -> Result<MeshAsset, String> {
    let long = path.split('.').next().unwrap_or(path);
    let pkg = pak_pkg(set, long)?;
    let m = fn_mesh::StaticMesh::parse(&pkg).map_err(|e| format!("{long}: {e}"))?;
    let lod = m.lods.first().ok_or("no LOD")?.clone();
    let (mn, mx) = m.position_bounds(0).unwrap_or(([0.; 3], [0.; 3]));
    let (mn, mx) = (Vec3::from(mn), Vec3::from(mx));
    Ok(MeshAsset { path: path.to_owned(), lod, default_materials: m.materials.iter().map(|x| x.material.clone()).collect(), center: (mn + mx) * 0.5, radius: ((mx - mn) * 0.5).length() })
}

impl LobbyAssets {
    pub fn load(install: &Path, hero: Option<&str>) -> Result<LobbyAssets, String> {
        let set = fn_pak::PakSet::open_install(install).map_err(|e| e.to_string())?;
        // the hero (parts + every AnimBP clip) decodes on its own thread while the scene's static assets decode on the others
        let t0 = std::time::Instant::now();
        let (character, built) = std::thread::scope(|s| {
            let h = s.spawn(|| { let t = std::time::Instant::now(); let r = load_character(&set, hero); eprintln!("[scene-load] hero + AnimBP clips {:.2} s", t.elapsed().as_secs_f32()); r });
            let built = (|| { let data = world::read_lobby_scene_from(&set)?; eprintln!("[scene-load] map data {:.2} s", t0.elapsed().as_secs_f32()); Self::from_data(&set, data, None, SceneReport::default()) })();
            (h.join().unwrap_or_else(|_| Err("character loader panicked".into())), built)
        });
        let mut assets = built?;
        match character { Ok(c) => assets.character = Some(c), Err(e) => assets.report.lines.push(format!("character: {e}")) }
        eprintln!("[scene-load] total {:.2} s", t0.elapsed().as_secs_f32());
        Ok(assets)
    }
    /// The assets of the LOCKER view (`world::read_cosmetics_scene_from`): the Cosmetics camera, its backdrop, the Vault hero placement. `character` is the lobby scene's
    /// hero (`CharAsset::share`), so the animation clips are decoded once.
    pub fn load_cosmetics(install: &Path, character: Option<CharAsset>) -> Result<LobbyAssets, String> {
        let set = fn_pak::PakSet::open_install(install).map_err(|e| e.to_string())?;
        let data = world::read_cosmetics_scene_from(&set)?;
        Self::from_data(&set, data, character, SceneReport::default())
    }

    /// The assets of any front-end camera view (`world::read_view_scene_from`): what `SetFrontEndCamera(<camera>)` shows behind a screen's widgets.
    pub fn load_view(install: &Path, camera: &str, radius_cm: f32) -> Result<LobbyAssets, String> {
        let set = fn_pak::PakSet::open_install(install).map_err(|e| e.to_string())?;
        let data = world::read_view_scene_from(&set, camera, radius_cm)?;
        Self::from_data(&set, data, None, SceneReport::default())
    }

    fn from_data(set: &fn_pak::PakSet, data: LobbySceneData, character: Option<CharAsset>, mut report: SceneReport) -> Result<LobbyAssets, String> {
        let set = set;
        let mut meshes: HashMap<String, MeshAsset> = HashMap::new();
        let mut materials: HashMap<String, Arc<fn_fx::material::Material>> = HashMap::new();
        let mut textures: HashMap<String, TexAsset> = HashMap::new();
        let t = std::time::Instant::now();
        // stage 1: every distinct visible mesh, in parallel
        let mut mesh_names: Vec<String> = vec![];
        for s in data.statics.iter().filter(|s| s.visible) { if !mesh_names.contains(&s.mesh) { mesh_names.push(s.mesh.clone()); } }
        for (name, r) in mesh_names.iter().zip(par_map(&mesh_names, |m| mesh_asset(set, m))) {
            match r { Ok(m) => { meshes.insert(name.clone(), m); } Err(e) => report.failed_meshes.push((name.clone(), e)) }
        }
        eprintln!("[scene-load]   meshes ({}) {:.2} s", mesh_names.len(), t.elapsed().as_secs_f32());
        // stage 2: every distinct material, in parallel
        let mut mat_paths: Vec<String> = vec![];
        for s in data.statics.iter().filter(|s| s.visible) {
            let Some(m) = meshes.get(&s.mesh) else { continue };
            for i in 0..m.default_materials.len().max(s.material_overrides.len()) {
                let path = s.material_overrides.get(i).cloned().flatten().or_else(|| m.default_materials.get(i).cloned().flatten());
                let Some(path) = path else { continue };
                if !mat_paths.contains(&path) { mat_paths.push(path); }
            }
        }
        for (path, r) in mat_paths.iter().zip(par_map(&mat_paths, |p| fn_fx::material::Material::load(set, p))) {
            match r { Ok(mat) => { materials.insert(path.clone(), Arc::new(mat)); } Err(e) => report.substituted.push((path.clone(), format!("material load: {e}"))) }
        }
        eprintln!("[scene-load]   materials ({}) {:.2} s", mat_paths.len(), t.elapsed().as_secs_f32());
        // M_Lobby_Pad: PartyCharacterPlacementHelper makes a dynamic instance per pad and sets Mouseover / Occupied / Selected; the occupied pad (PlayerOnPad) gets Occupied = 1
        if let Some(base) = materials.keys().find(|k| k.ends_with("M_Lobby_Pad.M_Lobby_Pad")).cloned() {
            for (suffix, occ) in [("", 0.0f32), ("#occupied", 1.0)] {
                if let Ok(mut m) = fn_fx::material::Material::load(&set, &base) {
                    for (k, v) in [("Mouseover", 0.0), ("Occupied", occ), ("Selected", 0.0)] { m.overrides.scalars.insert(k.to_owned(), v); }
                    materials.insert(format!("{base}{suffix}"), Arc::new(m));
                }
            }
        }
        // stage 3: every distinct texture, the sky-light cube map and the grading LUT, in parallel
        let mut tex_paths: Vec<String> = vec![];
        for mat in materials.values() {
            for k in 0..mat.uniform.textures2d.len() {
                let Some((tp, _)) = mat.texture_slot(k) else { continue };
                if !tex_paths.contains(&tp) { tex_paths.push(tp); }
            }
        }
        let mut sky = SkySh::default();
        let sky_light = data.lights.iter().find(|l| l.kind == LightKind::Sky && l.visible && l.cubemap.is_some());
        let lut_path = data.post.color_grading_lut.as_deref();
        let (tex_res, sky_res, lut_res) = std::thread::scope(|s| {
            let sky_h = sky_light.map(|l| s.spawn(move || cube_sh(set, l.cubemap.as_deref().unwrap(), l.intensity, true)));
            let lut_h = lut_path.map(|p| s.spawn(move || load_texture(set, p)));
            let tex = par_map(&tex_paths, |p| load_texture(set, p));
            (tex, sky_h.map(|h| h.join()), lut_h.map(|h| h.join()))
        });
        for (tp, r) in tex_paths.iter().zip(tex_res) { match r { Ok(t) => { textures.insert(tp.clone(), t); } Err(e) => report.lines.push(format!("texture {tp}: {e}")) } }
        if let Some(r) = sky_res { match r { Ok(Ok(s)) => sky = s, Ok(Err(e)) => report.lines.push(format!("sky light cubemap: {e}")), Err(_) => report.lines.push("sky light cubemap: panicked".into()) } }
        let lut = match lut_res { Some(Ok(Ok(t))) => Some(t), Some(Ok(Err(e))) => { report.lines.push(format!("LUT {}: {e}", lut_path.unwrap_or(""))); None } _ => None };
        eprintln!("[scene-load]   textures ({}) + sky + LUT {:.2} s", tex_paths.len(), t.elapsed().as_secs_f32());        Ok(LobbyAssets { data, meshes, materials, textures, sky, lut, character, report })
    }
}

/// Runs `f` over `items` on a small thread pool (std only), results in the order of `items`.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = items.len();
    let out: Vec<std::sync::Mutex<Option<R>>> = (0..n).map(|_| std::sync::Mutex::new(None)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map(|p| p.get()).unwrap_or(4).clamp(2, 12).min(n.max(1));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if i >= n { break; }
                let r = f(&items[i]);
                *out[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(r);
            });
        }
    });
    out.into_iter().filter_map(|m| m.into_inner().unwrap_or_else(|e| e.into_inner())).collect()
}

fn load_character(set: &fn_pak::PakSet, hero: Option<&str>) -> Result<CharAsset, String> { load_character_asset(set, hero.unwrap_or(fn_pose::br_character::DEFAULT_HERO)) }

/// CPU half of an outfit change: the parts (skeletal meshes, material sections, decoded textures) of a hero. `Clone` so a caller may keep recently used heroes.
#[derive(Clone)]
pub struct HeroParts { pub hero: String, pub parts: Vec<fn_pose::br_character::BrPart>, pub notes: Vec<String> }

/// Decodes a hero's parts from the paks (`fn_pose::br_character::BrCharacter`, ~0.1-0.3 s; no GPU, no clips). Pure CPU: may run on any thread.
pub fn load_hero_parts(set: &fn_pak::PakSet, hero: &str) -> Result<HeroParts, String> {
    let ch = fn_pose::br_character::BrCharacter::load(&fn_pose::ClipSourceDir::Pak(set), hero)?;
    Ok(HeroParts { hero: ch.hero.clone(), notes: ch.notes.clone(), parts: ch.parts })
}

/// What a worker thread needs to upload a hero to the GPU (`LobbyScene::hero_builder`): cheap handles of the scene's device / queue / character bind layouts. The upload
/// (vertex / index / bone buffers, textures with their CPU mip chains, bind groups) is the slow part of an outfit change (~1 s), so it runs off the render thread;
/// `LobbyScene::set_hero_ready` then only swaps the result in.
#[derive(Clone)]
pub struct HeroBuilder { device: wgpu::Device, queue: wgpu::Queue, char_layout: wgpu::BindGroupLayout, orig_layout: wgpu::BindGroupLayout, samp: wgpu::Sampler, model: Mat4 }
/// A hero uploaded by a `HeroBuilder` (opaque; hand it to `LobbyScene::set_hero_ready`).
pub struct HeroReady { hero: String, parts: Vec<fn_pose::br_character::BrPart>, rig: fn_pose::CharacterRig, notes: Vec<String>, gpu: Vec<CharPartGpu> }
impl HeroReady { pub fn hero(&self) -> &str { &self.hero } }
impl HeroBuilder {
    /// Uploads `p` (the rig is built from its body / head parts, like `BrCharacter::rig_parts`).
    pub fn build(&self, p: &HeroParts) -> HeroReady {
        let rig = fn_pose::CharacterRig::new(p.parts.iter().take(2).map(|b| b.part.clone()).collect());
        // `CharAsset` shape the part uploader expects (the clips are not used by it)
        let gpu = build_char_parts_of(&self.device, &self.queue, &p.parts, &self.model, &self.char_layout, &self.orig_layout, &self.samp);
        HeroReady { hero: p.hero.clone(), parts: p.parts.clone(), rig, notes: p.notes.clone(), gpu }
    }
}
pub fn load_character_asset(set: &fn_pak::PakSet, hero: &str) -> Result<CharAsset, String> {
    let src = fn_pose::ClipSourceDir::Pak(set);
    let t = std::time::Instant::now();
    let ch = fn_pose::br_character::BrCharacter::load(&src, hero)?;
    eprintln!("[scene-load]   hero parts {:.2} s", t.elapsed().as_secs_f32());
    let skel = fn_pose::clip::load_player_skeleton_info(&src)?;
    // (The `Fortnite_M_Avg_Player_MenusScreen_AnimBP` Commando / Ninja / Outlander / Constructor machines and the `Menu/...Montage_Lobby` montages are the Save the World
    // front end's hero classes: not used. The BR hero parts carry no `Animation.FrontEnd.Lobby` montage.)
    // BATTLE ROYALE path: the hero's pawn is `PlayerPawn_Athena_C` (`HID_*.OverridePawnClass`), whose parts follow the master `Fortnite_M_Avg_Player` skeleton driven by
    // `Fortnite_M_Avg_Player_AnimBlueprint`; the lobby pawn plays that AnimBP's idle (the port's `fn_animgraph::AnimState`, the same graph the game runs). All clips the graph can
    // ask for are loaded.
    let mut names = fn_animgraph::required_clips();
    names.sort();
    names.dedup();
    let clips = fn_pose::ClipStore::load(&src, &names, &skel, fn_pose::SampleMode::Ue)?;
    eprintln!("[scene-load]   AnimBP clips ({}) {:.2} s", names.len(), t.elapsed().as_secs_f32());
    let rig = fn_pose::CharacterRig::new(ch.rig_parts());
    let notes = ch.notes.clone();
    Ok(CharAsset { hero: ch.hero.clone(), parts: ch.parts, rig, clips: Arc::new(clips), notes })
}

// ---------------------------------------------------------------------------------------------------------------------------------
// GPU structures
// ---------------------------------------------------------------------------------------------------------------------------------
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuLight { pos: [f32; 4], color: [f32; 4], dir: [f32; 4], cone: [f32; 4] }

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameU {
    inv_vp: [[f32; 4]; 4],
    shadow_vp: [[f32; 4]; 4],
    cam: [f32; 4],
    viewport: [f32; 4],
    sun_dir: [f32; 4],
    sun_color: [f32; 4],
    sh: [[f32; 4]; 9],
    fog_a: [f32; 4],
    fog_b: [f32; 4],
    counts: [u32; 4],
    lights: [GpuLight; MAX_LIGHTS],
    pt_vp: [[[f32; 4]; 4]; 6],
    pt_idx: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostU {
    wb0: [f32; 4], wb1: [f32; 4], wb2: [f32; 4],
    film_stw: [f32; 4], film_clip: [f32; 4],
    bloom: [f32; 4], vignette: [f32; 4], view: [f32; 4],
    sat_shadows: [f32; 4], sat_mid: [f32; 4], sat_high: [f32; 4],
    grade: [f32; 4], lut: [f32; 4], bloom_tints: [[f32; 4]; 6], bloom_sizes: [[f32; 4]; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinVertex { pos: [f32; 3], normal: [f32; 3], uv: [f32; 2], joints: [u16; 4], weights: [f32; 4] }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Blend { Opaque, Translucent, Additive, Modulate }

struct MatPipe {
    pipeline: wgpu::RenderPipeline,
    layouts: Vec<wgpu::BindGroupLayout>,
    /// per group: entries to (re)build the bind group
    group_entries: Vec<Vec<GroupEntry>>,
    groups: Vec<wgpu::BindGroup>,
    dyn_count: usize,
    blend: Blend,
    uses_depth: bool,
}
#[derive(Clone)]
enum GroupEntry { Prim { binding: u32, size: u64, buf: wgpu::Buffer }, Buf { binding: u32, buf: wgpu::Buffer }, Tex { binding: u32, view: wgpu::TextureView }, Samp { binding: u32, s: wgpu::Sampler }, Depth { binding: u32 } }

struct MeshGpu { vb: wgpu::Buffer, ib: wgpu::Buffer, pairs: usize, sections: Vec<(usize, u32, u32)>, shadow_ok: bool }

struct DrawItem { mesh: usize, material_key: usize, first: u32, count: u32, prim: u64, pos: Vec3, sort_priority: i32, shadow: bool, translucent: bool }

struct CharGpu { parts: Vec<CharPartGpu>, cam_bg: wgpu::BindGroup, shadow_bg: wgpu::BindGroup, empty_bg: wgpu::BindGroup, orig_pipe: wgpu::RenderPipeline,
    /// kept so `LobbyScene::set_character` can rebuild the part buffers without recompiling the pipelines
    char_layout: wgpu::BindGroupLayout, orig_layout: wgpu::BindGroupLayout, samp: wgpu::Sampler, model: Mat4 }
struct CharPartGpu { prims: Vec<CharPrimGpu>, bones: wgpu::Buffer }
/// `bind` = diffuse-only group (shadow pass + fallback shading); `orig` = the original character material's resources (group 0 of `CharGpu::orig_pipe`).
struct CharPrimGpu { vb: wgpu::Buffer, ib: wgpu::Buffer, count: u32, bind: wgpu::BindGroup, orig: Option<wgpu::BindGroup> }

struct Targets { w: u32, h: u32, hdr: wgpu::Texture, hdr_mips: Vec<wgpu::TextureView>, hdr_full: wgpu::TextureView, depth: wgpu::Texture, depth_view: wgpu::TextureView, depth_only: wgpu::TextureView, post_bg: wgpu::BindGroup, down_bgs: Vec<wgpu::BindGroup> }

pub struct LobbyScene {
    assets_report: SceneReport,
    pub data: LobbySceneData,
    device: wgpu::Device,
    queue: wgpu::Queue,
    out_format: wgpu::TextureFormat,
    // shared
    frame_buf: wgpu::Buffer,
    view_buf: wgpu::Buffer,
    prim_buf: wgpu::Buffer,
    fade_buf: wgpu::Buffer,
    zero_buf: wgpu::Buffer,
    group3_layout: wgpu::BindGroupLayout,
    group3: wgpu::BindGroup,
    shadow_view: wgpu::TextureView,
    shadow_pipes: Vec<wgpu::RenderPipeline>,
    cview_buf: wgpu::Buffer,
    shadow_bg: wgpu::BindGroup,
    shadow_vp_buf: wgpu::Buffer,
    /// cube shadow of one point light: face views, per-face shadow bind groups, face matrices, light index (u32::MAX = none)
    pt_face_views: Vec<wgpu::TextureView>,
    pt_bgs: Vec<wgpu::BindGroup>,
    pt_vps: [Mat4; 6],
    pt_idx: u32,
    // statics
    meshes: Vec<MeshGpu>,
    mat_pipes: Vec<MatPipe>,
    mat_bufs: Vec<wgpu::Buffer>,
    textures: Vec<(wgpu::Texture, wgpu::TextureView)>,
    samplers: Vec<wgpu::Sampler>,
    draws: Vec<DrawItem>,
    // character
    char_pipe: Option<wgpu::RenderPipeline>,
    char_shadow_pipe: Option<wgpu::RenderPipeline>,
    char_asset: Option<CharAsset>,
    char_gpu: Option<CharGpu>,
    char_xf: Option<Xf>,
    char_pose: fn_pose::CharacterPose,
    /// scene time at which the current character appeared (its Menu intro restarts there; `set_character`) and the last rendered scene time
    char_t0: f32,
    last_time: f32,
    /// The BR player AnimBP state (`fn_animgraph::AnimState`) of the hero and the time it has been stepped to (seconds since `char_t0`).
    anim: Option<(fn_animgraph::AnimState, f32)>,
    // post
    post_pipe: wgpu::RenderPipeline,
    down_pipe: wgpu::RenderPipeline,
    post_buf: wgpu::Buffer,
    lut_view: wgpu::TextureView,
    post_samp: wgpu::Sampler,
    post_layout: wgpu::BindGroupLayout,
    down_layout: wgpu::BindGroupLayout,
    targets: Option<Targets>,
    camera: Xf,
    fov: f32,
    sun: Option<(Vec3, [f32; 3])>,
    sh: SkySh,
    fog: Option<world::FogInst>,
    lights: Vec<GpuLight>,
    pub report: SceneReport,
}

// ---------------------------------------------------------------------------------------------------------------------------------
// WGSL
// ---------------------------------------------------------------------------------------------------------------------------------
const PRELUDE: &str = r#"
struct LLight { pos: vec4<f32>, color: vec4<f32>, dir: vec4<f32>, cone: vec4<f32> }
struct LFrame {
    inv_vp: mat4x4<f32>, shadow_vp: mat4x4<f32>, cam: vec4<f32>, viewport: vec4<f32>,
    sun_dir: vec4<f32>, sun_color: vec4<f32>, sh: array<vec4<f32>, 9>,
    fog_a: vec4<f32>, fog_b: vec4<f32>, counts: vec4<u32>, lights: array<LLight, 64>,
    pt_vp: array<mat4x4<f32>, 6>, pt_idx: vec4<u32>,
}
@group(3) @binding(0) var<uniform> LF: LFrame;
@group(3) @binding(1) var shadow_map: texture_depth_2d;
@group(3) @binding(2) var shadow_cmp: sampler_comparison;
@group(3) @binding(3) var pt_shadow_map: texture_depth_2d_array;

const PI: f32 = 3.14159265;

fn lobby_world(frag: vec4<f32>) -> vec3<f32> {
    let ndc = vec4<f32>(frag.x * LF.viewport.z * 2.0 - 1.0, 1.0 - frag.y * LF.viewport.w * 2.0, frag.z, 1.0);
    let h = LF.inv_vp * ndc;
    return h.xyz / h.w;
}
fn sh_irradiance(n: vec3<f32>) -> vec3<f32> {
    // radiance SH projected on the cosine lobe: A_l / pi = 1, 2/3, 1/4
    var r = LF.sh[0].xyz * 0.282095;
    r = r + (LF.sh[1].xyz * (0.488603 * n.y) + LF.sh[2].xyz * (0.488603 * n.z) + LF.sh[3].xyz * (0.488603 * n.x)) * (2.0 / 3.0);
    r = r + (LF.sh[4].xyz * (1.092548 * n.x * n.y) + LF.sh[5].xyz * (1.092548 * n.y * n.z) + LF.sh[6].xyz * (0.315392 * (3.0 * n.z * n.z - 1.0))
        + LF.sh[7].xyz * (1.092548 * n.x * n.z) + LF.sh[8].xyz * (0.546274 * (n.x * n.x - n.y * n.y))) * 0.25;
    return max(r, vec3<f32>(0.0));
}
fn sun_shadow(world: vec3<f32>, n: vec3<f32>) -> f32 {
    if (LF.sun_dir.w < 0.5) { return 1.0; }
    let p = LF.shadow_vp * vec4<f32>(world + n * 1.5, 1.0);
    let uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || p.z > 1.0) { return 1.0; }
    let texel = 1.0 / 4096.0;
    var s = 0.0;
    for (var y = -1; y <= 1; y = y + 1) { for (var x = -1; x <= 1; x = x + 1) {
        s = s + textureSampleCompareLevel(shadow_map, shadow_cmp, uv + vec2<f32>(f32(x), f32(y)) * texel, p.z - 0.0004);
    } }
    return s / 9.0;
}
// cube shadow of the one shadow-casting point light (faces: +X -X +Y -Y +Z -Z, 90 degree frusta)
fn pt_shadow(world: vec3<f32>, n: vec3<f32>, lpos: vec3<f32>) -> f32 {
    let d = world - lpos;
    let a = abs(d);
    var f = 0u;
    if (a.x >= a.y && a.x >= a.z) { f = select(1u, 0u, d.x > 0.0); }
    else if (a.y >= a.z) { f = select(3u, 2u, d.y > 0.0); }
    else { f = select(5u, 4u, d.z > 0.0); }
    let p = LF.pt_vp[f] * vec4<f32>(world + n * 1.0, 1.0);
    if (p.w <= 0.0) { return 1.0; }
    let q = p.xyz / p.w;
    let uv = vec2<f32>(q.x * 0.5 + 0.5, 0.5 - q.y * 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || q.z > 1.0) { return 1.0; }
    let bias = 10.0 / (p.w * p.w);
    var s = 0.0;
    for (var y = -1; y <= 1; y = y + 1) { for (var x = -1; x <= 1; x = x + 1) {
        s = s + textureSampleCompareLevel(pt_shadow_map, shadow_cmp, uv + vec2<f32>(f32(x), f32(y)) / 1024.0, i32(f), q.z - bias);
    } }
    return s / 9.0;
}
fn ggx(nh: f32, r: f32) -> f32 { let a = max(r * r, 0.01); let a2 = a * a; let d = nh * nh * (a2 - 1.0) + 1.0; return a2 / (PI * d * d); }
fn light_term(l: LLight, world: vec3<f32>, n: vec3<f32>, v: vec3<f32>, diff: vec3<f32>, f0: vec3<f32>, rough: f32) -> vec3<f32> {
    let d = l.pos.xyz - world;
    let d2 = dot(d, d);
    let L = d * inverseSqrt(max(d2, 1e-4));
    var att = 1.0 / (d2 + 1.0);
    let rm = clamp(1.0 - (d2 * l.pos.w * l.pos.w) * (d2 * l.pos.w * l.pos.w), 0.0, 1.0);
    att = att * rm * rm;
    if (l.cone.y > 0.5) {
        // legacy (not inverse-squared) light: pow(1 - saturate((d / R)^2), FalloffExponent), intensity not x16
        att = pow(clamp(1.0 - d2 * l.pos.w * l.pos.w, 0.0, 1.0), l.cone.z);
    }
    if (l.color.w > 1.5) {
        let c = dot(-L, l.dir.xyz);
        let s = clamp((c - l.dir.w) * l.cone.x, 0.0, 1.0);
        att = att * s * s;
    }
    let nl = max(dot(n, L), 0.0);
    let h = normalize(L + v);
    let nh = max(dot(n, h), 0.0);
    let vh = max(dot(v, h), 0.0);
    let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - vh, 5.0);
    let spec = ggx(nh, rough) * 0.25 * f;
    return (diff / PI + spec) * l.color.xyz * att * nl;
}
fn lobby_shade(world: vec3<f32>, n_in: vec3<f32>, base: vec3<f32>, metallic: f32, specular: f32, rough_in: f32, ao: f32, emissive: vec3<f32>, front: bool) -> vec3<f32> {
    var n = normalize(n_in);
    let v = normalize(LF.cam.xyz - world);
    let rough = clamp(rough_in, 0.04, 1.0);
    let diff = base * (1.0 - metallic);
    let f0 = mix(vec3<f32>(0.08 * specular), base, metallic);
    var col = emissive * 0.0;
    // sky light (diffuse) with the albedo
    col = col + diff * sh_irradiance(n) * ao;
    // sun
    let sl = normalize(LF.sun_dir.xyz);
    let nl = max(dot(n, sl), 0.0);
    let h = normalize(sl + v);
    let sf = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - max(dot(v, h), 0.0), 5.0);
    let sun = (diff / PI + ggx(max(dot(n, h), 0.0), rough) * 0.25 * sf) * LF.sun_color.xyz * nl * sun_shadow(world, n);
    col = col + sun;
    let cnt = min(LF.counts.x, 64u);
    for (var i = 0u; i < cnt; i = i + 1u) {
        var t = light_term(LF.lights[i], world, n, v, diff, f0, rough);
        if (i == LF.pt_idx.x) { t = t * pt_shadow(world, n, LF.lights[i].pos.xyz); }
        col = col + t;
    }
    return col + emissive;
}
fn lobby_fog(c: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    // UE ExponentialHeightFog line integral (density and falloff already scaled by 0.001)
    let rel = world - LF.cam.xyz;
    let len = length(rel);
    let dens = LF.fog_a.x;
    let fall = LF.fog_a.y;
    let start = LF.fog_b.w;
    if (dens <= 0.0 || len < 1e-3) { return c; }
    let ex = min(start, len);
    let t0 = ex / len;
    let ray_len = (1.0 - t0) * len;
    let z0 = LF.cam.z + rel.z * t0;
    let dz = rel.z * (1.0 - t0);
    let origin_terms = dens * exp2(-fall * (z0 - LF.fog_a.z));
    let falloff = max(-127.0, fall * dz);
    var integral: f32;
    if (abs(falloff) > 1e-6) { integral = (1.0 - exp2(-falloff)) / falloff; } else { integral = 0.693147 - 0.5 * 0.480453 * falloff; }
    let optical = origin_terms * integral * ray_len;
    let trans = max(clamp(exp2(-optical), 0.0, 1.0), 1.0 - LF.fog_a.w);
    return c * trans + LF.fog_b.xyz * (1.0 - trans);
}
"#;

const SHADOW_WGSL: &str = r#"
struct SP { vp: mat4x4<f32> }
@group(0) @binding(0) var<uniform> sp: SP;
@group(0) @binding(1) var<uniform> prim: array<vec4<f32>, 16>;
@vertex fn vs(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {
    let w = p.x * prim[0].xyz + p.y * prim[1].xyz + p.z * prim[2].xyz + prim[3].xyz;
    return sp.vp * vec4<f32>(w, 1.0);
}
"#;

const CHAR_WGSL: &str = r#"
struct Obj { model: mat4x4<f32>, params: vec4<f32> }
@group(0) @binding(0) var<uniform> obj: Obj;
@group(0) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;
@group(0) @binding(2) var tex: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(1) @binding(0) var<uniform> view: mat4x4<f32>;
struct VIn { @location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) j: vec4<u32>, @location(4) w: vec4<f32> }
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) world: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32> }
fn skin(i: VIn) -> mat4x4<f32> {
    return bones[i.j.x] * i.w.x + bones[i.j.y] * i.w.y + bones[i.j.z] * i.w.z + bones[i.j.w] * i.w.w;
}
@vertex fn vs(i: VIn) -> VOut {
    let m = obj.model * skin(i);
    var o: VOut;
    let w = m * vec4<f32>(i.p, 1.0);
    o.world = w.xyz;
    o.n = (m * vec4<f32>(i.n, 0.0)).xyz;
    o.uv = i.uv;
    o.pos = view * vec4<f32>(w.xyz, 1.0);
    return o;
}
@fragment fn fs(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    if (c.a < 0.333 && obj.params.x > 0.5) { discard; }
    var n = normalize(i.n);
    if (obj.params.y > 0.5) { n = -n; }
    var col = lobby_shade(i.world, n, c.rgb, 0.0, 0.5, 0.65, 1.0, vec3<f32>(0.0), front);
    col = lobby_fog(col, i.world);
    return vec4<f32>(col, 1.0);
}
@vertex fn vs_shadow(i: VIn) -> @builtin(position) vec4<f32> {
    let m = obj.model * skin(i);
    return view * (m * vec4<f32>(i.p, 1.0));
}
"#;

/// The hero through the ORIGINAL compiled character base-pass pixel shaders (`fn-render/shaders/char_material_gen.wgsl`: head / hair / body permutation translated from the cooked
/// DXBC by fn-dxbc), evaluated per pixel exactly as `fn-render`'s `char_scene.wgsl` does; only the lighting tail is the lobby's own (`lobby_shade`). The constant buffer (cb3) is the cooked
/// uniform expression set evaluated with the material instance (`fn_pose::br_material`). The world of this pass is UE centimetres (as everywhere in the lobby).
const CHAR_ORIG_WGSL: &str = r#"
struct Obj { model: mat4x4<f32>, params: vec4<f32> }
@group(0) @binding(0) var<uniform> obj: Obj;
@group(0) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;
@group(0) @binding(2) var t_diffuse: texture_2d<f32>;
@group(0) @binding(3) var t_normal: texture_2d<f32>;
@group(0) @binding(4) var t_spec: texture_2d<f32>;
@group(0) @binding(5) var t_mask: texture_2d<f32>;
@group(0) @binding(6) var tex_samp: sampler;
struct CharMat { cb: array<vec4<f32>, 32>, perm_info: vec4<u32> }
@group(0) @binding(7) var<uniform> cmat: CharMat;
@group(0) @binding(8) var fx_tex7: texture_2d<f32>;
@group(0) @binding(9) var fx_samp7: sampler;
@group(1) @binding(0) var<uniform> view: mat4x4<f32>;
struct VIn { @location(0) p: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) j: vec4<u32>, @location(4) w: vec4<f32> }
struct VOut { @builtin(position) pos: vec4<f32>, @location(0) world: vec3<f32>, @location(1) n: vec3<f32>, @location(2) uv: vec2<f32> }
@vertex fn vs_orig(i: VIn) -> VOut {
    let sk = bones[i.j.x] * i.w.x + bones[i.j.y] * i.w.y + bones[i.j.z] * i.w.z + bones[i.j.w] * i.w.w;
    let m = obj.model * sk;
    var o: VOut;
    let w = m * vec4<f32>(i.p, 1.0);
    o.world = w.xyz;
    o.n = (m * vec4<f32>(i.n, 0.0)).xyz;
    o.uv = i.uv;
    o.pos = view * vec4<f32>(w.xyz, 1.0);
    return o;
}
// UE (cm, Z up) <-> the port's right-handed Y-up space the tangent frame is derived in (the swap is its own inverse)
fn swap_yz(r: vec3<f32>) -> vec3<f32> { return vec3<f32>(r.x, r.z, r.y); }
struct GBuf { emissive: vec3<f32>, normal_ue: vec3<f32>, metallic: f32, specular: f32, roughness: f32, base: vec3<f32>, ao: f32 }
fn char_run(perm: u32, v: array<vec4<f32>, 8>, tw: vec3<f32>, cam_ue: vec3<f32>, time: f32) -> GBuf {
    var o: array<vec4<f32>, 6>;
    var v7: array<vec4<f32>, 7>;
    for (var i = 0; i < 7; i = i + 1) { v7[i] = v[i]; }
    if (perm == 0u) {
        head_cb0[3] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        head_cb1[43] = vec4<f32>(tw, 1.0);
        head_cb1[62] = vec4<f32>(-cam_ue, 0.0);
        head_cb1[131] = vec4<f32>(0.0, 0.0, time, 0.0);
        for (var i = 0; i < 27; i = i + 1) { head_cb3[i] = cmat.cb[i]; }
        o = head_ps(v7);
    } else if (perm == 1u) {
        hair_cb0[3] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        hair_cb1[43] = vec4<f32>(tw, 1.0);
        hair_cb1[62] = vec4<f32>(-cam_ue, 0.0);
        hair_cb1[131] = vec4<f32>(0.0, 0.0, time, 0.0);
        for (var i = 0; i < 27; i = i + 1) { hair_cb3[i] = cmat.cb[i]; }
        o = hair_ps(v7);
    } else {
        body_cb0[3] = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        body_cb1[43] = vec4<f32>(tw, 1.0);
        body_cb1[62] = vec4<f32>(-cam_ue, 0.0);
        body_cb1[130] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        body_cb1[131] = vec4<f32>(0.0, 0.0, time, 0.0);
        body_cb2[9] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        for (var i = 0; i < 27; i = i + 1) { body_cb3[i] = cmat.cb[i]; }
        o = body_ps(v);
    }
    var g: GBuf;
    g.emissive = o[0].xyz;
    g.normal_ue = normalize(o[1].xyz * 2.0 - 1.0);
    g.metallic = o[2].x;
    g.specular = o[2].y;
    g.roughness = o[2].z;
    g.base = o[3].xyz;
    g.ao = o[3].w;
    return g;
}
@fragment fn fs_orig(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let p = swap_yz(i.world) * 0.01;
    let n0 = normalize(swap_yz(i.n));
    let dp1 = dpdx(p);
    let dp2 = -dpdy(p);
    let duv1 = dpdx(i.uv);
    let duv2 = -dpdy(i.uv);
    let dp2perp = cross(dp2, n0);
    let dp1perp = cross(n0, dp1);
    let t_r = dp2perp * duv1.x + dp1perp * duv2.x;
    let b_r = dp2perp * duv1.y + dp1perp * duv2.y;
    let n_u = normalize(swap_yz(n0));
    let t_u = normalize(swap_yz(t_r) - n_u * dot(n_u, swap_yz(t_r)));
    let w_sign = select(-1.0, 1.0, dot(cross(n_u, t_u), swap_yz(b_r)) >= 0.0);
    var v: array<vec4<f32>, 8>;
    v[0] = vec4<f32>(t_u, 0.0);
    v[1] = vec4<f32>(n_u, w_sign);
    v[2] = vec4<f32>(1.0);
    v[3] = vec4<f32>(i.uv, 0.0, 0.0);
    v[4] = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    v[6] = i.pos;
    v[7] = vec4<f32>(bitcast<f32>(select(0u, 1u, front)), 0.0, 0.0, 0.0);
    let g = char_run(cmat.perm_info.x, v, i.world - LF.cam.xyz, LF.cam.xyz, LF.cam.w);
    var col = lobby_shade(i.world, g.normal_ue, g.base, g.metallic, g.specular, g.roughness, g.ao, g.emissive, front);
    col = lobby_fog(col, i.world);
    return vec4<f32>(col, 1.0);
}
"#;

/// `CoreAnims_Male` key of what the lobby pawn holds (`FN_LOBBY_WEAPON` overrides it for comparisons). The lobby pawn holds nothing, so the native code never sets
/// `WeaponCoreAnim` and it keeps the AnimBP CDO default, `EFortWeaponCoreAnimation::PaperBlueprint` (read from the `Fortnite_M_Avg_Player_AnimBlueprint` default object): the hands-empty
/// set (pose `M_NoWep_Relaxed` + aim offset `M_NoWep_Relaxed_AO`). A key without a map entry (`"None"`) would leave the upper body on the bind pose (`upper_pre_slot` returns `Node::Ref`).
fn lobby_weapon() -> &'static str {
    match std::env::var("FN_LOBBY_WEAPON") { Ok(w) => Box::leak(w.into_boxed_str()), Err(_) => "PaperBlueprint" }
}

const DOWN_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let x = f32(i32(i & 1u) * 4 - 1); let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    var o: V; o.p = vec4<f32>(x, y, 0.0, 1.0); o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5); return o;
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> { return textureSampleLevel(src, samp, v.uv, 0.0); }
"#;

const POST_WGSL: &str = r#"
struct PostU {
    wb0: vec4<f32>, wb1: vec4<f32>, wb2: vec4<f32>, film_stw: vec4<f32>, film_clip: vec4<f32>,
    bloom: vec4<f32>, vignette: vec4<f32>, view: vec4<f32>,
    sat_shadows: vec4<f32>, sat_mid: vec4<f32>, sat_high: vec4<f32>, grade: vec4<f32>, lut: vec4<f32>,
    bloom_tints: array<vec4<f32>, 6>, bloom_sizes: array<vec4<f32>, 2>,
}
@group(0) @binding(0) var<uniform> pp: PostU;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var lin: sampler;
@group(0) @binding(3) var lut_tex: texture_2d<f32>;
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    let x = f32(i32(i & 1u) * 4 - 1); let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    var o: V; o.p = vec4<f32>(x, y, 0.0, 1.0); o.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5); return o;
}
fn to_srgb(c: vec3<f32>) -> vec3<f32> { return select(1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3<f32>(0.0031308)); }
fn bloom_size(i: u32) -> f32 { if (i < 4u) { return pp.bloom_sizes[0][i]; } return pp.bloom_sizes[1][i - 4u]; }
fn bloom_color(uv: vec2<f32>) -> vec3<f32> {
    let vp = pp.view.xy;
    let max_lod = floor(log2(max(vp.x, vp.y)));
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < 6u; i = i + 1u) {
        let lod = floor(clamp(log2(max(bloom_size(i) * 0.01 * vp.x * 0.5, 1.0)), 0.0, max_lod));
        let texel = 1.0 / vec2<f32>(textureDimensions(scene, i32(lod)));
        var acc = vec3<f32>(0.0);
        for (var y = -1; y <= 1; y = y + 1) { for (var x = -1; x <= 1; x = x + 1) {
            let w = select(1.0, 2.0, x == 0) * select(1.0, 2.0, y == 0);
            let c = textureSampleLevel(scene, lin, uv + vec2<f32>(f32(x), f32(y)) * texel * 1.5, lod).rgb;
            let l = dot(max(c, vec3<f32>(0.0)), vec3<f32>(0.3, 0.59, 0.11));
            acc = acc + c * clamp((l - pp.bloom.y) * 0.5, 0.0, 1.0) * w;
        } }
        sum = sum + acc / 16.0 * pp.bloom_tints[i].xyz;
    }
    return sum * pp.bloom.x;
}
fn film_curve(c0: vec3<f32>) -> vec3<f32> {
    let slope = pp.film_stw.x; let toe = pp.film_stw.y; let shoulder = pp.film_stw.z;
    let black = pp.film_clip.x; let white = pp.film_clip.y;
    let toe_scale = 1.0 + black - toe; let shoulder_scale = 1.0 + white - shoulder;
    let in_match = 0.18; let out_match = 0.18;
    var toe_match: f32;
    if (toe > 0.8) { toe_match = (1.0 - toe - out_match) / slope + log(in_match) / 2.302585093; }
    else { let bt = (out_match + black) / toe_scale - 1.0; toe_match = log(in_match) / 2.302585093 - 0.5 * log((1.0 + bt) / (1.0 - bt)) * (toe_scale / slope); }
    let straight_match = (1.0 - toe) / slope - toe_match;
    let shoulder_match = shoulder / slope - straight_match;
    let lc = log(max(c0, vec3<f32>(1e-10))) / 2.302585093;
    let straight = slope * (lc + straight_match);
    var toe_c = -black + 2.0 * toe_scale / (1.0 + exp(-2.0 * (slope / toe_scale) * (lc - toe_match)));
    var sh_c = (1.0 + white) - 2.0 * shoulder_scale / (1.0 + exp(2.0 * (slope / shoulder_scale) * (lc - shoulder_match)));
    toe_c = select(straight, toe_c, lc < vec3<f32>(toe_match));
    sh_c = select(straight, sh_c, lc > vec3<f32>(shoulder_match));
    var t = clamp((lc - toe_match) / (shoulder_match - toe_match), vec3<f32>(0.0), vec3<f32>(1.0));
    if (shoulder_match < toe_match) { t = 1.0 - t; }
    t = (3.0 - 2.0 * t) * t * t;
    return mix(toe_c, sh_c, t);
}
const SRGB_TO_AP0 = mat3x3<f32>(0.439701, 0.089792, 0.017544, 0.382978, 0.813423, 0.111544, 0.177335, 0.096762, 0.870704);
const AP0_TO_AP1 = mat3x3<f32>(1.451439, -0.076554, 0.008316, -0.236511, 1.176230, -0.006032, -0.214929, -0.099676, 0.997716);
const AP1_TO_SRGB = mat3x3<f32>(1.705052, -0.130257, -0.024003, -0.621791, 1.140803, -0.128969, -0.083258, -0.010549, 1.152972);
const SRGB_TO_AP1 = mat3x3<f32>(0.613191, 0.070207, 0.020619, 0.339512, 0.916336, 0.109567, 0.047366, 0.013450, 0.869607);
const AP1_LUMA = vec3<f32>(0.2722287168, 0.6740817658, 0.0536895174);
fn color_grade(c: vec3<f32>) -> vec3<f32> {
    let ap1 = SRGB_TO_AP1 * c;
    let luma = dot(ap1, AP1_LUMA);
    let xs = clamp(luma / pp.grade.x, 0.0, 1.0);
    let xh = clamp((luma - pp.grade.y) / (1.0 - pp.grade.y), 0.0, 1.0);
    let sw = 1.0 - xs * xs * (3.0 - 2.0 * xs);
    let hw = xh * xh * (3.0 - 2.0 * xh);
    let mw = 1.0 - sw - hw;
    let d = ap1 - vec3<f32>(luma);
    let shadows = max(vec3<f32>(luma) + d * pp.sat_shadows.xyz, vec3<f32>(0.0));
    let mids = max(vec3<f32>(luma) + d * pp.sat_mid.xyz, vec3<f32>(0.0));
    let highs = max(vec3<f32>(luma) + d * pp.sat_high.xyz, vec3<f32>(0.0));
    return AP1_TO_SRGB * (shadows * sw + mids * mw + highs * hw);
}
fn aces_red_modifier(a0: vec3<f32>) -> vec3<f32> {
    var a = a0;
    let minrgb = min(a.r, min(a.g, a.b)); let maxrgb = max(a.r, max(a.g, a.b));
    let sat = (max(maxrgb, 1e-10) - max(minrgb, 1e-10)) / max(maxrgb, 1e-2);
    var hue = 0.0;
    if (!(a.r == a.g && a.g == a.b)) { hue = degrees(atan2(1.7320508 * (a.g - a.b), 2.0 * a.r - a.g - a.b)); if (hue < 0.0) { hue = hue + 360.0; } }
    let centered = select(hue, hue - 360.0, hue > 180.0);
    let w = smoothstep(0.0, 1.0, 1.0 - abs(2.0 * centered / 135.0));
    a.r = a.r + w * w * sat * (0.03 - a.r) * (1.0 - 0.82);
    return a;
}
fn vignette_mask(uv: vec2<f32>) -> f32 {
    let a = pp.view.y / pp.view.x;
    let p = (uv * 2.0 - 1.0) * vec2<f32>(1.0, a) * (1.4142135624 / sqrt(1.0 + a * a)) * pp.vignette.x;
    let t = 1.0 / (1.0 + dot(p, p));
    return t * t;
}
fn lut_lookup(c: vec3<f32>) -> vec3<f32> {
    let cc = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    let b = cc.b * 15.0;
    let s0 = floor(b); let s1 = min(s0 + 1.0, 15.0); let f = b - s0;
    let y = (cc.g * 15.0 + 0.5) / 16.0;
    let x0 = (s0 * 16.0 + cc.r * 15.0 + 0.5) / 256.0;
    let x1 = (s1 * 16.0 + cc.r * 15.0 + 0.5) / 256.0;
    let a = textureSampleLevel(lut_tex, lin, vec2<f32>(x0, y), 0.0).rgb;
    let d = textureSampleLevel(lut_tex, lin, vec2<f32>(x1, y), 0.0).rgb;
    return mix(a, d, f);
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
    let uv = v.uv;
    var c = textureSampleLevel(scene, lin, uv, 0.0).rgb;
    if (pp.bloom.z > 0.5) { c = c + bloom_color(uv); }
    if (pp.vignette.y > 0.5) { c = c * vignette_mask(uv); }
    c = clamp(c, vec3<f32>(0.0026677086), vec3<f32>(43.707738));
    let wb = mat3x3<f32>(pp.wb0.xyz, pp.wb1.xyz, pp.wb2.xyz);
    c = color_grade(wb * c);
    var ap0 = SRGB_TO_AP0 * c;
    ap0 = aces_red_modifier(ap0);
    var ap1 = max(AP0_TO_AP1 * ap0, vec3<f32>(0.0));
    ap1 = mix(vec3<f32>(dot(ap1, AP1_LUMA)), ap1, 0.96);
    var toned = film_curve(ap1);
    toned = max(mix(vec3<f32>(dot(toned, AP1_LUMA)), toned, 0.93), vec3<f32>(0.0));
    c = max(AP1_TO_SRGB * toned, vec3<f32>(0.0));
    var enc = to_srgb(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)));
    if (pp.lut.y > 0.5) { enc = mix(enc, lut_lookup(enc), pp.lut.x); }
    return vec4<f32>(enc, 1.0);
}
"#;

fn patch_wgsl(raw: &str, lit: bool, fogged: bool, translucent_lit: bool) -> String {
    let mut s = raw.to_owned();
    if lit {
        if let Some(p) = s.find("struct FxLight") {
            let e = s[p..].find("var<uniform> fx_light: FxLight;\n").map(|q| p + q + "var<uniform> fx_light: FxLight;\n".len()).unwrap_or(p);
            s.replace_range(p..e, "");
        }
    }
    let head = "diagnostic(off, derivative_uniformity);\n";
    let at = s.find(head).map(|p| p + head.len()).unwrap_or(0);
    s.insert_str(at, PRELUDE);
    let tail_at = if lit { s.rfind("    let n_ue = normalize(o[1].xyz * 2.0 - 1.0);") } else { s.rfind("    return o[0];\n}") };
    if let Some(t) = tail_at {
        s.truncate(t);
        if lit && !translucent_lit {
            s.push_str("    let n_ue = normalize(o[1].xyz * 2.0 - 1.0);\n    let world = lobby_world(in.frag);\n    var col = lobby_shade(world, n_ue, o[3].xyz, o[2].x, o[2].y, o[2].z, o[3].w, o[0].xyz, in.front);\n    col = lobby_fog(col, world);\n    return vec4<f32>(col, 1.0);\n}\n");
        } else if fogged {
            s.push_str("    let world = lobby_world(in.frag);\n    return vec4<f32>(lobby_fog(o[0].xyz, world), o[0].w);\n}\n");
        } else {
            s.push_str("    return o[0];\n}\n");
        }
    }
    s
}

// ---------------------------------------------------------------------------------------------------------------------------------
// GPU construction
// ---------------------------------------------------------------------------------------------------------------------------------
fn buffer(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size.max(16).div_ceil(16) * 16, usage, mapped_at_creation: false })
}

fn upload_texture(device: &wgpu::Device, queue: &wgpu::Queue, label: &str, t: &TexAsset, srgb: bool) -> (wgpu::Texture, wgpu::TextureView) {
    let (w, h, _) = &t.levels[0];
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label), size: wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 }, mip_level_count: t.levels.len() as u32, sample_count: 1,
        dimension: wgpu::TextureDimension::D2, format: if srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[],
    });
    for (i, (lw, lh, px)) in t.levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex, mip_level: i as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            px, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * lw), rows_per_image: Some(*lh) }, wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 });
    }
    let view = tex.create_view(&Default::default());
    (tex, view)
}

fn sampler_for(device: &wgpu::Device, filter: &str, address: &str, source: i32) -> wgpu::Sampler {
    let a = match source { 1 => wgpu::AddressMode::Repeat, 2 => wgpu::AddressMode::ClampToEdge, _ => match address { "TA_Clamp" => wgpu::AddressMode::ClampToEdge, "TA_Mirror" => wgpu::AddressMode::MirrorRepeat, _ => wgpu::AddressMode::Repeat } };
    let (mag, min, mip, aniso) = match filter {
        "TF_Nearest" => (wgpu::FilterMode::Nearest, wgpu::FilterMode::Nearest, wgpu::MipmapFilterMode::Nearest, 1),
        "TF_Bilinear" => (wgpu::FilterMode::Linear, wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Nearest, 1),
        "TF_Trilinear" => (wgpu::FilterMode::Linear, wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear, 1),
        _ => (wgpu::FilterMode::Linear, wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear, 8),
    };
    device.create_sampler(&wgpu::SamplerDescriptor { label: Some("lobby sampler"), address_mode_u: a, address_mode_v: a, address_mode_w: a, mag_filter: mag, min_filter: min, mipmap_filter: mip, anisotropy_clamp: aniso, ..Default::default() })
}

/// Interleaved FLocalVertexFactory stream (fn-fx layout): position f32x3, TangentX / TangentZ unorm8x4 as stored, colour BGRA, UV pairs as float4.
fn local_vertices(lod: &fn_mesh::StaticLod, pairs: usize) -> Vec<u8> {
    let stride = 24 + 16 * pairs;
    let n = lod.positions.len();
    let mut out = vec![0u8; n * stride];
    for i in 0..n {
        let o = i * stride;
        for (k, f) in lod.positions[i].iter().enumerate() { out[o + 4 * k..o + 4 * k + 4].copy_from_slice(&f.to_le_bytes()); }
        out[o + 12..o + 16].copy_from_slice(&lod.tangent_x[i]);
        out[o + 16..o + 20].copy_from_slice(&lod.tangent_z[i]);
        out[o + 20..o + 24].copy_from_slice(&lod.colors.get(i).copied().unwrap_or([255; 4]));
        for u in 0..lod.num_tex_coords.min(pairs * 2) {
            let uv = lod.uvs.get(u).and_then(|s| s.get(i)).copied().unwrap_or([0.; 2]);
            let at = o + 24 + 16 * (u / 2) + 8 * (u % 2);
            out[at..at + 4].copy_from_slice(&uv[0].to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&uv[1].to_le_bytes());
        }
    }
    out
}

fn vertex_attrs(pairs: usize) -> Vec<wgpu::VertexAttribute> {
    let mut a = vec![
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4, offset: 16, shader_location: 2 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Unorm8x4Bgra, offset: 20, shader_location: 3 },
    ];
    for p in 0..pairs { a.push(wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 24 + 16 * p as u64, shader_location: 4 + p as u32 }); }
    a
}

struct PreMat { mat: Arc<fn_fx::material::Material>, asm: fn_fx::program::Assembled, wgsl: String, blend: Blend, lit: bool, two_sided: bool, max_attr: u32, buf: usize }

fn blend_of(flags: &fn_fx::material::MaterialFlags) -> Result<Blend, String> {
    Ok(match flags.blend.as_str() { "BLEND_Opaque" | "BLEND_Masked" => Blend::Opaque, "BLEND_Translucent" => Blend::Translucent, "BLEND_Additive" => Blend::Additive, "BLEND_Modulate" => Blend::Modulate, o => return Err(format!("blend mode {o}")) })
}

fn prepare_material(mat: &Arc<fn_fx::material::Material>) -> Result<(fn_fx::program::Assembled, String, Blend, bool), String> {
    use fn_dxbc::Opcode;
    const VF: &str = "FLocalVertexFactory";
    let lit = mat.flags.shading != "MSM_Unlit";
    let vs = mat.cooked.find(VF, "TBasePassVSFNoLightMapPolicy").ok_or("no FLocalVertexFactory / TBasePassVSFNoLightMapPolicy program")?;
    let ps = mat.cooked.find(VF, "TBasePassPSFNoLightMapPolicy").or_else(|| mat.cooked.find(VF, "TBasePassPSFNoLightMapPolicySkylight")).ok_or("no TBasePassPSFNoLightMapPolicy program")?;
    let clip = if mat.flags.blend == "BLEND_Masked" && !ps.shader.body().any(|i| i.op == Opcode::Discard) { Some(mat.cooked.find(VF, "FDepthOnlyPS").ok_or("masked material without a depth-only pixel shader")?) } else { None };
    let asm = fn_fx::program::assemble(vs, ps, lit, clip)?;
    let blend = blend_of(&mat.flags)?;
    let opaque = blend == Blend::Opaque;
    let wgsl = patch_wgsl(&asm.wgsl, lit, opaque, lit && !opaque);
    if let Ok(d) = std::env::var("FN_LOBBY_DUMP_WGSL") { if let Some(name) = mat.base_path.rsplit('/').next().filter(|n| n.contains(d.as_str())) { eprintln!("DUMP {} cb={:?}", mat.path, mat.packed(0.0).cb); let _ = std::fs::write(format!("{name}.wgsl"), &wgsl); } }
    Ok((asm, wgsl, blend, lit))
}

impl LobbyScene {
    /// Loads everything from the install (CPU + GPU). `output_format` must be a NON-sRGB colour format (the post pass writes sRGB-encoded values).
    pub fn load(install: &Path, device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat) -> Result<LobbyScene, String> {
        let assets = LobbyAssets::load(install, None)?;
        Self::from_assets(assets, device, queue, output_format)
    }

    pub fn from_assets(assets: LobbyAssets, device: &wgpu::Device, queue: &wgpu::Queue, output_format: wgpu::TextureFormat) -> Result<LobbyScene, String> {
        let (device, queue) = (device.clone(), queue.clone());
        let mut report = assets.report.clone();
        let data = assets.data;
        let cam = data.camera.as_ref().ok_or("scene has no camera")?;
        let (camera, fov) = (cam.xf, cam.fov_deg);

        // ---- textures / samplers ----
        let mut tex_index: HashMap<String, usize> = HashMap::new();
        let mut textures: Vec<(wgpu::Texture, wgpu::TextureView)> = vec![];
        for (path, t) in &assets.textures {
            tex_index.insert(path.clone(), textures.len());
            textures.push(upload_texture(&device, &queue, path, t, t.srgb));
        }
        let mut samplers: Vec<wgpu::Sampler> = vec![];
        let mut sampler_index: HashMap<(String, String, i32), usize> = HashMap::new();

        // ---- shared buffers ----
        let frame_buf = buffer(&device, "lobby frame", std::mem::size_of::<FrameU>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let view_buf = buffer(&device, "lobby view", 178 * 16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let fade_buf = buffer(&device, "lobby fade", 16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        queue.write_buffer(&fade_buf, 0, bytemuck::cast_slice(&[0f32, 1., 0., 0.]));
        let zero_buf = buffer(&device, "lobby zero", 65536, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        // `MaterialCollection0` of the non-lobby views (MPC defaults); the Lobby scene has none (empty) and keeps binding the zero buffer
        let coll_buf = buffer(&device, "lobby material collection", 4096, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        if !data.mpc.is_empty() { queue.write_buffer(&coll_buf, 0, bytemuck::cast_slice(&data.mpc.iter().flatten().copied().collect::<Vec<f32>>())); }
        let coll_buf_opt = if data.mpc.is_empty() { None } else { Some(&coll_buf) };
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("lobby shadow"), size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let shadow_view = shadow_tex.create_view(&Default::default());
        let pt_tex = device.create_texture(&wgpu::TextureDescriptor { label: Some("lobby point shadow"), size: wgpu::Extent3d { width: PT_SHADOW_SIZE, height: PT_SHADOW_SIZE, depth_or_array_layers: 6 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let pt_array_view = pt_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
        let pt_face_views: Vec<wgpu::TextureView> = (0..6).map(|i| pt_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: i, array_layer_count: Some(1), ..Default::default() })).collect();
        let shadow_cmp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("lobby shadow cmp"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, compare: Some(wgpu::CompareFunction::LessEqual), address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let group3_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby g3"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison), count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
        ] });
        let group3 = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby g3"), layout: &group3_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: frame_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_cmp) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&pt_array_view) },
        ] });

        // ---- materials: assemble every program pair once ----
        let mut pre: HashMap<String, PreMat> = HashMap::new();
        let mut mat_bufs: Vec<wgpu::Buffer> = vec![];
        for (path, m) in &assets.materials {
            match prepare_material(m) {
                Ok((asm, wgsl, blend, lit)) => {
                    let max_attr = asm.vs_inputs.iter().map(|v| v.attribute).max().unwrap_or(0);
                    let packed = m.packed(0.0);
                    let declared = asm.resources.iter().filter(|r| r.kind == fn_fx::program::ResKind::Uniform).map(|r| r.size as usize).max().unwrap_or(0);
                    let b = buffer(&device, "lobby material cb", (packed.registers.max(declared).max(1) * 16) as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
                    queue.write_buffer(&b, 0, bytemuck::cast_slice(&packed.cb.iter().flatten().copied().collect::<Vec<f32>>()));
                    mat_bufs.push(b);
                    pre.insert(path.clone(), PreMat { mat: m.clone(), asm, wgsl, blend, lit, two_sided: m.flags.two_sided, max_attr, buf: mat_bufs.len() - 1 });
                }
                Err(e) => report.substituted.push((path.clone(), e)),
            }
        }

        // ---- meshes: needed UV pairs from the materials used on them ----
        let mut mesh_paths: Vec<String> = vec![];
        let mut mesh_index: HashMap<String, usize> = HashMap::new();
        let mut mesh_pairs: Vec<usize> = vec![];
        let visible: Vec<&world::StaticInst> = data.statics.iter().filter(|s| s.visible && assets.meshes.contains_key(&s.mesh)).collect();
        let occupied: Vec<String> = data.placements.iter().filter(|p| p.player_on_pad).map(|p| p.actor.clone()).collect();
        let mat_of = |s: &world::StaticInst, slot: usize| -> Option<String> {
            let m = &assets.meshes[&s.mesh];
            let p = s.material_overrides.get(slot).cloned().flatten().or_else(|| m.default_materials.get(slot).cloned().flatten());
            p.map(|p| if p.ends_with("M_Lobby_Pad.M_Lobby_Pad") && occupied.contains(&s.actor) { format!("{p}#occupied") } else { p })
        };
        for s in &visible {
            let idx = *mesh_index.entry(s.mesh.clone()).or_insert_with(|| { mesh_paths.push(s.mesh.clone()); mesh_pairs.push(1); mesh_paths.len() - 1 });
            let m = &assets.meshes[&s.mesh];
            mesh_pairs[idx] = mesh_pairs[idx].max(m.lod.num_tex_coords.div_ceil(2));
            for sec in &m.lod.sections {
                if let Some(p) = mat_of(s, sec.material_index.max(0) as usize) { if let Some(pm) = pre.get(&p) { if pm.max_attr >= 4 { mesh_pairs[idx] = mesh_pairs[idx].max(pm.max_attr as usize - 3); } } }
            }
        }
        let mut meshes: Vec<MeshGpu> = vec![];
        for (i, p) in mesh_paths.iter().enumerate() {
            let m = &assets.meshes[p];
            let pairs = mesh_pairs[i].min(4);
            let vb = buffer(&device, p, (m.lod.positions.len() * (24 + 16 * pairs)) as u64, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST);
            queue.write_buffer(&vb, 0, &local_vertices(&m.lod, pairs));
            let ib = buffer(&device, p, (m.lod.indices.len() * 4) as u64, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST);
            queue.write_buffer(&ib, 0, bytemuck::cast_slice(&m.lod.indices));
            let sections = m.lod.sections.iter().map(|s| (s.material_index.max(0) as usize, s.first_index, s.num_triangles * 3)).collect();
            meshes.push(MeshGpu { vb, ib, pairs, sections, shadow_ok: true });
        }

        // ---- draw list + primitive buffer ----
        let mut draws: Vec<DrawItem> = vec![];
        let mut prims: Vec<u8> = vec![];
        let mut pipe_key: HashMap<(String, usize, bool), usize> = HashMap::new();
        let mut pipe_defs: Vec<(String, usize, bool)> = vec![];
        for s in &visible {
            let mi = mesh_index[&s.mesh];
            let m = &assets.meshes[&s.mesh];
            let l2w = s.xf.matrix();
            let prim = fn_matshader::frame::primitive_buffer(&l2w, (m.center.to_array(), m.radius));
            let offset = prims.len() as u64;
            for v in &prim { for f in v { prims.extend_from_slice(&f.to_le_bytes()); } }
            prims.resize((offset + PRIM_STRIDE) as usize, 0);
            let world_center = l2w.transform_point3(m.center);
            for (slot, first, count) in meshes[mi].sections.clone() {
                let Some(path) = mat_of(s, slot) else { continue };
                let Some(pm) = pre.get(&path) else { continue };
                // debug knob: FN_LOBBY_HIDE=token,token hides draws whose mesh or material path contains a token
                if std::env::var("FN_LOBBY_HIDE").is_ok_and(|v| v.split(',').any(|t| !t.is_empty() && (path.contains(t) || s.mesh.contains(t)))) { continue; }
                let key = (path.clone(), meshes[mi].pairs, data.selection.is_none() && l2w.determinant() < 0.0); // non-lobby views only (the Lobby render stays untouched): a mirrored instance (negative scale) flips the winding, as in UE
                let pk = *pipe_key.entry(key.clone()).or_insert_with(|| { pipe_defs.push(key); pipe_defs.len() - 1 });
                let translucent = pm.blend != Blend::Opaque;
                let foliage = pm.mat.flags.shading == "MSM_Subsurface";
                draws.push(DrawItem { mesh: mi, material_key: pk, first, count, prim: offset, pos: world_center, sort_priority: s.translucency_sort_priority, shadow: s.cast_shadow && !translucent && !foliage, translucent });
            }
        }
        let prim_buf = buffer(&device, "lobby primitives", prims.len().max(PRIM_STRIDE as usize) as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        queue.write_buffer(&prim_buf, 0, &prims);

        // ---- pipelines ----
        let mut mat_pipes: Vec<MatPipe> = vec![];
        let mut pipes_failed: HashMap<String, String> = HashMap::new();
        let mut pipe_ok: Vec<Option<usize>> = vec![None; pipe_defs.len()];
        for (pk, (path, pairs, mirror)) in pipe_defs.iter().enumerate() {
            let pm = &pre[path];
            match build_mat_pipe(&device, pm, *pairs, *mirror, coll_buf_opt, &group3_layout, &tex_index, &textures, &mut samplers, &mut sampler_index, &assets.textures, &view_buf, &fade_buf, &zero_buf, &prim_buf, &mat_bufs) {
                Ok(mp) => { pipe_ok[pk] = Some(mat_pipes.len()); mat_pipes.push(mp); }
                Err(e) => { pipes_failed.insert(path.clone(), e); }
            }
        }
        for d in draws.iter_mut() { d.material_key = pipe_ok[d.material_key].map(|v| v + 1).unwrap_or(0); }
        draws.retain(|d| d.material_key != 0);
        for d in draws.iter_mut() { d.material_key -= 1; }
        for (p, e) in &pipes_failed { report.substituted.push((p.clone(), format!("pipeline: {e}"))); }
        for (p, pm) in &pre { if !pipes_failed.contains_key(p) { report.original_materials.push(format!("{p} [{}{}]", if pm.lit { "lit" } else { "unlit" }, match pm.blend { Blend::Opaque => "", Blend::Translucent => ", translucent", Blend::Additive => ", additive", Blend::Modulate => ", modulate" })); } }
        report.original_materials.sort();

        // ---- shadow pipeline(s) ----
        let shadow_vp_buf = buffer(&device, "lobby shadow vp", 64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby shadow bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: true, min_binding_size: None }, count: None },
        ] });
        let shadow_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby shadow bg"), layout: &shadow_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: shadow_vp_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &prim_buf, offset: 0, size: wgpu::BufferSize::new(256) }) },
        ] });
        let pt_vp_bufs: Vec<wgpu::Buffer> = (0..6).map(|_| buffer(&device, "lobby point shadow vp", 64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST)).collect();
        let pt_bgs: Vec<wgpu::BindGroup> = pt_vp_bufs.iter().map(|b| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby point shadow bg"), layout: &shadow_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: b.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &prim_buf, offset: 0, size: wgpu::BufferSize::new(256) }) },
        ] })).collect();
        let shadow_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("lobby shadow"), source: wgpu::ShaderSource::Wgsl(SHADOW_WGSL.into()) });
        let shadow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("lobby shadow pl"), bind_group_layouts: &[Some(&shadow_layout)], immediate_size: 0 });
        // the shadow pipeline reads only the position; one pipeline per vertex stride (UV pair count)
        let shadow_pipes: Vec<wgpu::RenderPipeline> = (1..=4usize).map(|pairs| {
            let attrs = [wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }];
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lobby shadow"), layout: Some(&shadow_pl),
                vertex: wgpu::VertexState { module: &shadow_module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: (24 + 16 * pairs) as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })] },
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } }),
                multisample: Default::default(), fragment: None, multiview_mask: None, cache: None,
            })
        }).collect();

        // ---- post ----
        let post_samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("lobby post"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby post bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
        ] });
        let down_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby down bgl"), entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ] });
        let fs_pipeline = |label: &str, src: &str, layout: &wgpu::BindGroupLayout, fmt: wgpu::TextureFormat| -> wgpu::RenderPipeline {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(label), source: wgpu::ShaderSource::Wgsl(src.into()) });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(label), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label), layout: Some(&pl),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: fmt, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
                multiview_mask: None, cache: None,
            })
        };
        let post_pipe = fs_pipeline("lobby post", POST_WGSL, &post_layout, output_format);
        let down_pipe = fs_pipeline("lobby downsample", DOWN_WGSL, &down_layout, HDR);
        let post_buf = buffer(&device, "lobby post u", std::mem::size_of::<PostU>() as u64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let lut_view = match &assets.lut {
            Some(t) => upload_texture(&device, &queue, "lobby LUT", &TexAsset { levels: vec![t.levels[0].clone()], srgb: false, filter: String::new(), address: String::new() }, false).1,
            None => upload_texture(&device, &queue, "lobby LUT none", &TexAsset { levels: vec![(1, 1, vec![128, 128, 128, 255])], srgb: false, filter: String::new(), address: String::new() }, false).1,
        };

        // ---- lights ----
        let mut lights: Vec<GpuLight> = vec![];
        let mut sun: Option<(Vec3, [f32; 3])> = None;
        let mut pt_best: Option<(usize, f32, Vec3, f32)> = None; // the strongest shadow-casting inverse-squared point light (one cube shadow map)
        let off = std::env::var("FN_LOBBY_NOLIGHT").unwrap_or_default(); // debug: comma separated actor / component name fragments to leave out
        for l in data.lights.iter().filter(|l| l.visible && l.affects_world && !off.split(',').any(|t| !t.is_empty() && (l.actor.contains(t) || l.component.contains(t)))) {
            let c = l.linear_color();
            match l.kind {
                LightKind::Directional => {
                    if sun.is_none() { let k = l.intensity; sun = Some((-l.xf.forward(), [c[0] * k, c[1] * k, c[2] * k])); }
                }
                LightKind::Point | LightKind::Spot => {
                    if lights.len() >= MAX_LIGHTS { continue; }
                    // Inverse-squared lights: colour x Intensity x 16 (exe: every consumer of the proxy's parameters multiplies the colour by 16.0 when the proxy is inverse
                    // squared, e.g. RVA 0x181e2bf / 0x19989f5 / 0x18274ce / 0x19dbad5; see SCENE.md); legacy lights (`bUseInverseSquaredFalloff` false): x1, pow falloff.
                    let k = if l.inverse_squared { l.intensity * 16.0 } else { l.intensity };
                    let (cos_outer, inv_range) = if l.kind == LightKind::Spot {
                        let (co, ci) = (l.outer_cone_deg.to_radians().cos(), l.inner_cone_deg.to_radians().cos());
                        (co, 1.0 / (ci - co).max(1e-4))
                    } else { (0., 0.) };
                    let dir = l.xf.forward();
                    if l.kind == LightKind::Point && l.cast_shadows && l.inverse_squared {
                        let w = k * c[0].max(c[1]).max(c[2]) * l.attenuation_radius * l.attenuation_radius;
                        if pt_best.map_or(true, |b| w > b.1) { pt_best = Some((lights.len(), w, l.xf.t, l.attenuation_radius)); }
                    }
                    lights.push(GpuLight { pos: [l.xf.t.x, l.xf.t.y, l.xf.t.z, 1.0 / l.attenuation_radius.max(1.0)], color: [c[0] * k, c[1] * k, c[2] * k, if l.kind == LightKind::Spot { 2.0 } else { 1.0 }], dir: [dir.x, dir.y, dir.z, cos_outer], cone: [inv_range, if l.inverse_squared { 0. } else { 1. }, l.falloff_exponent, 0.] });
                }
                LightKind::Sky => {}
            }
        }
        let fog = data.fogs.iter().find(|f| f.visible).cloned();
        // cube shadow matrices of that light (UE left handed, 90 degree faces +X -X +Y -Y +Z -Z)
        let mut pt_vps = [Mat4::IDENTITY; 6];
        let pt_idx = pt_best.map_or(u32::MAX, |b| b.0 as u32);
        report.lines.push(format!("point light cube shadow: {:?}", pt_best));
        if let Some((_, _, pos, radius)) = pt_best {
            let dirs = [(Vec3::X, Vec3::Z), (-Vec3::X, Vec3::Z), (Vec3::Y, Vec3::Z), (-Vec3::Y, Vec3::Z), (Vec3::Z, Vec3::X), (-Vec3::Z, Vec3::X)];
            let proj = Mat4::perspective_lh(std::f32::consts::FRAC_PI_2, 1.0, 5.0, radius.max(100.0));
            for (i, (d, up)) in dirs.iter().enumerate() { pt_vps[i] = proj * Mat4::look_to_lh(pos, *d, *up); queue.write_buffer(&pt_vp_bufs[i], 0, bytemuck::cast_slice(&pt_vps[i].to_cols_array())); }
        }

        // ---- character ----
        let cview_buf = buffer(&device, "lobby cam vp", 64, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
        let char_result = build_character(&device, &queue, assets.character, &data, &group3_layout, &cview_buf, &shadow_vp_buf);
        let (char_pipe, char_shadow_pipe, char_gpu, char_xf, char_asset) = match char_result {
            Ok((p, sp, g, xf, a)) => (Some(p), Some(sp), Some(g), Some(xf), Some(a)),
            Err((e, a)) => { report.lines.push(format!("character not drawn: {e}")); (None, None, None, None, a) }
        };
        report.lines.push(format!("{} draws ({} instances), {} mesh buffers, {} material pipelines, {} lights + sun, sky SH from {:?}", draws.len(), visible.len(), meshes.len(), mat_pipes.len(), lights.len(), assets.sky.source));
        let sh = assets.sky;
        let _ = meshes.iter().map(|m| m.shadow_ok).count();
        let shadow_store = shadow_pipes;
        let mut scene = LobbyScene {
            assets_report: report.clone(), data, device, queue, out_format: output_format,
            frame_buf, view_buf, prim_buf, fade_buf, zero_buf, group3_layout, group3, shadow_view, shadow_pipes: shadow_store, cview_buf, shadow_bg, shadow_vp_buf, pt_face_views, pt_bgs, pt_vps, pt_idx,
            meshes, mat_pipes, mat_bufs, textures, samplers, draws,
            char_pipe, char_shadow_pipe, char_asset, char_gpu, char_xf, char_pose: fn_pose::CharacterPose::default(), char_t0: 0.0, last_time: 0.0, anim: None,
            post_pipe, down_pipe, post_buf, lut_view, post_samp, post_layout, down_layout, targets: None, camera, fov, sun, sh, fog, lights, report,
        };
        scene.post_uniform_dirty();
        Ok(scene)
    }
}

fn bgl_entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry { wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None } }

#[allow(clippy::too_many_arguments)]
fn build_mat_pipe(device: &wgpu::Device, pm: &PreMat, pairs: usize, mirror: bool, coll_buf: Option<&wgpu::Buffer>, group3: &wgpu::BindGroupLayout, tex_index: &HashMap<String, usize>, textures_gpu: &[(wgpu::Texture, wgpu::TextureView)],
    samplers: &mut Vec<wgpu::Sampler>, sampler_index: &mut HashMap<(String, String, i32), usize>, textures: &HashMap<String, TexAsset>,
    view_buf: &wgpu::Buffer, fade_buf: &wgpu::Buffer, zero_buf: &wgpu::Buffer, prim_buf: &wgpu::Buffer, mat_bufs: &[wgpu::Buffer]) -> Result<MatPipe, String> {
    use fn_fx::program::{ResKind, Stage};
    const VF: &str = "FLocalVertexFactory";
    let mat = &pm.mat;
    let vs = mat.cooked.find(VF, "TBasePassVSFNoLightMapPolicy").unwrap();
    let ps = mat.cooked.find(VF, "TBasePassPSFNoLightMapPolicy").or_else(|| mat.cooked.find(VF, "TBasePassPSFNoLightMapPolicySkylight")).unwrap();
    let clip = if mat.flags.blend == "BLEND_Masked" { mat.cooked.find(VF, "FDepthOnlyPS") } else { None };
    let mut layout_entries: [Vec<wgpu::BindGroupLayoutEntry>; 3] = [vec![], vec![], vec![]];
    let mut group_entries: Vec<Vec<GroupEntry>> = vec![vec![], vec![], vec![]];
    let mut uses_depth = false;
    let mut dyn_count = 0;
    let dummy3d = device.create_texture(&wgpu::TextureDescriptor { label: Some("dummy 3d"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D3, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
    let dummy_cube = device.create_texture(&wgpu::TextureDescriptor { label: Some("dummy cube"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 6 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
    for r in &pm.asm.resources {
        let prog = match r.stage { Stage::Vertex => vs, Stage::Pixel => ps, Stage::ClipPixel => clip.ok_or("clip program missing")? };
        let vis = if r.stage == Stage::Vertex { wgpu::ShaderStages::VERTEX } else { wgpu::ShaderStages::FRAGMENT };
        let g = r.group as usize;
        if g > 2 { return Err(format!("resource group {g}")); }
        match r.kind {
            ResKind::Uniform => {
                let name = prog.uniform_buffer_names.as_ref().and_then(|n| n.get(r.slot as usize)).cloned().unwrap_or_default();
                let is_prim = name == "Primitive";
                layout_entries[g].push(bgl_entry(r.binding, vis, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: is_prim, min_binding_size: None }));
                if is_prim {
                    dyn_count += 1;
                    group_entries[g].push(GroupEntry::Prim { binding: r.binding, size: (r.size as u64 * 16).max(256).min(PRIM_STRIDE), buf: prim_buf.clone() });
                } else {
                    let buf = match name.as_str() { "View" => view_buf, "Material" => &mat_bufs[pm.buf], "PrimitiveFade" => fade_buf, "MaterialCollection0" => coll_buf.unwrap_or(zero_buf), _ => zero_buf };
                    group_entries[g].push(GroupEntry::Buf { binding: r.binding, buf: buf.clone() });
                }
            }
            ResKind::Tex2D => {
                let b = prog.bindings.iter().find(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Texture && b.binding.register as u32 == r.slot);
                match b {
                    Some(b) => {
                        let k = (b.binding.member / 2) as usize;
                        let (path, _) = mat.texture_slot(k).ok_or_else(|| format!("texture slot {k} unresolved"))?;
                        let ti = *tex_index.get(&path).ok_or_else(|| format!("texture {path} not loaded"))?;
                        layout_entries[g].push(bgl_entry(r.binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }));
                        group_entries[g].push(GroupEntry::Tex { binding: r.binding, view: textures_gpu[ti].1.clone() });
                    }
                    None => {
                        if r.stage == Stage::Vertex { return Err(format!("vertex shader texture t{} is not a material texture", r.slot)); }
                        uses_depth = true;
                        layout_entries[g].push(bgl_entry(r.binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }));
                        group_entries[g].push(GroupEntry::Depth { binding: r.binding });
                    }
                }
            }
            ResKind::StorageBuf => {
                layout_entries[g].push(bgl_entry(r.binding, vis, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }));
                group_entries[g].push(GroupEntry::Buf { binding: r.binding, buf: zero_buf.clone() });
            }
            ResKind::Tex3D | ResKind::Cube => {
                let (dim, tex) = if r.kind == ResKind::Cube { (wgpu::TextureViewDimension::Cube, &dummy_cube) } else { (wgpu::TextureViewDimension::D3, &dummy3d) };
                layout_entries[g].push(bgl_entry(r.binding, vis, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: dim, multisampled: false }));
                group_entries[g].push(GroupEntry::Tex { binding: r.binding, view: tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(dim), ..Default::default() }) });
            }
            ResKind::Sampler => {
                let b = prog.bindings.iter().find(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Sampler && b.binding.register as u32 == r.slot);
                match b {
                    Some(b) => {
                        let (filter, address, src) = match fn_matshader::textures::sampler_member(b.binding.member as usize, mat.uniform.textures2d.len())? {
                            fn_matshader::textures::SamplerMember::Asset(k) => {
                                let (path, src) = mat.texture_slot(k).ok_or_else(|| format!("sampler of texture {k} unresolved"))?;
                                let t = textures.get(&path).ok_or_else(|| format!("texture {path} not loaded"))?;
                                (t.filter.clone(), t.address.clone(), src)
                            }
                            fn_matshader::textures::SamplerMember::WrapWorld => ("TF_Default".to_owned(), "TA_Wrap".to_owned(), 1),
                            fn_matshader::textures::SamplerMember::ClampWorld => ("TF_Default".to_owned(), "TA_Clamp".to_owned(), 2),
                        };
                        let key = (filter.clone(), address.clone(), src);
                        let si = *sampler_index.entry(key).or_insert_with(|| { samplers.push(sampler_for(device, &filter, &address, src)); samplers.len() - 1 });
                        layout_entries[g].push(bgl_entry(r.binding, vis, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)));
                        group_entries[g].push(GroupEntry::Samp { binding: r.binding, s: samplers[si].clone() });
                    }
                    None => {
                        let depth = r.stage != Stage::Vertex && !prog.bindings.iter().any(|b| b.kind == fn_dxbc::unreal_packet::ResourceKind::Texture && b.binding.register as u32 == r.slot) && prog.shader.decls.resources.iter().any(|d| d.slot == r.slot && d.dim == 3);
                        let s = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("lobby clamp"), address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, mag_filter: if depth { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear }, min_filter: if depth { wgpu::FilterMode::Nearest } else { wgpu::FilterMode::Linear }, ..Default::default() });
                        layout_entries[g].push(bgl_entry(r.binding, vis, wgpu::BindingType::Sampler(if depth { wgpu::SamplerBindingType::NonFiltering } else { wgpu::SamplerBindingType::Filtering })));
                        group_entries[g].push(GroupEntry::Samp { binding: r.binding, s });
                    }
                }
            }
        }
    }
    let layouts: Vec<wgpu::BindGroupLayout> = layout_entries.iter().map(|e| device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby mat bgl"), entries: e })).collect();
    let mut refs: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
    refs.push(Some(group3));
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("lobby mat pl"), bind_group_layouts: &refs, immediate_size: 0 });
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("lobby original material"), source: wgpu::ShaderSource::Wgsl(pm.wgsl.as_str().into()) });
    // vertex stream: every attribute the vertex program reads must exist
    for v in &pm.asm.vs_inputs { if v.attribute >= 4 + pairs as u32 { return Err(format!("vertex program reads ATTRIBUTE{} but the stream has {pairs} UV pair(s)", v.attribute)); } }
    let attrs = vertex_attrs(pairs);
    let blend = match pm.blend {
        Blend::Opaque => None,
        Blend::Translucent => Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add } }),
        Blend::Additive => Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } }),
        Blend::Modulate => Some(wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add } }),
    };
    let opaque = pm.blend == Blend::Opaque;
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lobby original material"), layout: Some(&pl),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[Some(wgpu::VertexBufferLayout { array_stride: (24 + 16 * pairs) as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs })] },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: if pm.two_sided { None } else { Some(wgpu::Face::Back) }, front_face: if mirror { wgpu::FrontFace::Cw } else { wgpu::FrontFace::Ccw }, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(opaque), depth_compare: Some(if mat.flags.disable_depth_test { wgpu::CompareFunction::Always } else { wgpu::CompareFunction::LessEqual }), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_main"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: HDR, blend, write_mask: wgpu::ColorWrites::ALL })] }),
        multiview_mask: None, cache: None,
    });
    if let Some(e) = pollster::block_on(scope.pop()) { return Err(e.to_string()); }
    Ok(MatPipe { pipeline, layouts, group_entries, groups: vec![], dyn_count, blend: pm.blend, uses_depth })
}

fn build_groups(device: &wgpu::Device, mp: &MatPipe, depth_only: &wgpu::TextureView) -> Vec<wgpu::BindGroup> {
    mp.group_entries.iter().enumerate().map(|(g, entries)| {
        let es: Vec<wgpu::BindGroupEntry> = entries.iter().map(|e| match e {
            GroupEntry::Prim { binding, size, buf } => wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: buf, offset: 0, size: wgpu::BufferSize::new(*size) }) },
            GroupEntry::Buf { binding, buf } => wgpu::BindGroupEntry { binding: *binding, resource: buf.as_entire_binding() },
            GroupEntry::Tex { binding, view } => wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::TextureView(view) },
            GroupEntry::Samp { binding, s } => wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::Sampler(s) },
            GroupEntry::Depth { binding } => wgpu::BindGroupEntry { binding: *binding, resource: wgpu::BindingResource::TextureView(depth_only) },
        }).collect();
        device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby mat bg"), layout: &mp.layouts[g], entries: &es })
    }).collect()
}

// ---------------------------------------------------------------------------------------------------------------------------------
// character (BR hero on the CharacterPlacement of party slot 0, Menu intro + idle)
// ---------------------------------------------------------------------------------------------------------------------------------
#[allow(clippy::type_complexity)]
fn build_character(device: &wgpu::Device, queue: &wgpu::Queue, asset: Option<CharAsset>, data: &LobbySceneData, group3: &wgpu::BindGroupLayout, cview: &wgpu::Buffer, shadow_vp: &wgpu::Buffer)
    -> Result<(wgpu::RenderPipeline, wgpu::RenderPipeline, CharGpu, Xf, CharAsset), (String, Option<CharAsset>)> {
    let Some(a) = asset else { return Err(("no character asset".into(), None)) };
    let Some(place) = data.placements.iter().find(|p| p.player_id == 0) else { return Err(("no party slot 0".into(), Some(a))) };
    let xf = Xf { t: place.character.t, q: place.character.q, s: Vec3::ONE };
    let src = format!("{PRELUDE}{CHAR_WGSL}");
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("lobby character"), source: wgpu::ShaderSource::Wgsl(src.into()) });
    let char_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby char bgl"), entries: &[
        bgl_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
        bgl_entry(1, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
        bgl_entry(2, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }),
        bgl_entry(3, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
    ] });
    let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby char view bgl"), entries: &[bgl_entry(0, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None })] });
    let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("empty"), entries: &[] });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("lobby char pl"), bind_group_layouts: &[Some(&char_layout), Some(&view_layout), Some(&empty), Some(group3)], immediate_size: 0 });
    let attrs = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 24, shader_location: 2 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Uint16x4, offset: 32, shader_location: 3 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x4, offset: 40, shader_location: 4 },
    ];
    let vb_layout = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<SkinVertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &attrs };
    // the glTF -> UE mirror (det -1) flips the winding: glTF front faces (CCW) become clockwise
    let prim = wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: Some(wgpu::Face::Back), front_face: wgpu::FrontFace::Cw, ..Default::default() };
    let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lobby character"), layout: Some(&pl),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[Some(vb_layout.clone())] },
        primitive: prim,
        depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: HDR, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
        multiview_mask: None, cache: None,
    });
    let spl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("lobby char shadow pl"), bind_group_layouts: &[Some(&char_layout), Some(&view_layout)], immediate_size: 0 });
    let spipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lobby character shadow"), layout: Some(&spl),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_shadow"), compilation_options: Default::default(), buffers: &[Some(vb_layout.clone())] },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 } }),
        multisample: Default::default(), fragment: None, multiview_mask: None, cache: None,
    });
    // ---- original character material pipeline ----
    let fx = |binding: u32| bgl_entry(binding, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false });
    let fs = |binding: u32| bgl_entry(binding, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering));
    let orig_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("lobby char orig bgl"), entries: &[
        bgl_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
        bgl_entry(1, wgpu::ShaderStages::VERTEX, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
        fx(2), fx(3), fx(4), fx(5), fs(6),
        bgl_entry(7, wgpu::ShaderStages::FRAGMENT, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
        fx(8), fs(9),
    ] });
    let gen_src = include_str!("../../fn-render/shaders/char_material_gen.wgsl").replace("diagnostic(off, derivative_uniformity);", "");
    let orig_module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("lobby character (original materials)"), source: wgpu::ShaderSource::Wgsl(format!("diagnostic(off, derivative_uniformity);\n{PRELUDE}{gen_src}{CHAR_ORIG_WGSL}").into()) });
    let orig_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("lobby char orig pl"), bind_group_layouts: &[Some(&orig_layout), Some(&view_layout), Some(&empty), Some(group3)], immediate_size: 0 });
    let orig_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lobby character original"), layout: Some(&orig_pl),
        vertex: wgpu::VertexState { module: &orig_module, entry_point: Some("vs_orig"), compilation_options: Default::default(), buffers: &[Some(vb_layout)] },
        // two-sided like the renderer's character pass (the body permutation reads the front-face flag); glTF -> UE mirror flips the winding
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, front_face: wgpu::FrontFace::Cw, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: DEPTH, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::LessEqual), stencil: Default::default(), bias: Default::default() }),
        multisample: Default::default(),
        fragment: Some(wgpu::FragmentState { module: &orig_module, entry_point: Some("fs_orig"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: HDR, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
        multiview_mask: None, cache: None,
    });
    let cam_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby char cam"), layout: &view_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: cview.as_entire_binding() }] });
    let shadow_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby char shadow"), layout: &view_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: shadow_vp.as_entire_binding() }] });
    let empty_bg = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("empty"), layout: &empty, entries: &[] });
    let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("char"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, address_mode_u: wgpu::AddressMode::Repeat, address_mode_v: wgpu::AddressMode::Repeat, anisotropy_clamp: 8, ..Default::default() });
    // glTF mesh space (m, Y up, +Z forward) -> UE (cm): (x, y, z) -> (x, z, y) * 100, then the placement
    let swap = Mat4::from_cols(Vec4::new(100., 0., 0., 0.), Vec4::new(0., 0., 100., 0.), Vec4::new(0., 100., 0., 0.), Vec4::W);
    let model = xf.matrix() * swap;
    let parts = build_char_parts(device, queue, &a, &model, &char_layout, &orig_layout, &samp);
    Ok((pipe, spipe, CharGpu { parts, cam_bg, shadow_bg, empty_bg, orig_pipe, char_layout, orig_layout, samp, model }, xf, a))
}

/// GPU resources (vertex / index / bone buffers, textures, bind groups) of the body + head parts of `a`: what `LobbyScene::set_character` rebuilds when the
/// Locker equips another outfit (the pipelines and bind group layouts stay).
fn build_char_parts(device: &wgpu::Device, queue: &wgpu::Queue, a: &CharAsset, model: &Mat4, char_layout: &wgpu::BindGroupLayout, orig_layout: &wgpu::BindGroupLayout, samp: &wgpu::Sampler) -> Vec<CharPartGpu> {
    build_char_parts_of(device, queue, &a.parts, model, char_layout, orig_layout, samp)
}
fn build_char_parts_of(device: &wgpu::Device, queue: &wgpu::Queue, bparts: &[fn_pose::br_character::BrPart], model: &Mat4, char_layout: &wgpu::BindGroupLayout, orig_layout: &wgpu::BindGroupLayout, samp: &wgpu::Sampler) -> Vec<CharPartGpu> {
    let mut parts = vec![];
    for (pi, bp) in bparts.iter().take(2).enumerate() {
        let n = bp.part.palette_len().max(1);
        let bones = buffer(device, "lobby bones", (n * 64) as u64, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let mut prims = vec![];
        for (prim, sec) in bp.part.prims.iter().zip(&bp.sections) {
            if sec.hidden.is_some() { continue; }
            let hair = sec.ue.as_ref().map(|u| u.perm == "hair").unwrap_or(false);
            let obj = buffer(device, "lobby char obj", 80, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
            let mut ob: Vec<f32> = model.to_cols_array().to_vec();
            ob.extend_from_slice(&[if hair { 1. } else { 0. }, 0., 0., 0.]);
            queue.write_buffer(&obj, 0, bytemuck::cast_slice(&ob));
            let verts: Vec<SkinVertex> = (0..prim.positions.len()).map(|i| SkinVertex { pos: prim.positions[i], normal: prim.normals[i], uv: prim.uvs[i], joints: prim.joints[i], weights: prim.weights[i] }).collect();
            let vb = buffer(device, "lobby char vb", (verts.len() * std::mem::size_of::<SkinVertex>()) as u64, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST);
            queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
            let ib = buffer(device, "lobby char ib", (prim.indices.len() * 4) as u64, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST);
            queue.write_buffer(&ib, 0, bytemuck::cast_slice(&prim.indices));
            let tex = match &sec.diffuse {
                Some(t) => upload_texture(device, queue, "char diffuse", &TexAsset { levels: vec![(t.width as u32, t.height as u32, t.rgba.clone())], srgb: true, filter: String::new(), address: String::new() }, true).1,
                None => upload_texture(device, queue, "char grey", &TexAsset { levels: vec![(1, 1, vec![128, 128, 128, 255])], srgb: true, filter: String::new(), address: String::new() }, true).1,
            };
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby char bg"), layout: &char_layout, entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: obj.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: bones.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&tex) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&samp) },
            ] });
            let orig = match (&sec.ue, &sec.diffuse, &sec.normal, &sec.spec, &sec.mask) {
                (Some(ue), Some(d), Some(n), Some(s), Some(m)) => {
                    // the renderer hands the translated shader the RAW texel values (the shader decodes what it needs): all four are uploaded linear
                    let up = |label: &str, srgb: bool, t: &fn_pose::br_character::RgbaTexture| -> Option<wgpu::TextureView> {
                        let img = image::RgbaImage::from_raw(t.width as u32, t.height as u32, t.rgba.clone())?;
                        Some(fn_render::texture::upload_rgba8(device, queue, &img, srgb, fn_render::texture::MipFilter::Triangle, label).create_view(&Default::default()))
                    };
                    let nz = &ue.noise;
                    let noise = image::RgbaImage::from_raw(nz.tex.width as u32, nz.tex.height as u32, nz.tex.rgba.clone()).map(|img| fn_render::texture::upload_rgba8(device, queue, &img, nz.srgb, fn_render::texture::MipFilter::Triangle, "char noise").create_view(&Default::default()));
                    match (up("char diffuse", true, d), up("char normal", false, n), up("char spec", false, s), up("char mask", false, m), noise) {
                        (Some(vd), Some(vn), Some(vs), Some(vm), Some(vnoise)) => {
                            let mut cm = [[0f32; 4]; 33];
                            for (k, r) in ue.cb.iter().take(32).enumerate() { cm[k] = *r; }
                            let perm = match ue.perm { "head" => 0u32, "hair" => 1, _ => 2 };
                            cm[32] = [f32::from_bits(perm), 0., 0., 0.];
                            let cbuf = buffer(device, "lobby char cb3", 33 * 16, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST);
                            queue.write_buffer(&cbuf, 0, bytemuck::cast_slice(&cm));
                            let nsamp = sampler_for(device, "", &nz.address[0], 0);
                            Some(device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby char orig bg"), layout: &orig_layout, entries: &[
                                wgpu::BindGroupEntry { binding: 0, resource: obj.as_entire_binding() },
                                wgpu::BindGroupEntry { binding: 1, resource: bones.as_entire_binding() },
                                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&vd) },
                                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&vn) },
                                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&vs) },
                                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&vm) },
                                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&samp) },
                                wgpu::BindGroupEntry { binding: 7, resource: cbuf.as_entire_binding() },
                                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(&vnoise) },
                                wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::Sampler(&nsamp) },
                            ] }))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            prims.push(CharPrimGpu { vb, ib, count: prim.indices.len() as u32, bind, orig });
        }
        let _ = pi;
        parts.push(CharPartGpu { prims, bones });
    }
    parts
}

// ---------------------------------------------------------------------------------------------------------------------------------
// frame
// ---------------------------------------------------------------------------------------------------------------------------------
impl LobbyScene {
    /// The party slots of the lobby (UE world centimetres) for the UI half: name plate / "+" marker anchors and the hero's feet.
    pub fn party_slots(&self) -> Vec<PartySlot> {
        self.data.placements.iter().map(|p| PartySlot {
            player_id: p.player_id, hidden: p.hidden, occupied: p.player_on_pad, pawn_world_cm: p.character.t, pad_world_cm: p.root.t,
            top_widget_cm: p.top_widget.t, add_widget_cm: p.add_widget.t, top_pivot: p.top_info.pivot, add_pivot: p.add_info.pivot,
        }).collect()
    }
    /// Pixel position (origin top-left, y down) of a UE world point for a `width` x `height` frame of the recovered camera; `None` behind the camera.
    pub fn project(&self, world_cm: Vec3, width: u32, height: u32) -> Option<[f32; 2]> {
        let c = CameraFrame::new(&self.camera, self.fov, width, height).view_proj() * world_cm.extend(1.0);
        if c.w <= 0.0 { return None; }
        Some([(c.x / c.w * 0.5 + 0.5) * width as f32, (0.5 - c.y / c.w * 0.5) * height as f32])
    }
    pub fn camera_xf(&self) -> Xf { self.camera }
    pub fn fov_deg(&self) -> f32 { self.fov }

    /// The hero (`HID_*` path) of the character on the pad.
    pub fn hero(&self) -> Option<&str> { self.char_asset.as_ref().map(|a| a.hero.as_str()) }

    /// Puts another outfit on the pad (LOCKER: the equipped outfit changed): the hero's parts / materials / textures (`load_hero_parts`, CPU only, may run on a worker
    /// thread) are uploaded into the existing character pipelines; the animation clips (shared by every BR hero: they follow the master skeleton) are kept and the
    /// AnimBP state restarts. `false` when the scene has no character pipelines (it was loaded without a hero).
    pub fn set_hero(&mut self, p: HeroParts) -> bool {
        let Some(b) = self.hero_builder() else { return false };
        let ready = b.build(&p);
        self.set_hero_ready(ready)
    }

    /// Handles a worker thread needs to upload a hero (`HeroBuilder::build`); `None` when the scene has no character pipelines.
    pub fn hero_builder(&self) -> Option<HeroBuilder> {
        let g = self.char_gpu.as_ref()?;
        Some(HeroBuilder { device: self.device.clone(), queue: self.queue.clone(), char_layout: g.char_layout.clone(), orig_layout: g.orig_layout.clone(), samp: g.samp.clone(), model: g.model })
    }

    /// Swaps an uploaded hero in (cheap): keeps the animation clips, restarts the AnimBP state.
    pub fn set_hero_ready(&mut self, r: HeroReady) -> bool {
        let Some(g) = self.char_gpu.as_mut() else { return false };
        let Some(old) = self.char_asset.take() else { return false };
        g.parts = r.gpu;
        self.char_asset = Some(CharAsset { hero: r.hero, parts: r.parts, rig: r.rig, clips: old.clips, notes: r.notes });
        self.char_pose = fn_pose::CharacterPose::default();
        self.char_t0 = self.last_time;
        self.anim = None;
        true
    }
    /// PRELOAD HOOK: like [`set_hero_ready`](Self::set_hero_ready) but hands the hero that was on the pad back as a ready-to-reuse [`HeroReady`] (so the outfit cache of
    /// `lobby_preload` keeps every skin the player has tried and swapping back costs nothing). `Err(r)` (the scene has no character pipelines) gives `r` back.
    pub fn swap_hero_ready(&mut self, r: HeroReady) -> Result<HeroReady, HeroReady> {
        if self.char_gpu.is_none() || self.char_asset.is_none() { return Err(r); }
        let (Some(g), Some(old)) = (self.char_gpu.as_mut(), self.char_asset.take()) else { return Err(r) };
        let prev = HeroReady { hero: old.hero, parts: old.parts, rig: old.rig, notes: old.notes, gpu: std::mem::replace(&mut g.parts, r.gpu) };
        self.char_asset = Some(CharAsset { hero: r.hero, parts: r.parts, rig: r.rig, clips: old.clips, notes: r.notes });
        self.char_pose = fn_pose::CharacterPose::default();
        self.char_t0 = self.last_time;
        self.anim = None;
        Ok(prev)
    }

    fn ensure_targets(&mut self, w: u32, h: u32) {
        if self.targets.as_ref().is_some_and(|t| t.w == w && t.h == h) { return; }
        let d = &self.device;
        let mips = w.max(h).max(1).ilog2() + 1;
        let hdr = d.create_texture(&wgpu::TextureDescriptor { label: Some("lobby hdr"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: mips, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: HDR, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC, view_formats: &[] });
        let hdr_mips: Vec<wgpu::TextureView> = (0..mips).map(|m| hdr.create_view(&wgpu::TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..Default::default() })).collect();
        let hdr_full = hdr.create_view(&Default::default());
        let depth = d.create_texture(&wgpu::TextureDescriptor { label: Some("lobby depth"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: DEPTH, usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
        let depth_view = depth.create_view(&Default::default());
        let depth_only = depth.create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
        let post_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby post bg"), layout: &self.post_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: self.post_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&hdr_full) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.post_samp) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&self.lut_view) },
        ] });
        let down_bgs = (1..mips as usize).map(|i| d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("lobby down bg"), layout: &self.down_layout, entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&hdr_mips[i - 1]) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.post_samp) },
        ] })).collect();
        for mp in self.mat_pipes.iter_mut() { mp.groups = build_groups(d, mp, &depth_only); }
        self.targets = Some(Targets { w, h, hdr, hdr_mips, hdr_full, depth, depth_view, depth_only, post_bg, down_bgs });
    }

    fn post_u(&self, w: u32, h: u32) -> PostU {
        let p = &self.data.post;
        let wb = fn_render::tonemap::white_balance_columns();
        let v4 = |n: &str| p.v4(n);
        let tints: [[f32; 4]; 6] = std::array::from_fn(|i| v4(&format!("Bloom{}Tint", i + 1)));
        let scale = p.f1("BloomSizeScale");
        let sizes: Vec<f32> = (1..=6).map(|i| p.f1(&format!("Bloom{i}Size")) * scale).collect();
        PostU {
            wb0: wb[0], wb1: wb[1], wb2: wb[2],
            film_stw: [p.f1("FilmSlope"), p.f1("FilmToe"), p.f1("FilmShoulder"), 0.], film_clip: [p.f1("FilmBlackClip"), p.f1("FilmWhiteClip"), 0., 0.],
            bloom: [p.f1("BloomIntensity"), p.f1("BloomThreshold"), 1., 0.], vignette: [p.f1("VignetteIntensity"), 1., 0., 0.], view: [w as f32, h as f32, 1., 0.],
            sat_shadows: v4("ColorSaturationShadows"), sat_mid: v4("ColorSaturationMidtones"), sat_high: v4("ColorSaturationHighlights"),
            grade: [p.f1("ColorCorrectionShadowsMax"), p.f1("ColorCorrectionHighlightsMin"), 0., 0.],
            lut: [p.f1("ColorGradingIntensity"), if self.data.post.color_grading_lut.is_some() { 1. } else { 0. }, 0., 0.],
            bloom_tints: tints, bloom_sizes: [[sizes[0], sizes[1], sizes[2], sizes[3]], [sizes[4], sizes[5], 0., 0.]],
        }
    }
    fn post_uniform_dirty(&mut self) {}

    /// Renders the lobby scene into `target` (a non-sRGB colour view of `width` x `height`). `time` in seconds since the lobby opened (drives the
    /// hero's Menu intro + idle and the materials' time inputs).
    pub fn render(&mut self, target: &wgpu::TextureView, width: u32, height: u32, time: f32) {
        self.last_time = time;
        self.ensure_targets(width, height);
        let cam = CameraFrame::new(&self.camera, self.fov, width, height);
        let vp = cam.view_proj();
        // ---- uniforms ----
        let mut fu: FrameU = FrameU::zeroed();
        fu.inv_vp = vp.inverse().to_cols_array_2d();
        fu.cam = [cam.eye.x, cam.eye.y, cam.eye.z, time];
        fu.viewport = [width as f32, height as f32, 1.0 / width as f32, 1.0 / height as f32];
        // sun + shadow matrix
        let mut shadow_vp = Mat4::IDENTITY;
        if let Some((to_light, col)) = self.sun {
            fu.sun_dir = [to_light.x, to_light.y, to_light.z, 1.0];
            fu.sun_color = [col[0], col[1], col[2], 0.0];
            let center = cam.eye + cam.fwd * 1200.0;
            let half = 2600.0;
            let up = if to_light.z.abs() > 0.95 { Vec3::X } else { Vec3::Z };
            let eye = center + to_light * 6000.0;
            let view = Mat4::look_to_lh(eye, -to_light, up);
            let proj = Mat4::orthographic_lh(-half, half, -half, half, 0.0, 12000.0);
            // snap to texels
            let texel = 2.0 * half / SHADOW_SIZE as f32;
            let mut m = proj * view;
            let o = m.transform_point3(Vec3::ZERO);
            let (sx, sy) = ((o.x * 0.5 * SHADOW_SIZE as f32).round() / (0.5 * SHADOW_SIZE as f32) - o.x, (o.y * 0.5 * SHADOW_SIZE as f32).round() / (0.5 * SHADOW_SIZE as f32) - o.y);
            m = Mat4::from_translation(Vec3::new(sx, sy, 0.0)) * m;
            let _ = texel;
            shadow_vp = m;
        }
        fu.shadow_vp = shadow_vp.to_cols_array_2d();
        for k in 0..9 { fu.sh[k] = [self.sh.c[k][0], self.sh.c[k][1], self.sh.c[k][2], 0.]; }
        if let Some(f) = &self.fog {
            let c = f.inscattering_color;
            fu.fog_a = [f.density * 0.001, f.height_falloff * 0.001, f.xf.t.z, f.max_opacity];
            fu.fog_b = [c[0], c[1], c[2], f.start_distance];
        }
        fu.counts = [self.lights.len() as u32, 0, 0, 0];
        for i in 0..6 { fu.pt_vp[i] = self.pt_vps[i].to_cols_array_2d(); }
        fu.pt_idx = [self.pt_idx, 0, 0, 0];
        for (i, l) in self.lights.iter().enumerate().take(MAX_LIGHTS) { fu.lights[i] = *l; }
        self.queue.write_buffer(&self.frame_buf, 0, bytemuck::bytes_of(&fu));
        self.queue.write_buffer(&self.shadow_vp_buf, 0, bytemuck::cast_slice(&shadow_vp.to_cols_array()));
        self.queue.write_buffer(&self.cview_buf, 0, bytemuck::cast_slice(&vp.to_cols_array()));
        let vb: Vec<f32> = cam.view_buffer(time).iter().flatten().copied().collect();
        self.queue.write_buffer(&self.view_buf, 0, bytemuck::cast_slice(&vb));
        let pu = self.post_u(width, height);
        self.queue.write_buffer(&self.post_buf, 0, bytemuck::bytes_of(&pu));
        // ---- character pose ----
        if let (Some(a), Some(g)) = (&self.char_asset, &self.char_gpu) {
            // BR player AnimBP, idle: no velocity, on ground, nothing held. Stepped at 60 Hz from the lobby start (a time that goes backwards restarts it).
            let ct = (time - self.char_t0).max(0.0);
            if self.anim.as_ref().is_some_and(|(_, t)| *t > ct + 1e-4) { self.anim = None; }
            let (st, st_t) = self.anim.get_or_insert_with(|| (fn_animgraph::AnimState::new(), 0.0));
            let weapon: &'static str = lobby_weapon();
            let input = fn_animgraph::PawnInput { weapon, ..fn_animgraph::PawnInput::default() };
            while *st_t + 1.0 / 60.0 <= ct { st.step(&input, 1.0 / 60.0); *st_t += 1.0 / 60.0; }
            if std::env::var("FN_LOBBY_NOIK").is_ok() { st.vars.right_hand_ik_alpha = 0.0; st.vars.left_hand_ik_alpha = 0.0; }
            let tree = st.plan(&[]);
            a.rig.evaluate_into(&tree, &*a.clips, &mut self.char_pose);
            // the body's pose drives the head part's bones too (the head clip's face bones are not applied)
            for (pi, part) in g.parts.iter().enumerate() {
                if let Some(pr) = self.char_pose.parts.get(pi) { self.queue.write_buffer(&part.bones, 0, &pr.gpu_bytes()); }
            }
        }
        let t = self.targets.as_ref().unwrap();
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("lobby frame") });
        // ---- point light cube shadow ----
        if self.pt_idx != u32::MAX {
            for f in 0..6 {
                let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("lobby point shadow"), color_attachments: &[], depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.pt_face_views[f], depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }), timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
                for d in self.draws.iter().filter(|d| d.shadow) {
                    let m = &self.meshes[d.mesh];
                    pass.set_pipeline(&self.shadow_pipes[m.pairs.clamp(1, 4) - 1]);
                    pass.set_bind_group(0, &self.pt_bgs[f], &[d.prim as u32]);
                    pass.set_vertex_buffer(0, m.vb.slice(..));
                    pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(d.first..d.first + d.count, 0, 0..1);
                }
            }
        }
        // ---- shadow ----
        if self.sun.is_some() {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("lobby shadow"), color_attachments: &[], depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.shadow_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }), timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            for d in self.draws.iter().filter(|d| d.shadow) {
                let m = &self.meshes[d.mesh];
                pass.set_pipeline(&self.shadow_pipes[m.pairs.clamp(1, 4) - 1]);
                pass.set_bind_group(0, &self.shadow_bg, &[d.prim as u32]);
                pass.set_vertex_buffer(0, m.vb.slice(..));
                pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(d.first..d.first + d.count, 0, 0..1);
            }
            if let (Some(sp), Some(g)) = (&self.char_shadow_pipe, &self.char_gpu) {
                pass.set_pipeline(sp);
                pass.set_bind_group(1, &g.shadow_bg, &[]);
                for part in &g.parts { for p in &part.prims {
                    pass.set_bind_group(0, &p.bind, &[]);
                    pass.set_vertex_buffer(0, p.vb.slice(..));
                    pass.set_index_buffer(p.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..p.count, 0, 0..1);
                } }
            }
        }
        // ---- scene (opaque + masked) ----
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("lobby scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.hdr_mips[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.depth_view, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            pass.set_bind_group(3, &self.group3, &[]);
            let mut last = usize::MAX;
            let mut order: Vec<&DrawItem> = self.draws.iter().filter(|d| !d.translucent).collect();
            order.sort_by_key(|d| d.material_key);
            for d in order { self.draw_static(&mut pass, d, &mut last); }
            if let (Some(p), Some(g)) = (&self.char_pipe, &self.char_gpu) {
                pass.set_pipeline(p);
                pass.set_bind_group(1, &g.cam_bg, &[]);
                pass.set_bind_group(2, &g.empty_bg, &[]);
                pass.set_bind_group(3, &self.group3, &[]);
                for part in &g.parts { for pr in &part.prims {
                    if let Some(ob) = &pr.orig {
                        pass.set_pipeline(&g.orig_pipe);
                        pass.set_bind_group(0, ob, &[]);
                        pass.set_bind_group(1, &g.cam_bg, &[]);
                        pass.set_bind_group(2, &g.empty_bg, &[]);
                        pass.set_bind_group(3, &self.group3, &[]);
                    } else {
                        pass.set_pipeline(p);
                        pass.set_bind_group(0, &pr.bind, &[]);
                        pass.set_bind_group(1, &g.cam_bg, &[]);
                        pass.set_bind_group(2, &g.empty_bg, &[]);
                        pass.set_bind_group(3, &self.group3, &[]);
                    }
                    pass.set_vertex_buffer(0, pr.vb.slice(..));
                    pass.set_index_buffer(pr.ib.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..pr.count, 0, 0..1);
                } }
            }
        }
        // ---- translucent / additive unlit FX meshes (depth read-only) ----
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("lobby translucent"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.hdr_mips[0], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.depth_view, depth_ops: None, stencil_ops: None }),
                timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            pass.set_bind_group(3, &self.group3, &[]);
            let mut order: Vec<&DrawItem> = self.draws.iter().filter(|d| d.translucent).collect();
            order.sort_by(|a, b| a.sort_priority.cmp(&b.sort_priority).then((b.pos - cam.eye).length().partial_cmp(&(a.pos - cam.eye).length()).unwrap_or(std::cmp::Ordering::Equal)));
            let mut last = usize::MAX;
            for d in order { self.draw_static(&mut pass, d, &mut last); }
        }
        // ---- bloom mips ----
        for i in 1..t.hdr_mips.len() {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("lobby mip"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.hdr_mips[i], depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            pass.set_pipeline(&self.down_pipe);
            pass.set_bind_group(0, &t.down_bgs[i - 1], &[]);
            pass.draw(0..3, 0..1);
        }
        // ---- post ----
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("lobby post"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
            pass.set_pipeline(&self.post_pipe);
            pass.set_bind_group(0, &t.post_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([enc.finish()]);
    }

    fn draw_static<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, d: &DrawItem, last: &mut usize) {
        let mp = &self.mat_pipes[d.material_key];
        if *last != d.material_key { pass.set_pipeline(&mp.pipeline); *last = d.material_key; }
        let offs: Vec<u32> = vec![d.prim as u32; mp.dyn_count];
        for (g, bg) in mp.groups.iter().enumerate() {
            if g == 0 { pass.set_bind_group(0, bg, &offs); } else { pass.set_bind_group(g as u32, bg, &[]); }
        }
        let m = &self.meshes[d.mesh];
        pass.set_vertex_buffer(0, m.vb.slice(..));
        pass.set_index_buffer(m.ib.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(d.first..d.first + d.count, 0, 0..1);
    }
}




// =====================================================================================================================================
// PER-TAB BACKDROPS (`SetFrontEndCamera` of every top-bar tab)
// =====================================================================================================================================

/// The top-bar tabs of `AthenaTabsScreen`. Each tab's content widget calls `FortFrontEndContext::SetFrontEndCamera(<EFrontEndCamera>)` in its `OnActivated`
/// (Kismet disassembly, `fn-inspect disasm`): `AthenaLobby` 11 (HeroSelect), `AthenaCustomizationTab` 16 (Cosmetics, plus `ViewVaultItem(null)`), `AthenaDirectAcquisitionScreen` 3 (Store,
/// plus `ViewVaultItem(null)`), `AthenaSeasonDirectAcquisitionScreen` 3, `AthenaStatsTab` 1 (HomeBase), `AthenaLeaderboardTab` 1; `StoreMain_Root` (the "Store" tab) sets none.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum LobbyScreen { Lobby, Locker, ItemShop, SeasonShop, Stats, Leaderboards, Store }

impl LobbyScreen {
    pub const ALL: [LobbyScreen; 7] = [LobbyScreen::Lobby, LobbyScreen::Locker, LobbyScreen::ItemShop, LobbyScreen::SeasonShop, LobbyScreen::Stats, LobbyScreen::Leaderboards, LobbyScreen::Store];
    /// `(EFrontEndCamera byte, enumerator name)` the tab requests; None when it requests none (the previous camera stays).
    pub fn front_end_camera(self) -> Option<(u8, &'static str)> {
        match self {
            LobbyScreen::Lobby => Some((11, "HeroSelect")),
            LobbyScreen::Locker => Some((16, "Cosmetics")),
            LobbyScreen::ItemShop | LobbyScreen::SeasonShop => Some((3, "Store")),
            LobbyScreen::Stats | LobbyScreen::Leaderboards => Some((1, "HomeBase")),
            LobbyScreen::Store => None,
        }
    }
    /// Index of the tab in the top bar (`PreregisteredTabInfoArray` order of the visible tabs): Lobby, Locker, Item Shop, Season Shop, Stats, Leaderboards, Store.
    pub fn from_tab_index(i: usize) -> LobbyScreen { Self::ALL.get(i).copied().unwrap_or(LobbyScreen::Lobby) }
    /// Full-screen darkening that the screen's camera puts in front of the view, as straight sRGB RGBA to alpha-blend between the 3D scene and the UI. Item Shop and Season Shop
    /// (camera Store): `StoreCamera_Blueprint`'s `Mesh-DarkenBG` plane with `M_StoreBGDarken` (unlit translucent: colour = `Color` parameter (0.023959722, 0.020375002, 0.025) linear,
    /// opacity = `saturate(BGChange * 0.8)` with `MPC_Pinata.BGChange` = 1, read from the material's translated pixel shader). The plane itself is not rasterised by this port (it
    /// does not appear in the render, cause not found), so its effect is applied as a flat blend: SUBSTITUTE, the colour is the sRGB encoding of the linear value (no film curve).
    pub fn darken(self) -> Option<[f32; 4]> {
        match self {
            LobbyScreen::ItemShop | LobbyScreen::SeasonShop => {
                let enc = |c: f32| if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
                Some([enc(0.023_959_722), enc(0.020_375_002), enc(0.025), 0.8])
            }
            _ => None,
        }
    }
}

impl LobbyAssets {
    /// CPU half of a non-lobby backdrop (`camera` = `"Cosmetics"` | `"Store"` | `"HomeBase"` | any `EFrontEndCamera` name of `FrontEndSettingsBP_31.Cameras`). `character` is the lobby
    /// hero (`CharAsset::share`), used by the Locker only. Safe to run on a worker thread; then `LobbyScene::from_assets` on the render thread.
    pub fn load_backdrop(install: &Path, camera: &str, character: Option<CharAsset>) -> Result<LobbyAssets, String> {
        match camera {
            "Cosmetics" => Self::load_cosmetics(install, character),
            other => Self::load_view(install, other, 5_000.0),
        }
    }
}

/// The non-lobby backdrop scenes, one per front-end camera, and the camera the current tab shows. The LOBBY scene is never held here (it stays the caller's own `LobbyScene`,
/// unchanged): `scene_mut()` returns None while the lobby camera (or no loaded backdrop) is current.
pub struct ScreenBackdrops {
    scenes: HashMap<&'static str, LobbyScene>,
    camera: &'static str,
    screen: LobbyScreen,
}

impl ScreenBackdrops {
    pub fn new() -> ScreenBackdrops { ScreenBackdrops { scenes: HashMap::new(), camera: "HeroSelect", screen: LobbyScreen::Lobby } }
    /// A tab became active (its `OnActivated` ran `SetFrontEndCamera`). Tabs that request no camera (Store) keep the previous one.
    pub fn set_screen(&mut self, screen: LobbyScreen) {
        self.screen = screen;
        if let Some((_, name)) = screen.front_end_camera() { self.camera = name; }
    }
    pub fn screen(&self) -> LobbyScreen { self.screen }
    /// The camera name of the current backdrop (`"HeroSelect"` = the lobby scene).
    pub fn camera(&self) -> &'static str { self.camera }
    /// The camera whose scene must be built before the current tab can be drawn (None: lobby, or already loaded).
    pub fn pending(&self) -> Option<&'static str> { if self.camera == "HeroSelect" || self.scenes.contains_key(self.camera) { None } else { Some(self.camera) } }
    pub fn insert(&mut self, camera: &'static str, scene: LobbyScene) { self.scenes.insert(camera, scene); }
    /// The scene to render for the current tab, or None to render the caller's lobby scene.
    pub fn scene_mut(&mut self) -> Option<&mut LobbyScene> { if self.camera == "HeroSelect" { None } else { self.scenes.get_mut(self.camera) } }
    pub fn scene_for_mut(&mut self, camera: &str) -> Option<&mut LobbyScene> { self.scenes.get_mut(camera) }
    /// `LobbyScreen::darken` of the current tab.
    pub fn darken(&self) -> Option<[f32; 4]> { self.screen.darken() }
}