//! Experimental dock (Windows, and GNOME on X11 and Wayland): windows docked into a panel along one screen
//! edge show as live previews. Clicking one brings that window into the arena
//! (the work area the panel leaves), and focusing any other window sends it back.
//!
//! Membership lives in the `.dock.json` sidecar, guarded by the cycle lock like
//! centered groups, so the CLI and the tray process share it. The tray process
//! owns the panel and reconciles it with the saved state.
#![cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
use crate::geometry::{Action, Rect};
pub use crate::platform::dock::{DockEdge, DockFill, Toward};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, io::Write, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Member {
    handle: u64,
    id: String,
    /// Where the window was before it was docked; undocking puts it back.
    restore: Rect,
}

/// A window that was docked when tt-pane quit. Quitting puts it back on
/// screen; the next start docks it again, unless it closed or moved since.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Resumed {
    handle: u64,
    id: String,
    /// Where it was before it was docked, for undocking it after the restart.
    restore: Rect,
    /// Where quitting left it. A window found elsewhere was moved, and stays out.
    left: Rect,
}

/// A key pressed in dock pick mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickStep {
    /// The mode just started: keep the window that's out, or bring out the first.
    Start,
    Next,
    Previous,
    /// The numbered preview, from 0.
    Slot(usize),
    /// The neighbouring preview on screen, this way along the edge.
    Toward(Toward),
    /// Move the picked window one place this way along the edge.
    Move(Toward),
}

impl PickStep {
    /// An on-screen step as a step through the dock's order, which runs from
    /// the `fill` end.
    fn in_order(self, fill: DockFill) -> Self {
        match self {
            Self::Toward(toward) if toward.offset(fill) > 0 => Self::Next,
            Self::Toward(_) => Self::Previous,
            step => step,
        }
    }
}

/// The member a pick step brings out, counting from `current` (the window out
/// of the dock, or the one picked last). Steps wrap around the ends. `None`
/// when there's nothing to bring out: an empty dock or an unnumbered slot.
pub(crate) fn pick_target(members: &[u64], current: Option<u64>, step: PickStep) -> Option<u64> {
    let count = members.len();
    if count == 0 {
        return None;
    }
    let index = current.and_then(|c| members.iter().position(|&m| m == c));
    let chosen = match (step, index) {
        (PickStep::Slot(slot), _) => slot,
        (PickStep::Start, Some(index)) => index,
        (PickStep::Start | PickStep::Next, None) => 0,
        (PickStep::Previous, None) => count - 1,
        (PickStep::Next, Some(index)) => (index + 1) % count,
        (PickStep::Previous, Some(index)) => (index + count - 1) % count,
        // Moving keeps the pick; on-screen steps are put in order first.
        (PickStep::Move(_), _) => return current.filter(|c| members.contains(c)),
        (PickStep::Toward(_), _) => return None,
    };
    members.get(chosen).copied()
}

/// The window dock pick mode chose last and its place in the dock then, so
/// the next pick mode starts there instead of at the first preview.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LastPick {
    handle: u64,
    slot: usize,
}

#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
impl LastPick {
    fn of(members: &[u64], handle: u64) -> Option<Self> {
        let slot = members.iter().position(|&m| m == handle)?;
        Some(Self { handle, slot })
    }

    /// The member to start from: the same window if it's still docked, or
    /// else whichever now holds its place (the last one, if the dock shrank).
    fn resume(self, members: &[u64]) -> Option<u64> {
        if members.contains(&self.handle) {
            return Some(self.handle);
        }
        let last = members.len().checked_sub(1)?;
        members.get(self.slot.min(last)).copied()
    }
}

#[cfg(any(windows, target_os = "linux"))]
static LAST_PICK: std::sync::Mutex<Option<LastPick>> = std::sync::Mutex::new(None);

/// The window a pick step counts from: the one waiting to come out, the one
/// out of the dock, or, as pick mode starts, the one picked last time.
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
fn pick_current(
    members: &[u64],
    pending: Option<u64>,
    active: Option<u64>,
    last: Option<LastPick>,
    step: PickStep,
) -> Option<u64> {
    pending.or(active).or_else(|| {
        matches!(step, PickStep::Start)
            .then(|| last.and_then(|last| last.resume(members)))
            .flatten()
    })
}

/// A snapped window cut back so it stops short of the panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Squeezed {
    handle: u64,
    id: String,
    /// Its bounds without the panel, restored once the panel no longer covers them.
    original: Rect,
    /// Where tt-pane put it. A window that has moved since is left alone.
    reduced: Rect,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Dock {
    members: Vec<Member>,
    /// The member shown in the arena.
    active: Option<u64>,
    /// A member that was docked while it had focus. It stays docked until the
    /// foreground moves on, instead of popping straight back into the arena.
    #[serde(default)]
    settled: Option<u64>,
    /// What the docked windows take of the edge, as the tray last laid it out.
    /// Snaps in any process stop short of it.
    #[serde(default)]
    panel: Option<Rect>,
    #[serde(default)]
    squeezed: Vec<Squeezed>,
    /// Windows minimized out of the dock. Each goes back to where it was before
    /// it was docked as soon as it is restored, rather than to its parking spot.
    #[serde(default)]
    minimized: Vec<Member>,
    /// Windows docked when tt-pane last quit, docked again by the next start.
    #[serde(default)]
    resume: Vec<Resumed>,
}

/// Native operations the dock needs, faked in tests.
pub(crate) trait Native {
    /// The id and visible bounds of a window that still exists.
    fn identity(&self, handle: u64) -> Option<(String, Rect)>;
    fn mark(&self, handle: u64, docked: bool) -> Result<()>;
    /// Park a window at its own spot for `step`, its place in the dock, so
    /// docked windows don't cover each other (Chrome stops drawing a window
    /// others cover completely).
    fn park(&self, handle: u64, step: usize) -> Result<()>;
    /// Send a window in the arena back to the dock at its current size,
    /// gliding into `slot` when animation is on.
    fn stow(&self, handle: u64, slot: Option<Rect>, step: usize) -> Result<()>;
    /// Whether `bounds` is where the window at `step` is parked.
    fn parked(&self, bounds: Rect, step: usize) -> bool;
    /// Move a window to `target`, gliding from `from` when animation is on.
    fn place(&self, handle: u64, target: Rect, from: Option<Rect>) -> Result<()>;
    fn focus(&self, handle: u64) -> Result<()>;
    fn arena(&self) -> Result<Rect>;
    fn foreground(&self) -> Option<u64>;
    fn minimized(&self, handle: u64) -> bool;
    /// The member's preview slot, where an activated window starts its glide.
    fn slot(&self, handle: u64) -> Option<Rect>;
}

impl Dock {
    fn sidecar(path: &Path) -> std::path::PathBuf {
        crate::positioning::sidecar(path, ".dock.json")
    }

    pub fn exists(path: &Path) -> bool {
        Self::sidecar(path).exists()
    }

    pub fn load(path: &Path) -> Result<Self> {
        match File::open(Self::sidecar(path)) {
            Ok(file) => Ok(serde_json::from_reader(file.take(128 * 1024)).unwrap_or_default()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("Read docked windows"),
        }
    }

    /// Save the dock, or remove its file once it tracks nothing: no docked,
    /// minimized-out, squeezed or resuming windows.
    pub fn save(&self, path: &Path) -> Result<()> {
        if self.members.is_empty()
            && self.minimized.is_empty()
            && self.squeezed.is_empty()
            && self.resume.is_empty()
        {
            return match std::fs::remove_file(Self::sidecar(path)) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e).context("Remove docked windows"),
            };
        }
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.persist(Self::sidecar(path))
            .map_err(|e| e.error)
            .context("Save docked windows")?;
        Ok(())
    }

    pub fn handles(&self) -> Vec<u64> {
        self.members.iter().map(|m| m.handle).collect()
    }

    pub fn active(&self) -> Option<u64> {
        self.active
    }

    pub fn panel(&self) -> Option<Rect> {
        self.panel
    }

    /// Remember a snap that was cut back for the panel, so it grows back to
    /// `original` when the panel shrinks away from it.
    pub(crate) fn remember_squeeze(
        &mut self,
        handle: u64,
        id: &str,
        original: Rect,
        reduced: Rect,
    ) {
        self.squeezed.retain(|s| s.handle != handle);
        if original != reduced {
            self.squeezed.push(Squeezed {
                handle,
                id: id.to_owned(),
                original,
                reduced,
            });
        }
    }

    /// The panel moved, grew, shrank or went away: refit the windows squeezed
    /// beside it, and cut back `snapped` windows (handle, id, bounds) it now covers.
    pub(crate) fn refit(
        &mut self,
        native: &dyn Native,
        panel: Option<Rect>,
        edge: DockEdge,
        gap: u32,
        snapped: &[(u64, String, Rect)],
    ) {
        use crate::platform::dock::avoid;
        self.panel = panel;
        let mut kept = Vec::new();
        for mut squeezed in std::mem::take(&mut self.squeezed) {
            let Some((id, bounds)) = native.identity(squeezed.handle) else {
                continue;
            };
            // Closed, reused, or moved by the user since: no longer ours to resize.
            if id != squeezed.id || !crate::positioning::near(bounds, squeezed.reduced) {
                continue;
            }
            let target = panel.map_or(squeezed.original, |p| {
                avoid(squeezed.original, p, edge, gap)
            });
            if target != squeezed.reduced && native.place(squeezed.handle, target, None).is_err() {
                continue;
            }
            if target != squeezed.original {
                squeezed.reduced = target;
                kept.push(squeezed);
            }
        }
        if let Some(panel) = panel {
            for (handle, id, bounds) in snapped {
                if self.contains(*handle) || kept.iter().any(|s| s.handle == *handle) {
                    continue;
                }
                let target = avoid(*bounds, panel, edge, gap);
                if target != *bounds && native.place(*handle, target, None).is_ok() {
                    kept.push(Squeezed {
                        handle: *handle,
                        id: id.clone(),
                        original: *bounds,
                        reduced: target,
                    });
                }
            }
        }
        self.squeezed = kept;
    }

    pub fn contains(&self, handle: u64) -> bool {
        self.members.iter().any(|m| m.handle == handle)
    }

    /// Give a member place `to` in the dock, clamped to the end, moving the
    /// others along. Its preview and parking spot follow at the next
    /// observation. Returns whether the order changed.
    pub(crate) fn move_member(&mut self, handle: u64, to: usize) -> bool {
        let Some(member) = self.members.iter().find(|m| m.handle == handle).cloned() else {
            return false;
        };
        crate::platform::dock::reorder(&mut self.members, &member, to)
    }

    /// Move a member one place `offset` along the dock's order, stopping at the ends.
    pub(crate) fn shift_member(&mut self, handle: u64, offset: isize) -> bool {
        let Some(index) = self.members.iter().position(|m| m.handle == handle) else {
            return false;
        };
        let Some(to) = index.checked_add_signed(offset) else {
            return false;
        };
        to < self.members.len() && self.move_member(handle, to)
    }

    /// Forget members whose windows closed, or whose handle now names another window.
    fn prune(&mut self, native: &dyn Native) {
        self.members
            .retain(|m| native.identity(m.handle).is_some_and(|(id, _)| id == m.id));
        let members = self.handles();
        self.active = self.active.filter(|h| members.contains(h));
        self.settled = self.settled.filter(|h| members.contains(h));
    }

    /// Dock a window. `restore` overrides where undocking puts it back, such
    /// as where a window dragged onto the panel started its drag.
    fn dock(&mut self, native: &dyn Native, handle: u64, restore: Option<Rect>) -> Result<()> {
        let (id, bounds) = native
            .identity(handle)
            .context("The window to dock is no longer open")?;
        native.mark(handle, true)?;
        if let Err(error) = native.park(handle, self.step_of(handle)) {
            let _ = native.mark(handle, false);
            return Err(error).context("Move the window into the dock");
        }
        self.members.push(Member {
            handle,
            id,
            restore: restore.unwrap_or(bounds),
        });
        if native.foreground() == Some(handle) {
            self.settled = Some(handle);
        }
        Ok(())
    }

    /// A member's place in the dock, or the next place for a new one.
    fn step_of(&self, handle: u64) -> usize {
        self.members
            .iter()
            .position(|m| m.handle == handle)
            .unwrap_or(self.members.len())
    }

    fn undock(&mut self, native: &dyn Native, handle: u64) -> Result<()> {
        let Some(index) = self.members.iter().position(|m| m.handle == handle) else {
            return Ok(());
        };
        let member = self.members.remove(index);
        if self.active == Some(handle) {
            self.active = None;
        }
        if self.settled == Some(handle) {
            self.settled = None;
        }
        native.mark(handle, false)?;
        native.place(handle, member.restore, None)
    }

    /// Take a member out without moving it, because the user positioned it.
    fn release(&mut self, native: &dyn Native, handle: u64) -> Result<()> {
        let before = self.members.len();
        self.members.retain(|m| m.handle != handle);
        if self.members.len() == before {
            return Ok(());
        }
        if self.active == Some(handle) {
            self.active = None;
        }
        if self.settled == Some(handle) {
            self.settled = None;
        }
        native.mark(handle, false)
    }

    pub(crate) fn toggle(&mut self, native: &dyn Native, handle: u64) -> Result<()> {
        self.prune(native);
        if self.contains(handle) {
            self.undock(native, handle)
        } else {
            self.dock(native, handle, None)
        }
    }

    /// Dock a window dropped on the panel, remembering where its drag began.
    pub(crate) fn dock_dropped(
        &mut self,
        native: &dyn Native,
        handle: u64,
        start: Option<Rect>,
    ) -> Result<()> {
        self.prune(native);
        if self.contains(handle) {
            return Ok(());
        }
        self.dock(native, handle, start)
    }

    /// Bring a member into the arena with the Center layout, sending the
    /// previous one back to the dock.
    fn activate(&mut self, native: &dyn Native, handle: u64) -> Result<()> {
        if self.active == Some(handle) {
            return native.focus(handle);
        }
        if let Some(previous) = self.active.take() {
            native.stow(previous, native.slot(previous), self.step_of(previous))?;
        }
        let arena = native.arena()?;
        let target = Action::Center.target(arena, arena);
        native.place(handle, target, native.slot(handle))?;
        self.active = Some(handle);
        self.settled = None;
        native.focus(handle)
    }

    fn return_active(&mut self, native: &dyn Native) -> Result<()> {
        match self.active.take() {
            Some(handle) => native.stow(handle, native.slot(handle), self.step_of(handle)),
            None => Ok(()),
        }
    }

    /// Minimizing a docked window takes it out of the dock. Restoring it later
    /// puts it back where it was before it was docked.
    fn follow_minimized(&mut self, native: &dyn Native) {
        let mut waiting = Vec::new();
        for member in std::mem::take(&mut self.minimized) {
            match native.identity(member.handle) {
                Some((id, _)) if id == member.id => {
                    if native.minimized(member.handle) {
                        waiting.push(member);
                    } else {
                        let _ = native.place(member.handle, member.restore, None);
                    }
                }
                _ => {}
            }
        }
        for member in std::mem::take(&mut self.members) {
            if !native.minimized(member.handle) {
                self.members.push(member);
                continue;
            }
            if self.active == Some(member.handle) {
                self.active = None;
            }
            if self.settled == Some(member.handle) {
                self.settled = None;
            }
            let _ = native.mark(member.handle, false);
            waiting.push(member);
        }
        self.minimized = waiting;
    }

    /// One observation: minimized windows, clicked previews, then focus changes.
    pub(crate) fn step(&mut self, native: &dyn Native, clicks: &[u64]) -> Result<()> {
        self.prune(native);
        self.resume(native);
        // Before anything parks: parking would restore a minimized window.
        self.follow_minimized(native);
        // Docked windows that aren't at their park spot, because the edge, fill
        // or their place in the dock changed or an older build parked them
        // elsewhere, move there now. They keep their size, so a window that is
        // already laid out doesn't flash on screen to grow to 1080p again.
        for (step, member) in self.members.iter().enumerate() {
            if Some(member.handle) != self.active
                && native
                    .identity(member.handle)
                    .is_some_and(|(_, bounds)| !native.parked(bounds, step))
            {
                let _ = native.stow(member.handle, None, step);
            }
        }
        if let Some(&clicked) = clicks.iter().rev().find(|h| self.contains(**h)) {
            return self.activate(native, clicked);
        }
        let Some(foreground) = native.foreground() else {
            return Ok(());
        };
        if self.settled == Some(foreground) {
            return Ok(());
        }
        self.settled = None;
        if self.active == Some(foreground) {
            return Ok(());
        }
        if self.contains(foreground) {
            // Alt+Tab or the taskbar picked a docked window.
            return self.activate(native, foreground);
        }
        self.return_active(native)
    }

    /// Dock again, in their old order, the windows that were docked when
    /// tt-pane last quit. Windows that closed, whose handle now names another
    /// window, or that were moved since are forgotten.
    fn resume(&mut self, native: &dyn Native) {
        for entry in std::mem::take(&mut self.resume) {
            if self.contains(entry.handle) {
                continue;
            }
            let Some((id, bounds)) = native.identity(entry.handle) else {
                continue;
            };
            if id == entry.id && crate::positioning::near(bounds, entry.left) {
                let _ = self.dock(native, entry.handle, Some(entry.restore));
            }
        }
    }

    /// Put every member back where it was before it was docked. With
    /// `remember`, for quitting, the next start docks them again; without it,
    /// for turning the dock off, they're forgotten.
    pub(crate) fn restore_all(&mut self, native: &dyn Native, remember: bool) -> Result<()> {
        let mut failure = None;
        // A start that never got to resume, such as one with the dock off,
        // leaves nothing older to carry over.
        self.resume.clear();
        for member in std::mem::take(&mut self.members) {
            let result = native
                .mark(member.handle, false)
                .and_then(|()| native.place(member.handle, member.restore, None));
            let identity = native.identity(member.handle);
            if remember
                && let Some((id, left)) = &identity
                && *id == member.id
            {
                self.resume.push(Resumed {
                    handle: member.handle,
                    id: member.id,
                    restore: member.restore,
                    left: *left,
                });
            }
            if let Err(error) = result
                && identity.is_some()
            {
                failure.get_or_insert(error);
            }
        }
        self.active = None;
        self.settled = None;
        // Squeezed windows get their full size back, unless they moved since.
        self.refit(native, None, DockEdge::default(), 0, &[]);
        failure.map_or(Ok(()), Err)
    }
}

/// Docks a window when a mouse move of it ends over the panel.
#[derive(Debug, Default)]
pub struct DragTracker {
    moving: Option<u64>,
    /// Where the moving window was when the drag was first seen.
    start: Option<Rect>,
    /// A window that isn't docked is being dragged, so the panel offers a drop slot.
    pub dragging: bool,
    /// The dragged window is over the panel.
    pub over: bool,
}

impl DragTracker {
    /// Feed one observation. Returns a window dropped on the panel, and where
    /// its drag began. `bounds` reads a window's bounds when a drag starts.
    pub(crate) fn observe(
        &mut self,
        moving: Option<u64>,
        cursor: Option<(i32, i32)>,
        panel: Option<Rect>,
        dock: &Dock,
        bounds: impl Fn(u64) -> Option<Rect>,
    ) -> Option<(u64, Option<Rect>)> {
        let inside = match (cursor, panel) {
            (Some(cursor), Some(panel)) => crate::platform::dock::contains(panel, cursor),
            _ => false,
        };
        let dropped = match (self.moving, moving) {
            (Some(previous), now) if now != Some(previous) && inside => {
                Some((previous, self.start))
            }
            _ => None,
        };
        if moving != self.moving {
            self.start = moving.and_then(&bounds);
        }
        self.moving = moving;
        self.dragging = moving.is_some_and(|h| !dock.contains(h));
        self.over = inside && self.dragging;
        dropped.filter(|(h, _)| !dock.contains(*h))
    }
}

#[cfg(any(windows, target_os = "linux"))]
mod native {
    use super::*;
    use crate::platform;

    /// The live desktop. The edge keeps arena windows clear of the panel, and
    /// with the fill end picks the corner docked windows are parked at.
    pub(super) struct Desktop {
        pub edge: DockEdge,
        pub fill: DockFill,
        /// Park with a corner on screen so Chromium apps keep drawing.
        pub live: bool,
    }

    impl Desktop {
        pub fn new(config: &crate::config::Config) -> Self {
            Self {
                edge: config.dock.edge,
                fill: config.dock.fill,
                live: config.dock.live_previews,
            }
        }
    }

    impl Native for Desktop {
        fn identity(&self, handle: u64) -> Option<(String, Rect)> {
            platform::docked_window(handle)
        }
        fn mark(&self, handle: u64, docked: bool) -> Result<()> {
            platform::set_docked(handle, docked)
        }
        fn park(&self, handle: u64, step: usize) -> Result<()> {
            platform::park_window(handle, self.edge, self.fill, self.live, step).map(|_| ())
        }
        fn stow(&self, handle: u64, slot: Option<Rect>, step: usize) -> Result<()> {
            platform::stow_window(handle, slot, self.edge, self.fill, self.live, step).map(|_| ())
        }
        fn parked(&self, bounds: Rect, step: usize) -> bool {
            platform::dock_parked(self.edge, self.fill, bounds, self.live, step)
        }
        fn place(&self, handle: u64, target: Rect, from: Option<Rect>) -> Result<()> {
            match from {
                Some(slot) => platform::lift_window(handle, slot, target),
                None => platform::place_window(handle, target),
            }
            .map(|_| ())
        }
        fn focus(&self, handle: u64) -> Result<()> {
            platform::activate_handle(handle)
        }
        fn arena(&self) -> Result<Rect> {
            let (_, work) = platform::dock_monitor()?;
            // A panel shorter than its edge reserves nothing, so keep the
            // arena clear of it here; a reserved one is already outside `work`.
            Ok(match platform::dock_layout() {
                Some(layout) => platform::dock::arena(work, layout.panel, self.edge),
                None => work,
            })
        }
        fn foreground(&self) -> Option<u64> {
            platform::foreground_handle()
        }
        fn minimized(&self, handle: u64) -> bool {
            platform::is_minimized(handle)
        }
        fn slot(&self, handle: u64) -> Option<Rect> {
            platform::dock_layout()?
                .slots
                .into_iter()
                .find(|(h, _)| *h == handle)
                .map(|(_, rect)| rect)
        }
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn cycle_lock(path: &Path) -> Result<File> {
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(crate::positioning::sidecar(path, ".cycle.lock"))?)
}

/// `tt-pane apply dock`: dock the window, or undock a docked one. The caller
/// holds the cycle lock.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn toggle(
    config: &crate::config::Config,
    path: &Path,
    window: Option<u64>,
) -> Result<()> {
    anyhow::ensure!(
        config.dock.enabled,
        "The dock is off. Turn it on in Settings → Dock."
    );
    anyhow::ensure!(
        crate::platform::dock_running(),
        "The dock is shown by tt-pane in the tray. Start tt-pane first."
    );
    let handle = window
        .or_else(crate::platform::foreground_handle)
        .context("No window to dock")?;
    let mut dock = Dock::load(path)?;
    let result = dock.toggle(&native::Desktop::new(config), handle);
    dock.save(path)?;
    result
}

/// One key of dock pick mode: choose the window it names, which the tray
/// brings out at its next observation. `Ok(false)` when the dock is empty, so
/// the mode should end.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn pick(config: &crate::config::Config, path: &Path, step: PickStep) -> Result<bool> {
    use crate::platform;
    anyhow::ensure!(
        config.dock.enabled,
        "The dock is off. Turn it on in Settings → Dock."
    );
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let PickStep::Move(toward) = step {
        // Moving changes the dock, so it waits for the tray's observation.
        let lock = cycle_lock(&path)?;
        lock.lock().context("Lock positioning cycle")?;
        let mut dock = Dock::load(&path)?;
        if dock.handles().is_empty() {
            return Ok(false);
        }
        let current = platform::pending_dock_pick().or(dock.active());
        if let Some(handle) = pick_target(&dock.handles(), current, step)
            && dock.shift_member(handle, toward.offset(config.dock.fill))
        {
            dock.save(&path)?;
        }
        return Ok(true);
    }
    // Saves are atomic, so reading needs no lock.
    let dock = Dock::load(&path)?;
    let members = dock.handles();
    if members.is_empty() {
        return Ok(false);
    }
    let mut last = LAST_PICK.lock().unwrap_or_else(|e| e.into_inner());
    let current = pick_current(
        &members,
        platform::pending_dock_pick(),
        dock.active(),
        *last,
        step,
    );
    if let Some(handle) = pick_target(&members, current, step.in_order(config.dock.fill)) {
        platform::pick_dock(handle);
        *last = LastPick::of(&members, handle);
    }
    Ok(true)
}

/// Positioning a docked window with another action takes it out of the dock.
/// The caller holds the cycle lock.
#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn release(path: &Path, window: Option<u64>) -> Result<()> {
    if !Dock::exists(path) {
        return Ok(());
    }
    let Some(handle) = window.or_else(crate::platform::foreground_handle) else {
        return Ok(());
    };
    let mut dock = Dock::load(path)?;
    if dock.contains(handle) {
        // Releasing never parks or enters the arena, so the edge and fill
        // don't matter.
        let desktop = native::Desktop {
            edge: DockEdge::default(),
            fill: DockFill::default(),
            live: true,
        };
        dock.release(&desktop, handle)?;
        dock.save(path)?;
    }
    Ok(())
}

/// The tray's periodic dock work: show or hide the panel, activate clicked
/// previews, return the active window when focus moves, and dock dropped windows.
#[cfg(any(windows, target_os = "linux"))]
pub fn observe(config: &crate::config::Config, path: &Path, drag: &mut DragTracker) -> Result<()> {
    use crate::platform;
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !config.dock.enabled {
        platform::hide_dock();
        *drag = DragTracker::default();
        if Dock::exists(&path) {
            return shutdown(&path, false);
        }
        return Ok(());
    }
    let lock = cycle_lock(&path)?;
    if lock.try_lock().is_err() {
        // A CLI action is running; clicks stay queued for the next observation.
        return Ok(());
    }
    let mut dock = Dock::load(&path)?;
    let before = dock.clone();
    let layout = platform::dock_layout();
    let dropped = drag.observe(
        platform::moving_window(),
        platform::cursor_position(),
        layout.as_ref().map(|l| l.panel),
        &dock,
        |handle| platform::docked_window(handle).map(|(_, bounds)| bounds),
    );
    let clicks = platform::take_dock_clicks();
    for (handle, to) in platform::take_dock_moves() {
        dock.move_member(handle, to);
    }
    let desktop = native::Desktop::new(config);
    let result = platform::with_snap_animation(config.animation.duration(), || {
        if let Some((handle, start)) = dropped {
            dock.dock_dropped(&desktop, handle, start)?;
        }
        dock.step(&desktop, &clicks)
    });
    let area = platform::dock_layout().and_then(|layout| layout.area);
    if area != dock.panel() {
        // Squeezed windows glide to their new size like any other snap.
        let snapped = snapped_windows(config, &path, area);
        platform::with_snap_animation(config.animation.duration(), || {
            dock.refit(
                &desktop,
                area,
                config.dock.edge,
                platform::dock::snap_gap(config.gaps.inner),
                &snapped,
            )
        });
    }
    if dock != before {
        dock.save(&path)?;
    }
    platform::show_dock(platform::DockRequest {
        edge: config.dock.edge,
        fill: config.dock.fill,
        percent: config.dock.size_percent,
        members: dock.handles(),
        active: dock.active(),
        dragging: drag.dragging,
        highlight: drag.over,
        numbered: platform::dock_picking(),
        gaps: config.gaps,
    })?;
    result
}

/// Snapped windows on the dock monitor that the panel `area` covers. Automatic
/// tiling arranges its own windows, so none are offered while it is on.
#[cfg(any(windows, target_os = "linux"))]
fn snapped_windows(
    config: &crate::config::Config,
    path: &Path,
    area: Option<Rect>,
) -> Vec<(u64, String, Rect)> {
    use crate::platform;
    let Some(area) = area else {
        return Vec::new();
    };
    if crate::tiling::active(config, path) {
        return Vec::new();
    }
    let (Ok((_, work)), Ok(desktop)) = (platform::dock_monitor(), platform::tiling_desktop())
    else {
        return Vec::new();
    };
    desktop
        .windows
        .into_iter()
        .map(|w| w.snapshot)
        // On X11 tiling works out its own work areas, which can differ from
        // GNOME's by the top bar, so any window on the dock monitor counts.
        .filter(|s| {
            (if cfg!(windows) {
                s.area == work
            } else {
                platform::dock::intersects(s.area, work)
            }) && platform::dock::intersects(s.bounds, area)
        })
        .filter(|s| platform::is_edge_snapped(s.bounds, work, config.gaps))
        .map(|s| (s.handle, s.id, s.bounds))
        .collect()
}

/// Put every docked window back and remove the panel, for quitting or turning
/// the dock off. With `remember`, for quitting, the next start docks the
/// windows again.
#[cfg(any(windows, target_os = "linux"))]
pub fn shutdown(path: &Path, remember: bool) -> Result<()> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let result = (|| {
        if !Dock::exists(&path) {
            return Ok(());
        }
        let lock = cycle_lock(&path)?;
        lock.lock().context("Lock positioning cycle")?;
        let mut dock = Dock::load(&path)?;
        // Restoring never parks or enters the arena, so the edge and fill
        // don't matter.
        let desktop = native::Desktop {
            edge: DockEdge::default(),
            fill: DockFill::default(),
            live: true,
        };
        let result = dock.restore_all(&desktop, remember);
        dock.save(&path)?;
        result
    })();
    crate::platform::hide_dock();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::HashMap};

    const ARENA: Rect = Rect {
        x: 0,
        y: 0,
        width: 2000,
        height: 1000,
    };

    #[derive(Default)]
    struct Fake {
        windows: RefCell<HashMap<u64, (String, Rect)>>,
        marked: RefCell<Vec<u64>>,
        parked: RefCell<Vec<u64>>,
        log: RefCell<Vec<String>>,
        foreground: RefCell<Option<u64>>,
        minimized: RefCell<Vec<u64>>,
    }

    const PARKED: Rect = Rect {
        x: 9000,
        y: 0,
        width: 1920,
        height: 1080,
    };

    /// Where the fake parks the window at `step`: each place its own spot.
    fn parked_at(step: usize) -> Rect {
        Rect {
            x: PARKED.x + 2 * step as i32,
            ..PARKED
        }
    }

    impl Fake {
        fn with(handles: &[u64]) -> Self {
            let fake = Self::default();
            for &h in handles {
                fake.windows.borrow_mut().insert(
                    h,
                    (
                        format!("win32:1:{h}"),
                        Rect {
                            x: h as i32,
                            y: 10,
                            width: 300,
                            height: 200,
                        },
                    ),
                );
            }
            fake
        }
        fn bounds(&self, handle: u64) -> Rect {
            self.windows.borrow()[&handle].1
        }
        fn set_foreground(&self, handle: Option<u64>) {
            *self.foreground.borrow_mut() = handle;
        }
    }

    impl Native for Fake {
        fn identity(&self, handle: u64) -> Option<(String, Rect)> {
            self.windows.borrow().get(&handle).cloned()
        }
        fn mark(&self, handle: u64, docked: bool) -> Result<()> {
            let mut marked = self.marked.borrow_mut();
            marked.retain(|h| *h != handle);
            if docked {
                marked.push(handle);
            }
            Ok(())
        }
        fn parked(&self, bounds: Rect, step: usize) -> bool {
            bounds == parked_at(step)
        }
        fn stow(&self, handle: u64, slot: Option<Rect>, step: usize) -> Result<()> {
            self.log
                .borrow_mut()
                .push(format!("stow {handle} to {}", slot.is_some()));
            self.park(handle, step)
        }
        fn park(&self, handle: u64, step: usize) -> Result<()> {
            self.parked.borrow_mut().push(handle);
            self.windows.borrow_mut().get_mut(&handle).unwrap().1 = parked_at(step);
            Ok(())
        }
        fn place(&self, handle: u64, target: Rect, from: Option<Rect>) -> Result<()> {
            self.log
                .borrow_mut()
                .push(format!("place {handle} from {}", from.is_some()));
            self.windows
                .borrow_mut()
                .get_mut(&handle)
                .context("closed")?
                .1 = target;
            Ok(())
        }
        fn focus(&self, handle: u64) -> Result<()> {
            self.set_foreground(Some(handle));
            Ok(())
        }
        fn arena(&self) -> Result<Rect> {
            Ok(ARENA)
        }
        fn foreground(&self) -> Option<u64> {
            *self.foreground.borrow()
        }
        fn minimized(&self, handle: u64) -> bool {
            self.minimized.borrow().contains(&handle)
        }
        fn slot(&self, _handle: u64) -> Option<Rect> {
            Some(Rect {
                x: 10,
                y: 900,
                width: 160,
                height: 90,
            })
        }
    }

    #[test]
    fn docking_parks_and_marks_and_undocking_restores_the_window() {
        let fake = Fake::with(&[1]);
        let original = fake.bounds(1);
        fake.set_foreground(Some(1));
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        assert_eq!(dock.handles(), [1]);
        assert_eq!(*fake.marked.borrow(), [1]);
        assert_eq!(fake.bounds(1), PARKED);
        // Still focused right after docking, it stays in the dock.
        dock.step(&fake, &[]).unwrap();
        assert_eq!(dock.active(), None);

        dock.toggle(&fake, 1).unwrap();
        assert!(dock.handles().is_empty());
        assert!(fake.marked.borrow().is_empty());
        assert_eq!(fake.bounds(1), original);
    }

    #[test]
    fn clicking_a_preview_brings_it_to_the_center_and_returns_the_previous_one() {
        let fake = Fake::with(&[1, 2, 3]);
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.toggle(&fake, 2).unwrap();
        fake.set_foreground(Some(3));

        dock.step(&fake, &[1]).unwrap();
        assert_eq!(dock.active(), Some(1));
        assert_eq!(fake.bounds(1), Action::Center.target(ARENA, ARENA));
        assert_eq!(fake.foreground(), Some(1));
        assert_eq!(fake.log.borrow().last().unwrap(), "place 1 from true");

        dock.step(&fake, &[2]).unwrap();
        assert_eq!(dock.active(), Some(2));
        assert_eq!(fake.bounds(1), PARKED);
        // The previous window glides back into its slot.
        assert!(fake.log.borrow().contains(&"stow 1 to true".to_owned()));
        assert_eq!(fake.bounds(2), Action::Center.target(ARENA, ARENA));
    }

    #[test]
    fn focusing_another_window_returns_the_active_one_and_alt_tab_activates() {
        let fake = Fake::with(&[1, 2, 3]);
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.toggle(&fake, 2).unwrap();
        dock.step(&fake, &[1]).unwrap();
        // Unchanged focus keeps it in the arena.
        dock.step(&fake, &[]).unwrap();
        assert_eq!(dock.active(), Some(1));

        fake.set_foreground(Some(3));
        dock.step(&fake, &[]).unwrap();
        assert_eq!(dock.active(), None);
        assert_eq!(fake.bounds(1), PARKED);
        assert_eq!(fake.log.borrow().last().unwrap(), "stow 1 to true");

        // Choosing a docked window some other way (Alt+Tab) activates it too.
        fake.set_foreground(Some(2));
        dock.step(&fake, &[]).unwrap();
        assert_eq!(dock.active(), Some(2));
        assert_eq!(fake.bounds(2), Action::Center.target(ARENA, ARENA));
    }

    #[test]
    fn docked_windows_away_from_the_park_spot_are_parked_again() {
        let fake = Fake::with(&[1, 2]);
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.toggle(&fake, 2).unwrap();
        dock.step(&fake, &[2]).unwrap();
        let stray = Rect {
            x: 50_000,
            y: 0,
            width: 1920,
            height: 1080,
        };
        fake.windows.borrow_mut().get_mut(&1).unwrap().1 = stray;
        dock.step(&fake, &[]).unwrap();
        assert_eq!(fake.bounds(1), PARKED);
        // The window in the arena stays put.
        assert_eq!(fake.bounds(2), Action::Center.target(ARENA, ARENA));
    }

    #[test]
    fn snapped_windows_the_panel_covers_shrink_and_grow_back() {
        let fake = Fake::with(&[7, 8]);
        let right_half = Rect {
            x: 1000,
            y: 0,
            width: 1000,
            height: 1000,
        };
        let left_half = Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 1000,
        };
        fake.windows.borrow_mut().get_mut(&7).unwrap().1 = right_half;
        fake.windows.borrow_mut().get_mut(&8).unwrap().1 = left_half;
        let small = Rect {
            x: 1800,
            y: 0,
            width: 200,
            height: 300,
        };
        let snapped = [
            (7, "win32:1:7".to_owned(), right_half),
            (8, "win32:1:8".to_owned(), left_half),
        ];
        let mut dock = Dock::default();
        dock.refit(&fake, Some(small), DockEdge::Right, 10, &snapped);
        assert_eq!(fake.bounds(7).width, 1800 - 10 - 1000);
        // The panel doesn't reach the left half.
        assert_eq!(fake.bounds(8), left_half);

        // The panel grows into a full strip: the squeeze follows its new size.
        let strip = Rect {
            x: 1700,
            y: 0,
            width: 300,
            height: 1000,
        };
        dock.refit(&fake, Some(strip), DockEdge::Right, 10, &[]);
        assert_eq!(fake.bounds(7).width, 1700 - 10 - 1000);

        // Nothing docked any more: the window gets its full size back.
        dock.refit(&fake, None, DockEdge::Right, 10, &[]);
        assert_eq!(fake.bounds(7), right_half);
        assert_eq!(dock.panel(), None);
    }

    #[test]
    fn squeezed_windows_the_user_moved_are_left_alone() {
        let fake = Fake::with(&[7]);
        let right_half = Rect {
            x: 1000,
            y: 0,
            width: 1000,
            height: 1000,
        };
        fake.windows.borrow_mut().get_mut(&7).unwrap().1 = right_half;
        let panel = Rect {
            x: 1800,
            y: 0,
            width: 200,
            height: 300,
        };
        let mut dock = Dock::default();
        dock.refit(
            &fake,
            Some(panel),
            DockEdge::Right,
            0,
            &[(7, "win32:1:7".to_owned(), right_half)],
        );
        let moved = Rect {
            x: 100,
            y: 100,
            width: 500,
            height: 500,
        };
        fake.windows.borrow_mut().get_mut(&7).unwrap().1 = moved;
        dock.restore_all(&fake, false).unwrap();
        assert_eq!(fake.bounds(7), moved);
        // A snap that tt-pane cut back itself is remembered the same way.
        dock.remember_squeeze(7, "win32:1:7", right_half, moved);
        dock.refit(&fake, None, DockEdge::Right, 0, &[]);
        assert_eq!(fake.bounds(7), right_half);
    }

    #[test]
    fn minimizing_a_docked_window_takes_it_out_and_restoring_puts_it_back() {
        let fake = Fake::with(&[1, 2]);
        let original = fake.bounds(1);
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.toggle(&fake, 2).unwrap();
        dock.step(&fake, &[1]).unwrap();
        assert_eq!(dock.active(), Some(1));

        // Minimizing the window in the arena moves focus on; it must not be parked.
        fake.minimized.borrow_mut().push(1);
        fake.set_foreground(Some(2));
        let parked_before = fake.parked.borrow().len();
        dock.step(&fake, &[]).unwrap();
        assert_eq!(dock.handles(), [2]);
        assert_eq!(dock.active(), Some(2));
        assert!(!fake.marked.borrow().contains(&1));
        assert!(!fake.parked.borrow()[parked_before..].contains(&1));

        // Still minimized: nothing moves, and the file keeps tracking it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        dock.toggle(&fake, 2).unwrap();
        assert!(dock.handles().is_empty());
        dock.save(&path).unwrap();
        assert!(Dock::exists(&path));
        dock = Dock::load(&path).unwrap();
        dock.step(&fake, &[]).unwrap();
        assert_ne!(fake.bounds(1), original);
        // Restored: back where it was before it was docked, and no longer tracked.
        fake.minimized.borrow_mut().clear();
        dock.step(&fake, &[]).unwrap();
        assert_eq!(fake.bounds(1), original);
        assert!(dock.minimized.is_empty());
    }

    #[test]
    fn pick_steps_wrap_and_start_from_the_window_out_of_the_dock() {
        let members = [10, 20, 30];
        assert_eq!(pick_target(&[], None, PickStep::Start), None);
        assert_eq!(pick_target(&members, None, PickStep::Start), Some(10));
        assert_eq!(pick_target(&members, Some(20), PickStep::Start), Some(20));
        assert_eq!(pick_target(&members, None, PickStep::Next), Some(10));
        assert_eq!(pick_target(&members, None, PickStep::Previous), Some(30));
        assert_eq!(pick_target(&members, Some(20), PickStep::Next), Some(30));
        assert_eq!(pick_target(&members, Some(30), PickStep::Next), Some(10));
        assert_eq!(
            pick_target(&members, Some(10), PickStep::Previous),
            Some(30)
        );
        assert_eq!(pick_target(&members, Some(10), PickStep::Slot(2)), Some(30));
        assert_eq!(pick_target(&members, Some(10), PickStep::Slot(5)), None);
        // A current window that has left the dock counts as none.
        assert_eq!(pick_target(&members, Some(99), PickStep::Next), Some(10));
    }

    #[test]
    fn stepping_through_picks_brings_each_out_and_returns_the_last() {
        let fake = Fake::with(&[1, 2, 3]);
        let mut dock = Dock::default();
        for h in [1, 2, 3] {
            dock.toggle(&fake, h).unwrap();
        }
        let center = Action::Center.target(ARENA, ARENA);
        let first = pick_target(&dock.handles(), dock.active(), PickStep::Start).unwrap();
        dock.step(&fake, &[first]).unwrap();
        assert_eq!((dock.active(), fake.bounds(1)), (Some(1), center));
        let next = pick_target(&dock.handles(), dock.active(), PickStep::Next).unwrap();
        dock.step(&fake, &[next]).unwrap();
        assert_eq!((dock.active(), fake.bounds(2)), (Some(2), center));
        assert_eq!(fake.bounds(1), PARKED);
        let slot = pick_target(&dock.handles(), dock.active(), PickStep::Slot(2)).unwrap();
        dock.step(&fake, &[slot]).unwrap();
        assert_eq!((dock.active(), fake.bounds(3)), (Some(3), center));
        assert_eq!(fake.bounds(2), parked_at(1));
    }

    #[test]
    fn pick_mode_starts_from_the_window_picked_last() {
        let members = [10, 20, 30];
        let start = |last| {
            let current = pick_current(&members, None, None, last, PickStep::Start);
            pick_target(&members, current, PickStep::Start)
        };
        assert_eq!(start(None), Some(10));
        assert_eq!(start(LastPick::of(&members, 20)), Some(20));
        // A window still out of the dock, or waiting to come out, comes first.
        let last = LastPick::of(&members, 20);
        assert_eq!(
            pick_current(&members, None, Some(30), last, PickStep::Start),
            Some(30)
        );
        assert_eq!(
            pick_current(&members, Some(10), None, last, PickStep::Start),
            Some(10)
        );
        // Only the start of the mode goes back to the last pick.
        assert_eq!(
            pick_current(&members, None, None, last, PickStep::Next),
            None
        );
        // The window left the dock: start from the place it had.
        let gone = LastPick::of(&[10, 20, 99], 99);
        assert_eq!(start(gone), Some(30));
        assert_eq!(gone.unwrap().resume(&[10]), Some(10));
        assert_eq!(gone.unwrap().resume(&[]), None);
    }

    #[test]
    fn on_screen_steps_and_moves_pick_by_the_fill_end() {
        let members = [10, 20, 30];
        let right = PickStep::Toward(Toward::RightOrDown);
        let left = PickStep::Toward(Toward::LeftOrUp);
        let pick = |step: PickStep, fill| pick_target(&members, Some(20), step.in_order(fill));
        assert_eq!(pick(right, DockFill::Start), Some(30));
        assert_eq!(pick(left, DockFill::Start), Some(10));
        // Filled from the right, the next window in order sits to the left.
        assert_eq!(pick(right, DockFill::End), Some(10));
        assert_eq!(pick(left, DockFill::End), Some(30));
        // A move keeps the window that's picked, if it's still docked.
        let shift = PickStep::Move(Toward::RightOrDown);
        assert_eq!(pick_target(&members, Some(20), shift), Some(20));
        assert_eq!(pick_target(&members, Some(99), shift), None);
        assert_eq!(pick_target(&members, None, shift), None);
    }

    #[test]
    fn moving_a_member_reorders_the_dock_and_reparks_it_there() {
        let fake = Fake::with(&[1, 2, 3]);
        let mut dock = Dock::default();
        for h in [1, 2, 3] {
            dock.toggle(&fake, h).unwrap();
        }
        dock.step(&fake, &[2]).unwrap();
        assert!(dock.move_member(3, 0));
        assert_eq!(dock.handles(), [3, 1, 2]);
        // Past the end clamps; the window that's out stays out.
        assert!(dock.move_member(1, 9));
        assert_eq!(dock.handles(), [3, 2, 1]);
        assert_eq!(dock.active(), Some(2));
        assert!(!dock.move_member(99, 0));
        // One place at a time, stopping at the ends.
        assert!(!dock.shift_member(3, -1));
        assert!(!dock.shift_member(1, 1));
        assert!(dock.shift_member(2, 1));
        assert_eq!(dock.handles(), [3, 1, 2]);
        // The next observation parks each docked window at its new place.
        dock.step(&fake, &[]).unwrap();
        assert_eq!(
            (fake.bounds(3), fake.bounds(1)),
            (parked_at(0), parked_at(1))
        );
        assert_eq!(fake.bounds(2), Action::Center.target(ARENA, ARENA));
    }

    #[test]
    fn docked_windows_park_apart_and_close_up_when_one_leaves() {
        let fake = Fake::with(&[1, 2, 3]);
        let mut dock = Dock::default();
        for h in [1, 2, 3] {
            dock.toggle(&fake, h).unwrap();
        }
        // Each window has its own spot, so none covers another completely.
        let spots: Vec<Rect> = [1, 2, 3].map(|h| fake.bounds(h)).to_vec();
        assert_eq!(spots, [parked_at(0), parked_at(1), parked_at(2)]);
        // Undocking the first moves the others up a place, keeping their size.
        dock.toggle(&fake, 1).unwrap();
        dock.step(&fake, &[]).unwrap();
        assert_eq!(
            (fake.bounds(2), fake.bounds(3)),
            (parked_at(0), parked_at(1))
        );
        assert!(fake.log.borrow().contains(&"stow 2 to false".to_string()));
    }

    #[test]
    fn closed_windows_and_reused_handles_leave_the_dock() {
        let fake = Fake::with(&[1, 2]);
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.toggle(&fake, 2).unwrap();
        dock.step(&fake, &[1]).unwrap();
        fake.windows.borrow_mut().remove(&1);
        fake.windows.borrow_mut().get_mut(&2).unwrap().0 = "win32:9:2".into();
        dock.step(&fake, &[]).unwrap();
        assert!(dock.handles().is_empty());
        assert_eq!(dock.active(), None);
    }

    #[test]
    fn restoring_puts_every_window_back_and_releasing_leaves_it_in_place() {
        let fake = Fake::with(&[1, 2]);
        let originals = [fake.bounds(1), fake.bounds(2)];
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.toggle(&fake, 2).unwrap();
        dock.step(&fake, &[2]).unwrap();
        dock.release(&fake, 2).unwrap();
        assert_eq!(dock.handles(), [1]);
        assert_eq!(dock.active(), None);
        assert_eq!(fake.bounds(2), Action::Center.target(ARENA, ARENA));

        dock.restore_all(&fake, false).unwrap();
        assert!(dock.handles().is_empty());
        assert!(fake.marked.borrow().is_empty());
        assert_eq!(fake.bounds(1), originals[0]);
    }

    #[test]
    fn quitting_puts_windows_back_and_the_next_start_docks_them_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let fake = Fake::with(&[1, 2, 3]);
        let originals = [fake.bounds(1), fake.bounds(2)];
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.toggle(&fake, 2).unwrap();
        fake.set_foreground(Some(3));
        dock.step(&fake, &[2]).unwrap();

        dock.restore_all(&fake, true).unwrap();
        assert!(dock.handles().is_empty());
        assert!(fake.marked.borrow().is_empty());
        assert_eq!([fake.bounds(1), fake.bounds(2)], originals);
        // The file outlives the quit, for the next start.
        dock.save(&path).unwrap();
        assert!(Dock::exists(&path));

        let mut dock = Dock::load(&path).unwrap();
        fake.set_foreground(Some(3));
        dock.step(&fake, &[]).unwrap();
        assert_eq!(dock.handles(), [1, 2]);
        assert_eq!(dock.active(), None);
        assert_eq!(*fake.marked.borrow(), [1, 2]);
        assert_eq!(
            [fake.bounds(1), fake.bounds(2)],
            [parked_at(0), parked_at(1)]
        );
        // Undocking after the restart still puts it back where it began.
        dock.toggle(&fake, 2).unwrap();
        assert_eq!(fake.bounds(2), originals[1]);
        assert!(dock.resume.is_empty());
    }

    #[test]
    fn the_next_start_forgets_windows_that_closed_changed_or_moved() {
        let fake = Fake::with(&[1, 2, 3, 4]);
        let mut dock = Dock::default();
        for handle in 1..=4 {
            dock.toggle(&fake, handle).unwrap();
        }
        dock.restore_all(&fake, true).unwrap();
        fake.windows.borrow_mut().remove(&1);
        fake.windows.borrow_mut().get_mut(&2).unwrap().0 = "win32:9:2".into();
        fake.windows.borrow_mut().get_mut(&3).unwrap().1.x += 50;

        dock.step(&fake, &[]).unwrap();
        assert_eq!(dock.handles(), [4]);
        assert_eq!(fake.bounds(4), parked_at(0));
        assert!(dock.resume.is_empty());
    }

    #[test]
    fn turning_the_dock_off_forgets_windows_instead_of_docking_them_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let fake = Fake::with(&[1]);
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.restore_all(&fake, false).unwrap();
        assert!(dock.resume.is_empty());

        // Quit, then started with the dock off: nothing is carried over.
        dock.toggle(&fake, 1).unwrap();
        dock.restore_all(&fake, true).unwrap();
        assert_eq!(dock.resume.len(), 1);
        dock.restore_all(&fake, false).unwrap();
        dock.save(&path).unwrap();
        assert!(!Dock::exists(&path));
    }

    #[test]
    fn dropping_a_dragged_window_on_the_panel_docks_it() {
        let panel = Rect {
            x: 0,
            y: 900,
            width: 2000,
            height: 100,
        };
        let start = Rect {
            x: 40,
            y: 40,
            width: 300,
            height: 200,
        };
        let fake = Fake::with(&[5]);
        let mut dock = Dock::default();
        let mut drag = DragTracker::default();
        let at = |drag: &mut DragTracker, moving, cursor, dock: &Dock| {
            drag.observe(moving, Some(cursor), Some(panel), dock, |_| Some(start))
        };
        assert_eq!(at(&mut drag, Some(5), (50, 50), &dock), None);
        assert!(!drag.over);
        assert_eq!(at(&mut drag, Some(5), (50, 950), &dock), None);
        assert!(drag.over);
        let dropped = at(&mut drag, None, (50, 950), &dock);
        assert_eq!(dropped, Some((5, Some(start))));
        assert!(!drag.over);
        // Undocking puts it back where the drag began, not over the panel.
        dock.dock_dropped(&fake, 5, Some(start)).unwrap();
        dock.toggle(&fake, 5).unwrap();
        assert_eq!(fake.bounds(5), start);
        // Released elsewhere, nothing is docked.
        at(&mut drag, Some(6), (50, 950), &dock);
        assert_eq!(at(&mut drag, None, (50, 50), &dock), None);
    }

    #[test]
    fn saved_state_round_trips_and_an_empty_dock_removes_its_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let fake = Fake::with(&[1]);
        let mut dock = Dock::default();
        dock.toggle(&fake, 1).unwrap();
        dock.save(&path).unwrap();
        assert!(Dock::exists(&path));
        assert_eq!(Dock::load(&path).unwrap(), dock);
        Dock::default().save(&path).unwrap();
        assert!(!Dock::exists(&path));
    }
}
