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

Experimental automatic tiling:

| Platform | Automatic tiling |
| --- | --- |
| Windows | Dwindle, opt-in; current virtual desktop, per monitor |
| macOS | Dwindle, opt-in; per Space and monitor |
| GNOME Wayland | Dwindle, opt-in; per workspace and monitor (extension version 4) |
| Linux X11 | Dwindle, opt-in; per desktop and monitor (EWMH window manager) |

Windows tiling polls every 100 ms while the tray runs. It skips dialogs and minimized, maximized, fullscreen, hidden, utility, and non-resizable windows. Windows that reject moves become floating; elevated apps may reject control from an unelevated process. See [experimental tiling configuration](/tt-pane/configuration/#tiling-experimental).

Run normally. Elevated applications can reject positioning from an unelevated process. GUI launches do not allocate a console; terminal launches attach to the terminal, where Ctrl+C terminates tt-pane.

## macOS

Grant the executable access under **System Settings → Privacy & Security → Accessibility**. Moving or rebuilding the unsigned executable can require granting access again. Exit fullscreen before positioning. tt-pane uses a menu-bar icon without a Dock icon.

Building requires Xcode and command-line tools, including Metal tooling.

Automatic tiling on macOS arranges standard, resizable windows on the visible Spaces through Accessibility, so it needs the same permission as positioning. Each Space and monitor has its own layout; windows in native fullscreen, minimized windows and tt-pane itself stay out. `float_apps` matches the application's name as shown in the menu bar, such as `Calculator`. Holding the mouse button pauses tiling, so dragging or resizing a window is never fought. Space detection uses a private macOS interface; if a future macOS removes it, all Spaces share one layout per monitor.

## Linux

Automatic tiling on X11 works with stacking window managers that implement EWMH, such as Openbox, Xfwm or KWin (tested with Openbox); tiling window managers such as i3 arrange windows themselves. Each desktop and monitor has its own layout; windows on all desktops share one per monitor. Dialogs, transient and fixed-size windows, minimized, maximized and fullscreen windows, and tt-pane itself stay out. `float_apps` matches the WM_CLASS class, such as `XTerm` or `firefox`. Terminals that resize in whole character cells can leave a gap of a few pixels in their tile. Holding the mouse button pauses tiling. Other Wayland compositors keep ordinary snapping.

GPUI needs a compatible GPU driver and native font, X11/XCB, XKB, and Wayland libraries. See the repository’s [build prerequisite list](https://github.com/thirtytech/tt-pane#build-prerequisites).

Enable a StatusNotifierItem tray host: KDE, Waybar’s tray module, or GNOME’s AppIndicator extension. X11 requires EWMH move/resize and RandR 1.5 monitor enumeration. Panels with only legacy struts are not accounted for.

## GNOME on Wayland

Wayland has no universal API for repositioning other applications, and native Wayland windows cannot be managed through the X11 backend. On GNOME, tt-pane installs a small Shell extension that moves and focuses windows on request, and registers your bindings as GNOME custom shortcuts. Tiling compositors such as Sway and Hyprland manage placement and focus themselves and aren't supported.

A release build sets this up at startup. You can also click **Set up GNOME** in Settings, or run:

```sh
tt-pane gnome setup
```

Log out and back in once after the first setup, and after an update changes the extension: GNOME only loads extension code at login. Automatic tiling needs extension version 4; with an older extension running, tiling reports that the extension is out of date until you log out and back in.

Tiling on GNOME follows the active workspace. Windows shown on every workspace (including other monitors when workspaces span only the primary display) share one layout per monitor. `float_apps` matches the window's WM class, such as `org.gnome.Nautilus` or `firefox`. Dragging or resizing a window pauses tiling until you let go. A window that opens maximized still tiles: GNOME maximizes new windows that open at nearly the size of the screen, which apps often do after they were tiled large, and apps can restore a maximized state. Maximizing a tiled window yourself takes it out of the layout until you restore it. Saving Settings re-registers the shortcuts. Holding a focus key does nothing extra on GNOME, because GNOME shortcuts only report presses.

## A shortcut does nothing

- Check that tt-pane is running and the intended window is active.
- Open Settings and check registration errors or disabled actions.
- Use number-row digits; numpad digits are distinct bindings.
- Change combinations reserved by your OS, compositor, or application.
- On macOS, check Accessibility access. On GNOME Wayland, run **Set up GNOME** and log out and back in once.
- If the position is constrained, check application minimum sizes or fullscreen state.

For focus actions, there may be no eligible window in that direction. For gap filling, first shrink a positioned window and then select a different window.
