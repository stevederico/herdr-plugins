#!/usr/bin/env bash
# New workspace + Grok. Cwd follows the focused pane (else ~/Projects).
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
fallback="${HERDR_NEW_AGENT_CWD:-$HOME/Projects}"
kind="${HERDR_NEW_AGENT_KIND:-grok}"

python3 - "$herdr" "$fallback" "$kind" <<'PY'
import json, subprocess, sys

herdr, fallback, kind = sys.argv[1], sys.argv[2], sys.argv[3]

def jcmd(*args):
    out = subprocess.check_output([herdr, *args], text=True, stderr=subprocess.DEVNULL)
    return json.loads(out)

def walk_pane_id(obj):
    if isinstance(obj, dict):
        for key in ("root_pane", "pane"):
            node = obj.get(key)
            if isinstance(node, dict) and node.get("pane_id"):
                return node["pane_id"]
        if obj.get("pane_id"):
            return obj["pane_id"]
        for v in obj.values():
            found = walk_pane_id(v)
            if found:
                return found
    elif isinstance(obj, list):
        for v in obj:
            found = walk_pane_id(v)
            if found:
                return found
    return None

def live_names():
    data = jcmd("agent", "list")
    blob = data.get("result", data)
    agents = blob.get("agents") if isinstance(blob, dict) else []
    names = set()
    for a in agents or []:
        if isinstance(a, dict):
            n = a.get("name") or a.get("agent")
            if isinstance(n, str):
                names.add(n)
    return names

cwd = fallback
try:
    panes = (jcmd("pane", "list").get("result") or {}).get("panes") or []
    focused = next((p for p in panes if p.get("focused")), None)
    if focused and focused.get("cwd"):
        cwd = focused["cwd"]
except Exception:
    pass

created = jcmd("workspace", "create", "--cwd", cwd, "--focus")
pane = walk_pane_id(created)
if not pane:
    raise SystemExit("workspace create did not return a pane id")

taken = live_names()
name = kind
n = 2
while name in taken:
    name = f"{kind}{n}"
    n += 1

jcmd("agent", "start", name, "--kind", kind, "--pane", pane)
print(json.dumps({"pane_id": pane, "agent": name, "cwd": cwd}))
PY
