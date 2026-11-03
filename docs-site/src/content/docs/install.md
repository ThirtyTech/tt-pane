---
title: Installation
description: Install sTile on Windows, macOS, or Linux and place your first window.
---

## Download sTile

Choose the matching package from [GitHub Releases](https://github.com/jsheely/sTile/releases). Packages appear after a matching version tag is published.

| Platform | Package | Start |
| --- | --- | --- |
| Windows x64 | `stile-vVERSION-windows-x64-setup.exe` | Install, then launch sTile from Start |
| Windows x64, portable | `.zip` | Extract and run the executable |
| macOS Apple Silicon | `.tar.gz` | Extract, then run `./stile` |
| Linux x64 | `.tar.gz` | Extract, then run `./stile` |

Windows installation is per-user and needs no administrator rights. The installer offers desktop and sign-in shortcuts. Linux and macOS downloads are standalone executables, not system packages or a macOS app bundle. See [Platforms & troubleshooting](/sTile/platforms/) for permissions and native dependencies.

Packages are unsigned. Each release includes `SHA256SUMS.txt`. Check your file with `Get-FileHash -Algorithm SHA256` on Windows, `shasum -a 256` on macOS, or `sha256sum` on Linux.

## Your first placement

1. Launch sTile. It starts in the tray without opening a settings window.
2. Select a window and press **Ctrl + Alt + 1** to place it in the left half.
3. Press **Ctrl + Alt + 2** to move it to the right half.
4. Try **Ctrl + Alt + 3** to center it at half width and half height.

On macOS, use **Control + Option**. On Sway or Hyprland, [export compositor bindings](/sTile/platforms/#wayland-shortcuts) first.

Open **Settings…** from the tray to customize shortcuts. Closing Settings leaves sTile running; **Quit sTile** exits. Run one tray instance at a time: single-instance forwarding is not implemented.

## Upgrade or uninstall

Quit the existing instance before upgrading. On Windows, rerun the installer. Deselecting a previously selected shortcut removes it. Uninstall through **Settings → Apps → Installed apps → sTile**; your configuration is preserved.

The installer does not change PATH. For terminal commands, use the installed executable’s full path. Automatic sign-in startup is an installer option on Windows; other platforms require manual setup.

## Build from source

Install a recent stable Rust toolchain and the [native build prerequisites](https://github.com/jsheely/sTile#build-prerequisites), then run from the repository:

```sh
cargo run --release
```

For the command-line positioning tool without GPUI:

```sh
cargo build --release --no-default-features
```
