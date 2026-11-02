# sTile

A keyboard-first window position manager written in Rust, with a GPUI settings screen and native platform backends.

Every action targets the **active window on its current monitor**, using that monitor's work area. Windows spanning displays use the display with the largest overlap; an off-screen window uses the nearest display. Center preserves the window size unless it is larger than the work area. There is no primary-monitor assumption in the positioning engine.

## Run

Install a recent stable Rust toolchain, then:

```sh
cargo run --release
```

sTile starts in the system tray without a settings window. Open **Settings…** from its tray menu, click a shortcut to record a replacement, then **Save changes**. Escape cancels recording; Backspace disables an action. Changes are validated for duplicate actions and shortcuts. Native registration conflicts appear in the settings screen, and a failed registration preserves the previous bindings. Saving uses a temporary file in the same directory followed by replacement.

Closing Settings leaves the tray and shortcuts running. **Quit sTile** in the tray menu exits the app. `stile settings` opens settings immediately when starting an instance. Start one tray instance at a time; single-instance forwarding is not implemented yet.

Windows GUI builds do not allocate a console. macOS uses an accessory app with a menu-bar icon and no Dock icon. Linux uses the StatusNotifierItem D-Bus protocol; enable a tray host such as KDE, a Waybar tray module, or the GNOME AppIndicator extension. No GTK tray event loop is required.

Launch-at-login installation, signed application bundles, and automatic configuration-file watching remain future work.

### Build prerequisites

- **Windows:** Visual Studio Build Tools with Desktop development with C++ and a Windows SDK. GPUI uses Direct3D. Run normally; elevated apps may reject changes from an unelevated process.
- **macOS:** Xcode and its command-line tools, including Metal tooling. Grant the executable Accessibility access in System Settings → Privacy & Security → Accessibility. Rebuilding or moving an unsigned executable can require granting access again.
- **Linux:** a GPU driver supporting GPUI, compiler/build tools, pkg-config, and the development packages for fontconfig, FreeType, X11/XCB, XKB, and Wayland. On Ubuntu/Debian, start with:

```sh
sudo apt install build-essential clang cmake pkg-config libssl-dev libfontconfig1-dev libfreetype-dev libx11-dev libx11-xcb-dev libxcb1-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libvulkan-dev
```

The command-line positioning tool can be built independently of GPUI:

```sh
cargo build --release --no-default-features
```

## Default bindings

| Shortcut | Action |
| --- | --- |
| Ctrl+Alt+Numpad1 | Left half |
| Ctrl+Alt+Numpad2 | Right half |
| Ctrl+Alt+Numpad3 | Center without resizing (clamp oversized windows) |
| Ctrl+Alt+Numpad4 | Bottom left quarter |
| Ctrl+Alt+Numpad5 | Bottom center (bottom half) |
| Ctrl+Alt+Numpad6 | Bottom right quarter |
| Ctrl+Alt+Numpad7 | Top left quarter |
| Ctrl+Alt+Numpad8 | Top center (top half) |
| Ctrl+Alt+Numpad9 | Top right quarter |
| Ctrl+Alt+Numpad0 | Fill work area |

These are numeric keypad keys, distinct from the number row. Enable Num Lock on Windows/Linux. The shortcut recorder preserves the numpad distinction; number-row keys can still be assigned explicitly. macOS uses the same Control+Option defaults; Command shortcuts can be recorded too. System and application shortcuts can reserve combinations; choose alternatives where needed. Some applications impose minimum window sizes, so requested bounds are not always achievable.

## Platforms and limits

| Platform | Window control | Global shortcuts |
| --- | --- | --- |
| Windows | Win32 `GetForegroundWindow`, `MonitorFromWindow`, `GetMonitorInfoW`, `SetWindowPos`; DWM visible-frame correction and per-monitor DPI context | Native `RegisterHotKey` through `global-hotkey` |
| macOS | Accessibility AX focused-window position/size, `NSScreen.visibleFrame`; conversion from Cocoa to AX coordinates | Native Carbon registration through `global-hotkey` |
| Linux X11 | EWMH active-window/move-resize messages, RandR monitors, monitor-specific partial panel struts | X11 key grabs through `global-hotkey` |
| Wayland / Sway | Native Unix-socket IPC; containing workspace rectangle excludes panels | Exported compositor bindings |
| Wayland / Hyprland | Native Unix-socket IPC; window monitor ID, scale, rotation, and reserved margins | Exported compositor bindings |
| Wayland / GNOME, KDE, other compositors | **Not implemented yet.** Requires compositor-specific integration | Not implemented |

Wayland does not offer a universal API to reposition another application's windows. Native Wayland windows cannot be controlled through the X11 backend. Sway and Hyprland are supported explicitly; GNOME Shell and KWin adapters remain future work. The Sway and Hyprland backends float the selected window so that exact placement is possible. Exit fullscreen before positioning on Hyprland or macOS. On X11, the window manager must implement EWMH move/resize and RandR 1.5 monitor enumeration. Panels exposing only legacy struts are not yet accounted for.

### Wayland shortcuts

Save your configuration in GPUI, then click **Export shortcuts**. Add the exported file to your compositor configuration with an `include` (Sway) or `source` (Hyprland) directive and reload the compositor. Repeat export/reload after changing bindings. Existing compositor shortcuts can intercept a key before the recorder receives it; use an available combination or edit the TOML file.

Alternatively, generate the file with the installed executable:

```sh
stile export sway > ~/.config/sway/stile.conf
# Add to ~/.config/sway/config:
# include ~/.config/sway/stile.conf

stile export hyprland > ~/.config/hypr/stile.conf
# Add to ~/.config/hypr/hyprland.conf:
# source = ~/.config/hypr/stile.conf
```

Export embeds the executable's absolute path. Generate bindings after installing the executable in its final location. Export supports letters, number-row and numpad digits, function keys, arrows, and common navigation keys; other physical keys report an explicit error. Generated shortcuts invoke the native Rust backend directly, so the GPUI settings window need not stay open on Wayland.

## Configuration and CLI

The default file is:

- Windows: `%APPDATA%\stile\config\config.toml`
- macOS: `~/Library/Application Support/stile/config.toml`
- Linux: `${XDG_CONFIG_HOME:-~/.config}/stile/config.toml`

The settings screen displays the resolved path. Use `--config PATH` for a different file. A missing file uses defaults; an invalid existing file returns an error instead of overwriting it. Omitted actions appear disabled in the UI. An empty shortcut disables an action.

```toml
version = 1

[[bindings]]
action = "left"
shortcut = "Ctrl+Alt+Numpad1"

[[bindings]]
action = "top-left"
shortcut = "Ctrl+Alt+Numpad7"
```

```sh
stile apply top-left
stile apply center
stile config
stile --config ./my-config.toml
```

`apply` acts immediately on the current foreground window; it does not depend on the configuration file, GPUI, a tray instance, or global key registration. It is suitable for shortcuts managed by an existing desktop environment.

## Executable plugin interface

The root `herdr-plugin.toml` follows [Herdr’s executable plugin contract](https://herdr.dev/tui/docs/plugins/): an argv command per action, without a shell wrapper or plugin SDK. It declares all ten positions and a build step for the standalone CLI. For a local checkout:

```sh
cargo build --release --locked --no-default-features --target-dir target/plugin
herdr plugin link /absolute/path/to/sTile
herdr plugin action invoke dev.stile.top-left
```

Build before linking: `plugin link` does not run build commands. Herdr executes relative commands from the plugin root; Windows resolves the executable’s `.exe` extension. Herdr pane IDs identify terminal panes, not OS windows. These actions position the foreground desktop window on the machine running the plugin; they do not reposition remote client windows over SSH. Use Herdr’s plugin keybindings to invoke them within Herdr. The manifest is tested against the CLI action inventory; installation/invocation inside Herdr still needs integration testing.

For scripts and test runners:

```sh
stile actions
stile apply top-left --json
# Windows only: target a decimal HWND without changing focus.
stile apply bottom-right --window 123456 --json
```

`actions` returns a JSON object with `schema_version: 1` and an `actions` array of `id`/`title` pairs without accessing the desktop. `apply --json` returns one JSON object with `schema_version`, `ok`, `action`, `window` (null for foreground), and `error`. Exit code 0 means the native request succeeded, 1 means execution failed, and 2 means invalid CLI arguments. Argument errors use Clap’s stderr diagnostics, not JSON. Success acknowledges the backend request; compositors or application minimum sizes may constrain the resulting geometry. Handles are transient and must be refreshed if a window closes.

## Icon and tray lifecycle

`assets/stile.svg` is the editable window-grid mark. PNG, multi-resolution ICO, ICNS, and macOS template tray assets are included. Windows embeds the ICO; macOS uses a template icon that adapts to the menu bar. Regenerate raster assets with Pillow and `python scripts/generate_icons.py`.

GPUI 0.2.2 normally exits after closing its last Windows/X11/Wayland window. The pinned, vendored dependency adds an opt-in `keep-alive` feature so settings can be destroyed while the tray keeps running. See `vendor/README.md` for the exact changes. No hidden GPUI window is created at startup.

## Native tests without Computer Use

Command-level tests spawn the actual binary and verify JSON and exit codes. On Windows, tests create a hidden disposable window, invoke all ten positioning commands for every attached monitor, and read back bounds with Win32. They do not activate a window, inject input, or move user windows. This was verified against four attached monitors (40 position checks).

An opt-in Windows GPU/desktop test verifies windowless tray startup and closing Settings while keeping the process alive:

```sh
cargo test --test tray_lifecycle -- --ignored --nocapture
```

The lifecycle test briefly creates its own settings window, closes it with a native message, and terminates only its own child processes. macOS and Linux tray rendering/lifecycle still require runtime validation on those platforms.

## Structure and validation

- `geometry.rs`: shared placement math and monitor selection.
- `config.rs`: versioned TOML, validation, atomic save.
- `shortcuts.rs`: native shortcut lifetime, conflict handling, Wayland export.
- `platform/`: native Windows, macOS, X11, Sway, and Hyprland adapters.
- `ui.rs`: GPUI settings, shortcut recorder, preview, save/error feedback.

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Tests cover secondary/negative-origin monitors, spanning/off-screen selection, centering, odd pixel dimensions, configuration persistence, duplicate validation, compositor export, and native Windows shortcut conflicts. Platform-specific runtime behavior still needs validation on physical macOS and Linux desktops; compiling a backend is not a substitute for that testing.

References: [GPUI](https://gpui.rs/), [EWMH](https://specifications.freedesktop.org/wm/latest-single/), [Sway IPC](https://man.archlinux.org/man/sway-ipc.7.en), [Hyprland IPC](https://wiki.hypr.land/IPC/), [KWin scripting API](https://develop.kde.org/docs/plasma/kwin/api/).
