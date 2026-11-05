//! Windows polling coordinator. Tray and CLI share state under the cycle lock.
use crate::{
    config::{Config, Tiling},
    geometry::{Action, Rect},
    platform,
    positioning::{near, sidecar},
};
use anyhow::{Context, Result, ensure};
use platform::tiling::Dwindle;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Member {
    handle: u64,
    id: String,
    last: Rect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Group {
    workspace: String,
    area: Rect,
    tree: Dwindle,
    members: Vec<Member>,
    focused: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Floating {
    workspace: String,
    id: String,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct State {
    settings: Option<Tiling>,
    override_enabled: Option<bool>,
    active: bool,
    groups: Vec<Group>,
    floating: Vec<Floating>,
}

impl State {
    fn load(path: &Path) -> Result<Self> {
        match File::open(sidecar(path, ".tiling.json")) {
            Ok(file) => {
                let state: Self =
                    serde_json::from_reader(file.take(1024 * 1024)).unwrap_or_default();
                if state.groups.len() > 128
                    || state.floating.len() > 4096
                    || state.groups.iter().any(|g| {
                        !g.tree.validate()
                            || g.members.len() > 1024
                            || !g
                                .tree
                                .layout(g.area, crate::geometry::SnapGaps { inner: 0, outer: 0 })
                                .is_ok_and(|tiles| tiles.len() == g.members.len())
                            || g.members.iter().any(|m| !g.tree.contains(m.handle))
                            || g.members
                                .iter()
                                .map(|m| m.handle)
                                .collect::<std::collections::HashSet<_>>()
                                .len()
                                != g.members.len()
                    })
                {
                    return Ok(Self::default());
                }
                Ok(state)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("Read tiling state"),
        }
    }

    fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.persist(sidecar(path, ".tiling.json"))
            .map_err(|e| e.error)
            .context("Save tiling state")?;
        Ok(())
    }

    fn configure(&mut self, settings: &Tiling) {
        if self.settings.as_ref() != Some(settings) {
            self.settings = Some(settings.clone());
            self.override_enabled = None;
            self.groups.clear();
            self.floating.clear();
        }
        let active = self.override_enabled.unwrap_or(settings.enabled);
        if self.active != active {
            self.groups.clear();
            self.floating.clear();
            self.active = active;
        }
    }

    fn float(&mut self, workspace: &str, id: &str) {
        if !self.floating.iter().any(|f| f.id == id) {
            self.floating.push(Floating {
                workspace: workspace.to_owned(),
                id: id.to_owned(),
            });
        }
        for group in &mut self.groups {
            if let Some(member) = group.members.iter().find(|m| m.id == id) {
                group.tree.remove(member.handle);
            }
            group.members.retain(|m| m.id != id);
        }
    }

    fn reconcile(
        &mut self,
        config: &Config,
        desktop: &platform::TilingDesktop,
        focused: Option<u64>,
        centered: &[u64],
        mut move_window: impl FnMut(&platform::TilingWindow, Rect) -> Result<Rect>,
    ) -> Result<()> {
        if !self.active || desktop.interacting {
            return Ok(());
        }
        self.floating.retain(|f| {
            !desktop.workspaces.contains(&f.workspace) || desktop.identities.contains(&f.id)
        });
        for floating in &mut self.floating {
            if let Some(live) = desktop
                .windows
                .iter()
                .find(|w| w.snapshot.id == floating.id)
            {
                floating.workspace = live.workspace.clone();
            }
        }
        let mut floats = Vec::new();
        for group in &mut self.groups {
            group.members.retain(|member| {
                let live = desktop
                    .windows
                    .iter()
                    .find(|w| w.snapshot.handle == member.handle);
                if let Some(live) = live {
                    // Evict stale membership when a window arrives from another
                    // desktop or its handle is reused by a different process.
                    if live.snapshot.id == member.id && live.workspace == group.workspace {
                        if !near(member.last, live.snapshot.bounds)
                            || live.snapshot.area != group.area
                        {
                            floats.push((live.workspace.clone(), member.id.clone()));
                        } else {
                            return true;
                        }
                    }
                } else if !desktop.workspaces.contains(&group.workspace) {
                    return true;
                }
                group.tree.remove(member.handle);
                false
            });
        }
        for (workspace, id) in floats {
            self.float(&workspace, &id);
        }
        for window in &desktop.windows {
            let live = &window.snapshot;
            if centered.contains(&live.handle)
                || config
                    .tiling
                    .float_apps
                    .iter()
                    .any(|app| app.eq_ignore_ascii_case(&window.app))
            {
                self.float(&window.workspace, &live.id);
            }
            if self.floating.iter().any(|f| f.id == live.id) {
                continue;
            }
            let index = self
                .groups
                .iter()
                .position(|g| g.workspace == window.workspace && g.area == live.area)
                .unwrap_or_else(|| {
                    self.groups.push(Group {
                        workspace: window.workspace.clone(),
                        area: live.area,
                        tree: config.tiling.tree().expect("validated config"),
                        members: Vec::new(),
                        focused: None,
                    });
                    self.groups.len() - 1
                });
            let group = &mut self.groups[index];
            if group.tree.contains(live.handle) {
                continue;
            }
            let target = focused
                .filter(|id| group.tree.contains(*id))
                .or(group.focused);
            if group.tree.insert(live.handle, target, live.area).is_ok() {
                group.members.push(Member {
                    handle: live.handle,
                    id: live.id.clone(),
                    last: live.bounds,
                });
            } else {
                self.float(&window.workspace, &live.id);
            }
        }
        // Reflow after constraints reject a tile. Each rejection removes one member,
        // so this converges without a repeating polling fight with the application.
        let mut failure = None;
        for group in &mut self.groups {
            if !desktop.workspaces.contains(&group.workspace) {
                continue;
            }
            let mut live_windows = desktop.windows.clone();
            for _ in 0..=group.members.len() {
                let targets = match group.tree.layout(group.area, config.gaps) {
                    Ok(targets) => targets,
                    Err(e) => {
                        failure = Some(anyhow::anyhow!("Cannot lay out tiles: {e:?}"));
                        break;
                    }
                };
                let mut rejected = Vec::new();
                for (handle, target) in targets {
                    let Some(live) = live_windows
                        .iter_mut()
                        .find(|w| w.snapshot.handle == handle)
                    else {
                        continue;
                    };
                    let original = live.snapshot.bounds;
                    let result = if near(original, target) {
                        Ok(original)
                    } else {
                        move_window(live, target)
                    };
                    match result {
                        Ok(actual) if near(actual, target) => {
                            live.snapshot.bounds = actual;
                            if let Some(member) =
                                group.members.iter_mut().find(|m| m.handle == handle)
                            {
                                member.last = actual;
                            }
                        }
                        result => {
                            // If an app constrained the requested rectangle, restore its
                            // prior placement when possible, then leave it floating.
                            if let Ok(actual) = &result {
                                live.snapshot.bounds = *actual;
                                let _ = move_window(live, original);
                            }
                            failure = Some(result.err().unwrap_or_else(|| {
                                anyhow::anyhow!("Window rejected its tile size; left floating")
                            }));
                            rejected.push((handle, live.snapshot.id.clone()));
                        }
                    }
                }
                if rejected.is_empty() {
                    break;
                }
                for (handle, id) in rejected {
                    group.tree.remove(handle);
                    group.members.retain(|m| m.handle != handle);
                    self.floating.push(Floating {
                        workspace: group.workspace.clone(),
                        id,
                    });
                }
            }
            if focused.is_some_and(|id| group.tree.contains(id)) {
                group.focused = focused;
            }
        }
        self.groups.retain(|g| !g.members.is_empty());
        failure.map_or(Ok(()), Err)
    }
}

fn lock(path: &Path) -> Result<File> {
    std::fs::create_dir_all(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sidecar(path, ".cycle.lock"))?)
}

pub fn observe(config: &Config, path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !config.tiling.enabled && !sidecar(&path, ".tiling.json").exists() {
        return Ok(());
    }
    let lock = lock(&path)?;
    if lock.try_lock().is_err() {
        return Ok(());
    }
    config.validate()?;
    let mut state = State::load(&path)?;
    let before = state.clone();
    state.configure(&config.tiling);
    let result = if state.active {
        let desktop = platform::tiling_desktop()?;
        let mut centers = crate::centered::Group::load(&path)?;
        centers.prune();
        let centered = centers.handles();
        state.reconcile(
            config,
            &desktop,
            platform::foreground_handle(),
            &centered,
            platform::move_tiling_window,
        )
    } else {
        Ok(())
    };
    if state != before {
        state.save(&path)?;
    }
    result
}

/// Saving Settings makes the saved toggle authoritative over CLI runtime overrides.
pub fn saved_settings(config: &Config, path: &Path) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !config.tiling.enabled && !sidecar(&path, ".tiling.json").exists() {
        return Ok(());
    }
    let lock = lock(&path)?;
    lock.lock()?;
    let mut state = State::load(&path)?;
    state.override_enabled = None;
    state.configure(&config.tiling);
    state.save(&path)
}

/// Called with the cycle lock already held. Returns true when fully handled.
pub(crate) fn action(
    config: &Config,
    path: &Path,
    action: Action,
    handle: Option<u64>,
) -> Result<bool> {
    if !config.tiling.enabled && !action.is_tiling() && !sidecar(path, ".tiling.json").exists() {
        return Ok(false);
    }
    let mut state = State::load(path)?;
    state.configure(&config.tiling);
    if action == Action::ToggleTiling {
        state.override_enabled = Some(!state.active);
        state.configure(&config.tiling);
        state.save(path)?;
        // The next tray tick enrolls existing windows; CLI also reconciles now.
        if !state.active {
            return Ok(true);
        }
    } else if !state.active {
        ensure!(!action.is_tiling(), "Enable experimental tiling first");
        return Ok(false);
    }
    let desktop = platform::tiling_desktop()?;
    ensure!(
        !desktop.interacting,
        "Finish moving or resizing the window first"
    );
    let handle = handle.or_else(platform::foreground_handle);
    let mut centers = crate::centered::Group::load(path)?;
    centers.prune();
    let centered = centers.handles();
    // Synchronize topology before interpreting the action, including CLI-only use.
    let result = state.reconcile(
        config,
        &desktop,
        handle,
        &centered,
        platform::move_tiling_window,
    );
    state.save(path)?;
    result?;
    if action == Action::ToggleTiling {
        return Ok(true);
    }
    let Some(live) = desktop
        .windows
        .iter()
        .find(|w| Some(w.snapshot.handle) == handle)
    else {
        ensure!(
            !action.is_tiling(),
            "Select an eligible restored application window"
        );
        return Ok(false);
    };
    let handle = live.snapshot.handle;
    let group_index = state.groups.iter().position(|g| {
        g.workspace == live.workspace && g.area == live.snapshot.area && g.tree.contains(handle)
    });
    if action == Action::ToggleFloat {
        if group_index.is_some() {
            state.float(&live.workspace, &live.snapshot.id);
        } else {
            ensure!(
                !centered.contains(&handle),
                "Use a snap shortcut to leave the centered group before retiling"
            );
            ensure!(
                !config
                    .tiling
                    .float_apps
                    .iter()
                    .any(|app| app.eq_ignore_ascii_case(&live.app)),
                "Remove this app from float_apps before retiling"
            );
            state.floating.retain(|f| f.id != live.snapshot.id);
        }
    } else if let Some(index) = group_index {
        let group = &mut state.groups[index];
        match control(group, config, action, handle)? {
            Control::Focus(next) => {
                let snapshot =
                    platform::window_snapshot(next)?.context("Tile is no longer available")?;
                platform::focus_snapshot(&snapshot)?;
                group.focused = Some(next);
                state.save(path)?;
                return Ok(true);
            }
            Control::Float => {
                state.float(&live.workspace, &live.snapshot.id);
                let desktop = platform::tiling_desktop()?;
                let result = state.reconcile(
                    config,
                    &desktop,
                    Some(handle),
                    &centered,
                    platform::move_tiling_window,
                );
                state.save(path)?;
                result?;
                return Ok(false);
            }
            Control::Reflow => {}
        }
    } else {
        ensure!(!action.is_tiling(), "Select a tiled window first");
        return Ok(false);
    }
    // Fresh observations avoid treating our just-completed moves as manual moves.
    let desktop = platform::tiling_desktop()?;
    let result = state.reconcile(
        config,
        &desktop,
        Some(handle),
        &centered,
        platform::move_tiling_window,
    );
    state.save(path)?;
    result?;
    Ok(true)
}

#[derive(Debug, PartialEq, Eq)]
enum Control {
    Focus(u64),
    Float,
    Reflow,
}

/// Interpret tile commands without native calls so deep-tree navigation and
/// split editing can be tested independently of the user's desktop.
fn control(group: &mut Group, config: &Config, action: Action, handle: u64) -> Result<Control> {
    let direction = match action {
        Action::SwapLeft => Action::FocusLeft,
        Action::SwapDown => Action::FocusDown,
        Action::SwapUp => Action::FocusUp,
        Action::SwapRight => Action::FocusRight,
        other => other,
    };
    let mut tree = group.tree.clone();
    if direction.focuses() {
        let tiles = tree
            .layout(group.area, config.gaps)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let current = tiles
            .iter()
            .find(|(id, _)| *id == handle)
            .context("Window is not tiled")?
            .1;
        let peers: Vec<_> = tiles.into_iter().filter(|(id, _)| *id != handle).collect();
        let rects: Vec<_> = peers.iter().map(|(_, r)| *r).collect();
        let neighbor = crate::geometry::focus_neighbor(current, &rects, direction)
            .context("No tile in that direction")?;
        let next = peers[neighbor].0;
        if action.focuses() {
            return Ok(Control::Focus(next));
        }
        tree.swap(handle, next)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    } else if matches!(
        action,
        Action::ToggleSplit
            | Action::SplitGrow
            | Action::SplitShrink
            | Action::ShrinkWidth
            | Action::ExpandWidth
            | Action::ResizeUp
            | Action::ResizeDown
    ) {
        let result = match action {
            Action::ToggleSplit => {
                ensure!(
                    config.tiling.preserve_split,
                    "Toggle split requires preserve_split = true"
                );
                tree.toggle_split(handle)
            }
            Action::ResizeUp | Action::ResizeDown => tree.move_boundary(
                handle,
                true,
                if action == Action::ResizeUp {
                    -0.05
                } else {
                    0.05
                },
                group.area,
            ),
            _ => tree.grow(
                handle,
                if matches!(action, Action::SplitShrink | Action::ShrinkWidth) {
                    -0.05
                } else {
                    0.05
                },
            ),
        };
        result.map_err(|e| anyhow::anyhow!("Cannot adjust split: {e:?}"))?;
    } else {
        return Ok(Control::Float);
    }
    tree.layout(group.area, config.gaps)
        .map_err(|e| anyhow::anyhow!("Split cannot fit this work area: {e:?}"))?;
    group.tree = tree;
    Ok(Control::Reflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect {
        x: -1200,
        y: 0,
        width: 1200,
        height: 800,
    };

    fn window(handle: u64) -> platform::TilingWindow {
        platform::TilingWindow {
            snapshot: platform::WindowSnapshot {
                handle,
                id: format!("test:{handle}"),
                area: AREA,
                bounds: Rect {
                    x: -1100,
                    y: 100,
                    width: 400,
                    height: 300,
                },
            },
            workspace: "desktop-a".into(),
            app: "app.exe".into(),
        }
    }

    fn desktop(windows: Vec<platform::TilingWindow>) -> platform::TilingDesktop {
        platform::TilingDesktop {
            identities: windows.iter().map(|w| w.snapshot.id.clone()).collect(),
            windows,
            workspaces: vec!["desktop-a".into()],
            interacting: false,
        }
    }

    fn config() -> Config {
        Config {
            tiling: Tiling {
                enabled: true,
                ..Tiling::default()
            },
            ..Config::default()
        }
    }

    fn tick(
        state: &mut State,
        desktop: &mut platform::TilingDesktop,
        config: &Config,
        focused: Option<u64>,
        centers: &[u64],
    ) -> usize {
        let mut moves = Vec::new();
        state.configure(&config.tiling);
        state
            .reconcile(config, desktop, focused, centers, |w, target| {
                moves.push((w.snapshot.handle, target));
                Ok(target)
            })
            .unwrap();
        for (id, target) in &moves {
            desktop
                .windows
                .iter_mut()
                .find(|w| w.snapshot.handle == *id)
                .unwrap()
                .snapshot
                .bounds = *target;
        }
        moves.len()
    }

    #[test]
    fn enable_existing_poll_stability_new_window_minimize_and_restore() {
        let mut state = State::default();
        let config = config();
        let mut desktop = desktop(vec![window(1), window(2)]);
        assert_eq!(tick(&mut state, &mut desktop, &config, Some(1), &[]), 2);
        assert_eq!(tick(&mut state, &mut desktop, &config, Some(1), &[]), 0);
        desktop.windows.push(window(3));
        desktop.identities.push("test:3".into());
        tick(&mut state, &mut desktop, &config, Some(3), &[]);
        let tiles = state.groups[0].tree.layout(AREA, config.gaps).unwrap();
        assert_eq!(
            tiles.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![1, 3, 2]
        );
        let minimized = desktop.windows.remove(1);
        tick(&mut state, &mut desktop, &config, Some(1), &[]);
        assert_eq!(state.groups[0].members.len(), 2);
        assert!(!state.groups[0].tree.contains(minimized.snapshot.handle));
        desktop.windows.push(minimized);
        tick(&mut state, &mut desktop, &config, Some(1), &[]);
        assert_eq!(state.groups[0].members.len(), 3);
        assert_eq!(tick(&mut state, &mut desktop, &config, Some(1), &[]), 0);
    }

    #[test]
    fn manual_moves_centers_and_app_rules_stay_floating() {
        let mut state = State::default();
        let mut config = config();
        config.tiling.float_apps = vec!["FLOAT.EXE".into()];
        let mut desktop = desktop(vec![window(1), window(2), window(3), window(4)]);
        desktop.windows[3].app = "float.exe".into();
        tick(&mut state, &mut desktop, &config, Some(1), &[3]);
        assert_eq!(state.groups[0].members.len(), 2);
        desktop.windows[0].snapshot.bounds.x += 50;
        let manual = desktop.windows[0].snapshot.bounds;
        tick(&mut state, &mut desktop, &config, Some(1), &[3]);
        assert_eq!(desktop.windows[0].snapshot.bounds, manual);
        assert_eq!(state.groups[0].members.len(), 1);
        assert_eq!(state.floating.len(), 3);
        assert_eq!(tick(&mut state, &mut desktop, &config, Some(1), &[3]), 0);
    }

    #[test]
    fn tile_commands_navigate_swap_resize_and_float() {
        let mut state = State::default();
        let config = config();
        let mut desktop = desktop(vec![window(1), window(2), window(3)]);
        tick(&mut state, &mut desktop, &config, Some(2), &[]);
        let group = &mut state.groups[0];
        let before = group.tree.layout(AREA, config.gaps).unwrap();
        // Tree is [left=1, right=[top=2, bottom=3]], independent of edge-snap visibility.
        assert_eq!(
            control(group, &config, Action::FocusDown, 2).unwrap(),
            Control::Focus(3)
        );
        assert_eq!(
            control(group, &config, Action::FocusUp, 3).unwrap(),
            Control::Focus(2)
        );
        assert_eq!(
            control(group, &config, Action::FocusLeft, 3).unwrap(),
            Control::Focus(1)
        );
        assert_eq!(
            control(group, &config, Action::FocusRight, 1).unwrap(),
            Control::Focus(2)
        );
        assert_eq!(
            control(group, &config, Action::SwapDown, 2).unwrap(),
            Control::Reflow
        );
        assert_eq!(
            group.tree.layout(AREA, config.gaps).unwrap()[1],
            (3, before[1].1)
        );
        control(group, &config, Action::SwapUp, 2).unwrap();
        assert_eq!(group.tree.layout(AREA, config.gaps).unwrap(), before);
        control(group, &config, Action::SplitGrow, 3).unwrap();
        assert!(group.tree.layout(AREA, config.gaps).unwrap()[2].1.height > before[2].1.height);
        control(group, &config, Action::SplitShrink, 3).unwrap();
        assert_eq!(group.tree.layout(AREA, config.gaps).unwrap(), before);
        control(group, &config, Action::ResizeUp, 3).unwrap();
        assert!(group.tree.layout(AREA, config.gaps).unwrap()[2].1.y < before[2].1.y);
        control(group, &config, Action::ResizeDown, 3).unwrap();
        control(group, &config, Action::ToggleSplit, 3).unwrap();
        assert_eq!(
            group.tree.layout(AREA, config.gaps).unwrap()[2].1.height,
            AREA.height - 40
        );
        for action in Action::POSITIONS.into_iter().chain([Action::FillGap]) {
            assert_eq!(control(group, &config, action, 3).unwrap(), Control::Float);
        }
        let before = group.clone();
        assert!(control(group, &config, Action::SwapLeft, 1).is_err());
        assert_eq!(*group, before);
    }

    #[test]
    fn desktops_monitors_and_dragging_are_isolated() {
        let mut state = State::default();
        let config = config();
        let mut a = desktop(vec![window(1), window(2)]);
        a.windows[1].snapshot.area.x = 0;
        tick(&mut state, &mut a, &config, Some(1), &[]);
        assert_eq!(state.groups.len(), 2);
        let mut b = desktop(vec![window(3)]);
        b.windows[0].workspace = "desktop-b".into();
        b.workspaces = vec!["desktop-b".into()];
        tick(&mut state, &mut b, &config, Some(3), &[]);
        assert_eq!(state.groups.len(), 3);
        assert_eq!(tick(&mut state, &mut a, &config, Some(1), &[]), 0);
        a.interacting = true;
        a.windows[0].snapshot.bounds.x += 100;
        let before = state.clone();
        assert_eq!(tick(&mut state, &mut a, &config, Some(1), &[]), 0);
        assert_eq!(state, before);
    }

    #[test]
    fn moving_to_another_desktop_and_handle_reuse_evict_old_membership() {
        let mut state = State::default();
        let config = config();
        let mut a = desktop(vec![window(1), window(2)]);
        tick(&mut state, &mut a, &config, Some(1), &[]);
        let mut b = desktop(vec![a.windows[0].clone()]);
        b.workspaces = vec!["desktop-b".into()];
        b.windows[0].workspace = "desktop-b".into();
        tick(&mut state, &mut b, &config, Some(1), &[]);
        assert_eq!(
            state.groups.iter().filter(|g| g.tree.contains(1)).count(),
            1
        );
        assert_eq!(
            state
                .groups
                .iter()
                .find(|g| g.tree.contains(1))
                .unwrap()
                .workspace,
            "desktop-b"
        );
        assert!(state.groups.iter().any(|g| g.tree.contains(2)));
        b.windows[0].snapshot.id = "new-process:1".into();
        b.identities = vec!["new-process:1".into()];
        tick(&mut state, &mut b, &config, Some(1), &[]);
        assert!(
            state
                .groups
                .iter()
                .flat_map(|g| &g.members)
                .all(|m| m.id != "test:1")
        );
        assert_eq!(
            state.groups.iter().filter(|g| g.tree.contains(1)).count(),
            1
        );
    }

    #[test]
    fn constraints_restore_then_float_and_reflow_without_retries() {
        let mut state = State::default();
        let config = config();
        state.configure(&config.tiling);
        let mut desktop = desktop(vec![window(1), window(2)]);
        let mut moves = Vec::new();
        assert!(
            state
                .reconcile(&config, &desktop, Some(1), &[], |w, target| {
                    moves.push((w.snapshot.handle, target));
                    Ok(if w.snapshot.handle == 1 {
                        w.snapshot.bounds
                    } else {
                        target
                    })
                })
                .is_err()
        );
        assert!(!state.groups[0].tree.contains(1));
        assert_eq!(state.groups[0].members.len(), 1);
        desktop.windows[1].snapshot.bounds = config.gaps.work_area(AREA);
        assert_eq!(tick(&mut state, &mut desktop, &config, Some(2), &[]), 0);
        assert_eq!(moves.iter().filter(|(id, _)| *id == 1).count(), 2); // request + restore
    }

    #[test]
    fn persistence_disable_and_saved_config_override() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut state = State::default();
        let mut config = config();
        let mut desktop = desktop(vec![window(1), window(2)]);
        tick(&mut state, &mut desktop, &config, Some(1), &[]);
        state.groups[0].tree.grow(2, 0.1).unwrap();
        state.save(&path).unwrap();
        assert_eq!(State::load(&path).unwrap(), state);
        config.tiling.enabled = false;
        assert_eq!(tick(&mut state, &mut desktop, &config, Some(1), &[]), 0);
        assert!(!state.active);
        assert!(state.groups.is_empty());
        state.override_enabled = Some(true);
        state.configure(&config.tiling);
        state.save(&path).unwrap();
        assert!(State::load(&path).unwrap().active);
        saved_settings(&config, &path).unwrap();
        assert!(!State::load(&path).unwrap().active);
        std::fs::write(sidecar(&path, ".tiling.json"), "invalid").unwrap();
        assert_eq!(State::load(&path).unwrap(), State::default());
    }

    #[test]
    fn observer_does_no_work_when_disabled_or_locked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        observe(&Config::default(), &path).unwrap();
        assert!(!sidecar(&path, ".cycle.lock").exists());
        let guard = lock(&path).unwrap();
        guard.lock().unwrap();
        observe(&config(), &path).unwrap();
        assert!(!sidecar(&path, ".tiling.json").exists());
    }

    #[test]
    fn disabled_tiling_actions_error_and_toggle_off_does_not_enumerate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for command in Action::TILING
            .into_iter()
            .filter(|a| *a != Action::ToggleTiling)
        {
            assert!(
                action(&Config::default(), &path, command, Some(0))
                    .unwrap_err()
                    .to_string()
                    .contains("Enable experimental tiling")
            );
        }
        let mut state = State::default();
        let config = config();
        state.configure(&config.tiling);
        state.save(&path).unwrap();
        assert!(action(&config, &path, Action::ToggleTiling, Some(0)).unwrap());
        assert!(!State::load(&path).unwrap().active);
    }

    #[test]
    #[ignore = "requires interactive Windows desktop; moves only disposable test windows"]
    fn native_reconcile_moves_only_disposable_windows_and_reads_back_bounds() {
        platform::test_support::with_group_windows(|handles| {
            let config = config();
            let mut state = State::default();
            state.configure(&config.tiling);
            let read = || {
                desktop(
                    handles
                        .iter()
                        .map(|handle| platform::TilingWindow {
                            snapshot: platform::window_snapshot(*handle).unwrap().unwrap(),
                            workspace: "desktop-a".into(),
                            app: "fixture.exe".into(),
                        })
                        .collect(),
                )
            };
            let before = read();
            state
                .reconcile(&config, &before, Some(handles[0]), &[], |w, rect| {
                    platform::move_snapshot(&w.snapshot, rect)
                })
                .unwrap();
            let after = read();
            for group in &state.groups {
                for (id, rect) in group.tree.layout(group.area, config.gaps).unwrap() {
                    assert!(near(
                        after
                            .windows
                            .iter()
                            .find(|w| w.snapshot.handle == id)
                            .unwrap()
                            .snapshot
                            .bounds,
                        rect
                    ));
                }
            }
            state
                .reconcile(&config, &after, Some(handles[0]), &[], |_, _| {
                    panic!("stable poll moved a window")
                })
                .unwrap();
            let selected = &after.windows[0].snapshot;
            platform::move_snapshot(
                selected,
                Rect {
                    x: selected.bounds.x + 30,
                    ..selected.bounds
                },
            )
            .unwrap();
            state
                .reconcile(&config, &read(), Some(handles[0]), &[], |w, rect| {
                    platform::move_snapshot(&w.snapshot, rect)
                })
                .unwrap();
            assert!(state.floating.iter().any(|f| f.id == selected.id));
        });
    }
}
