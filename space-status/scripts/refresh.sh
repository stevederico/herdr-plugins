#!/usr/bin/env bash
# Report $status=🔄/✅/⚠️ from workspace agent_status. Idle clears the token.
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
source_id="herdr-space-status"
export HERDR_BIN="$herdr" SOURCE_ID="$source_id"

python3 <<'PY'
import json, os, subprocess

herdr = os.environ["HERDR_BIN"]
source = os.environ["SOURCE_ID"]

# One glyph per state. Idle/unknown: no mark.
EMOJI = {
    "working": "🔄",
    "done": "✅",
    "blocked": "⚠️",
}

def jcmd(*args):
    try:
        out = subprocess.check_output([herdr, *args], text=True, stderr=subprocess.DEVNULL)
        return json.loads(out)
    except Exception:
        return None

def set_status(wid: str, emoji: str | None):
    args = [herdr, "workspace", "report-metadata", wid, "--source", source]
    if emoji:
        args += ["--token", f"status={emoji}"]
    else:
        args += ["--clear-token", "status"]
    subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

ws_rows = ((jcmd("workspace", "list") or {}).get("result") or {}).get("workspaces") or []
for w in ws_rows:
    wid = w.get("workspace_id") or w.get("id")
    if not wid:
        continue
    set_status(wid, EMOJI.get(w.get("agent_status") or ""))
PY
