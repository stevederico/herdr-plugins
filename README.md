# herdr-plugins

Steve’s local [herdr](https://herdr.dev) plugins in one repo.

| Dir | Plugin id | What |
|-----|-----------|------|
| `sidebar/` | `herdr-sidebar` | VS Code-style explorer + SCM (single-click folders) |
| `git-badge/` | `herdr-git-badge` | Spaces `*` when worktree is dirty |
| `space-title/` | `herdr-space-title` | Rename spaces from Grok session titles |
| `explorer/` | `herdr-explorer` | Thin file tree (optional; usually disabled) |

## Link (dev)

Repo lives at `~/Projects/herdr-plugins`. Links store absolute paths, so
moving the repo breaks every plugin (`manifest unavailable`) — unlink and
relink after any move.

```bash
herdr plugin uninstall herdr-sidebar   # if still GitHub-managed
herdr plugin link ~/Projects/herdr-plugins/sidebar
herdr plugin link ~/Projects/herdr-plugins/git-badge
herdr plugin link ~/Projects/herdr-plugins/space-title
herdr plugin link ~/Projects/herdr-plugins/explorer   # optional
herdr plugin disable herdr-explorer    # if you only want sidebar
```

Verify: `herdr plugin list` — no `warning:` lines.

## Build

```bash
(cd sidebar && cargo build --release)
(cd explorer && cargo build --release)
# git-badge is bash-only
```

After sidebar rebuild: `herdr plugin action invoke herdr-sidebar.redeploy`
