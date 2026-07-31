//! Transaction state for live numeric-property scrubbing.

use std::sync::Arc;

use glam::Affine2;

use crate::{Layout, NodeId, Style, TextData};

/// Numeric property that can be previewed relative to its value at drag start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericPropertyTarget {
    WorldX,
    WorldY,
    Opacity,
    StrokeWidth,
    TextSize,
    AutoLayoutSpacing,
    AutoLayoutPadding,
}

/// Precision mode used by numeric-property controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericAdjustmentMode {
    /// Use the field's normal precision.
    Fine,
    /// Use the larger, clean-number precision selected by Shift.
    Coarse,
}

/// Numeric adjustment policy shared by steppers and scrubs.
#[derive(Debug, Clone, Copy, PartialEq)]
struct NumericAdjustmentSpec {
    fine_step: f32,
    coarse_step: f32,
    clean_quantum: f32,
}

impl NumericAdjustmentSpec {
    const fn step(self, mode: NumericAdjustmentMode) -> f32 {
        match mode {
            NumericAdjustmentMode::Fine => self.fine_step,
            NumericAdjustmentMode::Coarse => self.coarse_step,
        }
    }

    fn apply_delta(self, value: f32, delta: f32, mode: NumericAdjustmentMode) -> f32 {
        let value = value + delta;
        if delta == 0.0 || matches!(mode, NumericAdjustmentMode::Fine) {
            return value;
        }
        (value / self.clean_quantum).round() * self.clean_quantum
    }
}

impl NumericPropertyTarget {
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::WorldX => "Move X",
            Self::WorldY => "Move Y",
            Self::Opacity => "Change Opacity",
            Self::StrokeWidth => "Change Stroke Width",
            Self::TextSize => "Change Text Size",
            Self::AutoLayoutSpacing => "Change Layout Spacing",
            Self::AutoLayoutPadding => "Change Layout Padding",
        }
    }

    const fn adjustment_spec(self) -> NumericAdjustmentSpec {
        match self {
            Self::Opacity => NumericAdjustmentSpec {
                fine_step: 0.01,
                coarse_step: 0.1,
                clean_quantum: 0.01,
            },
            Self::StrokeWidth => NumericAdjustmentSpec {
                fine_step: 0.1,
                coarse_step: 1.0,
                clean_quantum: 1.0,
            },
            Self::WorldX
            | Self::WorldY
            | Self::TextSize
            | Self::AutoLayoutSpacing
            | Self::AutoLayoutPadding => NumericAdjustmentSpec {
                fine_step: 1.0,
                coarse_step: 10.0,
                clean_quantum: 1.0,
            },
        }
    }

    /// Return the document-space step for the requested precision mode.
    pub const fn step(self, mode: NumericAdjustmentMode) -> f32 {
        self.adjustment_spec().step(mode)
    }

    /// Apply a relative delta and round only in coarse mode.
    pub fn apply_delta(self, value: f32, delta: f32, mode: NumericAdjustmentMode) -> f32 {
        self.adjustment_spec().apply_delta(value, delta, mode)
    }
}

/// Direction used by a numeric increment or decrement control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericAdjustmentDirection {
    Decrease,
    Increase,
}

impl NumericAdjustmentDirection {
    pub const fn multiplier(self) -> f32 {
        match self {
            Self::Decrease => -1.0,
            Self::Increase => 1.0,
        }
    }
}

/// Opaque token identifying one active numeric-property scrub.
///
/// The token is tied to the editor and selection that created it. Starting a
/// new scrub invalidates the previous token.
#[derive(Debug)]
#[must_use = "a property scrub session must be previewed, committed, or cancelled"]
pub struct NumericPropertyScrubSession {
    owner: Arc<()>,
    id: u64,
}

impl NumericPropertyScrubSession {
    pub(crate) fn new(owner: Arc<()>, id: u64) -> Self {
        Self { owner, id }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn belongs_to(&self, owner: &Arc<()>) -> bool {
        Arc::ptr_eq(&self.owner, owner)
    }
}

#[derive(Debug)]
pub(crate) struct NumericPropertyScrubState {
    pub id: u64,
    pub target: NumericPropertyTarget,
    pub selection: Vec<NodeId>,
    pub revision: u64,
    pub snapshot: NumericPropertySnapshot,
}

#[derive(Debug)]
pub(crate) enum NumericPropertySnapshot {
    Transforms(Vec<TransformSnapshot>),
    Styles(Vec<StyleSnapshot>),
    Text { id: NodeId, before: TextData },
    Layout { id: NodeId, before: Layout },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TransformSnapshot {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub before: Affine2,
    pub world: Affine2,
}

#[cfg(test)]
mod tests {
    use super::{NumericAdjustmentDirection, NumericAdjustmentMode, NumericPropertyTarget};

    #[test]
    fn numeric_steps_match_the_shared_fine_and_coarse_policy() {
        let expected = [
            (NumericPropertyTarget::WorldX, 1.0, 10.0),
            (NumericPropertyTarget::WorldY, 1.0, 10.0),
            (NumericPropertyTarget::Opacity, 0.01, 0.1),
            (NumericPropertyTarget::StrokeWidth, 0.1, 1.0),
            (NumericPropertyTarget::TextSize, 1.0, 10.0),
            (NumericPropertyTarget::AutoLayoutSpacing, 1.0, 10.0),
            (NumericPropertyTarget::AutoLayoutPadding, 1.0, 10.0),
        ];

        for (target, fine, coarse) in expected {
            assert_eq!(target.step(NumericAdjustmentMode::Fine), fine);
            assert_eq!(target.step(NumericAdjustmentMode::Coarse), coarse);
        }
    }

    #[test]
    fn coarse_adjustments_round_to_clean_values() {
        assert_eq!(
            NumericPropertyTarget::StrokeWidth
                .apply_delta(2.4, 1.0, NumericAdjustmentMode::Coarse,),
            3.0
        );
        let opacity =
            NumericPropertyTarget::Opacity.apply_delta(0.57, 0.1, NumericAdjustmentMode::Coarse);
        assert!((opacity - 0.67).abs() < 0.0001);
        assert_eq!(
            NumericPropertyTarget::WorldX.apply_delta(13.7, 10.0, NumericAdjustmentMode::Coarse,),
            24.0
        );
        assert_eq!(
            NumericPropertyTarget::StrokeWidth.apply_delta(2.4, 0.1, NumericAdjustmentMode::Fine,),
            2.5
        );
        assert_eq!(
            NumericPropertyTarget::WorldX.apply_delta(13.7, 10.0, NumericAdjustmentMode::Fine),
            23.7
        );
        assert!(
            (NumericPropertyTarget::StrokeWidth.apply_delta(
                2.4,
                -1.0,
                NumericAdjustmentMode::Coarse,
            ) - 1.0)
                .abs()
                < 0.0001
        );
        assert!(
            (NumericPropertyTarget::Opacity
                .apply_delta(0.57, -0.1, NumericAdjustmentMode::Coarse,)
                - 0.47)
                .abs()
                < 0.0001
        );
        assert_eq!(
            NumericPropertyTarget::WorldX.apply_delta(13.7, 0.0, NumericAdjustmentMode::Coarse),
            13.7
        );
    }

    #[test]
    fn numeric_adjustment_directions_have_opposite_multipliers() {
        assert_eq!(NumericAdjustmentDirection::Decrease.multiplier(), -1.0);
        assert_eq!(NumericAdjustmentDirection::Increase.multiplier(), 1.0);
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StyleSnapshot {
    pub id: NodeId,
    pub before: Style,
}
