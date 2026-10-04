//! `--camera-video`: scripted camera/movement sequence at a fixed 60 Hz step, rendered by the real
//! renderer at 1280x720; every frame is written to `video-frames/frame-NNNN.png` beside the exe.
//! Encoding to a video is done outside the runtime (see verification/camera-movement*.mp4).
//! `sim` / `save_frame` are shared with `--menu-video` (src/menu_video.rs).
use fn_game::camera;

pub const FRAMES: u32 = 960; // 16 s at 60 fps

/// Scripted input for frame `f`.
#[derive(Default)]
pub struct Step { pub yaw: f32, pub pitch: f32, pub forward: f32, pub side: f32, pub run: bool, pub jump: bool, pub crouch: bool, pub teleport: bool,
    /// Primary fire held (pickaxe swing / harvest).
    pub attack: bool,
    /// Reset to the spawn facing -Z and plant a tree 1.8 m ahead (deterministic harvest target).
    pub plant_tree: bool,
    /// Start the Melee_Equip montage (as selecting the pickaxe slot does in `App::update`).
    pub equip: bool }

fn lerp(a: f32, b: f32, t: f32) -> f32 { a + (b - a) * t.clamp(0., 1.) }
fn smooth(t: f32) -> f32 { let t = t.clamp(0., 1.); t * t * (3. - 2. * t) }

pub fn step(f: u32) -> Step {
    let mut s = Step { pitch: -12., ..Default::default() };
    let ff = f as f32;
    // 0-20 idle; 20-200 slow 360 degree yaw orbit (2 deg/frame = 120 deg/s)
    if f >= 20 { s.yaw = ((ff - 20.) * 2.).min(360.) % 360.; }
    // 200-320 pitch sweep: up to the +clamp, down to the -clamp, back to the default
    if (200..230).contains(&f) { s.pitch = lerp(-12., camera::PITCH_MAX, smooth((ff - 200.) / 30.)); }
    else if (230..290).contains(&f) { s.pitch = lerp(camera::PITCH_MAX, camera::PITCH_MIN, smooth((ff - 230.) / 60.)); }
    else if (290..320).contains(&f) { s.pitch = lerp(camera::PITCH_MIN, -12., smooth((ff - 290.) / 30.)); }
    // 320-380 walk forward, 380-410 strafe right, 410-440 strafe left
    if (320..380).contains(&f) { s.forward = 1.; }
    if (380..410).contains(&f) { s.side = 1.; }
    if (410..440).contains(&f) { s.side = -1.; }
    // 440-500 sprint forward with a jump at 445
    if (440..500).contains(&f) { s.forward = 1.; s.run = true; }
    if f == 445 { s.jump = true; }
    // 540-580 crouch (after landing), release at 580 (un-crouch), stand until 600
    if (540..580).contains(&f) { s.crouch = true; }
    // 600: cut to 10 m from the -Z world edge, then sprint into it until 700
    if f == 600 { s.teleport = true; }
    if (600..700).contains(&f) { s.forward = 1.; s.run = true; }
    // 700: back at the spawn facing a tree 1.8 m ahead; pickaxe swings (held fire = combo) from 725 to 845 harvest it
    if f >= 700 { s.yaw = 0.; }
    if f == 700 { s.plant_tree = true; }
    if (725..845).contains(&f) { s.attack = true; }
    // 870: re-equip the pickaxe (Melee_Equip) and jump while the equip montage plays
    if f == 870 { s.equip = true; }
    if f == 880 { s.jump = true; }
    s
}

