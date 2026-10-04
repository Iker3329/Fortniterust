//! Diff of the Rust Settings layout against reports/settings-ui-exact.json (tools/settings-ui-layout.cjs: an independent Slate/UMG layout
//! engine run over the decoded 1.8 widget trees) at 1920x1080 and 1280x720, tolerance 1 px.
use super::rows::{self, desired_col_units, page_geo, row_geometry};
use super::screen18::{self, Action};
use super::slate;
use super::visibility::{self, MenuContext};
use crate::menu::*;
use serde_json::Value as J;

fn fonts() -> Fonts { crate::testutil::fonts() }
const TOL: f32 = 1.0;
fn rect_of(j: &J, id: &str) -> Option<Rect> { let a = j.get(id)?.as_array()?; Some(Rect::new(a[0].as_f64()? as f32, a[1].as_f64()? as f32, a[2].as_f64()? as f32, a[3].as_f64()? as f32)) }
fn near(a: Rect, b: Rect) -> bool { (a.x - b.x).abs() <= TOL && (a.y - b.y).abs() <= TOL && (a.w - b.w).abs() <= TOL && (a.h - b.h).abs() <= TOL }
fn scaled(r: Rect, s: f32) -> Rect { Rect::new(r.x * s, r.y * s, r.w * s, r.h * s) }

struct Checker { n: usize, bad: Vec<String> }
impl Checker {
    fn eq(&mut self, scene: &str, vp: &str, rects: &J, id: &str, mine: Rect) {
        self.n += 1;
        match rect_of(rects, id) { Some(r) if near(mine, r) => {} Some(r) => self.bad.push(format!("{vp} {scene} {id}: rust [{:.1},{:.1},{:.1},{:.1}] reference [{:.1},{:.1},{:.1},{:.1}]", mine.x, mine.y, mine.w, mine.h, r.x, r.y, r.w, r.h)), None => self.bad.push(format!("{vp} {scene} {id}: missing in the reference")) }
    }
}

const LABELS: [&str; 7] = ["Video", "Game", "Brightness", "Audio", "Input", "Controller", "Live Streaming"];
const TAB_IDS: [&str; 7] = ["Video", "Game", "Brightness", "Audio", "Input", "Controller", "Account"];

#[test]
fn rust_layout_matches_the_independent_reference_layout_within_one_pixel() {
    let j: J = crate::testutil::report_json("settings-ui-exact.json");
    let f = fonts();
    let mut c = Checker { n: 0, bad: Vec::new() };
    let ctx = MenuContext::default();
    for vp in ["1920x1080", "1280x720"] {
        let v = &j["viewports"][vp];
        let (w, h): (f32, f32) = { let mut it = vp.split('x').map(|x| x.parse::<f32>().unwrap()); (it.next().unwrap(), it.next().unwrap()) };
        let s = ui_layout_scale(w, h);
        assert!((s - v["scale"].as_f64().unwrap() as f32).abs() < 0.01, "scale {s} vs {}", v["scale"]);
        let (vw, vh) = (w / s, h / s);
        let g = screen18::chrome_geo(&f, s, vw, vh, &LABELS);
        for scene in ["video", "game", "hud", "brightness", "audio", "input", "controller", "livestream"] {
            let rects = &v["scenes"][scene]["rects"];
            c.eq(scene, vp, rects, "menu.background", scaled(g.background, s));
            c.eq(scene, vp, rects, "menu.topbar", scaled(g.topbar, s));
            c.eq(scene, vp, rects, "menu.icon", scaled(g.icon, s));
            c.eq(scene, vp, rects, "menu.title", scaled(g.title, s));
            c.eq(scene, vp, rects, "menu.tabs", scaled(g.tabs, s));
            for (i, id) in TAB_IDS.iter().enumerate() { c.eq(scene, vp, rects, &format!("menu.tab.{id}"), scaled(g.tab_rects[i], s)); }
            c.eq(scene, vp, rects, "menu.content", scaled(g.content, s));
            c.eq(scene, vp, rects, "bottom.bar", scaled(g.bottom, s));
            // footer buttons of this scene
            let actions = match scene { "input" => vec![Action::ResetToDefault, Action::Back], "controller" => vec![Action::ToggleMode, Action::Restore, Action::Back], "livestream" => vec![Action::Back], _ => vec![Action::Restore, Action::Back] };
            for (a, (r, t)) in actions.iter().zip(screen18::footer_rects(&f, s, vw, vh, &actions)) {
                let name = match a { Action::Restore => "Restore", Action::Back => "Back", Action::ResetToDefault => "ResetToDefault", Action::ToggleMode => "ToggleMode", Action::Apply => "Apply", Action::ResetHud => "ResetHUDToDefault" };
                c.eq(scene, vp, rects, &format!("bottom.button.{name}"), scaled(r, s));
                let _ = t;
            }
            // rows pages
            let tab = match scene { "video" => "video", "game" | "hud" => "game", "audio" => "audio", _ => continue };
            let area = scaled(g.content, s);
            let t = super::layout_table::TABS.iter().find(|t| t.id == tab).unwrap();
            let sub = if scene == "hud" { &t.subs[1] } else { &t.subs[0] };
            let vis = visibility::visible_rows(tab, sub, &ctx);
            let pg = page_geo(tab, area, s, tab == "game", desired_col_units(tab, &vis));
            if tab == "game" {
                let labels = ["Game", "HUD"];
                let tr = slate::secondary_rects(&f, s, pg.strip.unwrap(), &labels);
                c.eq(scene, vp, rects, "game.subtab.Game", tr[0]);
                c.eq(scene, vp, rects, "game.subtab.HUD", tr[1]);
            }
            // the reference reports one text line of height for the tooltip (its real height depends on the hovered row's wrapped text): compare x, y, width
            let tref = rect_of(rects, "tooltip").unwrap();
            c.eq(scene, vp, rects, "tooltip", Rect::new(pg.tooltip.x, pg.tooltip.y, pg.tooltip.w, tref.h));
            let mut col = slate::Col::new(pg.col_x, pg.top, pg.col_w, s);
            for r in &vis {
                let geo = row_geometry(&mut col, tab, r);
                let key = r.key;
                let (lab, ctl) = slate::row_rects(geo.inner, s);
                c.eq(scene, vp, rects, &format!("row.{key}"), geo.row);
                c.eq(scene, vp, rects, &format!("row.{key}.label"), lab);
                let ctl = if matches!(r.wid, rows::Wid::Segmented) { Rect::new(ctl.x, ctl.y, geo.inner.x + geo.inner.w - ctl.x, ctl.h) } else { ctl };
                c.eq(scene, vp, rects, &format!("row.{key}.control"), ctl);
            }
        }
    }
    assert!(c.n > 300, "only {} rects compared", c.n);
    assert!(c.bad.is_empty(), "{} of {} rects differ by more than {TOL} px:\n{}", c.bad.len(), c.n, c.bad.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

fn ui_layout_scale(w: f32, h: f32) -> f32 { crate::layout::dpi_scale(w, h) }
