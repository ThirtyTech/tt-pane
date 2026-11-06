//! Pure experimental dwindle layout. Own one tree per monitor/workspace.
//! Floating windows are absent from the tree; native eligibility, persistence,
//! focus and reconciliation belong to the caller.
#![forbid(unsafe_code)]

use crate::geometry::{Rect, SnapGaps};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

/// A snap position a tile can be docked into. Quarters dock their column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Dock {
    fn axis(self) -> Axis {
        match self {
            Self::Top | Self::Bottom => Axis::Rows,
            _ => Axis::Columns,
        }
    }

    /// Whether the docked side is the root's first (left/top) child.
    fn leading(self) -> bool {
        matches!(
            self,
            Self::Left | Self::Top | Self::TopLeft | Self::BottomLeft
        )
    }

    /// For quarters, whether the window takes the top of its column.
    fn vertical(self) -> Option<bool> {
        match self {
            Self::TopLeft | Self::TopRight => Some(true),
            Self::BottomLeft | Self::BottomRight => Some(false),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Axis {
    Columns,
    Rows,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
enum Node {
    Window(u64),
    Split {
        axis: Axis,
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutError {
    InvalidOptions,
    InvalidArea,
    InsufficientSpace,
    DuplicateWindow,
    MissingWindow,
    NoSplit,
}

/// Ratios are the fraction allocated to the first (left/top) child, 0.1..=0.9.
/// With preserve_split=false, each layout chooses axes from current node shapes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dwindle {
    root: Option<Node>,
    default_split_ratio: f64,
    split_width_multiplier: f64,
    preserve_split: bool,
    preselection: Option<Direction>,
}

impl Default for Dwindle {
    fn default() -> Self {
        Self::new(0.5, 1.0, true).expect("valid defaults")
    }
}

impl Dwindle {
    /// Validate persisted trees before using them, including unique membership.
    pub fn validate(&self) -> bool {
        let mut ids = std::collections::HashSet::new();
        fn walk(node: &Node, ids: &mut std::collections::HashSet<u64>, depth: usize) -> bool {
            if depth > 64 || ids.len() > 1024 {
                return false;
            }
            match node {
                Node::Window(id) => ids.insert(*id),
                Node::Split {
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    valid_ratio(*ratio)
                        && walk(first, ids, depth + 1)
                        && walk(second, ids, depth + 1)
                }
            }
        }
        Self::new(
            self.default_split_ratio,
            self.split_width_multiplier,
            self.preserve_split,
        )
        .is_ok()
            && self
                .root
                .as_ref()
                .is_none_or(|node| walk(node, &mut ids, 0))
    }

    /// Grow/shrink the selected child's share at its immediate parent.
    pub fn grow(&mut self, window: u64, delta: f64) -> Result<(), LayoutError> {
        if !delta.is_finite() {
            return Err(LayoutError::InvalidOptions);
        }
        let Node::Split { ratio, first, .. } = self.parent_mut(window)? else {
            unreachable!()
        };
        *ratio = (*ratio
            + if first.contains(window) {
                delta
            } else {
                -delta
            })
        .clamp(0.1, 0.9);
        Ok(())
    }

    /// Move the nearest matching ancestor boundary. Positive deltas move right/down.
    pub fn move_boundary(
        &mut self,
        window: u64,
        vertical: bool,
        delta: f64,
        area: Rect,
    ) -> Result<(), LayoutError> {
        if !delta.is_finite() {
            return Err(LayoutError::InvalidOptions);
        }
        self.layout(area, SnapGaps { inner: 0, outer: 0 })?;
        fn resize(
            node: &mut Node,
            window: u64,
            axis: Axis,
            delta: f64,
            rect: Rect,
            options: (bool, f64),
        ) -> bool {
            let Node::Split {
                axis: current,
                ratio,
                first,
                second,
            } = node
            else {
                return false;
            };
            let current = if options.0 {
                *current
            } else {
                choose_axis(rect, options.1)
            };
            let extent = if current == Axis::Columns {
                rect.width
            } else {
                rect.height
            };
            let size = ((f64::from(extent) * *ratio).round() as i32).clamp(1, extent - 1);
            let mut child_rect = rect;
            let child = if first.contains(window) {
                if current == Axis::Columns {
                    child_rect.width = size;
                } else {
                    child_rect.height = size;
                }
                first
            } else if second.contains(window) {
                if current == Axis::Columns {
                    child_rect.x += size;
                    child_rect.width -= size;
                } else {
                    child_rect.y += size;
                    child_rect.height -= size;
                }
                second
            } else {
                return false;
            };
            if resize(child, window, axis, delta, child_rect, options) {
                return true;
            }
            if current != axis {
                return false;
            }
            *ratio = (*ratio + delta).clamp(0.1, 0.9);
            true
        }
        if !self.contains(window) {
            return Err(LayoutError::MissingWindow);
        }
        if self.root.as_mut().is_some_and(|root| {
            resize(
                root,
                window,
                if vertical { Axis::Rows } else { Axis::Columns },
                delta,
                area,
                (self.preserve_split, self.split_width_multiplier),
            )
        }) {
            Ok(())
        } else {
            Err(LayoutError::NoSplit)
        }
    }
    pub fn new(
        ratio: f64,
        width_multiplier: f64,
        preserve_split: bool,
    ) -> Result<Self, LayoutError> {
        if !valid_ratio(ratio) || !width_multiplier.is_finite() || width_multiplier <= 0.0 {
            return Err(LayoutError::InvalidOptions);
        }
        Ok(Self {
            root: None,
            default_split_ratio: ratio,
            split_width_multiplier: width_multiplier,
            preserve_split,
            preselection: None,
        })
    }

    /// Consumed by the next successful split; the direction is the new tile's side.
    pub fn preselect(&mut self, direction: Option<Direction>) {
        self.preselection = direction;
    }

    pub fn contains(&self, window: u64) -> bool {
        self.root.as_ref().is_some_and(|node| node.contains(window))
    }

    /// Split the focused tile, or the last tile if focus is outside this tree.
    /// Failed insertion leaves the tree and preselection unchanged.
    pub fn insert(
        &mut self,
        window: u64,
        focused: Option<u64>,
        area: Rect,
    ) -> Result<(), LayoutError> {
        if self.contains(window) {
            return Err(LayoutError::DuplicateWindow);
        }
        let tiles = self.layout(area, SnapGaps { inner: 0, outer: 0 })?;
        let target = focused
            .and_then(|id| tiles.iter().find(|(window, _)| *window == id))
            .or_else(|| tiles.last());
        let mut candidate = self.clone();
        if let Some(&(id, rect)) = target {
            let axis = match self.preselection {
                Some(Direction::Left | Direction::Right) => Axis::Columns,
                Some(Direction::Up | Direction::Down) => Axis::Rows,
                None => choose_axis(rect, self.split_width_multiplier),
            };
            let (first, second) =
                if matches!(self.preselection, Some(Direction::Left | Direction::Up)) {
                    (Node::Window(window), Node::Window(id))
                } else {
                    (Node::Window(id), Node::Window(window))
                };
            *candidate.root.as_mut().unwrap().leaf_mut(id).unwrap() = Node::Split {
                axis,
                ratio: self.default_split_ratio,
                first: Box::new(first),
                second: Box::new(second),
            };
            candidate.preselection = None;
        } else {
            candidate.root = Some(Node::Window(window));
        }
        candidate.layout(area, SnapGaps { inner: 0, outer: 0 })?;
        *self = candidate;
        Ok(())
    }

    /// Remove a closed, minimized or floated window, collapsing its parent.
    pub fn remove(&mut self, window: u64) -> bool {
        if !self.contains(window) {
            return false;
        }
        self.root = self.root.take().and_then(|node| node.remove(window));
        true
    }

    pub fn swap(&mut self, a: u64, b: u64) -> Result<(), LayoutError> {
        if !self.contains(a) || !self.contains(b) {
            return Err(LayoutError::MissingWindow);
        }
        self.root.as_mut().unwrap().swap(a, b);
        Ok(())
    }

    /// The window's share of the docked side when the root already docks it there.
    pub fn docked_share(&self, window: u64, dock: Dock) -> Option<f64> {
        let Some(Node::Split {
            axis,
            ratio,
            first,
            second,
        }) = &self.root
        else {
            return None;
        };
        let (side_first, side) = if dock.leading() {
            (true, first)
        } else {
            (false, second)
        };
        let share = if side_first { *ratio } else { 1.0 - *ratio };
        let alone = matches!(**side, Node::Window(w) if w == window);
        let ok = *axis == dock.axis()
            && match dock.vertical() {
                None => alone,
                // A quarter docked without a partner tile is a half.
                Some(top) => {
                    alone
                        || matches!(&**side, Node::Split { axis: Axis::Rows, first, second, .. }
                    if matches!(if top { &**first } else { &**second }, Node::Window(w) if *w == window))
                }
            };
        ok.then_some(share)
    }

    /// Rebuild the root so `window` occupies the snap position with `share` of the
    /// docked axis, and every other tile fills the remaining space. A quarter pairs
    /// the window with the tile nearest the rest of its column; with no spare tile
    /// it docks the half instead. Requires preserve_split so axes stay as placed.
    pub fn dock(
        &mut self,
        window: u64,
        dock: Dock,
        share: f64,
        area: Rect,
    ) -> Result<(), LayoutError> {
        if !self.preserve_split || !share.is_finite() {
            return Err(LayoutError::InvalidOptions);
        }
        let share = share.clamp(0.1, 0.9);
        let mut candidate = self.clone();
        candidate.remove(window);
        let Some(mut rest) = candidate.root.take() else {
            return Err(LayoutError::MissingWindow);
        };
        let mut side = Node::Window(window);
        if let Some(top) = dock.vertical()
            && matches!(rest, Node::Split { .. })
        {
            // Pick the remaining tile closest to the other half of the docked column.
            let mut column = area;
            column.width /= 2;
            if !dock.leading() {
                column.x += area.width - column.width;
            }
            let target = (
                i64::from(column.x) + i64::from(column.width) / 2,
                i64::from(column.y)
                    + if top {
                        i64::from(column.height) * 3 / 4
                    } else {
                        i64::from(column.height) / 4
                    },
            );
            let tiles = Self {
                root: Some(rest.clone()),
                ..self.clone()
            }
            .layout(area, SnapGaps { inner: 0, outer: 0 })?;
            let partner = tiles
                .iter()
                .min_by_key(|(_, r)| {
                    let cx = i64::from(r.x) + i64::from(r.width) / 2;
                    let cy = i64::from(r.y) + i64::from(r.height) / 2;
                    (cx - target.0).pow(2) + (cy - target.1).pow(2)
                })
                .map(|(id, _)| *id)
                .ok_or(LayoutError::MissingWindow)?;
            rest = rest.remove(partner).ok_or(LayoutError::MissingWindow)?;
            let (first, second) = if top {
                (Node::Window(window), Node::Window(partner))
            } else {
                (Node::Window(partner), Node::Window(window))
            };
            side = Node::Split {
                axis: Axis::Rows,
                ratio: 0.5,
                first: Box::new(first),
                second: Box::new(second),
            };
        }
        let (first, second, ratio) = if dock.leading() {
            (side, rest, share)
        } else {
            (rest, side, 1.0 - share)
        };
        candidate.root = Some(Node::Split {
            axis: dock.axis(),
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        });
        candidate.layout(area, SnapGaps { inner: 0, outer: 0 })?;
        *self = candidate;
        Ok(())
    }

    /// Toggle the immediate parent. Requires preserve_split to retain the change.
    pub fn toggle_split(&mut self, window: u64) -> Result<(), LayoutError> {
        let Node::Split { axis, .. } = self.parent_mut(window)? else {
            unreachable!()
        };
        *axis = match axis {
            Axis::Columns => Axis::Rows,
            Axis::Rows => Axis::Columns,
        };
        Ok(())
    }

    /// Set the immediate parent's first-child fraction, not a window-relative delta.
    pub fn set_split_ratio(&mut self, window: u64, ratio: f64) -> Result<(), LayoutError> {
        if !valid_ratio(ratio) {
            return Err(LayoutError::InvalidOptions);
        }
        let Node::Split { ratio: current, .. } = self.parent_mut(window)? else {
            unreachable!()
        };
        *current = ratio;
        Ok(())
    }

    fn parent_mut(&mut self, window: u64) -> Result<&mut Node, LayoutError> {
        if !self.contains(window) {
            return Err(LayoutError::MissingWindow);
        }
        self.root
            .as_mut()
            .and_then(|node| node.parent_mut(window))
            .ok_or(LayoutError::NoSplit)
    }

    /// Return deterministic tree order. Reject unrepresentable geometry instead
    /// of emitting zero-size or overlapping tiles when the work area is too small.
    pub fn layout(&self, area: Rect, gaps: SnapGaps) -> Result<Vec<(u64, Rect)>, LayoutError> {
        if area.width <= 0
            || area.height <= 0
            || area.x.checked_add(area.width).is_none()
            || area.y.checked_add(area.height).is_none()
        {
            return Err(LayoutError::InvalidArea);
        }
        let mut tiles = Vec::new();
        if let Some(root) = &self.root {
            self.visit(root, area, &mut tiles)?;
        }
        for (_, rect) in &mut tiles {
            *rect = gaps.inset(area, *rect);
        }
        Ok(tiles)
    }

    fn visit(
        &self,
        node: &Node,
        rect: Rect,
        tiles: &mut Vec<(u64, Rect)>,
    ) -> Result<(), LayoutError> {
        match node {
            Node::Window(id) => tiles.push((*id, rect)),
            Node::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let axis = if self.preserve_split {
                    *axis
                } else {
                    choose_axis(rect, self.split_width_multiplier)
                };
                let extent = match axis {
                    Axis::Columns => rect.width,
                    Axis::Rows => rect.height,
                };
                if extent < 2 {
                    return Err(LayoutError::InsufficientSpace);
                }
                let size = ((f64::from(extent) * ratio).round() as i32).clamp(1, extent - 1);
                let (mut a, mut b) = (rect, rect);
                match axis {
                    Axis::Columns => {
                        a.width = size;
                        b.x += size;
                        b.width -= size;
                    }
                    Axis::Rows => {
                        a.height = size;
                        b.y += size;
                        b.height -= size;
                    }
                }
                self.visit(first, a, tiles)?;
                self.visit(second, b, tiles)?;
            }
        }
        Ok(())
    }
}

fn valid_ratio(ratio: f64) -> bool {
    (0.1..=0.9).contains(&ratio)
}

fn choose_axis(rect: Rect, multiplier: f64) -> Axis {
    if f64::from(rect.width) > f64::from(rect.height) * multiplier {
        Axis::Columns
    } else {
        Axis::Rows
    }
}

impl Node {
    fn contains(&self, id: u64) -> bool {
        match self {
            Self::Window(window) => *window == id,
            Self::Split { first, second, .. } => first.contains(id) || second.contains(id),
        }
    }

    fn leaf_mut(&mut self, id: u64) -> Option<&mut Self> {
        match self {
            Self::Window(window) => (*window == id).then_some(self),
            Self::Split { first, second, .. } => first.leaf_mut(id).or_else(|| second.leaf_mut(id)),
        }
    }

    fn parent_mut(&mut self, id: u64) -> Option<&mut Self> {
        if let Self::Split { first, second, .. } = self
            && (matches!(**first, Self::Window(w) if w == id)
                || matches!(**second, Self::Window(w) if w == id))
        {
            return Some(self);
        }
        match self {
            Self::Split { first, second, .. } => {
                first.parent_mut(id).or_else(|| second.parent_mut(id))
            }
            _ => None,
        }
    }

    fn remove(self, id: u64) -> Option<Self> {
        match self {
            Self::Window(window) => (window != id).then_some(Self::Window(window)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.remove(id), second.remove(id)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (remaining, None) | (None, remaining) => remaining,
            },
        }
    }

    fn swap(&mut self, a: u64, b: u64) {
        match self {
            Self::Window(id) => {
                if *id == a {
                    *id = b;
                } else if *id == b {
                    *id = a;
                }
            }
            Self::Split { first, second, .. } => {
                first.swap(a, b);
                second.swap(a, b);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect {
        x: -1200,
        y: -100,
        width: 1201,
        height: 801,
    };
    const ZERO: SnapGaps = SnapGaps { inner: 0, outer: 0 };

    fn three() -> Dwindle {
        let mut tree = Dwindle::default();
        tree.insert(1, None, AREA).unwrap();
        tree.insert(2, Some(1), AREA).unwrap();
        tree.insert(3, Some(2), AREA).unwrap();
        tree
    }

    #[test]
    fn docking_places_the_window_and_tiles_the_rest_around_it() {
        let area = Rect {
            x: 0,
            y: 0,
            width: 1200,
            height: 800,
        };
        let rect = |tree: &Dwindle, id| {
            tree.layout(area, ZERO)
                .unwrap()
                .into_iter()
                .find(|(t, _)| *t == id)
                .unwrap()
                .1
        };
        // Halves: 1 takes the right half, 2 and 3 share the left half.
        let mut tree = three();
        tree.dock(1, Dock::Right, 0.5, area).unwrap();
        assert_eq!(
            rect(&tree, 1),
            Rect {
                x: 600,
                y: 0,
                width: 600,
                height: 800
            }
        );
        for id in [2, 3] {
            let r = rect(&tree, id);
            assert!(r.x + r.width <= 600);
        }
        assert_eq!(tree.docked_share(1, Dock::Right), Some(0.5));
        assert_eq!(tree.docked_share(1, Dock::Left), None);
        tree.dock(1, Dock::Right, 0.6, area).unwrap();
        assert_eq!(rect(&tree, 1).width, 720);
        tree.dock(1, Dock::Bottom, 0.5, area).unwrap();
        assert_eq!(
            rect(&tree, 1),
            Rect {
                x: 0,
                y: 400,
                width: 1200,
                height: 400
            }
        );
        // Quarter: 1 is top-left, its nearest neighbor fills bottom-left, the rest the right.
        tree.dock(1, Dock::TopLeft, 0.5, area).unwrap();
        assert_eq!(
            rect(&tree, 1),
            Rect {
                x: 0,
                y: 0,
                width: 600,
                height: 400
            }
        );
        assert_eq!(tree.layout(area, ZERO).unwrap().len(), 3);
        assert!(tree.docked_share(1, Dock::TopLeft).is_some());
        // With one other tile, a quarter docks the half instead.
        let mut pair = Dwindle::default();
        pair.insert(1, None, area).unwrap();
        pair.insert(2, Some(1), area).unwrap();
        pair.dock(2, Dock::TopLeft, 0.5, area).unwrap();
        assert_eq!(
            rect(&pair, 2),
            Rect {
                x: 0,
                y: 0,
                width: 600,
                height: 800
            }
        );
        assert_eq!(pair.docked_share(2, Dock::TopLeft), Some(0.5));
        // A lone window has nothing to arrange; adaptive axes cannot hold a dock.
        let mut lone = Dwindle::default();
        lone.insert(1, None, area).unwrap();
        assert_eq!(
            lone.dock(1, Dock::Left, 0.5, area),
            Err(LayoutError::MissingWindow)
        );
        let mut adaptive = Dwindle::new(0.5, 1.0, false).unwrap();
        adaptive.insert(1, None, area).unwrap();
        adaptive.insert(2, None, area).unwrap();
        assert_eq!(
            adaptive.dock(1, Dock::Top, 0.5, area),
            Err(LayoutError::InvalidOptions)
        );
    }

    #[test]
    fn persisted_tree_and_window_relative_growth_validate() {
        let mut tree = three();
        tree.grow(3, 0.1).unwrap();
        let tiles = tree.layout(AREA, ZERO).unwrap();
        assert!(tiles[2].1.height > tiles[1].1.height);
        tree.move_boundary(3, true, -0.05, AREA).unwrap();
        assert!(tree.layout(AREA, ZERO).unwrap()[2].1.height > tiles[2].1.height);
        let json = serde_json::to_string(&tree).unwrap();
        let loaded: Dwindle = serde_json::from_str(&json).unwrap();
        assert!(loaded.validate());
        assert_eq!(tree, loaded);
        let mut corrupt = serde_json::to_value(&tree).unwrap();
        corrupt["root"]["Split"]["ratio"] = serde_json::json!(2.0);
        assert!(
            !serde_json::from_value::<Dwindle>(corrupt)
                .unwrap()
                .validate()
        );
        let mut adaptive = Dwindle::new(0.5, 1.0, false).unwrap();
        adaptive.insert(1, None, AREA).unwrap();
        adaptive.insert(2, None, AREA).unwrap();
        let portrait = Rect {
            width: 400,
            height: 1000,
            ..AREA
        };
        adaptive.move_boundary(1, true, 0.1, portrait).unwrap();
        assert_eq!(adaptive.layout(portrait, ZERO).unwrap()[0].1.height, 600);
    }

    #[test]
    fn focused_insertion_partitions_odd_negative_origin_area() {
        assert_eq!(
            three().layout(AREA, ZERO).unwrap(),
            vec![
                (
                    1,
                    Rect {
                        x: -1200,
                        y: -100,
                        width: 601,
                        height: 801
                    }
                ),
                (
                    2,
                    Rect {
                        x: -599,
                        y: -100,
                        width: 600,
                        height: 401
                    }
                ),
                (
                    3,
                    Rect {
                        x: -599,
                        y: 301,
                        width: 600,
                        height: 400
                    }
                ),
            ]
        );
    }

    #[test]
    fn removal_promotes_sibling_then_empties_tree() {
        let mut tree = three();
        assert!(tree.remove(2));
        assert_eq!(
            tree.layout(AREA, ZERO).unwrap()[1],
            (
                3,
                Rect {
                    x: -599,
                    y: -100,
                    width: 600,
                    height: 801
                }
            )
        );
        assert!(tree.remove(1));
        assert_eq!(tree.layout(AREA, ZERO).unwrap(), vec![(3, AREA)]);
        assert!(!tree.remove(99));
        assert!(tree.remove(3));
        assert!(tree.layout(AREA, ZERO).unwrap().is_empty());
    }

    #[test]
    fn swaps_preserve_geometry_and_ratios_target_nearest_parent() {
        let mut tree = three();
        let before = tree.layout(AREA, ZERO).unwrap();
        tree.swap(1, 3).unwrap();
        let after = tree.layout(AREA, ZERO).unwrap();
        assert_eq!(after[0], (3, before[0].1));
        assert_eq!(after[2], (1, before[2].1));
        tree.set_split_ratio(1, 0.25).unwrap();
        let after = tree.layout(AREA, ZERO).unwrap();
        assert_eq!(after[0], (3, before[0].1));
        assert_eq!(after[1].1.height, 200);
        assert_eq!(after[2].1.height, 601);
        tree.toggle_split(1).unwrap();
        let after = tree.layout(AREA, ZERO).unwrap();
        assert_eq!(after[1].1.width, 150);
        assert_eq!(after[2].1.width, 450);
    }

    #[test]
    fn preselection_is_one_shot_and_controls_new_tile_side() {
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            let mut tree = Dwindle::default();
            tree.preselect(Some(direction));
            tree.insert(1, None, AREA).unwrap();
            tree.insert(2, Some(1), AREA).unwrap();
            assert_eq!(tree.preselection, None);
            let tiles = tree.layout(AREA, ZERO).unwrap();
            let new = tiles.iter().find(|(id, _)| *id == 2).unwrap().1;
            match direction {
                Direction::Left => assert_eq!((new.x, new.width), (AREA.x, 601)),
                Direction::Right => assert_eq!((new.x, new.width), (-599, 600)),
                Direction::Up => assert_eq!((new.y, new.height), (AREA.y, 401)),
                Direction::Down => assert_eq!((new.y, new.height), (301, 400)),
            }
        }
    }

    #[test]
    fn applies_outer_and_inner_gaps_once() {
        let tree = three();
        let tiles = tree.layout(AREA, SnapGaps::default()).unwrap();
        assert_eq!(
            tiles[0].1,
            Rect {
                x: -1180,
                y: -80,
                width: 576,
                height: 761
            }
        );
        assert_eq!(
            tiles[1].1,
            Rect {
                x: -594,
                y: -80,
                width: 575,
                height: 376
            }
        );
        assert_eq!(
            tiles[2].1,
            Rect {
                x: -594,
                y: 306,
                width: 575,
                height: 375
            }
        );
    }

    #[test]
    fn invalid_operations_do_not_mutate_tree() {
        let mut tree = three();
        tree.preselect(Some(Direction::Left));
        let before = tree.clone();
        assert_eq!(
            tree.insert(1, None, AREA),
            Err(LayoutError::DuplicateWindow)
        );
        assert_eq!(tree.swap(1, 99), Err(LayoutError::MissingWindow));
        assert_eq!(tree.toggle_split(99), Err(LayoutError::MissingWindow));
        for ratio in [f64::NAN, f64::INFINITY, 0.0, 1.0] {
            assert_eq!(
                tree.set_split_ratio(1, ratio),
                Err(LayoutError::InvalidOptions)
            );
            assert_eq!(
                Dwindle::new(ratio, 1.0, true),
                Err(LayoutError::InvalidOptions)
            );
        }
        assert_eq!(tree, before);
        let tiny = Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let mut tree = Dwindle::default();
        tree.insert(1, None, tiny).unwrap();
        tree.preselect(Some(Direction::Right));
        let before = tree.clone();
        assert_eq!(
            tree.insert(2, Some(1), tiny),
            Err(LayoutError::InsufficientSpace)
        );
        assert_eq!(tree, before);
        assert_eq!(tree.toggle_split(1), Err(LayoutError::NoSplit));
        assert_eq!(
            tree.layout(Rect { width: 0, ..tiny }, ZERO),
            Err(LayoutError::InvalidArea)
        );
        assert_eq!(
            tree.layout(
                Rect {
                    x: i32::MAX,
                    ..tiny
                },
                ZERO
            ),
            Err(LayoutError::InvalidArea)
        );
    }

    #[test]
    fn shape_changes_respect_preserve_split_and_multiplier() {
        let portrait = Rect {
            width: 400,
            height: 1000,
            ..AREA
        };
        let mut fixed = Dwindle::default();
        fixed.insert(1, None, AREA).unwrap();
        fixed.insert(2, None, AREA).unwrap();
        assert_eq!(fixed.layout(portrait, ZERO).unwrap()[0].1.width, 200);
        let mut adaptive = Dwindle::new(0.5, 1.0, false).unwrap();
        adaptive.insert(1, None, AREA).unwrap();
        adaptive.insert(2, None, AREA).unwrap();
        assert_eq!(adaptive.layout(portrait, ZERO).unwrap()[0].1.height, 500);
        let mut wide_threshold = Dwindle::new(0.5, 2.0, true).unwrap();
        wide_threshold.insert(1, None, AREA).unwrap();
        wide_threshold.insert(2, None, AREA).unwrap();
        assert_eq!(wide_threshold.layout(AREA, ZERO).unwrap()[0].1.height, 401);
        for multiplier in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                Dwindle::new(0.5, multiplier, true),
                Err(LayoutError::InvalidOptions)
            );
        }
    }

    #[test]
    fn many_inserts_and_removals_cover_area_without_overlap() {
        let area = Rect {
            x: -2000,
            y: -900,
            width: 2003,
            height: 1001,
        };
        let mut tree = Dwindle::default();
        for id in 0..40 {
            tree.insert(id, Some(id / 2), area).unwrap();
        }
        for id in (0..40).step_by(3) {
            tree.remove(id);
        }
        let tiles = tree.layout(area, ZERO).unwrap();
        assert_eq!(tiles.len(), 26);
        assert_eq!(
            tiles
                .iter()
                .map(|(_, r)| i64::from(r.width) * i64::from(r.height))
                .sum::<i64>(),
            i64::from(area.width) * i64::from(area.height)
        );
        for (index, (_, a)) in tiles.iter().enumerate() {
            assert!(a.width > 0 && a.height > 0);
            for (_, b) in &tiles[index + 1..] {
                assert!(
                    a.x + a.width <= b.x
                        || b.x + b.width <= a.x
                        || a.y + a.height <= b.y
                        || b.y + b.height <= a.y
                );
            }
        }
    }
}
