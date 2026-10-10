#![allow(dead_code, clippy::all)]
fn vadd(a: [f32; 4], b: [f32; 4]) -> [f32; 4] { [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]] }
fn vswz(a: [f32; 4], i: [i8; 4]) -> [f32; 4] { let g = |k: i8| if k >= 0 { a[k as usize] } else { 0.0 }; [g(i[0]), g(i[1]), g(i[2]), g(i[3])] }
fn vapp(a: [f32; 4], b: [f32; 4], n: usize) -> [f32; 4] { let mut o = [0.0_f32; 4]; for k in 0..4 { o[k] = if k < n { a[k] } else { b[k - n] }; } o }
pub fn minimap_storms_cb(p: &dyn Fn(&str, [f32; 4]) -> [f32; 4]) -> [[f32; 4]; 11] {
    let mut cb = [[0.0_f32; 4]; 11];
    cb[0] = p("SelectionColor", [0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32]);
    cb[1] = p("DangerLineColor", [0.298849_f32, 0.031443_f32, 0.515_f32, 1.0_f32]);
    cb[2] = p("TintOutsideSafeZone", [0.580289_f32, 0.061054_f32, 1.0_f32, 0.6_f32]);
    cb[3] = vswz(p("TintOutsideSafeZone", [0.580289_f32, 0.061054_f32, 1.0_f32, 0.6_f32]), [0, 1, 2, -1]);
    cb[4] = p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]);
    cb[5] = vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]);
    cb[6] = vapp(vswz(vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]), [0, 0, 0, 0]), vswz(vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]), [1, 1, 1, 1]), 1);
    cb[7] = vswz(p("DangerLineColor", [0.298849_f32, 0.031443_f32, 0.515_f32, 1.0_f32]), [0, 1, 2, -1]);
    cb[8] = vswz(p("SelectionColor", [0.0_f32, 0.0_f32, 0.0_f32, 1.0_f32]), [0, 1, 2, -1]);
    cb[9][0] = vswz(p("TintOutsideSafeZone", [0.580289_f32, 0.061054_f32, 1.0_f32, 0.6_f32]), [3, 3, 3, 3])[0];
    cb[9][1] = vswz(vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]), [1, 1, 1, 1])[0];
    cb[9][2] = vswz(vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]), [0, 0, 0, 0])[0];
    cb[9][3] = vswz(vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]), [2, 2, 2, 2])[0];
    cb[10][0] = vadd(vswz(vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]), [2, 2, 2, 2]), [0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32])[0];
    cb[10][1] = vadd(vswz(vswz(p("CurrentCircle", [0.307339_f32, 0.7_f32, 0.144954_f32, 1.0_f32]), [0, 1, 2, -1]), [2, 2, 2, 2]), [0.02_f32, 0.02_f32, 0.02_f32, 0.02_f32])[0];
    cb[10][2] = p("RefractionDepthBias", [0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32])[0];
    cb
}
