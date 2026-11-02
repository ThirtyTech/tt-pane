//! Repeated-position cycles shared by tray shortcuts and standalone CLI processes.
use crate::{
    config::{Config, Growth},
    geometry::{Action, HorizontalAnchor, Rect},
    platform,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize, Deserialize)]
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
}

impl Cycle {
    fn adjust_width(
        previous: Option<&Self>,
        window: &str,
        action: Action,
        area: Rect,
        current: Rect,
        config: &Config,
    ) -> Self {
        let previous = previous.filter(|p| {
            p.version == 1 && p.window == window && p.area == area && near(p.last, current)
        });
        let anchor = previous
            .and_then(|p| p.width_anchor.or_else(|| p.action.horizontal_anchor()))
            .unwrap_or_else(|| HorizontalAnchor::for_window(area, current));
        let base = previous
            .filter(|p| p.action.adjusts_width())
            .map_or(current, |p| p.base);
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
        }
    }
}

// Fractional scaling and decoration rounding can differ by a pixel across reads.
fn near(a: Rect, b: Rect) -> bool {
    [
        a.x.abs_diff(b.x),
        a.y.abs_diff(b.y),
        a.width.abs_diff(b.width),
        a.height.abs_diff(b.height),
    ]
    .into_iter()
    .all(|difference| difference <= 2)
}

fn sidecar(config: &Path, suffix: &str) -> PathBuf {
    let mut name = config.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Serialize concurrent tray/plugin/CLI requests using a separate lock file.
/// Only successful native requests advance the persisted cycle. Cache loss resets it.
pub fn apply(config: &Config, path: &Path, action: Action, window: Option<u64>) -> Result<()> {
    config.validate()?;
    // Resolve symlinks when the config exists so aliases share one cycle.
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
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
    let state_path = sidecar(&path, ".cycle.json");
    let previous: Option<Cycle> = match File::open(&state_path) {
        Ok(file) => serde_json::from_reader(file.take(64 * 1024)).ok(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Read positioning cycle"),
    };
    let mut next = None;
    let actual = platform::apply_with(window, &mut |id, area, current| {
        let cycle = if action.adjusts_width() {
            Cycle::adjust_width(previous.as_ref(), id, action, area, current, config)
        } else {
            Cycle::next(previous.as_ref(), id, action, area, current, config.growth)
        };
        let target = cycle.last;
        next = Some(cycle);
        target
    })?;
    if let Some(mut cycle) = next {
        cycle.last = actual;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(&cycle)?)?;
        file.persist(&state_path)
            .map_err(|e| e.error)
            .context("Save positioning cycle")?;
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
