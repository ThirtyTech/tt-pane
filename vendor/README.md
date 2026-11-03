# Vendored dependency patches

## GPUI 0.2.2

`gpui/` is the published GPUI 0.2.2 source from crates.io (Apache-2.0; upstream license is preserved). tt-pane enables a local `keep-alive` feature. Without it, GPUI automatically stops the Windows, X11, and Wayland event loops when the last window is destroyed, which also stops tray shortcuts.

Changes from the release:

- `Cargo.toml`: declare `keep-alive = []`.
- `src/platform/windows/platform.rs`: still remove the closed window, but skip automatic `PostQuitMessage` when `keep-alive` is enabled.
- `src/platform/linux/{x11,wayland}/client.rs`: still clean up the window, but skip stopping the event loop when `keep-alive` is enabled.
- `src/taffy.rs`: explicitly type the grid helper's `0.0` and `1.0` literals as `f32`, avoiding Rust's deprecated float fallback.

Explicit `App::quit()` remains unchanged. macOS already keeps running without windows. No hidden application window is used to keep the process alive. Revisit this patch when upstream exposes an equivalent supported lifecycle option. The Windows lifecycle integration test covers startup without a window and closing settings without exiting.

The opt-in `preserve-numpad` feature also preserves modified numeric keypad events as `Numpad0`–`Numpad9` in Windows, macOS, and Linux key translation. This lets the shortcut recorder distinguish them from number-row digits. Unmodified text input retains upstream behavior. Windows and Linux numeric shortcuts require Num Lock enabled.

## proc-macro-error2 2.0.1

`proc-macro-error2/` is the published 2.0.1 source (MIT OR Apache-2.0; both licenses are preserved), used transitively by GPUI's stacksafe macros. The only source change makes `extern crate proc_macro` public, matching its existing public re-export in `__export`. This fixes Rust's private-extern-crate re-export incompatibility (rust-lang/rust#127909) without suppressing the warning or changing macro behavior. Cargo's root patch and lockfile ensure clean checkouts use this fix. Remove the override when the dependency chain provides a compatible release.

## xattr 0.2.3

`xattr/` is the published 0.2.3 source (MIT OR Apache-2.0; both licenses and upstream tests are preserved), used by GPUI through `gpui_http_client` and `zed-async-tar`. In `src/sys/mod.rs`, Linux and Android use `libc::ENODATA` for missing extended attributes because modern libc versions removed their deprecated `ENOATTR` alias. macOS and BSD retain `libc::ENOATTR`. This preserves missing-attribute behavior without pinning an outdated libc. Remove the patch when the upstream dependency chain adopts a compatible xattr release.
