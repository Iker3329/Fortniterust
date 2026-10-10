// Summarise a V8 CPU profile of the browser build's main thread (written by `launch.mjs --cpu-profile`; DevTools can open the same file).
// usage: node tools/web/test/cpuprof.mjs <main-thread.cpuprofile> [--focus <regex>] [--from <ms> --to <ms>] [--top 40] [--min 1.0] [--depth 14]
//        [--frames <ms>]   (also list every main-thread task of at least <ms> with what it did: the freezes)
//   --focus   only samples whose stack has a frame matching the regex; the top-down tree starts at the outermost such frame
//   --from/to a window in ms relative to the profile start
//   prints: the total, self time per function, time per category (the most specific category on each sample's stack), a top-down tree
import fs from 'node:fs';

const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(n); return i >= 0 ? argv[i + 1] : d; };
const file = argv.find((a, i) => !a.startsWith('--') && (i === 0 || !argv[i - 1].startsWith('--')));
if (!file) { console.error('usage: node cpuprof.mjs <file.cpuprofile> [--focus re] [--from ms --to ms] [--top N] [--min pct] [--depth N]'); process.exit(2); }
const prof = JSON.parse(fs.readFileSync(file, 'utf8'));
const focus = opt('--focus') ? new RegExp(opt('--focus')) : null;
const from = Number(opt('--from', '0')), to = Number(opt('--to', 'Infinity'));
const top = Number(opt('--top', '40')), minPct = Number(opt('--min', '1.0')), maxDepth = Number(opt('--depth', '14'));

const byId = new Map(prof.nodes.map((n) => [n.id, n]));
const parent = new Map();
for (const n of prof.nodes) for (const c of n.children || []) parent.set(c, n.id);
const name = (n) => {
    const cf = n.callFrame; let f = cf.functionName || '(anonymous)';
    // legacy Rust mangling in the wasm name section: _ZN<len><ident>...17h<hash>E -> a::b::c
    if (/^_ZN/.test(f)) { const parts = []; let s = f.slice(3); while (/^\d/.test(s)) { const m = s.match(/^(\d+)/); const len = Number(m[1]); s = s.slice(m[1].length); parts.push(s.slice(0, len)); s = s.slice(len); } if (parts.length && /^h[0-9a-f]{16}$/.test(parts[parts.length - 1])) parts.pop(); f = parts.join('::').replace(/\$LT\$/g, '<').replace(/\$GT\$/g, '>').replace(/\$u20\$/g, ' ').replace(/\$u7b\$/g, '{').replace(/\$u7d\$/g, '}').replace(/\$RF\$/g, '&').replace(/\$C\$/g, ',').replace(/\.\./g, '::'); }
    if (cf.url && !cf.url.startsWith('wasm://')) f += ` (${cf.url.split('/').pop()}:${cf.lineNumber + 1})`;
    return f;
};
const names = new Map(prof.nodes.map((n) => [n.id, name(n)]));
const stackOf = (id) => { const s = []; for (let x = id; x !== undefined; x = parent.get(x)) s.push(x); return s; }; // leaf first

const CATS = [
    ['pipeline create (JS createRenderPipeline/createComputePipeline + wgpu)', /createRenderPipeline|createComputePipeline|create_render_pipeline|create_compute_pipeline/],
    ['shader module create (JS createShaderModule + wgpu/naga)', /createShaderModule|create_shader_module/],
    ['DXBC -> WGSL translate (fn_dxbc)', /fn_dxbc/],
    ['naga (outside shader-module create)', /naga::/],
    ['texture decode (fn_texture)', /fn_texture/],
    ['GPU uploads (writeTexture / writeBuffer / createTexture)', /writeTexture|writeBuffer|write_texture|write_buffer|createTexture|create_texture/],
    ['bind groups / layouts', /createBindGroup|create_bind_group|createPipelineLayout|create_pipeline_layout|BindGroupLayout/],
    ['package parse (fn_ueio)', /fn_ueio/],
    ['pak read / inflate (fn_pak, zlib)', /fn_pak|miniz|inflate|zlib/],
    ['file / http (fn_web_rt vfs)', /fn_web_rt::vfs|fnweb_http|XMLHttpRequest/],
    ['material (fn_fx::material)', /fn_fx::material/],
    ['allocator / memcpy', /dlmalloc|^malloc$|^free$|^realloc$|memcpy|memmove|memset/],
    ['(garbage collector)', /^\(garbage collector\)$/],
    ['(program) = browser native work', /^\(program\)$/],
    ['(idle)', /^\(idle\)$/],
];

let t = prof.startTime;
const ts = prof.timeDeltas.map((d) => (t += d));
const self = new Map(), cats = new Map();
const tree = { name: '(root)', us: 0, kids: new Map() };
let total = 0;
for (let i = 0; i < prof.samples.length; i++) {
    const at = (ts[i] - prof.startTime) / 1000;
    if (at < from || at > to) continue;
    const w = i + 1 < ts.length ? ts[i + 1] - ts[i] : 0;
    const st = stackOf(prof.samples[i]);
    const ns = st.map((id) => names.get(id));
    let start = ns.length - 1; // root end
    if (focus) { const k = ns.findLastIndex((n) => focus.test(n)); if (k < 0) continue; start = k; }
    total += w;
    self.set(ns[0], (self.get(ns[0]) || 0) + w);
    const cat = (CATS.find(([, re]) => ns.some((n) => re.test(n))) || ['other'])[0];
    cats.set(cat, (cats.get(cat) || 0) + w);
    let node = tree; node.us += w;
    for (let k = start, d = 0; k >= 0 && d < maxDepth; k--, d++) {
        const nm = ns[k];
        if (!node.kids.has(nm)) node.kids.set(nm, { name: nm, us: 0, kids: new Map() });
        node = node.kids.get(nm); node.us += w;
    }
}
const ms = (us) => (us / 1000).toFixed(0).padStart(7) + ' ms';
const pct = (us) => (100 * us / Math.max(total, 1)).toFixed(1).padStart(5) + ' %';
console.log(`profile ${((prof.endTime - prof.startTime) / 1e6).toFixed(1)} s, ${prof.samples.length} samples; selected ${(total / 1e6).toFixed(2)} s${focus ? ' with a frame matching ' + focus : ''}${from || to !== Infinity ? ` in [${from}, ${to}] ms` : ''}`);
console.log('\n== by category (most specific match on the stack) ==');
for (const [c, us] of [...cats].sort((a, b) => b[1] - a[1])) console.log(`${ms(us)} ${pct(us)}  ${c}`);
console.log(`\n== self time, top ${top} ==`);
for (const [n, us] of [...self].sort((a, b) => b[1] - a[1]).slice(0, top)) console.log(`${ms(us)} ${pct(us)}  ${n.slice(0, 160)}`);
console.log(`\n== top-down (>= ${minPct} %, depth ${maxDepth}) ==`);
const walk = (node, depth) => {
    for (const k of [...node.kids.values()].sort((a, b) => b.us - a.us)) {
        if (100 * k.us / Math.max(total, 1) < minPct) continue;
        console.log(`${ms(k.us)} ${pct(k.us)} ${'  '.repeat(depth)}${k.name.slice(0, 150)}`);
        walk(k, depth + 1);
    }
};
walk(tree, 0);

// --frames <ms>: every main-thread task (a run of busy samples, gaps <= 3 ms) of at least <ms>, with a short top-down of what it did below the
// game's frame function (the freezes the player sees)
if (opt('--frames')) {
    const minTask = Number(opt('--frames')) * 1000;
    const busy = (n) => !/^\((idle|program|garbage collector)\)$/.test(n);
    const ANCHORS = [/App>::(play_frame|loading_frame|finishing_frame|start_loading|picker_event|simple_frame)\b/, /App>::frame\b/, /App as .*ApplicationHandler>::window_event/];
    const tasks = [];
    let cur = null;
    for (let i = 0; i < prof.samples.length; i++) {
        const at = ts[i];
        const leaf = names.get(prof.samples[i]);
        const w = i + 1 < ts.length ? ts[i + 1] - ts[i] : 0;
        if (!busy(leaf)) { if (cur && at - cur.end > 3000) { tasks.push(cur); cur = null; } continue; }
        if (!cur || at - cur.end > 3000) { if (cur) tasks.push(cur); cur = { start: at, end: at, samples: [] }; }
        cur.end = at + w; cur.samples.push([i, w]);
    }
    if (cur) tasks.push(cur);
    // --frames-match <regex>: only tasks with a frame matching it (e.g. play_frame: the freezes in play)
    const fm = opt('--frames-match') ? new RegExp(opt('--frames-match')) : null;
    const hasMatch = (t) => !fm || t.samples.some(([i]) => stackOf(prof.samples[i]).some((id) => fm.test(names.get(id))));
    const long = tasks.filter((t) => t.end - t.start >= minTask && hasMatch(t));
    console.log(`\n== main-thread tasks >= ${minTask / 1000} ms: ${long.length} (of ${tasks.length}) ==`);
    for (const t of long) {
        const dur = t.end - t.start;
        const node = { kids: new Map() };
        for (const [i, w] of t.samples) {
            const ns = stackOf(prof.samples[i]).map((id) => names.get(id));
            let k = -1;
            for (const re of ANCHORS) { k = ns.findIndex((n) => re.test(n)); if (k >= 0) break; }
            if (k < 0) k = Math.min(ns.length - 1, 8);
            let nd = node;
            const levels = Number(opt('--frames-depth', '6'));
            for (let d = k; d >= 0 && d > k - levels; d--) {
                const nm = ns[d];
                if (!nd.kids.has(nm)) nd.kids.set(nm, { us: 0, kids: new Map() });
                nd = nd.kids.get(nm); nd.us += w;
            }
        }
        console.log(`\n-- task at ${((t.start - prof.startTime) / 1000).toFixed(0)} ms: ${(dur / 1000).toFixed(0)} ms`);
        const show = (nd, depth) => {
            for (const [nm, k] of [...nd.kids].sort((a, b) => b[1].us - a[1].us)) {
                if (k.us < dur * 0.12) continue;
                console.log(`   ${(k.us / 1000).toFixed(0).padStart(6)} ms ${'  '.repeat(depth)}${nm.slice(0, 140)}`);
                show(k, depth + 1);
            }
        };
        show(node, 0);
    }
}
