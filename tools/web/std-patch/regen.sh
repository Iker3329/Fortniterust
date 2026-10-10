#!/bin/bash
# Maintainer helper: regenerates fnweb-std.patch from two trees, <dir>/pristine/library (a copy of the pinned nightly's rust-src) and
# <dir>/patched/library (the edited copy). usage: tools/web/std-patch/regen.sh [dir]   (default D:/target-fn-web/std-src)
set -euo pipefail
D="${1:-D:/target-fn-web/std-src}"
OUT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/fnweb-std.patch"
cd "$D"
git -c core.autocrlf=false diff --no-index pristine/library/std patched/library/std 2>/dev/null \
  | sed -e 's#a/pristine/library/#a/#g; s#a/patched/library/#a/#g; s#b/patched/library/#b/#g; s#b/pristine/library/#b/#g' > "$OUT" || true
grep -c '^diff --git' "$OUT" | xargs echo "files in patch:"
