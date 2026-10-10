//! MATERIAL PARAMETERS (2026-10-06): the world's `FortniteMaterialParameters` instance (`UMaterialParameterCollectionInstance`), written the way the 1.8
//! time-of-day manager writes it (fn_fx::mpc: `TODM_A`'s construction script and `ReceiveBeginPlay`, the native manager tick), pushed over the FX renderer's one
//! shared buffer of the collection every frame. That buffer exists once a drawn program reads the collection (fn_fx::gpu creates it with the defaults); until
//! the world begins play ([`begin_play`]) it keeps the defaults, as UE's instance does before the manager writes it.
use fn_fx::mpc::{self, Instance, FORTNITE_MATERIAL_PARAMETERS};
use std::sync::Mutex;

/// The world's instance: the `GetShadowQuality()` level read at begin play, and the values written so far (the begin-play ones on the first frame the
/// collection is loaded).
struct World { shadow_quality: u32, inst: Option<Instance> }
static WORLD: Mutex<Option<World>> = Mutex::new(None);

/// The world begins play (the TOD manager is constructed and begins play): latches `UGameUserSettings::GetShadowQuality()` = the `sg.ShadowQuality` level.
/// A later settings change does not rewrite the begin-play values (TODM_A writes them only in `ReceiveBeginPlay`).
pub fn begin_play(shadow_quality: u8) {
    if let Ok(mut w) = WORLD.lock() { *w = Some(World { shadow_quality: shadow_quality as u32, inst: None }); }
    eprintln!("[mpc] world begins play: FortniteMaterialParameters AreDistanceFieldsEnabled? = {} (sg.ShadowQuality {shadow_quality} > 2), ShadowDistance {} cm, ShadowDistanceFadeoutFraction {}",
        if shadow_quality > 2 { 1 } else { 0 }, fn_render::lighting::SHADOW_DISTANCE_M * 100., fn_render::lighting::SHADOW_DISTANCE_FADEOUT_FRACTION);
}

/// Run report: which collections the material programs bound (fn_fx::gpu) and the FortniteMaterialParameters values written last.
pub fn report() -> String {
    let mut s = String::from("material parameter collections (program bindings):\n");
    for (path, n) in fn_fx::gpu::collection_bind_counts() { s.push_str(&format!("  {n:5}  {path}\n")); }
    if let (Some(c), Ok(g)) = (mpc::loaded(FORTNITE_MATERIAL_PARAMETERS), WORLD.lock()) {
        if let Some(inst) = g.as_ref().and_then(|w| w.inst.as_ref()) {
            let mut names: Vec<&String> = inst.scalars.keys().chain(inst.vectors.keys()).collect();
            names.sort();
            for n in names {
                let (i, k) = c.index_of(n).unwrap_or((usize::MAX, None));
                match (inst.scalars.get(n.as_str()), inst.vectors.get(n.as_str())) { (Some(v), _) => s.push_str(&format!("  FortniteMaterialParameters[{i}].{}  {n} = {v}\n", ["x", "y", "z", "w"][k.unwrap_or(0)])), (_, Some(v)) => s.push_str(&format!("  FortniteMaterialParameters[{i}]  {n} = {v:?}\n")), _ => {} }
            }
        }
    }
    s
}

/// One frame: the manager tick at the clock's hour (`real_time_s` = `UWorld::RealTimeSeconds`), then the instance over the shared buffer (queued in `out`).
pub fn frame(fx: &fn_render::fx::FxRenderer, real_time_s: f32, out: &mut fn_render::fx::FxFrame) {
    let (Some(buf), Some(c)) = (fx.collection_buffer(FORTNITE_MATERIAL_PARAMETERS), mpc::loaded(FORTNITE_MATERIAL_PARAMETERS)) else { return };
    let Ok(mut g) = WORLD.lock() else { return };
    let Some(w) = g.as_mut() else { return };
    let level = w.shadow_quality;
    let inst = w.inst.get_or_insert_with(|| { let mut i = Instance::default(); mpc::construction_script(&c, &mut i); mpc::begin_play(&c, &mut i, level); i });
    mpc::tick(&c, inst, fn_render::time_of_day::hour(), real_time_s);
    let data = c.data(&debug_overrides(&c, inst));
    out.buffer_writes.push((buf, fn_fx::view::bytes(&data)));
}

/// Verification aid (never set in play, like `FN_TOD_HOUR`): `FN_MPC_SET="Name=1.5;Other=0"` replaces those FortniteMaterialParameters scalars in the uploaded
/// buffer after the writers ran (the instance itself keeps the written values).
fn debug_overrides(c: &mpc::Collection, inst: &Instance) -> Instance {
    static SET: std::sync::OnceLock<Vec<(String, f32)>> = std::sync::OnceLock::new();
    let set = SET.get_or_init(|| {
        let v: Vec<(String, f32)> = fn_live::var("FN_MPC_SET").unwrap_or_default().split(';').filter_map(|kv| { let (k, v) = kv.split_once('=')?; Some((k.trim().to_owned(), v.trim().parse().ok()?)) }).collect();
        if !v.is_empty() { eprintln!("[mpc] FN_MPC_SET (verification override): {v:?}"); }
        v
    });
    let mut i = inst.clone();
    for (k, v) in set { if !i.set_scalar(c, k, *v) { eprintln!("[mpc] FN_MPC_SET: {k} is not a scalar of the collection"); } }
    i
}
