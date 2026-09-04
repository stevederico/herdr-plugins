# herdr-sidebar

**The sidebar your terminal was missing:** a VS Code-inspired file explorer + source
control panel in one dockable herdr pane.

<img src="docs/media/hero.png" alt="The sidebar: explorer view with a live file preview beside it" width="860">

Features, screenshots, keys, and settings live in the
[repo README](../README.md).

## Install

From this checkout:

```bash
cargo build --release
herdr plugin link .
```

Open it (or just focus a tab: the hook docks it):

```bash
herdr plugin action invoke herdr-sidebar.open-sidebar-windows   # windows
herdr plugin action invoke herdr-sidebar.open-sidebar           # linux / macos
```

After a rebuild: `herdr plugin action invoke herdr-sidebar.redeploy`, then click
the file again so the preview process restarts.
