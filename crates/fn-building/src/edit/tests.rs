//! Edit tests against the real 1.8 paks (`FN_TEST_ROOT`). Skipped when it is not set.

use std::sync::OnceLock;

use glam::{IVec3, Vec3};

use super::pattern::{find_match, Pattern, Tiles, BTV_ACTIVE, BTV_INACTIVE};
use super::*;
use crate::pieces::{Material, PieceKind};
use crate::world::{GridKey, PieceSpawn, WorldEvent};

fn db() -> Option<&'static EditDb> {
    static DB: OnceLock<Option<EditDb>> = OnceLock::new();
    DB.get_or_init(|| {
        let root = fn_live::var_os("FN_TEST_ROOT")?;
        let set = fn_pak::PakSet::open_install(std::path::PathBuf::from(root)).ok()?;
        Some(EditDb::load(&set).expect("edit db"))
    })
    .as_ref()
}

const W1: &str = "/Game/Building/ActorBlueprints/Player/Wood/L1/";

fn class(name: &str) -> String { format!("{W1}{name}.{name}_C") }
fn short(path: &str) -> &str { path.rsplit('.').next().unwrap().trim_end_matches("_C") }

fn spawn(world: &mut BuildingWorld, kind: PieceKind, name: &str) -> PieceId {
    world.spawn(PieceSpawn {
        kind, material: Material::Wood, class_path: class(name),
        key: GridKey { kind, cell: IVec3::ZERO, rot: 0 }, location: Vec3::ZERO, yaw_deg: 0.,
        max_health: 150., start_health: 150., build_time: 0.,
    })
}

/// Resolves every selection of a piece type (as `vfunc_93` does on a fresh, unmirrored actor) and prints the table.
fn table(db: &EditDb, kind: PieceKind, base: &str) -> Vec<(Vec<u8>, Option<(String, i32, bool)>)> {
    let of = db.class(&class(base)).unwrap();
    let cands = db.possible_edit_classes(of);
    let pats: Vec<Option<&Pattern>> = cands.iter().map(|c| db.pattern_of(c)).collect();
    let n = Pattern::class_default(kind).len();
    let mut out = Vec::new();
    for bits in 0..(1u32 << n) {
        let tiles: Vec<u8> = (0..n).map(|i| if bits >> i & 1 == 1 { BTV_INACTIVE } else { BTV_ACTIVE }).collect();
        let p = Pattern { kind, n: if n == 9 { 3 } else { 2 }, tiles: Tiles::Binary(tiles.clone()) };
        let m = find_match(kind, &p, false, &pats).map(|m| (short(&cands[m.class_index].class_path).to_string(), m.rotation_iterations, m.mirrored));
        out.push((tiles, m));
    }
    out
}

fn fmt_tiles(t: &[u8], n: usize) -> String {
    t.chunks(n).map(|r| r.iter().map(|&v| if v == BTV_ACTIVE { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("/")
}

#[test]
fn wall_floor_roof_tables() {
    let Some(db) = db() else { eprintln!("FN_TEST_ROOT not set, skipping"); return };
    for (kind, base, n) in [(PieceKind::Wall, "PBWA_W1_Solid", 3), (PieceKind::Floor, "PBWA_W1_Floor", 2), (PieceKind::Roof, "PBWA_W1_RoofC", 2)] {
        let t = table(db, kind, base);
        let valid = t.iter().filter(|(_, m)| m.is_some()).count();
        eprintln!("== {kind:?}: {valid}/{} selections map to a class", t.len());
        for (tiles, m) in &t {
            if let Some((c, r, mi)) = m { eprintln!("  {} -> {c} rot {r} mirror {mi}", fmt_tiles(tiles, n)); }
        }
    }
    let wall = table(db, PieceKind::Wall, "PBWA_W1_Solid");
    let get = |t: &[u8]| wall.iter().find(|(x, _)| x == t).unwrap().1.clone();
    // EMP_Wall_WindowC: centre tile off.
    assert_eq!(get(&[0, 0, 0, 0, 1, 0, 0, 0, 0]).unwrap().0, "PBWA_W1_WindowC");
    // EMP_Wall_DoorC: centre column, lower two tiles off.
    assert_eq!(get(&[0, 0, 0, 0, 1, 0, 0, 1, 0]).unwrap().0, "PBWA_W1_DoorC");
    assert_eq!(get(&[0; 9]).unwrap(), ("PBWA_W1_Solid".to_string(), 0, false));
    // All tiles off: no wall class matches -> invalid.
    assert!(get(&[1; 9]).is_none());
}

#[test]
fn session_drag_and_confirm() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Wall, "PBWA_W1_Solid");
    let mut c = EditController::new();
    // Out of range (centroid (0,0,192), 750 cancel distance).
    assert_eq!(c.edit_begin(db, &world, Some(id), Vec3::new(800., 0., 192.)), Err(BeginError::OutOfRange));
    c.edit_begin(db, &world, Some(id), Vec3::new(300., 0., 0.)).unwrap();
    // Drag from the centre tile down: first tile toggles off, the drag forces the next one off too.
    c.edit_press();
    c.edit_hover(Some(EditTarget::Tile(4)));
    c.edit_hover(Some(EditTarget::Tile(7)));
    c.edit_release();
    assert!(c.session.as_ref().unwrap().state_valid);
    assert!(c.edit_confirm(&mut world));
    assert_eq!(short(world.get(id).unwrap().effective_class()), "PBWA_W1_DoorC");
    let ev = c.drain_events();
    assert!(ev.iter().any(|e| matches!(e, EditEvent::Sound("BuildPreviewEditEnter", _))));
    assert!(ev.iter().any(|e| matches!(e, EditEvent::Sound("BuildPreviewEditTile", _))));

    // Re-edit the door, reset (all tiles on) and confirm: back to the solid wall.
    c.edit_begin(db, &world, Some(id), Vec3::ZERO).unwrap();
    assert_eq!(c.session.as_ref().unwrap().preview.binary(7), Some(BTV_INACTIVE));
    c.edit_reset();
    assert!(c.edit_confirm(&mut world));
    assert_eq!(short(world.get(id).unwrap().effective_class()), "PBWA_W1_Solid");

    // Invalid selection: confirm does nothing.
    c.edit_begin(db, &world, Some(id), Vec3::ZERO).unwrap();
    for i in 0..9 { c.edit_select_tile(i); }
    assert!(!c.session.as_ref().unwrap().state_valid);
    assert!(!c.edit_confirm(&mut world));
    assert_eq!(short(world.get(id).unwrap().effective_class()), "PBWA_W1_Solid");
}

#[test]
fn stairs_paths() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Stairs, "PBWA_W1_StairW");
    let mut s = EditSession::begin(db, &world, id, Vec3::ZERO).unwrap();
    // Path 2 -> 0 -> 1 -> 3 (a U turn): StairR is [2,3,1,4] = tile0 step2, tile1 step3, tile2 step1, tile3 step4.
    s.begin_action();
    for t in [2, 0, 1, 3] { s.update_hover(Some(EditTarget::Tile(t))); }
    s.end_action();
    eprintln!("stair state {:?} rot {} mirror {} valid {}", short(&s.state.class_path), s.state.rotation_iterations, s.state.mirrored, s.state_valid);
    assert!(s.state_valid);
    assert_eq!(short(&s.state.class_path), "PBWA_W1_StairR");
    // A non adjacent second step (2 -> 1, diagonal) is refused by the adjacency table.
    s.begin_action();
    assert!(s.update_hover(Some(EditTarget::Tile(2))));
    assert!(!s.update_hover(Some(EditTarget::Tile(1))));
    s.end_action();
    // Reset = the straight ramp.
    s.reset();
    assert_eq!(s.preview.tiles, Tiles::Stair(vec![2, 2, 1, 1]));
    assert_eq!(short(&s.state.class_path), "PBWA_W1_StairW");
    // Every reachable 4-step path / indicator path resolves or is reported invalid; print the table.
    let of = db.class(&class("PBWA_W1_StairW")).unwrap();
    let cands = db.possible_edit_classes(of);
    let pats: Vec<Option<&Pattern>> = cands.iter().map(|c| db.pattern_of(c)).collect();
    for perm in permutations(&[0, 1, 2, 3]) {
        let mut tiles = vec![0; 4];
        for (step, &t) in perm.iter().enumerate() { tiles[t] = step as i32 + 1; }
        let p = Pattern { kind: PieceKind::Stairs, n: 2, tiles: Tiles::Stair(tiles.clone()) };
        if let Some(m) = find_match(PieceKind::Stairs, &p, false, &pats) {
            eprintln!("  stair {tiles:?} -> {} rot {} mirror {}", short(&cands[m.class_index].class_path), m.rotation_iterations, m.mirrored);
        }
    }
}

/// A wood L1 piece as the placement path spawns it (catalog numbers of its class), fully built.
fn spawn_built(world: &mut BuildingWorld, db: &EditDb, kind: PieceKind, name: &str, cell: i32) -> PieceId {
    let b = db.class(&class(name)).unwrap().build.expect("build numbers");
    let id = world.spawn(PieceSpawn {
        kind, material: Material::Wood, class_path: class(name),
        key: GridKey { kind, cell: IVec3::new(cell, 0, 0), rot: 0 }, location: Vec3::ZERO, yaw_deg: 0.,
        max_health: b.max_health, start_health: b.max_health * b.initial_health_pct, build_time: b.build_time,
    });
    for _ in 0..200 { world.tick(0.1); }
    assert!(!world.get(id).unwrap().is_building());
    id
}

/// Fix 1: ABuildingSMActor::vfunc_247 @14047bed0 (InitReason 2): the edited piece rebuilds through the core build-up path.
#[test]
fn edit_rebuilds_new_actor() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn_built(&mut world, db, PieceKind::Wall, "PBWA_W1_Solid", 0);
    let door = db.class(&class("PBWA_W1_DoorC")).unwrap().build.unwrap();
    eprintln!("DoorC build numbers {door:?}");
    assert!(door.build_time > 0. && door.animates, "player pieces have a BuildTime and an AnimatingMaterial");
    // half health before the edit
    let max0 = world.get(id).unwrap().max_health;
    world.set_health(id, max0 * 0.5);
    let (_, cur) = world.events_since(0);
    let mut c = EditController::new();
    c.edit_begin(db, &world, Some(id), Vec3::ZERO).unwrap();
    c.edit_select_tile(4);
    c.edit_select_tile(7);
    assert!(c.edit_confirm(&mut world));
    let p = world.get(id).unwrap();
    assert!(p.is_building(), "the new actor builds up");
    let start = door.max_health * door.initial_health_pct * 0.5;
    assert!((p.health - start).abs() < 1e-3, "health {} != InitialHealth * old fraction {start}", p.health);
    assert!((p.build.health_to_build - (door.max_health * 0.5 - start)).abs() < 1e-3);
    // the Edited event comes after the build state was reset (the renderer sees is_building())
    let (ev, _) = world.events_since(cur);
    assert!(matches!(ev.last(), Some(WorldEvent::Edited(x)) if *x == id));
    // regains up to new max * max(old fraction, (old health + old pending) / old max) = half
    for _ in 0..200 { world.tick(0.1); }
    let p = world.get(id).unwrap();
    assert!(!p.is_building());
    assert!((p.health - door.max_health * 0.5).abs() <= 1.0, "health {} after the rebuild", p.health);
}

#[test]
fn rebuild_for_edit_formula() {
    use crate::world::EditRebuild;
    let mut w = BuildingWorld::new();
    let id = w.spawn(PieceSpawn { kind: PieceKind::Wall, material: Material::Wood, class_path: "A".into(), key: GridKey { kind: PieceKind::Wall, cell: IVec3::ZERO, rot: 0 }, location: Vec3::ZERO, yaw_deg: 0., max_health: 100., start_health: 10., build_time: 4. });
    w.tick(0.5); // health 10 + ceil(90 / 4 * 0.5) = 22, 78 still to build
    w.set_health(id, 12.); // damaged while building: 12 health, 78 pending
    let p = w.get(id).unwrap();
    assert_eq!((p.health, p.build.health_to_build), (12., 78.));
    assert!(w.rebuild_for_edit(id, EditRebuild { max_health: 200., initial_health_pct: 0.1, build_time: 2., animates: true }));
    let p = w.get(id).unwrap();
    // 200 * 0.1 * 0.12 = 2.4; to build 200 * max(0.12, (12 + 78) / 100) - 2.4 = 177.6
    assert!((p.health - 2.4).abs() < 1e-4 && (p.build.health_to_build - 177.6).abs() < 1e-3 && p.max_health == 200., "{:?} {}", p.build, p.health);
    // no animating material: built at full health
    assert!(w.rebuild_for_edit(id, EditRebuild { max_health: 150., initial_health_pct: 0.1, build_time: 2., animates: false }));
    assert!(!w.get(id).unwrap().is_building() && w.get(id).unwrap().health == 150.);
}

/// Fix 2: Stair::vfunc_74 @1404bbd00 -> vfunc_101 @1404c2a90 -> vfunc_90 @1404c5bb0: an invalid path release restores the last
/// valid tiles and re-applies the last valid match, valid again (+0x135 = 1), with the match flash.
#[test]
fn stair_invalid_release_reverts_valid() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Stairs, "PBWA_W1_StairW");
    let mut s = EditSession::begin(db, &world, id, Vec3::ZERO).unwrap();
    let before = s.preview.clone();
    s.drain_events();
    // a single step path matches nothing
    s.begin_action();
    assert!(s.update_hover(Some(EditTarget::Tile(2))));
    assert!(!s.state_valid);
    s.end_action();
    assert!(s.state_valid, "vfunc_90 sets the stored state valid again");
    assert_eq!(s.preview, before);
    assert_eq!(short(&s.state.class_path), "PBWA_W1_StairW");
    assert!(s.drain_events().contains(&EditEvent::MatchFlash));
}

/// Fix 3: FUN_14084f8a0 cancels the edit when the pawn dies; confirm ends the session (the edit tool is switched off).
#[test]
fn death_cancels_and_confirm_ends() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Wall, "PBWA_W1_Solid");
    let mut c = EditController::new();
    let far = Vec3::new(0., 5000., 5000.);
    c.edit_begin(db, &world, Some(id), Vec3::ZERO).unwrap();
    c.tick_frame(&world, Vec3::ZERO, false, far, Vec3::Z, 1. / 60.);
    assert!(c.is_editing());
    c.tick_frame(&world, Vec3::ZERO, true, far, Vec3::Z, 1. / 60.);
    assert!(!c.is_editing());
    assert!(c.drain_events().contains(&EditEvent::Ended { applied: false }));
    c.edit_begin(db, &world, Some(id), Vec3::ZERO).unwrap();
    assert!(!c.edit_confirm(&mut world));
    assert!(!c.is_editing(), "confirm leaves edit mode even without a change");
}

/// Fix 4a: vfunc_330 (attachment, pending structural operation) and vfunc_331 -> FUN_140816020 (input mode).
#[test]
fn edit_permission_blockers() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Wall, "PBWA_W1_Solid");
    let mut c = EditController::new();
    assert_eq!(c.edit_begin_checked(db, &world, Some(id), Vec3::ZERO, EditBlockers { has_attachment: true, structural_pending: false }), Err(BeginError::HasAttachment));
    assert_eq!(c.edit_begin_checked(db, &world, Some(id), Vec3::ZERO, EditBlockers { has_attachment: false, structural_pending: true }), Err(BeginError::StructuralPending));
    c.input_mode = EditInputMode { using_gamepad: true, edit_mode_usable_from_combat_mode: false, building_bar_focused: false };
    assert_eq!(c.edit_begin(db, &world, Some(id), Vec3::ZERO), Err(BeginError::InputMode));
    c.input_mode.building_bar_focused = true;
    assert_eq!(c.edit_begin(db, &world, Some(id), Vec3::ZERO), Ok(()));
    assert!(EditInputMode::default().allows_edit(), "keyboard / mouse always passes");
    assert!(EditInputMode { using_gamepad: true, edit_mode_usable_from_combat_mode: true, building_bar_focused: false }.allows_edit());
}

/// Fix 4b: Roof tiles ease to base + (0,0,32) while BTV_Active (Roof::vfunc_84 / vfunc_97 targets, vfunc_76 VInterpTo speed 4).
#[test]
fn roof_tiles_raise_eased() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Roof, "PBWA_W1_RoofC");
    let mut s = EditSession::begin(db, &world, id, Vec3::ZERO).unwrap();
    let l = db.layout(PieceKind::Roof);
    eprintln!("roof pattern {:?}", s.preview.tiles);
    // select tile 0 (inactive -> active)
    assert_eq!(s.preview.binary(0), Some(BTV_INACTIVE));
    s.click(EditTarget::Tile(0));
    let i = 0;
    assert_eq!(s.preview.binary(i), Some(BTV_ACTIVE));
    assert_eq!(s.visuals(&world).unwrap().tiles[i].local_position, l.tile_position(i), "starts at the base position");
    s.tick(0.1);
    let z = s.visuals(&world).unwrap().tiles[i].local_position.z - l.tile_position(i).z;
    assert!((z - 32. * 0.4).abs() < 1e-3, "one tick of VInterpTo(dt 0.1, speed 4): {z}");
    for _ in 0..100 { s.tick(0.1); }
    let v = s.visuals(&world).unwrap();
    assert_eq!(v.tiles[i].local_position, l.tile_position(i) + Vec3::new(0., 0., 32.));
    for t in &v.tiles { if !t.active { assert_eq!(t.local_position, l.tile_position(t.index)); } }
}

/// Fix 4c: the SuccessfulPatternMatchCurve flash (OnSuccessfulMatchInteractParamUpdate @1404bd9a0) and the hover parameter.
#[test]
fn match_flash_curve() {
    let Some(db) = db() else { return };
    let f = &db.match_flash;
    eprintln!("ValidBluePrintInteractValueFlash keys {:?} length {}", f.keys, f.length());
    assert!(f.length() > 0.);
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Wall, "PBWA_W1_Solid");
    let mut s = EditSession::begin(db, &world, id, Vec3::ZERO).unwrap();
    assert!(s.visuals(&world).unwrap().tiles.iter().all(|t| t.interact.is_none()), "track bound after vfunc_86: nothing written at enter");
    s.update_hover(Some(EditTarget::Tile(0)));
    s.tick(0.05);
    let v = s.visuals(&world).unwrap();
    assert_eq!(v.tiles[0].interact, Some(1.0), "hovered tile skipped by the flash");
    assert_eq!(v.tiles[1].interact, Some(f.eval(0.05)));
    for _ in 0..200 { s.tick(0.05); }
    let v = s.visuals(&world).unwrap();
    assert_eq!(v.tiles[1].interact, Some(f.eval(f.length())), "the flash stops on its last key");
    // a click changes the pattern: PatternChanged (LocalOnBlueprintEdit) and, on a match, the flash restarts
    s.drain_events();
    s.click(EditTarget::Tile(4));
    let ev = s.drain_events();
    assert!(ev.contains(&EditEvent::PatternChanged) && ev.contains(&EditEvent::MatchFlash));
}

/// Fix 5: stair indicators (Stair::vfunc_102 @1404c8d60 -> vfunc_103 @1404c3da0): modes, scalars and the arrow direction flag.
#[test]
fn stair_arrow_params() {
    let Some(db) = db() else { return };
    let mut world = BuildingWorld::new();
    let id = spawn(&mut world, PieceKind::Stairs, "PBWA_W1_StairW");
    let mut s = EditSession::begin(db, &world, id, Vec3::ZERO).unwrap();
    s.begin_action();
    s.update_hover(Some(EditTarget::Tile(0)));
    s.update_hover(Some(EditTarget::Tile(1)));
    let aux = s.visuals(&world).unwrap().aux;
    // 0 -> 1 is indicator 0 (path arrow, entry flag 0); from 1 the hint arrow to 3 is indicator 1 (entry [1][3] flag 0)
    assert_eq!((aux[0].mode, aux[0].reversed, aux[0].scalars), (1, false, [1., 0., 0.]));
    assert_eq!((aux[1].mode, aux[1].reversed, aux[1].scalars), (0, false, [0., 1., 0.]));
    assert_eq!((aux[2].mode, aux[2].scalars), (2, [0., 0., 1.]));
    assert_eq!(aux[0].local_yaw_deg, 0.);
    // the same path on a mirrored actor: entry [0][1] has the mirror byte set -> reversed, +180 yaw
    let mut w2 = BuildingWorld::new();
    let id2 = spawn(&mut w2, PieceKind::Stairs, "PBWA_W1_StairW");
    let mut st = w2.get(id2).unwrap().edit.clone();
    st.mirrored = true;
    w2.set_edit(id2, st);
    let mut m = EditSession::begin(db, &w2, id2, Vec3::ZERO).unwrap();
    m.begin_action();
    m.update_hover(Some(EditTarget::Tile(0)));
    m.update_hover(Some(EditTarget::Tile(1)));
    let aux = m.visuals(&w2).unwrap().aux;
    assert_eq!((aux[0].mode, aux[0].reversed, aux[0].local_yaw_deg), (1, true, 180.));
    // 1 -> 0 (indicator 0, entry flag 1) on the unmirrored actor
    s.end_action();
    s.begin_action();
    s.update_hover(Some(EditTarget::Tile(1)));
    s.update_hover(Some(EditTarget::Tile(0)));
    let aux = s.visuals(&world).unwrap().aux;
    assert_eq!((aux[0].mode, aux[0].reversed, aux[0].local_yaw_deg), (1, true, 180.));
}

fn permutations(v: &[usize]) -> Vec<Vec<usize>> {
    if v.len() <= 1 { return vec![v.to_vec()]; }
    let mut out = Vec::new();
    for i in 0..v.len() {
        let mut rest = v.to_vec();
        let x = rest.remove(i);
        for mut p in permutations(&rest) { p.insert(0, x); out.push(p); }
    }
    out
}

/// Writes `verification/building/edit/edit_tiles.png` (tile grids at their exe positions with real selections) and
/// `edit_table.txt` (every selection -> class) when `FN_EDIT_VERIFY_OUT` names the output directory.
#[test]
fn verification_diagram() {
    let (Some(db), Some(out)) = (db(), fn_live::var_os("FN_EDIT_VERIFY_OUT")) else { return };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    // Table.
    let mut txt = String::from("Fortnite 1.8 edit selections (wood L1, fresh unmirrored piece; '#' = tile kept, '.' = tile removed; rows top->bottom for walls, -X..+X columns / +Y rows for floors)\n");
    for (kind, base, n) in [(PieceKind::Wall, "PBWA_W1_Solid", 3), (PieceKind::Floor, "PBWA_W1_Floor", 2), (PieceKind::Roof, "PBWA_W1_RoofC", 2)] {
        txt += &format!("\n== {kind:?}\n");
        for (tiles, m) in table(db, kind, base) {
            match m {
                Some((c, r, mi)) => txt += &format!("{} -> {c} rotation {r} mirrored {mi}\n", fmt_tiles(&tiles, n)),
                None => txt += &format!("{} -> invalid\n", fmt_tiles(&tiles, n)),
            }
        }
    }
    std::fs::write(out.join("edit_table.txt"), txt).unwrap();

    // Panels from real sessions.
    let mut panels: Vec<(EditVisuals, PieceKind)> = Vec::new();
    let mut world = BuildingWorld::new();
    let mut add = |world: &mut BuildingWorld, kind: PieceKind, base: &str, f: &dyn Fn(&mut EditSession)| {
        let id = spawn(world, kind, base);
        let mut s = EditSession::begin(db, world, id, Vec3::ZERO).unwrap();
        f(&mut s);
        panels.push((s.visuals(world).unwrap(), kind));
        world.remove(id);
    };
    add(&mut world, PieceKind::Wall, "PBWA_W1_Solid", &|s| { s.begin_action(); s.update_hover(Some(EditTarget::Tile(4))); s.update_hover(Some(EditTarget::Tile(7))); s.end_action(); s.update_hover(Some(EditTarget::Tile(0))); });
    add(&mut world, PieceKind::Wall, "PBWA_W1_Solid", &|s| { for i in 0..9 { s.click(EditTarget::Tile(i)); } });
    add(&mut world, PieceKind::Floor, "PBWA_W1_Floor", &|s| { s.click(EditTarget::Tile(1)); s.click(EditTarget::Tile(2)); });
    add(&mut world, PieceKind::Roof, "PBWA_W1_RoofC", &|s| { s.click(EditTarget::Tile(0)); });
    add(&mut world, PieceKind::Stairs, "PBWA_W1_StairW", &|s| { s.begin_action(); for t in [2, 0, 1] { s.update_hover(Some(EditTarget::Tile(t))); } });
    add(&mut world, PieceKind::Stairs, "PBWA_W1_StairW", &|s| { s.reset(); });

    const P: u32 = 300;
    let mut img = image::RgbImage::from_pixel(P * 3, P * 2, image::Rgb([24, 24, 28]));
    let srgb = |c: f32| (c.clamp(0., 1.).powf(1. / 2.2) * 255.) as u8;
    for (pi, (v, kind)) in panels.iter().enumerate() {
        let (ox, oy) = ((pi as u32 % 3) * P, (pi as u32 / 3) * P);
        let s = 0.5f32;
        // (u, v) plane of the piece: walls x/z, the rest x/y. Piece outline in local cm.
        let (uv, rect): (Box<dyn Fn(Vec3) -> (f32, f32)>, [f32; 4]) = if *kind == PieceKind::Wall {
            (Box::new(|p: Vec3| (p.x, p.z)), [-256., 0., 256., 384.])
        } else {
            (Box::new(|p: Vec3| (p.x, p.y)), [-256., 0., 256., 512.])
        };
        let to_px = |u: f32, w: f32| -> (i32, i32) { (ox as i32 + ((u + 280.) * s) as i32, oy as i32 + P as i32 - 10 - ((w + 10.) * s) as i32) };
        let fill = |img: &mut image::RgbImage, a: (i32, i32), b: (i32, i32), c: [u8; 3], outline_only: bool| {
            let (x0, x1) = (a.0.min(b.0), a.0.max(b.0));
            let (y0, y1) = (a.1.min(b.1), a.1.max(b.1));
            for y in y0..=y1 { for x in x0..=x1 {
                if x < ox as i32 || y < oy as i32 || x >= (ox + P) as i32 || y >= (oy + P) as i32 { continue; }
                if outline_only && x != x0 && x != x1 && y != y0 && y != y1 { continue; }
                img.put_pixel(x as u32, y as u32, image::Rgb(c));
            } }
        };
        fill(&mut img, to_px(rect[0], rect[1]), to_px(rect[2], rect[3]), [90, 90, 90], true);
        let l = db.layout(*kind);
        let e = l.mesh_extent * l.scale;
        let (eu, ev) = if *kind == PieceKind::Wall { (e.x, e.z) } else { (e.x, e.y) };
        for t in &v.tiles {
            let (cu, cv) = uv(t.local_position);
            let col = if t.active { [srgb(t.color[0] * 2.5), srgb(t.color[1] * 2.5), srgb(t.color[2] * 2.5)] } else { [55, 55, 60] };
            fill(&mut img, to_px(cu - eu, cv - ev), to_px(cu + eu, cv + ev), col, false);
            if !t.active {
                // the BlueprintTestX "X"
                let (a, b) = (to_px(cu - eu, cv - ev), to_px(cu + eu, cv + ev));
                let n = (b.0 - a.0).abs().max(1);
                for k in 0..=n {
                    let y0 = a.1 + (b.1 - a.1) * k / n;
                    let y1 = b.1 + (a.1 - b.1) * k / n;
                    for (x, y) in [(a.0 + k, y0), (a.0 + k, y1)] {
                        if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() { img.put_pixel(x as u32, y as u32, image::Rgb([200, 200, 200])); }
                    }
                }
            }
            if t.step > 0 {
                // stair step: that many white dots
                let (px, py) = to_px(cu, cv);
                for k in 0..t.step { fill(&mut img, (px - 12 + k * 8, py - 2), (px - 9 + k * 8, py + 1), [255, 255, 255], false); }
            }
            if t.highlighted { fill(&mut img, to_px(cu - eu, cv - ev), to_px(cu + eu, cv + ev), [255, 255, 255], true); }
        }
        let (amo, ame) = db.stair_aux_bounds;
        for a in &v.aux {
            let c = a.local_position + glam::Quat::from_rotation_z(a.local_yaw_deg.to_radians()) * amo;
            let e = if a.index % 2 == 1 { Vec3::new(ame.y, ame.x, ame.z) } else { ame };
            let col = match a.mode { 0 => [230, 200, 60], 1 => [80, 220, 120], 2 => [110, 110, 140], _ => [200, 90, 200] };
            fill(&mut img, to_px(c.x - e.x, c.y - e.y), to_px(c.x + e.x, c.y + e.y), col, true);
        }
        eprintln!("panel {pi}: {kind:?} preview {:?} visible {:?}", v.preview.as_ref().map(|p| short(&p.class_path).to_string()), v.preview.as_ref().map(|p| p.visible));
    }
    img.save(out.join("edit_tiles.png")).unwrap();
}
