# Changelog

## Unreleased

- Sway and Hyprland support is removed, including `tt-pane export` and the **Export shortcuts** button. Tiling compositors already manage placement and focus; use GNOME or an X11 session.
- Keyboard shortcuts and window positioning now work on GNOME Wayland. tt-pane installs a bundled GNOME Shell extension and registers its bindings as GNOME custom shortcuts (**Set up GNOME** in Settings, or `tt-pane gnome setup`); log out and back in once after the first setup.
- Directional focus (Ctrl+Alt+H/J/K/L) and centered window groups now work on GNOME Wayland: H/L rotates through windows you centered, as on Windows. The extension gains window listing and focus methods, so run **Set up GNOME** again and log out and back in once. Holding a focus key isn't available on GNOME, because GNOME shortcuts only report presses. Paired vertical resizing is still Windows only.
- tt-pane starts at sign-in on Windows, macOS and Linux, on by default. Turn it off in **Settings → General → Launch at login** (`launch_at_login` in the configuration). It uses the per-user Run registry value, a LaunchAgent, or an XDG autostart entry. The Windows installer no longer has a startup-shortcut task and removes the one earlier versions created.
- macOS ships as `tt-pane.app` in a designed drag-to-Applications `.dmg`, like tt-git. It runs from Finder, Launchpad, or Spotlight as a menu-bar app with no terminal; the bare `.tar.gz` remains for scripted use.
- On macOS, tt-pane asks for Accessibility access at launch with the system dialog and opens Settings to explain it until access is granted. Opening the app again while it runs shows Settings, and startup errors appear in an alert.
- The macOS release is signed and notarized when signing secrets are configured, and signed ad hoc otherwise.
- Directional focus (Ctrl+Alt+H/J/K/L) now works on macOS. It uses Accessibility to raise the neighboring window on the current Space. Default vim focus keys are stored in lowercase, matching the shortcut recorder.

## 0.2.0 — 2026-11-03

Changes since 0.1.0:

- Windows centered window groups retain each member's focused size, shrink and stagger inactive members, and restore members on mouse or Alt+Tab focus.
- H/L rotates through centered groups. Hold either horizontal focus shortcut for 400 ms to enter middle windows from an edge-snapped window or return to background edge windows. J/K exits groups vertically.
- Regular directional navigation targets edge-snapped windows by default, including covered windows. Middle-window navigation and its visibility filter are separately configurable and default to off.
- Configurable Windows focus glow highlights successful positioning and focus changes. Fixed the settings color picker crash and confirmed activation before highlighting.
- New configurations default to three 15% growth steps. Saved growth values remain unchanged. Percentage controls now live under the collapsed Advanced settings section.
- Removed the redundant Close settings button; use the window close control to leave the tray running.
- Documentation includes updated navigation and settings guidance, an animated hero, a playable promo thumbnail, and social sharing previews.
- Fixed Linux/macOS compilation of shortcut handling by gating the Windows foreground-window lookup.
- Windows file and product versions now derive from Cargo, alongside CLI version reporting; packaging rejects version mismatches.

The new grouping, directional focus, and glow behavior is Windows-only. Hold shortcuts and continuous mouse/Alt+Tab group observation require the tray app.

## 0.1.0

Initial release with native window positioning, configurable shortcuts, anchored growth, width adjustment, gap filling, Windows shared-boundary resizing and directional focus, and release packages for Windows x64, Linux x64, and macOS Apple Silicon.
