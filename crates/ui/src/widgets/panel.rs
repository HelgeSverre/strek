//! Panel widget - a titled container for grouping content.

use crate::draw::TextStyle;
use crate::layout::{BoxConstraints, Padding, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetNode, WidgetState};
use crate::Color;

/// A panel with an optional header and content area.
pub struct Panel {
    /// Panel title (shown in header).
    pub title: Option<String>,
    /// Content widget.
    pub content: Option<WidgetNode>,
    /// Background color.
    pub background: Color,
    /// Header background color.
    pub header_background: Option<Color>,
    /// Border color.
    pub border_color: Option<Color>,
    /// Content padding.
    pub padding: Padding,
    /// Header height.
    pub header_height: f32,
    /// Fixed width.
    pub width: Option<f32>,
}

impl Panel {
    /// Create a new panel.
    pub fn new() -> Self {
        Self {
            title: None,
            content: None,
            background: Color::from_hex(0x2a2a2a),
            header_background: None,
            border_color: Some(Color::from_hex(0x1a1a1a)),
            padding: Padding::zero(),
            header_height: 32.0,
            width: None,
        }
    }

    /// Set the panel title.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Set the content widget.
    pub fn content(mut self, widget: impl Widget + 'static) -> Self {
        self.content = Some(WidgetNode::new(widget));
        self
    }

    /// Set the background color.
    pub fn background(mut self, color: Color) -> Self {
        self.background = color;
        self
    }

    /// Set the header background color.
    pub fn header_background(mut self, color: Color) -> Self {
        self.header_background = Some(color);
        self
    }

    /// Set content padding.
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Padding::all(padding);
        self
    }

    /// Set fixed width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Get the header height (0 if no title).
    fn effective_header_height(&self) -> f32 {
        if self.title.is_some() {
            self.header_height
        } else {
            0.0
        }
    }
}

impl Default for Panel {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Panel {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        let mut constraints = constraints;
        if let Some(w) = self.width {
            constraints = constraints.with_width(w);
        }

        let header_height = self.effective_header_height();

        // Measure content
        let content_size = if let Some(content) = &mut self.content {
            let content_constraints = BoxConstraints::new(
                0.0,
                (constraints.max_width - self.padding.horizontal()).max(0.0),
                0.0,
                (constraints.max_height - header_height - self.padding.vertical()).max(0.0),
            );
            content.measure(content_constraints, ctx)
        } else {
            Size::zero()
        };

        let size = Size::new(
            content_size.width + self.padding.horizontal(),
            content_size.height + header_height + self.padding.vertical(),
        );

        constraints.constrain(size)
    }

    fn draw(&self, rect: Rect, _state: &WidgetState, ctx: &mut DrawContext) {
        let header_height = self.effective_header_height();

        // Draw background
        ctx.draw_list.fill_rect(rect, self.background);

        // Draw header
        if let Some(title) = &self.title {
            let header_rect = Rect::new(rect.x, rect.y, rect.width, header_height);

            // Header background
            let header_bg = self.header_background.unwrap_or(self.background);
            ctx.draw_list.fill_rect(header_rect, header_bg);

            // Header border
            if let Some(border) = self.border_color {
                let border_rect = Rect::new(rect.x, rect.y + header_height - 1.0, rect.width, 1.0);
                ctx.draw_list.fill_rect(border_rect, border);
            }

            // Title text
            let text_rect = Rect::new(
                rect.x + ctx.theme.spacing,
                rect.y,
                rect.width - ctx.theme.spacing * 2.0,
                header_height,
            );

            let style = TextStyle {
                font_size: ctx.theme.font_size_small,
                color: ctx.theme.text_secondary,
                bold: true,
                italic: false,
            };

            ctx.draw_list.text(title.to_uppercase(), text_rect, style);
        }

        // Draw left border
        if let Some(border) = self.border_color {
            let border_rect = Rect::new(rect.x, rect.y, 1.0, rect.height);
            ctx.draw_list.fill_rect(border_rect, border);
        }

        // Draw content
        if let Some(content) = &self.content {
            let content_rect = Rect::new(
                rect.x + self.padding.left,
                rect.y + header_height + self.padding.top,
                rect.width - self.padding.horizontal(),
                rect.height - header_height - self.padding.vertical(),
            );

            let mut child_ctx = DrawContext {
                theme: ctx.theme,
                draw_list: ctx.draw_list,
            };
            content
                .widget
                .draw(content_rect, &content.state, &mut child_ctx);
        }
    }
}
