---
title: Platforms & troubleshooting
description: Platform support, permissions, Wayland integration, and common shortcut issues.
---

| Platform | Window positioning | Shortcuts |
| --- | --- | --- |
| Windows | Native Win32, monitor work area and visible-frame correction | Native global shortcuts |
| macOS | Accessibility API and visible screen frame | Native Control + Option defaults |
| Linux X11 | EWMH and RandR monitors | X11 key grabs |
| Sway | Native IPC; selected window becomes floating | Exported compositor bindings |
| Hyprland | Native IPC; selected window becomes floating | Exported compositor bindings |
| Other Wayland compositors | Not implemented | Not implemented |

Directional focus and shared-boundary resizing are **Windows-only**. Platform backends compile, but macOS and Linux runtime behavior still needs physical-desktop validation; compilation is not a substitute for that testing.

## Windows

Run normally. Elevated applications can reject positioning from an unelevated process. GUI launches do not allocate a console; terminal launches attach to the terminal, where Ctrl+C terminates tt-pane.

## macOS

Grant the executable access under **System Settings → Privacy & Security → Accessibility**. Moving or rebuilding the unsigned executable can require granting access again. Exit fullscreen before positioning. tt-pane uses a menu-bar icon without a Dock icon.

Building requires Xcode and command-line tools, including Metal tooling.

## Linux

GPUI needs a compatible GPU driver and native font, X11/XCB, XKB, and Wayland libraries. See the repository’s [build prerequisite list](https://github.com/thirtytech/tt-pane#build-prerequisites).

Enable a StatusNotifierItem tray host: KDE, Waybar’s tray module, or GNOME’s AppIndicator extension. X11 requires EWMH move/resize and RandR 1.5 monitor enumeration. Panels with only legacy struts are not accounted for.

## Wayland shortcuts

Wayland has no universal API for repositioning other applications. tt-pane explicitly supports Sway and Hyprland. Native Wayland windows cannot be managed through the X11 backend. GNOME Shell and KWin integrations are not implemented.

Save Settings, click **Export shortcuts**, and include the generated file in your compositor config. Or use the installed executable:

```sh
tt-pane export sway > ~/.config/sway/tt-pane.conf
# Add to ~/.config/sway/config:
# include ~/.config/sway/tt-pane.conf

tt-pane export hyprland > ~/.config/hypr/tt-pane.conf
# Add to ~/.config/hypr/hyprland.conf:
# source = ~/.config/hypr/tt-pane.conf
```

Reload the compositor. Repeat export and reload after every binding change. Export embeds the executable’s absolute path, so generate bindings after moving it to its final location. The settings window does not need to remain open. Exit fullscreen on Hyprland before positioning.

## A shortcut does nothing

- Check that tt-pane is running and the intended window is active.
- Open Settings and check registration errors or disabled actions.
- Use number-row digits; numpad digits are distinct bindings.
- Change combinations reserved by your OS, compositor, or application.
- On macOS, check Accessibility access. On Wayland, export and reload.
- If the position is constrained, check application minimum sizes or fullscreen state.

For focus actions, there may be no eligible window in that direction. For gap filling, first shrink a positioned window and then select a different window.
