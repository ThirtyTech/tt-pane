# Release packaging

`.github/workflows/release.yml` follows [tt-git's release workflow](https://github.com/ThirtyTech/tt-git/blob/main/.github/workflows/release.yml): version tags must match Cargo, each platform builds natively, and all successful builds are published together with SHA-256 checksums. Unlike tt-git's separate TUI/GUI jobs, tt-pane has one executable, so every platform must succeed before publication.

The initial matrix is Windows x64, Linux x64 (Ubuntu 22.04), and macOS Apple Silicon. Linux and macOS ship standalone archives; this task adds a native installer for Windows. Intel macOS, native Windows ARM64, Linux ARM64, signing, notarization, and Linux/macOS system installers are not included.

## Build locally on Windows

Install Rust, Visual Studio C++ Build Tools, and [Inno Setup 6 or newer](https://jrsoftware.org/isinfo.php). In PowerShell from the repository root:

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --target x86_64-pc-windows-msvc
./packaging/windows/package.ps1 -BinDir target/x86_64-pc-windows-msvc/release -Tag v0.1.0
./packaging/windows/test-installer.ps1 -BinDir target/x86_64-pc-windows-msvc/release -Tag v0.1.0
```

Use the current Cargo version for `-Tag`. Static CRT linking avoids requiring a separate Visual C++ runtime installation. `package.ps1` checks the PE architecture and the executable's reported version. It requires the compiler rather than silently omitting the installer. Set `ISCC` or pass `-Iscc` for a nonstandard compiler location. `-OutputDir` defaults to ignored `dist/`.

Outputs:

- `tt-pane-vVERSION-windows-x64-setup.exe`
- `tt-pane-vVERSION-windows-x64.zip`
- `SHA256SUMS.txt`

The test compiles the same installer script with a unique AppId and shortcut names, installs under `target/installer-smoke/`, checks the installed CLI and shortcuts, reinstalls with shortcuts deselected, and uninstalls. It verifies registration cleanup and preservation of user-owned files. It does not touch an existing tt-pane installation or start a tray instance. Logs remain under the test directory for diagnosis.

## Windows installer behavior

The stable AppId identifies upgrades. Files live in `%LOCALAPPDATA%\Programs\tt-pane`; no elevation or PATH changes are required. Desktop and login shortcuts are opt-in, and launch-after-install is unchecked to avoid starting a second tray instance. Settings are kept outside the installation directory and are never deleted. Windows Restart Manager handles files held open during upgrades; users should quit tt-pane before updating. See [Inno Setup's CloseApplications behavior](https://jrsoftware.org/ishelp/topic_setup_closeapplications.htm).

Silent install: `setup.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /TASKS=""`. Select optional tasks with `/TASKS="desktopicon,startup"`. Silent installs do not launch tt-pane. Uninstall uses `unins000.exe /VERYSILENT /SUPPRESSMSGBOXES /NORESTART`.

## GitHub Actions

Run **Actions → Release → Run workflow** for downloadable artifacts without publishing. To publish, update the package version and lockfile, then push a matching `vMAJOR.MINOR.PATCH` tag. Tags such as `v0.2.0-rc.1` create prereleases. The version guard rejects mismatched tags. Only the publication job has `contents: write`; builds use read-only permissions. Windows installation smoke tests run before uploading artifacts.

The release includes the three platform archives, Windows setup executable, generated release notes, and `SHA256SUMS.txt`. Manual builds are identified by the Actions run/commit, while their filenames and binary versions retain the Cargo version. No workflow run, tag, or release is created by local packaging.

## Installer artwork

`windows/artwork/sidebar-source.png` is original artwork generated with the built-in image-generation tool. The exact prompt is in `windows/artwork/prompt.txt`. It uses the existing tt-pane navy, mint, pale ice, and teal palette and its tall-left/two-right window arrangement. The header icon is the existing `assets/tt-pane.png`.

`windows/render-artwork.ps1` mechanically resizes/converts these sources into checked-in 24-bit BMPs at 202×386 and 404×772 (sidebar), and 58×58 and 116×116 (header). Run it after replacing a source. Normal packaging uses the checked-in assets and does not need an image service or Python. Inno Setup selects the appropriate resolution for display scaling.

Actual installer screenshots are in `windows/screenshots/` and on the linked Notion task. They show the built installer, not design mockups.

Upgrades retain the existing installation directory and AppId. The installer removes the former executable and shortcuts and creates tt-pane shortcuts; existing user settings remain available. New installations use the tt-pane directory.
