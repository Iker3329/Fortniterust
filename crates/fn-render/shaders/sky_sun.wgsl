// SM_FlatSunMoon_Arid (36-vertex disc, mesh plane = YZ, radius 1500 cm) as a camera-facing billboard, M_Sun_Arid pixel shader.
// Port of shader_25910 (ps_5_0, UE 4.16 / Fortnite 1.8). Material cbuffer cb2[0..20] uploaded verbatim (sky_params::SUNARID).
// Time-of-day colour weights (cb1[15], MPC values, NOT serialized) are `tod` = (morning, day, evening, night); Day = (0,1,0,0).
@group(0) @binding(0) var<uniform> sky: SkyU;
@group(0) @binding(1) var<uniform> cb2: array<vec4<f32>, 21>;
@group(0) @binding(2) var t_storm: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv1: vec2<f32>,
    @location(1) vcol: vec2<f32>,
    @location(2) world_pos: vec3<f32>,
}

// pos = mesh (Y, Z) in cm; vcol = vertex colour (R, G) drives the flare rays.
@vertex
fn vs_main(@location(0) pos: vec2<f32>, @location(1) uv1: vec2<f32>, @location(2) vcol: vec2<f32>) -> VsOut {
    var o: VsOut;
    o.uv1 = uv1;
    o.vcol = vcol;
    let scale = sky.sun_u.w;
    let rel = sky.sun_c.xyz + sky.sun_r.xyz * (pos.x * scale) + sky.sun_u.xyz * (pos.y * scale);
    o.world_pos = rel + sky.cam_world.xyz;
    o.clip = sky.vp * vec4<f32>(rel, 1.0);
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let time = sky.cam_world.w;
    let tod = sky.tod;
    var uvs = in.uv1.xyxy * 1.5 + time * vec4<f32>(0.06, 0.06, -0.06, -0.06);
    uvs = vec4<f32>(uvs.xy, uvs.zw + vec2<f32>(0.44, 0.663));
    let na = textureSample(t_storm, samp, uvs.xy).x;
    let nb = textureSample(t_storm, samp, uvs.zw).x;
    var noise = 2.0 * na * nb - 2.0;                           // dp2(a,a, b,b) - 2
    let inv_g = 1.0 - in.vcol.y;
    noise = clamp(inv_g * 2.0 + noise, 0.0, 1.0) * inv_g;
    let dist = length(in.uv1 - 0.5);
    var zz = dist / cb2[19].x;                                 // /GlowAmt(0.07)
    var yy = dist / cb2[18].x;                                 // /SunRadius
    var ww = dist / cb2[18].z;                                 // /GlowAmt(0.5)
    yy = 1.0 - yy; zz = 1.0 - zz; ww = 1.0 - ww;
    var e = zz * cb2[19].y; e = e * e;
    var g = exp2(e * 1.442695); g = 1.0 / g;                   // exp(-(...)^2)
    let inner = select(-1.0, -g, zz >= 0.0 && abs(zz) > 0.00001);
    let yq = yy * cb2[18].y;
    let wq = ww * cb2[18].w;
    var disc = 1.0 - 1.0 / exp2(yq * yq * 1.442695);
    var glow = 1.0 - 1.0 / exp2(wq * wq * 1.442695);
    disc = select(0.0, disc, abs(yy) > 0.00001 && yy >= 0.0);
    glow = select(0.0, glow, abs(ww) > 0.00001 && ww >= 0.0);
    var z = inner + glow + 1.0;
    // 1.8 compiled form: max(|v2.x|, 1e-6) before the log (1.10: max(v2.x, 0))
    let lx = log2(max(abs(in.vcol.x), 1e-6));
    var rx = exp2(lx * 20.0);                                  // ray sharpness from vertex colour R
    let ry = exp2(lx * 50.0);
    rx = min(rx * 0.5, 1.0) - 1.0;
    z = clamp(z * 2.0 + rx, 0.0, 1.0);
    z = ry * 0.2 + z;
    z = z * cb2[19].z;                                         // FlareAmount
    let outer = glow * cb2[19].w;                              // OuterGlowAmount
    var s = z * noise + outer;
    var g8 = max(glow, 1e-6); g8 = g8 * g8; g8 = g8 * g8; g8 = g8 * g8;   // 1.8: max(glow, 1e-6)^8
    s = g8 * cb2[20].x + s;                                    // InnerGlowAmount
    let k = s * 0.5 + disc;
    let a = s * 0.15 + disc;
    let c2 = tod.y * cb2[12].xyz + tod.x * cb2[10].xyz + tod.w * cb2[16].xyz + tod.z * cb2[14].xyz;
    let c1 = tod.y * cb2[4].xyz + tod.x * cb2[2].xyz + tod.w * cb2[8].xyz + tod.z * cb2[6].xyz;
    var col = k * (c2 - c1) + c1 + cb2[17].xyz;
    col = max(col, vec3<f32>(0.0));
    let fog = height_fog(sky.fog, in.world_pos, sky.cam_world.xyz, sky.to_sun.xyz, sky_forward());
    return vec4<f32>(col * fog.a + fog.rgb, clamp(a, 0.0, 1.0));
}
