#!/usr/bin/env bash
# New Grok in the current workspace (split, do not create a space).
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
fallback="${HERDR_NEW_AGENT_CWD:-$HOME/Desktop/projects}"
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

def is_chrome(p):
    label = p.get("label") or ""
    tokens = p.get("tokens") or {}
    blob = " ".join(tokens.keys())
    return label in ("Sidebar", "Explorer", "Preview", "Source Control") or "herdr-sidebar" in blob

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

panes = (jcmd("pane", "list").get("result") or {}).get("panes") or []
focused = next((p for p in panes if p.get("focused")), None)
if not focused:
    raise SystemExit("no focused pane")
tab = focused.get("tab_id")
cwd = focused.get("cwd") or fallback
target = focused
if is_chrome(target):
    others = [p for p in panes if p.get("tab_id") == tab and not is_chrome(p)]
    if others:
        target = others[0]
tid = target.get("pane_id")
if not tid:
    raise SystemExit("no split target")
created = jcmd("pane", "split", tid, "--direction", "down", "--cwd", cwd, "--focus")
pane = walk_pane_id(created)
if not pane:
    raise SystemExit("pane split did not return a pane id")

taken = live_names()
name = kind
n = 2
while name in taken:
    name = f"{kind}{n}"
    n += 1

jcmd("agent", "start", name, "--kind", kind, "--pane", pane)
print(json.dumps({"pane_id": pane, "agent": name, "cwd": cwd}))
PY
