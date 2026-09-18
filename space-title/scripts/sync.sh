#!/usr/bin/env bash
# Copy status emoji + folder name onto the workspace label (title).
# Baking the emoji into the label avoids Herdr's · between adjacent tokens.
# Copy the Grok/agent session title onto $folder (subtitle).
# Pin the live Grok session UUID on the workspace so `g` can resume it
# after the pane process dies.
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
export HERDR_BIN="$herdr"

python3 <<'PY'
import json, os, subprocess, re
from pathlib import Path

herdr = os.environ["HERDR_BIN"]
SKIP = {"grok", "claude", "codex", "zsh", "bash", "fish", "nu"}
STATUS_EMOJI = {
    "working": "🔨",
    "done": "✅",
    "blocked": "⚠️",
    "idle": "⚪",
}
IDLE_EMOJI = STATUS_EMOJI["idle"]
SUF = re.compile(
    r"\s+[-–—]\s+(grok|claude|codex|opencode|gemini|cursor)\s*$",
    re.I,
)
UUID = re.compile(
    r"/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})(?:/|$)"
)
GROK_SESS = Path.home() / ".grok" / "sessions"

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

def space_title(fold: str, status: str | None) -> str:
    return f"{STATUS_EMOJI.get(status or '', IDLE_EMOJI)} {fold}"

def folder_name(p: dict) -> str | None:
    cwd = p.get("foreground_cwd") or p.get("cwd")
    if not cwd:
        return None
    name = Path(cwd).name
    if not name or name in (".", "/"):
        return None
    return name

def parse_environ(raw: bytes) -> dict[str, str]:
    env = {}
    for item in raw.split(b"\x00"):
        if b"=" not in item:
            continue
        key, value = item.split(b"=", 1)
        try:
            env[key.decode()] = value.decode()
        except Exception:
            continue
    return env

def session_score(sid: str, cwd: str) -> int:
    matches = list(GROK_SESS.glob(f"*/{sid}"))
    if not matches:
        return -1
    path = matches[0]
    score = 0
    if "subagents" not in path.parts:
        score += 10
    summary_path = path / "summary.json"
    try:
        summary = json.loads(summary_path.read_text())
        scwd = (summary.get("info") or {}).get("cwd") or ""
        if cwd and scwd == cwd:
            score += 5
    except Exception:
        pass
    if (path / "chat_history.jsonl.lock").exists():
        score += 3
    return score

def grok_summaries() -> list[dict]:
    rows = []
    try:
        paths = GROK_SESS.glob("*/*/summary.json")
    except Exception:
        return rows
    for path in paths:
        if "subagents" in path.parts:
            continue
        try:
            data = json.loads(path.read_text())
        except Exception:
            continue
        info = data.get("info") or {}
        sid = info.get("id") or path.parent.name
        title = (data.get("generated_title") or data.get("session_summary") or "").strip()
        rows.append({"id": sid, "title": title, "cwd": info.get("cwd") or ""})
    return rows

def match_session_by_title(label: str, cwd: str, rows: list[dict]) -> str | None:
    lab = (label or "").replace("…", "").strip().lower()
    if len(lab) < 8:
        return None
    hits = []
    for row in rows:
        title = (row.get("title") or "").lower()
        if not title:
            continue
        if not (title.startswith(lab) or lab.startswith(title[: min(len(title), len(lab))])):
            continue
        score = 2
        if cwd and row.get("cwd") == cwd:
            score += 5
        hits.append((score, row["id"]))
    if not hits:
        return None
    hits.sort(reverse=True)
    top = hits[0][0]
    ids = {sid for score, sid in hits if score == top}
    if len(ids) != 1:
        return None
    return next(iter(ids))

def live_grok_sessions() -> dict[str, str]:
    """workspace_id -> grok session UUID from live processes."""
    best: dict[str, tuple[int, str]] = {}
    proc = Path("/proc")
    try:
        pids = list(proc.iterdir())
    except Exception:
        return {}
    for pdir in pids:
        if not pdir.name.isdigit():
            continue
        try:
            raw = (pdir / "environ").read_bytes()
        except Exception:
            continue
        if b"HERDR_WORKSPACE_ID=" not in raw:
            continue
        env = parse_environ(raw)
        wid = env.get("HERDR_WORKSPACE_ID")
        if not wid:
            continue
        cwd = env.get("PWD") or ""
        ids = set()
        try:
            for fd in (pdir / "fd").iterdir():
                try:
                    target = os.readlink(fd)
                except Exception:
                    continue
                if "/.grok/sessions/" not in target:
                    continue
                match = UUID.search(target)
                if match:
                    ids.add(match.group(1))
        except Exception:
            continue
        for sid in ids:
            score = session_score(sid, cwd)
            if score < 0:
                continue
            prev = best.get(wid)
            if prev is None or score > prev[0]:
                best[wid] = (score, sid)
    return {wid: sid for wid, (_score, sid) in best.items()}

panes = ((jcmd("pane", "list") or {}).get("result") or {}).get("panes") or []
spaces = ((jcmd("workspace", "list") or {}).get("result") or {}).get("workspaces") or []
labels = {w.get("workspace_id"): (w.get("label") or "") for w in spaces}
statuses = {w.get("workspace_id"): w.get("agent_status") for w in spaces}
cwds: dict[str, str] = {}
for p in panes:
    wid = p.get("workspace_id")
    cwd = p.get("foreground_cwd") or p.get("cwd")
    if wid and cwd and (wid not in cwds or p.get("agent") or p.get("focused")):
        cwds[wid] = cwd
summaries = grok_summaries()

best: dict[str, str] = {}
folders: dict[str, str] = {}
agent_panes: dict[str, str] = {}
sessions: dict[str, str] = live_grok_sessions()
for p in panes:
    wid = p.get("workspace_id")
    fold = folder_name(p)
    if wid and fold and (wid not in folders or p.get("focused") or p.get("agent")):
        folders[wid] = fold
    reported = p.get("agent_session_id") or (p.get("tokens") or {}).get("grok_session")
    if wid and reported and wid not in sessions:
        sessions[wid] = reported
    if p.get("agent") == "grok" and wid and p.get("pane_id"):
        agent_panes[wid] = p.get("pane_id")
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

for wid, fold in folders.items():
    lab = space_title(fold, statuses.get(wid))
    if lab != labels.get(wid, ""):
        subprocess.run(
            [herdr, "workspace", "rename", wid, lab],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )

for wid, lab in best.items():
    subprocess.run(
        [herdr, "workspace", "report-metadata", wid, "--source", "herdr-space-title", "--token", f"folder={lab}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )

for w in spaces:
    wid = w.get("workspace_id")
    if not wid or wid in sessions:
        continue
    existing = (w.get("tokens") or {}).get("grok_session")
    if existing:
        sessions[wid] = existing
        continue
    tokens = w.get("tokens") or {}
    sid = match_session_by_title(tokens.get("folder") or "", cwds.get(wid) or "", summaries)
    if not sid:
        sid = match_session_by_title(w.get("label") or "", cwds.get(wid) or "", summaries)
    if sid:
        sessions[wid] = sid

for wid, sid in sessions.items():
    subprocess.run(
        [herdr, "workspace", "report-metadata", wid, "--source", "herdr-space-title", "--token", f"grok_session={sid}"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )
    pane_id = agent_panes.get(wid)
    if pane_id:
        subprocess.run(
            [
                herdr,
                "pane",
                "report-agent-session",
                pane_id,
                "--source",
                "herdr-space-title",
                "--agent",
                "grok",
                "--agent-session-id",
                sid,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
PY
