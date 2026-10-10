use fn_pak::PakSet;
use fn_uasset::PackageData;
use glam::Vec3;
use serde_json::Value;

pub const TRAP_DIR: &str = "FortniteGame/Content/Athena/Items/Traps/*.uasset";
pub const DURABILITY_TABLE: &str = "/Game/Items/Datatables/WeaponDurabilityRarity";
pub const DELAY_BEFORE_DESTROY: f32 = 1.5;
pub const DEFAULT_TIMER_FALLBACK: f32 = 0.0;
pub const MAX_PLACEMENT_DISTANCE: f32 = 512.;
pub const ROTATION_ANGLE_INCREMENT: f32 = 90.;
pub const PLACEMENT_FX_DISTANCE: f32 = 2400.;
pub const PREVIEW_MATERIAL: &str = "/Game/Packages/Fortnite_UniqueMaterials_1/EdgeFinding_INST.EdgeFinding_INST";
pub const PREVIEW_PARAM: &str = "CanBePlaced";
pub const LAUNCHED_TAG_TIME: f32 = 0.75;
pub const FLOOR_TRAP_IMMUNE_TIME: f32 = 0.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Attach { Floor, Wall, Ceiling }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Behavior { Damage, Launch { strength: f32, pitch: Option<f32> } }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sounds { pub placed: Option<String>, pub active: Option<String>, pub fire: Option<String>, pub reload: Option<String>, pub explode: Option<String> }

#[derive(Clone, Debug, PartialEq)]
pub struct Emitter { pub system: String, pub location: Vec3, pub rotation: [f32; 3], pub socket: Option<String> }

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fx { pub idle: Vec<Emitter>, pub fire: Vec<Emitter>, pub destruction: Option<String>, pub placed_dust: Option<String> }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Curve { pub keys: [(f32, f32, f32, f32, u8); 4], pub n: usize }
impl Curve {
    pub fn from_json(v: &Value) -> Option<Curve> {
        let ks = v.get("FloatCurve")?.get("Keys")?.as_array()?;
        let mut keys = [(0., 0., 0., 0., 0u8); 4];
        let n = ks.len().min(4);
        for (i, k) in ks.iter().take(4).enumerate() {
            let g = |s: &str| k.get(s).and_then(|x| x.as_f64()).unwrap_or(0.) as f32;
            keys[i] = (g("Time"), g("Value"), g("ArriveTangent"), g("LeaveTangent"), k.get("InterpMode").and_then(|x| x.as_u64()).unwrap_or(0) as u8);
        }
        (n > 0).then_some(Curve { keys, n })
    }
    pub fn eval(&self, t: f32) -> f32 {
        let k = &self.keys[..self.n];
        if t <= k[0].0 { return k[0].1; }
        if t >= k[self.n - 1].0 { return k[self.n - 1].1; }
        for w in k.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t < a.0 || t > b.0 { continue; }
            let dt = b.0 - a.0;
            if dt <= 0. { return b.1; }
            let s = (t - a.0) / dt;
            return match a.4 {
                1 => a.1,
                2 => { let (s2, s3) = (s * s, s * s * s); (2. * s3 - 3. * s2 + 1.) * a.1 + (s3 - 2. * s2 + s) * a.3 * dt + (-2. * s3 + 3. * s2) * b.1 + (s3 - s2) * b.2 * dt }
                _ => a.1 + (b.1 - a.1) * s,
            };
        }
        k[self.n - 1].1
    }
    pub fn length(&self) -> f32 { self.keys[self.n - 1].0 }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spike { pub mesh: String, pub materials: Vec<String>, pub resting: f32, pub active: f32, pub hold: f32, pub fire: Curve, pub fire_len: f32, pub retract: Curve, pub retract_len: f32, pub retract_sound_at: f32, pub reset: Curve, pub reset_len: f32, pub sweep: Option<(f32, [f32; 4], f32)> }

#[derive(Clone, Debug, PartialEq)]
pub struct TargetBox { pub half: Vec3, pub range: f32, pub max_targets: u32, pub exclude_instigator: bool, pub exclude_friends: bool, pub exclude_enemies: bool, pub players_only: bool }

#[derive(Clone, Debug, PartialEq)]
pub struct TrapDef {
    pub id: String,
    pub item_path: String,
    pub name: String,
    pub max_stack: u32,
    pub rarity: u8,
    pub icon: String,
    pub large_icon: String,
    pub attach: Attach,
    pub allowed_shapes: Vec<String>,
    pub class: String,
    pub mesh: String,
    pub range_mesh: Option<String>,
    pub trigger_center: Vec3,
    pub trigger_extent: Vec3,
    pub trace_location: Vec3,
    pub trigger_excludes_instigator: bool,
    pub trigger_excludes_friends: bool,
    pub target: TargetBox,
    pub placement_offset: Vec3,
    pub arm_time: f32,
    pub fire_delay: f32,
    pub damage_delay: f32,
    pub reload_time: f32,
    pub damage: f32,
    pub durability_per_use: f32,
    pub max_durability: f32,
    pub sounds: Sounds,
    pub fx: Fx,
    pub spike: Option<Spike>,
    pub behavior: Behavior,
    pub in_loot: bool,
}

pub fn short(path: &str) -> &str { let p = path.rsplit('/').next().unwrap_or(path); p.split('.').next().unwrap_or(p) }
fn obj_path(path: &str) -> String { if path.contains('.') { path.to_owned() } else { format!("{path}.{}", short(path)) } }
fn s(v: Option<&Value>) -> Option<String> { v.and_then(|x| x.as_str()).filter(|x| x.starts_with('/')).map(str::to_owned) }
fn f(v: Option<&Value>, d: f32) -> f32 { v.and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(d) }
fn v3(v: Option<&Value>) -> Vec3 { v.and_then(|x| x.as_array()).filter(|a| a.len() >= 3).map(|a| Vec3::new(f(a.first(), 0.), f(a.get(1), 0.), f(a.get(2), 0.))).unwrap_or(Vec3::ZERO) }
fn b(v: Option<&Value>, d: bool) -> bool { v.and_then(|x| x.as_bool()).unwrap_or(d) }

pub struct Pkg { pub exports: Vec<(String, String, Value)>, pub super_class: Option<String> }
pub fn load_pkg(set: &PakSet, long: &str) -> Result<Pkg, String> {
    let long = long.split('.').next().unwrap_or(long);
    let pd = PackageData::load(set, long).map_err(|e| format!("{long}: {e}"))?;
    let pkg = pd.package().map_err(|e| format!("{long}: {e}"))?;
    let exports = (0..pkg.exports.len()).filter_map(|i| { let o = pkg.read_export(i).ok()?; Some((o.name.clone(), o.class.clone(), o.props.to_json(&pkg))) }).collect();
    Ok(Pkg { exports, super_class: pkg.super_class().map(|r| r.path().to_owned()) })
}
impl Pkg {
    pub fn cdo(&self) -> Option<&Value> { self.exports.iter().find(|e| e.0.starts_with("Default__")).map(|e| &e.2) }
    pub fn named(&self, name: &str) -> Option<&Value> { self.exports.iter().find(|e| e.0 == name).map(|e| &e.2) }
    pub fn of_class<'a>(&'a self, class: &'a str) -> impl Iterator<Item = (&'a str, &'a Value)> + 'a { self.exports.iter().filter(move |e| e.1 == class).map(|e| (e.0.as_str(), &e.2)) }
    pub fn by_ref(&self, r: &str) -> Option<&Value> { self.named(r.rsplit(':').next().unwrap_or(r)) }
}

pub fn table_rows(set: &PakSet, path: &str) -> Result<Vec<(String, Value)>, String> {
    let path = path.split('.').next().unwrap_or(path);
    let pd = PackageData::load(set, path).map_err(|e| format!("{path}: {e}"))?;
    let pkg = pd.package().map_err(|e| format!("{path}: {e}"))?;
    let tb = fn_uasset::classes::read_table(&pkg, 0).map_err(|e| format!("{path}: {e}"))?;
    Ok(tb.rows.into_iter().map(|(n, p)| (n, p.to_json(&pkg))).collect())
}

fn row(set: &PakSet, handle: Option<&Value>) -> Result<Value, String> {
    let h = handle.ok_or("no WeaponStatHandle")?;
    let table = h.get("DataTable").and_then(|x| x.as_str()).ok_or("no DataTable")?;
    let name = h.get("RowName").and_then(|x| x.as_str()).ok_or("no RowName")?;
    table_rows(set, table)?.into_iter().find(|r| r.0 == name).map(|r| r.1).ok_or_else(|| format!("{table}: no row {name}"))
}

pub fn class_fx(class_short: &str) -> (Option<&'static str>, &'static [(&'static str, &'static str)], bool) {
    match class_short {
        "Trap_Floor_Spikes" => (Some("/Game/Weapons/FORT_TRAPS/Spike_Trap/P_Spike_Trap_Destruction"), &[], true),
        "Trap_Wall_Electric_Athena" => (Some("/Game/Weapons/FORT_TRAPS/Dart_Trap/P_Dart_Trap_Destruction"), &[("/Game/Weapons/FORT_TRAPS/Wall_Electric_Trap/P_ElectricWallTrap_Fire_01", "FireFX_L"), ("/Game/Weapons/FORT_TRAPS/Wall_Electric_Trap/P_ElectricWallTrap_Fire_01", "FireFX_R")], true),
        "Trap_Ceiling_ElectricWeak_Athena" => (Some("/Game/Weapons/FORT_TRAPS/Electric_Trap/Particles/Electric_trap_Destruction"), &[("/Game/Weapons/FORT_TRAPS/Electric_Trap/Particles/P_ElectricWeakCeiling_Trap_Fire_01", "FireFX")], true),
        "Trap_Floor_Player_Jump_Pad" | "Trap_Floor_Player_Jump_Free_Direction_Pad" => (Some("/Game/Weapons/FORT_TRAPS/Spike_Trap/P_Spike_Trap_Destruction"), &[], true),
        _ => (None, &[], false),
    }
}
pub const PLACEMENT_DUST: &str = "/Game/Items/Traps/Particles/P_TrapPlacementDust_01";

fn attach_of(tags: &[String], shapes: &[String]) -> Attach {
    if tags.iter().any(|t| t == "trap.Ceiling") { Attach::Ceiling }
    else if tags.iter().any(|t| t == "trap.Floor") { Attach::Floor }
    else if shapes.iter().any(|s| s.contains("/Wall/")) { Attach::Wall }
    else { Attach::Floor }
}

fn icon_name(path: Option<String>) -> String { path.map(|p| format!("{}.png", short(&p))).unwrap_or_default() }

fn target_box(ga: &Value) -> Option<TargetBox> {
    let sel = ga.get("EffectContainers")?.get("TargetSelection")?.get("List")?.as_array()?.first()?;
    let flt = sel.get("TargetFilter")?;
    Some(TargetBox { half: v3(sel.get("HalfExtents")), range: f(sel.get("Range"), 0.), max_targets: sel.get("MaxTargets").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
        exclude_instigator: b(flt.get("bExcludeInstigator"), false), exclude_friends: b(flt.get("bExcludePawnFriends"), false), exclude_enemies: b(flt.get("bExcludePawnEnemies"), false),
        players_only: flt.get("ActorTypeFilter").and_then(|x| x.as_str()).map_or(false, |t| t.ends_with("::Players")) })
}

fn ability(set: &PakSet, gas: Option<String>) -> Result<(Value, String), String> {
    let gas = gas.ok_or("no AbilitySet")?;
    let p = load_pkg(set, &gas)?;
    let ga = p.exports.iter().find_map(|e| e.2.get("GameplayAbilities").and_then(|a| a.as_array()).and_then(|a| a.first()).and_then(|x| x.as_str()).map(str::to_owned)).ok_or("no ability")?;
    let gp = load_pkg(set, &ga)?;
    let cdo = gp.cdo().cloned().ok_or("no GA CDO")?;
    Ok((cdo, ga))
}

pub fn load_def(set: &PakSet, item_long: &str, loot_items: &[String]) -> Result<TrapDef, String> {
    let ip = load_pkg(set, item_long)?;
    let id = short(item_long).to_owned();
    let it = ip.named(&id).ok_or_else(|| format!("{item_long}: no item export"))?;
    let class = s(it.get("BlueprintClass")).ok_or("no BlueprintClass")?;
    let shapes: Vec<String> = it.get("AllowedShapes").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(|x| x.split('.').next().unwrap_or(x).to_owned()).collect()).unwrap_or_default();
    let tags: Vec<String> = it.get("GameplayTags").and_then(|g| g.get("GameplayTags")).and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(str::to_owned).collect()).unwrap_or_default();
    let st = row(set, it.get("WeaponStatHandle"))?;
    let rarity_name = it.get("Rarity").and_then(|x| x.as_str()).map(|r| r.rsplit("::").next().unwrap_or(r).to_owned()).unwrap_or_else(|| "Handmade".into());
    let durability_row = st.get("DurabilityRowName").and_then(|x| x.as_str()).unwrap_or("None").to_owned();
    let max_durability = table_rows(set, DURABILITY_TABLE)?.into_iter().find(|r| r.0 == durability_row).and_then(|r| r.1.get(&rarity_name).and_then(|x| x.as_f64())).map(|x| x as f32).ok_or_else(|| format!("{DURABILITY_TABLE}: no {durability_row}/{rarity_name}"))?;
    let bp = load_pkg(set, &class)?;
    let cdo = bp.cdo().ok_or("no BP CDO")?;
    let mesh = s(cdo.get("StaticMesh")).ok_or("no StaticMesh")?;
    let (trig_c, trig_e) = bp.of_class("BoxComponent").next().map(|(_, v)| (v3(v.get("RelativeLocation")), v3(v.get("BoxExtent")))).ok_or("no trigger box")?;
    let trace_location = bp.named("TraceLocation_GEN_VARIABLE").map(|v| v3(v.get("RelativeLocation"))).unwrap_or(trig_c);
    let filt = cdo.get("TriggerFilter");
    let (ga, ga_path) = ability(set, s(cdo.get("AbilitySet")))?;
    let target = target_box(&ga).ok_or_else(|| format!("{ga_path}: no target selection"))?;
    let behavior = match ga.get("LaunchStrength").and_then(|x| x.as_f64()) {
        Some(l) => Behavior::Launch { strength: l as f32, pitch: ga.get("LaunchPitch").and_then(|x| x.as_f64()).map(|x| x as f32) },
        None => Behavior::Damage,
    };
    let pick = |names: &[&str]| names.iter().find_map(|n| s(cdo.get(*n)));
    let sounds = Sounds { placed: pick(&["Place_Trap_Sound", "Placed_Sound_Cue", "Trap_Placed_Sound"]), active: pick(&["Trap_Active_Sound", "Activate_Sound_Cue"]), fire: pick(&["Trap_Fire_Sound", "Trap_Fire_Sound_Cue"]),
        reload: pick(&["Reload_Sound", "Trap_Reload_Sound"]), explode: pick(&["Trap_Explode_Sound", "Explode_Sound_Cue", "Explosion_Sound"]) };
    let cs = short(&class).to_owned();
    let (destruction, fire, dust) = class_fx(&cs);
    let comps: Vec<Emitter> = bp.of_class("ParticleSystemComponent").filter_map(|(_, v)| Some(Emitter { system: s(v.get("Template"))?.split('.').next()?.to_owned(), location: v3(v.get("RelativeLocation")), rotation: v3(v.get("RelativeRotation")).to_array(), socket: None })).collect();
    let (idle, mut fire_fx): (Vec<Emitter>, Vec<Emitter>) = if matches!(behavior, Behavior::Launch { .. }) { (vec![], comps) } else { (comps, vec![]) };
    fire_fx.extend(fire.iter().map(|(sys, sock)| Emitter { system: (*sys).to_owned(), location: Vec3::ZERO, rotation: [0.; 3], socket: Some((*sock).to_owned()) }));
    let fx = Fx { idle, fire: fire_fx, destruction: destruction.map(str::to_owned), placed_dust: dust.then(|| PLACEMENT_DUST.to_owned()) };
    let spike = bp.of_class("StaticMeshComponent").find(|(n, v)| !n.starts_with("StaticMeshComponent0") && v.get("StaticMesh").is_some()).and_then(|(_, v)| {
        let tl = |name: &str| -> Option<(Curve, f32, Option<f32>)> {
            let t = bp.named(name)?;
            let c = t.get("FloatTracks")?.as_array()?.first()?.get("CurveFloat")?.as_str()?;
            let ev = t.get("EventTracks").and_then(|a| a.as_array()).and_then(|a| a.first()).and_then(|e| e.get("CurveKeys")).and_then(|c| c.as_str()).and_then(|c| bp.by_ref(c)).and_then(Curve::from_json).map(|c| c.keys[0].0);
            Some((Curve::from_json(bp.by_ref(c)?)?, f(t.get("TimelineLength"), 0.), ev))
        };
        let (fire, fire_len, _) = tl("Fire_Template")?;
        let (retract, retract_len, rs) = tl("Retract_Template")?;
        let (reset, reset_len, _) = tl("ResetSpikes_Template")?;
        let mats = v.get("OverrideMaterials").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(str::to_owned).collect()).unwrap_or_default();
        let sweep = cdo.get("SweepSpeed").map(|_| (f(cdo.get("SweepSpeed"), 0.), { let c = cdo.get("SweepColor").and_then(|x| x.as_array()); [0, 1, 2, 3].map(|i| c.and_then(|c| c.get(i)).and_then(|x| x.as_f64()).unwrap_or(0.) as f32) }, f(cdo.get("SweepExponent"), 0.)));
        Some(Spike { mesh: s(v.get("StaticMesh"))?, materials: mats, resting: f(cdo.get("SpikeRestingScale"), 0.), active: f(cdo.get("SpikeActiveScale"), 1.), hold: f(cdo.get("SpikeHoldOnFireLength"), 0.),
            fire, fire_len, retract, retract_len, retract_sound_at: rs.unwrap_or(0.), reset, reset_len, sweep })
    });
    Ok(TrapDef {
        id: id.clone(), item_path: obj_path(item_long),
        name: it.get("DisplayName").and_then(|d| d.get("text")).and_then(|x| x.as_str()).unwrap_or(&id).to_owned(),
        max_stack: it.get("MaxStackSize").and_then(|x| x.as_u64()).unwrap_or(1) as u32,
        rarity: crate::pickup_items::rarity_of(&rarity_name),
        icon: icon_name(s(it.get("SmallPreviewImage"))), large_icon: icon_name(s(it.get("LargePreviewImage"))),
        attach: attach_of(&tags, &shapes), allowed_shapes: shapes, class, mesh, range_mesh: s(cdo.get("TrapRangeMesh")),
        trigger_center: trig_c, trigger_extent: trig_e, trace_location,
        trigger_excludes_instigator: filt.map_or(true, |x| b(x.get("bExcludeInstigator"), true)),
        trigger_excludes_friends: filt.map_or(true, |x| b(x.get("bExcludePawnFriends"), true)),
        target, placement_offset: v3(cdo.get("TrapPlacementOffset")),
        arm_time: f(st.get("ArmTime"), 0.), fire_delay: f(st.get("FireDelay"), 0.), damage_delay: f(st.get("DamageDelay"), 0.), reload_time: f(st.get("ReloadTime"), 0.),
        damage: f(st.get("DmgPB"), 0.), durability_per_use: f(st.get("DurabilityPerUse"), 1.), max_durability,
        sounds, fx, spike, behavior, in_loot: loot_items.iter().any(|l| short(l) == id),
    })
}

pub fn loot_trap_items(set: &PakSet) -> Vec<String> {
    table_rows(set, crate::containers::PACKAGE_TABLE).unwrap_or_default().into_iter().filter(|r| r.1.get("LootPackageID").and_then(|x| x.as_str()) == Some("WorldList.AthenaTraps"))
        .filter_map(|r| r.1.get("ItemDefinition").and_then(|x| x.as_str()).map(str::to_owned)).collect()
}

pub fn load_all(set: &PakSet) -> (Vec<TrapDef>, Vec<String>) {
    let loot = loot_trap_items(set);
    let mut longs: Vec<String> = set.glob(TRAP_DIR).into_iter().filter_map(|(_, e)| fn_pak::pak_path_to_game(e.rel_path()).or_else(|| fn_pak::pak_path_to_game(&e.path))).map(|g| g.trim_end_matches(".uasset").to_owned()).collect();
    longs.sort(); longs.dedup();
    let (mut defs, mut errs) = (vec![], vec![]);
    for l in longs { match load_def(set, &l, &loot) { Ok(d) => defs.push(d), Err(e) => errs.push(format!("{l}: {e}")) } }
    (defs, errs)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase { Arming, Ready, Firing, Expired }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event { Placed, ReloadEnd, DelayBegin, Activate, Damage, ReloadBegin, OutOfDurability, Destroy }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing { pub arm_time: f32, pub fire_delay: f32, pub damage_delay: f32, pub reload_time: f32, pub durability: f32, pub cost: f32 }
impl Timing { pub fn of(d: &TrapDef) -> Timing { Timing { arm_time: fn_live::f32("FN_TRAP_ARM_TIME", d.arm_time), fire_delay: fn_live::f32("FN_TRAP_FIRE_DELAY", d.fire_delay), damage_delay: fn_live::f32("FN_TRAP_DAMAGE_DELAY", d.damage_delay), reload_time: fn_live::f32("FN_TRAP_RELOAD_TIME", d.reload_time), durability: d.max_durability, cost: d.durability_per_use } } }

#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub t: Timing,
    pub phase: Phase,
    pub cooldown: f32,
    pub fire_t: Option<f32>,
    pub damage_t: Option<f32>,
    pub durability: f32,
    pub expire_t: f32,
    pub fired: u32,
}
impl Machine {
    pub fn new(t: Timing) -> Machine { Machine { t, phase: Phase::Arming, cooldown: t.arm_time, fire_t: None, damage_t: None, durability: t.durability, expire_t: 0., fired: 0 } }
    pub fn armed(&self) -> bool { self.cooldown <= 0. && self.phase != Phase::Arming }
    pub fn gone(&self) -> bool { self.phase == Phase::Expired && self.expire_t >= fn_live::f32("FN_TRAP_DELAY_BEFORE_DESTROY", DELAY_BEFORE_DESTROY) }
    pub fn update(&mut self, dt: f32, touching: bool) -> Vec<Event> {
        let mut ev = vec![];
        if self.phase == Phase::Expired {
            let was = self.expire_t;
            self.expire_t += dt;
            let delay = fn_live::f32("FN_TRAP_DELAY_BEFORE_DESTROY", DELAY_BEFORE_DESTROY);
            if was < delay && self.expire_t >= delay { ev.push(Event::Destroy); }
            return ev;
        }
        let mut left = dt;
        let mut fired_now = false;
        for _ in 0..16 {
            if let Some(ft) = self.fire_t {
                if left < ft { self.fire_t = Some(ft - left); break; }
                left -= ft;
                self.fire_t = None;
                self.durability -= self.t.cost;
                self.cooldown = self.t.reload_time;
                self.fired += 1;
                fired_now = true;
                ev.push(Event::Activate);
                self.damage_t = Some(self.t.damage_delay);
            }
            if let Some(dd) = self.damage_t {
                if left < dd { self.damage_t = Some(dd - left); self.cooldown = (self.cooldown - left).max(0.); break; }
                left -= dd;
                self.cooldown = (self.cooldown - dd).max(0.);
                self.damage_t = None;
                ev.push(Event::Damage);
                ev.push(Event::ReloadBegin);
                if self.durability <= 0. { self.phase = Phase::Expired; self.expire_t = 0.; ev.push(Event::OutOfDurability); return ev; }
                self.phase = Phase::Ready;
            }
            if self.cooldown > 0. {
                if left < self.cooldown { self.cooldown -= left; break; }
                left -= self.cooldown;
                self.cooldown = 0.;
                if self.phase == Phase::Arming { self.phase = Phase::Ready; }
                ev.push(Event::ReloadEnd);
            }
            if self.phase == Phase::Ready && touching && !fired_now {
                self.phase = Phase::Firing;
                self.fire_t = Some(self.t.fire_delay);
                ev.push(Event::DelayBegin);
                continue;
            }
            break;
        }
        ev
    }
}

pub fn spike_scale(sp: &Spike, since_reset: Option<f32>, since_fire: Option<f32>) -> Option<f32> {
    match (since_reset, since_fire) {
        (_, Some(t)) => {
            if t < sp.fire_len { return Some(sp.resting + (sp.active - sp.resting) * sp.fire.eval(t)); }
            let r = t - sp.fire_len - sp.hold;
            if r < 0. { return Some(sp.active); }
            if r < sp.retract_len { return Some(sp.active * sp.retract.eval(r)); }
            Some(0.)
        }
        (Some(t), None) => Some(sp.resting * sp.reset.eval(t.min(sp.reset_len))),
        (None, None) => None,
    }
}

pub fn shape_allowed(def: &TrapDef, pattern: Option<&str>) -> bool {
    let Some(p) = pattern else { return false };
    let p = p.split('.').next().unwrap_or(p);
    def.allowed_shapes.iter().any(|a| a.eq_ignore_ascii_case(p))
}

pub fn wall_side(wall_yaw: f32, wall_loc: Vec3, viewer: Vec3) -> bool {
    let n = Vec3::new(-(wall_yaw.to_radians().sin()), wall_yaw.to_radians().cos(), 0.);
    (viewer - wall_loc).dot(n) >= 0.
}

pub fn rot_z(yaw: f32, v: Vec3) -> Vec3 { let (s, c) = yaw.to_radians().sin_cos(); Vec3::new(v.x * c - v.y * s, v.x * s + v.y * c, v.z) }

pub fn floor_transform(cell_center: Vec3, yaw: f32) -> Vec3 { cell_center - rot_z(yaw, Vec3::new(0., 256., 0.)) }

pub fn best_quadrant(cell_center: Vec3, base_yaw: f32, offset: Vec3, target: Vec3) -> (Vec3, f32) {
    let mut best = (floor_transform(cell_center, base_yaw), base_yaw, f32::MAX);
    for k in 0..4 {
        let yaw = base_yaw + 90. * k as f32;
        let loc = floor_transform(cell_center, yaw);
        let d = (loc + rot_z(yaw, offset) - target).length_squared();
        if d < best.2 { best = (loc, yaw, d); }
    }
    (best.0, best.1)
}

pub fn in_box(local: Vec3, center: Vec3, extent: Vec3, radius: f32, half_height: f32) -> bool {
    let d = (local - center).abs();
    d.x <= extent.x + radius && d.y <= extent.y + radius && d.z <= extent.z + half_height
}

pub fn launch_velocity(b: Behavior, pawn_velocity: Vec3) -> Option<Vec3> {
    match b {
        Behavior::Launch { strength, pitch: None } => Some(Vec3::new(0., 0., strength)),
        Behavior::Launch { strength, pitch: Some(p) } => {
            let yaw = pawn_velocity.y.atan2(pawn_velocity.x);
            let (sp, cp) = p.to_radians().sin_cos();
            Some(Vec3::new(cp * yaw.cos(), cp * yaw.sin(), sp) * strength)
        }
        Behavior::Damage => None,
    }
}

pub fn shield_first(damage: f32, health: f32, shield: f32) -> (f32, f32) {
    let s = shield.min(damage).max(0.);
    let h = (damage - s).min(health).max(0.);
    (health - h, shield - s)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spikes() -> Timing { Timing { arm_time: 1.0, fire_delay: 0.5, damage_delay: 0.0, reload_time: 6.0, durability: 30., cost: 1. } }
    fn run(m: &mut Machine, secs: f32, touching: bool, out: &mut Vec<(f32, Event)>, t0: &mut f32) {
        let dt = 1. / 120.;
        let n = (secs / dt).round() as i32;
        for _ in 0..n { *t0 += dt; for e in m.update(dt, touching) { out.push((*t0, e)); } }
    }
    #[test] fn arming_blocks_triggers_until_arm_time() {
        let mut m = Machine::new(spikes());
        let (mut ev, mut t) = (vec![], 0.);
        run(&mut m, 0.9, true, &mut ev, &mut t);
        assert!(ev.is_empty(), "{ev:?}");
        run(&mut m, 0.2, true, &mut ev, &mut t);
        assert_eq!(ev.iter().map(|e| e.1).collect::<Vec<_>>(), vec![Event::ReloadEnd, Event::DelayBegin]);
        assert!((ev[0].0 - 1.0).abs() < 0.01);
    }
    #[test] fn fire_delay_then_damage_then_reload_cooldown() {
        let mut m = Machine::new(spikes());
        let (mut ev, mut t) = (vec![], 0.);
        run(&mut m, 1.0, false, &mut ev, &mut t);
        ev.clear();
        run(&mut m, 8.0, true, &mut ev, &mut t);
        let dmg: Vec<f32> = ev.iter().filter(|e| e.1 == Event::Damage).map(|e| e.0).collect();
        assert!((dmg[0] - 1.5).abs() < 0.02, "{dmg:?}");
        assert!((dmg[1] - (1.5 + 6.0 + 0.5)).abs() < 0.03, "next shot after ReloadTime + FireDelay: {dmg:?}");
        assert_eq!(dmg.len(), 2);
        assert_eq!(m.fired, 2);
    }
    #[test] fn nobody_on_it_means_no_fire_and_no_durability_loss() {
        let mut m = Machine::new(spikes());
        let (mut ev, mut t) = (vec![], 0.);
        run(&mut m, 20.0, false, &mut ev, &mut t);
        assert_eq!(ev.iter().map(|e| e.1).collect::<Vec<_>>(), vec![Event::ReloadEnd]);
        assert_eq!(m.durability, 30.);
    }
    #[test] fn leaving_during_fire_delay_still_fires() {
        let mut m = Machine::new(spikes());
        let (mut ev, mut t) = (vec![], 0.);
        run(&mut m, 1.0, false, &mut ev, &mut t);
        run(&mut m, 0.1, true, &mut ev, &mut t);
        run(&mut m, 1.0, false, &mut ev, &mut t);
        assert_eq!(ev.iter().filter(|e| e.1 == Event::Damage).count(), 1);
    }
    #[test] fn durability_runs_out_then_destroyed_after_delay() {
        let mut m = Machine::new(Timing { durability: 3., ..spikes() });
        let (mut ev, mut t) = (vec![], 0.);
        run(&mut m, 40.0, true, &mut ev, &mut t);
        assert_eq!(ev.iter().filter(|e| e.1 == Event::Damage).count(), 3);
        let out = ev.iter().find(|e| e.1 == Event::OutOfDurability).unwrap().0;
        let gone = ev.iter().find(|e| e.1 == Event::Destroy).unwrap().0;
        assert!((gone - out - DELAY_BEFORE_DESTROY).abs() < 0.02);
        assert!(m.gone());
    }
    #[test] fn jump_pad_has_no_delay_or_reload_and_fires_each_frame_it_is_touched() {
        let mut m = Machine::new(Timing { arm_time: 1.0, fire_delay: 0., damage_delay: 0., reload_time: 0., durability: 620., cost: 1. });
        let (mut ev, mut t) = (vec![], 0.);
        run(&mut m, 1.0, false, &mut ev, &mut t);
        ev.clear();
        run(&mut m, 0.1, true, &mut ev, &mut t);
        assert!(ev.iter().filter(|e| e.1 == Event::Damage).count() >= 10);
    }
    #[test] fn shield_takes_damage_first() {
        assert_eq!(shield_first(125., 100., 50.), (25., 0.));
        assert_eq!(shield_first(125., 100., 0.), (0., 0.));
        assert_eq!(shield_first(30., 100., 50.), (100., 20.));
    }
    #[test] fn spike_animation_follows_the_timelines() {
        let lin = Curve { keys: [(0., 0., 0., 0., 0), (0.05, 1., 0., 0., 0), (0., 0., 0., 0., 0), (0., 0., 0., 0., 0)], n: 2 };
        let down = Curve { keys: [(0., 1., 0., 0., 2), (0.4, 0., -6.27, 0., 2), (0., 0., 0., 0., 0), (0., 0., 0., 0., 0)], n: 2 };
        let reset = Curve { keys: [(0., 0., 0., 7.118, 2), (0.2539, 1.6369, -0.442, -0.442, 2), (0.4, 1.00835, -7.299, -7.299, 2), (0., 0., 0., 0., 0)], n: 3 };
        let sp = Spike { mesh: String::new(), materials: vec![], resting: 0.25, active: 1.0, hold: 0.75, fire: lin, fire_len: 0.05, retract: down, retract_len: 0.4, retract_sound_at: 0.0535, reset, reset_len: 0.4, sweep: None };
        assert_eq!(spike_scale(&sp, None, None), None);
        assert!((spike_scale(&sp, Some(1.0), None).unwrap() - 0.25 * 1.00835).abs() < 1e-4);
        assert!(spike_scale(&sp, Some(0.25), None).unwrap() > 0.4, "the reset overshoots");
        assert!((spike_scale(&sp, None, Some(0.05)).unwrap() - 1.0).abs() < 1e-4);
        assert_eq!(spike_scale(&sp, None, Some(0.5)), Some(1.0));
        assert_eq!(spike_scale(&sp, None, Some(2.0)), Some(0.0));
    }
    #[test] fn jump_pad_picks_the_quadrant_nearest_the_aim() {
        let c = Vec3::new(256., 256., 0.);
        let (loc, yaw) = best_quadrant(c, 0., Vec3::new(-128., 410., 0.), Vec3::new(100., 400., 0.));
        let centre = loc + rot_z(yaw, Vec3::new(-128., 384., 0.));
        assert!((centre - Vec3::new(128., 384., 0.)).length() < 1., "{centre}");
    }
    #[test] fn the_wall_trap_faces_the_side_it_was_placed_from() {
        assert!(wall_side(0., Vec3::ZERO, Vec3::new(0., 300., 100.)));
        assert!(!wall_side(0., Vec3::ZERO, Vec3::new(0., -300., 100.)));
        assert!(wall_side(90., Vec3::ZERO, Vec3::new(-300., 0., 100.)));
        let (s, t) = (Vec3::new(0., 0., 0.), Vec3::new(0., 278., 172.));
        assert!((s + rot_z(90., t) - Vec3::new(-278., 0., 172.)).length() < 1e-3);
    }
    #[test] fn the_trigger_box_overlap_counts_the_capsule() {
        assert!(in_box(Vec3::new(0., 256. + 270., 150.), Vec3::new(0., 256., 104.), Vec3::new(240., 240., 80.), 42., 90.));
        assert!(!in_box(Vec3::new(0., 256. + 300., 150.), Vec3::new(0., 256., 104.), Vec3::new(240., 240., 80.), 42., 90.));
    }
    fn set() -> Option<PakSet> {
        let root = fn_live::var("FN_TEST_ROOT").unwrap_or_else(|_| r"C:\Users\Brady\Downloads\1.8\Fortnite\Fortnite\FortniteGame".into());
        PakSet::open_install(&root).ok()
    }
    #[test] fn the_cooked_athena_traps() {
        let Some(set) = set() else { eprintln!("no 1.8 install: skipped"); return };
        let (defs, errs) = load_all(&set);
        assert!(errs.iter().all(|e| e.contains("TID_Ceiling_Falling_Athena")), "{errs:?}");
        let get = |id: &str| defs.iter().find(|d| d.id == id).unwrap_or_else(|| panic!("{id}"));
        let sp = get("TID_Floor_Spikes_Athena_R_T03");
        assert_eq!((sp.attach, sp.max_stack, sp.damage, sp.arm_time, sp.fire_delay, sp.reload_time, sp.max_durability), (Attach::Floor, 3, 125., 1.0, 0.5, 6.0, 30.));
        assert_eq!(sp.trigger_extent, Vec3::new(240., 240., 80.));
        assert!(sp.trigger_excludes_instigator && sp.target.exclude_instigator && sp.target.exclude_friends);
        assert!(sp.spike.as_ref().is_some_and(|s| s.resting == 0.25 && s.hold == 0.75));
        assert!(sp.in_loot);
        let w = get("TID_Wall_Electric_Athena_R_T03");
        assert_eq!((w.attach, w.allowed_shapes.len(), w.reload_time, w.max_durability), (Attach::Wall, 6, 12.0, 30.));
        assert_eq!(w.fx.fire.len(), 2);
        let c = get("TID_Ceiling_Electric_Single_Athena_R_T03");
        assert_eq!((c.attach, c.arm_time, c.max_durability, c.target.max_targets), (Attach::Ceiling, 3.0, 20., 1));
        let j = get("TID_Floor_Player_Jump_Pad_Athena");
        assert_eq!(j.behavior, Behavior::Launch { strength: 2150., pitch: None });
        assert!(!j.trigger_excludes_instigator, "jump pads launch their owner");
        assert_eq!((j.max_stack, j.max_durability), (20, 620.));
        let jd = get("TID_Floor_Player_Jump_Pad_Free_Direction_Athena");
        assert_eq!(jd.behavior, Behavior::Launch { strength: 2500., pitch: Some(35.) });
        assert_eq!(defs.iter().filter(|d| d.in_loot).count(), 5);
    }
    #[test] fn floor_loot_rolls_traps_at_the_trap_list_weights() {
        let Some(set) = set() else { return };
        let Ok(t) = crate::containers::LootTables::load(&set) else { return };
        let mut rng = crate::containers::Rng::new(11);
        let (mut traps, n) = (std::collections::HashMap::<String, u32>::new(), 40000);
        for _ in 0..n { for d in t.roll("Loot_AthenaFloorLoot", &mut rng) { if d.item.contains("/Traps/TID_") { *traps.entry(short(&d.item).to_owned()).or_default() += 1; } } }
        let total: u32 = traps.values().sum();
        assert!(total > 0);
        let share = |id: &str| *traps.get(id).unwrap_or(&0) as f32 / total as f32;
        assert!((share("TID_Ceiling_Electric_Single_Athena_R_T03") - 1.75 / 4.25).abs() < 0.03, "{traps:?}");
        assert!((share("TID_Floor_Spikes_Athena_R_T03") - 1.0 / 4.25).abs() < 0.03, "{traps:?}");
        assert!((share("TID_Floor_Player_Jump_Pad_Athena") - 0.25 / 4.25).abs() < 0.02, "{traps:?}");
        assert_eq!(t.roll("Loot_AthenaTreasure", &mut rng).iter().filter(|d| d.item.contains("TID_")).count(), 0);
    }
}
