//! fn-fx: the world-space harvest FX of the 1.8 port, built from the original packages (see the crate description). Modules:
//! * [`cooked`]   shader programs of a material package with their vertex factory / shader type ownership
pub mod cooked;
/// serialized loose shader parameters of the cooked programs (per-frame expression / velocity global offsets, read instead of inferred)
pub mod shader_params;
pub mod data;
pub mod material;
/// material parameter collections: the `MaterialCollectionN` uniform buffers (layout, instance values)
pub mod mpc;
pub mod particles;
pub mod uniform;
pub mod program;
pub mod view;
#[cfg(feature = "render")]
pub mod gpu;
#[cfg(feature = "render")]
pub mod webtex;
#[cfg(feature = "render")]
pub mod webcook;
#[cfg(feature = "render")]
pub mod ring;
#[cfg(feature = "render")]
pub mod numbers;
#[cfg(feature = "render")]
pub mod bridge;

pub use fn_pak::PakSet;

/// The install the research tools and the tests read when `FN_INSTALL` is not set (read only; the client is never run).
pub fn default_install() -> std::path::PathBuf { fn_dxbc_cli::spec::default_install() }








