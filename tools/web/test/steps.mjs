import { chromium } from 'playwright-core';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { BRAVE } from './lib.mjs';

const [url, outDir, size, ...steps] = process.argv.slice(2);
const [W, H] = size.split('x').map(Number);
fs.mkdirSync(outDir, { recursive: true });
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'fnweb-steps-'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lines = [];
const ctx = await chromium.launchPersistentContext(profile, {
    executablePath: BRAVE, headless: true, viewport: null,
    args: ['--headless=new', `--window-size=${W},${H}`, '--enable-unsafe-webgpu', '--no-first-run', '--no-default-browser-check', '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--disable-backgrounding-occluded-windows'],
});
const page = ctx.pages()[0] || await ctx.newPage();
page.on('console', (m) => lines.push(`[page:${m.type()}] ${m.text()}`));
page.on('pageerror', (e) => lines.push('[page:exception] ' + (e.stack || e)));
page.on('worker', (w) => w.on('console', (m) => lines.push(`[worker:${m.type()}] ${m.text()}`)));
const net = new Map();
if (process.env.FNWEB_NETLOG) ctx.on('requestfinished', async (r) => {
    try {
        const s = await r.sizes();
        const u = r.url().replace(/\?.*$/, '');
        const range = (await r.allHeaders())['range'] || '';
        const e = net.get(u) || { n: 0, bytes: 0, ranges: new Map() };
        e.n++; e.bytes += s.responseBodySize; if (range) e.ranges.set(range, (e.ranges.get(range) || 0) + 1);
        net.set(u, e);
    } catch (_) {}
});
if (process.env.FNWEB_PSO_LOG)await page.addInitScript(() => {
    if (!self.GPUDevice) return;
    const t0 = performance.now();
    for (const name of ['createRenderPipeline', 'createComputePipeline', 'createShaderModule', 'createRenderPipelineAsync']) {
        const f = GPUDevice.prototype[name];
        GPUDevice.prototype[name] = function (d) { const a = performance.now(); const r = f.call(this, d); console.log(`[pso] ${((a - t0) / 1000).toFixed(2)}s ${name} ${d && d.label || ''} ${(d && d.code ? d.code.length : '')}`); return r; };
    }
});
if (process.env.FNWEB_HANGWATCH) {
    let last = 0, since = Date.now(), caught = 0;
    globalThis.__hw = setInterval(async () => {
        if (lines.length !== last) { last = lines.length; since = Date.now(); return; }
        if (Date.now() - since < 30000 || caught >= 3 || !lines.some((l) => l.includes('-> Playing'))) return;
        caught++; since = Date.now();
        try {
            const s = await page.context().newCDPSession(page);
            await s.send('Debugger.enable');
            const got = new Promise((r) => s.once('Debugger.paused', r));
            await s.send('Debugger.pause');
            const ev = await Promise.race([got, sleep(20000).then(() => null)]);
            const txt = ev ? ev.callFrames.map((f) => '  ' + f.functionName).join('\n') : '  (no pause within 20 s)';
            fs.writeFileSync(path.join(outDir, `hang${caught}.txt`), txt);
            console.log(`HANG ${caught} (no console output for 30 s)\n` + txt.split('\n').slice(0, 30).join('\n'));
            if (ev) await s.send('Debugger.resume').catch(() => {});
        } catch (e) { console.log('hangwatch failed ' + e); }
    }, 2000);
}
await page.goto(url);
const canvas = async () => { const c = await page.$('canvas'); return c ? c.boundingBox() : { x: 0, y: 0 }; };
try {
    for (const s of steps) {
        const [op, arg = ''] = s.split(/:(.*)/s);
        if (op === 'wait') await sleep(Number(arg) * 1000);
        else if (op === 'waitlog') { const end = Date.now() + 240000; while (!lines.some((l) => l.includes(arg)) && Date.now() < end) await sleep(250); console.log((lines.some((l) => l.includes(arg)) ? 'seen ' : 'NOT seen ') + arg); }
        else if (op === 'waitphase') { const end = Date.now() + 600000; let ok = false; while (Date.now() < end) { ok = await page.evaluate((p) => (window.__fnwebPhaseLog || []).some((x) => x.p === p), arg).catch(() => false); if (ok) break; await sleep(500); } console.log((ok ? 'phase ' : 'NO phase ') + arg); }
        else if (op === 'clicksel') { for (let k = 0; k < 120; k++) { const v = await page.evaluate((q) => { const e = document.querySelector(q); return !!e && e.offsetParent !== null; }, arg).catch(() => false); if (v) { await page.click(arg); console.log('clicked ' + arg); break; } await sleep(500); } }
        else if (op === 'profstart') { globalThis.__cdp = await page.context().newCDPSession(page); await __cdp.send('Profiler.enable'); await __cdp.send('Profiler.setSamplingInterval', { interval: 200 }); await __cdp.send('Profiler.start'); console.log('profiling'); }
        else if (op === 'profstop') { const { profile } = await __cdp.send('Profiler.stop'); fs.writeFileSync(path.join(outDir, arg || 'main.cpuprofile'), JSON.stringify(profile)); console.log('profile saved'); }
        else if (op === 'key') await page.keyboard.press(arg);
        else if (op === 'down') await page.keyboard.down(arg);
        else if (op === 'up') await page.keyboard.up(arg);
        else if (op === 'type') await page.keyboard.type(arg, { delay: 30 });
        else if (op === 'click') { const [x, y] = arg.split(',').map(Number); await page.mouse.click(x, y); }
        else if (op === 'move') { const [x, y] = arg.split(',').map(Number); await page.mouse.move(x, y); }
        else if (op === 'shot') { await page.screenshot({ path: path.join(outDir, arg + '.png') }); console.log('shot ' + arg); }
        else if (op === 'burst') { const [name, n, ms] = arg.split(','); for (let i = 0; i < Number(n); i++) { await page.screenshot({ path: path.join(outDir, `${name}-${String(i).padStart(3, '0')}.png`) }); await sleep(Number(ms)); } console.log('burst ' + name); }
        else if (op === 'stack') {
            const s = await page.context().newCDPSession(page);
            await s.send('Debugger.enable');
            const got = new Promise((r) => s.once('Debugger.paused', r));
            await s.send('Debugger.pause');
            const ev = await Promise.race([got, sleep(20000).then(() => null)]);
            const txt = ev ? ev.callFrames.map((f) => '  ' + f.functionName).join('\n') : '  (no pause within 20 s)';
            fs.writeFileSync(path.join(outDir, (arg || 'stack') + '.txt'), txt);
            console.log('stack\n' + txt.split('\n').slice(0, 40).join('\n'));
            if (ev) await s.send('Debugger.resume').catch(() => {});
        }
        else if (op === 'bgstack') {
            const [delay, name] = arg.split(',');
            setTimeout(async () => {
                try {
                    const s = await page.context().newCDPSession(page);
                    await s.send('Debugger.enable');
                    const got = new Promise((r) => s.once('Debugger.paused', r));
                    await s.send('Debugger.pause');
                    const ev = await Promise.race([got, sleep(20000).then(() => null)]);
                    const txt = ev ? ev.callFrames.map((f) => '  ' + f.functionName).join('\n') : '  (no pause within 20 s)';
                    fs.writeFileSync(path.join(outDir, (name || 'bgstack') + '.txt'), txt);
                    console.log('bgstack ' + name + '\n' + txt.split('\n').slice(0, 30).join('\n'));
                    if (ev) await s.send('Debugger.resume').catch(() => {});
                } catch (e) { console.log('bgstack failed ' + e); }
            }, Number(delay) * 1000);
        }
        else if (op === 'psodump') {
            const b64 = await page.evaluate(() => { const u = window.__fnweb.fnweb_pso_dump(); let s = ''; for (let i = 0; i < u.length; i += 32768) s += String.fromCharCode.apply(null, u.subarray(i, i + 32768)); return btoa(s); });
            const buf = Buffer.from(b64, 'base64');
            fs.writeFileSync(arg || path.join(outDir, 'pso.bin'), buf);
            console.log(`psodump ${buf.length} bytes, ${await page.evaluate(() => window.__fnweb.fnweb_pso_count())} pipelines`);
        }
        else if (op === 'eval') console.log(JSON.stringify(await page.evaluate(arg)));
        else console.log('unknown step ' + s);
    }
} finally {
    fs.writeFileSync(path.join(outDir, 'console.txt'), lines.join('\n'));
    if (net.size) {
        const rows = [...net.entries()].map(([u, e]) => { let dup = 0; for (const c of e.ranges.values()) if (c > 1) dup += c - 1; return { u, n: e.n, mb: e.bytes / 1048576, dup }; }).sort((a, b) => b.mb - a.mb);
        const total = rows.reduce((s, r) => s + r.mb, 0);
        fs.writeFileSync(path.join(outDir, 'net.txt'), `total ${total.toFixed(1)} MB in ${rows.reduce((s, r) => s + r.n, 0)} requests\n` + rows.map((r) => `${r.mb.toFixed(1).padStart(8)} MB ${String(r.n).padStart(6)} req ${String(r.dup).padStart(5)} repeated-ranges  ${r.u}`).join('\n'));
    }
    if (globalThis.__hw) clearInterval(globalThis.__hw);
    await ctx.close();
    try { fs.rmSync(profile, { recursive: true, force: true }); } catch (_) {}
}
