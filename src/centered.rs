//! Explicit Center membership, shared by the tray and CLI under the cycle lock.
use crate::{
    geometry::{Action, Rect},
    platform,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Member {
    handle: u64,
    id: String,
    area: Rect,
    centered: Rect,
    last: Rect,
}

impl Member {
    fn matches(&self, live: &platform::WindowSnapshot) -> bool {
        self.id == live.id
            && self.area == live.area
            && crate::positioning::near(self.last, live.bounds)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Group {
    members: Vec<Member>,
}

impl Group {
    pub fn load(path: &Path) -> Result<Self> {
        match File::open(crate::positioning::sidecar(path, ".centered.json")) {
            Ok(file) => Ok(serde_json::from_reader(file.take(128 * 1024)).unwrap_or_default()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("Read centered window groups"),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.persist(crate::positioning::sidecar(path, ".centered.json"))
            .map_err(|e| e.error)
            .context("Save centered window groups")?;
        Ok(())
    }

    pub fn prune(&mut self) {
        self.members.retain(|m| {
            platform::window_snapshot(m.handle)
                .ok()
                .flatten()
                .is_some_and(|live| m.matches(&live))
        });
    }

    pub fn remove(&mut self, id: &str) {
        self.members.retain(|m| m.id != id);
    }

    pub fn register(&mut self, id: &str, bounds: Rect) {
        let Some(handle) = id.rsplit(':').next().and_then(|s| s.parse().ok()) else {
            return;
        };
        let Ok(Some(live)) = platform::window_snapshot(handle) else {
            return;
        };
        if live.id != id || !crate::positioning::near(live.bounds, bounds) {
            return;
        }
        let member = Member {
            handle,
            id: id.to_owned(),
            area: live.area,
            centered: bounds,
            last: live.bounds,
        };
        if let Some(old) = self.members.iter_mut().find(|m| m.id == id) {
            *old = member;
        } else {
            self.members.push(member);
        }
    }

    /// Stable circular order keeps every member reachable even when several peers
    /// overlap on one side or the ordinary 50% visibility filter would reject them.
    fn neighbor(&self, handle: u64, action: Action) -> Option<u64> {
        let selected = self.members.iter().find(|m| m.handle == handle)?;
        let peers: Vec<_> = self
            .members
            .iter()
            .filter(|m| m.area == selected.area)
            .collect();
        if peers.len() < 2 {
            return None;
        }
        let index = peers.iter().position(|m| m.handle == handle)?;
        let next = match action {
            Action::FocusLeft => (index + peers.len() - 1) % peers.len(),
            Action::FocusRight => (index + 1) % peers.len(),
            _ => return None,
        };
        Some(peers[next].handle)
    }

    pub fn focus(&mut self, handle: Option<u64>, action: Action) -> Result<bool> {
        let Some(handle) = handle.or_else(platform::foreground_handle) else {
            return Ok(false);
        };
        let Some(next) = self.neighbor(handle, action) else {
            return Ok(false);
        };
        let Some(live) = platform::window_snapshot(next)? else {
            return Ok(false);
        };
        let Some(member) = self.members.iter().find(|m| m.handle == next) else {
            return Ok(false);
        };
        if !member.matches(&live) {
            return Ok(false);
        }
        platform::focus_snapshot(&live)?;
        self.arrange(next)?;
        Ok(true)
    }

    pub fn refresh(&mut self) -> Result<()> {
        self.prune();
        if let Some(handle) = platform::foreground_handle() {
            self.arrange(handle)?;
        }
        Ok(())
    }

    pub fn arrange(&mut self, handle: u64) -> Result<()> {
        let Some(selected) = self.members.iter().find(|m| m.handle == handle).cloned() else {
            return Ok(());
        };
        let peers: Vec<_> = self
            .members
            .iter()
            .filter(|m| m.area == selected.area)
            .cloned()
            .collect();
        let index = peers.iter().position(|m| m.handle == handle).unwrap();
        // Move background members first, keeping their native Z order and focus.
        let mut moves: Vec<_> = peers
            .iter()
            .enumerate()
            .map(|(i, member)| {
                (
                    member,
                    target(
                        member,
                        &selected,
                        (i + peers.len() - index) % peers.len(),
                        peers.len(),
                    ),
                )
            })
            .collect();
        moves.sort_by_key(|(m, _)| m.handle == handle);
        let mut failure = None;
        for (member, target) in moves {
            if crate::positioning::near(member.last, target) {
                continue;
            }
            let result = (|| -> Result<Rect> {
                let live = platform::window_snapshot(member.handle)?
                    .context("Grouped window disappeared")?;
                anyhow::ensure!(member.matches(&live), "Grouped window was manually moved");
                let actual = platform::move_snapshot(&live, target)?;
                anyhow::ensure!(
                    crate::positioning::near(actual, target),
                    "Application rejected centered group placement"
                );
                Ok(actual)
            })();
            match result {
                Ok(actual) => {
                    if let Some(saved) = self.members.iter_mut().find(|m| m.id == member.id) {
                        saved.last = actual;
                    }
                }
                Err(error) => {
                    // Do not retry and fight a manual move or an app's constraints.
                    self.remove(&member.id);
                    failure = Some(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

fn target(member: &Member, selected: &Member, distance: usize, count: usize) -> Rect {
    let base = member.centered;
    if distance == 0 {
        return base;
    }
    let sign = if distance <= count / 2 { 1 } else { -1 };
    let area = member.area;
    Rect {
        x: (base.x + sign * (selected.centered.width / 4)).clamp(
            area.x,
            area.x.saturating_add((area.width - base.width).max(0)),
        ),
        y: (base.y + selected.centered.height / 4).clamp(
            area.y,
            area.y.saturating_add((area.height - base.height).max(0)),
        ),
        ..base
    }
}

/// Observe mouse/Alt+Tab focus as well as keybindings. Avoid blocking the UI on a
/// concurrent CLI command. The caller throttles this to ten observations/second.
pub fn observe(path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !crate::positioning::sidecar(&path, ".centered.json").exists() {
        return Ok(());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(crate::positioning::sidecar(&path, ".cycle.lock"))?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    let mut group = Group::load(&path)?;
    let before = group.clone();
    let result = group.refresh();
    if group != before {
        group.save(&path)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an interactive Windows desktop and briefly focuses disposable windows"]
    fn native_center_group_promotes_members_and_releases_manually_moved_windows() {
        platform::test_support::with_group_windows(|handles| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let config = crate::config::Config::default();
            for handle in handles {
                crate::positioning::apply(&config, &path, Action::Center, Some(handle)).unwrap();
            }
            let mut group = Group::load(&path).unwrap();
            assert_eq!(group.members.len(), 3);
            let cycle_before =
                std::fs::read(crate::positioning::sidecar(&path, ".cycle.json")).unwrap();
            // A regular native focus change is picked up by the tray observer.
            platform::focus_snapshot(&platform::window_snapshot(handles[0]).unwrap().unwrap())
                .unwrap();
            observe(&path).unwrap();
            group = Group::load(&path).unwrap();
            assert_eq!(
                platform::window_snapshot(handles[0])
                    .unwrap()
                    .unwrap()
                    .bounds,
                group.members[0].centered
            );
            // Hidden-by-overlap peers remain reachable with the default visibility setting.
            crate::positioning::apply(&config, &path, Action::FocusRight, Some(handles[0]))
                .unwrap();
            assert_eq!(platform::foreground_handle(), Some(handles[1]));
            crate::positioning::apply(&config, &path, Action::FocusLeft, Some(handles[1])).unwrap();
            assert_eq!(platform::foreground_handle(), Some(handles[0]));
            assert_eq!(
                std::fs::read(crate::positioning::sidecar(&path, ".cycle.json")).unwrap(),
                cycle_before
            );
            group = Group::load(&path).unwrap();
            for handle in handles {
                group.prune();
                assert_eq!(
                    group.members.len(),
                    3,
                    "Our moves must not evict group members"
                );
                group.arrange(handle).unwrap();
                let selected = group.members.iter().find(|m| m.handle == handle).unwrap();
                assert_eq!(
                    platform::window_snapshot(handle).unwrap().unwrap().bounds,
                    selected.centered
                );
                for peer in group.members.iter().filter(|m| m.handle != handle) {
                    assert_ne!(peer.last.x, peer.centered.x);
                    assert!(peer.last.y > peer.centered.y);
                    assert_eq!(peer.last.width, peer.centered.width);
                    assert_eq!(peer.last.height, peer.centered.height);
                }
            }
            let changed = platform::apply_with(Some(handles[0]), &mut |_, _, bounds| {
                Ok(Rect {
                    x: bounds.x + 20,
                    ..bounds
                })
            })
            .unwrap();
            group.prune();
            assert_eq!(group.members.len(), 2);
            group.arrange(handles[1]).unwrap();
            assert_eq!(
                platform::window_snapshot(handles[0])
                    .unwrap()
                    .unwrap()
                    .bounds,
                changed
            );
            group.save(&path).unwrap();
            // Center explicitly rejoins a manually moved window; another placement leaves.
            crate::positioning::apply(&config, &path, Action::Center, Some(handles[0])).unwrap();
            assert_eq!(Group::load(&path).unwrap().members.len(), 3);
            crate::positioning::apply(&config, &path, Action::Left, Some(handles[0])).unwrap();
            assert_eq!(Group::load(&path).unwrap().members.len(), 2);
            // Close/unavailable windows are pruned by the same live validation.
        });
    }
    fn member(handle: u64) -> Member {
        Member {
            handle,
            id: format!("win32:10:{handle}"),
            area: Rect {
                x: -1600,
                y: 20,
                width: 1600,
                height: 1000,
            },
            centered: Rect {
                x: -1200,
                y: 270,
                width: 800,
                height: 500,
            },
            last: Rect {
                x: -1200,
                y: 270,
                width: 800,
                height: 500,
            },
        }
    }

    #[test]
    fn three_windows_keep_sizes_and_offset_by_quarter_of_focused_size() {
        let m = member(1);
        assert_eq!(target(&m, &m, 0, 3), m.centered);
        assert_eq!(
            target(&m, &m, 1, 3),
            Rect {
                x: -1000,
                y: 395,
                ..m.centered
            }
        );
        assert_eq!(
            target(&m, &m, 2, 3),
            Rect {
                x: -1400,
                y: 395,
                ..m.centered
            }
        );
    }

    #[test]
    fn grown_windows_stay_inside_negative_origin_work_area() {
        let mut m = member(1);
        m.centered = Rect {
            x: -1520,
            y: 70,
            width: 1440,
            height: 900,
        };
        assert_eq!(
            target(&m, &m, 1, 3),
            Rect {
                x: -1440,
                y: 120,
                ..m.centered
            }
        );
        assert_eq!(
            target(&m, &m, 2, 3),
            Rect {
                x: -1600,
                y: 120,
                ..m.centered
            }
        );
    }

    #[test]
    fn navigation_wraps_within_monitor_and_preserves_vertical_navigation() {
        let mut other = member(9);
        other.area.x = 0;
        let group = Group {
            members: vec![member(1), other, member(2), member(3)],
        };
        assert_eq!(group.neighbor(1, Action::FocusLeft), Some(3));
        assert_eq!(group.neighbor(1, Action::FocusRight), Some(2));
        assert_eq!(group.neighbor(3, Action::FocusRight), Some(1));
        assert_eq!(group.neighbor(9, Action::FocusLeft), None);
        assert_eq!(group.neighbor(1, Action::FocusDown), None);
        assert_eq!(group.neighbor(1, Action::FocusUp), None);
        assert_eq!(group.neighbor(99, Action::FocusRight), None);
    }

    #[test]
    fn manual_move_resize_monitor_change_or_reused_handle_invalidates_membership() {
        let m = member(1);
        let live = platform::WindowSnapshot {
            handle: 1,
            id: m.id.clone(),
            area: m.area,
            bounds: m.last,
        };
        assert!(m.matches(&live));
        let mut changed = live.clone();
        changed.bounds.x += 3;
        assert!(!m.matches(&changed));
        changed = live.clone();
        changed.bounds.height += 3;
        assert!(!m.matches(&changed));
        changed = live.clone();
        changed.area.x = 0;
        assert!(!m.matches(&changed));
        changed = live.clone();
        changed.id = "win32:11:1".into();
        assert!(!m.matches(&changed));
        changed = live.clone();
        changed.bounds.x += 1;
        assert!(m.matches(&changed));
    }

    #[test]
    fn persistence_preserves_order_and_missing_or_corrupt_state_resets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(Group::load(&path).unwrap(), Group::default());
        let mut group = Group {
            members: vec![member(2), member(1), member(3)],
        };
        group.save(&path).unwrap();
        assert_eq!(Group::load(&path).unwrap(), group);
        group.remove("win32:10:1");
        group.save(&path).unwrap();
        assert_eq!(
            Group::load(&path).unwrap().neighbor(2, Action::FocusRight),
            Some(3)
        );
        std::fs::write(
            crate::positioning::sidecar(&path, ".centered.json"),
            "broken",
        )
        .unwrap();
        assert_eq!(Group::load(&path).unwrap(), Group::default());
    }
}
