//! Icon widget.

use crate::draw::IconKind;
use crate::layout::{BoxConstraints, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetState};
use crate::Color;

/// A widget that displays an icon.
#[derive(Debug, Clone)]
pub struct Icon {
    /// The icon to display.
    pub kind: IconKind,
    /// Size of the icon (width and height).
    pub size: Option<f32>,
    /// Color override.
    pub color: Option<Color>,
}

impl Icon {
    /// Create a new icon widget.
    pub fn new(kind: IconKind) -> Self {
        Self {
            kind,
            size: None,
            color: None,
        }
    }

    /// Set the icon size.
    pub fn size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }

    /// Set the icon color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl Widget for Icon {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        let size = self.size.unwrap_or(ctx.theme.icon_size);
        constraints.constrain(Size::new(size, size))
    }

    fn draw(&self, rect: Rect, state: &WidgetState, ctx: &mut DrawContext) {
        let color = if state.disabled {
            ctx.theme.text_disabled
        } else {
            self.color.unwrap_or(ctx.theme.text_secondary)
        };

        ctx.draw_list.icon(self.kind, rect, color);
    }
}
