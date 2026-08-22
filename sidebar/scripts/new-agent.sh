#!/usr/bin/env bash
# New Grok session in ~/Desktop/projects (new workspace + agent).
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
root="${HERDR_NEW_AGENT_CWD:-$HOME/Desktop/projects}"
kind="${HERDR_NEW_AGENT_KIND:-grok}"

python3 - "$herdr" "$root" "$kind" <<'PY'
import json, os, subprocess, sys

herdr, root, kind = sys.argv[1], sys.argv[2], sys.argv[3]

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
    names = set()
    blob = data.get("result", data)
    agents = blob.get("agents") if isinstance(blob, dict) else None
    if not isinstance(agents, list):
        return names
    for a in agents:
        if isinstance(a, dict):
            n = a.get("name") or a.get("agent")
            if isinstance(n, str):
                names.add(n)
    return names

created = jcmd("workspace", "create", "--cwd", root, "--label", kind, "--focus")
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
print(json.dumps({"pane_id": pane, "agent": name, "cwd": root}))
PY
