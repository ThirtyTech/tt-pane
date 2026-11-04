//! Windows release updates. Network and staging work runs off the UI thread.
use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const REPOSITORY: &str = "https://github.com/ThirtyTech/tt-pane/releases/download";
const LATEST: &str = "https://api.github.com/repos/ThirtyTech/tt-pane/releases/latest";

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

fn download(url: &str, path: &Path, max_bytes: u64) -> Result<()> {
    // Use the Windows system curl, ignoring user curlrc options and allowing
    // HTTPS redirects only (GitHub serves assets from its CDN).
    let curl = PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot is missing")?)
        .join("System32/curl.exe");
    let result = Command::new(curl)
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
        .creation_flags(0x08000000)
        .output()
        .context("Could not start Windows curl")?;
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

pub struct Prepared {
    directory: tempfile::TempDir,
}

pub fn prepare(release: &Release, config: &Path) -> Result<Prepared> {
    ensure!(
        !cfg!(debug_assertions),
        "Updating development builds is disabled; use an installed release"
    );
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
    let asset = directory.path().join(&name);
    let sums = directory.path().join("SHA256SUMS.txt");
    let base = format!("{REPOSITORY}/{}", release.tag);
    download(&format!("{base}/SHA256SUMS.txt"), &sums, 1024 * 1024)?;
    download(&format!("{base}/{name}"), &asset, 512 * 1024 * 1024)?;
    verify(&asset, &fs::read_to_string(sums)?, &name)?;
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

impl Prepared {
    /// Start the helper and wait for its readiness acknowledgement before quit.
    pub fn launch(self) -> Result<()> {
        let root = self.directory.path();
        let powershell =
            PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot is missing")?)
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut child = Command::new(powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(root.join("update.ps1"))
            .creation_flags(0x08000000)
            .spawn()
            .context("Could not start the update helper")?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            if root.join("ready").is_file() {
                // The helper owns cleanup after this handoff.
                let _ = self.directory.keep();
                return Ok(());
            }
            if child.try_wait()?.is_some() {
                bail!(
                    "Update helper failed: {}",
                    fs::read_to_string(root.join("error.txt"))
                        .unwrap_or_else(|_| "PowerShell could not initialize".into())
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
}
