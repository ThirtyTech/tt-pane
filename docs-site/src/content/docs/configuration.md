---
title: Configuration & CLI
description: Customize shortcuts, growth, width changes, and scripted window positioning.
---

## Settings

Open **Settings…** from the tray. Settings has six tabs: **Shortcuts** (keybindings and the position preview), **Snapping** (center layout, gaps, width and growth steps, snap animation), **Tiling**, **Focus** (focus glow and H/J/K/L navigation), **Dock** (the experimental dock panel) and **General** (launch at login, tray icon, updates, GNOME setup). A dot on a tab marks unsaved changes there. Hover a keybinding row to see the position preview; click its shortcut to record a replacement. **Save changes** validates and applies the settings on every tab, and **Reset defaults** resets all of them. Closing the window leaves tt-pane and its shortcuts running.

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

[animation]
enabled = false
duration_ms = 160

[dock]
enabled = false
edge = "bottom" # or "top", "left", "right"
fill = "start" # left or top end; "end" for right or bottom
size_percent = 10

[[bindings]]
action = "left"
shortcut = "Ctrl+Alt+1"

[[bindings]]
action = "right"
shortcut = "Ctrl+Alt+2"
```

**Include middle windows in H/J/K/L navigation** in Settings defaults to **Off**, including for existing configurations. Outside a centered group, regular H/J/K/L and CLI focus navigate background edge-snapped windows. Turn it On to include floating and centered windows. Inside a centered group, H/L always rotates through its members.

When middle-window navigation is enabled, **Settings → Focus → Filter middle windows by visibility** optionally requires more than the **Minimum visible** percentage of a middle window's frame to be on-screen and uncovered. The filter defaults to Off and the percentage to 10% (range 0–99%). This applies to spatial navigation, not H/L rotation inside a group or hold-to-enter. Edge snaps remain reachable when covered. Minimized, hidden, cloaked, and utility windows stay excluded.

Tap **Ctrl+Alt+H/L** (release before **400 ms**) for regular navigation. Hold for **400 ms** from a background edge-snapped window to enter middle windows, including covered ones; when a centered group lies in that direction, the hold goes to its active member even if another floating window is nearer. From a floating window outside a group, a hold reaches background edge-snapped windows or a centered group. From a centered member, hold to exit to background edge-snapped windows on the same monitor. Holds fire once; release before holding again. If no candidate qualifies, focus stays put. Remapped horizontal focus shortcuts work the same way. Holds require the running tray app.

Saved growth settings are preserved on upgrade. New configurations and files without growth settings default to 15% and three steps; reset defaults and save to adopt these values in an existing configuration. Width and growth controls are on the **Snapping** tab.

Old `include_obscured = false` settings migrate to 50%, and `true` or an omitted value migrates to 10%. An explicit `minimum_visible_percent` wins if both keys exist. Saving writes only the new percentage key.

## Tiling (experimental)

**Settings → Tiling → Tile windows automatically** enables dwindle tiling on Windows, macOS, GNOME on Wayland and X11 (see [platform notes](/tt-pane/platforms/)). It defaults to **Off**, including existing configurations. **Save changes** activates the setting and tiles already-open eligible windows on the current virtual desktop, Space or workspace. Turn it off and save to leave windows where they are. Keep the tray app running for automatic updates (polled every 100 ms); standalone CLI actions reconcile once.

```toml
[tiling]
enabled = false
layout = "dwindle"
default_split_ratio = 0.5
split_width_multiplier = 1.0
preserve_split = true
float_apps = ["example.exe"]
```

`default_split_ratio` is the left/top child's fraction, from 0.1 to 0.9. A node splits side-by-side when its width exceeds its height times `split_width_multiplier` (0.1–10); otherwise it splits top/bottom. `preserve_split = true` retains split directions; false recomputes directions as shapes change and disables the toggle-split action. `float_apps` matches without case sensitivity: exact executable basenames on Windows (`example.exe`), application names on macOS (`Calculator`), and WM classes on GNOME and X11 (`org.gnome.Calculator`, `XTerm`); paths and wildcards are not supported. Edit these advanced values in the config and restart the app; configuration-file watching is not implemented.

Each monitor/work-area and virtual desktop (Space on macOS, workspace on GNOME) has its own tree. New windows split the focused tile (or the last focused tile when the newly opened window has focus). Closed or minimized windows leave the tree and remaining tiles reflow; restored windows can rejoin. Existing inner and outer gap settings apply. Manually moving or resizing a tile, including moving it to another monitor, makes it floating. Snap, center, and fill-gap actions also float a tiled window, and a snapped window keeps its space: a window you snap (a tt-pane half or quarter snap, or moving it into a corner yourself, such as a system snap) is left out of the layout while it touches two adjacent work-area edges, even after you resize it. A tile that floats without being snapped, for example because it refused its tile size, takes no space, and the tiles fill the largest rectangle of the monitor that snapped windows don't cover. With the left half and the bottom-right quarter snapped, the tiles share the top-right quarter; re-snap the left window to the bottom left and they take the whole top half. Resizing a snapped window gives the space it frees to the tiles. When snapped windows leave less than a fifth of the monitor across or down, tiles use the whole monitor again. Tiles that were in the space a window snaps into move to the space that is left. A new window opened while a snapped window has focus joins the tiles in the free space; it never splits the snapped window. To put a snapped window back in the layout, use `toggle-float`. Earlier versions had a **Snaps rearrange tiles** setting that docked snaps into the layout; it is gone, and `snap_rearranges` in an older configuration is ignored. Centered groups and configured floating apps stay outside the tree; `toggle-float` on a centered member leaves its group and retiles it. A resize that squeezes a neighbor below its minimum size relocates that neighbor into the largest other tile, or is undone when no tile fits. Disabling and re-enabling tiling resets manual floating choices and split changes.

Only restored, resizable application windows are enrolled. Dialogs, owned windows, tool windows, hidden/cloaked windows, minimized/maximized/fullscreen windows, and tt-pane itself are excluded. Every move first rechecks that the window is still where tt-pane last saw it on the current desktop, so a window you just moved is never pulled back. Applications that reject tile sizes or moves become floating rather than being repeatedly resized; a size-constrained move attempts to restore the prior bounds. A monitor/work-area change currently floats affected windows.

While a tiled window has focus, H/J/K/L navigates tiles on its monitor, including deep nested tiles; past the last tile in that direction it falls back to regular focus navigation, which reaches other monitors. Width shortcuts (`-`/`=`) shrink/grow the immediate split by five percentage points instead of changing only the width. Up/Down moves the nearest horizontal split boundary by five percentage points. Resize mode (Win+Shift+E) moves dividers with H/J/K/L until Esc or Enter. New [tiling actions](/tt-pane/keybinds/#experimental-tiling-actions) default to Win+Shift shortcuts on Windows (split grow/shrink start unbound) and are unbound on macOS and Linux, where Super+Shift belongs to the shell or to apps; assign them in Settings or invoke them through the CLI/Herdr.

`tt-pane apply toggle-tiling` switches the shared runtime state and reconciles immediately when enabling. This override is stored in `<config>.tiling.json` alongside the tree, floating choices, and observed bounds. It persists across CLI/tray processes and restarts until Settings is saved or the `[tiling]` configuration changes. The Settings toggle shows the saved configuration; saving it overrides the runtime toggle. State updates use the same cycle lock as snapping and centered groups. Missing or corrupt sidecar data resets the layout.

## Center window layout

**Settings → Snapping → Center windows → Layout** switches between **Offset** (the default, including existing configurations) and **Stack**. In Stack mode, all group members stay centered and H/L cycles through the same order, bringing the selected window to the front even when it was fully covered. Set `center_layout = "stack"` at the top level of the TOML file to enable it from the CLI.

Sizing is unchanged: the selected window restores its saved size, background members use the base center size, and growth and width adjustments continue to work as before. Save to activate; existing groups adopt the layout when a member is focused or the next center/group action runs.

## Snapped window gaps

**Settings → Snapping → Snapped window gaps** (**Inner gap per window** and **Desktop edge gap**) controls spacing for half, quarter, and fill-work-area placements. Defaults match [Hyprland](https://wiki.hypr.land/configuring/core/config-options/): `inner = 5` per window edge (10 total between adjacent tiles), and `outer = 20` at desktop work-area edges. Values use native desktop coordinates (pixels on Windows, points on macOS), range from 0–100, and zero disables the corresponding spacing. Existing configurations inherit these defaults.

Changes apply on the next placement; existing windows are not rearranged when saving unless experimental automatic tiling is active. Repeated growth retains margins, width expansion stops at the inset work-area edge, and filling a saved strip beside a snapped window leaves inner spacing. Center placement and floating center groups retain their existing geometry. Background focus navigation recognizes both inset tiles and native edge snaps.

## Animated snapping (experimental)

**Settings → Snapping → Animation → Animate snapping** makes a snap glide the window to its new position instead of jumping. It works from any starting point: another snap, a window you placed yourself, or a maximized window. It applies to half, quarter, center, fill-work-area, width and growth actions on Windows, macOS, GNOME on Wayland and X11. It defaults to **Off**, including existing configurations.

The window moves on a fast-slow-fast curve: it leaves and arrives quickly and slows through the middle. A snap takes `duration_ms` (160 ms by default, 50–1000) in total; there is no Settings control for the duration yet. When the window is wider or taller than its target, it reaches the target size in the first 40% of the time, so the application lays itself out fewer times, and the edge that moves least stays put (from full width to the right half, the right edge doesn't move). Growing follows the normal curve. An application that is slow to redraw skips frames rather than slowing the snap down, and the window always ends exactly where an instant snap would put it.

In a centered group (Windows and GNOME on Wayland), focusing an offset member glides it into the center, whether you use H/L, hold into the group, or click or Alt+Tab to it. The member it replaces steps aside instantly first, so only one window animates at a time.

Tiling, swaps and shared-boundary resizes still move windows instantly.

## Dock (experimental) — Windows

**Settings → Dock → Show the dock** adds a panel along one edge of the primary monitor. Windows you dock appear in it as live previews; click one to bring that window into the middle of the screen. It goes back to the dock as soon as you focus another window. The dock is off by default, including for existing configurations.

- **Edge** puts the panel on the top, bottom, left or right of the screen (`edge`, default `"bottom"`).
- **Fill from** chooses the end of the edge the panel grows from: **Left** or **Right** on the top and bottom, **Top** or **Bottom** on the sides (`fill`, `"start"` for left or top, `"end"` for right or bottom; default `"start"`).
- **Size** is the panel's thickness: its share of the screen height for the top and bottom, or of its width for the sides (`size_percent`, 5–40, default 10).

The panel is only as long as its docked windows need. With the top edge filling from the left, the first window docks in the top-left corner and each new one adds a slot to its right. Once the edge is full, the slots shrink so every window fits. With nothing docked the panel disappears and gives its space back. While you drag a window, an empty slot appears at the end of the panel (at the fill corner when nothing is docked) for you to drop it on.

While something is docked, the panel reserves space the way the taskbar does, and it sits beside the taskbar when both are on the same edge. Maximized windows, snaps, Center, centered groups and automatic tiling all use the space that's left, the *arena*. Windows reserves the panel's thickness along the whole edge, even where the panel is shorter, so the arena loses that full strip.

**Docking a window.** Press **Ctrl + Alt + D** (**Dock / undock window**) to dock the focused window, or drag a window by its title bar and drop it on the panel. The same shortcut on a docked window takes it out of the dock and puts it back where it was before you docked it. A window dragged onto the panel goes back to where the drag started. From the command line, `tt-pane apply dock` (or `--window HWND`) does the same. tt-pane must be running in the tray, since that process shows the panel.

**Previews.** A docked window is resized to 1920×1080 and moved almost entirely off the screen, past the panel's fill corner. A small corner of it stays on screen, hidden under the panel or the taskbar, because applications such as Explorer stop drawing a window that is on no screen at all. If another monitor sits beside that corner, the window is parked past every monitor instead. Its preview is a live thumbnail of that window, scaled to fit its slot. Slots are 16:9 and fill the panel's thickness. An application that still stops drawing when it's covered shows the last thing it drew.

**The arena.** Clicking a preview moves its window to the Center position in the arena (half the arena's width and height, centered) and focuses it; its slot is outlined while it's out. With [animated snapping](#animated-snapping-experimental) on, the window glides out of its preview. Focusing any other window, by clicking it, with Alt + Tab or with the taskbar, sends it back to the dock. Choosing a docked window with Alt + Tab or the taskbar brings it into the arena too. Only one docked window is out at a time.

**Leaving the dock.** Closing a docked window removes it. Positioning a docked window with any other tt-pane action, for example snapping the one in the arena to the left half, takes it out of the dock where it is. Docked windows are left out of automatic tiling, centered groups and directional focus.

Turning the dock off and saving, or quitting tt-pane, puts every docked window back where it was before it was docked. Membership is saved in `<config-path>.dock.json`, so if tt-pane stops without quitting, the next start shows the same docked windows again, or puts them back if the dock has been turned off.

The dock is Windows only for now, because it relies on Windows' live window thumbnails.

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

On Windows, **Settings → Focus → Highlight the focused window** enables a soft, click-through gradient cue after moves/resizes and Vim-style focus navigation. When focus navigation has nowhere to go, the already-selected window still glows. It fades in and out over 480 ms. Click **Glow color** to open the color picker, then save. The default is enabled with neutral off-white (`#F2F0E8`); `glow_color` accepts `#RRGGBB`. Existing configurations inherit these defaults. CLI actions use the same cue and wait for it before exiting. Other platforms do not render the glow.

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
tt-pane apply dock
```

Focus commands work on Windows, macOS, and GNOME Wayland. Paired resize commands and hold gestures require Windows. On Windows, `--window HWND` can target an explicit window. `--json` provides structured output for scripting. Use `tt-pane apply --help` for the supported actions and arguments.

## Shared positioning state

Tray shortcuts, CLI actions, Herdr actions, and GNOME shortcuts share growth state for a configuration file. It is stored in `<config-path>.cycle.json`; a separate `.cycle.lock` serializes concurrent invocations, and also guards centered groups (`.centered.json`) and docked windows (`.dock.json`). Deleting the state file resets the cycle. Failed native requests do not advance it.

Windows and macOS store observed bounds. X11 and Wayland store requested bounds because placement is asynchronous; compositor-imposed bounds can restart a cycle.
