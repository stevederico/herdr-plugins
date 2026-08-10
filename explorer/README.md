# herdr-explorer

Thin left-dock **file tree** for [herdr](https://herdr.dev). Yours — not a fork of the marketplace explorers.

## Keys

| Key | Action |
|-----|--------|
| `j` / `k` | move |
| `h` | parent dir |
| `l` / `Enter` | open dir or file in `$EDITOR` |
| `e` | edit file |
| `r` | reload |
| `q` | quit pane |

## Dev

```bash
cargo build --release
herdr plugin link .
herdr plugin action invoke herdr-explorer.open
```

Optional keybind in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+e"
type = "plugin_action"
command = "herdr-explorer.open"
description = "toggle file tree"
```

Then `herdr server reload-config`.

## Disable competing sidebar

```bash
herdr plugin disable herdr-sidebar
```
