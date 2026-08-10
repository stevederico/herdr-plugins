#!/usr/bin/env bash
# Idempotent left-dock toggle for herdr-explorer.
# herdr pane split only goes right/down → split leftmost, swap into left slot.
set -euo pipefail

herdr="${HERDR_BIN_PATH:-herdr}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin="$root/target/release/herdr-explorer"
label="herdr-explorer"

if [ ! -x "$bin" ]; then
  echo "build first: cargo build --release" >&2
  exit 1
fi

# Already have our pane in this tab? focus / close (toggle).
current="$("$herdr" pane current 2>/dev/null || true)"
list="$("$herdr" pane list 2>/dev/null || true)"

# Prefer JSON when available; fall back to scanning list text for our label.
our_id=""
if command -v python3 >/dev/null 2>&1 && [ -n "$list" ]; then
  our_id="$(printf '%s' "$list" | python3 -c '
import json,sys
raw=sys.stdin.read().strip()
if not raw: raise SystemExit
try:
    data=json.loads(raw)
except Exception:
    raise SystemExit
panes=data.get("result", data) if isinstance(data, dict) else data
if isinstance(panes, dict):
    panes=panes.get("panes") or panes.get("result") or []
for p in panes if isinstance(panes, list) else []:
    if not isinstance(p, dict):
        continue
    title=(p.get("title") or p.get("name") or p.get("label") or "")
    if title == "Files" or "herdr-explorer" in str(p.get("command") or ""):
        print(p.get("pane_id") or p.get("id") or "")
        break
' 2>/dev/null || true)"
fi

if [ -z "$our_id" ] && [ -n "$list" ]; then
  # Text fallback: first line mentioning Files / herdr-explorer with a pane id-ish token
  our_id="$(printf '%s' "$list" | awk '/Files|herdr-explorer/ {for(i=1;i<=NF;i++) if($i ~ /^w[0-9]+:p[0-9]+$/){print $i; exit}}')"
fi

focused_id=""
if command -v python3 >/dev/null 2>&1 && [ -n "$current" ]; then
  focused_id="$(printf '%s' "$current" | python3 -c '
import json,sys
raw=sys.stdin.read().strip()
if not raw: raise SystemExit
try:
    data=json.loads(raw)
except Exception:
    raise SystemExit
p=data.get("result", data)
if isinstance(p, dict):
    print(p.get("pane_id") or p.get("id") or "")
' 2>/dev/null || true)"
fi

if [ -n "$our_id" ]; then
  if [ "$our_id" = "$focused_id" ]; then
    "$herdr" pane close "$our_id"
    exit 0
  fi
  "$herdr" pane zoom "$our_id" --on >/dev/null 2>&1 || true
  "$herdr" pane zoom "$our_id" --off >/dev/null 2>&1 || true
  exit 0
fi

# Open: split focused pane, swap new pane into left slot (~25% width).
fcwd=""
if [ -n "${HERDR_PLUGIN_CONTEXT_JSON:-}" ] && command -v python3 >/dev/null 2>&1; then
  fcwd="$(printf '%s' "$HERDR_PLUGIN_CONTEXT_JSON" | python3 -c '
import json,sys
try:
    d=json.loads(sys.stdin.read())
except Exception:
    raise SystemExit
print(d.get("workspace_cwd") or d.get("focused_pane_cwd") or "")
' 2>/dev/null || true)"
fi

target="$focused_id"
if [ -z "$target" ]; then
  target="$("$herdr" pane current 2>/dev/null | python3 -c '
import json,sys
try:
    d=json.loads(sys.stdin.read())
    p=d.get("result", d)
    print(p.get("pane_id") or p.get("id") or "")
except Exception:
    pass
' 2>/dev/null || true)"
fi

if [ -z "$target" ]; then
  exec "$herdr" plugin pane open \
    --plugin herdr-explorer \
    --entrypoint tree \
    --placement split \
    --direction right \
    --focus
fi

# ratio = original pane share; after swap, explorer sits in that left slot.
ratio="0.25"
out="$("$herdr" pane split "$target" --direction right --ratio "$ratio" \
  ${fcwd:+--cwd "$fcwd"} --no-focus 2>/dev/null || true)"
np="$(printf '%s' "$out" | python3 -c '
import json,sys,re
raw=sys.stdin.read()
try:
    d=json.loads(raw)
    p=d.get("result", d)
    if isinstance(p, dict):
        print(p.get("pane_id") or p.get("id") or "")
        raise SystemExit
except Exception:
    pass
m=re.search(r"w[0-9]+:p[0-9]+", raw)
print(m.group(0) if m else "")
' 2>/dev/null || true)"

if [ -z "$np" ]; then
  # Last resort
  exec "$herdr" plugin pane open \
    --plugin herdr-explorer \
    --entrypoint tree \
    --placement split \
    --direction right \
    --focus
fi

# Swap new pane into the original (left) slot so the tree sits left.
"$herdr" pane swap --source-pane "$np" --target-pane "$target" >/dev/null 2>&1 || true
"$herdr" pane run "$np" "exec \"$bin\""
"$herdr" pane rename "$np" Files >/dev/null 2>&1 || true
"$herdr" pane zoom "$np" --on >/dev/null 2>&1 || true
"$herdr" pane zoom "$np" --off >/dev/null 2>&1 || true
