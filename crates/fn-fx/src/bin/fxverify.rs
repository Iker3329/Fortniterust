//! Debug / verification: runs the vertex shader of a number section on the CPU interpreter for every vertex of the first digit quads and prints the clip positions.
use fn_dxbc::interp::{Env, Machine, TexOp, TexReq};
use fn_fx::gpu::{TextureCache, Vf};
use fn_fx::numbers::{mid_params, number_location, CamUe, NumberBp};
use fn_fx::view::ViewParams;
use glam::Vec3;

struct Tex { slots: Vec<Option<(usize, usize, Vec<u8>, bool)>> }
impl Env for Tex {
    fn texture(&mut self, r: &TexReq) -> [f32; 4] {
        let Some(Some((w, h, px, nearest))) = self.slots.get(r.texture as usize) else { return [0.; 4] };
        if matches!(r.op, TexOp::Resinfo) { return [*w as f32, *h as f32, 1., 1.]; }
        let (u, v) = (r.coord[0], r.coord[1]);
        let fetch = |x: i64, y: i64| { let (x, y) = (x.rem_euclid(*w as i64) as usize, y.rem_euclid(*h as i64) as usize); let o = (y * w + x) * 4; [px[o] as f32 / 255., px[o + 1] as f32 / 255., px[o + 2] as f32 / 255., px[o + 3] as f32 / 255.] };
        if *nearest { return fetch((u * *w as f32).floor() as i64, (v * *h as f32).floor() as i64); }
        let (fx, fy) = (u * *w as f32 - 0.5, v * *h as f32 - 0.5);
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64); let (ax, ay) = (fx - x0 as f32, fy - y0 as f32);
        let (a, b, c, d) = (fetch(x0, y0), fetch(x0 + 1, y0), fetch(x0, y0 + 1), fetch(x0 + 1, y0 + 1));
        std::array::from_fn(|k| (a[k] * (1. - ax) + b[k] * ax) * (1. - ay) + (c[k] * (1. - ax) + d[k] * ax) * ay)
    }
}

fn main() -> Result<(), String> {
    let set = fn_pak::PakSet::open_install(fn_fx::default_install()).map_err(|e| e.to_string())?;
    let crit = std::env::args().nth(1).map_or(false, |a| a == "crit");
    let bp = NumberBp::load(&set)?;
    let mesh_path = if crit { &bp.crit_building_mesh } else { &bp.standard_mesh };
    let (a, x, b) = fn_fx::data::package_files(&set, fn_fx::data::long_package(mesh_path))?;
    let p = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| e.to_string())?;
    let m = fn_mesh::StaticMesh::parse(&p).map_err(|e| e.to_string())?;
    let lod = &m.lods[0];
    let mat_path = m.materials[0].material.clone().unwrap();
    let mat = fn_fx::material::Material::load(&set, &mat_path)?;
    let vs = mat.cooked.find(Vf::Local.type_name(), "TBasePassVSFNoLightMapPolicy").ok_or("vs")?;
    // camera + number as in the demo
    let view = ViewParams { eye: Vec3::new(0., 1.6, 0.), forward: Vec3::new(0., -0.14, -1.).normalize(), up: Vec3::Y, tan_half: (0.839, 0.4719), near_m: 0.1, far_m: 3000., width: 960, height: 540, time: 0.2, real_time: 0.2, delta_world: 0.0 };
    let cam = CamUe::from_view(&view);
    let spot = Vec3::new(0.15 - 0.35, 1.1, -2.5);
    let mut rng = 11u64;
    let loc = number_location(fn_fx::view::to_ue(spot), &cam, 2, &mut rng);
    let digits = [0u8, 5, 0];
    let params = mid_params(&bp, 2, crit, &digits, &cam, loc, 0.1, &mut rng);
    for (n, v) in params.vectors.iter().take(9) { println!("param {n} = {v:?}"); }
    let ov = params.apply(&mat.overrides);
    let packed = fn_fx::uniform::evaluate(&mat.uniform, &ov, 0.2);
    let vb = view.view_buffer();
    let mb = fn_render::primitive_ub::BoxSphereBounds { origin: m.bounds.origin, extent: m.bounds.extent, radius: m.bounds.radius };
    let l2w = fn_fx::view::axes_matrix(loc, [cam.fwd, cam.right, cam.up]);
    let prim = fn_render::primitive_ub::mesh_primitive(&l2w, &mb, l2w.w_axis.truncate().to_array(), &fn_render::primitive_ub::PrimitiveFlags::default());
    // textures of the vertex shader
    let mut cache = TextureCache::default();
    let _ = &mut cache;
    let mut slots: Vec<Option<(usize, usize, Vec<u8>, bool)>> = vec![None; 8];
    for bnd in &vs.bindings {
        if bnd.kind == fn_dxbc::unreal_packet::ResourceKind::Texture {
            let k = (bnd.binding.member / 2) as usize;
            let (path, _) = mat.texture_slot(k).ok_or("slot")?;
            let (a, x, b) = fn_fx::data::package_files(&set, fn_fx::data::long_package(&path))?;
            let pk = fn_ueio::Package::parse(&a, &x, &b).map_err(|e| e.to_string())?;
            let t = fn_texture::Texture2D::parse(&pk).map_err(|e| e.to_string())?;
            let mip = t.best_mip().unwrap();
            let img = t.decode(mip, fn_texture::DecodeOptions::default()).map_err(|e| e.to_string())?;
            println!("vs t{} = {} ({}x{} {})", bnd.binding.register, path, img.width, img.height, t.compression_settings);
            slots[bnd.binding.register as usize] = Some((img.width, img.height, img.to_rgba8(), false));
        }
    }
    let mut env = Tex { slots };
    for vtx in 0..lod.positions.len().min(24) {
        let mut mach = Machine::new(&vs.shader);
        let u = |b: u8| b as f32 / 255.;
        mach.set_input(0, [lod.positions[vtx][0], lod.positions[vtx][1], lod.positions[vtx][2], 1.]);
        mach.set_input(1, [u(lod.tangent_x[vtx][0]), u(lod.tangent_x[vtx][1]), u(lod.tangent_x[vtx][2]), u(lod.tangent_x[vtx][3])]);
        mach.set_input(2, [u(lod.tangent_z[vtx][0]), u(lod.tangent_z[vtx][1]), u(lod.tangent_z[vtx][2]), u(lod.tangent_z[vtx][3])]);
        let (uv0, uv1) = (lod.uvs[0][vtx], lod.uvs[1][vtx]);
        mach.set_input(4, [uv0[0], uv0[1], uv1[0], uv1[1]]);
        mach.set_cb(0, &vec![[0.; 4]; 7]); mach.set_cb(1, &vb); mach.set_cb(2, &prim); mach.set_cb(3, &packed.cb);
        let out = mach.run(&mut env).map_err(|e| format!("{e:?}"))?;
        let clip = out.o[5];
        println!("v{vtx:2} pos {:?} uv0 {:?} uv1 {:?} -> clip {:?} ndc ({:.3}, {:.3}, {:.4}) w {:.1}", lod.positions[vtx].map(|f| (f * 10.).round() / 10.), uv0, uv1, clip.map(|f| (f * 100.).round() / 100.), clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3], clip[3]);
    }
    Ok(())
}
