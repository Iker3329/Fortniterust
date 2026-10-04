//! Textured static props in the viewer: `--props <dir>` loads the exported real 1.8 trees / rocks (`fn_render::static_loader`), without it
//! the synthetic foliage tree / rock of `fn_render::synthetic` stand in; `--prop-grid N` scatters N of them (jittered grid, several kinds).
use fn_render::{Renderer, StaticMeshHandle, StaticMeshInstance};
use glam::Vec3;
use std::path::Path;

/// One placeable kind (mesh handle + size) and how often the scatter picks it.
pub struct PropKind { pub name: String, pub mesh: StaticMeshHandle, pub weight: f32, pub rock: bool, pub scale: (f32, f32) }

/// Loads the kinds: every prop of `dir` when given, the synthetic set otherwise.
pub fn load_kinds(renderer: &mut Renderer, dir: Option<&Path>, only: &[String]) -> Result<Vec<PropKind>, String> {
    if let Some(d) = dir {
        let cat = fn_propdata::PropCatalog::load(d).map_err(|e| format!("{}: {e}", d.display()))?;
        let set = fn_render::static_loader::load_catalog(renderer, &cat, |k| if only.is_empty() { fn_render::static_loader::DEFAULT_CATEGORIES.contains(&k.category.as_str()) } else { let id = k.id.to_ascii_lowercase(); only.iter().any(|o| id.contains(o.as_str())) })?;
        if set.props.is_empty() { return Err(format!("{}: no props found", d.display())); }
        eprintln!("[props] {} props, {} materials from {}", set.props.len(), set.material_count, d.display());
        return Ok(set.props.into_iter().map(|p| { let rock = p.category == "rock" || fn_render::static_loader::looks_like_rock(&p.id); PropKind { scale: p.random_scale.unwrap_or(if rock { (0.8, 1.3) } else { (0.9, 1.2) }), name: p.id, mesh: p.mesh, weight: p.weight, rock } }).collect());
    }
    use fn_render::synthetic::*;
    let (bark, f1, f2, rock) = (renderer.create_static_material(&bark_material("synthetic bark")), renderer.create_static_material(&foliage_material("synthetic leaves A", 3)), renderer.create_static_material(&foliage_material("synthetic leaves B", 9)), renderer.create_static_material(&rock_material("synthetic rock")));
    Ok(vec![
        PropKind { name: "synthetic tree A".into(), mesh: renderer.create_static_mesh(&tree_mesh("tree A", 11), &[bark, f1]), weight: 3., rock: false, scale: (0.8, 1.3) },
        PropKind { name: "synthetic tree B".into(), mesh: renderer.create_static_mesh(&tree_mesh("tree B", 29), &[bark, f2]), weight: 2., rock: false, scale: (0.8, 1.3) },
        PropKind { name: "synthetic rock".into(), mesh: renderer.create_static_mesh(&rock_mesh("rock", 5), &[rock]), weight: 2., rock: true, scale: (0.6, 1.8) },
    ])
}

/// Deterministic jittered grid of `n` props in front of the spawn (x centred, z from -6 downwards), kinds picked by weight, random yaw and
/// 0.8..1.3 scale (trees) / 0.6..1.8 (rocks); every 7th is shaking so the shake path is exercised.
pub fn grid(kinds: &[PropKind], n: usize, seed: u32) -> Vec<StaticMeshInstance> {
    if kinds.is_empty() || n == 0 { return vec![]; }
    let mut s = seed.wrapping_mul(2654435761).wrapping_add(12345);
    let mut rnd = move || { s ^= s << 13; s ^= s >> 17; s ^= s << 5; (s >> 8) as f32 / (1u32 << 24) as f32 };
    let cols = ((n as f32 * 1.6).sqrt().ceil() as usize).max(1);
    let spacing = 6.0;
    let total: f32 = kinds.iter().map(|k| k.weight.max(0.01)).sum();
    (0..n).map(|i| {
        let (cx, cz) = ((i % cols) as f32, (i / cols) as f32);
        let pos = Vec3::new((cx - cols as f32 * 0.5) * spacing + (rnd() - 0.5) * spacing * 0.8, 0., -6. - cz * spacing + (rnd() - 0.5) * spacing * 0.8);
        let mut pick = rnd() * total;
        let kind = kinds.iter().find(|k| { pick -= k.weight.max(0.01); pick <= 0. }).unwrap_or(&kinds[0]);
        let scale = kind.scale.0 + rnd() * (kind.scale.1 - kind.scale.0);
        StaticMeshInstance::new(kind.mesh, pos).with_yaw(rnd() * std::f32::consts::TAU).with_scale(scale).with_shake(if i % 7 == 0 { 0.2 } else { 0. })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn kinds() -> Vec<PropKind> { (0..3).map(|i| PropKind { name: format!("k{i}"), mesh: StaticMeshHandle(i), weight: 1., rock: i == 2, scale: (0.8, 1.3) }).collect() }
    #[test]
    fn grid_is_deterministic_has_all_kinds_and_the_requested_count() {
        let a = grid(&kinds(), 300, 1);
        assert_eq!(a, grid(&kinds(), 300, 1));
        assert_ne!(a, grid(&kinds(), 300, 2));
        assert_eq!(a.len(), 300);
        for k in 0..3 { assert!(a.iter().any(|i| i.mesh.0 == k), "kind {k} missing"); }
        assert!(a.iter().any(|i| i.shake > 0.));
        assert!(grid(&kinds(), 0, 1).is_empty() && grid(&[], 5, 1).is_empty());
    }
    #[test]
    fn grid_stays_in_front_of_the_spawn() {
        let a = grid(&kinds(), 500, 3);
        assert!(a.iter().all(|i| i.pos.z < 0.0), "nothing near the pawn at z = 12");
    }
}
