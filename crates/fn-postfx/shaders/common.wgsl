// fn-postfx common: full-screen triangle (uv with y DOWN like D3D, as in the UE 4.16 post-process shaders) and shared helpers.
// All pixel-shader ports below are line-for-line translations of the DXBC in Engine/GlobalShaderCache-PCD3D_SM5.bin of the 1.8 install
// (see reports/postfx-shaders/*.asm); the register names of the original are kept in comments (rN) where it helps to diff against them.

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32((i >> 1u) & 1u) * 4 - 1);
    var o: VsOut;
    o.pos = vec4<f32>(x, y, 0.0, 1.0);
    o.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return o;
}

// ConvertFromDeviceZ (View.InvDeviceZToWorldZTransform): z * x + y + 1 / (z * z_ - w)
fn convert_from_device_z(d: f32, t: vec4<f32>) -> f32 {
    return d * t.x + t.y + 1.0 / (d * t.z - t.w);
}

fn sat(x: f32) -> f32 { return clamp(x, 0.0, 1.0); }
