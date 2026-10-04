//! Numeric diff of per-frame / per-capture text logs written by both executables. Lines are matched by index; every token is either a number
//! (classified by the word in front of it: position, bone, velocity, angle, camera eye, animation weight, ...) or a word (animation state names,
//! cardinal directions, ...) which must be equal.
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub struct Cat {
    pub n: usize, pub sum: f64, pub max: f64, pub max_line: usize, pub max_label: String, pub over: usize,
}
impl Cat { pub fn mean(&self) -> f64 { if self.n == 0 { 0. } else { self.sum / self.n as f64 } } }

#[derive(Clone, Debug, Default)]
pub struct LogDiff {
    pub lines_a: usize, pub lines_b: usize, pub compared: usize, pub identical_lines: usize,
    pub word_tokens: usize, pub word_mismatches: usize,
    pub first_word_mismatch: Option<(usize, String, String)>,
    /// 1-based numbers of the first differing lines (for the evidence excerpt)
    pub first_diff_lines: Vec<usize>,
    /// the prepared (drop / skip applied) lines of both logs, for excerpts
    pub prepped: (Vec<String>, Vec<String>),
    /// category -> stats (error already converted to the category's unit: cm for position / bone / eye)
    pub cats: BTreeMap<String, Cat>,
}
impl LogDiff {
    pub fn word_mismatch_pct(&self) -> f64 { if self.word_tokens == 0 { 0. } else { self.word_mismatches as f64 * 100. / self.word_tokens as f64 } }
    pub fn length_pct(&self) -> f64 { let m = self.lines_a.max(self.lines_b); if m == 0 { 0. } else { self.lines_a.abs_diff(self.lines_b) as f64 * 100. / m as f64 } }
}

fn is_sep(c: char) -> bool { c.is_whitespace() || "[](),=|;:@".contains(c) }
/// Number with an optional unit suffix (`0.027m`, `1.2s`, `5%`).
fn num(tok: &str) -> Option<f64> {
    if let Ok(v) = tok.parse::<f64>() { return if v.is_finite() { Some(v) } else { None }; }
    for suf in ["ms", "m", "s", "%", "x"] { if let Some(p) = tok.strip_suffix(suf) { if p.chars().last().map_or(false, |c| c.is_ascii_digit() || c == '.') { if let Ok(v) = p.parse::<f64>() { if v.is_finite() { return Some(v); } } } } }
    None
}

/// (category, scale to the category's unit). Positions in the logs are metres, so position / bone / eye errors are reported in centimetres.
pub fn classify(label: &str) -> (&'static str, f64) {
    let l = label.to_ascii_lowercase();
    let has = |ks: &[&str]| ks.iter().any(|k| l.contains(k));
    if matches!(l.as_str(), "f" | "frame" | "frames" | "line" | "n" | "ticks") { return ("frame", 1.); }
    if has(&["eye"]) { return ("eye_cm", 100.); }
    if has(&["foot", "pelvis", "hand", "bone", "weapon", "head", "spine", "grip", "along", "thigh", "clavicle", "neck", "butt", "residual", "gap"]) { return ("bone_cm", 100.); }
    if matches!(l.as_str(), "pos" | "x" | "y" | "z" | "feet" | "origin") || l.starts_with("pos") { return ("position_cm", 100.); }
    if has(&["vel", "speed", "vz"]) && !has(&["yaw"]) { return ("velocity", 1.); }
    if has(&["yaw", "pitch", "facing", "roll"]) { return ("angle_deg", 1.); }
    if has(&["mix", "blend", "weight", "fall_rate", "alpha", "equip", "boom", "hide", "time"]) || matches!(l.as_str(), "t" | "bs" | "add" | "full") { return ("anim_weight", 1.); }
    ("other", 1.)
}

/// Splits a line into (label, token) pairs; numbers get the last word seen in front of them as their label.
fn tokens(line: &str) -> Vec<(String, String)> {
    let mut out = Vec::new(); let mut label = String::new();
    for t in line.split(is_sep).filter(|t| !t.is_empty()) {
        if num(t).is_some() { out.push((label.clone(), t.to_owned())); }
        else { label = t.to_owned(); out.push((String::new(), t.to_owned())); }
    }
    out
}

/// Compare log `a` (reference = the old exe) with `b` (new). `drop` removes from every line the text between two markers (like `fn-sim compare`),
/// `skip_lines_containing` removes whole lines (e.g. window-size sweep lines that only one side can produce).
pub fn diff(a: &str, b: &str, drop: Option<(&str, &str)>, skip_lines_containing: Option<&str>) -> LogDiff {
    let prep = |t: &str| -> Vec<String> {
        t.lines().filter(|l| skip_lines_containing.map_or(true, |p| !l.contains(p))).map(|l| match drop { Some((f, to)) => drop_between(l, f, to), None => l.to_owned() }).collect()
    };
    let (la, lb) = (prep(a), prep(b));
    let mut r = LogDiff { lines_a: la.len(), lines_b: lb.len(), ..Default::default() };
    for i in 0..la.len().min(lb.len()) {
        r.compared += 1;
        if la[i] == lb[i] { r.identical_lines += 1; continue; }
        if r.first_diff_lines.len() < 8 { r.first_diff_lines.push(i + 1); }
        let (ta, tb) = (tokens(&la[i]), tokens(&lb[i]));
        for (k, (x, y)) in ta.iter().zip(tb.iter()).enumerate() {
            let _ = k;
            match (num(&x.1), num(&y.1)) {
                (Some(p), Some(q)) => {
                    let (cat, scale) = classify(if x.0.is_empty() { &y.0 } else { &x.0 });
                    let c = r.cats.entry(cat.to_owned()).or_default();
                    let mut e = (p - q).abs() * scale;
                    if cat == "angle_deg" { e %= 360.; if e > 180. { e = 360. - e; } }
                    c.n += 1; c.sum += e;
                    if e > c.max { c.max = e; c.max_line = i + 1; c.max_label = x.0.clone(); }
                    if e > 1e-9 { c.over += 1; }
                }
                (None, None) => {
                    r.word_tokens += 1;
                    if x.1 != y.1 { r.word_mismatches += 1; if r.first_word_mismatch.is_none() { r.first_word_mismatch = Some((i + 1, x.1.clone(), y.1.clone())); } }
                }
                _ => { r.word_tokens += 1; r.word_mismatches += 1; if r.first_word_mismatch.is_none() { r.first_word_mismatch = Some((i + 1, x.1.clone(), y.1.clone())); } }
            }
        }
        if ta.len() != tb.len() { r.word_tokens += 1; r.word_mismatches += 1; if r.first_word_mismatch.is_none() { r.first_word_mismatch = Some((i + 1, format!("{} tokens", ta.len()), format!("{} tokens", tb.len()))); } }
    }
    // numeric categories also count matching (zero-error) tokens so means are meaningful
    for l in 0..r.compared.min(la.len()) { if la[l] == lb[l] { for (lab, t) in tokens(&la[l]) { if num(&t).is_some() { let (cat, _) = classify(&lab); r.cats.entry(cat.to_owned()).or_default().n += 1; } else { r.word_tokens += 1; } } } }
    r.prepped = (la, lb);
    r
}

/// Excerpt of both logs around the first differences and the worst line of every category (written next to the report as evidence).
pub fn excerpt(d: &LogDiff, title: &str) -> String {
    let mut out = format!("{title}\nold: {} lines, new: {} lines, {} compared, {} identical\n\n", d.lines_a, d.lines_b, d.compared, d.identical_lines);
    let mut lines: Vec<(usize, String)> = d.first_diff_lines.iter().map(|&l| (l, "first difference".to_owned())).collect();
    for (cat, c) in &d.cats { if c.max_line > 0 && c.max > 0. { lines.push((c.max_line, format!("worst {cat}: {:.4}", c.max))); } }
    lines.sort(); lines.dedup_by_key(|x| x.0);
    for (l, why) in lines {
        let (a, b) = (d.prepped.0.get(l - 1), d.prepped.1.get(l - 1));
        out.push_str(&format!("--- line {l} ({why})\nold: {}\nnew: {}\n", a.map_or("-", |s| s.as_str()), b.map_or("-", |s| s.as_str())));
    }
    out
}

fn drop_between(line: &str, from: &str, to: &str) -> String {
    match line.find(from) { Some(s) => match line[s..].find(to) { Some(e) => format!("{}{}", &line[..s], &line[s + e..]), None => line[..s].to_owned() }, None => line.to_owned() }
}

/// First number following `key` in `text` (e.g. `liftoff t=` -> 0.017).
pub fn number_after(text: &str, key: &str) -> Option<f64> {
    let i = text.find(key)? + key.len();
    let rest = text[i..].trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+' || c == 'e')).unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn identical_logs_have_no_error_and_words_match() {
        let t = "f 1 yaw 0.0 vel 0 | foot_l 0.217 0.094 0.175 | lower Idles full 0\nf 2 yaw 0.0 vel 0 | foot_l 0.217 0.094 0.175 | lower Idles full 0";
        let d = diff(t, t, None, None); assert_eq!(d.compared, 2); assert_eq!(d.word_mismatches, 0); assert!(d.cats.values().all(|c| c.max == 0.));
        assert!(d.cats.contains_key("bone_cm") && d.cats.contains_key("angle_deg"));
    }
    #[test] fn bone_error_is_in_centimetres_and_state_names_are_compared() {
        let a = "f 1 foot_l 0.200 0.100 0.100 lower Idles"; let b = "f 1 foot_l 0.210 0.100 0.100 lower Walks";
        let d = diff(a, b, None, None); let c = &d.cats["bone_cm"]; assert!((c.max - 1.0).abs() < 1e-6, "{}", c.max); assert_eq!(d.word_mismatches, 1);
        assert_eq!(d.first_word_mismatch.as_ref().unwrap().1, "Idles");
    }
    #[test] fn unit_suffixes_and_length_mismatch() {
        assert_eq!(num("0.027m"), Some(0.027)); assert_eq!(num("abc"), None); assert_eq!(classify("eye").0, "eye_cm"); assert_eq!(classify("pos").0, "position_cm");
        let d = diff("a 1\na 2\na 3", "a 1\na 2", None, None); assert!(d.length_pct() > 30.);
    }
    #[test] fn number_after_reads_the_value() { assert_eq!(number_after("liftoff t=0.017; apex", "liftoff t="), Some(0.017)); assert_eq!(number_after("x", "y"), None); }
}
