#!/usr/bin/env python3
"""Sidebar-style agent picker: highlight walk, Enter focuses, Esc cancels."""
from __future__ import annotations

import json
import os
import subprocess
import sys
import termios
import tty

HERDR = os.environ.get("HERDR_BIN_PATH") or os.environ.get("HERDR_BIN") or "herdr"
STATUS_RANK = {"blocked": 0, "working": 1, "done": 2, "idle": 3, "unknown": 4}


def jcmd(*args: str):
    try:
        out = subprocess.check_output([HERDR, *args], text=True, stderr=subprocess.DEVNULL)
        return json.loads(out)
    except Exception:
        return None


def load_rows() -> list[dict]:
    agents = ((jcmd("agent", "list") or {}).get("result") or {}).get("agents") or []
    spaces = ((jcmd("workspace", "list") or {}).get("result") or {}).get("workspaces") or []
    labels = {
        w.get("workspace_id"): (w.get("label") or w.get("workspace_id") or "")
        for w in spaces
    }
    dirty = {
        w.get("workspace_id"): (w.get("tokens") or {}).get("dirty")
        for w in spaces
    }
    rows = []
    for a in agents:
        wid = a.get("workspace_id")
        title = labels.get(wid) or wid or "?"
        star = dirty.get(wid) or (a.get("tokens") or {}).get("dirty") or ""
        if star and not title.endswith(star):
            title = f"{title} {star}".rstrip()
        sub = (
            (a.get("tokens") or {}).get("folder")
            or a.get("terminal_title_stripped")
            or a.get("terminal_title")
            or a.get("agent")
            or ""
        )
        rows.append(
            {
                "pane_id": a.get("pane_id"),
                "name": a.get("name") or "",
                "title": title,
                "sub": sub,
                "status": a.get("agent_status") or "unknown",
                "focused": bool(a.get("focused")),
                "kind": a.get("agent") or "",
            }
        )
    # Match ui.agent_panel_sort = "priority"
    rows.sort(
        key=lambda r: (
            STATUS_RANK.get(r["status"], 9),
            0 if r["focused"] else 1,
            r["title"].lower(),
            r["sub"].lower(),
        )
    )
    return [r for r in rows if r.get("pane_id")]


def focus_row(row: dict) -> None:
    target = row.get("name") or row.get("pane_id")
    if not target:
        return
    subprocess.run(
        [HERDR, "agent", "focus", target],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=False,
    )


def read_key() -> str:
    ch = sys.stdin.read(1)
    if ch != "\x1b":
        return ch
    # ESC alone or CSI sequence. Poll briefly so bare Esc cancels.
    import select

    if not select.select([sys.stdin], [], [], 0.05)[0]:
        return "esc"
    rest = sys.stdin.read(1)
    if rest != "[":
        return "esc"
    if not select.select([sys.stdin], [], [], 0.05)[0]:
        return "esc"
    code = sys.stdin.read(1)
    if code == "A":
        return "up"
    if code == "B":
        return "down"
    return "esc"


def draw(rows: list[dict], idx: int) -> None:
    cols = 56
    try:
        cols = max(40, os.get_terminal_size().columns)
    except Exception:
        pass
    sys.stdout.write("\033[H\033[J")
    sys.stdout.write("agents  ·  ↑↓ move  ·  enter focus  ·  esc cancel\n")
    sys.stdout.write("─" * min(cols, 56) + "\n")
    for i, r in enumerate(rows):
        mark = ">" if i == idx else " "
        title = r["title"]
        sub = r["sub"]
        line1 = f"{mark} {title}"
        line2 = f"  {sub}" if sub and sub != title else ""
        if i == idx:
            sys.stdout.write(f"\033[7m{line1[:cols]}\033[0m\n")
            if line2:
                sys.stdout.write(f"\033[7m{line2[:cols]}\033[0m\n")
        else:
            sys.stdout.write(line1[:cols] + "\n")
            if line2:
                sys.stdout.write(f"\033[2m{line2[:cols]}\033[0m\n")
    sys.stdout.flush()


def main() -> int:
    rows = load_rows()
    if not rows:
        sys.stdout.write("no agents\n")
        sys.stdout.flush()
        return 0

    idx = next((i for i, r in enumerate(rows) if r["focused"]), 0)
    delta = (os.environ.get("HERDR_AGENT_PICKER_DELTA") or "").strip().lower()
    if delta in ("up", "-1", "prev"):
        idx = (idx - 1) % len(rows)
    elif delta in ("down", "+1", "next"):
        idx = (idx + 1) % len(rows)

    if not sys.stdin.isatty():
        # Non-interactive fallback: focus the selected row immediately.
        focus_row(rows[idx])
        return 0

    fd = sys.stdin.fileno()
    old = termios.tcgetattr(fd)
    try:
        tty.setraw(fd)
        # hide cursor
        sys.stdout.write("\033[?25l")
        sys.stdout.flush()
        while True:
            draw(rows, idx)
            key = read_key()
            if key in ("q", "\x03", "esc"):
                return 0
            if key in ("up", "k", "p"):
                idx = (idx - 1) % len(rows)
            elif key in ("down", "j", "n"):
                idx = (idx + 1) % len(rows)
            elif key in ("\r", "\n", " "):
                # restore tty before focusing so herdr can take over cleanly
                break
            elif key == "\x12":  # ctrl+r refresh
                focused = rows[idx]["pane_id"]
                rows = load_rows()
                if not rows:
                    return 0
                idx = next((i for i, r in enumerate(rows) if r["pane_id"] == focused), 0)
        chosen = rows[idx]
    finally:
        sys.stdout.write("\033[?25h\033[H\033[J")
        sys.stdout.flush()
        termios.tcsetattr(fd, termios.TCSADRAIN, old)

    focus_row(chosen)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
