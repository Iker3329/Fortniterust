//! CPU-side mesh / material descriptions handed to the renderer (the glTF loading lives in the viewer / asset crates).
use bytemuck::{Pod, Zeroable};
use image::RgbaImage;

/// Skinned vertex: 56 bytes, matches the vertex layout of `vs_skinned` in scene.wgsl / shadow.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct SkinnedVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialKind {
    /// Untextured white surface lit by sun + sky light.
    Generic = 0,
    /// Terrain grass (T_Grasslands_D x colour variation).
    Ground = 1,
    /// Character body / head: diffuse + normal + specular-mask + mask textures.
    Character = 2,
    /// Prop (pickaxe): character shading with constant specular / mask values.
    Prop = 3,
}

/// Material textures (RGBA8). Missing slots fall back to neutral 1x1 values (flat normal, no spec, no mask).
#[derive(Clone, Debug, Default)]
pub struct MaterialImages {
    pub diffuse: Option<RgbaImage>,
    pub normal: Option<RgbaImage>,
    pub spec: Option<RgbaImage>,
    pub mask: Option<RgbaImage>,
    /// CHAR MATERIAL HOOK: the original compiled material of a BR character primitive (see char_material): constants + fixed textures; None = the generic character shading.
    pub ue: Option<std::sync::Arc<crate::char_material::UeMaterial>>,
}

#[derive(Clone, Debug)]
pub struct SkinnedPrimitive {
    pub vertices: Vec<SkinnedVertex>,
    pub indices: Vec<u32>,
    pub material: MaterialImages,
    pub kind: MaterialKind,
}
impl SkinnedPrimitive {
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in &self.vertices { for i in 0..3 { lo[i] = lo[i].min(v.pos[i]); hi[i] = hi[i].max(v.pos[i]); } }
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vertex_is_56_bytes_without_padding() {
        assert_eq!(std::mem::size_of::<SkinnedVertex>(), 56);
        assert_eq!(std::mem::align_of::<SkinnedVertex>(), 4);
    }
    #[test]
    fn bounds_cover_all_vertices() {
        let p = SkinnedPrimitive { vertices: vec![SkinnedVertex { pos: [1., -2., 3.], ..Default::default() }, SkinnedVertex { pos: [-1., 5., 0.], ..Default::default() }], indices: vec![0, 1, 0], material: MaterialImages::default(), kind: MaterialKind::Character };
        assert_eq!(p.bounds(), ([-1., -2., 0.], [1., 5., 3.]));
    }
}
