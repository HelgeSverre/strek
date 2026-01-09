//! Text widget for displaying text.

use crate::draw::TextStyle;
use crate::layout::{BoxConstraints, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetState};
use crate::Color;

/// A widget that displays text.
#[derive(Debug, Clone)]
pub struct Text {
    /// The text content.
    pub content: String,
    /// Font size (if None, uses theme default).
    pub font_size: Option<f32>,
    /// Text color (if None, uses theme default).
    pub color: Option<Color>,
    /// Whether the text is bold.
    pub bold: bool,
    /// Whether to clip text that overflows.
    pub clip: bool,
}

impl Text {
    /// Create a new text widget.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            font_size: None,
            color: None,
            bold: false,
            clip: false,
        }
    }

    /// Set the font size.
    pub fn size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    /// Set the text color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Make the text bold.
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Enable clipping for overflow.
    pub fn clip(mut self) -> Self {
        self.clip = true;
        self
    }
}

impl Widget for Text {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        let font_size = self.font_size.unwrap_or(ctx.theme.font_size);
        let line_height = font_size * ctx.theme.line_height;

        // Estimate text width (rough approximation)
        // In a real implementation, this would use proper text measurement
        let char_width = font_size * 0.6;
        let text_width = self.content.len() as f32 * char_width;

        let size = Size::new(text_width, line_height);
        constraints.constrain(size)
    }

    fn draw(&self, rect: Rect, _state: &WidgetState, ctx: &mut DrawContext) {
        let style = TextStyle {
            font_size: self.font_size.unwrap_or(ctx.theme.font_size),
            color: self.color.unwrap_or(ctx.theme.text_primary),
            bold: self.bold,
            italic: false,
        };

        if self.clip {
            ctx.draw_list.text_clipped(&self.content, rect, style);
        } else {
            ctx.draw_list.text(&self.content, rect, style);
        }
    }
}
