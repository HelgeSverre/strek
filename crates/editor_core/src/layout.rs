//! Auto-layout system for automatic child positioning.
//!
//! This module provides types and algorithms for automatic layout of child nodes
//! within a group, similar to flexbox-style layouts.

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::node::NodeId;
use crate::path::Rect;

/// Auto-layout configuration for a group node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoLayout {
    /// Stack direction (horizontal or vertical)
    pub direction: Direction,

    /// Space between children
    pub spacing: f32,

    /// Inner padding
    pub padding: Edges,

    /// Main axis alignment
    pub align_main: AlignMain,

    /// Cross axis alignment
    pub align_cross: AlignCross,
}

impl Default for AutoLayout {
    fn default() -> Self {
        Self {
            direction: Direction::Horizontal,
            spacing: 0.0,
            padding: Edges::default(),
            align_main: AlignMain::Start,
            align_cross: AlignCross::Start,
        }
    }
}

impl AutoLayout {
    /// Create a new horizontal auto-layout.
    pub fn horizontal() -> Self {
        Self {
            direction: Direction::Horizontal,
            ..Default::default()
        }
    }

    /// Create a new vertical auto-layout.
    pub fn vertical() -> Self {
        Self {
            direction: Direction::Vertical,
            ..Default::default()
        }
    }

    /// Set the spacing between children.
    pub fn with_spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Set the padding.
    pub fn with_padding(mut self, padding: Edges) -> Self {
        self.padding = padding;
        self
    }

    /// Set uniform padding on all sides.
    pub fn with_uniform_padding(mut self, padding: f32) -> Self {
        self.padding = Edges::uniform(padding);
        self
    }

    /// Set main axis alignment.
    pub fn with_align_main(mut self, align: AlignMain) -> Self {
        self.align_main = align;
        self
    }

    /// Set cross axis alignment.
    pub fn with_align_cross(mut self, align: AlignCross) -> Self {
        self.align_cross = align;
        self
    }
}

/// Stack direction for auto-layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Direction {
    /// Children are arranged horizontally (left to right)
    #[default]
    Horizontal,

    /// Children are arranged vertically (top to bottom)
    Vertical,
}

/// Main axis alignment (along the direction of the stack).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlignMain {
    /// Align to the start of the container
    #[default]
    Start,

    /// Center in the container
    Center,

    /// Align to the end of the container
    End,

    /// Distribute space between children
    SpaceBetween,
}

/// Cross axis alignment (perpendicular to the stack direction).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AlignCross {
    /// Align to the start of the cross axis
    #[default]
    Start,

    /// Center on the cross axis
    Center,

    /// Align to the end of the cross axis
    End,

    /// Stretch to fill the cross axis (future: requires size constraints)
    Stretch,
}

/// Padding/margin edges.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Edges {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Edges {
    /// Create uniform padding on all sides.
    pub fn uniform(value: f32) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }

    /// Create symmetric padding (horizontal, vertical).
    pub fn symmetric(horizontal: f32, vertical: f32) -> Self {
        Self {
            left: horizontal,
            top: vertical,
            right: horizontal,
            bottom: vertical,
        }
    }

    /// Create padding with individual values.
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// Total horizontal padding.
    pub fn horizontal(&self) -> f32 {
        self.left + self.right
    }

    /// Total vertical padding.
    pub fn vertical(&self) -> f32 {
        self.top + self.bottom
    }
}

/// Computed layout offset for a child node.
/// This is separate from the node's author transform.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LayoutOffset {
    /// Translation offset computed by the layout engine
    pub offset: Vec2,
}

/// Result of measuring a node for layout.
#[derive(Debug, Clone, Copy)]
pub struct MeasuredSize {
    /// Width of the content
    pub width: f32,

    /// Height of the content
    pub height: f32,
}

impl MeasuredSize {
    pub fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    pub fn from_rect(rect: Rect) -> Self {
        Self {
            width: rect.width(),
            height: rect.height(),
        }
    }

    /// Get as Vec2
    pub fn as_vec2(&self) -> Vec2 {
        Vec2::new(self.width, self.height)
    }

    /// Main axis size for the given direction
    pub fn main(&self, direction: Direction) -> f32 {
        match direction {
            Direction::Horizontal => self.width,
            Direction::Vertical => self.height,
        }
    }

    /// Cross axis size for the given direction
    pub fn cross(&self, direction: Direction) -> f32 {
        match direction {
            Direction::Horizontal => self.height,
            Direction::Vertical => self.width,
        }
    }
}

/// Entry for layout computation.
pub struct LayoutEntry {
    pub id: NodeId,
    pub size: MeasuredSize,
}

/// Compute layout positions for children.
///
/// Returns a list of (NodeId, LayoutOffset) pairs.
pub fn compute_layout(
    layout: &AutoLayout,
    children: &[LayoutEntry],
    container_size: Option<MeasuredSize>,
) -> Vec<(NodeId, LayoutOffset)> {
    if children.is_empty() {
        return vec![];
    }

    let direction = layout.direction;
    let spacing = layout.spacing;
    let padding = &layout.padding;

    // Calculate total content size along main axis
    let total_main: f32 = children.iter().map(|c| c.size.main(direction)).sum();
    let total_spacing = spacing * (children.len().saturating_sub(1)) as f32;
    let content_main = total_main + total_spacing;

    // Calculate max cross size
    let max_cross: f32 = children
        .iter()
        .map(|c| c.size.cross(direction))
        .fold(0.0f32, f32::max);

    // Determine container size (use content size if not specified)
    let container = container_size.unwrap_or(MeasuredSize::new(
        match direction {
            Direction::Horizontal => content_main + padding.horizontal(),
            Direction::Vertical => max_cross + padding.horizontal(),
        },
        match direction {
            Direction::Horizontal => max_cross + padding.vertical(),
            Direction::Vertical => content_main + padding.vertical(),
        },
    ));

    // Available space for main axis
    let available_main = match direction {
        Direction::Horizontal => container.width - padding.horizontal(),
        Direction::Vertical => container.height - padding.vertical(),
    };

    // Calculate starting position and spacing based on main alignment
    let (mut main_pos, extra_spacing) = match layout.align_main {
        AlignMain::Start => {
            let start = match direction {
                Direction::Horizontal => padding.left,
                Direction::Vertical => padding.top,
            };
            (start, 0.0)
        }
        AlignMain::Center => {
            let start = match direction {
                Direction::Horizontal => padding.left + (available_main - content_main) / 2.0,
                Direction::Vertical => padding.top + (available_main - content_main) / 2.0,
            };
            (start.max(0.0), 0.0)
        }
        AlignMain::End => {
            let start = match direction {
                Direction::Horizontal => padding.left + available_main - content_main,
                Direction::Vertical => padding.top + available_main - content_main,
            };
            (start.max(0.0), 0.0)
        }
        AlignMain::SpaceBetween => {
            let start = match direction {
                Direction::Horizontal => padding.left,
                Direction::Vertical => padding.top,
            };
            if children.len() > 1 {
                let extra = (available_main - total_main) / (children.len() - 1) as f32;
                (start, extra.max(0.0))
            } else {
                (start, 0.0)
            }
        }
    };

    // Available space for cross axis
    let available_cross = match direction {
        Direction::Horizontal => container.height - padding.vertical(),
        Direction::Vertical => container.width - padding.horizontal(),
    };

    // Position each child
    let mut results = Vec::with_capacity(children.len());

    for entry in children {
        // Calculate cross position
        let cross_pos = match layout.align_cross {
            AlignCross::Start => match direction {
                Direction::Horizontal => padding.top,
                Direction::Vertical => padding.left,
            },
            AlignCross::Center => {
                let base = match direction {
                    Direction::Horizontal => padding.top,
                    Direction::Vertical => padding.left,
                };
                base + (available_cross - entry.size.cross(direction)) / 2.0
            }
            AlignCross::End => {
                let base = match direction {
                    Direction::Horizontal => padding.top,
                    Direction::Vertical => padding.left,
                };
                base + available_cross - entry.size.cross(direction)
            }
            AlignCross::Stretch => {
                // For now, treat same as Start (stretching requires modifying the child)
                match direction {
                    Direction::Horizontal => padding.top,
                    Direction::Vertical => padding.left,
                }
            }
        };

        // Create offset based on direction
        let offset = match direction {
            Direction::Horizontal => Vec2::new(main_pos, cross_pos),
            Direction::Vertical => Vec2::new(cross_pos, main_pos),
        };

        results.push((entry.id, LayoutOffset { offset }));

        // Advance main position
        main_pos += entry.size.main(direction);
        if layout.align_main == AlignMain::SpaceBetween {
            main_pos += extra_spacing;
        } else {
            main_pos += spacing;
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::NodeId;
    use slotmap::SlotMap;

    fn make_id() -> NodeId {
        let mut map: SlotMap<NodeId, ()> = SlotMap::with_key();
        map.insert(())
    }

    fn make_entries(sizes: &[(f32, f32)]) -> Vec<LayoutEntry> {
        sizes
            .iter()
            .map(|&(w, h)| LayoutEntry {
                id: make_id(),
                size: MeasuredSize::new(w, h),
            })
            .collect()
    }

    #[test]
    fn test_horizontal_start() {
        let layout = AutoLayout::horizontal().with_spacing(10.0);
        let entries = make_entries(&[(50.0, 30.0), (60.0, 40.0), (40.0, 20.0)]);

        let results = compute_layout(&layout, &entries, None);

        assert_eq!(results.len(), 3);

        // First child at (0, 0)
        assert!((results[0].1.offset.x - 0.0).abs() < 0.001);
        assert!((results[0].1.offset.y - 0.0).abs() < 0.001);

        // Second child at (50 + 10 = 60, 0)
        assert!((results[1].1.offset.x - 60.0).abs() < 0.001);
        assert!((results[1].1.offset.y - 0.0).abs() < 0.001);

        // Third child at (60 + 60 + 10 = 130, 0)
        assert!((results[2].1.offset.x - 130.0).abs() < 0.001);
        assert!((results[2].1.offset.y - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_vertical_start() {
        let layout = AutoLayout::vertical().with_spacing(5.0);
        let entries = make_entries(&[(100.0, 30.0), (80.0, 40.0)]);

        let results = compute_layout(&layout, &entries, None);

        assert_eq!(results.len(), 2);

        // First child at (0, 0)
        assert!((results[0].1.offset.x - 0.0).abs() < 0.001);
        assert!((results[0].1.offset.y - 0.0).abs() < 0.001);

        // Second child at (0, 30 + 5 = 35)
        assert!((results[1].1.offset.x - 0.0).abs() < 0.001);
        assert!((results[1].1.offset.y - 35.0).abs() < 0.001);
    }

    #[test]
    fn test_horizontal_center_cross() {
        let layout = AutoLayout::horizontal().with_align_cross(AlignCross::Center);
        let entries = make_entries(&[(50.0, 20.0), (50.0, 40.0)]);

        // Container height will be max cross = 40
        let results = compute_layout(&layout, &entries, None);

        // First child (height 20) should be centered in 40px: y = (40-20)/2 = 10
        assert!((results[0].1.offset.y - 10.0).abs() < 0.001);

        // Second child (height 40) should be at y = 0
        assert!((results[1].1.offset.y - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_with_padding() {
        let layout = AutoLayout::horizontal()
            .with_uniform_padding(10.0)
            .with_spacing(5.0);
        let entries = make_entries(&[(30.0, 20.0), (40.0, 20.0)]);

        let results = compute_layout(&layout, &entries, None);

        // First child at (padding.left, padding.top) = (10, 10)
        assert!((results[0].1.offset.x - 10.0).abs() < 0.001);
        assert!((results[0].1.offset.y - 10.0).abs() < 0.001);

        // Second child at (10 + 30 + 5 = 45, 10)
        assert!((results[1].1.offset.x - 45.0).abs() < 0.001);
        assert!((results[1].1.offset.y - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_main_center() {
        let layout = AutoLayout::horizontal().with_align_main(AlignMain::Center);
        let entries = make_entries(&[(20.0, 10.0), (20.0, 10.0)]);

        // Total content: 40px, container will be 40px, so center = start
        let container = MeasuredSize::new(100.0, 10.0);
        let results = compute_layout(&layout, &entries, Some(container));

        // Content is 40px, container is 100px
        // Offset = (100 - 40) / 2 = 30
        assert!((results[0].1.offset.x - 30.0).abs() < 0.001);
        assert!((results[1].1.offset.x - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_main_end() {
        let layout = AutoLayout::horizontal().with_align_main(AlignMain::End);
        let entries = make_entries(&[(20.0, 10.0), (30.0, 10.0)]);

        let container = MeasuredSize::new(100.0, 10.0);
        let results = compute_layout(&layout, &entries, Some(container));

        // Content is 50px, container is 100px
        // Start = 100 - 50 = 50
        assert!((results[0].1.offset.x - 50.0).abs() < 0.001);
        assert!((results[1].1.offset.x - 70.0).abs() < 0.001);
    }

    #[test]
    fn test_space_between() {
        let layout = AutoLayout::horizontal().with_align_main(AlignMain::SpaceBetween);
        let entries = make_entries(&[(20.0, 10.0), (20.0, 10.0), (20.0, 10.0)]);

        let container = MeasuredSize::new(100.0, 10.0);
        let results = compute_layout(&layout, &entries, Some(container));

        // Total content: 60px, extra space: 40px, gaps: 2, spacing: 20px each
        // Positions: 0, 20+20=40, 40+20+20=80
        assert!((results[0].1.offset.x - 0.0).abs() < 0.001);
        assert!((results[1].1.offset.x - 40.0).abs() < 0.001);
        assert!((results[2].1.offset.x - 80.0).abs() < 0.001);
    }

    #[test]
    fn test_edges() {
        let edges = Edges::symmetric(10.0, 5.0);
        assert_eq!(edges.horizontal(), 20.0);
        assert_eq!(edges.vertical(), 10.0);
    }

    #[test]
    fn test_measured_size_axes() {
        let size = MeasuredSize::new(100.0, 50.0);

        assert_eq!(size.main(Direction::Horizontal), 100.0);
        assert_eq!(size.cross(Direction::Horizontal), 50.0);

        assert_eq!(size.main(Direction::Vertical), 50.0);
        assert_eq!(size.cross(Direction::Vertical), 100.0);
    }
}
