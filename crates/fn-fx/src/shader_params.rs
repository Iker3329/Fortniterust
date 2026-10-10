//! The loose shader parameters a cooked material shader serialized for itself (its `FShader::Serialize` chain), read from the shader-map entry in the `.uexp` instead of
//! being inferred from the program: where the per-frame expressions (`UE_Material_PerFrame*ExpressionN`) and the base-pass vertex shader's velocity globals live in the
//! unnamed (`$Globals`) constant buffer.
//!
//! Layout (UE4.16 source; verified byte for byte on `M_SafeZoneStorm`'s `TBasePassVSFNoLightMapPolicy`, see the module tests):
//! * entry = `FName Type + u32 EndOffset` (12 bytes, `fn_dxbc::unreal_entry`), then `FShader::SerializeBase` (Shader.cpp:910): the virtual `Serialize`, then
//!   `OutputHash (20) MaterialShaderMapHash (20) ShaderPipeline (FName) VFType (FName) VFSourceHash (20) Type (FName) SourceHash (20) Target ...`;
//! * `FMaterialShader::Serialize` (ShaderBaseClasses.cpp:399): `MaterialUniformBuffer, ParameterCollectionUniformBuffers, DeferredParameters, SceneColorCopyTexture(+Sampler),
//!   FDebugUniformExpressionSet (6 x i32: vectors, scalars, per-frame scalars, per-frame vectors, 2D textures, cube textures; MaterialShader.h:89), the debug layout
//!   (FName "Material", u32 ConstantBufferSize, u32 ResourceOffset, TArray<u8> Resources), FString DebugDescription, EyeAdaptation (FShaderResourceParameter: u16 BaseIndex,
//!   u16 NumResources), TArray<FShaderParameter> PerFrameScalarExpressions, PerFrameVectorExpressions, PerFramePrevScalarExpressions, PerFramePrevVectorExpressions`;
//! * `FShaderParameter` (ShaderParameters.cpp:36) = `u16 BaseIndex (bytes into the buffer), u16 NumBytes (0 = unbound), u16 BufferIndex (constant buffer slot)`;
//! * `FMeshMaterialShader::Serialize` (ShaderBaseClasses.cpp:518): `FVertexFactoryParameterRef` (VertexFactory.cpp:317: FName VF type, u8 frequency, 20-byte VFHash,
//!   i32 SkipOffset = absolute file offset of the end of the VF parameters, the VF parameters), then `NonInstancedDitherLODFactorParameter`;
//! * `TBasePassVertexShaderPolicyParamType::Serialize` (BasePassRendering.h:348) ends with `PreviousLocalToWorldParameter, SkipOutputVelocityParameter,
//!   InstancedEyeIndexParameter, IsInstancedStereoParameter` (four `FShaderParameter`s right before `OutputHash`).

/// `FShaderParameter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ShaderParam { pub base_index: u16, pub num_bytes: u16, pub buffer_index: u16 }
impl ShaderParam {
    pub fn is_bound(&self) -> bool { self.num_bytes > 0 }
    /// First float written (BaseIndex is in bytes).
    pub fn float_index(&self) -> usize { self.base_index as usize / 4 }
}

/// What the serialized `FMaterialShader` / `FMeshMaterialShader` chain of one cooked program says about its loose parameters.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MaterialShaderParams {
    /// `FDebugUniformExpressionSet`: vectors, scalars, per-frame scalars, per-frame vectors, 2D textures, cube textures.
    pub debug_counts: [i32; 6],
    /// `ConstantBufferSize` of the `Material` uniform buffer the shader was compiled against (bytes).
    pub material_cb_size: u32,
    pub per_frame_scalars: Vec<ShaderParam>,
    pub per_frame_vectors: Vec<ShaderParam>,
    pub per_frame_prev_scalars: Vec<ShaderParam>,
    pub per_frame_prev_vectors: Vec<ShaderParam>,
    /// Vertex factory type of a mesh material shader (`None` = plain material shader).
    pub vertex_factory: Option<String>,
    pub non_instanced_dither_lod_factor: Option<ShaderParam>,
    /// Bytes of the derived shader type's own parameters (after `FMeshMaterialShader`, up to the end of `Serialize`).
    pub derived: Vec<u8>,
    /// GRASS (2026-10-06): the vertex factory's own serialized parameters (`FVertexFactoryParameterRef` after its SkipOffset, up to that offset).
    pub vf_params: Vec<u8>,
}

/// GRASS (2026-10-06): `FInstancedStaticMeshVertexFactoryShaderParameters` (UE4.16 InstancedStaticMesh.h:281 `Serialize`: `FLocalVertexFactoryShaderParameters`
/// = `LODParameter` (6 bytes) + `bAnySpeedTreeParamIsBound` (bool, 4 bytes; LocalVertexFactory.cpp:40), then InstancingFadeOutParams, InstancingViewZCompareZero,
/// InstancingViewZCompareOne, InstancingViewZConstant, InstancingWorldViewOriginZero, InstancingWorldViewOriginOne, CPUInstanceOrigin, CPUInstanceTransform,
/// CPUInstanceLightmapAndShadowMapBias): the loose `$Globals` values `SetMesh` writes (InstancedStaticMesh.cpp:1903).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct InstancedVfParams {
    pub fade_out_params: ShaderParam,
    pub view_z_compare_zero: ShaderParam,
    pub view_z_compare_one: ShaderParam,
    pub view_z_constant: ShaderParam,
    pub world_view_origin_zero: ShaderParam,
    pub world_view_origin_one: ShaderParam,
    pub cpu_instance_origin: ShaderParam,
    pub cpu_instance_transform: ShaderParam,
    pub cpu_lightmap_bias: ShaderParam,
}
/// Serialized size of [`InstancedVfParams`] with its `FLocalVertexFactoryShaderParameters` head: 6 + 4 + 9 x 6.
pub const INSTANCED_VF_PARAM_BYTES: usize = 64;
impl MaterialShaderParams {
    /// GRASS: the instanced static-mesh vertex factory's loose parameters of a program compiled for `FInstancedStaticMeshVertexFactory` (`None` for another
    /// vertex factory or a layout that does not validate: every bound value is one float4 except `CPUInstanceTransform` (float3x4)).
    pub fn instanced_vf(&self) -> Option<InstancedVfParams> {
        if self.vertex_factory.as_deref() != Some("FInstancedStaticMeshVertexFactory") || self.vf_params.len() != INSTANCED_VF_PARAM_BYTES { return None; }
        let p = |k: usize| param(&self.vf_params[10 + 6 * k..16 + 6 * k]);
        let v = InstancedVfParams { fade_out_params: p(0), view_z_compare_zero: p(1), view_z_compare_one: p(2), view_z_constant: p(3), world_view_origin_zero: p(4), world_view_origin_one: p(5), cpu_instance_origin: p(6), cpu_instance_transform: p(7), cpu_lightmap_bias: p(8) };
        let float4s = [v.fade_out_params, v.view_z_compare_zero, v.view_z_compare_one, v.view_z_constant, v.world_view_origin_zero, v.world_view_origin_one, v.cpu_instance_origin, v.cpu_lightmap_bias];
        (float4s.iter().all(|x| !x.is_bound() || x.num_bytes == 16) && (!v.cpu_instance_transform.is_bound() || v.cpu_instance_transform.num_bytes == 48)).then_some(v)
    }
}

/// LANDSCAPE MATERIAL (2026-10-07): `FLandscapeVertexFactoryVertexShaderParameters::Serialize` (UE4.16 LandscapeRender.cpp:2314): HeightmapTexture +
/// sampler, LodValues, NeighborSectionLod (float4[4]), LodBias, SectionLods, XYOffsetmapTexture + sampler. MEASURED on Athena_Terrain_LS_00's programs:
/// t0 / s0, LodBias cb0[2], LodValues cb0[3], SectionLods cb0[4], NeighborSectionLod cb0[5..8] (the base-pass VS asm reads them there).
pub const LANDSCAPE_VS_VF_BYTES: usize = 40;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LandscapeVsParams { pub heightmap: ShaderResource, pub heightmap_sampler: ShaderResource, pub lod_values: ShaderParam, pub neighbor_section_lod: ShaderParam, pub lod_bias: ShaderParam, pub section_lods: ShaderParam, pub xy_offset: ShaderResource, pub xy_offset_sampler: ShaderResource }
/// LANDSCAPE MATERIAL: `FLandscapeVertexFactoryPixelShaderParameters::Serialize` (LandscapeRender.cpp:2441): NormalmapTexture + sampler, LocalToWorldNoScaling.
/// MEASURED: t8 / s6, cb0[9..12] in the base-pass PS.
pub const LANDSCAPE_PS_VF_BYTES: usize = 14;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LandscapePsParams { pub normalmap: ShaderResource, pub normalmap_sampler: ShaderResource, pub local_to_world_no_scaling: ShaderParam }
fn resource(b: &[u8]) -> ShaderResource { ShaderResource { base_index: u16::from_le_bytes([b[0], b[1]]), num_resources: u16::from_le_bytes([b[2], b[3]]) } }
impl MaterialShaderParams {
    fn is_landscape(&self) -> bool { matches!(self.vertex_factory.as_deref(), Some("FLandscapeVertexFactory") | Some("FLandscapeXYOffsetVertexFactory")) }
    /// LANDSCAPE MATERIAL: the vertex-stage landscape VF parameters (a vertex program of the landscape vertex factories).
    pub fn landscape_vs(&self) -> Option<LandscapeVsParams> {
        if !self.is_landscape() || self.vf_params.len() != LANDSCAPE_VS_VF_BYTES { return None; }
        let b = &self.vf_params;
        Some(LandscapeVsParams { heightmap: resource(&b[0..4]), heightmap_sampler: resource(&b[4..8]), lod_values: param(&b[8..14]), neighbor_section_lod: param(&b[14..20]), lod_bias: param(&b[20..26]), section_lods: param(&b[26..32]), xy_offset: resource(&b[32..36]), xy_offset_sampler: resource(&b[36..40]) })
    }
    /// LANDSCAPE MATERIAL: the pixel-stage landscape VF parameters.
    pub fn landscape_ps(&self) -> Option<LandscapePsParams> {
        if !self.is_landscape() || self.vf_params.len() != LANDSCAPE_PS_VF_BYTES { return None; }
        let b = &self.vf_params;
        Some(LandscapePsParams { normalmap: resource(&b[0..4]), normalmap_sampler: resource(&b[4..8]), local_to_world_no_scaling: param(&b[8..14]) })
    }
}

/// The base-pass vertex shader's velocity globals (BasePassRendering.h:331, set in BasePassRendering.inl:31).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct BasePassVsGlobals { pub previous_local_to_world: ShaderParam, pub skip_output_velocity: ShaderParam, pub instanced_eye_index: ShaderParam, pub is_instanced_stereo: ShaderParam }
/// FX SHADOW: the shadow-depth vertex shader's globals (`FShadowDepthShaderParameters`, ShadowRendering.h:1081; set in :1089 `Set`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ShadowDepthVsGlobals { pub projection_matrix: ShaderParam, pub shadow_params: ShaderParam, pub clamp_to_near_plane: ShaderParam }

impl MaterialShaderParams {
    /// The last four parameters of a `TBasePassVS*` program.
    pub fn base_pass_vs(&self, shader_type: &str) -> Option<BasePassVsGlobals> {
        if !shader_type.starts_with("TBasePassVS") || self.derived.len() < 24 { return None; }
        let t = &self.derived[self.derived.len() - 24..];
        let p = |k: usize| param(&t[6 * k..6 * k + 6]);
        Some(BasePassVsGlobals { previous_local_to_world: p(0), skip_output_velocity: p(1), instanced_eye_index: p(2), is_instanced_stereo: p(3) })
    }
    /// FX SHADOW: `FShadowDepthShaderParameters` of a `TShadowDepthVS*` program, the first parameters `FShadowDepthVS::Serialize` writes after
    /// `FMeshMaterialShader` (UE4.16 ShadowDepthRendering.cpp:66; `operator<<` ShadowRendering.h:1117: ProjectionMatrix, ShadowParams, ClampToNearPlane).
    /// Measured on `Wood_lvl1_Wall` (OutputDepthfalse): ProjectionMatrix = cb0 bytes 112..176, ShadowParams = 96..104, bClampToNearPlane = 176..180.
    pub fn shadow_depth_vs(&self, shader_type: &str) -> Option<ShadowDepthVsGlobals> {
        if !shader_type.starts_with("TShadowDepthVS") || self.derived.len() < 18 { return None; }
        let p = |k: usize| param(&self.derived[6 * k..6 * k + 6]);
        Some(ShadowDepthVsGlobals { projection_matrix: p(0), shadow_params: p(1), clamp_to_near_plane: p(2) })
    }
    /// Every bound loose parameter in constant buffer `slot` (the per-frame lists and, for a base-pass VS, the velocity globals): the highest byte written.
    pub fn bytes_used_in(&self, slot: u16, shader_type: &str) -> usize {
        let mut all: Vec<ShaderParam> = self.per_frame_scalars.iter().chain(&self.per_frame_vectors).chain(&self.per_frame_prev_scalars).chain(&self.per_frame_prev_vectors).copied().collect();
        if let Some(v) = self.base_pass_vs(shader_type) { all.extend([v.previous_local_to_world, v.skip_output_velocity, v.instanced_eye_index, v.is_instanced_stereo]); }
        if let Some(v) = self.shadow_depth_vs(shader_type) { all.extend([v.projection_matrix, v.shadow_params, v.clamp_to_near_plane]); }
        if let Some(f) = self.height_fog(shader_type) { all.extend(f.values); } // FOG INPUTS
        // GRASS: the instanced vertex factory's loose values (a depth-only / shadow VS of the foliage has no other $Globals member)
        if let Some(v) = self.instanced_vf() { all.extend([v.fade_out_params, v.view_z_compare_zero, v.view_z_compare_one, v.view_z_constant, v.world_view_origin_zero, v.world_view_origin_one, v.cpu_instance_origin, v.cpu_instance_transform, v.cpu_lightmap_bias]); }
        // LANDSCAPE MATERIAL: the landscape vertex factory's loose values
        if let Some(v) = self.landscape_vs() { all.extend([v.lod_values, v.neighbor_section_lod, v.lod_bias, v.section_lods]); }
        if let Some(v) = self.landscape_ps() { all.push(v.local_to_world_no_scaling); }
        all.iter().filter(|p| p.is_bound() && p.buffer_index == slot).map(|p| p.base_index as usize + p.num_bytes as usize).max().unwrap_or(0)
    }
}

/// `FShaderResourceParameter` (ShaderParameters.cpp:73): `u16 BaseIndex` (the `t` / `s` register), `u16 NumResources` (0 = unbound).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ShaderResource { pub base_index: u16, pub num_resources: u16 }
impl ShaderResource { pub fn is_bound(&self) -> bool { self.num_resources > 0 } }

/// TRANSLUCENCY VOLUME (MIRRORFIX 2026-10-05): `FTranslucentLightingVolumeParameters` of a base-pass pixel shader, in the order of its serializer
/// (BasePassRendering.h:285): AmbientInner, AmbientOuter, DirectionalInner, DirectionalOuter, each texture + sampler.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TranslucencyVolumeParams { pub textures: [ShaderResource; 4], pub samplers: [ShaderResource; 4] }
/// Byte offset of the volume parameters in the derived bytes of a `TBasePassPS*`: `TBasePassPixelShaderPolicyParamType::Serialize` (BasePassRendering.h:777) writes,
/// after `FMeshMaterialShader`, `PixelParametersType`, `FBasePassReflectionParameters` (:641: `FPlanarReflectionParameters` 10 `FShaderParameter` + 2 resources =
/// 68 bytes, ReflectionCubemap + sampler 8, 4 `FShaderParameter` 24, `FSkyLightReflectionParameters` 4+4+6+4+4+6 = 28; total 128), then
/// `FTranslucentLightingParameters` starting with the volume parameters (:688). MEASURED in the 1.8 data: 128, i.e. the no-lightmap `PixelParametersType` adds no
/// bytes here (the 4.16 header's 6-byte `FShaderUniformBufferParameter` is not in the 1.8 layout). Verified on `MGreen` / `M_Blueprint_StairsBaseShader`
/// (t1 / s1 = AmbientInner, t2 / s2 = AmbientOuter) and `MPreviewPulse` (t0 / s0, t1 / s1): exactly the program's `Texture3D` declarations; the directional pair is
/// unbound (TLM_VolumetricNonDirectional). Callers still check the slots against the declarations.
pub const BASE_PASS_PS_VOLUME_OFFSET: usize = 128;
impl MaterialShaderParams {
    /// The translucency lighting volume parameters of a `TBasePassPS*` program (`None` for other shader types / short data).
    pub fn translucency_volume(&self, shader_type: &str) -> Option<TranslucencyVolumeParams> {
        if !shader_type.starts_with("TBasePassPS") { return None; }
        let b = self.derived.get(BASE_PASS_PS_VOLUME_OFFSET..BASE_PASS_PS_VOLUME_OFFSET + 32)?;
        let r = |k: usize| ShaderResource { base_index: u16::from_le_bytes([b[4 * k], b[4 * k + 1]]), num_resources: u16::from_le_bytes([b[4 * k + 2], b[4 * k + 3]]) };
        Some(TranslucencyVolumeParams { textures: [r(0), r(2), r(4), r(6)], samplers: [r(1), r(3), r(5), r(7)] })
    }
}

/// FOG INPUTS (2026-10-05): `FHeightFogShaderParameters` (= one `FExponentialHeightFogShaderParameters`, UE4.16 FogRendering.h:56-69) in the order of its
/// serializer (FogRendering.cpp:60-74, + `FVolumetricFogParameters`, VolumetricFog.h:62): ExponentialFogParameters, ExponentialFogColorParameter,
/// ExponentialFogParameters3, SinCosInscatteringColorCubemapRotation, FogInscatteringColorCubemap + Sampler, FogInscatteringTextureParameters,
/// InscatteringLightDirection, DirectionalInscatteringColor, DirectionalInscatteringStartDistance, ApplyVolumetricFog, IntegratedLightScattering + Sampler.
/// The 1.8 exe binds the same members in the same order (`FExponentialHeightFogShaderParameters__Bind` @141825190: +0x18 cubemap ... +0x42 the volume
/// sampler = 70 bytes). `values` index = [`HEIGHT_FOG_FIELDS`] order (the `fn_render::fog::shader_constants` array).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct HeightFogParams { pub values: [ShaderParam; HEIGHT_FOG_FIELDS], pub cubemap: ShaderResource, pub cubemap_sampler: ShaderResource, pub volume: ShaderResource, pub volume_sampler: ShaderResource }
/// The nine loose values of [`HeightFogParams`] (ExponentialFogParameters .. ApplyVolumetricFog).
pub const HEIGHT_FOG_FIELDS: usize = 9;
/// Serialized size of `FHeightFogShaderParameters`: 9 `FShaderParameter` (6 bytes) + 4 `FShaderResourceParameter` (4 bytes).
pub const HEIGHT_FOG_BYTES: usize = 70;
/// Value sizes in bytes (float4, float4, float4, float2, float3, float4, float4, float, float): a bound parameter has exactly this `NumBytes`.
const HEIGHT_FOG_VALUE_BYTES: [u16; HEIGHT_FOG_FIELDS] = [16, 16, 16, 8, 12, 16, 16, 4, 4];
/// Where `FHeightFogShaderParameters` starts in the derived bytes: `TBasePassVertexShaderPolicyParamType::Serialize` (BasePassRendering.h:348) writes it right after
/// `FMeshMaterialShader` + `VertexParametersType` (`FUniformLightMapPolicyShaderParametersType::Serialize`, LightMapRendering.h:650: the
/// `PrecomputedLightingBuffer` `FShaderUniformBufferParameter`, 6 bytes; MEASURED on the hologram VS `M_Pickup_Blueprint_01`: 6 zero bytes, then the fog block
/// at cb0[8..15], t1 cube / s0, t2 volume / s1, see the tests); `TBasePassPixelShaderPolicyParamType::Serialize` (:777) after the reflection parameters
/// (128 bytes, see [`BASE_PASS_PS_VOLUME_OFFSET`]) and `FTranslucentLightingParameters` (:685: 4 volumes + samplers 32, HZB texture + sampler 8,
/// HZBUvFactorAndInvFactor 6, PrevSceneColor + sampler 8 = 54).
pub const BASE_PASS_VS_FOG_OFFSET: usize = 6;
pub const BASE_PASS_PS_FOG_OFFSET: usize = BASE_PASS_PS_VOLUME_OFFSET + 54;
impl MaterialShaderParams {
    /// The height-fog parameters of a `TBasePassVS*` / `TBasePassPS*` program (`None` for other shader types, short data or a layout that does not validate:
    /// every bound value must have its member's size, every bound resource one register).
    pub fn height_fog(&self, shader_type: &str) -> Option<HeightFogParams> {
        let at = if shader_type.starts_with("TBasePassVS") { BASE_PASS_VS_FOG_OFFSET } else if shader_type.starts_with("TBasePassPS") { BASE_PASS_PS_FOG_OFFSET } else { return None };
        let b = self.derived.get(at..at + HEIGHT_FOG_BYTES)?;
        let res = |o: usize| ShaderResource { base_index: u16::from_le_bytes([b[o], b[o + 1]]), num_resources: u16::from_le_bytes([b[o + 2], b[o + 3]]) };
        // byte layout: 4 params (0..24), cubemap + sampler (24..32), 3 params (32..50)... see HEIGHT_FOG_BYTES
        let at_of = [0usize, 6, 12, 18, 32, 38, 44, 50, 56];
        let values: [ShaderParam; HEIGHT_FOG_FIELDS] = std::array::from_fn(|k| param(&b[at_of[k]..at_of[k] + 6]));
        let p = HeightFogParams { values, cubemap: res(24), cubemap_sampler: res(28), volume: res(62), volume_sampler: res(66) };
        let ok = p.values.iter().zip(HEIGHT_FOG_VALUE_BYTES).all(|(v, n)| !v.is_bound() || v.num_bytes == n)
            && [p.cubemap, p.cubemap_sampler, p.volume, p.volume_sampler].iter().all(|r| r.num_resources <= 1);
        ok.then_some(p)
    }
}

fn param(b: &[u8]) -> ShaderParam { ShaderParam { base_index: u16::from_le_bytes([b[0], b[1]]), num_bytes: u16::from_le_bytes([b[2], b[3]]), buffer_index: u16::from_le_bytes([b[4], b[5]]) } }

struct Rd<'a> { d: &'a [u8], o: usize }
impl Rd<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> { let s = self.d.get(self.o..self.o + n)?; self.o += n; Some(s) }
    fn i32(&mut self) -> Option<i32> { Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?)) }
    fn u32(&mut self) -> Option<u32> { Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?)) }
    fn params(&mut self) -> Option<Vec<ShaderParam>> { let n = self.i32()?; if !(0..=256).contains(&n) { return None; } (0..n).map(|_| self.take(6).map(param)).collect() }
    fn fname(&mut self, names: &[String]) -> Option<String> {
        let (i, n) = (self.i32()?, self.i32()?);
        if i < 0 || n < 0 || n > 1024 { return None; }
        let b = names.get(i as usize)?;
        Some(if n > 0 { format!("{b}_{}", n - 1) } else { b.clone() })
    }
    /// `FString`: i32 length (with the terminator; negative = UTF-16), then the characters.
    fn fstring(&mut self) -> Option<()> { let n = self.i32()?; if n >= 0 { if n > 4096 { return None; } self.take(n as usize)?; } else { if n < -4096 { return None; } self.take(2 * (-n) as usize)?; } Some(()) }
}

/// Parses the `Serialize` part of one shader-map entry. `data` = the `.uexp`, `entry_offset` / `entry_type` = the entry header's offset and type FName index + number,
/// `stream_start` = where the inline shader resource starts (the search bound), `header` = the package's total header size (SkipOffset is absolute).
pub fn parse(data: &[u8], names: &[String], entry_offset: usize, entry_type: (u32, u32), stream_start: usize, header: usize) -> Option<MaterialShaderParams> {
    let lo = entry_offset + 12;
    let hi = stream_start.min(data.len());
    if lo >= hi { return None; }
    let material = names.iter().position(|n| n == "Material")? as i32;
    // the end of `Serialize`: the entry's own type FName follows OutputHash, MaterialShaderMapHash, ShaderPipeline, VFType, VFSourceHash (20 + 20 + 8 + 8 + 20 = 76 bytes)
    let mut serialize_end = None;
    for t in lo + 76..hi.saturating_sub(8) {
        if u32::from_le_bytes(data[t..t + 4].try_into().unwrap()) != entry_type.0 || u32::from_le_bytes(data[t + 4..t + 8].try_into().unwrap()) != entry_type.1 { continue; }
        // ShaderPipeline (t - 36) and VFType (t - 28) must be FNames too (number 0)
        let ok = |at: usize| { let i = i32::from_le_bytes(data[at..at + 4].try_into().unwrap()); i >= 0 && (i as usize) < names.len() && i32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) == 0 };
        if ok(t - 36) && ok(t - 28) { serialize_end = Some(t - 76); break; }
    }
    let serialize_end = serialize_end?;
    // the debug layout FName "Material" preceded by the six debug counts
    for q in lo + 24..serialize_end.saturating_sub(8) {
        if i32::from_le_bytes(data[q..q + 4].try_into().unwrap()) != material || i32::from_le_bytes(data[q + 4..q + 8].try_into().unwrap()) != 0 { continue; }
        let mut debug_counts = [0i32; 6];
        for k in 0..6 { debug_counts[k] = i32::from_le_bytes(data[q - 24 + 4 * k..q - 20 + 4 * k].try_into().unwrap()); }
        if debug_counts.iter().any(|c| !(0..=1024).contains(c)) { continue; }
        let mut r = Rd { d: &data[..serialize_end], o: q + 8 };
        let attempt = (|| -> Option<MaterialShaderParams> {
            let material_cb_size = r.u32()?; let _resource_offset = r.u32()?;
            let n = r.i32()?; if !(0..=1024).contains(&n) { return None; } r.take(n as usize)?;
            r.fstring()?;
            r.take(4)?; // EyeAdaptation
            let (per_frame_scalars, per_frame_vectors, per_frame_prev_scalars, per_frame_prev_vectors) = (r.params()?, r.params()?, r.params()?, r.params()?);
            if per_frame_scalars.len() != debug_counts[2] as usize || per_frame_vectors.len() != debug_counts[3] as usize { return None; }
            let mut out = MaterialShaderParams { debug_counts, material_cb_size, per_frame_scalars, per_frame_vectors, per_frame_prev_scalars, per_frame_prev_vectors, ..Default::default() };
            // FMeshMaterialShader: a vertex factory reference follows
            let save = r.o;
            match r.fname(names) {
                Some(vf) if vf.ends_with("VertexFactory") || vf.starts_with("TGPUSkinVertexFactory") => {
                    r.take(1 + 20)?;
                    let skip = r.i32()? as usize;
                    let end = skip.checked_sub(header)?;
                    if end < r.o || end > serialize_end { return None; }
                    out.vf_params = data[r.o..end].to_vec();
                    r.o = end;
                    out.non_instanced_dither_lod_factor = Some(param(r.take(6)?));
                    out.vertex_factory = Some(vf);
                }
                _ => r.o = save,
            }
            out.derived = data[r.o..serialize_end].to_vec();
            Some(out)
        })();
        if attempt.is_some() { return attempt; }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::material::Material;
    /// The storm programs' serialized parameters (the offsets the verifier's report expected: the cloud VS reads cb0[2..6], the wall PS its panners at 9 / 13 / 17 / 19).
    #[test] fn storm_programs_serialize_their_globals() {
        let set = fn_pak::PakSet::open_install(crate::default_install()).unwrap();
        let cloud = Material::load(&set, "/Game/Athena/SafeZone/M_SafeZoneStorm.M_SafeZoneStorm").unwrap();
        let vs = cloud.cooked.find("FLocalVertexFactory", "TBasePassVSFNoLightMapPolicy").unwrap();
        let p = vs.params.as_ref().expect("cloud VS parameters");
        assert_eq!(p.vertex_factory.as_deref(), Some("FLocalVertexFactory"));
        let v = p.base_pass_vs(&vs.shader_type).unwrap();
        assert_eq!((v.previous_local_to_world.base_index, v.previous_local_to_world.num_bytes, v.previous_local_to_world.buffer_index), (32, 64, 0), "PreviousLocalToWorld = cb0[2..5]");
        assert_eq!((v.skip_output_velocity.base_index, v.skip_output_velocity.num_bytes), (96, 4), "SkipOutputVelocity = cb0[6].x");
        let wall = Material::load(&set, "/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere").unwrap();
        let ps = wall.cooked.find("FLocalVertexFactory", "TBasePassPSFNoLightMapPolicy").unwrap();
        let p = ps.params.as_ref().expect("wall PS parameters");
        assert_eq!(p.per_frame_scalars.len(), wall.uniform.per_frame_scalars.len());
        let bound: Vec<usize> = p.per_frame_scalars.iter().filter(|s| s.is_bound()).map(|s| s.float_index()).collect();
        assert_eq!(bound, vec![9, 13, 17, 19]);
    }
    /// FX SHADOW: the player-piece material's own shader map has the whole-scene directional shadow-depth pair (masked + WPO: `TShadowDepthVS<OutputDepth,
    /// false, false>` and `TShadowDepthPS<NonPerspectiveCorrect, false>`, ShadowDepthRendering.cpp:802 / :830) and the VS serializes ProjectionMatrix /
    /// ShadowParams / bClampToNearPlane in its $Globals; the default material carries the position-only VS of the opaque override (:653, :798).
    #[test] fn piece_material_serializes_its_shadow_depth_globals() {
        let set = fn_pak::PakSet::open_install(crate::default_install()).unwrap();
        let m = Material::load(&set, "/Game/Packages/PBW/Wood/Materials/Wood_lvl1_Wall.Wood_lvl1_Wall").unwrap();
        let vs = m.cooked.find("FLocalVertexFactory", "TShadowDepthVSVertexShadowDepth_OutputDepthfalse").expect("shadow depth VS");
        assert!(m.cooked.find("FLocalVertexFactory", "TShadowDepthPSPixelShadowDepth_NonPerspectiveCorrectfalse").is_some(), "masked: shadow depth PS");
        let g = vs.params.as_ref().unwrap().shadow_depth_vs(&vs.shader_type).unwrap();
        assert_eq!((g.projection_matrix.base_index, g.projection_matrix.num_bytes, g.projection_matrix.buffer_index), (112, 64, 0));
        assert_eq!((g.shadow_params.base_index, g.shadow_params.num_bytes), (96, 8));
        assert_eq!((g.clamp_to_near_plane.base_index, g.clamp_to_near_plane.num_bytes), (176, 4));
        let d = Material::load(&set, "/Engine/EngineMaterials/WorldGridMaterial.WorldGridMaterial").unwrap();
        assert!(d.cooked.find("FLocalVertexFactory", "TShadowDepthVSVertexShadowDepth_OutputDepthPositionOnly").is_some());
    }
    /// FX SHADOW: what the cooked components say about shadow casting. The player pieces' `StaticMeshComponent0` (template of the native `ABuildingSMActor`,
    /// whose ctor @14045b960 sets CastShadow / bCastDynamicShadow / bCastStaticShadow) serializes no override; every storm cloud / wall component of
    /// BP_SafeZoneStorm serializes `bCastDynamicShadow = false` and `bAffectDistanceFieldLighting = false`.
    #[test] fn cooked_components_shadow_flags() {
        let set = fn_pak::PakSet::open_install(crate::default_install()).unwrap();
        let flags = ["CastShadow", "bCastDynamicShadow", "bCastStaticShadow", "bAffectDistanceFieldLighting", "Mobility"];
        for pkg in ["/Game/Building/ActorBlueprints/Player/Wood/L1/PBWA_W1_Solid", "/Game/Building/ActorBlueprints/Player/Stone/L1/PBWA_S1_Floor", "/Game/Building/ActorBlueprints/Player/Metal/L1/PBWA_M1_StairW"] {
            let p = crate::data::PackageJson::load(&set, pkg).unwrap();
            let c = p.exports.iter().find(|e| e.class == "StaticMeshComponent").unwrap();
            for f in flags { assert!(c.props.get(f).is_none(), "{pkg}: {f} overridden"); }
            let cdo = p.exports.iter().find(|e| e.name.starts_with("Default__")).unwrap();
            for f in ["bIsDynamic", "bIsMovable"] { assert!(cdo.props.get(f).is_none(), "{pkg}: {f} overridden"); }
        }
        let storm = crate::data::PackageJson::load(&set, "/Game/Athena/SafeZone/BP_SafeZoneStorm").unwrap();
        let comps: Vec<_> = storm.exports.iter().filter(|e| e.class == "StaticMeshComponent").collect();
        assert!(!comps.is_empty());
        for c in comps { assert_eq!(c.props.get("bCastDynamicShadow").and_then(|v| v.as_bool()), Some(false), "{}", c.name); }
    }
    /// FOG INPUTS: the hologram's base-pass VS (`M_Pickup_Blueprint_01`, skin and local VF) serializes its height-fog parameters at the slots the program reads
    /// (cb0[8..15], t1 cube / s0, t2 volume / s1); the other translucent FX programs are listed with what they bind (MEASURED: the dome / rays / MGreen VS bind
    /// the block at cb0[6..13]; no base-pass PS and no storm VS binds it).
    #[test] fn base_pass_programs_serialize_their_height_fog_parameters() {
        let set = fn_pak::PakSet::open_install(crate::default_install()).unwrap();
        let holo = Material::load(&set, "/Game/Effects/Fort_Effects/Gameplay/Pickups/Materials/M_Pickup_Blueprint_01.M_Pickup_Blueprint_01").unwrap();
        // (vf, cb0 float4 of ExponentialFogParameters, t cube, t volume): the skin VS has MeshOrigin / MeshExtension at cb0[6..7] ahead of the block
        for (vf, first, cube, volume) in [("TGPUSkinVertexFactoryfalse", 8u16, 1u16, 2u16), ("FLocalVertexFactory", 6, 0, 1)] {
            let vs = holo.cooked.find(vf, "TBasePassVSFNoLightMapPolicy").unwrap();
            let f = vs.params.as_ref().unwrap().height_fog(&vs.shader_type).unwrap_or_else(|| panic!("{vf}: no fog layout"));
            let at: Vec<(u16, u16, u16)> = f.values.iter().map(|v| (v.base_index, v.num_bytes, v.buffer_index)).collect();
            eprintln!("{vf}: {at:?} cube {:?}/{:?} volume {:?}/{:?}", f.cubemap, f.cubemap_sampler, f.volume, f.volume_sampler);
            // ExponentialFogParameters, Color, Parameters3, SinCos at +0..+3, LightDirection +4, DirectionalColor +5, StartDistance + TextureParameters at +6
            // (.x / .yzw), ApplyVolumetricFog +7.x: the spec's cb0[8..15] of the skin VS
            let f4 = |k: usize| (f.values[k].base_index / 16 - first, f.values[k].base_index % 16);
            assert_eq!((0..9).map(f4).collect::<Vec<_>>(), vec![(0, 0), (1, 0), (2, 0), (3, 0), (6, 4), (4, 0), (5, 0), (6, 0), (7, 0)], "{vf}");
            assert!(f.values.iter().all(|v| v.is_bound() && v.buffer_index == 0));
            assert_eq!((f.cubemap.base_index, f.volume.base_index, f.cubemap_sampler.base_index, f.volume_sampler.base_index), (cube, volume, 0, 1));
            let ps = holo.cooked.find(vf, "TBasePassPSFNoLightMapPolicy").unwrap();
            eprintln!("{vf} PS fog: {:?}", ps.params.as_ref().unwrap().height_fog(&ps.shader_type));
        }
        for path in ["/Game/Effects/Fort_Effects/Gameplay/Pickups/Materials/M_Pickup_Blueprint_BG_01.M_Pickup_Blueprint_BG_01", "/Game/Effects/Fort_Effects/Gameplay/Pickups/Materials/M_Pickup_RaysBG_01.M_Pickup_RaysBG_01", "/Game/Athena/SafeZone/M_SafeZoneSphere.M_SafeZoneSphere", "/Game/Athena/SafeZone/M_SafeZoneStorm.M_SafeZoneStorm", "/Game/Packages/Fortnite_UniqueMaterials_1/Effects/MGreen.MGreen"] {
            let Ok(m) = Material::load(&set, path) else { eprintln!("{path}: not loadable"); continue };
            for p in m.cooked.programs.iter().filter(|p| p.shader_type.starts_with("TBasePass") && p.shader.major >= 5) {
                let f = p.params.as_ref().and_then(|x| x.height_fog(&p.shader_type));
                let bound = f.map(|f| f.values.iter().filter(|v| v.is_bound()).count());
                eprintln!("{} {} {}: fog bound values {bound:?} {:?}", path.rsplit('.').next().unwrap(), p.vertex_factory, p.shader_type, f.map(|f| (f.values[0].base_index, f.volume, f.cubemap)));            }
        }
    }
    /// TRANSLUCENCY VOLUME: the edit tile materials (TLM_VolumetricNonDirectional) bind exactly the ambient inner / outer volumes, at the registers of their
    /// Texture3D declarations; the unlit blueprint material binds none.
    #[test] fn translucent_lit_programs_serialize_the_volume_parameters() {
        let set = fn_pak::PakSet::open_install(crate::default_install()).unwrap();
        for (path, inner, outer) in [("/Game/Packages/Fortnite_UniqueMaterials_1/Effects/MGreen.MGreen", 1, 2), ("/Game/Packages/Fortnite_UniqueMaterials_1/Effects/MPreviewPulse.MPreviewPulse", 0, 1)] {
            let m = Material::load(&set, path).unwrap();
            assert_eq!(m.flags.translucency_lighting_mode, "TLM_VolumetricNonDirectional");
            let ps = m.cooked.find("FLocalVertexFactory", "TBasePassPSFNoLightMapPolicySkylight").unwrap();
            let v = ps.params.as_ref().unwrap().translucency_volume(&ps.shader_type).unwrap();
            assert_eq!((v.textures[0].base_index, v.textures[0].num_resources, v.textures[1].base_index, v.textures[1].num_resources), (inner, 1, outer, 1), "{path}: {v:?}");
            assert!(!v.textures[2].is_bound() && !v.textures[3].is_bound(), "non-directional: no directional volume");
            for t in &v.textures[..2] { assert!(ps.shader.decls.resources.iter().any(|d| d.slot == t.base_index as u32 && d.dim == 5)); }
        }
        let bp = Material::load(&set, "/Game/Packages/Fortnite_UniqueMaterials_1/Walls/PlayerBuiltWall_BluePrintShader.PlayerBuiltWall_BluePrintShader").unwrap();
        let ps = bp.cooked.find("FLocalVertexFactory", "TBasePassPSFNoLightMapPolicy").unwrap();
        assert!(ps.params.as_ref().unwrap().translucency_volume(&ps.shader_type).unwrap().textures.iter().all(|t| !t.is_bound()));
    }
}
