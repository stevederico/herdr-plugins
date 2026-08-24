#!/usr/bin/env bash
# ensure-sidebar.sh — unix [[events]] hook body: make sure the FOCUSED tab has
# a Sidebar pane docked on the left, WITHOUT stealing the user's focus.
#
# Runs on tab.focused / workspace.focused / workspace.created, so it must be
# idempotent and quiet: already present → exit; else open unfocused. After a
# spawn, hold the lock until the TUI stamps its token — prefix+n fires several
# of these events in sequence, and a released lock before the token is ready
# docks a second explorer (title is cleared, so launch_decision cannot see it).
set -uo pipefail

herdr_bin="${HERDR_BIN_PATH:-herdr}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
bin="$script_dir/../target/release/herdr-sidebar"
[ -x "$bin" ] || exit 0

# Focus events arrive in bursts (tab.focused + workspace.focused for one switch)
# and concurrent ensures each open an explorer — serialize with an atomic mkdir
# lock. Losing the race skips this ensure; the next focus event re-fires it.
lock_dir="${TMPDIR:-/tmp}/herdr-sidebar-ensure.lock"
if ! mkdir "$lock_dir" 2>/dev/null; then
  # Break locks older than 30s (a crashed ensure), otherwise yield.
  now="$(date +%s)"
  born="$(stat -c %Y "$lock_dir" 2>/dev/null || stat -f %m "$lock_dir" 2>/dev/null || echo "$now")"
  [ $((now - born)) -gt 30 ] || exit 0
  rm -rf "$lock_dir" 2>/dev/null
  mkdir "$lock_dir" 2>/dev/null || exit 0
fi
trap 'rmdir "$lock_dir" 2>/dev/null' EXIT

# Snapshot AFTER acquiring the lock, so a just-finished ensure's rename is visible.
panes="$("$herdr_bin" pane list 2>/dev/null || true)"
[ -n "$panes" ] || exit 0

decision="$(printf '%s' "$panes" | "$bin" --launch-decision 2>/dev/null || true)"
[ "$decision" = "OPEN" ] || exit 0

# Respect a tab the user toggled closed (open-explorer.sh writes the marker) —
# otherwise the very next focus event would reopen what they just closed.
snooze_dir="${TMPDIR:-/tmp}/herdr-sidebar-snooze"
tab="$(printf '%s' "$panes" | "$bin" --focused-tab 2>/dev/null || true)"
[ -n "$tab" ] && [ -f "$snooze_dir/${tab//:/_}" ] && exit 0

fp="$(printf '%s' "$panes" | "$bin" --focused-pane 2>/dev/null || true)"
fid="${fp%%	*}"
fcwd="${fp#*	}"
acwd="$(printf '%s' "$panes" | "$bin" --agent-cwd 2>/dev/null || true)"
[ -n "$acwd" ] && fcwd="$acwd"
[ -n "$fid" ] || exit 0

target="$fid"
ratio="0.25"
plan="$("$herdr_bin" pane layout --pane "$fid" 2>/dev/null | "$bin" --open-plan 2>/dev/null || true)"
if [ -n "$plan" ]; then
  target="${plan%%	*}"
  ratio="${plan#*	}"
fi

out="$("$herdr_bin" pane split "$target" --direction right --ratio "$ratio" \
  ${fcwd:+--cwd "$fcwd"} --no-focus 2>/dev/null || true)"
np="$(printf '%s' "$out" | sed -n 's/.*"pane_id":"\([^"]*\)".*/\1/p' | head -n1)"
[ -n "$np" ] || exit 0

"$herdr_bin" pane swap --source-pane "$np" --target-pane "$target" >/dev/null 2>&1 || true
"$herdr_bin" pane run "$np" "exec \"$bin\""
"$herdr_bin" pane rename "$np" --clear >/dev/null 2>&1 || true

# Hold the lock until the TUI stamps its identity token (~1-2s). Title is
# cleared (0.7.0) so launch_decision cannot see this pane until the token
# exists; sequential workspace.created / tab.created / pane.focused hooks
# from prefix+n would otherwise each OPEN and dock a second explorer.
for _ in $(seq 1 30); do
  list="$("$herdr_bin" pane list 2>/dev/null || true)"
  has="$(printf '%s' "$list" | "$bin" --has-token "$np" 2>/dev/null || true)"
  [ "$has" = "yes" ] && break
  sleep 0.2
done

# Hand focus back if the swap left it on the explorer (focus follows the slot).
if [ "$target" = "$fid" ]; then
  "$herdr_bin" pane focus --direction right --pane "$np" >/dev/null 2>&1 || true
fi
exit 0
