//! Repeated-position cycles shared by tray shortcuts and standalone CLI processes.
use crate::{
    config::{Config, Growth},
    geometry::{Action, HorizontalAnchor, Rect},
    platform,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Gap {
    source: String,
    area: Rect,
    base: Rect,
    source_bounds: Rect,
    anchor: HorizontalAnchor,
    target: Rect,
}

impl Gap {
    fn usable_on(&self, area: Rect) -> bool {
        self.area == area
            && self.target.width > 0
            && self.target.height > 0
            && self.target.x >= area.x
            && self.target.y >= area.y
            && i64::from(self.target.x) + i64::from(self.target.width)
                <= i64::from(area.x) + i64::from(area.width)
            && i64::from(self.target.y) + i64::from(self.target.height)
                <= i64::from(area.y) + i64::from(area.height)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cycle {
    version: u32,
    window: String,
    action: Action,
    area: Rect,
    base: Rect,
    last: Rect,
    step: u32,
    growth: Growth,
    #[serde(default)]
    width_anchor: Option<HorizontalAnchor>,
    #[serde(default)]
    gaps: Vec<Gap>,
}

impl Cycle {
    fn fill_gap(previous: Option<&Self>, window: &str, area: Rect, growth: Growth) -> Result<Self> {
        let gap = previous
            .filter(|p| p.version == 1)
            .and_then(|p| p.gaps.iter().rev().find(|g| g.usable_on(area)))
            .context("No saved gap on this monitor. Shrink a positioned window first.")?;
        ensure!(
            gap.source != window,
            "Select a different window to fill this gap"
        );
        Ok(Self {
            version: 1,
            window: window.to_owned(),
            action: Action::FillGap,
            area,
            base: gap.target,
            last: gap.target,
            step: 0,
            growth,
            width_anchor: None,
            gaps: Vec::new(),
        })
    }

    fn record_result(&mut self, previous: Option<&Self>, actual: Rect) {
        self.last = actual;
        self.gaps = previous
            .filter(|p| p.version == 1)
            .map_or_else(Vec::new, |p| p.gaps.clone());
        // A source moved by tt-pane, an occupied target, or a consumed gap is no longer free.
        self.gaps.retain(|g| {
            g.usable_on(g.area)
                && g.source != self.window
                && g.target.intersection(actual).is_none()
                && !(self.action == Action::FillGap && g.area == self.area)
        });
        if self.action.adjusts_width()
            && let Some(target) = self
                .base
                .freed_width_gap(actual)
                .and_then(|r| r.intersection(self.area))
        {
            self.gaps.retain(|g| g.area != self.area);
            self.gaps.push(Gap {
                source: self.window.clone(),
                area: self.area,
                base: self.base,
                source_bounds: actual,
                anchor: self.width_anchor.unwrap(),
                target,
            });
        }
        if self.gaps.len() > 16 {
            self.gaps.drain(..self.gaps.len() - 16);
        }
    }

    fn adjust_width(
        previous: Option<&Self>,
        window: &str,
        action: Action,
        area: Rect,
        current: Rect,
        config: &Config,
    ) -> Self {
        let saved_gap = previous.filter(|p| p.version == 1).and_then(|p| {
            p.gaps
                .iter()
                .rev()
                .find(|g| g.source == window && g.usable_on(area) && near(g.source_bounds, current))
        });
        let previous = previous.filter(|p| {
            p.version == 1 && p.window == window && p.area == area && near(p.last, current)
        });
        let anchor = previous
            .and_then(|p| p.width_anchor.or_else(|| p.action.horizontal_anchor()))
            .or_else(|| saved_gap.map(|g| g.anchor))
            .unwrap_or_else(|| HorizontalAnchor::for_window(area, current));
        let base = previous
            .filter(|p| p.action.adjusts_width())
            .map(|p| p.base)
            .or_else(|| saved_gap.map(|g| g.base))
            .unwrap_or(current);
        let percent =
            config.width_resize.percent as i32 * if action == Action::ShrinkWidth { -1 } else { 1 };
        Self {
            version: 1,
            window: window.to_owned(),
            action,
            area,
            base,
            last: current.adjust_width_from(area, anchor, percent, base),
            step: 0,
            growth: config.growth,
            width_anchor: Some(anchor),
            gaps: Vec::new(),
        }
    }

    fn next(
        previous: Option<&Self>,
        window: &str,
        action: Action,
        area: Rect,
        current: Rect,
        growth: Growth,
    ) -> Self {
        let previous = previous.filter(|p| {
            p.version == 1
                && p.window == window
                && p.action == action
                && p.area == area
                && p.growth == growth
                // Ignore baselines produced by older placement rules (e.g. Center).
                && p.base == action.target(area, current)
                && p.step <= growth.steps
                && near(p.last, current)
        });
        let (base, step) = match previous {
            Some(p) => (p.base, if p.step < growth.steps { p.step + 1 } else { 0 }),
            None => (action.target(area, current), 0),
        };
        let last = if step == 0 {
            base
        } else {
            action.grown_target(
                area,
                base,
                (1.0 + f64::from(growth.percent) / 100.0).powi(step as i32),
            )
        };
        Self {
            version: 1,
            window: window.to_owned(),
            action,
            area,
            base,
            last,
            step,
            growth,
            width_anchor: None,
            gaps: Vec::new(),
        }
    }
}

// Fractional scaling and decoration rounding can differ by a pixel across reads.
pub(crate) fn near(a: Rect, b: Rect) -> bool {
    [
        a.x.abs_diff(b.x),
        a.y.abs_diff(b.y),
        a.width.abs_diff(b.width),
        a.height.abs_diff(b.height),
    ]
    .into_iter()
    .all(|difference| difference <= 2)
}

pub(crate) fn sidecar(config: &Path, suffix: &str) -> PathBuf {
    let mut name = config.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Serialize concurrent tray/plugin/CLI requests using a separate lock file.
/// Only successful native requests advance the persisted cycle. Cache loss resets it.
pub fn apply(config: &Config, path: &Path, action: Action, window: Option<u64>) -> Result<()> {
    apply_gesture(config, path, action, window, false)
}

/// A hold enters covered middle windows from an edge, or exits them to background edges.
pub fn apply_gesture(
    config: &Config,
    path: &Path,
    action: Action,
    window: Option<u64>,
    include_hidden: bool,
) -> Result<()> {
    ensure!(
        !include_hidden || matches!(action, Action::FocusLeft | Action::FocusRight),
        "Only horizontal focus supports hold navigation"
    );
    // Capture the source before placement; focus actions resolve their destination afterward.
    #[cfg(windows)]
    let source = window.or_else(platform::foreground_handle);
    let result = apply_inner(config, path, action, window, include_hidden);
    #[cfg(windows)]
    if result.is_ok() && config.focus.glow_enabled {
        let target = if action.focuses() {
            platform::foreground_handle()
        } else {
            source
        };
        if let (Some(target), Some(color)) = (target, config.focus.glow_rgb()) {
            platform::show_focus_glow(target, color);
        }
    }
    result
}

fn apply_inner(
    config: &Config,
    path: &Path,
    action: Action,
    window: Option<u64>,
    include_hidden: bool,
) -> Result<()> {
    config.validate()?;
    let visibility = if include_hidden {
        platform::FocusVisibility::Hold
    } else if config.focus.visibility_filter_enabled {
        platform::FocusVisibility::MoreThanPercent(config.focus.minimum_visible_percent)
    } else {
        platform::FocusVisibility::IncludeHidden
    };
    #[cfg(not(windows))]
    if action.focuses() {
        return platform::focus_window_filtered(window, action, visibility, &[]);
    }
    // Resolve symlinks when the config exists so aliases share one cycle.
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    if action.focuses() && !sidecar(&path, ".centered.json").exists() {
        return platform::focus_window_filtered(window, action, visibility, &[]);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sidecar(&path, ".cycle.lock"))
        .context("Open positioning cycle lock")?;
    lock.lock().context("Lock positioning cycle")?;
    #[cfg(windows)]
    let mut group = crate::centered::Group::load(&path)?;
    #[cfg(windows)]
    let group_before = group.clone();
    #[cfg(windows)]
    group.prune();
    #[cfg(windows)]
    if action.focuses() {
        let result = (|| -> Result<()> {
            if include_hidden || !group.focus(window, action, visibility)? {
                platform::focus_window_filtered(
                    window,
                    action,
                    visibility,
                    &group.exit_members(window, action),
                )?;
                group.refresh()?;
            }
            Ok(())
        })();
        if group != group_before {
            group.save(&path)?;
        }
        return result;
    }
    let state_path = sidecar(&path, ".cycle.json");
    if action.adjusts_pair() {
        platform::resize_pair(window, action == Action::ResizeUp)?;
        #[cfg(windows)]
        {
            group.prune();
            if group != group_before {
                group.save(&path)?;
            }
        }
        // Both windows changed. Discard stale growth baselines and saved gaps.
        match std::fs::remove_file(&state_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Reset positioning state after paired resize"),
        }
        return Ok(());
    }
    let previous: Option<Cycle> = match File::open(&state_path) {
        Ok(file) => serde_json::from_reader(file.take(64 * 1024)).ok(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Read positioning cycle"),
    };
    if action == Action::FillGap {
        ensure!(
            previous
                .as_ref()
                .is_some_and(|p| p.version == 1 && !p.gaps.is_empty()),
            "No saved gap. Shrink a positioned window first."
        );
    }
    let mut next = None;
    let actual = platform::apply_with(window, &mut |id, area, current| {
        let cycle = if action == Action::FillGap {
            Cycle::fill_gap(previous.as_ref(), id, area, config.growth)?
        } else if action.adjusts_width() {
            Cycle::adjust_width(previous.as_ref(), id, action, area, current, config)
        } else {
            Cycle::next(previous.as_ref(), id, action, area, current, config.growth)
        };
        let target = cycle.last;
        next = Some(cycle);
        Ok(target)
    })?;
    if let Some(mut cycle) = next {
        cycle.record_result(previous.as_ref(), actual);
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(&cycle)?)?;
        file.persist(&state_path)
            .map_err(|e| e.error)
            .context("Save positioning cycle")?;
        #[cfg(windows)]
        {
            let result = if action == Action::Center {
                group.register(&cycle.window, actual);
                match cycle.window.rsplit(':').next().and_then(|s| s.parse().ok()) {
                    Some(handle) => group.arrange(handle),
                    None => Ok(()),
                }
            } else {
                group.remove(&cycle.window);
                group.refresh()
            };
            if group != group_before {
                group.save(&path)?;
            }
            result?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    const AREA: Rect = Rect {
        x: -2000,
        y: 40,
        width: 2000,
        height: 1200,
    };
    const CURRENT: Rect = Rect {
        x: -1800,
        y: 100,
        width: 800,
        height: 600,
    };

    fn shrink(previous: Option<&Cycle>, source: &str, area: Rect, current: Rect) -> Cycle {
        let mut cycle = Cycle::adjust_width(
            previous,
            source,
            Action::ShrinkWidth,
            area,
            current,
            &Config::default(),
        );
        cycle.record_result(previous, cycle.last);
        cycle
    }

    #[test]
    fn gaps_survive_window_switches_and_are_consumed_once() {
        let base = Action::BottomLeft.target(AREA, CURRENT);
        let first = shrink(None, "source", AREA, base);
        let second = shrink(Some(&first), "source", AREA, first.last);
        assert_eq!(
            second.gaps[0].target.x + second.gaps[0].target.width,
            base.x + base.width
        );
        let persisted: Cycle =
            serde_json::from_str(&serde_json::to_string(&second).unwrap()).unwrap();
        assert!(Cycle::fill_gap(Some(&persisted), "source", AREA, Growth::default()).is_err());
        let mut filled =
            Cycle::fill_gap(Some(&persisted), "destination", AREA, Growth::default()).unwrap();
        assert_eq!(filled.last, base.freed_width_gap(second.last).unwrap());
        filled.record_result(Some(&persisted), filled.last);
        assert!(Cycle::fill_gap(Some(&filled), "third", AREA, Growth::default()).is_err());
        // A later shrink starts a fresh gap beside the now smaller source, excluding the filled slot.
        let next = shrink(Some(&filled), "source", AREA, second.last);
        assert_eq!(next.gaps[0].target.intersection(filled.last), None);
    }

    #[test]
    fn saved_gap_keeps_original_baseline_after_unrelated_action() {
        let base = Action::BottomLeft.target(AREA, CURRENT);
        let source = shrink(None, "source", AREA, base);
        let mut other = Cycle::next(
            Some(&source),
            "other",
            Action::TopRight,
            AREA,
            CURRENT,
            Growth::default(),
        );
        other.record_result(Some(&source), other.last);
        let next = shrink(Some(&other), "source", AREA, source.last);
        assert_eq!(next.base, base);
        assert!(next.gaps[0].target.width > source.gaps[0].target.width);
        let mut moved = Cycle::next(
            Some(&next),
            "source",
            Action::Top,
            AREA,
            next.last,
            Growth::default(),
        );
        moved.record_result(Some(&next), moved.last);
        assert!(moved.gaps.is_empty());
    }

    #[test]
    fn gaps_are_scoped_to_monitors_and_removed_when_expanded_away() {
        let source = shrink(None, "one", AREA, Action::BottomLeft.target(AREA, AREA));
        let other_area = Rect { x: 0, ..AREA };
        let other = shrink(
            Some(&source),
            "two",
            other_area,
            Action::BottomRight.target(other_area, other_area),
        );
        assert_eq!(other.gaps.len(), 2);
        let mut fill = Cycle::fill_gap(Some(&other), "three", AREA, Growth::default()).unwrap();
        assert!(fill.last.x < 0);
        fill.record_result(Some(&other), fill.last);
        assert_eq!(fill.gaps.len(), 1);
        assert_eq!(fill.gaps[0].area, other_area);
        assert!(Cycle::fill_gap(Some(&fill), "four", AREA, Growth::default()).is_err());
        let mut expanded = Cycle::adjust_width(
            Some(&fill),
            "two",
            Action::ExpandWidth,
            other_area,
            other.last,
            &Config::default(),
        );
        // Application result reaches the original baseline, eliminating the gap.
        expanded.record_result(Some(&fill), other.base);
        assert!(expanded.gaps.is_empty());
    }

    #[test]
    fn width_actions_keep_anchor_past_saturation_without_resetting() {
        let config = Config::default();
        for placement in [Action::BottomLeft, Action::BottomRight, Action::Center] {
            let mut cycle = Cycle::next(None, "window", placement, AREA, CURRENT, config.growth);
            let base = cycle.last;
            let mut previous_width = base.width;
            for _ in 0..4 {
                cycle = Cycle::adjust_width(
                    Some(&cycle),
                    "window",
                    Action::ShrinkWidth,
                    AREA,
                    cycle.last,
                    &config,
                );
                assert!(cycle.last.width < previous_width);
                previous_width = cycle.last.width;
                assert_eq!((cycle.last.y, cycle.last.height), (base.y, base.height));
            }
            for _ in 0..30 {
                cycle = Cycle::adjust_width(
                    Some(&cycle),
                    "window",
                    Action::ExpandWidth,
                    AREA,
                    cycle.last,
                    &config,
                );
                assert!(cycle.last.width >= previous_width);
                previous_width = cycle.last.width;
            }
            assert_eq!(cycle.last.width, AREA.width);
            cycle = Cycle::adjust_width(
                Some(&cycle),
                "window",
                Action::ShrinkWidth,
                AREA,
                cycle.last,
                &config,
            );
            assert_eq!(cycle.last.width, 1500);
            assert_eq!(cycle.width_anchor, placement.horizontal_anchor());
            assert_eq!(
                cycle.last.x,
                match placement {
                    Action::BottomLeft => -2000,
                    Action::BottomRight => -1500,
                    _ => -1750,
                }
            );
            let moved = Rect {
                x: -1000,
                y: 100,
                width: 800,
                height: 400,
            };
            cycle = Cycle::adjust_width(
                Some(&cycle),
                "other-window",
                Action::ShrinkWidth,
                AREA,
                moved,
                &config,
            );
            assert_eq!(cycle.base, moved);
            assert_eq!(cycle.width_anchor, Some(HorizontalAnchor::Right));
        }
    }

    #[test]
    fn center_replaces_old_full_height_baseline_then_grows_and_resets() {
        let mut previous = Cycle::next(
            None,
            "one",
            Action::Center,
            AREA,
            CURRENT,
            Growth::default(),
        );
        previous.base = Rect {
            x: -1500,
            y: 40,
            width: 1000,
            height: 1200,
        };
        previous.last = previous.base;
        let mut cycle = Cycle::next(
            Some(&previous),
            "one",
            Action::Center,
            AREA,
            previous.last,
            previous.growth,
        );
        let base = Rect {
            x: -1500,
            y: 340,
            width: 1000,
            height: 600,
        };
        assert_eq!((cycle.step, cycle.last), (0, base));
        for expected_step in [1, 2, 0] {
            cycle = Cycle::next(
                Some(&cycle),
                "one",
                Action::Center,
                AREA,
                cycle.last,
                cycle.growth,
            );
            assert_eq!(cycle.step, expected_step);
            assert!(cycle.last.height < AREA.height);
            assert!((2 * (cycle.last.x - AREA.x) + cycle.last.width - AREA.width).abs() <= 1);
            assert!((2 * (cycle.last.y - AREA.y) + cycle.last.height - AREA.height).abs() <= 1);
        }
        assert_eq!(cycle.last, base);
    }

    #[test]
    fn two_growth_steps_then_original_for_every_anchor() {
        for action in Action::POSITIONS {
            let base = Cycle::next(None, "window", action, AREA, CURRENT, Growth::default());
            let first = Cycle::next(Some(&base), "window", action, AREA, base.last, base.growth);
            let second = Cycle::next(
                Some(&first),
                "window",
                action,
                AREA,
                first.last,
                first.growth,
            );
            let reset = Cycle::next(
                Some(&second),
                "window",
                action,
                AREA,
                second.last,
                second.growth,
            );
            assert_eq!(
                (base.step, first.step, second.step, reset.step),
                (0, 1, 2, 0)
            );
            assert_eq!(reset.last, base.last, "{action:?}");
            assert_eq!(
                first.last.width,
                (f64::from(base.last.width) * 1.15)
                    .round()
                    .min(f64::from(AREA.width)) as i32
            );
            assert_eq!(
                second.last.height,
                (f64::from(base.last.height) * 1.3225)
                    .round()
                    .min(f64::from(AREA.height)) as i32
            );
            assert_eq!(second.last.intersection(AREA), Some(second.last));
        }
    }

    #[test]
    fn changes_start_a_new_cycle() {
        let base = Cycle::next(
            None,
            "one",
            Action::Center,
            AREA,
            CURRENT,
            Growth::default(),
        );
        let growth = Cycle::next(
            Some(&base),
            "one",
            Action::Center,
            AREA,
            base.last,
            base.growth,
        );
        for (id, action, area, current, settings) in [
            ("two", growth.action, AREA, growth.last, growth.growth),
            ("one", Action::TopLeft, AREA, growth.last, growth.growth),
            (
                "one",
                growth.action,
                Rect { x: 0, ..AREA },
                growth.last,
                growth.growth,
            ),
            ("one", growth.action, AREA, CURRENT, growth.growth),
            (
                "one",
                growth.action,
                AREA,
                growth.last,
                Growth {
                    percent: 20,
                    steps: 1,
                },
            ),
        ] {
            let next = Cycle::next(Some(&growth), id, action, area, current, settings);
            assert_eq!(next.step, 0);
            assert_eq!(next.last, action.target(area, current));
        }
    }

    #[test]
    fn custom_growth_and_disabled_cycle() {
        for steps in [0, 1, 4] {
            let settings = Growth { percent: 25, steps };
            let mut cycle = Cycle::next(None, "one", Action::Center, AREA, CURRENT, settings);
            let original = cycle.last;
            for step in 1..=steps {
                cycle = Cycle::next(
                    Some(&cycle),
                    "one",
                    cycle.action,
                    AREA,
                    cycle.last,
                    settings,
                );
                assert_eq!(cycle.step, step);
            }
            cycle = Cycle::next(
                Some(&cycle),
                "one",
                cycle.action,
                AREA,
                cycle.last,
                settings,
            );
            assert_eq!(cycle.last, original);
        }
    }
}
