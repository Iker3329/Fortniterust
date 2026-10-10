//! Headless per-frame probe of the pickaxe swing through the real `GameSim::step` path (montage name, time, weight per tick).
//! Run with `cargo test -p fn-game --test swing_probe -- --nocapture` to print the table.
use fn_combat::{Harvestable, Kind};
use fn_game::{FrameInput, GameSim};
use glam::Vec3;

const DT: f32 = 1. / 60.;

fn sim_with_tree() -> GameSim {
    let mut g = GameSim::new();
    g.to_spawn();
    g.world.props = vec![Harvestable::new(Kind::Tree, Vec3::new(0., 0., 12. - 1.8))];
    for _ in 0..30 { g.step(&FrameInput::default(), DT); } // settle on the ground
    g
}

/// (frame time, stage, clip of the UpperBody track, montage time, weight, hits so far)
fn run(g: &mut GameSim, attack: impl Fn(f32) -> bool, secs: f32, print: bool) -> Vec<(f32, u8, String, f32, f32, u32)> {
    let mut rows = vec![];
    let n = (secs / DT) as usize;
    for i in 0..n {
        let t = i as f32 * DT;
        g.step(&FrameInput { attack: attack(t), ..Default::default() }, DT);
        let m = g.montages();
        let ub = m.iter().find(|x| x.slot == "UpperBody" && x.clip.starts_with("TwoHanded"));
        let row = (t, g.swing.stage, ub.map(|x| x.clip.clone()).unwrap_or_default(), ub.map_or(0., |x| x.time), ub.map_or(0., |x| x.weight), g.world.hits);
        if print { eprintln!("t={:.3} stage={} clip={:<38} mt={:.3} w={:.2} hits={} swing_flag={}", row.0, row.1, row.2, row.3, row.4, row.5, g.swing.stage > 0); }
        rows.push(row);
    }
    rows
}

#[test]
fn a_single_tap_on_a_tree_plays_swing1_then_end1_with_the_hit_at_the_notify_and_never_the_bounce() {
    let mut g = sim_with_tree();
    let rows = run(&mut g, |t| t < 0.03, 1.4, true);
    assert!(rows.iter().all(|r| r.1 != 5 && r.1 != 6), "Bounce (ineffective-hit reaction) must not play on a damaging harvest hit");
    // Swing1 plays the whole 0.5333 s section
    let s1: Vec<_> = rows.iter().filter(|r| r.1 == 1).collect();
    assert!(s1.iter().all(|r| r.2 == "TwoHanded_Loop_Swing1_InMotion" || r.2 == "TwoHanded_Loop_Swing1"), "{:?}", s1.first());
    assert!(s1.last().unwrap().3 > 0.5, "Swing1 ran to {:.3}", s1.last().unwrap().3);
    // then End1 (Swing1_2Idle) for its 0.5333 s
    let e1: Vec<_> = rows.iter().filter(|r| r.1 == 3).collect();
    assert!(e1.len() as f32 * DT > 0.5, "End1 only {} frames", e1.len());
    assert!(e1[0].2.contains("Swing1_2Idle"));
    // exactly one hit, on the first frame at/after TriggerAbility 0.1603 s
    let first_hit = rows.iter().find(|r| r.5 == 1).unwrap();
    assert!(first_hit.0 >= fn_combat::TRIGGER_ABILITY[0] - 1.5 * DT && first_hit.0 <= fn_combat::TRIGGER_ABILITY[0] + 2. * DT, "hit at {:.4}", first_hit.0);
    assert_eq!(rows.last().unwrap().5, 1);
    assert_eq!(rows.last().unwrap().1, 0, "back to idle after End1");
}

#[test]
fn a_held_attack_alternates_swing1_swing2_and_hits_every_notify() {
    let mut g = sim_with_tree();
    let rows = run(&mut g, |_| true, 3., false);
    assert!(rows.iter().all(|r| r.1 != 5 && r.1 != 6));
    let stages: Vec<u8> = rows.iter().map(|r| r.1).fold(vec![], |mut v, s| { if v.last() != Some(&s) { v.push(s); } v });
    assert_eq!(&stages[..4], &[1, 2, 1, 2], "{stages:?}");
    // a hit per swing: 3 s hold = Swing1 0.5333 + Swing2 0.6333 repeated -> hits at 0.16, 0.73, 1.33, 1.90, 2.50
    let hits = rows.last().unwrap().5;
    assert!((4..=6).contains(&hits), "hits {hits}");
}

#[test]
fn a_tap_after_the_decision_window_opens_continues_into_swing2() {
    let mut g = sim_with_tree();
    // press, release, press again inside the Swing1 DecisionWindow (0.1285..0.5332)
    let rows = run(&mut g, |t| t < 0.03 || (0.3..0.33).contains(&t), 1.5, false);
    let stages: Vec<u8> = rows.iter().map(|r| r.1).fold(vec![], |mut v, s| { if v.last() != Some(&s) { v.push(s); } v });
    assert_eq!(&stages[..3], &[1, 2, 4], "{stages:?}");
}
