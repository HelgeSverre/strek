//! Snapping system for precise alignment during editing.
//!
//! This module provides snap point generation and snap target finding
//! for aligning nodes during drag operations.

use glam::Vec2;

use crate::node::NodeId;
use crate::path::Rect;

/// The kind of snap point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapKind {
    /// Left edge of bounding box
    Left,
    /// Horizontal center of bounding box
    CenterX,
    /// Right edge of bounding box
    Right,
    /// Top edge of bounding box
    Top,
    /// Vertical center of bounding box
    CenterY,
    /// Bottom edge of bounding box
    Bottom,
    /// Corner point
    Corner,
    /// Text baseline
    Baseline,
}

impl SnapKind {
    /// Check if this is a horizontal snap (affects X position).
    pub fn is_horizontal(&self) -> bool {
        matches!(self, SnapKind::Left | SnapKind::CenterX | SnapKind::Right)
    }

    /// Check if this is a vertical snap (affects Y position).
    pub fn is_vertical(&self) -> bool {
        matches!(self, SnapKind::Top | SnapKind::CenterY | SnapKind::Bottom | SnapKind::Baseline)
    }
}

/// A snap point from a node.
#[derive(Debug, Clone, Copy)]
pub struct SnapPoint {
    /// The position of the snap point in world coordinates.
    pub position: Vec2,

    /// The kind of snap point.
    pub kind: SnapKind,

    /// The node this snap point belongs to (None for guides/grid).
    pub node_id: Option<NodeId>,
}

impl SnapPoint {
    /// Create a new snap point.
    pub fn new(position: Vec2, kind: SnapKind, node_id: Option<NodeId>) -> Self {
        Self {
            position,
            kind,
            node_id,
        }
    }

    /// Create snap points from a bounding box.
    pub fn from_bounds(bounds: &Rect, node_id: Option<NodeId>) -> Vec<Self> {
        let center = bounds.center();

        vec![
            // Edges
            Self::new(Vec2::new(bounds.min.x, center.y), SnapKind::Left, node_id),
            Self::new(Vec2::new(center.x, center.y), SnapKind::CenterX, node_id),
            Self::new(Vec2::new(bounds.max.x, center.y), SnapKind::Right, node_id),
            Self::new(Vec2::new(center.x, bounds.min.y), SnapKind::Top, node_id),
            Self::new(Vec2::new(center.x, center.y), SnapKind::CenterY, node_id),
            Self::new(Vec2::new(center.x, bounds.max.y), SnapKind::Bottom, node_id),
            // Corners
            Self::new(bounds.min, SnapKind::Corner, node_id),
            Self::new(Vec2::new(bounds.max.x, bounds.min.y), SnapKind::Corner, node_id),
            Self::new(Vec2::new(bounds.min.x, bounds.max.y), SnapKind::Corner, node_id),
            Self::new(bounds.max, SnapKind::Corner, node_id),
        ]
    }
}

/// Result of a snap operation.
#[derive(Debug, Clone, Copy)]
pub struct SnapResult {
    /// The snapped position.
    pub position: Vec2,

    /// The horizontal snap point that was matched (if any).
    pub snap_x: Option<SnapMatch>,

    /// The vertical snap point that was matched (if any).
    pub snap_y: Option<SnapMatch>,
}

impl SnapResult {
    /// Create a result with no snapping.
    pub fn none(position: Vec2) -> Self {
        Self {
            position,
            snap_x: None,
            snap_y: None,
        }
    }

    /// Check if any snapping occurred.
    pub fn has_snap(&self) -> bool {
        self.snap_x.is_some() || self.snap_y.is_some()
    }
}

/// A matched snap point.
#[derive(Debug, Clone, Copy)]
pub struct SnapMatch {
    /// The coordinate value that was snapped to.
    pub value: f32,

    /// The kind of snap that matched.
    pub kind: SnapKind,

    /// The source snap point that matched.
    pub source_kind: SnapKind,

    /// The node that was snapped to (None for guides/grid).
    pub target_node: Option<NodeId>,
}

/// Configuration for snapping behavior.
#[derive(Debug, Clone, Copy)]
pub struct SnapConfig {
    /// Whether snapping is enabled.
    pub enabled: bool,

    /// Snap threshold in screen pixels.
    pub threshold: f32,

    /// Snap to other nodes.
    pub snap_to_nodes: bool,

    /// Snap to grid (if grid is enabled).
    pub snap_to_grid: bool,

    /// Grid spacing (if snap_to_grid is enabled).
    pub grid_spacing: f32,
}

impl Default for SnapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 8.0,
            snap_to_nodes: true,
            snap_to_grid: false,
            grid_spacing: 10.0,
        }
    }
}

/// Snap engine for finding snap targets.
#[derive(Debug, Default)]
pub struct SnapEngine {
    /// Available snap targets from other nodes.
    targets: Vec<SnapPoint>,

    /// Configuration.
    pub config: SnapConfig,
}

impl SnapEngine {
    /// Create a new snap engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set snap targets from other nodes.
    pub fn set_targets(&mut self, targets: Vec<SnapPoint>) {
        self.targets = targets;
    }

    /// Clear snap targets.
    pub fn clear_targets(&mut self) {
        self.targets.clear();
    }

    /// Find snap for a set of source snap points being dragged.
    ///
    /// Returns the snapped position and snap matches.
    pub fn find_snap(&self, source_points: &[SnapPoint], drag_offset: Vec2, zoom: f32) -> SnapResult {
        if !self.config.enabled {
            return SnapResult::none(drag_offset);
        }

        let threshold = self.config.threshold / zoom;
        let mut best_snap_x: Option<(f32, SnapMatch)> = None; // (distance, match)
        let mut best_snap_y: Option<(f32, SnapMatch)> = None;

        // Check each source point against each target
        for source in source_points {
            let source_pos = source.position + drag_offset;

            for target in &self.targets {
                // Skip self-snapping
                if source.node_id.is_some() && source.node_id == target.node_id {
                    continue;
                }

                // Check horizontal snap (X alignment)
                if source.kind.is_horizontal() && target.kind.is_horizontal() {
                    let dist = (source_pos.x - target.position.x).abs();
                    if dist < threshold
                        && (best_snap_x.is_none() || dist < best_snap_x.unwrap().0)
                    {
                        best_snap_x = Some((
                            dist,
                            SnapMatch {
                                value: target.position.x,
                                kind: target.kind,
                                source_kind: source.kind,
                                target_node: target.node_id,
                            },
                        ));
                    }
                }

                // Check vertical snap (Y alignment)
                if source.kind.is_vertical() && target.kind.is_vertical() {
                    let dist = (source_pos.y - target.position.y).abs();
                    if dist < threshold
                        && (best_snap_y.is_none() || dist < best_snap_y.unwrap().0)
                    {
                        best_snap_y = Some((
                            dist,
                            SnapMatch {
                                value: target.position.y,
                                kind: target.kind,
                                source_kind: source.kind,
                                target_node: target.node_id,
                            },
                        ));
                    }
                }
            }
        }

        // Also check grid snapping
        if self.config.snap_to_grid {
            for source in source_points {
                let source_pos = source.position + drag_offset;

                // Snap X to grid
                let grid_x = (source_pos.x / self.config.grid_spacing).round() * self.config.grid_spacing;
                let dist_x = (source_pos.x - grid_x).abs();
                if dist_x < threshold
                    && (best_snap_x.is_none() || dist_x < best_snap_x.unwrap().0)
                {
                    best_snap_x = Some((
                        dist_x,
                        SnapMatch {
                            value: grid_x,
                            kind: source.kind,
                            source_kind: source.kind,
                            target_node: None,
                        },
                    ));
                }

                // Snap Y to grid
                let grid_y = (source_pos.y / self.config.grid_spacing).round() * self.config.grid_spacing;
                let dist_y = (source_pos.y - grid_y).abs();
                if dist_y < threshold
                    && (best_snap_y.is_none() || dist_y < best_snap_y.unwrap().0)
                {
                    best_snap_y = Some((
                        dist_y,
                        SnapMatch {
                            value: grid_y,
                            kind: source.kind,
                            source_kind: source.kind,
                            target_node: None,
                        },
                    ));
                }
            }
        }

        // Calculate final snapped position
        let mut snapped_offset = drag_offset;

        if let Some((_, ref snap)) = best_snap_x {
            // Find the source point that triggered this snap
            for source in source_points {
                if source.kind == snap.source_kind {
                    snapped_offset.x = snap.value - source.position.x;
                    break;
                }
            }
        }

        if let Some((_, ref snap)) = best_snap_y {
            for source in source_points {
                if source.kind == snap.source_kind {
                    snapped_offset.y = snap.value - source.position.y;
                    break;
                }
            }
        }

        SnapResult {
            position: snapped_offset,
            snap_x: best_snap_x.map(|(_, m)| m),
            snap_y: best_snap_y.map(|(_, m)| m),
        }
    }
}

/// Alignment axis for align commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlignAxis {
    /// Align left edges
    Left,
    /// Align horizontal centers
    CenterX,
    /// Align right edges
    Right,
    /// Align top edges
    Top,
    /// Align vertical centers
    CenterY,
    /// Align bottom edges
    Bottom,
}

/// Distribution axis for distribute commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributeAxis {
    /// Distribute horizontally (equal spacing between nodes)
    Horizontal,
    /// Distribute vertically (equal spacing between nodes)
    Vertical,
}

/// Calculate alignment positions for a set of bounds.
pub fn calculate_alignment(bounds: &[Rect], axis: AlignAxis) -> Vec<Vec2> {
    if bounds.is_empty() {
        return vec![];
    }

    // Find the reference value (typically from the first selected or the extremes)
    let reference = match axis {
        AlignAxis::Left => bounds.iter().map(|b| b.min.x).fold(f32::INFINITY, f32::min),
        AlignAxis::CenterX => {
            let total: f32 = bounds.iter().map(|b| b.center().x).sum();
            total / bounds.len() as f32
        }
        AlignAxis::Right => bounds.iter().map(|b| b.max.x).fold(f32::NEG_INFINITY, f32::max),
        AlignAxis::Top => bounds.iter().map(|b| b.min.y).fold(f32::INFINITY, f32::min),
        AlignAxis::CenterY => {
            let total: f32 = bounds.iter().map(|b| b.center().y).sum();
            total / bounds.len() as f32
        }
        AlignAxis::Bottom => bounds.iter().map(|b| b.max.y).fold(f32::NEG_INFINITY, f32::max),
    };

    // Calculate the delta for each node
    bounds
        .iter()
        .map(|b| match axis {
            AlignAxis::Left => Vec2::new(reference - b.min.x, 0.0),
            AlignAxis::CenterX => Vec2::new(reference - b.center().x, 0.0),
            AlignAxis::Right => Vec2::new(reference - b.max.x, 0.0),
            AlignAxis::Top => Vec2::new(0.0, reference - b.min.y),
            AlignAxis::CenterY => Vec2::new(0.0, reference - b.center().y),
            AlignAxis::Bottom => Vec2::new(0.0, reference - b.max.y),
        })
        .collect()
}

/// Calculate distribution positions for a set of bounds.
pub fn calculate_distribution(bounds: &[Rect], axis: DistributeAxis) -> Vec<Vec2> {
    if bounds.len() < 3 {
        // Need at least 3 items to distribute
        return vec![Vec2::ZERO; bounds.len()];
    }

    // Get centers and sort by position
    let mut indexed: Vec<(usize, Vec2)> = bounds
        .iter()
        .enumerate()
        .map(|(i, b)| (i, b.center()))
        .collect();

    match axis {
        DistributeAxis::Horizontal => {
            indexed.sort_by(|a, b| a.1.x.partial_cmp(&b.1.x).unwrap());
        }
        DistributeAxis::Vertical => {
            indexed.sort_by(|a, b| a.1.y.partial_cmp(&b.1.y).unwrap());
        }
    }

    // Calculate total span and spacing
    let first_center = indexed.first().unwrap().1;
    let last_center = indexed.last().unwrap().1;

    let total_span = match axis {
        DistributeAxis::Horizontal => last_center.x - first_center.x,
        DistributeAxis::Vertical => last_center.y - first_center.y,
    };

    let spacing = total_span / (bounds.len() - 1) as f32;

    // Calculate deltas
    let mut deltas = vec![Vec2::ZERO; bounds.len()];

    for (sorted_idx, (original_idx, center)) in indexed.iter().enumerate() {
        let target = match axis {
            DistributeAxis::Horizontal => first_center.x + spacing * sorted_idx as f32,
            DistributeAxis::Vertical => first_center.y + spacing * sorted_idx as f32,
        };

        let delta = match axis {
            DistributeAxis::Horizontal => Vec2::new(target - center.x, 0.0),
            DistributeAxis::Vertical => Vec2::new(0.0, target - center.y),
        };

        deltas[*original_idx] = delta;
    }

    deltas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snap_point_from_bounds() {
        let bounds = Rect::new(Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0));
        let points = SnapPoint::from_bounds(&bounds, None);

        // Should have 10 points: 6 edges + 4 corners
        assert_eq!(points.len(), 10);

        // Check some specific points
        let left = points.iter().find(|p| p.kind == SnapKind::Left).unwrap();
        assert_eq!(left.position.x, 0.0);

        let right = points.iter().find(|p| p.kind == SnapKind::Right).unwrap();
        assert_eq!(right.position.x, 100.0);

        let center_x = points.iter().find(|p| p.kind == SnapKind::CenterX).unwrap();
        assert_eq!(center_x.position.x, 50.0);
    }

    #[test]
    fn test_snap_engine_horizontal() {
        let mut engine = SnapEngine::new();

        // Set up a target at x=100
        let target_bounds = Rect::new(Vec2::new(100.0, 0.0), Vec2::new(150.0, 50.0));
        engine.set_targets(SnapPoint::from_bounds(&target_bounds, None));

        // Source at x=0, dragging to x=95 (within threshold of 100)
        let source_bounds = Rect::new(Vec2::new(0.0, 100.0), Vec2::new(50.0, 150.0));
        let source_points = SnapPoint::from_bounds(&source_bounds, None);

        let result = engine.find_snap(&source_points, Vec2::new(95.0, 0.0), 1.0);

        // Should snap the right edge (50) to the left edge (100)
        // So offset should be 50 (to move right edge from 50 to 100)
        assert!(result.snap_x.is_some());
    }

    #[test]
    fn test_calculate_alignment_left() {
        let bounds = vec![
            Rect::new(Vec2::new(10.0, 0.0), Vec2::new(50.0, 20.0)),
            Rect::new(Vec2::new(30.0, 30.0), Vec2::new(80.0, 50.0)),
            Rect::new(Vec2::new(20.0, 60.0), Vec2::new(60.0, 80.0)),
        ];

        let deltas = calculate_alignment(&bounds, AlignAxis::Left);

        // All should align to x=10 (the leftmost)
        assert_eq!(deltas.len(), 3);
        assert!((deltas[0].x - 0.0).abs() < 0.001); // Already at 10
        assert!((deltas[1].x - (-20.0)).abs() < 0.001); // Move from 30 to 10
        assert!((deltas[2].x - (-10.0)).abs() < 0.001); // Move from 20 to 10
    }

    #[test]
    fn test_calculate_distribution_horizontal() {
        let bounds = vec![
            Rect::new(Vec2::new(0.0, 0.0), Vec2::new(20.0, 20.0)),   // center.x = 10
            Rect::new(Vec2::new(100.0, 0.0), Vec2::new(120.0, 20.0)), // center.x = 110
            Rect::new(Vec2::new(30.0, 0.0), Vec2::new(50.0, 20.0)),   // center.x = 40
        ];

        let deltas = calculate_distribution(&bounds, DistributeAxis::Horizontal);

        // Should distribute evenly from 10 to 110
        // Spacing = (110 - 10) / 2 = 50
        // Positions should be: 10, 60, 110
        // First (center=10): delta = 0
        // Second (center=110): delta = 0
        // Third (center=40): delta = 60 - 40 = 20

        assert_eq!(deltas.len(), 3);
        assert!((deltas[0].x - 0.0).abs() < 0.001);
        assert!((deltas[1].x - 0.0).abs() < 0.001);
        assert!((deltas[2].x - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_snap_kind_axis() {
        assert!(SnapKind::Left.is_horizontal());
        assert!(SnapKind::CenterX.is_horizontal());
        assert!(SnapKind::Right.is_horizontal());
        assert!(!SnapKind::Top.is_horizontal());

        assert!(SnapKind::Top.is_vertical());
        assert!(SnapKind::CenterY.is_vertical());
        assert!(SnapKind::Bottom.is_vertical());
        assert!(SnapKind::Baseline.is_vertical());
        assert!(!SnapKind::Left.is_vertical());
    }
}
