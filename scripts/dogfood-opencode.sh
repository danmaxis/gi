#!/bin/bash
# Dogfood / regression-check gi's opencode interop against the vendored cases in
# tests/opencode-cases/. Imports each config (JSON output), asserts valid JSON +
# clean translation, and round-trips one. No model/network required.
#
#   GI_BIN=~/.local/bin/gi scripts/dogfood-opencode.sh
set -u
GI="${GI_BIN:-$HOME/.local/bin/gi}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CASES="$ROOT/tests/opencode-cases"
PASS=0; FAIL=0
note() { printf '  %s\n' "$1"; }

for case in "$CASES"/*.json "$CASES"/*.jsonc; do
  [ -e "$case" ] || continue
  name="$(basename "$case")"
  echo "== $name =="
  out="$("$GI" opencode import "$case" --output-format json 2>/dev/null)"
  if ! printf '%s' "$out" | jq -e . >/dev/null 2>&1; then
    note "FAIL: import did not emit valid JSON"; FAIL=$((FAIL+1)); continue
  fi
  kind="$(printf '%s' "$out" | jq -r '.kind // empty')"
  model="$(printf '%s' "$out" | jq -r '..|.model? // empty' | head -1)"
  mcp="$(printf '%s' "$out" | jq -r '..|.mcpServers? // empty | keys? | join(",")' | head -1)"
  warns="$(printf '%s' "$out" | jq -r '.warnings // [] | length')"
  if [ "$kind" != "opencode" ]; then
    note "FAIL: kind != opencode (got '$kind')"; FAIL=$((FAIL+1)); continue
  fi
  note "ok · model=${model:-<none>} · mcpServers=[${mcp:-}] · warnings=$warns"
  printf '%s' "$out" | jq -r '.warnings[]? | "       warn: " + .' 2>/dev/null
  PASS=$((PASS+1))
done

# Round-trip one case: import → settings.json → export, both valid JSON.
echo "== round-trip: rich-user-config.json =="
tmp="$(mktemp -d)"
mkdir -p "$tmp/.gi"
if "$GI" opencode import "$CASES/rich-user-config.json" --out "$tmp/.gi/settings.json" >/dev/null 2>&1 \
   && (cd "$tmp" && "$GI" opencode export --output-format json 2>/dev/null | jq -e '.kind=="opencode"' >/dev/null); then
  note "ok · import --out then export round-trips"; PASS=$((PASS+1))
else
  note "FAIL: round-trip"; FAIL=$((FAIL+1))
fi
rm -rf "$tmp"

echo "== opencode-cases: $PASS passed, $FAIL failed =="
[ "$FAIL" -eq 0 ]
