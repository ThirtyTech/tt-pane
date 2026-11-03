---
title: Configuration & CLI
description: Customize shortcuts, growth, width changes, and scripted window positioning.
---

## Settings

Open **Settings…** from the tray. Hover a keybinding row to see the position preview; click its shortcut to record a replacement. **Save changes** validates and applies your settings. Closing the window leaves tt-pane and its shortcuts running.

The settings screen displays the resolved configuration path:

| Platform | Default file |
| --- | --- |
| Windows | `%APPDATA%\tt-pane\config\config.toml` |
| macOS | `~/Library/Application Support/tt-pane/config.toml` |
| Linux | `${XDG_CONFIG_HOME:-~/.config}/tt-pane/config.toml` |

Existing installations continue using their previous `stile` configuration directory when it contains `config.toml` and the new `tt-pane` configuration does not exist. This preserves custom shortcuts and adjacent saved placement state. Explicit `--config` paths are unchanged.

Use `--config PATH` to select a different file. Missing files use defaults; invalid existing files return an error instead of being overwritten. Configuration-file watching is not implemented.

## TOML configuration

This is a **partial example**, not the complete default binding list. Omitted actions appear disabled in the UI (missing focus bindings have migration behavior described in [keybinds](/tt-pane/keybinds/)). An empty shortcut explicitly disables an action.

```toml
version = 1

[growth]
percent = 15
steps = 2

[width_resize]
percent = 25

[focus]
minimum_visible_percent = 10
glow_enabled = true
glow_color = "#F2F0E8"

[[bindings]]
action = "left"
shortcut = "Ctrl+Alt+1"

[[bindings]]
action = "right"
shortcut = "Ctrl+Alt+2"
```

**Settings → Advanced** starts collapsed and contains the visibility percentage, width adjustment, and repeat-growth controls. **Minimum middle-window visibility** defaults to 10% and accepts 0–99%. Regular focus presses and CLI focus require strictly more than this percentage of a middle window's full frame to be on-screen and uncovered. Centered-group traversal uses the same threshold. Long H/L presses from edge-snapped windows bypass it to reach covered middle windows. From a centered-group member or floating middle window, they jump out to a background edge-snapped window on the same monitor, skipping other middle windows. If no candidate exists, focus stays put. Edge-snapped windows remain reachable regardless of coverage.

Old `include_obscured = false` settings migrate to 50%, and `true` or an omitted value migrates to 10%. An explicit `minimum_visible_percent` wins if both keys exist. Saving writes only the new percentage key.

## Command line

On Windows, **Settings → Window focus → Focus glow** enables a soft, click-through gradient cue after moves/resizes and Vim-style focus navigation. It fades in and out over 480 ms. Click **Color** to open the color picker, then save. The default is enabled with neutral off-white (`#F2F0E8`); `glow_color` accepts `#RRGGBB`. Existing configurations inherit these defaults. CLI actions use the same cue and wait for it before exiting. Other platforms do not render the glow.

Use the installed executable’s full path if it is not on PATH. On Windows, the installer does not add it automatically.

```sh
tt-pane --help
tt-pane settings
tt-pane apply left
tt-pane apply center --json
tt-pane apply shrink-width
tt-pane apply expand-width
tt-pane apply fill-gap --json
tt-pane apply focus-right
tt-pane apply resize-up
```

Focus and paired resize commands require Windows. On Windows, `--window HWND` can target an explicit window. `--json` provides structured output for scripting. Use `tt-pane apply --help` for the supported actions and arguments.

## Shared positioning state

Tray shortcuts, CLI actions, Herdr actions, and exported Wayland shortcuts share growth state for a configuration file. It is stored in `<config-path>.cycle.json`; a separate `.cycle.lock` serializes concurrent invocations. Deleting the state file resets the cycle. Failed native requests do not advance it.

Windows and macOS store observed bounds. X11 and Wayland store requested bounds because placement is asynchronous; compositor-imposed bounds can restart a cycle.
