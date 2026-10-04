//! `SKYM` compact static mesh files, as consumed by the old runtime's sky renderer (`sky.rs::parse_mesh`).
//!
//! The old pipeline exported the sky meshes with UEViewer to `.pskx` and converted them with
//! `tools/sky-mesh-export.cjs`. This module produces the same bytes directly from a decoded [`StaticMesh`]:
//!
//! ```text
//! "SKYM"  u32 vertexCount  u32 indexCount  u32 uvChannels (=3)
//! per vertex, 13 x f32: x y z   u0 v0   u1 v1   u2 v2   r g b a
//! u32 indices[indexCount]
//! ```
//!
//! Conventions inherited from the PSK exporter: vertices are the mesh's vertex buffer entries ("wedges", no
//! welding); positions are UE centimetres but with **Y mirrored** (`PSK MIRROR_MESH`); triangles are listed per
//! section with their first two indices swapped (to flip the winding for the mirrored mesh); UV channels the mesh
//! lacks are 0; vertex colour is `byte / 255` in R, G, B, A order (1,1,1,1 when the mesh has none).

use crate::gltf::{legacy_half, Options};
use crate::static_mesh::StaticMesh;
use fn_ueio::{Error, Result};

/// Build the SKYM bytes for one LOD of a static mesh.
///
/// `opts.legacy_uev_half_uv` reproduces UEViewer's half-float zero bug in the UVs (see [`Options`]).
pub fn static_mesh_to_skym(mesh: &StaticMesh, lod: usize, opts: &Options) -> Result<Vec<u8>> {
    let l = mesh.lods.get(lod).ok_or_else(|| Error::Invalid(format!("mesh has no LOD {lod}")))?;
    if l.positions.is_empty() || l.indices.is_empty() {
        return Err(Error::Invalid(format!("LOD {lod} has no geometry")));
    }
    let n = l.positions.len();
    let mut indices: Vec<u32> = Vec::new();
    for s in &l.sections {
        for t in 0..s.num_triangles as usize {
            let at = s.first_index as usize + t * 3;
            let g = |k: usize| l.indices.get(at + k).copied().unwrap_or(0);
            indices.extend_from_slice(&[g(1), g(0), g(2)]);
        }
    }
    let legacy = opts.legacy_uev_half_uv && !l.uv_full_precision;
    let mut out = Vec::with_capacity(16 + n * 52 + indices.len() * 4);
    out.extend_from_slice(b"SKYM");
    out.extend_from_slice(&(n as u32).to_le_bytes());
    out.extend_from_slice(&(indices.len() as u32).to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes());
    let mut put = |v: f32| out.extend_from_slice(&v.to_le_bytes());
    for i in 0..n {
        let p = l.positions[i];
        put(p[0]);
        put(-p[1]);
        put(p[2]);
        for ch in 0..3 {
            match l.uvs.get(ch) {
                Some(uv) => {
                    let v = uv[i];
                    if legacy {
                        put(legacy_half(v[0]));
                        put(legacy_half(v[1]));
                    } else {
                        put(v[0]);
                        put(v[1]);
                    }
                }
                None => {
                    put(0.0);
                    put(0.0);
                }
            }
        }
        match l.colors.get(i) {
            // FColor memory order (B, G, R, A) -> r, g, b, a
            Some(c) => {
                put((c[2] as f64 / 255.0) as f32);
                put((c[1] as f64 / 255.0) as f32);
                put((c[0] as f64 / 255.0) as f32);
                put((c[3] as f64 / 255.0) as f32);
            }
            None => {
                for _ in 0..4 {
                    put(1.0);
                }
            }
        }
    }
    for i in indices {
        out.extend_from_slice(&i.to_le_bytes());
    }
    Ok(out)
}
