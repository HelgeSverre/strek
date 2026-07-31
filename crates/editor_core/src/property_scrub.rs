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

#[derive(Debug, Clone)]
pub(crate) struct StyleSnapshot {
    pub id: NodeId,
    pub before: Style,
}
