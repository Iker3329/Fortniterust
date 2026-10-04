// M_AridClouds dome (SkyDomeMeshComp: FortniteSkyDome_Optimized scaled (2000,2000,167.7) cm, centred 1000 m below the TOD origin).
// Port of the pixel shader compiled into M_AridClouds (shader_17639: ps_5_0 of UE 4.16 / Fortnite 1.8, translucent base pass) with the
// M_AridClouds_Inst_Day constants (material cbuffer cb3[0..21] uploaded verbatim). Substitutions: View.RealTime -> time,
// View.ViewOrigin / ObjectWorldPosition -> cam_world / center_world, UE world +Z (up) is this runtime's +Y, and the
// ExponentialHeightFog tail -> height_fog() (the volumetric-fog 3D texture lookup is done by the "vfog composite" pass, src/volfog.rs).
@group(0) @binding(0) var<uniform> sky: SkyU;
@group(0) @binding(1) var<uniform> cb3: array<vec4<f32>, 22>;
@group(0) @binding(2) var t_cloud: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv0: vec2<f32>,
    @location(1) uv1y: f32,
    @location(2) tangent_x: vec3<f32>,
    @location(3) tangent_z: vec3<f32>,
    @location(4) world_pos: vec3<f32>,
}

// pos = dome-local metres (GL world); tc = (uv0.x, uv0.y, uv1.y, uv2.y); tz = tangent Z, tx = tangent X (unit, GL world).
@vertex
fn vs_main(@location(0) pos: vec3<f32>, @location(1) tc: vec4<f32>, @location(2) tz: vec3<f32>, @location(3) tx: vec3<f32>) -> VsOut {
    var o: VsOut;
    o.uv0 = tc.xy;
    o.uv1y = tc.z;
    o.tangent_x = tx;
    o.tangent_z = tz;
    o.world_pos = pos + sky.center_world.xyz;
    o.clip = sky.vp * vec4<f32>(pos - sky.cam_local.xyz, 1.0);
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let time = sky.cam_world.w;
    let cam_world = sky.cam_world.xyz;
    let Tx = normalize(in.tangent_x);
    let Tz = normalize(in.tangent_z);
    let Ty = cross(Tz, Tx);                      // tangent sign assumed +1 (not recoverable without the mesh tangents)
    let V = normalize(cam_world - in.world_pos); // pixel -> camera
    let up = normalize(cam_world - sky.center_world.xyz);   // dome centre -> camera
    var sunk = clamp((dot(V, up) + cb3[16].x) * cb3[16].y, 0.0, 1.0);
    sunk = sunk * sunk; sunk = sunk * sunk;
    let sun_tint = sunk * cb3[2].xyz;
    let a = in.uv0 - 1.0;
    let scroll = time * cb3[16].w;               // BigCloudSpeed
    let uv_base = a * cb3[16].z + scroll;
    let swirl = a * cb3[17].x;                   // SwirlTile
    let r5 = time * vec2<f32>(0.013125, -0.02625) + swirl;
    let row = ceil(r5.x + 0.5);
    let p = vec2<f32>(r5.x, row * 0.5 + r5.y) + 0.5;
    let fr = fract(p) - 0.5;
    var d = max(1.0 - 2.0 * length(fr), 0.0);
    d = d * d * cb3[17].y;                       // RotAmtMod
    let ang = d * vec2<f32>(6.283185, -6.283185);
    let cell = ceil(p) - 0.5;
    var off = p - cell;
    let perp = vec2<f32>(-off.y, off.x) * sin(ang.x);
    off = off * cos(ang.x) + perp;
    let disp = (cell + off - p) * 0.09375;
    let uvA = uv_base * 0.125 + disp;
    let tA = textureSample(t_cloud, samp, uvA).xyz;
    let hA = tA.z;
    let uvB = uvA * cb3[17].w + time * cb3[17].z;
    let tB = textureSample(t_cloud, samp, uvB).xyz;
    let hh = (1.0 - hA) * ((tB.z - cb3[18].x) * cb3[18].y) + hA;
    let zen = (in.uv1y - cb3[18].w) * cb3[19].x;  // zenith -> horizon factor
    var thr = zen * (cb3[19].y - cb3[19].z) + cb3[19].z;
    thr = sunk * cb3[18].z + thr;
    let dens = hh - thr;
    let dsat = clamp(dens, 0.0, 1.0);
    let om = 1.0 - dsat;
    let lg = log2(max(om, 1e-6));                 // 1.8 compiled form: max(1 - dsat, 1e-6) before the log
    let p10 = exp2(lg * 10.0);
    let edge = om * 0.05 + p10;
    let nl = exp2(lg * cb3[19].w) * cb3[20].x;    // detail normal power / Normal Lerp
    let nxy = tA.xy + nl * (tB.xy - tA.xy);
    let n2 = (nxy - 0.5) * 2.0;
    let nz = sqrt(max(1.0 - dot(n2, n2), 0.0));
    let n6 = vec3<f32>(n2, nz);
    let a5 = vec3<f32>(n2, 0.0);
    let a7 = vec3<f32>(-n2.y, n2.x, 0.0) * sin(ang.y);
    var r5v = a5 * cos(ang.y) + a7 + vec3<f32>(0.0, 0.0, nz);
    r5v = (r5v - n6) * 0.5 + n6;
    r5v = normalize(r5v * cb3[4].xyz);            // Normal Modulator
    let N = Ty * r5v.y + Tx * r5v.x + Tz * r5v.z;
    var down = min(max(N.y, -1.0), 0.0);          // UE N.z (world up)
    down = down * down;
    var col2 = sun_tint * edge + (dsat * down) * cb3[6].xyz;   // Ground Cloud Color
    col2 = col2 + p10 * cb3[8].xyz * 0.1;         // Clouds - Direct Sun Light Color
    let edge_light = 2.0 * exp2(lg * cb3[20].y);  // Cloud Transparency Edge Lighting Falloff
    var L = clamp((0.75 - dot(up, N)) * 0.571429, 0.0, 1.0);
    L = edge_light * L + L;
    let lit = L * cb3[8].xyz;
    let zc = clamp((in.uv1y - cb3[20].z) * cb3[20].w, 0.0, 1.0);
    let tint = cb3[12].xyz + zc * (cb3[11].xyz - cb3[12].xyz);
    var col = lit * tint + col2;
    col = mix(col, cb3[14].xyz, 1.0 - clamp(1.0 - in.uv1y, 0.0, 1.0));   // FogColor
    col = min(max(col, vec3<f32>(0.0)), vec3<f32>(1000.0)) + cb3[15].xyz;
    let opacity = clamp(zen, 0.0, 1.0) * (cb3[21].x - cb3[21].y) + cb3[21].y;
    let alpha = clamp(dens * opacity * cb3[21].z, 0.0, 1.0);
    let fog = height_fog(in.world_pos, cam_world, sky.to_sun.xyz, sky_forward());
    return vec4<f32>(col * fog.a + fog.rgb, alpha);
}
