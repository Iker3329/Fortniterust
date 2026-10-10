#!/bin/bash
# Puts the BROWSER build (dist/web, from tools/web/build.sh) into a static site folder in REMOTE MODE: the page reads the game files from the
# user's private file Worker (tools/web/fn-web-worker) after a password login, through its service worker (www/sw.js). Copies files only; it
# deploys nothing, and no game file ever goes into the site (the folder gets the page, the wasm and the JS, nothing of Epic's).
#
#   tools/web/site.sh --to <dir> --worker <url> [--cache-version N]
#       the page files + fnweb-remote.json into <dir> (e.g. a local test folder served by `dist/fortnite-web.exe --www <dir> --no-browser`)
#   tools/web/site.sh --vercel <site project dir> --worker <url> [--cache-version N]
#       the same into <site>/fortnite/, and vercel.json headers for /fortnite/(.*): COOP same-origin + COEP require-corp (the shared wasm memory
#       needs a cross-origin isolated page); an existing vercel.json is merged, not replaced
#
# --cache-version: forces every browser to re-download everything (the service worker keys its cache by each file's size + upload time, so new or
# replaced files in the bucket do NOT need a bump).
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TO="" VERCEL="" WORKER="" CACHE_VERSION=1
while [ $# -gt 0 ]; do
  case "$1" in
    --to) TO="$2"; shift 2 ;;
    --vercel) VERCEL="$2"; shift 2 ;;
    --worker) WORKER="$2"; shift 2 ;;
    --cache-version) CACHE_VERSION="$2"; shift 2 ;;
    *) echo "unknown argument $1"; exit 2 ;;
  esac
done
[ -n "$WORKER" ] || { echo "--worker <url> is required (the file Worker's base URL)"; exit 2; }
[ -n "$TO$VERCEL" ] || { echo "--to <dir> or --vercel <site dir> is required"; exit 2; }
SRC="$REPO/dist/web"
[ -f "$SRC/index.html" ] && [ -d "$SRC/pkg" ] || { echo "$SRC is not built: run tools/web/build.sh first"; exit 1; }
[ -n "$VERCEL" ] && TO="$VERCEL/fortnite"
mkdir -p "$TO"
# the page's own files only (the folder may hold other things of the site: replace exactly these)
rm -rf "$TO/pkg"
cp -r "$SRC/pkg" "$TO/pkg"
for f in index.html fnweb-boot.js fnweb-prefetch.js fnweb-boot-manifest.json worker.js sw.js build-info.txt; do [ -f "$SRC/$f" ] && cp "$SRC/$f" "$TO/$f"; done
printf '{\n  "worker": "%s",\n  "cacheVersion": %s\n}\n' "${WORKER%/}" "$CACHE_VERSION" > "$TO/fnweb-remote.json"
echo "[site] page files + fnweb-remote.json (worker ${WORKER%/}, cache version $CACHE_VERSION) -> $TO"
if [ -n "$VERCEL" ]; then
  node - "$VERCEL/vercel.json" <<'EOF'
const fs = require('fs');
const file = process.argv[2];
let cfg = {};
if (fs.existsSync(file)) cfg = JSON.parse(fs.readFileSync(file, 'utf8'));
cfg.headers = (cfg.headers || []).filter((h) => h.source !== '/fortnite/(.*)');
cfg.headers.push({
  source: '/fortnite/(.*)',
  headers: [
    { key: 'Cross-Origin-Opener-Policy', value: 'same-origin' },
    { key: 'Cross-Origin-Embedder-Policy', value: 'require-corp' },
    { key: 'Cross-Origin-Resource-Policy', value: 'same-origin' },
    { key: 'Cache-Control', value: 'no-cache' },
  ],
});
// the page resolves its files and its service worker relative to its URL: /fortnite must be /fortnite/
cfg.redirects = (cfg.redirects || []).filter((r) => r.source !== '/fortnite');
cfg.redirects.push({ source: '/fortnite', destination: '/fortnite/', permanent: false });
fs.writeFileSync(file, JSON.stringify(cfg, null, 2) + '\n');
console.log('[site] ' + file + ': COOP/COEP headers for /fortnite/(.*), /fortnite -> /fortnite/');
EOF
fi
