//! Keeps a second tray app from starting beside a running one. Two copies would
//! each grab the shortcuts, watch the same windows and draw their own dock, so
//! they fight over the desktop.
//!
//! The lock is an exclusive lock on a file beside the configuration. The OS
//! drops it when the process exits, however it exits, so a crash never leaves
//! a stale lock behind.

use anyhow::{Context, Result};
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
};

/// The lock file's name, in the configuration's directory.
const LOCK_FILE: &str = "tt-pane.lock";

/// Held for as long as this process is the running tray app.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    /// Claim the tray app for the configuration at `config`. `None` means
    /// another copy using that configuration is already running.
    pub fn acquire(config: &Path) -> Result<Option<Self>> {
        let directory = config
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(directory)
            .with_context(|| format!("Could not create {}", directory.display()))?;
        let path = directory.join(LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("Could not open {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self { _file: file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(error)) => {
                Err(error).with_context(|| format!("Could not lock {}", path.display()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_fails_until_first_is_released() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("config.toml");
        let first = InstanceLock::acquire(&config).unwrap();
        assert!(first.is_some());
        assert!(InstanceLock::acquire(&config).unwrap().is_none());
        drop(first);
        assert!(InstanceLock::acquire(&config).unwrap().is_some());
    }

    #[test]
    fn creates_missing_config_directory() {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("nested").join("config.toml");
        assert!(InstanceLock::acquire(&config).unwrap().is_some());
        assert!(directory.path().join("nested").join(LOCK_FILE).exists());
    }

    #[test]
    fn separate_configurations_lock_separately() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        let _first = InstanceLock::acquire(&one.path().join("config.toml")).unwrap();
        assert!(
            InstanceLock::acquire(&two.path().join("config.toml"))
                .unwrap()
                .is_some()
        );
    }
}
