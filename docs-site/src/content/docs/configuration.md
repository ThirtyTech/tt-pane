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
launch_at_login = true
center_layout = "offset" # or "stack"
tray_icon = "monochrome" # macOS default; Linux defaults to "color"

[gaps]
inner = 5
outer = 20

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

Tap **Ctrl+Alt+H/L** (release before **400 ms**) for regular navigation. Hold for **400 ms** from a background edge-snapped window to enter middle windows, including covered ones. From a centered member or floating middle window, hold to exit to background edge-snapped windows on the same monitor. Holds fire once; release before holding again. If no candidate qualifies, focus stays put. Remapped horizontal focus shortcuts work the same way. Holds require the running tray app.

Saved growth settings are preserved on upgrade. New configurations and files without growth settings default to 15% and three steps; reset defaults and save to adopt these values in an existing configuration. Width and growth controls are under **Advanced**, which starts collapsed.

Old `include_obscured = false` settings migrate to 50%, and `true` or an omitted value migrates to 10%. An explicit `minimum_visible_percent` wins if both keys exist. Saving writes only the new percentage key.

## Tiling (experimental, Windows)

On the `experimental/hypr-tiling` branch, **Settings → Experimental → Automatic tiling** enables dwindle tiling. It defaults to **Off**, including existing configurations. **Save changes** activates the setting and tiles already-open eligible windows on the current virtual desktop. Turn it off and save to leave windows where they are. Keep the tray app running for automatic updates (polled every 100 ms); standalone CLI actions reconcile once.

```toml
[tiling]
enabled = false
layout = "dwindle"
default_split_ratio = 0.5
split_width_multiplier = 1.0
preserve_split = true
float_apps = ["example.exe"]
```

`default_split_ratio` is the left/top child's fraction, from 0.1 to 0.9. A node splits side-by-side when its width exceeds its height times `split_width_multiplier` (0.1–10); otherwise it splits top/bottom. `preserve_split = true` retains split directions; false recomputes directions as shapes change and disables the toggle-split action. `float_apps` matches exact executable basenames without case sensitivity; paths and wildcards are not supported. Edit these advanced values in the config and restart the app; configuration-file watching is not implemented.

Each monitor/work-area and virtual desktop has its own tree. New windows split the focused tile (or the last focused tile when the newly opened window has focus). Closed or minimized windows leave the tree and remaining tiles reflow; restored windows can rejoin. Existing inner and outer gap settings apply. Manually moving or resizing a tile, including moving it to another monitor, makes it floating. Snap, center, and fill-gap actions also float a tiled window. Centered groups and configured floating apps stay outside the tree; `toggle-float` on a centered member leaves its group and retiles it. A resize that squeezes a neighbor below its minimum size relocates that neighbor into the largest other tile, or is undone when no tile fits. Disabling and re-enabling tiling resets manual floating choices and split changes.

Only restored, resizable application windows are enrolled. Dialogs, owned windows, tool windows, hidden/cloaked windows, minimized/maximized/fullscreen windows, and tt-pane itself are excluded. Windows checks the current virtual desktop again before each move. Applications that reject tile sizes or moves become floating rather than being repeatedly resized; a size-constrained move attempts to restore the prior bounds. A monitor/work-area change currently floats affected windows.

While a tiled window has focus, H/J/K/L navigates tiles on its monitor, including deep nested tiles; past the last tile in that direction it falls back to regular focus navigation, which reaches other monitors. Width shortcuts (`-`/`=`) shrink/grow the immediate split by five percentage points instead of changing only the width. Up/Down moves the nearest horizontal split boundary by five percentage points. New [tiling actions](/tt-pane/keybinds/#experimental-tiling-actions-windows) default to Win+Shift shortcuts (split grow/shrink start unbound) and can be reassigned in Settings or invoked through the CLI/Herdr.

`tt-pane apply toggle-tiling` switches the shared runtime state and reconciles immediately when enabling. This override is stored in `<config>.tiling.json` alongside the tree, floating choices, and observed bounds. It persists across CLI/tray processes and restarts until Settings is saved or the `[tiling]` configuration changes. The Settings toggle shows the saved configuration; saving it overrides the runtime toggle. State updates use the same cycle lock as snapping and centered groups. Missing or corrupt sidecar data resets the layout.

## Center window layout

**Settings → Center windows → Layout** switches between **Offset** (the default, including existing configurations) and **Stack**. In Stack mode, all group members stay centered and H/L cycles through the same order, bringing the selected window to the front even when it was fully covered. Set `center_layout = "stack"` at the top level of the TOML file to enable it from the CLI.

Sizing is unchanged: the selected window restores its saved size, background members use the base center size, and growth and width adjustments continue to work as before. Save to activate; existing groups adopt the layout when a member is focused or the next center/group action runs.

## Snapped window gaps

**Settings → Snapped window gaps** controls spacing for half, quarter, and fill-work-area placements. Defaults match [Hyprland](https://wiki.hypr.land/configuring/core/config-options/): `inner = 5` per window edge (10 total between adjacent tiles), and `outer = 20` at desktop work-area edges. Values use native desktop coordinates (pixels on Windows, points on macOS), range from 0–100, and zero disables the corresponding spacing. Existing configurations inherit these defaults.

Changes apply on the next placement; existing windows are not rearranged when saving unless experimental automatic tiling is active. Repeated growth retains margins, width expansion stops at the inset work-area edge, and filling a saved strip beside a snapped window leaves inner spacing. Center placement and floating center groups retain their existing geometry. Background focus navigation recognizes both inset tiles and native edge snaps.

## Launch at login

**Settings → General → Launch at login** starts tt-pane in the tray when you sign in. It is on by default, including for existing configurations; turn it off and save to opt out (`launch_at_login = false`). Each tray launch refreshes the entry so it follows the app if you move it:

| Platform | Sign-in entry |
| --- | --- |
| Windows | `tt-pane` value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` |
| macOS | `~/Library/LaunchAgents/net.thirtytech.tt-pane.plist` |
| Linux | `${XDG_CONFIG_HOME:-~/.config}/autostart/tt-pane.desktop` |

The entry is written the first time the tray starts, so install and open tt-pane once. Debug builds (`cargo run` without `--release`) do not add the entry on launch. Uninstalling removes it.

## Tray icon — macOS and Linux

**Settings → General → Tray icon** (**Menu bar icon** on macOS) switches between the full-color app icon and a monochrome silhouette (`tray_icon = "color"` or `"monochrome"`). The change applies when you save.

| Platform | Default | Monochrome |
| --- | --- | --- |
| macOS | `monochrome` | A template image the menu bar tints for light and dark appearance |
| Linux | `color` | A white silhouette, for dark panels such as GNOME's top bar |

Windows always uses the color icon and ignores the setting.

## Focus glow — Windows

On Windows, **Settings → Window focus → Focus glow** enables a soft, click-through gradient cue after moves/resizes and Vim-style focus navigation. It fades in and out over 480 ms. Click **Color** to open the color picker, then save. The default is enabled with neutral off-white (`#F2F0E8`); `glow_color` accepts `#RRGGBB`. Existing configurations inherit these defaults. CLI actions use the same cue and wait for it before exiting. Other platforms do not render the glow.

## Command line

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

Focus commands work on Windows, macOS, and GNOME Wayland. Paired resize commands and hold gestures require Windows. On Windows, `--window HWND` can target an explicit window. `--json` provides structured output for scripting. Use `tt-pane apply --help` for the supported actions and arguments.

## Shared positioning state

Tray shortcuts, CLI actions, Herdr actions, and GNOME shortcuts share growth state for a configuration file. It is stored in `<config-path>.cycle.json`; a separate `.cycle.lock` serializes concurrent invocations. Deleting the state file resets the cycle. Failed native requests do not advance it.

Windows and macOS store observed bounds. X11 and Wayland store requested bounds because placement is asynchronous; compositor-imposed bounds can restart a cycle.
