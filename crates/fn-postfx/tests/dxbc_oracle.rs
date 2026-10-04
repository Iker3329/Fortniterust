//! The CPU references (and therefore, through `gpu_vs_cpu`, the WGSL) are checked against the ORIGINAL compiled shaders of the 1.8 install:
//! the DXBC pixel shaders are extracted from `GlobalShaderCache-PCD3D_SM5.bin` and executed with the `fn-dxbc` interpreter on the same
//! inputs. Skipped when the install is not present. Compute shaders (velocity flatten / gather) are not interpretable (TGSM + UAV).
use fn_dxbc::interp::{Env, Machine, TexOp, TexReq};
use fn_dxbc::Shader;
use fn_pak::PakSet;
use fn_postfx::cpu::{self, Image};
use fn_postfx::params::*;
use fn_postfx::shadercache::{self, CachedShader};
use fn_postfx::synth;

fn install() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into()));
    p.exists().then_some(p)
}

fn shaders() -> Option<Vec<CachedShader>> {
    let set = PakSet::open_install(install()?).ok()?;
    let gsc = set.read("Engine/GlobalShaderCache-PCD3D_SM5.bin").ok()?.into_owned();
    Some(shadercache::extract(&gsc))
}

fn find<'a>(all: &'a [CachedShader], name: &str) -> &'a CachedShader { all.iter().find(|s| s.name == name).unwrap_or_else(|| panic!("shader {name} not in cache")) }

#[derive(Clone, Copy)]
enum Mode { Bilinear, Point }

/// texture slot -> (image, filter); gather4 returns component x of the 2x2 block (D3D order x=(i0,j1) y=(i1,j1) z=(i1,j0) w=(i0,j0)).
struct ImgEnv<'a> {
    tex: Vec<(&'a Image, Mode)>,
}
impl Env for ImgEnv<'_> {
    fn texture(&mut self, r: &TexReq) -> [f32; 4] {
        let (img, mode) = self.tex[r.texture as usize];
        match r.op {
            TexOp::Gather4 => {
                let bx = (r.coord[0] * img.w as f32 - 0.5).floor() as i32;
                let by = (r.coord[1] * img.h as f32 - 0.5).floor() as i32;
                [img.load(bx, by + 1)[0], img.load(bx + 1, by + 1)[0], img.load(bx + 1, by)[0], img.load(bx, by)[0]]
            }
            TexOp::Sample | TexOp::SampleL => match mode {
                Mode::Bilinear => img.sample(r.coord[0], r.coord[1]),
                Mode::Point => img.point(r.coord[0], r.coord[1]),
            },
            op => panic!("unexpected texture op {op:?}"),
        }
    }
}

fn scene(w: usize, h: usize) -> synth::Scene { scene_with(w, h, 0.04, 0.15) }

fn scene_with(w: usize, h: usize, yaw_delta: f32, strafe: f32) -> synth::Scene {
    let mut s = synth::build(&synth::SceneOptions { width: w, height: h, yaw_delta, strafe, object_velocity: [0.04, 0.0], ..Default::default() });
    s.color.quantize_f16();
    s
}

fn rel(a: f32, b: f32) -> f32 { (a - b).abs() / b.abs().max(1.0) }

/// Parameters for the UE-unit (cm) setting the original shaders run in: focal 150 m, far transition 500 m, big blur so every branch is hit.
fn cm_params(s: &synth::Scene) -> PostFxParams {
    let mut p = s.params;
    p.cm_per_unit = 1.0;
    // depth in cm: rebuild the transform for centimetres (near 10 cm, far 600000 cm)
    p.depth = DepthTransform::from_perspective(synth::NEAR * 100.0, synth::FAR * 100.0);
    p.dof.focal_distance = 3000.0;
    p.dof.far_transition = 40000.0;
    p.dof.sky_focus_distance = 200000.0;
    p
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u32 { self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (self.0 >> 33) as u32 }
}

#[test]
fn dof_setup_far_matches_dxbc() {
    let Some(all) = shaders() else { eprintln!("no install, skipping"); return };
    let sh = Shader::parse(&find(&all, "FPostProcessDOFSetupPS10").dxbc).unwrap();
    let s = scene(256, 144);
    let p = cm_params(&s);
    let u = cpu::DofUniforms::new(&p);
    // depth must be device z of the cm transform: rebuild from the metre scene via view depth
    let depth = Image::from_fn(s.depth.w, s.depth.h, |x, y| {
        let lin_m = s.params.depth.linearize(s.depth.px[y * s.depth.w + x][0]);
        let d = if lin_m >= synth::FAR * 0.999 { 1.0 } else { p.depth.device_z_from_linear(lin_m * 100.0) };
        [d, 0.0, 0.0, 0.0]
    });
    let reference = cpu::dof_setup_far(&depth, &s.color, &u);
    let mut m = Machine::new(&sh);
    let (ow, oh) = (reference.w, reference.h);
    let mut cb1 = vec![[0.0f32; 4]; 144];
    cb1[44] = p.view.screen_to_world_rows[0];
    cb1[45] = p.view.screen_to_world_rows[1];
    cb1[46] = p.view.screen_to_world_rows[2];
    cb1[57] = [p.depth.x, p.depth.y, p.depth.z, p.depth.w];
    cb1[142] = [0.0, p.dof.focal_distance, 0.0, 0.0];
    cb1[143] = [p.dof.focal_region, p.dof.near_transition, p.dof.far_transition, 0.0];
    let mut cb0 = vec![[0.0f32; 4]; 19];
    cb0[0] = [s.color.w as f32, s.color.h as f32, 1.0 / s.color.w as f32, 1.0 / s.color.h as f32];
    cb0[18] = [p.dof.sky_focus_distance, 0.0, 0.0, 0.0];
    m.set_cb(0, &cb0);
    m.set_cb(1, &cb1);
    let mut worst = 0.0f32;
    let mut touched = 0;
    for yy in 0..oh {
        for xx in 0..ow {
            let uv = [(xx as f32 + 0.5) / ow as f32, (yy as f32 + 0.5) / oh as f32];
            m.set_input(0, [uv[0], uv[1], uv[0] * 2.0 - 1.0, -(uv[1] * 2.0 - 1.0)]);
            let mut env = ImgEnv { tex: vec![(&depth, Mode::Point), (&s.color, Mode::Bilinear)] };
            let out = m.run(&mut env).unwrap();
            let r = reference.px[yy * ow + xx];
            for c in 0..4 { worst = worst.max(rel(out.o[0][c], r[c])); }
            if r[3] > 0.0 { touched += 1; }
        }
    }
    eprintln!("DOFSetupPS10: {} px, {touched} with far weight, worst rel diff {worst:.2e}", ow * oh);
    assert!(touched > 100, "scene never reached the far layer");
    assert!(worst < 1e-4, "CPU port deviates from the DXBC: {worst}");
}

#[test]
fn dof_recombine_far_matches_dxbc() {
    let Some(all) = shaders() else { return };
    let sh = Shader::parse(&find(&all, "FPostProcessDOFRecombinePS100").dxbc).unwrap();
    let s = scene(256, 144);
    let p = cm_params(&s);
    let u = cpu::DofUniforms::new(&p);
    let depth = Image::from_fn(s.depth.w, s.depth.h, |x, y| {
        let lin_m = s.params.depth.linearize(s.depth.px[y * s.depth.w + x][0]);
        [if lin_m >= synth::FAR * 0.999 { 1.0 } else { p.depth.device_z_from_linear(lin_m * 100.0) }, 0.0, 0.0, 0.0]
    });
    // a plausible far layer: the real setup + blur result with a visible radius
    let setup = cpu::dof_setup_far(&depth, &s.color, &u);
    let taps = cpu::gaussian_taps(4.0);
    let far = cpu::blur_pass(&cpu::blur_pass(&setup, &taps, [1.0, 0.0], [1.0; 4], None), &taps, [0.0, 1.0], [1.0; 4], None);
    let reference = cpu::dof_recombine_far(&depth, &s.color, &far, &u);
    let mut m = Machine::new(&sh);
    let mut cb1 = vec![[0.0f32; 4]; 144];
    cb1[57] = [p.depth.x, p.depth.y, p.depth.z, p.depth.w];
    cb1[142] = [0.0, p.dof.focal_distance, 0.0, 0.0];
    cb1[143] = [p.dof.focal_region, p.dof.near_transition, p.dof.far_transition, 0.0];
    let (fw, fh) = (far.w as f32, far.h as f32);
    let mut cb0 = vec![[0.0f32; 4]; 21];
    cb0[0] = [s.color.w as f32, s.color.h as f32, 1.0 / s.color.w as f32, 1.0 / s.color.h as f32];
    cb0[20] = [0.5 / fw, 0.5 / fh, 1.0 - 0.5 / fw, 1.0 - 0.5 / fh];
    m.set_cb(0, &cb0);
    m.set_cb(1, &cb1);
    let mut worst = 0.0f32;
    let mut mixed = 0;
    for y in 0..s.color.h {
        for x in 0..s.color.w {
            let uv = [(x as f32 + 0.5) / s.color.w as f32, (y as f32 + 0.5) / s.color.h as f32];
            m.set_input(0, [uv[0], uv[1], 0.0, 0.0]);
            m.set_input(1, [x as f32 + 0.5, y as f32 + 0.5, 0.0, 0.0]);
            let mut env = ImgEnv { tex: vec![(&depth, Mode::Point), (&s.color, Mode::Bilinear), (&far, Mode::Bilinear)] };
            let out = m.run(&mut env).unwrap();
            let r = reference.px[y * s.color.w + x];
            for c in 0..3 { worst = worst.max(rel(out.o[0][c], r[c])); }
            let c0 = s.color.sample(uv[0], uv[1]);
            if (r[0] - c0[0]).abs() > 1e-3 { mixed += 1; }
        }
    }
    eprintln!("DOFRecombinePS100: {mixed} px changed by the blur, worst rel diff {worst:.2e}");
    assert!(mixed > 100);
    assert!(worst < 1e-4, "{worst}");
}

#[test]
fn bloom_setup_matches_dxbc() {
    let Some(all) = shaders() else { return };
    let sh = Shader::parse(&find(&all, "FPostProcessBloomSetupPS").dxbc).unwrap();
    let s = scene(256, 144);
    let (exposure, threshold) = (1.3f32, 1.0f32);
    let reference = cpu::bloom_setup(&s.color, exposure, threshold);
    let mut m = Machine::new(&sh);
    let mut cb0 = vec![[0.0f32; 4]; 20];
    cb0[19] = [threshold, 0.0, 0.0, 0.0];
    m.set_cb(0, &cb0);
    let mut worst = 0.0f32;
    let mut lit = 0;
    for y in 0..reference.h {
        for x in 0..reference.w {
            let uv = [(x as f32 + 0.5) / reference.w as f32, (y as f32 + 0.5) / reference.h as f32];
            m.set_input(0, [uv[0], uv[1], 0.0, 0.0]);
            m.set_input(1, [exposure, 0.0, 0.0, 0.0]);
            let mut env = ImgEnv { tex: vec![(&s.color, Mode::Bilinear)] };
            let out = m.run(&mut env).unwrap();
            let r = reference.px[y * reference.w + x];
            for c in 0..3 { worst = worst.max(rel(out.o[0][c], r[c])); }
            if r[0] > 0.0 { lit += 1; }
        }
    }
    eprintln!("BloomSetupPS: {lit} px above threshold, worst rel diff {worst:.2e}");
    assert!(lit > 10);
    assert!(worst < 1e-5);
}

fn motion_blur_case(all: &[CachedShader], yaw: f32, strafe: f32, label: &str, need_complex: bool) {
    let sh = Shader::parse(&find(all, "FPostProcessMotionBlurPS4").dxbc).unwrap();
    let s = scene_with(320, 180, yaw, strafe);
    let mut p = s.params;
    p.motion_blur.amount = 1.0;
    p.motion_blur.max_percent = 8.0;
    let u = cpu::MbUniforms::new(&p);
    let mut depth = s.depth.clone();
    depth.quantize_f16();
    let fl = cpu::velocity_flatten(Some(&s.velocity), &depth, &u);
    let tiles = cpu::velocity_gather(&fl.tiles, &u);
    let mut m = Machine::new(&sh);
    let mut cb0 = vec![[0.0f32; 4]; 20];
    cb0[0] = [u.width as f32, u.height as f32, 1.0 / u.width as f32, 1.0 / u.height as f32];
    cb0[3] = [0.0, 0.0, 1.0 / tiles.w as f32, 1.0 / tiles.h as f32];
    cb0[15] = [f32::from_bits(0), f32::from_bits(0), f32::from_bits(u.width as u32), f32::from_bits(u.height as u32)];
    cb0[16] = [u.edge, 0.0, 0.0, 0.0];
    cb0[19] = [0.0, u.scale, 0.0, u.max_px];
    m.set_cb(0, &cb0);
    let mut rng = Lcg(7);
    let mut worst = 0.0f32;
    let (mut early, mut uniform, mut complex) = (0, 0, 0);
    let samples = if need_complex { 60000 } else { 8000 };
    for i in 0..samples {
        let (x, y) = if i % 2 == 0 { ((rng.next() as usize) % u.width, (rng.next() as usize) % u.height) } else {
            // bias half of the samples towards the moving pole and the depth edges
            ((u.width as f32 * 0.50) as usize + (rng.next() as usize) % (u.width / 3), (u.height as f32 * 0.3) as usize + (rng.next() as usize) % (u.height / 2))
        };
        let uv = [(x as f32 + 0.5) / u.width as f32, (y as f32 + 0.5) / u.height as f32];
        m.set_input(0, [uv[0], uv[1], uv[0] * 2.0 - 1.0, -(uv[1] * 2.0 - 1.0)]);
        m.set_input(1, [x as f32 + 0.5, y as f32 + 0.5, 0.0, 0.0]);
        let mut env = ImgEnv { tex: vec![(&s.color, Mode::Bilinear), (&fl.flat, Mode::Point), (&tiles, Mode::Point)] };
        let out = m.run(&mut env).unwrap();
        let r = cpu::motion_blur_pixel(&s.color, &fl.flat, &tiles, &u, x, y);
        let mut d = 0.0f32;
        for c in 0..3 { d = d.max(rel(out.o[0][c], r[c])); }
        worst = worst.max(d);
        // which branch ran (same tile fetch as the shader)
        let ign = |px: f32, py: f32| { let a = (px * 0.06711056 + py * 0.00583715).fract(); (a * 52.9829189).fract() };
        let (n1, n2) = (ign(x as f32, y as f32), ign(x as f32 + 32.665, y as f32 + 11.815));
        let tc = [((x as f32 + 0.5) * 0.0625 + (n1 - 0.5) * 0.5).min(u.width as f32), ((y as f32 + 0.5) * 0.0625 + (n2 - 0.5) * 0.5).min(u.height as f32)];
        let t = tiles.point(tc[0] / tiles.w as f32, tc[1] / tiles.h as f32);
        let vmax = (t[2] * u.scale).powi(2) + (t[3] * u.scale).powi(2);
        let vmin = (t[0] * u.scale).powi(2) + (t[1] * u.scale).powi(2);
        if vmax < 0.25 { early += 1 } else if vmax * 0.4 < vmin { uniform += 1 } else { complex += 1 }
    }
    eprintln!("MotionBlurPS4 [{label}]: {samples} px (early-out {early}, uniform branch {uniform}, weighted branch {complex}), worst rel diff {worst:.2e}");
    assert!(uniform + complex > 100);
    if need_complex { assert!(complex > 300, "weighted branch not exercised"); }
    assert!(worst < 2e-4, "CPU port deviates from the DXBC: {worst}");
}

#[test]
fn motion_blur_ps4_matches_dxbc() {
    let Some(all) = shaders() else { return };
    motion_blur_case(&all, 0.04, 0.15, "camera + object", false);
    motion_blur_case(&all, 0.0, 0.0, "object only (static camera)", true);
}