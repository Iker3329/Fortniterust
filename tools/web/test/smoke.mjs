// Runs the platform-layer smoke test (dist/web-smoke) in Brave and prints every check; exits 1 when one fails.
// usage: node tools/web/test/smoke.mjs   (hold the GPU lock: verification/perf_assets/tools/gpu_lock.sh acquire ...)
import { startServer, launchBrowser, REPO } from './lib.mjs';
import path from 'node:path';
import fs from 'node:fs';

const server = await startServer({ www: path.join(REPO, 'dist', 'web-smoke') });
const b = await launchBrowser({ width: 900, height: 700 });
let failed = 0;
try {
    await b.page.goto(server.url);
    await b.page.waitForFunction(() => window.__smokeDone === true, null, { timeout: 120000 });
    const lines = await b.page.evaluate(() => window.__smokeLines);
    for (const l of lines) { console.log(l); if (l.startsWith('FAIL')) failed++; }
    const out = path.join(REPO, 'verification', 'browser');
    fs.mkdirSync(out, { recursive: true });
    await b.page.screenshot({ path: path.join(out, 'smoke.png') });
    fs.writeFileSync(path.join(out, 'smoke-console.txt'), b.lines.join('\n') + '\n\nRESULTS\n' + lines.join('\n') + '\n');
} catch (e) {
    console.log('FAIL harness: ' + e);
    console.log(b.lines.slice(-40).join('\n'));
    failed++;
} finally {
    await b.close();
    server.proc.kill();
}
console.log(failed ? `${failed} check(s) FAILED` : 'all checks passed');
process.exit(failed ? 1 : 0);
