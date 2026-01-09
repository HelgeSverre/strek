//! Button widget.

use crate::draw::TextStyle;
use crate::event::{Event, EventResponse};
use crate::layout::{BoxConstraints, Padding, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetState};
use crate::Color;

/// A clickable button widget.
#[derive(Debug, Clone)]
pub struct Button {
    /// Button label text.
    pub label: String,
    /// Padding around the label.
    pub padding: Padding,
    /// Background color override.
    pub background: Option<Color>,
    /// Text color override.
    pub text_color: Option<Color>,
    /// Corner radius.
    pub corner_radius: f32,
    /// Callback ID for click handling.
    pub on_click: Option<u64>,
}

impl Button {
    /// Create a new button with a label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            padding: Padding::symmetric(12.0, 6.0),
            background: None,
            text_color: None,
            corner_radius: 4.0,
            on_click: None,
        }
    }

    /// Set padding.
    pub fn padding(mut self, horizontal: f32, vertical: f32) -> Self {
        self.padding = Padding::symmetric(horizontal, vertical);
        self
    }

    /// Set background color.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Set text color.
    pub fn text_color(mut self, color: Color) -> Self {
        self.text_color = Some(color);
        self
    }

    /// Set corner radius.
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }
}

impl Widget for Button {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        let font_size = ctx.theme.font_size;
        let line_height = font_size * ctx.theme.line_height;
        let char_width = font_size * 0.6;
        let text_width = self.label.len() as f32 * char_width;

        let size = Size::new(
            text_width + self.padding.horizontal(),
            line_height + self.padding.vertical(),
        );

        constraints.constrain(size)
    }

    fn draw(&self, rect: Rect, state: &WidgetState, ctx: &mut DrawContext) {
        // Determine background color based on state
        let bg_color = if state.disabled {
            ctx.theme.bg_tertiary
        } else if state.pressed {
            self.background
                .map(|c| c.lerp(Color::BLACK, 0.2))
                .unwrap_or(ctx.theme.bg_pressed)
        } else if state.hovered {
            self.background
                .map(|c| c.lerp(Color::WHITE, 0.1))
                .unwrap_or(ctx.theme.bg_hover)
        } else {
            self.background.unwrap_or(ctx.theme.bg_tertiary)
        };

        // Draw background
        ctx.draw_list
            .fill_rounded_rect(rect, bg_color, self.corner_radius);

        // Draw text
        let text_color = if state.disabled {
            ctx.theme.text_disabled
        } else {
            self.text_color.unwrap_or(ctx.theme.text_primary)
        };

        let text_rect = Rect::new(
            rect.x + self.padding.left,
            rect.y + self.padding.top,
            rect.width - self.padding.horizontal(),
            rect.height - self.padding.vertical(),
        );

        ctx.draw_list.text(
            &self.label,
            text_rect,
            TextStyle::new(ctx.theme.font_size, text_color),
        );
    }

    fn event(&mut self, event: &Event, rect: Rect, state: &mut WidgetState) -> EventResponse {
        match event {
            Event::PointerEnter(_) => {
                state.hovered = true;
                EventResponse::Handled
            }
            Event::PointerLeave => {
                state.hovered = false;
                state.pressed = false;
                EventResponse::Handled
            }
            Event::PointerDown(e) if rect.contains(e.position) => {
                state.pressed = true;
                EventResponse::Capture
            }
            Event::PointerUp(e) if state.pressed => {
                state.pressed = false;
                if rect.contains(e.position) {
                    // Click!
                    EventResponse::Handled
                } else {
                    EventResponse::Handled
                }
            }
            _ => EventResponse::Ignored,
        }
    }
}
