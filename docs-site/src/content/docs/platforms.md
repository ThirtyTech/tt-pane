---
title: Platforms & troubleshooting
description: Platform support, permissions, Wayland integration, and common shortcut issues.
---

| Platform | Window positioning | Shortcuts |
| --- | --- | --- |
| Windows | Native Win32, monitor work area and visible-frame correction | Native global shortcuts |
| macOS | Accessibility API and visible screen frame | Native Control + Option defaults |
| Linux X11 | EWMH and RandR monitors | X11 key grabs |
| GNOME (Wayland) | Bundled GNOME Shell extension | GNOME custom shortcuts, registered by tt-pane |
| Other Wayland compositors (KDE, Sway, Hyprland, …) | Not supported; use an X11 session | Not supported |

Shared-boundary resizing is **Windows-only**. Centered window groups work on Windows and GNOME on Wayland. Directional focus works on Windows, macOS and GNOME on Wayland. Platform backends compile, but macOS and Linux runtime behavior still needs physical-desktop validation; compilation is not a substitute for that testing.

## Windows

Run normally. Elevated applications can reject positioning from an unelevated process. GUI launches do not allocate a console; terminal launches attach to the terminal, where Ctrl+C terminates tt-pane.

## macOS

Grant the executable access under **System Settings → Privacy & Security → Accessibility**. Moving or rebuilding the unsigned executable can require granting access again. Exit fullscreen before positioning. tt-pane uses a menu-bar icon without a Dock icon.

Building requires Xcode and command-line tools, including Metal tooling.

## Linux

GPUI needs a compatible GPU driver and native font, X11/XCB, XKB, and Wayland libraries. See the repository’s [build prerequisite list](https://github.com/thirtytech/tt-pane#build-prerequisites).

Enable a StatusNotifierItem tray host: KDE, Waybar’s tray module, or GNOME’s AppIndicator extension. X11 requires EWMH move/resize and RandR 1.5 monitor enumeration. Panels with only legacy struts are not accounted for.

## GNOME on Wayland

Wayland has no universal API for repositioning other applications, and native Wayland windows cannot be managed through the X11 backend. On GNOME, tt-pane installs a small Shell extension that moves and focuses windows on request, and registers your bindings as GNOME custom shortcuts. Tiling compositors such as Sway and Hyprland manage placement and focus themselves and aren't supported.

A release build sets this up at startup. You can also click **Set up GNOME** in Settings, or run:

```sh
tt-pane gnome setup
```

Log out and back in once after the first setup, and after an update changes the extension: GNOME only loads extension code at login. Saving Settings re-registers the shortcuts. Holding a focus key does nothing extra on GNOME, because GNOME shortcuts only report presses.

## A shortcut does nothing

- Check that tt-pane is running and the intended window is active.
- Open Settings and check registration errors or disabled actions.
- Use number-row digits; numpad digits are distinct bindings.
- Change combinations reserved by your OS, compositor, or application.
- On macOS, check Accessibility access. On GNOME Wayland, run **Set up GNOME** and log out and back in once.
- If the position is constrained, check application minimum sizes or fullscreen state.

For focus actions, there may be no eligible window in that direction. For gap filling, first shrink a positioned window and then select a different window.
