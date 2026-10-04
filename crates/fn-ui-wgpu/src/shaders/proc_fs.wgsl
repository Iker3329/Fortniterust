// Hand translation of the Esc-menu procedural materials (fn-ui menu/outline.rs outline_pixel / primary_pixel / panel_pixel).
// kind (P[3].z): 0 outline family, 1 Primary-M button, 2 ShellUberBG backdrop, 3 gradient panel (modal body / header).
// P[4] = (margin_px_x, margin_px_y, rect_w_px, rect_h_px), P[5] = (margin_uv_x, margin_uv_y, 0, 0).
// Outline (kind 0): P[6] = (primary.rgb, dark_base_alpha), P[7] = (base.rgb, gradient_alpha),
//                   P[8] = (gradient_min_alpha, outline_alpha, inner_fill_offset, switch_offset), P[9].x = box_size.
// Primary-M (kind 1): P[6] = (rgba0.rgb, alpha), P[7] = (rgba1.rgb, 0), P[8] = (drop shadow rgb, 0).
// Gradient panel (kind 3): P[6] = (rgba0.rgb, 0), P[7] = (rgba1.rgb, 0).
fn smooth01(x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
fn lin_to_srgb1(c0: f32) -> f32 {
    let c = clamp(c0, 0.0, 1.0);
    if (c <= 0.0031308) { return c * 12.92; }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}
fn lin_to_srgb3(c: vec3<f32>) -> vec3<f32> { return vec3<f32>(lin_to_srgb1(c.x), lin_to_srgb1(c.y), lin_to_srgb1(c.z)); }

fn slice_uv(p: f32, size: f32, margin_px: f32, margin_uv: f32) -> f32 {
    let m = max(min(margin_px, size * 0.5), 1e-4);
    if (p < m) { return p / m * margin_uv; }
    if (p > size - m) { return 1.0 - (size - p) / m * margin_uv; }
    let mid = max(size - 2.0 * m, 1e-4);
    return margin_uv + (p - m) / mid * (1.0 - 2.0 * margin_uv);
}

fn ceil01(x: f32) -> f32 { if (x > 0.0) { return 1.0; } return 0.0; }

fn outline_pixel(u: f32, v: f32) -> vec4<f32> {
    let primary = P[6].xyz; let dark_base_alpha = P[6].w;
    let base = P[7].xyz; let gradient_alpha = P[7].w;
    let gmin = P[8].x; let outline_alpha = P[8].y; let inner_fill = P[8].z; let switch_off = P[8].w;
    let box_size = P[9].x;
    let cx = abs(u - 0.5) - box_size;
    let cy = abs(v - 0.5) - box_size;
    let band = min(length(vec2<f32>(ceil01(cx), ceil01(cy))), 1.0);
    var fill = switch_off;
    if (u > inner_fill) { fill = 1.0 - switch_off; }
    let s = smooth01(v);
    let g = (1.0 - s) + gmin * s;
    let a0 = (1.0 - band) * g * gradient_alpha + band * outline_alpha;
    let rgb = lin_to_srgb3(mix(primary, base, (1.0 - band) * s * dark_base_alpha));
    return vec4<f32>(rgb, clamp(clamp(band + fill, 0.0, 1.0) * a0, 0.0, 1.0));
}

const PM_SIZE: f32 = 0.3;
const PM_SHADOW_DIST: f32 = 0.22;
const PM_SS_MIN: f32 = 0.18;
const PM_SS_MAX: f32 = 0.15;
const PM_SHADOW_ALPHA: f32 = 1.0;

fn pm_sdf(px: f32, py: f32, ex: f32, ey: f32) -> f32 {
    return length(vec2<f32>(max(abs(px) - ex, 0.0), max(abs(py) - ey, 0.0)));
}
fn pm_inv(d: f32) -> f32 { return smooth01((d - PM_SS_MIN) / (PM_SS_MAX - PM_SS_MIN)); }

fn primary_pixel(u: f32, v: f32, v1: f32) -> vec4<f32> {
    let rgba0 = P[6].xyz; let alpha = P[6].w;
    let rgba1 = P[7].xyz;
    let shadow = P[8].xyz;
    let px = u - 0.5;
    let py = v - 0.5;
    var gate = 0.0;
    if (v + 0.7 >= 1.0) { gate = 1.0; }
    let shadow_mix = (1.0 - pm_inv(pm_sdf(px, py, PM_SIZE, PM_SHADOW_DIST))) * gate * PM_SHADOW_ALPHA;
    let face = pm_inv(pm_sdf(px, py, PM_SIZE, PM_SIZE));
    let g = smooth01(v1);
    let base = mix(rgba0, rgba1, g);
    let c = mix(mix(shadow, base, face), shadow, shadow_mix);
    let rgb = lin_to_srgb3(max(c, vec3<f32>(0.0)));
    return vec4<f32>(rgb, clamp(face * alpha, 0.0, 1.0));
}

fn backdrop_pixel(u: f32, v: f32) -> vec4<f32> {
    // ShellUberBG sphere gradient (UBER_* constants of outline.rs).
    let rgba0 = vec4<f32>(0.0, 0.002125, 0.017642, 1.0);
    let rgba1 = vec4<f32>(0.0, 0.004391, 0.064803, 0.6);
    let d = length(vec2<f32>(u - 1.0, v - 2.2));
    let n = 1.0 - d / 2.0;
    let y = (n * 1.0) * (n * 1.0) * 1.442695;
    let w = max(1.0 - 1.0 / exp(y), 1e-6);
    var lg = -19.931568;
    if (n >= 0.0 && abs(n) > 1e-5) { lg = log(w); }
    let t = clamp(exp(lg * 0.54317) * 1.886526, 0.0, 1.0);
    let rgb = lin_to_srgb3(mix(rgba0.xyz, rgba1.xyz, t));
    return vec4<f32>(rgb, clamp(clamp(mix(rgba0.w, rgba1.w, t), 0.0, 1.0), 0.0, 1.0));
}

fn gradient_panel_pixel(u: f32, v: f32) -> vec4<f32> {
    let a = 1.0 * 0.25;
    let dir = vec2<f32>(cos(a), -sin(a));
    let x = dir.x * (u - 0.5) + dir.y * (v - 0.5) + 0.5;
    let t = smooth01(x);
    return vec4<f32>(lin_to_srgb3(mix(P[6].xyz, P[7].xyz, t)), 1.0);
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let lin = in.lin;
    let kind = u32(P[3].z + 0.5);
    var c: vec4<f32>;
    if (kind <= 1u) {
        let pos = lin * P[4].zw;
        let u = slice_uv(pos.x, P[4].z, P[4].x, P[5].x);
        let v = slice_uv(pos.y, P[4].w, P[4].y, P[5].y);
        if (kind == 0u) { c = outline_pixel(u, v); } else { c = primary_pixel(u, v, clamp(lin.y, 0.0, 1.0)); }
    } else if (kind == 2u) {
        c = backdrop_pixel(lin.x, lin.y);
    } else {
        c = gradient_panel_pixel(lin.x, lin.y);
    }
    return finish(c);
}
