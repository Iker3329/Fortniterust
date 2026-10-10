// Prints the WebGPU adapter limits of headless Brave (temporary profile): what the browser build can request (e.g. maxSampledTexturesPerShaderStage).
// usage: node tools/web/test/gpu-limits.mjs   (hold the GPU lock: tools/web/test/locked.sh)
import { chromium } from 'playwright-core';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { REPO, BRAVE, startServer } from './lib.mjs';
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'fnweb-limits-'));
fs.writeFileSync(path.join(dir, 'index.html'), '<!doctype html><title>limits</title>');
const s = await startServer({ www: dir, port: 8797 });
const ctx = await chromium.launchPersistentContext(fs.mkdtempSync(path.join(os.tmpdir(), 'fnweb-limits-profile-')), { executablePath: BRAVE, headless: true, args: ['--headless=new', '--enable-unsafe-webgpu'] });
try {
    const page = ctx.pages()[0] || await ctx.newPage();
    await page.goto(s.url);
    const r = await page.evaluate(async () => {
        const a = await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' });
        const l = {}; for (const k in a.limits) l[k] = a.limits[k];
        return { info: a.info ? { vendor: a.info.vendor, architecture: a.info.architecture } : null, limits: l };
    });
    console.log(JSON.stringify(r, null, 1));
} finally { await ctx.close(); s.proc.kill(); fs.rmSync(dir, { recursive: true, force: true }); }
