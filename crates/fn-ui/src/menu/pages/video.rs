//! Video tab of the Settings screen (replaces the generic `video` page).
//!
//! What it adds on top of the generic descriptor rows (`rows::draw_row18`):
//! * the **resolution list is built at run time** from `EnumDisplaySettings` of the monitor the game window is on
//!   (`settings::DisplayInfo::modes`, provided by the platform layer: only modes the monitor reports, 32-bit, non-interlaced, accepted by a `CDS_TEST` probe),
//!   sorted, labelled `"{X}x{Y} {AspectRatio}"` (the recovered `Fortnite.VideoOptions` format) with the current one selected;
//! Window mode / resolution changes go through the core's Apply button, i.e. the original "Do you want to keep these video
//! settings?" dialog with the 15 s revert countdown (`pages::bridge`, `pages::dialogs::KeepVideoDialog`).
//!
//! PROVENANCE. RECOVERED: row labels/order/hover texts (layout_table.rs), window-mode strings, the `"{X}x{Y} {AspectRatio}"` and
//! `"{0} FPS"` formats, the aspect strings 10:16 16:10 16:9 3:4 4:3 9:16. ASSUMED: the other aspect labels (21:9, 32:9, 5:4, 3:2),
//! the frame-rate list 30/60/120/144/165/240/Unlimited, every level -> behaviour mapping (the original scalability table is
//! encrypted). No extra description lines, strips or markers are drawn: only the original rows and hover texts.
use super::{bridge, rows};
use crate::menu::*;
use crate::settings::{self, Settings, Value, WindowMode};

// ---- pure helpers (unit-tested) -------------------------------------------------------------------------------------

fn gcd(a: u32, b: u32) -> u32 { if b == 0 { a } else { gcd(b, a % b) } }

/// Aspect-ratio label of a resolution. The six labels of the original (`AspectRatio-*` strings) come out exactly as recovered; the
/// extra common ones (21:9, 32:9, 5:4, 3:2) are ASSUMED; anything else falls back to the reduced ratio (or `1.85:1` style).
pub fn aspect_label(w: u32, h: u32) -> String {
    if w == 0 || h == 0 { return "?".into(); }
    let r = w as f32 / h as f32;
    let known: [(f32, &str); 10] = [(16. / 9., "16:9"), (16. / 10., "16:10"), (4. / 3., "4:3"), (3. / 4., "3:4"), (10. / 16., "10:16"), (9. / 16., "9:16"), (21. / 9., "21:9"), (32. / 9., "32:9"), (5. / 4., "5:4"), (3. / 2., "3:2")];
    // 21:9 monitors are 64:27 / 43:18 (2.37 / 2.39): allow 3 percent there, 1 percent elsewhere.
    for (k, name) in known { let tol = if name == "21:9" { 0.03 } else { 0.01 }; if ((r - k) / k).abs() <= tol { return name.into(); } }
    let g = gcd(w, h); let (a, b) = (w / g, h / g);
    if a <= 32 && b <= 32 { format!("{a}:{b}") } else { format!("{r:.2}:1") }
}
/// `"{X}x{Y} {AspectRatio}"`, the recovered display-resolution row format.
pub fn resolution_label(w: u32, h: u32) -> String { format!("{w}x{h} {}", aspect_label(w, h)) }
pub fn fps_label(fps: u32) -> String { if fps == 0 { "Unlimited".into() } else { format!("{fps} FPS") } }

/// Result of [`build_resolution_list`].
#[derive(Clone, Debug, PartialEq)]
pub struct ResolutionList {
    pub sizes: Vec<(u32, u32)>,
    pub labels: Vec<String>,
    pub selected: usize,
    /// The saved size is not offered by the monitor and exclusive Fullscreen needs a real mode: the page snaps the setting to `sizes[selected]`.
    pub needs_correction: bool,
    /// The saved size was added to the list (valid windowed size that is not a display mode).
    pub inserted_current: bool,
}
/// The size with the least area difference (ties: the smaller one).
pub fn nearest_size(sizes: &[(u32, u32)], cur: (u32, u32)) -> usize {
    let area = |s: (u32, u32)| s.0 as i64 * s.1 as i64;
    let cur_area = area(cur);
    sizes.iter().enumerate().min_by_key(|(_, s)| ((area(**s) - cur_area).abs(), area(**s))).map(|(i, _)| i).unwrap_or(0)
}
/// Builds the dropdown model from the monitor's display modes. `monitor` = the monitor's current size (windowed sizes must fit on it).
/// Rules: only valid modes are listed, sorted by width then height; the saved size is selected when present; when it is not offered
/// it is inserted for Windowed/WindowedFullscreen if it fits the monitor (any window size is valid there), otherwise the nearest mode is
/// selected and `needs_correction` is set (exclusive Fullscreen would refuse an unlisted mode).
pub fn build_resolution_list(modes: &[(u32, u32)], current: (u32, u32), mode: WindowMode, monitor: Option<(u32, u32)>) -> ResolutionList {
    let mut sizes: Vec<(u32, u32)> = modes.iter().copied().filter(|&(w, h)| w >= 640 && h >= 360).collect();
    sizes.sort_unstable(); sizes.dedup();
    let (mut needs_correction, mut inserted_current) = (false, false);
    let selected;
    if let Some(i) = sizes.iter().position(|s| *s == current) { selected = i; }
    else if sizes.is_empty() {
        // no mode information (headless / enumeration failed): show the saved size so the row is never blank
        sizes.push(current); selected = 0; inserted_current = true;
    } else {
        let fits = current.0 >= 640 && current.1 >= 360 && monitor.map_or(true, |m| current.0 <= m.0 && current.1 <= m.1);
        if mode != WindowMode::Fullscreen && fits { sizes.push(current); sizes.sort_unstable(); selected = sizes.iter().position(|s| *s == current).unwrap(); inserted_current = true; }
        else { selected = nearest_size(&sizes, current); needs_correction = mode == WindowMode::Fullscreen; }
    }
    let labels = sizes.iter().map(|&(w, h)| resolution_label(w, h)).collect();
    ResolutionList { sizes, labels, selected, needs_correction, inserted_current }
}

// ---- page ---------------------------------------------------------------------------------------------------------

pub struct VideoPage {
    st: rows::PageState,
    inner: rows::GenericPage,
    rows: &'static [rows::RowDef],
    modes: Vec<(u32, u32)>,
    monitor: Option<settings::DisplayMode>,
    loaded: bool,
}
impl VideoPage {
    pub fn new() -> Self {
        let rows = super::layout_table::TABS.iter().find(|t| t.id == "video").and_then(|t| t.subs.first()).map_or(&[][..], |s| s.rows);
        Self { st: rows::PageState::default(), inner: rows::GenericPage::new(super::layout_table::TABS.iter().find(|t| t.id == "video").unwrap()), rows, modes: Vec::new(), monitor: None, loaded: false }
    }
    /// Re-read the monitor's modes (page shown, or the window moved to another monitor).
    pub fn refresh_modes(&mut self, sx: &bridge::Session) {
        self.modes = sx.display_sizes();
        self.monitor = sx.current_display_mode();
        self.loaded = true;
    }
    pub fn modes(&self) -> &[(u32, u32)] { &self.modes }
    pub fn monitor_hz(&self) -> u32 { self.monitor.map_or(0, |m| m.hz) }
    fn list_for(&self, snap: &Settings) -> ResolutionList {
        build_resolution_list(&self.modes, (snap.video.width, snap.video.height), snap.video.window_mode, self.monitor.map(|m| (m.width, m.height)))
    }
}

impl SettingsPage for VideoPage {
    fn id(&self) -> &str { "video" }
    fn title(&self) -> &str { "Video" }
    fn order(&self) -> i32 { super::ORDER_VIDEO }
    fn on_show(&mut self) { self.st.desc_row = None; self.loaded = false; self.inner.on_show(); }
    fn reset_prompt(&self) -> Option<Option<String>> { Some(None) }
    fn reset_to_default(&mut self, sx: &mut bridge::Session) { let rows = self.rows; sx.with_settings(|s| for r in rows { rows::reset_row(s, r); }); }
    fn ui(&mut self, ui: &mut Ui, area: Rect) {
        if !self.loaded { self.refresh_modes(ui.session); }
        let snap = ui.session.settings_clone();
        // resolution list model (+ correction of an unlisted exclusive-fullscreen size: never silently apply a mode the monitor would refuse)
        let rl = self.list_for(&snap);
        if rl.needs_correction && !ui.is_inert() { if let Some(&(w, h)) = rl.sizes.get(rl.selected) { ui.session.with_settings(|s| { let _ = s.set("video.resolution", Value::Size(w, h)); }); } }
        // The exact 1.8 rows (TabVideoOptions) are drawn by the generic page (slate.rs widgets); the resolution rotator lists the monitor's modes.
        self.inner.ui(ui, area);
    }
}

// ---- --video-tab-test scenes ----------------------------------------------------------------------------------------
pub const TEST_SCENES: [&str; 8] = ["video", "video-resolution", "video-frame-limit", "video-aa", "video-textures", "brightness-default", "brightness-dark", "brightness-bright"];
fn find_text(m: &MenuSystem, s: &str) -> Option<(f32, f32)> {
    m.parts().0.cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text, .. } = c { if text == s { Some((*x, top + box_h * 0.5)) } else { None } } else { None })
}
fn find_text_prefix(m: &MenuSystem, s: &str) -> Option<(f32, f32)> {
    m.parts().0.cmds.iter().find_map(|c| if let Cmd::Text { x, top, box_h, text, .. } = c { if text.starts_with(s) { Some((*x, top + box_h * 0.5)) } else { None } } else { None })
}
/// Scripts the Settings screen into the Video or Brightness tab (mouse positions are taken from the drawn labels, so every size works).
pub fn test_scene(m: &mut MenuSystem, name: &str, w: f32, h: f32) {
    crate::menu::scripts::test_scene(m, "settings-widgets", w, h);
    {
        let frame = |m: &mut MenuSystem| { m.frame(w, h, 1. / 60.); };
        let mv = |m: &mut MenuSystem, p: (f32, f32)| { m.event(Event::MouseMove(p.0, p.1)); m.frame(w, h, 1. / 60.); };
        let hover = |m: &mut MenuSystem, label: &str| { if let Some((x, y)) = find_text(m, label) { mv(m, (x + w * 0.03, y)); } };
        for _ in 0..2 { frame(m); }
        match name {
            "video" => hover(m, "Quality"),
            "video-frame-limit" => hover(m, "Frame Rate Limit"),
            "video-aa" => hover(m, "Anti-Aliasing"),
            "video-textures" => hover(m, "Textures"),
            "video-resolution" => {
                if let Some((_, y)) = find_text(m, "Display Resolution") {
                    if let Some((vx, vy)) = find_text_prefix(m, "1280x720 ").or_else(|| find_text_prefix(m, "1920x1080 ")) { mv(m, (vx + 10., vy)); m.event(Event::MouseDown(MouseButton::Left, vx + 10., vy)); frame(m); m.event(Event::MouseUp(MouseButton::Left, vx + 10., vy)); frame(m); } else { mv(m, (w * 0.6, y)); }
                }
            }
            n if n.starts_with("brightness") => {
                m.set_settings_tab(2);
                for _ in 0..2 { frame(m); }
                let v = match n { "brightness-dark" => 0.1, "brightness-bright" => 0.9, _ => 0.5 };
                m.session_mut().with_settings(|s| { let _ = s.set("video.brightness", Value::Float(v)); });
            }
            _ => {}
        }
        for _ in 0..3 { frame(m); }
        m.settle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn aspect_labels_use_the_recovered_strings() {
        for (w, h, want) in [(1920, 1080, "16:9"), (1280, 720, "16:9"), (1366, 768, "16:9"), (1920, 1200, "16:10"), (1280, 800, "16:10"), (1024, 768, "4:3"), (1600, 1200, "4:3"), (768, 1024, "3:4"), (720, 1280, "9:16"), (800, 1280, "10:16")] {
            assert_eq!(aspect_label(w, h), want, "{w}x{h}");
        }
        for (w, h, want) in [(2560, 1080, "21:9"), (3440, 1440, "21:9"), (5120, 1440, "32:9"), (1280, 1024, "5:4"), (1440, 960, "3:2")] { assert_eq!(aspect_label(w, h), want, "{w}x{h}"); }
        assert_eq!(aspect_label(0, 5), "?");
        assert_eq!(resolution_label(1920, 1080), "1920x1080 16:9");
        assert!(aspect_label(1000, 777).contains(':'));
    }
    #[test] fn fps_labels() { assert_eq!(fps_label(0), "Unlimited"); assert_eq!(fps_label(144), "144 FPS"); }

    #[test] fn resolution_list_sorted_unique_valid_only_and_current_selected() {
        let modes = [(1920, 1080), (640, 480), (1280, 720), (1920, 1080), (800, 600), (320, 200), (1024, 768), (2560, 1440), (400, 300)];
        let l = build_resolution_list(&modes, (1280, 720), WindowMode::Windowed, Some((2560, 1440)));
        assert_eq!(l.sizes, vec![(640, 480), (800, 600), (1024, 768), (1280, 720), (1920, 1080), (2560, 1440)], "sorted, deduplicated, tiny modes dropped");
        assert_eq!(l.sizes[l.selected], (1280, 720)); assert!(!l.needs_correction && !l.inserted_current);
        assert_eq!(l.labels[l.selected], "1280x720 16:9");
        assert_eq!(l.labels.len(), l.sizes.len());
    }
    #[test] fn unlisted_windowed_size_is_added_only_if_it_fits_the_monitor() {
        let modes = [(1280, 720), (1920, 1080), (2560, 1440)];
        let l = build_resolution_list(&modes, (1600, 900), WindowMode::Windowed, Some((2560, 1440)));
        assert!(l.inserted_current && !l.needs_correction); assert_eq!(l.sizes[l.selected], (1600, 900)); assert_eq!(l.sizes.len(), 4);
        let too_big = build_resolution_list(&modes, (3840, 2160), WindowMode::Windowed, Some((2560, 1440)));
        assert!(!too_big.inserted_current && !too_big.needs_correction, "windowed does not force a correction"); assert_eq!(too_big.sizes[too_big.selected], (2560, 1440), "nearest shown");
        assert_eq!(too_big.sizes.len(), 3, "an oversize window is never offered");
    }
    #[test] fn exclusive_fullscreen_never_offers_an_unlisted_mode() {
        let modes = [(1280, 720), (1920, 1080), (2560, 1440)];
        let l = build_resolution_list(&modes, (1600, 900), WindowMode::Fullscreen, Some((2560, 1440)));
        assert!(l.needs_correction && !l.inserted_current); assert_eq!(l.sizes.len(), 3);
        assert!(modes.contains(&l.sizes[l.selected]), "snaps to a real mode");
        let ok = build_resolution_list(&modes, (1920, 1080), WindowMode::Fullscreen, None);
        assert!(!ok.needs_correction); assert_eq!(ok.sizes[ok.selected], (1920, 1080));
    }
    #[test] fn nearest_size_picks_closest_area() {
        let s = [(640, 480), (1280, 720), (1920, 1080)];
        assert_eq!(nearest_size(&s, (1300, 700)), 1); assert_eq!(nearest_size(&s, (100, 100)), 0); assert_eq!(nearest_size(&s, (9999, 9999)), 2); assert_eq!(nearest_size(&[], (1, 1)), 0);
    }
    #[test] fn empty_mode_list_still_shows_the_saved_size() {
        let l = build_resolution_list(&[], (1280, 720), WindowMode::Windowed, None);
        assert_eq!(l.sizes, vec![(1280, 720)]); assert_eq!(l.selected, 0);
    }
    #[test] fn video_rows_exist_in_the_layout_and_page_registers_under_the_generic_id() {
        let p = VideoPage::new();
        assert!(p.rows.iter().any(|r| r.key == "DisplayResolution") && p.rows.iter().any(|r| r.key == "FrameRateLimit"), "rows come from the recovered layout");
        assert_eq!(p.id(), "video"); assert_eq!(p.order(), super::super::ORDER_VIDEO); assert!(!p.is_fallback());
    }
}
