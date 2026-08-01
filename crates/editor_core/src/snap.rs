//! Snapping system for precise alignment during editing.
//!
//! This module provides snap point generation and snap target finding
//! for aligning nodes during drag operations.

use glam::Vec2;
use std::fmt;

use crate::node::NodeId;
use crate::path::Rect;
use crate::precision::{Guide, GuideAxis, GuideId};

/// Coarse category used to enable and prioritize snap targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapTarget {
    Object,
    Guide,
    Grid,
}

/// Minimum configurable screen-space snapping tolerance.
pub const MIN_SNAP_TOLERANCE: f32 = 1.0;
/// Maximum configurable screen-space snapping tolerance.
pub const MAX_SNAP_TOLERANCE: f32 = 32.0;

/// Workspace-owned snapping preferences exposed by the editor core.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapSettings {
    pub enabled: bool,
    pub objects: bool,
    pub guides: bool,
    pub grid: bool,
    pub tolerance: f32,
}

impl Default for SnapSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            objects: true,
            guides: true,
            grid: false,
            tolerance: 8.0,
        }
    }
}

impl SnapSettings {
    pub(crate) fn validate(self) -> Result<(), SnapSettingsError> {
        if self.tolerance.is_finite()
            && (MIN_SNAP_TOLERANCE..=MAX_SNAP_TOLERANCE).contains(&self.tolerance)
        {
            Ok(())
        } else {
            Err(SnapSettingsError::InvalidTolerance)
        }
    }
}

/// Invalid workspace snapping preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapSettingsError {
    InvalidTolerance,
}

impl fmt::Display for SnapSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTolerance => write!(
                formatter,
                "snap tolerance must be finite and between {MIN_SNAP_TOLERANCE} and {MAX_SNAP_TOLERANCE} screen pixels"
            ),
        }
    }
}

impl std::error::Error for SnapSettingsError {}

impl SnapTarget {
    const fn tie_priority(self) -> u8 {
        match self {
            Self::Guide => 3,
            Self::Object => 2,
            Self::Grid => 1,
        }
    }
}

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
    /// Horizontal ruler guide (affects Y position)
    GuideHorizontal,
    /// Vertical ruler guide (affects X position)
    GuideVertical,
}

impl SnapKind {
    /// Check if this is a horizontal snap (affects X position).
    pub fn is_horizontal(&self) -> bool {
        matches!(
            self,
            SnapKind::Left | SnapKind::CenterX | SnapKind::Right | SnapKind::GuideVertical
        )
    }

    /// Check if this is a vertical snap (affects Y position).
    pub fn is_vertical(&self) -> bool {
        matches!(
            self,
            SnapKind::Top
                | SnapKind::CenterY
                | SnapKind::Bottom
                | SnapKind::Baseline
                | SnapKind::GuideHorizontal
        )
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

    /// Coarse target category. Source points use `Object`.
    pub target: SnapTarget,

    /// Stable guide identity when this point represents a guide.
    pub guide_id: Option<GuideId>,
}

impl SnapPoint {
    /// Create a new snap point.
    pub fn new(position: Vec2, kind: SnapKind, node_id: Option<NodeId>) -> Self {
        Self {
            position,
            kind,
            node_id,
            target: SnapTarget::Object,
            guide_id: None,
        }
    }

    /// Create a target point for a persistent horizontal or vertical guide.
    pub fn from_guide(guide: Guide) -> Self {
        let (position, kind) = match guide.axis {
            GuideAxis::Horizontal => (Vec2::new(0.0, guide.position), SnapKind::GuideHorizontal),
            GuideAxis::Vertical => (Vec2::new(guide.position, 0.0), SnapKind::GuideVertical),
        };
        Self {
            position,
            kind,
            node_id: None,
            target: SnapTarget::Guide,
            guide_id: Some(guide.id),
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
            Self::new(
                Vec2::new(bounds.max.x, bounds.min.y),
                SnapKind::Corner,
                node_id,
            ),
            Self::new(
                Vec2::new(bounds.min.x, bounds.max.y),
                SnapKind::Corner,
                node_id,
            ),
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

    /// Stable guide matched by the operation, if any.
    pub target_guide: Option<GuideId>,

    /// Coarse category that won target selection.
    pub target: SnapTarget,
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

    /// Snap to persistent ruler guides.
    pub snap_to_guides: bool,

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
            snap_to_guides: true,
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
    pub fn find_snap(
        &self,
        source_points: &[SnapPoint],
        drag_offset: Vec2,
        zoom: f32,
    ) -> SnapResult {
        self.find_snap_with_bypass(source_points, drag_offset, zoom, false)
    }

    /// Find a snap while allowing a pointer modifier to temporarily bypass snapping.
    pub fn find_snap_with_bypass(
        &self,
        source_points: &[SnapPoint],
        drag_offset: Vec2,
        zoom: f32,
        bypass: bool,
    ) -> SnapResult {
        if !self.config.enabled || bypass {
            return SnapResult::none(drag_offset);
        }

        let zoom = zoom.abs().max(f32::EPSILON);
        let threshold = self.config.threshold;
        if !threshold.is_finite() || threshold <= 0.0 {
            return SnapResult::none(drag_offset);
        }
        let mut best_snap_x: Option<SnapCandidate> = None;
        let mut best_snap_y: Option<SnapCandidate> = None;

        // Check each source point against each target
        for source in source_points {
            let source_pos = source.position + drag_offset;

            for target in &self.targets {
                let category_enabled = match target.target {
                    SnapTarget::Object => self.config.snap_to_nodes,
                    SnapTarget::Guide => self.config.snap_to_guides,
                    SnapTarget::Grid => self.config.snap_to_grid,
                };
                if !category_enabled {
                    continue;
                }
                // Skip self-snapping
                if source.node_id.is_some() && source.node_id == target.node_id {
                    continue;
                }

                // Check horizontal snap (X alignment)
                if source.kind.is_horizontal() && target.kind.is_horizontal() {
                    let distance_screen = (source_pos.x - target.position.x).abs() * zoom;
                    if distance_screen < threshold {
                        choose_candidate(
                            &mut best_snap_x,
                            SnapCandidate {
                                distance_screen,
                                correction: target.position.x - source.position.x,
                                snap_match: SnapMatch {
                                    value: target.position.x,
                                    kind: target.kind,
                                    source_kind: source.kind,
                                    target_node: target.node_id,
                                    target_guide: target.guide_id,
                                    target: target.target,
                                },
                            },
                        );
                    }
                }

                // Check vertical snap (Y alignment)
                if source.kind.is_vertical() && target.kind.is_vertical() {
                    let distance_screen = (source_pos.y - target.position.y).abs() * zoom;
                    if distance_screen < threshold {
                        choose_candidate(
                            &mut best_snap_y,
                            SnapCandidate {
                                distance_screen,
                                correction: target.position.y - source.position.y,
                                snap_match: SnapMatch {
                                    value: target.position.y,
                                    kind: target.kind,
                                    source_kind: source.kind,
                                    target_node: target.node_id,
                                    target_guide: target.guide_id,
                                    target: target.target,
                                },
                            },
                        );
                    }
                }
            }
        }

        // Also check grid snapping
        if self.config.snap_to_grid
            && self.config.grid_spacing.is_finite()
            && self.config.grid_spacing > 0.0
        {
            for source in source_points {
                let source_pos = source.position + drag_offset;

                // Snap X to grid
                let grid_x =
                    (source_pos.x / self.config.grid_spacing).round() * self.config.grid_spacing;
                let distance_screen = (source_pos.x - grid_x).abs() * zoom;
                if distance_screen < threshold {
                    choose_candidate(
                        &mut best_snap_x,
                        SnapCandidate {
                            distance_screen,
                            correction: grid_x - source.position.x,
                            snap_match: SnapMatch {
                                value: grid_x,
                                kind: source.kind,
                                source_kind: source.kind,
                                target_node: None,
                                target_guide: None,
                                target: SnapTarget::Grid,
                            },
                        },
                    );
                }

                // Snap Y to grid
                let grid_y =
                    (source_pos.y / self.config.grid_spacing).round() * self.config.grid_spacing;
                let distance_screen = (source_pos.y - grid_y).abs() * zoom;
                if distance_screen < threshold {
                    choose_candidate(
                        &mut best_snap_y,
                        SnapCandidate {
                            distance_screen,
                            correction: grid_y - source.position.y,
                            snap_match: SnapMatch {
                                value: grid_y,
                                kind: source.kind,
                                source_kind: source.kind,
                                target_node: None,
                                target_guide: None,
                                target: SnapTarget::Grid,
                            },
                        },
                    );
                }
            }
        }

        // Calculate final snapped position
        let mut snapped_offset = drag_offset;

        if let Some(candidate) = best_snap_x {
            snapped_offset.x = candidate.correction;
        }

        if let Some(candidate) = best_snap_y {
            snapped_offset.y = candidate.correction;
        }

        SnapResult {
            position: snapped_offset,
            snap_x: best_snap_x.map(|candidate| candidate.snap_match),
            snap_y: best_snap_y.map(|candidate| candidate.snap_match),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SnapCandidate {
    distance_screen: f32,
    correction: f32,
    snap_match: SnapMatch,
}

fn choose_candidate(best: &mut Option<SnapCandidate>, candidate: SnapCandidate) {
    const EFFECTIVE_TIE_SCREEN_PX: f32 = 0.001;
    let replace = best.is_none_or(|current| {
        candidate.distance_screen + EFFECTIVE_TIE_SCREEN_PX < current.distance_screen
            || ((candidate.distance_screen - current.distance_screen).abs()
                <= EFFECTIVE_TIE_SCREEN_PX
                && candidate.snap_match.target.tie_priority()
                    > current.snap_match.target.tie_priority())
    });
    if replace {
        *best = Some(candidate);
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
            let min = bounds
                .iter()
                .map(|bounds| bounds.min.x)
                .fold(f32::INFINITY, f32::min);
            let max = bounds
                .iter()
                .map(|bounds| bounds.max.x)
                .fold(f32::NEG_INFINITY, f32::max);
            (min + max) * 0.5
        }
        AlignAxis::Right => bounds
            .iter()
            .map(|b| b.max.x)
            .fold(f32::NEG_INFINITY, f32::max),
        AlignAxis::Top => bounds.iter().map(|b| b.min.y).fold(f32::INFINITY, f32::min),
        AlignAxis::CenterY => {
            let min = bounds
                .iter()
                .map(|bounds| bounds.min.y)
                .fold(f32::INFINITY, f32::min);
            let max = bounds
                .iter()
                .map(|bounds| bounds.max.y)
                .fold(f32::NEG_INFINITY, f32::max);
            (min + max) * 0.5
        }
        AlignAxis::Bottom => bounds
            .iter()
            .map(|b| b.max.y)
            .fold(f32::NEG_INFINITY, f32::max),
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

    let start = |bounds: &Rect| match axis {
        DistributeAxis::Horizontal => bounds.min.x,
        DistributeAxis::Vertical => bounds.min.y,
    };
    let end = |bounds: &Rect| match axis {
        DistributeAxis::Horizontal => bounds.max.x,
        DistributeAxis::Vertical => bounds.max.y,
    };
    let size = |bounds: &Rect| end(bounds) - start(bounds);

    let mut indexed = bounds.iter().enumerate().collect::<Vec<_>>();
    indexed.sort_by(|(_, a), (_, b)| start(a).total_cmp(&start(b)));

    let first_start = start(indexed[0].1);
    let last_end = end(indexed[indexed.len() - 1].1);
    let occupied = indexed.iter().map(|(_, bounds)| size(bounds)).sum::<f32>();
    let gap = (last_end - first_start - occupied) / (bounds.len() - 1) as f32;

    let mut deltas = vec![Vec2::ZERO; bounds.len()];
    let mut next_start = first_start;
    for (original_idx, bounds) in indexed {
        let delta = next_start - start(bounds);
        let delta = match axis {
            DistributeAxis::Horizontal => Vec2::new(delta, 0.0),
            DistributeAxis::Vertical => Vec2::new(0.0, delta),
        };
        deltas[original_idx] = delta;
        next_start += size(bounds) + gap;
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
    fn guide_wins_effective_ties_then_object_then_grid() {
        let mut engine = SnapEngine::new();
        engine.config.snap_to_grid = true;
        engine.config.grid_spacing = 10.0;
        let object = SnapPoint::new(Vec2::new(10.0, 0.0), SnapKind::Left, None);
        let guide = SnapPoint::from_guide(Guide {
            id: GuideId::from_raw(1),
            axis: GuideAxis::Vertical,
            position: 10.0,
        });
        engine.set_targets(vec![object, guide]);

        let source = [SnapPoint::new(Vec2::ZERO, SnapKind::Left, None)];
        let result = engine.find_snap(&source, Vec2::new(9.0, 0.0), 1.0);
        assert_eq!(result.snap_x.unwrap().target, SnapTarget::Guide);

        engine.config.snap_to_guides = false;
        let result = engine.find_snap(&source, Vec2::new(9.0, 0.0), 1.0);
        assert_eq!(result.snap_x.unwrap().target, SnapTarget::Object);

        engine.config.snap_to_nodes = false;
        let result = engine.find_snap(&source, Vec2::new(9.0, 0.0), 1.0);
        assert_eq!(result.snap_x.unwrap().target, SnapTarget::Grid);
    }

    #[test]
    fn categories_and_temporary_bypass_are_independent() {
        let mut engine = SnapEngine::new();
        engine.set_targets(vec![SnapPoint::from_guide(Guide {
            id: GuideId::from_raw(1),
            axis: GuideAxis::Horizontal,
            position: 20.0,
        })]);
        let source = [SnapPoint::new(Vec2::ZERO, SnapKind::Top, None)];

        assert!(engine
            .find_snap(&source, Vec2::new(0.0, 16.0), 1.0)
            .has_snap());
        assert!(!engine
            .find_snap_with_bypass(&source, Vec2::new(0.0, 16.0), 1.0, true)
            .has_snap());
        engine.config.snap_to_guides = false;
        assert!(!engine
            .find_snap(&source, Vec2::new(0.0, 16.0), 1.0)
            .has_snap());
    }

    #[test]
    fn tolerance_remains_in_screen_pixels_across_zoom() {
        let mut engine = SnapEngine::new();
        engine.set_targets(vec![SnapPoint::new(
            Vec2::new(10.0, 0.0),
            SnapKind::Left,
            None,
        )]);
        let source = [SnapPoint::new(Vec2::ZERO, SnapKind::Left, None)];

        assert!(engine
            .find_snap(&source, Vec2::new(6.1, 0.0), 2.0)
            .has_snap());
        assert!(!engine
            .find_snap(&source, Vec2::new(5.9, 0.0), 2.0)
            .has_snap());
    }

    #[test]
    fn winning_source_point_supplies_the_exact_correction() {
        let mut engine = SnapEngine::new();
        engine.set_targets(vec![SnapPoint::new(
            Vec2::new(30.0, 0.0),
            SnapKind::Left,
            None,
        )]);
        let sources = [
            SnapPoint::new(Vec2::new(0.0, 0.0), SnapKind::Left, None),
            SnapPoint::new(Vec2::new(20.0, 0.0), SnapKind::Left, None),
        ];

        let result = engine.find_snap(&sources, Vec2::new(8.0, 0.0), 1.0);
        assert_eq!(result.position.x, 10.0);
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
            Rect::new(Vec2::new(0.0, 0.0), Vec2::new(20.0, 20.0)), // center.x = 10
            Rect::new(Vec2::new(100.0, 0.0), Vec2::new(120.0, 20.0)), // center.x = 110
            Rect::new(Vec2::new(30.0, 0.0), Vec2::new(50.0, 20.0)), // center.x = 40
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
    fn distribution_uses_equal_gaps_for_different_sizes() {
        let bounds = vec![
            Rect::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
            Rect::new(Vec2::new(20.0, 0.0), Vec2::new(40.0, 10.0)),
            Rect::new(Vec2::new(100.0, 0.0), Vec2::new(110.0, 10.0)),
        ];

        let deltas = calculate_distribution(&bounds, DistributeAxis::Horizontal);

        assert_eq!(deltas[0], Vec2::ZERO);
        assert_eq!(deltas[1], Vec2::new(25.0, 0.0));
        assert_eq!(deltas[2], Vec2::ZERO);
    }

    #[test]
    fn center_alignment_uses_the_selection_bounds_center() {
        let bounds = vec![
            Rect::new(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0)),
            Rect::new(Vec2::new(20.0, 0.0), Vec2::new(30.0, 10.0)),
            Rect::new(Vec2::new(90.0, 0.0), Vec2::new(100.0, 10.0)),
        ];

        let deltas = calculate_alignment(&bounds, AlignAxis::CenterX);

        assert_eq!(deltas[0], Vec2::new(45.0, 0.0));
        assert_eq!(deltas[1], Vec2::new(25.0, 0.0));
        assert_eq!(deltas[2], Vec2::new(-45.0, 0.0));
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
