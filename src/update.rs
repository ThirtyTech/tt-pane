//! Release updates for Windows, macOS and Linux. Network and staging work runs
//! off the UI thread.
//!
//! Every platform checks the latest stable GitHub release, downloads the asset
//! matching how tt-pane is installed, verifies it against `SHA256SUMS.txt` and
//! hands off to a small helper that waits for tt-pane to exit and starts the new
//! version with the same `--config`.
//!
//! - Windows: the helper (`update_windows.ps1`) runs the setup or replaces the
//!   portable executable after exit, since Windows locks a running executable.
//! - macOS and Linux: a running executable or app bundle can be renamed, so the
//!   new version is staged next to the installed one, checked (`--version`, and
//!   `codesign` for the app bundle) and swapped in before tt-pane quits, keeping
//!   the old one as a backup. Any failure up to that point leaves tt-pane
//!   running. The helper (`update_unix.sh`) restores the backup if the new
//!   version does not start.
use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const REPOSITORY: &str = "https://github.com/ThirtyTech/tt-pane/releases/download";
const LATEST: &str = "https://api.github.com/repos/ThirtyTech/tt-pane/releases/latest";
#[cfg(unix)]
const MAX_ASSET: u64 = 512 * 1024 * 1024;

#[cfg(any(feature = "gui", test))]
const CHECK_INTERVAL: Duration = Duration::from_secs(8 * 60 * 60);

#[cfg(any(feature = "gui", test))]
#[derive(Default)]
pub(crate) struct CheckSchedule {
    last_started: Option<Instant>,
}

#[cfg(any(feature = "gui", test))]
impl CheckSchedule {
    pub(crate) fn due(&self, now: Instant, busy: bool) -> bool {
        !busy
            && self
                .last_started
                .is_none_or(|last| now.saturating_duration_since(last) >= CHECK_INTERVAL)
    }

    pub(crate) fn started(&mut self, now: Instant) {
        self.last_started = Some(now);
    }
}

/// Shown while an update installs; macOS ties Accessibility access to the
/// code signature, which an ad hoc signed build changes on every release.
pub const RESTART_NOTE: &str = if cfg!(target_os = "macos") {
    "Your saved settings will be kept. Window shortcuts remain available while downloading. macOS may ask for Accessibility access again after the restart."
} else {
    "Your saved settings will be kept. Window shortcuts remain available while downloading."
};

#[derive(Clone, Debug)]
pub struct Release {
    pub tag: String,
}

#[derive(Deserialize)]
struct Metadata {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn select_release(json: &[u8], current: &str) -> Result<Option<Release>> {
    let release: Metadata = serde_json::from_slice(json).context("Invalid release response")?;
    let tag = &release.tag_name;
    ensure!(
        tag.starts_with('v')
            && tag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)),
        "Invalid release tag"
    );
    let version = Version::parse(&tag[1..]).context("Invalid release version")?;
    ensure!(
        !release.draft && !release.prerelease && version.pre.is_empty(),
        "The latest release is not a stable release"
    );
    Ok(version
        .cmp_precedence(&Version::parse(current)?)
        .is_gt()
        .then_some(Release {
            tag: release.tag_name,
        }))
}

fn curl() -> Result<Command> {
    // Use the system curl, never one earlier on PATH.
    #[cfg(windows)]
    {
        let path = PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot is missing")?)
            .join("System32/curl.exe");
        let mut command = Command::new(path);
        command.creation_flags(0x08000000);
        Ok(command)
    }
    #[cfg(unix)]
    {
        let path = ["/usr/bin/curl", "/bin/curl"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .context("curl is required to update tt-pane; install it and try again")?;
        Ok(Command::new(path))
    }
}

fn download(url: &str, path: &Path, max_bytes: u64) -> Result<()> {
    // Ignore user curlrc options and allow HTTPS redirects only (GitHub serves
    // assets from its CDN).
    let result = curl()?
        .args([
            "--disable",
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--connect-timeout",
            "15",
            "--max-time",
            "600",
            "--max-filesize",
        ])
        .arg(max_bytes.to_string())
        .args(["--user-agent", "tt-pane-updater", "--output"])
        .arg(path)
        .arg(url)
        .output()
        .context("Could not start curl")?;
    ensure!(
        result.status.success(),
        "Download failed: {}",
        String::from_utf8_lossy(&result.stderr).trim()
    );
    ensure!(
        fs::metadata(path)?.len() <= max_bytes,
        "Download exceeds the size limit"
    );
    Ok(())
}

pub fn check() -> Result<Option<Release>> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("release.json");
    download(LATEST, &file, 2 * 1024 * 1024)?;
    select_release(&fs::read(file)?, env!("CARGO_PKG_VERSION"))
}

fn verify(path: &Path, manifest: &str, name: &str) -> Result<()> {
    let hashes: Vec<_> = manifest
        .lines()
        .filter_map(|line| {
            let (hash, file) = line.split_once(char::is_whitespace)?;
            (file.trim_start().trim_start_matches('*') == name).then_some(hash)
        })
        .collect();
    ensure!(
        hashes.len() == 1,
        "Missing or duplicate checksum for {name}"
    );
    let expected = hashes[0];
    ensure!(
        expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid checksum for {name}"
    );
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    ensure!(
        format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected),
        "Checksum mismatch for {name}; update canceled"
    );
    Ok(())
}

/// Download `name` from `release` into `directory` and verify its checksum.
fn fetch(release: &Release, name: &str, directory: &Path, max_bytes: u64) -> Result<PathBuf> {
    let asset = directory.join(name);
    let sums = directory.join("SHA256SUMS.txt");
    let base = format!("{REPOSITORY}/{}", release.tag);
    download(&format!("{base}/SHA256SUMS.txt"), &sums, 1024 * 1024)?;
    download(&format!("{base}/{name}"), &asset, max_bytes)?;
    verify(&asset, &fs::read_to_string(sums)?, name)?;
    Ok(asset)
}

// Install layouts. Pure path logic, tested on every platform.

/// The `.app` holding `exe` when it is `<name>.app/Contents/MacOS/<exe>`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.to_path_buf())
}

/// Release asset for a macOS or Linux install: the disk image for the app
/// bundle, the archive install.sh uses otherwise.
#[cfg_attr(windows, allow(dead_code))]
fn unix_asset(tag: &str, platform: &str, bundle: bool) -> String {
    format!(
        "tt-pane-{tag}-{platform}{}",
        if bundle { ".dmg" } else { ".tar.gz" }
    )
}

/// The icon install.sh puts in `<prefix>/share/icons` for `<prefix>/bin/tt-pane`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn install_sh_icon(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    (bin.file_name()? == "bin").then(|| {
        bin.parent()
            .unwrap_or(bin)
            .join("share/icons/hicolor/256x256/apps/tt-pane.png")
    })
}

/// Whether `tt-pane --version` output names `tag`'s version.
#[cfg_attr(windows, allow(dead_code))]
fn reports_version(output: &str, tag: &str) -> bool {
    let mut words = output.split_whitespace();
    let (Some("tt-pane"), Some(reported)) = (words.next(), words.next()) else {
        return false;
    };
    match (
        Version::parse(reported),
        Version::parse(tag.trim_start_matches('v')),
    ) {
        (Ok(a), Ok(b)) => a.cmp_precedence(&b).is_eq(),
        _ => false,
    }
}

/// A swapped-in update: the old version waits at `backup`, inside `backup_dir`
/// next to `target`, until the new one has started.
#[cfg_attr(windows, allow(dead_code))]
#[derive(Debug)]
struct Swapped {
    target: PathBuf,
    backup_dir: PathBuf,
    backup: PathBuf,
}

/// Replace `target` (a file or directory) with `staged`, keeping the old one.
/// Both must be on one filesystem. On error `target` is left as it was.
#[cfg_attr(windows, allow(dead_code))]
fn swap(staged: &Path, target: &Path) -> Result<Swapped> {
    let parent = target.parent().context("Install path has no parent")?;
    let name = target.file_name().context("Install path has no name")?;
    let backup_dir = tempfile::Builder::new()
        .prefix(".tt-pane-backup-")
        .tempdir_in(parent)
        .with_context(|| format!("{} is not writable", parent.display()))?
        .keep();
    let backup = backup_dir.join(name);
    if let Err(error) = fs::rename(target, &backup) {
        let _ = fs::remove_dir(&backup_dir);
        return Err(error).context("Could not move the installed version aside");
    }
    if let Err(error) = fs::rename(staged, target) {
        let restored = fs::rename(&backup, target);
        let _ = fs::remove_dir(&backup_dir);
        restored.context("Could not restore the installed version")?;
        return Err(error).context("Could not install the new version");
    }
    Ok(Swapped {
        target: target.into(),
        backup_dir,
        backup,
    })
}

#[cfg_attr(windows, allow(dead_code))]
impl Swapped {
    /// Put the old version back, discarding the new one.
    fn restore(self) -> Result<()> {
        let failed = self.backup_dir.join("failed");
        fs::rename(&self.target, &failed)?;
        fs::rename(&self.backup, &self.target)?;
        self.discard();
        Ok(())
    }

    fn discard(self) {
        let _ = fs::remove_dir_all(&self.backup_dir);
    }
}

pub struct Prepared {
    directory: tempfile::TempDir,
    #[cfg(unix)]
    swapped: Option<Swapped>,
    #[cfg(unix)]
    helper: Vec<std::ffi::OsString>,
}

fn development_check() -> Result<()> {
    ensure!(
        !cfg!(debug_assertions),
        "Updating development builds is disabled; use an installed release"
    );
    Ok(())
}

#[cfg(windows)]
pub fn prepare(release: &Release, config: &Path) -> Result<Prepared> {
    development_check()?;
    let exe = std::env::current_exe()?;
    let parent = exe.parent().context("Executable has no parent directory")?;
    // Fail while the application is still running if its directory is read-only.
    let _probe = tempfile::NamedTempFile::new_in(parent)
        .context("The application folder is not writable")?;
    let installed = parent.join("unins000.exe").is_file();
    let directory = tempfile::Builder::new()
        .prefix("tt-pane-update-")
        .tempdir()?;
    let name = format!(
        "tt-pane-{}-windows-x64{}",
        release.tag,
        if installed { "-setup.exe" } else { ".zip" }
    );
    let asset = fetch(release, &name, directory.path(), 512 * 1024 * 1024)?;
    let job = serde_json::json!({
        "pid": std::process::id(), "exe": exe, "config": std::path::absolute(config)?,
        "asset": asset, "installed": installed,
        "bundle": format!("tt-pane-{}-windows-x64", release.tag),
    });
    fs::write(directory.path().join("job.json"), serde_json::to_vec(&job)?)?;
    fs::write(
        directory.path().join("update.ps1"),
        include_str!("update_windows.ps1"),
    )?;
    Ok(Prepared { directory })
}

#[cfg(unix)]
fn platform() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("macos-arm64"),
        ("linux", "x86_64") => Ok("linux-x64"),
        (os, arch) => bail!("No tt-pane releases are published for {os} {arch}"),
    }
}

/// Run a system tool, failing with its error output.
#[cfg(unix)]
fn run(program: impl AsRef<Path>, args: &[&std::ffi::OsStr]) -> Result<std::process::Output> {
    let program = program.as_ref();
    let output = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .with_context(|| format!("Could not start {}", program.display()))?;
    ensure!(
        output.status.success(),
        "{} failed: {}",
        program.display(),
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}

/// A regular file (not a link) no larger than the asset limit.
#[cfg(unix)]
fn regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("The release has no {}", path.display()))?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_ASSET,
        "{} in the release is not a regular file",
        path.display()
    );
    Ok(())
}

/// A verified new version, staged in `staging`.
#[cfg(unix)]
struct Staged {
    path: PathBuf,
    /// install.sh's launcher icon, from the Linux archive.
    icon: Option<PathBuf>,
}

/// Download the release asset for this install into `directory`, verify it and
/// put the new executable (or app bundle) in `staging`. Changes no install.
#[cfg(unix)]
fn stage(release: &Release, bundle: bool, staging: &Path, directory: &Path) -> Result<Staged> {
    use std::os::unix::fs::PermissionsExt;
    let platform = platform()?;
    let asset = fetch(
        release,
        &unix_asset(&release.tag, platform, bundle),
        directory,
        MAX_ASSET,
    )?;
    let extracted = directory.join("extracted");
    fs::create_dir(&extracted)?;
    let (staged, staged_exe, icon) = if bundle {
        // Mount the disk image read-only and copy the signed bundle out of it.
        let mount = directory.join("mount");
        fs::create_dir(&mount)?;
        run(
            "/usr/bin/hdiutil",
            &[
                "attach".as_ref(),
                "-nobrowse".as_ref(),
                "-readonly".as_ref(),
                "-noautoopen".as_ref(),
                "-mountpoint".as_ref(),
                mount.as_os_str(),
                asset.as_os_str(),
            ],
        )?;
        let staged = staging.join("tt-pane.app");
        let copied = (|| {
            let app = mount.join("tt-pane.app");
            ensure!(
                fs::symlink_metadata(&app)?.is_dir(),
                "The disk image has no tt-pane.app"
            );
            run("/usr/bin/ditto", &[app.as_os_str(), staged.as_os_str()])?;
            Ok(())
        })();
        if run("/usr/bin/hdiutil", &["detach".as_ref(), mount.as_os_str()]).is_err() {
            let _ = run(
                "/usr/bin/hdiutil",
                &["detach".as_ref(), "-force".as_ref(), mount.as_os_str()],
            );
        }
        copied?;
        run(
            "/usr/bin/codesign",
            &["--verify".as_ref(), "--strict".as_ref(), staged.as_os_str()],
        )
        .context("The new tt-pane.app failed its signature check")?;
        let staged_exe = staged.join("Contents/MacOS/tt-pane");
        regular_file(&staged_exe)?;
        (staged, staged_exe, None)
    } else {
        let tar = ["/usr/bin/tar", "/bin/tar"]
            .into_iter()
            .find(|path| Path::new(path).is_file())
            .context("tar is required to update tt-pane")?;
        run(
            tar,
            &[
                "-xzf".as_ref(),
                asset.as_os_str(),
                "-C".as_ref(),
                extracted.as_os_str(),
            ],
        )?;
        let unpacked = extracted.join(format!("tt-pane-{}-{platform}", release.tag));
        let binary = unpacked.join("tt-pane");
        regular_file(&binary)?;
        let staged = staging.join("tt-pane");
        fs::copy(&binary, &staged)?;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
        let icon = unpacked.join("tt-pane.png");
        (
            staged.clone(),
            staged,
            regular_file(&icon).is_ok().then_some(icon),
        )
    };
    // Catches a wrong architecture, missing libraries or a mislabeled asset.
    let reported = run(&staged_exe, &["--version".as_ref()])
        .context("The new version does not run on this system")?;
    let reported = String::from_utf8_lossy(&reported.stdout);
    ensure!(
        reports_version(&reported, &release.tag),
        "The downloaded release reports {:?}, not {}",
        reported.trim(),
        release.tag
    );
    Ok(Staged { path: staged, icon })
}

#[cfg(unix)]
pub fn prepare(release: &Release, config: &Path) -> Result<Prepared> {
    use std::ffi::OsStr;
    development_check()?;
    let exe = fs::canonicalize(std::env::current_exe()?)?;
    let bundle = cfg!(target_os = "macos")
        .then(|| app_bundle(&exe))
        .flatten();
    let target = bundle.clone().unwrap_or_else(|| exe.clone());
    ensure!(
        !target.to_string_lossy().contains("/AppTranslocation/"),
        "macOS is running tt-pane from a temporary read-only copy. Move tt-pane.app to Applications, open it from there and update again."
    );
    let parent = target.parent().context("Install path has no parent")?;
    // Stage on the install folder's filesystem so the swap is a rename; this
    // also fails now, while tt-pane still runs, if the folder is read-only.
    let staging = tempfile::Builder::new()
        .prefix(".tt-pane-new-")
        .tempdir_in(parent)
        .with_context(|| {
            format!(
                "{} is not writable; update by reinstalling tt-pane (with sudo for a system-wide install)",
                parent.display()
            )
        })?;
    let directory = tempfile::Builder::new()
        .prefix("tt-pane-update-")
        .tempdir()?;
    let Staged { path: staged, icon } =
        stage(release, bundle.is_some(), staging.path(), directory.path())?;
    let swapped = swap(&staged, &target)?;
    drop(staging);
    // install.sh's launcher icon; best effort.
    if cfg!(target_os = "linux")
        && let (Some(icon), Some(installed)) = (icon, install_sh_icon(&exe))
        && installed.is_file()
    {
        let new = installed.with_extension("png.new");
        if fs::copy(icon, &new).is_ok() && fs::rename(&new, &installed).is_err() {
            let _ = fs::remove_file(new);
        }
    }
    let script = directory.path().join("update.sh");
    fs::write(&script, include_str!("update_unix.sh"))?;
    let helper = [
        script.as_os_str(),
        directory.path().as_os_str(),
        OsStr::new(&std::process::id().to_string()),
        swapped.target.as_os_str(),
        std::path::absolute(config)?.as_os_str(),
        swapped.backup_dir.as_os_str(),
        swapped.backup.as_os_str(),
        OsStr::new(if bundle.is_some() { "bundle" } else { "binary" }),
    ]
    .map(ToOwned::to_owned)
    .to_vec();
    Ok(Prepared {
        directory,
        swapped: Some(swapped),
        helper,
    })
}

impl Prepared {
    fn helper(&self) -> Result<Command> {
        #[cfg(windows)]
        {
            let powershell =
                PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot is missing")?)
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            let mut command = Command::new(powershell);
            command
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                ])
                .arg(self.directory.path().join("update.ps1"))
                .creation_flags(0x08000000);
            Ok(command)
        }
        #[cfg(unix)]
        {
            use std::{os::unix::process::CommandExt, process::Stdio};
            let mut command = Command::new("/bin/sh");
            // Its own process group, so it outlives tt-pane and the terminal
            // tt-pane may have been started from.
            command
                .args(&self.helper)
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            Ok(command)
        }
    }

    /// Start the helper and wait for its readiness acknowledgement before quit.
    /// On failure an already swapped-in version is rolled back.
    pub fn launch(self) -> Result<()> {
        let result = self.start_helper();
        #[cfg(unix)]
        let mut this = self;
        #[cfg(not(unix))]
        let this = self;
        match result {
            Ok(()) => {
                // The helper owns cleanup after this handoff.
                let _ = this.directory.keep();
                Ok(())
            }
            Err(error) => {
                #[cfg(unix)]
                if let Some(swapped) = this.swapped.take() {
                    swapped
                        .restore()
                        .context("Could not restore the previous version")?;
                }
                Err(error)
            }
        }
    }

    fn start_helper(&self) -> Result<()> {
        let root = self.directory.path();
        let mut child = self
            .helper()?
            .spawn()
            .context("Could not start the update helper")?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            if root.join("ready").is_file() {
                return Ok(());
            }
            if child.try_wait()?.is_some() {
                bail!(
                    "Update helper failed: {}",
                    fs::read_to_string(root.join("error.txt"))
                        .unwrap_or_else(|_| "the helper could not initialize".into())
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        bail!("Update helper did not become ready; the application was not closed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_checks_start_immediately_then_every_eight_hours_without_overlap() {
        let now = Instant::now();
        let mut schedule = CheckSchedule::default();
        assert!(schedule.due(now, false));
        assert!(!schedule.due(now, true));
        schedule.started(now);
        assert!(!schedule.due(now + CHECK_INTERVAL - Duration::from_secs(1), false));
        assert!(schedule.due(now + CHECK_INTERVAL, false));
        assert!(!schedule.due(now + CHECK_INTERVAL, true));
        // A manual check (successful or failed) starts a fresh interval.
        schedule.started(now + Duration::from_secs(60));
        assert!(!schedule.due(now + CHECK_INTERVAL, false));
        assert!(schedule.due(now + CHECK_INTERVAL + Duration::from_secs(60), false));
    }

    #[test]
    #[ignore = "contacts the public GitHub releases API"]
    fn live_release_check() {
        println!("Latest release check: {:?}", check().unwrap());
    }

    #[test]
    fn release_selection_uses_semver_and_never_downgrades() {
        let metadata = |tag: &str| {
            serde_json::to_vec(
                &serde_json::json!({"tag_name": tag, "draft": false, "prerelease": false}),
            )
            .unwrap()
        };
        assert!(
            select_release(&metadata("v0.10.0"), "0.9.0")
                .unwrap()
                .is_some()
        );
        assert!(
            select_release(&metadata("v0.2.1"), "0.2.1")
                .unwrap()
                .is_none()
        );
        assert!(
            select_release(&metadata("v0.2.0"), "0.2.1")
                .unwrap()
                .is_none()
        );
        assert!(select_release(&metadata("v0.3.0-rc.1"), "0.2.1").is_err());
        assert!(select_release(&metadata("v0.3.0/../../evil"), "0.2.1").is_err());
        assert!(select_release(b"{}", "0.2.1").is_err());
        assert!(
            select_release(&metadata("v0.2.1+build.2"), "0.2.1+build.1")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn checksum_requires_one_exact_matching_asset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.zip");
        fs::write(&path, b"abc").unwrap();
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let line = format!("{hash}  asset.zip\n");
        verify(&path, &line, "asset.zip").unwrap();
        verify(&path, &format!("{hash} *asset.zip\n"), "asset.zip").unwrap();
        assert!(verify(&path, &line, "other.zip").is_err());
        assert!(verify(&path, &line.repeat(2), "asset.zip").is_err());
        fs::write(&path, b"corrupted").unwrap();
        assert!(verify(&path, &line, "asset.zip").is_err());
    }

    #[test]
    fn install_layouts_pick_the_matching_release_asset() {
        let app = Path::new("/Applications/tt-pane.app/Contents/MacOS/tt-pane");
        assert_eq!(
            app_bundle(app).as_deref(),
            Some(Path::new("/Applications/tt-pane.app"))
        );
        // A renamed bundle is still updated in place.
        assert!(app_bundle(Path::new("/Applications/Panes.app/Contents/MacOS/tt-pane")).is_some());
        assert!(app_bundle(Path::new("/usr/local/bin/tt-pane")).is_none());
        assert!(app_bundle(Path::new("/opt/tt-pane/Contents/MacOS/tt-pane")).is_none());
        assert_eq!(
            unix_asset("v0.3.0", "macos-arm64", true),
            "tt-pane-v0.3.0-macos-arm64.dmg"
        );
        assert_eq!(
            unix_asset("v0.3.0", "macos-arm64", false),
            "tt-pane-v0.3.0-macos-arm64.tar.gz"
        );
        assert_eq!(
            unix_asset("v0.3.0", "linux-x64", false),
            "tt-pane-v0.3.0-linux-x64.tar.gz"
        );
        assert_eq!(
            install_sh_icon(Path::new("/home/me/.local/bin/tt-pane")).as_deref(),
            Some(Path::new(
                "/home/me/.local/share/icons/hicolor/256x256/apps/tt-pane.png"
            ))
        );
        assert!(install_sh_icon(Path::new("/home/me/tt-pane/tt-pane")).is_none());
    }

    #[test]
    fn staged_version_must_match_the_release() {
        assert!(reports_version("tt-pane 0.3.0\n", "v0.3.0"));
        assert!(!reports_version("tt-pane 0.2.9\n", "v0.3.0"));
        assert!(!reports_version("other 0.3.0\n", "v0.3.0"));
        assert!(!reports_version("", "v0.3.0"));
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "downloads the latest release from GitHub"]
    fn live_stage_latest_release() {
        // Stages (never installs) the latest release the way this install
        // would update: app bundle when TTPANE_TEST_BUNDLE is set on macOS.
        let json = tempfile::tempdir().unwrap();
        download(LATEST, &json.path().join("r.json"), 2 * 1024 * 1024).unwrap();
        let tag = select_release(&fs::read(json.path().join("r.json")).unwrap(), "0.0.0")
            .unwrap()
            .unwrap()
            .tag;
        let staging = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let bundle = cfg!(target_os = "macos") && std::env::var_os("TTPANE_TEST_BUNDLE").is_some();
        let staged = stage(
            &Release { tag: tag.clone() },
            bundle,
            staging.path(),
            directory.path(),
        )
        .unwrap();
        println!(
            "Staged {tag} at {} (icon: {:?})",
            staged.path.display(),
            staged.icon
        );
        assert!(staged.path.exists());
    }

    #[test]
    fn swap_keeps_a_backup_and_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        // A file, like a Linux or standalone macOS install.
        let target = dir.path().join("tt-pane");
        let staged = dir.path().join("staged");
        fs::write(&target, b"old").unwrap();
        fs::write(&staged, b"new").unwrap();
        let swapped = swap(&staged, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read(&swapped.backup).unwrap(), b"old");
        swapped.restore().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);

        // A directory, like tt-pane.app.
        let app = dir.path().join("tt-pane.app");
        let staged_app = dir.path().join("staged.app");
        fs::create_dir_all(app.join("Contents")).unwrap();
        fs::write(app.join("Contents/version"), b"old").unwrap();
        fs::create_dir_all(staged_app.join("Contents")).unwrap();
        fs::write(staged_app.join("Contents/version"), b"new").unwrap();
        let swapped = swap(&staged_app, &app).unwrap();
        assert_eq!(fs::read(app.join("Contents/version")).unwrap(), b"new");
        let backup_dir = swapped.backup_dir.clone();
        swapped.discard();
        assert!(!backup_dir.exists());
        assert!(!staged_app.exists());

        // A missing staged version leaves the install untouched.
        assert!(swap(&dir.path().join("missing"), &target).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
