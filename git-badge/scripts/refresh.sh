#!/usr/bin/env bash
# Report $dirty=* when a workspace has uncommitted changes or unpushed commits.
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
source_id="herdr-git-badge"
export HERDR_BIN="$herdr" SOURCE_ID="$source_id"

python3 <<'PY'
import json, os, subprocess
from pathlib import Path

herdr = os.environ["HERDR_BIN"]
source = os.environ["SOURCE_ID"]

def jcmd(*args):
    try:
        out = subprocess.check_output([herdr, *args], text=True, stderr=subprocess.DEVNULL)
        return json.loads(out)
    except Exception:
        return None

def dirty(cwd: str) -> bool:
    try:
        subprocess.check_call(
            ["git", "-C", cwd, "rev-parse", "--is-inside-work-tree"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
    except Exception:
        return False
    try:
        porcelain = subprocess.check_output(
            ["git", "-C", cwd, "status", "--porcelain"],
            text=True, stderr=subprocess.DEVNULL,
        )
        if porcelain.strip():
            return True
    except Exception:
        return False
    try:
        ahead = subprocess.check_output(
            ["git", "-C", cwd, "rev-list", "--count", "@{upstream}..HEAD"],
            text=True, stderr=subprocess.DEVNULL,
        )
        return int(ahead.strip() or "0") > 0
    except Exception:
        return False

def git_root(cwd: str) -> str | None:
    try:
        return subprocess.check_output(
            ["git", "-C", cwd, "rev-parse", "--show-toplevel"],
            text=True, stderr=subprocess.DEVNULL,
        ).strip() or None
    except Exception:
        return None

def child_git_roots(cwd: str) -> list[str]:
    """One-level scan: umbrella folders like ~/Projects."""
    p = Path(cwd)
    if not p.is_dir():
        return []
    roots = []
    try:
        for child in p.iterdir():
            if child.is_dir() and not child.name.startswith(".") and (child / ".git").exists():
                roots.append(str(child))
    except Exception:
        pass
    return roots

panes = ((jcmd("pane", "list") or {}).get("result") or {}).get("panes") or []
ws_rows = ((jcmd("workspace", "list") or {}).get("result") or {}).get("workspaces") or []

cands: dict[str, set[str]] = {}
for p in panes:
    wid = p.get("workspace_id")
    for key in ("foreground_cwd", "cwd"):
        cwd = p.get(key)
        if wid and cwd:
            cands.setdefault(wid, set()).add(cwd)

try:
    sess = json.loads(Path.home().joinpath(".config/herdr/session.json").read_text())
    for w in sess.get("workspaces") or []:
        wid, icwd = w.get("id"), w.get("identity_cwd")
        if wid and icwd:
            cands.setdefault(wid, set()).add(icwd)
except Exception:
    pass

def set_dirty(wid: str, on: bool):
    args = [herdr, "workspace", "report-metadata", wid, "--source", source]
    if on:
        args += ["--token", "dirty=*"]
    else:
        args += ["--clear-token", "dirty"]
    subprocess.run(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

for w in ws_rows:
    wid = w.get("workspace_id") or w.get("id")
    if not wid:
        continue
    roots: set[str] = set()
    for cwd in cands.get(wid, ()):
        root = git_root(cwd)
        if root:
            roots.add(root)
        for child in child_git_roots(cwd):
            roots.add(child)
    set_dirty(wid, any(dirty(r) for r in roots))
PY
