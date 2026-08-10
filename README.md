# herdr-plugins

Steve’s local [herdr](https://herdr.dev) plugins in one repo.

| Dir | Plugin id | What |
|-----|-----------|------|
| `sidebar/` | `herdr-sidebar` | VS Code-style explorer + SCM (single-click folders) |
| `git-badge/` | `herdr-git-badge` | Spaces `*` when worktree is dirty |
| `explorer/` | `herdr-explorer` | Thin file tree (optional; usually disabled) |

## Link (dev)

```bash
herdr plugin uninstall herdr-sidebar   # if still GitHub-managed
herdr plugin link ~/Desktop/projects/herdr-plugins/sidebar
herdr plugin link ~/Desktop/projects/herdr-plugins/git-badge
herdr plugin link ~/Desktop/projects/herdr-plugins/explorer   # optional
herdr plugin disable herdr-explorer    # if you only want sidebar
```

## Build

```bash
(cd sidebar && cargo build --release)
(cd explorer && cargo build --release)
# git-badge is bash-only
```

After sidebar rebuild: `herdr plugin action invoke herdr-sidebar.redeploy`
