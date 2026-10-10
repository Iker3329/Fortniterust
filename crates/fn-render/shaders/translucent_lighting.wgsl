// TRANSLUCENCY LIGHTING VOLUME (UE4.16), see src/translucent_lighting.rs:
//   inject = ClearTranslucentVolumeLighting + InjectMainPS (TranslucentLightInjectionShaders.usf:109) of the directional light, summed over the
//            whole-scene CSM cascades exactly as the additive blend (BF_One, BF_One; CW_RGBA for the ambient target, CW_RGB for the directional one,
//            TranslucentLighting.cpp:1524) accumulates them; one thread per voxel at the FWriteToSliceVS position (UV = texel centre, layer centre).
//   filter_volume = FilterMainPS (TranslucentLightingShaders.usf:93): 8 bilinear taps at +-0.5 texel, / 8.
// Units: UE world cm, Z up. The CSM matrices of the renderer's light block take renderer metres (x, z, y) * 0.01.

struct Tlv {
    vmin: array<vec4<f32>, 2>,      // View.TranslucencyLightingVolumeMin (xyz, w = 1 / Dim)
    vinv: array<vec4<f32>, 2>,      // View.TranslucencyLightingVolumeInvSize (xyz, w = VoxelSize)
    view_origin: vec4<f32>,         // View.WorldCameraOrigin
    view_forward: vec4<f32>,        // View.ViewForward
    light_dir: vec4<f32>,           // DeferredLightUniforms.NormalizedLightDirection (towards the light), w = light function shadow factor
    light_color: vec4<f32>,         // DeferredLightUniforms.LightColor / PI
    split_near: vec4<f32>,          // per cascade: NearFrustumPlane distance (cm along the view direction)
    split_far: vec4<f32>,           // per cascade: FarFrustumPlane distance (SplitFar + FadeExtension)
    near_fade: vec4<f32>,           // ShadowInjectParams.x per cascade
    far_fade: vec4<f32>,            // ShadowInjectParams.y per cascade
    far_plane: vec4<f32>,           // 1 = the cascade has a far plane (not the last one)
    misc: vec4<f32>,                // x shadowed cascades (0 = unshadowed injection), y Dim, zw ShadowInjectParams.zw (distance fade MAD)
    shadow: vec4<f32>,              // x shadow map size, y SHADOW_BORDER
}
// must mirror the prefix of `struct Light` of lit_common.wgsl (the renderer's light block)
struct Light {
    sun_dir: vec4<f32>,
    sun_radiance: vec4<f32>,
    sky_radiance: vec4<f32>,
    shadow_matrix: array<mat4x4<f32>, 3>,
    cascade_far: vec4<f32>,
    cascade_texel: vec4<f32>,
    shadow_params: vec4<f32>,
    misc: vec4<f32>,
    cascade_ext: vec4<f32>,
    cascade_bias: vec4<f32>,
}

@group(0) @binding(0) var<uniform> tlv: Tlv;
@group(0) @binding(1) var<uniform> light: Light;
@group(0) @binding(2) var shadow_map: texture_depth_2d_array;
@group(0) @binding(3) var amb_inner_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(4) var dir_inner_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(5) var amb_outer_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(6) var dir_outer_out: texture_storage_3d<rgba16float, write>;
@group(0) @binding(11) var lin_samp: sampler;
@group(0) @binding(7) var amb_inner_in: texture_3d<f32>;
@group(0) @binding(8) var dir_inner_in: texture_3d<f32>;
@group(0) @binding(9) var amb_outer_in: texture_3d<f32>;
@group(0) @binding(10) var dir_outer_in: texture_3d<f32>;

// ComputeVolumeShadowing (VolumeLightingCommon.usf, DYNAMICALLY_SHADOWED, directional): point-sampled depth inside ShadowmapMinMax, compared with the
// cascade's ShaderDepthBias (DepthBiasParameters.x), then the whole-scene distance fade
fn volume_shadowing(world_lighting: vec3<f32>, cascade: i32) -> f32 {
    var shadow = 1.0;
    let pm = vec3<f32>(world_lighting.x, world_lighting.z, world_lighting.y) * 0.01;
    let h = light.shadow_matrix[cascade] * vec4<f32>(pm, 1.0);
    let uv = h.xy / h.w;
    let size = tlv.shadow.x;
    let lo = tlv.shadow.y / size;
    // ShadowmapMinMax = ((X + Border) / Buffer, (X + 2 Border + Resolution) / Buffer) (ShadowRendering.cpp:1149): the unatlased map's centre viewport
    if (all(uv >= vec2<f32>(lo)) && all(uv <= vec2<f32>(1.0))) {
        let t = clamp(vec2<i32>(floor(uv * size)), vec2<i32>(0), vec2<i32>(i32(size) - 1));
        let d = textureLoad(shadow_map, t, cascade, 0);
        shadow = select(0.0, 1.0, h.z < d - light.cascade_bias[cascade]);
    }
    let depth = dot(world_lighting - tlv.view_origin.xyz, tlv.view_forward.xyz);
    let fade = clamp(depth * tlv.misc.z + tlv.misc.w, 0.0, 1.0);
    return mix(shadow, 1.0, fade * fade);
}

struct Inj { amb: vec4<f32>, dir: vec4<f32> }

// InjectMainPS for one whole-scene shadow (cascade >= 0) or the unshadowed light (cascade < 0)
fn inject_one(world: vec3<f32>, vuv: vec3<f32>, vc: u32, cascade: i32) -> Inj {
    var o: Inj;
    o.amb = vec4<f32>(0.0);
    o.dir = vec4<f32>(0.0);
    var masking = 1.0;
    if (cascade >= 0) {
        // ClippingPlanes: DistToNear = -dot(Plane0, P) = depth - SplitNear, DistToFar = SplitFar - depth (FPlane orientation of ComputeShadowCullingVolume);
        // the plane (0, 0, 0, -1) of "no plane" gives 1
        let depth = dot(world - tlv.view_origin.xyz, tlv.view_forward.xyz);
        let dist_near = depth - tlv.split_near[cascade];
        var dist_far = 1.0;
        if (tlv.far_plane[cascade] > 0.5) { dist_far = tlv.split_far[cascade] - depth; }
        if (dist_near < 0.0 || dist_far < 0.0) { return o; } // clip()
        masking = clamp(dist_near * tlv.near_fade[cascade], 0.0, 1.0) * clamp(dist_far * tlv.far_fade[cascade], 0.0, 1.0);
    }
    let l = tlv.light_dir.xyz;
    let voxel = tlv.vinv[vc].w;
    // WorldPosition + GetBoxPushout(L, .5 * VoxelSize): GetBoxPushout returns a SCALAR (Common.usf:1690), added to every component
    let world_lighting = world + vec3<f32>(dot(abs(l * (0.5 * voxel)), vec3<f32>(1.0)));
    let attenuation = 1.0; // CalcLightAttenuation of a directional light
    var shadow = 1.0;
    if (cascade >= 0) { shadow = volume_shadowing(world_lighting, cascade); }
    if (vc == 1u) {
        // the outer cascade's border voxels fade the shadowing out (TransitionScale 10)
        let w = tlv.vmin[1].w;
        let fade_uvs = vuv * (1.0 + 4.0 * w) - 2.0 * w;
        let lf = clamp((vec3<f32>(0.5) - abs(fade_uvs - vec3<f32>(0.5))) * 10.0, vec3<f32>(0.0), vec3<f32>(1.0));
        shadow = mix(1.0, shadow, lf.x * lf.y * lf.z);
    }
    shadow = shadow * tlv.light_dir.w; // GetLightFunctionShadowFactor
    let lighting = tlv.light_color.rgb * attenuation * shadow;
    // MulSH(SHBasisFunction(L), Lighting): (0.282095, -0.488603 L.y, 0.488603 L.z, -0.488603 L.x) per channel
    let sh = vec4<f32>(0.282095, -0.488603 * l.y, 0.488603 * l.z, -0.488603 * l.x);
    o.amb = vec4<f32>(lighting * sh.x, attenuation * shadow) * masking;
    let luma = dot(lighting, vec3<f32>(0.3, 0.59, 0.11));
    o.dir = vec4<f32>(luma * sh.y, luma * sh.z, luma * sh.w, 0.0) * masking;
    return o;
}

@compute @workgroup_size(4, 4, 4)
fn inject(@builtin(global_invocation_id) id: vec3<u32>) {
    let dim = u32(tlv.misc.y);
    if (id.x >= dim || id.y >= dim || id.z >= 2u * dim) { return; }
    let vc = id.z / dim;
    let layer = id.z % dim;
    let vmin = tlv.vmin[vc];
    let vinv = tlv.vinv[vc];
    let voxel = vinv.w;
    // FWriteToSliceVS / InjectMainPS: Z at the layer centre, XY = Min + UV / InvSize - .5 VoxelSize with UV the texel centre
    let uv = (vec2<f32>(id.xy) + 0.5) / f32(dim);
    let world = vec3<f32>(vmin.xy + uv / vinv.xy - 0.5 * voxel, vmin.z + (f32(layer) + 0.5) * voxel);
    let vuv = vec3<f32>(uv, (f32(layer) + 0.5) * vmin.w);
    var amb = vec4<f32>(0.0);
    var dir = vec4<f32>(0.0);
    let n = i32(tlv.misc.x);
    if (n == 0) {
        let r = inject_one(world, vuv, vc, -1);
        amb = r.amb; dir = r.dir;
    } else {
        for (var c = 0; c < n; c = c + 1) { let r = inject_one(world, vuv, vc, c); amb = amb + r.amb; dir = dir + r.dir; }
    }
    let p = vec3<i32>(vec3<u32>(id.xy, layer));
    if (vc == 0u) { textureStore(amb_inner_out, p, amb); textureStore(dir_inner_out, p, dir); }
    else { textureStore(amb_outer_out, p, amb); textureStore(dir_outer_out, p, dir); }
}

fn filtered(t: texture_3d<f32>, uvw: vec3<f32>, texel: f32) -> vec4<f32> {
    var s = vec4<f32>(0.0);
    for (var i = 0u; i < 8u; i = i + 1u) {
        let o = vec3<f32>(select(-1.0, 1.0, (i & 1u) == 0u), select(-1.0, 1.0, (i & 2u) == 0u), select(-1.0, 1.0, (i & 4u) == 0u));
        s = s + textureSampleLevel(t, lin_samp, uvw + 0.5 * texel * o, 0.0);
    }
    return s * (1.0 / 8.0);
}

@compute @workgroup_size(4, 4, 4)
fn filter_volume(@builtin(global_invocation_id) id: vec3<u32>) {
    let dim = u32(tlv.misc.y);
    if (id.x >= dim || id.y >= dim || id.z >= 2u * dim) { return; }
    let vc = id.z / dim;
    let layer = id.z % dim;
    let texel = 1.0 / f32(dim);
    let uvw = (vec3<f32>(f32(id.x), f32(id.y), f32(layer)) + 0.5) * texel;
    let p = vec3<i32>(vec3<u32>(id.xy, layer));
    if (vc == 0u) {
        textureStore(amb_inner_out, p, filtered(amb_inner_in, uvw, texel));
        textureStore(dir_inner_out, p, filtered(dir_inner_in, uvw, texel));
    } else {
        textureStore(amb_outer_out, p, filtered(amb_outer_in, uvw, texel));
        textureStore(dir_outer_out, p, filtered(dir_outer_in, uvw, texel));
    }
}
