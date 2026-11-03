# Changelog

## Unreleased

- macOS ships as `tt-pane.app` in a designed drag-to-Applications `.dmg`, like tt-git. It runs from Finder, Launchpad, or Spotlight as a menu-bar app with no terminal; the bare `.tar.gz` remains for scripted use.
- On macOS, tt-pane asks for Accessibility access at launch with the system dialog and opens Settings to explain it until access is granted. Opening the app again while it runs shows Settings, and startup errors appear in an alert.
- The macOS release is signed and notarized when signing secrets are configured, and signed ad hoc otherwise.

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
