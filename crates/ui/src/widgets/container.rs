//! Container widget - wraps a child with padding, background, etc.

use crate::layout::{BoxConstraints, Padding, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetNode, WidgetState};
use crate::Color;

/// A container that wraps a child widget with styling.
pub struct Container {
    /// The child widget.
    pub child: Option<WidgetNode>,
    /// Padding around the child.
    pub padding: Padding,
    /// Background color.
    pub background: Option<Color>,
    /// Border color.
    pub border_color: Option<Color>,
    /// Border width.
    pub border_width: f32,
    /// Corner radius.
    pub corner_radius: f32,
    /// Fixed width.
    pub width: Option<f32>,
    /// Fixed height.
    pub height: Option<f32>,
}

impl Container {
    /// Create a new empty container.
    pub fn new() -> Self {
        Self {
            child: None,
            padding: Padding::zero(),
            background: None,
            border_color: None,
            border_width: 0.0,
            corner_radius: 0.0,
            width: None,
            height: None,
        }
    }

    /// Set the child widget.
    pub fn child(mut self, child: impl Widget + 'static) -> Self {
        self.child = Some(WidgetNode::new(child));
        self
    }

    /// Set padding on all sides.
    pub fn padding(mut self, padding: f32) -> Self {
        self.padding = Padding::all(padding);
        self
    }

    /// Set padding with individual values.
    pub fn padding_lrtb(mut self, left: f32, right: f32, top: f32, bottom: f32) -> Self {
        self.padding = Padding::new(left, right, top, bottom);
        self
    }

    /// Set the background color.
    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    /// Set a border.
    pub fn border(mut self, color: Color, width: f32) -> Self {
        self.border_color = Some(color);
        self.border_width = width;
        self
    }

    /// Set corner radius.
    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    /// Set fixed width.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Set fixed height.
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }
}

impl Default for Container {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Container {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        // Apply fixed dimensions
        let mut constraints = constraints;
        if let Some(w) = self.width {
            constraints = constraints.with_width(w);
        }
        if let Some(h) = self.height {
            constraints = constraints.with_height(h);
        }

        // Measure child
        let child_size = if let Some(child) = &mut self.child {
            let inner_constraints = BoxConstraints::new(
                0.0,
                (constraints.max_width - self.padding.horizontal()).max(0.0),
                0.0,
                (constraints.max_height - self.padding.vertical()).max(0.0),
            );
            child.measure(inner_constraints, ctx)
        } else {
            Size::zero()
        };

        // Add padding
        let size = Size::new(
            child_size.width + self.padding.horizontal(),
            child_size.height + self.padding.vertical(),
        );

        constraints.constrain(size)
    }

    fn draw(&self, rect: Rect, _state: &WidgetState, ctx: &mut DrawContext) {
        // Draw background
        if let Some(bg) = self.background {
            if self.corner_radius > 0.0 {
                ctx.draw_list
                    .fill_rounded_rect(rect, bg, self.corner_radius);
            } else {
                ctx.draw_list.fill_rect(rect, bg);
            }
        }

        // Draw border
        if let Some(border) = self.border_color {
            if self.border_width > 0.0 {
                ctx.draw_list.stroke_rect(rect, border, self.border_width);
            }
        }

        // Draw child
        if let Some(child) = &self.child {
            let child_rect = Rect::new(
                rect.x + self.padding.left,
                rect.y + self.padding.top,
                rect.width - self.padding.horizontal(),
                rect.height - self.padding.vertical(),
            );
            // Note: In a full implementation, we'd update child.rect and call draw
            // For now, we draw at the computed position
            let mut child_ctx = DrawContext {
                theme: ctx.theme,
                draw_list: ctx.draw_list,
            };
            child.widget.draw(child_rect, &child.state, &mut child_ctx);
        }
    }

    fn children(&self) -> &[WidgetNode] {
        match &self.child {
            Some(child) => std::slice::from_ref(child),
            None => &[],
        }
    }

    fn children_mut(&mut self) -> &mut [WidgetNode] {
        match &mut self.child {
            Some(child) => std::slice::from_mut(child),
            None => &mut [],
        }
    }
}
