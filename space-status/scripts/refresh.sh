#!/usr/bin/env bash
# Clear $status tokens so the Agents sidebar stays emoji-free.
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
source_id="herdr-space-status"
export HERDR_BIN="$herdr" SOURCE_ID="$source_id"

python3 <<'PY'
import json, os, subprocess

herdr = os.environ["HERDR_BIN"]
source = os.environ["SOURCE_ID"]

def jcmd(*args):
    try:
        out = subprocess.check_output([herdr, *args], text=True, stderr=subprocess.DEVNULL)
        return json.loads(out)
    except Exception:
        return None

def clear_status(wid: str):
    subprocess.run(
        [herdr, "workspace", "report-metadata", wid, "--source", source, "--clear-token", "status"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

ws_rows = ((jcmd("workspace", "list") or {}).get("result") or {}).get("workspaces") or []
for w in ws_rows:
    wid = w.get("workspace_id") or w.get("id")
    if wid:
        clear_status(wid)
PY
