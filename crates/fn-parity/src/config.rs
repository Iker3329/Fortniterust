//! Thresholds (TOML) and the 0..100 scoring rule shared by every comparer.
use crate::img::Rect;

pub const DEFAULT_TOML: &str = include_str!("../thresholds.default.toml");

#[derive(Clone, Debug)]
pub struct Config { table: toml::Table, pub source: String }

impl Config {
    pub fn defaults() -> Config { Config { table: DEFAULT_TOML.parse::<toml::Table>().expect("default thresholds parse"), source: "built-in defaults".into() } }
    /// Defaults overlaid with the user's file (keys missing from the file keep their default).
    pub fn load(path: &std::path::Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let user = text.parse::<toml::Table>().map_err(|e| format!("{}: {e}", path.display()))?;
        let mut base = Config::defaults().table;
        merge(&mut base, &user);
        Ok(Config { table: base, source: path.display().to_string() })
    }
    fn get(&self, key: &str) -> Option<&toml::Value> {
        let mut cur: Option<&toml::Value> = None;
        let mut tab = Some(&self.table);
        for part in key.split('.') {
            let v = tab?.get(part)?;
            cur = Some(v);
            tab = v.as_table();
        }
        cur
    }
    pub fn f(&self, key: &str, default: f64) -> f64 { self.get(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).unwrap_or(default) }
    /// `[green, amber]`
    pub fn pair(&self, key: &str) -> Option<(f64, f64)> {
        let a = self.get(key)?.as_array()?;
        let n = |v: &toml::Value| v.as_float().or_else(|| v.as_integer().map(|i| i as f64));
        Some((n(a.first()?)?, n(a.get(1)?)?))
    }
    pub fn pair_or(&self, key: &str, d: (f64, f64)) -> (f64, f64) { self.pair(key).unwrap_or(d) }
    pub fn region(&self, name: &str) -> Option<Rect> { self.region_in("regions", name) }
    pub fn region_in(&self, table: &str, name: &str) -> Option<Rect> {
        let a = self.get(&format!("{table}.{name}"))?.as_array()?;
        let n = |i: usize| a.get(i).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|x| x as f64))).map(|v| v as f32);
        Some(Rect { x0: n(0)?, y0: n(1)?, x1: n(2)?, y1: n(3)? })
    }
    pub fn region_names(&self) -> Vec<String> { self.names_in("regions") }
    pub fn names_in(&self, table: &str) -> Vec<String> { self.get(table).and_then(|v| v.as_table()).map(|t| t.keys().cloned().collect()).unwrap_or_default() }
    pub fn to_json(&self) -> serde_json::Value { serde_json::to_value(&self.table).unwrap_or(serde_json::Value::Null) }
}

fn merge(base: &mut toml::Table, user: &toml::Table) {
    for (k, v) in user {
        match (base.get_mut(k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(u)) => merge(b, u),
            _ => { base.insert(k.clone(), v.clone()); }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grade { Green, Amber, Red, Na }
impl Grade {
    pub fn css(&self) -> &'static str { match self { Grade::Green => "g", Grade::Amber => "a", Grade::Red => "r", Grade::Na => "n" } }
    pub fn name(&self) -> &'static str { match self { Grade::Green => "green", Grade::Amber => "amber", Grade::Red => "red", Grade::Na => "n/a" } }
}

/// Score a "lower is better" value against `(green, amber)`: 100 up to green, 70 at amber, 0 at 4x amber.
pub fn score_lower(v: f64, (g, a): (f64, f64)) -> f64 {
    if !v.is_finite() { return 0.; }
    if v <= g { return 100.; }
    if v <= a { return 100. - 30. * (v - g) / (a - g).max(1e-12); }
    let span = (3. * a.abs()).max(1e-12);
    (70. * (1. - (v - a) / span)).clamp(0., 70.)
}
/// "Higher is better" (SSIM): limits are minimum values.
pub fn score_higher(v: f64, (g, a): (f64, f64)) -> f64 { score_lower(1. - v, (1. - g, 1. - a)) }

pub fn grade_of(score: f64, cfg: &Config) -> Grade {
    if score >= cfg.f("grade.area_green", 90.) { Grade::Green } else if score >= cfg.f("grade.area_amber", 70.) { Grade::Amber } else { Grade::Red }
}
/// An item is green when its value is within the green limit (score 100), amber up to the amber limit (score >= 70), else red.
pub fn item_grade(score: f64) -> Grade { if score >= 99.999 { Grade::Green } else if score >= 70. { Grade::Amber } else { Grade::Red } }

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn defaults_parse_and_expose_regions_and_pairs() {
        let c = Config::defaults();
        assert_eq!(c.pair("image.mae"), Some((4.0, 12.0)));
        assert!(c.region("minimap").is_some() && c.region("nope").is_none());
        assert_eq!(c.pair_or("image.region_mae.sky", (0., 0.)), (6.0, 18.0));
        assert!(c.region_names().len() >= 7);
    }
    #[test] fn scoring_is_piecewise_linear() {
        assert_eq!(score_lower(0.0, (1., 3.)), 100.); assert_eq!(score_lower(1., (1., 3.)), 100.);
        assert!((score_lower(3., (1., 3.)) - 70.).abs() < 1e-9); assert!((score_lower(2., (1., 3.)) - 85.).abs() < 1e-9);
        assert_eq!(score_lower(100., (1., 3.)), 0.); assert!(score_higher(0.96, (0.95, 0.85)) == 100. && score_higher(0.5, (0.95, 0.85)) < 70.);
    }
    #[test] fn user_file_overrides_only_what_it_names() {
        let p = std::env::temp_dir().join("fn-parity-cfg-test.toml"); std::fs::write(&p, "[image]\nmae = [1.0, 2.0]\n").unwrap();
        let c = Config::load(&p).unwrap(); assert_eq!(c.pair("image.mae"), Some((1., 2.))); assert_eq!(c.pair("image.ssim"), Some((0.95, 0.85)));
    }
}
