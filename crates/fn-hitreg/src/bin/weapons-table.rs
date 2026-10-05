//! Research tool: prints the shooting-relevant stat row + WID fields of every BR (Athena) ranged weapon in the install. `weapons-table <install> [csv]`
use fn_pose::{loader, loadout_stats::data_table_row, ClipSourceDir};
use fn_ueio::props::{read_props, Ctx, PropValue};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let paks = fn_pak::PakSet::open_install(&a[1]).unwrap();
    let src = ClipSourceDir::Pak(&paks);
    let mut wids: Vec<String> = paks.glob("*WID_*Athena*.uasset").into_iter().map(|(_, e)| e.rel_path().to_owned()).collect();
    wids.sort();
    for rel in wids {
        let long = format!("/Game/{}", rel.trim_start_matches("FortniteGame/Content/").trim_end_matches(".uasset"));
        if long.contains("Harvest") { continue; }
        let Ok(pkg) = loader::package(&src, &long) else { continue };
        let (s, e) = pkg.export_range(0);
        let Ok(wid) = read_props(&mut fn_ueio::Reader::at(&pkg.data[..e], s), &Ctx { names: &pkg.names, file_version: pkg.summary.file_version }) else { continue };
        let Some(h) = wid.fields("WeaponStatHandle") else { println!("{long}: no stat handle"); continue };
        let table = h.object("DataTable").filter(|i| *i < 0).map(|i| pkg.import_path(i)).unwrap_or_default();
        let row = h.text("RowName").unwrap_or("").to_owned();
        let r = match data_table_row(&src, &table, &row) { Ok(r) => r, Err(er) => { println!("{long}: {er}"); continue } };
        let g = |k: &str| r.get(k).and_then(PropValue::as_f64).unwrap_or(f64::NAN);
        let trig = wid.text("TriggerType").unwrap_or("?").to_owned();
        let name = long.rsplit('/').next().unwrap();
        let perfect = wid.bool("bShouldUsePerfectAimWhenTargetingMinSpread").unwrap_or(false);
        println!("{name} | row {row} | trig {trig} | perfectAimMinSpread {perfect} | ammo {} | proj {} | fireAbility {}", wid.text("AmmoData").unwrap_or("?").rsplit('/').next().unwrap(), wid.text("ProjectileTemplate").map_or("-".into(), |p| p.rsplit('/').next().unwrap().to_owned()), wid.text("PrimaryFireAbility").unwrap_or("?").rsplit('/').next().unwrap());
        if a.get(2).map(String::as_str) == Some("all") { for pr in &r.0 { println!("    {}: {:?}", pr.name, pr.value); } continue; }
        let cols = ["Spread","SpreadDownsights","StandingStillSpreadMultiplier","AthenaCrouchingSpreadMultiplier","AthenaJumpingFallingSpreadMultiplier","AthenaSprintingSpreadMultiplier","MinSpeedForSpreadMultiplier","MaxSpeedForSpreadMultiplier","SpreadDownsightsAdditionalCooldownTime","HeatX1","HeatY1","HeatX2","HeatY2","HeatX3","HeatY3","HeatXScale","HeatYScale","CoolX1","CoolY1","CoolX2","CoolY2","CoolX3","CoolY3","CoolXScale","CoolYScale","BulletsPerCartridge","FiringRate","ROFScale","BurstFiringRate","DmgPB","DmgMid","DmgLong","DmgMaxRange","EnvDmgPB","EnvDmgMid","EnvDmgLong","EnvDmgMaxRange","RngPB","RngMid","RngLong","RngMax","DamageZone_Critical","ReloadTime","ClipSize","CartridgePerFire","AmmoCostPerFire","MinChargeTime","MaxChargeTime","SweepRadius","MaxActualRange","bReloadIndividualBullets","DmgScale","EnvDmgScale"];
        let mut line = String::from("    ");
        for c in cols { let v = r.get(c).map_or("?".into(), |v| match v { PropValue::Bool(b) => format!("{b}"), o => o.as_f64().map_or(format!("{o:?}"), |x| format!("{}", (x * 1e4).round() / 1e4)) }); line += &format!("{c}={v} "); }
        println!("{line}");
        let _ = g("Spread");
    }
}
