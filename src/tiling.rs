//! Windows polling coordinator. Tray and CLI share state under the cycle lock.
use crate::{
    config::{Config, Tiling},
    geometry::{Action, Rect},
    platform,
    positioning::{near, sidecar},
};
use anyhow::{Context, Result, ensure};
use platform::tiling::{Dock, Dwindle};
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
    /// Floated on purpose with toggle-float. Snaps that rearrange tiles leave these
    /// alone, but retile windows floated by drags or earlier snaps.
    #[serde(default)]
    explicit: bool,
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
        // Snap behavior does not change existing layouts, so toggling it keeps them.
        let settings = &Tiling {
            snap_rearranges: false,
            ..settings.clone()
        };
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
                explicit: false,
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
        // Reflow after constraints reject a tile. A rejected window is relocated into
        // the largest other tile it has not tried (never splitting the focused window,
        // which is usually the one being resized); only when no tile remains is it
        // floated. Each attempt consumes a (window, tile) pair, so this converges
        // without a repeating polling fight with the application.
        let mut failure = None;
        for group in &mut self.groups {
            if !desktop.workspaces.contains(&group.workspace) {
                continue;
            }
            let mut live_windows = desktop.windows.clone();
            let mut originals = std::collections::HashMap::new();
            let mut tried: std::collections::HashMap<u64, Vec<u64>> =
                std::collections::HashMap::new();
            let members = group.members.len();
            for _ in 0..=members * members {
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
                            originals.entry(handle).or_insert(original);
                            if let Ok(actual) = &result {
                                live.snapshot.bounds = *actual;
                            }
                            rejected.push((handle, live.snapshot.id.clone(), result.err()));
                        }
                    }
                }
                if rejected.is_empty() {
                    break;
                }
                for (handle, id, error) in rejected {
                    group.tree.remove(handle);
                    let tried = tried.entry(handle).or_default();
                    let candidate = group
                        .tree
                        .layout(group.area, config.gaps)
                        .ok()
                        .and_then(|tiles| {
                            tiles
                                .into_iter()
                                .filter(|(other, _)| {
                                    Some(*other) != focused && !tried.contains(other)
                                })
                                .max_by_key(|(_, r)| i64::from(r.width) * i64::from(r.height))
                        })
                        .map(|(other, _)| other);
                    if let Some(other) = candidate
                        && group.tree.insert(handle, Some(other), group.area).is_ok()
                    {
                        tried.push(other);
                        continue;
                    }
                    // Nowhere left: restore its prior placement when possible and float.
                    if let Some(live) = live_windows
                        .iter_mut()
                        .find(|w| w.snapshot.handle == handle)
                        && let Some(original) = originals.get(&handle)
                        && error.is_none()
                    {
                        let _ = move_window(live, *original);
                    }
                    failure = Some(error.unwrap_or_else(|| {
                        anyhow::anyhow!("Window rejected its tile size; left floating")
                    }));
                    group.members.retain(|m| m.handle != handle);
                    self.floating.push(Floating {
                        workspace: group.workspace.clone(),
                        id,
                        explicit: false,
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
        // Without tiling, swaps still trade places with the nearest window.
        if let Some(direction) = swap_direction(action) {
            let desktop = platform::tiling_desktop()?;
            ensure!(
                !desktop.interacting,
                "Finish moving or resizing the window first"
            );
            let handle = handle
                .or_else(platform::foreground_handle)
                .context("Select a window first")?;
            swap_windows(&desktop, handle, direction, platform::move_tiling_window)?;
            return Ok(true);
        }
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
    let mut centered = centers.handles();
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
    if config.tiling.snap_rearranges
        && let Some(dock) = dock_for(action)
        && !config
            .tiling
            .float_apps
            .iter()
            .any(|app| app.eq_ignore_ascii_case(&live.app))
    {
        let before = state.clone();
        if dock_window(&mut state, config, &desktop, &centered, live, dock) {
            // Docking a centered member leaves its group, as another snap would.
            if centered.contains(&handle) {
                centers.remove(&live.snapshot.id);
                centers.refresh(config.center_layout)?;
                centers.save(path)?;
                centered.retain(|h| *h != handle);
            }
            let desktop = platform::tiling_desktop()?;
            let result = reflow(
                &mut state,
                before,
                config,
                &desktop,
                handle,
                &centered,
                platform::move_tiling_window,
            );
            state.save(path)?;
            result?;
            return Ok(true);
        }
    }
    let group_index = state.groups.iter().position(|g| {
        g.workspace == live.workspace && g.area == live.snapshot.area && g.tree.contains(handle)
    });
    if action == Action::ToggleFloat {
        if group_index.is_some() {
            state.float(&live.workspace, &live.snapshot.id);
            for floating in &mut state.floating {
                if floating.id == live.snapshot.id {
                    floating.explicit = true;
                }
            }
        } else {
            ensure!(
                !config
                    .tiling
                    .float_apps
                    .iter()
                    .any(|app| app.eq_ignore_ascii_case(&live.app)),
                "Remove this app from float_apps before retiling"
            );
            // Retiling a centered member leaves its group, as another snap would.
            if centered.contains(&handle) {
                centers.remove(&live.snapshot.id);
                centers.refresh(config.center_layout)?;
                centers.save(path)?;
                centered.retain(|h| *h != handle);
            }
            state.floating.retain(|f| f.id != live.snapshot.id);
        }
    } else if let Some(index) = group_index {
        let before = state.clone();
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
            Control::Pass => {
                state.save(path)?;
                return Ok(false);
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
            Control::Reflow => {
                let desktop = platform::tiling_desktop()?;
                let result = reflow(
                    &mut state,
                    before,
                    config,
                    &desktop,
                    handle,
                    &centered,
                    platform::move_tiling_window,
                );
                state.save(path)?;
                result?;
                return Ok(true);
            }
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

fn dock_for(action: Action) -> Option<Dock> {
    Some(match action {
        Action::Left => Dock::Left,
        Action::Right => Dock::Right,
        Action::Top => Dock::Top,
        Action::Bottom => Dock::Bottom,
        Action::TopLeft => Dock::TopLeft,
        Action::TopRight => Dock::TopRight,
        Action::BottomLeft => Dock::BottomLeft,
        Action::BottomRight => Dock::BottomRight,
        _ => return None,
    })
}

/// Dock a tiled or floating window into its monitor's tree at a snap position.
/// Windows on the same monitor that were floated by drags or earlier snaps rejoin
/// the layout first; explicit toggle-float choices, centered groups and float_apps
/// stay floating. Repeating the same snap grows its share through the configured
/// growth steps, then returns to half. Returns false, leaving state untouched, when
/// there is no other tile to arrange around it or the tree cannot hold the dock.
fn dock_window(
    state: &mut State,
    config: &Config,
    desktop: &platform::TilingDesktop,
    centered: &[u64],
    live: &platform::TilingWindow,
    dock: Dock,
) -> bool {
    let handle = live.snapshot.handle;
    let mut next = state.clone();
    let index = next
        .groups
        .iter()
        .position(|g| g.workspace == live.workspace && g.area == live.snapshot.area)
        .unwrap_or_else(|| {
            next.groups.push(Group {
                workspace: live.workspace.clone(),
                area: live.snapshot.area,
                tree: config.tiling.tree().expect("validated config"),
                members: Vec::new(),
                focused: None,
            });
            next.groups.len() - 1
        });
    let rejoin: Vec<_> = desktop
        .windows
        .iter()
        .filter(|w| {
            w.snapshot.handle != handle
                && w.workspace == live.workspace
                && w.snapshot.area == live.snapshot.area
                && !centered.contains(&w.snapshot.handle)
                && !config
                    .tiling
                    .float_apps
                    .iter()
                    .any(|app| app.eq_ignore_ascii_case(&w.app))
                && next
                    .floating
                    .iter()
                    .any(|f| f.id == w.snapshot.id && !f.explicit)
        })
        .collect();
    let group = &mut next.groups[index];
    for window in rejoin {
        let target = group.focused.filter(|id| group.tree.contains(*id));
        if group
            .tree
            .insert(window.snapshot.handle, target, group.area)
            .is_ok()
        {
            group.members.push(Member {
                handle: window.snapshot.handle,
                id: window.snapshot.id.clone(),
                last: window.snapshot.bounds,
            });
            next.floating.retain(|f| f.id != window.snapshot.id);
        }
    }
    let group = &mut next.groups[index];
    if !group.members.iter().any(|m| m.handle != handle) {
        return false;
    }
    let growth = config.growth;
    let shares: Vec<f64> = (0..=growth.steps)
        .map(|step| (0.5 * (1.0 + f64::from(growth.percent) / 100.0).powi(step as i32)).min(0.9))
        .collect();
    let share = group
        .tree
        .docked_share(handle, dock)
        .and_then(|current| shares.iter().position(|s| (s - current).abs() < 0.01))
        .map_or(shares[0], |step| shares[(step + 1) % shares.len()]);
    let mut tree = group.tree.clone();
    if tree.dock(handle, dock, share, group.area).is_err()
        || tree.layout(group.area, config.gaps).is_err()
    {
        return false;
    }
    group.tree = tree;
    group.focused = Some(handle);
    if !group.members.iter().any(|m| m.handle == handle) {
        group.members.push(Member {
            handle,
            id: live.snapshot.id.clone(),
            last: live.snapshot.bounds,
        });
    }
    next.floating.retain(|f| f.id != live.snapshot.id);
    *state = next;
    true
}

/// Apply a resize/swap. If a tile that was already tiled could not be relocated
/// and would be floated, undo the change instead: a layout edit never evicts.
fn reflow(
    state: &mut State,
    before: State,
    config: &Config,
    desktop: &platform::TilingDesktop,
    handle: u64,
    centered: &[u64],
    mut move_window: impl FnMut(&platform::TilingWindow, Rect) -> Result<Rect>,
) -> Result<()> {
    let result = state.reconcile(config, desktop, Some(handle), centered, &mut move_window);
    let evicted = state
        .floating
        .iter()
        .any(|f| !before.floating.iter().any(|b| b.id == f.id));
    if !evicted {
        return result;
    }
    *state = before;
    // Put the edited group back on the rectangles it accepted before the change.
    // Moving directly avoids re-observing our own moves as manual moves.
    if let Some(group) = state.groups.iter().find(|g| g.tree.contains(handle)) {
        for member in &group.members {
            if let Some(live) = desktop
                .windows
                .iter()
                .find(|w| w.snapshot.handle == member.handle)
            {
                let _ = move_window(live, member.last);
            }
        }
    }
    anyhow::bail!("Neighboring tile can't get any smaller")
}

fn swap_direction(action: Action) -> Option<Action> {
    match action {
        Action::SwapLeft => Some(Action::FocusLeft),
        Action::SwapDown => Some(Action::FocusDown),
        Action::SwapUp => Some(Action::FocusUp),
        Action::SwapRight => Some(Action::FocusRight),
        _ => None,
    }
}

/// Trade bounds with the nearest window in `direction` on the same virtual
/// desktop, across monitors too. A failed second move puts the first back.
fn swap_windows(
    desktop: &platform::TilingDesktop,
    handle: u64,
    direction: Action,
    mut move_window: impl FnMut(&platform::TilingWindow, Rect) -> Result<Rect>,
) -> Result<()> {
    let live = desktop
        .windows
        .iter()
        .find(|w| w.snapshot.handle == handle)
        .context("Select a restored application window")?;
    let peers: Vec<_> = desktop
        .windows
        .iter()
        .filter(|w| w.snapshot.handle != handle && w.workspace == live.workspace)
        .collect();
    let rects: Vec<_> = peers.iter().map(|w| w.snapshot.bounds).collect();
    let other = peers[crate::geometry::focus_neighbor(live.snapshot.bounds, &rects, direction)
        .context("No window in that direction")?];
    let (first, second) = (live.snapshot.bounds, other.snapshot.bounds);
    let moved = move_window(live, second)?;
    if let Err(error) = move_window(other, first) {
        let mut back = live.clone();
        back.snapshot.bounds = moved;
        back.snapshot.area = other.snapshot.area;
        let _ = move_window(&back, first);
        return Err(error);
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum Control {
    Focus(u64),
    /// Not handled by the tree; continue with regular positioning/focus.
    Pass,
    Float,
    Reflow,
}

/// Interpret tile commands without native calls so deep-tree navigation and
/// split editing can be tested independently of the user's desktop.
fn control(group: &mut Group, config: &Config, action: Action, handle: u64) -> Result<Control> {
    let direction = swap_direction(action).unwrap_or(action);
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
        let Some(neighbor) = crate::geometry::focus_neighbor(current, &rects, direction) else {
            // At the tree's edge, focus falls through to regular spatial navigation,
            // which reaches other monitors. Swaps stay within one tree.
            ensure!(action.focuses(), "No tile in that direction");
            return Ok(Control::Pass);
        };
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
            | Action::ResizeLeft
            | Action::ResizeRight
    ) {
        let result = match action {
            Action::ToggleSplit => {
                ensure!(
                    config.tiling.preserve_split,
                    "Toggle split requires preserve_split = true"
                );
                tree.toggle_split(handle)
            }
            // The divider moves the way the key points, whichever side the window is on.
            Action::ResizeUp | Action::ResizeDown | Action::ResizeLeft | Action::ResizeRight => {
                tree.move_boundary(
                    handle,
                    matches!(action, Action::ResizeUp | Action::ResizeDown),
                    if matches!(action, Action::ResizeUp | Action::ResizeLeft) {
                        -0.05
                    } else {
                        0.05
                    },
                    group.area,
                )
            }
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

    #[test]
    fn swaps_without_tiling_trade_places_with_the_nearest_window() {
        let at = |handle, x, y| {
            let mut w = window(handle);
            w.snapshot.bounds = Rect {
                x,
                y,
                width: 400,
                height: 300,
            };
            w
        };
        let mut far = at(4, -1100, 0);
        far.workspace = "desktop-b".into();
        let desktop = desktop(vec![at(1, -1100, 0), at(2, -600, 0), at(3, -600, 400), far]);
        let mut moves = Vec::new();
        swap_windows(&desktop, 1, Action::FocusRight, |w, r| {
            moves.push((w.snapshot.handle, r.x, r.y));
            Ok(r)
        })
        .unwrap();
        assert_eq!(moves, [(1, -600, 0), (2, -1100, 0)]);
        // Other virtual desktops and empty directions are never touched.
        assert!(swap_windows(&desktop, 1, Action::FocusLeft, |_, r| Ok(r)).is_err());
        // A refused second move puts the first window back.
        let mut moves = Vec::new();
        let result = swap_windows(&desktop, 2, Action::FocusDown, |w, r| {
            moves.push((w.snapshot.handle, r.y));
            anyhow::ensure!(w.snapshot.handle != 3, "refused");
            Ok(r)
        });
        assert!(result.is_err());
        assert_eq!(moves, [(2, 400), (3, 0), (2, 0)]);
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
        // Left/right move the same divider the way the key points from either side.
        let level = group.tree.layout(AREA, config.gaps).unwrap();
        control(group, &config, Action::ResizeLeft, 3).unwrap();
        let moved = group.tree.layout(AREA, config.gaps).unwrap();
        assert!(moved[0].1.width < level[0].1.width && moved[2].1.x < level[2].1.x);
        control(group, &config, Action::ResizeRight, 1).unwrap();
        assert_eq!(group.tree.layout(AREA, config.gaps).unwrap(), level);
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
        // Focus past the tree's edge passes through to regular (cross-monitor) focus.
        assert_eq!(
            control(group, &config, Action::FocusLeft, 1).unwrap(),
            Control::Pass
        );
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
    fn constraints_relocate_once_then_restore_float_and_reflow_without_retries() {
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
        assert_eq!(moves.iter().filter(|(id, _)| *id == 1).count(), 3); // request + one relocation + restore
    }

    // Returns the window's current rectangle (a rejection) when `refuses` says so.
    fn constrained(
        moves: &mut Vec<(u64, Rect)>,
        refuses: impl Fn(u64, Rect) -> bool + 'static,
    ) -> impl FnMut(&platform::TilingWindow, Rect) -> Result<Rect> + '_ {
        move |w, target| {
            moves.push((w.snapshot.handle, target));
            Ok(if refuses(w.snapshot.handle, target) {
                w.snapshot.bounds
            } else {
                target
            })
        }
    }

    #[test]
    fn squeezed_neighbor_relocates_into_largest_other_tile() {
        let mut state = State::default();
        let config = config();
        let mut desktop = desktop(vec![window(1), window(2)]);
        tick(&mut state, &mut desktop, &config, Some(2), &[]);
        desktop.windows.push(window(3));
        desktop.identities.push("test:3".into());
        tick(&mut state, &mut desktop, &config, Some(2), &[]);
        // Tree is [left=1, right=[top=2, bottom=3]]; growing 2 squeezes 3.
        for _ in 0..3 {
            control(&mut state.groups[0], &config, Action::SplitGrow, 2).unwrap();
        }
        let mut moves = Vec::new();
        state
            .reconcile(
                &config,
                &desktop,
                Some(2),
                &[],
                constrained(&mut moves, |id, r| id == 3 && r.height < 300),
            )
            .unwrap();
        assert!(state.floating.is_empty());
        let tiles = state.groups[0].tree.layout(AREA, config.gaps).unwrap();
        let rect = |id| tiles.iter().find(|(t, _)| *t == id).unwrap().1;
        assert!(rect(3).height >= 300);
        assert_eq!(rect(3).x, rect(1).x); // now shares the left column with 1
        assert!(rect(2).height > rect(1).height); // the grow was kept
    }

    #[test]
    fn resize_that_cannot_relocate_is_undone_instead_of_floating() {
        let mut state = State::default();
        let config = config();
        let mut desktop = desktop(vec![window(1), window(2)]);
        tick(&mut state, &mut desktop, &config, Some(1), &[]);
        let before = state.clone();
        for _ in 0..3 {
            control(&mut state.groups[0], &config, Action::SplitGrow, 1).unwrap();
        }
        let mut moves = Vec::new();
        let error = reflow(
            &mut state,
            before.clone(),
            &config,
            &desktop,
            1,
            &[],
            constrained(&mut moves, |id, r| id == 2 && r.width < 500),
        )
        .unwrap_err();
        assert!(error.to_string().contains("can't get any smaller"));
        assert_eq!(state, before);
        // Both tiles are put back on the rectangles they accepted before the grow.
        for member in &before.groups[0].members {
            assert_eq!(
                moves
                    .iter()
                    .rev()
                    .find(|(id, _)| *id == member.handle)
                    .unwrap()
                    .1,
                member.last
            );
        }
    }

    #[test]
    fn snaps_dock_into_the_layout_grow_on_repeat_and_skip_lone_windows() {
        let mut state = State::default();
        let mut config = config();
        config.tiling.snap_rearranges = true;
        let mut desktop = desktop(vec![window(1), window(2), window(3)]);
        tick(&mut state, &mut desktop, &config, Some(2), &[]);
        let work = config.gaps.work_area(AREA);
        let tile = |state: &State, id| {
            state.groups[0]
                .tree
                .layout(AREA, config.gaps)
                .unwrap()
                .into_iter()
                .find(|(t, _)| *t == id)
                .unwrap()
                .1
        };
        // Snap the window at `index` on `desktop`, as the action path does.
        fn snap(
            state: &mut State,
            config: &Config,
            desktop: &platform::TilingDesktop,
            centered: &[u64],
            index: usize,
            dock: Dock,
        ) -> bool {
            dock_window(
                state,
                config,
                desktop,
                centered,
                &desktop.windows[index],
                dock,
            )
        }
        // Ctrl+Alt+2 on tile 1: it takes the right half, the others share the left.
        assert!(snap(&mut state, &config, &desktop, &[], 0, Dock::Right));
        tick(&mut state, &mut desktop, &config, Some(1), &[]);
        let right = tile(&state, 1);
        assert_eq!(right.x + right.width, work.x + work.width);
        assert_eq!(right.height, work.height);
        for id in [2, 3] {
            assert!(tile(&state, id).x + tile(&state, id).width < right.x);
        }
        assert!(state.floating.is_empty());
        // Repeating grows through the growth steps, then returns to half.
        let mut widths = vec![right.width];
        for _ in 0..=config.growth.steps {
            assert!(snap(&mut state, &config, &desktop, &[], 0, Dock::Right));
            widths.push(tile(&state, 1).width);
        }
        assert!(widths.windows(2).take(3).all(|w| w[1] > w[0]));
        assert_eq!(widths.last(), widths.first());
        // A floating window docks into the tree too.
        state.float("desktop-a", "test:3");
        tick(&mut state, &mut desktop, &config, Some(1), &[]);
        assert!(!state.groups[0].tree.contains(3));
        assert!(snap(&mut state, &config, &desktop, &[], 2, Dock::Left));
        assert!(state.groups[0].tree.contains(3) && state.floating.is_empty());
        assert_eq!(tile(&state, 3).x, work.x);
        // One tile with drag-floated windows behind it: snapping it rearranges them
        // all, while explicit toggle-float choices and centered windows stay put.
        let mut state = State::default();
        let mut desktop = self::desktop((1..=5).map(window).collect());
        tick(&mut state, &mut desktop, &config, Some(1), &[4]);
        for id in ["test:2", "test:3", "test:5"] {
            state.float("desktop-a", id);
        }
        state
            .floating
            .iter_mut()
            .find(|f| f.id == "test:5")
            .unwrap()
            .explicit = true;
        tick(&mut state, &mut desktop, &config, Some(1), &[4]);
        assert_eq!(state.groups[0].members.len(), 1);
        assert!(snap(&mut state, &config, &desktop, &[4], 0, Dock::Right));
        let group = &state.groups[0];
        assert!([1, 2, 3].iter().all(|id| group.tree.contains(*id)));
        assert!(!group.tree.contains(4) && !group.tree.contains(5));
        assert_eq!(
            tile(&state, 1).x + tile(&state, 1).width,
            work.x + work.width
        );
        // A window alone on its monitor keeps the normal snap.
        let mut lone = State::default();
        let mut single = self::desktop(vec![window(9)]);
        tick(&mut lone, &mut single, &config, Some(9), &[]);
        let before = lone.clone();
        assert!(!snap(&mut lone, &config, &single, &[], 0, Dock::Right));
        assert_eq!(lone, before);
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
            // Swaps work without tiling, so they enumerate the live desktop.
            .filter(|a| *a != Action::ToggleTiling && swap_direction(*a).is_none())
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
