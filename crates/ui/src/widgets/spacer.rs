//! Spacer widget for adding flexible space.

use crate::layout::{BoxConstraints, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetState};

/// A widget that takes up space but doesn't draw anything.
#[derive(Debug, Default)]
pub struct Spacer {
    /// Flex factor (how much of available space to take).
    pub flex: f32,
    /// Minimum size.
    pub min_size: Size,
}

impl Spacer {
    /// Create a spacer with a flex factor.
    pub fn flex(flex: f32) -> Self {
        Self {
            flex,
            min_size: Size::zero(),
        }
    }

    /// Create a fixed-size spacer.
    pub fn fixed(width: f32, height: f32) -> Self {
        Self {
            flex: 0.0,
            min_size: Size::new(width, height),
        }
    }

    /// Create a horizontal spacer.
    pub fn horizontal(width: f32) -> Self {
        Self::fixed(width, 0.0)
    }

    /// Create a vertical spacer.
    pub fn vertical(height: f32) -> Self {
        Self::fixed(0.0, height)
    }
}

impl Widget for Spacer {
    fn measure(&mut self, constraints: BoxConstraints, _ctx: &LayoutContext) -> Size {
        if self.flex > 0.0 {
            // Flexible spacer takes minimum required space during measure
            constraints.constrain(self.min_size)
        } else {
            // Fixed spacer takes its minimum size
            constraints.constrain(self.min_size)
        }
    }

    fn draw(&self, _rect: Rect, _state: &WidgetState, _ctx: &mut DrawContext) {
        // Spacer draws nothing
    }
}
