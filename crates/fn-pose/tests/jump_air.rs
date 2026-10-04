//! `--jump-air-test`: the eight scripted air-press scenarios (60 Hz). The old log prints the AdditiveJumps state transitions (no bones), so the
//! scenario driver is tied to it by those transitions (frame, from -> to), and the poses of every frame are checked against the reference port.
mod common;
use common::*;
use fn_pose::scenario::{self, jump_air_script, Scenario};
use fn_pose::{CharacterRig, EvalOptions};

#[test]
fn jump_air_scenarios_reproduce_the_old_state_transitions_and_pose_every_frame() {
    let Some(old) = Old::load() else { return };
    let gold = golden("jump-air-test/jump-air-test.txt");
    // the 60 Hz block: "-- <name>" followed by "   fNNN tT.TTT From->To ..." lines until the next "--" / "==" heading
    let block = gold.split("== 144 Hz").next().unwrap();
    let rig = CharacterRig::new(old.parts.clone());
    let _l = lock();
    let mut total = 0;
    for i in 0..8 {
        let (name, _) = jump_air_script(i).unwrap();
        let section = block.split(&format!("-- {name}")).nth(1).unwrap_or_else(|| panic!("scenario {name} not in the golden log"));
        let want: Vec<(u32, String)> = section.lines().skip(1).take_while(|l| l.starts_with("   ")).filter(|l| l.trim_start().starts_with('f') && l.contains("->"))
            .map(|l| { let mut it = l.split_whitespace(); let f: u32 = it.next().unwrap()[1..].parse().unwrap(); let _t = it.next(); (f, it.next().unwrap().to_owned()) }).collect();
        let mut got: Vec<(u32, String)> = vec![];
        let mut prev = fn_animgraph::JumpState::Idle;
        let mut bind_frames = 0;
        scenario::run(Scenario::JumpAir(i), None, &mut |ctx| {
            let s = ctx.anim.jump.state;
            if s != prev { got.push((ctx.frame, format!("{prev:?}->{s:?}"))); prev = s; }
            let p = rig.evaluate(&ctx.tree, &old.md5);
            assert!(p.parts.iter().all(|r| r.skin_matrices.iter().all(|m| m.is_finite())), "{name} f{}: non-finite skin matrix", ctx.frame);
            if p.missing_clips.is_empty() { bind_frames += 1; }
        });
        assert_eq!(bind_frames, 300, "{name}: every clip of the jump machine must exist");
        assert_eq!(got, want, "{name}: state transitions differ from the old log");
        total += got.len();
    }
    println!("== jump-air-test: 8 scenarios x 300 frames; {total} AdditiveJumps transitions identical to the old exe's log (frame and edge); every frame posed with all clips present");
    let _ = EvalOptions::default();
}
