// DISTANCE FIELD HOOK: distance-field ambient occlusion (UE4.16 screen-grid DFAO), every pass of
//   DistanceFieldScreenGridLighting.usf / DistanceFieldObjectCulling.usf / GlobalDistanceField.usf / DistanceFieldLightingPost.usf
// ported to wgpu compute. Coordinates are "DF space" (cm, Z up, left handed, see distance_field.rs); "SceneDepth" is the view-space depth in cm.
// Each entry point uses its own explicit bind group layout (a subset of the bindings below).

struct AoU {
    eye: vec4<f32>,        // DF-space camera origin (cm)
    right: vec4<f32>,      // xyz right, w tan(half fov x)
    up: vec4<f32>,         // xyz up, w tan(half fov y)
    fwd: vec4<f32>,        // xyz forward, w View.NearPlane (cm)
    prev_eye: vec4<f32>,
    prev_right: vec4<f32>,
    prev_up: vec4<f32>,
    prev_fwd: vec4<f32>,
    size: vec4<f32>,       // full buffer (W, H, 1/W, 1/H)
    half_size: vec4<f32>,  // GetBufferSizeForAO = W/2, H/2 (floor) and inverses
    grid: vec4<f32>,       // GetBufferSizeForConeTracing = half / 4 (floor) and inverses
    tiles: vec4<f32>,      // x/y TileListGroupSize, z number of view-culled objects, w r.AOMaxObjectsPerCullTile
    jitter: vec4<f32>,     // xy JitterOffset (base-level texels), z history valid, w UseHistoryFilter
    ao_a: vec4<f32>,       // AOObjectMaxDistance, AOStepScale, AOStepExponentScale, AOMaxViewDistance
    ao_b: vec4<f32>,       // AOGlobalMaxOcclusionDistance, TanConeHalfAngle, BentNormalNormalizeFactor, DistanceFadeScale
    hist: vec4<f32>,       // HistoryWeight, AOHistoryMinConfidenceScale, HistoryDistanceThreshold, unused
    depth: vec4<f32>,      // renderer near / far (m) of the depth buffer
    clip: array<vec4<f32>, 4>,  // GlobalVolumeCenterAndExtent (xyz centre, w extent)
    clip_misc: vec4<f32>,  // x GlobalVolumeDimension, y GlobalVolumeTexelSize, z MaxGlobalDistance, w global DF valid
    cones: array<vec4<f32>, 9>, // AOSamples2.SampleDirections (r.AOSampleSet 1: RelaxedSpacedVectors9)
    // GDF PARTIAL UPDATES (2026-10-06): xyz = each clipmap's toroidal origin FullUpdateOrigin * CellSize (DF cm). ComputeGlobalUV = (WorldPosition - Center +
    // ScrollOffset) / (2 * Extent) + .5 with ScrollOffset = (LastPartialUpdateOrigin - FullUpdateOrigin) * CellSize and Center = LastPartialUpdateOrigin * CellSize,
    // i.e. (WorldPosition - FullUpdateOrigin * CellSize) / (2 * Extent) + .5 (GlobalDistanceField.cpp:115 WorldToUVAddAndMul), sampled with AM_Wrap.
    clip_full: array<vec4<f32>, 4>,
}
@group(0) @binding(0) var<uniform> u: AoU;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var normal_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var normal_tex: texture_2d<f32>;
@group(0) @binding(4) var<storage, read_write> tile_cones_rw: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> tile_cones: array<vec4<f32>>;
@group(0) @binding(6) var<storage, read> view_objects: array<u32>;
@group(0) @binding(7) var<storage, read> objects: array<vec4<f32>>;
@group(0) @binding(8) var df_atlas: texture_3d<f32>;
@group(0) @binding(9) var df_samp: sampler;
@group(0) @binding(10) var<storage, read_write> tile_lists_rw: array<u32>;
@group(0) @binding(11) var<storage, read> tile_lists: array<u32>;
@group(0) @binding(12) var gdf: texture_3d<f32>;
@group(0) @binding(13) var bent_low_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(14) var bent_low: texture_2d<f32>;
@group(0) @binding(15) var bent_half_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(16) var conf_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(17) var bent_half: texture_2d<f32>;
@group(0) @binding(18) var conf_tex: texture_2d<f32>;
@group(0) @binding(19) var hist_prev: texture_2d<f32>;
@group(0) @binding(20) var conf_prev: texture_2d<f32>;
@group(0) @binding(21) var hist_new_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(22) var conf_new_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(23) var hist_new: texture_2d<f32>;
@group(0) @binding(24) var conf_new: texture_2d<f32>;
@group(0) @binding(25) var state_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(26) var state_conf_out: texture_storage_2d<rgba16float, write>;
@group(0) @binding(27) var gdf_out: texture_storage_3d<r32float, write>;
@group(0) @binding(28) var<storage, read> grid_lists: array<u32>;
@group(0) @binding(29) var<uniform> gc: GdfClip;
@group(0) @binding(30) var lin_samp: sampler;

const NUM_CONE_STEPS: u32 = 10u;      // DistanceFieldAOShared.usf
const NUM_CONE_DIRECTIONS: u32 = 9u;
const TRACE_DOWNSAMPLE_FACTOR: u32 = 4u;  // GConeTraceDownsampleFactor
const CULLED_TILE_SIZEX: u32 = 16u;       // GDistanceFieldAOTileSizeX
const CONE_TILE_SIZEX: u32 = 4u;          // CULLED_TILE_SIZEX / TRACE_DOWNSAMPLE_FACTOR
const OBJ_STRIDE: u32 = 9u;               // vec4s per object (distance_field.rs OBJECT_VEC4S)

// ---- shared helpers (Common.usf / DistanceFieldLightingShared.usf) ----
fn view_pos(uv: vec2<f32>, depth_cm: f32) -> vec3<f32> {
    // ScreenToWorld of (ScreenPosition * SceneDepth, SceneDepth)
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return u.eye.xyz + (u.right.xyz * (ndc.x * u.right.w) + u.up.xyz * (ndc.y * u.up.w) + u.fwd.xyz) * depth_cm;
}
fn prev_view_pos(uv: vec2<f32>, depth_cm: f32) -> vec3<f32> {
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    return u.prev_eye.xyz + (u.prev_right.xyz * (ndc.x * u.prev_right.w) + u.prev_up.xyz * (ndc.y * u.prev_up.w) + u.prev_fwd.xyz) * depth_cm;
}
fn prev_uv_of(p: vec3<f32>) -> vec2<f32> {
    let d = p - u.prev_eye.xyz;
    let z = dot(d, u.prev_fwd.xyz);
    let ndc = vec2<f32>(dot(d, u.prev_right.xyz) / (z * u.prev_right.w), dot(d, u.prev_up.xyz) / (z * u.prev_up.w));
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}
fn box_distance(mins: vec3<f32>, maxs: vec3<f32>, p: vec3<f32>) -> f32 {
    // ComputeDistanceFromBoxToPoint ("actually incorrect, it gives manhattan distance")
    let dmin = select(vec3<f32>(0.0), abs(p - mins), p < mins);
    let dmax = select(vec3<f32>(0.0), abs(p - maxs), p > maxs);
    return dot(dmin, vec3<f32>(1.0)) + dot(dmax, vec3<f32>(1.0));
}
fn box_sq_distance(c: vec3<f32>, e: vec3<f32>, p: vec3<f32>) -> f32 {
    let a = max(abs(p - c) - e, vec3<f32>(0.0));
    return dot(a, a);
}
fn box_distance_inside(c: vec3<f32>, e: vec3<f32>, p: vec3<f32>) -> f32 {
    let dmin = max(p - c + e, vec3<f32>(0.0));
    let dmax = max(c + e - p, vec3<f32>(0.0));
    let cl = min(dmin, dmax);
    return min(cl.x, min(cl.y, cl.z));
}
fn find_best_axis_vectors2(z: vec3<f32>) -> mat2x3<f32> {
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(z.z) < 0.999);
    let x = normalize(cross(up, z));
    return mat2x3<f32>(x, cross(z, x));
}
fn obj_w2v(i: u32) -> mat4x4<f32> {
    let b = i * OBJ_STRIDE;
    return mat4x4<f32>(objects[b + 1u], objects[b + 2u], objects[b + 3u], objects[b + 4u]);
}
fn sample_mesh_df(uv: vec3<f32>, mad: vec2<f32>) -> f32 {
    // SampleMeshDistanceField, EIGHT_BIT_MESH_DISTANCE_FIELDS: TextureValue * MAD.x + MAD.y
    return textureSampleLevel(df_atlas, df_samp, uv, 0.0).x * mad.x + mad.y;
}
fn get_step_offset(step: f32) -> f32 {
    let t = u.ao_a.z * step;
    return u.ao_a.y * (t * t + 1.0);
}
// GetDownsampledGBuffer: SF_Point / AM_Wrap sample of the DistanceFieldNormal texture
fn ds_gbuffer(uv: vec2<f32>) -> vec4<f32> {
    let sz = vec2<i32>(u.half_size.xy);
    let t = vec2<i32>(floor(uv * u.half_size.xy));
    let w = ((t % sz) + sz) % sz;
    return textureLoad(normal_tex, w, 0);
}

// ================================================================================================================================
// 1. ComputeDistanceFieldNormal: one texel per AO buffer texel, sampled at the top-left full-resolution texel. The renderer has no G-buffer, so
//    the normal is rebuilt from the depth buffer (the geometric normal of the visible surface; UE stores the G-buffer normal here).
// ================================================================================================================================
fn linear_depth_cm(t: vec2<i32>) -> f32 {
    let sz = vec2<i32>(u.size.xy);
    let c = clamp(t, vec2<i32>(0), sz - vec2<i32>(1));
    let d = textureLoad(scene_depth, c, 0);
    let n = u.depth.x;
    let f = u.depth.y;
    return n * f / (f - d * (f - n)) * 100.0;
}
fn full_pos(t: vec2<i32>) -> vec3<f32> {
    let uv = (vec2<f32>(t) + 0.5) * u.size.zw;
    return view_pos(uv, linear_depth_cm(t));
}
@compute @workgroup_size(8, 8, 1)
fn cs_normal(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= u.half_size.x || f32(id.y) >= u.half_size.y) { return; }
    let t = vec2<i32>(id.xy) * 2;
    let depth = linear_depth_cm(t);
    let p = full_pos(t);
    let px = full_pos(t + vec2<i32>(1, 0));
    let nx = full_pos(t - vec2<i32>(1, 0));
    let py = full_pos(t + vec2<i32>(0, 1));
    let ny = full_pos(t - vec2<i32>(0, 1));
    // the neighbour on the side with the smaller depth step (avoids silhouettes)
    let dx = select(p - nx, px - p, abs(dot(px - p, u.fwd.xyz)) < abs(dot(p - nx, u.fwd.xyz)));
    let dy = select(p - ny, py - p, abs(dot(py - p, u.fwd.xyz)) < abs(dot(p - ny, u.fwd.xyz)));
    var n = normalize(cross(dx, dy));
    if (dot(n, u.eye.xyz - p) < 0.0) { n = -n; }
    if (any(n != n)) { n = -u.fwd.xyz; }
    textureStore(normal_out, vec2<i32>(id.xy), vec4<f32>(n, depth));
}

// ================================================================================================================================
// 2. BuildTileConesMain (DistanceFieldObjectCulling.usf:261): per 16x16 AO-buffer tile, two depth ranges and the bounding cone.
// ================================================================================================================================
var<workgroup> tile_min_z: atomic<u32>;
var<workgroup> tile_max_z: atomic<u32>;
var<workgroup> tile_min_z2: atomic<u32>;
var<workgroup> tile_max_z2: atomic<u32>;
@compute @workgroup_size(16, 16, 1)
fn cs_tile_cones(@builtin(global_invocation_id) id: vec3<u32>, @builtin(workgroup_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    // BaseLevelScreenUV = (DispatchThreadId + .5) * DOWNSAMPLE_FACTOR * BufferSizeAndInvSize.zw
    let uv = (vec2<f32>(id.xy) + 0.5) * 2.0 * u.size.zw;
    let scene_depth_v = ds_gbuffer(uv).w;
    if (li == 0u) {
        atomicStore(&tile_min_z, 0x7F7FFFFFu);
        atomicStore(&tile_max_z, 0u);
        atomicStore(&tile_min_z2, 0x7F7FFFFFu);
        atomicStore(&tile_max_z2, 0u);
    }
    workgroupBarrier();
    let max_view = u.ao_a.w;
    if (scene_depth_v < max_view) {
        atomicMin(&tile_min_z, bitcast<u32>(scene_depth_v));
        atomicMax(&tile_max_z, bitcast<u32>(scene_depth_v));
    }
    workgroupBarrier();
    let min_tile_z = bitcast<f32>(atomicLoad(&tile_min_z));
    let max_tile_z = bitcast<f32>(atomicLoad(&tile_max_z));
    let half_z = 0.5 * (min_tile_z + max_tile_z);
    if (scene_depth_v >= half_z && scene_depth_v < max_view) { atomicMin(&tile_min_z2, bitcast<u32>(scene_depth_v)); }
    if (scene_depth_v <= half_z) { atomicMax(&tile_max_z2, bitcast<u32>(scene_depth_v)); }
    workgroupBarrier();
    if (li == 0u) {
        let min_tile_z2 = bitcast<f32>(atomicLoad(&tile_min_z2));
        let max_tile_z2 = bitcast<f32>(atomicLoad(&tile_max_z2));
        let vs = vec2<f32>(u.right.w, u.up.w);
        let ng = u.tiles.xy;
        let g = vec2<f32>(gid.xy);
        let c00 = normalize(vec3<f32>((g.x + 0.0) / ng.x * vs.x * 2.0 - vs.x, vs.y - (g.y + 0.0) / ng.y * vs.y * 2.0, 1.0));
        let c10 = normalize(vec3<f32>((g.x + 1.0) / ng.x * vs.x * 2.0 - vs.x, vs.y - (g.y + 0.0) / ng.y * vs.y * 2.0, 1.0));
        let c01 = normalize(vec3<f32>((g.x + 0.0) / ng.x * vs.x * 2.0 - vs.x, vs.y - (g.y + 1.0) / ng.y * vs.y * 2.0, 1.0));
        let c11 = normalize(vec3<f32>((g.x + 1.0) / ng.x * vs.x * 2.0 - vs.x, vs.y - (g.y + 1.0) / ng.y * vs.y * 2.0, 1.0));
        let axis = normalize(c00 + c10 + c01 + c11);
        let cos_a = dot(axis, c00);
        let dist_near = length(axis / axis.z * u.fwd.w);
        let mul = 1.0 / axis.z;
        let ranges = vec4<f32>(mul * min_tile_z + dist_near, mul * max_tile_z2 + dist_near, mul * min_tile_z2 + dist_near, mul * max_tile_z + dist_near);
        let ti = gid.y * u32(ng.x) + gid.x;
        tile_cones_rw[ti * 2u] = vec4<f32>(axis, cos_a);
        tile_cones_rw[ti * 2u + 1u] = ranges;
    }
}

// ================================================================================================================================
// 3. ObjectCullPS (DistanceFieldObjectCulling.usf:448) per tile: sphere / tile-cone + depth ranges + distance-field tests, appended into the tile list.
//    UE rasterises the (r + AOObjectMaxDistance) * 1.414 sphere first; the cone test below is stricter than that footprint, so the raster is skipped.
// ================================================================================================================================
var<workgroup> wg_count: atomic<u32>;
fn to_view(p: vec3<f32>) -> vec3<f32> { let d = p - u.eye.xyz; return vec3<f32>(dot(d, u.right.xyz), dot(d, u.up.xyz), dot(d, u.fwd.xyz)); }
fn from_view(v: vec3<f32>) -> vec3<f32> { return u.eye.xyz + u.right.xyz * v.x + u.up.xyz * v.y + u.fwd.xyz * v.z; }
fn sphere_intersect_cone(s: vec4<f32>, vertex: vec3<f32>, axis: vec3<f32>, cos_a: f32, sin_a: f32) -> bool {
    let uu = vertex - (s.w / sin_a) * axis;
    var d = s.xyz - uu;
    var dsq = dot(d, d);
    var e = dot(axis, d);
    if (e > 0.0 && e * e >= dsq * cos_a * cos_a) {
        d = s.xyz - vertex;
        dsq = dot(d, d);
        e = -dot(axis, d);
        if (e > 0.0 && e * e >= dsq * sin_a * sin_a) { return dsq <= s.w * s.w; }
        return true;
    }
    return false;
}
fn intersect_object_with_cone_depth_range(axis: vec3<f32>, cos_a: f32, sin_a: f32, range: vec2<f32>, minmax: vec2<f32>, oi: u32) -> bool {
    if (minmax.x > range.x && minmax.y < range.y) {
        let vcenter = axis * (0.5 * (range.x + range.y));
        let wcenter = from_view(vcenter);
        let along = 0.5 * (range.y - range.x);
        let far_edge = range.y * sin_a / cos_a;
        let radius = sqrt(along * along + far_edge * far_edge);
        let b = oi * OBJ_STRIDE;
        let lpe = objects[b + 5u].xyz;
        let uvs = objects[b + 6u];
        let vpos = (obj_w2v(oi) * vec4<f32>(wcenter, 1.0)).xyz;
        let bd = box_distance(-lpe, lpe, vpos) * abs(uvs.w);
        if (bd < radius + u.ao_a.x) {
            let clamped = clamp(vpos, -lpe, lpe);
            let dc = length(vpos - clamped);
            let d = (sample_mesh_df(clamped * uvs.xyz + objects[b + 7u].xyz, objects[b + 8u].xy) + dc) * abs(uvs.w);
            if (d < radius + u.ao_a.x) { return true; }
        }
    }
    return false;
}
@compute @workgroup_size(64, 1, 1)
fn cs_tile_cull(@builtin(workgroup_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let ti = gid.y * u32(u.tiles.x) + gid.x;
    if (li == 0u) { atomicStore(&wg_count, 0u); }
    workgroupBarrier();
    let cone = tile_cones[ti * 2u];
    let ranges = tile_cones[ti * 2u + 1u];
    let axis = cone.xyz;
    let cos_a = cone.w;
    let sin_a = sqrt(1.0 - cos_a * cos_a);
    let max_per_tile = u32(u.tiles.w);
    let base = ti * (max_per_tile + 1u);
    let n = u32(u.tiles.z);
    for (var k = li; k < n; k = k + 64u) {
        let oi = view_objects[k];
        let ws = objects[oi * OBJ_STRIDE];
        // RadiusScale = .5f: "A value of 1 is conservative, but has a huge impact on performance"
        let s = vec4<f32>(to_view(ws.xyz), ws.w + 0.5 * u.ao_a.x);
        if (sphere_intersect_cone(s, vec3<f32>(0.0), axis, cos_a, sin_a)) {
            let cd = dot(s.xyz, axis);
            let mm = vec2<f32>(cd + s.w, cd - s.w);
            var hit = intersect_object_with_cone_depth_range(axis, cos_a, sin_a, ranges.xy, mm, oi);
            if (!hit) { hit = intersect_object_with_cone_depth_range(axis, cos_a, sin_a, ranges.zw, mm, oi); }
            if (hit) {
                let slot = atomicAdd(&wg_count, 1u);
                if (slot < max_per_tile) { tile_lists_rw[base + 1u + slot] = oi; }
            }
        }
    }
    workgroupBarrier();
    if (li == 0u) { tile_lists_rw[base] = min(atomicLoad(&wg_count), max_per_tile); }
}

// ================================================================================================================================
// 4. ConeTraceGlobalOcclusionCS + ConeTraceObjectOcclusionCS + CombineConeVisibilityCS, one invocation per screen-grid cell (the InterlockedMin of
//    the two traces is a plain min here).
// ================================================================================================================================
// PERF (2026-10-06): true when the device filters R32Float (distance_field_ao.rs replaces this line): the clipmap is then read with ONE hardware trilinear
// sample like UE's Texture3DSampleLevel, instead of the 8 loads below (the global trace was ~90 % of the cone-trace pass).
const GDF_HW: bool = false;
@group(0) @binding(33) var gdf_samp: sampler;
fn sample_gdf(clip: u32, wp: vec3<f32>) -> f32 {
    // Texture3DSampleLevel with SF_Bilinear + AM_Wrap of the R16F clipmap at ComputeGlobalUV (GDF PARTIAL UPDATES: the scrolled, world-anchored addressing of
    // the partial updates, `u.clip_full`)
    let c = u.clip[clip];
    let res = u.clip_misc.x;
    let uvw = (wp - u.clip_full[clip].xyz) / (c.w * 2.0) + 0.5;
    // The four clipmaps are z slabs of one texture (z + clip * res): the sampler wraps x / y over the clipmap as AM_Wrap does; z (wrapped into the slab here)
    // is exact while the footprint stays inside this clipmap's slab, else (the slab edge, where z must wrap within the clipmap) the manual filter below.
    let wz = fract(uvw.z);
    let iz = floor(wz * res - 0.5);
    if (GDF_HW && iz >= 0.0 && iz <= res - 2.0) {
        return textureSampleLevel(gdf, gdf_samp, vec3<f32>(uvw.x, uvw.y, (f32(clip) + wz) / 4.0), 0.0).x;
    }
    let t = uvw * res - 0.5;
    let i0 = floor(t);
    let f = t - i0;
    let r = i32(res);
    var acc = 0.0;
    for (var k = 0; k < 8; k = k + 1) {
        let o = vec3<f32>(f32(k & 1), f32((k >> 1) & 1), f32((k >> 2) & 1));
        let ii = vec3<i32>(i0 + o);
        let w = ((ii % r) + r) % r;
        let wt = select(1.0 - f.x, f.x, o.x > 0.5) * select(1.0 - f.y, f.y, o.y > 0.5) * select(1.0 - f.z, f.z, o.z > 0.5);
        acc = acc + wt * textureLoad(gdf, vec3<i32>(w.x, w.y, w.z + i32(clip) * r), 0).x;
    }
    return acc;
}
fn cone_dir(i: u32, tx: vec3<f32>, ty: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let c = u.cones[i].xyz;
    return c.x * tx + c.y * ty + c.z * n;
}
fn trace_global_clipmap(clip: u32, p: vec3<f32>, n: vec3<f32>, tx: vec3<f32>, ty: vec3<f32>, vis: ptr<function, array<f32, 9>>) {
    let min_step = u.clip[clip].w * 2.0 / 300.0;
    let gmax = u.ao_b.x;
    let inv_gmax = 1.0 / gmax;
    let tan_cone = u.ao_b.y;
    for (var ci = 0u; ci < NUM_CONE_DIRECTIONS; ci = ci + 1u) {
        let d = cone_dir(ci, tx, ty, n);
        var min_vis = 1.0;
        // CONE_TRACE_AGAINST_OBJECTS: the global trace starts where the object trace ends
        var off = get_step_offset(f32(NUM_CONE_STEPS));
        for (var s = 0u; s < NUM_CONE_STEPS && off < gmax; s = s + 1u) {
            let dist = sample_gdf(clip, p + d * off);
            let r = off * tan_cone;
            var v = clamp(dist / r, 0.0, 1.0);
            let frac = (off + dist) * inv_gmax;
            v = max(v, clamp(frac * frac * 0.6, 0.0, 1.0));
            min_vis = min(min_vis, v);
            off = off + max(dist, min_step);
        }
        (*vis)[ci] = min((*vis)[ci], min_vis);
    }
}
fn trace_global(p: vec3<f32>, n: vec3<f32>, tx: vec3<f32>, ty: vec3<f32>, vis: ptr<function, array<f32, 9>>) {
    let gmax = u.ao_b.x;
    if (box_distance_inside(u.clip[0].xyz, vec3<f32>(u.clip[0].w), p) > gmax) { trace_global_clipmap(0u, p, n, tx, ty, vis); return; }
    if (box_distance_inside(u.clip[1].xyz, vec3<f32>(u.clip[1].w), p) > gmax) { trace_global_clipmap(1u, p, n, tx, ty, vis); return; }
    let d2 = box_distance_inside(u.clip[2].xyz, vec3<f32>(u.clip[2].w), p);
    let d3 = box_distance_inside(u.clip[3].xyz, vec3<f32>(u.clip[3].w), p);
    if (d2 > gmax) { trace_global_clipmap(2u, p, n, tx, ty, vis); }
    else if (d3 > gmax) { trace_global_clipmap(3u, p, n, tx, ty, vis); }
}
fn trace_object(oi: u32, p: vec3<f32>, n: vec3<f32>, tx: vec3<f32>, ty: vec3<f32>, vis: ptr<function, array<f32, 9>>) {
    // HemisphereConeTraceAgainstTileCulledObjects with USE_GLOBAL_DISTANCE_FIELD
    let max_off = get_step_offset(f32(NUM_CONE_STEPS));
    let inv_max_occ = 1.0 / u.ao_b.x;
    let b = oi * OBJ_STRIDE;
    let pr = objects[b];
    let dsq = dot(pr.xyz - p, pr.xyz - p);
    if (pr.w * pr.w / dsq <= 0.25 * 0.25) { return; } // "Skip tracing objects with a small projected angle"
    let lpe = objects[b + 5u].xyz;
    let uvs = objects[b + 6u];
    let vol_scale = abs(uvs.w);
    let w2v = obj_w2v(oi);
    let vp = (w2v * vec4<f32>(p, 1.0)).xyz;
    let bd = box_distance(-lpe, lpe, vp) * vol_scale;
    if (bd >= u.ao_a.x) { return; }
    let uva = objects[b + 7u];
    let mad = objects[b + 8u].xy;
    let occluder_radius = length(lpe) * 0.5 * vol_scale;
    let self_scale = 1.0 / max(uva.w, 0.0001);
    let tan_cone = u.ao_b.y;
    let w2v3 = mat3x3<f32>(w2v[0].xyz, w2v[1].xyz, w2v[2].xyz);
    for (var ci = 0u; ci < NUM_CONE_DIRECTIONS; ci = ci + 1u) {
        let d = cone_dir(ci, tx, ty, n);
        let ld = w2v3 * d;   // mul(RotatedConeDirection, (float3x3)WorldToVolume)
        var min_vis = 1.0;
        var off = get_step_offset(0.0) + 2.0;
        var cur = off;
        for (var s = 0u; s < NUM_CONE_STEPS && off < max_off; s = s + 1u) {
            let sp = vp + ld * off;
            let cl = clamp(sp, -lpe, lpe);
            let dc = length(sp - cl);
            let occ = (sample_mesh_df(cl * uvs.xyz + uva.xyz, mad) + dc) * vol_scale;
            let r = off * tan_cone;
            let inv_r = 1.0 / r;
            var v = clamp(occ * inv_r, 0.0, 1.0);
            let small = 1.0 - clamp(occluder_radius * inv_r, 0.0, 1.0);
            let selfv = 1.0 - clamp(off * self_scale, 0.0, 1.0);
            v = max(v, max(small, selfv));
            let frac = (off + occ) * inv_max_occ;
            v = max(v, clamp(frac * frac * 0.6, 0.0, 1.0));
            min_vis = min(v, min_vis);
            let next = get_step_offset(f32(s + 1u));
            let min_step = 2.0 * (next - cur); // MinStepScale = 2 with the global distance field
            cur = next;
            off = off + max(occ, min_step);
        }
        (*vis)[ci] = min((*vis)[ci], min_vis);
    }
}
@compute @workgroup_size(8, 8, 1)
fn cs_cone_trace(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= u.grid.x || f32(id.y) >= u.grid.y) { return; }
    let coord = vec2<f32>(id.xy);
    let jit = u.jitter.xy;
    // GetBaseLevelScreenUVFromScreenGrid / GetScreenUVFromScreenGrid
    let base_uv = (coord * f32(TRACE_DOWNSAMPLE_FACTOR) + jit + 0.5) * u.half_size.zw;
    let screen_uv = ((coord * f32(TRACE_DOWNSAMPLE_FACTOR) + jit) * 2.0 + 0.5) * u.size.zw;
    let g = ds_gbuffer(base_uv);
    let n = normalize(g.xyz);
    let depth = g.w;
    let axes = find_best_axis_vectors2(n);
    var vis: array<f32, 9>;
    for (var i = 0u; i < NUM_CONE_DIRECTIONS; i = i + 1u) { vis[i] = 1.0; }
    if (depth < u.ao_a.w + u.ao_b.x) {
        let p = view_pos(screen_uv, depth);
        if (u.clip_misc.w > 0.5) { trace_global(p, n, axes[0], axes[1], &vis); }
        let tile = vec2<u32>(id.xy) / CONE_TILE_SIZEX;
        if (f32(tile.x) < u.tiles.x && f32(tile.y) < u.tiles.y) {
            let ti = tile.y * u32(u.tiles.x) + tile.x;
            let base = ti * (u32(u.tiles.w) + 1u);
            let cnt = tile_lists[base];
            for (var k = 0u; k < cnt; k = k + 1u) { trace_object(tile_lists[base + 1u + k], p, n, axes[0], axes[1], &vis); }
        }
    }
    // CombineConeVisibilityCS
    var unoccluded = vec3<f32>(0.0);
    for (var i = 0u; i < NUM_CONE_DIRECTIONS; i = i + 1u) { unoccluded = unoccluded + vis[i] * cone_dir(i, axes[0], axes[1], n); }
    let bent = unoccluded * (u.ao_b.z / f32(NUM_CONE_DIRECTIONS));
    textureStore(bent_low_out, vec2<i32>(id.xy), vec4<f32>(bent, depth));
}

// ================================================================================================================================
// 5. GeometryAwareUpsamplePS (DistanceFieldScreenGridLighting.usf:449) into the AO buffer (W/2 x H/2) + confidence.
// ================================================================================================================================
fn low_tap(uv: vec2<f32>) -> vec4<f32> {
    // BentNormalAOSampler: SF_Bilinear, AM_Clamp, sampled at texel centres
    let sz = vec2<i32>(u.grid.xy);
    let t = clamp(vec2<i32>(floor(uv * u.grid.xy)), vec2<i32>(0), sz - vec2<i32>(1));
    return textureLoad(bent_low, t, 0);
}
fn position_weight(plane: vec4<f32>, uv: vec2<f32>, depth: f32) -> f32 {
    let wp = view_pos(uv, depth);
    let pd = dot(plane, vec4<f32>(wp, 1.0));
    let rel = 1000.0 * abs(pd) / depth;
    return min(10.0 / (rel + 0.0001), 1.0);
}
fn q8(x: f32) -> f32 { return floor(clamp(x, 0.0, 1.0) * 255.0 + 0.5) / 255.0; } // PF_G8 confidence
@compute @workgroup_size(8, 8, 1)
fn cs_upsample(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= u.half_size.x || f32(id.y) >= u.half_size.y) { return; }
    let uv = (vec2<f32>(id.xy) + 0.5) * u.half_size.zw;
    let g = ds_gbuffer(uv);
    let n = normalize(g.xyz);
    let depth = g.w;
    let wp = view_pos(uv, depth);
    let plane = vec4<f32>(n, -dot(wp, n));
    let low_size = u.grid.xy;
    let low_texel = u.grid.zw;
    let gtexel = u.half_size.zw; // DistanceFieldGBufferTexelSize
    let jit = u.jitter.xy;
    let c00 = floor((uv - jit * gtexel) * low_size) * low_texel;
    let bw = (uv - (c00 + jit * gtexel)) * low_size;
    let lc = c00 + 0.5 * low_texel;
    let t00 = low_tap(lc);
    let t10 = low_tap(lc + vec2<f32>(low_texel.x, 0.0));
    let t01 = low_tap(lc + vec2<f32>(0.0, low_texel.y));
    let t11 = low_tap(lc + low_texel);
    let cw = vec4<f32>((1.0 - bw.y) * (1.0 - bw.x), (1.0 - bw.y) * bw.x, bw.y * (1.0 - bw.x), bw.y * bw.x);
    let pw = vec4<f32>(position_weight(plane, lc, t00.w), position_weight(plane, lc + vec2<f32>(low_texel.x, 0.0), t10.w),
        position_weight(plane, lc + vec2<f32>(0.0, low_texel.y), t01.w), position_weight(plane, lc + low_texel, t11.w));
    let fr = c00 + (jit + 0.5) * gtexel;
    let nw = max(vec4<f32>(dot(normalize(ds_gbuffer(fr).xyz), n), dot(normalize(ds_gbuffer(fr + vec2<f32>(low_texel.x, 0.0)).xyz), n),
        dot(normalize(ds_gbuffer(fr + vec2<f32>(0.0, low_texel.y)).xyz), n), dot(normalize(ds_gbuffer(fr + low_texel).xyz), n)), vec4<f32>(0.0001));
    let fw = cw * pw * nw;
    let inv = 1.0 / max(dot(fw, vec4<f32>(1.0)), 0.00001);
    var avg = (fw.x * t00.xyz + fw.y * t10.xyz + fw.z * t01.xyz + fw.w * t11.xyz) * inv;
    let conf = select(1.0, 0.0, dot(pw, vec4<f32>(1.0)) < 0.5);
    var bl = length(avg);
    let avg_len = (fw.x * length(t00.xyz) + fw.y * length(t10.xyz) + fw.z * length(t01.xyz) + fw.w * length(t11.xyz)) * inv;
    if (bl < avg_len && bl > 0.0) { avg = avg / bl * avg_len; }
    // fade to unoccluded in the distance
    bl = length(avg);
    let fade = clamp((u.ao_a.w - depth) * u.ao_b.w, 0.0, 1.0);
    let nb = avg / max(bl, 0.0001);
    avg = nb * mix(1.0, bl, fade);
    textureStore(bent_half_out, vec2<i32>(id.xy), vec4<f32>(avg, depth));
    textureStore(conf_out, vec2<i32>(id.xy), vec4<f32>(q8(conf), 0.0, 0.0, 0.0));
}

// ================================================================================================================================
// 6. UpdateHistoryDepthRejectionPS (DistanceFieldLightingPost.usf:50), hardware-interpolation path (MANUAL_HISTORY_RESAMPLE 0). The motion is the
//    camera motion of a static scene (no velocity buffer: nothing in the DF scene moves).
// ================================================================================================================================
@compute @workgroup_size(8, 8, 1)
fn cs_history(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= u.half_size.x || f32(id.y) >= u.half_size.y) { return; }
    let new_value = textureLoad(bent_half, vec2<i32>(id.xy), 0);
    let new_conf = textureLoad(conf_tex, vec2<i32>(id.xy), 0).x;
    let depth = new_value.w;
    if (u.jitter.z < 0.5) {
        // no history yet (or a camera cut): "Use the current frame's mask for next frame's history"
        textureStore(hist_new_out, vec2<i32>(id.xy), vec4<f32>(new_value.xyz, depth));
        textureStore(conf_new_out, vec2<i32>(id.xy), vec4<f32>(new_conf, 0.0, 0.0, 0.0));
        return;
    }
    let uv = (vec2<f32>(id.xy) + 0.5) * u.half_size.zw;
    let wp = view_pos(uv, depth);
    let old_uv = prev_uv_of(wp);
    let speed = length(old_uv - uv);
    let max_uv = vec2<f32>(1.0) - 2.0 * u.size.zw; // (ViewSize - 1 * DOWNSAMPLE_FACTOR) * BufferInvSize
    var w = u.hist.x;
    if (any(old_uv > max_uv) || any(old_uv < vec2<f32>(0.0))) { w = 0.0; }
    let min_conf_scale = mix(1.0, u.hist.y, clamp(speed, 0.0, 1.0));
    let hv = textureSampleLevel(hist_prev, lin_samp, old_uv, 0.0);
    let hc = textureSampleLevel(conf_prev, lin_samp, old_uv, 0.0).x;
    // ComputeHistoryWeightBasedOnPosition
    let prev_wp = prev_view_pos(old_uv, hv.w);
    let rel_threshold = u.hist.z / 1000.0;
    let pos_w = select(1.0, 0.0, length(prev_wp - wp) / depth > rel_threshold);
    w = w * pos_w;
    w = w * clamp(hc, min_conf_scale, 1.0);
    var out_bent = mix(new_value.xyz, hv.xyz, w);
    let out_conf = mix(new_conf, hc, w);
    let conf_threshold = mix(0.0, 0.99, clamp(speed, 0.0, 1.0));
    if (hc < conf_threshold) { w = 0.0; }
    if (any(out_bent != out_bent)) { out_bent = vec3<f32>(0.0); }
    var a = depth;
    if (u.jitter.w > 0.5) { a = a * select(-1.0, 1.0, w > 0.0); }
    textureStore(hist_new_out, vec2<i32>(id.xy), vec4<f32>(out_bent, a));
    textureStore(conf_new_out, vec2<i32>(id.xy), vec4<f32>(q8(out_conf), 0.0, 0.0, 0.0));
}

// ================================================================================================================================
// 7. FilterHistoryPS (DistanceFieldLightingPost.usf:244): spatial fill of the pixels whose history was rejected.
// ================================================================================================================================
@compute @workgroup_size(8, 8, 1)
fn cs_history_filter(@builtin(global_invocation_id) id: vec3<u32>) {
    if (f32(id.x) >= u.half_size.x || f32(id.y) >= u.half_size.y) { return; }
    var hv = textureLoad(hist_new, vec2<i32>(id.xy), 0);
    let hc = textureLoad(conf_new, vec2<i32>(id.xy), 0).x;
    if (u.jitter.z > 0.5 && hv.w < 0.0) {
        let depth = abs(hv.w);
        var acc = vec4<f32>(0.0);
        let sz = vec2<i32>(u.half_size.xy);
        for (var y = -2; y <= 2; y = y + 1) {
            for (var x = -2; x <= 2; x = x + 1) {
                // BentNormalAOSampler (bilinear, clamp) at texel centres
                let t = clamp(vec2<i32>(id.xy) + vec2<i32>(x, y), vec2<i32>(0), sz - vec2<i32>(1));
                let tv = textureLoad(hist_new, t, 0);
                let sd = abs(tv.w);
                let valid = select(0.0, 1.0, tv.w > 0.0);
                let dw = exp2(-1000.0 * abs(depth - sd) / depth);
                let w2 = exp2(-abs(vec2<f32>(f32(x), f32(y)) * 10.0 / 2.0));
                let wgt = valid * max(w2.x, w2.y) * dw;
                acc = acc + vec4<f32>(tv.xyz * wgt, wgt);
            }
        }
        if (acc.w > 0.0) { hv = vec4<f32>(mix(hv.xyz, acc.xyz / acc.w, u.hist.x), hv.w); }
    }
    textureStore(state_out, vec2<i32>(id.xy), vec4<f32>(hv.xyz, abs(hv.w)));
    textureStore(state_conf_out, vec2<i32>(id.xy), vec4<f32>(hc, 0.0, 0.0, 0.0));
}

// ================================================================================================================================
// 8. CompositeObjectDistanceFieldsCS + CompositeHeightfieldsIntoGlobalDistanceFieldCS (GlobalDistanceField.usf:204 / 314) for ONE UPDATE REGION of a clipmap
//    (GDF PARTIAL UPDATES, 2026-10-06: global_distance_field.rs; a full update is the one region covering the clipmap). The cull grid (CullGridDimension =
//    ceil(UpdateRegionSize / 16) cells of 16^3 texels) is built by cs_gdf_cull (CullObjectsToGridCS, section 9) for the same region just before.
//    WorldPosition = UpdateRegionVolumeMin + (DispatchThreadId + .5) * VolumeTexelSize * GlobalVolumeExtent * 2, written at
//    IntCoordinate = frac(ComputeGlobalUV(WorldPosition)) * GlobalVolumeDimension (the scrolled toroidal addressing, `gc.full`).
// ================================================================================================================================
struct GdfClip {
    center_extent: vec4<f32>,   // xyz UpdateRegionVolumeMin (DF cm), w the clipmap's extent (GlobalVolumeCenterAndExtent.w)
    misc: vec4<f32>,            // x clipmap index, y AOGlobalMaxSphereQueryRadius, z resolution, w the clipmap's volume-culled list offset in gdf_near
    hf: vec4<f32>,              // heightfield: x 0 none / 1 flat plate (y height cm, z/w half size of its xy footprint cm) / 2 landscape lattice hf_tex (y cell cm, zw origin cm)
    region: vec4<f32>,          // xyz UpdateRegionSize (texels), w unused
    grid: vec4<f32>,            // xyz CullGridDimension (cells per axis), w unused
    full: vec4<f32>,            // xyz FullUpdateOrigin * CellSize of the clipmap (DF cm): ComputeGlobalUV's origin
}
@group(0) @binding(34) var hf_tex: texture_2d<f32>;   // TERRAIN MAIN PASS: the landscape heights (DF cm) on its lattice (DfAo::set_heightfield)
fn f16q(x: f32) -> f32 { return unpack2x16float(pack2x16float(vec2<f32>(x, 0.0))).x; } // PF_R16F storage
@compute @workgroup_size(4, 4, 4)
fn cs_gdf_composite(@builtin(global_invocation_id) id: vec3<u32>) {
    let res = gc.misc.z;
    let size = vec3<u32>(gc.region.xyz);
    // `if (all(DispatchThreadId.xyz < UpdateRegionSize))` guards the write; the threads beyond the region compute nothing else of use
    if (id.x >= size.x || id.y >= size.y || id.z >= size.z) { return; }
    let c = gc.center_extent;
    let texel = 1.0 / res;
    let wpos = c.xyz + (vec3<f32>(id.xyz) + 0.5) * texel * c.w * 2.0;
    let radius = gc.misc.y;
    var min_d = radius;
    let gdim = vec3<u32>(gc.grid.xyz);
    let cell = id / 16u;
    let cbase = ((cell.z * gdim.y + cell.y) * gdim.x + cell.x) * (2047u + 1u);
    let cnt = min(grid_lists[cbase], 2047u);
    for (var k = 0u; k < cnt; k = k + 1u) {
        let oi = grid_lists[cbase + 1u + k];
        let b = oi * OBJ_STRIDE;
        let lpe = objects[b + 5u].xyz;
        let uvs = objects[b + 6u];
        let vs = abs(uvs.w);
        let vp = (obj_w2v(oi) * vec4<f32>(wpos, 1.0)).xyz;
        let bd = box_distance(-lpe, lpe, vp) * vs;
        if (bd < radius) {
            let cl = clamp(vp, -lpe, lpe);
            let dc = length(vp - cl);
            let d = (sample_mesh_df(cl * uvs.xyz + objects[b + 7u].xyz, objects[b + 8u].xy) + dc) * vs;
            min_d = min(min_d, d);
        }
    }
    // TERRAIN MAIN PASS (2026-10-07): CompositeHeightfieldsIntoGlobalDistanceFieldCS (GlobalDistanceField.usf:314-360): inside the heightfield's footprint
    // (MinMaxHeightfieldUV; a lattice cell touching a hole is outside) DistanceToHeightfieldPlane = dot(N, WorldPosition - (xy, Height)) with the heightfield
    // normal under the point, clamped to +-AOGlobalMaxSphereQueryRadius. Height bilinear over the lattice cell (textureLoad: no filterable R32Float on WebGPU),
    // N from the cell's height gradient (UE reads its normal texture: ASSUMED equivalent).
    if (gc.hf.x > 1.5) {
        let dims = vec2<i32>(textureDimensions(hf_tex));
        let g = (wpos.xy - gc.hf.zw) / gc.hf.y;
        if (all(g >= vec2<f32>(0.0)) && all(g <= vec2<f32>(dims - vec2<i32>(1)))) {
            let i0 = min(vec2<i32>(floor(g)), dims - vec2<i32>(2));
            let f = g - vec2<f32>(i0);
            let h00 = textureLoad(hf_tex, i0, 0).x;
            let h10 = textureLoad(hf_tex, i0 + vec2<i32>(1, 0), 0).x;
            let h01 = textureLoad(hf_tex, i0 + vec2<i32>(0, 1), 0).x;
            let h11 = textureLoad(hf_tex, i0 + vec2<i32>(1, 1), 0).x;
            if (min(min(h00, h10), min(h01, h11)) > -1.0e8) {
                let h = mix(mix(h00, h10, f.x), mix(h01, h11, f.x), f.y);
                let dx = mix(h10 - h00, h11 - h01, f.y) / gc.hf.y;
                let dy = mix(h01 - h00, h11 - h10, f.x) / gc.hf.y;
                let n = normalize(vec3<f32>(-dx, -dy, 1.0));
                min_d = min(min_d, clamp(n.z * (wpos.z - h), -radius, radius));
            }
        }
    } else if (gc.hf.x > 0.5 && abs(wpos.x) < gc.hf.z && abs(wpos.y) < gc.hf.w) {
        // no landscape: the old flat plate plane (normal +Z) over its footprint
        min_d = min(min_d, clamp(wpos.z - gc.hf.y, -radius, radius));
    }
    let r = u32(res);
    let guv = (wpos - gc.full.xyz) / (c.w * 2.0) + 0.5;
    let ic = min(vec3<u32>(fract(guv) * res), vec3<u32>(r - 1u));
    textureStore(gdf_out, vec3<i32>(vec3<u32>(ic.x, ic.y, ic.z + u32(gc.misc.x) * r)), vec4<f32>(f16q(min_d), 0.0, 0.0, 0.0));
}

// ================================================================================================================================
// 9. CullObjectsToGridCS (GlobalDistanceField.usf:139, UE4.16) for one update region: one workgroup of 256 threads per cull-grid cell (16^3 texels), the
//    cell's count cleared by thread 0, then every object of the clipmap's volume-culled list (CullObjectsForVolumeCS, done on the CPU: `gdf_near` from
//    gc.misc.w = count, indices) whose bounding sphere is within AOGlobalMaxSphereQueryRadius of the cell box is appended with an atomic counter (InterlockedAdd).
//    PERF (2026-10-05): was the same loop on the CPU, re-uploading 512 cells x 2048 slots (4 MB) per clipmap update.
// ================================================================================================================================
@group(0) @binding(31) var<storage, read> gdf_near: array<u32>;
@group(0) @binding(32) var<storage, read_write> grid_rw: array<atomic<u32>>;
@compute @workgroup_size(256, 1, 1)
fn cs_gdf_cull(@builtin(workgroup_id) g: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let gdim = vec3<u32>(gc.grid.xyz);
    if (g.x >= gdim.x || g.y >= gdim.y || g.z >= gdim.z) { return; }
    let base = ((g.z * gdim.y + g.y) * gdim.x + g.x) * (2047u + 1u);
    if (li == 0u) { atomicStore(&grid_rw[base], 0u); }
    storageBarrier();
    workgroupBarrier();
    let extent = gc.center_extent.w;
    let texel = 2.0 * extent / gc.misc.z;
    let vmin = gc.center_extent.xyz;
    let tile_ext = vec3<f32>(0.5 * 16.0 * texel);
    let tc = vmin + ((vec3<f32>(g) + 0.5) * 16.0 + 0.5) * texel;
    let radius = gc.misc.y;
    let nb = u32(gc.misc.w);
    let n = gdf_near[nb];
    for (var k = li; k < n; k = k + 256u) {
        let oi = gdf_near[nb + 1u + k];
        let o = objects[oi * OBJ_STRIDE];
        let a = max(abs(o.xyz - tc) - tile_ext, vec3<f32>(0.0));
        let comb = o.w + radius;
        if (dot(a, a) < comb * comb) {
            let d = atomicAdd(&grid_rw[base], 1u);
            if (d < 2047u) { atomicStore(&grid_rw[base + 1u + d], oi); }
        }
    }
}
