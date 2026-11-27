# Changelog

## Unreleased

- Experimental, Windows: a dock for windows you want close at hand. Turn on **Settings → Dock → Show the dock** to add a panel along the bottom, top, left or right edge of the primary monitor, 15% of the screen by default (5–40%, `[dock] size_percent`). Dock the focused window with **Ctrl+Alt+D** (**Dock / undock window**, `tt-pane apply dock`) or by dragging it onto the panel. Each docked window shows as a live preview, rendered at 1920×1080 off screen and scaled into a 16:9 slot. Click a preview to bring that window into the Center position of the space the panel leaves; it goes back to the dock as soon as you focus another window, and choosing a docked window with Alt+Tab or the taskbar brings it out too. The panel reserves its space like the taskbar, so maximized windows, snaps and tiling stay clear of it. Ctrl+Alt+D on a docked window undocks it, and turning the dock off or quitting tt-pane puts every docked window back where it was. Off by default.

## 0.3.0 — 2026-11-25

- Experimental: snaps can glide the window into place instead of jumping. Turn on **Settings → Snapping → Animation → Animate snapping**. It works from a snapped, floating or maximized window, on a fast-slow-fast curve that takes 160 ms in total (`[animation] duration_ms`, 50–1000; no Settings control yet). A window larger than its target shrinks in the first 40% of that time, so the application relayouts fewer times. In a centered group, focusing an offset window glides it into the center, while the window it replaces steps aside instantly. Off by default; tiling moves stay instant.
- On Windows, pressing Ctrl+C in the terminal you started tt-pane from now quits it cleanly, the same as **Quit** in the tray, with exit code 0. Before, Windows killed it with STATUS_CONTROL_C_EXIT, which `cargo run` reported as `process didn't exit successfully`. If tt-pane doesn't respond, a second Ctrl+C still kills it.
- If another application already owns one of your shortcuts, the rest now keep working. Before, one taken key left tt-pane with no shortcuts at all. On Windows, Microsoft Edge's search bar takes Shift+Win+F, the default for **Toggle float**, which stopped Ctrl+Alt+1 and every other shortcut. Settings marks each shortcut that couldn't be registered with a red "In use by another app" beside its name, and the message at the bottom and the tray tooltip name them, so you can pick another key.
- With automatic tiling on, a snapped window now keeps its space. Tiles move out of the way into the largest area that snapped windows leave free: with the left half and the bottom-right quarter snapped, only the top-right quarter is tiled, and re-snapping the left window to the bottom left frees the top left too. Resizing a snapped window gives what it frees back to the tiles. Before, tiles kept covering the whole monitor behind snapped windows.
- **Settings → Tiling → Snaps rearrange tiles** is removed. It docked a snapped window into the layout, so the next new window split it (a window snapped to the right half could end up sharing it with the new window). Snaps now always keep their space as above. `snap_rearranges` in an existing configuration is ignored, and it is dropped the next time Settings saves.
- Settings is reorganized into **Shortcuts**, **Snapping**, **Tiling**, **Focus** and **General** tabs, so the other settings are no longer squeezed into a small scroll box beside the shortcuts. Shortcuts are grouped into Snap, Size, Focus and Tiling, a dot marks tabs with unsaved changes, and **Reset defaults** and **Save changes** sit at the bottom of every tab. On/Off buttons are now switches, the **Advanced** fold is gone, and the **Clear** button is replaced by a red ⊘ that appears when you hover a shortcut's row. The tiling switches now appear on every platform that tiles, not only Windows.
- Experimental automatic tiling now works on macOS, GNOME on Wayland and X11 as well as Windows. Turn it on in **Settings → Tiling**; tiling shortcuts start unbound on these platforms. macOS keeps a layout per Space and monitor. GNOME keeps one per workspace and monitor and needs the updated Shell extension (version 4): run **Set up GNOME**, then log out and back in. Windows that open maximized, including ones GNOME maximizes because they open nearly full-screen, tile too; maximizing a tile yourself takes it out of the layout. X11 keeps one layout per desktop and monitor and needs an EWMH window manager; terminals that size in character cells stay tiled.
- When a tiling change would squeeze a window below its minimum size, the layout is now put back exactly. Before, the window that had already moved could be left at the rejected size and floated.
- On Windows, holding Ctrl+Alt+H/L into the middle of the screen now goes to your centered windows, even when an unrelated floating window is nearer. Holding from a floating window can also reach the centered group.
- On Windows, centered groups no longer forget a window that is briefly minimized, maximized, hidden, on another virtual desktop, or can't be read for a moment. It keeps its place and rejoins when it's back where tt-pane left it. Closing a window, or moving or resizing it yourself, still takes it out of the group. Each time a window leaves the group, the reason is written to `config.toml.centered.log` beside your configuration.
- macOS and Linux: **Check for updates…** and **Update and restart** now work from the tray / menu bar and Settings too. The macOS app updates from the release's disk image (signature checked), and standalone macOS executables and Linux archive or `install.sh` installs update from the archive. The new version is verified, test-run and swapped in before tt-pane quits; if it doesn't start, the previous version is restored. Configuration is kept.
- Windows: check for the latest stable release automatically at startup and every eight hours, or from the tray or Settings, then choose update and restart. Installer and portable copies verify downloads before exiting and preserve configuration; failures report an error and allow retry.
- Choose a color or monochrome tray icon on macOS and Linux in **Settings → General** (`tray_icon = "color"` or `"monochrome"`). macOS defaults to monochrome, which follows the menu bar's appearance; Linux defaults to color, and its monochrome icon is white for dark panels.
- On GNOME Wayland the Settings window has its own title bar, border and resize edges, so it can be moved, resized, minimized, maximized and closed. GNOME draws no window frame for apps.
- On Linux tt-pane installs its icon and a `dev.ttpane.TtPane.desktop` entry in `~/.local/share` when none exists, so the dock and app switcher show its icon for the Settings window however tt-pane was started. Entries written by `install.sh` or a package are left alone.

## 0.2.1 — 2026-11-03

- Fixed centered windows shrinking to the default Center size when selecting an unrelated window, including a snapped window behind them or a window on another monitor. Each monitor keeps its centered layout; the main window shrinks only when another centered member on the same monitor takes over. Applies to centered groups on Windows and GNOME Wayland.
- Added regression coverage for unrelated focus changes and independent monitor groups, and updated the centered-group documentation.

## 0.2.0 — 2026-11-03

Changes since 0.1.0:

- Sway and Hyprland support is removed, including `tt-pane export` and the **Export shortcuts** button. Tiling compositors already manage placement and focus; use GNOME or an X11 session.
- Keyboard shortcuts and window positioning now work on GNOME Wayland. tt-pane installs a bundled GNOME Shell extension and registers its bindings as GNOME custom shortcuts (**Set up GNOME** in Settings, or `tt-pane gnome setup`); log out and back in once after the first setup.
- Directional focus (Ctrl+Alt+H/J/K/L) and centered window groups now work on GNOME Wayland: H/L rotates through windows you centered, as on Windows. The extension gains window listing and focus methods, so run **Set up GNOME** again and log out and back in once. Holding a focus key isn't available on GNOME, because GNOME shortcuts only report presses. Paired vertical resizing is still Windows only.
- tt-pane starts at sign-in on Windows, macOS and Linux, on by default. Turn it off in **Settings → General → Launch at login** (`launch_at_login` in the configuration). It uses the per-user Run registry value, a LaunchAgent, or an XDG autostart entry. The Windows installer no longer has a startup-shortcut task and removes the one earlier versions created.
- macOS ships as `tt-pane.app` in a designed drag-to-Applications `.dmg`, like tt-git. It runs from Finder, Launchpad, or Spotlight as a menu-bar app with no terminal; the bare `.tar.gz` remains for scripted use.
- On macOS, tt-pane asks for Accessibility access at launch with the system dialog and opens Settings to explain it until access is granted. Opening the app again while it runs shows Settings, and startup errors appear in an alert.
- The macOS release is signed and notarized when signing secrets are configured, and signed ad hoc otherwise.
- Directional focus (Ctrl+Alt+H/J/K/L) now works on macOS. It uses Accessibility to raise the neighboring window on the current Space. Default vim focus keys are stored in lowercase, matching the shortcut recorder.

- New install scripts download and verify release packages for Windows, Linux, and macOS CLI use. Linux script installs include an app launcher and icon.
- Windows centered window groups retain each member's focused size, shrink and stagger inactive members, and restore members on mouse or Alt+Tab focus.
- H/L rotates through centered groups. Hold either horizontal focus shortcut for 400 ms to enter middle windows from an edge-snapped window or return to background edge windows. J/K exits groups vertically.
- Regular directional navigation targets edge-snapped windows by default, including covered windows. Middle-window navigation and its visibility filter are separately configurable and default to off.
- Configurable Windows focus glow highlights successful positioning and focus changes. Fixed the settings color picker crash and confirmed activation before highlighting.
- New configurations default to three 15% growth steps. Saved growth values remain unchanged. Percentage controls now live under the collapsed Advanced settings section.
- Removed the redundant Close settings button; use the window close control to leave the tray running.
- Documentation includes updated navigation and settings guidance, an animated hero, a playable promo thumbnail, and social sharing previews.
- Fixed Linux/macOS compilation of shortcut handling by gating the Windows foreground-window lookup.
- Windows file and product versions now derive from Cargo, alongside CLI version reporting; packaging rejects version mismatches.

Centered groups work on Windows and GNOME Wayland; directional focus also works on macOS. Focus glow, hold shortcuts, and paired vertical resizing are Windows-only. Holds and continuous mouse/Alt+Tab group observation require the tray app. Linux X11 supports positioning but not directional focus or centered groups.

## 0.1.0

Initial release with native window positioning, configurable shortcuts, anchored growth, width adjustment, gap filling, Windows shared-boundary resizing and directional focus, and release packages for Windows x64, Linux x64, and macOS Apple Silicon.
