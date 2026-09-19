#!/usr/bin/env bash
# Copy status emoji + project folder onto the workspace label (title).
# Baking the emoji into the label avoids Herdr's · between adjacent tokens.
# Copy the Grok/agent session title onto $folder (subtitle).
# Pin the live Grok session UUID on the workspace so `g` can resume it
# after the pane process dies.
#
# Pane cwd is often an umbrella (~/Projects, ~) from session start. Prefer
# the git root, then a sibling pane in a child repo, then tool/memory
# paths from this Grok session (so a later restocks-gpu chat still sitting
# in ~/Projects gets the new folder), then chat path mentions.
set -euo pipefail
herdr="${HERDR_BIN_PATH:-herdr}"
export HERDR_BIN="$herdr"

python3 <<'PY'
import json, os, subprocess, re
from functools import lru_cache
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
UMBRELLA_NAMES = {
    "projects",
    "project",
    "work",
    "src",
    "code",
    "repos",
    "repo",
    "dev",
    "desktop",
    "documents",
    "downloads",
    "home",
}
# Always pulled into Grok context; never treat as "the project" unless the
# session title or a pane cwd actually points at it.
CONTEXT_REPOS = {"brain"}
SUF = re.compile(
    r"\s+[-–—]\s+(grok|claude|codex|opencode|gemini|cursor)\s*$",
    re.I,
)
UUID = re.compile(
    r"/([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})(?:/|$)"
)
GROK_SESS = Path.home() / ".grok" / "sessions"
HOME = Path.home()

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

def is_tmp(path: str) -> bool:
    p = Path(path).as_posix()
    return (
        p == "/tmp"
        or p.startswith("/tmp/")
        or p.startswith("/var/tmp")
        or p.startswith("/dev/")
        or p.startswith("/proc/")
    )

def usable_path(path: str | None) -> str | None:
    if not path:
        return None
    p = Path(path)
    if not p.is_absolute() or is_tmp(path):
        return None
    return str(p)

def is_umbrella(path: str) -> bool:
    p = Path(path)
    try:
        if p.resolve() == HOME.resolve():
            return True
    except Exception:
        if p == HOME:
            return True
    return p.name.lower() in UMBRELLA_NAMES

@lru_cache(maxsize=256)
def git_root(cwd: str) -> str | None:
    try:
        return subprocess.check_output(
            ["git", "-C", cwd, "rev-parse", "--show-toplevel"],
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip() or None
    except Exception:
        return None

@lru_cache(maxsize=32)
def child_git_roots(cwd: str) -> tuple[str, ...]:
    p = Path(cwd)
    if not p.is_dir():
        return ()
    roots = []
    try:
        for child in p.iterdir():
            if child.is_dir() and not child.name.startswith(".") and (child / ".git").exists():
                roots.append(str(child))
    except Exception:
        pass
    return tuple(roots)

def bags_for(cwd: str) -> list[str]:
    bags = [cwd]
    p = Path(cwd)
    try:
        home = p.resolve() == HOME.resolve()
    except Exception:
        home = p == HOME
    if home:
        for extra in ("Projects", "Work", "code", "src"):
            d = HOME / extra
            if d.is_dir():
                bags.append(str(d))
    return bags

def project_from_path(path: str) -> str | None:
    root = git_root(path)
    if root and not is_umbrella(root):
        return Path(root).name
    if not is_umbrella(path):
        name = Path(path).name
        if name and name not in (".", "/"):
            return name
    return None

PATH_KEYS = {
    "path",
    "target_file",
    "file_path",
    "cwd",
    "working_directory",
    "target_directory",
    "directory",
}
TOPIC_RE = re.compile(r"/memory-v2/workspaces/[^/]+/topics/([^/]+)\.md$")

def session_dir(sid: str) -> Path | None:
    hits = list(GROK_SESS.glob(f"*/{sid}"))
    return hits[0] if hits else None

def read_tail(path: Path, limit: int) -> str:
    try:
        raw = path.read_bytes()
    except Exception:
        return ""
    if len(raw) > limit:
        raw = raw[-limit:]
    return raw.decode("utf-8", errors="ignore")

def session_summary(sid: str) -> dict:
    p = session_dir(sid)
    if not p:
        return {}
    summary = p / "summary.json"
    if not summary.exists():
        return {}
    try:
        return json.loads(summary.read_text())
    except Exception:
        return {}

def session_text(sid: str) -> str:
    p = session_dir(sid)
    if not p:
        return ""
    chunks = []
    data = session_summary(sid)
    for key in ("generated_title", "session_summary", "last_recap", "last_turn_summary"):
        val = data.get(key)
        if val:
            chunks.append(str(val))
    root = data.get("git_root_dir")
    if root:
        chunks.append(str(root))
    hist = p / "chat_history.jsonl"
    if hist.exists():
        chunks.append(read_tail(hist, 2_000_000))
    return "\n".join(chunks)

def session_headings(sid: str) -> str:
    data = session_summary(sid)
    parts = [
        data.get("generated_title"),
        data.get("session_summary"),
        data.get("last_recap"),
        data.get("last_turn_summary"),
    ]
    return " ".join(str(p) for p in parts if p)

def title_mentions(name: str, title: str) -> bool:
    if len(name) < 4:
        return False
    return re.search(rf"(?i)(?<![A-Za-z0-9]){re.escape(name)}(?![A-Za-z0-9])", title) is not None

def map_tool_path(path: str, child_by_name: dict[str, str], children: list[str]) -> str | None:
    if not path:
        return None
    m = TOPIC_RE.search(path.replace("\\", "/"))
    if m:
        return child_by_name.get(m.group(1).lower())
    for child in children:
        if path == child or path.startswith(child.rstrip("/") + "/"):
            return child
    return None

def paths_from_tool_obj(obj) -> list[str]:
    out = []
    if isinstance(obj, dict):
        for key, val in obj.items():
            if key in PATH_KEYS and isinstance(val, str):
                out.append(val)
            elif key == "locations" and isinstance(val, list):
                for loc in val:
                    if isinstance(loc, dict) and isinstance(loc.get("path"), str):
                        out.append(loc["path"])
            elif key == "rawInput" and isinstance(val, dict):
                out.extend(paths_from_tool_obj(val))
            elif key == "arguments" and isinstance(val, str):
                try:
                    out.extend(paths_from_tool_obj(json.loads(val)))
                except Exception:
                    pass
            elif key == "input" and isinstance(val, str) and val.startswith("{"):
                try:
                    out.extend(paths_from_tool_obj(json.loads(val)))
                except Exception:
                    pass
    elif isinstance(obj, list):
        for item in obj:
            out.extend(paths_from_tool_obj(item))
    return out

def session_tool_projects(sid: str, children: list[str]) -> dict[str, int]:
    """Count this session's tool paths / memory topics that map to child repos.

    Ignores tool result bodies so a read of omarchy-plugins.md does not
    count every repo that file happens to mention.
    """
    scores: dict[str, int] = {}
    child_by_name = {Path(c).name.lower(): c for c in children}
    p = session_dir(sid)
    if not p:
        return scores

    def bump(path: str):
        hit = map_tool_path(path, child_by_name, children)
        if hit:
            scores[hit] = scores.get(hit, 0) + 1

    git_root_dir = session_summary(sid).get("git_root_dir")
    if isinstance(git_root_dir, str):
        bump(git_root_dir.rstrip("/"))

    updates = p / "updates.jsonl"
    if updates.exists():
        for line in read_tail(updates, 12_000_000).splitlines():
            try:
                rec = json.loads(line)
            except Exception:
                continue
            update = ((rec.get("params") or {}).get("update")) or {}
            for path in paths_from_tool_obj(update):
                bump(path)

    hist = p / "chat_history.jsonl"
    if hist.exists():
        for line in read_tail(hist, 3_000_000).splitlines():
            try:
                rec = json.loads(line)
            except Exception:
                continue
            kind = rec.get("type")
            if kind == "assistant":
                for path in paths_from_tool_obj(rec.get("tool_calls")):
                    bump(path)
            elif kind == "backend_tool_call":
                for path in paths_from_tool_obj(rec.get("kind")):
                    bump(path)
    return scores

def infer_project(cwd: str, sid: str | None, title: str, extra_paths: list[str]) -> str | None:
    children: list[str] = []
    for bag in bags_for(cwd):
        children.extend(child_git_roots(bag))
    if not children:
        return None
    pane_hits: set[str] = set()
    for extra in extra_paths:
        root = git_root(extra) or extra
        for child in children:
            if extra == child or extra.startswith(child.rstrip("/") + "/") or root == child:
                pane_hits.add(child)
                break
    if len(pane_hits) == 1:
        return Path(next(iter(pane_hits))).name

    t = " ".join(x for x in (title, session_headings(sid) if sid else "") if x)
    pool = pane_hits or set(children)
    title_hits = {c for c in pool if title_mentions(Path(c).name, t)}
    if len(title_hits) == 1:
        return Path(next(iter(title_hits))).name

    if sid:
        tool_hits = {
            child: n
            for child, n in session_tool_projects(sid, children).items()
            if child in pool
        }
        for child in list(tool_hits):
            if Path(child).name.lower() in CONTEXT_REPOS and child not in title_hits:
                tool_hits.pop(child, None)
        ranked_tools = sorted(tool_hits.items(), key=lambda kv: kv[1], reverse=True)
        if ranked_tools:
            if len(ranked_tools) == 1 or ranked_tools[0][1] >= ranked_tools[1][1] * 1.5:
                return Path(ranked_tools[0][0]).name

    text = session_text(sid) if sid else ""
    mentions: dict[str, int] = {}
    for child in pool:
        name = Path(child).name
        if name.lower() in CONTEXT_REPOS and child not in title_hits:
            continue
        n = text.count(child) if text else 0
        try:
            n += text.count("~/" + str(Path(child).relative_to(HOME))) if text else 0
        except Exception:
            pass
        if n:
            mentions[child] = n
    ranked = sorted(mentions.items(), key=lambda kv: kv[1], reverse=True)
    if not ranked or ranked[0][1] < 12:
        return None
    if len(ranked) > 1 and ranked[0][1] < ranked[1][1] * 1.4:
        return None
    return Path(ranked[0][0]).name

def leaf_name(path: str) -> str | None:
    name = Path(path).name
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
ws_paths: dict[str, list[str]] = {}
ws_base: dict[str, str] = {}
for p in panes:
    wid = p.get("workspace_id")
    if not wid:
        continue
    cwd = usable_path(p.get("cwd"))
    fg = usable_path(p.get("foreground_cwd"))
    if cwd:
        ws_paths.setdefault(wid, []).append(cwd)
        if wid not in ws_base or p.get("agent") or p.get("focused"):
            ws_base[wid] = cwd
        if wid not in cwds or p.get("agent") or p.get("focused"):
            cwds[wid] = cwd
    if fg and fg != cwd:
        ws_paths.setdefault(wid, []).append(fg)
        if wid not in ws_base:
            ws_base[wid] = fg
        if wid not in cwds:
            cwds[wid] = fg
summaries = grok_summaries()

best: dict[str, str] = {}
agent_panes: dict[str, str] = {}
sessions: dict[str, str] = live_grok_sessions()
for p in panes:
    wid = p.get("workspace_id")
    reported = p.get("agent_session_id") or (p.get("tokens") or {}).get("grok_session")
    if wid and reported and wid not in sessions:
        sessions[wid] = reported
    if p.get("agent") == "grok" and wid and p.get("pane_id"):
        agent_panes[wid] = p.get("pane_id")
    if not p.get("agent"):
        continue
    title = p.get("terminal_title_stripped") or p.get("terminal_title")
    lab = session_label(title)
    if not wid or not lab:
        continue
    if wid not in best or p.get("focused"):
        best[wid] = lab

for w in spaces:
    wid = w.get("workspace_id")
    if not wid or wid in sessions:
        continue
    existing = (w.get("tokens") or {}).get("grok_session")
    if existing:
        sessions[wid] = existing

folders: dict[str, str] = {}
for wid, base in ws_base.items():
    fold = project_from_path(base)
    if not fold:
        extras = [path for path in ws_paths.get(wid, []) if path != base]
        fold = infer_project(base, sessions.get(wid), best.get(wid) or "", extras)
    if not fold:
        fold = leaf_name(base)
    if fold:
        folders[wid] = fold

for p in panes:
    if not p.get("agent"):
        continue
    pid = p.get("pane_id")
    wid = p.get("workspace_id")
    fold = folders.get(wid) if wid else None
    if pid and fold:
        subprocess.run(
            [herdr, "pane", "report-metadata", pid, "--source", "herdr-space-title", "--token", f"folder={fold}"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )

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
