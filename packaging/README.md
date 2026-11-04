# Release packaging

`.github/workflows/release.yml` follows [tt-git's release workflow](https://github.com/ThirtyTech/tt-git/blob/main/.github/workflows/release.yml): version tags must match Cargo, each platform builds natively, and all successful builds are published together with SHA-256 checksums. Unlike tt-git's separate TUI/GUI jobs, tt-pane has one executable, so every platform must succeed before publication.

The initial matrix is Windows x64, Linux x64 (Ubuntu 22.04), and macOS Apple Silicon. Every platform ships a standalone archive, which the root `install.sh` (Linux, macOS) downloads, verifies and installs (adding a desktop launcher on Linux, from the `tt-pane.png` the Linux archive carries). Windows also ships a native installer, which the root `install.ps1` downloads, verifies and runs silently, and macOS a `.dmg` holding `tt-pane.app`. Intel macOS, native Windows ARM64, Linux ARM64, Windows signing, and Linux system packages are not included.

## Build locally on Windows

Install Rust, Visual Studio C++ Build Tools, and [Inno Setup 6 or newer](https://jrsoftware.org/isinfo.php). In PowerShell from the repository root:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --target x86_64-pc-windows-msvc
./packaging/windows/package.ps1 -BinDir target/x86_64-pc-windows-msvc/release -Tag v0.2.1
./packaging/windows/test-installer.ps1 -BinDir target/x86_64-pc-windows-msvc/release -Tag v0.2.1
```

Use the current Cargo version for `-Tag`. Static CRT linking avoids requiring a separate Visual C++ runtime installation. `package.ps1` checks the PE architecture, the executable's reported version, and Windows file/product versions. CLI and Windows metadata derive from the application Cargo version; the installer version derives from the matching tag. It requires the compiler rather than silently omitting the installer. Set `ISCC` or pass `-Iscc` for a nonstandard compiler location. `-OutputDir` defaults to ignored `dist/`.

Outputs:

- `tt-pane-vVERSION-windows-x64-setup.exe`
- `tt-pane-vVERSION-windows-x64.zip`
- `SHA256SUMS.txt`

The test compiles the same installer script with a unique AppId and shortcut names, installs under `target/installer-smoke/`, checks the installed CLI and shortcuts, reinstalls with shortcuts deselected, and uninstalls. It verifies registration cleanup and preservation of user-owned files. It does not touch an existing tt-pane installation or start a tray instance. Logs remain under the test directory for diagnosis.

## Windows installer behavior

The stable AppId identifies upgrades. Files live in `%LOCALAPPDATA%\Programs\tt-pane`; no elevation or PATH changes are required. The desktop shortcut is opt-in, launch at login is managed in Settings, and launch-after-install is unchecked to avoid starting a second tray instance. Settings are kept outside the installation directory and are never deleted. Windows Restart Manager handles files held open during upgrades; users should quit tt-pane before updating. See [Inno Setup's CloseApplications behavior](https://jrsoftware.org/ishelp/topic_setup_closeapplications.htm).

Silent install: `setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TASKS=""`. Select the optional desktop shortcut with `/TASKS="desktopicon"`. There is no startup task: tt-pane registers its own sign-in entry (`HKCU\…\Run`) when it first runs, and setup removes a Startup-folder shortcut left by earlier installers. Uninstall removes the Run value. Silent installs do not launch tt-pane. Uninstall uses `unins000.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`.

## Windows in-app updates

Windows checks for stable releases at startup and every eight hours while running, and also supports manual checks from the tray and Settings. A single tray item changes from **Check for updates** to **Update and restart** when an update is available. Manual checks reset the eight-hour interval. Checks never overlap a check or installation; failures retain any previously discovered release and retry on the next interval without opening a window. `src/update.rs` fetches GitHub release metadata using Windows system curl, compares semantic versions, downloads the matching setup or ZIP plus `SHA256SUMS.txt`, and verifies the asset before handing off. Network calls run on a worker thread; only HTTPS downloads and redirects are permitted. Debug builds do not install updates.

The embedded `src/update_windows.ps1` reads a local JSON job using UTF-8, stages the portable executable when applicable, and acknowledges readiness before the app quits. It waits for the original process to exit, runs setup with the current directory (preserving shortcut choices), or replaces the portable executable with a backup, then starts the same executable with its absolute configuration path. It never asks Windows to reboot. Portable replacement failures attempt rollback; setup failures retain installer logs and restart the existing executable when possible. Failed handoffs retain the running app; post-exit failures display a native error dialog. Successful updates clean up staging files.

Run `powershell.exe -NoProfile -ExecutionPolicy Bypass -File packaging/windows/test-updater.ps1` to exercise portable and installer handoffs and invalid archives with disposable local executables. It verifies that replacement waits for exit, paths with spaces and Unicode survive restart, configuration is preserved, and portable backup files are cleaned up. This check also runs in Windows release CI. It does not download a release or update a real installation. Rust unit tests cover semantic version selection and checksum rejection.

## Build locally on macOS

Install Rust, Xcode with the Metal Toolchain, and Python 3. From the repository root:

```sh
cargo build --release --locked
packaging/macos/package.sh target/release v0.2.1
```

Use the current Cargo version for the tag. `package.sh` checks the binary's reported version, builds `tt-pane.app` (Info.plist from `packaging/macos/Info.plist`, `.icns` from the committed `packaging/macos/icons/`), signs it, and builds the disk image with a pinned [dmgbuild](https://dmgbuild.readthedocs.io) installed by hash into a throwaway venv (pip needs network access). The output is `dist/tt-pane-vVERSION-macos-arm64.dmg`; pass a platform name and output directory as the third and fourth arguments to change it. This follows tt-git's `crates/tt-git-gui/packaging/macos/`, including its macOS 26 Finder background workaround in `dmg.py`.

Signing uses the same environment as tt-git: `MACOS_CERT_P12` (base64 Developer ID Application .p12), `MACOS_CERT_PASSWORD`, optional `MACOS_SIGNING_IDENTITY`, and `MACOS_NOTARY_APPLE_ID`, `MACOS_NOTARY_TEAM_ID`, `MACOS_NOTARY_PASSWORD` for notarization and stapling. Without a certificate the app is signed ad hoc: it runs, but Gatekeeper blocks the first launch from a download, and macOS can forget its Accessibility grant after an update. Add these as repository secrets to sign releases.

## macOS app behavior

`tt-pane.app` (bundle id `net.thirtytech.tt-pane`) is an `LSUIElement` menu-bar app, so it never shows a Dock icon and needs no terminal. At launch it calls `AXIsProcessTrustedWithOptions` with the prompt option: macOS shows its Accessibility dialog and lists tt-pane in System Settings. While access is missing, Settings opens with an explanation. Opening the app again while it runs (Finder, Launchpad, Spotlight) opens Settings in the running instance rather than starting a second one. An error that stops startup is shown in an alert, because stderr is not visible from Finder.

## macOS installer artwork

`macos/dmg-background.svg` is the Finder window background (660×420 points), drawn in the tt-pane palette after tt-git's layout: icon and wordmark header, the app and Applications icons at (190, 200) and (470, 200) with light tags under Finder's black labels, and a snapping-window arrow between them. `macos/render-artwork.sh` (needs `rsvg-convert`) renders it to the checked-in `dmg-background.png` and `@2x`, and renders `assets/tt-pane.svg` into `macos/icons/`. Run it after changing either SVG. `macos/screenshots/installer.png` is the built disk image open in Finder on macOS 26.

## GitHub Actions

Run **Actions → Release → Run workflow** for downloadable artifacts without publishing. To publish, update the application and platform crate versions, Herdr plugin version, and lockfile, then push a matching `vMAJOR.MINOR.PATCH` tag. Tags such as `v0.2.1-rc.1` create prereleases. The version guard rejects mismatched tags. Only the publication job has `contents: write`; builds use read-only permissions. Windows installation smoke tests run before uploading artifacts.

The release includes the three platform archives, Windows setup executable, macOS disk image, `install.sh`, `install.ps1`, generated release notes, and `SHA256SUMS.txt`. Manual builds are identified by the Actions run/commit, while their filenames and binary versions retain the Cargo version. No workflow run, tag, or release is created by local packaging.

## Installer artwork

`windows/artwork/sidebar-source.png` is original artwork generated with the built-in image-generation tool. The exact prompt is in `windows/artwork/prompt.txt`. It uses the existing tt-pane navy, mint, pale ice, and teal palette and its tall-left/two-right window arrangement. The header icon is the existing `assets/tt-pane.png`.

`windows/render-artwork.ps1` mechanically resizes/converts these sources into checked-in 24-bit BMPs at 202×386 and 404×772 (sidebar), and 58×58 and 116×116 (header). Run it after replacing a source. Normal packaging uses the checked-in assets and does not need an image service or Python. Inno Setup selects the appropriate resolution for display scaling.

Actual installer screenshots are in `windows/screenshots/` and on the linked Notion task. They show the built installer, not design mockups.

Upgrades retain the existing installation directory and AppId. The installer removes the former executable and shortcuts and creates tt-pane shortcuts; existing user settings remain available. New installations use the tt-pane directory.
