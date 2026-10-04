//! Settings session: the working copy of the live `Settings` that menu pages edit, plus the apply/save/revert protocol with the
//! application. There are NO thread-locals: a [`Session`] is owned by the `MenuSystem`, reaches pages through `Ui::session`, and the
//! host-supplied things (the game's own `Settings`, the platform, the apply layer) arrive once per frame in a [`UiContext`].
//!
//! Data flow
//! * `UiContext::settings` = what the game is running with ("applied"). The session keeps `applied` (a clone of it, refreshed by
//!   `sync_from` when the menu opens and after every apply) and `working` (what the pages edit).
//! * Pages call `ui.session.with_settings(|s| s.set("video.vsync", Value::Bool(true)))`. Nothing reaches the game until Apply.
//! * Apply (footer button, or `request_apply()`) queues a `Request`; `MenuSystem::frame` calls [`Session::pump`] once per frame with the
//!   `UiContext`: working values are copied into the game's `Settings` (diff + `Settings::set`, so the apply layer's own "last applied"
//!   bookkeeping is preserved), the [`ApplyBackend`] runs the hooks and saves atomically, and the result comes back as a status line.
//! * Risky video changes (window mode / display resolution) are followed by the original confirmation
//!   ("Do you want to keep these video settings?", Yes/No) with a 15 s revert-if-not-confirmed countdown. Until the user
//!   keeps them the settings FILE still holds the previous values (so a crash/kill during the countdown restores them).
use super::rows::extra_keys;
use crate::settings::{self, ApplyReport, DisplayMode, PlatformHost, Settings, Value};
use std::path::PathBuf;

/// Seconds the "keep these video settings?" dialog waits before reverting (original behaviour; value ASSUMED = 15 s per the task brief).
pub const REVERT_SECONDS: f32 = 15.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReqKind {
    /// Apply working -> game and save. `confirm`: show the keep/revert dialog if display mode/resolution changed.
    Apply { confirm: bool },
    /// Apply working -> game without writing the settings file (live preview, e.g. brightness drag).
    Preview,
    /// Write the game's current settings to the file (after the user kept risky video settings).
    Save,
    /// Apply working -> game and save; used to undo a refused/unconfirmed video change.
    Revert,
    /// Apply working -> game WITHOUT saving and forget the preview (Discard after a live preview).
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind { Info, Ok, Warn, Error }
#[derive(Clone, Debug)]
pub struct Status { pub text: String, pub kind: StatusKind, pub age: f32 }

/// The application's apply layer: runs the hooks over the game's settings (video through `LiveVideo`, input, audio, game) and
/// persists them. Implemented by the front-end (`fn-app`); [`StandardBackend`] and the test/scene `NullBackend` are provided.
pub trait ApplyBackend {
    /// Run all apply hooks over `settings` (the game's own copy; apply bookkeeping lives inside); `save` = also write the settings file.
    fn apply(&mut self, settings: &mut Settings, platform: &mut dyn PlatformHost, save: bool) -> ApplyReport;
    /// Write `s` to the settings file.
    fn write_file(&mut self, s: &Settings) -> Result<(), String>;
    /// Whether this run persists settings at all (false in automated runs).
    fn persists(&self) -> bool { true }
}

/// Everything the UI needs from the host for one frame, passed explicitly (no globals):
/// * `settings`: the game's live, applied [`Settings`] (the Settings screen edits a working copy and pushes it here on Apply);
/// * `platform`: platform capabilities (valid resolution list / current monitor mode via [`PlatformHost`], window mode on apply);
/// * `backend`: the apply layer that runs the hooks and writes the ini;
/// * `pad`: state of the gamepad shown by the Controller tab.
///
/// (Not to be confused with `scene::UiContext`, which only carries textures and fonts for rendering.)
pub struct UiContext<'a> {
    pub settings: &'a mut Settings,
    pub platform: &'a mut dyn PlatformHost,
    pub backend: &'a mut dyn ApplyBackend,
    pub pad: crate::inputdev::pad::PadStatus,
}
impl<'a> UiContext<'a> {
    pub fn new(settings: &'a mut Settings, platform: &'a mut dyn PlatformHost, backend: &'a mut dyn ApplyBackend) -> Self {
        UiContext { settings, platform, backend, pad: Default::default() }
    }
    pub fn with_pad(mut self, pad: crate::inputdev::pad::PadStatus) -> Self { self.pad = pad; self }
}

/// An apply layer over the real `fn-settings` apply functions with caller-chosen renderer hooks: window/vsync/cap go to the platform,
/// renderer-bound settings to `renderer`, input to `input`, audio is stored only. Writes the ini atomically to `path` (None = never).
pub struct StandardBackend<R: settings::VideoHooks> {
    pub renderer: R,
    pub runtime: settings::Runtime,
    pub input: settings::InputLive,
    pub path: Option<PathBuf>,
}
impl<R: settings::VideoHooks> StandardBackend<R> {
    pub fn new(renderer: R, path: Option<PathBuf>) -> Self { StandardBackend { renderer, runtime: Default::default(), input: Default::default(), path } }
}
impl<R: settings::VideoHooks> ApplyBackend for StandardBackend<R> {
    fn apply(&mut self, s: &mut Settings, platform: &mut dyn PlatformHost, save: bool) -> ApplyReport {
        let mut r = {
            let mut hooks = settings::LiveVideo { platform, renderer: &mut self.renderer, runtime: &mut self.runtime };
            s.apply_video(&mut hooks)
        };
        r.merge(s.apply_input(&mut self.input));
        r.merge(s.apply_audio(&mut settings::StoredOnlyAudio));
        r.merge(s.apply_game());
        s.drain_changes();
        if save { if let Err(e) = self.write_file(s) { r.failed.push(("file", e)); } }
        r
    }
    fn write_file(&mut self, s: &Settings) -> Result<(), String> { match &self.path { Some(p) => s.save(p).map_err(|e| e.to_string()), None => Ok(()) } }
    fn persists(&self) -> bool { self.path.is_some() }
}

/// An apply layer with no-op hooks that writes nothing (headless runs, captures, tests). `persists()` is false so status lines never claim "saved".
pub struct NullBackend { pub applied_calls: Vec<bool>, pub files: Vec<Settings>, pub fail_display: bool, pub persist: bool }
impl Default for NullBackend { fn default() -> Self { NullBackend { applied_calls: Vec::new(), files: Vec::new(), fail_display: false, persist: false } } }
impl ApplyBackend for NullBackend {
    fn apply(&mut self, s: &mut Settings, _platform: &mut dyn PlatformHost, save: bool) -> ApplyReport {
        self.applied_calls.push(save);
        struct V { fail: bool }
        impl settings::VideoHooks for V { fn set_display(&mut self, _: settings::WindowMode, _: u32, _: u32) -> Result<(), String> { if self.fail { Err("refused".into()) } else { Ok(()) } } }
        let mut r = s.apply_video(&mut V { fail: self.fail_display });
        r.merge(s.apply_input(&mut settings::InputLive::default()));
        r.merge(s.apply_audio(&mut settings::StoredOnlyAudio));
        r.merge(s.apply_game());
        s.drain_changes();
        if save && self.persist { self.files.push(s.clone()); }
        r
    }
    fn write_file(&mut self, s: &Settings) -> Result<(), String> { if self.persist { self.files.push(s.clone()); } Ok(()) }
    fn persists(&self) -> bool { self.persist }
}

/// A complete headless host: owns the game's settings, a platform with a fixed 16:9 mode list and a [`NullBackend`]. `cx()` borrows it as a [`UiContext`].
pub struct NullHost { pub s: Settings, pub platform: settings::NullPlatform, pub backend: NullBackend }
impl NullHost {
    /// Starts like the real game: everything already applied once (so reports only list what a later Apply changes).
    pub fn new() -> Self {
        let mut h = NullHost { s: Settings::default(), platform: settings::NullPlatform::static_16_9(), backend: NullBackend::default() };
        h.backend.apply(&mut h.s, &mut h.platform, false);
        h.backend.applied_calls.clear();
        h
    }
    pub fn cx(&mut self) -> UiContext<'_> { UiContext::new(&mut self.s, &mut self.platform, &mut self.backend) }
}
impl Default for NullHost { fn default() -> Self { Self::new() } }

pub struct Session {
    pub working: Settings,
    pub applied: Settings,
    pub attached: bool,
    /// The session has been filled from a host (`sync_from` / `attach_standalone`) since the menu was last closed.
    pub synced: bool,
    pub forced_dirty: bool,
    pub request: Option<ReqKind>,
    pub screens: Vec<Box<dyn super::super::Screen>>,
    pub status: Option<Status>,
    /// Standalone (splash/lobby) mode: where Apply writes.
    pub detached_path: Option<PathBuf>,
    pub applying: bool,
    /// Applied state before the first unsaved live preview (so Discard can roll the game back).
    pub preview_base: Option<Settings>,
    /// Bumped on every sync/apply/discard so pages can drop cached state.
    pub generation: u64,
    /// Gamepad state shown by the Controller tab (copied from `UiContext::pad` every frame).
    pub pad_status: crate::inputdev::pad::PadStatus,
    /// Valid display sizes for the Video tab (`PlatformHost::display_sizes`, refreshed on sync and after every apply).
    pub sizes: Vec<(u32, u32)>,
    /// The monitor's current desktop mode (`PlatformHost::current_display_mode`).
    pub monitor: Option<DisplayMode>,
    /// Segment of a segmented row the mouse is over this frame (row key, segment), reset by the page before it draws its rows.
    pub hover_seg: Option<(&'static str, usize)>,
    /// Footer actions the Brightness/Audio tabs inherit from the previously shown tab: [restore, reset_hud].
    pub inherited: [bool; 2],
}
impl Default for Session { fn default() -> Self { Session::new() } }
impl Session {
    pub fn new() -> Self {
        let d = Settings::default();
        Session {
            working: d.clone(), applied: d, attached: false, synced: false, forced_dirty: false, request: None, screens: Vec::new(), status: None, detached_path: None,
            applying: false, preview_base: None, generation: 0, pad_status: Default::default(), sizes: Vec::new(), monitor: None, hover_seg: None, inherited: [true, false],
        }
    }

    // ---- page-facing API ------------------------------------------------------------------------------------------
    /// Run `f` on the WORKING settings (what the pages show).
    pub fn with_settings<R>(&mut self, f: impl FnOnce(&mut Settings) -> R) -> R { let r = f(&mut self.working); self.working.drain_changes(); r }
    /// Clone of the working settings.
    pub fn settings_clone(&self) -> Settings { self.working.clone() }
    /// Clone of what the game is currently running with.
    pub fn applied_clone(&self) -> Settings { self.applied.clone() }
    /// Flag the screen dirty even though no descriptor value changed (page-private state that Apply must commit).
    pub fn mark_dirty(&mut self) { self.forced_dirty = true; }
    /// Ids of settings (and `HUD.*` extra keys) whose working value differs from the applied one.
    pub fn changed_ids(&self) -> Vec<String> { differences(&self.working, &self.applied) }
    pub fn is_dirty(&self) -> bool { self.forced_dirty || !differences(&self.working, &self.applied).is_empty() }
    /// Same as pressing the Apply button (confirmation flow for risky video changes, then atomic save). No-op when nothing changed.
    pub fn request_apply(&mut self) { self.apply_button(); }
    /// Apply the working values to the running game without saving them and without confirmation (live preview).
    pub fn request_preview(&mut self) { if self.attached && self.request.is_none() { self.request = Some(ReqKind::Preview); } }
    /// Drop all unapplied edits (working := applied). A running (unsaved) preview is rolled back to the last saved state.
    pub fn discard_changes(&mut self) {
        match self.preview_base.take() {
            Some(base) if self.attached => { self.working = base; self.request = Some(ReqKind::Rollback); }
            _ => self.working = self.applied.clone(),
        }
        self.forced_dirty = false; self.generation += 1;
    }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn pad_status(&self) -> crate::inputdev::pad::PadStatus { self.pad_status }
    /// Valid (width, height) modes of the game window's monitor (from the host's `PlatformHost`).
    pub fn display_sizes(&self) -> Vec<(u32, u32)> { self.sizes.clone() }
    /// The monitor's current desktop mode.
    pub fn current_display_mode(&self) -> Option<DisplayMode> { self.monitor }
    pub fn set_status(&mut self, text: &str, kind: StatusKind) { self.status = Some(Status { text: text.to_owned(), kind, age: 0. }); }
    pub fn status(&self) -> Option<Status> { self.status.clone() }
    pub(crate) fn tick_status(&mut self, dt: f32) { if let Some(s) = self.status.as_mut() { s.age += dt; if s.age > 10. && s.kind != StatusKind::Error { self.status = None; } } }
    /// Queue a screen (dialog) to be pushed by the menu after the current frame.
    pub fn push_screen(&mut self, s: Box<dyn super::super::Screen>) { self.screens.push(s); }
    pub fn take_screens(&mut self) -> Vec<Box<dyn super::super::Screen>> { std::mem::take(&mut self.screens) }
    /// True while an apply request is queued/being processed (the footer disables Apply).
    pub fn busy(&self) -> bool { self.request.is_some() }
    pub fn is_attached(&self) -> bool { self.attached }
    pub fn pending_request(&self) -> Option<ReqKind> { self.request }

    // ---- host-facing API ------------------------------------------------------------------------------------------
    /// The game owns these settings: called when the menu opens (and by `pump` after each apply). Discards unapplied edits, like re-opening the original screen.
    pub fn sync_from(&mut self, app: &Settings) {
        self.applied = app.clone(); self.working = app.clone(); self.attached = true; self.synced = true; self.forced_dirty = false; self.preview_base = None; self.status = None; self.generation += 1;
    }
    /// Re-read the monitor's valid sizes and current mode from the platform.
    pub fn refresh_display(&mut self, platform: &mut dyn PlatformHost) {
        let mut m: Vec<(u32, u32)> = platform.display_sizes().into_iter().filter(|&(w, h)| w >= 640 && h >= 360).collect();
        m.sort(); m.dedup();
        self.sizes = m;
        self.monitor = platform.current_display_mode();
    }
    /// Standalone mode (Settings opened where no game exists yet, e.g. a future lobby/splash): load the user's file; Apply then
    /// just saves it. `path` None = never write (tests).
    pub fn attach_standalone(&mut self, path: Option<PathBuf>) {
        let (s, _) = match &path { Some(p) => Settings::load(p), None => (Settings::default(), settings::LoadReport { status: settings::LoadStatus::Missing, path: PathBuf::new(), file_version: None, warnings: vec![], backup: None }) };
        self.applied = s.clone(); self.working = s; self.attached = false; self.synced = true; self.detached_path = path; self.forced_dirty = false; self.generation += 1;
    }
    /// Forget queued requests / dialogs / status (scene start, menu closed).
    pub fn reset_transients(&mut self) { self.request = None; self.screens.clear(); self.status = None; self.hover_seg = None; self.inherited = [true, false]; }

    /// Footer Apply button (and `request_apply`).
    pub(crate) fn apply_button(&mut self) {
        let dirty = self.forced_dirty || !differences(&self.working, &self.applied).is_empty();
        if !dirty || self.request.is_some() { return; }
        if self.attached { self.request = Some(ReqKind::Apply { confirm: true }); self.status = Some(Status { text: "Applying...".into(), kind: StatusKind::Info, age: 0. }); return; }
        // standalone: no game to push values into; commit and save.
        let path = self.detached_path.clone();
        self.applied = self.working.clone(); self.forced_dirty = false; self.generation += 1;
        let r = self.applied.clone();
        match path {
            Some(p) => match r.save(&p) { Ok(()) => self.set_status("Settings saved. They take effect when the game starts.", StatusKind::Ok), Err(e) => self.set_status(&format!("Could not save settings: {e}"), StatusKind::Error) },
            None => self.set_status("Settings kept (not written: no settings file in this run).", StatusKind::Warn),
        }
    }
    /// Restore the given (id, value) pairs in the working copy and queue a Revert apply (keep-video-settings dialog: No / timeout).
    pub(crate) fn revert_values(&mut self, vals: &[(&'static str, Value)]) {
        for (id, v) in vals { let _ = self.working.set(id, *v); }
        self.working.drain_changes(); self.request = Some(ReqKind::Revert); self.generation += 1; self.status = Some(Status { text: "Video settings reverted.".into(), kind: StatusKind::Warn, age: 0. });
    }
    /// The user kept the risky video settings: write them to the file.
    pub(crate) fn confirm_keep(&mut self) { self.request = Some(ReqKind::Save); }

    /// Process the queued request, if any. Called by `MenuSystem::frame` once per frame while the menu is open.
    pub fn pump(&mut self, cx: &mut UiContext) {
        let Some(kind) = self.request.take() else { return };
        let persists = cx.backend.persists();
        match kind {
            ReqKind::Save => {
                let s = cx.settings.clone();
                match cx.backend.write_file(&s) {
                    Ok(()) if persists => self.set_status("Settings applied and saved.", StatusKind::Ok),
                    Ok(()) => self.set_status("Settings applied (automated run: not saved).", StatusKind::Info),
                    Err(e) => self.set_status(&format!("Could not save settings: {e}"), StatusKind::Error),
                }
            }
            ReqKind::Apply { .. } | ReqKind::Preview | ReqKind::Revert | ReqKind::Rollback => {
                let confirm = matches!(kind, ReqKind::Apply { confirm: true });
                let working = self.working.clone();
                let before = cx.settings.clone();
                copy_working_into(&working, cx.settings);
                let report = cx.backend.apply(cx.settings, cx.platform, !matches!(kind, ReqKind::Preview | ReqKind::Rollback));
                let mut after = cx.settings.clone();
                let display_failed = report.failed.iter().any(|(g, _)| *g == "display");
                if display_failed {
                    // The display mode was refused and left alone: put the previous display values back everywhere and re-sync.
                    for id in ["video.window_mode", "video.resolution"] { if let Some(v) = before.get(id) { let _ = cx.settings.set(id, v); } }
                    let r2 = cx.backend.apply(cx.settings, cx.platform, matches!(kind, ReqKind::Apply { .. } | ReqKind::Revert));
                    let _ = r2;
                    after = cx.settings.clone();
                }
                let risky = confirm && !display_failed && report.ok() && is_display_change(&before, &after);
                if risky && persists {
                    // Keep the old values in the file until the user confirms (a crash during the countdown must not strand a bad mode).
                    let _ = cx.backend.write_file(&before);
                }
                // `Handle Apply`: the video confirmation wins; only without it a changed language shows "Restart the game to show all language changes."
                let language_changed = confirm && !risky && !display_failed && report.ok() && before.get("game.language") != after.get("game.language");
                let revert: Vec<(&'static str, Value)> = if risky { ["video.window_mode", "video.resolution"].into_iter().filter_map(|id| settings::entry(id).and_then(|e| before.get(id).map(|v| (e.id, v)))).collect() } else { Vec::new() };
                let n_stored = report.stored_only.len();
                let kind_s = if display_failed {
                    (StatusKind::Error, format!("The display mode was refused by the system and has not been changed ({}).", failed_text(&report)))
                } else if !report.ok() {
                    (StatusKind::Warn, format!("Some settings could not be applied: {}", failed_text(&report)))
                } else if matches!(kind, ReqKind::Preview) {
                    (StatusKind::Info, "Preview (not saved).".to_owned())
                } else if matches!(kind, ReqKind::Rollback) {
                    (StatusKind::Info, "Changes discarded.".to_owned())
                } else if matches!(kind, ReqKind::Revert) {
                    (StatusKind::Warn, "Video settings reverted.".to_owned())
                } else if risky {
                    (StatusKind::Info, "Video settings applied. Keep them?".to_owned())
                } else {
                    let mut t = if persists { "Applied and saved.".to_owned() } else { "Applied (automated run: not saved).".to_owned() };
                    if n_stored > 0 { t.push_str(&format!(" {n_stored} changed setting{} stored only (no effect yet).", if n_stored == 1 { " is" } else { "s are" })); }
                    (StatusKind::Ok, t)
                };
                self.applied = after.clone();
                // A preview is running but NOT saved: keep the screen dirty so the Apply button still saves it.
                if matches!(kind, ReqKind::Preview) {
                    let base = self.preview_base.take().unwrap_or_else(|| before.clone());
                    if differences(&base, &after).is_empty() { self.forced_dirty = false; } else { self.forced_dirty = true; self.preview_base = Some(base); }
                } else { self.working = after.clone(); self.forced_dirty = false; self.preview_base = None; }
                self.generation += 1;
                self.status = Some(Status { text: kind_s.1, kind: kind_s.0, age: 0. });
                if risky { self.screens.push(Box::new(super::dialogs::KeepVideoDialog::new(revert))); }
                else if language_changed { self.screens.push(Box::new(super::dialogs::language_restart_dialog())); }
                // the monitor may have changed (display mode switch / window moved)
                if is_display_change(&before, &after) || display_failed { self.refresh_display(cx.platform); }
            }
        }
    }
}

/// Differences (setting ids + HUD tags such as "HUD.Center.Reticle") between two settings values.
pub fn differences(a: &Settings, b: &Settings) -> Vec<String> {
    let mut v: Vec<String> = a.diff(b).into_iter().map(|c| c.id.to_string()).collect();
    for (sec, t) in extra_keys() { if a.extra(sec, t) != b.extra(sec, t) { v.push(t.to_string()); } }
    v
}

fn is_display_change(before: &Settings, after: &Settings) -> bool {
    before.video.window_mode != after.video.window_mode || (before.video.width, before.video.height) != (after.video.width, after.video.height)
}
/// Copy working values into the game's settings through the validated setter (keeps the apply layer's own bookkeeping).
pub fn copy_working_into(working: &Settings, live: &mut Settings) {
    for c in live.diff(working) { let _ = live.set(c.id, c.new); }
    for (sec, t) in extra_keys() { match working.extra(sec, t) { Some(v) => live.set_extra(sec, t, v), None => live.clear_extra(sec, t) } }
    live.drain_changes();
}
fn failed_text(r: &ApplyReport) -> String { r.failed.iter().map(|(g, e)| format!("{g}: {e}")).collect::<Vec<_>>().join("; ") }
