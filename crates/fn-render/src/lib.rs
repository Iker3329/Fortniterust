//! fn-render: the pure-Rust wgpu renderer of the Fortnite 1.8 runtime.
//!
//! * [`gpu`]      device / adapter setup (Vulkan + DX12), offscreen target with read-back
//! * [`camera`]   camera parameters, projection maths, the GPU camera uniform
//! * [`lighting`] recovered sun / sky-light constants, terrain colour variation, cascaded shadow map fitting
//! * [`sky`]      dome + sun + clouds (ported from the recovered UE 4.16 pixel shaders)
//! * [`tonemap`]  UE 4.16 tonemapper / exposure constants and CPU mirrors of the post chain
//! * [`graph`]    the small frame graph
//! * [`renderer`] pipelines, render targets and the frame description
//! * [`texture`], [`mesh`] upload helpers and CPU mesh/material descriptions
//!
//! All shader code lives in `crates/fn-render/shaders/*.wgsl` (see the crate README for the rust-gpu migration plan).
pub mod camera;
pub mod char_material; // CHAR MATERIAL HOOK
pub mod fog; // FOG / ATMOSPHERE HOOK: recovered Athena fog data + exact CPU ports of the fog / volumetric fog shaders
pub mod fx; // RENDER FX HOOK
pub mod gpu;
pub mod graph;
pub mod gun_material; // GUN MATERIAL HOOK
pub mod lighting;
pub mod mesh;
pub mod particles; // PARTICLE HOOK
pub mod profiler;
pub mod props;
pub mod quality;
pub mod renderer;
pub mod sky;
pub mod static_loader;
pub mod static_mesh;
pub mod statics;
pub mod synthetic;
#[allow(dead_code)]
pub mod sky_params;
pub mod texture;
pub mod tonemap;
pub mod volfog; // VOLUMETRIC FOG HOOK: GPU volumetric fog passes (material setup, light scattering, final integration, composite)

pub use camera::{CameraParams, CameraUniform};
pub use gpu::{Gpu, GpuErrorSink, OffscreenTarget};
pub use mesh::{MaterialImages, MaterialKind, SkinnedPrimitive, SkinnedVertex};
pub use quality::{Backdrop, FxaaParams};
pub use renderer::{ExposureInfo, FrameParams, FrameStats, HdrImage, ModelId, PropVertex, RenderOptions, Renderer, RendererAssets, UiOverlay};
pub use profiler::{GpuPassTime, PassTimings};
pub use props::{PropInstance, PropTemplates};
pub use sky::SkyAssets;
pub use static_mesh::{AlphaMode, StaticLod, StaticMaterialData, StaticMaterialHandle, StaticMeshData, StaticMeshHandle, StaticMeshInstance, StaticSection, StaticVertex};
pub use statics::StaticStats;
pub use wgpu;