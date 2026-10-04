//! `parity/report.html` (self-contained: inline CSS, inline PNG thumbnails as data URIs) and `parity/summary.json`.
use crate::config::*;
use crate::consts::ConstRow;
use crate::model::*;
use serde_json::json;

pub struct Report<'a> {
    pub generated: String,
    pub items: &'a [Item],
    pub runs: &'a [RunRecord],
    pub warnings: &'a [String],
    pub gaps: &'a [String],
    pub consts: &'a [ConstRow],
    pub cfg: &'a Config,
    pub thresholds_toml: String,
    pub old_exe_info: String,
    pub new_exe_info: String,
}

pub fn esc(s: &str) -> String { s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;") }

fn grade_css(g: Grade) -> &'static str { g.css() }
fn fmt_score(s: Option<f64>) -> String { s.map_or("n/a".to_owned(), |v| format!("{v:.1}")) }

pub fn summary_json(r: &Report) -> serde_json::Value {
    let areas = area_scores(r.items, r.cfg);
    let item_json = |i: &Item| json!({
        "id": i.id, "area": i.area.name(), "group": i.group, "label": i.label, "score": i.score, "grade": i.grade.name(), "weight": i.weight,
        "headline": i.headline, "note": i.note, "evidence": i.evidence,
        "metrics": i.metrics.iter().map(|m| json!({"name": m.name, "value": m.value, "grade": m.grade.name()})).collect::<Vec<_>>(),
    });
    let scored: Vec<f64> = areas.iter().filter_map(|a| a.score).collect();
    let overall = if scored.is_empty() { None } else { Some(scored.iter().sum::<f64>() / scored.len() as f64) };
    json!({
        "generated": r.generated,
        "overall_score": overall,
        "old_exe": r.old_exe_info, "new_exe": r.new_exe_info,
        "areas": areas.iter().map(|a| json!({
            "area": a.area.name(), "score": a.score, "grade": a.grade.name(), "items": a.items, "scored_items": a.scored, "note": a.note,
            "worst": a.worst.iter().map(|&k| item_json(&r.items[k])).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "items": r.items.iter().map(item_json).collect::<Vec<_>>(),
        "runs": r.runs.iter().map(|x| json!({"id": x.id, "side": x.side, "cmd": x.cmd, "exit": x.exit, "timed_out": x.timed_out, "wall_s": x.wall_s, "cpu_s": x.cpu_s, "peak_ws_mb": x.peak_ws_mb, "stragglers_killed": x.stragglers_killed, "cursor_released": x.cursor_released, "ok": x.ok, "note": x.note})).collect::<Vec<_>>(),
        "warnings": r.warnings, "not_covered": r.gaps,
        "constants": r.consts.iter().filter(|c| c.verdict == "DRIFT" || c.verdict == "assumption-changed").map(|c| json!({"group": c.group, "constant": c.constant, "verdict": c.verdict, "new": c.new, "expected": c.expected, "report_status": c.report_status})).collect::<Vec<_>>(),
        "thresholds": r.cfg.to_json(),
    })
}

const CSS: &str = r#"
:root{--bg:#f6f7f9;--fg:#1b1f24;--mut:#5b6571;--card:#fff;--line:#d9dee5;--g:#2e9e4f;--a:#d99a12;--r:#d24b3e;--n:#8b95a1;--gbg:#e3f4e8;--abg:#fbf0d4;--rbg:#f9e0dd;--nbg:#eceff2;--link:#1f5fbf}
@media (prefers-color-scheme:dark){:root{--bg:#14171b;--fg:#e6e9ed;--mut:#9aa5b1;--card:#1d2127;--line:#303741;--gbg:#173523;--abg:#3a2f12;--rbg:#40201c;--nbg:#262b32;--link:#7fb0ff}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.45 system-ui,Segoe UI,sans-serif}
main{max-width:1280px;margin:0 auto;padding:20px 16px 60px}h1{font-size:22px;margin:0 0 4px}h2{font-size:17px;margin:28px 0 8px}h3{font-size:14px;margin:0}
.sub{color:var(--mut);margin:0 0 16px}a{color:var(--link)}code{font:12px ui-monospace,Consolas,monospace}
table{border-collapse:collapse;width:100%;background:var(--card);border:1px solid var(--line);border-radius:8px;overflow:hidden}
th,td{padding:6px 10px;border-bottom:1px solid var(--line);text-align:left;vertical-align:top}th{font-size:12px;color:var(--mut);font-weight:600;background:var(--nbg)}tr:last-child td{border-bottom:0}
.s{display:inline-block;min-width:56px;text-align:center;padding:2px 8px;border-radius:12px;font-weight:700;color:#fff}.s.g{background:var(--g)}.s.a{background:var(--a)}.s.r{background:var(--r)}.s.n{background:var(--n)}
.chip{display:inline-block;margin:1px 4px 1px 0;padding:1px 7px;border-radius:10px;font-size:12px;border:1px solid var(--line)}.chip.g{background:var(--gbg)}.chip.a{background:var(--abg)}.chip.r{background:var(--rbg)}.chip.n{background:var(--nbg)}
.thumb{display:block;max-width:300px;width:100%;image-rendering:auto;border:1px solid var(--line);border-radius:4px}
.warn{background:var(--abg);border:1px solid var(--a);border-radius:8px;padding:8px 14px;margin:12px 0}.warn li{margin:2px 0}
details{margin:10px 0}summary{cursor:pointer;padding:8px 10px;background:var(--card);border:1px solid var(--line);border-radius:8px;display:flex;gap:10px;align-items:center}
.note{color:var(--mut);font-size:12px}.legend{display:flex;gap:14px;flex-wrap:wrap;color:var(--mut);font-size:12px;margin:6px 0 12px}
.legend i{display:inline-block;width:12px;height:12px;border-radius:3px;vertical-align:-2px;margin-right:4px}
@media (max-width:700px){th:nth-child(1),td:nth-child(1){display:none}}
"#;

pub fn html(r: &Report) -> String {
    let areas = area_scores(r.items, r.cfg);
    let mut h = String::with_capacity(1 << 20);
    h.push_str("<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>Parity dashboard</title><style>");
    h.push_str(CSS);
    h.push_str("</style></head><body><main>");
    let scored: Vec<f64> = areas.iter().filter_map(|a| a.score).collect();
    let overall = if scored.is_empty() { None } else { Some(scored.iter().sum::<f64>() / scored.len() as f64) };
    h.push_str(&format!("<h1>Parity dashboard: pure-Rust game vs old prototype</h1><p class=sub>Generated {} &middot; old exe {} &middot; new exe {} &middot; overall (mean of scored areas) <span class='s {}'>{}</span></p>",
        esc(&r.generated), esc(&r.old_exe_info), esc(&r.new_exe_info), overall.map_or("n", |s| grade_css(grade_of(s, r.cfg))), fmt_score(overall)));
    h.push_str(&format!("<div class=legend><span><i style=background:var(--g)></i>green &ge; {:.0} (item: within the green limit)</span><span><i style=background:var(--a)></i>amber &ge; {:.0}</span><span><i style=background:var(--r)></i>red below</span><span><i style=background:var(--n)></i>n/a, not compared</span><span>Thumbnails: <b>old | new | difference heatmap</b> (blue / green / red top bars); click for the full-size sheet.</span></div>", r.cfg.f("grade.area_green", 90.), r.cfg.f("grade.area_amber", 70.)));
    if !r.warnings.is_empty() { h.push_str("<div class=warn><b>Harness warnings</b><ul>"); for w in r.warnings { h.push_str(&format!("<li>{}</li>", esc(w))); } h.push_str("</ul></div>"); }

    // summary
    h.push_str("<h2>Summary per area</h2><table><tr><th>Area</th><th>Score</th><th>Items</th><th>Worst offenders</th></tr>");
    for a in &areas {
        h.push_str(&format!("<tr><td><a href=#a-{}><b>{}</b></a></td><td><span class='s {}'>{}</span></td><td>{} scored / {}</td><td>", a.area.id(), a.area.name(), grade_css(a.grade), fmt_score(a.score), a.scored, a.items));
        if a.worst.is_empty() { h.push_str(&format!("<span class=note>{}</span>", esc(&a.note))); }
        for &k in &a.worst.iter().take(3).copied().collect::<Vec<_>>() {
            let i = &r.items[k];
            let label = format!("{} ({:.0}): {}", i.label, i.score.unwrap_or(0.), i.headline);
            match &i.evidence { Some(e) => h.push_str(&format!("<div><a href=\"{}\">{}</a></div>", esc(e), esc(&label))), None => h.push_str(&format!("<div>{}</div>", esc(&label))) }
        }
        h.push_str("</td></tr>");
    }
    h.push_str("</table>");

    // per area
    for a in &areas {
        let mut idx: Vec<usize> = r.items.iter().enumerate().filter(|(_, i)| i.area == a.area).map(|(k, _)| k).collect();
        idx.sort_by(|&x, &y| r.items[x].score.unwrap_or(1000.).partial_cmp(&r.items[y].score.unwrap_or(1000.)).unwrap_or(std::cmp::Ordering::Equal));
        h.push_str(&format!("<details id=a-{} {}><summary><h3>{}</h3><span class='s {}'>{}</span><span class=note>{} items, {} scored{}</span></summary>", a.area.id(), if a.grade == Grade::Green { "" } else { "open" }, a.area.name(), grade_css(a.grade), fmt_score(a.score), a.items, a.scored, if a.note.is_empty() { String::new() } else { format!(" &middot; {}", esc(&a.note)) }));
        if idx.is_empty() { h.push_str("</details>"); continue; }
        h.push_str("<table><tr><th>Evidence</th><th>Item</th><th>Measured</th><th>Score</th></tr>");
        for k in idx {
            let i = &r.items[k];
            h.push_str("<tr><td style=width:310px>");
            if let Some(t) = &i.thumb { match &i.evidence { Some(e) => h.push_str(&format!("<a href=\"{}\"><img class=thumb alt=\"{}\" src=\"{}\"></a>", esc(e), esc(&i.label), t)), None => h.push_str(&format!("<img class=thumb alt=\"\" src=\"{t}\">")) } }
            if i.thumb.is_none() { if let Some(e) = &i.evidence { h.push_str(&format!("<a href=\"{}\">log excerpt</a>", esc(e))); } }
            h.push_str(&format!("</td><td><b>{}</b><div class=note>{} &middot; {}</div>", esc(&i.label), esc(&i.group), esc(&i.id)));
            if !i.note.is_empty() { h.push_str(&format!("<div class=note>{}</div>", esc(&i.note))); }
            h.push_str("</td><td>");
            h.push_str(&format!("<div>{}</div>", esc(&i.headline)));
            for m in &i.metrics { h.push_str(&format!("<span class='chip {}'>{}: {}</span>", grade_css(m.grade), esc(&m.name), esc(&m.value))); }
            h.push_str(&format!("</td><td><span class='s {}'>{}</span>{}</td></tr>", grade_css(i.grade), fmt_score(i.score), if (i.weight - 1.).abs() > 1e-9 { format!("<div class=note>weight {}</div>", i.weight) } else { String::new() }));
        }
        h.push_str("</table></details>");
    }

    // constants
    let drift: Vec<&ConstRow> = r.consts.iter().filter(|c| c.verdict == "DRIFT" || c.verdict == "assumption-changed").collect();
    h.push_str("<h2>Recovered 1.8 constants (reports/verify-1.8.json) re-evaluated against the Rust crates</h2>");
    let cnt = |v: &str| r.consts.iter().filter(|c| c.verdict == v).count();
    h.push_str(&format!("<p class=note>{} rows: {} ok, {} DRIFT, {} assumptions kept, {} assumptions changed, {} not exposed by a Rust crate yet (not counted as passing).</p>", r.consts.len(), cnt("ok"), cnt("DRIFT"), cnt("same-assumption"), cnt("assumption-changed"), cnt("not-exposed")));
    if !drift.is_empty() {
        h.push_str("<table><tr><th>Group</th><th>Constant</th><th>Verdict</th><th>Rust value</th><th>Expected</th></tr>");
        for c in drift { h.push_str(&format!("<tr><td>{}</td><td><code>{}</code></td><td><span class='s {}'>{}</span></td><td>{:?}</td><td>{:?}</td></tr>", esc(&c.group), esc(&c.constant), if c.verdict == "DRIFT" { "r" } else { "a" }, c.verdict, c.new, c.expected)); }
        h.push_str("</table>");
    }

    // runs
    h.push_str("<h2>Runs</h2><table><tr><th>Side</th><th>Run</th><th>Result</th><th>Wall</th><th>CPU</th><th>Peak WS</th><th>Notes</th></tr>");
    for x in r.runs {
        let (cls, txt) = if x.timed_out { ("r", "timeout".to_owned()) } else if x.ok { ("g", "ok".to_owned()) } else { ("a", format!("exit {:?}", x.exit)) };
        h.push_str(&format!("<tr><td>{}</td><td><code>{}</code><div class=note>{}</div></td><td><span class='s {cls}'>{txt}</span></td><td>{:.1}s</td><td>{:.1}s</td><td>{:.0} MB</td><td class=note>{}{}{}</td></tr>", esc(&x.side), esc(&x.id), esc(&x.cmd), x.wall_s, x.cpu_s, x.peak_ws_mb, esc(&x.note), if x.stragglers_killed > 0 { " stragglers killed;" } else { "" }, if x.cursor_released { " cursor clip released;" } else { "" }));
    }
    h.push_str("</table>");

    // gaps
    h.push_str("<h2>What the scenarios could not cover</h2><ul>");
    for g in r.gaps { h.push_str(&format!("<li>{}</li>", esc(g))); }
    h.push_str("</ul>");
    h.push_str(&format!("<h2>Thresholds in use</h2><details><summary>thresholds ({})</summary><pre><code>{}</code></pre></details>", esc(&r.cfg.source), esc(&r.thresholds_toml)));
    h.push_str("</main></body></html>");
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn report_renders_and_escapes() {
        let cfg = Config::defaults();
        let mut it = Item::new("img:a<b", Area::Hud, "g", "label & co").scored(100.); it.headline = "x".into();
        let items = vec![it];
        let rep = Report { generated: "now".into(), items: &items, runs: &[], warnings: &["w <1>".to_owned()], gaps: &["gap".to_owned()], consts: &[], cfg: &cfg, thresholds_toml: "t".into(), old_exe_info: "o".into(), new_exe_info: "n".into() };
        let h = html(&rep); assert!(h.contains("label &amp; co") && h.contains("w &lt;1&gt;") && h.contains("Audio") && !h.contains("<1>"));
        let j = summary_json(&rep); assert_eq!(j["areas"].as_array().unwrap().len(), 9); assert_eq!(j["items"][0]["grade"], "green");
    }
}
