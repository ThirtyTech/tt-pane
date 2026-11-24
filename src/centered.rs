//! Explicit Center membership, shared by the tray and CLI under the cycle lock.
// macOS has no centered groups; tiling only reads this state to skip members.
#![cfg_attr(target_os = "macos", allow(dead_code))]
use crate::{
    config::CenterLayout,
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
    /// Only a deliberate change releases a member. A window that is briefly
    /// unreadable, minimized, maximized, hidden or cloaked (another virtual
    /// desktop) keeps its place and rejoins once it is back where we left it.
    fn presence(
        &self,
        snapshot: Result<Option<platform::WindowSnapshot>>,
        identity: impl FnOnce() -> Option<String>,
    ) -> Presence {
        match snapshot {
            Ok(Some(live)) if live.id != self.id => Presence::Gone("window handle was reused"),
            Ok(Some(live)) if live.area != self.area => Presence::Gone("moved to another monitor"),
            Ok(Some(live)) if !crate::positioning::near(self.last, live.bounds) => {
                Presence::Gone("moved or resized outside tt-pane")
            }
            Ok(Some(live)) => Presence::Live(live),
            _ if identity().as_deref() == Some(self.id.as_str()) => Presence::Away,
            _ => Presence::Gone("window closed"),
        }
    }

    fn probe(&self) -> Presence {
        self.presence(platform::window_snapshot(self.handle), || {
            platform::window_identity(self.handle)
        })
    }
}

#[derive(Debug)]
enum Presence {
    Live(platform::WindowSnapshot),
    /// Still open, but not currently arrangeable.
    Away,
    Gone(&'static str),
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Group {
    members: Vec<Member>,
    /// Why members left since the last load, appended to a log on save.
    #[serde(skip)]
    released: Vec<String>,
}

impl Group {
    pub fn handles(&self) -> Vec<u64> {
        self.members.iter().map(|m| m.handle).collect()
    }
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
        // Diagnostics only: a failed log write never fails the group update.
        let _ = self.append_log(path);
        Ok(())
    }

    fn append_log(&self, path: &Path) -> std::io::Result<()> {
        const LIMIT: u64 = 64 * 1024;
        if self.released.is_empty() {
            return Ok(());
        }
        let log = crate::positioning::sidecar(path, ".centered.log");
        let full = std::fs::metadata(&log).is_ok_and(|m| m.len() > LIMIT);
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(!full)
            .write(true)
            .truncate(full)
            .open(log)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        for line in &self.released {
            writeln!(file, "{now} {line}")?;
        }
        Ok(())
    }

    fn release(&mut self, id: &str, reason: &str) {
        let before = self.members.len();
        self.members.retain(|m| m.id != id);
        if self.members.len() != before {
            self.released.push(format!("released {id}: {reason}"));
        }
    }

    /// Releases closed or manually moved members and keeps temporarily
    /// unavailable ones. Returns the members that can be arranged right now.
    fn live_members(&mut self) -> Vec<(Member, platform::WindowSnapshot)> {
        self.live_members_with(&Member::probe)
    }

    fn live_members_with(
        &mut self,
        probe: &dyn Fn(&Member) -> Presence,
    ) -> Vec<(Member, platform::WindowSnapshot)> {
        let mut live = Vec::new();
        for member in self.members.clone() {
            match probe(&member) {
                Presence::Live(snapshot) => live.push((member, snapshot)),
                Presence::Away => {}
                Presence::Gone(reason) => self.release(&member.id, reason),
            }
        }
        live
    }

    pub fn prune(&mut self) {
        self.live_members();
    }

    /// Positioning a member with any other action deliberately takes it out.
    pub fn remove(&mut self, id: &str) {
        self.release(id, "positioned with another action");
    }

    pub fn register(&mut self, id: &str, bounds: Rect) {
        // X11 can inspect windows for tiling, but groups also need directional focus.
        if !platform::centered_groups_supported() {
            return;
        }
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

    /// Stable circular order keeps both sides populated as focus rotates.
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
            Action::FocusLeft => (index + peers.len() - 1) % peers.len(),
            Action::FocusRight => (index + 1) % peers.len(),
            _ => return None,
        };
        peers.get(next).map(|m| m.handle)
    }

    pub fn focus(
        &mut self,
        handle: Option<u64>,
        action: Action,
        visibility: platform::FocusVisibility,
        layout: CenterLayout,
    ) -> Result<bool> {
        let Some(handle) = handle.or_else(platform::foreground_handle) else {
            return Ok(false);
        };
        if !matches!(action, Action::FocusLeft | Action::FocusRight)
            || !self.members.iter().any(|m| m.handle == handle)
        {
            return Ok(false);
        }
        // Minimized or cloaked members keep their place but are skipped.
        let live = self.live_members();
        let mut eligible: Vec<u64> = live.iter().map(|(m, _)| m.handle).collect();
        if visibility != platform::FocusVisibility::IncludeHidden {
            let visible = platform::focus_eligible_handles(handle, visibility)?;
            eligible.retain(|handle| visible.contains(handle));
        }
        let Some(next) = self.neighbor_filtered(handle, action, Some(&eligible)) else {
            // A singleton carousel stays put; only a hold (or J/K) leaves it.
            return Ok(true);
        };
        let Some((_, snapshot)) = live.iter().find(|(m, _)| m.handle == next) else {
            return Ok(false);
        };
        platform::focus_snapshot(snapshot)?;
        self.arrange(next, layout)?;
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

    pub fn refresh(&mut self, layout: CenterLayout) -> Result<()> {
        self.prune();
        if let Some(handle) = platform::foreground_handle() {
            self.arrange(handle, layout)?;
        }
        Ok(())
    }

    pub fn arrange(&mut self, handle: u64, layout: CenterLayout) -> Result<()> {
        self.arrange_with(handle, layout, &Member::probe)
    }

    fn arrange_with(
        &mut self,
        handle: u64,
        layout: CenterLayout,
        probe: &dyn Fn(&Member) -> Presence,
    ) -> Result<()> {
        // Only members that are on screen take a slot; away ones keep theirs.
        let live = self.live_members_with(probe);
        let selected = live
            .iter()
            .find(|(m, _)| m.handle == handle)
            .map(|(m, _)| m.area);
        let peers: Vec<_> = live
            .iter()
            .filter(|(m, _)| Some(m.area) == selected)
            .map(|(m, _)| m.clone())
            .collect();
        let selected_index = peers.iter().position(|m| m.handle == handle);
        // Move background members first, keeping their native Z order and focus.
        // Unrelated focus changes leave each monitor's current layout intact.
        let mut moves: Vec<_> = live
            .iter()
            .map(|(member, snapshot)| {
                let bounds = match (
                    selected_index,
                    peers.iter().position(|m| m.handle == member.handle),
                ) {
                    (Some(selected), Some(index)) => layout_target(&peers, index, selected, layout),
                    _ => member.last,
                };
                (member, snapshot, bounds)
            })
            .collect();
        moves.sort_by_key(|(m, _, _)| m.handle == handle);
        // With animated snapping on, only the selected member glides into its
        // slot; background members step aside instantly first.
        let animation = platform::snap_animation();
        let mut failure = None;
        for (member, snapshot, target) in moves {
            if crate::positioning::near(member.last, target) {
                continue;
            }
            let glide = animation.filter(|_| member.handle == handle);
            match platform::with_snap_animation(glide, || platform::move_snapshot(snapshot, target))
            {
                Ok(actual) if crate::positioning::near(actual, target) => {
                    if let Some(saved) = self.members.iter_mut().find(|m| m.id == member.id) {
                        saved.last = actual;
                    }
                }
                Ok(_) => {
                    // Do not retry and fight an app's size constraints.
                    self.release(&member.id, "application rejected the group placement");
                    failure = Some(anyhow::anyhow!(
                        "Application rejected centered group placement"
                    ));
                }
                Err(error) => {
                    // The window may have just been minimized or closed. Re-check
                    // instead of forgetting it, and release it only if it stayed
                    // put and still refused (for example an elevated window).
                    match probe(member) {
                        Presence::Away => {}
                        Presence::Gone(reason) => self.release(&member.id, reason),
                        Presence::Live(_) => {
                            self.release(&member.id, &format!("could not be moved: {error:#}"))
                        }
                    }
                    failure = Some(error);
                }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

// Signed distance around the ring: the next member is always on the right,
// the previous on the left (with two members, the sole peer sits on the right).
fn slot(index: usize, selected: usize, count: usize) -> i64 {
    let forward = (index + count - selected) % count;
    if forward <= count / 2 {
        forward as i64
    } else {
        forward as i64 - count as i64
    }
}

fn layout_target(peers: &[Member], index: usize, selected: usize, layout: CenterLayout) -> Rect {
    if layout == CenterLayout::Stack && index != selected {
        // Keep the same inactive size as Offset, but remove the displacement.
        return Action::Center.target(peers[index].area, peers[index].area);
    }
    target(peers, index, selected)
}

fn target(peers: &[Member], index: usize, selected_index: usize) -> Rect {
    if index == selected_index {
        return peers[index].centered;
    }
    let selected = &peers[selected_index];
    let base = Action::Center.target(peers[index].area, peers[index].area);
    let offset = slot(index, selected_index, peers.len());
    let mut step_x = i64::from(selected.centered.width / 4);
    let mut step_y = i64::from(selected.centered.height / 4);
    // Compress a whole side together to retain distinct positions at the edges.
    for (i, member) in peers.iter().enumerate() {
        let distance = slot(i, selected_index, peers.len());
        if distance == 0 || distance.signum() != offset.signum() {
            continue;
        }
        let rect = Action::Center.target(member.area, member.area);
        let area = member.area;
        let room_x = if distance > 0 {
            i64::from(area.x) + i64::from(area.width) - i64::from(rect.x) - i64::from(rect.width)
        } else {
            i64::from(rect.x) - i64::from(area.x)
        };
        let room_y =
            i64::from(area.y) + i64::from(area.height) - i64::from(rect.y) - i64::from(rect.height);
        step_x = step_x.min(room_x.max(0) / distance.abs());
        step_y = step_y.min(room_y.max(0) / distance.abs());
    }
    Rect {
        x: (i64::from(base.x) + step_x * offset) as i32,
        y: (i64::from(base.y) + step_y * offset.abs()) as i32,
        ..base
    }
}

/// Observe mouse/Alt+Tab focus as well as keybindings. Avoid blocking the UI on a
/// concurrent CLI command. The caller throttles this to ten observations/second.
pub fn observe(
    path: &Path,
    layout: CenterLayout,
    animation: Option<std::time::Duration>,
) -> Result<()> {
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
    let result = platform::with_snap_animation(animation, || group.refresh(layout));
    if group != before {
        group.save(&path)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    #[ignore = "requires interactive Windows desktop; briefly focuses disposable windows"]
    fn native_taps_filter_middle_visibility_and_holds_reach_covered_windows() {
        platform::test_support::with_group_windows(|handles| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let mut config = crate::config::Config::default();
            config.focus.glow_enabled = false;
            config.focus.visibility_filter_enabled = true;
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
            // Default taps skip the middle, but holding enters it.
            focus(middle);
            focus(left);
            crate::positioning::apply(&config, &path, Action::FocusRight, Some(left)).unwrap();
            assert_eq!(platform::foreground_handle(), Some(right));
            crate::positioning::apply_gesture(&config, &path, Action::FocusRight, Some(left), true)
                .unwrap();
            assert_eq!(platform::foreground_handle(), Some(middle));
            config.focus.include_middle_windows = true;
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
                // Disabling the filter also makes covered candidates reachable
                // with an ordinary press, without changing the saved percentage.
                config.focus.visibility_filter_enabled = false;
                focus(middle);
                focus(right);
                focus(left);
                crate::positioning::apply(&config, &path, Action::FocusRight, Some(left)).unwrap();
                let target = platform::foreground_handle().unwrap();
                assert!(all.contains(&target));
                assert!(!visible.contains(&target));
                config.focus.visibility_filter_enabled = true;
            }
        });
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "requires interactive Windows desktop; briefly focuses disposable windows"]
    fn native_hold_exits_middle_group_in_both_directions() {
        platform::test_support::with_focus_windows(|[left, first, second, right]| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let mut config = crate::config::Config::default();
            config.focus.glow_enabled = false;
            for (handle, action) in [
                (left, Action::Left),
                (right, Action::Right),
                (first, Action::Center),
                (second, Action::Center),
            ] {
                crate::positioning::apply(&config, &path, action, Some(handle)).unwrap();
            }
            for source in [first, second] {
                for (action, expected) in [(Action::FocusLeft, left), (Action::FocusRight, right)] {
                    platform::focus_snapshot(&platform::window_snapshot(source).unwrap().unwrap())
                        .unwrap();
                    observe(&path, CenterLayout::Offset, None).unwrap();
                    config.focus.minimum_visible_percent = 99;
                    crate::positioning::apply_gesture(&config, &path, action, Some(source), true)
                        .unwrap();
                    assert_eq!(platform::foreground_handle(), Some(expected));
                }
            }
            // Move a member manually so it becomes an ungrouped floating window.
            let live = platform::window_snapshot(first).unwrap().unwrap();
            platform::move_snapshot(
                &live,
                Rect {
                    x: live.bounds.x + 7,
                    ..live.bounds
                },
            )
            .unwrap();
            for (action, expected) in [(Action::FocusLeft, left), (Action::FocusRight, right)] {
                platform::focus_snapshot(&platform::window_snapshot(first).unwrap().unwrap())
                    .unwrap();
                crate::positioning::apply_gesture(&config, &path, action, Some(first), true)
                    .unwrap();
                assert_eq!(platform::foreground_handle(), Some(expected));
            }
        });
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "requires an interactive Windows desktop and briefly focuses disposable windows"]
    fn native_focus_glides_only_the_selected_member_into_the_center() {
        platform::test_support::with_group_windows(|handles| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let mut config = crate::config::Config::default();
            for handle in &handles[..2] {
                crate::positioning::apply(&config, &path, Action::Center, Some(*handle)).unwrap();
            }
            config.animation.enabled = true;
            config.animation.duration_ms = 300;
            let start = |handle| platform::window_snapshot(handle).unwrap().unwrap().bounds;
            let (selected, background) = (start(handles[0]), start(handles[1]));
            let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let watcher = {
                let done = done.clone();
                let watched = [handles[0], handles[1]];
                std::thread::spawn(move || {
                    let mut seen = [vec![], vec![]];
                    while !done.load(std::sync::atomic::Ordering::Relaxed) {
                        for (frames, handle) in seen.iter_mut().zip(watched) {
                            if let Ok(Some(w)) = platform::window_snapshot(handle)
                                && frames.last() != Some(&w.bounds)
                            {
                                frames.push(w.bounds);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    seen
                })
            };
            crate::positioning::apply(&config, &path, Action::FocusRight, Some(handles[1]))
                .unwrap();
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            let [entering, left_centre] = watcher.join().unwrap();
            assert_eq!(platform::foreground_handle(), Some(handles[0]));
            // The member being focused passes through intermediate frames...
            let end = start(handles[0]);
            assert_ne!(end, selected);
            let between = entering.iter().filter(|b| **b != selected && **b != end);
            assert!(between.count() >= 3, "{entering:?}");
            // ...while the one stepping aside jumps straight to its slot.
            let end = start(handles[1]);
            assert!(
                left_centre.iter().all(|b| *b == background || *b == end),
                "{left_centre:?}"
            );
        });
    }

    #[test]
    #[cfg(windows)]
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
            // Give one member a larger saved size, then switch away and back.
            for _ in 0..2 {
                crate::positioning::apply(&config, &path, Action::Center, Some(handles[0]))
                    .unwrap();
            }
            let cycle_before =
                std::fs::read(crate::positioning::sidecar(&path, ".cycle.json")).unwrap();
            // A regular native focus change is picked up by the tray observer.
            platform::focus_snapshot(&platform::window_snapshot(handles[0]).unwrap().unwrap())
                .unwrap();
            observe(&path, CenterLayout::Offset, None).unwrap();
            group = Group::load(&path).unwrap();
            assert_eq!(
                platform::window_snapshot(handles[0])
                    .unwrap()
                    .unwrap()
                    .bounds,
                group.members[0].centered
            );
            // Regular presses traverse group peers even with middle inclusion off.
            crate::positioning::apply_gesture(
                &config,
                &path,
                Action::FocusRight,
                Some(handles[0]),
                false,
            )
            .unwrap();
            assert_eq!(platform::foreground_handle(), Some(handles[1]));
            crate::positioning::apply_gesture(
                &config,
                &path,
                Action::FocusLeft,
                Some(handles[1]),
                false,
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
                group.arrange(handle, CenterLayout::Offset).unwrap();
                let selected = group.members.iter().find(|m| m.handle == handle).unwrap();
                assert_eq!(
                    platform::window_snapshot(handle).unwrap().unwrap().bounds,
                    selected.centered
                );
                for peer in group.members.iter().filter(|m| m.handle != handle) {
                    let base = Action::Center.target(peer.area, peer.area);
                    assert_ne!(peer.last.x, peer.centered.x);
                    assert!(peer.last.y > peer.centered.y);
                    assert_eq!(peer.last.width, base.width);
                    assert_eq!(peer.last.height, base.height);
                }
            }
            // Last-to-first wraps; it must not escape to a background window.
            group.save(&path).unwrap();
            crate::positioning::apply(&config, &path, Action::FocusRight, Some(handles[2]))
                .unwrap();
            assert_eq!(platform::foreground_handle(), Some(handles[0]));
            group = Group::load(&path).unwrap();
            let saved = group.members[0].centered;
            group.arrange(u64::MAX, CenterLayout::Offset).unwrap();
            assert_eq!(group.members[0].last, saved);
            group.arrange(handles[0], CenterLayout::Offset).unwrap();
            assert_eq!(group.members[0].last, saved);
            let changed = platform::apply_with(Some(handles[0]), &mut |_, _, bounds| {
                Ok(Rect {
                    x: bounds.x + 20,
                    ..bounds
                })
            })
            .unwrap();
            group.prune();
            assert_eq!(group.members.len(), 2);
            group.arrange(handles[1], CenterLayout::Offset).unwrap();
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
    /// Every member is open and exactly where it was left.
    fn on_screen(m: &Member) -> Presence {
        Presence::Live(platform::WindowSnapshot {
            handle: m.handle,
            id: m.id.clone(),
            area: m.area,
            bounds: m.last,
        })
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
    #[cfg(windows)]
    #[ignore = "requires interactive Windows desktop; briefly focuses disposable windows"]
    fn native_stacked_cycle_promotes_covered_members_and_switches_layout() {
        platform::test_support::with_group_windows(|handles| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.toml");
            let mut config = crate::config::Config::default();
            config.focus.glow_enabled = false;
            for handle in handles {
                crate::positioning::apply(&config, &path, Action::Center, Some(handle)).unwrap();
            }
            config.center_layout = CenterLayout::Stack;
            platform::focus_snapshot(&platform::window_snapshot(handles[0]).unwrap().unwrap())
                .unwrap();
            observe(&path, config.center_layout, None).unwrap();
            for turn in 0..6 {
                let selected = turn % handles.len();
                let group = Group::load(&path).unwrap();
                assert_eq!(group.members.len(), handles.len());
                for member in &group.members {
                    let live = platform::window_snapshot(member.handle).unwrap().unwrap();
                    assert_eq!(live.bounds, member.centered);
                }
                crate::positioning::apply_gesture(
                    &config,
                    &path,
                    Action::FocusRight,
                    Some(handles[selected]),
                    false,
                )
                .unwrap();
                assert_eq!(
                    platform::foreground_handle(),
                    Some(handles[(selected + 1) % handles.len()])
                );
            }
            observe(&path, CenterLayout::Offset, None).unwrap();
            let group = Group::load(&path).unwrap();
            assert_eq!(group.members.len(), handles.len());
            assert!(
                group
                    .members
                    .iter()
                    .any(|member| member.last != member.centered)
            );
        });
    }

    #[test]
    fn stacked_cycle_stays_centered_and_preserves_offset_sizing() {
        let mut group = Group {
            members: (1..=3).map(member).collect(),
            ..Default::default()
        };
        group.members[0].centered = Rect {
            x: -1520,
            y: 70,
            width: 1440,
            height: 900,
        };
        let mut handle = 1;
        for turn in 0..9 {
            let selected = turn % 3;
            assert_eq!(handle, selected as u64 + 1);
            for (index, peer) in group.members.iter().enumerate() {
                let stacked = layout_target(&group.members, index, selected, CenterLayout::Stack);
                let offset = layout_target(&group.members, index, selected, CenterLayout::Offset);
                assert_eq!(
                    (stacked.width, stacked.height),
                    (offset.width, offset.height)
                );
                assert_eq!(
                    stacked.x * 2 + stacked.width,
                    peer.area.x * 2 + peer.area.width
                );
                assert_eq!(
                    stacked.y * 2 + stacked.height,
                    peer.area.y * 2 + peer.area.height
                );
                if index == selected {
                    assert_eq!(stacked, peer.centered);
                }
            }
            handle = group.neighbor(handle, Action::FocusRight).unwrap();
        }
        assert_eq!(group.neighbor(1, Action::FocusLeft), Some(3));
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
    fn circular_layout_balances_sides_and_restores_saved_sizes() {
        for count in [2, 3, 6, 12] {
            let mut group = Group {
                members: (1..=count).map(member).collect(),
                ..Default::default()
            };
            group.members[0].centered = Rect {
                x: -1520,
                y: 70,
                width: 1440,
                height: 900,
            };
            let saved = group.members[0].centered;
            let mut handle = 1;
            for turn in 0..count * 3 {
                let selected = (turn % count) as usize;
                assert_eq!(handle, selected as u64 + 1);
                let mut positions = Vec::new();
                let mut left: i32 = 0;
                let mut right = 0;
                for (index, peer) in group.members.iter().enumerate() {
                    let rect = target(&group.members, index, selected);
                    assert!(rect.x >= peer.area.x && rect.y >= peer.area.y);
                    assert!(rect.x + rect.width <= peer.area.x + peer.area.width);
                    assert!(rect.y + rect.height <= peer.area.y + peer.area.height);
                    if index == selected {
                        assert_eq!(rect, peer.centered);
                    } else {
                        assert_eq!((rect.width, rect.height), (800, 500));
                        if rect.x < -1200 {
                            left += 1;
                        } else {
                            right += 1;
                        }
                    }
                    assert!(!positions.contains(&(rect.x, rect.y)));
                    positions.push((rect.x, rect.y));
                }
                assert!((left - right).abs() <= 1);
                handle = group.neighbor(handle, Action::FocusRight).unwrap();
            }
            assert_eq!(target(&group.members, 0, 0), saved);
            assert_eq!(group.neighbor(1, Action::FocusLeft), Some(count));
        }
    }

    #[test]
    fn unrelated_focus_preserves_grown_center_and_staggered_peers() {
        let mut m = member(1);
        m.centered = Rect {
            x: -1520,
            y: 70,
            width: 1440,
            height: 900,
        };
        m.last = m.centered;
        let mut group = Group {
            members: vec![m, member(2), member(3)],
            ..Default::default()
        };
        for index in 1..group.members.len() {
            group.members[index].last = target(&group.members, index, 0);
        }
        let before = group.clone();
        // An unregistered window can be snapped behind the center or on any screen.
        group
            .arrange_with(u64::MAX, CenterLayout::Offset, &on_screen)
            .unwrap();
        assert_eq!(group, before);
    }

    #[test]
    fn focusing_center_on_another_monitor_preserves_existing_group() {
        let mut first = member(1);
        first.centered = Rect {
            x: -1520,
            y: 70,
            width: 1440,
            height: 900,
        };
        first.last = first.centered;
        let mut other = member(3);
        other.area.x += 1600;
        other.centered.x += 1600;
        other.last = other.centered;
        let mut group = Group {
            members: vec![first, member(2), other],
            ..Default::default()
        };
        group.members[1].last = target(&group.members[..2], 1, 0);
        let before = group.clone();
        group
            .arrange_with(3, CenterLayout::Offset, &on_screen)
            .unwrap();
        assert_eq!(group, before);
    }

    #[test]
    fn inactive_size_and_offsets_follow_focused_size() {
        let mut peers: Vec<_> = (1..=3).map(member).collect();
        peers[1].centered = Rect {
            x: -1520,
            y: 70,
            width: 1440,
            height: 900,
        };
        assert_eq!(
            target(&peers, 0, 1),
            Rect {
                x: -1560,
                y: 495,
                width: 800,
                height: 500
            }
        );
        assert_eq!(
            target(&peers, 2, 1),
            Rect {
                x: -840,
                y: 495,
                width: 800,
                height: 500
            }
        );
        assert_eq!(target(&peers, 1, 0).width, 800);
        assert_eq!(target(&peers, 1, 1), peers[1].centered);
    }

    #[test]
    fn navigation_wraps_at_both_ends_and_preserves_vertical_navigation() {
        let mut other = member(9);
        other.area.x = 0;
        let group = Group {
            members: vec![member(1), other, member(2), member(3)],
            ..Default::default()
        };
        assert_eq!(group.neighbor(1, Action::FocusLeft), Some(3));
        assert_eq!(group.neighbor(1, Action::FocusRight), Some(2));
        assert_eq!(group.neighbor(3, Action::FocusRight), Some(1));
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
            ..Default::default()
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
        let same = || Some(m.id.clone());
        let check =
            |snapshot: &platform::WindowSnapshot| m.presence(Ok(Some(snapshot.clone())), same);
        assert!(matches!(check(&live), Presence::Live(_)));
        let mut changed = live.clone();
        changed.bounds.x += 3;
        assert!(matches!(check(&changed), Presence::Gone(_)));
        changed = live.clone();
        changed.bounds.height += 3;
        assert!(matches!(check(&changed), Presence::Gone(_)));
        changed = live.clone();
        changed.area.x = 0;
        assert!(matches!(check(&changed), Presence::Gone(_)));
        changed = live.clone();
        changed.id = "win32:11:1".into();
        assert!(matches!(check(&changed), Presence::Gone(_)));
        changed = live.clone();
        changed.bounds.x += 1;
        assert!(matches!(check(&changed), Presence::Live(_)));
    }

    #[test]
    fn briefly_unavailable_members_are_kept_and_closed_ones_released() {
        let m = member(1);
        // Minimized, maximized, hidden, cloaked, or a failed read: still open.
        assert!(matches!(
            m.presence(Ok(None), || Some(m.id.clone())),
            Presence::Away
        ));
        assert!(matches!(
            m.presence(Err(anyhow::anyhow!("transient")), || Some(m.id.clone())),
            Presence::Away
        ));
        // Closed, or the handle now belongs to another process.
        assert!(matches!(m.presence(Ok(None), || None), Presence::Gone(_)));
        assert!(matches!(
            m.presence(Ok(None), || Some("win32:11:1".into())),
            Presence::Gone(_)
        ));
    }

    #[test]
    fn releases_are_logged_with_their_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut group = Group {
            members: vec![member(1), member(2)],
            ..Default::default()
        };
        group.remove("win32:10:1");
        group.remove("win32:10:9");
        group.save(&path).unwrap();
        let log =
            std::fs::read_to_string(crate::positioning::sidecar(&path, ".centered.log")).unwrap();
        assert_eq!(log.lines().count(), 1);
        assert!(log.contains("released win32:10:1: positioned with another action"));
        // A reloaded group starts with nothing pending, so nothing repeats.
        Group::load(&path).unwrap().save(&path).unwrap();
        let again =
            std::fs::read_to_string(crate::positioning::sidecar(&path, ".centered.log")).unwrap();
        assert_eq!(again, log);
    }

    #[test]
    fn persistence_preserves_order_and_missing_or_corrupt_state_resets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(Group::load(&path).unwrap(), Group::default());
        let mut group = Group {
            members: vec![member(2), member(1), member(3)],
            ..Default::default()
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
