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

    /// Stable bounded order keeps covered peers reachable and lets navigation
    /// leave the group at either end.
    #[cfg(test)]
    fn neighbor(&self, handle: u64, action: Action) -> Option<u64> {
        self.neighbor_filtered(handle, action, None)
    }

    fn neighbor_filtered(
        &self,
        handle: u64,
        action: Action,
        eligible: Option<&[u64]>,
    ) -> Option<u64> {
        let selected = self.members.iter().find(|m| m.handle == handle)?;
        let peers: Vec<_> = self
            .members
            .iter()
            .filter(|m| m.area == selected.area)
            .filter(|m| {
                m.handle == handle || eligible.is_none_or(|handles| handles.contains(&m.handle))
            })
            .collect();
        if peers.len() < 2 {
            return None;
        }
        let index = peers.iter().position(|m| m.handle == handle)?;
        let next = match action {
            Action::FocusLeft => index.checked_sub(1)?,
            Action::FocusRight => index + 1,
            _ => return None,
        };
        peers.get(next).map(|m| m.handle)
    }

    pub fn focus(
        &mut self,
        handle: Option<u64>,
        action: Action,
        visibility: platform::FocusVisibility,
    ) -> Result<bool> {
        let Some(handle) = handle.or_else(platform::foreground_handle) else {
            return Ok(false);
        };
        if !matches!(action, Action::FocusLeft | Action::FocusRight)
            || !self.members.iter().any(|m| m.handle == handle)
        {
            return Ok(false);
        }
        let eligible = if visibility == platform::FocusVisibility::IncludeHidden {
            None
        } else {
            Some(platform::focus_eligible_handles(handle, visibility)?)
        };
        let Some(next) = self.neighbor_filtered(handle, action, eligible.as_deref()) else {
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

    pub fn exit_members(&self, handle: Option<u64>, action: Action) -> Vec<u64> {
        if !action.focuses() {
            return Vec::new();
        }
        let Some(selected) = handle
            .or_else(platform::foreground_handle)
            .and_then(|handle| self.members.iter().find(|m| m.handle == handle))
        else {
            return Vec::new();
        };
        self.members
            .iter()
            .filter(|m| m.area == selected.area)
            .map(|m| m.handle)
            .collect()
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
            .map(|(i, member)| (member, target(&peers, i, index)))
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

fn target(peers: &[Member], index: usize, selected_index: usize) -> Rect {
    let base = peers[index].centered;
    if index == selected_index {
        return base;
    }
    let selected = &peers[selected_index];
    let right = index > selected_index;
    let mut step_x = i64::from(selected.centered.width / 4);
    let mut step_y = i64::from(selected.centered.height / 4);
    // Share one step on each side. Limit it before placing any member so
    // clamping at the work-area edge cannot pile the outer windows together.
    for (i, member) in peers.iter().enumerate() {
        if i == selected_index || (i > selected_index) != right {
            continue;
        }
        let distance = i.abs_diff(selected_index) as i64;
        let rect = member.centered;
        let area = member.area;
        let room_x = if right {
            i64::from(area.x) + i64::from(area.width) - i64::from(rect.x) - i64::from(rect.width)
        } else {
            i64::from(rect.x) - i64::from(area.x)
        };
        let room_y =
            i64::from(area.y) + i64::from(area.height) - i64::from(rect.y) - i64::from(rect.height);
        step_x = step_x.min(room_x.max(0) / distance);
        step_y = step_y.min(room_y.max(0) / distance);
    }
    let distance = index.abs_diff(selected_index) as i64;
    let sign = if right { 1 } else { -1 };
    Rect {
        x: (i64::from(base.x) + sign * step_x * distance) as i32,
        y: (i64::from(base.y) + step_y * distance) as i32,
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
    #[ignore = "requires interactive Windows desktop; briefly focuses disposable windows"]
    fn native_taps_filter_middle_visibility_and_holds_reach_covered_windows() {
        platform::test_support::with_group_windows(|handles| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let mut config = crate::config::Config::default();
            config.focus.glow_enabled = false;
            let [left, middle, right] = handles;
            for (handle, action) in [
                (left, Action::Left),
                (middle, Action::Center),
                (right, Action::Right),
            ] {
                crate::positioning::apply(&config, &path, action, Some(handle)).unwrap();
            }
            let focus = |handle| {
                platform::focus_snapshot(&platform::window_snapshot(handle).unwrap().unwrap())
                    .unwrap();
            };
            // The left window covers half of the middle window. Taps should
            // visit it at 10%, but skip it at a higher configured threshold.
            for (percent, expected) in [(10, middle), (75, right)] {
                focus(middle);
                focus(left);
                config.focus.minimum_visible_percent = percent;
                crate::positioning::apply(&config, &path, Action::FocusRight, Some(left)).unwrap();
                assert_eq!(platform::foreground_handle(), Some(expected));
            }
            // Both halves above the middle cover it completely. Only a hold
            // can now reach it, even with the normal threshold at 0%.
            focus(middle);
            focus(right);
            focus(left);
            config.focus.minimum_visible_percent = 0;
            crate::positioning::apply(&config, &path, Action::FocusRight, Some(left)).unwrap();
            assert_eq!(platform::foreground_handle(), Some(right));
            for percent in [10, 50] {
                config.focus.minimum_visible_percent = percent;
                focus(middle);
                focus(right);
                focus(left);
                let visible = platform::focus_eligible_handles(
                    left,
                    platform::FocusVisibility::MoreThanPercent(percent),
                )
                .unwrap();
                let all = platform::focus_eligible_handles(
                    left,
                    platform::FocusVisibility::IncludeHidden,
                )
                .unwrap();
                assert!(!visible.contains(&middle));
                assert!(all.contains(&middle));
                crate::positioning::apply_gesture(
                    &config,
                    &path,
                    Action::FocusRight,
                    Some(left),
                    true,
                )
                .unwrap();
                let target = platform::foreground_handle().unwrap();
                // Other covered applications on the desktop can be nearer than
                // our fixture. The hold must reveal a threshold-excluded target.
                assert!(all.contains(&target));
                assert!(!visible.contains(&target));
            }
        });
    }

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
            // Holds keep hidden-by-overlap peers reachable.
            crate::positioning::apply_gesture(
                &config,
                &path,
                Action::FocusRight,
                Some(handles[0]),
                true,
            )
            .unwrap();
            assert_eq!(platform::foreground_handle(), Some(handles[1]));
            crate::positioning::apply_gesture(
                &config,
                &path,
                Action::FocusLeft,
                Some(handles[1]),
                true,
            )
            .unwrap();
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
        assert_eq!(target(&[m.clone(), m.clone(), m.clone()], 1, 1), m.centered);
        assert_eq!(
            target(&[m.clone(), m.clone(), m.clone()], 2, 1),
            Rect {
                x: -1000,
                y: 395,
                ..m.centered
            }
        );
        assert_eq!(
            target(&[m.clone(), m.clone(), m.clone()], 0, 1),
            Rect {
                x: -1400,
                y: 395,
                ..m.centered
            }
        );
    }

    #[test]
    fn repeated_right_shifts_visited_members_left_until_exit() {
        for count in [2, 3, 6] {
            let group = Group {
                members: (1..=count).map(member).collect(),
            };
            let mut handle = 1;
            for selected_index in 0..count as usize {
                assert_eq!(handle, selected_index as u64 + 1);
                for (index, peer) in group.members.iter().enumerate() {
                    let bounds = target(&group.members, index, selected_index);
                    assert_eq!(bounds.x.cmp(&peer.centered.x), index.cmp(&selected_index));
                }
                match group.neighbor(handle, Action::FocusRight) {
                    Some(next) => handle = next,
                    None => assert_eq!(selected_index, count as usize - 1),
                }
            }
            assert_eq!(group.neighbor(handle, Action::FocusRight), None);
        }
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
            target(&[m.clone(), m.clone(), m.clone()], 2, 1),
            Rect {
                x: -1440,
                y: 120,
                ..m.centered
            }
        );
        assert_eq!(
            target(&[m.clone(), m.clone(), m.clone()], 0, 1),
            Rect {
                x: -1600,
                y: 120,
                ..m.centered
            }
        );
    }

    #[test]
    fn same_side_members_stagger_and_compress_to_fit() {
        for count in [3, 4, 6, 12] {
            let peers: Vec<_> = (1..=count).map(member).collect();
            for selected in 0..peers.len() {
                let mut positions = Vec::new();
                for index in 0..peers.len() {
                    let rect = target(&peers, index, selected);
                    let area = peers[index].area;
                    assert!(rect.x >= area.x && rect.y >= area.y);
                    assert!(rect.x + rect.width <= area.x + area.width);
                    assert!(rect.y + rect.height <= area.y + area.height);
                    assert_eq!((rect.width, rect.height), (800, 500));
                    assert!(!positions.contains(&(rect.x, rect.y)));
                    positions.push((rect.x, rect.y));
                }
                assert!(positions.windows(2).all(|p| p[0].0 < p[1].0));
            }
        }
        let peers = vec![member(1), member(2), member(3)];
        assert_eq!(target(&peers, 1, 0).x, -1000);
        assert_eq!(target(&peers, 2, 0).x, -800);
        assert_eq!(target(&peers, 2, 0).y, 520);
        let peers: Vec<_> = (1..=6).map(member).collect();
        assert_eq!(target(&peers, 1, 0).x, -1120);
        assert_eq!(target(&peers, 5, 0).x, -800);
    }

    #[test]
    fn mixed_sizes_limit_spacing_on_only_the_crowded_side() {
        let mut peers: Vec<_> = (1..=5).map(member).collect();
        peers[4].centered = Rect {
            x: -1520,
            y: 70,
            width: 1440,
            height: 900,
        };
        // The outer large member has only 80px right and 50px below available.
        assert_eq!(target(&peers, 3, 2).x, -1160);
        assert_eq!(target(&peers, 3, 2).y, 295);
        assert_eq!(
            target(&peers, 4, 2),
            Rect {
                x: -1440,
                y: 120,
                ..peers[4].centered
            }
        );
        assert_eq!(target(&peers, 1, 2).x, -1400);
        assert_eq!(target(&peers, 0, 2).x, -1600);
        assert_eq!(target(&peers, 2, 2), peers[2].centered);
    }

    #[test]
    fn navigation_exits_at_both_ends_and_preserves_vertical_navigation() {
        let mut other = member(9);
        other.area.x = 0;
        let group = Group {
            members: vec![member(1), other, member(2), member(3)],
        };
        assert_eq!(group.neighbor(1, Action::FocusLeft), None);
        assert_eq!(group.neighbor(1, Action::FocusRight), Some(2));
        assert_eq!(group.neighbor(3, Action::FocusRight), None);
        assert_eq!(group.neighbor(9, Action::FocusLeft), None);
        assert_eq!(group.neighbor(1, Action::FocusDown), None);
        assert_eq!(group.neighbor(1, Action::FocusUp), None);
        assert_eq!(group.neighbor(99, Action::FocusRight), None);
        assert_eq!(
            group.exit_members(Some(1), Action::FocusDown),
            vec![1, 2, 3]
        );
        assert_eq!(group.exit_members(Some(3), Action::FocusUp), vec![1, 2, 3]);
        assert_eq!(group.exit_members(Some(9), Action::FocusUp), vec![9]);
        assert_eq!(
            group.exit_members(Some(1), Action::FocusRight),
            vec![1, 2, 3]
        );
        assert_eq!(group.neighbor(2, Action::FocusRight), Some(3));
        assert_eq!(group.neighbor(3, Action::FocusLeft), Some(2));
        assert_eq!(group.neighbor(2, Action::FocusLeft), Some(1));
        assert!(group.exit_members(Some(99), Action::FocusDown).is_empty());
    }

    #[test]
    fn group_navigation_skips_ineligible_peers_but_hold_keeps_them() {
        let group = Group {
            members: (1..=4).map(member).collect(),
        };
        assert_eq!(
            group.neighbor_filtered(1, Action::FocusRight, Some(&[3, 4])),
            Some(3)
        );
        assert_eq!(
            group.neighbor_filtered(4, Action::FocusLeft, Some(&[1, 2])),
            Some(2)
        );
        assert_eq!(
            group.neighbor_filtered(1, Action::FocusRight, Some(&[])),
            None
        );
        assert_eq!(
            group.neighbor_filtered(1, Action::FocusRight, None),
            Some(2)
        );
        assert_eq!(group.neighbor_filtered(4, Action::FocusLeft, None), Some(3));
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
