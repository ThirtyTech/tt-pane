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
steps = 3

[width_resize]
percent = 25

[focus]
include_middle_windows = false
visibility_filter_enabled = false
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

**Include middle windows in H/J/K/L navigation** in Settings defaults to **Off**, including for existing configurations. Outside a centered group, regular H/J/K/L and CLI focus navigate background edge-snapped windows. Turn it On to include floating and centered windows. Inside a centered group, H/L always rotates through its members.

When middle-window navigation is enabled, **Settings → Advanced → Filter middle windows by visibility** optionally requires more than the **Minimum middle-window visibility** percentage of a middle window's frame to be on-screen and uncovered. The filter defaults to Off and the percentage to 10% (range 0–99%). This applies to spatial navigation, not H/L rotation inside a group or hold-to-enter. Edge snaps remain reachable when covered. Minimized, hidden, cloaked, and utility windows stay excluded. Advanced starts collapsed and also contains width and growth controls.

Tap **Ctrl+Alt+H/L** (release before **400 ms**) for regular navigation. Hold for **400 ms** from a background edge-snapped window to enter middle windows, including covered ones. From a centered member or floating middle window, hold to exit to background edge-snapped windows on the same monitor. Holds fire once; release before holding again. If no candidate qualifies, focus stays put. Remapped horizontal focus shortcuts work the same way. Holds require the running Windows tray app.

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
