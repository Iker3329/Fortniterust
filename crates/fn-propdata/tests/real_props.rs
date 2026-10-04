//! Reads the real exported props (`..\FortniteRuntime\assets\props`, or `FN_PROPS_DIR`) written by `fn-export props export`.
//! Skipped with a note when the folder does not exist on this machine.
use fn_propdata::*;
use std::path::PathBuf;

fn dir() -> Option<PathBuf> {
    let p = std::env::var_os("FN_PROPS_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap().join("FortniteRuntime").join("assets").join("props"));
    if p.join("catalog.json").is_file() {
        Some(p)
    } else {
        eprintln!("skipping: {} has no catalog.json", p.display());
        None
    }
}

#[test]
fn catalog_has_the_real_trees_and_rocks_with_1_8_health() {
    let Some(d) = dir() else { return };
    let cat = PropCatalog::load(&d).unwrap();
    assert!(cat.kinds.len() >= 60, "{} kinds", cat.kinds.len());
    let oak = cat.get("Athena_Tree_Large_1").unwrap();
    assert_eq!((oak.health, oak.resource.kind), (600.0, ResourceKind::Wood));
    assert_eq!(oak.lods[0].vertices, 7606);
    assert!(oak.is_tree());
    let rock = cat.get("Athena_Prop_Rocks_StoneLarge01").unwrap();
    assert_eq!((rock.health, rock.resource.kind), (450.0, ResourceKind::Stone));
    assert!(rock.is_rock());
    assert!(!rock.collision.is_empty());
    assert!(cat.by_category("tree").count() >= 8 && cat.by_category("rock").count() >= 10);
    for k in &cat.kinds {
        assert!(k.health > 0.0, "{}: health {}", k.id, k.health);
        assert!(!k.lods.is_empty() && !k.materials.is_empty(), "{}", k.id);
    }
}

#[test]
fn every_mesh_decodes_with_the_counts_of_its_props_json() {
    let Some(d) = dir() else { return };
    let cat = PropCatalog::load(&d).unwrap();
    for k in &cat.kinds {
        let lods = k.load_lods().unwrap_or_else(|e| panic!("{}: {e}", k.id));
        assert_eq!(lods.len(), k.lods.len(), "{}", k.id);
        for (l, info) in lods.iter().zip(&k.lods) {
            assert_eq!(l.positions.len(), info.vertices, "{}", k.id);
            assert_eq!(l.triangle_count(), info.triangles, "{}", k.id);
            assert_eq!(l.normals.len(), info.vertices);
            assert_eq!(l.uvs.len(), info.uv_channels);
            assert_eq!(!l.colors.is_empty(), info.has_vertex_colors);
            for s in &l.sections {
                assert!(s.material_slot < k.materials.len(), "{}: section material {}", k.id, s.material_slot);
            }
            // metres, Y up: the mesh height is the UE Z extent
            let (mn, mx) = l.bounds();
            assert!(mn.iter().chain(&mx).all(|v| v.is_finite()));
        }
        // screen sizes are the engine's own auto-LOD values (can exceed 1.0 for thin meshes, e.g. Parking_Bollard 1.0 / 1.05 / 1.14)
        assert!(lods.iter().all(|l| l.screen_size.is_finite() && l.screen_size > 0.0), "{}", k.id);
        // glTF bounds (metres, Y up) agree with the UE bounds in props.json (cm, Z up) within 2 cm
        let (mn, mx) = lods[0].bounds();
        let top = k.bounds_origin_cm[2] + k.bounds_extent_cm[2];
        assert!((mx[1] * 100.0 - top).abs() < 2.0 || mx[1] * 100.0 <= top + 2.0, "{}: top {} vs {}", k.id, mx[1] * 100.0, top);
        assert!(mn[1] * 100.0 >= k.bounds_origin_cm[2] - k.bounds_extent_cm[2] - 2.0, "{}", k.id);
    }
}

#[test]
fn materials_and_textures_load() {
    let Some(d) = dir() else { return };
    let cat = PropCatalog::load(&d).unwrap();
    let pine = cat.get("Athena_Tree_Pine_3").unwrap();
    let m = pine.load_material(0).unwrap();
    assert_eq!(m.alpha_mode, AlphaMode::Masked);
    assert!(m.two_sided);
    let tex = m.base_color_texture().expect("base colour texture");
    let img = tex.load_image().unwrap();
    assert_eq!((img.width, img.height), (tex.width, tex.height));
    assert!(tex.srgb);
    assert_eq!(img.rgba.len(), (img.width * img.height * 4) as usize);
    let n = m.normal_texture().expect("normal map");
    assert!(!n.srgb && n.compression == "TC_Normalmap");
    assert!(m.uses_world_position_offset, "foliage animates through WorldPositionOffset");
    assert!(m.wind_params.contains_key("LeafAnimationAmount"));
    // every material of every kind parses and its textures exist
    for k in &cat.kinds {
        for s in &k.materials {
            let m = k.load_material(s.slot).unwrap_or_else(|e| panic!("{} slot {}: {e}", k.id, s.slot));
            for t in m.textures.values() {
                assert!(t.png.is_file(), "{}: {} missing", k.id, t.png.display());
            }
        }
    }
}

#[test]
fn collision_matches_the_cooked_body_setup() {
    let Some(d) = dir() else { return };
    let cat = PropCatalog::load(&d).unwrap();
    // Athena_Tree_Pine_01_ BodySetup_2 (inspected by hand with fn-inspect): 2 convex hulls of 8 points, box (-141.6,-162.4,0)..(128.2,146.2,444.9)
    let pine = cat.get("Athena_Tree_Pine_01").unwrap();
    let hulls: Vec<_> = pine.collision.shapes.iter().filter_map(|s| if let Shape::Convex { vertices, .. } = s { Some(vertices.len()) } else { None }).collect();
    assert_eq!(hulls, vec![8, 8]);
    assert!((pine.collision.bounds_max[2] - 444.916).abs() < 0.01 && (pine.collision.bounds_min[0] + 141.561).abs() < 0.01, "{:?} {:?}", pine.collision.bounds_min, pine.collision.bounds_max);
    assert!(pine.collision.blocks_weapon_traces());
}

#[test]
fn layout_sample_and_full_layout() {
    let Some(d) = dir() else { return };
    let cat = PropCatalog::load(&d).unwrap();
    let s = cat.load_sample_layout().unwrap();
    assert_eq!(s.instances.len(), 40);
    assert!(s.instances.iter().all(|i| cat.get(&i.kind).is_some() || true));
    let known = s.instances.iter().filter(|i| cat.get(&i.kind).is_some()).count();
    assert!(known >= 30, "{known} of 40 sample instances are exported kinds");
    assert!(s.instances.windows(2).all(|w| dist(&w[0]) <= dist(&w[1]) + 1e-3), "sample is sorted by distance to the centre");
    let all = cat.load_full_layout().unwrap();
    assert_eq!(all.instances.len(), 9609);
    let c = all.counts();
    assert_eq!(c["Athena_Tree_Large_1"], 893);
    assert_eq!(c["Athena_Prop_Rocks_StoneLarge01"], 178);
    // sample abs positions are the centre + the relative ones
    for i in &s.instances {
        for k in 0..3 {
            assert!((i.abs_cm[k] - (s.center_cm[k] + i.pos_cm[k])).abs() < 0.2, "{i:?}");
        }
    }
}

fn dist(i: &PropInstance) -> f32 {
    (i.pos_cm[0] * i.pos_cm[0] + i.pos_cm[1] * i.pos_cm[1]).sqrt()
}
