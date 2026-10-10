//! ARCH (2026-10-07): the cooked archway walls of the POIs (`WoodMines_Archway` / `Boardwalk_Archway`, `BuildingWall`) collide through their five
//! `FKConvexElem` pieces (two pillars, two shoulders, the lintel), so the opening is open: the pawn walks through it, a pickaxe swing through it hits
//! nothing, a pillar blocks both. Built from the exported `lodge.json` the same way fn-app `poi.rs` builds a POI actor (one hull per element, UE cm ->
//! game metres `(x, z, y) * 0.01`). Needs the exported props (`FN_PROPS_DIR`, default the sibling FortniteRuntime\assets\props); skipped without.
use fn_combat::{Harvestable, Kind, Resource, World};
use fn_hull::{Hull, HullSet};
use fn_movement::Controller;
use fn_props::player_hulls;
use glam::{Quat, Vec2, Vec3};
use std::sync::Arc;

fn props_dir() -> std::path::PathBuf {
    std::env::var("FN_PROPS_DIR").map(Into::into).unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../FortniteRuntime/assets/props"))
}

/// The archway mesh's pieces in game metres, placed with its open side along game z at `z0` (UE yaw 0: the wall plane is UE x/z).
fn arch(mesh: &str, z0: f32) -> Option<Harvestable> {
    let doc: serde_json::Value = serde_json::from_slice(&std::fs::read(props_dir().join("poi/lodge.json")).ok()?).ok()?;
    let m = doc["meshes"].as_object()?.iter().find(|(k, _)| k.ends_with(mesh))?.1.clone();
    let col = fn_propdata::Collision::from_json(&m).ok()?;
    let to_game = |u: Vec3| Vec3::new(u.x, u.z, u.y) * 0.01 + Vec3::new(0., 0., z0);
    let hulls: Vec<Hull> = col.shapes.iter().filter_map(|s| match s {
        fn_propdata::Shape::Convex { vertices, transform_translation, transform_rotation, transform_scale, .. } => {
            let (r, sc, t) = (Quat::from_array(*transform_rotation).normalize(), Vec3::from(*transform_scale), Vec3::from(*transform_translation));
            Hull::from_points(&vertices.iter().map(|v| to_game(r * (Vec3::from(*v) * sc) + t)).collect::<Vec<_>>())
        }
        _ => None,
    }).collect();
    assert_eq!(hulls.len(), 5, "{mesh}: the cooked BodySetup has 5 convex elements");
    let mut h = Harvestable::real(Kind::Tree, Resource::Wood, Vec3::new(0., 0., z0), 0., 1., 300., fn_combat::POI_PROTO | 1, Some(Arc::new(HullSet::new(hulls))));
    h.no_yield = false;
    Some(h)
}

fn walk(props: &[Harvestable], x: f32, secs: f32) -> Controller {
    let mut c = Controller::default();
    c.position = Vec2::new(x, 0.); // world z 12
    let mut t = 0.;
    while t < secs { c.hulls = player_hulls(props, 12., c.position, 6.); c.step(Vec2::new(0., -1.), false, true, 1. / 60.); t += 1. / 60.; }
    c
}

#[test]
fn the_pawn_walks_through_the_opening_and_a_pillar_blocks() {
    for mesh in ["S_WoodMines_Archway", "S_Boardwalk_Archway"] {
        let Some(a) = arch(mesh, 8.) else { eprintln!("no lodge.json: skipped"); return };
        let props = vec![a];
        let c = walk(&props, 0., 3.);
        assert!(c.position.y < -6., "{mesh}: the pawn passes the arch (world z 8 = controller y -4): {:?}", c.position);
        let c = walk(&props, 2.3, 3.); // the pillar (UE x 208..256 = 2.08..2.56 m)
        assert!(c.position.y > -4. + 0.42, "{mesh}: the pillar stops the pawn: {:?}", c.position);
    }
}

#[test]
fn the_pickaxe_and_the_camera_probe_go_through_the_opening_and_hit_a_pillar() {
    for mesh in ["S_WoodMines_Archway", "S_Boardwalk_Archway"] {
        let Some(a) = arch(mesh, 8.) else { return };
        let mut w = World { props: vec![a], ..Default::default() };
        // pawn 1.5 m in front of the wall, swinging through the middle of the opening
        assert!(!w.strike_plain(Vec3::new(0., 0., 9.5), Vec3::NEG_Z), "{mesh}: swing through the opening");
        assert!(fn_props::trace(&w.props, Vec3::new(0., 1.2, 12.), Vec3::new(0., 1.2, 4.), 0.08).is_none(), "{mesh}: camera probe through the opening");
        // ... and into a pillar
        assert!(w.strike_plain(Vec3::new(2.3, 0., 9.5), Vec3::NEG_Z), "{mesh}: swing into the pillar");
    }
}

#[test]
fn a_hit_on_a_pillar_gets_a_weak_spot_on_the_arch() {
    use fn_combat::harvest_fx::Aim;
    for mesh in ["S_WoodMines_Archway", "S_Boardwalk_Archway"] {
        for (px, pz, ex) in [(2.3f32, 9.5f32, 0f32), (-2.3, 9.6, 0.), (2.0, 10.5, -1.5), (-1.7, 10.2, 1.5), (1.9, 9.6, -2.4), (-1.9, 9.6, 2.4)] {
            let Some(a) = arch(mesh, 8.) else { return };
            let mut w = World { props: vec![a], ..Default::default() };
            let pawn = Vec3::new(px, 0., pz);
            let pillar = Vec3::new(px.signum() * 2.3, 1.1, 8.);
            let eye = pawn + Vec3::new(ex, 1.6, 2.); // the camera boom: behind the pawn, possibly across the opening
            w.fx.aim = Some(Aim { eye, dir: (pillar - eye).normalize() });
            assert!(w.strike(pawn, (Vec3::new(pillar.x, 0., pillar.z) - pawn).normalize()), "{mesh} ({px}, {pz}): the swing lands on the pillar");
            let s = w.fx.weak_spot(0, &w.props[0]).unwrap_or_else(|| panic!("{mesh} ({px}, {pz}): no weak spot on the arch after a hit"));
            assert!(w.props[0].hull.as_ref().unwrap().sdf(s.pos).abs() < 0.05, "{mesh}: the spot lies on the collision surface: {:?}", s.pos);
            assert!(s.pos.x * px > 0. && (s.pos - pawn).length() < fn_combat::harvest_fx::WEAKSPOT_LINE_RANGE, "{mesh} ({px}, {pz}): the spot is on the struck side, in reach: {:?}", s.pos);
        }
    }
}

#[test]
fn hitting_an_arch_until_it_breaks_removes_all_its_collision() {
    let Some(a) = arch("S_WoodMines_Archway", 8.) else { return };
    let mut w = World { props: vec![a], ..Default::default() };
    let pawn = Vec3::new(2.3, 0., 9.5);
    let mut n = 0;
    while w.props[0].alive() && n < 20 { assert!(w.strike_plain(pawn, Vec3::NEG_Z)); n += 1; }
    assert_eq!(n, 6, "300 hp (BuildingWood.2 MaxHealth t0) / 50 per ordinary hit");
    assert!(w.events.last().unwrap().destroyed);
    assert!(fn_props::trace(&w.props, Vec3::new(2.3, 1.2, 12.), Vec3::new(2.3, 1.2, 4.), 0.08).is_none() && player_hulls(&w.props, 12., Vec2::new(2.3, -4.), 6.).is_empty(), "a destroyed arch keeps no collision");
}
