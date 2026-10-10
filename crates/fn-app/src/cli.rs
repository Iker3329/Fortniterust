//! Command line of `fortnite-rs`.
//!
//! ```text
//! fortnite-rs                         normal game: splash, start-up loading screen, game-mode picker, level-travel loading screen, world (settings from FortniteSettings.ini beside the exe)
//! fortnite-rs --assets <dir>          exported asset tree (default: FortniteRuntime.toml / paths.txt beside the exe, `assets` beside the exe, FN_ASSETS; an interactive launch finally tries the known default folders, see paths.rs)
//! fortnite-rs --paks <Paks dir>       Fortnite 1.8 install (or its Paks folder; default: FortniteRuntime.toml / paths.txt `paks =`, else the known install of an interactive launch): exported into a cache by `fn-export` when the asset tree lacks files
//! fortnite-rs --smoke-test            fixed-60 Hz scripted run (walk, jump, crouch, sprint, swing) with captures + perf in --out, exit 0 on success
//! fortnite-rs --script <file|builtin:NAME>   replay an input timeline (see script.rs), write captures / frame log / frame-time stats
//! fortnite-rs --screenshot out.png    render the spawn view once (after loading) and exit
//! fortnite-rs --startup-capture --out DIR [--size WxH]   run the WHOLE start-up flow on a fake 60 Hz clock, driving the picker with synthetic events, and
//!                                     write numbered PNGs of every screen plus a timestamped state log into DIR
//! ```
//! Other options: `--out <dir>` (captures, logs; default `verification` beside the exe), `--size WxH` (client size for scripted runs, default
//! 1280x720), `--window-mode windowed|borderless|fullscreen` (one run only, never saved), `--no-settings` (defaults; the ini is neither read nor
//! written; implied by every scripted/diagnostic mode), `--no-gamepad`, `--no-vsync`, `--compare-sim` (script mode: replay the built-in scenario
//! through fn-sim and diff the frame log line for line), `--frames N` (stop scripted runs early), `--quick` (short splash/loading holds), `--skip-loading` (no splash, start-up screen, picker or
//! loading minimum: straight into the load; implied by every scripted mode),
//! `--help`.
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Options {
    pub assets: Option<PathBuf>,
    pub paks: Option<PathBuf>,
    pub manifest: Option<PathBuf>,
    pub script: Option<String>,
    pub smoke: bool,
    pub screenshot: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub size: Option<(u32, u32)>,
    pub window_mode: Option<fn_settings::WindowMode>,
    pub no_settings: bool,
    pub no_gamepad: bool,
    pub no_vsync: bool,
    /// Scripted runs default to vsync off (frame times then measure the renderer); `--vsync` keeps the monitor pacing.
    pub vsync: bool,
    pub compare_sim: bool,
    pub frames: Option<u32>,
    pub quick: bool,
    /// Skip the splash, the start-up screen, the picker and every hold (tests / scripts). The default launch shows everything.
    pub skip_loading: bool,
    /// Deterministic capture of the whole start-up flow (see `startup::capture`).
    pub startup_capture: bool,
    /// Game picker between start-up and level travel. This is part of a normal launch; `--game-picker` remains accepted for compatibility.
pub game_picker: bool,
    /// Interactive code path exercised on a schedule by window operations / handler calls (see `app.rs`).
    pub live_test: bool,
    /// `--props real|sandbox|none`: overrides what fills the world (default: the real 1.8 props in the live game and for `--screenshot`, the script's own `props` line for scripts).
    pub props: Option<PropsChoice>,
    /// `--prop-radius M`: radius of the real layout around the anchor (default `props::DEFAULT_RADIUS_M`).
    pub prop_radius: Option<f32>,
    /// `--world-center salty | X,Y[,Z]` (POI dev): the UE world point (cm) the layout / landscape / town load around instead of the forest sample centre.
    pub world_center: Option<[f32; 3]>,
    /// `--assume-unrecovered`: development stand-ins for weapon behaviour the 1.8 data does not decide (see `arms::ASSUMPTIONS`). Off by default.
    pub assume_unrecovered: bool,
    /// `--standin-gun-material`: draw the held gun, held pickaxe and dropped weapons with the renderer's Prop model (original Diffuse / Normals, dielectric) instead of their
    /// cooked material. Off by default (comparison / fallback switch).
    pub standin_gun_material: bool,
    /// GUN MATERIAL HOOK: the held gun, held pickaxe and dropped weapons run their ORIGINAL cooked material (WeaponMat_Base base-pass programs) through the gun pass, as
    /// 1.8 does. ON by default since the gun pass casts CSM shadows and gets the DFAO + the sky light's specular (docs/spec/pickup_visual_gaps.md section 2); a material
    /// that fails to load falls back to the Prop model, logged per mesh. `--standin-gun-material` turns it off; `--original-gun-material` is accepted (the default).
    pub original_gun_material: bool,
    /// `--headless`: never show the window or take focus (the run still renders; captures work). Automated runs are headless anyway, see [`headless`].
    pub headless: bool,
    /// INVENTORY HOOK `--dev-loadout` (or `FN_DEV_LOADOUT=1`): start with the dev quickbar (AR, pump, sniper, bandages x5) instead of the 1.8 BR spawn inventory (pickaxe only, 0 mats).
    pub dev_loadout: bool,
    /// SCALABILITY: `--video-quality 0..3` sets every video quality group (3D resolution, view distance, shadows, AA, textures, effects, post) of an
    /// automated run to one level (Low / Medium / High / Epic) for comparisons.
    pub video_quality: Option<u32>,
    /// POSTFX: `--motion-blur on|off` sets the Motion Blur setting (`video.motion_blur`) of an automated run (before / after comparisons).
    pub motion_blur: Option<bool>,
    pub help: bool,
}

/// Is the window kept hidden (and focus never taken)? `--headless`, or any automated run (script / screenshot / smoke / start-up capture / live test) unless
/// `FN_SHOW_WINDOW=1` asks to watch it.
pub fn headless(o: &Options) -> bool {
    if o.headless { return true; }
    let automated = o.scripted() || o.startup_capture || o.live_test;
    automated && fn_live::var("FN_SHOW_WINDOW").map_or(true, |v| v != "1")
}

impl Default for Options {
    fn default() -> Self {
        Self {
            assets: None, paks: None, manifest: None, script: None, smoke: false, screenshot: None, out: None, size: None,
            window_mode: None, no_settings: false, no_gamepad: false, no_vsync: false, vsync: false, compare_sim: false,
            frames: None, quick: false, skip_loading: false, startup_capture: false, game_picker: true, live_test: false,
            props: None, prop_radius: None, world_center: None, assume_unrecovered: false, standin_gun_material: false, original_gun_material: true, headless: false, dev_loadout: false, video_quality: None, motion_blur: None, help: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropsChoice { Real, Sandbox, None }

impl Options {
    /// Scripted / diagnostic runs: fixed time step, no mouse capture, defaults instead of the user's settings.
    pub fn scripted(&self) -> bool { self.smoke || self.script.is_some() || self.screenshot.is_some() }
    /// Runs that never show the start-up flow: every scripted / diagnostic mode (they start straight in the load) and an explicit `--skip-loading`.
    pub fn skips_flow(&self) -> bool { self.skip_loading || self.scripted() || self.live_test }
    /// Automated runs use defaults and never touch the user's settings file.
    pub fn uses_user_settings(&self) -> bool { !self.no_settings && !self.scripted() && !self.live_test && !self.startup_capture }
}

pub const USAGE: &str = "fortnite-rs [--assets DIR] [--paks DIR] [--smoke-test] [--script FILE|builtin:NAME] [--screenshot OUT.png] [--out DIR] [--size WxH]\n\
    [--window-mode windowed|borderless|fullscreen] [--no-settings] [--no-gamepad] [--vsync|--no-vsync] [--compare-sim] [--frames N] [--quick] [--skip-loading] [--startup-capture] [--live-test]\n\
    [--props real|sandbox|none] [--prop-radius M] [--world-center salty|X,Y] [--assume-unrecovered] [--standin-gun-material] [--headless] [--dev-loadout]\n\
    builtin scripts: smoke, pickaxe, walk";

pub fn parse_size(s: &str) -> Result<(u32, u32), String> {
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(|| format!("expected WxH, got {s:?}"))?;
    let (w, h): (u32, u32) = (w.trim().parse().map_err(|_| format!("bad width in {s:?}"))?, h.trim().parse().map_err(|_| format!("bad height in {s:?}"))?);
    if w == 0 || h == 0 { return Err(format!("size {s:?} must be positive")); }
    Ok((w, h))
}

pub fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut i = 0;
    let value = |i: &mut usize, name: &str| -> Result<String, String> { *i += 1; args.get(*i).cloned().ok_or_else(|| format!("{name} needs a value")) };
    while i < args.len() {
        let a = args[i].clone();
        match a.as_str() {
            "--assets" => o.assets = Some(value(&mut i, &a)?.into()),
            "--paks" => o.paks = Some(value(&mut i, &a)?.into()),
            "--manifest" => o.manifest = Some(value(&mut i, &a)?.into()),
            "--script" => o.script = Some(value(&mut i, &a)?),
            "--smoke-test" => o.smoke = true,
            "--screenshot" => o.screenshot = Some(value(&mut i, &a)?.into()),
            "--out" => o.out = Some(value(&mut i, &a)?.into()),
            "--size" => o.size = Some(parse_size(&value(&mut i, &a)?)?),
            "--window-mode" => {
                o.window_mode = Some(match value(&mut i, &a)?.to_ascii_lowercase().as_str() {
                    "windowed" => fn_settings::WindowMode::Windowed,
                    "borderless" | "windowedfullscreen" | "windowed-fullscreen" => fn_settings::WindowMode::WindowedFullscreen,
                    "fullscreen" | "exclusive" => fn_settings::WindowMode::Fullscreen,
                    other => return Err(format!("unknown window mode {other:?}")),
                })
            }
            "--no-settings" => o.no_settings = true,
            "--no-gamepad" => o.no_gamepad = true,
            "--no-vsync" => o.no_vsync = true,
            "--vsync" => o.vsync = true,
            "--compare-sim" => o.compare_sim = true,
            "--frames" => o.frames = Some(value(&mut i, &a)?.parse().map_err(|_| "--frames needs a number".to_owned())?),
            "--quick" => o.quick = true,
            "--skip-loading" => o.skip_loading = true,
            "--startup-capture" => o.startup_capture = true,
            "--game-picker" => o.game_picker = true,
            "--live-test" => o.live_test = true,
            "--assume-unrecovered" => o.assume_unrecovered = true,
            "--standin-gun-material" => { o.standin_gun_material = true; o.original_gun_material = false; }
            "--original-gun-material" => o.original_gun_material = true, // GUN MATERIAL HOOK
            "--props" => o.props = Some(match value(&mut i, &a)?.to_ascii_lowercase().as_str() { "real" => PropsChoice::Real, "sandbox" => PropsChoice::Sandbox, "none" => PropsChoice::None, other => return Err(format!("--props: unknown value {other:?} (real, sandbox, none)")) }),
            "--prop-radius" => { let r: f32 = value(&mut i, &a)?.parse().map_err(|_| "--prop-radius needs a number of metres".to_owned())?; if !(5. ..=250.).contains(&r) { return Err("--prop-radius must be 5..250 metres".into()); } o.prop_radius = Some(r); }
            "--world-center" => { let c = crate::poi::parse_center(&value(&mut i, &a)?)?; crate::poi::set_world_center(c); o.world_center = Some(c); } // POI (dev)
            "--headless" => o.headless = true,
            "--dev-loadout" => o.dev_loadout = true, // INVENTORY HOOK
            "--video-quality" => o.video_quality = Some(value(&mut i, &a)?.parse::<u32>().ok().filter(|l| *l <= 3).ok_or_else(|| "--video-quality needs 0..3".to_owned())?), // SCALABILITY
            "--motion-blur" => o.motion_blur = Some(match value(&mut i, &a)?.to_ascii_lowercase().as_str() { "on" | "1" | "true" => true, "off" | "0" | "false" => false, _ => return Err("--motion-blur needs on|off".to_owned()) }), // POSTFX
            "--tuning" => { value(&mut i, &a)?; }
            "--help" | "-h" | "/?" => o.help = true,
            other => return Err(format!("unknown option {other}")),
        }
        i += 1;
    }
    Ok(o)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn a(s: &str) -> Vec<String> { s.split_whitespace().map(String::from).collect() }
    #[test] fn parses_flags_and_values() {
        let o = parse(&a("--assets x --smoke-test --size 800x600 --window-mode borderless --frames 5 --no-vsync")).unwrap();
        assert_eq!(o.assets.as_deref().map(|p| p.to_str().unwrap()), Some("x"));
        assert!(o.smoke && o.no_vsync && o.scripted() && !o.uses_user_settings());
        assert_eq!(o.size, Some((800, 600))); assert_eq!(o.frames, Some(5));
        assert_eq!(o.window_mode, Some(fn_settings::WindowMode::WindowedFullscreen));
    }
    #[test] fn rejects_garbage() { assert!(parse(&a("--size 12")).is_err()); assert!(parse(&a("--bogus")).is_err()); assert!(parse(&a("--assets")).is_err()); assert!(parse(&a("--window-mode x")).is_err()); assert!(parse(&a("--size 0x5")).is_err()); }
    #[test] fn game_picker_is_part_of_a_normal_launch() {
        assert!(parse(&[]).unwrap().game_picker, "the default flow includes the picker");
        let o = parse(&a("--game-picker")).unwrap(); assert!(o.game_picker && !o.skips_flow());
    }
    #[test] fn skip_loading_and_startup_capture_flags() {
        let o = parse(&a("--skip-loading")).unwrap(); assert!(o.skip_loading && o.skips_flow() && o.uses_user_settings());
        assert!(!parse(&[]).unwrap().skips_flow(), "the default launch shows the whole flow");
        for f in ["--smoke-test", "--live-test", "--screenshot x.png", "--script builtin:walk"] { assert!(parse(&a(f)).unwrap().skips_flow(), "{f}"); }
        let c = parse(&a("--startup-capture --out d --size 800x600")).unwrap(); assert!(c.startup_capture && !c.skips_flow() && !c.uses_user_settings() && c.size == Some((800, 600)));
    }
    #[test] fn plain_run_uses_the_user_settings() { assert!(parse(&[]).unwrap().uses_user_settings()); assert!(!parse(&a("--no-settings")).unwrap().uses_user_settings()); }
}


