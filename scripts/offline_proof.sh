#!/bin/bash
# Offline proof (macOS): runs the Search test binaries with every network operation denied by the OS sandbox,
# then lists any denied network attempt the sandbox logged. A control (curl) proves the denial is in force.
# Evidence recorded in docs/knowledge-engine-audit.md. Usage: scripts/offline_proof.sh [extra cargo test args]
set -u
cd "$(dirname "$0")/.."
DENY='(version 1)(allow default)(deny network*)'
START=$(date +"%Y-%m-%d %H:%M:%S")

echo "== control: network must fail"
if sandbox-exec -p "$DENY" curl -sS -m 3 https://example.com >/dev/null 2>&1; then
  echo "FAIL: network was reachable inside the sandbox"; exit 2
fi
echo "ok: network denied"

bins() {
  cargo test "$@" --no-run --message-format=json 2>/dev/null | python3 -c "
import sys, json
for line in sys.stdin:
    try: j = json.loads(line)
    except ValueError: continue
    if j.get('executable') and j.get('profile', {}).get('test'): import re; print(re.split(r'/(?:src|tests)/', j['target']['src_path'])[0] + '|' + j['executable'])"
}

status=0
run() {
  local label=$1; shift
  while IFS='|' read -r dir exe; do
    [ -z "$exe" ] && continue
    echo "== $label: $(basename "$exe")"
    (cd "$dir" && sandbox-exec -p "$DENY" "$exe" "${@:-}" 2>&1 | grep -E "^test result|FAILED|panicked" | head -5) || status=1
  done
}

bins -p search-core --features semantic-search,ocr | run "search-core (semantic, ocr)"
bins -p native-search | run "native-search"
bins -p app-tui --features gpu-viewer --lib | run "app-tui lib"

echo "== sandbox denials logged since $START "
# Only denials attributed to our own test binaries count; other system processes log their own.
OURS=$(/usr/bin/log show --start "$START" --predicate 'eventMessage CONTAINS "deny(1) network"' --style compact 2>/dev/null \
  | grep -E "Sandbox: (search_core|native_search|app_tui|search-cli|realistic_mixed|fixtures|index_scheduling)" || true)
if [ -n "$OURS" ]; then echo "$OURS"; echo "FAIL: our binaries attempted network access"; status=1; else echo "none from our binaries"; fi
exit $status
