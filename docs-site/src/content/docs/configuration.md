---
title: Configuration & CLI
description: Customize shortcuts, growth, width changes, and scripted window positioning.
---

## Settings

Open **Settings…** from the tray. Hover a keybinding row to see the position preview; click its shortcut to record a replacement. **Save changes** validates and applies your settings. Closing the window leaves sTile and its shortcuts running.

The settings screen displays the resolved configuration path:

| Platform | Default file |
| --- | --- |
| Windows | `%APPDATA%\stile\config\config.toml` |
| macOS | `~/Library/Application Support/stile/config.toml` |
| Linux | `${XDG_CONFIG_HOME:-~/.config}/stile/config.toml` |

Use `--config PATH` to select a different file. Missing files use defaults; invalid existing files return an error instead of being overwritten. Configuration-file watching is not implemented.

## TOML configuration

This is a **partial example**, not the complete default binding list. Omitted actions appear disabled in the UI (missing focus bindings have migration behavior described in [keybinds](/sTile/keybinds/)). An empty shortcut explicitly disables an action.

```toml
version = 1

[growth]
percent = 15
steps = 2

[width_resize]
percent = 25

[focus]
include_obscured = false

[[bindings]]
action = "left"
shortcut = "Ctrl+Alt+1"

[[bindings]]
action = "right"
shortcut = "Ctrl+Alt+2"
```

## Command line

Use the installed executable’s full path if it is not on PATH. On Windows, the installer does not add it automatically.

```sh
stile --help
stile settings
stile apply left
stile apply center --json
stile apply shrink-width
stile apply expand-width
stile apply fill-gap --json
stile apply focus-right
stile apply resize-up
```

Focus and paired resize commands require Windows. On Windows, `--window HWND` can target an explicit window. `--json` provides structured output for scripting. Use `stile apply --help` for the supported actions and arguments.

## Shared positioning state

Tray shortcuts, CLI actions, Herdr actions, and exported Wayland shortcuts share growth state for a configuration file. It is stored in `<config-path>.cycle.json`; a separate `.cycle.lock` serializes concurrent invocations. Deleting the state file resets the cycle. Failed native requests do not advance it.

Windows and macOS store observed bounds. X11 and Wayland store requested bounds because placement is asynchronous; compositor-imposed bounds can restart a cycle.
