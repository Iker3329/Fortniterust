// REMOTE MODE start-up prefetch (a dedicated worker started by index.html right after the login, while the game module compiles and boots).
//
// The game reads its files with synchronous requests, one at a time per thread: over the internet every read waits a full round trip, so a
// cold load is bound by latency, not by the line. fnweb-boot-manifest.json (written by the test harness from a recorded load: vfs.rs `record`)
// lists the file ranges a load reads, in the order it reads them; this worker asks the page's service worker for them up to CONCURRENCY at a time,
// in that order. The service worker keeps each reply in its persistent cache (and fetches a range the game asks for meanwhile only once), so the
// game's own reads are answered from this browser's disk. Already cached ranges cost a lookup (204, no body); a stale manifest only costs
// requests the game will not make. Content versions (`v`, see vfs.rs `version_of`) come from the same bucket listings the game reads.
const CONCURRENCY = 16;
// COALESCING: neighbouring ranges of one file (gap <= MERGE_GAP) go out as one request of at most RUN_MAX bytes / RUN_PARTS ranges; the service
// worker cuts the reply into the game's own ranges (each its own cache entry)
const MERGE_GAP = 2 << 20, RUN_MAX = 16 << 20, RUN_PARTS = 300;
const BUNDLE_FILE_MAX = 256 << 10;
const BUNDLE_EXT = ['json', 'txt', 'ini', 'csv'];

self.onmessage = async (e) => {
    const d = e.data || {};
    if (d.fnweb !== 'prefetch') return;
    const t0 = performance.now();
    const auth = { Authorization: 'Bearer ' + d.token };
    const api = (op, q) => `${d.base}/__fnvfs/${op}?${q}`;
    let manifest;
    try {
        const r = await fetch(d.manifest, { cache: 'no-cache' });
        if (!r.ok) { self.postMessage({ fnweb: 'prefetch-done', skipped: 'no boot manifest (HTTP ' + r.status + ')' }); return; }
        manifest = await r.json();
    } catch (x) { self.postMessage({ fnweb: 'prefetch-done', skipped: 'no boot manifest: ' + x }); return; }
    const entries = Array.isArray(manifest) ? manifest : (manifest.entries || []);
    // the listings of the roots the manifest touches: lower-case absolute path -> [kind, size, mtime]
    const roots = [...new Set(entries.map((x) => String(x[1]).split('/').slice(0, 3).join('/')))].filter((r) => r === '/fn/paks' || r === '/fn/assets');
    const stat = new Map();
    await Promise.all(roots.map(async (root) => {
        try {
            const r = await fetch(api('tree', 'p=' + encodeURIComponent(root)), { headers: auth });
            if (!r.ok) return;
            const base = root.toLowerCase();
            for (const line of (await r.text()).split('\n')) {
                const f = line.split('\t');
                if (f.length < 4) continue;
                stat.set(base + '/' + f.slice(3).join('\t').toLowerCase(), [Number(f[0]), Number(f[1]), Number(f[2])]);
            }
        } catch (_) {}
    }));
    // vfs.rs `version_of` / the bundle version of `small_file`
    const fileVersion = (p) => { const s = stat.get(p.toLowerCase()); return s ? `${s[1]}-${Math.round(s[2])}` : null; };
    const bundleVersion = (folder) => {
        const prefix = folder.toLowerCase() + '/';
        let n = 0, total = 0, newest = 0;
        for (const [k, s] of stat) {
            if (s[0] !== 1 || s[1] > BUNDLE_FILE_MAX || !k.startsWith(prefix)) continue;
            const name = k.slice(k.lastIndexOf('/') + 1), dot = name.lastIndexOf('.');
            if (dot < 0 || !BUNDLE_EXT.includes(name.slice(dot + 1))) continue;
            n++; total += s[1]; newest = Math.max(newest, s[2]);
        }
        return n ? `${n}-${total}-${Math.round(newest)}` : null;
    };
    // the jobs: ranged reads of a file merged into runs, everything else on its own; in the order the load first needs them
    const sizeOf = (x) => x[2] >= 0 ? x[3] - x[2] + 1 : ((stat.get(String(x[1]).toLowerCase()) || [0, 0])[1] || 0);
    const cut = entries.findIndex((x) => x[0] === 'menu');
    const build = (lo, hi) => {
    const jobs = [];
    const byFile = new Map();
    entries.slice(lo, hi).forEach((x, k) => { const i = lo + k; if (x[0] === 'menu') return; if (x[0] === 'file' && x[2] >= 0 && !x[4]) { const k = x[1]; if (!byFile.has(k)) byFile.set(k, []); byFile.get(k).push([x[2], x[3], i]); } else jobs.push({ first: i, single: x, bytes: sizeOf(x) }); });
    for (const [path, rs] of byFile) {
        rs.sort((p, q) => p[0] - q[0]);
        let run = null;
        const flush = () => { if (run) jobs.push({ first: run.first, path, a: run.a, b: run.b, parts: run.parts, bytes: run.bytes }); run = null; };
        for (const [a, b, i] of rs) {
            if (run && a - run.b - 1 <= MERGE_GAP && Math.max(b, run.b) - run.a + 1 <= RUN_MAX && run.parts.length < RUN_PARTS) { run.b = Math.max(run.b, b); run.parts.push([a, b]); run.first = Math.min(run.first, i); run.bytes += b - a + 1; }
            else { flush(); run = { a, b, parts: [[a, b]], first: i, bytes: b - a + 1 }; }
        }
        flush();
    }
    jobs.sort((p, q) => p.first - q.first);
    return jobs;
    };
    const phases = cut >= 0 ? [build(0, cut), build(cut + 1, entries.length)] : [build(0, entries.length)];
    const totalBytes = phases.flat().reduce((s, j) => s + j.bytes, 0);
    let jobs = phases[0];
    let next = 0, done = 0, failed = 0, bytesDone = 0, requests = 0;
    let lastReport = 0;
    const one = async (j) => {
        const [op, path, a, b, extra] = j.single || ['file', j.path, j.a, j.b, ''];
        const v = op === 'bundle' ? bundleVersion(path) : fileVersion(path);
        if (!v) { failed += j.parts ? j.parts.length : 1; return; } // not in the listing (removed from the bucket)
        const headers = { ...auth, 'X-Fnvfs-Prefetch': '1' };
        if (a >= 0) headers.Range = `bytes=${a}-${b}`;
        if (j.parts) headers['X-Fnvfs-Parts'] = j.parts.map(([x, y]) => x + '-' + y).join(',');
        try {
            requests++;
            const r = await fetch(api(op, 'p=' + encodeURIComponent(path) + (extra ? '&' + extra : '') + '&v=' + encodeURIComponent(v)), { headers });
            if (!(r.status === 204 || r.ok)) failed += j.parts ? j.parts.length : 1;
        } catch (_) { failed += j.parts ? j.parts.length : 1; }
    };
    const lane = async () => {
        while (next < jobs.length) {
            const j = jobs[next++];
            await one(j);
            done += j.parts ? j.parts.length : 1;
            bytesDone += j.bytes;
            const now = performance.now();
            if (now - lastReport > 250) { lastReport = now; self.postMessage({ fnweb: 'prefetch-progress', done, total: entries.length, bytes: bytesDone, totalBytes }); }
        }
    };
    for (let ph = 0; ph < phases.length; ph++) {
        jobs = phases[ph]; next = 0;
        await Promise.all(Array.from({ length: Math.min(CONCURRENCY, jobs.length) }, lane));
        if (ph === 0 && phases.length > 1) self.postMessage({ fnweb: 'prefetch-menu-done', mb: bytesDone / 1048576, ms: performance.now() - t0 });
    }
    self.postMessage({ fnweb: 'prefetch-done', done, failed, total: entries.length, requests, mb: bytesDone / 1048576, totalMb: totalBytes / 1048576, ms: performance.now() - t0 });
};
