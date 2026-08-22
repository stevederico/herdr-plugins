#!/usr/bin/env bash
# Copy Grok session titles onto workspace labels.
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
export HERDR_BIN="$herdr"

python3 <<'PY'
import json, os, subprocess, re
from pathlib import Path

herdr = os.environ["HERDR_BIN"]
SKIP = {"grok", "claude", "codex", "zsh", "bash", "fish", "nu"}
SUF = re.compile(
    r"\s+[-–—]\s+(grok|claude|codex|opencode|gemini|cursor)\s*$",
    re.I,
)

def jcmd(*args):
    try:
        out = subprocess.check_output([herdr, *args], text=True, stderr=subprocess.DEVNULL)
        return json.loads(out)
    except Exception:
        return None

def session_label(title: str | None) -> str | None:
    t = (title or "").replace("…", "").strip()
    t = SUF.sub("", t).strip(" -–—")
    if not t or t.lower() in SKIP:
        return None
    if len(t) > 42:
        t = t[:41].rstrip() + "…"
    return t

def folder_name(p: dict) -> str | None:
    cwd = p.get("foreground_cwd") or p.get("cwd")
    if not cwd:
        return None
    name = Path(cwd).name
    if not name or name in (".", "/"):
        return None
    return name

panes = ((jcmd("pane", "list") or {}).get("result") or {}).get("panes") or []
spaces = ((jcmd("workspace", "list") or {}).get("result") or {}).get("workspaces") or []
labels = {w.get("workspace_id"): (w.get("label") or "") for w in spaces}

best: dict[str, str] = {}
folders: dict[str, str] = {}
for p in panes:
    wid = p.get("workspace_id")
    fold = folder_name(p)
    if wid and fold and (wid not in folders or p.get("focused") or p.get("agent")):
        folders[wid] = fold
    if not p.get("agent"):
        continue
    pid = p.get("pane_id")
    if pid and fold:
        subprocess.run(
            [herdr, "pane", "report-metadata", pid, "--source", "herdr-space-title", "--token", f"folder={fold}"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    title = p.get("terminal_title_stripped") or p.get("terminal_title")
    lab = session_label(title)
    if not wid or not lab:
        continue
    if wid not in best or p.get("focused"):
        best[wid] = lab

for wid, lab in best.items():
    cur = labels.get(wid, "")
    if lab and lab != cur:
        subprocess.run(
            [herdr, "workspace", "rename", wid, lab],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )

for wid, fold in folders.items():
    subprocess.run(
        [herdr, "workspace", "report-metadata", wid, "--source", "herdr-space-title", "--token", f"folder={fold}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
PY
