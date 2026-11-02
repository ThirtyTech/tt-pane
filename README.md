# sTile

A keyboard-first window position manager written in Rust, with a GPUI settings screen and native platform backends.

Every action targets the **active window on its current monitor**, using that monitor's work area. Windows spanning displays use the display with the largest overlap; an off-screen window uses the nearest display. The first Center press resizes the window to 50% of the work area's width and height and centers it on both axes. There is no primary-monitor assumption in the positioning engine.

## Install

Download packages from [GitHub Releases](https://github.com/jsheely/sTile/releases). Release packages become available after a matching version tag is published.

- **Windows x64:** run `stile-vVERSION-windows-x64-setup.exe`. It installs for your account without administrator rights, adds a Start menu shortcut, and offers optional desktop and sign-in shortcuts. Launch sTile from Start, then open Settings from its tray menu. The portable ZIP can be unpacked anywhere instead.
- **Linux x64 / macOS Apple Silicon:** extract the matching `.tar.gz` and run `./stile`. These are standalone executables, not system packages or a macOS app bundle. See the platform prerequisites below; macOS requires Accessibility permission and Linux requires its native libraries and a tray host.

Packages are unsigned. Each release includes `SHA256SUMS.txt`; compare your download with `Get-FileHash -Algorithm SHA256` (Windows), `sha256sum` (Linux), or `shasum -a 256` (macOS).

Quit the existing tray instance before upgrading. On Windows, rerun the installer; deselecting a previously selected shortcut removes it. Uninstall through **Settings → Apps → Installed apps → sTile**. Your configuration in `%APPDATA%\stile\config` is preserved. The installer does not change PATH; use the installed executable's full path for CLI commands. Login startup is available through the Windows installer; other platforms still require manual setup.

For local packaging, release workflow details, and installer artwork, see [packaging/README.md](packaging/README.md).

## Run from source

Install a recent stable Rust toolchain, then:

```sh
cargo run --release
```

sTile starts in the system tray without a settings window. Open **Settings…** from its tray menu, click a shortcut to record a replacement, then **Save changes**. Escape cancels recording; Backspace disables an action. Changes are validated for duplicate actions and shortcuts. Native registration conflicts appear in the settings screen, and a failed registration preserves the previous bindings. Saving uses a temporary file in the same directory followed by replacement.

Closing Settings leaves the tray and shortcuts running. **Quit sTile** in the tray menu exits the app. `stile settings` opens settings immediately when starting an instance. Start one tray instance at a time; single-instance forwarding is not implemented yet.

Hover over a keybinding row to animate **Position preview** to that action's initial placement. The green rectangle represents the window and the dark rectangle represents the monitor's usable space. The preview keeps the last hovered action when the pointer leaves. Clicking the shortcut button starts recording; hovering only changes the illustration.

Windows GUI builds do not allocate a console. When launched from a terminal (including `cargo run`), they attach to that terminal so **Ctrl+C terminates sTile**. Launching from Explorer or a shortcut remains console-free; piped CLI output is preserved. macOS uses an accessory app with a menu-bar icon and no Dock icon. Linux uses the StatusNotifierItem D-Bus protocol; enable a tray host such as KDE, a Waybar tray module, or the GNOME AppIndicator extension. No GTK tray event loop is required.

Signed application bundles, launch-at-login installation outside Windows, and automatic configuration-file watching remain future work.

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
| Ctrl+Alt+1 | Left half |
| Ctrl+Alt+2 | Right half |
| Ctrl+Alt+3 | Center at 50% width and 50% height |
| Ctrl+Alt+4 | Bottom left quarter |
| Ctrl+Alt+5 | Bottom center (bottom half) |
| Ctrl+Alt+6 | Bottom right quarter |
| Ctrl+Alt+7 | Top left quarter |
| Ctrl+Alt+8 | Top center (top half) |
| Ctrl+Alt+9 | Top right quarter |
| Ctrl+Alt+0 | Fill work area |
| Ctrl+Alt+- | Shrink width by 25% |
| Ctrl+Alt+= | Expand width by 25% |
| Ctrl+Alt+Shift+8 (*) | Fill the last saved gap |
| Ctrl+Alt+Up | Move shared horizontal boundary up |
| Ctrl+Alt+Down | Move shared horizontal boundary down |
| Ctrl+Alt+H | Focus left |
| Ctrl+Alt+J | Focus down |
| Ctrl+Alt+K | Focus up |
| Ctrl+Alt+L | Focus right |

The digit shortcuts use number-row keys (Digit0 through Digit9). The minus shortcut uses the `-` key; expansion uses the single key normally labeled `=` and `+`. **Shift is not required**. Ctrl+Alt+Shift+= also expands, unless that combination is explicitly assigned to another action. The recorder also supports distinct numpad keys when explicitly assigned. macOS uses the same Control+Option defaults; Command shortcuts can be recorded too. System and application shortcuts can reserve combinations; choose alternatives where needed. Some applications impose minimum window sizes, so requested bounds are not always achievable.

### Width adjustment

**Ctrl+Alt+-** reduces the current width by 25%; **Ctrl+Alt+=** increases it by 25%. Both preserve height and vertical position. The last matching placement supplies the anchor: left positions keep their left edge, right positions keep their right edge, and centered positions keep their midpoint. For a manually placed window, its horizontal position on the current monitor determines the anchor. Shrinking a bottom-left window moves only its right edge inward.

Repeated presses keep resizing, with no reset cycle. Expansion stops at the available monitor boundary for that anchor, and shrinking stops at the application's minimum width (at least one pixel). Percentages apply to the current width, so shrinking and expanding once are not inverse operations: 1000 → 750 → 938 pixels. Adjust the shared percentage under **Width adjustment → Width change per press** in Settings (1–99%, default 25%). The two-dimensional growth controls below it apply only to repeated positioning shortcuts.

In Position preview, hover a placement first, then Shrink width or Expand width to see how the width changes for that placement. The preview uses the configured width percentage.

### Resize neighboring windows vertically (Windows)

**Ctrl+Alt+H/J/K/L** switches focus left/down/up/right on Windows without moving or resizing windows. Each press selects the nearest window by center distance within a 90-degree cone in that direction, including windows on other monitors. For example, moving right from bottom-left visits a nearby center window before the far-right window. Equal-distance ties prefer the topmost window. Hidden, minimized, cloaked (such as windows on other virtual desktops), and utility/no-activate windows are skipped. Focus stays put when there is no candidate; there is no wraparound. Saved growth and gap state is preserved.

The focus actions are editable in Settings and available as `stile apply focus-left`, `focus-down`, `focus-up`, and `focus-right` (also with `--window HWND --json`). Existing configurations gain missing focus bindings on load; custom or explicitly disabled focus bindings remain unchanged. If a default key is already assigned to another action, the new focus action stays unbound so you can choose a key in Settings. Other platforms report that directional focus is currently Windows-only.

**Ctrl+Alt+Up/Down** moves the shared horizontal boundary by **25% of the selected window's current height**. With the bottom-left window selected, Up grows it upward and shrinks the window above: a 50/50 split becomes 62.5/37.5. Down shrinks the bottom window and grows the top window by the same pixel amount. There is a **50/50 stop** in the middle: 400 pixels grows to 500, then Down returns it to 400; shrinking to 300 then pressing Up also returns to 400. When moving toward equal heights, sTile snaps there if the boundary is within one normal step from equal (25% of half the combined height), or if the current step would cross it. Another press continues past equal. Elsewhere, each press uses the selected window's current height. Odd combined heights give the bottom window the extra pixel. With the top window selected, Up shrinks it and grows the window below. Widths and the pair's outside edges stay fixed; any existing gap is preserved.

The neighbor must be visible, restored, on the same monitor, and have matching left/right edges (within two pixels). sTile accepts up to 16 pixels of vertical spacing and two pixels of border-rounding overlap. It excludes minimized, maximized, cloaked, desktop, and tool windows. If there are neighbors both above and below, the arrow chooses that side; with only one neighbor, both arrows adjust that pair. Missing or ambiguous neighbors produce an error without moving windows.

The boundary stops before either window reaches zero height. If an application rejects the requested size (for example, its minimum height), sTile attempts to restore both original bounds and reports the error. Successful paired adjustments clear saved growth/gap state to avoid reusing stale rectangles. These actions currently require Windows; other backends return an explicit unsupported error.

The actions are editable in Settings and available as `stile apply resize-up` / `stile apply resize-down`, including `--window HWND --json` and Herdr actions. Older saved configurations preserve their bindings: record Ctrl+Alt+Up/Down for the two **Move shared boundary** actions in Settings, or reset defaults and save. These use a fixed 25% step independent of the width and repeated-growth settings. The preview shows the selected bottom-left window in green and its neighbor in blue.

### Fill a freed gap

After positioning and shrinking a window, select a **different window on the same monitor** and press **Ctrl+Alt+Shift+8** (`*` on the number row). sTile resizes and places that window into the space freed from the original position. For example, shrinking a 1000-pixel-wide bottom-left slot to 750 pixels leaves a 250-pixel-wide gap immediately to its right, at the same height. Repeated shrinking accumulates the freed space within that original slot.

One gap is remembered per monitor. Switching windows does not lose it; successfully filling it consumes it. If a centered window leaves two gaps, the larger side is selected, with the right side winning ties. A missing gap or selecting the source window reports an error without repositioning it. Use `stile apply fill-gap --json` for the same operation from scripts or Herdr. The binding is editable in Settings; a numpad Multiply key can also be recorded.

Gaps record sTile's own resizing operations, rather than scanning the desktop for empty space. sTile updates or clears them when it resizes/repositions their source, or places a window over the saved slot. Manual dragging, resizing, and closing windows outside sTile are not monitored; make a fresh width adjustment to update the gap. A monitor work-area change requires a new gap. Application minimum sizes can constrain how closely a destination fits.

### Repeated shortcuts

Press the same positioning shortcut again on the same window to grow it. By default, each repeat multiplies its width and height by **1.15**, with **two growth steps** before returning to the original placement:

**Place → 115% → 132.25% → original placement → repeat.** Percentages describe dimensions, not area; dimensions round to the nearest pixel.

Corners stay anchored: bottom left grows upward and to the right. Left/right halves grow inward horizontally; top/bottom halves grow inward vertically. Center starts at 50% width and 50% height, grows equally in every direction, and resets to that centered size. For example, Center after a left-half snap reduces the full height to half height. Growth is capped at the current monitor's available work area, so an already full dimension stays full. Fill work area remains full throughout the cycle.

In Settings, use **Growth per press** (1–100%) and **Growth steps before reset** (0–20), then **Save changes**. Zero steps disables growth. Changing the action, target window, monitor work area, or growth settings starts a new cycle. Moving or resizing the window externally also starts a new cycle (allowing two pixels for platform rounding). There is no timeout between presses.

Tray shortcuts, `apply`, Herdr actions, and exported Wayland shortcuts share the same cycle for a configuration file. State is stored in `<config-path>.cycle.json`, with a separate `.cycle.lock` file to serialize concurrent invocations. Deleting the state file resets the cycle. Failed native requests do not advance it. Windows and macOS store observed bounds; X11/Wayland store requested bounds because their operations are asynchronous, so compositor-imposed bounds can restart a cycle.

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

[growth]
percent = 15
steps = 2

[width_resize]
percent = 25

[[bindings]]
action = "left"
shortcut = "Ctrl+Alt+1"

[[bindings]]
action = "top-left"
shortcut = "Ctrl+Alt+7"

[[bindings]]
action = "shrink-width"
shortcut = "Ctrl+Alt+-"

[[bindings]]
action = "expand-width"
shortcut = "Ctrl+Alt+="

[[bindings]]
action = "fill-gap"
shortcut = "Ctrl+Alt+Shift+8"
```

```sh
stile apply top-left
stile apply center
stile apply shrink-width
stile apply expand-width
stile apply fill-gap
stile config
stile --config ./my-config.toml
```

`apply` acts immediately on the current foreground window and reads the configuration's resize options (or defaults when the file is absent). It does not require GPUI, a tray instance, or global key registration. Repeated positioning commands advance the same growth cycle; width commands resize continually. Use `--config PATH` to select a configuration and its independent state. Older files without `[growth]` use 15% and two steps; missing `[width_resize]` uses 25%. Existing bindings are preserved: add the two width bindings above, record them in Settings, or reset defaults to enable them in an older file. Exported Wayland shortcuts include the selected configuration path and the shifted-plus alias when available.

## Executable plugin interface

The root `herdr-plugin.toml` follows [Herdr’s executable plugin contract](https://herdr.dev/tui/docs/plugins/): an argv command per action, without a shell wrapper or plugin SDK. It declares all ten positions, two width adjustments, gap filling, two shared-boundary adjustments, and a build step for the standalone CLI. For a local checkout:

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

Command-level tests spawn the actual binary and verify JSON and exit codes. On Windows, tests create hidden disposable windows, invoke all ten positioning commands and their growth/reset cycles for every attached monitor, and read back bounds with Win32. They also repeatedly shrink/expand bottom-left, bottom-right, and centered windows, verifying width-only changes and maximum-width clamping. Two-window tests fill gaps from 25%/50% reductions, leave the source untouched, reject source-window fills, and reject consumed gaps without changing bounds or state. Each press is a separate CLI process. Paired-resize tests briefly show a disposable neighbor without activation, verify both windows and successive Up/Down steps, and reject missing/hidden neighbors. Native constraint tests verify rollback when either window rejects its size. Tests do not activate windows, inject input, or move user windows. This was verified against four attached monitors (784 command-level bounds checks).

An opt-in Windows GPU/desktop test verifies windowless tray startup and closing Settings while keeping the process alive:

```sh
cargo test --test tray_lifecycle -- --ignored --nocapture
```

The lifecycle test briefly creates its own settings window, closes it with a native message, and terminates only its own child processes. macOS and Linux tray rendering/lifecycle still require runtime validation on those platforms.

## Structure and validation

- `crates/stile-platform/src/geometry.rs`: shared placement math and monitor selection.
- `config.rs`: versioned TOML, validation, atomic save.
- `positioning.rs`: shared repeated-action cycles, persisted baseline and cross-process locking.
- `shortcuts.rs`: native shortcut lifetime, conflict handling, Wayland export.
- `crates/stile-platform/`: native Windows, macOS, X11, Sway, and Hyprland adapters, plus Windows icon/console interop. The application reexports geometry and platform APIs at their existing paths.
- `ui.rs`: GPUI settings, shortcut recorder, preview, save/error feedback.

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo test --workspace --locked --no-default-features
cargo clippy --workspace --locked --all-targets --all-features -- -D warnings
```

Tests cover secondary/negative-origin monitors, spanning/off-screen selection, centering, odd pixel dimensions, configuration persistence, duplicate validation, compositor export, and native Windows shortcut conflicts. Platform-specific runtime behavior still needs validation on physical macOS and Linux desktops; compiling a backend is not a substitute for that testing.

References: [GPUI](https://gpui.rs/), [EWMH](https://specifications.freedesktop.org/wm/latest-single/), [Sway IPC](https://man.archlinux.org/man/sway-ipc.7.en), [Hyprland IPC](https://wiki.hypr.land/IPC/), [KWin scripting API](https://develop.kde.org/docs/plasma/kwin/api/).

## Rust safety boundary

The application package forbids unsafe Rust through Cargo's `unsafe_code = "forbid"` lint and explicit `#![forbid(unsafe_code)]` attributes on its library, executable, build script, and integration tests. These restrictions apply to our application code; dependencies such as GPUI still contain unsafe implementations.

`stile-platform` exposes safe functions and denies unsafe code by default. Only its private Windows/macOS modules and feature-gated native test helpers allow it. Unsafe operations must be explicit and have `SAFETY:` explanations, enforced by Clippy. Portable geometry forbids unsafe entirely; the Linux adapters inherit the platform crate's default denial.

Windows icon setup borrows a native window handle for the duration of synchronous calls. Window positioning accepts opaque OS identifiers and handles invalid windows as errors; DPI awareness is restored by a thread-bound guard, including on early returns and unwinding. Handles can still expire or be reused by the OS, so explicit targeting does not establish a persistent window identity.

macOS position/size helpers bind each native AX type tag to `CGPoint` or `CGSize`, check Core Foundation object types before interpreting Accessibility results, and adopt Create/Copy references exactly once. The native signatures and type checks follow Apple's [AX value](https://developer.apple.com/documentation/applicationservices/1462933-axvaluegetvalue) and [AX element type](https://developer.apple.com/documentation/applicationservices/1460085-axuielementgettypeid) APIs. macOS unit tests cover typed round trips and rejection of mismatched/non-AX values without requiring Accessibility permission.

The `test-support` feature is enabled by the application's dev-dependency only. It keeps native test mechanics inside the same safety boundary while integration tests exercise the real executable through safe entry points. It is not enabled by normal application builds. Use workspace-wide validation to include the extracted geometry tests and platform-specific tests.
