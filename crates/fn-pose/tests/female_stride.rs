use fn_animgraph::{AnimState, PawnInput};
use fn_pose::{loader, BoneTree, ClipSourceDir, ClipStore, EvalOptions, PoseEvaluator};
use glam::{Mat4, Vec3};
use std::path::Path;

const PLANT_BAND: f32 = 0.012;
const FEMALE_HERO: &str = "/Game/Athena/Heroes/HID_001_Athena_Commando_F";

struct Feet { idx: [(usize, usize); 2], heel_local: [Vec3; 2], ball_drop: f32, fwd: Vec3, right: Vec3 }
impl Feet {
    fn new(tree: &BoneTree) -> Feet {
        let g = tree.globals(&tree.bind_pose());
        let (mut idx, mut heel_local, mut drop, mut fwd_sum) = ([(0, 0); 2], [Vec3::ZERO; 2], 0., Vec3::ZERO);
        for (k, s) in ["l", "r"].iter().enumerate() {
            let fi = tree.bone_index(&format!("foot_{s}")).unwrap();
            let bi = tree.bone_index(&format!("ball_{s}")).unwrap();
            let ankle = g[fi].transform_point3(Vec3::ZERO);
            let ball = g[bi].transform_point3(Vec3::ZERO);
            let fwd = Vec3::new(ball.x - ankle.x, 0., ball.z - ankle.z).normalize();
            heel_local[k] = g[fi].inverse().transform_point3(Vec3::new(ankle.x - fwd.x * 0.05, 0., ankle.z - fwd.z * 0.05));
            idx[k] = (fi, bi);
            drop = ball.y;
            fwd_sum += fwd;
        }
        let fwd = fwd_sum.normalize();
        let (l, r) = (g[idx[0].0].transform_point3(Vec3::ZERO), g[idx[1].0].transform_point3(Vec3::ZERO));
        let left = Vec3::new(l.x - r.x, 0., l.z - r.z).normalize();
        Feet { idx, heel_local, ball_drop: drop, fwd, right: -left }
    }
    fn points(&self, g: &[Mat4], k: usize) -> [Vec3; 3] {
        let (fi, bi) = self.idx[k];
        [g[fi].transform_point3(Vec3::ZERO), g[fi].transform_point3(self.heel_local[k]), g[bi].transform_point3(Vec3::ZERO) - Vec3::new(0., self.ball_drop, 0.)]
    }
}
fn contact_id(f: &[Vec3; 3]) -> usize { if f[1].y <= f[2].y { 1 } else { 2 } }

fn clip_direction(name: &str) -> (f32, f32) {
    let s = std::f32::consts::FRAC_1_SQRT_2;
    match name.split('_').find(|t| matches!(*t, "N" | "S" | "E" | "W" | "NE" | "SE" | "NW" | "SW")) {
        Some("N") => (1., 0.), Some("S") => (-1., 0.), Some("E") => (0., 1.), Some("W") => (0., -1.), Some("NE") => (s, s), Some("SE") => (-s, s), Some("NW") => (s, -s), Some("SW") => (-s, -s), _ => (1., 0.),
    }
}

fn planted(tree: &BoneTree, clips: &ClipStore, feet: &Feet, female: bool, vel: (f32, f32), sprint: bool) -> f32 {
    let speed = (vel.0 * vel.0 + vel.1 * vel.1).sqrt();
    let u = (vel.0 / speed, vel.1 / speed);
    let p = PawnInput { velocity: [vel.0, vel.1, 0.], accel: [750. * u.0, 750. * u.1], crouched: false, sprinting: sprint, ..PawnInput::melee() };
    let mut a = AnimState::new();
    a.set_female(female);
    a.stride_match = false;
    let dt = 1. / 240.;
    let vdir = feet.fwd * u.0 + feet.right * u.1;
    let (warm, total) = (720, 1200);
    let mut fr: Vec<[[Vec3; 3]; 2]> = vec![];
    let mut g = vec![];
    for _ in 0..total {
        a.step(&p, dt);
        let ev = PoseEvaluator::new(tree, clips, EvalOptions::default());
        let pose = ev.eval_pose(&a.plan(&[]));
        tree.globals_into(&pose, &mut g);
        fr.push([feet.points(&g, 0), feet.points(&g, 1)]);
    }
    let mut low = [9f32; 2];
    for f in &fr[warm..] { for k in 0..2 { low[k] = low[k].min(f[k][contact_id(&f[k])].y); } }
    let (mut sc, mut n) = (0f32, 0usize);
    for i in warm..total {
        for k in 0..2 {
            let id = contact_id(&fr[i][k]);
            let back = -(fr[i][k][id] - fr[i - 1][k][id]).dot(vdir) / dt;
            if fr[i][k][id].y < low[k] + PLANT_BAND && back > 0. { sc += back; n += 1; }
        }
    }
    if n == 0 { return 0.; }
    sc / n as f32 * 100. / a.vars.speed_adjusted_playrate
}

fn axis_value(tree: &BoneTree, clips: &ClipStore, feet: &Feet, female: bool, name: &str, sprint: bool) -> f32 {
    let d = clip_direction(name);
    let ang = d.1.atan2(d.0).to_degrees().abs();
    let speed = if sprint { 550. } else { 410. * if ang <= 45.5 { 1. } else if ang <= 100. { 0.72 } else { 0.65 } };
    planted(tree, clips, feet, female, (speed * d.0, speed * d.1), sprint)
}

#[test]
fn female_stride_table() {
    let root = std::env::var("FN_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
    let Ok(set) = fn_pak::PakSet::open_install(root) else { eprintln!("no install: skipped"); return };
    let src = ClipSourceDir::Pak(&set);
    let skel = fn_pose::clip::load_player_skeleton_info(&src).unwrap();
    let mut names = fn_animgraph::required_clips();
    for n in fn_animgraph::female_clips() { if !names.contains(&n) { names.push(n); } }
    let clips = ClipStore::load(&src, &names, &skel, fn_pose::SampleMode::Ue).unwrap();
    let male_master = fn_pose::Part::from_skeletal_mesh("m", &fn_mesh::SkeletalMesh::parse(&loader::package(&src, loader::MASTER_PKG).unwrap()).unwrap(), 0).unwrap();
    let ch = fn_pose::br_character::BrCharacter::load(&src, FEMALE_HERO).unwrap();
    let female_master = ch.master.unwrap();
    assert!(ch.female);
    let mf = Feet::new(&male_master.tree);
    for n in ["M_Jog_AssaultRifle_N_Core", "M_Jog_AssaultRifle_E_Core", "M_Jog_AssaultRifle_S_Core"] {
        let v = axis_value(&male_master.tree, &clips, &mf, false, n, false);
        println!("male check {n:<40} measured {v:>7.2}  table {:?}", fn_animgraph::stride_cm_s(n));
    }
    let ff = Feet::new(&female_master.tree);
    let mut rows: Vec<(String, f32)> = vec![];
    for b in ["AssaultRifle_NonTargeted_CMF_COR", "AssaultRifle_NonTargeted_CMF_COR_No45s"] {
        for s in fn_animgraph::blendspace(b).unwrap().samples {
            if rows.iter().any(|r| r.0 == s.anim) { continue; }
            let v = axis_value(&female_master.tree, &clips, &ff, true, s.anim, false);
            rows.push((s.anim.to_owned(), v));
        }
    }
    let v = axis_value(&female_master.tree, &clips, &ff, true, "Sprint_TwoHanded_Melee_CMF", true);
    rows.push(("Sprint_TwoHanded_Melee_CMF".into(), v));
    for (n, v) in &rows { println!("female {n:<40} {v:>7.2}"); }
    if std::env::var("FNRT_WRITE_STRIDE_FEMALE").is_ok() {
        let mut s = String::from("pub static CLIP_STRIDE_FEMALE_CM_S:&[(&str,f32)]=&[\n");
        for (n, v) in &rows { s += &format!("    (\"{n}\",{v:.2}),\n"); }
        s += "];\n";
        std::fs::write(Path::new(env!("CARGO_MANIFEST_DIR")).join("../fn-animgraph/src/anim_stride_female.rs"), s).unwrap();
        return;
    }
    for (n, v) in rows {
        let t = fn_animgraph::stride_cm_s_female(&n).unwrap_or_else(|| panic!("{n} missing from anim_stride_female.rs"));
        assert!((t - v).abs() <= 0.01 * v.max(1.) + 0.05, "{n}: table {t} vs measured {v}");
    }
}
