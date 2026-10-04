// Tiled motion blur of UE 4.16 (Fortnite 1.8), three compiled shaders ported line by line:
//   cs_flatten     = FPostProcessVelocityFlattenCS  (reports/postfx-shaders/FPostProcessVelocityFlattenCS__755867.asm)
//   cs_gather      = FPostProcessVelocityGatherCS   (...__775918.asm)
//   fs_motion_blur = FPostProcessMotionBlurPS<4>    (...__902870.asm; quality 4 = the r.MotionBlurQuality default, 16 taps)
// Differences from the DXBC (all numerically equivalent):
//   * camera velocity of static pixels is computed from `clip_to_prev` (the DXBC evaluates the same projection expanded into the polynomial
//     coefficients `FCameraMotionParameters` cb2[0..4], which C++ derives from the same matrices);
//   * the velocity texture is a float texture holding the DECODED NDC velocity with alpha = 1 where a dynamic object wrote it (the DXBC
//     reads the UE-encoded RG texture: v = enc * 4.008016 - 2.003978 and tests enc.x > 0 for "written");
//   * workgroupBarrier() between every reduction step (the DXBC relies on warp-synchronous execution below 32 threads).
// Layout of the flattened velocity texture (rgba32float): x = |v| (W-normalised NDC units), y = atan2(v)/2pi + 0.5, z = w = linear depth (cm).
// Layout of the tile textures (rgba32float): xy = min-length velocity vector, zw = max-length velocity vector (same units as |v|).
// Prepended: common.wgsl.

struct MbU {
    size: vec4<f32>,       // cb0[0]: w, h, 1/w, 1/h of the colour image
    tile_inv: vec4<f32>,   // cb0[3]: zw = 1 / tile texture size
    rect: vec4<u32>,       // cb0[15]: view rect min.xy, max.zw (pixels)
    edge: vec4<f32>,       // cb0[16]: x = NDC edge scale (pixels per NDC unit along the dominant axis)
    p18: vec4<f32>,        // flatten/gather: x = H/W, y = velocity scale, z = velocity scale / 16, w = max velocity (px)
    p19: vec4<f32>,        // motion blur PS: y = velocity scale (px per W-normalised unit), w = max velocity (px)
    depth_xf: vec4<f32>,   // InvDeviceZToWorldZTransform
    misc: vec4<f32>,       // x = centimetres per renderer unit
    clip_to_prev: mat4x4<f32>,
    flags: vec4<u32>,      // x = velocity texture bound
    tile_grid: vec4<u32>,  // gather: xy = tile grid size
}

@group(0) @binding(0) var<uniform> u: MbU;
@group(0) @binding(1) var vel_tex: texture_2d<f32>;
@group(0) @binding(2) var depth_tex: texture_depth_2d;
@group(0) @binding(3) var flat_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(4) var tile_out: texture_storage_2d<rgba32float, write>;
@group(0) @binding(5) var tile_in: texture_2d<f32>;
@group(0) @binding(6) var color_tex: texture_2d<f32>;
@group(0) @binding(7) var flat_tex: texture_2d<f32>;
@group(0) @binding(8) var samp: sampler;

var<workgroup> g0: array<vec4<f32>, 256>;

fn atan2_fast(y: f32, x: f32) -> f32 {
    // the polynomial of the compiled shader: atan(a) ~ a * (1 + a^2 * (-0.301895 + 0.087293 a^2)), a = min/max of |x|,|y|
    let ax = abs(x);
    let ay = abs(y);
    let mx = max(ax, ay);
    let mn = min(ax, ay);
    if (mx == 0.0) { return 0.0; }
    let a = mn / mx;
    let a2 = a * a;
    let p = (a2 * 0.087293 - 0.301895) * a2 + 1.0;
    var r = a * p;
    if (ax < ay) { r = 1.570796 - p * a; }
    if (x < 0.0) { r = 3.141593 - r; }
    if (y < 0.0) { r = -r; }
    return r;
}

@compute @workgroup_size(16, 16, 1)
fn cs_flatten(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) gid: vec3<u32>, @builtin(global_invocation_id) tid: vec3<u32>) {
    let pix = vec2<i32>(tid.xy) + vec2<i32>(u.rect.xy);
    let dims = vec2<i32>(textureDimensions(depth_tex));
    let pc = clamp(pix, vec2<i32>(0), dims - vec2<i32>(1));
    var venc = vec4<f32>(0.0);
    if (u.flags.x != 0u) { venc = textureLoad(vel_tex, pc, 0); }
    let dev_z = textureLoad(depth_tex, pc, 0);
    let written = venc.a > 0.5;
    let view_size = vec2<f32>(vec2<i32>(u.rect.zw) - vec2<i32>(u.rect.xy));
    let uv = (vec2<f32>(tid.xy) + vec2<f32>(0.5)) / view_size;
    // velocity of static pixels from depth + the previous view-projection (NDC, y up)
    let cur = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, dev_z, 1.0);
    let prev = u.clip_to_prev * cur;
    var vcam = vec2<f32>(0.0);
    if (abs(prev.w) > 1e-6) { vcam = cur.xy - prev.xy / prev.w; }
    let v = select(vcam, venc.xy, written);
    let vy = v.y * -u.p18.x;
    let len = sqrt(v.x * v.x + vy * vy);
    let ang = atan2_fast(vy, v.x);
    let inside = (u32(pix.x) < u.rect.z) && (u32(pix.y) < u.rect.w);
    let lin = convert_from_device_z(dev_z, u.depth_xf) * u.misc.x;
    if (inside) {
        textureStore(flat_out, pix, vec4<f32>(len, ang * 0.159155 + 0.5, lin, lin));
    }
    let lc = min(u.p18.w / u.p18.y, len);
    g0[li] = vec4<f32>(select(2.0, lc, inside), ang, select(0.0, lc, inside), ang);
    workgroupBarrier();
    // reduction: xy keeps the SHORTEST velocity (length, angle), zw the LONGEST
    for (var s = 128u; s >= 1u; s = s >> 1u) {
        var res = g0[li];
        if (li < s) {
            let a = g0[li];
            let b = g0[li + s];
            let mn = select(b.xy, a.xy, a.x < b.x);
            let mx = select(b.zw, a.zw, b.z < a.z);
            res = vec4<f32>(mn, mx);
        }
        workgroupBarrier();
        if (li < s) { g0[li] = res; }
        workgroupBarrier();
    }
    if (li == 0u) {
        let r = g0[0];
        let mn = r.x * vec2<f32>(cos(r.y), sin(r.y));
        let mx = r.z * vec2<f32>(cos(r.w), sin(r.w));
        textureStore(tile_out, vec2<i32>(gid.xy), vec4<f32>(mn, mx));
    }
}

@compute @workgroup_size(16, 16, 1)
fn cs_gather(@builtin(global_invocation_id) tid: vec3<u32>) {
    let t = vec2<i32>(tid.xy);
    let grid = vec2<i32>(u.tile_grid.xy);
    if (t.x >= grid.x || t.y >= grid.y) { return; }
    var acc = textureLoad(tile_in, t, 0);
    for (var ox = -3; ox <= 3; ox = ox + 1) {
        for (var oy = -3; oy <= 3; oy = oy + 1) {
            if (ox == 0 && oy == 0) { continue; }
            let nb = t + vec2<i32>(ox, oy);
            if (nb.x < 0 || nb.y < 0 || nb.x >= grid.x || nb.y >= grid.y) { continue; }
            let n = textureLoad(tile_in, nb, 0);
            let v = n.zw * u.p18.z;
            let d2 = dot(v, v);
            // Quake fast inverse square root without refinement, exactly as compiled (0x5f3759df - (bits >> 1))
            let y = bitcast<f32>(bitcast<i32>(0x5f3759df) - (bitcast<i32>(d2) >> 1u));
            let nrm = v * y;
            let l1 = abs(nrm.y) + abs(nrm.x);
            let thr = vec2<f32>(l1 * 0.99 + y * d2, l1 * 0.99);
            let d = vec2<f32>(f32(ox), f32(oy));
            let along = dot(nrm, d);
            let perp = dot(vec2<f32>(-nrm.y, nrm.x), d);
            let reach = (abs(along) < thr.x) && (abs(perp) < thr.y);
            let mn = select(n.xy, acc.xy, dot(acc.xy, acc.xy) < dot(n.xy, n.xy));
            let mx = select(acc.zw, n.zw, dot(n.zw, n.zw) < dot(acc.zw, acc.zw));
            if (reach) { acc = vec4<f32>(mn, mx); }
        }
    }
    textureStore(tile_out, t, acc);
}

fn flat_fetch(uv: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(flat_tex));
    let p = clamp(vec2<i32>(floor(uv * vec2<f32>(dims))), vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(flat_tex, p, 0);
}
fn tile_fetch(uv: vec2<f32>) -> vec4<f32> {
    let dims = vec2<i32>(textureDimensions(tile_in));
    let p = clamp(vec2<i32>(floor(uv * vec2<f32>(dims))), vec2<i32>(0), dims - vec2<i32>(1));
    return textureLoad(tile_in, p, 0);
}

@fragment
fn fs_motion_blur(in: VsOut) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.pos.xy);                       // ftou
    let fp = vec2<f32>(pixel);
    // two interleaved-gradient noises (r1.x, r1.y)
    let n = fract(52.9829189 * fract(dot(fp, vec2<f32>(0.06711056, 0.00583715))));
    let n2 = fract(52.9829189 * fract(dot(fp + vec2<f32>(32.665, 11.815), vec2<f32>(0.06711056, 0.00583715))));
    let jit = (vec2<f32>(n, n2) - vec2<f32>(0.5)) * 0.5;
    let rel = vec2<f32>(vec2<i32>(pixel) - vec2<i32>(u.rect.xy)) + vec2<f32>(0.5);
    var tc = rel * 0.0625 + jit;
    tc = min(vec2<f32>(u.rect.zw), tc);
    let tl = tile_fetch(tc * u.tile_inv.zw);                // xy = min velocity, zw = max velocity
    let scale = u.p19.y;
    let uv = in.pos.xy * u.size.zw;                         // v0.xy
    let scr = vec2<f32>(uv.x * 2.0 - 1.0, -(uv.y * 2.0 - 1.0));  // v0.zw
    let vmax = tl.zw * scale;
    let m = dot(vmax, vmax);                                // r1.y
    let c0 = textureSampleLevel(color_tex, samp, uv, 0.0).rgb;
    if (m < 0.25) { return vec4<f32>(c0, 0.0); }
    let vmin = tl.xy * scale;
    let uniform_motion = (m * 0.4) < dot(vmin, vmin);
    // screen-edge clamp of the sample line: distance (in line parameter) to the +-1 NDC border along +V and -V
    let vc = tl.zw * scale + vec2<f32>(0.00001);
    let rc = vec2<f32>(1.0) / vc;
    let e = u.edge.x;
    let tp = rc * e - rc * e * scr;                          // r0.yz
    let tn = -(rc * e) - rc * e * scr;                       // r4.zw
    let fwd = max(tp, tn);                                   // r5.xy: distance along +V per axis
    let bwd = max(-tp, -tn);                                 // r5.zw: distance along -V per axis
    let cl_pos = sat(min(fwd.y, fwd.x));
    let cl_neg = sat(min(bwd.y, bwd.x));
    let vpos = vmax * cl_pos * u.size.zw;                    // r2.xy (uv units)
    let vneg = vmax * (-cl_neg) * u.size.zw;                 // r2.zw
    if (uniform_motion) {
        var acc = vec3<f32>(0.0);
        for (var k = 0; k < 8; k = k + 1) {
            let a = (n + f32(k)) * 0.125;
            let b = (f32(k + 1) - n) * 0.125;
            acc = acc + textureSampleLevel(color_tex, samp, uv + a * vpos, 0.0).rgb;
            acc = acc + textureSampleLevel(color_tex, samp, uv + b * vneg, 0.0).rgb;
        }
        return vec4<f32>(acc * 0.0625, 0.0);
    }
    let kinv = inverseSqrt(m) * 8.0;                         // r0.w
    let fc = flat_fetch(uv);
    let lc = min(fc.x * scale, u.p19.w);                     // r4.x
    let zc = fc.z;                                           // r1.z
    var acc = vec4<f32>(0.0);
    for (var k = 0; k < 8; k = k + 1) {
        let uva = uv + (n + f32(k)) * 0.125 * vpos;
        let uvb = uv + (f32(k + 1) - n) * 0.125 * vneg;
        let fa = flat_fetch(uva);
        let fb = flat_fetch(uvb);
        let ca = textureSampleLevel(color_tex, samp, uva, 0.0).rgb;
        let cb = textureSampleLevel(color_tex, samp, uvb, 0.0).rgb;
        let la = min(fa.x * scale, u.p19.w);
        let lb = min(fb.x * scale, u.p19.w);
        let off = select(n + f32(k - 1), 0.0, k == 0);
        let dza = fa.z - zc;
        let dzb = fb.z - zc;
        let aa = vec2<f32>(sat(dza + 0.5), sat(0.5 - dza));
        let ab = vec2<f32>(sat(dzb + 0.5), sat(0.5 - dzb));
        let cova = vec2<f32>(sat(kinv * lc - off), sat(kinv * la - off));
        let covb = vec2<f32>(sat(kinv * lc - off), sat(kinv * lb - off));
        let wa = dot(aa, cova);
        let wb = dot(ab, covb);
        let c1 = la < lb;
        let c2 = fb.z < fa.z;
        let wa2 = select(wa, wb, c1 && c2);
        let wb2 = select(wa, wb, c1 || c2);
        acc = acc + vec4<f32>(ca, 1.0) * wa2 + vec4<f32>(cb, 1.0) * wb2;
    }
    let out_rgb = acc.rgb * 0.0625 + c0 * (1.0 - acc.a * 0.0625);
    return vec4<f32>(out_rgb, 0.0);
}
