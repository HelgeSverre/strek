//! ScrollView widget for scrollable content.

use crate::event::{Event, EventResponse};
use crate::layout::{BoxConstraints, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetNode, WidgetState};
use glam::Vec2;

/// A scrollable container.
pub struct ScrollView {
    /// The content widget.
    pub content: Option<WidgetNode>,
    /// Current scroll offset.
    pub scroll_offset: Vec2,
    /// Whether to show vertical scrollbar.
    pub vertical: bool,
    /// Whether to show horizontal scrollbar.
    pub horizontal: bool,
    /// Content size (computed during layout).
    content_size: Size,
    /// Viewport size.
    viewport_size: Size,
    /// Scrollbar width.
    pub scrollbar_width: f32,
}

impl ScrollView {
    /// Create a new vertical scroll view.
    pub fn new() -> Self {
        Self {
            content: None,
            scroll_offset: Vec2::ZERO,
            vertical: true,
            horizontal: false,
            content_size: Size::zero(),
            viewport_size: Size::zero(),
            scrollbar_width: 8.0,
        }
    }

    /// Set the content widget.
    pub fn content(mut self, widget: impl Widget + 'static) -> Self {
        self.content = Some(WidgetNode::new(widget));
        self
    }

    /// Enable horizontal scrolling.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Disable vertical scrolling.
    pub fn no_vertical(mut self) -> Self {
        self.vertical = false;
        self
    }

    /// Scroll by a delta.
    pub fn scroll_by(&mut self, delta: Vec2) {
        self.scroll_offset += delta;
        self.clamp_scroll();
    }

    /// Scroll to a position.
    pub fn scroll_to(&mut self, offset: Vec2) {
        self.scroll_offset = offset;
        self.clamp_scroll();
    }

    /// Clamp scroll offset to valid range.
    fn clamp_scroll(&mut self) {
        let max_x = (self.content_size.width - self.viewport_size.width).max(0.0);
        let max_y = (self.content_size.height - self.viewport_size.height).max(0.0);

        self.scroll_offset.x = self.scroll_offset.x.clamp(0.0, max_x);
        self.scroll_offset.y = self.scroll_offset.y.clamp(0.0, max_y);
    }

    /// Check if vertical scrollbar should be visible.
    fn needs_v_scrollbar(&self) -> bool {
        self.vertical && self.content_size.height > self.viewport_size.height
    }

    /// Check if horizontal scrollbar should be visible.
    fn needs_h_scrollbar(&self) -> bool {
        self.horizontal && self.content_size.width > self.viewport_size.width
    }
}

impl Default for ScrollView {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for ScrollView {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        // Measure content with unbounded constraints in scroll directions
        let content_constraints = BoxConstraints::new(
            0.0,
            if self.horizontal {
                f32::INFINITY
            } else {
                constraints.max_width
            },
            0.0,
            if self.vertical {
                f32::INFINITY
            } else {
                constraints.max_height
            },
        );

        if let Some(content) = &mut self.content {
            self.content_size = content.measure(content_constraints, ctx);
        } else {
            self.content_size = Size::zero();
        }

        // Viewport takes available space
        self.viewport_size = constraints.max_size();
        self.clamp_scroll();

        constraints.constrain(self.viewport_size)
    }

    fn draw(&self, rect: Rect, _state: &WidgetState, ctx: &mut DrawContext) {
        // Push clip rect
        ctx.draw_list.push_clip(rect);

        // Draw content offset by scroll
        if let Some(content) = &self.content {
            let content_rect = Rect::new(
                rect.x - self.scroll_offset.x,
                rect.y - self.scroll_offset.y,
                self.content_size.width,
                self.content_size.height,
            );

            let mut child_ctx = DrawContext {
                theme: ctx.theme,
                draw_list: ctx.draw_list,
            };
            content
                .widget
                .draw(content_rect, &content.state, &mut child_ctx);
        }

        ctx.draw_list.pop_clip();

        // Draw scrollbars
        if self.needs_v_scrollbar() {
            let scrollbar_rect = Rect::new(
                rect.right() - self.scrollbar_width,
                rect.y,
                self.scrollbar_width,
                rect.height,
            );

            // Track
            ctx.draw_list
                .fill_rect(scrollbar_rect, ctx.theme.bg_tertiary);

            // Thumb
            let visible_ratio = self.viewport_size.height / self.content_size.height;
            let thumb_height = (rect.height * visible_ratio).max(20.0);
            let max_scroll = self.content_size.height - self.viewport_size.height;
            let scroll_ratio = if max_scroll > 0.0 {
                self.scroll_offset.y / max_scroll
            } else {
                0.0
            };
            let thumb_y = rect.y + (rect.height - thumb_height) * scroll_ratio;

            let thumb_rect = Rect::new(
                rect.right() - self.scrollbar_width + 1.0,
                thumb_y,
                self.scrollbar_width - 2.0,
                thumb_height,
            );

            ctx.draw_list
                .fill_rounded_rect(thumb_rect, ctx.theme.text_disabled, 3.0);
        }

        if self.needs_h_scrollbar() {
            let scrollbar_rect = Rect::new(
                rect.x,
                rect.bottom() - self.scrollbar_width,
                rect.width,
                self.scrollbar_width,
            );

            // Track
            ctx.draw_list
                .fill_rect(scrollbar_rect, ctx.theme.bg_tertiary);

            // Thumb
            let visible_ratio = self.viewport_size.width / self.content_size.width;
            let thumb_width = (rect.width * visible_ratio).max(20.0);
            let max_scroll = self.content_size.width - self.viewport_size.width;
            let scroll_ratio = if max_scroll > 0.0 {
                self.scroll_offset.x / max_scroll
            } else {
                0.0
            };
            let thumb_x = rect.x + (rect.width - thumb_width) * scroll_ratio;

            let thumb_rect = Rect::new(
                thumb_x,
                rect.bottom() - self.scrollbar_width + 1.0,
                thumb_width,
                self.scrollbar_width - 2.0,
            );

            ctx.draw_list
                .fill_rounded_rect(thumb_rect, ctx.theme.text_disabled, 3.0);
        }
    }

    fn event(&mut self, event: &Event, _rect: Rect, _state: &mut WidgetState) -> EventResponse {
        match event {
            Event::Scroll { delta, .. } => {
                self.scroll_by(-*delta);
                EventResponse::Handled
            }
            _ => EventResponse::Ignored,
        }
    }
}
