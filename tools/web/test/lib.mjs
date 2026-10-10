// Shared helpers for the browser-build drivers: start the launcher (fortnite-web.exe) without opening a browser, launch Brave (or Chromium) with
// a TEMPORARY profile through Playwright (never the user's own browser windows), collect console output from the page and its workers.
import { chromium } from 'playwright-core';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const REPO = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..');
export const BRAVE = process.env.FNWEB_BROWSER || 'C:\\Users\\Brady\\AppData\\Local\\BraveSoftware\\Brave-Browser\\Application\\brave.exe';

export async function startServer({ www, port = 8791, extra = [] }) {
    const exe = path.join(REPO, 'dist', 'fortnite-web.exe');
    const args = ['--no-browser', '--quiet', '--port', String(port), '--www', www, ...extra];
    const p = spawn(exe, args, { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    p.stdout.on('data', (d) => { out += d; });
    p.stderr.on('data', (d) => { out += d; });
    const t0 = Date.now();
    while (!/http:\/\/127\.0\.0\.1:(\d+)/.test(out)) {
        if (p.exitCode !== null) throw new Error('launcher exited: ' + out);
        if (Date.now() - t0 > 15000) throw new Error('launcher did not start: ' + out);
        await new Promise((r) => setTimeout(r, 100));
    }
    const url = out.match(/(http:\/\/127\.0\.0\.1:\d+\/\S*)/)[1];
    return { proc: p, url, log: () => out };
}

export async function launchBrowser({ width = 1600, height = 900, headless = false } = {}) {
    const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'fnweb-profile-'));
    const ctx = await chromium.launchPersistentContext(profile, {
        executablePath: BRAVE,
        headless,
        viewport: null,
        args: ['--no-first-run', '--no-default-browser-check', `--window-size=${width},${height}`, '--enable-unsafe-webgpu', '--disable-features=Translate', '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--disable-backgrounding-occluded-windows'],
    });
    const page = ctx.pages()[0] || await ctx.newPage();
    const lines = [];
    const tag = (src) => (m) => { const t = `[${src}:${m.type()}] ${m.text()}`; lines.push(t); if (process.env.FNWEB_ECHO) console.log(t); };
    page.on('console', tag('page'));
    page.on('pageerror', (e) => lines.push('[page:exception] ' + (e.stack || e)));
    page.on('worker', (w) => w.on('console', tag('worker')));
    return { ctx, page, lines, profile, close: async () => { await ctx.close(); try { fs.rmSync(profile, { recursive: true, force: true }); } catch (_) {} } };
}
