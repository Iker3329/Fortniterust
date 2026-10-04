//! Result model shared by the comparers and the report writers.
use crate::config::{item_grade, Grade};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area { Movement, Camera, Animation, Lighting, Hud, Menus, Loading, Audio, Performance }
impl Area {
    pub const ALL: [Area; 9] = [Area::Movement, Area::Camera, Area::Animation, Area::Lighting, Area::Hud, Area::Menus, Area::Loading, Area::Audio, Area::Performance];
    pub fn name(&self) -> &'static str { match self { Area::Movement => "Movement", Area::Camera => "Camera", Area::Animation => "Animation", Area::Lighting => "Lighting", Area::Hud => "HUD", Area::Menus => "Menus", Area::Loading => "Loading", Area::Audio => "Audio", Area::Performance => "Performance" } }
    pub fn id(&self) -> String { self.name().to_ascii_lowercase() }
}

#[derive(Clone, Debug)]
pub struct Metric { pub name: String, pub value: String, pub grade: Grade }

#[derive(Clone, Debug)]
pub struct Item {
    pub id: String,
    pub area: Area,
    pub group: String,
    pub label: String,
    /// 0..100, `None` = not applicable (excluded from the area score)
    pub score: Option<f64>,
    pub grade: Grade,
    pub weight: f64,
    pub headline: String,
    pub metrics: Vec<Metric>,
    /// path relative to the parity folder (`evidence/...png`)
    pub evidence: Option<String>,
    /// data: URI of a small side-by-side thumbnail
    pub thumb: Option<String>,
    pub note: String,
}
impl Item {
    pub fn new(id: impl Into<String>, area: Area, group: impl Into<String>, label: impl Into<String>) -> Item {
        Item { id: id.into(), area, group: group.into(), label: label.into(), score: None, grade: Grade::Na, weight: 1., headline: String::new(), metrics: vec![], evidence: None, thumb: None, note: String::new() }
    }
    pub fn scored(mut self, score: f64) -> Item { self.score = Some(score); self.grade = item_grade(score); self }
    pub fn na(mut self, why: impl Into<String>) -> Item { self.score = None; self.grade = Grade::Na; self.note = why.into(); self }
    pub fn metric(mut self, name: &str, value: String, grade: Grade) -> Item { self.metrics.push(Metric { name: name.to_owned(), value, grade }); self }
}

/// One executed process (old exe mode, new tool, cargo build, ...).
#[derive(Clone, Debug, Default)]
pub struct RunRecord {
    pub id: String,
    pub side: String,        // "old" | "new" | "build"
    pub cmd: String,
    pub exit: Option<i32>,
    pub timed_out: bool,
    pub wall_s: f64,
    pub cpu_s: f64,
    pub peak_ws_mb: f64,
    pub stragglers_killed: u32,
    pub cursor_released: bool,
    pub ok: bool,
    pub note: String,
    pub out_dir: String,
}

#[derive(Clone, Debug)]
pub struct AreaScore { pub area: Area, pub score: Option<f64>, pub grade: Grade, pub items: usize, pub scored: usize, pub worst: Vec<usize> /* indices into items */, pub note: String }

pub fn area_scores(items: &[Item], cfg: &crate::config::Config) -> Vec<AreaScore> {
    Area::ALL.iter().map(|&a| {
        let idx: Vec<usize> = items.iter().enumerate().filter(|(_, i)| i.area == a).map(|(k, _)| k).collect();
        let scored: Vec<usize> = idx.iter().copied().filter(|&k| items[k].score.is_some() && items[k].weight > 0.).collect();
        let wsum: f64 = scored.iter().map(|&k| items[k].weight).sum();
        let score = if scored.is_empty() { None } else { Some(scored.iter().map(|&k| items[k].score.unwrap() * items[k].weight).sum::<f64>() / wsum) };
        let mut worst = scored.clone();
        worst.sort_by(|&x, &y| items[x].score.partial_cmp(&items[y].score).unwrap_or(std::cmp::Ordering::Equal));
        worst.truncate(5);
        let grade = score.map_or(Grade::Na, |s| crate::config::grade_of(s, cfg));
        let note = if a == Area::Audio && idx.is_empty() { "n/a: the rewrite's audio is the null sink and the old prototype has no audio to compare against".to_owned() } else if scored.is_empty() { "no comparable scenario ran".to_owned() } else { String::new() };
        AreaScore { area: a, score, grade, items: idx.len(), scored: scored.len(), worst, note }
    }).collect()
}
