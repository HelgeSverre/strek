//! Label widget - single-line text display.

use crate::draw::TextStyle;
use crate::layout::{Align, BoxConstraints, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetState};
use crate::Color;

/// A single-line text label.
#[derive(Debug, Clone)]
pub struct Label {
    /// The text content.
    pub text: String,
    /// Font size override.
    pub font_size: Option<f32>,
    /// Color override.
    pub color: Option<Color>,
    /// Whether bold.
    pub bold: bool,
    /// Vertical alignment.
    pub v_align: Align,
}

impl Label {
    /// Create a new label.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font_size: None,
            color: None,
            bold: false,
            v_align: Align::Center,
        }
    }

    /// Set font size.
    pub fn size(mut self, size: f32) -> Self {
        self.font_size = Some(size);
        self
    }

    /// Set color.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Make bold.
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Set secondary color from theme.
    pub fn secondary(mut self) -> Self {
        // Will use theme.text_secondary in draw
        self.color = None;
        self
    }
}

impl Widget for Label {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        let font_size = self.font_size.unwrap_or(ctx.theme.font_size);
        let line_height = font_size * ctx.theme.line_height;
        let char_width = font_size * 0.6;
        let text_width = self.text.len() as f32 * char_width;

        constraints.constrain(Size::new(text_width, line_height))
    }

    fn draw(&self, rect: Rect, _state: &WidgetState, ctx: &mut DrawContext) {
        let font_size = self.font_size.unwrap_or(ctx.theme.font_size);
        let line_height = font_size * ctx.theme.line_height;

        // Calculate vertical offset for alignment
        let y_offset = self.v_align.offset(line_height, rect.height);

        let text_rect = Rect::new(rect.x, rect.y + y_offset, rect.width, line_height);

        let style = TextStyle {
            font_size,
            color: self.color.unwrap_or(ctx.theme.text_primary),
            bold: self.bold,
            italic: false,
        };

        ctx.draw_list.text_clipped(&self.text, text_rect, style);
    }
}
