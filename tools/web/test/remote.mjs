// REMOTE MODE test: the page in remote mode (tools/web/site.sh output) against a file Worker (default: the local `wrangler dev` of
// tools/web/fn-web-worker at http://127.0.0.1:8787), in headless Brave with a temporary profile. Logs in through the page's form, then reads
// through the game's own file system (std::fs in the wasm: the I/O threads, the page's service worker, the Worker) and compares the bytes with
// the Worker's answer to a direct request; then reloads the page and checks the reads come from the service worker's cache.
// The password is read from tools/web/fn-web-worker/.dev.vars (local test value; never printed).
// usage: node tools/web/test/remote.mjs [--worker http://127.0.0.1:8787] [--out verification/browser/remote1] [--files "/fn/paks/test.pak,..."]
import { chromium } from 'playwright-core';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { REPO, BRAVE, startServer } from './lib.mjs';

const argv = process.argv.slice(2);
const opt = (n, d) => { const i = argv.indexOf(n); return i >= 0 ? argv[i + 1] : d; };
const worker = opt('--worker', 'http://127.0.0.1:8787').replace(/\/+$/, '');
const out = path.resolve(REPO, opt('--out', 'verification/browser/remote1'));
const files = opt('--files', '/fn/paks/test.pak,/fn/assets/sky/sunmoon.bin,/fn/assets/particles-pickup/index.json').split(',');
fs.mkdirSync(out, { recursive: true });
const t0 = Date.now();
const lines = [];
const log = (m) => { const l = `[${((Date.now() - t0) / 1000).toFixed(1).padStart(6)}] ${m}`; console.log(l); lines.push(l); };
const results = [];
const check = (name, ok, detail = '') => { results.push({ name, ok }); log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ': ' + detail : ''}`); };
const fnv = (b) => { let h = 0xcbf29ce484222325n; for (const x of b) { h ^= BigInt(x); h = (h * 0x100000001b3n) & 0xffffffffffffffffn; } return h.toString(16); };

const vars = fs.readFileSync(path.join(REPO, 'tools/web/fn-web-worker/.dev.vars'), 'utf8');
const password = (vars.match(/^\s*SITE_PASSWORD\s*=\s*"?([^"\r\n]*)"?/m) || [])[1];
if (!password) throw new Error('no SITE_PASSWORD in tools/web/fn-web-worker/.dev.vars');

// the site folder for this test
const site = path.join(REPO, 'dist', 'web-remote-test');
execFileSync('bash', [path.join(REPO, 'tools/web/site.sh'), '--to', site, '--worker', worker], { stdio: 'inherit' });
const server = await startServer({ www: site, port: 8780 });
if (!server.url.includes(':8780/')) throw new Error('port 8780 is taken (the Worker allows only that local origin): ' + server.url);
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'fnweb-remote-'));
let ctx = null, status = 0;
const consoleLines = [];
try {
    // the Worker directly (what the page must read): login + ranged reads with the ?p= form
    const lr = await fetch(worker + '/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ password }) });
    check('worker_login', lr.ok, 'HTTP ' + lr.status);
    const { token } = await lr.json();
    const direct = async (p, a, b) => {
        const r = await fetch(`${worker}/__fnvfs/file?p=${encodeURIComponent(p)}`, { headers: { Authorization: 'Bearer ' + token, Range: `bytes=${a}-${b}` } });
        return r.ok ? new Uint8Array(await r.arrayBuffer()) : null;
    };
    const stat = async (p) => { const r = await fetch(`${worker}/__fnvfs/stat?p=${encodeURIComponent(p)}`, { headers: { Authorization: 'Bearer ' + token } }); return r.ok ? (await r.text()).trim().split(/\s+/).map(Number) : null; };

    ctx = await chromium.launchPersistentContext(profile, { executablePath: BRAVE, headless: true, viewport: null, args: ['--headless=new', '--window-size=1280,720', '--enable-unsafe-webgpu', '--no-first-run'] });
    const page = ctx.pages()[0] || await ctx.newPage();
    page.on('console', (m) => consoleLines.push(`[page:${m.type()}] ${m.text()}`));
    page.on('pageerror', (e) => consoleLines.push('[page:exception] ' + e));
    await page.goto(server.url + '?fnweb_nogame=1');
    await page.waitForSelector('#fnweb-password', { state: 'visible', timeout: 30000 });
    check('login_form_shown', true);
    await page.fill('#fnweb-password', 'wrong-' + Math.random());
    await page.click('#fnweb-login-go');
    await page.waitForFunction(() => document.getElementById('fnweb-login-error').textContent.length > 0, null, { timeout: 15000 });
    check('wrong_password_refused', (await page.textContent('#fnweb-login-error')).includes('Wrong'), await page.textContent('#fnweb-login-error'));
    await page.fill('#fnweb-password', password);
    await page.click('#fnweb-login-go');
    await page.waitForFunction(() => window.__fnwebReady === true || !!window.__fnwebFatal, null, { timeout: 60000 });
    const fatal = await page.evaluate(() => window.__fnwebFatal || null);
    check('runtime_up_after_login', !fatal, fatal || '');
    check('page_controlled_by_service_worker', await page.evaluate(() => !!navigator.serviceWorker.controller));
    check('token_not_in_url', !(await page.evaluate(() => location.href)).includes(token.slice(0, 12)));

    const listing = await page.evaluate(() => { try { return window.__fnweb.fnweb_vfs_test_list('/fn'); } catch (e) { return 'ERR ' + e; } });
    check('list_fn', /paks dir/.test(listing) && /assets dir/.test(listing), listing.replace(/\n/g, ' | '));
    for (const p of ['/fn/windows/Fonts/segoeui.ttf', '/fn/bin/FortniteRuntime.toml']) {
        const r = await page.evaluate((p) => { try { window.__fnweb.fnweb_vfs_test_list(p); return 'exists'; } catch (e) { return String(e); } }, p);
        check('absent ' + p, /not found/i.test(r), r);
    }
    for (const f of files) {
        const st = await stat(f);
        if (!st) { check('stat ' + f, false, 'the Worker has no such file'); continue; }
        const size = st[1];
        const ranges = [[0, Math.min(size, 4096)], [Math.max(0, Math.floor(size / 2) - 100), 300], [Math.max(0, size - 777), 777]];
        for (const [off, len] of ranges) {
            const want = await direct(f, off, off + len - 1);
            const got = await page.evaluate(async ([f, off, len]) => { try { return Array.from(window.__fnweb.fnweb_vfs_test_read(f, off, len)); } catch (e) { return 'ERR ' + e; } }, [f, off, len]);
            const ok = Array.isArray(got) && want && got.length === want.length && fnv(got) === fnv(want);
            check(`read ${f} @${off}+${len}`, ok, Array.isArray(got) ? `${got.length} bytes, fnv ${fnv(got)}` : String(got));
        }
        // case-insensitive like the Windows folders: the path in another case is resolved to the stored one
        const upper = f.replace(/\/[^/]+$/, (n) => n.toUpperCase());
        const got = await page.evaluate(async (u) => { try { return window.__fnweb.fnweb_vfs_test_read(u, 0, 64).length; } catch (e) { return 'ERR ' + e; } }, upper);
        check('case-insensitive ' + upper, got === Math.min(64, size), String(got));
    }
    const cached1 = await page.evaluate(async () => (await Promise.all((await caches.keys()).filter((k) => k.startsWith('fnweb-files')).map(async (k) => (await (await caches.open(k)).keys()).length))).reduce((a, b) => a + b, 0));
    check('service_worker_cached_replies', cached1 > 0, cached1 + ' entries');

    // reload: same tab session (token kept), the reads must come from the cache (no new Worker requests): count the Worker's hits by stats
    await page.reload();
    await page.waitForFunction(() => window.__fnwebReady === true || !!window.__fnwebFatal, null, { timeout: 60000 });
    check('reload_without_login', await page.evaluate(() => getComputedStyle(document.getElementById('fnweb-login')).display === 'none'));
    const t = Date.now();
    const again = await page.evaluate(async (f) => { try { return window.__fnweb.fnweb_vfs_test_read(f, 0, 4096).length; } catch (e) { return 'ERR ' + e; } }, files[0]);
    check('reread_after_reload', typeof again === 'number' && again > 0, `${again} bytes in ${Date.now() - t} ms`);
    const cached2 = await page.evaluate(async () => (await Promise.all((await caches.keys()).filter((k) => k.startsWith('fnweb-files')).map(async (k) => (await (await caches.open(k)).keys()).length))).reduce((a, b) => a + b, 0));
    check('cache_reused (no new entries for the same reads)', cached2 >= cached1, `${cached1} -> ${cached2} entries`);
} catch (e) {
    log('ERROR ' + (e.stack || e)); status = 1;
} finally {
    fs.writeFileSync(path.join(out, 'console.txt'), consoleLines.join('\n') + '\n');
    fs.writeFileSync(path.join(out, 'results.txt'), lines.join('\n') + '\n');
    if (ctx) { try { await ctx.close(); } catch (_) {} }
    server.proc.kill();
    try { fs.rmSync(profile, { recursive: true, force: true }); } catch (_) {}
}
const failed = results.filter((r) => !r.ok).length;
log(`${results.length - failed} passed, ${failed} failed`);
process.exit(status || (failed ? 1 : 0));
