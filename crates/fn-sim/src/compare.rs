//! Line-by-line comparison of two text logs (old exe vs this harness) with a numeric tolerance. Lines are split into tokens on whitespace and
//! `[](),=|;:@`; tokens that parse as numbers compare by absolute difference, all others must be equal.

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub lines_a: usize, pub lines_b: usize, pub compared: usize,
    /// Lines that are textually identical.
    pub exact: usize,
    /// Lines whose tokens all matched within the tolerance (includes `exact`).
    pub within_tol: usize,
    pub max_abs: f64,
    /// (1-based line, a, b, why) of the first mismatching lines (at most 5).
    pub first_mismatches: Vec<(usize, String, String, String)>,
}
impl Report {
    pub fn ok(&self) -> bool { self.lines_a == self.lines_b && self.within_tol == self.compared }
    pub fn summary(&self) -> String {
        format!("{} lines vs {} lines: {} compared, {} identical, {} within tolerance, {} mismatching, max |diff| {:.6}", self.lines_a, self.lines_b, self.compared, self.exact, self.within_tol, self.compared - self.within_tol, self.max_abs)
    }
}
fn tokens(line: &str) -> Vec<&str> { line.split(|c: char| c.is_whitespace() || "[](),=|;:@".contains(c)).filter(|t| !t.is_empty()).collect() }
/// Removes the text from the first occurrence of `from` up to (not including) the first occurrence of `to` after it.
fn drop_between(line: &str, from: &str, to: &str) -> String {
    match line.find(from) { Some(s) => match line[s..].find(to) { Some(e) => format!("{}{}", &line[..s], &line[s + e..]), None => line[..s].to_owned() }, None => line.to_owned() }
}
/// Compares two logs. `drop` = optional (from, to) markers removed from every line of both logs before comparing (e.g. the mesh-based
/// grip numbers that only the old exe can produce). Lines are matched by index; `skip_a` drops leading lines of `a` (headers).
pub fn compare_text(a: &str, b: &str, tol: f64, drop: Option<(&str, &str)>, skip_a: usize) -> Report {
    let la: Vec<&str> = a.lines().skip(skip_a).collect(); let lb: Vec<&str> = b.lines().collect();
    let mut r = Report { lines_a: la.len(), lines_b: lb.len(), ..Default::default() };
    for i in 0..la.len().min(lb.len()) {
        r.compared += 1;
        let (sa, sb) = match drop { Some((f, t)) => (drop_between(la[i], f, t), drop_between(lb[i], f, t)), None => (la[i].to_owned(), lb[i].to_owned()) };
        if sa == sb { r.exact += 1; r.within_tol += 1; continue; }
        let (ta, tb) = (tokens(&sa), tokens(&sb));
        let mut why = None;
        if ta.len() != tb.len() { why = Some(format!("token count {} vs {}", ta.len(), tb.len())); }
        else { for (x, y) in ta.iter().zip(&tb) {
            match (x.parse::<f64>(), y.parse::<f64>()) {
                (Ok(p), Ok(q)) => { let d = (p - q).abs(); if d.is_finite() { r.max_abs = r.max_abs.max(d); } if !(d <= tol) { why = Some(format!("{x} vs {y} (diff {d})")); break; } }
                _ => if x != y { why = Some(format!("{x:?} vs {y:?}")); break; },
            } } }
        match why { None => r.within_tol += 1, Some(w) => if r.first_mismatches.len() < 5 { r.first_mismatches.push((i + 1 + skip_a, sa, sb, w)); } }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn identical_lines_are_exact_and_numbers_get_a_tolerance() {
        let r = compare_text("f 1 pos 1.000 2.000\nf 2 x 3.0", "f 1 pos 1.000 2.000\nf 2 x 3.0004", 0.001, None, 0); assert!(r.ok() && r.exact == 1 && r.within_tol == 2, "{}", r.summary());
        let r = compare_text("f 2 x 3.0", "f 2 x 3.01", 0.001, None, 0); assert!(!r.ok() && r.first_mismatches.len() == 1);
    }
    #[test] fn punctuation_splits_tokens_so_numbers_inside_brackets_compare_numerically() {
        let r = compare_text("bs [A=0.50 B=0.5]", "bs [A=0.5000 B=0.5001]", 0.001, None, 0); assert!(r.ok(), "{}", r.summary());
        let r = compare_text("bs [A=0.5]", "bs [C=0.5]", 1., None, 0); assert!(!r.ok(), "names must match");
    }
    #[test] fn marker_range_is_dropped_and_length_mismatch_fails() {
        let r = compare_text("n frame 3 GRIP a b c height 1", "n frame 3 GRIP height 1", 0., Some(("GRIP", "height")), 0); assert!(r.ok(), "{}", r.summary());
        let r = compare_text("a\nb", "a", 0., None, 0); assert!(!r.ok());
    }
}
