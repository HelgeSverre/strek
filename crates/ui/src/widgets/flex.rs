//! Flex widget - flexbox-like layout.

use crate::layout::{Axis, BoxConstraints, CrossAlign, MainAlign, Size};
use crate::rect::Rect;
use crate::widget::{DrawContext, LayoutContext, Widget, WidgetNode, WidgetState};

/// A flex container that lays out children along an axis.
pub struct Flex {
    /// Layout direction.
    pub axis: Axis,
    /// Children widgets.
    pub children: Vec<FlexChild>,
    /// Main axis alignment.
    pub main_align: MainAlign,
    /// Cross axis alignment.
    pub cross_align: CrossAlign,
    /// Spacing between children.
    pub spacing: f32,
}

/// A child in a flex container with optional flex factor.
pub struct FlexChild {
    /// The widget node.
    pub node: WidgetNode,
    /// Flex factor (0 = fixed size, >0 = flexible).
    pub flex: f32,
}

impl Flex {
    /// Create a new horizontal flex container.
    pub fn row() -> Self {
        Self {
            axis: Axis::Horizontal,
            children: Vec::new(),
            main_align: MainAlign::Start,
            cross_align: CrossAlign::Center,
            spacing: 0.0,
        }
    }

    /// Create a new vertical flex container.
    pub fn column() -> Self {
        Self {
            axis: Axis::Vertical,
            children: Vec::new(),
            main_align: MainAlign::Start,
            cross_align: CrossAlign::Stretch,
            spacing: 0.0,
        }
    }

    /// Add a child with no flex.
    pub fn child(mut self, widget: impl Widget + 'static) -> Self {
        self.children.push(FlexChild {
            node: WidgetNode::new(widget),
            flex: 0.0,
        });
        self
    }

    /// Add a child with a flex factor.
    pub fn flex_child(mut self, flex: f32, widget: impl Widget + 'static) -> Self {
        self.children.push(FlexChild {
            node: WidgetNode::new(widget),
            flex,
        });
        self
    }

    /// Set main axis alignment.
    pub fn main_align(mut self, align: MainAlign) -> Self {
        self.main_align = align;
        self
    }

    /// Set cross axis alignment.
    pub fn cross_align(mut self, align: CrossAlign) -> Self {
        self.cross_align = align;
        self
    }

    /// Set spacing between children.
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Lay out children and compute their positions.
    #[allow(dead_code)]
    fn layout_children(&mut self, rect: Rect, ctx: &LayoutContext) {
        if self.children.is_empty() {
            return;
        }

        let axis = self.axis;
        let main_size = axis.main(Size::new(rect.width, rect.height));
        let cross_size = axis.cross_size(Size::new(rect.width, rect.height));

        // First pass: measure non-flex children and collect flex factors
        let mut total_flex = 0.0;
        let mut fixed_main = 0.0;
        let spacing_total = self.spacing * self.children.len().saturating_sub(1) as f32;

        for child in &mut self.children {
            if child.flex > 0.0 {
                total_flex += child.flex;
            } else {
                // Measure fixed child
                let constraints = match axis {
                    Axis::Horizontal => BoxConstraints::new(0.0, f32::INFINITY, 0.0, cross_size),
                    Axis::Vertical => BoxConstraints::new(0.0, cross_size, 0.0, f32::INFINITY),
                };
                let size = child.node.measure(constraints, ctx);
                fixed_main += axis.main(size);
            }
        }

        // Calculate flex space
        let flex_space = (main_size - fixed_main - spacing_total).max(0.0);
        let flex_unit = if total_flex > 0.0 {
            flex_space / total_flex
        } else {
            0.0
        };

        // Second pass: measure flex children
        for child in &mut self.children {
            if child.flex > 0.0 {
                let child_main = flex_unit * child.flex;
                let constraints = match axis {
                    Axis::Horizontal => BoxConstraints::tight(Size::new(child_main, cross_size)),
                    Axis::Vertical => BoxConstraints::tight(Size::new(cross_size, child_main)),
                };
                child.node.measure(constraints, ctx);
            }
        }

        // Third pass: position children
        let mut main_offset = match self.main_align {
            MainAlign::Start => 0.0,
            MainAlign::End => main_size - fixed_main - flex_space - spacing_total,
            MainAlign::Center => (main_size - fixed_main - flex_space - spacing_total) / 2.0,
            MainAlign::SpaceBetween => 0.0,
            MainAlign::SpaceAround => 0.0,
            MainAlign::SpaceEvenly => 0.0,
        };

        for child in &mut self.children {
            let child_size = child.node.measured_size;
            let child_main = axis.main(child_size);
            let child_cross = axis.cross_size(child_size);

            // Cross axis alignment
            let cross_offset = match self.cross_align {
                CrossAlign::Start => 0.0,
                CrossAlign::End => cross_size - child_cross,
                CrossAlign::Center => (cross_size - child_cross) / 2.0,
                CrossAlign::Stretch => 0.0,
            };

            let final_cross_size = if self.cross_align == CrossAlign::Stretch {
                cross_size
            } else {
                child_cross
            };

            // Compute child rect
            let child_rect = match axis {
                Axis::Horizontal => Rect::new(
                    rect.x + main_offset,
                    rect.y + cross_offset,
                    child_main,
                    final_cross_size,
                ),
                Axis::Vertical => Rect::new(
                    rect.x + cross_offset,
                    rect.y + main_offset,
                    final_cross_size,
                    child_main,
                ),
            };

            child.node.layout(child_rect);
            main_offset += child_main + self.spacing;
        }
    }
}

impl Widget for Flex {
    fn measure(&mut self, constraints: BoxConstraints, ctx: &LayoutContext) -> Size {
        if self.children.is_empty() {
            return constraints.constrain(Size::zero());
        }

        let axis = self.axis;
        let (_, max_main) = axis.main_constraint(constraints);
        let (_, max_cross) = axis.cross_constraint(constraints);

        let mut total_main = 0.0;
        let mut max_child_cross = 0.0f32;
        let mut has_flex = false;

        for child in &mut self.children {
            if child.flex > 0.0 {
                has_flex = true;
                continue;
            }

            let child_constraints = match axis {
                Axis::Horizontal => BoxConstraints::new(0.0, f32::INFINITY, 0.0, max_cross),
                Axis::Vertical => BoxConstraints::new(0.0, max_cross, 0.0, f32::INFINITY),
            };
            let size = child.node.measure(child_constraints, ctx);
            total_main += axis.main(size);
            max_child_cross = max_child_cross.max(axis.cross_size(size));
        }

        // Add spacing
        let spacing_total = self.spacing * self.children.len().saturating_sub(1) as f32;
        total_main += spacing_total;

        // If we have flex children, we want to take all available main space
        let final_main = if has_flex { max_main } else { total_main };

        let size = axis.pack(final_main, max_child_cross);
        constraints.constrain(size)
    }

    fn draw(&self, _rect: Rect, _state: &WidgetState, ctx: &mut DrawContext) {
        // Note: layout_children should have been called before draw
        // In a proper implementation, we'd store the layout results
        for child in &self.children {
            let mut child_ctx = DrawContext {
                theme: ctx.theme,
                draw_list: ctx.draw_list,
            };
            child
                .node
                .widget
                .draw(child.node.rect, &child.node.state, &mut child_ctx);
        }
    }
}
