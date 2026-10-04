//! Loader tests on a hand-built catalog (no game data needed).
use fn_propdata::*;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("fn-propdata-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// One triangle LOD0 (3 vertices) + one quad LOD1 with a shared vertex set, u16 indices.
fn write_gltf(dir: &std::path::Path) {
    let mut bin: Vec<u8> = vec![];
    let f = |v: &[f32], bin: &mut Vec<u8>| {
        for x in v {
            bin.extend_from_slice(&x.to_le_bytes());
        }
    };
    // LOD0: positions(36) normals(36) uv(24) indices(6 -> pad 8)
    f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0], &mut bin);
    f(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0], &mut bin);
    f(&[0.0, 0.0, 1.0, 0.0, 0.0, 1.0], &mut bin);
    for i in [0u16, 1, 2, 0, 0, 0] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let idx0_off = 36 + 36 + 24;
    // LOD1: 4 vertices
    let p1 = bin.len();
    f(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0], &mut bin);
    f(&[0.0, 0.0, 1.0].repeat(4), &mut bin);
    f(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0], &mut bin);
    let i1 = bin.len();
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    std::fs::write(dir.join("M.bin"), &bin).unwrap();
    let j = serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"name": "LOD0", "mesh": 0, "extras": {"screen_size": 1.0}}, {"name": "LOD1", "mesh": 1, "extras": {"screen_size": 0.4}}],
        "meshes": [
            {"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1, "TEXCOORD_0": 2}, "indices": 3, "material": 0, "mode": 4}]},
            {"primitives": [{"attributes": {"POSITION": 4, "NORMAL": 5, "TEXCOORD_0": 6}, "indices": 7, "material": 1, "mode": 4}]},
        ],
        "buffers": [{"uri": "M.bin", "byteLength": bin.len()}],
        "bufferViews": [
            {"buffer": 0, "byteOffset": 0, "byteLength": 36}, {"buffer": 0, "byteOffset": 36, "byteLength": 36}, {"buffer": 0, "byteOffset": 72, "byteLength": 24},
            {"buffer": 0, "byteOffset": idx0_off, "byteLength": 12},
            {"buffer": 0, "byteOffset": p1, "byteLength": 48}, {"buffer": 0, "byteOffset": p1 + 48, "byteLength": 48}, {"buffer": 0, "byteOffset": p1 + 96, "byteLength": 32},
            {"buffer": 0, "byteOffset": i1, "byteLength": 12},
        ],
        "accessors": [
            {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"}, {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"}, {"bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC2"},
            {"bufferView": 3, "byteOffset": 0, "componentType": 5123, "count": 3, "type": "SCALAR"},
            {"bufferView": 4, "componentType": 5126, "count": 4, "type": "VEC3"}, {"bufferView": 5, "componentType": 5126, "count": 4, "type": "VEC3"}, {"bufferView": 6, "componentType": 5126, "count": 4, "type": "VEC2"},
            {"bufferView": 7, "componentType": 5123, "count": 6, "type": "SCALAR"},
        ],
    });
    std::fs::write(dir.join("M.gltf"), j.to_string()).unwrap();
}

fn fixture() -> PathBuf {
    let root = tmp("catalog");
    let kind = root.join("Test_Tree");
    std::fs::create_dir_all(kind.join("materials")).unwrap();
    std::fs::create_dir_all(root.join("textures")).unwrap();
    std::fs::create_dir_all(root.join("layout")).unwrap();
    write_gltf(&kind);
    let mut img = image::RgbaImage::new(2, 2);
    for (i, p) in img.pixels_mut().enumerate() {
        *p = image::Rgba([i as u8 * 60, 10, 20, if i == 3 { 0 } else { 255 }]);
    }
    img.save(root.join("textures/T_D.png")).unwrap();
    std::fs::write(
        kind.join("materials/M_Leaf.mat.json"),
        serde_json::json!({
            "schema": 1, "name": "M_Leaf", "ue_path": "/Game/M_Leaf", "blend_mode": "BLEND_Masked", "alpha_mode": "MASK", "opacity_mask_clip_value": 0.4, "two_sided": true, "shading_model": "MSM_TwoSidedFoliage",
            "textures": {"Diffuse": {"texture": "T_D", "png": "../../textures/T_D.png", "srgb": true, "compression": "TC_Default", "width": 2, "height": 2, "source_width": 2048, "source_height": 2048, "has_alpha": true}},
            "slots": {"base_color": "Diffuse", "opacity_from": "base_color"},
            "scalars": {"LeafAnimationAmount": 0.05}, "vectors": {"Tint": [1.0, 0.5, 0.25, 1.0]}, "static_switches": {"Animate": true},
            "wind": {"uses_world_position_offset": true, "parameters": {"LeafAnimationAmount": 0.05}}, "tint_guess": [0.5, 1.0, 0.25],
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        kind.join("collision.json"),
        serde_json::json!({
            "schema": 1, "trace_flag": "UseDefault", "profile": "BlockAll", "collision_enabled": "ECollisionEnabled::QueryOnly",
            "responses": [{"channel": "FCC_Trace_Weapon", "response": "ECR_Block"}],
            "shapes": [
                {"type": "box", "center": [0, 0, 100], "rotation": [0, 0, 0, 1], "half_extent": [20, 20, 100]},
                {"type": "sphere", "center": [0, 0, 300], "radius": 150},
                {"type": "capsule", "center": [0, 0, 50], "rotation": [0, 0, 0, 1], "radius": 30, "half_length": 70},
                {"type": "convex", "vertices": [[0, 0, 0], [10, 0, 0], [0, 10, 0], [0, 0, 10]], "transform": {"translation": [1, 2, 3], "rotation": [0, 0, 0, 1], "scale": [1, 1, 1]}, "bounds_min": [0, 0, 0], "bounds_max": [10, 10, 10]},
            ],
            "bounds_min": [-20, -20, 0], "bounds_max": [20, 20, 450],
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        kind.join("props.json"),
        serde_json::json!({
            "schema": 1, "id": "Test_Tree", "display": "Test Tree", "category": "tree", "blueprint": "/Game/Test_Tree",
            "mesh": {"name": "M", "ue_path": "/Game/M.M", "gltf": "M.gltf", "bounds_cm": {"origin": [0, 0, 100], "extent": [100, 100, 100], "radius": 200},
                     "lods": [{"vertices": 3, "triangles": 1, "screen_size": 1.0, "uv_channels": 1, "vertex_colors": false, "sections": [{"material_slot": 0, "triangles": 1}]},
                              {"vertices": 4, "triangles": 2, "screen_size": 0.4, "uv_channels": 1, "vertex_colors": false, "sections": [{"material_slot": 1, "triangles": 2}]}]},
            "materials": [{"slot": 0, "slot_name": "Bark", "name": "M_Leaf", "mat_json": "materials/M_Leaf.mat.json", "source": "mesh"}, {"slot": 1, "slot_name": "Leaves", "name": "M_Leaf", "mat_json": "materials/M_Leaf.mat.json", "source": "mesh"}],
            "collision": "collision.json",
            "health": {"max": 600.0, "attribute_row": "ResourceWood.4.FortHealthSet.MaxHealth"},
            "resource": {"type": "Wood", "amount_row": "ResourceWoodHigh", "rate_t0": 40.0, "rate_t1": 0.3, "athena_rate_t0": 48.0, "athena_rate_t1": 0.3},
            "destruction": {"death_particles": "/Game/P", "hit_sound": "/Game/S"},
            "render": {"cull_distance_cm": 16395.5, "random_scale": true, "random_scale_range": [0.9, 1.25], "random_yaw": true, "cast_shadow": true},
            "placement": {"athena_terrain_count": 7},
        })
        .to_string(),
    )
    .unwrap();
    std::fs::write(root.join("catalog.json"), serde_json::json!({"schema": 1, "kinds": [{"id": "Test_Tree", "dir": "Test_Tree"}]}).to_string()).unwrap();
    std::fs::write(
        root.join("layout/sample.json"),
        serde_json::json!({"map": "M", "center_abs": [1000.0, 2000.0, 50.0], "radius_cm": 300.0, "instances": [
            {"class": "Test_Tree", "pos": [100.0, 0.0, 10.0], "abs": [1100.0, 2000.0, 60.0], "rot": [0.0, 90.0, 0.0], "scale": [1.0, 1.0, 1.0]}]})
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        root.join("layout/all.json"),
        serde_json::json!({"map": "M", "classes": ["A", "Test_Tree"], "instances": [[1, 1.0, 2.0, 3.0, 0.0, 45.0, 0.0, 1.0, 1.0, 1.0], [0, 4.0, 5.0, 6.0, 0.0, 0.0, 0.0, 2.0, 2.0, 2.0]]}).to_string(),
    )
    .unwrap();
    root
}

#[test]
fn catalog_kind_fields() {
    let root = fixture();
    let cat = PropCatalog::load(&root).unwrap();
    let k = cat.get("Test_Tree").unwrap();
    assert_eq!((k.health, k.resource.kind, k.placed_in_map), (600.0, ResourceKind::Wood, 7));
    assert_eq!(k.resource.rates, Some((40.0, 0.3)));
    assert_eq!(k.resource.athena_rates, Some((48.0, 0.3)));
    assert_eq!(k.resource.amount_row.as_deref(), Some("ResourceWoodHigh"));
    assert_eq!(k.destruction.death_particles.as_deref(), Some("/Game/P"));
    assert_eq!(k.render.random_scale_range, Some((0.9, 1.25)));
    assert!((k.render.cull_distance_cm - 16395.5).abs() < 0.1);
    assert_eq!(k.lods.len(), 2);
    assert_eq!(k.lods[1].sections, vec![(1, 2)]);
    assert!((k.height_m() - 2.0).abs() < 1e-6);
    assert!(k.is_tree() && !k.is_rock());
    assert_eq!(cat.by_resource(ResourceKind::Wood).count(), 1);
    assert_eq!(cat.by_resource(ResourceKind::Stone).count(), 0);
}

#[test]
fn mesh_lods_decode() {
    let root = fixture();
    let cat = PropCatalog::load(&root).unwrap();
    let lods = cat.get("Test_Tree").unwrap().load_lods().unwrap();
    assert_eq!(lods.len(), 2);
    assert_eq!(lods[0].positions.len(), 3);
    assert_eq!(lods[0].triangle_count(), 1);
    assert_eq!(lods[0].sections[0].indices, vec![0, 1, 2]);
    assert_eq!(lods[1].sections[0].material_slot, 1);
    assert_eq!(lods[1].sections[0].indices, vec![0, 1, 2, 0, 2, 3]);
    assert_eq!(lods[1].screen_size, 0.4);
    assert_eq!(lods[0].bounds(), ([0.0, 0.0, 0.0], [1.0, 2.0, 0.0]));
    assert_eq!(lods[1].uvs[0][3], [0.0, 1.0]);
}

#[test]
fn material_collision_and_texture() {
    let root = fixture();
    let cat = PropCatalog::load(&root).unwrap();
    let k = cat.get("Test_Tree").unwrap();
    let m = k.load_material(0).unwrap();
    assert_eq!((m.alpha_mode, m.two_sided, m.alpha_cutoff), (AlphaMode::Masked, true, 0.4));
    assert_eq!(m.vectors["Tint"], [1.0, 0.5, 0.25, 1.0]);
    assert_eq!(m.tint_guess, Some([0.5, 1.0, 0.25]));
    assert!(m.uses_world_position_offset && m.wind_params["LeafAnimationAmount"] == 0.05);
    let t = m.base_color_texture().unwrap();
    assert_eq!((t.source_width, t.width), (2048, 2));
    let im = t.load_image().unwrap();
    assert_eq!(im.pixel(1, 1)[3], 0);
    assert_eq!(im.pixel(0, 0), [0, 10, 20, 255]);
    assert!(k.load_material(5).is_err());
    let c = &k.collision;
    assert_eq!(c.shapes.len(), 4);
    assert!(matches!(c.shapes[0], Shape::Box { half_extent, .. } if half_extent == [20.0, 20.0, 100.0]));
    assert!(matches!(c.shapes[1], Shape::Sphere { radius, .. } if radius == 150.0));
    assert!(matches!(c.shapes[2], Shape::Capsule { half_length, .. } if half_length == 70.0));
    assert!(matches!(&c.shapes[3], Shape::Convex { vertices, transform_translation, .. } if vertices.len() == 4 && *transform_translation == [1.0, 2.0, 3.0]));
    assert!(c.blocks_weapon_traces());
    assert_eq!(c.bounds_max, [20.0, 20.0, 450.0]);
}

#[test]
fn layouts_and_axis_conversion() {
    let root = fixture();
    let cat = PropCatalog::load(&root).unwrap();
    let s = cat.load_sample_layout().unwrap();
    assert_eq!(s.center_cm, [1000.0, 2000.0, 50.0]);
    let i = &s.instances[0];
    let gp = i.game_position();
    assert!((gp[0] - 1.0).abs() < 1e-6 && (gp[1] - 0.1).abs() < 1e-6 && gp[2].abs() < 1e-6, "{gp:?}");
    // UE yaw +90 turns UE +X into UE +Y (= game +Z); the game's rotation about +Y must do the same
    let y = i.game_yaw();
    let (c, sn) = (y.cos(), y.sin());
    let rotated = [c * 1.0 + sn * 0.0, -sn * 1.0 + c * 0.0]; // (x, z) -> (x cos + z sin, -x sin + z cos) applied to game +X
    assert!(rotated[0].abs() < 1e-6 && (rotated[1] - 1.0).abs() < 1e-6, "{rotated:?}");
    let all = cat.load_full_layout().unwrap();
    assert_eq!(all.instances.len(), 2);
    assert_eq!(all.instances[0].kind, "Test_Tree");
    assert_eq!(all.instances[1].kind, "A");
    assert_eq!(all.instances[1].uniform_scale(), 2.0);
    assert_eq!(all.counts()["Test_Tree"], 1);
}

#[test]
fn bad_catalogs_are_reported() {
    let root = tmp("bad");
    assert!(matches!(PropCatalog::load(&root), Err(Error::Io(_))));
    std::fs::write(root.join("catalog.json"), r#"{"schema": 2, "kinds": []}"#).unwrap();
    assert!(matches!(PropCatalog::load(&root), Err(Error::Schema(_))));
}
