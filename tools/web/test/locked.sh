#!/bin/bash
# Runs a browser test session while holding the machine's GPU lock (verification/perf_assets/tools/gpu_lock.sh), and releases it ONLY if this
# script acquired it. Exit 75 = the GPU is busy (do other work, retry later).
#   tools/web/test/locked.sh "<what>" <command...>
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
LOCK="$REPO/verification/perf_assets/tools/gpu_lock.sh"
what="$1"; shift
bash "$LOCK" acquire "browser-port: $what" || exit 75
trap 'bash "$LOCK" release >/dev/null' EXIT
"$@"
