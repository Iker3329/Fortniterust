//! UE POSE CAMERA (dev, 2026-10-07): `FN_CAM_UE=X,Y,Z,PITCH,YAW[,FOV]` places the render camera at a real-client pose (UE world cm, FRotator degrees, horizontal
//! FOV, default 80 = the reference captures' `DebugCameraController.FOV 80`), so port shots line up with the 1.8 reference shots
//! (C:\FortniteRef\lighting_ref\match\pose_*.json). Mapping = the landscape's (fn_terrain `Mapping::to_game`: `--world-center` sets its centre, use the pose's
//! X,Y so props / grass surround the camera). Needs the landscape (backdrop) to be loaded; otherwise ignored. UE yaw 0 = +X (north), 90 = +Y (east).
use fn_render::camera::CameraParams;
use glam::Vec3;

fn pose() -> Option<[f32; 6]> {
    static P: std::sync::OnceLock<Option<[f32; 6]>> = std::sync::OnceLock::new();
    *P.get_or_init(|| {
        let s = fn_live::var("FN_CAM_UE").ok()?;
        let v: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() < 5 { eprintln!("[cam pose] FN_CAM_UE={s:?}: expected X,Y,Z,PITCH,YAW[,FOV]"); return None; }
        Some([v[0], v[1], v[2], v[3], v[4], v.get(5).copied().unwrap_or(80.)])
    })
}

fn anim() -> Option<[f32; 6]> {
    static A: std::sync::OnceLock<Option<[f32; 6]>> = std::sync::OnceLock::new();
    *A.get_or_init(|| {
        let v: Vec<f32> = fn_live::var("FN_CAM_UE_ANIM").ok()?.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() < 5 { return None; }
        Some([v[0], v[1], v[2], v[3], v[4], v.get(5).copied().unwrap_or(0.)])
    })
}

fn path() -> &'static [(u32, [f32; 6])] {
    static P: std::sync::OnceLock<Vec<(u32, [f32; 6])>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let Some(f) = fn_live::var_os("FN_CAM_PATH") else { return vec![] };
        let text = std::fs::read_to_string(&f).unwrap_or_else(|e| { eprintln!("[cam path] {f:?}: {e}"); String::new() });
        let mut v: Vec<(u32, [f32; 6])> = text.lines().filter_map(|l| {
            let t: Vec<f32> = l.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (t.len() >= 6).then(|| (t[0] as u32, [t[1], t[2], t[3], t[4], t[5], t.get(6).copied().unwrap_or(80.)]))
        }).collect();
        v.sort_by_key(|r| r.0);
        eprintln!("[cam path] {} poses from {f:?}", v.len());
        v
    })
}

fn path_pose(frame: Option<u32>) -> Option<[f32; 6]> {
    let p = path();
    let (first, last) = (p.first()?, p.last()?);
    let f = frame.unwrap_or(0);
    Some(if f <= first.0 { first.1 } else if f >= last.0 { last.1 } else { p[p.partition_point(|r| r.0 <= f) - 1].1 })
}

/// Overrides `c` with the `FN_CAM_UE` pose (no-op without it).
pub fn apply(c: &mut CameraParams, frame: Option<u32>) {
    let Some(w) = fn_terrain::world() else { return };
    let Some(mut p) = path_pose(frame).or_else(pose) else { return };
    if let Some(a) = anim() {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let k = (N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as f32 - a[5]).max(0.);
        p[3] += a[0] * k; p[4] += a[1] * k; p[0] += a[2] * k; p[1] += a[3] * k; p[2] += a[4] * k;
        eprintln!("[cam pose] anim k={k} pose={:.4},{:.4},{:.4},{:.5},{:.5}", p[0], p[1], p[2], p[3], p[4]);
    }
    let (pi, ya) = (p[3].to_radians(), p[4].to_radians());
    let f = Vec3::new(pi.cos() * ya.cos(), pi.cos() * ya.sin(), pi.sin());
    let u = Vec3::new(-pi.sin() * ya.cos(), -pi.sin() * ya.sin(), pi.cos());
    c.eye = w.map.to_game(Vec3::new(p[0], p[1], p[2]));
    c.forward = Vec3::new(f.x, f.z, f.y);
    c.up = Vec3::new(u.x, u.z, u.y);
    c.fov_deg = p[5];
}
