// Summarise a browser trace written by `launch.mjs --trace` (Chrome trace-event JSON): where the time went per process / thread / event,
// optionally inside a window between two of the page's phase marks (`performance.mark('fnweb:<phase>')`, e.g. stage:Finishing .. stage:Playing).
// usage: node tools/web/test/tracestat.mjs <trace.json> [--from-mark stage:Finishing] [--to-mark stage:Playing] [--top 40] [--match regex]
import fs from 'node:fs';

const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(n); return i >= 0 ? argv[i + 1] : d; };
const file = argv[0];
if (!file) { console.error('usage: node tracestat.mjs <trace.json> [--from-mark m] [--to-mark m] [--top N] [--match re]'); process.exit(2); }
const raw = JSON.parse(fs.readFileSync(file, 'utf8'));
const ev = Array.isArray(raw) ? raw : raw.traceEvents;
const top = Number(opt('--top', '40'));
const match = opt('--match') ? new RegExp(opt('--match')) : null;

// process / thread names
const pname = new Map(), tname = new Map();
for (const e of ev) {
    if (e.ph !== 'M') continue;
    if (e.name === 'process_name') pname.set(e.pid, e.args?.name);
    if (e.name === 'thread_name') tname.set(`${e.pid}:${e.tid}`, e.args?.name);
}
// phase marks of the page
const marks = new Map();
for (const e of ev) if (e.name && e.name.startsWith('fnweb:') && !marks.has(e.name.slice(6))) marks.set(e.name.slice(6), e.ts);
console.log('marks: ' + [...marks].map(([k, v]) => `${k}@${((v - Math.min(...marks.values())) / 1000).toFixed(0)}ms`).join(' '));
const from = opt('--from-mark') ? marks.get(opt('--from-mark')) : -Infinity;
const to = opt('--to-mark') ? marks.get(opt('--to-mark')) : Infinity;
if (from === undefined || to === undefined) { console.error('mark not found'); process.exit(1); }

// complete events ('X') and begin/end pairs, clipped to the window; self time is not computed (nested events overlap), so read totals per name
const stacks = new Map();
const agg = new Map();
const add = (e, ts, dur) => {
    const a = Math.max(ts, from), b = Math.min(ts + dur, to);
    if (b <= a) return;
    const thread = `${pname.get(e.pid) || e.pid} / ${tname.get(`${e.pid}:${e.tid}`) || e.tid}`;
    if (match && !match.test(e.name) && !match.test(thread)) return;
    const k = `${thread} | ${e.cat} | ${e.name}`;
    const r = agg.get(k) || { us: 0, n: 0, max: 0 };
    r.us += b - a; r.n++; r.max = Math.max(r.max, dur);
    agg.set(k, r);
};
for (const e of ev) {
    if (e.ph === 'X') add(e, e.ts, e.dur || 0);
    else if (e.ph === 'B') { const k = `${e.pid}:${e.tid}`; if (!stacks.has(k)) stacks.set(k, []); stacks.get(k).push(e); }
    else if (e.ph === 'E') { const k = `${e.pid}:${e.tid}`; const b = stacks.get(k)?.pop(); if (b) add(b, b.ts, e.ts - b.ts); }
}
const win = (to === Infinity || from === -Infinity) ? '' : ` in ${opt('--from-mark')} .. ${opt('--to-mark')} (${((to - from) / 1000).toFixed(0)} ms)`;
console.log(`top ${top} events by total duration${win} (events nest: totals overlap)`);
for (const [k, r] of [...agg].sort((a, b) => b[1].us - a[1].us).slice(0, top)) {
    console.log(`${(r.us / 1000).toFixed(0).padStart(8)} ms  n=${String(r.n).padStart(6)}  max ${(r.max / 1000).toFixed(1).padStart(7)} ms  ${k.slice(0, 170)}`);
}
