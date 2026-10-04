//! Checks against the recovered-data report `reports/verify-1.8.json` (written by fn-inspect from the 1.8 paks): every constant the Rust crates expose is
//! re-evaluated and compared with the value recovered from the 1.8 data (rows the old code already matched) or with the old code's assumption (rows the
//! data cannot pin down). Constants no Rust crate exposes yet are counted as "not exposed", never as passing.
use crate::config::*;
use crate::exec::Ctx;
use crate::model::*;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct ConstRow { pub group: String, pub constant: String, pub report_status: String, pub verdict: String, pub new: Option<Vec<f64>>, pub expected: Vec<f64>, pub note: String }

pub fn flatten(v: &Value, out: &mut Vec<f64>) {
    match v {
        Value::Number(n) => out.push(n.as_f64().unwrap_or(f64::NAN)),
        Value::Bool(b) => out.push(*b as u8 as f64),
        Value::Array(a) => a.iter().for_each(|x| flatten(x, out)),
        Value::Object(o) => { if let Some(x) = o.get("value") { flatten(x, out); } }
        _ => {}
    }
}
fn flat(v: &Value) -> Vec<f64> { let mut o = vec![]; flatten(v, &mut o); o }

pub fn approx(a: &[f64], b: &[f64], rel: f64, abs: f64) -> bool { a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= abs + rel * x.abs().max(y.abs())) }

#[cfg(feature = "crate-consts")]
pub fn rust_value(group: &str, constant: &str) -> Option<Vec<f64>> {
    use fn_camera as cam;
    use fn_movement::params as p;
    let key = constant.split(|c: char| c == ' ' || c == '(').next().unwrap_or(constant);
    trait ToF { fn f(self) -> f64; }
    impl ToF for f32 { fn f(self) -> f64 { self as f64 } }
    impl ToF for u32 { fn f(self) -> f64 { self as f64 } }
    impl ToF for bool { fn f(self) -> f64 { self as u8 as f64 } }
    macro_rules! mv { ($($n:ident),* $(,)?) => { match key { $( stringify!($n) => return Some(vec![ToF::f(p::$n)]), )* _ => {} } } }
    match group {
        "movement" | "pawn" => {
            mv!(SPEED_WALK, SPEED_RUN, SPEED_SPRINT, SPEED_CROUCHED_RUN, SPEED_CROUCHED_SPRINT, SPEED_BACKWARDS_MULTIPLIER, SPEED_DBNO, ACCELERATION_STRAFE_MULTIPLIER_SPRINT,
                MAX_JUMP_TIME_STAT, MAX_ACCELERATION, GROUND_FRICTION, BRAKING_DECELERATION_WALKING, JUMP_Z_VELOCITY, AIR_CONTROL, PERCH_RADIUS_THRESHOLD, CAPSULE_HALF_HEIGHT,
                KILL_Z, GRAVITY_SCALE, MAX_STEP_HEIGHT, CROUCHED_HALF_HEIGHT, PERCH_ADDITIONAL_HEIGHT, BRAKING_FRICTION_FACTOR, AIR_CONTROL_BOOST_MULTIPLIER,
                AIR_CONTROL_BOOST_VELOCITY_THRESHOLD, LEDGE_CHECK_THRESHOLD, MAX_SIMULATION_TIME_STEP, MAX_SIMULATION_ITERATIONS, MAX_DEPENETRATION_WITH_GEOMETRY,
                MIN_ANALOG_WALK_SPEED, CAPSULE_RADIUS, BRAKING_DECELERATION_FALLING, FALLING_LATERAL_FRICTION, JUMP_MAX_HOLD_TIME, JUMP_MAX_COUNT, WALKABLE_FLOOR_Z,
                BRAKING_SUB_STEP_TIME, USE_SEPARATE_BRAKING_FRICTION, BRAKING_FRICTION, TERMINAL_VELOCITY, GRAVITY_Z_SCALE_ATTRIBUTE, APPLY_GRAVITY_WHILE_JUMPING, CAN_WALK_OFF_LEDGES);
            match key {
                "WORLD_GRAVITY_Z" => Some(vec![p::WORLD_GRAVITY_Z as f64]),
                "DIRECTIONAL_SPEED_CURVE" if constant.contains("keys") => Some(p::DIRECTIONAL_SPEED_CURVE.iter().flat_map(|(a, b)| [*a as f64, *b as f64]).collect()),
                _ => None,
            }
        }
        "combat" => {
            use fn_combat as c;
            let idx = |k: &str| -> Option<usize> { k.split('[').nth(1)?.split(']').next()?.parse().ok() };
            match key.split('[').next().unwrap_or(key) {
                "TRIGGER_ABILITY" => c::TRIGGER_ABILITY.get(idx(key)?).map(|&v| vec![v as f64]),
                "BOUNCE_LENGTH" => c::BOUNCE_LENGTH.get(idx(key)?).map(|&v| vec![v as f64]),
                "DECISION_WINDOW" => { let w = c::DECISION_WINDOW.get(idx(key)?)?; if key.ends_with(".start") { Some(vec![w.0 as f64]) } else if key.ends_with(".end") { Some(vec![w.1 as f64]) } else { None } }
                "MELEE_REACH" => Some(vec![c::MELEE_REACH as f64]), "MELEE_RADIUS" => Some(vec![c::MELEE_RADIUS as f64]), "HARVEST_DAMAGE" => Some(vec![c::HARVEST_DAMAGE as f64]),
                "TREE_HEALTH" => Some(vec![c::TREE_HEALTH as f64]), "ROCK_HEALTH" => Some(vec![c::ROCK_HEALTH as f64]),
                _ => None,
            }
        }
        "camera" => {
            if key == "STAND_HALF_HEIGHT" { return Some(vec![cam::STAND_HALF_HEIGHT as f64]); }
            let (mode, field) = if let Some(f) = key.strip_prefix("BASE.") { (&cam::BASE, f) } else if let Some(f) = key.strip_prefix("MELEE.") { (&cam::MELEE, f) } else if key == "FEELERS" {
                return Some(cam::FEELERS.iter().flat_map(|f| [f.adjust_pitch, f.adjust_yaw, f.world_weight, f.pawn_weight, f.extent, f.trace_interval as f32].map(|x| x as f64)).collect());
            } else { return None };
            let samples = |s: &[cam::Sample]| -> Vec<f64> { s.iter().flat_map(|x| [x.pitch as f64, x.offset[0] as f64, x.offset[1] as f64, x.offset[2] as f64]).collect() };
            Some(match field {
                "fov" => vec![mode.fov as f64], "forward_distance" => vec![mode.forward_distance as f64],
                "origin_interp" => mode.origin_interp.iter().map(|&x| x as f64).collect(), "vt_interp" => mode.vt_interp.iter().map(|&x| x as f64).collect(),
                "push_out" => vec![mode.push_out as f64], "hide_pawn_pct" => vec![mode.hide_pawn_pct as f64], "transition_time" => vec![mode.transition_time as f64],
                "vt_offset" => samples(mode.vt_offset), "vt_offset_crouch" => samples(mode.vt_offset_crouch), "cs_offset" => samples(mode.cs_offset),
                _ => return None,
            })
        }
        _ => None,
    }
}
#[cfg(not(feature = "crate-consts"))]
pub fn rust_value(_group: &str, _constant: &str) -> Option<Vec<f64>> { None }

pub fn group_area(g: &str) -> Area {
    match g {
        "movement" | "pawn" => Area::Movement,
        "camera" => Area::Camera,
        "anim" | "anim-refs" | "combat" => Area::Animation,
        "lighting" | "post" | "fog" | "sky" | "sky-params" | "shadow" => Area::Lighting,
        "hud" | "vitals" | "quickbar" => Area::Hud,
        "settings-ui" | "esc-menu" | "input" => Area::Menus,
        "assets" => Area::Loading,
        _ => Area::Animation,
    }
}

pub fn evaluate(rows: &Value, rel: f64, abs: f64) -> Vec<ConstRow> {
    let mut out = vec![];
    for r in rows.as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let (group, constant, status) = (r["group"].as_str().unwrap_or("").to_owned(), r["constant"].as_str().unwrap_or("").to_owned(), r["status"].as_str().unwrap_or("").to_owned());
        let (code, data) = (flat(&r["valueInCode"]), flat(&r["value1_8"]));
        let new = rust_value(&group, &constant);
        let (verdict, expected) = match &new {
            None => ("not-exposed".to_owned(), if status == "match" { data.clone() } else { code.clone() }),
            Some(n) if status == "match" => (if approx(n, &data, rel, abs) { "ok" } else { "DRIFT" }.to_owned(), data.clone()),
            Some(n) => (if approx(n, &code, rel, abs) { "same-assumption" } else { "assumption-changed" }.to_owned(), code.clone()),
        };
        out.push(ConstRow { group, constant, report_status: status, verdict, new, expected, note: r["note"].as_str().unwrap_or("").to_owned() });
    }
    out
}

pub fn const_items(ctx: &Ctx) -> (Vec<Item>, Vec<ConstRow>) {
    let path = ctx.o.old_tree.join("reports").join("verify-1.8.json");
    let Some(text) = std::fs::read_to_string(&path).ok() else { return (vec![Item::new("const:all", Area::Movement, "data", "recovered constants (verify-1.8.json)").na(format!("{} not found", path.display()))], vec![]) };
    let Ok(v) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return (vec![Item::new("const:all", Area::Movement, "data", "recovered constants (verify-1.8.json)").na("report does not parse")], vec![]) };
    let rows = evaluate(&v["rows"], ctx.cfg.f("data.rel_tol", 1e-4), ctx.cfg.f("data.abs_tol", 1e-6));
    let mut by: BTreeMap<String, Vec<&ConstRow>> = BTreeMap::new();
    for r in &rows { by.entry(r.group.clone()).or_default().push(r); }
    let mut items = vec![];
    for (g, rs) in &by {
        let cnt = |v: &str| rs.iter().filter(|r| r.verdict == v).count();
        let (ok, drift, same, changed, unexp) = (cnt("ok"), cnt("DRIFT"), cnt("same-assumption"), cnt("assumption-changed"), cnt("not-exposed"));
        let report_match = rs.iter().filter(|r| r.report_status == "match").count();
        let mut it = Item::new(format!("const:{g}"), group_area(g), "data", format!("recovered 1.8 constants: {g}"));
        if ok + drift == 0 {
            it = it.na(format!("no Rust crate exposes the {} verified constants of this group yet ({} rows)", report_match, rs.len()));
            it.headline = format!("{} rows, {} verified against the 1.8 data in the old code, 0 re-evaluated in Rust", rs.len(), report_match);
        } else {
            let s = score_lower(drift as f64, (0., 1.));
            it.headline = format!("{ok} ok, {drift} drift of {} re-evaluated ({unexp} not exposed)", ok + drift);
            let bad: Vec<String> = rs.iter().filter(|r| r.verdict == "DRIFT").map(|r| r.constant.clone()).collect();
            if !bad.is_empty() { it.note = format!("drift: {}", bad.join("; ")); }
            it = it.scored(s);
        }
        let ch: Vec<String> = rs.iter().filter(|r| r.verdict == "assumption-changed").map(|r| r.constant.clone()).collect();
        if !ch.is_empty() { it.note = format!("{} assumption changed vs the old code: {}", it.note, ch.join("; ")).trim().to_owned(); }
        it = it.metric("ok", ok.to_string(), Grade::Na).metric("drift", drift.to_string(), if drift == 0 { Grade::Green } else { Grade::Red }).metric("assumptions kept", same.to_string(), Grade::Na).metric("assumptions changed", changed.to_string(), if changed == 0 { Grade::Green } else { Grade::Amber }).metric("not exposed", unexp.to_string(), Grade::Na);
        items.push(it);
    }
    (items, rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn flatten_follows_the_report_shapes() {
        let v: Value = serde_json::from_str(r#"[{"value":[0,[0,45,75]],"Count":2},{"value":[90,[-10,0,25]],"Count":2}]"#).unwrap();
        assert_eq!(flat(&v), vec![0., 0., 45., 75., 90., -10., 0., 25.]);
        assert_eq!(flat(&serde_json::json!(true)), vec![1.]);
    }
    #[test] fn approx_uses_relative_tolerance() { assert!(approx(&[0.65], &[0.6499999761581421], 1e-4, 1e-6)); assert!(!approx(&[200.], &[201.], 1e-4, 1e-6)); assert!(!approx(&[1., 2.], &[1.], 1e-4, 1e-6)); }
    #[cfg(feature = "crate-consts")]
    #[test] fn movement_and_camera_constants_are_exposed() {
        assert_eq!(rust_value("movement", "SPEED_WALK"), Some(vec![200.]));
        assert_eq!(rust_value("camera", "BASE.fov"), Some(vec![80.]));
        assert_eq!(rust_value("camera", "FEELERS (5 x pitch,yaw,worldW,pawnW,extent,interval)").map(|v| v.len()), Some(30));
        assert_eq!(rust_value("hud", "anything"), None);
    }
    #[test] fn verdicts() {
        let rows = serde_json::json!([{"group":"nope","constant":"X","status":"match","valueInCode":1,"value1_8":1},{"group":"nope","constant":"Y","status":"not-found","valueInCode":2}]);
        let r = evaluate(&rows, 1e-4, 1e-6); assert!(r.iter().all(|x| x.verdict == "not-exposed"));
    }
}
