// Resolve of the GBuffer the ORIGINAL base-pass pixel program of the gun material writes (GBufferA normal, B metallic / specular / roughness, C base colour, the
// scene-colour target = emissive + sky-light diffuse the program itself evaluated from the View sky SH). Appended after common.wgsl + lit_common.wgsl, so the
// camera / light blocks, the cascaded sun shadows, the fog and the HDR tail are the renderer's own.
//
// The lighting model below is the UE 4.16 DefaultLit BxDF of the deferred directional light (Lambert diffuse, D_GGX * Vis_SmithJointApprox * F_Schlick specular,
// DiffuseColor = BaseColor * (1 - Metallic), SpecularColor = lerp(0.08 * Specular, BaseColor, Metallic)); it is NOT recovered from the 1.8 shader bytecode (the
// deferred light programs live in the global shader cache and need the scene texture set), the sun / sky constants it consumes are the renderer's recovered ones.
// Not applied: reflection captures / screen space reflections and the AO passes of the deferred path (the renderer has none; the AO of post.wgsl applies to the
// ambient share stored in the HDR alpha, as for every other surface).
const GUN_PI: f32 = 3.14159265;
fn gun_f_schlick(spec: vec3<f32>, vh: f32) -> vec3<f32> {
    let fc = pow(clamp(1.0 - vh, 0.0, 1.0), 5.0);
    return clamp(50.0 * spec.g, 0.0, 1.0) * fc + (1.0 - fc) * spec;
}
fn gun_resolve(world: vec3<f32>, n: vec3<f32>, indirect: vec3<f32>, base: vec3<f32>, metallic: f32, specular: f32, roughness_in: f32) -> vec4<f32> {
    let cam = camera.position.xyz;
    if (light.misc.x <= 0.5) { return output_hdr(base, vec3<f32>(0.0), world, cam); }
    let v = normalize(cam - world);
    let l = light.sun_dir.xyz;
    let nl = clamp(dot(n, l), 0.0, 1.0);
    let vis = sun_visibility(world, n, nl);
    let diffuse_color = base * (1.0 - metallic);
    let spec_color = mix(vec3<f32>(0.08 * specular), base, metallic);
    let roughness = clamp(roughness_in, 0.02, 1.0);
    let h = normalize(v + l);
    let nv = max(dot(n, v), 1e-3);
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let vh = clamp(dot(v, h), 0.0, 1.0);
    let a = roughness * roughness;
    let a2 = a * a;
    let dd = (nh * a2 - nh) * nh + 1.0;
    let d = a2 / (GUN_PI * dd * dd);
    let vis_s = 0.5 / (nl * (nv * (1.0 - a) + a) + nv * (nl * (1.0 - a) + a) + 1e-5);
    let sun = light.sun_radiance.xyz;
    // sun_radiance carries the Lambert 1/pi (scene.wgsl); the specular lobe is not divided by pi, so its light colour is sun_radiance * pi
    let lit = indirect + diffuse_color * sun * nl * vis + sun * GUN_PI * d * vis_s * gun_f_schlick(spec_color, vh) * nl * vis;
    return output_hdr(lit, indirect, world, cam);
}
